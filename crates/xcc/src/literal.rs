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
    // The prefix says what the value's *type* is, which the caller decides; the value is the
    // character's code point whatever the prefix - and `'é'` in a UTF-8 source is one character
    // however many bytes it is written in.
    let body = body_of(spelling)?;
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
    let bytes = escape.as_bytes();
    let first = *bytes.first()?;
    if let Some(value) = named_escape(first) {
        return if bytes.len() == 1 {
            Some(i64::from(value))
        } else {
            None
        };
    }
    // A numeric escape is one or more digits and nothing else; the range rule is the same one
    // a string literal follows.
    let radix = if first == b'x' { 16 } else { 8 };
    let mut at = if radix == 16 { 1 } else { 0 };
    let value = numeric_escape(bytes, &mut at, radix)?;
    if at == bytes.len() {
        Some(i64::from(value))
    } else {
        None
    }
}

/// The escapes that name one character, by the letter after the backslash. The one table, so a
/// string and a character constant cannot disagree about what `\n` is.
fn named_escape(letter: u8) -> Option<u8> {
    Some(match letter {
        b'n' => 10,
        b't' => 9,
        b'r' => 13,
        b'a' => 7,
        b'b' => 8,
        b'f' => 12,
        b'v' => 11,
        b'\\' => 92,
        b'\'' => 39,
        b'"' => 34,
        b'?' => 63,
        _ => return None,
    })
}

/// A numeric escape's value: `\x` takes any number of hex digits, octal at most three. `None`
/// when the value does not fit a byte, because C requires it to (`\xff` is 255, `\x1ff` is not
/// a character) and truncating would be a wrong program rather than a refusal.
fn numeric_escape(bytes: &[u8], at: &mut usize, radix: u32) -> Option<u8> {
    let start = *at;
    let limit = if radix == 8 { 3 } else { usize::MAX };
    while *at < bytes.len() && (*at - start) < limit && (bytes[*at] as char).is_digit(radix) {
        *at += 1;
    }
    let digits = std::str::from_utf8(&bytes[start..*at]).ok()?;
    let value = u32::from_str_radix(digits, radix).ok()?;
    u8::try_from(value).ok()
}

/// What a string literal spells, byte for byte, without its quotes or the NUL.
///
/// `None` when an escape is one this compiler does not read: a data object filled with a guess
/// would be a program that reads the wrong bytes, which is the failure CONSTRAINTS 3 forbids.
#[allow(dead_code)] // the global initialiser reader is the caller: ADR-004, patch 2
pub fn string_bytes(spelling: &str) -> Option<Vec<u8>> {
    let body = body_of(spelling)?;
    let bytes = body.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] != b'\\' {
            // A character outside ASCII travels as the bytes it is written in.
            out.push(bytes[at]);
            at += 1;
            continue;
        }
        at += 1;
        let escape = *bytes.get(at)?;
        at += 1;
        match escape {
            b'x' => out.push(numeric_escape(bytes, &mut at, 16)?),
            b'0'..=b'7' => {
                // The first digit has already been read; `numeric_escape` wants the cursor on it.
                at -= 1;
                out.push(numeric_escape(bytes, &mut at, 8)?);
            }
            other => out.push(named_escape(other)?),
        }
    }
    Some(out)
}

/// Whether a literal carries the `L` prefix. Its characters are `wchar_t`s - two bytes here -
/// and a string of them is an array of those, which is the whole of what "wide" means in C99.
/// The encoding prefix a string or character literal was written with (C11 6.4.5p1).
///
/// It was a `bool` - "wide or not" - while `L` was the only prefix. C11 has four, and a boolean
/// would have made `u` and `U` and `u8` one thing: they are two, two and one byte respectively.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prefix {
    /// No prefix: `"abc"` is an array of `char`, and `'a'` is an `int`.
    None,
    /// `L`: an array of `wchar_t`, which is 16 bits on this target.
    Wide,
    /// `u`: an array of `char16_t`, which is 16 bits.
    Utf16,
    /// `U`: an array of `char32_t`, which is 32.
    Utf32,
    /// `u8`: an array of `char`, holding UTF-8 - which is how this compiler's source is already
    /// read, so the same bytes as no prefix at all.
    Utf8,
}

impl Prefix {
    /// The prefix as the source writes it, for a diagnostic: a message that says `Utf16` is
    /// speaking Rust at a C programmer.
    pub fn spelling(self) -> &'static str {
        match self {
            Prefix::None => "",
            Prefix::Wide => "L",
            Prefix::Utf16 => "u",
            Prefix::Utf32 => "U",
            Prefix::Utf8 => "u8",
        }
    }

    /// How wide one element is, in bytes.
    pub fn width(self) -> usize {
        match self {
            Prefix::Utf32 => 4,
            Prefix::Wide | Prefix::Utf16 => 2,
            Prefix::None | Prefix::Utf8 => 1,
        }
    }
}

