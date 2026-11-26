//! Diagnostics as data.
//!
//! A diagnostic says what went wrong, where, and optionally why. Rendering it as text is a
//! separate step ([`crate::render`]), so a host that draws its own squiggles never has to
//! parse a message.

use crate::source::Position;

/// How bad a diagnostic is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// The compilation cannot go on, or would produce something wrong.
    Error,
    /// Something suspicious that still compiles.
    Warning,
    /// Extra information attached to another diagnostic.
    Note,
}

impl Severity {
    /// The word used for this severity in rendered output.
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Note => "note",
        }
    }
}

/// A byte range in a source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Span {
    /// The offset of the first byte.
    pub start: u32,
    /// How many bytes the span covers; zero is a point, rendered as a single caret.
    pub len: u32,
    /// Which file those bytes are in: an index into the compilation's file table.
    ///
    /// It is here because an offset alone cannot say. `#include <stdio.h>` puts tokens from two
    /// files into one translation unit, both numbered from their own first byte, so a span of
    /// offset 100 in each is two different places - and the `cir` keeps spans and not tokens, which
    /// is where the file used to live. A line number computed from an offset without this would be
    /// right for the file being compiled and wrong for every byte that came from a header.
    ///
    /// Zero is the file the host handed over; a span this compiler synthesized carries it too,
    /// which is the honest answer: it belongs to the file being compiled.
    pub file: u16,
}

impl Span {
    /// A span covering `len` bytes from `start`.
    pub const fn new(start: u32, len: u32) -> Self {
        Self {
            start,
            len,
            file: 0,
        }
    }

    /// The same span, in a named file of the compilation.
    pub const fn in_file(self, file: u16) -> Self {
        Self { file, ..self }
    }

    /// A span covering the single byte at `start`.
    pub const fn at(start: u32) -> Self {
        Self {
            start,
            len: 1,
            file: 0,
        }
    }

    /// The offset one past the last byte.
    pub const fn end(self) -> u32 {
        self.start + self.len
    }
}

/// One message: an error, a warning or a note, with a place in a file if it has one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// How bad it is.
    pub severity: Severity,
    /// What went wrong, in one sentence, without a full stop.
    pub message: String,
    /// The name of the file it is about, as it was passed to [`crate::Compile::compile`].
    pub file: Option<String>,
    /// The byte range it is about.
    pub span: Option<Span>,
    /// Anything else worth saying: usually where to look or what the standard requires.
    pub notes: Vec<String>,
}

impl Diagnostic {
    /// A diagnostic of `severity` saying `message`.
    pub fn new(severity: Severity, message: impl Into<String>) -> Self {
        Self {
            severity,
            message: message.into(),
            file: None,
            span: None,
            notes: Vec::new(),
        }
    }

    /// An error saying `message`.
    pub fn error(message: impl Into<String>) -> Self {
        Self::new(Severity::Error, message)
    }

    /// A warning saying `message`.
    pub fn warning(message: impl Into<String>) -> Self {
        Self::new(Severity::Warning, message)
    }

    /// The same diagnostic, about `file`.
    pub fn with_file(mut self, file: impl Into<String>) -> Self {
        self.file = Some(file.into());
        self
    }

    /// The same diagnostic, about `span`.
    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }

    /// The same diagnostic, with a note added.
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    /// Whether this diagnostic stops the compilation.
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }

    /// Where this diagnostic points, as a line and column in `source`, if it points anywhere
    /// and `source` is the file it is about.
    ///
    /// The file is checked, not trusted: rendering against another file's text would otherwise
    /// report a line and column that belong to different code.
    pub fn position(&self, source: Option<&crate::source::SourceFile>) -> Option<Position> {
        let source = source?;
        if self.file.as_deref() != Some(source.name()) {
            return None;
        }
        let span = self.span?;
        Some(source.position(span.start))
    }
}

/// Diagnostics, with the sources they are about.
///
/// A diagnostic carries a file *name*; turning it into a line and a caret needs that file's
/// text, which a caller may not have - an include was read by the resolver, not by the caller.
/// So the two travel together, and [`Diagnostics::render`] is the one call that needs nothing
/// else.
#[derive(Debug, Default, Clone)]
pub struct Diagnostics {
    items: Vec<Diagnostic>,
    sources: Vec<crate::source::SourceFile>,
}

impl Diagnostics {
    /// Diagnostics about `sources`.
    pub fn new(items: Vec<Diagnostic>, sources: Vec<crate::source::SourceFile>) -> Self {
        Self { items, sources }
    }

    /// The diagnostics themselves, for a host that draws its own.
    pub fn items(&self) -> &[Diagnostic] {
        &self.items
    }

    /// The files they point into.
    pub fn sources(&self) -> &[crate::source::SourceFile] {
        &self.sources
    }

    /// Whether any of them is an error.
    pub fn has_errors(&self) -> bool {
        self.items.iter().any(Diagnostic::is_error)
    }

    /// Whether there are none at all.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Render them all, with positions and carets when the file is at hand.
    pub fn render(&self, format: crate::render::Format) -> String {
        crate::render::render_all(&self.items, &self.sources, format)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builders_set_what_they_say() {
        let diagnostic = Diagnostic::warning("unused variable `x`")
            .with_file("t.c")
            .with_span(Span::new(10, 1))
            .with_note("add `(void)x` to say it is deliberate");
        assert_eq!(diagnostic.severity, Severity::Warning);
        assert_eq!(diagnostic.file.as_deref(), Some("t.c"));
        assert_eq!(diagnostic.span, Some(Span::at(10)));
        assert_eq!(diagnostic.notes.len(), 1);
        assert!(!diagnostic.is_error());
    }

    #[test]
    fn a_position_needs_the_file_the_diagnostic_is_about() {
        use crate::source::SourceFile;
        let diagnostic = Diagnostic::error("bad")
            .with_file("t.c")
            .with_span(Span::at(4));
        let t = SourceFile::new(
            "t.c", "int x;
",
        );
        let u = SourceFile::new(
            "u.c", "int y;
",
        );
        assert_eq!(
            diagnostic.position(Some(&t)),
            Some(Position { line: 1, column: 5 })
        );
        assert_eq!(
            diagnostic.position(Some(&u)),
            None,
            "another file's text is not a place"
        );
        assert_eq!(diagnostic.position(None), None);
        assert_eq!(
            Diagnostic::error("bad")
                .with_span(Span::at(0))
                .position(Some(&t)),
            None
        );
    }

    #[test]
    fn a_zero_length_span_is_a_point() {
        assert_eq!(Span::at(4).end(), 5);
        assert_eq!(Span::new(4, 0).end(), 4);
    }
}
