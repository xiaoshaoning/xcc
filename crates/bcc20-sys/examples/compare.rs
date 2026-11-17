//! Compile the same files twice and print the table: xcc to 64-bit COFF, the ported Borland
//! C++ 2.0 to 16-bit OMF.
//!
//! This is M9 of PLAN.md. It is a measuring stick, not a product: the numbers are how a change to
//! the front end gets checked against a 1991 implementation, and how the plan's successor argues
//! about code size instead of guessing.
//!
//!     cargo run -p bcc20-sys --example compare -- [--opt-level=N] [FILE.c]...
//!
//! Without files it compiles `corpus/*.c`. `BCC20_LIB` names `libbcc20.dll` when it is not on
//! `PATH`; without the library the xcc column is still printed, and the reason the other one is
//! missing is said out loud rather than left as an empty column.

use std::path::{Path, PathBuf};
use std::time::Instant;

use bcc20_sys::object::{coff_text, omf_text};
use bcc20_sys::{Compiler, Job};
use xcc::{Compile, Options};

fn main() {
    // `--opt-level=N` for xcc's half; Borland's is always the same compiler.
    let mut opt_level = 0u8;
    let mut files: Vec<PathBuf> = Vec::new();
    for argument in std::env::args().skip(1) {
        if let Some(level) = argument.strip_prefix("--opt-level=") {
            opt_level = level.parse().unwrap_or_else(|_| {
                eprintln!("compare: --opt-level wants a number");
                std::process::exit(2);
            });
            continue;
        }
        files.push(PathBuf::from(argument));
    }
    if files.is_empty() {
        files = corpus();
    }
    if files.is_empty() {
        eprintln!("compare: no source files and no corpus beside this example");
        std::process::exit(2);
    }

    let compiler = Compiler::load();
    if compiler.is_none() {
        eprintln!(
            "compare: no libbcc20 on this machine (set BCC20_LIB or put it on PATH): the DOS \
             column will be empty"
        );
    }

    println!(
        "{:<16} {:>8} {:>8} {:>8} {:>8} {:>6} {:>6}",
        "file", "xcc us", "xcc code", "dos us", "dos code", "xcc e", "dos e"
    );
    let (mut ours_time, mut ours_code) = (0u128, 0usize);
    let (mut theirs_time, mut theirs_code) = (0u128, 0usize);
    for path in &files {
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        let source = std::fs::read_to_string(path).unwrap_or_else(|error| {
            eprintln!("compare: cannot read {}: {error}", path.display());
            std::process::exit(2);
        });

        // xcc: what this compiler writes today.
        let started = Instant::now();
        let options = Options {
            opt_level,
            ..Options::default()
        };
        let artifact = Compile::new(options).compile(&name, &source);
        let elapsed = started.elapsed().as_micros();
        let (our_code, our_errors) = match &artifact {
            Ok(artifact) => (coff_text(artifact.object()).unwrap_or(0), 0),
            Err(diagnostics) => (0, diagnostics.items().len()),
        };

        // Borland: the same source, 1991's rules, through the port.
        let (their_code, their_errors, their_time) = match &compiler {
            Some(compiler) => {
                let dos = dos_name(&name);
                let job = Job::new(&dos, &source, &["-c", "-ml", "-O"]);
                let started = Instant::now();
                let outcome = compiler.compile(&job);
                let elapsed = started.elapsed().as_micros();
                let object = outcome
                    .outputs
                    .iter()
                    .find(|(file, _)| file.ends_with(".OBJ"))
                    .map(|(_, data)| omf_text(data).unwrap_or(0))
                    .unwrap_or(0);
                let errors = u32::from(!outcome.succeeded());
                (object, errors as usize, elapsed)
            }
            None => (0, 0, 0),
        };

        ours_time += elapsed;
        ours_code += our_code;
        theirs_time += their_time;
        theirs_code += their_code;
        println!(
            "{name:<16} {elapsed:>8} {our_code:>8} {their_time:>8} {their_code:>8} {our_errors:>6} {their_errors:>6}"
        );
    }
    println!(
        "{:<16} {:>8} {:>8} {:>8} {:>8}",
        "total", ours_time, ours_code, theirs_time, theirs_code
    );
    println!(
        "\nxcc code is the COFF .text section, in bytes; the DOS column is the OMF code segment, \
         in bytes,\nwhich is 16-bit code - not comparable byte for byte, and comparable in shape."
    );
}

/// The six files of the corpus, sorted, beside this example's crate.
fn corpus() -> Vec<PathBuf> {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&directory)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.extension().is_some_and(|extension| extension == "c"))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}

/// A DOS name for a source, as BCC wants it: uppercase, and eight characters plus an extension.
fn dos_name(name: &str) -> String {
    let stem: String = name
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .take(8)
        .collect::<String>()
        .to_ascii_uppercase();
    format!("{stem}.C")
}
