//! What the command line does, end to end: exit codes, streams and formats.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A directory of our own, removed when the test's `Temp` is dropped.
struct Temp(PathBuf);

impl Temp {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("xcc-cli-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a temporary directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn write(&self, name: &str, text: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, text).expect("a file");
        path
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn xcc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_xcc"))
}

fn run(command: &mut Command) -> Output {
    command.output().expect("the xcc binary runs")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn version_says_who_it_is() {
    let output = run(xcc().arg("--version"));
    assert!(output.status.success());
    assert!(stdout(&output).starts_with("xcc "), "{}", stdout(&output));
}

#[test]
fn help_is_offered() {
    let output = run(xcc().arg("--help"));
    assert!(output.status.success());
    let text = stdout(&output);
    assert!(text.contains("Usage"), "{text}");
    assert!(text.contains("--format"), "{text}");
}

#[test]
fn explain_says_what_each_instruction_was_for() {
    let temp = Temp::new("explain");
    let source = temp.write("t.c", "int f(char a, char b) {\n    return a + b;\n}\n");
    let output = run(xcc().arg("--explain").arg(&source));
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    // The function, the line it came from, the instruction C's rules added, and the registers.
    assert!(text.contains("function f"), "{text}");
    assert!(text.contains("return a + b;"), "{text}");
    // The parameters are values already: mem2reg took the slot and the two loads away, so the
    // conversions read `v0` and `v1` rather than two loads (M10).
    assert!(text.contains("sext char to int v0"), "{text}");
    assert!(text.contains("add int"), "{text}");
    assert!(!text.contains("slot"), "{text}");
    assert!(text.contains("sextend.i32"), "{text}");
    // A mode of its own: no object is written.
    assert!(
        !source.with_extension("obj").exists(),
        "--explain wrote an object"
    );
}

#[test]
fn explain_is_a_mode_and_says_so_when_given_another() {
    let temp = Temp::new("explain-mode");
    let source = temp.write("t.c", "int f(void) { return 1; }\n");
    let output = run(xcc().arg("--explain").arg("-c").arg(&source));
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("--explain is a mode of its own"),
        "{}",
        stderr(&output)
    );
    // `-o` is not part of that mode either: accepting it and writing nothing would be a promise
    // this mode cannot keep.
    let output = run(xcc()
        .arg("--explain")
        .arg("-o")
        .arg(temp.path().join("o.obj"))
        .arg(&source));
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("has nothing to name"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn explain_a_source_that_does_not_compile_reports_its_errors_and_no_explanation() {
    let temp = Temp::new("explain-bad");
    let source = temp.write("t.c", "int f(void) { return nope; }\n");
    let output = run(xcc().arg("--explain").arg(&source));
    assert_eq!(output.status.code(), Some(1));
    assert!(stdout(&output).is_empty(), "{}", stdout(&output));
    assert!(
        stderr(&output).contains("`nope` is not declared"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn an_opt_level_this_back_end_does_not_have_is_refused_as_the_callers_mistake() {
    let temp = Temp::new("opt-level");
    let source = temp.write(
        "t.c",
        "int f(void) { return 1; }
",
    );
    let output = run(xcc().arg("-c").arg("--opt-level=3").arg(&source));
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let message = stderr(&output);
    assert!(
        message.contains("needs this compiler's own back end"),
        "{message}"
    );
    assert!(!message.contains("internal"), "{message}");
    // And the levels it does have compile.
    for level in ["0", "1"] {
        let output = run(xcc()
            .arg("-c")
            .arg(format!("--opt-level={level}"))
            .arg(&source)
            .arg("-o")
            .arg(temp.path().join(format!("t{level}.obj"))));
        assert!(output.status.success(), "{level}: {}", stderr(&output));
    }
}

#[test]
fn the_dos_target_is_the_measuring_stick_and_says_so() {
    let temp = Temp::new("dos16");
    let source = temp.write(
        "t.c",
        "int add(int a, int b) { return a + b; }
",
    );
    let out = temp.path().join("t.obj");
    let output = run(xcc()
        .arg("--target=dos16")
        .arg("-c")
        .arg(&source)
        .arg("-o")
        .arg(&out));
    if std::env::var_os("BCC20_LIB").is_none() && !output.status.success() {
        // The library is an opt-in dependency of this test's machine: without it the command
        // says which one it wanted, which is the part this test can always check.
        assert_eq!(output.status.code(), Some(2));
        assert!(
            stderr(&output).contains("needs libbcc20"),
            "{}",
            stderr(&output)
        );
        return;
    }
    assert!(output.status.success(), "{}", stderr(&output));
    let object = std::fs::read(&out).expect("the DOS object");
    assert_eq!(object[0], 0x80, "an OMF record starts with its type");
    assert!(
        stdout(&output).contains("Borland C++"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn the_headers_that_ship_with_the_compiler_are_found_without_a_flag() {
    let temp = Temp::new("headers");
    let source = temp.write(
        "t.c",
        "#include <stdint.h>
#include <stddef.h>
         struct S { char c; int i; };
         int f(void) { return (int)sizeof(struct S) + (int)offsetof(struct S, i) + UINT8_MAX; }
",
    );
    // No -I: a host's own files are consulted first, and then the compiler's.
    let out = temp.path().join("t.obj");
    let output = run(xcc().arg("-c").arg(&source).arg("-o").arg(&out));
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(std::fs::metadata(&out).is_ok(), "no object was written");
}

#[test]
fn the_same_source_writes_the_same_bytes() {
    // CONSTRAINTS 4: same source, same options, same object. Two *processes*, because that is what
    // catches an order that came from a hash map's iteration or from an address.
    let temp = Temp::new("deterministic");
    let source = temp.write(
        "t.c",
        "struct Point { int x, y; };\n\
         static char greeting[] = \"hello, world\";\n\
         int counter;\n\
         int distance(struct Point p) { return p.x * p.x + p.y * p.y; }\n\
         int add(int a, int b) { return a + b; }\n\
         char *say(void) { return greeting; }\n\
         double half(double x) { return x / 2; }\n\
         int main(void) { struct Point p = { 3, 4 }; return distance(p) + add(counter, 1); }\n",
    );
    let mut bytes = Vec::new();
    for attempt in 0..2 {
        let out = temp.path().join(format!("t{attempt}.obj"));
        let output = run(xcc().arg("-c").arg(&source).arg("-o").arg(&out));
        assert!(output.status.success(), "{}", stderr(&output));
        bytes.push(std::fs::read(&out).expect("the object"));
    }
    assert_eq!(
        bytes[0],
        bytes[1],
        "two compilations of one source disagree: {} vs {} bytes",
        bytes[0].len(),
        bytes[1].len()
    );
}

#[test]
fn diagnostics_go_to_stderr_in_the_asked_for_format() {
    let temp = Temp::new("format");
    let source = temp.write("t.c", "int x = ;\n");
    for (format, expected) in [
        ("msvc", "t.c(1,9): error: expected an initialiser"),
        ("gcc", "t.c:1:9: error: expected an initialiser"),
    ] {
        let output = run(xcc()
            .current_dir(temp.path())
            .args(["-c", "--format", format])
            .arg(&source));
        assert_eq!(output.status.code(), Some(1), "{format}");
        assert!(
            stderr(&output).contains(expected),
            "{format}: {}",
            stderr(&output)
        );
    }
}

#[test]
fn the_human_format_shows_the_line() {
    let temp = Temp::new("human");
    let source = temp.write("t.c", "int x = ;\n");
    let output = run(xcc().args(["-c"]).arg(&source));
    assert_eq!(output.status.code(), Some(1));
    let text = stderr(&output);
    assert!(text.contains("error: expected an initialiser"), "{text}");
    assert!(text.contains("1 | int x = ;"), "{text}");
    assert!(text.contains("  |         ^"), "{text}");
}

#[test]
fn without_c_it_says_linking_is_missing() {
    let temp = Temp::new("noconly");
    let source = temp.write("t.c", "int x;\n");
    let output = run(xcc().arg(&source));
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("linking is not implemented yet"));
}

#[test]
fn a_source_that_is_not_there_is_an_operating_error() {
    let temp = Temp::new("missing");
    let output = run(xcc().current_dir(temp.path()).args(["-c", "not-there.c"]));
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("cannot read not-there.c"));
}

#[test]
fn a_response_file_is_expanded() {
    let temp = Temp::new("response");
    let source = temp.write("t.c", "int x;\n");
    let response = temp.write("args.rsp", &format!("-c --format gcc {}", source.display()));
    let output = run(xcc()
        .current_dir(temp.path())
        .arg(format!("@{}", response.display())));
    // A translation unit of declarations lowers to an empty object, so this succeeds now.
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
}

#[test]
fn an_include_is_read_through_the_include_paths() {
    let temp = Temp::new("include");
    std::fs::create_dir_all(temp.path().join("include")).expect("a directory");
    std::fs::write(
        temp.path().join("include/a.h"),
        "int from_header;
",
    )
    .expect("a header");
    let source = temp.write(
        "t.c",
        "#include <a.h>
int from_source;
",
    );
    let output = run(xcc()
        .args(["-c", "-I"])
        .arg(temp.path().join("include"))
        .arg(&source));
    // Preprocessing and parsing succeeded; a declaration-only unit lowers to an empty object.
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
}

#[test]
fn a_missing_include_says_which_one() {
    let temp = Temp::new("noinclude");
    let source = temp.write(
        "t.c",
        "#include \"nowhere.h\"
",
    );
    let output = run(xcc().args(["-c"]).arg(&source));
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("cannot find `nowhere.h`"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn defines_come_from_the_command_line() {
    let temp = Temp::new("define");
    let source = temp.write(
        "t.c",
        "#ifndef FLAG
#error no flag
#endif
",
    );
    let without = run(xcc().args(["-c"]).arg(&source));
    assert!(stderr(&without).contains("no flag"), "{}", stderr(&without));

    let with = run(xcc().args(["-c", "-DFLAG"]).arg(&source));
    assert!(
        !stderr(&with).contains("no flag"),
        "the define reached the preprocessor: {}",
        stderr(&with)
    );
}

#[test]
fn a_missing_response_file_says_so() {
    let temp = Temp::new("noresponse");
    let output = run(xcc().current_dir(temp.path()).arg("@nope.rsp"));
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("cannot read response file"));
}

#[test]
fn run_calls_main_and_leaves_with_its_value() {
    let temp = Temp::new("run");
    let source = temp.write("t.c", "int main(void) { return 42; }\n");
    let output = run(xcc().arg("--run").arg(&source));
    assert_eq!(output.status.code(), Some(42), "{}", stderr(&output));
}

#[test]
fn run_gives_main_its_arguments_and_reports_a_missing_one() {
    let temp = Temp::new("runargs");
    let source = temp.write(
        "t.c",
        "int main(int argc, char **argv) { return argc + (argv[0] != 0) + (argv[1] == 0); }\n",
    );
    // `argv[0]` is the source's path and `argv[1]` is the null that ends it: 1 + 1 + 1.
    let output = run(xcc().arg("--run").arg(&source));
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));

    // A source with no `main` is a usage error, not a program that exits 2 by accident.
    let nothing = temp.write("empty.c", "int f(void) { return 0; }\n");
    let output = run(xcc().arg("--run").arg(&nothing));
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("defines no `main`"),
        "{}",
        stderr(&output)
    );

    // The JIT compiles with the defaults, so a flag that would change them is refused rather than
    // accepted and quietly ignored.
    let output = run(xcc().args(["--run", "-DFOO"]).arg(&source));
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("--run takes no -D, -U or -I"),
        "{}",
        stderr(&output)
    );
}
