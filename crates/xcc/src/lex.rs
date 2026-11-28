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
use crate::intern::{Interner, Symbol};

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
    /// A punctuator; digraphs are spelled as their primary form.
    Punct,
    /// A byte that is no token at all. Kept as a token rather than refused here because this layer
    /// runs *before* conditional inclusion: `sqlite3.c` has a page of Tcl inside `#if 0`, where `$`
    /// is not C and does not matter, and a lexer that judged it would refuse the file. The
    /// preprocessor reports one it is about to hand on, which is where the standard puts it.
    Stray,
    /// The end of the file.
    Eof,
}

/// One token: what it is, how it is spelled, and where it came from.
///
/// The spelling is a symbol, interned from the *prepared* text, so a digraph says `[` and a
/// spliced identifier says what it spells rather than how it was broken across lines. Every
/// later stage wants the spelling, and none of them should have to know about phases 1 and 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    /// What it is.
    pub kind: TokenKind,
    /// How it is spelled.
    pub text: Symbol,
    /// Its bytes in the original source.
    pub span: Span,
    /// Whether a newline that phase 2 did not splice away comes before it: the only thing that ends
    /// a preprocessor directive. A `\`-continued macro is one line however many it covers, and the
    /// line *number* cannot say so.
    pub starts_line: bool,
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

/// The punctuators of C99 6.4.6, longest first so that `<<=` is not lexed as `<<` then `=`.
/// `->*` is not here: it is C++'s, and a C lexer that invents it makes `p->*q` one token.
const PUNCTUATORS: [&str; 54] = [
    "%:%:", "<<=", ">>=", "...", "<:", ":>", "<%", "%>", "%:", "##", "&&", "||", "++", "--", "->",
    "<<", ">>", "<=", ">=", "==", "!=", "+=", "-=", "*=", "/=", "%=", "&=", "^=", "|=", "[", "]",
    "(", ")", "{", "}", ".", "&", "*", "+", "-", "~", "!", "/", "%", "<", ">", "^", "|", "?", ":",
    ";", "=", ",", "#",
];

/// Lex a prepared source, which the caller calls `file`, interning every spelling it meets.
pub fn lex(prepared: &Prepared, file: &str, interner: &mut Interner) -> Lexed {
    let mut lexer = Lexer {
        bytes: prepared.text.as_bytes(),
        prepared,
        file,
        at: 0,
        interner,
        lexed: Lexed::default(),
        starts_line: true,
    };
    lexer.run();
    lexer.lexed
}

struct Lexer<'a> {
    bytes: &'a [u8],
    prepared: &'a Prepared,
    file: &'a str,
    at: usize,
    interner: &'a mut Interner,
    lexed: Lexed,
    /// Whether a newline that phase 2 did *not* splice away has been skipped since the last token -
    /// the only thing that ends a preprocessor directive. It starts true: the first token of a file
    /// begins its first line.
    starts_line: bool,
}

