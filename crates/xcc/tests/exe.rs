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
    run_linked_with("", test, source, args)
}

/// The same, with a leading define that names the dialect: `c11` compiles at `--std=c11`.
fn run_linked_with(dialect: &str, test: &str, source: &str, args: &[&str]) -> std::process::Output {
    run_units(dialect, test, &[("t.c", source)], args)
}

/// The same for a program of several translation units: the linker is where a name defined twice is
/// caught, and one unit cannot show that.
fn run_units(
    dialect: &str,
    test: &str,
    units: &[(&str, &str)],
    args: &[&str],
) -> std::process::Output {
    let standard = if dialect == "c11" {
        xcc::CStandard::C11
    } else {
        xcc::CStandard::default()
    };
    let mut compiler = Compile::new(Options {
        opt_level: 2,
        standard,
        ..Options::default()
    });
    let image = compiler.link(units).expect("it links");
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

/// `__readgsqword` and its seven siblings: `mov rax, gs:[offset]`, which is how a Windows program
/// reaches the thread environment block. `gs:[0x30]` is the TEB's self pointer and `gs:[0x48]` is
/// `ClientId.UniqueThread`, so the oracle is the runtime's own answer: `GetCurrentThreadId`.
///
/// The offset is an *address*, and an address is a value here, so `gs:[rcx]` is the same encoding as
/// `gs:[0x30]` - which is what `mimalloc` needs, because its slot is `slot * sizeof(void*)`.
#[test]
#[cfg(target_os = "windows")]
fn the_thread_environment_block_is_one_instruction_away() {
    let source = r#"
#include <stdio.h>

unsigned long GetCurrentThreadId(void);

int main(void) {
    unsigned long long teb = __readgsqword(0x30);
    unsigned long long at_0x48 = __readgsqword(0x48);
    unsigned int dword = __readgsdword(0x48);
    unsigned long long ofs = 0x48;
    unsigned long long computed = __readgsqword(ofs);
    unsigned int expected = GetCurrentThreadId();
    /* The offset in a register that is live across the read, with a value accumulating while it is:
       an operand the allocator does not know about ends its live range at the read, and the
       destination takes its register - a read from wherever the destination landed. */
    unsigned long long total = 0;
    unsigned long long offset = 0x40;
    for (int i = 0; i < 4; i++) {
        total += __readgsqword(offset);
        offset += 8;
    }
    printf("%d %d %d %d %d\n", teb != 0, (unsigned long)at_0x48 == expected,
           dword == expected, (unsigned long)computed == expected, total != 0);
    return 0;
}
"#;
    let run = run_linked("teb", source, &[]);
    let out = String::from_utf8_lossy(&run.stdout);
    assert_eq!(run.status.code(), Some(0), "{out}");
    assert_eq!(out.trim_end(), "1 1 1 1 1");
}

/// Two things C99 has that mimalloc needs and this compiler did not answer: `__func__` (6.4.2.2, the
/// enclosing function's name as if a `static const char[]` had been declared) and `__thread`, which
/// is GCC's and clang's spelling of `_Thread_local` - and the one a header library uses in the branch
/// for "some other compiler", which is this one.
#[test]
#[cfg(target_os = "windows")]
fn the_name_of_the_function_and_the_other_spelling_of_thread_local() {
    let source = r#"
#include <stdio.h>
#include <string.h>

__thread int slot = 7;

static const char *where(void) { return __func__; }

int main(void) {
    slot = slot + 1;
    printf("%s %s %zu %d %d\n", __func__, where(), strlen(__func__),
           (int)sizeof(__func__), slot);
    return 0;
}
"#;
    let run = run_linked("funcname", source, &[]);
    let out = String::from_utf8_lossy(&run.stdout);
    assert_eq!(run.status.code(), Some(0), "{out}");
    // `main` is four characters, `where` is five, and `sizeof` of the array counts its terminator.
    assert_eq!(out.trim_end(), "main where 4 5 8");

    // C99 6.4.2.2p1 declares it `static const char[]`, so a program cannot write to it - and this
    // compiled and *wrote* until this test existed, which is the wrong kind of silence.
    let refused = Compile::new(Options {
        opt_level: 2,
        ..Options::default()
    })
    .compile("t.c", "int main(void) { __func__[0] = 'x'; return 0; }");
    assert!(refused.is_err(), "writing to `__func__` must be refused");
}

/// A pointer narrowed to an integer: C99 6.3.2.3p6 makes the result implementation-defined, and the
/// machine's answer is the low bits - the same truncation an integer of that width gives. `tre` hashes
/// a pointer with `(int)p`, and this compiler refused the cast.
#[test]
#[cfg(target_os = "windows")]
fn a_pointer_narrows_to_an_integer_the_way_the_machine_does() {
    let source = r#"
#include <stdio.h>

int main(void) {
    char *p = (char *)0x7f3a5c10;
    printf("%x %x %x %llx %d %d\n", (unsigned)p, (unsigned short)(unsigned)p,
           (unsigned char)(unsigned)p, (unsigned long long)p, (int)(p != 0),
           (int)(short)(unsigned)p);
    return 0;
}
"#;
    let run = run_linked("ptrtoint", source, &[]);
    let out = String::from_utf8_lossy(&run.stdout);
    assert_eq!(run.status.code(), Some(0), "{out}");
    // What a `clang -O2` build of the same program prints: the low 32, 16 and 8 bits, the full
    // 64, a `_Bool` that is still a truth test rather than a truncation, and the sign of a `short`.
    assert_eq!(out.trim_end(), "7f3a5c10 5c10 10 7f3a5c10 1 23568");
}

/// A `_Bool` that came from a comparison, and then converted: the shape `yyjson` wrote as
/// `(u8)(sign)` where `sign = (*cur == '-')`.
///
/// The bug was in a pass, not in the front end. `truth` removes `cmp ne v, 0` when it knows `v` is
/// already 0 or 1 - and a comparison's answer is `int` (C99 6.5.9p3) or `_Bool`, so substituting one
/// for the other changed a value's *type*, and the `convert _Bool -> unsigned char` that read it had
/// an operand of another type. The verifier said so; the pass now asks first.
#[test]
#[cfg(target_os = "windows")]
fn a_boolean_from_a_comparison_survives_a_conversion() {
    let source = r#"
#include <stdio.h>

typedef unsigned char u8;
typedef unsigned long long u64;

int main(void) {
    _Bool sign;
    sign = (5 > 3);
    u8 byte = (u8)sign;
    /* the same through a field of a wider expression, which is how the fixture uses it */
    u64 tag = ((u8)4) | (u64)((u8)sign << 3);
    printf("%d %llu %d\n", (int)byte, tag, (int)(u8)(1 > 0));
    return 0;
}
"#;
    let run = run_linked("boolconv", source, &[]);
    // `trim_end` takes the platform's line ending with it, so no escaping is needed here.
    let out = String::from_utf8_lossy(&run.stdout);
    assert_eq!(run.status.code(), Some(0), "{out}");
    assert_eq!(out.trim_end(), "1 12 1");
}

/// Two declarations that are not about this program's names: `__asm__("...")` after a declarator is
/// the name to *link* by, and `inline` on a definition decides linkage (C99 6.7.4p6).
///
/// Both come from the Windows SDK, which is why they are here: mingw-w64 renames every CRT function
/// whose wide and narrow spellings are one name here (`vsscanf` is `__mingw_vsscanf`) through
/// `__asm__`, and defines every `FORCEINLINE` helper in `winnt.h` as an *inline definition* - which
/// provides no external definition, so emitting one per unit made two units that include
/// `windows.h` a duplicate at link time. Lua 5.5.1 hit both, in that order.
#[test]
#[cfg(target_os = "windows")]
fn a_declaration_can_rename_what_it_links_to_and_an_inline_definition_is_private() {
    // `puts_` is not a function any library has: if the rename were dropped, this would not link.
    let source = r#"
#include <stdio.h>

int emit(const char *text) __asm__("puts");

int main(void) {
    emit("renamed");
    return 0;
}
"#;
    let run = run_linked("asmlabel", source, &[]);
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim_end(), "renamed");
    assert_eq!(run.status.code(), Some(0));

    // One header, two units, one `inline` definition. It provides no external definition, so the
    // program links; before, both units offered one and the linker said so.
    let dir = temp_dir("inlinelink");
    std::fs::write(
        dir.join("h.h"),
        "inline int twice(int x) { return x + x; }
static inline int thrice(int x) { return x + twice(x); }
",
    )
    .expect("the header is written");
    let one = "#include \"h.h\"
int one(int x) { return twice(x); }
";
    let two = "#include \"h.h\"
int one(int x);
int main(void) { return one(20) + thrice(0); }
";
    let image = Compile::with_includes(
        Options {
            opt_level: 2,
            ..Options::default()
        },
        Box::new(FromDir(dir.clone())),
    )
    .link(&[("one.c", one), ("two.c", two)])
    .unwrap_or_else(|diagnostics| {
        panic!(
            "two units with one inline definition do not link: {}",
            diagnostics.render(xcc::Format::Human)
        )
    });
    let exe = dir.join("t.exe");
    std::fs::write(&exe, image.bytes()).expect("the image is written");
    let run = Command::new(&exe).output().expect("the program runs");
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(run.status.code(), Some(40), "one(20) is twice(20)");
}

/// `#include "..."` from one directory, for the test above: the header it declares is one file this
/// test wrote a moment earlier.
#[derive(Debug)]
struct FromDir(std::path::PathBuf);

impl xcc::Includes for FromDir {
    fn read(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(self.0.join(path)).ok()
    }
}

/// Three initialisers that a static object can have and this compiler once wrote as zeroes: an
/// integer constant cast to a pointer (C99 6.6p9 - sqlite's `SQLITE_INT_TO_PTR`), a string literal
/// cast to a pointer (`(u8*)" "`, sqlite's one-character set), and the same literal reached through
/// a `static` local's table. Each is a *value*, not a computation, so each is bytes in the image.
#[test]
#[cfg(target_os = "windows")]
fn the_initialisers_that_are_values_stay_values() {
    let source = r#"
#include <stdio.h>

#define INT_TO_PTR(x) ((void*)(long)(x))

struct FuncDef { short nArg; unsigned flags; void *pUserData; };
static struct FuncDef table[] = {
    { 1, 0x1234, INT_TO_PTR(7) },
    { 2, 0,       INT_TO_PTR(1 | 2) },
};

int count(void) {
    static const char * const names[] = { (char*)"one", (char*)"two" };
    return (int)names[1][2];
}

int main(void) {
    printf("%d %d %d %d\n", table[0].nArg, table[0].pUserData == (void*)7,
           table[1].pUserData == (void*)3, count());
    return 0;
}
"#;
    let run = run_linked("values", source, &[]);
    let out = String::from_utf8_lossy(&run.stdout);
    assert_eq!(
        out.trim_end(),
        "1 1 1 111",
        "the table holds its numbers and its addresses"
    );
    assert_eq!(run.status.code(), Some(0));
}

