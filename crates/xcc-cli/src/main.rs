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

    /// Run the source now: compile it in process, call `main`, and exit with what it returns.
    ///
    /// The embedding API, used the way a host would: the same options a compile takes, minus `-I`,
    /// because `#include` is answered from the headers inside this binary and not from a filesystem.
    /// `main` is called as `int main(int, char **)` whether it was written that way or
    /// `int main(void)`, with one argument - the source's name. A program that needs its own
    /// arguments is a program to link and run.
    #[arg(long = "run")]
    run: bool,

    /// Say what each function became, and write no object.
    ///
    /// Every C-level instruction under the source line it came from - promotions, decays and
    /// conversions included, since they are instructions - and then the Cranelift function,
    /// which is where the virtual registers appear.
    #[arg(long = "explain")]
    explain: bool,

    /// Write the object here instead of next to the source.
    #[arg(short = 'o', value_name = "PATH")]
    output: Option<PathBuf>,

    /// Makefile dependencies: write `object: source host-headers...` to `PATH`, the way `-MMD`
    /// does. A header that ships inside this compiler is not listed - a build system cannot rebuild
    /// what is not a file.
    #[arg(long = "depfile", value_name = "PATH")]
    depfile: Option<PathBuf>,

    /// Which compiler to be. `dos16` drives borland-reborn's port of Borland C++ 2.0 - the
    /// measuring stick of PLAN.md's M9, not a second product: it writes a 16-bit DOS object for
    /// comparison, and needs `libbcc20` on this machine.
    ///
    /// Compiled in only with the `dos16` feature (on by default); a build without it has no
    /// `--target` flag and no FFI at all (CONSTRAINTS 7).
    #[cfg(feature = "dos16")]
    #[arg(long = "target", value_enum, default_value_t = TargetArg::Win64)]
    target: TargetArg,

    /// How hard to work over the code: `0` compiles fastest, `1` lets Cranelift optimise (measured
    /// on the six-file corpus: 3822 bytes of x64 become 2459, and 55 ms become 180). `2` and `3`
    /// are this compiler's own back end, which is not written: asking for them is refused.
    /// How hard the back end works: `0` (the default) compiles fast, `1` runs the optimising
    /// pipeline, `2` lowers with this compiler's own back end. Linking goes through that back end,
    /// so a link is level 2 whether or not it is asked for - and another level with a link is
    /// refused rather than quietly served by the level the user did not choose.
    #[arg(long = "opt-level", value_name = "N")]
    opt_level: Option<u8>,

    /// Which C the source is allowed to use. `c11` adds the pieces a host meets in practice -
    /// `_Static_assert`, `_Generic` - and nothing that changes what a C99 program means.
    #[arg(long = "std", value_name = "DIALECT", default_value = "c99")]
    standard: StandardArg,

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

/// The two compilers this command line can be.
#[cfg(feature = "dos16")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum TargetArg {
    /// xcc itself: 64-bit COFF for Windows.
    Win64,
    /// The ported Borland C++ 2.0: 16-bit OMF, large model, for comparing against 1991.
    Dos16,
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

/// The two dialects the library knows, spelled the way a compiler's manual spells them.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum StandardArg {
    /// C99, the baseline and the default.
    C99,
    /// C99 plus the C11 pieces a host meets in practice.
    C11,
}