impl Lexer<'_> {
    fn run(&mut self) {
        loop {
            self.skip_blank();
            let start = self.at;
            let Some(&byte) = self.bytes.get(self.at) else {
                self.push(TokenKind::Eof, "", start, start);
                return;
            };
            if self.is_literal_start() {
                let kind = self.literal();
                self.push_scanned(kind, start);
            } else if starts_identifier(byte) {
                let decoded = self.identifier(start);
                if self.at == start {
                    // An identifier that consumed nothing: a backslash that begins no universal
                    // character name. One byte, one token - and without this the loop asks again at
                    // the same byte for ever, which is a twenty-gigabyte allocation rather than a
                    // wrong answer.
                    self.at += 1;
                    self.push_scanned(TokenKind::Stray, start);
                } else if let Some(decoded) = decoded {
                    let end = self.at;
                    self.push(TokenKind::Ident, &decoded, start, end);
                } else {
                    self.push_scanned(TokenKind::Ident, start);
                }
            } else if byte.is_ascii_digit() || (byte == b'.' && self.digits_follow()) {
                self.number();
                self.push_scanned(TokenKind::Number, start);
            } else if let Some(spelling) = self.punctuator() {
                let end = self.at;
                self.push(TokenKind::Punct, spelling, start, end);
            } else {
                // One byte, one token; the diagnostic is the preprocessor's (`pp/mod.rs`), because a
                // group `#if 0` is skipping is not part of the program.
                self.at += 1;
                self.push_scanned(TokenKind::Stray, start);
            }
        }
    }

    /// Skip whitespace and comments. An unterminated comment is an error at its opening.
    fn skip_blank(&mut self) {
        while let Some(&byte) = self.bytes.get(self.at) {
            match byte {
                // A newline in the *prepared* text is one phase 2 let through, so it ends the logical
                // line a directive is read from - and the token after it begins a new one.
                b'\n' => {
                    self.starts_line = true;
                    self.at += 1;
                }
                b' ' | b'\t' | 0x0b | 0x0c | b'\r' => self.at += 1,
                b'/' if self.peek(1) == Some(b'/') => {
                    while let Some(&byte) = self.bytes.get(self.at) {
                        self.at += 1;
                        if byte == b'\n' {
                            // The comment swallowed the newline, and a newline is what says
                            // the next token begins a logical line. Without this the line
                            // after a `//` comment was invisible to directives - and a
                            // `#define` reads until the next line that starts one, so
                            // `#define A 1 // c` took the rest of the file as its body.
                            self.starts_line = true;
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

    /// Scan an identifier and answer the spelling it *decodes* to when a universal character name
    /// was met, or `None` when the token is the run of bytes in the prepared text.
    ///
    /// C99 6.4.2.1 puts a universal character name inside an identifier, and 6.4.3p2's rules make it
    /// the *same* identifier as the character it names - so the token's spelling is the character,
    /// not the six bytes of the escape, and `\u00e9` and `é` intern to one symbol. That is also why
    /// a keyword cannot be smuggled in: a character below `0xA0` is not a universal character name
    /// at all, unless it is `$`, `@` or the backtick, and none of those is an identifier character.
    ///
    /// A character outside ASCII travels as the bytes it is written in: the source is valid UTF-8 by
    /// the time it arrives, so its bytes are the spelling.
    fn identifier(&mut self, start: usize) -> Option<String> {
        let mut decoded: Option<String> = None;
        loop {
            let Some(&byte) = self.bytes.get(self.at) else {
                return decoded;
            };
            if byte == b'\\' {
                let from = self.at;
                let Some(character) = self.universal_character_name() else {
                    // Not a universal character name: the identifier **ends here**, and the backslash
                    // becomes a token of its own. C99 6.10.3p3 makes each non-white-space character
                    // that is no token a preprocessing token, so a macro's replacement list may hold
                    // one - `driverspecs.h` writes `(#kind, param \t)` - and it is an error only when
                    // it reaches the program. That question is the preprocessor's `emit`, which says
                    // so; this one may not, because a directive's words never pass through it.
                    return decoded;
                };
                let text = match &mut decoded {
                    Some(text) => text,
                    None => decoded.insert(self.prepared.text[start..from].to_string()),
                };
                text.push(character);
                continue;
            }
            if starts_identifier(byte) || byte.is_ascii_digit() {
                let from = self.at;
                self.at += 1;
                // The rest of a UTF-8 sequence travels with its first byte: the prepared text is
                // valid UTF-8, so those bytes are the spelling.
                while self.bytes.get(self.at).is_some_and(|&b| b & 0xC0 == 0x80) {
                    self.at += 1;
                }
                if let Some(text) = &mut decoded {
                    // Bytes after a decoded name are part of the same identifier: `a\u00e9b` is
                    // three characters, and stopping at two was a symbol nothing could find.
                    text.push_str(&self.prepared.text[from..self.at]);
                }
                continue;
            }
            return decoded;
        }
    }

    /// Read a universal character name where the cursor is, advancing past it, and say what
    /// character it names. `None` means it is not one and the error says which rule it broke.
    fn universal_character_name(&mut self) -> Option<char> {
        let start = self.at;
        self.at += 1;
        let digits = match self.bytes.get(self.at) {
            Some(b'u') => 4,
            Some(b'U') => 8,
            _ => {
                // Not a universal character name at all - a bare backslash - and **no diagnostic**:
                // the caller ends the identifier here, the scanner makes the byte a token of its own,
                // and whether that is a mistake is decided where the token ends up. C99 6.10.3p3
                // makes it a preprocessing token, so a directive's replacement list may hold one -
                // the Windows SDK's `driverspecs.h` writes `(#kind, param\t)` - and only a token that
                // reaches the program is an error. Which is what the preprocessor's `emit` says, and
                // the reason nothing here may say it: a directive's words never pass through `emit`,
                // and the ones that do are program text by definition.
                self.at = start;
                return None;
            }
        };
        self.at += 1;
        let from = self.at;
        let character = crate::literal::universal_character_name(self.bytes, &mut self.at, digits);
        let Some(character) = character else {
            // One diagnostic, naming the rule - which is the standard's, not this compiler's. Its
            // span is what the reader got through: all four digits when they were hex and wrong,
            // and only `\u` when a digit was not one, because the reader stops at the first it
            // would have had to read.
            let end = self.at.max(from);
            self.error(
                start,
                end - start,
                "a universal character name is four hex digits after `\\u` or eight after `\\U`, naming a character at or above `0xA0` that is not a surrogate (C99 6.4.3p2)",
            );
            return None;
        };
        // `$`, `@` and the backtick are the three below `0xA0` a universal character name may name,
        // and none of them is part of an identifier.
        if character < '\u{a0}' {
            self.error(
                start,
                self.at - start,
                format!("`{character}` is not part of an identifier, so a universal character name may not spell it"),
            );
            return None;
        }
        Some(character)
    }

    /// A pp-number: it starts a number and then takes almost anything, which is what lets the
    /// preprocessor carry `1e+5` and `0x1p-3` around without knowing C's arithmetic.
    fn number(&mut self) {
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
    }

    /// How many bytes of encoding prefix a literal at the cursor has: `L`, `u`, `U` is one, `u8` is
    /// two, and none is zero. C11 6.4.5p1, and the prefix only counts when a quote follows it - or
    /// an identifier called `u8` would be read as the start of a literal.
    fn literal_prefix(&self) -> usize {
        let quote_follows = |ahead: usize| {
            self.peek(ahead)
                .is_some_and(|byte| byte == b'"' || byte == b'\'')
        };
        if self.peek(0) == Some(b'u') && self.peek(1) == Some(b'8') && quote_follows(2) {
            return 2;
        }
        if matches!(self.peek(0), Some(b'L' | b'u' | b'U')) && quote_follows(1) {
            return 1;
        }
        0
    }

    /// Whether a literal starts here, of either kind and with any prefix: the byte after the
    /// prefix is a quote. With no prefix that byte is the one here, which is the same question -
    /// so one clause, not two.
    fn is_literal_start(&self) -> bool {
        matches!(self.peek(self.literal_prefix()), Some(b'"' | b'\''))
    }

    /// Scan a literal from its prefix: the quote after the prefix says which kind it is.
    fn literal(&mut self) -> TokenKind {
        self.at += self.literal_prefix();
        if self.peek(0) == Some(b'"') {
            self.quoted(b'"', "string")
        } else {
            self.quoted(b'\'', "character constant")
        }
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
    fn punctuator(&mut self) -> Option<&'static str> {
        let rest = &self.prepared.text[self.at..];
        for spelling in PUNCTUATORS {
            if rest.starts_with(spelling) {
                self.at += spelling.len();
                return Some(primary(spelling));
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

    fn push(&mut self, kind: TokenKind, text: &str, start: usize, end: usize) {
        let span = self.span(start, end);
        let text = self.interner.intern(text);
        let starts_line = std::mem::replace(&mut self.starts_line, false);
        self.lexed.tokens.push(Token {
            kind,
            text,
            span,
            starts_line,
        });
    }

    /// Push the token that scanning has just passed, spelled as it is written. The spelling is
    /// interned straight out of the prepared text: no `String` is made for a token that is
    /// already in a buffer.
    fn push_scanned(&mut self, kind: TokenKind, start: usize) {
        let prepared = self.prepared;
        let text = self.interner.intern(&prepared.text[start..self.at]);
        let span = self.span(start, self.at);
        let starts_line = std::mem::replace(&mut self.starts_line, false);
        self.lexed.tokens.push(Token {
            kind,
            text,
            span,
            starts_line,
        });
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

/// Whether a byte can begin an identifier: a letter or `_`, a character outside ASCII - which is
/// one of the bytes of its UTF-8 sequence - or the backslash of a universal character name
/// (C99 6.4.2.1). The continuation rule is this one with digits, in `identifier`, which is the only
/// place that asks: a second list would have to be told what `\\` and `>= 0x80` mean all over again.
fn starts_identifier(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphabetic() || byte == b'\\' || byte >= 0x80
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lexed(source: &str) -> (Lexed, Interner) {
        let prepared = Prepared::new(source);
        let mut interner = Interner::new();
        let lexed = lex(&prepared, "t.c", &mut interner);
        (lexed, interner)
    }

    /// The spellings of the tokens, without the `Eof`, asserting nothing went wrong.
    fn spelled(source: &str) -> Vec<String> {
        let (lexed, interner) = lexed(source);
        assert!(lexed.diagnostics.is_empty(), "{:?}", lexed.diagnostics);
        assert_eq!(lexed.tokens.last().unwrap().kind, TokenKind::Eof);
        let mut spells: Vec<String> = lexed
            .tokens
            .iter()
            .map(|token| interner.resolve(token.text).to_owned())
            .collect();
        spells.pop();
        spells
    }

    /// A `//` comment eats its own newline, so it has to say so: a newline is what tells the next
    /// token it begins a logical line, and a directive reads until one does.
    ///
    /// Without this, `#define A 1 // c` read to the *end of the file* as its body - the line after
    /// the comment was invisible to every directive, and stb_image.h is written that way.
    #[test]
    fn a_line_comment_still_ends_the_line() {
        let (lexed, interner) = lexed(
            "#define A 1 // c
int x;
",
        );
        let after: Vec<&str> = lexed
            .tokens
            .iter()
            // The end of the file starts a line too, and is not a token anyone means.
            .filter(|token| token.starts_line && token.kind != TokenKind::Eof)
            .map(|token| interner.resolve(token.text))
            .collect();
        // Exactly these two begin a logical line - the `#` and the `int`. A `contains` here would
        // pass for a lexer that marked *every* token, which is the failure the flag exists to
        // prevent, so the set is the assertion.
        assert_eq!(
            after,
            ["#", "int"],
            "the line after a `//` comment starts one, and the comment's own line does not"
        );
    }

    #[test]
    fn identifiers_and_keywords_are_the_same_token() {
        let (lexed, _) = lexed("int x; _y9");
        let kinds: Vec<TokenKind> = lexed.tokens.iter().map(|t| t.kind).collect();
        assert_eq!(
            kinds,
            [
                TokenKind::Ident,
                TokenKind::Ident,
                TokenKind::Punct,
                TokenKind::Ident,
                TokenKind::Eof
            ]
        );
        assert_eq!(spelled("int x; _y9"), ["int", "x", ";", "_y9"]);
    }

    #[test]
    fn numbers_are_pp_numbers() {
        for text in ["0", "42u", "0x1f", "1e+5", "0x1p-3", "1.5f", ".5", "1e"] {
            assert_eq!(spelled(text), [text], "{text}");
        }
        // a pp-number takes what follows an exponent sign, and stops at the next operator
        assert_eq!(spelled("1e+5+2"), ["1e+5", "+", "2"]);
    }

    #[test]
    fn a_dot_alone_is_a_punctuator() {
        assert_eq!(spelled("s.x"), ["s", ".", "x"]);
    }

    #[test]
    fn literals_keep_their_escapes_intact() {
        assert_eq!(
            spelled(r#""a\"b" 'x' '\n' L"w" L'w'"#),
            [r#""a\"b""#, "'x'", r"'\n'", r#"L"w""#, "L'w'"]
        );
    }

    #[test]
    fn punctuators_take_the_longest_match() {
        assert_eq!(
            spelled("a<<=b>>=c...d->e"),
            ["a", "<<=", "b", ">>=", "c", "...", "d", "->", "e"]
        );
    }

    #[test]
    fn digraphs_become_their_primary_spelling() {
        assert_eq!(
            spelled("<% %> <: :> %: %:%:"),
            ["{", "}", "[", "]", "#", "##"]
        );
    }

    #[test]
    fn comments_are_blank() {
        assert_eq!(spelled("a /* b\nc */ d // e\nf"), ["a", "d", "f"]);
        let (lexed, _) = lexed("/* unterminated");
        assert!(lexed.has_errors());
        assert_eq!(lexed.diagnostics[0].message, "comment is not terminated");
    }

    #[test]
    fn an_escaped_newline_joins_the_next_line() {
        assert_eq!(
            spelled("ab\\\ncd"),
            ["abcd"],
            "one identifier across a splice"
        );
        assert_eq!(spelled("1 + \\\n2"), ["1", "+", "2"]);
    }

    #[test]
    fn trigraphs_are_phase_one() {
        assert_eq!(spelled("??=define"), ["#", "define"]);
        assert_eq!(spelled("??("), ["["]);
    }

    #[test]
    fn crlf_is_a_newline() {
        assert_eq!(spelled("a\r\nb\r\n"), ["a", "b"]);
    }

    #[test]
    fn spans_point_at_the_original_bytes() {
        let source = "a ??=b\nx\\\ny";
        let (lexed, interner) = lexed(source);
        let spells: Vec<&str> = lexed
            .tokens
            .iter()
            .map(|token| interner.resolve(token.text))
            .collect();
        assert_eq!(
            spells,
            ["a", "#", "b", "xy", ""],
            "the spelling is what it spells, with the splice gone"
        );
        // `x\<newline>y` is one identifier covering both letters, splice included
        assert_eq!(lexed.tokens[3].span, Span::new(7, 4));
        assert_eq!(&source[7..11], "x\\\ny");
    }

    #[test]
    fn unterminated_literals_say_so() {
        let (lexed, _) = lexed("char *s = \"abc;\n");
        assert!(lexed.has_errors());
        assert_eq!(
            lexed.diagnostics[0].message,
            "string is not terminated before the end of its line"
        );
        assert_eq!(lexed.diagnostics[0].span, Some(Span::new(10, 1)));
    }

    /// A backslash that begins no universal character name **ends** the identifier and becomes a
    /// token of its own. It is not a diagnostic here: C99 6.10.3p3 makes it a preprocessing token, so
    /// a macro's replacement list may hold one - the Windows SDK's `driverspecs.h` writes
    /// `(#kind, param backslash-t)` - and the preprocessor says a word about it only if it reaches the
    /// program.
    #[test]
    fn a_backslash_ends_an_identifier_and_is_a_token_of_its_own() {
        let (lexed, _) = lexed("a\\tb");
        assert!(lexed.diagnostics.is_empty(), "{:?}", lexed.diagnostics);
        let kinds: Vec<TokenKind> = lexed.tokens.iter().map(|token| token.kind).collect();
        assert_eq!(
            kinds,
            vec![
                TokenKind::Ident,
                TokenKind::Stray,
                TokenKind::Ident,
                TokenKind::Eof
            ],
            "`a`, the backslash, `tb`"
        );
    }

    #[test]
    fn a_stray_byte_is_a_token_and_not_a_diagnostic() {
        // Phase 3 makes tokens; phase 4 decides what is in the program, so a byte that is no
        // token is kept as one and reported by the preprocessor only where it survives - a page
        // of Tcl inside `#if 0` is the case (`sqlite3.c`).
        let (lexed, _) = lexed("a \u{7f} b");
        assert!(!lexed.has_errors(), "the lexer does not judge");
        assert_eq!(
            lexed.tokens.len(),
            4,
            "the tokens either side survive, and the byte between"
        );
        assert_eq!(lexed.tokens[1].kind, TokenKind::Stray);
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
            let mut interner = Interner::new();
            let lexed = lex(&Prepared::new(&source), "corpus.c", &mut interner);
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
