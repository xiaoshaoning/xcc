//! One compilation: the front end, the back end and the artefact it leaves behind.

use crate::diag::{Diagnostic, Diagnostics, Severity, Span};
use crate::options::Options;
use crate::pp::{self, Includes};
use crate::source::SourceFile;

/// What a successful compilation produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    object: Vec<u8>,
    diagnostics: Vec<Diagnostic>,
    /// The files this compilation read that somebody else could rebuild: the source, and whatever
    /// `#include` found on the host. A header that ships inside this compiler is not one of them.
    dependencies: Vec<String>,
}

/// What a build system would have to rebuild for this compilation: the files read, minus anything
/// this compiler answers for itself, sorted and deduplicated so the answer does not depend on the
/// order the preprocessor happened to read them in.
fn dependencies(files: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut dependencies: Vec<String> = files
        .into_iter()
        .filter(|name| !crate::headers::is_shipped(name))
        .collect();
    dependencies.sort();
    dependencies.dedup();
    dependencies
}

impl Artifact {
    fn new(object: Vec<u8>, diagnostics: Vec<Diagnostic>, dependencies: Vec<String>) -> Self {
        Self {
            object,
            diagnostics,
            dependencies,
        }
    }

    /// The files this compilation read that a build system can rebuild: what a depfile lists.
    pub fn dependencies(&self) -> &[String] {
        &self.dependencies
    }

    /// What the compiler produced: a COFF object for `-c`, a PE32+ image for a link.
    pub fn bytes(&self) -> &[u8] {
        &self.object
    }

    /// The warnings and notes produced along the way.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }
}

/// What one compilation produced: the object, and - when the caller asked to be told - what each
/// function became.
///
/// A struct rather than a tuple because the two callers each want one half, and `result.1` at a
/// call site says nothing about which half that is.
struct Outcome {
    artifact: Artifact,
    explanation: Option<String>,
}

/// One compilation, owning all of its state.
///
/// No state is shared between compilations: two `Compile`s can run on two threads, and a host
/// may keep one around to compile many sources in turn. Nothing here touches a filesystem of
/// its own accord: sources arrive as strings, and includes will be resolved through what the
/// host gives the compiler (M2).
#[derive(Debug)]
pub struct Compile {
    options: Options,
    includes: Box<dyn Includes>,
}

impl Compile {
    /// A compiler that will compile with `options`, and read no files of its own.
    pub fn new(options: Options) -> Self {
        // No host files, but the compiler's own headers: `#include <stddef.h>` is part of the
        // compiler, and a host that wanted to be asked would be a host that cannot compile
        // `size_t` without work.
        Self {
            options,
            includes: Box::new(crate::headers::Headers::new(Box::new(()))),
        }
    }

    /// A compiler that reads `#include` through `includes`.
    ///
    /// The compiler itself never touches a filesystem: a host that wants includes gives it
    /// something that can answer, which is also how an embedder keeps includes in memory.
    pub fn with_includes(options: Options, includes: Box<dyn Includes>) -> Self {
        Self {
            options,
            includes: Box::new(crate::headers::Headers::new(includes)),
        }
    }

    /// The options this compiler was made with.
    pub fn options(&self) -> &Options {
        &self.options
    }

    /// Preprocess `source` without parsing it: the tokens, the files read, and what was wrong.
    ///
    /// This is what `-E` prints and what a depfile lists, and it is the whole front end up to
    /// the parser: a host can run it on its own without asking for an object.
    pub fn preprocess(&self, name: &str, source: &str) -> Result<pp::Preprocessed, Diagnostics> {
        let file = SourceFile::new(name, source);
        if file.len() > self.options.budgets.max_source_bytes {
            let error = Diagnostic::error(format!(
                "`{name}` is {} bytes, more than the {} a single source may be",
                file.len(),
                self.options.budgets.max_source_bytes
            ))
            .with_file(name);
            let items = truncate_diagnostics(vec![error], &self.options);
            return Err(Diagnostics::new(items, vec![file]));
        }
        let preprocessed = pp::preprocess(&file, &self.options, &*self.includes);
        if preprocessed.has_errors() {
            let items = truncate_diagnostics(preprocessed.diagnostics.clone(), &self.options);
            return Err(Diagnostics::new(items, preprocessed.files.clone()));
        }
        Ok(preprocessed)
    }

