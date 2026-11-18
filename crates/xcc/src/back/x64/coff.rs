//! The object file: one `.text` section, the function symbols it defines, the ones it needs, and
//! the relocations a call leaves behind.
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
/// `IMAGE_SCN_LNK_INFO`'s absence is the default for code; this is the "4-byte relative" reloc.
const REL32: u16 = 0x0004;
/// `IMAGE_SYM_CLASS_EXTERNAL`.
const EXTERNAL: u8 = 2;
/// `IMAGE_SYM_CLASS_STATIC`, which is what a section symbol is.
const STATIC: u8 = 3;
/// `IMAGE_SYM_DTYPE_FUNCTION`, in the top byte of the type field.
const FUNCTION: u16 = 0x20;

/// One function: where its code went, and which symbols it defines.
struct Placed {
    name: String,
    start: u32,
    code_len: u32,
    relocations: Vec<Relocation>,
}

/// Encode every function, then write the object that holds them.
pub(crate) fn write(functions: &[MachineFunction]) -> Vec<u8> {
    let mut text = Vec::new();
    let mut placed = Vec::new();
    for function in functions {
        let allocation = regalloc::allocate(function);
        let encoded = encode::function(function, &allocation);
        placed.push(Placed {
            name: function.name.clone(),
            start: text.len() as u32,
            code_len: encoded.code.len() as u32,
            relocations: encoded.relocations,
        });
        text.extend_from_slice(&encoded.code);
    }
    write_object(&text, &placed)
}

fn write_object(text: &[u8], placed: &[Placed]) -> Vec<u8> {
    // Symbols: the section, then every function, then every name a relocation wants that no
    // function here defines.
    let mut symbols: Vec<(String, u32, u16, u8)> = vec![(".text".into(), 0, 1, STATIC)];
    for function in placed {
        symbols.push((function.name.clone(), function.start, 1, EXTERNAL));
    }
    let mut externals: Vec<String> = Vec::new();
    for function in placed {
        for relocation in &function.relocations {
            let defined = placed.iter().any(|other| other.name == relocation.symbol);
            if !defined && !externals.contains(&relocation.symbol) {
                externals.push(relocation.symbol.clone());
            }
        }
    }
    for external in &externals {
        symbols.push((external.clone(), 0, 0, EXTERNAL));
    }

    // A relocation names a symbol by index, so the index of each is needed before the text can be
    // described. Function symbols start at 1, after the section symbol.
    let mut relocations = Vec::new();
    for function in placed {
        for relocation in &function.relocations {
            let index = symbols
                .iter()
                .position(|(name, ..)| *name == relocation.symbol)
                .expect("every relocation names a symbol") as u32;
            relocations.push((function.start + relocation.offset, index, REL32));
        }
    }

    let text_size = text.len();
    let text_padded = text_size.div_ceil(4) * 4;
    let mut bytes = Vec::new();
    let header_size = 20 + 40; // file header, one section header
    let text_at = header_size;
    let relocations_at = text_at + text_padded;
    let symbol_table_at = relocations_at + relocations.len() * 10;

    // The file header.
    push_u16(&mut bytes, MACHINE_AMD64);
    push_u16(&mut bytes, 1); // one section
    push_u32(&mut bytes, 0); // no timestamp, so the same source writes the same bytes
    push_u32(&mut bytes, symbol_table_at as u32);
    push_u32(&mut bytes, symbols.len() as u32);
    push_u16(&mut bytes, 0); // no optional header
    push_u16(&mut bytes, 0); // no flags

    // The `.text` section header.
    let mut name = [0u8; 8];
    name[..5].copy_from_slice(b".text");
    bytes.extend_from_slice(&name);
    push_u32(&mut bytes, 0); // physical address / virtual size
    push_u32(&mut bytes, 0); // virtual address
    push_u32(&mut bytes, text_padded as u32);
    push_u32(&mut bytes, text_at as u32);
    push_u32(&mut bytes, relocations_at as u32);
    push_u32(&mut bytes, 0); // no line numbers
    push_u16(&mut bytes, relocations.len() as u16);
    push_u16(&mut bytes, 0);
    push_u32(&mut bytes, TEXT_FLAGS);

    bytes.extend_from_slice(text);
    bytes.resize(text_at + text_padded, 0);

    for (offset, symbol, kind) in &relocations {
        push_u32(&mut bytes, *offset);
        push_u32(&mut bytes, *symbol);
        push_u16(&mut bytes, *kind);
    }

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
        // `.text` is data, a function is a function: the type field's top byte says which.
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

    #[test]
    fn an_empty_object_is_a_header_and_an_empty_section() {
        let object = write_object(&[0x90], &[]);
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
}
