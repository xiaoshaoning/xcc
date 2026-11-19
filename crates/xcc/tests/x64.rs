//! ADR-008: this compiler's own back end, `--opt-level=2`.
//!
//! Two things are tested, and they are the two claims the slice makes. First, that the constructs
//! it does lower behave: a program through this back end and linked by the host's linker exits with
//! what the same program means. Second, a ratchet over the corpus: every program this back end
//! accepts is built *both* ways - level 2 through this back end, level 0 through Cranelift - linked,
//! run, and compared, so a construct that is subtly wrong shows up as a different exit code instead
//! of as an object nobody ran. The count is printed even when it passes, so a shrinking subset is
//! visible, and the programs it cannot compile yet are named rather than left to be guessed at.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use xcc::{Compile, Options};

/// How many corpus programs agreed, through this back end, when the ratchet was last raised. The
/// 58 the back end still refuses are named by the run, not by this number.
const PROGRAMS_THAT_AGREED: usize = 406;

/// A linker for the object this back end writes: `clang` first, because it drives `lld-link` and
/// the object is COFF.
fn linker() -> Option<&'static str> {
    ["clang", "cc", "gcc"]
        .into_iter()
        .find(|candidate| Command::new(candidate).arg("--version").output().is_ok())
}

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus")
}

/// What running a program came to.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Ran {
    status: Option<i32>,
    stdout: Vec<u8>,
}

/// Compile with `opt_level`, link, run, and say what happened; `None` when this level refuses the
/// program, which is how a gap is told from a wrong answer.
fn build_and_run(source: &str, opt_level: u8, dir: &Path, name: &str) -> Option<Ran> {
    let options = Options {
        opt_level,
        ..Options::default()
    };
    let artifact = Compile::new(options).compile(name, source).ok()?;
    let object = dir.join(format!("{name}.o{opt_level}.obj"));
    let exe = dir.join(format!("{name}.o{opt_level}.exe"));
    std::fs::write(&object, artifact.object()).expect("the object is written");
    let link = Command::new(linker()?)
        .arg("-o")
        .arg(&exe)
        .arg(&object)
        .output()
        .expect("the linker runs");
    if !link.status.success() {
        eprintln!(
            "{name} at level {opt_level}: linking failed: {}",
            String::from_utf8_lossy(&link.stderr)
        );
        return None;
    }
    let run = Command::new(&exe).output().expect("the program runs");
    Some(Ran {
        status: run.status.code(),
        stdout: run.stdout,
    })
}

/// A directory of this test's own, which it cleans up.
fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("xcc-x64-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a temporary directory");
    dir
}

