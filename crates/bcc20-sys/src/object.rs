//! What the two compilers wrote, read back: the code size in each one's object format.
//!
//! xcc writes COFF (`crates/xcc/src/back`), Borland writes OMF (the 16-bit record format described
//! in the Open Architecture Handbook). The comparison needs one number from each - the bytes of
//! code - and both formats put it somewhere different, so the two readers live here, side by side,
//! where the difference between them is visible.
//!
//! Neither reader is a general object-file library: each answers the one question the measuring
//! stick asks, and refuses (with `None`) rather than guessing when the file is not what it expects.

/// The bytes in an x86-64 COFF object's `.text` section.
///
/// The header is 20 bytes, then the optional header (`SizeOfOptionalHeader`), then one 40-byte
/// section header each: an 8-byte name and, at offset 16, `SizeOfRawData`.
pub fn coff_text(object: &[u8]) -> Option<usize> {
    let count = u16::from_le_bytes([*object.get(2)?, *object.get(3)?]) as usize;
    let optional = u16::from_le_bytes([*object.get(16)?, *object.get(17)?]) as usize;
    let mut at = 20 + optional;
    for _ in 0..count {
        let header = object.get(at..at + 40)?;
        let name = &header[..8];
        let name = name
            .iter()
            .take_while(|byte| **byte != 0)
            .copied()
            .collect::<Vec<u8>>();
        if name == b".text" {
            return Some(
                u32::from_le_bytes([header[16], header[17], header[18], header[19]]) as usize,
            );
        }
        at += 40;
    }
    None
}

/// The bytes in a 16-bit Borland OMF object's code segment.
///
/// Records are `[type][length: u16][data]`, ending with `MODEND` (0x8B). The names Borland gives
/// its segments come from `LNAMES` (0x96); a `SEGDEF` (0x98, or 0x99 for 32-bit) names one, and
/// `LEDATA` (0xA0) holds the bytes a segment holds. So: read the names, find the segment whose name
/// ends in `TEXT` (Borland's `_TEXT`), and add up its `LEDATA` payloads.
pub fn omf_text(object: &[u8]) -> Option<usize> {
    let mut names: Vec<String> = vec![String::new()]; // OMF indices count from one
                                                      // segment index -> its name, and the bytes seen per segment index
    let mut segments: Vec<Option<String>> = vec![None];
    let mut sizes: Vec<usize> = vec![0];
    let mut at = 0usize;
    while at + 3 <= object.len() {
        let kind = object[at];
        let length = u16::from_le_bytes([object[at + 1], object[at + 2]]) as usize;
        let data = object.get(at + 3..at + 3 + length)?;
        at += 3 + length;
        match kind {
            0x96 => {
                // LNAMES: one length-prefixed name after another, then a checksum byte (every
                // record here ends with one that makes the record sum to zero). A length that
                // would read past the record is that checksum, not a truncated name.
                let mut name_at = 0;
                while name_at < data.len() {
                    let size = data[name_at] as usize;
                    let Some(text) = data.get(name_at + 1..name_at + 1 + size) else {
                        break;
                    };
                    names.push(String::from_utf8_lossy(text).into_owned());
                    name_at += 1 + size;
                }
            }
            0x98 | 0x99 => {
                // SEGDEF: [attributes][length: u16][name index]...
                let index = *data.get(3)? as usize;
                let name = names.get(index).cloned();
                segments.push(name);
                sizes.push(0);
            }
            0xA0 => {
                // LEDATA: [segment index][offset: u16][bytes]
                let index = *data.first()? as usize;
                let payload = data.get(3..)?;
                if let Some(size) = sizes.get_mut(index) {
                    *size += payload.len();
                }
            }
            _ => {}
        }
    }
    // The code segment: the one whose name says so; otherwise the first that has bytes.
    let code = segments
        .iter()
        .position(|name| name.as_deref().is_some_and(|name| name.ends_with("TEXT")))
        .filter(|index| *index > 0)
        .or_else(|| sizes.iter().position(|size| *size > 0))?;
    sizes.get(code).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_coff_text_section_is_its_size_of_raw_data() {
        // A minimal COFF: two sections, no optional header; the second is `.text` with 64 bytes.
        let mut object = vec![0u8; 20];
        object[2..4].copy_from_slice(&2u16.to_le_bytes()); // two sections
        let mut sections = Vec::new();
        let mut first = vec![0u8; 40];
        first[..6].copy_from_slice(b".data\0");
        first[16..20].copy_from_slice(&8u32.to_le_bytes());
        sections.extend(first);
        let mut second = vec![0u8; 40];
        second[..6].copy_from_slice(b".text\0");
        second[16..20].copy_from_slice(&64u32.to_le_bytes());
        sections.extend(second);
        object.extend(sections);
        assert_eq!(coff_text(&object), Some(64));
        assert_eq!(
            coff_text(&object[..30]),
            None,
            "a truncated file is refused"
        );
    }

    #[test]
    fn an_omf_code_segment_sums_its_ledata_payloads() {
        // LNAMES with one name, `_TEXT`; a SEGDEF naming it; two LEDATA records of 4 and 3 bytes.
        let mut object = Vec::new();
        let mut record = |kind: u8, data: &[u8]| {
            object.push(kind);
            object.extend((data.len() as u16).to_le_bytes());
            object.extend(data);
        };
        record(0x96, &[5, b'_', b'T', b'E', b'X', b'T']);
        // [attributes][length: u16][name index][class index]
        record(0x98, &[0x48, 0x10, 0x00, 1, 1]);
        // [segment index][offset: u16][bytes]
        record(0xA0, &[1, 0x00, 0x00, 0x90, 0x90, 0x90, 0x90]);
        record(0xA0, &[1, 0x04, 0x00, 0xC3, 0x90, 0x90]);
        record(0x8B, &[0, 0]);
        assert_eq!(omf_text(&object), Some(7));
    }
}
