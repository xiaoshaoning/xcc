//! The object file: a `.text` section, a `.data` section when the program has objects in it, the
//! unwinder's `.pdata`/`.xdata` when any function has a prologue, the symbols those define, the ones
//! the file needs, and the relocations that tie them together.
//!
//! COFF's shape is fixed and old: a 20-byte file header, a 40-byte header per section, the section
//! bytes, a relocation per place that needs a symbol, then an 18-byte record per symbol with a
//! string table for names longer than eight bytes. What is written here is the subset `lld-link`
//! needs to link `main` and nothing else - which is the whole of this slice's claim.

use super::encode::{self, Relocation};
use super::regalloc;
use super::MachineFunction;

/// The machine type for x86-64.
const MACHINE_AMD64: u16 = 0x8664;
/// `IMAGE_SCN_CNT_CODE | IMAGE_SCN_MEM_EXECUTE | IMAGE_SCN_MEM_READ`.
const TEXT_FLAGS: u32 = 0x6000_0020;
/// `IMAGE_SCN_CNT_INITIALIZED_DATA | IMAGE_SCN_MEM_READ`: what a table of numbers nobody
/// writes is.
const DATA_READ_ONLY_FLAGS: u32 = 0x4000_0040;
/// `IMAGE_SCN_CNT_INITIALIZED_DATA | IMAGE_SCN_MEM_READ | IMAGE_SCN_MEM_WRITE`, aligned to sixteen
/// bytes. A string literal lives here as readily as a mutable counter, so the section is writable for
/// the second - and readable for both, which is the flag that is easy to leave out and that no test
/// would notice while a linker still mapped the section read-write.
const DATA_FLAGS: u32 = 0xC050_0040;
/// `IMAGE_SCN_CNT_INITIALIZED_DATA | IMAGE_SCN_MEM_READ`, four-byte aligned: the unwinder's two
/// tables, which are read and never executed.
const UNWIND_FLAGS: u32 = 0x4020_0040;
/// `IMAGE_SCN_LNK_INFO`'s absence is the default for code; this is the "4-byte relative" reloc.
const REL32: u16 = 0x0004;
/// `IMAGE_REL_AMD64_ADDR32NB`: a thirty-two-bit address relative to the image's start, which is what
/// the exception directory's entries hold.
const ADDR32NB: u16 = 0x0003;
/// `IMAGE_REL_AMD64_ADDR64`: a sixty-four-bit absolute address. A global's initialiser may hold the
/// address of another object - `char *p = "hello"` - and this is how the linker is told which one.
const ADDR64: u16 = 0x0001;
/// `IMAGE_SYM_CLASS_EXTERNAL`.
const EXTERNAL: u8 = 2;
/// `IMAGE_SYM_CLASS_STATIC`, which is what a section symbol is.
const STATIC: u8 = 3;
/// `IMAGE_SYM_DTYPE_FUNCTION`, in the top byte of the type field.
const FUNCTION: u16 = 0x20;

/// Everything a program keeps in memory that is not code: the bytes, the objects inside them, and
/// the names it only declares.
#[derive(Debug, Default)]
pub(crate) struct Data {
    /// The section's bytes, each object laid out at its own alignment.
    pub bytes: Vec<u8>,
    /// The strictest alignment any object in those bytes asks for. A linker concatenating units
    /// has to start this one there: `jmp_buf` is `_Alignas(16)`, and a unit placed at an 8-byte
    /// boundary made `__intrinsic_setjmp`'s `movdqa` fault on a program that had worked - adding
    /// eight bytes to the C runtime moved the program's data.
    pub align: u32,
    /// The objects in it.
    pub symbols: Vec<DataSymbol>,
    /// Names the file needs from somewhere else.
    pub undefined: Vec<String>,
    /// The places in those bytes that hold an address rather than a value.
    pub relocations: Vec<DataRelocation>,
}