/// The image names its own functions. A COFF symbol table sits after the last section's bytes -
/// nothing maps it and the loader never reads it - and `objdump -t` and `gdb` do: it is the
/// difference between "the program faulted at `.text+0x129e2d`" and "the program faulted in
/// `sqlite3InsertBuiltinFuncs`", which is how the fault inside `sqlite3_open` was finally named.
///
/// Local names count. Almost every function in `sqlite3.c` is `static`, so a table of the external
/// ones alone would name almost nothing in the program this exists for.
#[test]
#[cfg(target_os = "windows")]
fn the_linked_image_names_its_own_functions() {
    let source = r#"
static int a_long_static_name(int x) { return x + 1; }
static int helper(int x) { return a_long_static_name(x) * 2; }
long a_global_object = 3;
int main(void) { return helper(a_global_object); }
"#;
    let mut compiler = Compile::new(Options {
        opt_level: 2,
        ..Options::default()
    });
    let image = compiler.link(&[("t.c", source)]).expect("it links");
    let bytes = image.bytes();
    let word = |offset: usize| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
    // The COFF header: `Machine`, `NumberOfSections`, `TimeDateStamp`, then the two fields this is
    // about - where the table is in the file, and how many 18-byte records it has.
    let header = word(0x3C) as usize + 4;
    let table = word(header + 8) as usize;
    let count = word(header + 12) as usize;
    assert!(
        count > 3,
        "the table has {count} symbols, which is not a program"
    );
    assert!(table < bytes.len(), "the table starts inside the file");
    assert_eq!(
        table % 0x200,
        0,
        "it starts where the last section's bytes end"
    );
    // Eighteen bytes a record, then a string table whose first word is its own length: a name of
    // more than eight bytes lives there and the record holds its offset instead.
    let strings = table + count * 18;
    let mut symbols = Vec::new();
    for index in 0..count {
        let record = table + index * 18;
        let raw = &bytes[record..record + 8];
        let offset = u32::from_le_bytes(raw[4..8].try_into().unwrap()) as usize;
        let inline = raw.iter().position(|byte| *byte == 0).unwrap_or(8);
        let name = if raw[..4] == [0, 0, 0, 0] && offset != 0 {
            let start = strings + offset;
            let length = bytes[start..].iter().position(|byte| *byte == 0).unwrap();
            String::from_utf8_lossy(&bytes[start..start + length]).into_owned()
        } else {
            String::from_utf8_lossy(&raw[..inline]).into_owned()
        };
        symbols.push((
            name,
            word(record + 8),
            u16::from_le_bytes(bytes[record + 12..record + 14].try_into().unwrap()),
            u16::from_le_bytes(bytes[record + 14..record + 16].try_into().unwrap()),
            bytes[record + 16],
        ));
    }
    let find = |name: &str| {
        symbols
            .iter()
            .find(|symbol| symbol.0 == name)
            .unwrap_or_else(|| {
                panic!("`{name}` is not in the symbol table: {symbols:?}");
            })
    };
    // `.text` is section 1, a function has type 0x20, and `main` is an external definition. The
    // record is (name, value, section, type, storage class), so the value is where it starts.
    let main = find("main");
    assert_eq!((main.2, main.3, main.4), (1, 0x20, 2));
    assert!(main.1 > 0, "`main` is not at `.text`'s zero");
    // A `static` keeps its name and says so: storage class 3, and a name longer than eight bytes,
    // which is the string table working rather than a truncated record.
    let helper = find("a_long_static_name");
    assert_eq!((helper.2, helper.3, helper.4), (1, 0x20, 3));
    assert!(helper.1 > 0, "a static function is inside `.text`");
    // An object is in `.data` (section 2), is not a function, and is external.
    let object = find("a_global_object");
    assert_eq!((object.2, object.3, object.4), (2, 0, 2));
    // The runtime is compiled into every link, so a name from it is here too - and it is longer
    // than eight bytes, which is the second case the string table exists for.
    assert!(symbols
        .iter()
        .any(|symbol| symbol.0 == "_InterlockedCompareExchange"));
    // In address order, so a reader can bisect and a name's end is the next name's start.
    let text: Vec<u32> = symbols
        .iter()
        .filter(|symbol| symbol.2 == 1)
        .map(|symbol| symbol.1)
        .collect();
    assert!(
        text.windows(2).all(|pair| pair[0] <= pair[1]),
        "sorted by address"
    );
}

/// `argc`/`argv` come from the runtime the link compiles in, so this is the test that a linked
/// program can be a command-line tool: a quoted argument stays one argument, and `main`'s return
/// value is the exit status.
/// Windows commits a thread's stack one page at a time, at the single guard page below what is
/// committed. A frame taken with one `sub rsp` steps over that page, and the first write past it is
/// an access violation: `char a[65536]` in one function crashed this compiler, and so did lz4's
/// streaming decoder, which keeps sixty-four kilobytes of dictionary on the stack. The back end now
/// walks down a page at a time, touching each one, which this test holds: every one of these frames
/// was an access violation before, and the deep recursion takes a page-sized frame five hundred
/// times over. The largest takes seventy-three pages, which is a statement about the walk and
/// not about how much stack the image asks for.
/// The program the transcendental test below compiles, as a raw Rust string.
const TRANSCENDENTAL_PROGRAM: &str = r#"
#include <stdio.h>
#include <math.h>
#include <complex.h>

/* `+ 0.0` turns a negative zero into a positive one: the sign of a zero is not a value, and two
 * implementations that agree on every number can disagree about it. */
static void report(const char *name, double _Complex z, double _Complex w) {
    printf("%-5s %+.6f %+.6f -> %+.9f %+.9f\n", name, creal(z) + 0.0, cimag(z) + 0.0,
           creal(w) + 0.0, cimag(w) + 0.0);
}

int main(void) {
    static const double _Complex points[] = {
        { 1.5, -0.75 }, { -2.0, 0.5 }, { 0.25, 2.0 }, { 3.0, 1.0 }, { -0.5, -1.5 }, { 0.5, 0.5 },
    };
    int i;
    for (i = 0; i < (int)(sizeof points / sizeof points[0]); i++) {
        double _Complex z = points[i];
        report("exp", z, cexp(z));
        report("log", z, clog(z));
        report("sqrt", z, csqrt(z));
        report("sin", z, csin(z));
        report("cos", z, ccos(z));
        report("tan", z, ctan(z));
        report("sinh", z, csinh(z));
        report("cosh", z, ccosh(z));
        report("tanh", z, ctanh(z));
        report("asin", z, casin(z));
        report("acos", z, cacos(z));
        report("atan", z, catan(z));
        report("asinh", z, casinh(z));
        report("acosh", z, cacosh(z));
        report("atanh", z, catanh(z));
        report("pow", z, cpow(z, (double _Complex){ 1.5, 0.5 }));
    }
    /* The points where a formula divides by zero or crosses a cut, which is where the first
     * version of these was wrong. `cpow(0, -2)` is deliberately absent: Annex G says the result is
     * infinite, this compiler returns `inf 0` there, and `clang`'s own imaginary part is a NaN -
     * a disagreement about the *second* half of an answer that is infinite either way. */
    {
        static const double _Complex edges[] = {
            { 0.0, 0.0 }, { -4.0, 0.0 }, { 2.0, 0.0 }, { -1.0, 0.0 }, { 1.0, 0.0 }, { 0.0, 1.0 },
            { 0.0, -1.0 },
        };
        for (i = 0; i < (int)(sizeof edges / sizeof edges[0]); i++) {
            double _Complex z = edges[i];
            report("edge", z, csqrt(z));
            report("easin", z, casin(z));
            report("eacos", z, cacos(z));
            report("eatan", z, catan(z));
            report("esinh", z, casinh(z));
            report("eacsh", z, cacosh(z));
            report("eatnh", z, catanh(z));
            report("etanh", z, ctanh(z));
            report("expw", z, cpow(z, (double _Complex){ 3.0, 0.0 }));
        }
        /* `ctanh` of an infinity, where this compiler and `clang` agree. A large *finite* number
         * is where they do not: the limit is 1, this compiler writes it, and `clang` NaNs there -
         * so the finite case stays out of a comparison whose oracle is `clang`. */
        report("sat", (double _Complex){ INFINITY, 0.0 }, ctanh((double _Complex){ INFINITY, 0.0 }));
    }
    /* The four identities that say the branches are the ones C99 Annex G describes. */
    {
        double _Complex z = { 1.5, -0.75 };
        report("sin^2", z, csin(z) * csin(z) + ccos(z) * ccos(z));
        report("expog", z, cexp(clog(z)) - z);
        report("sqrtsq", z, csqrt(z) * csqrt(z) - z);
        report("asinsin", z, csin(casin(z)) - z);
    }
    /* The `float` forms compute in `double` and narrow once - like the four `math.h` wrappers -
     * so their last bit can differ from the runtime's own float routines; four decimals is
     * coarser than that difference and finer than any mistake. */
    {
        float _Complex f = { 0.75F, -1.25F };
        printf("float %.4f %.4f | %.4f %.4f | %.4f %.4f | %.4f %.4f\n",
               (double)crealf(cexpf(f)), (double)cimagf(cexpf(f)),
               (double)crealf(csqrtf(f)), (double)cimagf(csqrtf(f)),
               (double)crealf(csinf(f)), (double)cimagf(csinf(f)),
               (double)crealf(catanhf(f)), (double)cimagf(catanhf(f)));
    }
    return 0;
}
"#;

/// The program the uchar test below compiles, as a raw Rust string.
const UCHAR_PROGRAM: &str = r#"
#include <stdio.h>
#include <uchar.h>
#include <uchar.h>

/* The two character types and the four conversions the header adds, and the three literal families
 * the compiler adds: `u` is UTF-16 code units, `U` is code points, `u8` is UTF-8 bytes. */
