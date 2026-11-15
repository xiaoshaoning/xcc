//! Lexing: the translation phases before tokens.
//!
//! C99's first two phases happen here, in one place so that nothing downstream has to know
//! about them: trigraphs are replaced, escaped newlines are spliced out, and CRLF and CR are
//! read as newlines. The result is a [`Prepared`] text plus a map back to the bytes it came
//! from, so a diagnostic points where the writer pointed, not where the normalized text
//! happens to sit.
//!
//! Then the tokens: identifiers, numbers, string and character literals, punctuators. Numbers
//! are *pp-numbers* (C99 6.4.8), the unclassified form the preprocessor works on; whether
//! `0x1p3` is a floating constant or `1e+` is nonsense is a question for the parser (M3),
//! which is the only stage that knows what context a number appears in.

use crate::diag::{Diagnostic, Span};

/// C99 6.4.4.4: the two-character sequences, and the character each stands for.
const TRIGRAPHS: [(u8, u8); 9] = [
    (b'=', b'#'),
    (b'/', b'\\'),
    (b'\'', b'^'),
    (b'(', b'['),
    (b')', b']'),
    (b'!', b'|'),
    (b'<', b'{'),
    (b'>', b'}'),
    (b'-', b'~'),
];

/// Source text after translation phases 1 and 2, and where each byte came from.
#[derive(Debug, Clone)]
pub struct Prepared {
    text: String,
    /// For every byte of `text`, the offset in the original source it came from.
    map: Vec<u32>,
}

impl Prepared {
    /// Normalize `source`: trigraphs, escaped newlines, CRLF and CR.
    pub fn new(source: &str) -> Self {
        let bytes = source.as_bytes();
        let mut text = Vec::with_capacity(bytes.len());
        let mut map = Vec::with_capacity(bytes.len());
        let mut at = 0;
        while at < bytes.len() {
            // Phase 1: a trigraph is one character, credited to its first byte.
            if let Some(to) = trigraph_at(bytes, at) {
                text.push(to);
                map.push(at as u32);
                at += 3;
                continue;
            }
            // Phase 2: backslash-newline disappears, together with the newline.
            if bytes[at] == b'\\' {
                let mut after = at + 1;
                if bytes.get(after) == Some(&b'\r') {
                    after += 1;
                }
                if bytes.get(after) == Some(&b'\n') {
                    at = after + 1;
                    continue;
                }
            }
            // A newline is a newline, however the file spells it.
            if bytes[at] == b'\r' {
                text.push(b'\n');
                map.push(at as u32);
                at += if bytes.get(at + 1) == Some(&b'\n') {
                    2
                } else {
                    1
                };
                continue;
            }
            text.push(bytes[at]);
            map.push(at as u32);
            at += 1;
        }
        Self {
            text: match String::from_utf8(text) {
                Ok(text) => text,
                // Cannot happen: only ASCII bytes are removed or replaced in valid UTF-8, so
                // this branch exists to keep a bug from becoming a panic (CONSTRAINTS 6).
                Err(error) => String::from_utf8_lossy(error.as_bytes()).into_owned(),
            },
            map,
        }
    }

    /// The normalized text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Whether there is nothing to lex.
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// The original offset of a byte of the normalized text; the end of the source for
    /// offsets past its end.
    pub fn original(&self, offset: u32) -> u32 {
        match self.map.get(offset as usize) {
            Some(&offset) => offset,
            None => self.map.last().copied().unwrap_or(0) + 1,
        }
    }
}

/// The character a trigraph at `at` stands for, if there is one there.
fn trigraph_at(bytes: &[u8], at: usize) -> Option<u8> {
    if bytes[at] != b'?' || bytes.get(at + 1) != Some(&b'?') {
        return None;
    }
    TRIGRAPHS
        .iter()
        .find(|(from, _)| Some(from) == bytes.get(at + 2))
        .map(|&(_, to)| to)
}

/// What a token is. Identifiers are not yet keywords: which identifiers are keywords depends
/// on the language level, which the parser knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// An identifier or a keyword.
    Ident,
    /// A pp-number (C99 6.4.8).
    Number,
    /// A string literal, with any `L` prefix.
    Str,
    /// A character literal, with any `L` prefix.
    Chr,
    /// A punctuator, spelled as C writes it; digraphs are given their primary spelling.
    Punct(&'static str),
    /// The end of the file.
    Eof,
}

/// One token: what it is, and where it came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    /// What it is.
    pub kind: TokenKind,
    /// Its bytes in the original source.
    pub span: Span,
}

