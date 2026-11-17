//! The M6 acceptance item: the corpus compiled *and run*, xcc against `clang -O0`.
//!
//! This is the gate that asks what a program *does*, not what a compiler says about it. Every
//! corpus program both compilers accept is run as a process - `xcc --run file.c` on one side,
//! `clang -O0` and its executable on the other - and the exit status and stdout are compared.
//! Everything is a subprocess, so a crash in one program is a result for that program and not the
//! end of the run, and a hang is killed after a few seconds and reported as a difference.
//!
//! What is left out, and why: a program that calls a variadic function (`printf`; ADR-006) is
//! *refused* by xcc, and a refusal is not a behavioural difference - it is a recorded gap,
//! counted and printed here. Both compilers read the same headers (`-nostdinc` and the ones xcc
//! ships), so an `#include` compares like anything else. A program `clang` itself cannot build is
//! skipped: the corpus holds programs that are deliberately not C99, and this test is about the
//! ones both compilers claim.
//!
//! The count is printed even when the test passes, so a shrinking subset is visible.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How long one program may run. Programs in the corpus finish in microseconds; this is the
/// difference between "slow" and "does not terminate".
const TIME_LIMIT: Duration = Duration::from_secs(10);

/// Files that have to disagree, and the reason. Every entry is a reason, never a count (the
/// preprocessor gate's rule). Empty: everything in the subset agrees.
const EXPECTED_MISMATCHES: &[(&str, &str)] = &[];

/// How many programs ran when this was last measured. A ratchet: it goes up when a gap closes
/// (headers, `printf`) and never down, because a drop means something that used to run is now
/// refused, which is the regression this test exists to catch.
const PROGRAMS_THAT_RAN: usize = 412;

/// How many programs xcc refused and `clang` built, when this was last measured. A ratchet in the
/// other direction: the gap may shrink, and never grow. Which programs they are, and why each is
/// refused, is `crates/xcc/tests/parse.rs`'s table - this only counts them.
const AT_MOST_REFUSED: usize = 8;

/// How the subset split.
#[derive(Default)]
struct Counts {
    /// Programs both compilers built and ran.
    ran: usize,
    /// Programs xcc would not compile *and* `clang` would: a recorded gap of ours, not a
    /// difference. A program neither would build is `skipped`.
    refused: usize,
    /// Programs outside the subset: `clang` will not build them, with or without our headers.
    skipped: usize,
    /// Programs that ran out of time in *both* compilers: the machine was busy, or the program
    /// does not terminate under either. Counted apart so a loaded machine cannot fail the ratchet,
    /// and printed, so a silent rise is visible.
    timed_out: usize,
}

/// What running a program came to.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
    /// It exited with this status.
    Exited(i32),
    /// It died with this Windows exception code (`ud2`, a bad address, a stack overflow).
    Crashed(u32),
    /// It was still running when the time limit came.
    Hung,
}

impl std::fmt::Display for Outcome {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Outcome::Exited(status) => write!(out, "exit {status}"),
            Outcome::Crashed(code) => write!(out, "crash {code:#x}"),
            Outcome::Hung => write!(out, "hung"),
        }
    }
}

/// A process's outcome, from its status: on Windows an abnormal end is an `NTSTATUS`, which is
/// above every normal exit code, and `code()` hands it over as one.
fn outcome(status: ExitStatus) -> Outcome {
    match status.code() {
        Some(code) if (code as u32) < 0xC000_0000 => Outcome::Exited(code),
        Some(code) => Outcome::Crashed(code as u32),
        None => Outcome::Crashed(0),
    }
}

/// Run `command`, waiting at most [`TIME_LIMIT`], and give its outcome and its output.
///
/// The pipes are drained by threads *while* the program runs: a program that writes more than
/// a pipe buffer would block on its own output, miss the time limit, and be reported as a hang.
fn run(mut command: Command) -> (Outcome, Vec<u8>, Vec<u8>) {
    let started = Instant::now();
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the program to run");
    let out = drain(child.stdout.take());
    let err = drain(child.stderr.take());
    let outcome = loop {
        match child.try_wait() {
            Ok(Some(status)) => break outcome(status),
            Ok(None) if started.elapsed() > TIME_LIMIT => break Outcome::Hung,
            Ok(None) => std::thread::sleep(Duration::from_millis(2)),
            Err(error) => panic!("waiting for a program: {error}"),
        }
    };
    if outcome == Outcome::Hung {
        let _ = child.kill();
    }
    let _ = child.wait();
    (outcome, collect(out), collect(err))
}

