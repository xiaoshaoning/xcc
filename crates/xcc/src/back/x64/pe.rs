//! The portable executable: what a linked program is, written by this compiler.
//!
//! Everything below the linker already exists - `coff::assemble` gives code, data and the unwinder's
//! tables; `encode` gives the relocations to fill in - so this module is the one new thing a link
//! needs: a *file* whose sections sit at the addresses the program will run at, with an import table
//! for the C libraries and the base relocations that let Windows move it.
//!
//! The design decisions, once each:
//!
//! - **Every function is in `.text`**, in the order the units were assembled, so a `REL32` is a
//!   subtraction. A call to an import goes through a six-byte thunk, `jmp [rip+slot]`, because the
//!   encoder emits only `E8 rel32` and a thunk is what makes a library function reachable by one.
//! - **Every object is in `.data`**, writable; an `ADDR64` there becomes an absolute address *and* an
//!   entry in `.reloc`, so the program works at the base it is given and at any other Windows picks.
//! - **The unwinder's tables are regenerated, not patched**: `coff::unwind_layout` lays `.pdata` and
//!   `.xdata` out for a *set* of functions, which is exactly what a link has.
//! - **Sizes, then addresses, then contents.** A section's bytes depend on where the sections land, so
//!   the layout is computed from sizes first; nothing is written and then patched.
//! - **The entry point is a name**, resolved like any other - `mainCRTStartup`, the C function in
//!   `crates/xcc-crt/src/main.c`, so the first program this linker links is one this compiler
//!   compiled.

use std::collections::BTreeMap;

use super::coff::{self, Image};

/// Where an image is loaded unless something moves it: the address every Windows linker picks.
const IMAGE_BASE: u64 = 0x1_4000_0000;
/// Four kilobytes in memory and 512 bytes in the file: what the loader accepts and `link.exe` writes.
const SECTION_ALIGNMENT: u32 = 0x1000;
const FILE_ALIGNMENT: u32 = 0x200;
/// Two megabytes of stack and a megabyte of heap, which is what a console program asks for:
/// mingw's two megabytes are the default `clang` writes here, measured off a program it linked,
/// and the back end probes the stack so a frame anywhere inside it can be taken.
const STACK_RESERVE: u64 = 0x20_0000;
const HEAP_RESERVE: u64 = 0x10_0000;
const STACK_COMMIT: u64 = 0x1000;
const HEAP_COMMIT: u64 = 0x1000;
/// `IMAGE_FILE_EXECUTABLE_IMAGE | IMAGE_FILE_LARGE_ADDRESS_AWARE`.
const CHARACTERISTICS: u16 = 0x0022;
/// `IMAGE_SUBSYSTEM_WINDOWS_CUI`: a console program.
const SUBSYSTEM_CONSOLE: u16 = 0x0003;
/// `IMAGE_REL_BASED_DIR64`: the relocation Windows applies to a sixty-four-bit address.
const REL_BASED_DIR64: u16 = 10;
/// The data directories of the optional header, of which three are filled in.
const DIRECTORY_EXCEPTION: usize = 3;
const DIRECTORY_IMPORT: usize = 1;
const DIRECTORY_BASERELOC: usize = 5;
/// `IMAGE_DIRECTORY_ENTRY_IAT`: where the import address table is. The loader fills it either way,
/// but naming it costs eight bytes and removes a question.
const DIRECTORY_IAT: usize = 12;
/// The DOS header's size, and where the PE signature goes.
const DOS_HEADER: u32 = 0x40;
/// The optional header of a PE32+ image, which is fixed at 240 bytes.
const OPTIONAL_HEADER: u32 = 240;
/// One section header.
const SECTION_HEADER: u32 = 40;
/// Six sections: code, data, read-only data, the unwinder's two tables, and the relocations.
const SECTIONS: u32 = 6;

/// `IMAGE_SCN_CNT_CODE | IMAGE_SCN_MEM_EXECUTE | IMAGE_SCN_MEM_READ`.
const CODE_FLAGS: u32 = 0x6000_0020;
/// `IMAGE_SCN_CNT_INITIALIZED_DATA | IMAGE_SCN_MEM_READ | IMAGE_SCN_MEM_WRITE`.
const DATA_FLAGS: u32 = 0xC000_0040;
/// `IMAGE_SCN_CNT_INITIALIZED_DATA | IMAGE_SCN_MEM_READ`.
const READ_ONLY_FLAGS: u32 = 0x4000_0040;

/// A function this program does not hold: its name, and the library that has it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Imported<'a> {
    /// The library's file name, as the import table must spell it: `ucrtbase.dll`, `kernel32.dll`.
    pub library: &'a str,
    /// The name the program calls it by - what its relocations ask for.
    pub name: &'a str,
    /// The name the library exports it under, which is what the import table must look up. The two
    /// differ where the target's ABI differs from C's spelling: a program writes `time`, and the
    /// library exports `_time64`.
    pub exported: &'a str,
}

/// A program that has been linked.
pub(crate) struct Linked {
    /// The image: headers, sections, import table, base relocations.
    pub bytes: Vec<u8>,
}

/// A name nothing defined and no library offered.
pub(crate) enum LinkError {
    /// A name nothing defines and no library exports.
    Undefined { name: String },
    /// Two *tentative* definitions of one name that are not the same size. C99 6.9.2p2 lets a
    /// program write `int shared;` in every unit, and the one object they mean has one size - so
    /// keeping the first and dropping the second would leave the larger unit writing past the end of
    /// the object, into whatever `.data` put next.
    Mismatched { name: String, here: u32, there: u32 },
    /// A name more than one unit *defines*. C99 6.9p5 wants exactly one external definition of each
    /// name in a program, and a linker that takes the first of two silently builds whichever unit
    /// came first - a wrong program whenever the two are not identical, which is what `clang`'s
    /// "multiple definition of `f`" exists to prevent. A *tentative* definition does not count.
    Duplicate { name: String },
}