impl From<StandardArg> for xcc::CStandard {
    fn from(standard: StandardArg) -> Self {
        match standard {
            StandardArg::C99 => xcc::CStandard::C99,
            StandardArg::C11 => xcc::CStandard::C11,
        }
    }
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

/// How much stack the compiler thread gets.
///
/// The parser is recursive descent with a budget of 256 nested levels, and a level's frame is whatever
/// the compiler chose to make it: a debug build's frames are several times a release build's, and the
/// main thread of a Windows process has one megabyte. Measured, not assumed - a source nested twenty
/// thousand parens deep was refused on this machine running the release binary and overflowed the
/// stack running the debug one, from the same source and the same compiler. So the work happens on a
/// thread whose stack is stated here, and the budget's promise ("refused, not crashed") holds in both.
const COMPILER_STACK: usize = 16 * 1024 * 1024;

fn main() -> ExitCode {
    // The thread below owns the whole compile, so a panic in it still ends the process with a
    // failure: `join` maps both a panic and an exit code onto one answer.
    let compiler = std::thread::Builder::new()
        .stack_size(COMPILER_STACK)
        .spawn(compile_command_line)
        .expect("a thread with a stated stack");
    match compiler.join() {
        Ok(status) => status,
        Err(_) => ExitCode::from(1),
    }
}

/// The command line as it was before the stack mattered.
fn compile_command_line() -> ExitCode {
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

    #[cfg(feature = "dos16")]
    if args.target == TargetArg::Dos16 {
        return dos16(&args);
    }
    if args.explain && (args.compile_only || args.preprocess_only || args.run) {
        eprintln!("xcc: --explain is a mode of its own: no -c, -E or --run with it");
        return ExitCode::from(2);
    }
    if args.depfile.is_some() && (args.run || args.preprocess_only || args.explain) {
        eprintln!("xcc: --depfile names what a mode wrote, and this one writes no file");
        return ExitCode::from(2);
    }
    if args.depfile.is_some() && args.compile_only && args.inputs.len() > 1 {
        eprintln!(
            "xcc: --depfile names one object: compile one source at a time, or pass one depfile per source"
        );
        return ExitCode::from(2);
    }
    if args.explain && args.output.is_some() {
        eprintln!("xcc: --explain writes no object, so -o has nothing to name");
        return ExitCode::from(2);
    }
    let linking = !args.compile_only && !args.preprocess_only && !args.run && !args.explain;
    let opt_level = match (linking, args.opt_level) {
        (_, Some(2)) => 2,
        (true, None) => 2,
        (true, Some(level)) => {
            eprintln!(
                "xcc: linking lowers with this compiler's own back end, which is `--opt-level=2`, not {level}; pass -c to compile at another level"
            );
            return ExitCode::from(2);
        }
        (false, level) => level.unwrap_or(0),
    };
    let format = xcc::Format::from(args.format);
    if linking {
        return link(&args, format, opt_level);
    }
    if args.run {
        return run(&args, opt_level);
    }
    if args.output.is_some() && args.inputs.len() > 1 {
        eprintln!("xcc: -o with more than one source is not supported yet");
        return ExitCode::from(2);
    }

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
        let options = options(&args, opt_level);
        let includes = Box::new(Directories {
            roots: args.include_paths.clone(),
        });
        let mut compiler = Compile::with_includes(options, includes);
        if args.preprocess_only {
            match compiler.preprocess(&name, &text) {
                // `-o` names the file the preprocessed text goes to, the way every other mode's `-o`
                // names its output. It used to print to stdout and write nothing, which is a flag
                // that is accepted and ignored - `xcc -E t.c -o t.i` looked like it had worked.
                Ok(preprocessed) => match &args.output {
                    Some(path) => {
                        let text = spellings(&preprocessed);
                        if let Err(error) = std::fs::write(path, text) {
                            eprintln!("xcc: cannot write {}: {error}", path.display());
                            return ExitCode::from(2);
                        }
                    }
                    None => print!("{}", spellings(&preprocessed)),
                },
                Err(diagnostics) => {
                    eprintln!("{}", diagnostics.render(format));
                    status = ExitCode::from(1);
                }
            }
            continue;
        }
        if args.explain {
            match compiler.explain(&name, &text) {
                Ok(explanation) => print!("{explanation}"),
                Err(diagnostics) => {
                    eprintln!("{}", diagnostics.render(format));
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
                if let Err(error) = std::fs::write(&path, artifact.bytes()) {
                    eprintln!("xcc: cannot write {}: {error}", path.display());
                    return ExitCode::from(2);
                }
                if let Some(depfile) = &args.depfile {
                    let rule = dependencies(&path, artifact.dependencies());
                    if let Err(error) = std::fs::write(depfile, rule) {
                        eprintln!("xcc: cannot write {}: {error}", depfile.display());
                        return ExitCode::from(2);
                    }
                }
                if !artifact.diagnostics().is_empty() {
                    eprintln!(
                        "{}",
                        xcc::render_all(artifact.diagnostics(), &[source], format)
                    );
                }
            }
            Err(diagnostics) => {
                eprintln!("{}", diagnostics.render(format));
                status = ExitCode::from(1);
            }
        }
    }

    status
}

/// A makefile rule naming what an object was made from: `target: source header...`, one line, with
/// the spaces of a path escaped, which is the syntax `make` reads.
fn dependencies(target: &std::path::Path, files: &[String]) -> String {
    let mut rule = escape(target.display().to_string());
    rule.push(':');
    for file in files {
        rule.push(' ');
        rule.push_str(&escape(file.clone()));
    }
    rule.push('\n');
    rule
}

/// A path as `make` wants it in a rule: a space is escaped, and a `$` is doubled so it is not read
/// as a variable.
fn escape(path: String) -> String {
    path.replace('$', "$$").replace(' ', "\\ ")
}

/// The options a mode compiles with. `opt_level` is the mode's answer, not the flag's: a link has
/// one back end that can produce an image, so the level comes from the mode.
fn options(args: &Args, opt_level: u8) -> Options {
    Options {
        standard: args.standard.into(),
        defines: args.defines.clone(),
        undefines: args.undefines.clone(),
        opt_level,
        ..Options::default()
    }
}

/// Link every source into one `.exe`, with the C runtime that makes `main` a program.
///
/// The runtime is not the caller's business: `crates/xcc-crt/src/main.c` goes through the same
/// pipeline, so a link tests the compiler as well as the linker.
fn link(args: &Args, format: xcc::Format, opt_level: u8) -> ExitCode {
    if args.inputs.is_empty() {
        eprintln!("xcc: nothing to link");
        return ExitCode::from(2);
    }
    let mut units = Vec::new();
    for input in &args.inputs {
        let name = input.display().to_string();
        match std::fs::read_to_string(input) {
            Ok(text) => units.push((name, text)),
            Err(error) => {
                eprintln!("xcc: cannot read {name}: {error}");
                return ExitCode::from(2);
            }
        }
    }
    let borrowed: Vec<(&str, &str)> = units
        .iter()
        .map(|(name, text)| (name.as_str(), text.as_str()))
        .collect();
    let sources: Vec<SourceFile> = units
        .iter()
        .map(|(name, text)| SourceFile::new(name.clone(), text.clone()))
        .collect();
    let includes = Box::new(Directories {
        roots: args.include_paths.clone(),
    });
    let mut compiler = Compile::with_includes(options(args, opt_level), includes);
    match compiler.link(&borrowed) {
        Ok(artifact) => {
            if !artifact.diagnostics().is_empty() {
                eprintln!(
                    "{}",
                    xcc::render_all(artifact.diagnostics(), &sources, format)
                );
            }
            let path = args
                .output
                .clone()
                .unwrap_or_else(|| args.inputs[0].with_extension("exe"));
            match std::fs::write(&path, artifact.bytes()) {
                Ok(()) => {
                    if let Some(depfile) = &args.depfile {
                        let rule = dependencies(&path, artifact.dependencies());
                        if let Err(error) = std::fs::write(depfile, rule) {
                            eprintln!("xcc: cannot write {}: {error}", depfile.display());
                            return ExitCode::from(2);
                        }
                    }
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("xcc: cannot write {}: {error}", path.display());
                    ExitCode::from(2)
                }
            }
        }
        Err(diagnostics) => {
            eprintln!("{}", diagnostics.render(format));
            ExitCode::from(1)
        }
    }
}

/// `--target=dos16`: hand the source to the ported Borland C++ 2.0 and write its 16-bit object.
#[cfg(feature = "dos16")]
///
/// The measuring stick of PLAN.md's M9: the same source through a compiler that shipped in 1991,
/// so the front end's behaviour and the code sizes can be compared against something that was not
/// written here. Nothing of it ships: the library is loaded where it is found, `-c` is the only
/// mode (`BCC.EXE` is a compiler, not a linker for us), and the object it writes is the proof.
fn dos16(args: &Args) -> ExitCode {
    if !args.compile_only {
        eprintln!("xcc: --target=dos16 compiles only; pass -c");
        return ExitCode::from(2);
    }
    if args.preprocess_only || args.run || args.explain {
        eprintln!("xcc: --target=dos16 takes -c alone: -E, --run and --explain are xcc's own");
        return ExitCode::from(2);
    }
    let [input] = args.inputs.as_slice() else {
        eprintln!("xcc: --target=dos16 takes one source");
        return ExitCode::from(2);
    };
    let Some(compiler) = bcc20_sys::Compiler::load() else {
        eprintln!(
            "xcc: --target=dos16 needs libbcc20 (set BCC20_LIB or put it on PATH); it is              borland-reborn's port, and it is not here"
        );
        return ExitCode::from(2);
    };
    let name = input.display().to_string();
    let source = match std::fs::read_to_string(input) {
        Ok(source) => source,
        Err(error) => {
            eprintln!("xcc: cannot read {name}: {error}");
            return ExitCode::from(2);
        }
    };
    // BCC's own flags: large model, optimise, and the host's `-D`/`-U`/`-I` (BCC spells them the
    // same). The source goes in under a DOS name, uppercase, as the DOS filesystem has it.
    let dos = format!(
        "{}.C",
        input
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| String::from("T"))
            .chars()
            .filter(|character| character.is_ascii_alphanumeric())
            .take(8)
            .collect::<String>()
            .to_ascii_uppercase()
    );
    let mut arguments: Vec<String> =
        vec![String::from("-c"), String::from("-ml"), String::from("-O")];
    for define in &args.defines {
        arguments.push(format!("-D{define}"));
    }
    for undefine in &args.undefines {
        arguments.push(format!("-U{undefine}"));
    }
    for path in &args.include_paths {
        arguments.push(format!("-I{}", path.display()));
    }
    let job = bcc20_sys::Job {
        args: arguments
            .into_iter()
            .chain(std::iter::once(dos.clone()))
            .collect(),
        files: vec![(dos.as_str(), source.as_bytes())],
        root: None,
        time: 0,
        stack_limit: true,
    };
    let outcome = compiler.compile(&job);
    print!("{}", outcome.console);
    if !outcome.succeeded() {
        return ExitCode::from(1);
    }
    let Some((_, object)) = outcome
        .outputs
        .iter()
        .find(|(written, _)| written.ends_with(".OBJ"))
    else {
        eprintln!("xcc: the DOS compiler wrote no object");
        return ExitCode::from(1);
    };
    let path = args
        .output
        .clone()
        .unwrap_or_else(|| input.with_extension("obj"));
    if let Err(error) = std::fs::write(&path, object) {
        eprintln!("xcc: cannot write {}: {error}", path.display());
        return ExitCode::from(2);
    }
    ExitCode::SUCCESS
}

