//! Preprocessing: directives, macros, and the token stream the parser will read.
//!
//! What exists today is the spine: object-like `#define` and `#undef`, and a diagnostic for
//! every directive that is not implemented yet. That last part is deliberate - a directive the
//! preprocessor cannot honour is an error, never silence (CONSTRAINTS 3) - so a source using
//! conditionals or includes fails loudly at M2.1 instead of preprocessing into nonsense.
//!
//! The output is a flat token vector: what the parser gets, with every spelling interned and
//! every span pointing into the original source.

use std::collections::{HashMap, HashSet};

use crate::diag::{Diagnostic, Span};
use crate::intern::{Interner, Symbol};
use crate::lex::{self, Prepared, Token, TokenKind};
use crate::options::Budgets;
use crate::source::SourceFile;

/// One token after preprocessing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PpToken {
    /// What it is.
    pub kind: TokenKind,
    /// How it is spelled.
    pub text: Symbol,
    /// Its bytes in the original source.
    pub span: Span,
}

/// The tokens of a source, and whatever was wrong with it.
#[derive(Debug, Default, Clone)]
pub struct Preprocessed {
    /// The tokens, ending with [`TokenKind::Eof`].
    pub tokens: Vec<PpToken>,
    /// Errors and warnings, in source order.
    pub diagnostics: Vec<Diagnostic>,
}

impl Preprocessed {
    /// Whether anything went wrong badly enough to stop.
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(Diagnostic::is_error)
    }
}

/// Preprocess `source`, interning the spellings of everything it keeps.
pub fn preprocess(source: &SourceFile, budgets: &Budgets, interner: &mut Interner) -> Preprocessed {
    let prepared = Prepared::new(source.text());
    let lexed = lex::lex(&prepared, source.name(), interner);
    if lexed.has_errors() {
        return Preprocessed {
            tokens: Vec::new(),
            diagnostics: lexed.diagnostics,
        };
    }
    let mut preprocessor = Preprocessor {
        source,
        budgets,
        interner,
        tokens: lexed.tokens,
        at: 0,
        out: Vec::new(),
        macros: HashMap::new(),
        diagnostics: lexed.diagnostics,
        reported: HashSet::new(),
        last_line: 0,
    };
    preprocessor.run();
    Preprocessed {
        tokens: preprocessor.out,
        diagnostics: preprocessor.diagnostics,
    }
}

/// A macro definition: its replacement tokens, unexpanded.
#[derive(Debug, Clone)]
struct Macro {
    body: Vec<PpToken>,
}

struct Preprocessor<'a> {
    source: &'a SourceFile,
    budgets: &'a Budgets,
    interner: &'a mut Interner,
    tokens: Vec<Token>,
    at: usize,
    out: Vec<PpToken>,
    macros: HashMap<Symbol, Macro>,
    diagnostics: Vec<Diagnostic>,
    /// Directives whose "not implemented yet" has already been said, once each.
    reported: HashSet<&'static str>,
    last_line: u32,
}