/// Link one program: the units in the order they were compiled, and the library functions to import.
///
/// `entry` is the symbol the PE header points at. A name that is neither defined by a unit nor in
/// `imports` is `Unlinked`, which the caller reports as a missing symbol instead of writing a
/// program that jumps to zero.
pub(crate) fn link(
    units: &[Image],
    entry: &str,
    imports: &[Imported<'_>],
) -> Result<Linked, LinkError> {
    let mut program = Program::gather(units, imports)?;
    program.entry = *program
        .functions
        .get(entry)
        .ok_or_else(|| LinkError::Undefined {
            name: entry.to_owned(),
        })?;
    Ok(Linked {
        bytes: program.write()?,
    })
}

/// The program being linked: what it defines, where, and what it needs.
struct Program<'a> {
    units: &'a [Image],
    /// Every function by name, and where it starts in `.text`.
    functions: BTreeMap<String, u32>,
    /// Every object by name, and where it starts in `.data`.
    objects: BTreeMap<String, u32>,
    /// What each unit's own names mean inside it, in the order `units` is.
    ///
    /// A unit's string literals are named `.str0`, `.str1`, ... and each unit numbers them from zero;
    /// a `static` function or object may share its name with one in another unit. The object format
    /// says which is which - static storage class means internal linkage - and a linker that keys
    /// them by name alone resolves the second unit's to the first's: measured, by a program whose
    /// second translation unit printed the first one's format string, and by one that called the
    /// first unit's `static` helper.
    local: Vec<UnitLocals>,
    /// Where each unit's code landed inside the program's `.text`, in the same order as `units`.
    unit_text: Vec<u32>,
    /// Where each unit's data landed inside the program's `.data`, in the same order as `units`.
    unit_data: Vec<u32>,
    /// The imports, grouped by library, each with the slot and thunk it was given.
    imports: Vec<Library<'a>>,
    /// `.text`: every unit's code, then one thunk per import.
    text: Vec<u8>,
    /// `.data`: every unit's objects, then the import address table.
    data: Vec<u8>,
    /// `.pdata`/`.xdata` for the whole program at once.
    unwind: coff::UnwindLayout,
    /// The offsets in `.data` that hold an absolute address, which `.reloc` must describe.
    addr64: Vec<u32>,
    /// Where the entry point is, from `.text`'s start.
    entry: u32,
    /// How many bytes one thread's copy of every `_Thread_local` object is. The runtime allocates
    /// that much per thread on first use; the linker is the only place that can add the objects up,
    /// which is why the number is a word in `.data` rather than an immediate in the code.
    thread_block: u32,
}

/// One entry of the image's symbol table: a name, the section it lives in, and where it starts in it.
///
/// The table is not a section. Nothing maps it, the loader never reads it, and it sits after the last
/// section's bytes where only a tool looks - but it is the difference between "the program faulted at
/// `.text+0x129e2d`" and "the program faulted in `sqlite3_open`", which is the whole reason it exists.
struct Symbol {
    name: String,
    /// One-based, as a section number is: `.text` is 1 unless a program has no data, which moves it.
    section: u16,
    /// From that section's start, which is what a COFF symbol's value means.
    value: u32,
    /// Type 0x20 is a function and 0 is not, which is what tells a debugger a name is a place to stop.
    function: bool,
    /// Storage class 2 is external and 3 is static. Two `static` functions of one name in two units
    /// are legal and are two symbols; two external ones are impossible - the linker refused the link.
    external: bool,
}

/// The names a unit keeps to itself: its `static` functions and objects, and the literals and other
/// compiler-made objects that are local by construction.
#[derive(Default)]
struct UnitLocals {
    functions: BTreeMap<String, u32>,
    objects: BTreeMap<String, u32>,
}

/// One library's imports, in the order the address table holds them.
struct Library<'a> {
    name: &'a str,
    /// Each function, with its slot in `.data` and its thunk in `.text`.
    functions: Vec<Slot>,
}

/// One imported function's two homes.
struct Slot {
    /// The name the program calls it by, which is how a relocation finds its slot.
    name: String,
    /// The name the library exports it under, which is what the hint/name entry says.
    exported: String,
    /// The eight-byte address-table slot, from `.data`'s start.
    slot: u32,
    /// The six-byte `jmp [rip+slot]`, from `.text`'s start.
    thunk: u32,
}