/// Which prefix a spelling starts with. `u8` is read before `u`, or two prefixes would be one.
pub fn prefix_of(spelling: &str) -> Prefix {
    if spelling.starts_with("u8") {
        Prefix::Utf8
    } else if spelling.starts_with('u') {
        Prefix::Utf16
    } else if spelling.starts_with('U') {
        Prefix::Utf32
    } else if spelling.starts_with('L') {
        Prefix::Wide
    } else {
        Prefix::None
    }
}

/// The spelling without its prefix and its quotes.
fn body_of(spelling: &str) -> Option<&str> {
    let rest = match prefix_of(spelling) {
        Prefix::Utf8 => spelling.strip_prefix("u8")?,
        Prefix::Utf16 => spelling.strip_prefix('u')?,
        Prefix::Utf32 => spelling.strip_prefix('U')?,
        Prefix::Wide => spelling.strip_prefix('L')?,
        Prefix::None => spelling,
    };
    let quote = rest.chars().next()?;
    rest.strip_prefix(quote)?.strip_suffix(quote)
}

/// What a string literal holds, as bytes, without the terminator: one element per character of the
/// literal, in the width and encoding its prefix asks for.
pub fn bytes_of(spelling: &str) -> Option<Vec<u8>> {
    match prefix_of(spelling) {
        // `L` and `u` are both UTF-16 units here, because this target's `wchar_t` is 16 bits - one
        // encoder for the two, and the *type* is what differs.
        Prefix::Wide | Prefix::Utf16 => wide_bytes(spelling),
        Prefix::Utf32 => utf32_bytes(spelling),
        Prefix::None | Prefix::Utf8 => string_bytes(spelling),
    }
}

/// A `U` literal: one 32-bit unit per code point, with no surrogate pairs to keep.
fn utf32_bytes(spelling: &str) -> Option<Vec<u8>> {
    let body = body_of(spelling)?;
    let bytes = body.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() * 4);
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] != b'\\' {
            let rest = std::str::from_utf8(&bytes[at..]).ok()?;
            let character = rest.chars().next()?;
            out.extend_from_slice(&u32::from(character).to_le_bytes());
            at += character.len_utf8();
            continue;
        }
        at += 1;
        let escape = *bytes.get(at)?;
        at += 1;
        let value = match escape {
            b'x' => numeric_escape_utf32(bytes, &mut at, 16)?,
            b'0'..=b'7' => {
                at -= 1;
                numeric_escape_utf32(bytes, &mut at, 8)?
            }
            other => u32::from(named_escape(other)?),
        };
        out.extend_from_slice(&value.to_le_bytes());
    }
    Some(out)
}

/// A numeric escape in a `U` literal: any value a `char32_t` can hold.
fn numeric_escape_utf32(bytes: &[u8], at: &mut usize, radix: u32) -> Option<u32> {
    let start = *at;
    let limit = if radix == 8 { 3 } else { usize::MAX };
    while *at < bytes.len() && (*at - start) < limit && (bytes[*at] as char).is_digit(radix) {
        *at += 1;
    }
    let digits = std::str::from_utf8(&bytes[start..*at]).ok()?;
    u32::from_str_radix(digits, radix)
        .ok()
        .filter(|value| *value <= 0x10FFFF)
}

/// What a wide string literal holds, as UTF-16 code units in little-endian bytes, without the
/// NUL: the target's `wchar_t` is 16 bits, so a character above the basic plane is two units.
///
/// The escapes are read here rather than by [`string_bytes`] because a wide literal's escapes are
/// *code unit* values: `L"\x1FF"` is one character of 0x1FF, where a narrow literal's `\x1FF` does
/// not fit a byte at all.
///
/// `None` when an escape is one this compiler does not read, or a numeric escape does not fit the
/// target's `wchar_t`. A guessed character is a program that prints the wrong text.
pub fn wide_bytes(spelling: &str) -> Option<Vec<u8>> {
    let body = body_of(spelling)?;
    let bytes = body.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() * 2);
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] != b'\\' {
            // Everything that is not an escape is a character of the source, which is UTF-8: its
            // *code point* is what a wide literal stores, so a literal `e` with an acute accent is
            // one unit, not the two bytes it is written in.
            let rest = std::str::from_utf8(&bytes[at..]).ok()?;
            let character = rest.chars().next()?;
            push_code_point(&mut out, u32::from(character))?;
            at += character.len_utf8();
            continue;
        }
        at += 1;
        let escape = *bytes.get(at)?;
        at += 1;
        let value = match escape {
            b'x' => numeric_escape_wide(bytes, &mut at, 16)?,
            b'0'..=b'7' => {
                at -= 1;
                numeric_escape_wide(bytes, &mut at, 8)?
            }
            other => u32::from(named_escape(other)?),
        };
        push_code_point(&mut out, value)?;
    }
    Some(out)
}