/// Read one of the child's pipes to its end on a thread of its own.
fn drain<P: std::io::Read + Send + 'static>(pipe: Option<P>) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut buffer = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = std::io::Read::read_to_end(&mut pipe, &mut buffer);
        }
        buffer
    })
}

fn collect(reader: std::thread::JoinHandle<Vec<u8>>) -> Vec<u8> {
    reader.join().unwrap_or_default()
}

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus")
}

/// The headers xcc ships: what a corpus program's `#include` resolves to for both compilers.
fn headers() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../xcc/include")
}

fn clang_exists() -> bool {
    Command::new("clang")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

/// Run one side, and once more if it timed out: a program that hung while the machine was running
/// forty of them at once is a measurement about the machine, and a real hang hangs twice. Two
/// clean runs and one full of false timeouts is exactly the flakiness this avoids.
fn again_if_hung<T>(mut attempt: impl FnMut() -> Option<(Outcome, T)>) -> Option<(Outcome, T)> {
    let first = attempt()?;
    if first.0 != Outcome::Hung {
        return Some(first);
    }
    attempt()
}

/// The program compiled by xcc at `opt_level` and run in process, or `None` when xcc refused it -
/// which is how a refusal is told from a program that exits: the refusal is a diagnostic on stderr.
fn xcc_runs(source: &Path, opt_level: u8) -> Option<(Outcome, Vec<u8>)> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_xcc"));
    command
        .arg("--run")
        .arg(format!("--opt-level={opt_level}"))
        .arg(source);
    let (outcome, out, err) = run(command);
    if err.is_empty() {
        Some((outcome, out))
    } else {
        None
    }
}

/// The program compiled by `clang -O0` and run, or `None` when clang will not build it.
fn clang_runs(source: &Path, exe: &Path) -> Option<(Outcome, Vec<u8>)> {
    // `-nostdinc` plus our own headers: the two compilers must read the same declarations, or a
    // program that includes one would be comparing two different programs. It also means every
    // header xcc ships is checked by a real compiler the moment a corpus program includes it.
    let built = Command::new("clang")
        .args(["-O0", "-std=c99", "-w", "-nostdinc", "-isystem"])
        .arg(headers())
        .arg("-o")
        .arg(exe)
        .arg(source)
        .output()
        .expect("clang to run");
    if !built.status.success() {
        return None;
    }
    let mut command = Command::new(exe);
    command.stdin(Stdio::null());
    let (outcome, out, _) = run(command);
    Some((outcome, out))
}