impl<'a> Program<'a> {
    /// Concatenate the units, and give each import its slot and its thunk.
    fn gather(units: &'a [Image], imports: &[Imported<'a>]) -> Result<Self, LinkError> {
        let mut program = Program {
            units,
            functions: BTreeMap::new(),
            objects: BTreeMap::new(),
            local: (0..units.len()).map(|_| UnitLocals::default()).collect(),
            unit_text: Vec::new(),
            unit_data: Vec::new(),
            imports: group(imports),
            text: Vec::new(),
            data: Vec::new(),
            unwind: coff::unwind_layout(&[]),
            addr64: Vec::new(),
            entry: 0,
            thread_block: 0,
        };
        // C99 6.9p5: one external definition of each name in a program, and 6.9.2p2: a *tentative*
        // definition - `int shared;` with no initialiser - is not an external definition and may
        // appear in every unit. So a duplicate is a name defined for real twice, or a real
        // definition beside a tentative one; **two tentative definitions are one object**, which is
        // what the standard says and what a compiler older than `-fno-common` did by merging `.comm`
        // symbols. That last rule is the one place this linker accepts a program the `clang` beside
        // it refuses; it is conforming, and AUDIT says so.
        //
        // The state lives outside the loops because every unit's functions are gathered before any
        // unit's objects, and a name checked against half of them would be checked wrongly.
        let mut real: std::collections::HashSet<&str> = std::collections::HashSet::new();
        // The names defined tentatively, and how big the object they mean is: two tentatives are one
        // object only if they agree about that.
        let mut tentative: std::collections::HashMap<&str, u32> = std::collections::HashMap::new();
        for (index, unit) in units.iter().enumerate() {
            let base = program.text.len() as u32;
            // A unit's functions are placed relative to its own code, and its relocations are
            // written there too - so where its code landed is a fact the patch needs.
            program.unit_text.push(base);
            for function in &unit.functions {
                if function.external {
                    // A function definition is never tentative: two units that both define `f` are a
                    // duplicate whatever the bodies say - and so is a function beside an object of
                    // the same name, which no declaration could have made one name.
                    let name = function.name.as_str();
                    if real.contains(name) || tentative.contains_key(name) || !real.insert(name) {
                        return Err(LinkError::Duplicate {
                            name: function.name.clone(),
                        });
                    }
                    program
                        .functions
                        .insert(function.name.clone(), base + function.start);
                } else {
                    program.local[index]
                        .functions
                        .insert(function.name.clone(), base + function.start);
                }
            }
            program.text.extend_from_slice(&unit.text);
            // Sixteen bytes apart, so a thunk is a clean place to look at in a debugger.
            while program.text.len() % 16 != 0 {
                program.text.push(0xCC); // `int3`: unreachable, and a breakpoint if it is not
            }
        }
        for library in &mut program.imports {
            for function in &mut library.functions {
                while program.text.len() % 16 != 0 {
                    program.text.push(0xCC);
                }
                function.thunk = program.text.len() as u32;
                // `jmp [rip + disp32]`: six bytes, with the displacement filled in once `.data` is
                // placed, because a slot's address is not known until the sections are.
                program.text.extend_from_slice(&[0xFF, 0x25, 0, 0, 0, 0]);
            }
        }
        for unit in units {
            // Each unit's data starts where its strictest object can: an object's alignment is a
            // property of the object, and a unit is a run of them. The padding comes first, because
            // `base` is what every symbol of this unit is placed from - computing it before the
            // padding put the symbols eight bytes above their bytes, and `__intrinsic_setjmp`'s
            // `movdqa` faulted on a `jmp_buf` that was really at 16.
            while program.data.len() % unit.data.align.max(1) as usize != 0 {
                program.data.push(0);
            }
            let base = program.data.len() as u32;
            program.unit_data.push(base);
            for object in &unit.data.symbols {
                if object.external {
                    // `int shared;` with no initialiser is a *tentative* definition, and a tentative
                    // one in every unit is still one object (C99 6.9.2p2) - the one thing this
                    // linker accepts that the `clang` beside it refuses. Beside a *real* definition
                    // it is a duplicate, in either order.
                    let name = object.name.as_str();
                    let repeated = if object.tentative {
                        if real.contains(name) {
                            true
                        } else if let Some(&earlier) = tentative.get(name) {
                            // One object, so one size: two units that disagree about it are a
                            // program error this link can see, and merging them silently would give
                            // the larger one an object it can write past.
                            if earlier != object.size {
                                return Err(LinkError::Mismatched {
                                    name: object.name.clone(),
                                    here: earlier,
                                    there: object.size,
                                });
                            }
                            false
                        } else {
                            tentative.insert(name, object.size);
                            false
                        }
                    } else {
                        real.contains(name) || tentative.contains_key(name) || !real.insert(name)
                    };
                    if repeated {
                        return Err(LinkError::Duplicate {
                            name: object.name.clone(),
                        });
                    }
                    program
                        .objects
                        .insert(object.name.clone(), base + object.offset);
                } else {
                    program.local[program.unit_data.len() - 1]
                        .objects
                        .insert(object.name.clone(), base + object.offset);
                }
            }
            program.data.extend_from_slice(&unit.data.bytes);
        }
        // The thread-local block's layout, decided here because this is where every unit's objects
        // are in one place. Each object is placed at its own alignment, and its *offset in the
        // block* is written into a four-byte word named `__xcc_tls_offset_<name>` in `.data`: the
        // code that reads a `_Thread_local` object is compiled before this layout exists, so it
        // reads the offset rather than carrying it as an immediate. The block's total size goes the
        // same way, into one word, because the runtime allocates the whole block at once.
        // The template: every thread-local object's bytes, dense, at the offset it will occupy in a
        // thread's block. It has to be *one* run of bytes rather than one per object, because the
        // runtime copies a whole block when a thread first touches any of it - per-object copies
        // would leave every object but the first at zero, which is exactly what the first version
        // of this did.
        let mut block: u32 = 0;
        let mut template: Vec<u8> = Vec::new();
        for unit in units.iter() {
            for object in &unit.data.symbols {
                if !object.thread {
                    continue;
                }
                let align = u64::from(object.align.max(1));
                block = (u64::from(block).div_ceil(align) * align) as u32;
                let offset = block;
                let start = object.offset as usize;
                let end = start + object.size as usize;
                // The template grows to the object's *whole* size before its bytes go in: an object
                // with no initialiser has no bytes in `.data` at all, and a template shorter than
                // the block would have the runtime copy past the end of it.
                template.resize(offset as usize + object.size as usize, 0);
                let bytes = unit.data.bytes.get(start..end).unwrap_or(&[]);
                template[offset as usize..offset as usize + bytes.len()].copy_from_slice(bytes);
                block += object.size.max(1);
                let offset_word = program.data.len() as u32;
                program.data.extend_from_slice(&offset.to_le_bytes());
                program
                    .objects
                    .insert(format!("__xcc_tls_offset_{}", object.name), offset_word);
            }
        }
        program.thread_block = block;
        if block > 0 {
            template.resize(block as usize, 0);
            while program.data.len() % 16 != 0 {
                program.data.push(0);
            }
            let template_at = program.data.len() as u32;
            program.data.extend_from_slice(&template);
            program
                .objects
                .insert("__xcc_tls_template".to_owned(), template_at);
            let total_word = program.data.len() as u32;
            program.data.extend_from_slice(&block.to_le_bytes());
            program
                .objects
                .insert("__xcc_tls_total".to_owned(), total_word);
        }
        // The address table: one eight-byte slot per import, then the null that ends it. The slots
        // start empty here and are filled from the import table, which is what knows the RVAs.
        while program.data.len() % 8 != 0 {
            program.data.push(0);
        }
        for library in &mut program.imports {
            for function in &mut library.functions {
                function.slot = program.data.len() as u32;
                program.data.extend_from_slice(&0u64.to_le_bytes());
            }
        }
        program.data.extend_from_slice(&0u64.to_le_bytes());
        let described: Vec<coff::FunctionUnwind> = units
            .iter()
            .enumerate()
            .flat_map(|(index, unit)| {
                unit.unwind
                    .iter()
                    .map(move |function| coff::FunctionUnwind {
                        name: function.name.clone(),
                        unit: index,
                        size: function.size,
                        info: function.info.clone(),
                    })
            })
            .collect();
        program.unwind = coff::unwind_layout(&described);
        Ok(program)
    }

    /// The offset of a name from the start of the image: `.text` for a function, `.data` for an
    /// object, a thunk for anything imported.
    /// The address `name` refers to from inside unit `unit`: its own objects first, then the
    /// program's.
    ///
    /// The order is the whole point. `.str0` means this unit's first literal, and every unit has
    /// one; a name that is not local - a function, an external object, an import - is the program's
    /// and is found by `address`.
    fn within(&self, unit: Option<usize>, name: &str, sections: &Sections) -> Option<u32> {
        if let Some(local) = unit.and_then(|unit| self.local.get(unit)) {
            if let Some(offset) = local.functions.get(name) {
                return Some(sections.text.address + offset);
            }
            if let Some(offset) = local.objects.get(name) {
                return Some(sections.data.address + offset);
            }
        }
        if let Some(offset) = self.functions.get(name) {
            return Some(sections.text.address + offset);
        }
        if let Some(offset) = self.objects.get(name) {
            return Some(sections.data.address + offset);
        }
        self.imports
            .iter()
            .flat_map(|library| &library.functions)
            .find(|function| function.name == name)
            .map(|function| sections.text.address + function.thunk)
    }

    /// Lay the sections out, fill the relocations in, then write the file.
    fn write(&mut self) -> Result<Vec<u8>, LinkError> {
        let sizes = Sizes::of(self);
        let sections = Sections::new(sizes);
        self.resolve(&sections)?;
        Ok(Writer::new(self, &sections).write())
    }

    /// Fill every relocation in, now that the addresses are known.
    fn resolve(&mut self, sections: &Sections) -> Result<(), LinkError> {
        for (index, unit) in self.units.iter().enumerate() {
            for function in &unit.functions {
                for relocation in &function.relocations {
                    let target = self
                        .within(Some(index), &relocation.symbol, sections)
                        .ok_or_else(|| LinkError::Undefined {
                            name: relocation.symbol.clone(),
                        })?;
                    // `rel32` is measured from the byte after the four-byte displacement, and it is
                    // *written* at the unit's own place in `.text` - a second unit's code starts
                    // where the first one ended, and patching at its unit-relative offset would put
                    // its displacement inside the first unit's instructions.
                    let place = sections.text.address
                        + self.unit_text[index]
                        + function.start
                        + relocation.offset;
                    let displacement = i64::from(target) - i64::from(place) - 4;
                    let at = (self.unit_text[index] + function.start + relocation.offset) as usize;
                    self.text[at..at + 4].copy_from_slice(&(displacement as i32).to_le_bytes());
                }
            }
            // An address in an object is absolute, and `.reloc` tells the loader to fix it if the
            // image is moved.
            for relocation in &unit.data.relocations {
                let target = self
                    .within(Some(index), &relocation.symbol, sections)
                    .ok_or_else(|| LinkError::Undefined {
                        name: relocation.symbol.clone(),
                    })?;
                let at = (self.unit_data[index] + relocation.offset) as usize;
                // `S + A`: the addend is how far into the symbol the address is, and it is what
                // makes `&table[3]` an address and not a missing feature.
                let absolute =
                    (IMAGE_BASE + u64::from(target)).wrapping_add(relocation.addend as u64);
                self.data[at..at + 8].copy_from_slice(&absolute.to_le_bytes());
                self.addr64.push(at as u32);
            }
        }
        // Each thunk's displacement, which points at its own slot.
        for library in &self.imports {
            for function in &library.functions {
                let at = function.thunk as usize + 2;
                let next = sections.text.address + function.thunk + 6;
                let slot = sections.data.address + function.slot;
                let displacement = i64::from(slot) - i64::from(next);
                self.text[at..at + 4].copy_from_slice(&(displacement as i32).to_le_bytes());
            }
        }
        Ok(())
    }
}

