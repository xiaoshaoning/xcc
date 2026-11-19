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

#[test]
fn a_program_that_prints_links_and_prints_the_same_as_clang() {
    // M8's acceptance item, the one ADR-006 blocked: `printf`, `snprintf` and `fprintf` with
    // integer, character, string and pointer arguments - a variadic call whose signature only the
    // call site knows, lowered as an indirect call. The program is a `.c` file beside this test,
    // because the last two attempts to keep it in a string literal lost an escape each time.
    let Some(linker) = linker() else {
        eprintln!("no linker on this machine: the printf test is skipped");
        return;
    };
    let source = include_str!("printf_acceptance.c");
    let artifact = Compile::new(Options::default())
        .compile("t.c", source)
        .expect("a variadic call compiles");
    let dir = temp_dir("printf");
    let object = dir.join("t.o");
    let exe = dir.join("t.exe");
    std::fs::write(&object, artifact.object()).expect("the object is written");
    let link = Command::new(linker)
        .arg("-o")
        .arg(&exe)
        .arg(&object)
        .output()
        .expect("the linker runs");
    if !link.status.success() {
        let _ = std::fs::remove_dir_all(&dir);
        panic!("linking failed: {}", String::from_utf8_lossy(&link.stderr));
    }

    // The same source through clang, with the headers xcc ships, so the output is compared with
    // a reference rather than eyeballed.
    let source_file = dir.join("t.c");
    std::fs::write(&source_file, source).expect("the source is written");
    let reference = dir.join("reference.exe");
    let built = Command::new(linker)
        .arg("-O0")
        .arg("-w")
        .arg("-isystem")
        .arg(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../xcc/include"))
        .arg("-o")
        .arg(&reference)
        .arg(&source_file)
        .status()
        .expect("clang runs");

    let ours = Command::new(&exe)
        .current_dir(&dir)
        .output()
        .expect("the program runs");
    let theirs = built
        .success()
        .then(|| Command::new(&reference).current_dir(&dir).output())
        .and_then(Result::ok);
    let _ = std::fs::remove_dir_all(&dir);

    // `answer=42` is nine characters, which is what snprintf returns - the count that *would*
    // have been written, and the program returns it.
    assert_eq!(ours.status.code(), Some(9), "snprintf wrote nine bytes");
    if let Some(theirs) = theirs {
        assert_eq!(ours.stdout, theirs.stdout, "stdout differs from clang's");
        assert_eq!(ours.stderr, theirs.stderr, "stderr differs from clang's");
    }
}

/// Compile `ours` with this compiler and `theirs` with `clang`, link the two, run the program, and
/// give its exit status. `linker` is named rather than found because the *other* side is part of the
/// test: a convention has two of them, and only a foreign compiler can disagree with ours.
fn linked_with_clang(
    linker: &str,
    dir: &std::path::Path,
    ours: &str,
    theirs: &str,
) -> Result<Option<i32>, String> {
    let object = dir.join("ours.obj");
    let artifact = Compile::new(Options {
        opt_level: 2,
        ..Options::default()
    })
    .compile("ours.c", ours)
    .map_err(|diagnostics| format!("{diagnostics:?}"))?;
    std::fs::write(&object, artifact.object()).expect("the object is written");
    let their_source = dir.join("theirs.c");
    std::fs::write(&their_source, theirs).expect("the other source is written");
    let exe = dir.join("t.exe");
    let link = Command::new(linker)
        .args(["-O0", "-w", "-o"])
        .arg(&exe)
        .arg(&object)
        .arg(&their_source)
        .output()
        .expect("the linker runs");
    if !link.status.success() {
        return Err(format!(
            "the link failed: {}",
            String::from_utf8_lossy(&link.stderr)
        ));
    }
    Ok(Command::new(&exe)
        .output()
        .expect("the program runs")
        .status
        .code())
}

/// The ABI, checked against a compiler that is not this one.
///
/// A calling convention has two sides, and a test that compiles both sides with the same compiler
/// cannot see a mistake they share: `f(int a, double b, double c)` had its `double`s in `xmm1` and
/// `xmm2` here, would have had them in `xmm0` and `xmm1` from a per-class counter, and every test of
/// ours passed because it called our own code from our own code. Two such mistakes shipped before
/// this test existed. Both directions are here, and the variadic read with them.
#[test]
fn the_calling_convention_agrees_with_clang_in_both_directions() {
    // `clang` by name, not whichever linker is first on PATH: the *other* side has to follow Win64's
    // varargs rule, and MinGW's gcc does not duplicate a variadic `double` into a general register, so
    // a gcc caller would read this compiler's callee as zeros. The oracle is named for the same
    // reason a measurement is instrumented.
    let Ok(clang) = Command::new("clang").arg("--version").output() else {
        eprintln!("no clang on this machine: the ABI test is skipped");
        return;
    };
    assert!(clang.status.success(), "clang answers");
    // The callee has an integer between two doubles, so the *position* decides the register - the
    // mistake that shipped twice. The variadic half goes one way only: a clang caller duplicates a
    // `double` into a general register and our callee reads it, while our *caller* cannot yet
    // duplicate one, which is the row ADR-006 still refuses.
    let fixed_callee = r#"
        double g(int a, double b, double c) { return b + c + a; }
    "#;
    let variadic_callee = r#"
        #include <stdarg.h>
        double g(int a, double b, double c) { return b + c + a; }
        double pick(int count, ...) {
            va_list ap; double total = 0.0; int i;
            va_start(ap, count);
            for (i = 0; i < count; i++) total = total + va_arg(ap, double);
            va_end(ap); return total;
        }
    "#;
    let caller_calling_both = r#"
        double g(int a, double b, double c);
        double pick(int count, ...);
        int main(void) { return (int)((g(1, 2.5, 3.5) + pick(3, 1.5, 2.25, 3.25)) * 2.0); }
    "#;
    let caller_calling_fixed = r#"
        double g(int a, double b, double c);
        int main(void) { return (int)(g(1, 2.5, 3.5) * 2.0); }
    "#;
    // `g(1, 2.5, 3.5)` is 7.0 and `pick(3, 1.5, 2.25, 3.25)` is 7.0, so the answers are 28 and 14.
    let cases = [
        (
            "xcc callee, clang caller",
            variadic_callee,
            caller_calling_both,
            28,
        ),
        (
            "clang callee, xcc caller",
            fixed_callee,
            caller_calling_fixed,
            14,
        ),
    ];
    let dir = temp_dir("abi");
    let mut wrong = Vec::new();
    for (what, ours, theirs, expected) in cases {
        match linked_with_clang("clang", &dir, ours, theirs) {
            Ok(Some(status)) if status == expected => {}
            Ok(other) => wrong.push(format!(
                "{what}: exited with {other:?}, expected {expected}"
            )),
            Err(why) => wrong.push(format!("{what}: {why}")),
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
