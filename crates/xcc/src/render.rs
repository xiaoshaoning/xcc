//! Turning diagnostics into text.
//!
//! Three shapes are supported: for people (a caret under the line), for MSVC-style tools
//! (`file(line,col): error: ...`, which MSBuild and Visual Studio understand) and for GCC-style
//! tools (`file:line:col: error: ...`, which everything else understands). A host that draws
//! its own squiggles should use the diagnostics directly and skip this module.

use crate::diag::Diagnostic;
use crate::source::SourceFile;

/// The shape of rendered diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum Format {
    /// A line of source with a caret under what is wrong.
    #[default]
    Human,
    /// `t.c(3,7): error: ...`, as MSVC and Visual Studio print, and MSBuild parses.
    Msvc,
    /// `t.c:3:7: error: ...`, as GCC and clang print.
    Gcc,
}

/// Render one diagnostic.
///
/// `source` is the file the diagnostic is about, if the host has it: without it there is no
/// line to show and the caret is left out.
pub fn render(diagnostic: &Diagnostic, source: Option<&SourceFile>, format: Format) -> String {
    if format == Format::Human {
        return human(diagnostic, source);
    }
    let place = place(diagnostic, source, format);
    let mut out = vec![header(
        place.as_deref(),
        diagnostic.severity.as_str(),
        &diagnostic.message,
    )];
    for note in &diagnostic.notes {
        out.push(header(place.as_deref(), "note", note));
    }
    out.join("\n")
}

/// Render many diagnostics, looking each one's file up in `sources` by name.
pub fn render_all(diagnostics: &[Diagnostic], sources: &[SourceFile], format: Format) -> String {
    let rendered: Vec<String> = diagnostics
        .iter()
        .map(|diagnostic| render(diagnostic, source_for(diagnostic, sources), format))
        .collect();
    rendered.join("\n")
}

/// The file a diagnostic is about, out of the ones the host handed over.
fn source_for<'a>(diagnostic: &Diagnostic, sources: &'a [SourceFile]) -> Option<&'a SourceFile> {
    let name = diagnostic.file.as_deref()?;
    sources.iter().find(|file| file.name() == name)
}

/// Where the diagnostic points, as the formats write it: `t.c(3,7)`, `t.c:3:7`, or the file
/// alone when there is no span. `None` if there is no file either.
fn place(diagnostic: &Diagnostic, source: Option<&SourceFile>, format: Format) -> Option<String> {
    let file = diagnostic.file.as_deref()?;
    let Some(point) = diagnostic.position(source) else {
        return Some(file.to_owned());
    };
    Some(match format {
        Format::Msvc => format!("{file}({},{})", point.line, point.column),
        _ => format!("{file}:{}:{}", point.line, point.column),
    })
}

/// `place: severity: message`, with the place left out when it is unknown.
fn header(place: Option<&str>, severity: &str, message: &str) -> String {
    match place {
        Some(place) => format!("{place}: {severity}: {message}"),
        None => format!("{severity}: {message}"),
    }
}

/// The format for people: the line the diagnostic is on, and a caret under the span.
fn human(diagnostic: &Diagnostic, source: Option<&SourceFile>) -> String {
    let severity = diagnostic.severity.as_str();
    let mut out = vec![header(
        place(diagnostic, source, Format::Human).as_deref(),
        severity,
        &diagnostic.message,
    )];

    let position = diagnostic.position(source);
    if let (Some(source), Some(span), Some(position)) = (source, diagnostic.span, position) {
        if let Some(text) = source.line(position.line) {
            let width = position.line.to_string().len();
            let gutter = " ".repeat(width);
            let (shown, caret, carets) = window(
                text,
                position.column.saturating_sub(1) as usize,
                span.len.max(1) as usize,
            );
            out.push(format!("{:>width$} | {shown}", position.line));
            let carets = " ".repeat(caret) + &"^".repeat(carets);
            out.push(format!("{gutter} | {carets}"));
        }
    }

    for note in &diagnostic.notes {
        out.push(format!("= note: {note}"));
    }
    out.join("\n")
}

/// The most characters of a source line one diagnostic shows.
///
/// A line can be megabytes - machine-generated code often is, and a hostile source always can be -
/// and the reader needs the caret's column, not the rest of the line. A window also keeps a
/// hundred diagnostics at a hundred lines instead of forty million, which is the difference
/// between reporting a refusal and burying the host in it.
const LINE_WIDTH: usize = 120;

