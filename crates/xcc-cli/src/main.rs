//! The `xcc` command line compiler.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{ArgAction, Parser, ValueEnum};
use xcc::{Compile, Options, SourceFile};

#[derive(Parser, Debug)]
#[command(
    name = "xcc",
    version,
    about = "A small, embeddable C compiler for Windows x86-64",
    disable_help_subcommand = true
)]
struct Args {
    /// Compile only; do not link. The only mode until the linker lands (M5).
    #[arg(short = 'c')]
    compile_only: bool,

    /// Preprocess only: print the token stream, one token after another.
    #[arg(short = 'E')]
    preprocess_only: bool,

    /// Write the object here instead of next to the source.
    #[arg(short = 'o', value_name = "PATH")]
    output: Option<PathBuf>,

    /// How to print diagnostics.
    #[arg(long, value_enum, default_value_t = FormatArg::Human)]
    format: FormatArg,

    /// Look for angled `#include`s here, as well as beside the including file.
    #[arg(short = 'I', value_name = "DIR", action = ArgAction::Append)]
    include_paths: Vec<PathBuf>,

    /// Define a macro, as `NAME` or `NAME=VALUE`.
    #[arg(short = 'D', value_name = "MACRO", action = ArgAction::Append)]
    defines: Vec<String>,

    /// Undefine a macro, after the `-D`s have been applied.
    #[arg(short = 'U', value_name = "NAME", action = ArgAction::Append)]
    undefines: Vec<String>,

    /// C source files.
    #[arg(value_name = "FILE", required = true)]
    inputs: Vec<PathBuf>,
}

/// The diagnostic shapes the command line offers.
///
/// `xcc::Format` is the source of truth; this exists only to keep `clap` out of the library,
/// and the `From` impl below is the one place the two are mapped.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum FormatArg {
    /// The line of source, with a caret under what is wrong.
    Human,
    /// `t.c(3,7): error: ...`, for MSBuild and Visual Studio.
    Msvc,
    /// `t.c:3:7: error: ...`, for GCC-style tools.
    Gcc,
}

impl From<FormatArg> for xcc::Format {
    fn from(format: FormatArg) -> Self {
        match format {
            FormatArg::Human => xcc::Format::Human,
            FormatArg::Msvc => xcc::Format::Msvc,
            FormatArg::Gcc => xcc::Format::Gcc,
        }
    }
}

fn main() -> ExitCode {
    // parse_from wants argv[0]: the program's own name stays in front of the arguments
    let mut argv: Vec<OsString> = std::env::args_os().collect();
    let program = argv.remove(0);
    let mut full = vec![program];
    match expand_response_files(argv.into_iter()) {
        Ok(expanded) => full.extend(expanded),
        Err(message) => {
            eprintln!("xcc: {message}");
            return ExitCode::from(2);
        }
    }
    let args = Args::parse_from(full);

    if !args.compile_only && !args.preprocess_only {
        eprintln!("xcc: linking is not implemented yet (PLAN.md, M5); pass -c to compile only");
        return ExitCode::from(2);
    }
    if args.output.is_some() && args.inputs.len() > 1 {
        eprintln!("xcc: -o with more than one source is not supported yet");
        return ExitCode::from(2);
    }

    let format = xcc::Format::from(args.format);
    let mut status = ExitCode::SUCCESS;

    for input in &args.inputs {
        let name = input.display().to_string();
        let text = match std::fs::read_to_string(input) {
            Ok(text) => text,
            Err(error) => {
                eprintln!("xcc: cannot read {name}: {error}");
                return ExitCode::from(2);
            }
        };

        let source = SourceFile::new(name.clone(), text.clone());
        let options = Options {
            defines: args.defines.clone(),
            undefines: args.undefines.clone(),
            ..Options::default()
        };
        let includes = Box::new(Directories {
            roots: args.include_paths.clone(),
        });
        let mut compiler = Compile::with_includes(options, includes);
        if args.preprocess_only {
            match compiler.preprocess(&name, &text) {
                Ok(preprocessed) => print_spellings(&preprocessed),
                Err(diagnostics) => {
                    eprintln!("{}", xcc::render_all(&diagnostics, &[source], format));
                    status = ExitCode::from(1);
                }
            }
            continue;
        }
        match compiler.compile(&name, &text) {
            Ok(artifact) => {
                let path = args
                    .output
                    .clone()
                    .unwrap_or_else(|| input.with_extension("obj"));
                if let Err(error) = std::fs::write(&path, artifact.object()) {
                    eprintln!("xcc: cannot write {}: {error}", path.display());
                    return ExitCode::from(2);
                }
                if !artifact.diagnostics().is_empty() {
                    eprintln!(
                        "{}",
                        xcc::render_all(artifact.diagnostics(), &[source], format)
                    );
                }
            }
            Err(diagnostics) => {
                eprintln!("{}", xcc::render_all(&diagnostics, &[source], format));
                status = ExitCode::from(1);
            }
        }
    }

    status
}

/// `-E`: the tokens, whitespace between them, wrapped so a person can read it.
fn print_spellings(preprocessed: &xcc::Preprocessed) {
    let mut line = String::new();
    for token in &preprocessed.tokens {
        if token.kind == xcc::TokenKind::Eof {
            continue;
        }
        let spelling = preprocessed.text(token);
        if line.len() + spelling.len() + 1 > 96 {
            println!("{line}");
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(spelling);
    }
    if !line.is_empty() {
        println!("{line}");
    }
}

/// The files `#include` may read: the path as written, then under each `-I` directory.
#[derive(Debug)]
struct Directories {
    roots: Vec<PathBuf>,
}

impl xcc::Includes for Directories {
    fn read(&self, path: &str) -> Option<String> {
        if let Ok(text) = std::fs::read_to_string(path) {
            return Some(text);
        }
        for root in &self.roots {
            if let Ok(text) = std::fs::read_to_string(root.join(path)) {
                return Some(text);
            }
        }
        None
    }
}

/// Replace `@file` arguments with the words in that file, as a response file.
///
/// One level only, split on ASCII whitespace, which is what a build system writes. Fails with
/// a message for the caller to print: a helper that ends the process cannot be tested on its
/// own, and hides the exit from whoever reads the call.
fn expand_response_files(args: impl Iterator<Item = OsString>) -> Result<Vec<OsString>, String> {
    let mut out = Vec::new();
    for arg in args {
        let text = arg.to_string_lossy().into_owned();
        let Some(path) = text.strip_prefix('@') else {
            out.push(arg);
            continue;
        };
        match std::fs::read_to_string(path) {
            Ok(contents) => out.extend(contents.split_whitespace().map(OsString::from)),
            Err(error) => return Err(format!("cannot read response file {path}: {error}")),
        }
    }
    Ok(out)
}
