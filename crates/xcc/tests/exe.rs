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

#[test]
fn a_program_using_the_c_library_links_against_it_and_runs() {
    // M8's libc claim, minus the variadic half: a program that includes our headers and calls the
    // C runtime's own `fopen`, `fwrite`, `fseek`, `fgetc`, `fclose` and `remove` - a 32-bit
    // `fseek` offset and all - built by xcc and linked by the host's linker.
    let Some(linker) = linker() else {
        eprintln!("no linker on this machine: the C library test is skipped");
        return;
    };
    let source = "
#include <stdio.h>

int main(void) {
    const char *path = \"xcc_c_library_test.bin\";
    FILE *out = fopen(path, \"wb\");
    if (!out) return 1;
    if (fwrite(\"abcdef\", 1, 6, out) != 6) return 2;
    if (fclose(out) != 0) return 3;
    FILE *in = fopen(path, \"rb\");
    if (!in) return 4;
    if (fseek(in, 4, SEEK_SET) != 0) return 5;
    int c = fgetc(in);
    if (fclose(in) != 0) return 6;
    remove(path);
    return c == 'e' ? 7 : 8;
}
";
    let artifact = Compile::new(Options::default())
        .compile("t.c", source)
        .expect("it compiles");
    let dir = temp_dir("clib");
    let object = dir.join("t.o");
    let exe = dir.join("t.exe");
    std::fs::write(&object, artifact.object()).expect("the object is written");
    let link = Command::new(linker)
        .arg("-o")
        .arg(&exe)
        .arg(&object)
        .output()
        .expect("the linker runs");
    let linked = link.status.success();
    if !linked {
        let _ = std::fs::remove_dir_all(&dir);
        panic!(
            "linking against the C library failed: {}",
            String::from_utf8_lossy(&link.stderr)
        );
    }
    // The program's working directory is the test's, so the file it writes goes to a temp path.
    let run = Command::new(&exe)
        .current_dir(&dir)
        .output()
        .expect("the program runs");
    let status = run.status.code();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(status, Some(7), "the C library answered as C says it does");
}