/// One address inside the data section.
#[derive(Debug)]
pub(crate) struct DataRelocation {
    /// Where it goes, in bytes from the section's start.
    pub offset: u32,
    /// The symbol whose address it is.
    pub symbol: String,
    /// How many bytes into it: written into the field, because that is what an `IMAGE_REL_AMD64_
    /// ADDR64` means - the linker's answer is `S + A` and `A` is what the bytes already hold.
    pub addend: i64,
}

/// One object in the data section.
#[derive(Debug)]
pub(crate) struct DataSymbol {
    /// How many bytes it is, and how strictly it must start: a thread-local one is copied into a
    /// block per thread, and the block has to give each object the same alignment `.data` did.
    pub size: u32,
    pub align: u32,
    /// Whether it has thread storage duration: its address is not a link-time constant, and the
    /// program reaches it through the runtime's per-thread block instead of through `.data`.
    pub thread: bool,
    /// Its link name.
    pub name: String,
    /// Where it starts within the section.
    pub offset: u32,
    /// Whether the name is visible outside this translation unit.
    pub external: bool,
    /// Whether this is a *tentative* definition - `int shared;` at file scope, which C99 6.9.2p2
    /// says behaves as if it were one definition initialised to zero. Tentative definitions may
    /// appear in every unit of a program; a real one may appear in exactly one, which is what the
    /// linker's duplicate check reads this for.
    pub tentative: bool,
}

/// One function: where its code went, and which symbols it defines.
pub(crate) struct Placed {
    /// Its link name.
    pub name: String,
    /// Whether the name is visible outside this translation unit: what a `static` function is not.
    pub external: bool,
    /// Where it starts within `.text`.
    pub start: u32,
    /// The places in its code that name a symbol.
    pub relocations: Vec<Relocation>,
}

/// One function as the unwinder needs to find it: where its code is, and the bytes that describe what
/// its prologue did.
pub(crate) struct FunctionUnwind {
    /// Its link name, whose address `.pdata`'s begin and end are.
    pub name: String,
    /// Which unit it belongs to, because a `static` function's name is only unique in its own unit:
    /// a program linking two units that each define `static int helper(void)` has two of them.
    /// A single image's own copy of this is unit zero, which is the only unit it has.
    pub unit: usize,
    /// How long its code is.
    pub size: u32,
    /// The `UNWIND_INFO` that describes it.
    pub info: Vec<u8>,
}

/// What one translation unit comes to before it is written: the sections and the relocations that tie
/// them together. The object writer fills the relocations in with the linker's help; the loader fills
/// them in itself, which is why the two halves are separate.
pub(crate) struct Image {
    /// The code section.
    pub text: Vec<u8>,
    /// The functions in it.
    pub functions: Vec<Placed>,
    /// The data section, which the image owns so the loader can keep it.
    pub data: Data,
    /// Every function whose prologue did anything, in the order the functions were placed - which is
    /// ascending address, the order `.pdata` requires.
    pub unwind: Vec<FunctionUnwind>,
    /// Where each operation begins in `.text`, and the source it came from: the address-to-line
    /// table a debugger or a crash lookup wants, collected where the addresses are still being
    /// decided.
    pub lines: Vec<(u32, crate::diag::Span)>,
}

/// `.xdata` and `.pdata`, laid out once for both consumers: the object writer turns the entries into
/// relocations and the loader turns them into addresses. Two layouts would be two chances to disagree
/// with the unwinder.
pub(crate) struct UnwindLayout {
    /// Every function's `UNWIND_INFO`, four-byte aligned as the format asks.
    pub xdata: Vec<u8>,
    /// The entries, in ascending address.
    pub entries: Vec<UnwindEntry>,
}

/// One `.pdata` entry, before anything has said where the sections will be.
pub(crate) struct UnwindEntry {
    /// The function whose address `begin` and `end` are relative to.
    pub name: String,
    /// The unit that function belongs to, which is what makes its name resolvable.
    pub unit: usize,
    /// Its length: `end` is its address plus this.
    pub size: u32,
    /// Where its `UNWIND_INFO` starts in `.xdata`.
    pub info: u32,
}

