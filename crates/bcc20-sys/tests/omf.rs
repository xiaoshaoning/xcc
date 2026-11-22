//! The OMF reader against an object the ported compiler actually wrote.
//!
//! The crate's own tests build records by hand and check the arithmetic; this one checks the
//! arithmetic against 1991's output, which is where the parts a hand-built record set does not have
//! live - the checksum byte at the end of every record, and Borland's own segment names.
//!
//! `aggregates.omf` is `corpus/aggregates.c` compiled by the port (`xcc --target=dos16 -c`, or
//! `bcc20run`): 401 bytes of OMF, 107 of them the code segment `AGG_TEXT`.

#[test]
fn a_real_borland_object_reports_its_code_segment() {
    let object = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/aggregates.omf"))
        .expect("the fixture");
    assert_eq!(
        object.len(),
        401,
        "the fixture is the object the port wrote"
    );
    assert_eq!(bcc20_sys::object::omf_text(&object), Some(107));
    // And the same object is not a COFF file, which is the other reader's answer.
    assert_eq!(bcc20_sys::object::coff_text(&object), None);
}
