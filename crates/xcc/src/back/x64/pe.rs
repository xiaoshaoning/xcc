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
/// A megabyte of stack and heap to begin with, which is what a console program asks for.
const STACK_RESERVE: u64 = 0x10_0000;
const STACK_COMMIT: u64 = 0x1000;
const HEAP_RESERVE: u64 = 0x10_0000;
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
    /// The function's name, as the library exports it.
    pub name: &'a str,
}

/// A program that has been linked.
pub(crate) struct Linked {
    /// The image: headers, sections, import table, base relocations.
    pub bytes: Vec<u8>,
}

/// A name nothing defined and no library offered.
pub(crate) struct Unlinked {
    /// Which name.
    pub name: String,
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
) -> Result<Linked, Unlinked> {
    let mut program = Program::gather(units, imports);
    program.entry = *program.functions.get(entry).ok_or_else(|| Unlinked {
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
}

/// One library's imports, in the order the address table holds them.
struct Library<'a> {
    name: &'a str,
    /// Each function, with its slot in `.data` and its thunk in `.text`.
    functions: Vec<Slot>,
}

/// One imported function's two homes.
struct Slot {
    name: String,
    /// The eight-byte address-table slot, from `.data`'s start.
    slot: u32,
    /// The six-byte `jmp [rip+slot]`, from `.text`'s start.
    thunk: u32,
}

impl<'a> Program<'a> {
    /// Concatenate the units, and give each import its slot and its thunk.
    fn gather(units: &'a [Image], imports: &[Imported<'a>]) -> Self {
        let mut program = Program {
            units,
            functions: BTreeMap::new(),
            objects: BTreeMap::new(),
            unit_text: Vec::new(),
            unit_data: Vec::new(),
            imports: group(imports),
            text: Vec::new(),
            data: Vec::new(),
            unwind: coff::unwind_layout(&[]),
            addr64: Vec::new(),
            entry: 0,
        };
        for unit in units {
            let base = program.text.len() as u32;
            // A unit's functions are placed relative to its own code, and its relocations are
            // written there too - so where its code landed is a fact the patch needs.
            program.unit_text.push(base);
            for function in &unit.functions {
                // A name two units define is a duplicate symbol; the first wins and a `-c` build's
                // object writer is where the duplicate is reported.
                program
                    .functions
                    .entry(function.name.clone())
                    .or_insert(base + function.start);
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
            let base = program.data.len() as u32;
            program.unit_data.push(base);
            for object in &unit.data.symbols {
                program
                    .objects
                    .entry(object.name.clone())
                    .or_insert(base + object.offset);
            }
            program.data.extend_from_slice(&unit.data.bytes);
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
            .flat_map(|unit| {
                unit.unwind.iter().map(|function| coff::FunctionUnwind {
                    name: function.name.clone(),
                    size: function.size,
                    info: function.info.clone(),
                })
            })
            .collect();
        program.unwind = coff::unwind_layout(&described);
        program
    }

    /// The offset of a name from the start of the image: `.text` for a function, `.data` for an
    /// object, a thunk for anything imported.
    fn address(&self, name: &str, sections: &Sections) -> Option<u32> {
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
    fn write(&mut self) -> Result<Vec<u8>, Unlinked> {
        let sizes = Sizes::of(self);
        let sections = Sections::new(sizes);
        self.resolve(&sections)?;
        Ok(Writer::new(self, &sections).write())
    }

    /// Fill every relocation in, now that the addresses are known.
    fn resolve(&mut self, sections: &Sections) -> Result<(), Unlinked> {
        for (index, unit) in self.units.iter().enumerate() {
            for function in &unit.functions {
                for relocation in &function.relocations {
                    let target =
                        self.address(&relocation.symbol, sections)
                            .ok_or_else(|| Unlinked {
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
                let target =
                    self.address(&relocation.symbol, sections)
                        .ok_or_else(|| Unlinked {
                            name: relocation.symbol.clone(),
                        })?;
                let at = (self.unit_data[index] + relocation.offset) as usize;
                let absolute = IMAGE_BASE + u64::from(target);
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
        .map(|function| name_entry_size(&function.name))
        .sum();
    let libraries_named: u32 = libraries
        .iter()
        .map(|library| library.name.len() as u32 + 1)
        .sum();
    // No slack: the assertion in the writer checks this against what is actually written, and a
    // size with two bytes of room in it is a size nobody is checking.
    descriptors + lookups + names + libraries_named
}

/// `.rdata`'s import table, and the address table's contents that go with it.
#[derive(Default)]
struct ImportTables {
    /// The section's bytes.
    bytes: Vec<u8>,
    /// One hint/name RVA per import, in the order `.data` reserved its slots.
    address_table: Vec<u64>,
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
        let mut bytes = self.headers(&sections, reloc_size);
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
        bytes
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
    fn headers(&self, list: &[Section], reloc_size: u32) -> Vec<u8> {
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
        bytes.extend_from_slice(&0u32.to_le_bytes()); // no symbol table
        bytes.extend_from_slice(&0u32.to_le_bytes()); // no symbols
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
                names.extend_from_slice(function.name.as_bytes());
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
            address_table: name_rvas.into_iter().flatten().map(u64::from).collect(),
        }
    }

    /// `.data` with the address table filled in: the import table's hint/name RVAs, in the order the
    /// slots were reserved in.
    fn data_with(&self, tables: &ImportTables) -> Vec<u8> {
        let mut data = self.program.data.clone();
        let slots: Vec<u32> = self
            .program
            .imports
            .iter()
            .flat_map(|library| &library.functions)
            .map(|function| function.slot)
            .collect();
        assert_eq!(slots.len(), tables.address_table.len());
        for (slot, value) in slots.into_iter().zip(&tables.address_table) {
            data[slot as usize..slot as usize + 8].copy_from_slice(&value.to_le_bytes());
        }
        data
    }

    /// `.pdata`: one entry per described function, `begin`, `end` and the address of its description.
    fn exception_table(&self) -> Vec<u8> {
        let mut pdata = Vec::new();
        for entry in &self.program.unwind.entries {
            let begin = *self
                .program
                .functions
                .get(&entry.name)
                .expect("a described function is a function of the program")
                + self.sections.text.address;
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
            },
            Imported {
                library: "kernel32.dll",
                name: "ExitProcess",
            },
            Imported {
                library: "ucrtbase.dll",
                name: "exit",
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