/// The tokens of a source, and whatever was wrong with it.
#[derive(Debug, Default, Clone)]
pub struct Lexed {
    /// The tokens, ending with [`TokenKind::Eof`].
    pub tokens: Vec<Token>,
    /// Lexical errors and warnings, in source order.
    pub diagnostics: Vec<Diagnostic>,
}

impl Lexed {
    /// Whether anything went wrong badly enough to stop.
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(Diagnostic::is_error)
    }
}

/// The punctuators, longest first so that `<<=` is not lexed as `<<` then `=`.
const PUNCTUATORS: [&str; 55] = [
    "%:%:", "<<=", ">>=", "...", "->*", "<:", ":>", "<%", "%>", "%:", "##", "&&", "||", "++", "--",
    "->", "<<", ">>", "<=", ">=", "==", "!=", "+=", "-=", "*=", "/=", "%=", "&=", "^=", "|=", "[",
    "]", "(", ")", "{", "}", ".", "&", "*", "+", "-", "~", "!", "/", "%", "<", ">", "^", "|", "?",
    ":", ";", "=", ",", "#",
];

/// Lex a prepared source, which the caller calls `file`.
pub fn lex(prepared: &Prepared, file: &str) -> Lexed {
    let mut lexer = Lexer {
        bytes: prepared.text.as_bytes(),
        prepared,
        file,
        at: 0,
        lexed: Lexed::default(),
    };
    lexer.run();
    lexer.lexed
}

struct Lexer<'a> {
    bytes: &'a [u8],
    prepared: &'a Prepared,
    file: &'a str,
    at: usize,
    lexed: Lexed,
}

impl Lexer<'_> {
    fn run(&mut self) {
        loop {
            self.skip_blank();
            let start = self.at;
            let Some(&byte) = self.bytes.get(self.at) else {
                self.push(TokenKind::Eof, start, start);
                return;
            };
            let kind = if byte == b'"' || (byte == b'L' && self.peek(1) == Some(b'"')) {
                self.string_literal()
            } else if byte == b'\'' || (byte == b'L' && self.peek(1) == Some(b'\'')) {
                self.char_literal()
            } else if is_ident_start(byte) {
                self.identifier()
            } else if byte.is_ascii_digit() || (byte == b'.' && self.digits_follow()) {
                self.number()
            } else {
                match self.punctuator() {
                    Some(kind) => kind,
                    None => {
                        let what = if byte.is_ascii_graphic() {
                            format!("stray `{}` in program", byte as char)
                        } else {
                            format!("stray byte 0x{byte:02x} in program")
                        };
                        self.error(start, 1, what);
                        self.at += 1;
                        continue;
                    }
                }
            };
            self.push(kind, start, self.at);
        }
    }

    /// Skip whitespace and comments. An unterminated comment is an error at its opening.
    fn skip_blank(&mut self) {
        while let Some(&byte) = self.bytes.get(self.at) {
            match byte {
                b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r' => self.at += 1,
                b'/' if self.peek(1) == Some(b'/') => {
                    while let Some(&byte) = self.bytes.get(self.at) {
                        self.at += 1;
                        if byte == b'\n' {
                            break;
                        }
                    }
                }
                b'/' if self.peek(1) == Some(b'*') => {
                    let start = self.at;
                    self.at += 2;
                    loop {
                        match self.bytes.get(self.at) {
                            None => {
                                self.error(start, 2, "comment is not terminated");
                                return;
                            }
                            Some(b'*') if self.peek(1) == Some(b'/') => {
                                self.at += 2;
                                break;
                            }
                            Some(_) => self.at += 1,
                        }
                    }
                }
                _ => return,
            }
        }
    }

    fn identifier(&mut self) -> TokenKind {
        while self
            .bytes
            .get(self.at)
            .is_some_and(|&b| is_ident_continue(b))
        {
            self.at += 1;
        }
        TokenKind::Ident
    }

    /// A pp-number: it starts a number and then takes almost anything, which is what lets the
    /// preprocessor carry `1e+5` and `0x1p-3` around without knowing C's arithmetic.
    fn number(&mut self) -> TokenKind {
        self.at += 1;
        while let Some(&byte) = self.bytes.get(self.at) {
            let continues = byte.is_ascii_alphanumeric()
                || byte == b'_'
                || byte == b'.'
                || matches!(byte, b'+' | b'-')
                    && self
                        .bytes
                        .get(self.at - 1)
                        .is_some_and(|&b| matches!(b, b'e' | b'E' | b'p' | b'P'));
            if !continues {
                break;
            }
            self.at += 1;
        }
        TokenKind::Number
    }

    fn string_literal(&mut self) -> TokenKind {
        if self.bytes[self.at] == b'L' {
            self.at += 1;
        }
        self.quoted(b'"', "string")
    }

    fn char_literal(&mut self) -> TokenKind {
        if self.bytes[self.at] == b'L' {
            self.at += 1;
        }
        self.quoted(b'\'', "character constant")
    }

    /// Scan from the opening quote to its partner, honouring backslash escapes.
    fn quoted(&mut self, quote: u8, what: &str) -> TokenKind {
        let start = self.at;
        self.at += 1;
        loop {
            match self.bytes.get(self.at) {
                None => {
                    self.error(start, 1, format!("{what} is not terminated"));
                    break;
                }
                Some(b'\n') => {
                    self.error(
                        start,
                        1,
                        format!("{what} is not terminated before the end of its line"),
                    );
                    break;
                }
                Some(b'\\') => {
                    self.at += 2;
                }
                Some(&byte) if byte == quote => {
                    self.at += 1;
                    break;
                }
                Some(_) => self.at += 1,
            }
        }
        if quote == b'"' {
            TokenKind::Str
        } else {
            TokenKind::Chr
        }
    }

    /// The longest punctuator at the cursor, with digraphs spelled as their primary forms.
    fn punctuator(&mut self) -> Option<TokenKind> {
        let rest = &self.prepared.text[self.at..];
        for spelling in PUNCTUATORS {
            if rest.starts_with(spelling) {
                self.at += spelling.len();
                return Some(TokenKind::Punct(primary(spelling)));
            }
        }
        None
    }

    fn digits_follow(&self) -> bool {
        self.peek(1).is_some_and(|b| b.is_ascii_digit())
    }

    fn peek(&self, ahead: usize) -> Option<u8> {
        self.bytes.get(self.at + ahead).copied()
    }

    fn push(&mut self, kind: TokenKind, start: usize, end: usize) {
        let span = self.span(start, end);
        self.lexed.tokens.push(Token { kind, span });
    }

    /// A span in the original source: a spliced token covers from its first byte to its last.
    fn span(&self, start: usize, end: usize) -> Span {
        let from = self.prepared.original(start as u32);
        let to = if end == start {
            from
        } else {
            self.prepared.original(end as u32 - 1) + 1
        };
        Span::new(from, to.saturating_sub(from))
    }

    fn error(&mut self, start: usize, len: usize, message: impl Into<String>) {
        let span = self.span(start, start + len);
        self.lexed.diagnostics.push(
            Diagnostic::error(message)
                .with_file(self.file)
                .with_span(span),
        );
    }
}

