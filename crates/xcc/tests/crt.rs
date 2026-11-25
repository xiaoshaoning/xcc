//! The C runtime has to be C that xcc can compile, or ADR-005's claim is a wish.
//!
//! It is compiled at `--opt-level=2` because a link is: the runtime's `printf` front ends walk a
//! `va_list`, and only this compiler's own back end lowers that. A level the link cannot use is a
//! level at which the runtime's source is not a claim worth making.

use xcc::{Compile, Options};

/// The runtime's source, as the repository has it.
const CRT: &str = include_str!("../../xcc-crt/src/main.c");

#[test]
fn the_crt_compiles_with_xcc() {
    // C11, the dialect a link compiles the runtime in: it is this compiler's own source, so the
    // program's `--std` is not its choice, and `threads.h` - which it includes - is C11's.
    let artifact = Compile::new(Options {
        opt_level: 2,
        standard: xcc::CStandard::C11,
        ..Options::default()
    })
    .compile("main.c", CRT)
    .expect("the crt compiles");
    assert!(
        !artifact.bytes().is_empty(),
        "and it produces an object, not just no diagnostics"
    );
    assert_eq!(&artifact.bytes()[0..2], &[0x64, 0x86], "x86-64 COFF");
}
