//! The C runtime has to be C that xcc can compile, or ADR-005's claim is a wish.
//!
//! This test is the crt's only caller until the linking story lands: it keeps the file honest
//! (it compiles with the compiler it belongs to) and keeps it from drifting as the front end
//! changes.

use xcc::{Compile, Options};

/// The runtime's source, as the repository has it.
const CRT: &str = include_str!("../../xcc-crt/src/main.c");

#[test]
fn the_crt_compiles_with_xcc() {
    let artifact = Compile::new(Options::default())
        .compile("main.c", CRT)
        .expect("the crt compiles");
    assert!(
        !artifact.object().is_empty(),
        "and it produces an object, not just no diagnostics"
    );
    assert_eq!(&artifact.object()[0..2], &[0x64, 0x86], "x86-64 COFF");
}
