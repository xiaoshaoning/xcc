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

mod eval;

use std::collections::{HashMap, HashSet, VecDeque};

use crate::diag::{Diagnostic, Span};
use crate::intern::{Interner, Symbol};
use crate::lex::{self, Prepared, TokenKind};
use crate::options::{Budgets, Options, Stamp};
use crate::source::SourceFile;

/// One token after preprocessing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PpToken {
    /// What it is.
    pub kind: TokenKind,
    /// How it is spelled.
    pub text: Symbol,
    /// Its bytes in the file it came from.
    pub span: Span,
    /// Which file: an index into [`Preprocessed::files`].
    pub file: u16,
}

/// The name the command line's own text is reported under.
const COMMAND_LINE: &str = "<command line>";

/// Where included files come from.
///
/// The compiler reads nothing by itself: a host that wants `#include` gives it something that
/// can answer, and a host that does not (an embedder compiling an in-memory script) leaves it
/// out. Paths are the candidates the preprocessor worked out; returning `None` means "not here".
pub trait Includes: std::fmt::Debug {
    /// The text of the file at `path`, or `None` if this source does not have it.
    fn read(&self, path: &str) -> Option<String>;
}

/// A source with no files of its own: what a compilation gets when the host gives nothing.
impl Includes for () {
    fn read(&self, _: &str) -> Option<String> {
        None
    }
}

/// The tokens of a source, and whatever was wrong with it.
#[derive(Debug, Default, Clone)]
pub struct Preprocessed {
    /// The tokens, ending with [`TokenKind::Eof`].
    pub tokens: Vec<PpToken>,
    /// The files the tokens came from, in the order they were first read: index 0 is the
    /// source handed to [`preprocess`], the rest are what it included. Their text travels with
    /// them, so a diagnostic inside an included file can be rendered with a line and a caret.
    pub files: Vec<SourceFile>,
    /// Errors and warnings, in the order they were found.
    pub diagnostics: Vec<Diagnostic>,
    /// The names the tokens are spelled with: ids are only meaningful with the table that
    /// minted them, so it travels with them rather than being left behind.
    names: Interner,
}

impl Preprocessed {
    /// The name table the tokens' ids are meaningful in.
    pub fn names(&self) -> &Interner {
        &self.names
    }
}

impl Preprocessed {
    /// Whether anything went wrong badly enough to stop.
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(Diagnostic::is_error)
    }

    /// The name a symbol stands for.
    ///
    /// `symbol` must have been minted while producing *this* value: ids are per compilation,
    /// and one from another says nothing, so mixing them is a compiler bug, not an input error.
    pub fn spell(&self, symbol: Symbol) -> &str {
        self.names.resolve(symbol)
    }

    /// How `token` is spelled. The token must come from this value.
    pub fn text(&self, token: &PpToken) -> &str {
        debug_assert!(
            token.file as usize <= self.files.len(),
            "a token from another compilation"
        );
        self.names.resolve(token.text)
    }

    /// The spellings of the tokens, in order, leaving out the end of the file.
    pub fn spellings(&self) -> Vec<String> {
        self.tokens
            .iter()
            .filter(|token| token.kind != TokenKind::Eof)
            .map(|token| self.names.resolve(token.text).to_owned())
            .collect()
    }
}

/// Preprocess `source`, reading includes from `includes` and interning what it keeps.
pub fn preprocess(source: &SourceFile, options: &Options, includes: &dyn Includes) -> Preprocessed {
    let mut interner = Interner::new();
    let mut preprocessor = Preprocessor {
        files: vec![FileEntry::new(source)],
        includes,
        budgets: &options.budgets,
        stamp: &options.stamp,
        interner: &mut interner,
        pending: VecDeque::new(),
        out: Vec::new(),
        macros: HashMap::new(),
        diagnostics: Vec::new(),
        reported: HashSet::new(),
        once: HashSet::new(),
        current: 0,
        depth: 0,
        conditionals: Vec::new(),
        in_command_line: false,
    };
    preprocessor.read_root();
    for definition in &options.defines {
        preprocessor.install_define(definition);
    }
    for name in &options.undefines {
        preprocessor.install_undefine(name);
    }
    preprocessor.run();
    // Take the pieces out one at a time: the preprocessor still borrows the interner.
    let files: Vec<SourceFile> = preprocessor
        .files
        .iter()
        .map(|entry| entry.source.clone())
        .collect();
    let diagnostics = std::mem::take(&mut preprocessor.diagnostics);
    let tokens = std::mem::take(&mut preprocessor.out);
    drop(preprocessor);
    Preprocessed {
        tokens,
        files,
        diagnostics,
        names: interner,
    }
}

/// One file being read: where its lines are, and what `#line` has made of them.
#[derive(Debug)]
struct FileEntry {
    /// The path it was found at; index into [`Preprocessed::files`].
    physical: String,
    /// The name `__FILE__` reports: the include name as written, or what `#line` said.
    logical: String,
    /// The text, for positions.
    source: SourceFile,
    /// What `#line` added to the physical line numbers.
    line_offset: i64,
    /// The last line a token was seen on, so a `#` can be told from a `#` mid-expression.
    last_line: u32,
}