    /// Compile `source`, which the caller calls `name`.
    ///
    /// Returns the object and any warnings, or every error the front end found. The source is
    /// read as UTF-8: hosts hold their sources as strings, and a file that is not UTF-8 is a
    /// host problem, not a compiler one.
    ///
    /// Takes `&mut self` although nothing is kept yet: the preprocessor's arenas and interner
    /// arrive with M2, and callers should not have to change because of it.
    pub fn compile(&mut self, name: &str, source: &str) -> Result<Artifact, Diagnostics> {
        Ok(self.run(name, source, false)?.artifact)
    }

    /// Compile `source`, and say what each function became: the C-level instruction next to the
    /// source line it came from, and the Cranelift function with the virtual registers it chose.
    ///
    /// This is `--explain`, and it is the same pipeline as [`Compile::compile`]: a second way to
    /// ask one compilation what it did, not a second compilation.
    pub fn explain(&mut self, name: &str, source: &str) -> Result<String, Diagnostics> {
        Ok(self
            .run(name, source, true)?
            .explanation
            .unwrap_or_default())
    }

    /// The pipeline both entry points share: tokens, declarations, `cir`, and the passes - up to the
    /// point where a back end is chosen. `compile` then writes an object and `link` keeps the image,
    /// so a linked program and a `-c` object are the same `cir` and the same passes.
    fn front_end(
        &mut self,
        name: &str,
        source: &str,
    ) -> Result<(crate::cir::Module, crate::pp::Preprocessed, Vec<Diagnostic>), Diagnostics> {
        let mut diagnostics = Vec::new();
        // A level this back end does not have is the caller's mistake, and it is reported as one:
        // the back end's refusal would arrive as an internal error, which blames the compiler for
        // a number it was handed. Asked first, because it is not about the source.
        if let Err(reason) = crate::back::check_opt_level(self.options.opt_level) {
            diagnostics.push(
                Diagnostic::error(reason)
                    .with_file(name)
                    .with_span(Span::at(0)),
            );
            let items = truncate_diagnostics(diagnostics, &self.options);
            return Err(Diagnostics::new(items, vec![SourceFile::new(name, source)]));
        }

        // Phases 1 and 2, tokens, and the directives that exist so far.
        let preprocessed = self.preprocess(name, source)?;
        let sources = preprocessed.files.clone();
        diagnostics.extend(preprocessed.diagnostics.clone());

        // Declarations and their types: the first slice of M3.
        let parsed = crate::parse::parse(&preprocessed, &self.options.budgets);
        let failed = parsed.diagnostics.iter().any(Diagnostic::is_error);
        diagnostics.extend(parsed.diagnostics);
        if failed {
            let items = truncate_diagnostics(diagnostics, &self.options);
            return Err(Diagnostics::new(items, sources));
        }

        // The passes, before either lowering sees the module: one `cir` in, one optimized `cir`
        // out, so an object and a JIT run the same code (M10). A pass that breaks a function is a
        // compiler bug, so it says so rather than blaming the source.
        let mut module = parsed.module;
        if let Err((function, broken)) = crate::cir::pass::optimise(&mut module) {
            diagnostics.push(
                // Neither layer is named: `optimise` verifies the front end's `cir` before its first
                // pass, so this covers both, and the function is named either way. The message says
                // what is true - the compiler wrote something the verifier refuses - rather than
                // guessing which half of it did.
                Diagnostic::error(format!(
                    "internal: `{}` is not valid: {}",
                    preprocessed.names().resolve(function),
                    broken.message
                ))
                .with_file(name)
                .with_span(Span::at(0)),
            );
            let items = truncate_diagnostics(diagnostics, &self.options);
            return Err(Diagnostics::new(items, sources));
        }

        Ok((module, preprocessed, diagnostics))
    }