/// Group the imports by library, keeping each library's functions in the order given.
fn group<'a>(imports: &[Imported<'a>]) -> Vec<Library<'a>> {
    let mut libraries: Vec<Library<'a>> = Vec::new();
    for import in imports {
        let index = match libraries.iter().position(|one| one.name == import.library) {
            Some(index) => index,
            None => {
                libraries.push(Library {
                    name: import.library,
                    functions: Vec::new(),
                });
                libraries.len() - 1
            }
        };
        libraries[index].functions.push(Slot {
            name: import.name.to_owned(),
            exported: import.exported.to_owned(),
            slot: 0,
            thunk: 0,
        });
    }
    libraries
}

/// How big each section is, before anything knows where it goes.
struct Sizes {
    text: u32,
    data: u32,
    rdata: u32,
    pdata: u32,
    xdata: u32,
    // No `.reloc`: it is the last section, so its size changes nothing before it, and it is computed
    // exactly - by the one function that writes it - once the addresses are known. A guessed size
    // here was a *lower* bound (it forgot a page header per page of data), so a large program would
    // have written past the space reserved for it.
}

impl Sizes {
    fn of(program: &Program<'_>) -> Self {
        Sizes {
            text: program.text.len() as u32,
            data: program.data.len() as u32,
            rdata: import_table_size(&program.imports),
            pdata: program.unwind.entries.len() as u32 * 12,
            xdata: program.unwind.xdata.len() as u32,
        }
    }
}

/// Where a section is: its address in memory, and where its bytes start in the file. Two numbers with
/// two meanings, so they are named rather than a pair that every reader has to decode.
#[derive(Clone, Copy)]
struct Where {
    address: u32,
    file: u32,
}