int main(void) {
    const char *narrow = u8"héllo";
    const unsigned short *sixteen = u"héllo";
    const unsigned int *thirty_two = U"héllo";
    char16_t sixteen_from_mb = 0;
    char32_t thirty_two_from_mb = 0;
    mbstate_t state16 = { 0 };
    mbstate_t state32 = { 0 };
    char out[8];
    size_t count;

    printf("sizes %d %d %d %d %d\n", (int)sizeof(u8"héllo"), (int)sizeof(u"héllo"),
           (int)sizeof(U"héllo"), (int)sizeof(char16_t), (int)sizeof(char32_t));
    printf("firsts %d %d %d %d %d %d\n", (int)narrow[0], (int)sixteen[0], (int)thirty_two[0],
           (int)narrow[1], (int)sixteen[1], (int)thirty_two[1]);
    printf("chars %d %d %d %d %d\n", (int)sizeof(u'é'), (int)u'é', (int)sizeof(U'é'), (int)U'é',
           (int)sizeof(u'x'));
    count = mbrtoc16(&sixteen_from_mb, "abc", 1, &state16);
    printf("mbrtoc16 %d %d\n", (int)sixteen_from_mb, (int)count);
    count = mbrtoc32(&thirty_two_from_mb, "abc", 1, &state32);
    printf("mbrtoc32 %d %d\n", (int)thirty_two_from_mb, (int)count);
    count = c16rtomb(out, (char16_t)'Z', &state16);
    printf("c16rtomb %d %c\n", (int)count, out[0]);
    count = c32rtomb(out, (char32_t)'Q', &state32);
    printf("c32rtomb %d %c\n", (int)count, out[0]);
    return 0;
}
"#;

/// The program the threads test below compiles, as a raw Rust string.
const THREADS_PROGRAM: &str = r#"
#include <stdio.h>
#include <threads.h>

/* A producer and a consumer, which is the standard's own use of a mutex and a condition variable,
 * plus the three other things `threads.h` provides: thread-specific storage, one-time
 * initialisation, and a thread whose return value is the join's result. */
static mtx_t lock;
static cnd_t ready;
static int produced;
static tss_t slot;
static once_flag once = ONCE_FLAG_INIT;
static int initialised;

static void initialise(void) { initialised = initialised + 1; }

static int consumer(void *argument) {
    int seen;
    (void)argument;
    call_once(&once, initialise);
    tss_set(slot, (void *)0x1234);
    mtx_lock(&lock);
    while (produced == 0) {
        cnd_wait(&ready, &lock);
    }
    seen = produced;
    mtx_unlock(&lock);
    printf("consumer saw %d, tss %d, once %d\n", seen, tss_get(slot) == (void *)0x1234, initialised);
    return seen + 1;
}

int main(void) {
    thrd_t thread;
    int result = 0;
    if (mtx_init(&lock, mtx_plain) != thrd_success || cnd_init(&ready) != thrd_success ||
        tss_create(&slot, 0) != thrd_success) {
        printf("setup failed\n");
        return 1;
    }
    call_once(&once, initialise);
    if (thrd_create(&thread, consumer, 0) != thrd_success) {
        printf("thrd_create failed\n");
        return 1;
    }
    thrd_yield();
    mtx_lock(&lock);
    produced = 42;
    cnd_signal(&ready);
    mtx_unlock(&lock);
    if (thrd_join(thread, &result) != thrd_success) {
        printf("thrd_join failed\n");
        return 1;
    }
    printf("joined with %d, once ran %d time(s)\n", result, initialised);
    printf("equal %d, timed mutex %d\n", thrd_equal(thrd_current(), thrd_current()),
           mtx_init(&lock, mtx_timed));
    tss_delete(slot);
    mtx_destroy(&lock);
    cnd_destroy(&ready);
    return 0;
}
"#;

/// The program the narrow-atomic test below compiles, as a raw Rust string.
const NARROW_ATOMIC_PROGRAM: &str = r#"
#include <stdio.h>
#include <stdatomic.h>

/* The narrow widths: `lock cmpxchg` has a form for one byte and for two, so an `_Atomic char` and
 * an `_Atomic short` are real atomics and not a wider word pretending. The signed ones matter
 * because `cmpxchg` writes only the low byte or word of `rax` - a result that is not extended back
 * to its type reads as a large positive number where the program sees a negative one. */
int main(void) {
    atomic_char small = 10;
    atomic_short medium = 100;
    char expected = 15;
    short wanted = 0;

    printf("char %d -> %d\n", (int)atomic_fetch_add(&small, 5), (int)atomic_load(&small));
    printf("short %d -> %d\n", (int)atomic_fetch_add(&medium, 20), (int)atomic_load(&medium));
    printf("char cas %d -> %d\n", atomic_compare_exchange_strong(&small, &expected, 99),
           (int)atomic_load(&small));
    printf("short cas fail %d -> %d\n", atomic_compare_exchange_strong(&medium, &wanted, 1),
           (int)wanted);

    {
        atomic_char negative = -5;
        atomic_short bigger = -300;
        char seen = -5;
        short other = 0;
        printf("signed cas %d -> %d\n", atomic_compare_exchange_strong(&negative, &seen, 7),
               (int)atomic_load(&negative));
        printf("signed fail %d -> %d\n", atomic_compare_exchange_strong(&bigger, &other, 1),
               (int)other);
        printf("signed add %d -> %d\n", (int)atomic_fetch_add(&bigger, 300), (int)atomic_load(&bigger));
    }
    return 0;
}
"#;

/// The program the atomic test below compiles, as a raw Rust string.
const ATOMIC_PROGRAM: &str = r#"
#include <stdio.h>
#include <stdatomic.h>

unsigned long long _beginthreadex(void *security, unsigned stack, unsigned (*start)(void *),
                                  void *argument, unsigned flags, unsigned *id);
unsigned long WaitForSingleObject(void *handle, unsigned long milliseconds);
int CloseHandle(void *handle);

#define ROUNDS 100000

static atomic_int counter;
static atomic_ullong wide;
static atomic_flag lock = ATOMIC_FLAG_INIT;
static int guarded;

static unsigned worker(void *argument) {
    int i;
    (void)argument;
    for (i = 0; i < ROUNDS; i++) {
        atomic_fetch_add(&counter, 1);
        atomic_fetch_add(&wide, 1);
        /* A spinlock over the same operations, which is the standard's own use of a flag. */
        while (atomic_flag_test_and_set(&lock)) {
        }
        guarded = guarded + 1;
        atomic_flag_clear(&lock);
    }
    return 0;
}

int main(void) {
    void *threads[2] = { 0, 0 };
    int i;
    for (i = 0; i < 2; i++) {
        threads[i] = (void *)_beginthreadex(0, 0, worker, 0, 0, 0);
    }
    for (i = 0; i < 2; i++) {
        WaitForSingleObject(threads[i], 0xFFFFFFFFul);
        CloseHandle(threads[i]);
    }
    /* Three exact numbers, and each of them loses some to a non-atomic implementation. */
    printf("counters %d %d %d, expected %d\n", atomic_load(&counter), (int)atomic_load(&wide),
           guarded, 2 * ROUNDS);
    return 0;
}
"#;

/// The program the thread-local test below compiles, as a raw Rust string.
const THREAD_LOCAL_PROGRAM: &str = r#"
#include <stdio.h>

/* The two kernel calls a thread is waited on with, and the C runtime's own thread entry point:
 * this compiler ships no `windows.h`, and `ucrtbase.dll` exports all three. */
unsigned long long _beginthreadex(void *security, unsigned stack, unsigned (*start)(void *),
                                  void *argument, unsigned flags, unsigned *id);
unsigned long WaitForSingleObject(void *handle, unsigned long milliseconds);
int CloseHandle(void *handle);

_Thread_local int counter = 100;
static _Thread_local int hidden = 7;
/* Shared, so the test's output does not depend on which thread printed first. */
static int seen[2];

static unsigned worker(void *argument) {
    unsigned id = *(const unsigned *)argument;
    /* Each thread adds to *its own* copy, which starts at 100. */
    counter += (int)id;
    seen[id - 1] = counter;
    return 0;
}

int main(void) {
    static const unsigned ids[2] = { 1, 2 };
    void *handles[2] = { 0, 0 };
    int i;
    printf("before: main %d hidden %d\n", counter, hidden);
    for (i = 0; i < 2; i++) {
        handles[i] = (void *)_beginthreadex(0, 0, worker, (void *)&ids[i], 0, 0);
    }
    for (i = 0; i < 2; i++) {
        if (handles[i] != 0) {
            WaitForSingleObject(handles[i], 0xFFFFFFFFul);
            CloseHandle(handles[i]);
        }
    }
    /* Two threads, two copies: each saw 100 + its own id, and this thread's is untouched. */
    printf("threads saw %d %d, main still %d\n", seen[0], seen[1], counter);
    counter = 42;
    hidden = 1;
    printf("after: main %d hidden %d\n", counter, hidden);
    return 0;
}
"#;

/// The program the complex test below compiles, as a raw Rust string so the C source keeps its
/// own quotes and its own backslashes.
const COMPLEX_PROGRAM: &str = r#"
#include <stdio.h>
#include <math.h>
#include <complex.h>

static double _Complex scaled(double _Complex z) {
    /* A complex value across a call boundary in both directions: sixteen bytes, so the caller
     * passes a pointer and the return comes back through one. */
    return z * (2.0 + 0.5 * I);
}

static float _Complex scaledf(float _Complex z) {
    /* And eight bytes, which travels in a register. */
    return z + z;
}

int main(void) {
    double _Complex a = { 2.0, 3.0 }, b = { 4.0, -1.0 };
    double _Complex s = a + b, d = a - b, p = a * b, q = a / b;
    float _Complex f = { 1.5F, -2.5F };

    printf("sizeof %d %d align %d\n", (int)sizeof a, (int)sizeof f,
           (int)_Alignof(double _Complex));
    printf("add %.6f %.6f\n", creal(s), cimag(s));
    printf("sub %.6f %.6f\n", creal(d), cimag(d));
    printf("mul %.6f %.6f\n", creal(p), cimag(p));
    printf("div %.6f %.6f\n", creal(q), cimag(q));
    printf("neg %.6f %.6f\n", creal(-a), cimag(-a));
    printf("cmp %d %d %d\n", a == a, a == b, a != b);
    printf("convert %.1f %.1f %d\n", (double)a, (double)a + 1.0, (int)(double)b);
    printf("truth %d %d %d\n", a ? 1 : 0, (double _Complex){ 0.0, 0.0 } ? 1 : 0, !f ? 1 : 0);
    printf("i %.1f %.1f\n", creal(I), cimag(I));
    printf("calls %.2f %.2f | %.2f %.2f\n", creal(scaled(a)), cimag(scaled(a)),
           (double)crealf(scaledf(f)), (double)cimagf(scaledf(f)));
    printf("cabs %.4f %.4f\n", cabs(a), (double)cabsf(f));
    printf("carg %.4f\n", carg(a));
    printf("conj %.1f %.1f\n", creal(conj(a)), cimag(conj(a)));
    printf("cproj %.1f %.1f\n", creal(cproj(a)), cimag(cproj(a)));
    /* 6.3.1.2 for a complex value, and the `l` spelling, which is a macro. */
    printf("bool %d %d\n", (int)(_Bool)a, (int)(_Bool)(double _Complex){ 0.0, 0.0 });
    printf("long %.4f\n", (double)cabsl(a));
    return 0;
}
"#;

