//! One compilation: the front end, the back end and the artefact it leaves behind.

use crate::diag::{Diagnostic, Severity, Span};
use crate::options::Options;
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
#[derive(Debug, Clone)]
pub struct Compile {
    options: Options,
}

impl Compile {
    /// A compiler that will compile with `options`.
    pub fn new(options: Options) -> Self {
        Self { options }
    }

    /// The options this compiler was made with.
    pub fn options(&self) -> &Options {
        &self.options
    }

    /// Compile `source`, which the caller calls `name`.
    ///
    /// Returns the object and any warnings, or every error the front end found. The source is
    /// read as UTF-8: hosts hold their sources as strings, and a file that is not UTF-8 is a
    /// host problem, not a compiler one.
    ///
    /// Takes `&mut self` although nothing is kept yet: the preprocessor's arenas and interner
    /// arrive with M2, and callers should not have to change because of it.
    pub fn compile(&mut self, name: &str, source: &str) -> Result<Artifact, Vec<Diagnostic>> {
        let file = SourceFile::new(name, source);
        let mut diagnostics = Vec::new();

        if file.len() as usize > self.options.budgets.max_source_bytes {
            diagnostics.push(
                Diagnostic::error(format!(
                    "`{name}` is {} bytes, more than the {} a single source may be",
                    file.len(),
                    self.options.budgets.max_source_bytes
                ))
                .with_file(name),
            );
            return Err(truncate_diagnostics(diagnostics, &self.options));
        }

        // M1: the pipeline behind this line is M2 (preprocessor) and M3 (parser). Until then,
        // say so, at the start of the file, rather than pretending.
        diagnostics.push(
            Diagnostic::error("the C front end is not implemented yet")
                .with_file(name)
                .with_span(Span::at(0))
                .with_note("M2 adds the preprocessor, M3 the parser and types; see PLAN.md"),
        );
        Err(truncate_diagnostics(diagnostics, &self.options))
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
            .unwrap_err();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].severity, Severity::Error);
        assert!(errors[0].message.contains("more than the 8"));
        assert_eq!(errors[0].file.as_deref(), Some("t.c"));
    }

    #[test]
    fn the_front_end_is_not_implemented_yet() {
        let mut compiler = Compile::new(Options::default());
        let errors = compiler
            .compile("t.c", "int main(void) { return 0; }\n")
            .unwrap_err();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].message, "the C front end is not implemented yet");
        assert_eq!(errors[0].file.as_deref(), Some("t.c"));
        assert_eq!(errors[0].span, Some(Span::at(0)));
        assert_eq!(errors[0].notes.len(), 1);
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