/// Where each section is.
struct Sections {
    text: Where,
    data: Where,
    rdata: Where,
    pdata: Where,
    xdata: Where,
    reloc: Where,
    /// Where the sections start in the file: the headers, rounded up.
    headers: u32,
}

impl Sections {
    fn new(sizes: Sizes) -> Self {
        let headers = align(
            DOS_HEADER + 4 + 20 + OPTIONAL_HEADER + SECTION_HEADER * SECTIONS,
            FILE_ALIGNMENT,
        );
        // Each section starts after the one before it: its *bytes*, which is what the sizes say.
        let text = place(headers, sizes.text);
        let data = place(text.address + sizes.text, sizes.data);
        let rdata = place(data.address + sizes.data, sizes.rdata);
        let pdata = place(rdata.address + sizes.rdata, sizes.pdata);
        let xdata = place(pdata.address + sizes.pdata, sizes.xdata);
        let reloc = place(xdata.address + sizes.xdata, 0);
        Sections {
            text,
            data,
            rdata,
            pdata,
            xdata,
            reloc,
            headers,
        }
    }
}

/// One section's place, from where the section before it ended. Every section starts on a page in
/// memory and on a file block, which is what makes the two offsets computable from sizes alone.
fn place(after: u32, _size: u32) -> Where {
    let address = align(after, SECTION_ALIGNMENT);
    Where {
        address,
        file: align(address, FILE_ALIGNMENT),
    }
}

/// One hint/name entry: a two-byte hint, the name, a null, and a pad byte if that would leave the
/// entry at an odd address - which the loader requires it not to. The size count and the writer both
/// ask here, because a size that disagrees with what is written truncates the table.
fn name_entry_size(name: &str) -> u32 {
    let entry = 2 + name.len() as u32 + 1;
    entry + entry % 2
}

/// Round `value` up to `alignment`, which is always a power of two here.
fn align(value: u32, alignment: u32) -> u32 {
    value.div_ceil(alignment) * alignment
}

/// The import table's size: one descriptor per library and the null one, each library's lookup table,
/// the hint/name entries, and the library names.
fn import_table_size(libraries: &[Library<'_>]) -> u32 {
    if libraries.is_empty() {
        return 0;
    }
    let descriptors = (libraries.len() as u32 + 1) * 20;
    let lookups: u32 = libraries
        .iter()
        .map(|library| (library.functions.len() as u32 + 1) * 8)
        .sum();
    let names: u32 = libraries
        .iter()
        .flat_map(|library| &library.functions)
        .map(|function| name_entry_size(&function.exported))
        .sum();
    let libraries_named: u32 = libraries
        .iter()
        .map(|library| library.name.len() as u32 + 1)
        .sum();
    // No slack: the assertion in the writer checks this against what is actually written, and a
    // size with two bytes of room in it is a size nobody is checking.
    descriptors + lookups + names + libraries_named
}

/// Where the sections' bytes end in the file: where the symbol table starts, since nothing maps it.
fn end_of_sections(list: &[Section], headers: u32) -> u32 {
    list.last().map_or(headers, |section| {
        section.place.file + align(section.content.len() as u32, FILE_ALIGNMENT)
    })
}

/// The COFF symbol table and its string table, which is where a name longer than eight bytes lives.
///
/// Eighteen bytes per symbol - an eight-byte name, a value, a section, a type, a storage class and an
/// auxiliary-record count this never uses - and then a string table whose first four bytes are its own
/// length, including themselves. A name of eight bytes or fewer is stored in the record and needs no
/// null; anything longer puts four zero bytes in the name field and its offset in the last four. One
/// record per symbol, which is the number the file header promises.
fn symbol_table(symbols: &[Symbol]) -> Vec<u8> {
    let mut records = Vec::with_capacity(symbols.len() * 18);
    let mut strings: Vec<u8> = vec![0, 0, 0, 0];
    for symbol in symbols {
        let mut name = [0u8; 8];
        if symbol.name.len() <= 8 {
            name[..symbol.name.len()].copy_from_slice(symbol.name.as_bytes());
        } else {
            let offset = strings.len() as u32;
            name[4..].copy_from_slice(&offset.to_le_bytes());
            strings.extend_from_slice(symbol.name.as_bytes());
            strings.push(0);
        }
        records.extend_from_slice(&name);
        records.extend_from_slice(&symbol.value.to_le_bytes());
        records.extend_from_slice(&symbol.section.to_le_bytes());
        let kind: u16 = if symbol.function { 0x20 } else { 0 };
        records.extend_from_slice(&kind.to_le_bytes());
        records.push(if symbol.external { 2 } else { 3 });
        records.push(0); // no auxiliary records
    }
    let size = strings.len() as u32;
    strings[..4].copy_from_slice(&size.to_le_bytes());
    records.extend_from_slice(&strings);
    records
}

/// `.rdata`'s import table, and what `.data`'s address table must hold before the loader runs.
#[derive(Default)]
struct ImportTables {
    /// The section's bytes.
    bytes: Vec<u8>,
    /// The hint/name entry of each import, in the order `.data` reserved its slots - the same RVAs
    /// the lookup table holds, because the loader reads a zero slot as no import at all.
    lookup_rvas: Vec<u32>,
}

/// One section, as both the header writer and the byte writer see it: its name, its flags, where it
/// goes, and what is in it.
struct Section {
    name: &'static str,
    flags: u32,
    place: Where,
    content: Vec<u8>,
}

/// The file being written: headers, then the sections, each where `Sections` said.
struct Writer<'a> {
    program: &'a Program<'a>,
    sections: &'a Sections,
}

impl<'a> Writer<'a> {
    fn new(program: &'a Program<'a>, sections: &'a Sections) -> Self {
        Writer { program, sections }
    }