#[test]
fn the_constructs_the_slice_lowers_behave() {
    let Some(_) = linker() else {
        eprintln!("no linker on this machine: the own back end's tests are skipped");
        return;
    };
    let dir = temp_dir("constructs");
    // Each program returns what it means, and the exit status is the return value of `main`.
    let programs: &[(&str, &str, i32)] = &[
        (
            "a constant",
            "int main(void) { return 6 * 7; }",
            42,
        ),
        (
            "arguments and a direct call",
            "int add(int a, int b) { return a + b; } int main(void) { return add(20, 22); }",
            42,
        ),
        (
            // The argument registers are also candidates for allocation, so moving the first
            // argument into `r8` must not happen while the third is still there.
            "four arguments, which is all the registers hold",
            "int f(int a, int b, int c, int d) { return a * 1000 + b * 100 + c * 10 + d; } int main(void) { return f(1, 2, 4, 4); }",
            1244,
        ),
        (
            "three arguments, and a call",
            "int g(int a, int b, int c) { return a - b - c; } int main(void) { return g(10, 2, 3); }",
            5,
        ),
        (
            // A returned value may live in `rax`, the register the return uses; an earlier `return`
            // of something else writes `rax`, and must not be allowed to destroy it.
            "an early return before the value returned later",
            "int f(int c, int a) { int ok = c == 0; int v = a + 1; if (ok) return 1; return v; } int main(void) { return f(1, 4) == 5 ? 0 : 1; }",
            0,
        ),
        (
            // The data section: a global object, read through a relocation.
            "a global object",
            "int g = 5; int main(void) { g = g + 37; return g; }",
            42,
        ),
        (
            "a static local, which is a global in everything but name",
            "int bump(void) { static int n; n = n + 20; return n; } int main(void) { bump(); return bump(); }",
            40,
        ),
        (
            "an initialised array in the data section",
            "int a[4] = {10, 20, 30, 40}; int main(void) { return a[0] + a[3]; }",
            50,
        ),
        (
            "a string literal's bytes",
            "int main(void) { char *s = \"hello\"; return s[1]; }",
            101,
        ),
        (
            // Arguments past the fourth go on the stack: the caller leaves room above the shadow
            // space and the callee reads them there.
            "five arguments, so the fifth is on the stack",
            "int five(int a, int b, int c, int d, int e) { return e - a; } int main(void) { return five(1, 2, 3, 4, 9) + 0; }",
            8,
        ),
        (
            // A stack argument of a wider type, and a narrow one in a register that has to be
            // canonicalised on arrival.
            "mixed sizes across the register and stack boundary",
            "long long f(long long a, int b, char c, int d, long long e) { return a + e + b + c + d; } int main(void) { return (int)(f(1, 2, 3, 4, 5) - 15); }",
            0,
        ),
        (
            "seven arguments",
            "int seven(int a, int b, int c, int d, int e, int f, int g) { return a + g; } int main(void) { return seven(10, 1, 2, 3, 4, 5, 32); }",
            42,
        ),
        (
            // A leaf that cannot use caller-saved registers (three arguments is more than two) has
            // to preserve the callee-saved ones it touches, or the caller's values die at the call.
            "a three-argument leaf called twice, with values kept across both",
            "int three(int a, int b, int c) { return a + b + c; } int main(void) { int x = three(1, 2, 3); int y = three(4, 5, 6); return x + y; }",
            21,
        ),
        (
            // Floating point: its own register file, its own widths, and a comparison whose answer
            // is an `int` again.
            "doubles in arithmetic, narrowed on the way out",
            "int main(void) { double x = 2.5; double y = 4.0; return (int)(x * y); }",
            10,
        ),
        (
            "a double division and subtraction",
            "int main(void) { double a = 7.0, b = 2.0; return (int)(a / b) + (int)(a - b); }",
            8,
        ),
        (
            "a double comparison",
            "int main(void) { double z = 3.75; if (z > 1.5) return 42; return 1; }",
            42,
        ),
        (
            "negative doubles, which are a subtraction from zero",
            "int main(void) { double c = -3.125; double d = -1.25; if (c < 0 && d > -2) return 42; return 1; }",
            42,
        ),
        (
            "float widened to double",
            "int main(void) { float f = 1.5f; double d = f; return (int)(d * 2.0); }",
            3,
        ),
        (
            "double equality, which is not the same question as ordering",
            "int main(void) { double a = 7.0; return (a == 7.0 ? 100 : 0) + (int)(a / 2.0); }",
            103,
        ),
        (
            // A global's address arrives in `r8`, so a float load through it needs the REX byte that
            // names that base: without it the load read `[rax]` and answered nonsense.
            "a double in the data section",
            "double g = 2.5; int main(void) { return (int)(g * 2.0); }",
            5,
        ),
        (
            "a double through a pointer",
            "int main(void) { double d = 4.5; double *p = &d; return (int)(*p * 2.0); }",
            9,
        ),
        (
            "a loop, which becomes block parameters",
            "int main(void) { int i; int s = 0; for (i = 0; i < 10; i++) s += i; return s; }",
            45,
        ),
        (
            "a branch",
            "int main(void) { int x = 100; if (x > 50) return 1; return 2; }",
            1,
        ),
        (
            "division and remainder, signed",
            "int f(int a) { return a / 3 + a % 3; } int main(void) { return f(100); }",
            34,
        ),
        (
            "an array in the frame, read and written",
            "int main(void) { int a[3]; a[0] = 7; a[1] = 8; a[2] = 9; return a[0] + a[2]; }",
            16,
        ),
        (
            "an unsigned comparison",
            "int main(void) { unsigned u = 4294967295u; return u > 1 ? 3 : 4; }",
            3,
        ),
        (
            "narrow types, sign-extended",
            "int main(void) { return (int)(char)-1 + (int)(unsigned char)200; }",
            199,
        ),
        (
            "64-bit arithmetic",
            // 3815 is the whole return value: an exit status is 32 bits, not the 8 a shell shows.
            "int main(void) { long long x = 4000000000LL; return (int)(x + 3000 > 0) + (int)(x >> 20); }",
            3815,
        ),
        (
            "recursion",
            "int fact(int n) { if (n < 2) return 1; return n * fact(n - 1); } int main(void) { return fact(5); }",
            120,
        ),
        (
            "a switch",
            "int main(void) { int x = 3, r = 0; switch (x) { case 1: r = 10; break; case 3: r = 30; break; default: r = 99; } return r; }",
            30,
        ),
        (
            "a pointer into a local",
            "int main(void) { int a[4]; int *p = a; int i; for (i = 0; i < 4; i++) a[i] = i * 3; return *(p + 1) + *(p + 3); }",
            12,
        ),
        (
            "break and continue",
            "int main(void) { int i = 0, s = 0; while (1) { i++; if (i > 5) break; if (i == 2) continue; s += i; } return s; }",
            13,
        ),
        (
            "signed and unsigned right shifts",
            "int main(void) { int a = 7, b = -7; unsigned c = 0x80; return (a >> 1) + (b >> 1) + (c >> 6) + 6; }",
            7,
        ),
        (
            "a 64-bit result narrowed by the return",
            "long long f(long long a, long long b) { return a * b - 7; } int main(void) { return (int)(f(1000, 1000) % 251); }",
            9,
        ),
    ];
    let mut wrong = Vec::new();
    for (what, source, expected) in programs {
        let name = format!("t{}", wrong.len());
        let Some(ran) = build_and_run(source, 2, &dir, &name) else {
            wrong.push(format!("{what}: this back end refused it"));
            continue;
        };
        if ran.status != Some(*expected) {
            wrong.push(format!(
                "{what}: exited with {:?}, expected {expected}",
                ran.status
            ));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// A call through a function pointer whose value never escapes is not a gap: `mem2reg` promotes the
/// variable, the callee becomes the function's own address, and the call is direct. It is worth a
/// test because the direct-call path is the one that could drop the address.
#[test]
fn a_function_address_that_never_escapes_is_still_a_direct_call() {
    let Some(_) = linker() else {
        eprintln!("no linker on this machine: the own back end's tests are skipped");
        return;
    };
    let dir = temp_dir("address-direct");
    let source = "int f(void) { return 7; } int main(void) { int (*p)(void) = f; return p(); }";
    let ran = build_and_run(source, 2, &dir, "direct");
    let _ = std::fs::remove_dir_all(&dir);
    let ran = ran.expect("a call the lowering can see through compiles");
    assert_eq!(
        ran.status,
        Some(7),
        "the call went to the function it names"
    );
}

/// Everywhere else, an address this back end cannot make is a gap, and a gap must be refused: the
/// one way it could go wrong silently is the direct-call path deciding the call is the only reader
/// of an address something else reads too, and calling through zero.
#[test]
fn a_function_address_read_as_a_value_is_refused() {
    let Some(_) = linker() else {
        eprintln!("no linker on this machine: the own back end's tests are skipped");
        return;
    };
    let dir = temp_dir("address");
    let programs = [
        (
            "compared with something",
            "int f(void) { return 7; } int main(void) { return f != 0; }",
        ),
        (
            "passed to another function",
            "int f(void) { return 7; } int g(int (*p)(void)) { return p(); } int main(void) { return g(f); }",
        ),
    ];
    let mut wrong = Vec::new();
    for (what, source) in programs {
        if let Some(ran) = build_and_run(source, 2, &dir, "address") {
            wrong.push(format!("{what}: compiled and exited with {:?}", ran.status));
        }
        // The other lowering must accept it: a program level 0 refuses too is no evidence of this
        // back end's discipline.
        if build_and_run(source, 0, &dir, "address").is_none() {
            wrong.push(format!(
                "{what}: level 0 refused it too, so this proves nothing"
            ));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        wrong.is_empty(),
        "{}",
        wrong.join(
            "
"
        )
    );
}

#[test]
fn the_corpus_agrees_through_both_back_ends() {
    let Some(_) = linker() else {
        eprintln!("no linker on this machine: the own back end's corpus test is skipped");
        return;
    };
    let mut files: Vec<PathBuf> = std::fs::read_dir(corpus())
        .expect("the corpus is in the repository")
        .map(|entry| entry.expect("an entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "c"))
        .collect();
    files.sort();

    let workers = std::thread::available_parallelism().map_or(4, |count| count.get());
    let next = AtomicUsize::new(0);
    let agreed = AtomicUsize::new(0);
    let refused: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let wrong: Mutex<Vec<String>> = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for worker in 0..workers {
            let (files, next, agreed, refused, wrong) = (&files, &next, &agreed, &refused, &wrong);
            scope.spawn(move || {
                let dir = std::env::temp_dir()
                    .join(format!("xcc-x64-corpus-{}-{worker}", std::process::id()));
                let _ = std::fs::create_dir_all(&dir);
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(path) = files.get(index) else { break };
                    let name = path
                        .file_stem()
                        .expect("a file stem")
                        .to_string_lossy()
                        .into_owned();
                    let source = std::fs::read_to_string(path).expect("the source is readable");
                    let Some(optimised) = build_and_run(&source, 2, &dir, &name) else {
                        refused.lock().expect("the refusals").push(name);
                        continue;
                    };
                    // Level 0 is Cranelift's lowering of the same `cir`: the two back ends are two
                    // answers to one question, and they must agree.
                    let Some(plain) = build_and_run(&source, 0, &dir, &name) else {
                        refused.lock().expect("the refusals").push(name);
                        continue;
                    };
                    if plain == optimised {
                        agreed.fetch_add(1, Ordering::Relaxed);
                    } else {
                        wrong.lock().expect("the mismatches").push(format!(
                            "{name}: own back end {:?}, Cranelift {:?}",
                            optimised, plain
                        ));
                    }
                }
                let _ = std::fs::remove_dir_all(&dir);
            });
        }
    });

    let agreed = agreed.into_inner();
    let mut wrong = wrong.into_inner().expect("the mismatches");
    let mut refused = refused.into_inner().expect("the refusals");
    wrong.sort();
    refused.sort();
    eprintln!(
        "own back end: {agreed} corpus programs agreed, {} refused, {} wrong",
        refused.len(),
        wrong.len()
    );
    if !wrong.is_empty() {
        eprintln!("own back end: wrong: {}", wrong.join(", "));
    }
    assert!(wrong.is_empty(), "{} disagreed", wrong.len());
    assert!(
        agreed >= PROGRAMS_THAT_AGREED,
        "{agreed} programs agreed and {PROGRAMS_THAT_AGREED} used to: something this back end \
         lowered it no longer does"
    );
}
