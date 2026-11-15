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
}

impl Artifact {
    /// The object file, as bytes.
    pub fn object(&self) -> &[u8] {
        &self.object
    }

    /// The warnings and notes produced along the way.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }
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
        Self {
            options,
            includes: Box::new(()),
        }
    }

    /// A compiler that reads `#include` through `includes`.
    ///
    /// The compiler itself never touches a filesystem: a host that wants includes gives it
    /// something that can answer, which is also how an embedder keeps includes in memory.
    pub fn with_includes(options: Options, includes: Box<dyn Includes>) -> Self {
        Self { options, includes }
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
        let mut diagnostics = Vec::new();

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

        // M3 continues with statements and expressions; M4 generates code.
        diagnostics.push(
            Diagnostic::error("the code generator is not implemented yet")
                .with_file(name)
                .with_span(Span::at(0))
                .with_note("M3 adds statements and expressions; M4 generates code"),
        );
        let items = truncate_diagnostics(diagnostics, &self.options);
        Err(Diagnostics::new(items, sources))
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
    fn declarations_are_read_and_the_code_generator_is_missing() {
        let mut compiler = Compile::new(Options::default());
        let errors = compiler
            .compile(
                "t.c", "int x;
",
            )
            .unwrap_err()
            .items()
            .to_vec();
        assert_eq!(errors.len(), 1);
        assert_eq!(
            errors[0].message,
            "the code generator is not implemented yet"
        );
        assert_eq!(errors[0].file.as_deref(), Some("t.c"));
        assert_eq!(errors[0].span, Some(Span::at(0)));
        assert_eq!(errors[0].notes.len(), 1);
    }

    #[test]
    fn a_body_is_read_and_then_the_code_generator_is_missing() {
        let mut compiler = Compile::new(Options::default());
        let errors = compiler
            .compile(
                "t.c",
                "int main(void) { return 0; }
",
            )
            .unwrap_err()
            .items()
            .to_vec();
        assert_eq!(
            errors[0].message,
            "this function body is not lowered yet (M4.2b)"
        );
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