/// Lay out the unwinder's data.
pub(crate) fn unwind_layout(unwind: &[FunctionUnwind]) -> UnwindLayout {
    let mut xdata = Vec::new();
    let mut entries = Vec::new();
    for function in unwind {
        while xdata.len() % 4 != 0 {
            xdata.push(0);
        }
        entries.push(UnwindEntry {
            name: function.name.clone(),
            unit: function.unit,
            size: function.size,
            info: xdata.len() as u32,
        });
        xdata.extend_from_slice(&function.info);
    }
    UnwindLayout { xdata, entries }
}

/// Encode every function into one image.
pub(crate) fn assemble(functions: &[MachineFunction], data: Data) -> Image {
    let mut text = Vec::new();
    let mut placed = Vec::new();
    let mut unwind = Vec::new();
    let mut lines = Vec::new();
    for function in functions {
        let allocation = regalloc::allocate(function);
        let encoded = encode::function(function, &allocation);
        placed.push(Placed {
            name: function.name.clone(),
            external: function.external,
            start: text.len() as u32,
            relocations: encoded.relocations,
        });
        if let Some(described) = &encoded.unwind {
            unwind.push(FunctionUnwind {
                name: function.name.clone(),
                unit: 0,
                size: encoded.code.len() as u32,
                info: unwind_info(described),
            });
        }
        // The encoder's offsets are the function's; the object's are the section's.
        let start = text.len() as u32;
        lines.extend(
            encoded
                .lines
                .iter()
                .map(|(offset, span)| (start + offset, *span)),
        );
        text.extend_from_slice(&encoded.code);
    }
    Image {
        text,
        functions: placed,
        lines,
        data,
        unwind,
    }
}

/// The `UNWIND_INFO` for one prologue: PE/COFF's four-byte header, then the operations as two-byte
/// slots in *decreasing* offset order, each one's extra data right behind it.
fn unwind_info(unwind: &encode::Unwind) -> Vec<u8> {
    debug_assert!(
        unwind.codes.iter().all(|code| code.at <= unwind.prologue),
        "an operation past the prologue it belongs to"
    );
    let mut slots: Vec<u16> = Vec::new();
    for code in unwind.codes.iter().rev() {
        let word =
            u16::from(code.at) | (u16::from(code.operation) << 8) | (u16::from(code.info) << 12);
        slots.push(word);
        if let Some(extra) = code.extra {
            slots.push(extra as u16);
            // `ALLOC_LARGE` with `op_info` 1 names a size in bytes, which takes two slots.
            if code.operation == encode::UWOP_ALLOC_LARGE && code.info == 1 {
                slots.push((extra >> 16) as u16);
            }
        }
    }
    // The count field counts two-byte slots and has to be even, because the array is read as words.
    if slots.len() % 2 == 1 {
        slots.push(0);
    }
    let mut bytes = vec![1, unwind.prologue, slots.len() as u8, 0];
    for slot in slots {
        bytes.extend_from_slice(&slot.to_le_bytes());
    }
    bytes
}

/// Write an image that has already been assembled - which is what a linker holds before it has a
/// file to put it in.
pub(crate) fn write_image(image: &Image) -> Vec<u8> {
    write_object(image)
}

/// `value` in base 128, low group first, the high bit set on every group but the last: one byte for
/// anything under 128, which is what a delta between two operations is.
fn varint(out: &mut Vec<u8>, value: u32) {
    let mut left = value;
    loop {
        let group = (left & 0x7f) as u8;
        left >>= 7;
        if left == 0 {
            out.push(group);
            return;
        }
        out.push(group | 0x80);
    }
}

