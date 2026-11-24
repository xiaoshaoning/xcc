//! A global's bytes: what one constant lays down in memory.
//!
//! ADR-004 puts this in the front end, because the front end owns the layout (`Types::size_of`,
//! member offsets, bitfields). A back end that recomputed it would be a second implementation of
//! C's layout rules, and the two would disagree silently.
//!
//! These are the leaf writers, free functions over the type table. The list walker that places
//! leaves inside an aggregate calls whichever one the token implies; nothing here knows about
//! initialiser syntax.

#![allow(dead_code)] // the list walker is the caller: ADR-004, patch 1's other half

use super::*;

/// The bytes a constant of type `ty` lays down: little-endian, exactly `size_of(ty)` of them,
/// zeroes beyond the value's own width.
///
/// `None` when the value is not a constant this can write - a float (M6), a pointer that is not
/// null, an aggregate, or an error - which the caller turns into one diagnostic rather than a
/// guess.
pub(super) fn constant_bytes(types: &Types, ty: TypeId, value: Value) -> Option<Vec<u8>> {
    if types.is_error(ty) || types.is_error(value.ty) {
        return None;
    }
    let size = types.size_of(ty)? as usize;
    let mut bytes = vec![0u8; size];
    if matches!(types.kind(ty), Type::Bool) {
        // C99 6.3.1.2: a `_Bool` holds 0 or 1, whatever constant it was converted from - `2` is
        // one byte of 1, and `0.0` is one byte of 0.
        let bits = value.constant?;
        return Some(vec![u8::from(!types.constant_is_zero(value.ty, bits))]);
    }
    if types.is_pointer(ty) {
        // Only the null pointer is a constant, and its bytes are already zeroes.
        return match value.constant {
            Some(0) => Some(bytes),
            _ => None,
        };
    }
    if types.is_arithmetic(ty) {
        let constant = value.constant?;
        let width = size.min(8);
        bytes[..width].copy_from_slice(&constant.to_le_bytes()[..width]);
        return Some(bytes);
    }
    None
}

/// Put a bitfield's constant into the storage unit at `at`: the field's bits of the unit are
/// replaced, its neighbours' are left as they are. The unit is as wide as the value's bytes, so
/// `bytes` is exactly the unit - the same width `constant_bytes` gave it.
///
/// The code path merges with shifts and masks on values; this one does the same arithmetic on the
/// object's bytes. Both take the mask from `field_mask`, so a field of `width` bits is the same
/// field whichever end writes it.
pub(super) fn merge_bits(out: &mut [u8], at: usize, bytes: &[u8], start: u8, width: u8) {
    let unit_width = bytes.len().min(8);
    if at + unit_width > out.len() || unit_width == 0 {
        return;
    }
    let read = |from: &[u8]| {
        let mut unit = [0u8; 8];
        unit[..unit_width].copy_from_slice(&from[..unit_width]);
        u64::from_le_bytes(unit)
    };
    let mask = super::expr::field_mask(width);
    let field = (read(bytes) & mask) << start;
    let unit = (read(out) & !(mask << start)) | field;
    out[at..at + unit_width].copy_from_slice(&unit.to_le_bytes()[..unit_width]);
}

/// The bytes a string literal lays down in an array of type `ty`: its characters and their NUL,
/// then zeroes to the end of the array, which is what C99 6.7.8p21 says the rest holds.
///
/// `literal` is what `string_literal` read from the source, NUL included: the literal is
/// decoded once, and both destinations - a global's bytes and a local's stores - take these.
/// `None` when the array is not an array of `char` or the literal does not fit.
pub(super) fn string_literal_bytes(types: &Types, ty: TypeId, literal: &[u8]) -> Option<Vec<u8>> {
    let Type::Array { element, count } = *types.kind(ty) else {
        return None;
    };
    // A character array holds the literal's own characters: one byte each for `char`, two for the
    // target's `wchar_t`. The literal's bytes are already in that encoding - `L"ab"` arrives as
    // four UTF-16 bytes - so the room is the element width times the count.
    let width = match *types.kind(element) {
        Type::Int { bits: 8, .. } => 1,
        Type::Int { bits: 16, .. } => 2,
        _ => return None,
    };
    let count = count? as usize;
    let room = count * width;
    // C99 6.7.8p14: the characters fill what room there is and the terminating NUL is dropped when
    // there is none - so a literal one character longer than the array is exact, not too long. Two
    // longer is the constraint violation the caller has already warned about, and what fits is what
    // is written: a truncation the source asked for, not one the compiler chose.
    let mut bytes = literal[..literal.len().min(room)].to_vec();
    bytes.resize(room, 0);
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn types() -> (Types, TypeId) {
        let types = Types::new();
        let int = types.fundamental("int").unwrap();
        (types, int)
    }

    #[test]
    fn a_constant_lays_down_its_own_width_little_endian() {
        let (types, int) = types();
        let long = types.fundamental("long long").unwrap();
        let char = types.fundamental("char").unwrap();
        assert_eq!(
            constant_bytes(&types, int, Value::constant(int, 1)),
            Some(vec![1, 0, 0, 0])
        );
        assert_eq!(
            constant_bytes(&types, long, Value::constant(long, 0x0102_0304)),
            Some(vec![4, 3, 2, 1, 0, 0, 0, 0])
        );
        // A negative value is its two's complement, all the way out.
        assert_eq!(
            constant_bytes(&types, int, Value::constant(int, -1)),
            Some(vec![0xff, 0xff, 0xff, 0xff])
        );
        assert_eq!(
            constant_bytes(&types, char, Value::constant(char, 65)),
            Some(vec![65])
        );
    }

    #[test]
    fn what_is_not_a_constant_is_not_guessed() {
        let (mut types, int) = types();
        let pointer = types.pointer(int);
        assert_eq!(constant_bytes(&types, int, Value::temporary(int)), None);
        assert_eq!(
            constant_bytes(&types, pointer, Value::constant(pointer, 4096)),
            None,
            "a non-null address is not a constant"
        );
        assert_eq!(
            constant_bytes(&types, pointer, Value::constant(pointer, 0)),
            Some(vec![0; 8]),
            "the null pointer is zeroes"
        );
    }

    #[test]
    fn a_string_fills_its_array_and_zeroes_the_rest() {
        let (mut types, _) = types();
        let char = types.fundamental("char").unwrap();
        let six = types.array(char, Some(6));
        let two = types.array(char, Some(2));
        let ints = types.array(types.fundamental("int").unwrap(), Some(3));
        assert_eq!(
            string_literal_bytes(&types, six, b"hi\0"),
            Some(vec![b'h', b'i', 0, 0, 0, 0])
        );
        // C99 6.7.8p14: the characters fill what room there is and the NUL is dropped when
        // there is none, so a two-byte array takes "hi" exactly. A literal even longer is the
        // caller's warning, and what fits is what is written.
        assert_eq!(
            string_literal_bytes(&types, two, b"hi\0"),
            Some(vec![b'h', b'i']),
            "the NUL is dropped when there is no room for it"
        );
        assert_eq!(
            string_literal_bytes(&types, two, b"hii\0"),
            Some(vec![b'h', b'i']),
            "what fits is what is written"
        );
        assert_eq!(
            string_literal_bytes(&types, ints, b"hi\0"),
            None,
            "not an array of char"
        );
        assert_eq!(
            string_literal_bytes(&types, two, b"h\0"),
            Some(vec![b'h', 0]),
            "an exact fit leaves no zeroes to add"
        );
    }
}
