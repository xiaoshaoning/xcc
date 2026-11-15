//! Source files and positions in them.

/// A position in a source file: one-based line and column.
///
/// Columns count bytes, not characters: C source is read as bytes, and a column is only ever
/// used to place a caret under the line it is printed with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Position {
    /// One-based line.
    pub line: u32,
    /// One-based column, in bytes.
    pub column: u32,
}

/// A source file: the name the compiler knows it by, and its text.
///
/// Rendering a diagnostic needs the text of the line it points at, so hosts pass the
/// [`SourceFile`] they compiled to [`crate::render_all`]; a diagnostic only carries the name
/// and the span, so it stays cheap to move around.
#[derive(Debug, Clone)]
pub struct SourceFile {
    name: String,
    text: String,
    /// Byte offset of the start of every line.
    line_starts: Vec<u32>,
}

impl SourceFile {
    /// A file named `name` holding `text`.
    pub fn new(name: impl Into<String>, text: impl Into<String>) -> Self {
        let text = text.into();
        let mut line_starts = vec![0];
        line_starts.extend(
            text.bytes()
                .enumerate()
                .filter(|&(_, b)| b == b'\n')
                .map(|(i, _)| i as u32 + 1),
        );
        Self {
            name: name.into(),
            text,
            line_starts,
        }
    }

    /// The name the compiler knows this file by.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The text of the file.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The length of the file in bytes.
    pub fn len(&self) -> u32 {
        self.text.len() as u32
    }

    /// Whether the file is empty.
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// The position of a byte offset. An offset past the end gives the end of the file.
    pub fn position(&self, offset: u32) -> Position {
        let offset = offset.min(self.len());
        let line = match self.line_starts.binary_search(&offset) {
            Ok(index) => index,
            Err(index) => index - 1,
        };
        Position {
            line: line as u32 + 1,
            column: offset - self.line_starts[line] + 1,
        }
    }

    /// The text of a one-based line, without its line ending, or `None` if there is no such
    /// line.
    pub fn line(&self, line: u32) -> Option<&str> {
        let index = (line as usize).checked_sub(1)?;
        let start = *self.line_starts.get(index)? as usize;
        let end = self
            .line_starts
            .get(index + 1)
            .map_or(self.text.len(), |&offset| offset as usize);
        Some(self.text[start..end].trim_end_matches(['\r', '\n']))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_count_from_one() {
        let file = SourceFile::new("t.c", "ab\ncde\n\nf");
        let cases = [
            (0, (1, 1)),
            (1, (1, 2)),
            (2, (1, 3)), // the newline itself
            (3, (2, 1)),
            (6, (2, 4)),
            (7, (3, 1)), // the empty line
            (8, (4, 1)),
            (99, (4, 2)), // past the end clamps
        ];
        for (offset, (line, column)) in cases {
            assert_eq!(
                file.position(offset),
                Position { line, column },
                "offset {offset}"
            );
        }
    }

    #[test]
    fn lines_strip_their_line_ending() {
        let file = SourceFile::new("t.c", "ab\r\ncd\n\nef");
        assert_eq!(file.line(1), Some("ab"));
        assert_eq!(file.line(2), Some("cd"));
        assert_eq!(file.line(3), Some(""));
        assert_eq!(file.line(4), Some("ef"));
        assert_eq!(file.line(5), None);
        assert_eq!(file.line(0), None);
    }

    #[test]
    fn an_empty_file_has_position_one() {
        let file = SourceFile::new("t.c", "");
        assert!(file.is_empty());
        assert_eq!(file.position(0), Position { line: 1, column: 1 });
        assert_eq!(file.line(1), Some(""));
    }
}
