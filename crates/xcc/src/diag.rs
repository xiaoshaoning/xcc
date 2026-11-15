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
}

impl Span {
    /// A span covering `len` bytes from `start`.
    pub const fn new(start: u32, len: u32) -> Self {
        Self { start, len }
    }

    /// A span covering the single byte at `start`.
    pub const fn at(start: u32) -> Self {
        Self { start, len: 1 }
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
    /// and the host handed the file over.
    pub fn position(&self, source: Option<&crate::source::SourceFile>) -> Option<Position> {
        let source = source?;
        let span = self.span?;
        Some(source.position(span.start))
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
        assert_eq!(diagnostic.span, Some(Span { start: 10, len: 1 }));
        assert_eq!(diagnostic.notes.len(), 1);
        assert!(!diagnostic.is_error());
    }

    #[test]
    fn a_zero_length_span_is_a_point() {
        assert_eq!(Span::at(4).end(), 5);
        assert_eq!(Span::new(4, 0).end(), 4);
    }
}
