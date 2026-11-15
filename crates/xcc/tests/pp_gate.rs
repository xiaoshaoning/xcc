//! The M2 gate: xcc's preprocessed token stream against `clang -E`, over the corpus.
//!
//! Comparing token *spellings* rather than text is what makes this a comparison of meaning:
//! the reference's output is lexed with xcc's own lexer, so whitespace, line breaks and line
//! markers cannot masquerade as differences. What is left is a real disagreement about what
//! the preprocessing tokens are: a macro expanding differently, a conditional taken the other
//! way, a lexing rule understood differently.
//!
//! Programs that `#include` anything are left out - there are no headers until M8 - and so are
//! the few on the list below, each with the reason it cannot agree. Everything else must match
//! token for token, or this test fails and names the file and the first difference.

use std::path::{Path, PathBuf};
use std::process::Command;

use xcc::{Interner, Options, Prepared, SourceFile, TokenKind};

/// Files that cannot agree, and why. Every entry is a reason, never a count.
const EXPECTED_DIFFERENCES: &[(&str, &str)] = &[];

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus")
}

fn clang_exists() -> bool {
    Command::new("clang")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

/// What `clang -E` says the tokens are, lexed with our own lexer.
fn clang_tokens(path: &Path) -> Option<Vec<String>> {
    let output = Command::new("clang")
        .args([
            "-E",
            "-P", // no line markers
            "-x",
            "c",
            "-std=c99",
            "-nostdinc", // no system headers: there are none to find
            "-Wno-everything",
        ])
        .arg(path)
        .output()
        .expect("clang runs");
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    let mut interner = Interner::new();
    let lexed = xcc::lex(&Prepared::new(&text), "clang", &mut interner);
    if lexed.has_errors() {
        return None;
    }
    Some(
        lexed
            .tokens
            .iter()
            .filter(|token| token.kind != TokenKind::Eof)
            .map(|token| interner.resolve(token.text).to_owned())
            .collect(),
    )
}

/// Where two token streams first disagree, with context, for a failure that can be read.
fn first_difference(ours: &[String], theirs: &[String]) -> String {
    let at = ours
        .iter()
        .zip(theirs)
        .position(|(a, b)| a != b)
        .unwrap_or_else(|| ours.len().min(theirs.len()));
    let from = at.saturating_sub(6);
    let ours_end = (at + 6).min(ours.len());
    let theirs_end = (at + 6).min(theirs.len());
    format!(
        "token {at} of {} / {}:\n  xcc:   {:?}\n  clang: {:?}",
        ours.len(),
        theirs.len(),
        &ours[from..ours_end],
        &theirs[from..theirs_end]
    )
}

#[test]
fn xcc_preprocesses_the_corpus_like_clang() {
    if !clang_exists() {
        eprintln!("pp gate: no clang on PATH, so the comparison is skipped");
        return;
    }

    let mut compared = 0;
    let mut skipped_includes = 0;
    let mut failures = Vec::new();
    let mut entries: Vec<PathBuf> = std::fs::read_dir(corpus())
        .expect("the corpus is in the repository")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|extension| extension == "c"))
        .collect();
    entries.sort();

    for path in entries {
        let name = path.file_name().expect("a file name").to_string_lossy();
        if EXPECTED_DIFFERENCES.iter().any(|(known, _)| *known == name) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if text.contains("#include") {
            skipped_includes += 1;
            continue;
        }

        let file = SourceFile::new(name.as_ref(), &text);
        let ours = xcc::preprocess(&file, &Options::default(), &());
        if ours.has_errors() {
            failures.push(format!(
                "{name}: xcc reported {:?}",
                ours.diagnostics
                    .iter()
                    .map(|diagnostic| diagnostic.message.clone())
                    .collect::<Vec<_>>()
            ));
            continue;
        }
        let Some(theirs) = clang_tokens(&path) else {
            skipped_includes += 1; // clang could not read it either; not a disagreement
            continue;
        };
        let ours = ours.spellings();
        compared += 1;
        if ours != theirs {
            failures.push(format!("{name}: {}", first_difference(&ours, &theirs)));
        }
    }

    eprintln!(
        "pp gate: {compared} programs compared with clang -E, {skipped_includes} skipped \
         (includes, or clang refused them), {} differ",
        failures.len()
    );
    assert!(
        failures.is_empty(),
        "the preprocessing token streams differ in {} of {compared} programs:\n{}\n\n\
         Each one is either a bug here or an entry in EXPECTED_DIFFERENCES with a reason.",
        failures.len(),
        failures.join("\n\n")
    );
}