/// A signed number as an unsigned one that zigzags - `0, -1, 1, -2` become `0, 1, 2, 3` - so a small
/// negative delta is as short as a small positive one. A loop's back edge is the case.
fn zigzag(value: i32) -> u32 {
    ((value << 1) ^ (value >> 31)) as u32
}

/// One section of the object: its bytes, the names inside it, and the places that need the linker.
struct SectionOut {
    name: &'static str,
    bytes: Vec<u8>,
    flags: u32,
    /// Symbols defined *in* this section: name, offset, storage class.
    symbols: Vec<(String, u32, u8)>,
    /// Relocations of its bytes: offset, symbol, kind.
    relocations: Vec<(u32, String, u16)>,
}

/// Write an image out as a COFF object.
///
/// Every section is described the same way and written by the same loop, which is what adding
/// `.pdata` and `.xdata` needed: two hard-coded sections would have been two more copies of the same
/// fourteen lines, and four chances for one of them to be wrong.
fn write_object(image: &Image) -> Vec<u8> {
    let data = &image.data;
    let has_data = !data.bytes.is_empty() || !data.symbols.is_empty();
    let layout = unwind_layout(&image.unwind);
    let mut sections = vec![SectionOut {
        name: ".text",
        flags: TEXT_FLAGS,
        bytes: image.text.clone(),
        symbols: image
            .functions
            .iter()
            .map(|function| {
                let class = if function.external { EXTERNAL } else { STATIC };
                (function.name.clone(), function.start, class)
            })
            .collect(),
        relocations: image
            .functions
            .iter()
            .flat_map(|function| {
                function
                    .relocations
                    .iter()
                    .map(|relocation| {
                        (
                            function.start + relocation.offset,
                            relocation.symbol.clone(),
                            REL32,
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .collect(),
    }];
    // `.xccmap`: where each operation begins, and the source byte range it came from - one record
    // of twelve bytes: the offset in `.text`, the span's offset, and its length. Not a debug
    // section any tool reads yet, and the smallest thing a crash address can be looked up against.
    // The line *number* is the CLI's to compute, because a span is an offset and only the source
    // file knows where its lines begin.
    if !image.lines.is_empty() {
        // **Twelve bytes a record is too many**, and this is in every object: measured on `tre`,
        // one record per operation was 207 148 bytes against 177 436 of code, and one per *line*
        // still 122 592. So the records are deltas - each relative to the one before it, as
        // `uleb128` varints, one byte for a delta under 128:
        //
        //     offset delta (>= 0)   span.start delta (zigzag: a loop goes backwards)
        //     file delta (zigzag: a macro body and its use are two files)              length
        //
        // A reader walks them in order and accumulates; a lookup takes the last record whose
        // accumulated offset is at or below the address.
        let mut bytes = Vec::with_capacity(image.lines.len() * 4);
        let mut previous: Option<(u32, u32)> = None;
        let (mut at, mut start) = (0u32, 0i32);
        let mut file = 0i32;
        for (offset, span) in &image.lines {
            // A span of no length names no line: `Span::at(0)` is what an instruction the front end
            // made up carries, and a record that says "somewhere, nowhere" is worse than no record.
            if span.len == 0 {
                continue;
            }
            let line = (span.start, span.len);
            if previous == Some(line) {
                continue;
            }
            previous = Some(line);
            varint(&mut bytes, offset - at);
            varint(&mut bytes, zigzag(span.start as i32 - start));
            varint(&mut bytes, zigzag(i32::from(span.file) - file));
            varint(&mut bytes, span.len);
            at = *offset;
            start = span.start as i32;
            file = i32::from(span.file);
        }
        sections.push(SectionOut {
            name: ".xccmap",
            flags: DATA_READ_ONLY_FLAGS,
            bytes,
            symbols: Vec::new(),
            relocations: Vec::new(),
        });
    }
    if has_data {
        // Every relocation's addend is written into the field it belongs to, because that is what an
        // `ADDR64` *means*: the linker's answer is `S + A` and `A` is what the bytes already hold.
        // The struct carries it as well, for this compiler's own linker; leaving it out of the
        // object made every address written into an object file that of the symbol's *start*.
        let mut bytes = data.bytes.clone();
        for relocation in &data.relocations {
            let at = relocation.offset as usize;
            if at + 8 <= bytes.len() {
                bytes[at..at + 8].copy_from_slice(&(relocation.addend as u64).to_le_bytes());
            }
        }
        sections.push(SectionOut {
            name: ".data",
            flags: DATA_FLAGS,
            bytes,
            symbols: data
                .symbols
                .iter()
                .map(|object| {
                    (
                        object.name.clone(),
                        object.offset,
                        if object.external { EXTERNAL } else { STATIC },
                    )
                })
                .collect(),
            relocations: data
                .relocations
                .iter()
                .map(|relocation| (relocation.offset, relocation.symbol.clone(), ADDR64))
                .collect(),
        });
    }
    if !layout.entries.is_empty() {
        // `.xdata` first, so that `.pdata`'s entries can name it: each `UNWIND_INFO` gets a symbol of
        // its own. No function has a language-specific handler, so no chain field is needed either.
        sections.push(SectionOut {
            name: ".xdata",
            flags: UNWIND_FLAGS,
            bytes: layout.xdata.clone(),
            symbols: layout
                .entries
                .iter()
                .enumerate()
                .map(|(index, entry)| (format!(".unwind{index}"), entry.info, STATIC))
                .collect(),
            relocations: Vec::new(),
        });
        // Three words per function, each an image-relative address: where the code starts, where it
        // ends, and the `UNWIND_INFO` that describes it. The end is an *addend* - the function's
        // address plus its length - which is why the length goes in as the field's contents.
        let mut pdata = Vec::new();
        for entry in &layout.entries {
            push_u32(&mut pdata, 0);
            push_u32(&mut pdata, entry.size);
            push_u32(&mut pdata, 0);
        }
        sections.push(SectionOut {
            name: ".pdata",
            flags: UNWIND_FLAGS,
            bytes: pdata,
            symbols: Vec::new(),
            relocations: layout
                .entries
                .iter()
                .enumerate()
                .flat_map(|(index, entry)| {
                    let at = (index * 12) as u32;
                    vec![
                        (at, entry.name.clone(), ADDR32NB),
                        (at + 4, entry.name.clone(), ADDR32NB),
                        (at + 8, format!(".unwind{index}"), ADDR32NB),
                    ]
                })
                .collect(),
        });
    }

    // Every symbol the file has, in the order the relocation indices are numbered by: one per
    // section, then the names inside the sections, then the ones the file only needs.
    let mut symbols: Vec<(String, u32, u16, u8)> = Vec::new();
    for (index, section) in sections.iter().enumerate() {
        symbols.push((section.name.to_string(), 0, index as u16 + 1, STATIC));
    }
    for (index, section) in sections.iter().enumerate() {
        for (name, offset, class) in &section.symbols {
            symbols.push((name.clone(), *offset, index as u16 + 1, *class));
        }
    }
    let mut externals: Vec<String> = data.undefined.clone();
    for section in &sections {
        for (_, symbol, _) in &section.relocations {
            let defined = symbols.iter().any(|(name, ..)| name == symbol);
            if !defined && !externals.contains(symbol) {
                externals.push(symbol.clone());
            }
        }
    }
    for external in &externals {
        symbols.push((external.clone(), 0, 0, EXTERNAL));
    }

    let index_of = |name: &str| {
        symbols
            .iter()
            .position(|(candidate, ..)| candidate == name)
            .expect("every relocation names a symbol") as u32
    };

    // Each section's bytes are four-byte aligned in the file, and its relocations follow them.
    let header_size = 20 + 40 * sections.len();
    let mut at = header_size;
    let mut placed = Vec::new();
    for section in &sections {
        let padded = section.bytes.len().div_ceil(4) * 4;
        let bytes_at = at;
        let relocations_at = bytes_at + padded;
        placed.push((bytes_at, relocations_at, padded));
        at = relocations_at + section.relocations.len() * 10;
    }
    let symbol_table_at = at;

    let mut bytes = Vec::new();
    push_u16(&mut bytes, MACHINE_AMD64);
    push_u16(&mut bytes, sections.len() as u16);
    push_u32(&mut bytes, 0); // no timestamp, so the same source writes the same bytes
    push_u32(&mut bytes, symbol_table_at as u32);
    push_u32(&mut bytes, symbols.len() as u32);
    push_u16(&mut bytes, 0); // no optional header
    push_u16(&mut bytes, 0); // no flags

    for (section, (bytes_at, relocations_at, padded)) in sections.iter().zip(&placed) {
        let mut name = [0u8; 8];
        name[..section.name.len()].copy_from_slice(section.name.as_bytes());
        bytes.extend_from_slice(&name);
        push_u32(&mut bytes, 0); // physical address / virtual size
        push_u32(&mut bytes, 0); // virtual address
        push_u32(&mut bytes, *padded as u32);
        push_u32(&mut bytes, *bytes_at as u32);
        push_u32(&mut bytes, *relocations_at as u32);
        push_u32(&mut bytes, 0); // no line numbers
        push_u16(&mut bytes, section.relocations.len() as u16);
        push_u16(&mut bytes, 0);
        push_u32(&mut bytes, section.flags);
    }

    for (section, (bytes_at, relocations_at, padded)) in sections.iter().zip(&placed) {
        bytes.extend_from_slice(&section.bytes);
        bytes.resize(bytes_at + padded, 0);
        for (offset, symbol, kind) in &section.relocations {
            push_u32(&mut bytes, *offset);
            push_u32(&mut bytes, index_of(symbol));
            push_u16(&mut bytes, *kind);
        }
        bytes.resize(relocations_at + section.relocations.len() * 10, 0);
    }
    bytes.resize(symbol_table_at, 0);

    // The symbol table, then the string table: names of eight bytes or fewer are stored in place,
    // longer ones as an offset into the string table, whose first four bytes are its own length.
    let mut strings: Vec<u8> = vec![0, 0, 0, 0];
    for (name, value, section, class) in &symbols {
        let mut field = [0u8; 8];
        if name.len() <= 8 {
            field[..name.len()].copy_from_slice(name.as_bytes());
            bytes.extend_from_slice(&field);
        } else {
            let offset = strings.len() as u32;
            push_u32(&mut bytes, 0);
            push_u32(&mut bytes, offset);
            strings.extend_from_slice(name.as_bytes());
            strings.push(0);
        }
        push_u32(&mut bytes, *value);
        push_u16(&mut bytes, *section);
        // A function's type field says so; the rest is data, including the unwinder's tables.
        let ty = if *class == EXTERNAL && *section == 1 {
            FUNCTION
        } else {
            0
        };
        push_u16(&mut bytes, ty);
        bytes.push(*class);
        bytes.push(0); // no auxiliary records
    }
    let length = strings.len() as u32;
    strings[..4].copy_from_slice(&length.to_le_bytes());
    bytes.extend_from_slice(&strings);
    bytes
}

fn push_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(text: Vec<u8>) -> Image {
        Image {
            text,
            functions: Vec::new(),
            data: Data::default(),
            unwind: Vec::new(),
            lines: Vec::new(),
        }
    }

    #[test]
    fn a_data_relocation_puts_its_addend_in_the_field() {
        // `ADDR64` means `S + A`, and `A` is what the bytes already hold. An object that wrote zero
        // there would resolve every `&table[4]` to the start of `table` for any linker but this
        // compiler's own - which is what happened until this was fixed, and `clang` linking that
        // object printed `0 a 0 000` where its own build prints `4 a 10 def`.
        let mut data = Data {
            bytes: vec![0u8; 16],
            ..Data::default()
        };
        data.symbols.push(DataSymbol {
            size: 16,
            align: 1,
            thread: false,
            name: "table".into(),
            offset: 0,
            external: true,
            tentative: false,
        });
        data.relocations.push(DataRelocation {
            offset: 8,
            symbol: "table".into(),
            addend: 4,
        });
        let object = write_object(&Image {
            text: vec![0x90],
            functions: Vec::new(),
            data,
            unwind: Vec::new(),
            lines: Vec::new(),
        });
        // The `.data` section's bytes are the second section's, and its contents start after the
        // headers; the field at offset 8 holds the addend, little-endian.
        let field = [4u8, 0, 0, 0, 0, 0, 0, 0];
        assert!(
            object.windows(8).any(|window| window == field),
            "the addend is in the field the relocation names"
        );
    }

    #[test]
    fn an_empty_object_is_a_header_and_an_empty_section() {
        let object = write_object(&image(vec![0x90]));
        // The machine type, and one section.
        assert_eq!(&object[..4], &[0x64, 0x86, 1, 0]);
        // Header, section header, four bytes of text, one 18-byte symbol (`.text`), and a string
        // table holding nothing but its own length.
        assert_eq!(object.len(), 20 + 40 + 4 + 18 + 4);
        // The section's name field is ".text" in eight bytes.
        assert_eq!(&object[20..28], b".text\0\0\0");
        // One byte of text, padded to four.
        assert_eq!(object[60], 0x90);
    }

    #[test]
    fn two_pushes_and_a_frame_are_described_in_decreasing_order() {
        // The shape every function with a saved register has: `push rbx` (one byte), `push r12` (two,
        // for the REX prefix), `sub rsp, 0x28` (four: `83 EC 28`... with a REX prefix, five).
        let unwind = encode::Unwind {
            prologue: 8,
            codes: vec![
                encode::UnwindCode {
                    at: 1,
                    operation: encode::UWOP_PUSH_NONVOL,
                    info: 3,
                    extra: None,
                },
                encode::UnwindCode {
                    at: 3,
                    operation: encode::UWOP_PUSH_NONVOL,
                    info: 12,
                    extra: None,
                },
                encode::UnwindCode {
                    at: 8,
                    operation: encode::UWOP_ALLOC_SMALL,
                    info: 4,
                    extra: None,
                },
            ],
        };
        let info = unwind_info(&unwind);
        // Version 1, the prologue's length, four slots for three operations (one word each, plus the
        // padding word that makes the count even), no frame register.
        assert_eq!(&info[..4], &[1, 8, 4, 0]);
        // The operations come back to front: the subtraction first, then `push r12`, then `push rbx`.
        // `at` 8, `ALLOC_SMALL`, `op_info` 4: a 40-byte frame names itself as five eight-byte units.
        assert_eq!(u16::from_le_bytes([info[4], info[5]]), 0x4208);
        assert_eq!(u16::from_le_bytes([info[6], info[7]]), 0xC003);
        assert_eq!(u16::from_le_bytes([info[8], info[9]]), 0x3001);
        assert_eq!(u16::from_le_bytes([info[10], info[11]]), 0);
    }

    #[test]
    fn the_layout_puts_each_functions_description_four_byte_aligned() {
        let layout = unwind_layout(&[
            FunctionUnwind {
                name: "a".into(),
                unit: 0,
                size: 3,
                info: vec![1, 2, 3],
            },
            FunctionUnwind {
                name: "b".into(),
                unit: 1,
                size: 5,
                info: vec![4, 5, 6],
            },
        ]);
        // Three bytes, padded to four, then the next.
        assert_eq!(layout.xdata, vec![1, 2, 3, 0, 4, 5, 6]);
        assert_eq!(layout.entries[0].info, 0);
        assert_eq!(layout.entries[1].info, 4);
        assert_eq!(layout.entries[1].size, 5);
    }
}