    /// Compile every source and link them into one executable, with the C runtime that makes
    /// `main` a program.
    ///
    /// The runtime is not the caller's business: `crates/xcc-crt/src/main.c` goes through this same
    /// pipeline on every link, so the first program a link produces is also a test of the compiler.
    /// Linking needs the *image* of each unit - sections, symbols, relocations - and only level 2 has
    /// one, so a link at any other level is refused rather than quietly lowered by a back end that
    /// keeps its sections to itself.
    pub fn link(&mut self, units: &[(&str, &str)]) -> Result<Artifact, Diagnostics> {
        if self.options.opt_level < 2 {
            return Err(Diagnostics::new(
                vec![Diagnostic::error(
                    "linking needs `--opt-level=2`: the Cranelift back end keeps its sections to itself, so a unit lowered by it cannot be linked",
                )
                .with_file("link")
                .with_span(Span::at(0))],
                Vec::new(),
            ));
        }
        let mut images = Vec::new();
        let mut sources = Vec::new();
        // A warning is about one compilation, and a link makes several: they are collected here so
        // that the caller of a link is told what the caller of `-c` is told.
        let mut diagnostics = Vec::new();
        // A link's dependencies are the units it was given: the runtime it compiles in is nobody's file.
        let dependencies = dependencies(units.iter().map(|(name, _)| (*name).to_owned()));
        let runtime = [("xcc-crt/main.c", crate::back::CRT_SOURCE)];
        for (name, source) in runtime.iter().copied().chain(units.iter().copied()) {
            let (module, preprocessed, warnings) = self.front_end(name, source)?;
            diagnostics.extend(warnings);
            match crate::back::image(&module, preprocessed.names(), self.options.opt_level) {
                Ok(Some(image)) => images.push(image),
                Ok(None) => unreachable!("the level was checked above"),
                Err(error) => {
                    let message = match error.not_lowered {
                        Some(_) => format!("not lowered at `--opt-level=2`: {error}"),
                        None => format!("internal: {error}"),
                    };
                    return Err(Diagnostics::new(
                        vec![Diagnostic::error(message)
                            .with_file(name)
                            .with_span(Span::at(0))],
                        preprocessed.files.clone(),
                    ));
                }
            }
            sources.extend(preprocessed.files.clone());
        }
        // What nothing defines comes from a library, and *which* library, under *which* name, is a
        // fact about the target rather than about the program - so it lives next to the runtime's
        // own name list.
        // The program's name, the library, and the name the library knows it by: the first is what
        // a relocation asks for, the last is what the import table must look up.
        let mut wanted: Vec<(String, &'static str, String)> = Vec::new();
        for name in required_names(&images) {
            match crate::back::import_for(&name) {
                Some((library, exported)) => wanted.push((name, library, exported)),
                None => {
                    return Err(Diagnostics::new(
                        vec![Diagnostic::error(format!("undefined symbol `{name}`"))
                            .with_file("link")
                            .with_span(Span::at(0))],
                        sources,
                    ))
                }
            }
        }
        let imports: Vec<crate::back::x64::pe::Imported<'_>> = wanted
            .iter()
            .map(|(name, library, exported)| crate::back::x64::pe::Imported {
                library,
                name,
                exported,
            })
            .collect();

        match crate::back::x64::pe::link(&images, "mainCRTStartup", &imports) {
            Ok(linked) => Ok(Artifact::new(linked.bytes, diagnostics, dependencies)),
            Err(missing) => Err(Diagnostics::new(
                vec![
                    Diagnostic::error(format!("undefined symbol `{}`", missing.name))
                        .with_file("link")
                        .with_span(Span::at(0)),
                ],
                sources,
            )),
        }
    }

    /// One compilation. `explain` asks the back end to keep what each function became, which
    /// costs the formatting of every function and nothing else.
    fn run(&mut self, name: &str, source: &str, explain: bool) -> Result<Outcome, Diagnostics> {
        let mut diagnostics = Vec::new();
        let (module, preprocessed, from_front_end) = self.front_end(name, source)?;
        diagnostics.extend(from_front_end);
        let sources = preprocessed.files.clone();
        let opt_level = self.options.opt_level;

        // A module with no function is a translation unit of *objects* - a table, a constant, the
        // bytes of a lookup - and they still need an object file: `lopcodes.c` in Lua is one such
        // file, and skipping the back end when the function list was empty wrote a zero-byte object
        // whose symbols then "did not exist" at link time. There is one path, and it handles both.
        let explained;
        let object = match crate::back::object(&module, preprocessed.names(), explain, opt_level) {
            Ok(lowered) => {
                explained = explain.then(|| crate::explain::render(&lowered.explained, &sources));
                lowered.bytes
            }
            // A gap the back end has not lowered yet is refused at the expression that asked
            // for it, like every other thing this compiler does not do; anything else is a
            // compiler bug and says so rather than blaming the source (CONSTRAINTS 6).
            Err(error) => {
                let diagnostic = match error.not_lowered {
                    Some(span) => Diagnostic::error(format!(
                        "not lowered yet at opt level {}: {}",
                        self.options.opt_level, error.message
                    ))
                    .with_file(name)
                    .with_span(span),
                    None => Diagnostic::error(format!("internal: {error}"))
                        .with_file(name)
                        .with_span(Span::at(0)),
                };
                diagnostics.push(diagnostic);
                let items = truncate_diagnostics(diagnostics, &self.options);
                return Err(Diagnostics::new(items, sources));
            }
        };
        let items = truncate_diagnostics(diagnostics, &self.options);
        let dependencies = dependencies(sources.iter().map(|file| file.name().to_owned()));
        Ok(Outcome {
            artifact: Artifact::new(object, items, dependencies),
            explanation: explained,
        })
    }
}

/// Cut a diagnostic list down to the budget, saying how many were dropped.
fn truncate_diagnostics(mut diagnostics: Vec<Diagnostic>, options: &Options) -> Vec<Diagnostic> {
    let budget = options.budgets.max_diagnostics;
    if diagnostics.len() > budget {
        let dropped = diagnostics.len() - budget;
        diagnostics.truncate(budget);
        diagnostics.push(Diagnostic::new(
            Severity::Note,
            format!("{dropped} further diagnostics not shown"),
        ));
    }
    diagnostics
}

/// Every name the units refer to and none of them defines, in a stable order so that the same
/// program links to the same file twice.
fn required_names(images: &[crate::back::x64::coff::Image]) -> Vec<String> {
    let defined: std::collections::BTreeSet<&str> = images
        .iter()
        .flat_map(|image| {
            image
                .functions
                .iter()
                .map(|function| function.name.as_str())
                .chain(image.data.symbols.iter().map(|object| object.name.as_str()))
        })
        .collect();
    let mut wanted: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for image in images {
        for function in &image.functions {
            for relocation in &function.relocations {
                wanted.insert(relocation.symbol.clone());
            }
        }
        for relocation in &image.data.relocations {
            wanted.insert(relocation.symbol.clone());
        }
    }
    wanted
        .into_iter()
        .filter(|name| !defined.contains(name.as_str()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::Budgets;

    #[test]
    fn a_source_over_budget_is_refused_without_reading_it() {
        let options = Options {
            budgets: Budgets {
                max_source_bytes: 8,
                ..Budgets::default()
            },
            ..Options::default()
        };
        let mut compiler = Compile::new(options);
        let errors = compiler
            .compile("t.c", "int main(void) { return 0; }")
            .unwrap_err()
            .items()
            .to_vec();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].severity, Severity::Error);
        assert!(errors[0].message.contains("more than the 8"));
        assert_eq!(errors[0].file.as_deref(), Some("t.c"));
    }

    #[test]
    fn a_translation_unit_of_objects_still_writes_an_object() {
        // `lopcodes.c` in Lua is a table and nothing else. This used to write a zero-byte file, so
        // every symbol it defined was missing at link time - and the compile reported success.
        let mut compiler = Compile::new(Options::default());
        let artifact = compiler
            .compile(
                "t.c", "int x;
",
            )
            .expect("no error");
        assert_eq!(&artifact.bytes()[0..2], &[0x64, 0x86], "x86-64 COFF");
        assert!(artifact.bytes().len() > 20 + 40, "a header and a section");
    }

    #[test]
    fn a_body_is_read_and_lowered() {
        let mut compiler = Compile::new(Options::default());
        let artifact = compiler
            .compile(
                "t.c",
                "int main(void) { return 0; }
",
            )
            .expect("a function lowers");
        assert!(!artifact.bytes().is_empty(), "an object came out");
        assert_eq!(&artifact.bytes()[0..2], &[0x64, 0x86], "x86-64 COFF");
    }

    #[test]
    fn a_lexical_error_stops_before_the_missing_stages() {
        let mut compiler = Compile::new(Options::default());
        let errors = compiler
            .compile(
                "t.c",
                "char *s = \"abc;
",
            )
            .unwrap_err()
            .items()
            .to_vec();
        assert_eq!(errors.len(), 1);
        assert_eq!(
            errors[0].message,
            "string is not terminated before the end of its line"
        );
        assert_eq!(errors[0].span, Some(Span::new(10, 1)));
    }

    #[test]
    fn too_many_diagnostics_are_cut_and_counted() {
        let options = Options {
            budgets: Budgets {
                max_diagnostics: 1,
                ..Budgets::default()
            },
            ..Options::default()
        };
        let mut list = vec![
            Diagnostic::error("one"),
            Diagnostic::error("two"),
            Diagnostic::error("three"),
        ];
        list = truncate_diagnostics(list, &options);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].message, "one");
        assert_eq!(list[1].severity, Severity::Note);
        assert_eq!(list[1].message, "2 further diagnostics not shown");
    }
}