/// `--run`: compile one source in process, call `main`, and leave with its value.
///
/// The exit is `std::process::exit`, not a returned `ExitCode`: `main`'s `int` is the whole
/// status on Windows, and truncating it to a byte would make a comparison with a real executable
/// wrong for a program that returns more than 255.
fn run(args: &Args, opt_level: u8) -> ExitCode {
    let [input] = args.inputs.as_slice() else {
        eprintln!("xcc: --run takes one source");
        return ExitCode::from(2);
    };
    // The defines and the standard travel with the options, like a compile's; `#include` does not,
    // because the JIT reads the headers that ship inside the compiler and nothing else.
    if !args.include_paths.is_empty() {
        eprintln!("xcc: --run takes no -I: the JIT reads the headers inside this binary, and no host files");
        return ExitCode::from(2);
    }
    let name = input.display().to_string();
    let text = match std::fs::read_to_string(input) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("xcc: cannot read {name}: {error}");
            return ExitCode::from(2);
        }
    };
    let mut jit = match xcc::Jit::configured(&options(args, opt_level)) {
        Ok(jit) => jit,
        Err(message) => {
            eprintln!("xcc: {message}");
            return ExitCode::from(2);
        }
    };
    if let Err(message) = jit.add_source(&text) {
        eprintln!("xcc: {message}");
        return ExitCode::from(1);
    }
    let Some(address) = jit.function("main") else {
        eprintln!("xcc: {name} defines no `main`");
        return ExitCode::from(2);
    };
    // The Win64 shape of `main`: arguments are ignored by an `int main(void)`, which is what a
    // program written that way compiles to.
    let main: extern "C" fn(i32, *const *const u8) -> i32 = unsafe { std::mem::transmute(address) };
    let argv = [name.as_ptr(), std::ptr::null()];
    let status = main(1, argv.as_ptr());
    // Leave through the C runtime's own `exit`, the one the compiled code used: it flushes the
    // stdio buffers `printf` filled and runs the at-exit handlers. `std::process::exit` calls
    // `ExitProcess` and loses every buffered byte, which is why a program whose output goes to a
    // pipe would print nothing - the same reason ADR-005's crt exists for a linked `.exe`.
    if let Some(address) = jit.runtime("exit") {
        // SAFETY: the C runtime's `exit` has this signature and does not return.
        let exit: extern "C" fn(i32) -> ! = unsafe { std::mem::transmute(address) };
        exit(status);
    }
    std::process::exit(status)
}

/// `-E`: the tokens, whitespace between them, wrapped so a person can read it.
/// The preprocessed text, as `-E` prints it: the tokens' own spellings, wrapped at 96 columns.
fn spellings(preprocessed: &xcc::Preprocessed) -> String {
    let mut out = String::new();
    let mut line = String::new();
    for token in &preprocessed.tokens {
        if token.kind == xcc::TokenKind::Eof {
            continue;
        }
        let spelling = preprocessed.text(token);
        if line.len() + spelling.len() + 1 > 96 {
            out.push_str(&line);
            out.push('\n');
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(spelling);
    }
    if !line.is_empty() {
        out.push_str(&line);
        out.push('\n');
    }
    out
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