/// The program the anonymous-member test below compiles, as a raw Rust string so the C source
/// keeps its own quotes and its own backslashes.
const ANONYMOUS_PROGRAM: &str = r#"
#include <stdio.h>
#include <math.h>

struct s { int a; union { int b; double c; }; struct { short d, e; }; int tail; };
struct node { const char *name; int type; void *context; };

static double negate(double v) { return -v; }
static double keep(double v) { return v; }

static double pick(int row) {
    /* A `static` table of function pointers, which is how tinyexpr's `power()` is written. */
    static double (*const column[2])(double) = { negate, keep };
    return column[row](5.0);
}

int main(void) {
    struct s x = { 0 };
    char *base = (char *)&x;
    /* An array whose size comes from its list: the elements are partly initialised. */
    struct node rows[] = { { "x" }, { "y", 9 } };
    printf("layout %d %ld %ld %ld %ld %ld %ld\n", (int)sizeof x,
           (long)((char *)&x.a - base), (long)((char *)&x.b - base), (long)((char *)&x.c - base),
           (long)((char *)&x.d - base), (long)((char *)&x.e - base), (long)((char *)&x.tail - base));
    x.b = 7;
    x.d = 3; x.e = 4;
    printf("members %d %d %d %d\n", x.a, x.b, x.d + x.e, x.tail);
    printf("names %s %s type %d %d context %d\n", rows[0].name, rows[1].name, rows[0].type,
           rows[1].type, rows[0].context == 0);
    printf("static table %.1f %.1f\n", pick(0), pick(1));
    printf("nan %d %d %d | inf %d %d\n", isnan(NAN), signbit(NAN) ? 1 : 0, isnan(1.0),
           isinf(INFINITY), isinf(1.0));
    return 0;
}
"#;

#[test]
#[cfg(target_os = "windows")]
fn a_frame_larger_than_a_stack_page_is_taken_one_page_at_a_time() {
    // A raw string, so the C source keeps its own quotes and its own backslashes.
    let source = r#"
#include <stdio.h>

static int walk(int depth) {
    char pad[5000];
    pad[0] = (char)depth;
    pad[4999] = 1;
    if (depth == 0) return pad[0] + pad[4999];
    return walk(depth - 1) + pad[4999] - pad[4999];
}

int main(void) {
    char small[8192], large[65536], huge[300000];
    small[0] = 1; small[8191] = 2;
    large[0] = 3; large[65535] = 4;
    huge[0] = 5; huge[299999] = 6;
    printf("%d %d %d %d %d %d\n", small[0], small[8191], large[0], large[65535],
           huge[0], huge[299999]);
    printf("deep %d\n", walk(100));
    return 0;
}
"#;
    let run = run_linked("bigframe", source, &[]);
    assert_eq!(run.status.code(), Some(0), "it ran to completion");
    let out = String::from_utf8_lossy(&run.stdout);
    // A redirected stream is a text stream on this target, so its lines end in CR LF.
    assert_eq!(out, "1 2 3 4 5 6\r\ndeep 1\r\n");
}

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

#[test]
#[cfg(target_os = "windows")]
fn wide_arrays_at_file_scope_are_filled_with_utf16() {
    // C99 6.7.8p14 fills an array of the *literal's* own character type - `wchar_t` for a wide
    // literal - and takes the array's length from the literal when it has none. This was refused
    // outright ("initialises an array of `char`, not unsigned short"), which made a wide string
    // constant impossible to write at file scope.
    let source = r#"
#include <stddef.h>
#include <stdio.h>

const wchar_t g[] = L"ab";
const wchar_t h[4] = L"xy";
wchar_t exact[2] = L"ab";
static const wchar_t *p = L"ptr";

int main(void) {
    printf("%d %d %d %d\n", (int)sizeof(g), (int)g[0], (int)g[1], (int)g[2]);
    printf("%d %d %d %d %d\n", (int)sizeof(h), (int)h[0], (int)h[1], (int)h[2], (int)h[3]);
    printf("%d %d %d\n", (int)exact[0], (int)exact[1], (int)sizeof(exact));
    printf("%d %d %d\n", (int)p[0], (int)p[1], (int)p[2]);
    return 0;
}
"#;
    let run = run_linked("wideglobals", source, &[]);
    assert_eq!(run.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&run.stdout).replace('\r', ""),
        "6 97 98 0\n8 120 121 0 0\n97 98 4\n112 116 114\n"
    );
}

#[test]
#[cfg(target_os = "windows")]
fn wide_stdio_and_conversion_state() {
    // C99 7.24: the wide `printf`/`scanf` front ends (the runtime carries them, as it does the
    // narrow ones), the four `wmem*`, `mbsinit`, and `fwide` - which is the platform's stub, so
    // asking is 0 and setting is the mode asked for, because this runtime has one orientation.
    let source = r#"
#include <stdio.h>
#include <string.h>
#include <wchar.h>

int main(void) {
    wchar_t text[32] = {0};
    int n = swprintf(text, 32, L"%d-%ls-%c", 42, L"wide", L'!');
    printf("swprintf=%d text=%ls len=%d\n", n, text, (int)wcslen(text));
    wprintf(L"wprintf: %ls %d\n", text, n);
    wchar_t word[8] = {0};
    int number = 0;
    int read = swscanf(L"77 words", L"%d %ls", &number, word);
    printf("swscanf=%d %d %ls\n", read, number, word);
    wchar_t buffer[8];
    wmemset(buffer, L'x', 3);
    buffer[3] = 0;
    wchar_t copy[8] = {0};
    wmemcpy(copy, buffer, 4);
    printf("wmem=%ls cmp=%d\n", copy, wmemcmp(buffer, copy, 4));
    mbstate_t state;
    memset(&state, 0, sizeof state);
    printf("mbsinit=%d null=%d fwide=%d\n", mbsinit(&state) != 0, mbsinit(0) != 0,
           fwide(stdout, 0));
    return 0;
}
"#;
    let run = run_linked("wideio", source, &[]);
    let out = String::from_utf8_lossy(&run.stdout).replace('\r', "");
    assert_eq!(run.status.code(), Some(0));
    assert_eq!(
        out,
        "swprintf=9 text=42-wide-! len=9\nwprintf: 42-wide-! 9\nswscanf=2 77 words\nwmem=xxx cmp=0\nmbsinit=1 null=1 fwide=0\n"
    );
}

#[test]
#[cfg(target_os = "windows")]
fn tgmath_picks_the_narrowest_function_and_evaluates_once() {
    // C99 7.22: one name per function, chosen by the widest argument. This needs `_Generic` - a
    // `sizeof`-based guess gets the value right and the *type* wrong, because a conditional over a
    // float call and a double call has type double - so `tgmath.h` needs `--std=c11` and says so
    // by using it. The type is asserted with `_Generic` itself, and the side effect proves the
    // arms that were not chosen were not evaluated.
    let source = r#"
#include <stdio.h>
#include <tgmath.h>

static int calls = 0;
static float value(void) { calls++; return 4.0f; }

int main(void) {
    float narrow = sqrt(value());
    printf("calls=%d\n", calls);
    printf("types=%d %d %d\n",
           _Generic(sqrt(4.0f), float: 1, double: 2, default: 0),
           _Generic(sqrt(4.0), float: 1, double: 2, default: 0),
           _Generic(pow(2.0f, 3.0), float: 1, double: 2, default: 0));
    printf("value=%g narrow=%d\n", (double)narrow, (int)(sizeof(narrow) == sizeof(float)));
    return 0;
}
"#;
    let run = run_linked_with("c11", "tgmath", source, &[]);
    let out = String::from_utf8_lossy(&run.stdout).replace('\r', "");
    assert_eq!(run.status.code(), Some(0), "{out}");
    assert_eq!(out, "calls=1\ntypes=1 2 2\nvalue=2 narrow=1\n");
}

/// Three things the fourth real library reached and nothing else had: an anonymous struct or union
/// (C11 6.7.2.1p13) whose members are borrowed by the record that holds it; a `static` table of
/// function pointers whose initialiser must be data and not code; and an array whose size comes
/// from its list, whose elements' unnamed members are zero. The last line is `NAN`'s sign bit,
/// which `0.0/0.0` gets wrong. Every number here is what `clang` prints for this program.
#[test]
#[cfg(target_os = "windows")]
fn anonymous_members_static_tables_and_unlisted_members_are_laid_out_and_zeroed() {
    let run = run_linked("anonymous", ANONYMOUS_PROGRAM, &[]);
    assert_eq!(run.status.code(), Some(0), "it ran to completion");
    let out = String::from_utf8_lossy(&run.stdout);
    assert_eq!(
        out,
        "layout 24 0 8 8 16 18 20@@members 0 7 7 0@@names x y type 0 9 context 1@@static table -5.0 5.0@@nan 1 0 0 | inf 1 0@@"
            .replace("@@", "\r\n")
    );
}

/// `_Complex`, end to end: the type's layout, its four operators and its comparisons (which the
/// compiler computes), the two conversions, the `I` macro, a value crossing a call boundary in each
/// of the two shapes the ABI has for it, and the four functions the C runtime source carries. Every
/// number here is what `clang` prints for this program.
#[test]
#[cfg(target_os = "windows")]
fn complex_numbers_are_laid_out_computed_and_passed_the_way_clang_does() {
    // `--std=c11` for `_Alignof`, which is the only C11 spelling in the program.
    let run = run_linked_with("c11", "complex", COMPLEX_PROGRAM, &[]);
    assert_eq!(run.status.code(), Some(0), "it ran to completion");
    let out = String::from_utf8_lossy(&run.stdout);
    // What `clang` printed for this program, recorded when the test was written. Lines are
    // separated by `@@` and turned back into CRLF below, because a redirected stream on
    // this target ends its lines that way.
    // What `clang` printed for this program, recorded when the test was written. Lines are
    // separated by `@@` and turned back into a redirected stream's CRLF below.
    let expected = "sizeof 16 8 align 8@@add 6.000000 2.000000@@sub -2.000000 4.000000@@mul 11.000000 10.000000@@div 0.294118 0.823529@@neg -2.000000 -3.000000@@cmp 1 0 1@@convert 2.0 3.0 4@@truth 1 0 0@@i 0.0 1.0@@calls 2.50 7.00 | 3.00 -5.00@@cabs 3.6056 2.9155@@carg 0.9828@@conj 2.0 -3.0@@cproj 2.0 3.0@@bool 1 0@@long 3.6056@@";
    assert_eq!(out, expected.replace("@@", "\r\n"));
}