#[test]
fn the_corpus_runs_like_clang() {
    if !clang_exists() {
        eprintln!("differential: no clang on PATH, so nothing to compare against");
        return;
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(corpus())
        .expect("the corpus is in the repository")
        .map(|entry| entry.expect("an entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "c"))
        .collect();
    files.sort();

    // One compilation and one executable per worker: `clang` is the slow half, and the corpus
    // takes minutes in one thread.
    let workers = std::thread::available_parallelism().map_or(4, |count| count.get());
    let next = AtomicUsize::new(0);
    let counts = Mutex::new(Counts::default());
    let refused: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let mismatched: Mutex<Vec<String>> = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for worker in 0..workers {
            let (files, next, counts, mismatched, refused) =
                (&files, &next, &counts, &mismatched, &refused);
            scope.spawn(move || {
                let exe = std::env::temp_dir().join(format!(
                    "xcc-differential-{}-{worker}.exe",
                    std::process::id()
                ));
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(path) = files.get(index) else {
                        break;
                    };
                    let name = path
                        .file_name()
                        .expect("a file name")
                        .to_string_lossy()
                        .into_owned();
                    // Both lowerings, because `--opt-level=1` is a path a host picks and the plan
                    // asks for a harness that compiles with both: one that disagrees with the
                    // other is a bug in whichever moved the code, whether or not clang notices.
                    let plain = again_if_hung(|| xcc_runs(path, 0));
                    let optimised = again_if_hung(|| xcc_runs(path, 1));
                    let theirs = again_if_hung(|| clang_runs(path, &exe));
                    let built_by_clang = theirs.is_some();
                    if plain.is_some() != optimised.is_some() {
                        mismatched.lock().expect("the mismatches").push(format!(
                            "{name}: xcc compiled it at {}, and not at {}",
                            if plain.is_some() { 0 } else { 1 },
                            if plain.is_some() { 1 } else { 0 }
                        ));
                        continue;
                    }
                    let (Some((ours, our_output)), Some((theirs, their_output))) = (plain, theirs)
                    else {
                        // One of them would not build it. Which one matters: a program `clang`
                        // cannot build is outside the subset - the corpus holds programs that are
                        // not C99 on purpose, and ones that need headers nobody has - while a
                        // program only *xcc* refuses is a gap of ours, and is counted as one.
                        // Asking both, not just xcc first, is what tells them apart.
                        if !built_by_clang {
                            counts.lock().expect("the counts").skipped += 1;
                        } else {
                            counts.lock().expect("the counts").refused += 1;
                            refused.lock().expect("the refusals").push(name.clone());
                        }
                        continue;
                    };
                    let (optimised, optimised_output) = optimised.expect("both compiled");
                    if ours == Outcome::Hung && theirs == Outcome::Hung {
                        // Neither finished, twice: not a disagreement about a value. A program
                        // that hangs in one and not the other is still a mismatch below.
                        counts.lock().expect("the counts").timed_out += 1;
                        continue;
                    }
                    counts.lock().expect("the counts").ran += 1;
                    let known = EXPECTED_MISMATCHES.iter().any(|(file, _)| *file == name);
                    // Three answers, and every pair of them must be the same one: the two
                    // lowerings, and the compiler this project is defined against.
                    let answers: [(&str, &Outcome, &Vec<u8>); 3] = [
                        ("opt-level=0", &ours, &our_output),
                        ("opt-level=1", &optimised, &optimised_output),
                        ("clang", &theirs, &their_output),
                    ];
                    if !known {
                        for left in 0..answers.len() {
                            for right in left + 1..answers.len() {
                                let (a_name, a_outcome, a_output) = answers[left];
                                let (b_name, b_outcome, b_output) = answers[right];
                                if a_outcome == b_outcome && a_output == b_output {
                                    continue;
                                }
                                let mut note =
                                    format!("{name}: {a_name} {a_outcome}, {b_name} {b_outcome}");
                                if a_output != b_output {
                                    note.push_str(&format!(
                                        ", output {:?} against {:?}",
                                        String::from_utf8_lossy(a_output),
                                        String::from_utf8_lossy(b_output)
                                    ));
                                }
                                mismatched.lock().expect("the mismatches").push(note);
                            }
                        }
                    }
                }
                let _ = std::fs::remove_file(&exe);
            });
        }
    });

    let counts = counts.lock().expect("the counts");
    let mut mismatched = mismatched.into_inner().expect("the mismatches");
    mismatched.sort();
    eprintln!(
        "differential: {} ran, {} refused, {} skipped, {} timed out, {} mismatched",
        counts.ran,
        counts.refused,
        counts.skipped,
        counts.timed_out,
        mismatched.len()
    );
    if !refused.lock().expect("the refusals").is_empty() {
        eprintln!(
            "differential: refused what clang builds: {}",
            refused.lock().expect("the refusals").join(", ")
        );
    }
    assert!(
        counts.refused <= AT_MOST_REFUSED,
        "{} programs are refused that clang builds, and {AT_MOST_REFUSED} used to be: something          refuses code it used to compile",
        counts.refused
    );
    assert!(
        counts.ran + counts.timed_out >= PROGRAMS_THAT_RAN,
        "{} programs ran (and {} timed out), and {PROGRAMS_THAT_RAN} used to: something refuses          code that ran",
        counts.ran,
        counts.timed_out
    );
    assert!(
        mismatched.is_empty(),
        "the corpus behaves differently from clang -O0:\n{}",
        mismatched.join("\n")
    );
}
