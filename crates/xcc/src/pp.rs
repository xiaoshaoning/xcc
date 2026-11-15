//! Preprocessing: directives, macros, and the token stream the parser will read.
//!
//! Object-like and function-like macros both work: parameters, `#` stringification, `##>
//! pasting, `...`/`__VA_ARGS__`, argument prescan, rescanning after substitution, and the rule
//! that stops a macro expanding inside its own expansion (C99 6.10.3.4p2, "painted blue" here by
//! a set of names each spliced token carries).
//!
//! Directives that cannot be honoured yet are errors, never silence (CONSTRAINTS 3), so a
//! source using conditionals or includes fails loudly instead of preprocessing into nonsense.
//!
//! The output is a flat token vector: what the parser gets, with every spelling interned and
//! every span pointing into the original source.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::diag::{Diagnostic, Span};
use crate::intern::{Interner, Symbol};
use crate::lex::{self, Prepared, TokenKind};
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
        pending: lexed
            .tokens
            .iter()
            .map(|token| Raw {
                token: PpToken {
                    kind: token.kind,
                    text: token.text,
                    span: token.span,
                },
                hide: Vec::new(),
                spliced: false,
            })
            .collect(),
        out: Vec::new(),
        macros: HashMap::new(),
        diagnostics: lexed.diagnostics,
        reported: HashSet::new(),
        last_line: 0,
        depth: 0,
    };
    preprocessor.run();
    Preprocessed {
        tokens: preprocessor.out,
        diagnostics: preprocessor.diagnostics,
    }
}

/// A working token: what the parser will see, plus the names that may not expand it.
#[derive(Debug, Clone)]
struct Raw {
    token: PpToken,
    /// Macro names already being expanded here: C's "painted blue" (6.10.3.4p2).
    hide: Vec<Symbol>,
    /// Whether this token came from an expansion rather than the source: only source tokens
    /// can begin a directive.
    spliced: bool,
}

/// What a macro's parameter list says.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Params {
    /// The named parameters, in order.
    names: Vec<Symbol>,
    /// Whether the last parameter is `...`, reachable as `__VA_ARGS__`.
    variadic: bool,
}

/// A macro definition: its parameters (none for an object-like macro) and its body.
#[derive(Debug, Clone)]
struct Macro {
    params: Option<Params>,
    body: Vec<PpToken>,
}

struct Preprocessor<'a> {
    source: &'a SourceFile,
    budgets: &'a Budgets,
    interner: &'a mut Interner,
    /// The tokens still to look at, sources and expansions alike.
    pending: VecDeque<Raw>,
    out: Vec<PpToken>,
    macros: HashMap<Symbol, Macro>,
    diagnostics: Vec<Diagnostic>,
    /// Directives whose "not implemented yet" has already been said, once each.
    reported: HashSet<&'static str>,
    last_line: u32,
    /// How deep expansion is: prescanning an argument nests.
    depth: usize,
}