impl FileEntry {
    fn new(source: &SourceFile) -> Self {
        Self {
            physical: source.name().to_owned(),
            logical: source.name().to_owned(),
            source: source.clone(),
            line_offset: 0,
            last_line: 0,
        }
    }

    /// The line number `__LINE__` reports for an offset in this file.
    fn logical_line(&self, offset: u32) -> u32 {
        let physical = i64::from(self.source.position(offset).line);
        (physical + self.line_offset).max(1) as u32
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

/// Which form opened a conditional.
#[derive(Debug, Clone, Copy)]
enum Branch {
    /// `#if expression`.
    If,
    /// `#ifdef name`.
    Ifdef,
    /// `#ifndef name`.
    Ifndef,
}

/// One `#if` and the groups below it, while they are being read.
#[derive(Debug, Clone, Copy)]
struct Conditional {
    /// Whether this group's tokens are kept.
    active: bool,
    /// Whether some group of this chain has been taken, so no other may be.
    taken: bool,
    /// Whether the region around this chain was active: a skip stays a skip.
    parent_active: bool,
    /// Whether `#else` has been seen, after which `#elif` is an error.
    seen_else: bool,
    /// Where the `#if` was, for the diagnostic about it never being closed.
    span: Span,
}

/// A macro definition: its parameters (none for an object-like macro) and its body.
#[derive(Debug, Clone)]
struct Macro {
    params: Option<Params>,
    body: Vec<PpToken>,
}

struct Preprocessor<'a> {
    /// The files read so far: index 0 is the source, the rest are includes.
    files: Vec<FileEntry>,
    includes: &'a dyn Includes,
    budgets: &'a Budgets,
    stamp: &'a Stamp,
    interner: &'a mut Interner,
    /// The tokens still to look at, sources and expansions alike.
    pending: VecDeque<Raw>,
    out: Vec<PpToken>,
    macros: HashMap<Symbol, Macro>,
    diagnostics: Vec<Diagnostic>,
    /// Directives whose "not implemented yet" has already been said, once each.
    reported: HashSet<&'static str>,
    /// Files that said `#pragma once`.
    once: HashSet<String>,
    /// The file the token being read came from.
    current: u16,
    /// How many includes deep we are, for the budget.
    depth: usize,
    /// The conditionals being read, innermost last.
    conditionals: Vec<Conditional>,
    /// Whether the tokens being read are a `-D` from the command line, whose spans point into
    /// text the user never wrote.
    in_command_line: bool,
}

impl Preprocessor<'_> {
    /// Lex the source the host handed over.
    fn read_root(&mut self) {
        self.read_file(None);
    }

    /// Lex file `physical` (or the root when `None`) and put its tokens in front of the queue.
    fn read_file(&mut self, include: Option<(String, String, String)>) {
        let is_include = include.is_some();
        let (physical, logical, text) = match include {
            None => {
                let root = &self.files[0];
                (
                    root.physical.clone(),
                    root.logical.clone(),
                    root.source.text().to_owned(),
                )
            }
            Some(found) => found,
        };
        let source = SourceFile::new(physical.clone(), text);
        let prepared = Prepared::new(source.text());
        let lexed = lex::lex(&prepared, &physical, self.interner);
        if lexed.has_errors() {
            self.diagnostics.extend(lexed.diagnostics);
            return;
        }
        let file = if is_include {
            let id = self.files.len() as u16;
            self.files.push(FileEntry {
                physical,
                logical,
                source,
                line_offset: 0,
                last_line: 0,
            });
            id
        } else {
            0 // the source the host handed over is always file 0
        };
        let tokens: Vec<Raw> = lexed
            .tokens
            .iter()
            .map(|token| Raw {
                token: PpToken {
                    kind: token.kind,
                    text: token.text,
                    span: token.span,
                    file,
                },
                hide: Vec::new(),
                spliced: false,
            })
            .collect();
        for raw in tokens.into_iter().rev() {
            self.pending.push_front(raw);
        }
    }

    /// `#define NAME VALUE` from the command line, as if it were the first line.
    fn install_define(&mut self, definition: &str) {
        let text = format!("#define {}", definition.replacen('=', " ", 1));
        self.in_command_line = true;
        let prepared = Prepared::new(&text);
        let lexed = lex::lex(&prepared, "<command line>", self.interner);
        let words: Vec<Raw> = lexed
            .tokens
            .iter()
            .filter(|token| token.kind != TokenKind::Eof)
            .map(|token| Raw {
                token: PpToken {
                    kind: token.kind,
                    text: token.text,
                    span: token.span,
                    file: 0,
                },
                hide: Vec::new(),
                spliced: false,
            })
            .collect();
        match words.get(2) {
            Some(name) => {
                let span = name.token.span;
                self.define(&words[2..], span);
            }
            None => {
                let span = Span::default();
                self.error(span, format!("`-D{definition}` does not name a macro"));
            }
        }
        self.in_command_line = false;
    }

