//! M9's acceptance: compile the six-file corpus both ways and report a table.
//!
//! It is a test rather than a printed report because the numbers are only worth anything if they
//! keep being produced: a change that breaks the DOS comparison should fail here, in the same run
//! as everything else. The table itself is `examples/compare.rs`.

use bcc20_sys::object::{coff_text, omf_text};
use bcc20_sys::{Compiler, Job};
use xcc::{Compile, Options};

/// The six files, sorted, as `(name, source)`.
fn corpus() -> Vec<(String, String)> {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("corpus");
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&directory)
        .expect("the corpus directory")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "c"))
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|path| {
            let name = path
                .file_name()
                .expect("a file name")
                .to_string_lossy()
                .into_owned();
            let source = std::fs::read_to_string(&path).expect("a UTF-8 source");
            (name, source)
        })
        .collect()
}

/// `bcc20.h` is explicit: "not reentrant: one compile at a time per process".
static TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn the_six_files_compile_with_both_and_report_their_sizes() {
    let files = corpus();
    assert_eq!(files.len(), 6, "the measuring stick's corpus is six files");
    let Some(compiler) = Compiler::load() else {
        eprintln!("no libbcc20 on this machine (set BCC20_LIB or put it on PATH): M9's table cannot be made");
        return;
    };
    let _turn = TURN.lock().expect("the compiler turn");

    let mut rows = Vec::new();
    for (name, source) in &files {
        // xcc's half: 64-bit COFF, one .text section.
        let artifact = Compile::new(Options::default())
            .compile(name, source)
            .unwrap_or_else(|diagnostics| panic!("{name}: {diagnostics:?}"));
        let our_code = coff_text(artifact.object())
            .unwrap_or_else(|| panic!("{name}: no .text in the COFF object"));

        // Borland's half: 16-bit OMF, one code segment.
        let dos = format!(
            "{}.C",
            name.chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .take(8)
                .collect::<String>()
                .to_ascii_uppercase()
        );
        let job = Job::new(&dos, source, &["-c", "-ml", "-O"]);
        let outcome = compiler.compile(&job);
        assert!(outcome.succeeded(), "{name}: {}", outcome.console);
        let object = outcome
            .outputs
            .iter()
            .find(|(file, _)| file.ends_with(".OBJ"))
            .map(|(_, data)| data)
            .unwrap_or_else(|| panic!("{name}: no object written"));
        let their_code = omf_text(object)
            .unwrap_or_else(|| panic!("{name}: no code segment in {} bytes", object.len()));

        assert!(
            our_code > 0 && their_code > 0,
            "{name}: an empty code section"
        );
        rows.push((name.clone(), our_code, their_code));
    }

    // The report the plan asks for, printed so a run of the suite shows the state of the stick.
    println!("\n{:<16} {:>10} {:>10}", "file", "xcc x64", "bcc20 x86");
    for (name, ours, theirs) in &rows {
        println!("{name:<16} {ours:>10} {theirs:>10}");
    }
    let ours: usize = rows.iter().map(|(_, ours, _)| ours).sum();
    let theirs: usize = rows.iter().map(|(_, _, theirs)| theirs).sum();
    println!("{:<16} {ours:>10} {theirs:>10}", "total");
    assert_eq!(rows.len(), 6);
}
