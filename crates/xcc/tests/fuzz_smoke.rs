//! Hostile and random input must not panic.
//!
//! This is not a substitute for `cargo-fuzz` (M3 adds that), just a guard that the plumbing -
//! source positions, spans, rendering - survives bytes nobody wrote for a compiler.

use xcc::{render, Diagnostic, Format, SourceFile, Span};

/// A deterministic generator: no dependency, and a failure can be reproduced from the seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, bound: u32) -> u32 {
        (self.next() % u64::from(bound.max(1))) as u32
    }
}

#[test]
fn random_text_positions_renders_and_spans_never_panic() {
    let mut rng = Rng(0x5eed_1234_abcd_0001);
    let alphabet = b"ab \n\r\t;{}()[]\"'\\\0\x7f\xc3\xa9";

    for case in 0..2_000 {
        let len = rng.below(300);
        let text: Vec<u8> = (0..len)
            .map(|_| alphabet[rng.below(alphabet.len() as u32) as usize])
            .collect();
        let text = String::from_utf8_lossy(&text).into_owned();
        let file = SourceFile::new("fuzz.c", text.clone());

        // every offset, including past the end, in both directions
        for offset in 0..file.len() + 3 {
            let position = file.position(offset);
            assert!(position.line >= 1 && position.column >= 1);
            assert!(file.line(position.line).is_some());
        }

        let start = rng.below(file.len() + 2);
        let length = rng.below(64);
        let diagnostic = Diagnostic::error(format!("case {case}"))
            .with_file("fuzz.c")
            .with_span(Span::new(start, length))
            .with_note("a note");
        for format in [Format::Human, Format::Msvc, Format::Gcc] {
            let rendered = render(&diagnostic, Some(&file), format);
            assert!(rendered.contains(&format!("case {case}")));
        }
    }
}

#[test]
fn a_span_beyond_the_file_renders_at_the_end() {
    let file = SourceFile::new("t.c", "int x;");
    let diagnostic = Diagnostic::error("far away")
        .with_file("t.c")
        .with_span(Span::new(u32::MAX - 1, 10));
    let rendered = render(&diagnostic, Some(&file), Format::Human);
    assert!(rendered.contains("t.c:1:7: error: far away"), "{rendered}");
}