/// The sixteen complex transcendentals of C99 7.3.5 to 7.3.8, at six points in every quadrant, with
/// the four identities that say the branches are Annex G's. Every line is what `clang` printed for
/// this program; the `float` line is four decimals because those forms compute in `double` and
/// narrow once, so their last bit is not the runtime's.
#[test]
#[cfg(target_os = "windows")]
fn the_complex_transcendentals_agree_with_clang() {
    let run = run_linked_with("c11", "complex-tr", TRANSCENDENTAL_PROGRAM, &[]);
    assert_eq!(run.status.code(), Some(0), "it ran to completion");
    let out = String::from_utf8_lossy(&run.stdout);
    let expected = "exp   +1.500000 -0.750000 -> +3.279202007 -3.054892981@@log   +1.500000 -0.750000 -> +0.517036884 -0.463647609@@sqrt  +1.500000 -0.750000 -> +1.260367205 -0.297532337@@sin   +1.500000 -0.750000 -> +1.291440086 -0.058168385@@cos   +1.500000 -0.750000 -> +0.091582273 +0.820256818@@tan   +1.500000 -0.750000 -> +0.103580619 -1.562868982@@sinh  +1.500000 -0.750000 -> +1.557970076 -1.603493573@@cosh  +1.500000 -0.750000 -> +1.721231931 -1.451399408@@tanh  +1.500000 -0.750000 -> +0.988112101 -0.098387819@@asin  +1.500000 -0.750000 -> +1.024288057 -1.162792098@@acos  +1.500000 -0.750000 -> +0.546508270 +1.162792098@@atan  +1.500000 -0.750000 -> +1.057136961 -0.207933336@@asinh +1.500000 -0.750000 -> +1.265075043 -0.402910830@@acosh +1.500000 -0.750000 -> +1.162792098 -0.546508270@@atanh +1.500000 -0.750000 -> +0.531599631 -1.225127862@@pow   +1.500000 -0.750000 -> +2.481127683 -1.158844163@@exp   -2.000000 +0.500000 -> +0.118767885 +0.064883191@@log   -2.000000 +0.500000 -> +0.723459491 +2.896613990@@sqrt  -2.000000 +0.500000 -> +0.175432056 +1.425053124@@sin   -2.000000 +0.500000 -> -1.025347389 -0.216852163@@cos   -2.000000 +0.500000 -> -0.469257978 +0.473830620@@tan   -2.000000 +0.500000 -> +0.850878121 +1.321286584@@sinh  -2.000000 +0.500000 -> -3.182869448 +1.803692696@@cosh  -2.000000 +0.500000 -> +3.301637333 -1.738809504@@tanh  -2.000000 +0.500000 -> -0.979940850 +0.030215987@@asin  -2.000000 +0.500000 -> -1.293042070 +1.361800901@@acos  -2.000000 +0.500000 -> +2.863838397 -1.361800901@@atan  -2.000000 +0.500000 -> -1.126556441 +0.096415620@@asinh -2.000000 +0.500000 -> -1.465715352 +0.221018636@@acosh -2.000000 +0.500000 -> +1.361800901 +2.863838397@@atanh -2.000000 +0.500000 -> -0.500370000 +1.421546861@@pow   -2.000000 +0.500000 -> -0.003990956 -0.695493077@@exp   +0.250000 +2.000000 -> -0.534343115 +1.167561007@@log   +0.250000 +2.000000 -> +0.700899274 +1.446441332@@sqrt  +0.250000 +2.000000 -> +1.064322422 +0.939564909@@sin   +0.250000 +2.000000 -> +0.930782109 +3.514110101@@cos   +0.250000 +2.000000 -> +3.645238138 -0.897299625@@tan   +0.250000 +2.000000 -> +0.017009461 +0.968214572@@sinh  +0.250000 +2.000000 -> -0.105123817 +0.937861278@@cosh  +0.250000 +2.000000 -> -0.429219299 +0.229699730@@tanh  +0.250000 +2.000000 -> +1.099398135 -1.596689211@@asin  +0.250000 +2.000000 -> +0.111477925 +1.449208152@@acos  +0.250000 +2.000000 -> +1.459318401 -1.449208152@@atan  +0.250000 +2.000000 -> +1.489877611 +0.535880100@@asinh +0.250000 +2.000000 -> +1.328763370 +1.427924145@@acosh +0.250000 +2.000000 -> +1.449208152 +1.459318401@@atanh +0.250000 +2.000000 -> +0.049544232 +1.112111334@@pow   +0.250000 +2.000000 -> -1.128766039 +0.808362658@@exp   +3.000000 +1.000000 -> +10.852261914 +16.901396535@@log   +3.000000 +1.000000 -> +1.151292546 +0.321750554@@sqrt  +3.000000 +1.000000 -> +1.755317302 +0.284848785@@sin   +3.000000 +1.000000 -> +0.217759552 -1.163440364@@cos   +3.000000 +1.000000 -> -1.527638250 -0.165844402@@tan   +3.000000 +1.000000 -> -0.059168540 +0.768017647@@sinh  +3.000000 +1.000000 -> +5.412680923 +8.471645454@@cosh  +3.000000 +1.000000 -> +5.439580991 +8.429751081@@tanh  +3.000000 +1.000000 -> +1.002054988 +0.004517137@@asin  +3.000000 +1.000000 -> +1.233095218 +1.824198702@@acos  +3.000000 +1.000000 -> +0.337701109 -1.824198702@@atan  +3.000000 +1.000000 -> +1.276795025 +0.091931195@@asinh +3.000000 +1.000000 -> +1.864161544 +0.307603650@@acosh +3.000000 +1.000000 -> +1.824198702 +0.337701109@@atanh +3.000000 +1.000000 -> +0.305943858 +1.461461854@@pow   +3.000000 +1.000000 -> +2.347817547 +4.172581814@@exp   -0.500000 -1.500000 -> +0.042904282 -0.605011292@@log   -0.500000 -1.500000 -> +0.458145366 -1.892546881@@sqrt  -0.500000 -1.500000 -> +0.735234259 -1.020083043@@sin   -0.500000 -1.500000 -> -1.127805247 -1.868618519@@cos   -0.500000 -1.500000 -> +2.064433657 -1.020830950@@tan   -0.500000 -1.500000 -> -0.079324455 -0.944372986@@sinh  -0.500000 -1.500000 -> -0.036860824 -1.124801247@@cosh  -0.500000 -1.500000 -> +0.079765105 +0.519789955@@tanh  -0.500000 -1.500000 -> -2.124799128 -0.255149222@@asin  -0.500000 -1.500000 -> -0.273487290 -1.226456871@@acos  -0.500000 -1.500000 -> +1.844283617 +1.226456871@@atan  -0.500000 -1.500000 -> -1.276795025 -0.641237339@@asinh -0.500000 -1.500000 -> -1.069311043 -1.171157259@@acosh -0.500000 -1.500000 -> +1.226456871 -1.844283617@@atanh -0.500000 -1.500000 -> -0.146946666 -1.017221968@@pow   -0.500000 -1.500000 -> -4.414276760 -2.597352074@@exp   +0.500000 +0.500000 -> +1.446889037 +0.790439083@@log   +0.500000 +0.500000 -> -0.346573590 +0.785398163@@sqrt  +0.500000 +0.500000 -> +0.776886987 +0.321797126@@sin   +0.500000 +0.500000 -> +0.540612686 +0.457304153@@cos   +0.500000 +0.500000 -> +0.989584883 -0.249826398@@tan   +0.500000 +0.500000 -> +0.403896455 +0.564083141@@sinh  +0.500000 +0.500000 -> +0.457304153 +0.540612686@@cosh  +0.500000 +0.500000 -> +0.989584883 +0.249826398@@tanh  +0.500000 +0.500000 -> +0.564083141 +0.403896455@@asin  +0.500000 +0.500000 -> +0.452278447 +0.530637531@@acos  +0.500000 +0.500000 -> +1.118517880 -0.530637531@@atan  +0.500000 +0.500000 -> +0.553574359 +0.402359478@@asinh +0.500000 +0.500000 -> +0.530637531 +0.452278447@@acosh +0.500000 +0.500000 -> +0.530637531 +1.118517880@@atanh +0.500000 +0.500000 -> +0.402359478 +0.553574359@@pow   +0.500000 +0.500000 -> +0.215301135 +0.338886253@@edge  +0.000000 +0.000000 -> +0.000000000 +0.000000000@@easin +0.000000 +0.000000 -> +0.000000000 +0.000000000@@eacos +0.000000 +0.000000 -> +1.570796327 +0.000000000@@eatan +0.000000 +0.000000 -> +0.000000000 +0.000000000@@esinh +0.000000 +0.000000 -> +0.000000000 +0.000000000@@eacsh +0.000000 +0.000000 -> +0.000000000 +1.570796327@@eatnh +0.000000 +0.000000 -> +0.000000000 +0.000000000@@etanh +0.000000 +0.000000 -> +0.000000000 +0.000000000@@expw  +0.000000 +0.000000 -> +0.000000000 +0.000000000@@edge  -4.000000 +0.000000 -> +0.000000000 +2.000000000@@easin -4.000000 +0.000000 -> -1.570796327 +2.063437069@@eacos -4.000000 +0.000000 -> +3.141592654 -2.063437069@@eatan -4.000000 +0.000000 -> -1.325817664 +0.000000000@@esinh -4.000000 +0.000000 -> -2.094712547 +0.000000000@@eacsh -4.000000 +0.000000 -> +2.063437069 +3.141592654@@eatnh -4.000000 +0.000000 -> -0.255412812 +1.570796327@@etanh -4.000000 +0.000000 -> -0.999329300 +0.000000000@@expw  -4.000000 +0.000000 -> -64.000000000 +0.000000000@@edge  +2.000000 +0.000000 -> +1.414213562 +0.000000000@@easin +2.000000 +0.000000 -> +1.570796327 +1.316957897@@eacos +2.000000 +0.000000 -> +0.000000000 -1.316957897@@eatan +2.000000 +0.000000 -> +1.107148718 +0.000000000@@esinh +2.000000 +0.000000 -> +1.443635475 +0.000000000@@eacsh +2.000000 +0.000000 -> +1.316957897 +0.000000000@@eatnh +2.000000 +0.000000 -> +0.549306144 +1.570796327@@etanh +2.000000 +0.000000 -> +0.964027580 +0.000000000@@expw  +2.000000 +0.000000 -> +8.000000000 +0.000000000@@edge  -1.000000 +0.000000 -> +0.000000000 +1.000000000@@easin -1.000000 +0.000000 -> -1.570796327 +0.000000000@@eacos -1.000000 +0.000000 -> +3.141592654 +0.000000000@@eatan -1.000000 +0.000000 -> -0.785398163 +0.000000000@@esinh -1.000000 +0.000000 -> -0.881373587 +0.000000000@@eacsh -1.000000 +0.000000 -> +0.000000000 +3.141592654@@eatnh -1.000000 +0.000000 -> -inf +0.000000000@@etanh -1.000000 +0.000000 -> -0.761594156 +0.000000000@@expw  -1.000000 +0.000000 -> -1.000000000 +0.000000000@@edge  +1.000000 +0.000000 -> +1.000000000 +0.000000000@@easin +1.000000 +0.000000 -> +1.570796327 +0.000000000@@eacos +1.000000 +0.000000 -> +0.000000000 +0.000000000@@eatan +1.000000 +0.000000 -> +0.785398163 +0.000000000@@esinh +1.000000 +0.000000 -> +0.881373587 +0.000000000@@eacsh +1.000000 +0.000000 -> +0.000000000 +0.000000000@@eatnh +1.000000 +0.000000 -> +inf +0.000000000@@etanh +1.000000 +0.000000 -> +0.761594156 +0.000000000@@expw  +1.000000 +0.000000 -> +1.000000000 +0.000000000@@edge  +0.000000 +1.000000 -> +0.707106781 +0.707106781@@easin +0.000000 +1.000000 -> +0.000000000 +0.881373587@@eacos +0.000000 +1.000000 -> +1.570796327 -0.881373587@@eatan +0.000000 +1.000000 -> +0.000000000 +inf@@esinh +0.000000 +1.000000 -> +0.000000000 +1.570796327@@eacsh +0.000000 +1.000000 -> +0.881373587 +1.570796327@@eatnh +0.000000 +1.000000 -> +0.000000000 +0.785398163@@etanh +0.000000 +1.000000 -> +0.000000000 +1.557407725@@expw  +0.000000 +1.000000 -> -0.000000000 -1.000000000@@edge  +0.000000 -1.000000 -> +0.707106781 -0.707106781@@easin +0.000000 -1.000000 -> +0.000000000 -0.881373587@@eacos +0.000000 -1.000000 -> +1.570796327 +0.881373587@@eatan +0.000000 -1.000000 -> +0.000000000 -inf@@esinh +0.000000 -1.000000 -> +0.000000000 -1.570796327@@eacsh +0.000000 -1.000000 -> +0.881373587 -1.570796327@@eatnh +0.000000 -1.000000 -> +0.000000000 -0.785398163@@etanh +0.000000 -1.000000 -> +0.000000000 -1.557407725@@expw  +0.000000 -1.000000 -> -0.000000000 +1.000000000@@sat   +inf +0.000000 -> +1.000000000 +0.000000000@@sin^2 +1.500000 -0.750000 -> +1.000000000 +0.000000000@@expog +1.500000 -0.750000 -> +0.000000000 +0.000000000@@sqrtsq +1.500000 -0.750000 -> +0.000000000 +0.000000000@@asinsin +1.500000 -0.750000 -> +0.000000000 +0.000000000@@float 0.6675 -2.0090 | 1.0507 -0.5949 | 1.2872 -1.1721 | 0.2615 -0.9968@@";
    assert_eq!(out, expected.replace("@@", "\r\n"));
}