    /// The completed image.
    fn write(&self) -> Vec<u8> {
        let sections = self.sections();
        // The image ends where its last section's bytes do, rounded to a page - known only here,
        // which is why the header writer asks rather than storing it.
        let reloc_size = sections
            .iter()
            .find(|section| section.name == ".reloc")
            .map_or(0, |section| section.content.len() as u32);
        let symbols = self.symbols(&sections);
        let mut bytes = self.headers(&sections, reloc_size, &symbols);
        for section in &sections {
            bytes.resize(section.place.file as usize, 0);
            bytes.extend_from_slice(&section.content);
            // A section's header promises `SizeOfRawData` bytes *in the file*, which is its content
            // rounded up to a file block. A file that stops at the last byte of the last section's
            // content is one Windows refuses: `ERROR_BAD_EXE_FORMAT`, with nothing to say about why.
            let end = section.place.file as usize
                + align(section.content.len() as u32, FILE_ALIGNMENT) as usize;
            bytes.resize(end, 0);
        }
        // The symbol table goes where the sections' bytes end, which is the offset the header named -
        // so this assertion is the only thing keeping a pointer and a position from disagreeing.
        debug_assert_eq!(
            bytes.len() as u32,
            end_of_sections(&sections, self.sections.headers)
        );
        bytes.extend_from_slice(&symbol_table(&symbols));
        bytes
    }

    /// Every name the program has, and where it went. Derived, never stored: `Program` already holds
    /// each name against the place it was given, and a second list would be a second thing to keep
    /// right. Local names count - `sqlite3.c`'s functions are almost all `static`, so a table of the
    /// external ones alone would name almost nothing in the program this exists for.
    fn symbols(&self, list: &[Section]) -> Vec<Symbol> {
        let number = |name: &str| list.iter().position(|one| one.name == name);
        let (Some(text), Some(data)) = (number(".text"), number(".data")) else {
            return Vec::new();
        };
        let (text, data) = (text as u16 + 1, data as u16 + 1);
        let mut symbols = Vec::new();
        let add = |symbols: &mut Vec<Symbol>,
                   name: &str,
                   section: u16,
                   value: u32,
                   function,
                   external| {
            // A name the compiler made up for itself - `.str0` and the rest of the assembler's
            // dotted names - is bookkeeping, not a name the program has. Three thousand `.strN`
            // entries bury the functions this table is for.
            if name.starts_with('.') {
                return;
            }
            symbols.push(Symbol {
                name: name.to_owned(),
                section,
                value,
                function,
                external,
            });
        };
        for (name, at) in &self.program.functions {
            add(&mut symbols, name, text, *at, true, true);
        }
        for (name, at) in &self.program.objects {
            add(&mut symbols, name, data, *at, false, true);
        }
        for unit in &self.program.local {
            for (name, at) in &unit.functions {
                add(&mut symbols, name, text, *at, true, false);
            }
            for (name, at) in &unit.objects {
                add(&mut symbols, name, data, *at, false, false);
            }
        }
        // In address order, so `objdump -t` prints a list a reader can bisect and a debugger can
        // walk - and so a name's *end* is the next symbol's start, which is the only size a COFF
        // symbol has.
        symbols.sort_by(|one, two| {
            (one.section, one.value, &one.name).cmp(&(two.section, two.value, &two.name))
        });
        symbols
    }

    /// The sections this program has, in file order: **one list**, read by the header writer and by
    /// the byte writer alike. A section with no bytes is left out rather than written empty - a
    /// zero-sized `.reloc` is a header the Windows loader refuses to run.
    fn sections(&self) -> Vec<Section> {
        let tables = self.import_table();
        let rdata = tables.bytes.clone();
        let pdata = self.exception_table();
        let reloc = self.base_relocations();
        let candidates = [
            Section {
                name: ".text",
                flags: CODE_FLAGS,
                place: self.sections.text,
                content: self.program.text.clone(),
            },
            Section {
                name: ".data",
                flags: DATA_FLAGS,
                place: self.sections.data,
                content: self.data_with(&tables),
            },
            Section {
                name: ".rdata",
                flags: READ_ONLY_FLAGS,
                place: self.sections.rdata,
                content: rdata,
            },
            Section {
                name: ".pdata",
                flags: READ_ONLY_FLAGS,
                place: self.sections.pdata,
                content: pdata,
            },
            Section {
                name: ".xdata",
                flags: READ_ONLY_FLAGS,
                place: self.sections.xdata,
                content: self.program.unwind.xdata.clone(),
            },
            Section {
                name: ".reloc",
                flags: READ_ONLY_FLAGS,
                place: self.sections.reloc,
                content: reloc,
            },
        ];
        candidates
            .into_iter()
            .filter(|section| !section.content.is_empty())
            .collect()
    }

