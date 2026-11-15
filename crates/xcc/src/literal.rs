//! What a literal's spelling means: the escapes in a character constant, and how many bytes a
//! string literal spells.
//!
//! One place, because the preprocessor and the parser both need it: `#if 'a' == 97` and
//! `char c = 'a'` are the same question about the same spelling, and two tables would answer it
//! two ways (they did - one knew `\x41`, the other did not, and the parser turned what it did
//! not know into zero without saying so).

/// A character constant's value, or `None` when it is one this compiler does not read.
///
/// `None` is not "zero": a multi-character constant (`'ab'`) has an implementation-defined value
/// and is refused, and so is an escape that does not name a character. C99 6.4.4.4.
pub fn character_constant(spelling: &str) -> Option<i64> {
    let body = spelling.strip_prefix('L').unwrap_or(spelling);
    let body = body.strip_prefix('\'')?.strip_suffix('\'')?;
    if let Some(escape) = body.strip_prefix('\\') {
        return escape_value(escape);
    }
    let mut characters = body.chars();
    let first = characters.next()?;
    // `'ab'` is not a single character, and C leaves its value to the implementation.
    if characters.next().is_some() {
        return None;
    }
    Some(i64::from(u32::from(first)))
}

/// The value of an escape sequence, given what follows the backslash.
fn escape_value(escape: &str) -> Option<i64> {
    let simple = match escape {
        "n" => 10,
        "t" => 9,
        "r" => 13,
        "a" => 7,
        "b" => 8,
        "f" => 12,
        "v" => 11,
        "\\" => 92,
        "'" => 39,
        "\"" => 34,
        "?" => 63,
        _ => {
            // `\x41` takes any number of hex digits; octal takes at most three.
            if let Some(hex) = escape.strip_prefix('x') {
                return u32::from_str_radix(hex, 16).ok().map(i64::from);
            }
            let octal = escape.len() <= 3 && escape.bytes().all(|byte| matches!(byte, b'0'..=b'7'));
            if !octal {
                return None;
            }
            return u32::from_str_radix(escape, 8).ok().map(i64::from);
        }
    };
    Some(simple)
}

/// How many bytes a string literal spells, not counting its quotes or the NUL.
///
/// An escape is one byte however it is written, so `"\x41"`, `"\101"` and `"A"` are one
/// byte each: an escape is scanned to its end rather than counted by its letters. A character
/// outside ASCII takes the bytes it takes in UTF-8, which is what a compilation reads.
pub fn string_length(spelling: &str) -> u64 {
    let body = spelling.strip_prefix('L').unwrap_or(spelling);
    let body = body
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(body);
    let bytes = body.as_bytes();
    let mut at = 0;
    let mut length = 0u64;
    while at < bytes.len() {
        if bytes[at] == b'\\' && at + 1 < bytes.len() {
            at += 2;
            match bytes[at - 1] {
                b'x' => {
                    while at < bytes.len() && bytes[at].is_ascii_hexdigit() {
                        at += 1;
                    }
                }
                b'0'..=b'7' => {
                    let mut digits = 1;
                    while digits < 3 && at < bytes.len() && matches!(bytes[at], b'0'..=b'7') {
                        at += 1;
                        digits += 1;
                    }
                }
                _ => {}
            }
            length += 1;
            continue;
        }
        let width = match bytes[at] {
            0x00..=0x7f => 1,
            0xc0..=0xdf => 2,
            0xe0..=0xef => 3,
            _ => 4,
        }
        .min(bytes.len() - at);
        at += width;
        length += width as u64;
    }
    length
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn character_constants_decode_every_escape_c99_has() {
        for (spelling, value) in [
            ("'a'", 97),
            ("L'z'", 122),
            ("'\\n'", 10),
            ("'\\t'", 9),
            ("'\\r'", 13),
            ("'\\a'", 7),
            ("'\\b'", 8),
            ("'\\f'", 12),
            ("'\\v'", 11),
            ("'\\0'", 0),
            ("'\\\\'", 92),
            ("'\\''", 39),
            ("'\\\"'", 34),
            ("'\\?'", 63),
            ("'\\x41'", 65),
            ("'\\101'", 65),
            ("'\\7'", 7),
        ] {
            assert_eq!(character_constant(spelling), Some(value), "{spelling}");
        }
    }

    #[test]
    fn what_is_not_a_character_constant_is_not_zero() {
        for spelling in ["'ab'", "'\\q'", "'\\x'", "'\\8'", "''", "'a", "a'"] {
            assert_eq!(character_constant(spelling), None, "{spelling}");
        }
    }

    #[test]
    fn a_string_length_counts_bytes_not_spellings() {
        for (spelling, length) in [
            (r#""""#, 0),
            (r#""abc""#, 3),
            (r#"L"ab""#, 2),
            (r#""\x41""#, 1),
            (r#""\x41z""#, 2),
            (r#""\101""#, 1),
            (r#""\0""#, 1),
            (r#""a\\""#, 2),
            (r#""a\"b""#, 3),
            ("\"\u{e9}\"", 2),
        ] {
            assert_eq!(string_length(spelling), length, "{spelling}");
        }
    }
}
