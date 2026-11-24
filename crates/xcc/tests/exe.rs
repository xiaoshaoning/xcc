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
    std::fs::write(&object, artifact.bytes()).expect("the object is written");
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
    std::fs::write(&object, artifact.bytes()).expect("the object is written");
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
    // Level 2, because the floating-point half of a variadic call is the one construct Cranelift
    // cannot lower: it would write the vector register alone, and `printf` prints zeros.
    let artifact = Compile::new(Options {
        opt_level: 2,
        ..Options::default()
    })
    .compile("t.c", source)
    .expect("a variadic call with floating-point arguments compiles");
    let dir = temp_dir("printf");
    let object = dir.join("t.o");
    let exe = dir.join("t.exe");
    std::fs::write(&object, artifact.bytes()).expect("the object is written");
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
    std::fs::write(&object, artifact.bytes()).expect("the object is written");
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
    // Our caller against a callee *clang* compiled, reading a double through `va_arg`: the direction
    // that can see our half of the duplicate rule, because clang's callee spills its vector registers
    // according to `AL` and reads the general ones when we say none were used - which is where our
    // copies are. `pick(3, 1.5, 2.25, 3.25)` is 7.0, so the answer is 28.
    let caller_calling_variadic = r#"
        double pick(int count, ...);
        int main(void) { return (int)(pick(3, 1.5, 2.25, 3.25) * 4.0); }
    "#;
    // `g(1, 2.5, 3.5)` is 7.0, so the fixed-argument answer is 14.
    let caller_calling_fixed = r#"
        double g(int a, double b, double c);
        int main(void) { return (int)(g(1, 2.5, 3.5) * 2.0); }
    "#;
    let cases = [
        (
            "xcc callee, clang caller",
            variadic_callee,
            caller_calling_both,
            28,
        ),
        (
            "clang callee, xcc caller, fixed arguments",
            fixed_callee,
            caller_calling_fixed,
            14,
        ),
        (
            "xcc caller, clang callee, variadic",
            caller_calling_variadic,
            variadic_callee,
            28,
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

/// Link `source` with this compiler's own back end and linker, and run the image.
///
/// The temporary directory goes with the result: the output is already captured, so a test that
/// leaves its own binary behind is a slow leak and nothing more.
#[cfg(target_os = "windows")]
fn run_linked(test: &str, source: &str, args: &[&str]) -> std::process::Output {
    let mut compiler = Compile::new(Options {
        opt_level: 2,
        ..Options::default()
    });
    let image = compiler.link(&[("t.c", source)]).expect("it links");
    let dir = temp_dir(test);
    let exe = dir.join("t.exe");
    std::fs::write(&exe, image.bytes()).expect("the image is written");
    let out = Command::new(&exe)
        .args(args)
        .output()
        .expect("the program runs");
    let _ = std::fs::remove_dir_all(&dir);
    out
}

/// The whole pipeline in one test: this compiler's front end, its own back end, and its own linker,
/// with no host tool anywhere - the image is written by `Compile::link` and run by the operating
/// system, which is the only thing that can say whether the import table is bound.
#[test]
#[cfg(target_os = "windows")]
fn this_compilers_own_linker_writes_a_program_that_runs() {
    // A raw string, so the C source keeps its own quotes and its own \n.
    let source = r#"
#include <stdio.h>
#include <string.h>

int main(void) {
    char text[32];
    strcpy(text, "own linker");
    printf("%s %d\n", text, (int)strlen(text));
    return 11;
}
"#;
    let run = run_linked("ownlink", source, &[]);
    let code = run.status.code();
    let out = String::from_utf8_lossy(&run.stdout);
    assert_eq!(code, Some(11), "the exit status is main's return value");
    assert_eq!(out.trim_end(), "own linker 10");
}

/// `argc`/`argv` come from the runtime the link compiles in, so this is the test that a linked
/// program can be a command-line tool: a quoted argument stays one argument, and `main`'s return
/// value is the exit status.
#[test]
#[cfg(target_os = "windows")]
fn a_linked_program_reads_its_arguments() {
    let source = r#"
#include <stdio.h>

int main(int argc, char **argv) {
    for (int i = 1; i < argc; i++) printf("[%d] %s\n", i, argv[i]);
    return argc;
}
"#;
    let run = run_linked("argv", source, &["one", "two words", ""]);
    let code = run.status.code();
    let out = String::from_utf8_lossy(&run.stdout);
    assert_eq!(
        code,
        Some(4),
        "the exit status is argc, counting the program's own name"
    );
    // `argv[0]` is the path the program was started by, and this test only cares that it
    // exists: the lines compared are the arguments. Splitting on the newline byte and stripping
    // a trailing carriage return keeps the space the program printed after `[3]`, which a
    // `trim_end` would have eaten.
    let lines: Vec<&str> = out
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .filter(|line| !line.is_empty())
        .collect();
    assert_eq!(lines, ["[1] one", "[2] two words", "[3] "], "{out:?}");
}

/// The four classes of name that had no home in a link until the runtime and the rename table gave
/// them one: `sscanf` (the scanning front end), `time` (exported as `_time64`), `setjmp`/`longjmp`
/// (the library's intrinsic), and `atexit` (registered with `_crt_atexit`, and shown running by
/// printing after everything else).
#[test]
#[cfg(target_os = "windows")]
fn the_names_the_headers_declare_can_be_called_from_a_linked_program() {
    let source = r#"
#include <setjmp.h>
#include <stdio.h>
#include <stdlib.h>
#include <time.h>

static jmp_buf escape;

static void at_exit(void) { printf("at exit\n"); }

static void jump(int value) { longjmp(escape, value); }

int main(void) {
    int number = 0;
    char word[16] = {0};
    atexit(at_exit);
    int read = sscanf("42 words", "%d %15s", &number, word);
    printf("read=%d %d %s\n", read, number, word);
    time_t now = time(0);
    printf("time is a real time: %d\n", (int)(now > 1600000000));
    int back = setjmp(escape);
    if (back == 0) jump(9);
    printf("jumped with %d\n", back);
    return 3;
}
"#;
    let run = run_linked("lastnames", source, &[]);
    let code = run.status.code();
    let out = String::from_utf8_lossy(&run.stdout).into_owned();
    let lines: Vec<&str> = out
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .filter(|line| !line.is_empty())
        .collect();
    assert_eq!(
        code,
        Some(3),
        "main's return value, after the at-exit handler ran"
    );
    assert_eq!(
        lines,
        [
            "read=2 42 words",
            "time is a real time: 1",
            "jumped with 9",
            "at exit",
        ],
        "{out:?}"
    );
}

/// C99 7.19.6.8's own example: a program writes its own variadic function over `vfprintf` and
/// `vsnprintf`. Two layers of `...` in one program - the program's `va_list` forwarded into the
/// runtime's front end - so this fails if either the header or the runtime is missing the `v*`
/// forms, or if forwarding a `va_list` by argument does not survive the ABI.
#[test]
#[cfg(target_os = "windows")]
fn a_linked_program_can_write_its_own_variadic_function() {
    let source = r#"
#include <stdarg.h>
#include <stdio.h>

static void error(const char *format, ...) {
    va_list args;
    va_start(args, format);
    fprintf(stderr, "error: ");
    vfprintf(stderr, format, args);
    va_end(args);
}

static int shout(char *to, unsigned long long count, const char *format, ...) {
    va_list args;
    va_start(args, format);
    int written = vsnprintf(to, count, format, args);
    va_end(args);
    return written;
}

int main(void) {
    char text[64];
    int written = shout(text, sizeof text, "%s=%d", "answer", 42);
    printf("%s written=%d\n", text, written);
    error("%s\n", "to stderr");
    return 0;
}
"#;
    let run = run_linked("variadic", source, &[]);
    assert_eq!(run.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).trim_end(),
        "answer=42 written=9"
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stderr).trim_end(),
        "error: to stderr"
    );
}