/// `_Thread_local` (C11 6.7.3), proved where it matters: a second and third thread, each with its
/// own copy of every thread-local object of the program, initialised from the template the linker
/// laid out. The threads record what they saw in a shared array, so the program's output does not
/// depend on which of them ran first.
#[test]
#[cfg(target_os = "windows")]
fn a_thread_local_object_has_one_copy_per_thread() {
    let run = run_linked_with("c11", "thread-local", THREAD_LOCAL_PROGRAM, &[]);
    assert_eq!(run.status.code(), Some(0), "it ran to completion");
    let out = String::from_utf8_lossy(&run.stdout);
    let expected =
        "before: main 100 hidden 7@@threads saw 101 102, main still 100@@after: main 42 hidden 1@@";
    assert_eq!(out, expected.replace("@@", "\r\n"));
}

/// `stdatomic.h` (C11 7.17), proved where it matters: two threads hammer a counter and a spinlock,
/// and the totals must be *exact*. A non-atomic implementation loses increments and nothing else in
/// a test would notice - which is why this is the assertion.
#[test]
#[cfg(target_os = "windows")]
fn atomic_counters_lose_nothing_across_two_threads() {
    let run = run_linked_with("c11", "atomic", ATOMIC_PROGRAM, &[]);
    assert_eq!(run.status.code(), Some(0), "it ran to completion");
    let out = String::from_utf8_lossy(&run.stdout);
    let expected = "counters 200000 200000 200000, expected 200000@@";
    assert_eq!(out, expected.replace("@@", "\r\n"));
}

/// `threads.h` (C11 7.26): a producer and a consumer over a mutex and a condition variable, a
/// thread-specific slot, a one-time initialisation that must run exactly once, and a join that
/// carries the thread's return value.
///
/// There is no `clang` oracle here: mingw ships no `threads.h` at all - measured, `clang -std=c11`
/// says "file not found" - so these three lines are what C11 7.26 says must happen, written down
/// when the header was added.
#[test]
#[cfg(target_os = "windows")]
fn threads_a_mutex_a_condition_variable_and_a_once_flag_work() {
    let run = run_linked_with("c11", "threads", THREADS_PROGRAM, &[]);
    assert_eq!(run.status.code(), Some(0), "it ran to completion");
    let out = String::from_utf8_lossy(&run.stdout);
    let expected = "consumer saw 42, tss 1, once 1@@joined with 43, once ran 1 time(s)@@equal 1, timed mutex 4@@";
    assert_eq!(out, expected.replace("@@", "\r\n"));
}

/// `uchar.h` (C11 7.28): the two character types, the four conversions, and the three literal
/// families - `u` is UTF-16 code units, `U` is code points, `u8` is UTF-8 bytes, which is the
/// encoding this compiler reads its source in. Every number is what `clang` prints for this
/// program with the same locale; the header being included twice is the include guard's business.
#[test]
#[cfg(target_os = "windows")]
fn uchar_types_conversions_and_the_three_literal_families() {
    let run = run_linked_with("c11", "uchar", UCHAR_PROGRAM, &[]);
    assert_eq!(run.status.code(), Some(0), "it ran to completion");
    let out = String::from_utf8_lossy(&run.stdout);
    let expected = "sizes 7 12 24 2 4@@firsts 104 104 104 -61 233 233@@chars 2 233 4 233 2@@mbrtoc16 97 1@@mbrtoc32 97 1@@c16rtomb 1 Z@@c32rtomb 1 Q@@";
    assert_eq!(out, expected.replace("@@", "\r\n"));
}

/// The `#pragma pack` matrix the test below runs: every line is what `clang -std=c11` prints for
/// this program, and the `sizeof`/`offsetof` numbers are the whole of what packing decides.
const PACK_PROGRAM: &str = r#"#include <stddef.h>
#include <stdio.h>
#include <string.h>

/* The default: nothing packed. */
struct plain {
  char c;
  int i;
  short s;
};
#pragma pack(1)
struct p1 {
  char c;
  int i;
  short s;
};
#pragma pack(2)
struct p2 {
  char c;
  int i;
  short s;
};
#pragma pack(4)
struct p4 {
  char c;
  double d;
};
#pragma pack(push, 2)
struct pushed {
  char c;
  int i;
};
#pragma pack(pop)
struct popped {
  char c;
  int i;
};
#pragma pack()
struct reset {
  char c;
  int i;
};
#pragma pack(1)
struct bits1 {
  unsigned a : 3;
  unsigned b : 20;
  char c;
};
#pragma pack(2)
struct bits2 {
  unsigned a : 3;
  unsigned b : 20;
  char c;
};
struct inner {
  char c;
  int i;
};
#pragma pack(1)
struct outer1 {
  char c;
  struct inner in;
};
#pragma pack(2)
struct outer2 {
  char c;
  struct inner in;
};
#pragma pack()
struct aligned {
  _Alignas(16) char c;
  int i;
};
#pragma pack(2)
struct capped_alignment {
  _Alignas(16) char c;
  int i;
};

/* A packed struct the program actually uses: by value through the ABI, in an array, initialised,
 * and as bytes. */
#pragma pack(push, 1)
struct packet {
  unsigned char tag;
  unsigned int length;
  unsigned short kind;
};
#pragma pack(pop)

struct packet make(unsigned char tag, unsigned int length, unsigned short kind) {
  struct packet made;
  made.tag = tag;
  made.length = length;
  made.kind = kind;
  return made;
}

unsigned int total(struct packet one) { return one.tag + one.length + one.kind; }

int main(void) {
  printf("plain %d %d %d %d\n", (int)sizeof(struct plain), (int)offsetof(struct plain, i),
         (int)offsetof(struct plain, s), (int)_Alignof(struct plain));
  printf("p1 %d %d %d %d\n", (int)sizeof(struct p1), (int)offsetof(struct p1, i),
         (int)offsetof(struct p1, s), (int)_Alignof(struct p1));
  printf("p2 %d %d %d %d\n", (int)sizeof(struct p2), (int)offsetof(struct p2, i),
         (int)offsetof(struct p2, s), (int)_Alignof(struct p2));
  printf("p4 %d %d %d\n", (int)sizeof(struct p4), (int)offsetof(struct p4, d),
         (int)_Alignof(struct p4));
  printf("push %d %d pop %d %d reset %d %d\n", (int)sizeof(struct pushed),
         (int)offsetof(struct pushed, i), (int)sizeof(struct popped),
         (int)offsetof(struct popped, i), (int)sizeof(struct reset),
         (int)offsetof(struct reset, i));
  printf("bits %d %d\n", (int)sizeof(struct bits1), (int)sizeof(struct bits2));
  printf("outer %d %d %d %d %d\n", (int)sizeof(struct inner), (int)sizeof(struct outer1),
         (int)offsetof(struct outer1, in), (int)sizeof(struct outer2),
         (int)offsetof(struct outer2, in));
  printf("align %d %d %d %d\n", (int)sizeof(struct aligned), (int)_Alignof(struct aligned),
         (int)sizeof(struct capped_alignment), (int)_Alignof(struct capped_alignment));

  struct packet one = make(1, 4096, 7);
  struct packet list[2] = {{2, 300, 9}, {3, 400, 11}};
  printf("packet %d %u %d %u %u\n", (int)sizeof(struct packet), one.length, (int)one.kind,
         total(one), total(list[1]));
  printf("array %d %d %u %u\n", (int)list[0].tag, (int)list[1].tag, list[0].length,
         list[1].length);
  {
    unsigned char bytes[7];
    memcpy(bytes, &one, sizeof(struct packet));
    printf("bytes %d %d %d\n", (int)bytes[0], (int)bytes[1], (int)bytes[2]);
  }
  return 0;
}
"#;