    /// The DOS stub, the PE signature, the COFF header, the optional header and the section table.
    fn headers(&self, list: &[Section], reloc_size: u32, symbols: &[Symbol]) -> Vec<u8> {
        let sections = self.sections;
        let code = align(self.program.text.len() as u32, FILE_ALIGNMENT);
        let initialised = list
            .iter()
            .filter(|section| section.flags != CODE_FLAGS)
            .map(|section| align(section.content.len() as u32, FILE_ALIGNMENT))
            .sum::<u32>();
        let pdata_size = list
            .iter()
            .find(|section| section.name == ".pdata")
            .map_or(0, |section| section.content.len() as u32);
        let rdata_size = list
            .iter()
            .find(|section| section.name == ".rdata")
            .map_or(0, |section| section.content.len() as u32);
        let image_size = align(self.sections.reloc.address + reloc_size, SECTION_ALIGNMENT);
        let _ = (rdata_size, pdata_size, reloc_size);
        let mut bytes = Vec::new();
        // A DOS program that does nothing but point at the PE header, which is all a loader reads.
        bytes.extend_from_slice(b"MZ");
        bytes.resize(0x3C, 0);
        bytes.extend_from_slice(&DOS_HEADER.to_le_bytes());
        bytes.resize(DOS_HEADER as usize, 0);
        bytes.extend_from_slice(b"PE\0\0");
        bytes.extend_from_slice(&0x8664u16.to_le_bytes()); // AMD64
        bytes.extend_from_slice(&(list.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes()); // no timestamp: the same input is the same file
                                                      // Where the symbol table is in the file and how many 18-byte records it has. An image with
                                                      // names in it is one a debugger can stop in and a crash address can be looked up against.
        bytes.extend_from_slice(&end_of_sections(list, sections.headers).to_le_bytes());
        bytes.extend_from_slice(&(symbols.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(OPTIONAL_HEADER as u16).to_le_bytes());
        bytes.extend_from_slice(&CHARACTERISTICS.to_le_bytes());
        // The optional header, PE32+.
        bytes.extend_from_slice(&0x20Bu16.to_le_bytes());
        bytes.push(14); // linker version: this compiler's milestone, in the traditional field
        bytes.push(0);
        bytes.extend_from_slice(&code.to_le_bytes()); // size of code
        bytes.extend_from_slice(&initialised.to_le_bytes()); // size of initialised data
        bytes.extend_from_slice(&0u32.to_le_bytes()); // size of uninitialised data
        bytes.extend_from_slice(&(sections.text.address + self.program.entry).to_le_bytes());
        bytes.extend_from_slice(&sections.text.address.to_le_bytes()); // base of code
        bytes.extend_from_slice(&IMAGE_BASE.to_le_bytes());
        bytes.extend_from_slice(&SECTION_ALIGNMENT.to_le_bytes());
        bytes.extend_from_slice(&FILE_ALIGNMENT.to_le_bytes());
        bytes.extend_from_slice(&6u16.to_le_bytes()); // major operating system version
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes()); // major image version
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&6u16.to_le_bytes()); // major subsystem version
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes()); // win32 version value
        bytes.extend_from_slice(&image_size.to_le_bytes());
        bytes.extend_from_slice(&sections.headers.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes()); // checksum
        bytes.extend_from_slice(&SUBSYSTEM_CONSOLE.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes()); // no DLL characteristics
        for size in [STACK_RESERVE, STACK_COMMIT, HEAP_RESERVE, HEAP_COMMIT] {
            bytes.extend_from_slice(&size.to_le_bytes());
        }
        bytes.extend_from_slice(&0u32.to_le_bytes()); // loader flags
        bytes.extend_from_slice(&16u32.to_le_bytes()); // data directories
                                                       // The directories: imports, the exception table and the relocations are the three this
                                                       // program has, and the other thirteen stay empty.
        let mut directories = [(0u32, 0u32); 16];
        // A directory with nothing in it is (`0`, `0`): a non-zero address with a zero size is a
        // header Windows reads and then thinks about.
        if !self.program.imports.is_empty() {
            let descriptors = (self.program.imports.len() as u32 + 1) * 20;
            directories[DIRECTORY_IMPORT] = (sections.rdata.address, descriptors);
        }
        if pdata_size > 0 {
            directories[DIRECTORY_EXCEPTION] = (sections.pdata.address, pdata_size);
        }
        if reloc_size > 0 {
            directories[DIRECTORY_BASERELOC] = (sections.reloc.address, reloc_size);
        }
        // `.data`'s address table is every import's slot and then *one* null, not one per library.
        let slots: u32 = self
            .program
            .imports
            .iter()
            .map(|library| library.functions.len() as u32)
            .sum::<u32>()
            + 1;
        if slots > 1 {
            let first = self
                .program
                .imports
                .first()
                .and_then(|library| library.functions.first())
                .map_or(0, |function| function.slot);
            directories[DIRECTORY_IAT] = (sections.data.address + first, slots * 8);
        }
        for (address, size) in directories {
            bytes.extend_from_slice(&address.to_le_bytes());
            bytes.extend_from_slice(&size.to_le_bytes());
        }
        // A section has two sizes and they are not the same number: the *virtual* size is what the
        // loader maps, and the raw size is that rounded up to a file block. Writing the padded size
        // in both made `.pdata` 512 bytes of table where there were 24 - a file the loader accepted
        // and refused to run.
        for section in list {
            let mut field = [0u8; 8];
            field[..section.name.len()].copy_from_slice(section.name.as_bytes());
            bytes.extend_from_slice(&field);
            bytes.extend_from_slice(&(section.content.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&section.place.address.to_le_bytes());
            bytes.extend_from_slice(
                &align(section.content.len() as u32, FILE_ALIGNMENT).to_le_bytes(),
            );
            bytes.extend_from_slice(&section.place.file.to_le_bytes());
            bytes.extend_from_slice(&0u32.to_le_bytes()); // no COFF relocations
            bytes.extend_from_slice(&0u32.to_le_bytes()); // no line numbers
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(&section.flags.to_le_bytes());
        }
        bytes
    }