#[test]
#[cfg(target_os = "windows")]
fn a_state_machine_whose_cases_live_inside_do_while_blocks_runs() {
    // miniz's inflater is written this way: one `switch` over the machine's state, with each `case`
    // inside a `do { } while (0)` macro so a `goto` can re-enter it. A local's `Slot` used to be
    // emitted where the declaration stood, which put it in one case's block and read it in another -
    // invalid SSA, and the verifier refused the function. The frame slot now belongs to the frame.
    let source = r#"
#include <stdio.h>

static int run(int state, int steps) {
    int total = 0;
    switch (state) {
    case 0:
        do {
            total += 1;
            if (total > steps) { goto done; }
            do { case 1: ; } while (0);
            total += 10;
        } while (0);
        break;
    case 2:
        do { case 3: ; } while (0);
        total += 100;
        break;
    }
done:
    return total;
}

int main(void) {
    printf("%d %d %d\n", run(0, 5), run(1, 0), run(3, 0));
    return 0;
}
"#;
    let run = run_linked("statemachine", source, &[]);
    assert_eq!(run.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim_end(), "11 10 100");
}

#[test]
#[cfg(target_os = "windows")]
fn a_conditional_whose_arm_is_a_locals_address_survives_promotion() {
    // `int *p = c ? &x : &y;` threads those addresses through the join as block arguments. The
    // promotion pass deletes the `Slot` instructions of locals it can prove never escape, and its
    // escape test walked instructions only - so the block argument kept naming a value nothing
    // defined, and the verifier called the function invalid. miniz's zip writer is written that way.
    let source = r#"
#include <stdio.h>

static int f(int c) {
    int x = 7;
    int *p = c ? &x : 0;
    return p ? *p : 0;
}

static int g(int c) {
    int a = 1, b = 2;
    int *p = c ? &a : &b;
    *p += 1;
    return a + b;
}

int main(void) {
    printf("%d %d %d\n", f(1), g(0), g(1));
    return 0;
}
"#;
    let run = run_linked("addressjoin", source, &[]);
    assert_eq!(run.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim_end(), "7 4 4");
}