    /// `-U NAME`: forget a macro the command line defined.
    fn install_undefine(&mut self, name: &str) {
        let symbol = self.interner.intern(name);
        self.macros.remove(&symbol);
    }

    fn run(&mut self) {
        while let Some(raw) = self.next() {
            // The end of an included file leaves it; the end of the source reaches the parser.
            if raw.token.kind == TokenKind::Eof {
                if raw.token.file == 0 {
                    self.out.push(raw.token);
                } else {
                    self.depth = self.depth.saturating_sub(1);
                }
                continue;
            }
            self.current = raw.token.file;
            if !raw.spliced {
                let entry = &mut self.files[raw.token.file as usize];
                let line = entry.source.position(raw.token.span.start).line;
                let at_line_start = line > entry.last_line;
                entry.last_line = entry.last_line.max(line);
                if at_line_start && self.is_punct(&raw.token, "#") {
                    self.directive(raw);
                    continue;
                }
            }
            if !self.active() {
                continue;
            }
            self.emit(raw);
        }
        if let Some(open) = self.conditionals.first() {
            let span = open.span;
            self.error(span, "`#if` is never closed by `#endif`");
        }
    }

    /// Whether the tokens being read are in a group that is kept.
    fn active(&self) -> bool {
        self.conditionals
            .last()
            .is_none_or(|conditional| conditional.active)
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

    fn directive(&mut self, hash: Raw) {
        let line = self.line(hash.token.span);
        let mut words = Vec::new();
        while let Some(next) = self.peek() {
            if next.token.kind == TokenKind::Eof || self.line(next.token.span) != line {
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

        // Conditionals are read whatever the state: the `#else` that ends a skip is one.
        match name_text.as_str() {
            "if" => return self.open_conditional(rest, span, Branch::If),
            "ifdef" => return self.open_conditional(rest, span, Branch::Ifdef),
            "ifndef" => return self.open_conditional(rest, span, Branch::Ifndef),
            "elif" => return self.elif(rest, span),
            "else" => return self.else_group(span),
            "endif" => {
                if self.conditionals.pop().is_none() {
                    self.error(span, "`#endif` without `#if`");
                }
                return;
            }
            _ => {}
        }
        if !self.active() {
            return; // inside a skipped group: no definitions, no errors, no includes
        }

        match name_text.as_str() {
            "define" => self.define(rest, span),
            "undef" => self.undef(rest),
            "error" => self.message_directive(rest, span, true),
            "warning" => self.message_directive(rest, span, false),
            "pragma" => self.pragma(rest, span),
            "include" => self.include(rest, span),
            "include_next" => self.error(span, "`#include_next` is not supported"),
            "line" => self.line_directive(rest, span),
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

    fn open_conditional(&mut self, rest: &[Raw], span: Span, branch: Branch) {
        let parent_active = self.active();
        let condition = if parent_active {
            match branch {
                Branch::If => self.condition(rest, span),
                Branch::Ifdef => self.defined(rest, span, true),
                Branch::Ifndef => self.defined(rest, span, false),
            }
        } else {
            // Nothing to decide: the whole chain sits inside a group that is already skipped,
            // and its expression may name macros that do not exist there.
            false
        };
        self.conditionals.push(Conditional {
            active: parent_active && condition,
            taken: condition,
            parent_active,
            seen_else: false,
            span,
        });
    }

    fn elif(&mut self, rest: &[Raw], span: Span) {
        let Some(top) = self.conditionals.last() else {
            self.error(span, "`#elif` without `#if`");
            return;
        };
        let (parent_active, taken, seen_else) = (top.parent_active, top.taken, top.seen_else);
        if seen_else {
            self.error(span, "`#elif` after `#else`");
            return;
        }
        let condition = parent_active && !taken && self.condition(rest, span);
        if let Some(top) = self.conditionals.last_mut() {
            top.active = condition;
            top.taken = taken || condition;
        }
    }

    fn else_group(&mut self, span: Span) {
        let Some(top) = self.conditionals.last() else {
            self.error(span, "`#else` without `#if`");
            return;
        };
        let (parent_active, taken, seen_else) = (top.parent_active, top.taken, top.seen_else);
        if seen_else {
            self.error(span, "`#else` after `#else`");
            return;
        }
        if let Some(top) = self.conditionals.last_mut() {
            top.active = parent_active && !taken;
            top.taken = true;
            top.seen_else = true;
        }
    }

    /// `#ifdef X` and `#ifndef X`: is `X` defined?
    fn defined(&mut self, rest: &[Raw], span: Span, want_defined: bool) -> bool {
        let Some(name) = rest.first() else {
            self.error(span, "expected a macro name");
            return false;
        };
        if name.token.kind != TokenKind::Ident {
            let span = name.token.span;
            self.error(span, "expected a macro name");
            return false;
        }
        let present = self.macros.contains_key(&name.token.text);
        present == want_defined
    }

    /// The value of an `#if` expression: `defined` first, then macros, then what is left.
    fn condition(&mut self, rest: &[Raw], span: Span) -> bool {
        if rest.is_empty() {
            self.error(span, "expected an expression");
            return false;
        }
        let Some(replaced) = self.replace_defined(rest) else {
            return false;
        };
        let spelled: Vec<Raw> = replaced
            .iter()
            .map(|token| Raw {
                token: *token,
                hide: Vec::new(),
                spliced: true,
            })
            .collect();
        let expanded = self.expand_tokens(&spelled);
        let mut tokens = Vec::with_capacity(expanded.len());
        for raw in expanded {
            // Any identifier left after expansion is `0`, which is what C99 6.10.1 says.
            if raw.token.kind == TokenKind::Ident {
                let zero = self.interner.intern("0");
                tokens.push(PpToken {
                    kind: TokenKind::Number,
                    text: zero,
                    span: raw.token.span,
                    file: raw.token.file,
                });
            } else {
                tokens.push(raw.token);
            }
        }
        let interner: &Interner = self.interner;
        match eval::evaluate(&tokens, interner) {
            Ok(value) => value.is_true(),
            Err(failure) => {
                self.error(failure.span, failure.message);
                false
            }
        }
    }

    /// Rewrite `defined X` and `defined(X)` as `1` or `0`, before anything is expanded.
    fn replace_defined(&mut self, rest: &[Raw]) -> Option<Vec<PpToken>> {
        let mut out = Vec::new();
        let mut at = 0;
        while at < rest.len() {
            let token = rest[at].token;
            if !(token.kind == TokenKind::Ident && self.is_spelled(&token, "defined")) {
                out.push(token);
                at += 1;
                continue;
            }
            at += 1;
            let parenthesised = rest
                .get(at)
                .is_some_and(|raw| self.is_punct(&raw.token, "("));
            if parenthesised {
                at += 1;
            }
            let Some(name) = rest.get(at) else {
                self.error(token.span, "expected a macro name after `defined`");
                return None;
            };
            if name.token.kind != TokenKind::Ident {
                let span = name.token.span;
                self.error(span, "expected a macro name after `defined`");
                return None;
            }
            at += 1;
            if parenthesised {
                let closes = rest
                    .get(at)
                    .is_some_and(|raw| self.is_punct(&raw.token, ")"));
                if !closes {
                    self.error(token.span, "expected `)` after the name in `defined`");
                    return None;
                }
                at += 1;
            }
            let present = self.macros.contains_key(&name.token.text);
            let text = self.interner.intern(if present { "1" } else { "0" });
            out.push(PpToken {
                kind: TokenKind::Number,
                text,
                span: token.span,
                file: token.file,
            });
        }
        Some(out)
    }

    /// `#include "name"` and `#include <name>`, from the files the host allows.
    fn include(&mut self, rest: &[Raw], span: Span) {
        // A macro may name the file: expand first, then read the name out.
        let expanded = self.expand_tokens(rest);
        let Some((name, angled)) = self.include_name(&expanded, span) else {
            return;
        };
        if self.depth + 1 > self.budgets.max_include_depth {
            let limit = self.budgets.max_include_depth;
            self.error(span, format!("`#include` is nested more than {limit} deep"));
            return;
        }

        // Quoted names look beside the including file first; angled ones only where told.
        let here = self.file().physical.clone();
        let mut candidates = Vec::new();
        if !angled {
            if let Some(directory) = parent_of(&here) {
                candidates.push(format!("{directory}/{name}"));
            }
        }
        candidates.push(name.clone());

        for candidate in candidates {
            if self.once.contains(&candidate) {
                return; // included before, and that file said `#pragma once`
            }
            if let Some(text) = self.includes.read(&candidate) {
                let logical = name.clone();
                self.depth += 1;
                self.read_file(Some((candidate, logical, text)));
                return;
            }
        }
        self.error(span, format!("cannot find `{name}`"));
    }

    /// The file name an `#include` asks for, and whether it was written `<>` or `""`.
    fn include_name(&mut self, expanded: &[Raw], span: Span) -> Option<(String, bool)> {
        match expanded.first() {
            Some(first) if first.token.kind == TokenKind::Str => {
                let spelling = self.spell(first.token.text).to_owned();
                let name = spelling
                    .strip_prefix('"')
                    .and_then(|rest| rest.strip_suffix('"'))
                    .unwrap_or(&spelling)
                    .to_owned();
                Some((name, false))
            }
            Some(first) if self.is_punct(&first.token, "<") => {
                let mut name = String::new();
                for raw in expanded.iter().skip(1) {
                    if self.is_punct(&raw.token, ">") {
                        return Some((name, true));
                    }
                    name.push_str(self.spell(raw.token.text));
                }
                self.error(span, "expected `>` to end the file name");
                None
            }
            _ => {
                self.error(span, "expected \"name\" or <name> after `#include`");
                None
            }
        }
    }

    /// `#line N ["file"]`: renumber what `__LINE__` and `__FILE__` report from here on.
    fn line_directive(&mut self, rest: &[Raw], span: Span) {
        let Some(first) = rest.first() else {
            self.error(span, "expected a line number after `#line`");
            return;
        };
        let spelling = self.spell(first.token.text).to_owned();
        let Some(number) = eval::integer_literal(&spelling) else {
            self.error(first.token.span, "expected a line number after `#line`");
            return;
        };
        if let Some(next) = rest.get(1) {
            if next.token.kind != TokenKind::Str {
                let span = next.token.span;
                self.error(span, "expected a file name in quotes after the line number");
                return;
            }
            let spelling = self.spell(next.token.text).to_owned();
            let name = spelling.trim_matches('"').to_owned();
            self.files[self.current as usize].logical = name;
        }
        let physical = i64::from(self.file().source.position(span.start).line);
        self.files[self.current as usize].line_offset = number.bits as i64 - (physical + 1);
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
            Some(once) if self.is_spelled(&once.token, "once") => {
                let physical = self.file().physical.clone();
                self.once.insert(physical);
            }
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
        // The token budget is spent: every further expansion would be work whose result is thrown
        // away, so the tokens pass through as they are and the one diagnostic stands.
        if self.reported.contains("tokens") {
            return None;
        }
        if let Some(predefined) = self.predefined(raw) {
            return Some(predefined);
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

    /// The macros the language defines itself: `__FILE__`, `__LINE__`, and the standard's.
    fn predefined(&mut self, raw: &Raw) -> Option<Vec<Raw>> {
        let spelling = self.spell(raw.token.text).to_owned();
        let entry = &self.files[raw.token.file as usize];
        let quoted =
            |text: &str| format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""));
        let (kind, text) = match spelling.as_str() {
            "__FILE__" => (TokenKind::Str, quoted(&entry.logical)),
            "__LINE__" => (
                TokenKind::Number,
                entry.logical_line(raw.token.span.start).to_string(),
            ),
            "__DATE__" => (TokenKind::Str, quoted(&self.stamp.date)),
            "__TIME__" => (TokenKind::Str, quoted(&self.stamp.time)),
            "__STDC__" => (TokenKind::Number, "1".to_owned()),
            "__STDC_VERSION__" => (TokenKind::Number, "199901L".to_owned()),
            "__STDC_HOSTED__" => (TokenKind::Number, "0".to_owned()),
            _ => return None,
        };
        let text = self.interner.intern(&text);
        Some(vec![Raw {
            token: PpToken {
                kind,
                text,
                span: raw.token.span,
                file: raw.token.file,
            },
            hide: Vec::new(),
            spliced: true,
        }])
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
                                file: token.file,
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
        let lexed = lex::lex(&prepared, &self.file().physical.clone(), self.interner);
        match lexed.tokens.as_slice() {
            [token, eof] if !lexed.has_errors() && eof.kind == TokenKind::Eof => Raw {
                token: PpToken {
                    kind: token.kind,
                    text: token.text,
                    span: spanned,
                    file: left.token.file,
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
                file: self.current,
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
            // An argument is prescanned in full, and a prescan can be where a hostile source does
            // its work: `#define F(x) F(x) F(x)`-style fan-out expands *inside* an argument
            // before the queue that is checked ever grows. So the budget is spent here too.
            if out.len() >= self.budgets.max_tokens {
                let span = raw.token.span;
                self.too_many_tokens(span);
                break;
            }
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

    fn file(&self) -> &FileEntry {
        &self.files[self.current as usize]
    }

    fn line(&self, span: Span) -> u32 {
        self.file().source.position(span.start).line
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
        self.diagnostics
            .push(self.diagnostic(Diagnostic::error(message), span));
    }

    fn warning(&mut self, span: Span, message: impl Into<String>) {
        self.diagnostics
            .push(self.diagnostic(Diagnostic::warning(message), span));
    }

    /// Attach the place a diagnostic is about: a span in the file being read, or - for a `-D`
    /// - the command line, where the span would point into text nobody wrote.
    fn diagnostic(&self, diagnostic: Diagnostic, span: Span) -> Diagnostic {
        if self.in_command_line {
            return diagnostic.with_file(COMMAND_LINE);
        }
        diagnostic.with_file(&self.file().physical).with_span(span)
    }
}
/// The directory part of a path, whichever separator it uses.
fn parent_of(path: &str) -> Option<&str> {
    let cut = path.rfind(['/', '\\'])?;
    Some(&path[..cut])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diag::Severity;

    fn run(source: &str) -> (Vec<String>, Vec<Diagnostic>) {
        run_with(source, &Budgets::default())
    }

    fn run_with(source: &str, budgets: &Budgets) -> (Vec<String>, Vec<Diagnostic>) {
        let options = Options {
            budgets: budgets.clone(),
            ..Options::default()
        };
        run_options(source, &options, &())
    }

    fn run_options(
        source: &str,
        options: &Options,
        includes: &dyn Includes,
    ) -> (Vec<String>, Vec<Diagnostic>) {
        let file = SourceFile::new("t.c", source);
        let preprocessed = preprocess(&file, options, includes);
        (preprocessed.spellings(), preprocessed.diagnostics)
    }

    /// The spellings, asserting nothing went wrong.
    fn spelled(source: &str) -> Vec<String> {
        let (spells, diagnostics) = run(source);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
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
    fn directives_that_are_refused_are_errors() {
        assert_eq!(
            diagnostics(
                "#include_next <stdio.h>
"
            )[0]
            .message,
            "`#include_next` is not supported"
        );
    }

    // ---- conditionals -----------------------------------------------------------

    #[test]
    fn if_takes_or_drops_a_group() {
        assert_eq!(spelled("#if 1\nkeep\n#endif"), ["keep"]);
        assert_eq!(spelled("#if 0\ndrop\n#endif"), Vec::<String>::new());
        assert_eq!(spelled("#if 2 + 3 * 4 == 14\nyes\n#endif"), ["yes"]);
    }

    #[test]
    fn else_and_elif_choose_one_group() {
        assert_eq!(spelled("#if 0\na\n#elif 1\nb\n#else\nc\n#endif"), ["b"]);
        assert_eq!(spelled("#if 0\na\n#elif 0\nb\n#else\nc\n#endif"), ["c"]);
        assert_eq!(
            spelled("#if 1\na\n#elif 1\nb\n#endif"),
            ["a"],
            "the first taken group wins"
        );
    }

    #[test]
    fn ifdef_and_ifndef_ask_the_macro_table() {
        assert_eq!(spelled("#ifdef X\nyes\n#endif"), Vec::<String>::new());
        assert_eq!(spelled("#define X\n#ifdef X\nyes\n#endif"), ["yes"]);
        assert_eq!(spelled("#ifndef X\nyes\n#endif"), ["yes"]);
        assert_eq!(
            spelled("#define X\n#ifndef X\nyes\n#endif"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn defined_is_answered_before_expansion() {
        assert_eq!(spelled("#define X 1\n#if defined(X)\nyes\n#endif"), ["yes"]);
        assert_eq!(spelled("#define X 1\n#if defined X\nyes\n#endif"), ["yes"]);
        assert_eq!(spelled("#if defined(X)\nyes\n#endif"), Vec::<String>::new());
        assert_eq!(
            spelled("#define X 3\n#if defined(X) && X > 1\nyes\n#endif"),
            ["yes"]
        );
        // `defined` is not a macro call, and its operand is not expanded
        assert_eq!(
            spelled("#define X Y\n#if defined(X) && !defined(Y)\nyes\n#endif"),
            ["yes"]
        );
    }

    #[test]
    fn an_identifier_that_is_not_a_macro_is_zero() {
        assert_eq!(spelled("#if FOO\nyes\n#endif"), Vec::<String>::new());
        assert_eq!(spelled("#if !FOO\nyes\n#endif"), ["yes"]);
    }

    #[test]
    fn macros_are_expanded_before_the_expression_is_evaluated() {
        assert_eq!(spelled("#define N 3\n#if N == 3\nyes\n#endif"), ["yes"]);
        assert_eq!(spelled("#define N 3\n#if N * 2 == 6\nyes\n#endif"), ["yes"]);
    }

    #[test]
    fn the_evaluator_has_c_s_precedence_and_operators() {
        for source in [
            "#if (1 + 2) * 3 == 9\nyes\n#endif",
            "#if 1 << 4 == 16\nyes\n#endif",
            "#if (7 & 3) == 3\nyes\n#endif",
            "#if (1 ? 2 : 3) == 2\nyes\n#endif",
            "#if 'a' == 97\nyes\n#endif",
            "#if -1 < 0\nyes\n#endif",
            "#if 0x10 == 16 && 010 == 8\nyes\n#endif",
            "#if ~0 == -1\nyes\n#endif",
        ] {
            assert_eq!(spelled(source), ["yes"], "{source}");
        }
    }

    #[test]
    fn what_is_not_evaluated_is_not_diagnosed() {
        assert_eq!(
            spelled("#if 0 && 1/0\nyes\n#endif"),
            Vec::<String>::new(),
            "the right side of && is not evaluated"
        );
        assert_eq!(spelled("#if 1 || 1/0\nyes\n#endif"), ["yes"]);
    }

    #[test]
    fn only_the_chosen_arm_of_a_conditional_is_evaluated() {
        // C99 6.10.1: the arm that is not taken is parsed, not evaluated.
        assert_eq!(
            spelled(
                "#if 1 ? 1 : 1/0
yes
#endif"
            ),
            ["yes"]
        );
        assert_eq!(
            spelled(
                "#if 0 ? 1/0 : 2
yes
#endif"
            ),
            ["yes"]
        );
        assert_eq!(
            diagnostics(
                "#if 1 ? 1/0 : 2
#endif"
            )[0]
            .message,
            "division by zero",
            "the arm that is taken is still checked"
        );
    }

    #[test]
    fn a_command_line_define_is_reported_as_the_command_line() {
        let options = Options {
            defines: vec!["1BAD=2".to_owned()],
            ..Options::default()
        };
        let (_, diagnostics) = run_options("int x;", &options, &());
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].file.as_deref(), Some("<command line>"));
        assert_eq!(
            diagnostics[0].span, None,
            "no offset into a file nobody wrote"
        );
    }

    #[test]
    fn division_by_zero_and_overflow_are_reported() {
        assert_eq!(
            diagnostics("#if 1/0\n#endif")[0].message,
            "division by zero"
        );
        assert_eq!(
            diagnostics("#if 1%0\n#endif")[0].message,
            "division by zero"
        );
        let overflow = diagnostics("#if 9223372036854775807 + 1\n#endif");
        assert_eq!(overflow[0].message, "integer overflow in `+`");
        let shift = diagnostics("#if 1 << 64\n#endif");
        assert_eq!(shift[0].message, "shift by 64 does not fit in 64 bits");
    }

    #[test]
    fn a_skipped_group_is_not_read_at_all() {
        // no error from the inner `#error`, no definition leaking out
        let source =
            "#if 0\n#error not today\n#define X 1\n#frobnicate\n#endif\n#ifdef X\nleak\n#endif";
        assert!(diagnostics(source).is_empty(), "{:?}", diagnostics(source));
        assert_eq!(spelled(source), Vec::<String>::new());
        // and a `#if` inside a skipped group is not evaluated, so its errors cannot fire
        let source = "#if 0\n#if 1/0\nx\n#endif\n#endif\ny";
        assert!(diagnostics(source).is_empty());
        assert_eq!(spelled(source), ["y"]);
    }

    #[test]
    fn conditionals_nest() {
        let source = "#define A\n#ifdef A\n#if 0\nno\n#else\nyes\n#endif\n#endif";
        assert_eq!(spelled(source), ["yes"]);
    }

    #[test]
    fn a_file_without_a_final_newline_still_ends() {
        // the directive must not eat the end of the file with its line
        assert_eq!(spelled("#if 1\nkeep\n#endif"), ["keep"]);
        assert_eq!(spelled("#define N 3\nN"), ["3"]);
    }

    #[test]
    fn unbalanced_conditionals_are_reported() {
        assert_eq!(diagnostics("#endif\n")[0].message, "`#endif` without `#if`");
        assert_eq!(diagnostics("#else\n")[0].message, "`#else` without `#if`");
        assert_eq!(diagnostics("#elif 1\n")[0].message, "`#elif` without `#if`");
        assert_eq!(
            diagnostics("#if 1\n#else\n#else\n#endif\n")[0].message,
            "`#else` after `#else`"
        );
        assert_eq!(
            diagnostics("#if 1\n#else\n#elif 0\n#endif\n")[0].message,
            "`#elif` after `#else`"
        );
        assert_eq!(
            diagnostics("#if 1\n")[0].message,
            "`#if` is never closed by `#endif`"
        );
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
    fn a_macro_that_fans_out_inside_an_argument_is_stopped_too() {
        // The queue stays short here - the growth is inside the argument that is prescanned, not
        // in what is waiting to be expanded - so only spending the budget during a prescan stops
        // it. Without that, ten levels of this is ten-to-the-hundredth tokens of work.
        let mut source = String::from("#define D0(x) x x x x x x x x x x\n");
        for step in 1..10 {
            source.push_str(&format!(
                "#define D{step}(x) D{}(D{}(x))\n",
                step - 1,
                step - 1
            ));
        }
        source.push_str("D9(1)\n");
        let budgets = Budgets {
            max_tokens: 1_000,
            ..Budgets::default()
        };
        let (_, diagnostics) = run_with(&source, &budgets);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(
            diagnostics[0].message,
            "preprocessing produced more than 1000 tokens"
        );
    }

    #[test]
    fn spans_survive_expansion() {
        let source = "#define N 3\nint x = N;\n";
        let file = SourceFile::new("t.c", source);
        let preprocessed = preprocess(&file, &Options::default(), &());
        let three = preprocessed
            .tokens
            .iter()
            .find(|token| preprocessed.text(token) == "3")
            .expect("the expansion is in the output");
        assert_eq!(
            &source[three.span.start as usize..][..1],
            "3",
            "the span is the body's"
        );
    }
    // ---- includes ---------------------------------------------------------------

    /// Files a test lets the preprocessor read.
    #[derive(Debug, Default)]
    struct Map(std::collections::HashMap<String, String>);

    impl Map {
        fn new(files: &[(&str, &str)]) -> Self {
            Self(
                files
                    .iter()
                    .map(|(name, text)| ((*name).to_owned(), (*text).to_owned()))
                    .collect(),
            )
        }
    }

    impl Includes for Map {
        fn read(&self, path: &str) -> Option<String> {
            self.0.get(path).cloned()
        }
    }

    fn with_includes(source: &str, files: &[(&str, &str)]) -> (Vec<String>, Vec<Diagnostic>) {
        run_options(source, &Options::default(), &Map::new(files))
    }

    #[test]
    fn a_quoted_include_is_read() {
        let (spells, diagnostics) = with_includes("#include \"b.h\"\nhere", &[("b.h", "there\n")]);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(spells, ["there", "here"]);
    }

    #[test]
    fn an_angled_include_is_read() {
        let (spells, diagnostics) = with_includes("#include <b.h>\nhere", &[("b.h", "there\n")]);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(spells, ["there", "here"]);
    }

    #[test]
    fn a_missing_include_says_which_one() {
        let errors = with_includes("#include \"nowhere.h\"\n", &[]).1;
        assert_eq!(errors[0].message, "cannot find `nowhere.h`");
    }

    #[test]
    fn an_include_beside_the_including_file_wins() {
        // `sub/a.h` includes "b.h": the file next to it is preferred to the bare name
        let files = [("sub/b.h", "beside\n"), ("b.h", "elsewhere\n")];
        let source = "#include \"sub/a.h\"\n";
        let (spells, diagnostics) = with_includes(
            source,
            &[("sub/a.h", "#include \"b.h\"\n"), files[0], files[1]],
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(spells, ["beside"]);
    }

    #[test]
    fn includes_nest_to_the_budget() {
        let source = "#include \"a.h\"\n";
        let files = [("a.h", "#include \"b.h\"\n"), ("b.h", "deep\n")];
        let (spells, diagnostics) = with_includes(source, &files);
        assert!(diagnostics.is_empty());
        assert_eq!(spells, ["deep"]);

        let options = Options {
            budgets: Budgets {
                max_include_depth: 1,
                ..Budgets::default()
            },
            ..Options::default()
        };
        let (_, diagnostics) = run_options(source, &options, &Map::new(&files));
        assert_eq!(
            diagnostics[0].message,
            "`#include` is nested more than 1 deep"
        );
    }

    #[test]
    fn pragma_once_reads_a_file_once() {
        let source = "#include \"a.h\"\n#include \"a.h\"\n";
        let files = [("a.h", "#pragma once\nonce\n")];
        let (spells, diagnostics) = with_includes(source, &files);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(spells, ["once"]);
    }

    #[test]
    fn an_include_guard_works_without_pragma_once() {
        let guard = "#ifndef A_H\n#define A_H\nguarded\n#endif\n";
        let files = [("a.h", guard)];
        let (spells, diagnostics) = with_includes("#include \"a.h\"\n#include \"a.h\"\n", &files);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(spells, ["guarded"]);
    }

    #[test]
    fn included_tokens_say_which_file_they_came_from() {
        let file = SourceFile::new("t.c", "#include \"b.h\"\nx\n");
        let preprocessed = preprocess(&file, &Options::default(), &Map::new(&[("b.h", "y\n")]));
        assert_eq!(
            preprocessed
                .files
                .iter()
                .map(SourceFile::name)
                .collect::<Vec<_>>(),
            ["t.c", "b.h"]
        );
        let y = preprocessed
            .tokens
            .iter()
            .find(|token| preprocessed.text(token) == "y")
            .expect("the included token");
        assert_eq!(y.file, 1);
    }

    // ---- what the command line defines ------------------------------------------

    #[test]
    fn options_defines_and_undefines_are_installed_first() {
        let options = Options {
            defines: vec!["N=3".to_owned(), "FLAG".to_owned(), "GONE=1".to_owned()],
            undefines: vec!["GONE".to_owned()],
            ..Options::default()
        };
        let (spells, diagnostics) = run_options(
            "#ifdef FLAG\nint n = N;\n#endif\n#ifdef GONE\ngone\n#endif",
            &options,
            &(),
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(spells, ["int", "n", "=", "3", ";"]);
    }

    // ---- the macros the language defines itself ---------------------------------

    #[test]
    fn file_and_line_say_where_they_are() {
        let (spells, diagnostics) = run_options("__LINE__\n__LINE__", &Options::default(), &());
        assert!(diagnostics.is_empty());
        assert_eq!(spells, ["1", "2"]);

        let source = SourceFile::new("dir/t.c", "__FILE__\n");
        let preprocessed = preprocess(&source, &Options::default(), &());
        assert_eq!(preprocessed.text(&preprocessed.tokens[0]), "\"dir/t.c\"");
    }

    #[test]
    fn date_and_time_come_from_the_options() {
        let options = Options {
            stamp: Stamp::new("Nov 15 2026", "12:34:56"),
            ..Options::default()
        };
        let (spells, diagnostics) = run_options("__DATE__ __TIME__", &options, &());
        assert!(diagnostics.is_empty());
        assert_eq!(spells, ["\"Nov 15 2026\"", "\"12:34:56\""]);

        let (spells, _) = run_options("__DATE__", &Options::default(), &());
        assert_eq!(spells, ["\"Jan  1 1970\""], "no clock is ever read");
    }

    #[test]
    fn the_standard_macros_are_defined() {
        let (spells, _) = run_options(
            "__STDC__ __STDC_VERSION__ __STDC_HOSTED__",
            &Options::default(),
            &(),
        );
        assert_eq!(spells, ["1", "199901L", "0"]);
    }

    // ---- #line ------------------------------------------------------------------

    #[test]
    fn line_directive_renumbers_what_follows() {
        let (spells, diagnostics) = run_options(
            "#line 100\n__LINE__\n__LINE__\n#line 7 \"other.c\"\n__LINE__ __FILE__",
            &Options::default(),
            &(),
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(spells, ["100", "101", "7", "\"other.c\""]);
    }
}