/// Digraphs are their two characters everywhere after the lexer (C99 6.4.6).
fn primary(spelling: &'static str) -> &'static str {
    match spelling {
        "<:" => "[",
        ":>" => "]",
        "<%" => "{",
        "%>" => "}",
        "%:" => "#",
        "%:%:" => "##",
        other => other,
    }
}

fn is_ident_start(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphabetic()
}

fn is_ident_continue(byte: u8) -> bool {
    is_ident_start(byte) || byte.is_ascii_digit()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(source: &str) -> (Vec<TokenKind>, Vec<Diagnostic>) {
        let prepared = Prepared::new(source);
        let lexed = lex(&prepared, "t.c");
        (
            lexed.tokens.iter().map(|t| t.kind).collect(),
            lexed.diagnostics,
        )
    }

    fn kinds(source: &str) -> Vec<TokenKind> {
        let (mut kinds, diagnostics) = tokens(source);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        kinds.pop(); // Eof
        kinds
    }

    #[test]
    fn identifiers_and_keywords_are_the_same_token() {
        assert_eq!(
            kinds("int x; _y9"),
            [
                TokenKind::Ident,
                TokenKind::Ident,
                TokenKind::Punct(";"),
                TokenKind::Ident,
            ]
        );
    }

    #[test]
    fn numbers_are_pp_numbers() {
        for text in ["0", "42u", "0x1f", "1e+5", "0x1p-3", "1.5f", ".5", "1e"] {
            assert_eq!(kinds(text), [TokenKind::Number], "{text}");
        }
        // a pp-number swallows what follows an exponent sign, and stops at the next token
        assert_eq!(
            kinds("1e+5+2"),
            [TokenKind::Number, TokenKind::Punct("+"), TokenKind::Number]
        );
    }

    #[test]
    fn a_dot_alone_is_a_punctuator() {
        assert_eq!(
            kinds("s.x"),
            [TokenKind::Ident, TokenKind::Punct("."), TokenKind::Ident]
        );
    }

    #[test]
    fn literals_keep_their_escapes_intact() {
        assert_eq!(
            kinds(r#""a\"b" 'x' '\n' L"w" L'w'"#),
            [
                TokenKind::Str,
                TokenKind::Chr,
                TokenKind::Chr,
                TokenKind::Str,
                TokenKind::Chr,
            ]
        );
    }

    #[test]
    fn punctuators_take_the_longest_match() {
        assert_eq!(
            kinds("a<<=b>>=c...d->e"),
            [
                TokenKind::Ident,
                TokenKind::Punct("<<="),
                TokenKind::Ident,
                TokenKind::Punct(">>="),
                TokenKind::Ident,
                TokenKind::Punct("..."),
                TokenKind::Ident,
                TokenKind::Punct("->"),
                TokenKind::Ident,
            ]
        );
    }

    #[test]
    fn digraphs_become_their_primary_spelling() {
        assert_eq!(
            kinds("<% %> <: :> %: %:%:"),
            [
                TokenKind::Punct("{"),
                TokenKind::Punct("}"),
                TokenKind::Punct("["),
                TokenKind::Punct("]"),
                TokenKind::Punct("#"),
                TokenKind::Punct("##"),
            ]
        );
    }

    #[test]
    fn comments_are_blank() {
        assert_eq!(
            kinds("a /* b\nc */ d // e\nf"),
            [TokenKind::Ident, TokenKind::Ident, TokenKind::Ident]
        );
        let prepared = Prepared::new("/* unterminated");
        let lexed = lex(&prepared, "t.c");
        assert!(lexed.has_errors());
        assert_eq!(lexed.diagnostics[0].message, "comment is not terminated");
    }

    #[test]
    fn an_escaped_newline_joins_the_next_line() {
        assert_eq!(
            kinds("ab\\\ncd"),
            [TokenKind::Ident],
            "one identifier across a splice"
        );
        assert_eq!(
            kinds("1 + \\\n2"),
            [TokenKind::Number, TokenKind::Punct("+"), TokenKind::Number]
        );
    }

    #[test]
    fn trigraphs_are_phase_one() {
        assert_eq!(
            kinds("??=define"),
            [TokenKind::Punct("#"), TokenKind::Ident]
        );
        assert_eq!(kinds("??("), [TokenKind::Punct("[")]);
    }

    #[test]
    fn crlf_is_a_newline() {
        assert_eq!(kinds("a\r\nb\r\n"), [TokenKind::Ident, TokenKind::Ident]);
    }

    #[test]
    fn spans_point_at_the_original_bytes() {
        let source = "a ??=b\nx\\\ny";
        let prepared = Prepared::new(source);
        let lexed = lex(&prepared, "t.c");
        let kinds: Vec<_> = lexed.tokens.iter().map(|t| t.kind).collect();
        assert_eq!(
            kinds,
            [
                TokenKind::Ident,
                TokenKind::Punct("#"),
                TokenKind::Ident,
                TokenKind::Ident,
                TokenKind::Eof,
            ]
        );
        // `x\<newline>y` is one identifier covering both letters, splice included
        let last = lexed.tokens[3];
        assert_eq!(last.span, Span::new(7, 4));
        assert_eq!(&source[7..11], "x\\\ny");
    }

    #[test]
    fn unterminated_literals_say_so() {
        let prepared = Prepared::new("char *s = \"abc;\n");
        let lexed = lex(&prepared, "t.c");
        assert!(lexed.has_errors());
        assert_eq!(
            lexed.diagnostics[0].message,
            "string is not terminated before the end of its line"
        );
        assert_eq!(lexed.diagnostics[0].span, Some(Span::new(10, 1)));
    }

    #[test]
    fn a_stray_byte_is_reported_and_skipped() {
        let prepared = Prepared::new("a \u{7f} b");
        let lexed = lex(&prepared, "t.c");
        assert!(lexed.has_errors());
        assert_eq!(&lexed.diagnostics[0].message, "stray byte 0x7f in program");
        assert_eq!(lexed.tokens.len(), 3, "the tokens either side survive");
    }

    #[test]
    fn the_whole_corpus_lexes_without_panicking() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus");
        let mut files = 0;
        for entry in std::fs::read_dir(&root).expect("the corpus is in the repository") {
            let path = entry.expect("a directory entry").path();
            if path.extension().is_none_or(|extension| extension != "c") {
                continue;
            }
            let source = std::fs::read_to_string(&path).expect("a UTF-8 source");
            let lexed = lex(&Prepared::new(&source), "corpus.c");
            assert!(
                lexed.tokens.len() > 1,
                "{} lexed to nothing",
                path.display()
            );
            for token in &lexed.tokens {
                assert!(
                    token.span.start + token.span.len <= source.len() as u32,
                    "{}: span past the end",
                    path.display()
                );
            }
            files += 1;
        }
        assert!(files > 400, "expected the corpus, found {files} files");
    }
}