#[test]
#[cfg(target_os = "windows")]
fn wide_literals_are_wchar_ts_and_utf16() {
    // C99 6.4.4.4p10 and 6.4.5p5: `L'a'` has type `wchar_t` and `L"ab"` is an array of them, NUL
    // included. Before this, a wide literal was emitted as its *narrow* bytes and `L'a'` was an
    // `int` - a program that read `wchar_t *s = L"ab"` got 25185 and then two bytes past the end.
    let source = r#"
#include <stddef.h>
#include <stdio.h>

int main(void) {
    const wchar_t *s = L"ab";
    const wchar_t *e = L"caf\xe9";
    const wchar_t *u = L"é";
    printf("%d %d %d %d\n", (int)sizeof(L"ab"), (int)sizeof(L"a"), (int)sizeof(wchar_t),
           (int)sizeof(L'a'));
    printf("%d %d %d\n", (int)s[0], (int)s[1], (int)s[2]);
    printf("%d %d %d %d\n", (int)e[0], (int)e[1], (int)e[2], (int)e[3]);
    printf("%d %d\n", (int)u[0], (int)sizeof(L"é"));
    return 0;
}
"#;
    let run = run_linked("wide", source, &[]);
    assert_eq!(run.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).replace('\r', ""),
        "6 4 2 2\n97 98 0\n99 97 102 233\n233 4\n"
    );
}

#[test]
#[cfg(target_os = "windows")]
fn the_floating_point_environment_round_trips() {
    // C99 7.6: the rounding direction is set and read back, the status flags are cleared, raised
    // and held, and `FE_DFL_ENV` restores the environment the program started in. The last line is
    // where a mingw build differs: its `FE_DFL_ENV` is an object its own library fills, and
    // restoring it left the mode at `FE_TOWARDZERO`. Ours is captured by `fegetenv` before `main`,
    // so "the default" means what 7.6p6 says it means.
    let source = r#"
#include <fenv.h>
#include <stdio.h>

int main(void) {
    int start = fegetround();
    fesetround(FE_UPWARD);
    int upward = fegetround();
    fesetround(FE_TOWARDZERO);
    int toward = fegetround();
    int restored = fesetenv(FE_DFL_ENV);
    int after = fegetround();
    feclearexcept(FE_ALL_EXCEPT);
    int cleared = fetestexcept(FE_ALL_EXCEPT);
    feraiseexcept(FE_DIVBYZERO);
    int raised = fetestexcept(FE_ALL_EXCEPT);
    fenv_t saved;
    feholdexcept(&saved);
    int held = fetestexcept(FE_ALL_EXCEPT);
    feupdateenv(&saved);
    int updated = fetestexcept(FE_ALL_EXCEPT);
    printf("%d %#x %#x %d %d\n", start, upward, toward, restored, after);
    printf("%#x %#x %#x %#x\n", cleared, raised, held, updated);
    return 0;
}
"#;
    let run = run_linked("fenv", source, &[]);
    assert_eq!(run.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).replace('\r', ""),
        "0 0x200 0x300 0 0\n0 0x8 0 0x8\n"
    );
}

#[test]
#[cfg(target_os = "windows")]
fn wide_character_classes_and_case_mapping() {
    // C99 7.25: eighteen functions the runtime exports under their own names, over the target's
    // two-byte `wint_t`. The class and mapping *names* are the interesting half: `wctype("alpha")`
    // and `wctrans("toupper")` are the runtime's tables, so this fails if `wctype_t` or `wctrans_t`
    // is the wrong width.
    let source = r#"
#include <stdio.h>
#include <wctype.h>

int main(void) {
    printf("%d %d %d %d\n", iswalpha(L'A') != 0, iswdigit(L'7') != 0, (int)towlower(L'A'),
           iswspace(L' ') != 0);
    printf("%d %d %d %d\n", iswctype(L'a', wctype("alpha")) != 0,
           (int)towctrans(L'a', wctrans("toupper")), (int)sizeof(wint_t), WEOF);
    return 0;
}
"#;
    let run = run_linked("wctype", source, &[]);
    assert_eq!(run.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).replace('\r', ""),
        "1 1 97 1\n1 65 2 65535\n"
    );
}
