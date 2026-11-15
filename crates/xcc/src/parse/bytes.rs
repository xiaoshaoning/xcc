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
    if types.is_error(ty) || types.is_error(value.ty) || value.constant.is_none() {
        return None;
    }
    let size = types.size_of(ty)? as usize;
    let mut bytes = vec![0u8; size];
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

/// The bytes a string literal lays down in an array of type `ty`: its characters, then zeroes to
/// the end of the array, which is what C99 6.7.8p21 says the rest of the array holds.
///
/// `None` when the array is not an array of `char`, when the literal does not fit (there must be
/// room for its NUL), or when a byte in it is an escape this compiler does not read.
pub(super) fn string_literal_bytes(types: &Types, ty: TypeId, spelling: &str) -> Option<Vec<u8>> {
    let Type::Array { element, count } = *types.kind(ty) else {
        return None;
    };
    if !matches!(types.kind(element), Type::Int { bits: 8, .. }) {
        return None;
    }
    let count = count? as usize;
    let mut bytes = crate::literal::string_bytes(spelling)?;
    if bytes.len() >= count {
        return None;
    }
    bytes.resize(count, 0);
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
            string_literal_bytes(&types, six, r#""hi""#),
            Some(vec![b'h', b'i', 0, 0, 0, 0])
        );
        assert_eq!(
            string_literal_bytes(&types, two, r#""hi""#),
            None,
            "no room for the NUL"
        );
        assert_eq!(
            string_literal_bytes(&types, ints, r#""hi""#),
            None,
            "not an array of char"
        );
        assert_eq!(
            string_literal_bytes(&types, six, r#""\q""#),
            None,
            "an escape nobody reads"
        );
    }
}
