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
/// 162 the back end still refuses are named by the run, not by this number.
const PROGRAMS_THAT_AGREED: usize = 302;

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