/// One character as UTF-16 units in little-endian bytes, or `None` if it is not a character.
fn push_code_point(out: &mut Vec<u8>, at: u32) -> Option<()> {
    let character = char::from_u32(at)?;
    let mut units = [0u16; 2];
    for unit in character.encode_utf16(&mut units) {
        out.extend_from_slice(&unit.to_le_bytes());
    }
    Some(())
}

/// A numeric escape's value for a wide literal: `\x` takes any number of hex digits, octal at most
/// three, and the value must fit the target's 16-bit `wchar_t`.
fn numeric_escape_wide(bytes: &[u8], at: &mut usize, radix: u32) -> Option<u32> {
    let start = *at;
    let limit = if radix == 8 { 3 } else { usize::MAX };
    while *at < bytes.len() && (*at - start) < limit && (bytes[*at] as char).is_digit(radix) {
        *at += 1;
    }
    let digits = std::str::from_utf8(&bytes[start..*at]).ok()?;
    let value = u32::from_str_radix(digits, radix).ok()?;
    u16::try_from(value).ok().map(u32::from)
}

/// How many bytes a string literal spells, not counting its quotes or the NUL.
///
/// An escape is one byte however it is written, so `"\x41"`, `"\101"` and `"A"` are one
/// byte each: an escape is scanned to its end rather than counted by its letters. A character
/// outside ASCII takes the bytes it takes in UTF-8, which is what a compilation reads.
pub fn string_length(spelling: &str) -> u64 {
    let body = body_of(spelling).unwrap_or(spelling);
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

/// The value of a floating constant (C99 6.4.4.2): how many bits its type has - 32 for the `f`
/// suffix, 64 otherwise, since this compiler makes `long double` 64-bit - and the bits
/// themselves.
///
/// `None` for a hexadecimal constant (`0x1.8p3`): C99 has them and this reader does not, which
/// the caller says rather than reading the digits as something else. The value is rounded to
/// the type exactly once, through `f64`, which is what C says the result is.
pub fn float_value(spelling: &str) -> Option<(u8, u64)> {
    let lower = spelling.to_ascii_lowercase();
    let single = lower.ends_with('f');
    let body = lower.trim_end_matches(['f', 'l']);
    if body.starts_with("0x") {
        return None;
    }
    // `1.` and `.5` are constants; `f64`'s reader wants a digit on both sides of the point.
    let body = if body.starts_with('.') {
        format!("0{body}")
    } else if body.ends_with('.') {
        format!("{body}0")
    } else {
        body.to_owned()
    };
    let value: f64 = body.parse().ok()?;
    Some(if single {
        (32, u64::from((value as f32).to_bits()))
    } else {
        (64, value.to_bits())
    })
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
    fn a_string_literal_decodes_to_its_bytes() {
        assert_eq!(string_bytes(r#""""#), Some(Vec::new()));
        assert_eq!(string_bytes(r#""hi""#), Some(b"hi".to_vec()));
        assert_eq!(string_bytes(r#""a\nb""#), Some(b"a\nb".to_vec()));
        assert_eq!(string_bytes(r#""\x41\101""#), Some(b"AA".to_vec()));
        assert_eq!(string_bytes(r#""\\""#), Some(b"\\".to_vec()));
        assert_eq!(string_bytes(r#""\q""#), None, "an escape nobody reads");
        assert_eq!(string_bytes(r#""\x""#), None, "a hex escape with no digits");
        // C requires an escape to fit a byte; truncating would be a wrong program, not a refusal.
        assert_eq!(string_bytes(r#""\x1ff""#), None, "a hex escape above 255");
    }

    #[test]
    fn the_length_and_the_bytes_agree() {
        for spelling in [r#""""#, r#""hi""#, r#""a\nb""#, r#""\x41\101""#, r#""\0""#] {
            assert_eq!(
                string_length(spelling) as usize,
                string_bytes(spelling).expect("it decodes").len(),
                "{spelling}"
            );
        }
    }

    #[test]
    fn a_floating_constant_is_read_once_at_its_type() {
        assert_eq!(float_value("1.5"), Some((64, 1.5f64.to_bits())));
        assert_eq!(float_value("1.5f"), Some((32, u64::from(1.5f32.to_bits()))));
        assert_eq!(float_value("1.5L"), Some((64, 1.5f64.to_bits())));
        assert_eq!(float_value("1."), Some((64, 1.0f64.to_bits())));
        assert_eq!(float_value(".5"), Some((64, 0.5f64.to_bits())));
        assert_eq!(float_value("1e3"), Some((64, 1000.0f64.to_bits())));
        assert_eq!(float_value("1.5e-3"), Some((64, 1.5e-3f64.to_bits())));
        // An `f` constant is rounded to `float`, not kept as the wider double.
        assert_eq!(float_value("0.1f"), Some((32, u64::from(0.1f32.to_bits()))));
        assert_eq!(
            float_value("0x1.8p1"),
            None,
            "hexadecimal floats are not read yet"
        );
        assert_eq!(float_value("1.5x"), None);
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
