//! What the command line does, end to end: exit codes, streams and formats.
//!
//! **Windows only.** Half of it compiles a program, links it and runs it, and the image is a
//! Windows one. The diagnostics half would run anywhere; it is gated with the rest because a
//! file that is half-skipped is a file whose green means two different things.
#![cfg(target_os = "windows")]

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
    assert!(message.contains("is not written"), "{message}");
    assert!(!message.contains("internal"), "{message}");
    // And the levels that do exist compile: Cranelift's two, and this compiler's own back end.
    for level in ["0", "1", "2"] {
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
fn without_c_it_links_an_exe_that_runs() {
    let temp = Temp::new("link");
    let source = temp.write(
        "t.c",
        "#include <stdio.h>\nint main(void) { printf(\"linked by xcc\\n\"); return 5; }\n",
    );
    let exe = temp.path().join("t.exe");
    let output = run(xcc().arg(&source).arg("-o").arg(&exe));
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let ran = run(&mut Command::new(&exe));
    assert_eq!(ran.status.code(), Some(5));
    // The stream is a console in text mode, so the C library writes the platform's line
    // ending: the test is that the program linked and ran, not what it thinks a newline is.
    assert_eq!(stdout(&ran).trim_end(), "linked by xcc");
}

#[test]
fn two_sources_link_into_one_program() {
    // Two units, not one twice: the second unit's code and data land after the first's, and every
    // relocation inside it is patched from *its* place. A linker that puts a unit at offset zero
    // still links and still runs when there is one source, which is why this one has two.
    let temp = Temp::new("two");
    let a = temp.write(
        "a.c",
        "int twice(int);\nint main(void) { return twice(21); }\n",
    );
    let b = temp.write("b.c", "int twice(int x) { return x * 2; }\n");
    let exe = temp.path().join("two.exe");
    let output = run(xcc().arg(&a).arg(&b).arg("-o").arg(&exe));
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let ran = run(&mut Command::new(&exe));
    assert_eq!(ran.status.code(), Some(42));
}

#[test]
fn two_sources_keep_their_own_string_literals() {
    // A unit's literals are named `.str0`, `.str1`, ... and every unit numbers them from zero, so
    // the names collide across units while meaning different bytes. A linker that keys them by
    // name alone resolves the second unit's literals to the first unit's - which is how this was
    // found: a JSON library linked with a driver printed the driver's own format strings.
    let temp = Temp::new("literals");
    let a = temp.write(
        "a.c",
        r#"#include <stdio.h>
void b_literals(void);
int main(void) {
    printf("a=%s\n", "alpha");
    b_literals();
    return 0;
}
"#,
    );
    let b = temp.write(
        "b.c",
        r#"#include <stdio.h>
void b_literals(void) {
    printf("b=%s\n", "bravo");
}
"#,
    );
    let exe = temp.path().join("two.exe");
    let output = run(xcc().arg(&a).arg(&b).arg("-o").arg(&exe));
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let ran = run(&mut Command::new(&exe));
    assert_eq!(stdout(&ran).replace('\r', ""), "a=alpha\nb=bravo\n");
}

#[test]
fn two_sources_keep_their_own_statics() {
    // C99 6.2.2p3: a file-scope `static` has internal linkage, so two units may each define
    // `helper` and `counter` without naming the same thing. A linker that keys them by name alone
    // gives the second unit the first's code and bytes - measured: `a: 2 7` where clang printed
    // `a: 2 500`, and `b=11` where clang printed `b=500`.
    let temp = Temp::new("statics");
    let a = temp.write(
        "a.c",
        r#"#include <stdio.h>
static int helper(int x) { return x + 1; }
static int counter = 10;
int b_value(void);
int b_counter(void);
int main(void) {
    counter++;
    printf("a: %d %d %d\n", helper(1), b_value(), b_counter());
    return 0;
}
"#,
    );
    let b = temp.write(
        "b.c",
        r#"static int helper(int x) { return x * 100; }
static int counter = 500;
int b_value(void) { return helper(2) + helper(3); }
int b_counter(void) { return counter; }
"#,
    );
    let exe = temp.path().join("statics.exe");
    let output = run(xcc().arg(&a).arg(&b).arg("-o").arg(&exe));
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let ran = run(&mut Command::new(&exe));
    assert_eq!(stdout(&ran).replace('\r', ""), "a: 2 500 500\n");
}

#[test]
fn a_link_at_another_level_says_which_one_it_needs() {
    let temp = Temp::new("linklevel");
    let source = temp.write("t.c", "int main(void) { return 0; }\n");
    let output = run(xcc().arg(&source).args(["--opt-level=1"]));
    assert_eq!(output.status.code(), Some(2));
    let message = stderr(&output);
    assert!(message.contains("--opt-level=2"), "{message}");
    assert!(message.contains("pass -c"), "{message}");
}

#[test]
fn a_thread_local_object_needs_the_linker() {
    // Its place in a thread's block is decided once every unit is in one place, so an object file
    // on its own would carry a call with an offset nobody wrote. Both modes that cannot lay the
    // block out say why, and the link works.
    let temp = Temp::new("thread-local");
    let source = temp.write(
        "t.c",
        "_Thread_local int counter = 1;
int main(void) { return counter - 1; }
",
    );
    for mode in [["-c"].as_slice(), ["--run", "--opt-level=2"].as_slice()] {
        let output = run(xcc().arg(&source).args(mode).args(["--std=c11"]));
        let message = stderr(&output);
        // 1, not 2: a compile diagnostic is about the program, and 2 is for a command line this
        // compiler cannot make sense of.
        assert_eq!(output.status.code(), Some(1), "{message}");
        assert!(message.contains("_Thread_local"), "{message}");
        assert!(message.contains("link the program"), "{message}");
    }
    let linked = run(xcc()
        .arg(&source)
        .args(["--std=c11", "--opt-level=2"])
        .arg("-o")
        .arg(temp.write("t.exe", "")));
    assert!(linked.status.success(), "{}", stderr(&linked));
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
fn preprocess_to_a_file_honours_the_o_flag() {
    // `-E t.c -o t.i` wrote nothing and printed to stdout, so a build system saw a file that was
    // never made and a status that said it had worked.
    let temp = Temp::new("preprocess");
    let source = temp.write("t.c", "int  x ;\n");
    let out = temp.path().join("t.i");
    let output = run(xcc().arg("-E").arg(&source).arg("-o").arg(&out));
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(stdout(&output).is_empty(), "the text went to the file");
    let text = std::fs::read_to_string(&out).expect("the file is written");
    assert!(text.contains("int"), "{text}");
    assert!(!text.contains('#'), "directives are gone: {text}");

    // With no `-o` the text goes to stdout, which is what `-E` means on its own.
    let output = run(xcc().arg("-E").arg(&source));
    assert!(stdout(&output).contains("int"), "{}", stdout(&output));
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
fn a_depfile_names_the_files_a_build_system_can_rebuild() {
    // What `-MMD` writes: the object, a colon, and everything read that is a file. The headers this
    // compiler ships inside itself are not files, and naming one would send `make` looking for a
    // `stdio.h` that is not on any disk.
    let temp = Temp::new("depfile");
    let source = temp.write(
        "t.c",
        "#include <stdio.h>\n#include \"own.h\"\nint main(void) { return WIDTH; }\n",
    );
    temp.write("own.h", "#define WIDTH 0\n");
    let object = temp.path().join("t.obj");
    let depfile = temp.path().join("t.d");
    let output = run(xcc()
        .arg("-c")
        .arg(&source)
        .arg("-o")
        .arg(&object)
        .arg("--depfile")
        .arg(&depfile));
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let rule = std::fs::read_to_string(&depfile).expect("the depfile is written");
    assert!(
        rule.starts_with(&format!("{}:", object.display())),
        "{rule}"
    );
    assert!(rule.contains("t.c"), "{rule}");
    assert!(
        rule.contains("own.h"),
        "a host header is a dependency: {rule}"
    );
    assert!(!rule.contains("stdio.h"), "a shipped header is not: {rule}");
    assert!(rule.ends_with('\n'), "{rule}");

    // A depfile names the object a source became, so a mode that writes no object refuses it.
    let output = run(xcc()
        .arg("--run")
        .arg("--depfile")
        .arg(&depfile)
        .arg(&source));
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("--depfile names what a mode wrote"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn a_link_writes_the_rule_that_names_its_image() {
    // The depfile is not a `-c`-only idea: an image is a file a build system makes, and the units it
    // was made from are what would rebuild it.
    let temp = Temp::new("linkdep");
    let a = temp.write(
        "a.c",
        "int twice(int);\nint main(void) { return twice(21); }\n",
    );
    let b = temp.write("b.c", "int twice(int x) { return x * 2; }\n");
    let exe = temp.path().join("t.exe");
    let depfile = temp.path().join("t.d");
    let output = run(xcc()
        .arg(&a)
        .arg(&b)
        .arg("-o")
        .arg(&exe)
        .arg("--depfile")
        .arg(&depfile));
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let rule = std::fs::read_to_string(&depfile).expect("the depfile is written");
    assert!(rule.starts_with(&format!("{}:", exe.display())), "{rule}");
    assert!(rule.contains("a.c") && rule.contains("b.c"), "{rule}");
    assert!(
        !rule.contains("xcc-crt"),
        "the runtime is inside the binary: {rule}"
    );

    // One depfile cannot name two objects, so `-c` with two sources says so rather than writing the
    // last rule over the first.
    let output = run(xcc()
        .arg("-c")
        .arg(&a)
        .arg(&b)
        .arg("--depfile")
        .arg(&depfile));
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("--depfile names one object"),
        "{}",
        stderr(&output)
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
fn level_two_runs_what_only_it_compiles() {
    // `va_arg` and a floating-point argument to a variadic function are level 2's, and the JIT used to
    // be Cranelift-only: the two constructs this compiler is proudest of could be compiled to an
    // object and never run in process. They run now, through the same API a host uses.
    let temp = Temp::new("run2");
    let source = temp.write(
        "t.c",
        "#include <stdarg.h>
         int sum(int count, ...) { va_list ap; int total = 0; int i; va_start(ap, count);
             for (i = 0; i < count; i++) total += va_arg(ap, int); va_end(ap); return total; }
         double pick(int count, ...) { va_list ap; double t = 0.0; int i; va_start(ap, count);
             for (i = 0; i < count; i++) t = t + va_arg(ap, double); va_end(ap); return t; }
         int main(void) { return sum(4, 10, 20, 30, 2) + (int)pick(3, 1.5, 2.25, 3.25); }
",
    );
    let output = run(xcc().args(["--run", "--opt-level=2"]).arg(&source));
    // 62 from the integers, 7 from the doubles.
    assert_eq!(output.status.code(), Some(69), "{}", stderr(&output));
}

#[test]
fn run_reaches_the_runtimes_errno() {
    // `errno.h` names `_errno`, so `--run` must have it: the JIT resolves only what its symbol list
    // holds, and nothing compared that list with what the headers declare until `_errno` was found
    // missing from it. The program does not merely include the header - it reads the value the C
    // runtime's own `strtol` sets.
    let temp = Temp::new("runerrno");
    let source = temp.write(
        "t.c",
        "#include <errno.h>
         #include <stdlib.h>
         int main(void) { errno = 0; strtol(\"99999999999999999999\", 0, 10);
             return errno == ERANGE ? 42 : 1; }
",
    );
    let output = run(xcc().args(["--run", "--opt-level=2"]).arg(&source));
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

    // A define means what it means in a compile: the JIT takes the same options.
    let defined = temp.write(
        "defined.c",
        "int main(void) { return VALUE; }
",
    );
    let output = run(xcc().args(["--run", "-DVALUE=5"]).arg(&defined));
    assert_eq!(output.status.code(), Some(5), "{}", stderr(&output));

    // `-I` is the one that cannot travel: the JIT reads the headers inside this binary, and a
    // promise to read host files would be a promise it cannot keep.
    let output = run(xcc().args(["--run", "-I/tmp"]).arg(&source));
    assert_eq!(output.status.code(), Some(2));
    assert!(
        stderr(&output).contains("--run takes no -I"),
        "{}",
        stderr(&output)
    );
}