impl Preprocessor<'_> {
    fn run(&mut self) {
        while let Some(raw) = self.next() {
            if !raw.spliced {
                let line = self.line(raw.token.span);
                let at_line_start = line > self.last_line;
                self.last_line = self.last_line.max(line);
                if at_line_start && self.is_punct(&raw.token, "#") {
                    self.directive(raw);
                    continue;
                }
            }
            self.emit(raw);
        }
    }

    /// Keep a token, or splice what it expands to in front of the tokens still to come.
    fn emit(&mut self, raw: Raw) {
        if self.out.len() >= self.budgets.max_tokens {
            self.too_many_tokens(raw.token.span);
            return;
        }
        match self.expansion_of(&raw) {
            Some(expansion) => self.splice_front(expansion),
            None => self.out.push(raw.token),
        }
    }

    /// Everything the directive on this line does, having consumed the line.
    fn directive(&mut self, hash: Raw) {
        let line = self.line(hash.token.span);
        let mut words = Vec::new();
        while let Some(next) = self.peek() {
            if self.line(next.token.span) != line {
                break;
            }
            words.push(self.next().expect("peeked and present"));
        }

        let Some(name) = words.first() else {
            return; // `#` alone is the null directive
        };
        if name.token.kind != TokenKind::Ident {
            let span = name.token.span;
            self.error(span, "expected a directive name after `#`");
            return;
        }
        let name_text = self.spell(name.token.text).to_owned();
        let span = name.token.span;
        let rest = &words[1..];

        match name_text.as_str() {
            "define" => self.define(rest, span),
            "undef" => self.undef(rest),
            "error" => self.message_directive(rest, span, true),
            "warning" => self.message_directive(rest, span, false),
            "pragma" => self.pragma(rest, span),
            "include" | "include_next" => {
                self.error(span, "`#include` is not implemented yet (M2.4)")
            }
            "if" | "ifdef" | "ifndef" | "elif" | "else" | "endif" => {
                if self.reported.insert("conditional") {
                    self.error(
                        span,
                        "conditional compilation is not implemented yet (M2.3)",
                    );
                }
            }
            "line" => self.error(span, "`#line` is not implemented yet (M2.3)"),
            other => self.error(span, format!("unknown directive `#{other}`")),
        }
    }

    fn define(&mut self, rest: &[Raw], keyword: Span) {
        let Some(name) = rest.first() else {
            self.error(keyword, "expected a macro name after `#define`");
            return;
        };
        if name.token.kind != TokenKind::Ident {
            let span = name.token.span;
            self.error(span, "a macro name must be an identifier");
            return;
        }

        // `#define f(x)` is function-like only when the `(` touches the name.
        let function_like = rest.get(1).is_some_and(|open| {
            self.is_punct(&open.token, "(") && open.token.span.start == name.token.span.end()
        });

        let (params, body_start) = if function_like {
            match self.parameter_list(rest, name.token.span) {
                Some((params, next)) => (Some(params), next),
                None => return,
            }
        } else {
            (None, 1)
        };

        let body: Vec<PpToken> = rest[body_start..].iter().map(|raw| raw.token).collect();
        let variadic = params.as_ref().is_some_and(|params| params.variadic);
        if !variadic
            && body
                .iter()
                .any(|token| self.is_spelled(token, "__VA_ARGS__"))
        {
            let span = name.token.span;
            self.error(span, "`__VA_ARGS__` used in a macro without `...`");
            return;
        }

        let definition = Macro { params, body };
        if let Some(previous) = self.macros.get(&name.token.text) {
            let same = previous.params == definition.params
                && previous.body.len() == definition.body.len()
                && previous
                    .body
                    .iter()
                    .zip(&definition.body)
                    .all(|(a, b)| a.kind == b.kind && a.text == b.text);
            if !same {
                let named = self.spell(name.token.text).to_owned();
                let span = name.token.span;
                self.error(span, format!("redefinition of `{named}` is different"));
                return;
            }
        }
        self.macros.insert(name.token.text, definition);
    }

    /// The parameter list of a function-like macro, from the `(` at `rest[1]`.
    ///
    /// Returns the parameters and the index just past the `)`.
    fn parameter_list(&mut self, rest: &[Raw], name: Span) -> Option<(Params, usize)> {
        let mut params = Params::default();
        let mut at = 2;
        loop {
            let Some(token) = rest.get(at) else {
                self.error(name, "unterminated parameter list");
                return None;
            };
            if self.is_punct(&token.token, ")") {
                return Some((params, at + 1));
            }
            if self.is_punct(&token.token, "...") {
                let closes = rest
                    .get(at + 1)
                    .is_some_and(|next| self.is_punct(&next.token, ")"));
                if !closes {
                    let span = token.token.span;
                    self.error(span, "`...` must be the last parameter");
                    return None;
                }
                params.variadic = true;
                return Some((params, at + 2));
            }
            if token.token.kind != TokenKind::Ident {
                let span = token.token.span;
                self.error(span, "expected a parameter name");
                return None;
            }
            if params.names.contains(&token.token.text) {
                let named = self.spell(token.token.text).to_owned();
                let span = token.token.span;
                self.error(span, format!("duplicate parameter `{named}`"));
                return None;
            }
            params.names.push(token.token.text);
            at += 1;
            match rest.get(at) {
                Some(separator) if self.is_punct(&separator.token, ",") => at += 1,
                Some(close) if self.is_punct(&close.token, ")") => return Some((params, at + 1)),
                _ => {
                    let span = rest.get(at).map_or(name, |token| token.token.span);
                    self.error(span, "expected `,` or `)` in a parameter list");
                    return None;
                }
            }
        }
    }

    fn undef(&mut self, rest: &[Raw]) {
        match rest.first() {
            Some(name) if name.token.kind == TokenKind::Ident => {
                self.macros.remove(&name.token.text);
            }
            _ => {
                let span = rest.first().map_or(Span::default(), |raw| raw.token.span);
                self.error(span, "expected a macro name after `#undef`");
            }
        }
    }

    /// `#error` and `#warning`: the rest of the line is the message.
    fn message_directive(&mut self, rest: &[Raw], span: Span, is_error: bool) {
        let message = if rest.is_empty() {
            if is_error {
                "#error".to_owned()
            } else {
                "#warning".to_owned()
            }
        } else {
            let mut text = String::new();
            for raw in rest {
                if !text.is_empty() {
                    text.push(' ');
                }
                text.push_str(self.spell(raw.token.text));
            }
            text
        };
        if is_error {
            self.error(span, message);
        } else {
            self.warning(span, message);
        }
    }

    fn pragma(&mut self, rest: &[Raw], span: Span) {
        match rest.first() {
            Some(once) if self.is_spelled(&once.token, "once") => {}
            Some(known) => {
                let name = self.spell(known.token.text).to_owned();
                self.warning(span, format!("unknown `#pragma {name}` ignored"));
            }
            None => self.warning(span, "empty `#pragma` ignored"),
        }
    }

    /// What this token expands to, if it is a macro invocation that may expand here.
    fn expansion_of(&mut self, raw: &Raw) -> Option<Vec<Raw>> {
        if raw.token.kind != TokenKind::Ident || raw.hide.contains(&raw.token.text) {
            return None;
        }
        let definition = self.macros.get(&raw.token.text)?.clone();
        if self.depth >= self.budgets.max_macro_expansion {
            if self.reported.insert("depth") {
                let span = raw.token.span;
                let limit = self.budgets.max_macro_expansion;
                self.error(
                    span,
                    format!("macro expansion is nested more than {limit} deep"),
                );
            }
            return None;
        }

        let Some(params) = &definition.params else {
            return Some(self.object_like(&definition, raw));
        };

        // A function-like macro expands only when its `(` follows, whitespace allowed.
        if !self
            .peek()
            .is_some_and(|next| self.is_punct(&next.token, "("))
        {
            return None;
        }
        self.next(); // the `(`
        let mut arguments = self.arguments(raw.token.span)?;
        if params.names.is_empty()
            && !params.variadic
            && arguments.len() == 1
            && arguments[0].is_empty()
        {
            arguments.clear(); // `f()` on a macro with no parameters has no arguments
        }
        if !self.check_count(params, arguments.len(), raw.token.span) {
            return None;
        }
        Some(self.substitute(&definition, params, &arguments, raw))
    }

    fn object_like(&self, definition: &Macro, raw: &Raw) -> Vec<Raw> {
        let mut hide = raw.hide.clone();
        hide.push(raw.token.text);
        definition
            .body
            .iter()
            .map(|token| Raw {
                token: *token,
                hide: hide.clone(),
                spliced: true,
            })
            .collect()
    }

    /// The tokens between the parentheses of an invocation, split at top-level commas.
    fn arguments(&mut self, name: Span) -> Option<Vec<Vec<Raw>>> {
        let mut arguments = vec![Vec::new()];
        let mut depth = 0usize;
        loop {
            let Some(raw) = self.next() else {
                self.error(name, "unterminated macro invocation");
                return None;
            };
            if raw.token.kind == TokenKind::Punct {
                if self.is_punct(&raw.token, "(") {
                    depth += 1;
                } else if self.is_punct(&raw.token, ")") {
                    if depth == 0 {
                        return Some(arguments);
                    }
                    depth -= 1;
                } else if self.is_punct(&raw.token, ",") && depth == 0 {
                    arguments.push(Vec::new());
                    continue;
                }
            }
            arguments
                .last_mut()
                .expect("at least one argument")
                .push(raw);
        }
    }

    fn check_count(&mut self, params: &Params, given: usize, span: Span) -> bool {
        let fixed = params.names.len();
        let (least, most) = if params.variadic {
            (fixed, None)
        } else {
            (fixed, Some(fixed))
        };
        let wrong = given < least || most.is_some_and(|most| given > most);
        if !wrong {
            return true;
        }
        let expected = match most {
            Some(most) if most == least => format!("{least}"),
            Some(most) => format!("{least} to {most}"),
            None => format!("at least {least}"),
        };
        self.error(
            span,
            format!("expected {expected} arguments, found {given}"),
        );
        false
    }

    /// The body with the parameters replaced: `#` stringifies, `##` pastes, everything else
    /// is prescanned unless it is being pasted.
    fn substitute(
        &mut self,
        definition: &Macro,
        params: &Params,
        arguments: &[Vec<Raw>],
        invocation: &Raw,
    ) -> Vec<Raw> {
        let mut hide = invocation.hide.clone();
        hide.push(invocation.token.text);

        let mut out: Vec<Raw> = Vec::new();
        let mut at = 0;
        while at < definition.body.len() {
            let token = definition.body[at];

            // `# parameter`: the argument as written, quoted.
            if self.is_punct(&token, "#") {
                if let Some(next) = definition.body.get(at + 1) {
                    if let Some(index) = self.parameter(params, next.text) {
                        let written = self.argument(params, arguments, index, token.span);
                        let literal = self.stringify(&written);
                        let text = self.interner.intern(&literal);
                        out.push(Raw {
                            token: PpToken {
                                kind: TokenKind::Str,
                                text,
                                span: token.span,
                            },
                            hide: hide.clone(),
                            spliced: true,
                        });
                        at += 2;
                        continue;
                    }
                }
                let span = token.span;
                self.error(span, "`#` must be followed by a macro parameter");
                at += 1;
                continue;
            }

            // `... ## ...` with nothing before it: there is no token to paste onto.
            if self.is_punct(&token, "##") {
                let span = token.span;
                self.error(span, "`##` must have a token before it");
                at += 1;
                continue;
            }

            // `left ## right`: the token on each side, pasted into one.
            if let Some(next) = definition.body.get(at + 1) {
                if self.is_punct(next, "##") {
                    let mut left = self.paste_side(token, params, arguments, &hide);
                    let right = match definition.body.get(at + 2) {
                        Some(right) => self.paste_side(*right, params, arguments, &hide),
                        None => {
                            let span = next.span;
                            self.error(span, "`##` must have a token after it");
                            out.extend(left);
                            at += 2;
                            continue;
                        }
                    };
                    match (left.pop(), right.first().cloned()) {
                        (Some(last), Some(first)) => {
                            out.extend(left);
                            out.push(self.paste(&last, &first));
                            out.extend(right.into_iter().skip(1));
                        }
                        // an empty argument on the right leaves the left token alone
                        (Some(last), None) => {
                            out.extend(left);
                            out.push(last);
                        }
                        // a placemarker on the left leaves the right tokens alone
                        (None, _) => out.extend(right),
                    }
                    at += 3;
                    continue;
                }
            }

            if let Some(index) = self.parameter(params, token.text) {
                let written = self.argument(params, arguments, index, token.span);
                // Prescanning the argument happens as if it stood on its own, so the macro
                // being expanded is *not* hidden from it: `id(id(1))` is `1`.
                let expansion = self.expand_tokens(&written);
                out.extend(expansion.into_iter().map(|raw| Raw {
                    hide: hide.clone(),
                    spliced: true,
                    ..raw
                }));
                at += 1;
                continue;
            }

            out.push(Raw {
                token,
                hide: hide.clone(),
                spliced: true,
            });
            at += 1;
        }
        out
    }

    /// The tokens a `##` operand stands for: an argument as written, or the token itself.
    fn paste_side(
        &mut self,
        token: PpToken,
        params: &Params,
        arguments: &[Vec<Raw>],
        hide: &[Symbol],
    ) -> Vec<Raw> {
        let side = match self.parameter(params, token.text) {
            Some(index) => self.argument(params, arguments, index, token.span),
            None => vec![Raw {
                token,
                hide: Vec::new(),
                spliced: true,
            }],
        };
        side.into_iter()
            .map(|raw| Raw {
                hide: hide.to_vec(),
                spliced: true,
                ..raw
            })
            .collect()
    }

    /// Join two tokens into one by re-lexing their spellings.
    fn paste(&mut self, left: &Raw, right: &Raw) -> Raw {
        let mut text = self.spell(left.token.text).to_owned();
        text.push_str(self.spell(right.token.text));
        let spanned = Span::new(
            left.token.span.start,
            right.token.span.end().saturating_sub(left.token.span.start),
        );
        let prepared = Prepared::new(&text);
        let lexed = lex::lex(&prepared, self.source.name(), self.interner);
        match lexed.tokens.as_slice() {
            [token, eof] if !lexed.has_errors() && eof.kind == TokenKind::Eof => Raw {
                token: PpToken {
                    kind: token.kind,
                    text: token.text,
                    span: spanned,
                },
                hide: left.hide.clone(),
                spliced: true,
            },
            _ => {
                let spelling = text.clone();
                self.error(
                    spanned,
                    format!("pasting `{spelling}` does not give one token"),
                );
                Raw {
                    token: left.token,
                    hide: left.hide.clone(),
                    spliced: true,
                }
            }
        }
    }

    /// `#x`: the argument as written, spaces between tokens where the source had them, with
    /// quotes and backslashes escaped (C99 6.10.3.2).
    fn stringify(&self, argument: &[Raw]) -> String {
        let mut text = String::from("\"");
        let mut previous: Option<Span> = None;
        for raw in argument {
            if let Some(before) = previous {
                if before.end() != raw.token.span.start {
                    text.push(' ');
                }
            }
            for character in self.spell(raw.token.text).chars() {
                if character == '"' || character == '\\' {
                    text.push('\\');
                }
                text.push(character);
            }
            previous = Some(raw.token.span);
        }
        text.push('"');
        text
    }

    /// The tokens of one argument; `__VA_ARGS__` is the rest of them, commas included.
    fn argument(
        &mut self,
        params: &Params,
        arguments: &[Vec<Raw>],
        index: usize,
        span: Span,
    ) -> Vec<Raw> {
        if index == params.names.len() {
            let comma = PpToken {
                kind: TokenKind::Punct,
                text: self.interner.intern(","),
                span,
            };
            let mut joined = Vec::new();
            for (position, part) in arguments.iter().enumerate().skip(params.names.len()) {
                if position > params.names.len() {
                    joined.push(Raw {
                        token: comma,
                        hide: Vec::new(),
                        spliced: true,
                    });
                }
                joined.extend(part.iter().cloned());
            }
            return joined;
        }
        arguments.get(index).cloned().unwrap_or_default()
    }

    fn parameter(&self, params: &Params, name: Symbol) -> Option<usize> {
        if let Some(index) = params.names.iter().position(|&parameter| parameter == name) {
            return Some(index);
        }
        if params.variadic && self.spell(name) == "__VA_ARGS__" {
            return Some(params.names.len());
        }
        None
    }

    /// Expand a token sequence on its own, as an argument is prescanned. Each token keeps the
    /// names already hidden from it; nothing new is hidden here.
    fn expand_tokens(&mut self, tokens: &[Raw]) -> Vec<Raw> {
        let saved = std::mem::take(&mut self.pending);
        for token in tokens {
            self.pending.push_back(token.clone());
        }
        let mut out = Vec::new();
        self.depth += 1;
        while let Some(raw) = self.next() {
            match self.expansion_of(&raw) {
                Some(expansion) => self.splice_front(expansion),
                None => out.push(raw),
            }
        }
        self.depth -= 1;
        self.pending = saved;
        out
    }

    fn splice_front(&mut self, expansion: Vec<Raw>) {
        if self.pending.len() + expansion.len() > self.budgets.max_tokens {
            let span = expansion
                .first()
                .map_or(Span::default(), |raw| raw.token.span);
            self.too_many_tokens(span);
            return;
        }
        for raw in expansion.into_iter().rev() {
            self.pending.push_front(raw);
        }
    }

    /// An expansion that outgrows the budget stops everything that is left.
    fn too_many_tokens(&mut self, span: Span) {
        if self.reported.insert("tokens") {
            let limit = self.budgets.max_tokens;
            self.error(
                span,
                format!("preprocessing produced more than {limit} tokens"),
            );
        }
        self.pending.clear();
    }

    fn peek(&self) -> Option<&Raw> {
        self.pending.front()
    }

    fn next(&mut self) -> Option<Raw> {
        self.pending.pop_front()
    }

    fn line(&self, span: Span) -> u32 {
        self.source.position(span.start).line
    }

    fn is_punct(&self, token: &PpToken, spelling: &str) -> bool {
        token.kind == TokenKind::Punct && self.spell(token.text) == spelling
    }

    fn is_spelled(&self, token: &PpToken, spelling: &str) -> bool {
        self.spell(token.text) == spelling
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

    // ---- object-like macros -----------------------------------------------------

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

    // ---- function-like macros ---------------------------------------------------

    #[test]
    fn a_function_like_macro_substitutes_its_parameters() {
        assert_eq!(
            spelled("#define SQR(x) ((x)*(x))\nSQR(3)"),
            ["(", "(", "3", ")", "*", "(", "3", ")", ")"]
        );
    }

    #[test]
    fn arguments_are_expanded_before_they_are_substituted() {
        assert_eq!(
            spelled("#define A 2\n#define SQR(x) ((x)*(x))\nSQR(A)"),
            ["(", "(", "2", ")", "*", "(", "2", ")", ")"]
        );
    }

    #[test]
    fn a_macro_name_without_parentheses_is_not_an_invocation() {
        assert_eq!(spelled("#define f(x) x\nf"), ["f"]);
        assert_eq!(spelled("#define f(x) x\nf + 1"), ["f", "+", "1"]);
        // and with a space before the parenthesis it is still an invocation
        assert_eq!(spelled("#define f(x) x\nf (1)"), ["1"]);
    }

    #[test]
    fn arguments_split_at_top_level_commas_only() {
        assert_eq!(
            spelled("#define P(a,b) a|b\nP(x,(y,z))"),
            ["x", "|", "(", "y", ",", "z", ")"]
        );
    }

    #[test]
    fn an_empty_argument_is_an_argument() {
        assert_eq!(spelled("#define E(x) [x]\nE()"), ["[", "]"]);
    }

    #[test]
    fn an_argument_count_mismatch_is_an_error() {
        let errors = diagnostics("#define f(a,b) a\nf(1)\n");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].message, "expected 2 arguments, found 1");
        let errors = diagnostics("#define f(a) a\nf(1,2)\n");
        assert_eq!(errors[0].message, "expected 1 arguments, found 2");
    }

    #[test]
    fn no_parameters_means_no_arguments() {
        assert_eq!(spelled("#define N() 42\nN()"), ["42"]);
    }

    #[test]
    fn a_macro_does_not_expand_inside_itself_when_function_like() {
        assert_eq!(spelled("#define f(x) f(x)\nf(2)"), ["f", "(", "2", ")"]);
        assert_eq!(
            spelled("#define f(x) g(x)\n#define g(x) f(x)\nf(1)"),
            ["f", "(", "1", ")"],
            "the two do not expand each other forever"
        );
    }

    #[test]
    fn the_result_is_rescanned_with_what_follows() {
        // NAME expands to `g`, and the `(` that follows comes from the source.
        let source = "#define NAME g\n#define g(y) y\n#define f(x) NAME(x)\nf(3)";
        assert_eq!(spelled(source), ["3"]);
    }

    // ---- `#` and `##` -----------------------------------------------------------

    #[test]
    fn a_hash_stringifies_the_argument_as_written() {
        assert_eq!(spelled("#define S(x) #x\nS(a+b)"), ["\"a+b\""]);
        assert_eq!(spelled("#define S(x) #x\nS(a + b)"), ["\"a + b\""]);
        assert_eq!(spelled("#define S(x) #x\nS(\"q\")"), ["\"\\\"q\\\"\""]);
        assert_eq!(
            spelled("#define A 1\n#define S(x) #x\nS(A)"),
            ["\"A\""],
            "the argument is not expanded first"
        );
    }

    #[test]
    fn a_double_hash_pastes_two_tokens() {
        assert_eq!(spelled("#define CAT(a,b) a##b\nCAT(x,1)"), ["x1"]);
        assert_eq!(spelled("#define CAT(a,b) a##b\nCAT(1,2)"), ["12"]);
        assert_eq!(spelled("#define CAT(a,b) a##b\nCAT(+,+)"), ["++"]);
    }

    #[test]
    fn a_paste_that_is_not_a_token_is_diagnosed() {
        let errors = diagnostics("#define CAT(a,b) a##b\nCAT(x,+)");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].message, "pasting `x+` does not give one token");
    }

    // ---- variadic macros --------------------------------------------------------

    #[test]
    fn variadic_arguments_arrive_as_va_args() {
        let source = "#define LOG(fmt, ...) printf(fmt, __VA_ARGS__)\nLOG(\"x\", 1, 2)";
        assert_eq!(
            spelled(source),
            ["printf", "(", "\"x\"", ",", "1", ",", "2", ")"]
        );
    }

    #[test]
    fn vi_variadic_with_nothing_left_expands_to_nothing() {
        assert_eq!(
            spelled("#define V(...) f(__VA_ARGS__)\nV()"),
            ["f", "(", ")"]
        );
    }

    #[test]
    fn a_variadic_macro_can_have_no_fixed_parameters() {
        assert_eq!(
            spelled("#define V(...) f(__VA_ARGS__)\nV(1,2)"),
            ["f", "(", "1", ",", "2", ")"]
        );
    }

    #[test]
    fn definition_mistakes_are_caught() {
        let errors = diagnostics("#define f(a,a) a\n");
        assert_eq!(errors[0].message, "duplicate parameter `a`");

        let errors = diagnostics("#define f(...,a) a\n");
        assert_eq!(errors[0].message, "`...` must be the last parameter");

        let errors = diagnostics("#define f(a) __VA_ARGS__\n");
        assert_eq!(
            errors[0].message,
            "`__VA_ARGS__` used in a macro without `...`"
        );

        let errors = diagnostics("#define f(a b) a\n");
        assert_eq!(errors[0].message, "expected `,` or `)` in a parameter list");
    }

    // ---- directives -------------------------------------------------------------

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
        assert_eq!(spelled("  #  define N 1\nN"), ["1"]);
    }

    // ---- limits and spans -------------------------------------------------------

    #[test]
    fn expansion_depth_is_bounded() {
        // Depth comes from nested arguments: each one is prescanned inside the last.
        let source = format!("#define id(x) x\n{}1{}\n", "id(".repeat(20), ")".repeat(20));
        let budgets = Budgets {
            max_macro_expansion: 8,
            ..Budgets::default()
        };
        let (_, diagnostics) = run_with(&source, &budgets);
        assert_eq!(
            diagnostics.len(),
            1,
            "said once, however many levels exceed it"
        );
        assert_eq!(
            diagnostics[0].message,
            "macro expansion is nested more than 8 deep"
        );
    }

    #[test]
    fn a_macro_that_doubles_is_stopped_by_the_token_budget() {
        // No nesting at all, so the depth limit cannot help: this one grows sideways.
        let mut source = String::from("#define A0 xx\n");
        for step in 1..25 {
            source.push_str(&format!("#define A{step} A{} A{}\n", step - 1, step - 1));
        }
        source.push_str("A24\n");
        let budgets = Budgets {
            max_tokens: 1_000,
            ..Budgets::default()
        };
        let (_, diagnostics) = run_with(&source, &budgets);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].message,
            "preprocessing produced more than 1000 tokens"
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