/// `#pragma pack` decides a struct's offsets, and a compiler that ignores it writes a program that
/// touches the wrong bytes. The numbers here are the measurement: `{char,int,short}` is 12/4/8
/// unpacked, 7/1/5 under `pack(1)`, 8/2/6 under `pack(2)`; a `double` moves from offset 8 to 4
/// under `pack(4)`; a bitfield unit's alignment is capped the same way (5 and 6 bytes against 8);
/// `pack` caps an explicit `_Alignas(16)` too (6 bytes, align 2 under `pack(2)` against 16/16).
///
/// The last three lines are the ones a `sizeof` table cannot prove: a packed struct passed **by
/// value**, in an array, initialised in braces, and read back as raw bytes - so the offsets are
/// used by the object writer and the ABI, not merely computed.
#[test]
#[cfg(target_os = "windows")]
fn packing_decides_offsets_the_way_clang_does() {
    let run = run_linked_with("c11", "pack", PACK_PROGRAM, &[]);
    assert_eq!(run.status.code(), Some(0), "it ran to completion");
    let out = String::from_utf8_lossy(&run.stdout);
    let expected = "plain 12 4 8 4@@p1 7 1 5 1@@p2 8 2 6 2@@p4 12 4 4@@push 6 2 pop 8 4 reset 8 4@@bits 5 6@@outer 6 7 1 8 2@@align 16 16 6 2@@packet 7 4096 7 4104 414@@array 2 3 300 400@@bytes 1 0 16@@";
    assert_eq!(out, expected.replace("@@", "\r\n"));
}

/// The `__attribute__((packed))` / `((aligned(N)))` matrix the test below runs: every line is what
/// `clang -std=c11` prints for this program.
const ATTRIBUTE_LAYOUT_PROGRAM: &str = r#"#include <stddef.h>
#include <stdio.h>

struct whole {
  char c;
  int i;
} __attribute__((packed));

struct member {
  char c;
  int i __attribute__((packed));
};

struct raised {
  char c;
} __attribute__((aligned(16)));

struct both {
  int i;
} __attribute__((packed, aligned(8)));

struct member_aligned {
  char c;
  int i __attribute__((aligned(8)));
};

struct packed_alignas {
  _Alignas(16) char c;
  int i;
} __attribute__((packed));

struct packed_member_aligned {
  char c;
  int i __attribute__((packed, aligned(8)));
};

int global __attribute__((aligned(16)));


/* A member packed, the record not: `i` at 1, `s` aligned to 2, 8 bytes - not 7, which is what
 * treating the member's attribute as the record's produced. */
struct one_member {
  char c;
  int i __attribute__((packed));
  short s;
};

/* A member's attribute written *before* its type, which a range that started at the declarator
 * missed entirely: 16 bytes with `i` at 8, against 8 and 4 for the natural layout. */
struct attribute_first {
  char c;
  __attribute__((aligned(8))) int i;
};

int extra(void) {
  printf("one_member %d %d %d %d\n", (int)sizeof(struct one_member),
         (int)offsetof(struct one_member, i), (int)offsetof(struct one_member, s),
         (int)_Alignof(struct one_member));
  printf("attribute_first %d %d %d\n", (int)sizeof(struct attribute_first),
         (int)offsetof(struct attribute_first, i), (int)_Alignof(struct attribute_first));
  return 0;
}

int main(void) {
  extra();
  printf("whole %d %d %d\n", (int)sizeof(struct whole), (int)offsetof(struct whole, i),
         (int)_Alignof(struct whole));
  printf("member %d %d %d\n", (int)sizeof(struct member), (int)offsetof(struct member, i),
         (int)_Alignof(struct member));
  printf("raised %d %d\n", (int)sizeof(struct raised), (int)_Alignof(struct raised));
  printf("both %d %d\n", (int)sizeof(struct both), (int)_Alignof(struct both));
  printf("member_aligned %d %d %d\n", (int)sizeof(struct member_aligned),
         (int)offsetof(struct member_aligned, i), (int)_Alignof(struct member_aligned));
  printf("packed_alignas %d %d %d\n", (int)sizeof(struct packed_alignas),
         (int)offsetof(struct packed_alignas, i), (int)_Alignof(struct packed_alignas));
  printf("packed_member_aligned %d %d %d\n", (int)sizeof(struct packed_member_aligned),
         (int)offsetof(struct packed_member_aligned, i),
         (int)_Alignof(struct packed_member_aligned));
  printf("global %d\n", (int)((unsigned long long)&global % 16));
  return 0;
}
"#;

/// The two attributes that decide where bytes go, and the one place they differ from `#pragma pack`.
///
/// `struct { char c; int i; } __attribute__((packed))` is 5 bytes with `i` at 1 and alignment 1;
/// on one member it is the same 5 bytes; `aligned(16)` on a record is 16 bytes and alignment 16;
/// `packed, aligned(8)` is 8 and 8, so `aligned` is a floor over the packed cap.
///
/// The row that measures the difference: a `_Alignas(16)` member in a packed struct keeps its 16
/// (`packed` caps the *natural* alignment only), where `#pragma pack(1)` would cap it to 1 - the
/// previous commit's matrix says so, and both are `clang`'s answer.
#[test]
#[cfg(target_os = "windows")]
fn packed_and_aligned_decide_offsets_the_way_clang_does() {
    let run = run_linked_with("c11", "attribute-layout", ATTRIBUTE_LAYOUT_PROGRAM, &[]);
    assert_eq!(run.status.code(), Some(0), "it ran to completion");
    let out = String::from_utf8_lossy(&run.stdout);
    let expected = "one_member 8 1 6 2@@attribute_first 16 8 8@@whole 5 1 1@@member 5 1 1@@raised 16 16@@both 8 8@@member_aligned 16 8 8@@packed_alignas 16 1 16@@packed_member_aligned 16 8 8@@global 0@@";
    assert_eq!(out, expected.replace("@@", "\r\n"));
}

/// The segment-relative read, which the Windows SDK's `__readgsqword` family is: `mov rax,
/// gs:[offset]`. The proof is not that the number looks plausible - it is that the TEB's client id,
/// 0x48 bytes into what `gs:0x30` points at, is the thread id `GetCurrentThreadId` returns.
///
/// That second half is the import search: `GetCurrentThreadId` is a `kernel32` export the program
/// declares itself, found through the captured export tables rather than a curated list.
#[test]
#[cfg(target_os = "windows")]
fn a_segment_read_reaches_the_thread_environment_block() {
    let source = r#"
#include <stdio.h>

unsigned long long __readgsqword(unsigned long offset);
unsigned long __readgsdword(unsigned long offset);
unsigned long GetCurrentThreadId(void);

int main(void) {
  unsigned long long teb = __readgsqword(0x30);
  unsigned long *client = (unsigned long *)(teb + 0x48);
  printf("%d %d %d\n", (int)(*client == GetCurrentThreadId()),
         (int)(__readgsdword(0x30) != 0), (int)(__readgsqword(0x30) == teb));
  return 0;
}
"#;
    let run = run_linked_with("c99", "segment-read", source, &[]);
    assert_eq!(run.status.code(), Some(0), "it ran to completion");
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "1 1 1");
}

/// The Microsoft intrinsics the runtime implements, as a program: every line is what `clang`
/// prints, so the arithmetic is checked against the compiler that has them as instructions.
const INTRINSIC_PROGRAM: &str = r#"#include <stdio.h>

unsigned long long __shiftright128(unsigned long long, unsigned long long, unsigned char);
unsigned long long __shiftleft128(unsigned long long, unsigned long long, unsigned char);
unsigned long long _umul128(unsigned long long, unsigned long long, unsigned long long *);
long long _mul128(long long, long long, long long *);

static void umul(unsigned long long a, unsigned long long b) {
  unsigned long long high = 0;
  unsigned long long low = _umul128(a, b, &high);
  printf("umul %llx %llx %llx\n", a, low, high);
}

static void mul(long long a, long long b) {
  long long high = 0;
  long long low = _mul128(a, b, &high);
  printf("mul %lld %llx %llx\n", a, (unsigned long long)low, (unsigned long long)high);
}

int main(void) {
  umul(0, 0);
  umul(1, 0xFFFFFFFFFFFFFFFFULL);
  umul(0xFFFFFFFFFFFFFFFFULL, 0xFFFFFFFFFFFFFFFFULL);
  umul(0x100000000ULL, 0x100000000ULL);
  umul(0xFFFFFFFFULL, 0xFFFFFFFFULL);
  umul(0x123456789ABCDEFULL, 0x0FEDCBA98765432ULL);
  mul(-1, -1);
  mul(-5, 3);
  mul(5, -3);
  mul(-9223372036854775807LL - 1, -1);
  mul(-9223372036854775807LL - 1, -9223372036854775807LL - 1);
  mul(9223372036854775807LL, 9223372036854775807LL);
  mul(-123456789012345LL, 98765432109876LL);
  printf("shr1 %llx %llx\n", __shiftright128(0xFFFFFFFFFFFFFFFFULL, 0, 1),
         __shiftright128(0, 0xFFFFFFFFFFFFFFFFULL, 63));
  printf("shl1 %llx %llx\n", __shiftleft128(0xFFFFFFFFFFFFFFFFULL, 0, 63),
         __shiftleft128(0, 0xFFFFFFFFFFFFFFFFULL, 1));
  return 0;
}
"#;

