//! M5's first claim: an object from this compiler links into a `.exe` that runs.
//!
//! The linker is the host's (`cc`, i.e. MinGW's gcc, which drives GNU ld on this machine): xcc
//! writes the object, and nothing about linking is xcc's business yet.

use std::process::Command;

use xcc::{Compile, Options};

fn temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("xcc-exe-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a temporary directory");
    dir
}

/// Whether a linker is on this machine: without one there is nothing to test.
fn linker() -> Option<&'static str> {
    ["cc", "gcc", "clang"]
        .into_iter()
        .find(|candidate| Command::new(candidate).arg("--version").output().is_ok())
}

#[test]
fn a_program_links_and_exits_with_its_return_value() {
    let Some(linker) = linker() else {
        eprintln!("no linker on this machine: the .exe test is skipped");
        return;
    };
    let source = "int main(void) { return 42; }";
    let artifact = Compile::new(Options::default())
        .compile("t.c", source)
        .expect("it compiles");
    let dir = temp_dir("exit");
    let object = dir.join("t.o");
    let exe = dir.join("t.exe");
    std::fs::write(&object, artifact.object()).expect("the object is written");
    let link = Command::new(linker)
        .arg("-o")
        .arg(&exe)
        .arg(&object)
        .output()
        .expect("the linker runs");
    assert!(
        link.status.success(),
        "linking failed: {}",
        String::from_utf8_lossy(&link.stderr)
    );
    let run = Command::new(&exe).output().expect("the program runs");
    let status = run.status.code();
    // The binary and the object are somebody's temp directory; a test that leaves one behind on
    // every run is a slow leak, so it cleans up before it asserts.
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(status, Some(42), "the exit status is the return value");
}