/// A window of `text` holding `caret`, at most [`LINE_WIDTH`] characters: what it shows, where the
/// caret sits in it, and how many carets fit. `…` at either end says the line continues.
fn window(text: &str, caret: usize, carets: usize) -> (String, usize, usize) {
    let chars: Vec<char> = text.chars().collect();
    let caret = caret.min(chars.len());
    let carets = carets.min(chars.len().saturating_sub(caret)).max(1);
    if chars.len() <= LINE_WIDTH {
        return (text.to_owned(), caret, carets);
    }
    // A third of the window before the caret, the rest after: enough context to recognise the
    // expression, all of it on screen.
    let mut start = caret.saturating_sub(LINE_WIDTH / 3);
    let mut end = start + LINE_WIDTH;
    if end > chars.len() {
        end = chars.len();
        start = end.saturating_sub(LINE_WIDTH);
    }
    let mut shown = String::new();
    if start > 0 {
        shown.push('…');
    }
    shown.extend(&chars[start..end]);
    if end < chars.len() {
        shown.push('…');
    }
    let offset = usize::from(start > 0) + (caret - start);
    (shown, offset, carets.min(end - caret).max(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diag::{Diagnostic, Span};

    fn compiler_error() -> (Diagnostic, SourceFile) {
        let source = SourceFile::new("t.c", "int x = ;\nint y = 0;\n");
        let diagnostic = Diagnostic::error("expected expression")
            .with_file("t.c")
            .with_span(Span::at(8))
            .with_note("a value is missing to the right of `=`");
        (diagnostic, source)
    }

    #[test]
    fn human_shows_the_line_and_a_caret() {
        let (diagnostic, source) = compiler_error();
        assert_eq!(
            render(&diagnostic, Some(&source), Format::Human),
            "t.c:1:9: error: expected expression\n\
             1 | int x = ;\n  |         ^\n\
             = note: a value is missing to the right of `=`"
        );
    }

    #[test]
    fn a_span_underlines_all_of_itself() {
        let source = SourceFile::new("t.c", "ab abc\n");
        let diagnostic = Diagnostic::error("bad")
            .with_file("t.c")
            .with_span(Span::new(3, 3));
        assert_eq!(
            render(&diagnostic, Some(&source), Format::Human),
            "t.c:1:4: error: bad\n1 | ab abc\n  |    ^^^"
        );
    }

    /// Without the file there is no offset to turn into a line: the file is named, the place
    /// is not invented.
    #[test]
    fn human_without_a_source_names_the_file_and_nothing_more() {
        let (diagnostic, _) = compiler_error();
        assert_eq!(
            render(&diagnostic, None, Format::Human),
            "t.c: error: expected expression\n= note: a value is missing to the right of `=`"
        );
    }

    #[test]
    fn msvc_and_gcc_agree_on_the_place() {
        let (diagnostic, source) = compiler_error();
        assert_eq!(
            render(&diagnostic, Some(&source), Format::Msvc),
            "t.c(1,9): error: expected expression\n\
             t.c(1,9): note: a value is missing to the right of `=`"
        );
        assert_eq!(
            render(&diagnostic, Some(&source), Format::Gcc),
            "t.c:1:9: error: expected expression\n\
             t.c:1:9: note: a value is missing to the right of `=`"
        );
    }

    #[test]
    fn a_diagnostic_without_a_place_is_still_rendered() {
        let diagnostic = Diagnostic::error("something went wrong");
        for format in [Format::Human, Format::Msvc, Format::Gcc] {
            assert_eq!(
                render(&diagnostic, None, format),
                "error: something went wrong"
            );
        }
    }

    #[test]
    fn a_file_without_a_span_keeps_its_name() {
        let diagnostic = Diagnostic::error("bad").with_file("t.c");
        assert_eq!(render(&diagnostic, None, Format::Gcc), "t.c: error: bad");
    }

    #[test]
    fn a_span_past_the_end_does_not_panic() {
        let source = SourceFile::new("t.c", "int x;\n");
        let diagnostic = Diagnostic::error("x")
            .with_file("t.c")
            .with_span(Span::new(9999, 9999));
        assert!(render(&diagnostic, Some(&source), Format::Human).contains("error: x"));
    }

    #[test]
    fn a_line_too_long_to_show_is_a_window_around_the_caret() {
        // Machine-generated code has lines like this and a hostile source has worse: a hundred
        // diagnostics must not become a hundred copies of a megabyte.
        let line = format!("int x = {}0;", "1 + ".repeat(4_000));
        let source = SourceFile::new("long.c", &line);
        let at = line.find("0;").expect("the last term") as u32;
        let diagnostic = Diagnostic::error("this one")
            .with_file("long.c")
            .with_span(Span::new(at, 1));
        let rendered = render(&diagnostic, Some(&source), Format::Human);

        for shown in rendered.lines() {
            assert!(
                shown.chars().count() <= LINE_WIDTH + 16,
                "a window, not the whole line: {} characters",
                shown.chars().count()
            );
        }
        assert!(rendered.contains('…'), "the line says it continues");

        // And the caret still sits under what the diagnostic is about: the window moved, the
        // caret moved with it.
        let lines: Vec<&str> = rendered.lines().collect();
        assert_eq!(lines.len(), 3, "{rendered}");
        let shown = lines[1].split_once(" | ").expect("a gutter").1;
        let carets = lines[2].split_once(" | ").expect("a gutter").1;
        let column = carets.find('^').expect("a caret");
        assert_eq!(shown.chars().nth(column), Some('0'), "{rendered}");
    }

    #[test]
    fn many_diagnostics_are_rendered_one_after_another() {
        let source = SourceFile::new("t.c", "int x;\n");
        let first = Diagnostic::error("one")
            .with_file("t.c")
            .with_span(Span::at(0));
        let second = Diagnostic::error("two")
            .with_file("t.c")
            .with_span(Span::at(4));
        assert_eq!(
            render_all(&[first, second], std::slice::from_ref(&source), Format::Gcc),
            "t.c:1:1: error: one\nt.c:1:5: error: two"
        );
    }
}