/// The Microsoft intrinsics a Windows SDK's headers call, implemented as *C* in the runtime - the
/// 128-bit shift pair, the two 64x64 products and the `__stos*` fills. They are arithmetic, not
/// instruction encodings: `clang` has them as `shrd`/`mul`/`stos`, and every line here is what
/// `clang` prints for the same program.
///
/// The first version of `_umul128` had the wrong *low* word - it dropped the `al*bh` term - and the
/// high word was right, which is exactly the shape of bug a `sizeof`-sized test would miss and this
/// one did not.
#[test]
#[cfg(target_os = "windows")]
fn the_sdk_intrinsics_compute_what_clang_computes() {
    let run = run_linked_with("c99", "intrinsics", INTRINSIC_PROGRAM, &[]);
    assert_eq!(run.status.code(), Some(0), "it ran to completion");
    let out = String::from_utf8_lossy(&run.stdout);
    let expected = "umul 0 0 0@@umul 1 ffffffffffffffff 0@@umul ffffffffffffffff 1 fffffffffffffffe@@umul 100000000 0 1@@umul ffffffff fffffffe00000001 0@@umul 123456789abcdef 42100282174aa4ae 121fa00ad77d7@@mul -1 1 0@@mul -5 fffffffffffffff1 ffffffffffffffff@@mul 5 fffffffffffffff1 ffffffffffffffff@@mul -9223372036854775808 8000000000000000 0@@mul -9223372036854775808 0 4000000000000000@@mul 9223372036854775807 1 3fffffffffffffff@@mul -123456789012345 1f3127d607a6b86c ffffffffd899f81a@@shr1 7fffffffffffffff fffffffffffffffe@@shl1 7fffffffffffffff fffffffffffffffe@@";
    assert_eq!(out, expected.replace("@@", "\r\n"));
}

/// The addend program: addresses *into* objects in a data-section initialiser, and the constant
/// pointer sqlite makes out of an integer. Every line is what `clang` prints.
const ADDEND_PROGRAM: &str = r#"#include <stdio.h>

/* An address in a data-section initialiser, which is a symbol *and how far into it* - the addend.
 * Every shape here needs one: an element of a table, a member of an element, a cast on the way,
 * and sqlite's `SQLITE_INT_TO_PTR`, which turns an integer into a pointer with no symbol at all. */
static const unsigned char table[16] = "0123456789abcdef";
const char *const later = (const char *)&table[4];
const char *const direct = &table[10];

struct point {
  int x;
  int y;
};
static struct point points[4] = {{0, 0}, {1, 10}, {2, 20}, {3, 30}};
static const int *const second = &points[1].y;

static const char letters[] = "abcdef";
const char *const from = &letters[3];

/* sqlite's shape: `&((char *)0)[X]` is `(char *)0 + X`, and C99 6.5.3.2p3 makes the address of a
 * dereference the pointer that produced it. A constant base makes it a constant, so the bytes go in
 * with no relocation; a table of them is sqlite's function registry. */
#define INT_TO_PTR(X) ((void *)&((char *)0)[X])
static void *const registry[] = {INT_TO_PTR(0), INT_TO_PTR(5), INT_TO_PTR(9)};

int main(void) {
  printf("%c %c %d %s\n", *later, *direct, *second, from);
  printf("%d %d %d\n", (int)(long long)registry[1], (int)(long long)registry[2],
         (int)(size_t)registry[0]);
  return 0;
}
"#;

/// An address in a file-scope initialiser is a symbol **and how far into it**. `&table[4]` is that,
/// `&points[1].y` is that plus a member's offset, and both need the relocation's addend - without it
/// the only addresses a compiler can write are the ones that start an object.
///
/// The last three lines are the other half: `&((char*)0)[X]` is sqlite's `SQLITE_INT_TO_PTR`, an
/// integer made into a pointer. With a constant `X` there is no symbol at all, so the bytes go in
/// directly - and C99 6.5.3.2p3 is what makes the address of a dereference the pointer behind it.
#[test]
#[cfg(target_os = "windows")]
fn an_address_into_an_object_needs_an_addend() {
    let run = run_linked_with("c11", "addend", ADDEND_PROGRAM, &[]);
    assert_eq!(run.status.code(), Some(0), "it ran to completion");
    let out = String::from_utf8_lossy(&run.stdout);
    let expected = "4 a 10 def@@5 9 0@@";
    assert_eq!(out, expected.replace("@@", "\r\n"));
}

/// A universal character name in an identifier (C99 6.4.2.1): `caf\u00e9` and `café` are one
/// name, and an object written with one links against a use written with the other. That exercises
/// the interning of a spelling which is not a slice of the source, the COFF symbol table with bytes
/// outside ASCII in a name, and the linker's copy of that name.
#[test]
#[cfg(target_os = "windows")]
fn a_universal_character_name_in_an_identifier_links() {
    // The escape is a Rust *raw* string: `"\u00e9"` in an ordinary one is a Rust character, and the
    // compiler would never see the six bytes that spell it.
    let source = concat!(
        "int caf",
        r"\u00e9",
        " = 7;\nint f(void) { return café; }\nint main(void) { return f() - 7; }\n",
    );
    let run = run_linked_with("c99", "ucn-identifier", source, &[]);
    assert_eq!(run.status.code(), Some(0), "the two spellings are one name");
}

/// C99 5.1.2.2.3p1: reaching the `}` that ends `main` returns 0. That is not undefined behaviour, so
/// it must not be the trap this compiler used for every other function that falls off its end - and
/// it was: a program with no final `return 0` died of `ud2` at exit. toml-c's `example/table.c` has
/// exactly that shape, which is where the bug was found.
#[test]
#[cfg(target_os = "windows")]
fn main_that_falls_off_its_end_returns_zero() {
    let run = run_linked_with("c99", "main-falls-off", "int main(void) { }\n", &[]);
    assert_eq!(
        run.status.code(),
        Some(0),
        "the standard's answer, not a trap"
    );
}

/// C99 6.9p5: a program has one external definition of each name. Two units that define `f` - or an
/// object with an initialiser twice, or a function beside an object of one name - are refused, where
/// this linker used to keep whichever came first and build a program nobody wrote. Measured against
/// `clang`, which says "multiple definition of `f`".
///
/// The exemption is the standard's: a *tentative* definition, `int shared;` with no initialiser, is
/// not an external definition (6.9.2p2) and may appear in every unit, so two of them are one object
/// and the program runs. That is the one place this linker accepts what the `clang` beside it
/// refuses - its default is `-fno-common`, which makes each tentative a real definition.
#[test]
#[cfg(target_os = "windows")]
fn a_name_defined_in_two_units_is_refused_unless_both_definitions_are_tentative() {
    let mut compiler = Compile::new(Options {
        opt_level: 2,
        ..Options::default()
    });
    let mut duplicate = |units: &[(&str, &str)], name: &str| {
        let error = compiler.link(units).expect_err("it is a duplicate");
        let said = format!("{error:?}");
        assert!(
            said.contains(&format!(
                "`{name}` is defined in more than one translation unit"
            )),
            "{said}"
        );
    };
    duplicate(
        &[
            ("a.c", "int f(void) { return 1; }"),
            (
                "b.c",
                "int f(void) { return 2; }\nint main(void) { return f(); }",
            ),
        ],
        "f",
    );
    duplicate(
        &[
            ("a.c", "int shared = 1;"),
            ("b.c", "int shared = 2;\nint main(void) { return shared; }"),
        ],
        "shared",
    );
    // A tentative beside a real definition is a duplicate too, in either order.
    duplicate(
        &[
            ("a.c", "int shared;"),
            ("b.c", "int shared = 2;\nint main(void) { return shared; }"),
        ],
        "shared",
    );
    duplicate(
        &[
            ("a.c", "int shared = 2;"),
            ("b.c", "int shared;\nint main(void) { return shared; }"),
        ],
        "shared",
    );
    // A function and an object of one name: no declaration could have made those one name.
    duplicate(
        &[
            ("a.c", "int clash = 1;"),
            (
                "b.c",
                "int clash(void) { return 2; }\nint main(void) { return clash(); }",
            ),
        ],
        "clash",
    );

    // Two tentatives are one object only if they agree about how big it is: the merged object is
    // the first one's bytes, so a unit that declared the name larger would write past its end into
    // whatever `.data` put next.
    let error = compiler
        .link(&[
            ("a.c", "char buf[8];"),
            (
                "b.c",
                "char buf[64];
int main(void) { buf[0] = 1; return 0; }",
            ),
        ])
        .expect_err("the sizes disagree");
    assert!(
        format!("{error:?}").contains("`buf` is 8 bytes in one translation unit and 64 in another"),
        "{error:?}"
    );

    // Two tentatives of one size are one object, and the program links and runs.
    let run = run_units(
        "c99",
        "tentative",
        &[
            (
                "a.c",
                "int shared;\nint f(void);\nint main(void) { shared = 4; return f(); }",
            ),
            ("b.c", "int shared;\nint f(void) { return shared * 10; }"),
        ],
        &[],
    );
    assert_eq!(
        run.status.code(),
        Some(40),
        "the two tentatives are one object"
    );
}

/// A name the headers do not declare and the library does: `strnlen` is `ucrtbase`'s export and
/// C99's `<string.h>` says nothing about it, so the compiler's import list - which is what the
/// shipped headers declare - used to answer "undefined symbol" to a program that declared it
/// itself. `clang` links this and prints 3, which is what the test asserts.
#[test]
#[cfg(target_os = "windows")]
fn a_name_the_library_exports_links_even_without_a_header() {
    let source = "#include <stdio.h>\n#include <stddef.h>\n\nsize_t strnlen(const char *s, size_t maxlen);\n\n\
                  int main(void) {\n  printf(\"%d\\n\", (int)strnlen(\"hello\", 3));\n  return 0;\n}\n";
    let run = run_linked_with("c99", "strnlen", source, &[]);
    assert_eq!(run.status.code(), Some(0), "it ran");
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "3");
    // A name the library does *not* have is still refused, at link time, by name.
    let typo = source.replace("strnlen", "strnlenz");
    let mut compiler = Compile::new(Options {
        opt_level: 2,
        ..Options::default()
    });
    let error = compiler
        .link(&[("t.c", typo.as_str())])
        .expect_err("no such export");
    let said = format!("{error:?}");
    assert!(said.contains("undefined symbol"), "{said}");
}

/// The narrow atomics: `_Atomic char` and `_Atomic short`, signed ones included. The signed cases
/// are the point - `cmpxchg` writes one byte or two and leaves the rest of `rax` as it was, so a
/// result that is not extended back to its type reads as a large positive number. Every line is
/// what `clang` prints for this program.
#[test]
#[cfg(target_os = "windows")]
fn atomic_char_and_short_are_real_atomics() {
    let run = run_linked_with("c11", "narrow-atomics", NARROW_ATOMIC_PROGRAM, &[]);
    assert_eq!(run.status.code(), Some(0), "it ran to completion");
    let out = String::from_utf8_lossy(&run.stdout);
    let expected = "char 10 -> 15@@short 100 -> 120@@char cas 1 -> 99@@short cas fail 0 -> 120@@signed cas 1 -> 7@@signed fail 0 -> -300@@signed add -300 -> 0@@";
    assert_eq!(out, expected.replace("@@", "\r\n"));
}