impl Preprocessor<'_> {
    fn run(&mut self) {
        while let Some(token) = self.next() {
            let line = self.line(token.span);
            let at_line_start = line > self.last_line;
            self.last_line = self.last_line.max(line);

            if at_line_start && token.kind == TokenKind::Punct && self.spell(token.text) == "#" {
                self.directive(token);
            } else {
                self.expand_token(token, &mut Vec::new());
            }
        }
    }

    /// Everything on the directive's line, and then the directive itself.
    fn directive(&mut self, hash: Token) {
        let line = self.line(hash.span);
        let mut words = Vec::new();
        while let Some(token) = self.peek() {
            if self.line(token.span) != line {
                break;
            }
            words.push(self.next().expect("peeked and present"));
        }

        let Some(name) = words.first().copied() else {
            return; // `#` alone is the null directive
        };
        if name.kind != TokenKind::Ident {
            self.error(name.span, "expected a directive name after `#`");
            return;
        }
        let name_text = self.name(name.text).to_owned();
        let rest = &words[1..];

        match name_text.as_str() {
            "define" => self.define(rest, name.span),
            "undef" => self.undef(rest),
            "error" => self.quoted_error(rest, name.span, true),
            "warning" => self.quoted_error(rest, name.span, false),
            "pragma" => self.pragma(rest, name.span),
            "include" | "include_next" => {
                self.error(name.span, "`#include` is not implemented yet (M2.4)")
            }
            "if" | "ifdef" | "ifndef" | "elif" | "else" | "endif" => {
                if self.reported.insert("conditional") {
                    self.error(
                        name.span,
                        "conditional compilation is not implemented yet (M2.3)",
                    );
                }
            }
            "line" => self.error(name.span, "`#line` is not implemented yet (M2.3)"),
            other => self.error(name.span, format!("unknown directive `#{other}`")),
        }
    }

    fn define(&mut self, rest: &[Token], keyword: Span) {
        let Some(name) = rest.first().copied() else {
            self.error(keyword, "expected a macro name after `#define`");
            return;
        };
        if name.kind != TokenKind::Ident {
            self.error(name.span, "a macro name must be an identifier");
            return;
        }
        // `#define f(x) ...` is function-like only when the `(` touches the name.
        if let Some(open) = rest.get(1) {
            if self.spell(open.text) == "(" && open.span.start == name.span.end() {
                self.error(
                    name.span,
                    "function-like macros are not implemented yet (M2.2)",
                );
                return;
            }
        }
        let body: Vec<PpToken> = rest[1..].iter().map(|token| self.as_pp(*token)).collect();
        if let Some(previous) = self.macros.get(&name.text) {
            let same = previous.body.len() == body.len()
                && previous
                    .body
                    .iter()
                    .zip(&body)
                    .all(|(a, b)| a.kind == b.kind && a.text == b.text);
            if !same {
                let named = self.name(name.text).to_owned();
                self.error(name.span, format!("redefinition of `{named}` is different"));
                return;
            }
        }
        self.macros.insert(name.text, Macro { body });
    }

    fn undef(&mut self, rest: &[Token]) {
        match rest.first() {
            Some(name) if name.kind == TokenKind::Ident => {
                self.macros.remove(&name.text);
            }
            _ => {
                let span = rest.first().map_or(Span::default(), |token| token.span);
                self.error(span, "expected a macro name after `#undef`");
            }
        }
    }

    /// `#error` and `#warning`: the rest of the line is the message.
    fn quoted_error(&mut self, rest: &[Token], span: Span, is_error: bool) {
        let message = if rest.is_empty() {
            if is_error {
                "#error".to_owned()
            } else {
                "#warning".to_owned()
            }
        } else {
            let mut text = String::new();
            for token in rest {
                if !text.is_empty() {
                    text.push(' ');
                }
                text.push_str(self.spell(token.text));
            }
            text
        };
        if is_error {
            self.error(span, message);
        } else {
            self.warning(span, message);
        }
    }

    fn pragma(&mut self, rest: &[Token], span: Span) {
        match rest.first() {
            Some(once) if self.spell(once.text) == "once" => {}
            Some(known) => {
                let name = self.spell(known.text).to_owned();
                self.warning(span, format!("unknown `#pragma {name}` ignored"));
            }
            None => self.warning(span, "empty `#pragma` ignored"),
        }
    }

    /// Expand a token if it names a macro, otherwise keep it.
    fn expand_token(&mut self, token: Token, disabled: &mut Vec<Symbol>) {
        let pp = self.as_pp(token);
        self.expand(pp, disabled);
    }

    fn expand(&mut self, token: PpToken, disabled: &mut Vec<Symbol>) {
        if token.kind == TokenKind::Ident && !disabled.contains(&token.text) {
            if let Some(definition) = self.macros.get(&token.text) {
                let body = definition.body.clone();
                if body.is_empty() {
                    return;
                }
                if disabled.len() >= self.budgets.max_macro_expansion {
                    self.error(
                        token.span,
                        format!(
                            "macro expansion is nested more than {} deep",
                            self.budgets.max_macro_expansion
                        ),
                    );
                    return;
                }
                disabled.push(token.text);
                for part in body {
                    self.expand(part, disabled);
                }
                disabled.pop();
                return;
            }
        }
        self.out.push(token);
    }

    fn as_pp(&self, token: Token) -> PpToken {
        PpToken {
            kind: token.kind,
            text: token.text,
            span: token.span,
        }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.at).copied();
        if token.is_some() {
            self.at += 1;
        }
        token
    }

    fn line(&self, span: Span) -> u32 {
        self.source.position(span.start).line
    }

    fn name(&self, symbol: Symbol) -> &str {
        self.interner.resolve(symbol)
    }

    fn spell(&self, symbol: Symbol) -> &str {
        self.interner.resolve(symbol)
    }

    fn error(&mut self, span: Span, message: impl Into<String>) {
        self.diagnostics.push(
            Diagnostic::error(message)
                .with_file(self.source.name())
                .with_span(span),
        );
    }

    fn warning(&mut self, span: Span, message: impl Into<String>) {
        self.diagnostics.push(
            Diagnostic::warning(message)
                .with_file(self.source.name())
                .with_span(span),
        );
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::diag::Severity;

    fn run(source: &str) -> (Vec<String>, Vec<Diagnostic>) {
        run_with(source, &Budgets::default())
    }

    fn run_with(source: &str, budgets: &Budgets) -> (Vec<String>, Vec<Diagnostic>) {
        let file = SourceFile::new("t.c", source);
        let mut interner = Interner::new();
        let preprocessed = preprocess(&file, budgets, &mut interner);
        let spells = preprocessed
            .tokens
            .iter()
            .map(|token| interner.resolve(token.text).to_owned())
            .collect();
        (spells, preprocessed.diagnostics)
    }

    /// The spellings, without the `Eof`, asserting nothing went wrong.
    fn spelled(source: &str) -> Vec<String> {
        let (mut spells, diagnostics) = run(source);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        spells.pop();
        spells
    }

    fn diagnostics(source: &str) -> Vec<Diagnostic> {
        run(source).1
    }

    #[test]
    fn an_object_like_macro_is_replaced() {
        assert_eq!(
            spelled("#define N 3\nint x = N;"),
            ["int", "x", "=", "3", ";"]
        );
    }

    #[test]
    fn macros_expand_through_macros() {
        assert_eq!(spelled("#define A B\n#define B 1\nA"), ["1"]);
        assert_eq!(spelled("#define A 1\n#define B A+2\nB"), ["1", "+", "2"]);
    }

    #[test]
    fn a_macro_does_not_expand_inside_itself() {
        assert_eq!(spelled("#define A A\nA"), ["A"]);
        assert_eq!(spelled("#define A B\n#define B A\nA"), ["A"]);
        assert_eq!(spelled("#define A B\n#define B A\nB"), ["B"]);
    }

    #[test]
    fn an_empty_macro_expands_to_nothing() {
        assert_eq!(spelled("#define E\nE x"), ["x"]);
        assert_eq!(spelled("#define E\nE"), Vec::<String>::new());
    }

    #[test]
    fn undef_forgets_the_macro() {
        assert_eq!(spelled("#define N 1\n#undef N\nN"), ["N"]);
        assert_eq!(
            spelled("#undef N\nN"),
            ["N"],
            "undefining nothing is not an error"
        );
    }

    #[test]
    fn redefinition_is_checked() {
        assert!(
            diagnostics("#define N 1\n#define N 1\nN").is_empty(),
            "identical is fine"
        );
        let errors = diagnostics("#define N 1\n#define N 2\n");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].message, "redefinition of `N` is different");
    }

    #[test]
    fn a_function_like_macro_is_refused_not_guessed() {
        let errors = diagnostics("#define f(x) x\nf(1)");
        assert_eq!(errors.len(), 1);
        assert!(errors[0]
            .message
            .contains("function-like macros are not implemented"));
        // and with a space before the parenthesis it is an object-like macro, as C says
        assert_eq!(spelled("#define f (x)\nf"), ["(", "x", ")"]);
    }

    #[test]
    fn directives_that_are_not_implemented_are_errors() {
        assert!(diagnostics("#include <stdio.h>")[0]
            .message
            .contains("M2.4"));
        let conditionals = diagnostics("#if 1\n#endif\n#if 0\n#endif\n");
        assert_eq!(conditionals.len(), 1, "said once, not once per line");
        assert!(conditionals[0].message.contains("M2.3"));
    }

    #[test]
    fn an_unknown_directive_is_an_error() {
        let errors = diagnostics("#frobnicate\n");
        assert_eq!(errors[0].message, "unknown directive `#frobnicate`");
    }

    #[test]
    fn error_and_warning_carry_their_text() {
        let errors = diagnostics("#error do not build this\n");
        assert_eq!(errors[0].severity, Severity::Error);
        assert_eq!(errors[0].message, "do not build this");

        let warnings = diagnostics("#warning this is odd\n");
        assert_eq!(warnings[0].severity, Severity::Warning);
        assert_eq!(warnings[0].message, "this is odd");
    }

    #[test]
    fn pragmas_are_ignored_with_an_answer() {
        assert!(diagnostics("#pragma once\n").is_empty());
        let warnings = diagnostics("#pragma pack(1)\n");
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].severity, Severity::Warning);
        assert_eq!(warnings[0].message, "unknown `#pragma pack` ignored");
    }

    #[test]
    fn only_a_hash_at_the_start_of_a_line_is_a_directive() {
        assert_eq!(spelled("#\nint x;"), ["int", "x", ";"]);
        assert_eq!(spelled("int x # y"), ["int", "x", "#", "y"]);
        assert_eq!(
            spelled("  #  define N 1\nN"),
            ["1"],
            "anything before the hash must be whitespace only"
        );
    }

    #[test]
    fn expansion_depth_is_bounded() {
        let mut source = String::from("#define M0 1\n");
        for step in 1..40 {
            source.push_str(&format!("#define M{step} M{}\n", step - 1));
        }
        source.push_str("M39\n");
        let budgets = Budgets {
            max_macro_expansion: 8,
            ..Budgets::default()
        };
        let (_, diagnostics) = run_with(&source, &budgets);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].message,
            "macro expansion is nested more than 8 deep"
        );
    }

    #[test]
    fn spans_survive_expansion() {
        let source = "#define N 3\nint x = N;\n";
        let file = SourceFile::new("t.c", source);
        let mut interner = Interner::new();
        let preprocessed = preprocess(&file, &Budgets::default(), &mut interner);
        let three = preprocessed
            .tokens
            .iter()
            .find(|token| interner.resolve(token.text) == "3")
            .expect("the expansion is in the output");
        assert_eq!(
            &source[three.span.start as usize..][..1],
            "3",
            "the span is the body's"
        );
    }
}