    /// `.rdata`: one descriptor per library, then each library's lookup table, the hint/name entries,
    /// and the library names - in that order, because every address in a descriptor is computed here.
    ///
    /// It also says what the address table must hold *before* the loader runs: the same hint/name
    /// RVA as the lookup table. An address table of zeros is not a table the loader fills - it reads
    /// no imports at all and the first call through a thunk jumps to zero. Measured: filling these
    /// slots from the lookup table turned a fault at `rip = 0` into a program that exits 7.
    fn import_table(&self) -> ImportTables {
        let libraries = &self.program.imports;
        if libraries.is_empty() {
            return ImportTables::default();
        }
        let descriptors = (libraries.len() as u32 + 1) * 20;
        let mut lookup_at = Vec::new();
        let mut cursor = descriptors;
        for library in libraries {
            lookup_at.push(cursor);
            cursor += (library.functions.len() as u32 + 1) * 8;
        }
        let names_at = cursor;
        let mut names: Vec<u8> = Vec::new();
        let mut name_rvas: Vec<Vec<u32>> = Vec::new();
        for library in libraries {
            let mut rvas = Vec::new();
            for function in &library.functions {
                rvas.push(self.sections.rdata.address + names_at + names.len() as u32);
                names.extend_from_slice(&0u16.to_le_bytes()); // hint 0
                names.extend_from_slice(function.exported.as_bytes());
                names.push(0);
                // Pad to an even length, which is the whole of `name_entry_size`'s rounding.
                while names.len() % 2 == 1 {
                    names.push(0);
                }
            }
            name_rvas.push(rvas);
        }
        let libraries_at = names_at + names.len() as u32;
        let mut library_rvas = Vec::new();
        let mut library_names: Vec<u8> = Vec::new();
        for library in libraries {
            library_rvas
                .push(self.sections.rdata.address + libraries_at + library_names.len() as u32);
            library_names.extend_from_slice(library.name.as_bytes());
            library_names.push(0);
        }
        let mut table = Vec::new();
        let mut slot_at = 0u32;
        for (index, library) in libraries.iter().enumerate() {
            let first_thunk = self.sections.data.address
                + self.program.imports[index]
                    .functions
                    .first()
                    .map_or(0, |function| function.slot);
            let _ = slot_at;
            table
                .extend_from_slice(&(self.sections.rdata.address + lookup_at[index]).to_le_bytes());
            table.extend_from_slice(&0u32.to_le_bytes()); // timestamp
            table.extend_from_slice(&0u32.to_le_bytes()); // forwarder chain
            table.extend_from_slice(&library_rvas[index].to_le_bytes());
            table.extend_from_slice(&first_thunk.to_le_bytes());
            slot_at += library.functions.len() as u32;
        }
        table.extend_from_slice(&[0u8; 20]); // the null descriptor that ends the directory
        for (index, library) in libraries.iter().enumerate() {
            for rva in &name_rvas[index] {
                table.extend_from_slice(&rva.to_le_bytes());
                table.extend_from_slice(&0u32.to_le_bytes());
            }
            let _ = library;
            table.extend_from_slice(&[0u8; 8]); // the null that ends a lookup table
        }
        table.extend_from_slice(&names);
        table.extend_from_slice(&library_names);
        assert_eq!(
            table.len() as u32,
            import_table_size(libraries),
            "the import table's declared size and what was written disagree"
        );
        ImportTables {
            bytes: table,
            lookup_rvas: name_rvas.into_iter().flatten().collect(),
        }
    }

    /// `.data` with the address table filled in: the lookup table's RVAs, in the order the slots
    /// were reserved in.
    fn data_with(&self, tables: &ImportTables) -> Vec<u8> {
        let mut data = self.program.data.clone();
        let slots: Vec<u32> = self
            .program
            .imports
            .iter()
            .flat_map(|library| &library.functions)
            .map(|function| function.slot)
            .collect();
        assert_eq!(slots.len(), tables.lookup_rvas.len());
        for (slot, rva) in slots.into_iter().zip(&tables.lookup_rvas) {
            data[slot as usize..slot as usize + 8].copy_from_slice(&u64::from(*rva).to_le_bytes());
        }
        data
    }

    /// `.pdata`: one entry per described function, `begin`, `end` and the address of its description.
    fn exception_table(&self) -> Vec<u8> {
        let mut pdata = Vec::new();
        for entry in &self.program.unwind.entries {
            let begin = self
                .program
                .within(Some(entry.unit), &entry.name, self.sections)
                .expect("a described function is a function of the program");
            pdata.extend_from_slice(&begin.to_le_bytes());
            pdata.extend_from_slice(&(begin + entry.size).to_le_bytes());
            pdata.extend_from_slice(&(self.sections.xdata.address + entry.info).to_le_bytes());
        }
        pdata
    }

    /// `.reloc`: the addresses in `.data`, page by page.
    fn base_relocations(&self) -> Vec<u8> {
        let mut pages: BTreeMap<u32, Vec<u16>> = BTreeMap::new();
        for offset in &self.program.addr64 {
            let address = self.sections.data.address + offset;
            pages
                .entry(address & !0xFFF)
                .or_default()
                .push(((address & 0xFFF) as u16) | (REL_BASED_DIR64 << 12));
        }
        let mut block = Vec::new();
        for (page, entries) in pages {
            block.extend_from_slice(&page.to_le_bytes());
            let size = (8 + 2 * entries.len() as u32) as u16;
            block.extend_from_slice(&size.to_le_bytes());
            for entry in entries {
                block.extend_from_slice(&entry.to_le_bytes());
            }
            while block.len() % 4 != 0 {
                block.push(0); // each block starts where the last one ended, aligned to four
            }
        }
        block
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_start_on_a_section_boundary_and_files_on_a_file_one() {
        assert_eq!(align(0x1234, SECTION_ALIGNMENT), 0x2000);
        assert_eq!(align(0x1234, FILE_ALIGNMENT), 0x1400);
        assert_eq!(align(0x1000, SECTION_ALIGNMENT), 0x1000, "already aligned");
    }

    #[test]
    fn a_section_starts_after_the_bytes_of_the_one_before_it() {
        // Two sections of 100 and 200 bytes: the second starts at the page after the first's bytes,
        // not after its page - which is the mistake that put `.data` on top of `.text` while every
        // header still looked right.
        let first = place(0x400, 100);
        assert_eq!(first.address, 0x1000);
        assert_eq!(first.file, 0x1000);
        let second = place(first.address + 100, 200);
        assert_eq!(second.address, 0x2000);
        assert_eq!(second.file, 0x2000);
    }

    #[test]
    fn the_import_table_holds_a_descriptor_per_library_and_the_null_one() {
        let imports = [
            Imported {
                library: "ucrtbase.dll",
                name: "printf",
                exported: "printf",
            },
            Imported {
                library: "kernel32.dll",
                name: "ExitProcess",
                exported: "ExitProcess",
            },
            Imported {
                library: "ucrtbase.dll",
                name: "time",
                exported: "_time64",
            },
        ];
        let grouped = group(&imports);
        assert_eq!(grouped.len(), 2, "two libraries");
        assert_eq!(grouped[0].name, "ucrtbase.dll");
        assert_eq!(grouped[0].functions.len(), 2, "in the order given");
        assert_eq!(grouped[1].functions.len(), 1);
        let size = import_table_size(&grouped);
        // Three descriptors, a lookup table of three and one of two slots, and the names.
        let least = 3 * 20 + (3 * 8 + 2 * 8);
        assert!(size > least, "the names are counted too: {size}");
        assert_eq!(import_table_size(&[]), 0, "nothing imported, no table");
    }
}
