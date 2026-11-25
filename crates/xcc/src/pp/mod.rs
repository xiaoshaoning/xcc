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
    /// Which macro invocation produced this token: an index into [`Preprocessed::sites`], or `None`
    /// for a token the program wrote itself. An *index* rather than a span, because every expansion
    /// of one macro produces tokens with the same body spans - which is exactly the mistake a
    /// span-keyed table made: sqlite3.c's two errors were both reported at use number one.
    pub expansion: Option<u32>,
    /// Whether a newline **that survived phase 2** comes before this token.
    ///
    /// The line a token sits on is not the question a directive asks - the phase-2 splice removes a
    /// newline from the text, and the tokens either side of it are one logical line however many
    /// physical lines they cover. `#define ADD(a, b) \` and its body are a macro, and reading them
    /// as two lines made `\`-continued macros a syntax error, which real C uses everywhere and this
    /// corpus never once did.
    pub starts_line: bool,
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
    /// The C the source was read as. It travels with the tokens because the parser needs it for the
    /// constructs that are one standard's and not another's, and because `__STDC_VERSION__` is
    /// already a fact about it.
    pub standard: crate::options::CStandard,
    /// The names the tokens are spelled with: ids are only meaningful with the table that
    /// minted them, so it travels with them rather than being left behind.
    names: Interner,
    /// Where `#pragma pack` changed the limit: `(token index, pack)`, in order.
    pack_changes: Vec<(usize, u8)>,
    /// The invocations every macro-produced token came from, in the order they were recorded. A
    /// token holds an index into this (`PpToken::expansion`), because **the span cannot be the key**:
    /// every expansion of one macro produces tokens with the *same* body spans, so a span-keyed table
    /// answers with the first use for all of them - which is what sqlite3.c's measurement hit, two
    /// errors both reported at use number one.
    sites: Vec<Site>,
    /// Where the attributes that change a layout were written: `(token index, attribute)`. The
    /// *position* is the whole of it - `struct S { ... } __attribute__((packed))` is packed because
    /// the attribute sits where the declaration does - so a parser asks about the range it just
    /// parsed instead of meeting an attribute where it does not expect one.
    layout_attributes: Vec<(usize, crate::types::LayoutAttribute)>,
}

impl Preprocessed {
    /// The name table the tokens' ids are meaningful in.
    pub fn names(&self) -> &Interner {
        &self.names
    }

    /// The invocation a token came from, when it came from one: the macro's name and where it was
    /// used. `None` for a token the program wrote itself - a diagnostic about a token in a macro
    /// *body* points at the body, which is where it was written, and this is what lets it add the
    /// one thing the body cannot say: which use it came from.
    pub fn site(&self, token: &PpToken) -> Option<&Site> {
        token.expansion.and_then(|at| self.sites.get(at as usize))
    }

    /// The `#pragma pack` limit in force at token `index`: the limit a record whose body ends
    /// *there* is laid out with. Without a pragma it is the ABI's, which is what `_Alignas(16)`
    /// may ask for.
    pub fn pack_at(&self, index: usize) -> u8 {
        // How many changes have happened at or before this token; the last of them is in force.
        let past = self.pack_changes.partition_point(|(at, _)| *at <= index);
        match past {
            0 => crate::types::Types::DEFAULT_PACK,
            _ => self.pack_changes[past - 1].1,
        }
    }

    /// The layout attributes written between tokens `from` and `to`, both included: everything one
    /// declaration can have said about where its bytes go.
    pub fn layout_attributes(&self, from: usize, to: usize) -> Vec<crate::types::LayoutAttribute> {
        let first = self.layout_attributes.partition_point(|(at, _)| *at < from);
        self.layout_attributes[first..]
            .iter()
            .take_while(|(at, _)| *at <= to)
            .map(|(_, attribute)| *attribute)
            .collect()
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
/// The macros this implementation defines for every translation unit: C99 6.10.8p2's three, and
/// the target's own names.
///
/// They are *installed as macros* rather than special-cased during expansion, because 6.10.8 makes
/// them macros: `#ifdef __STDC__` and `defined(__STDC__)` must agree with `__STDC__` itself, and a
/// version that only substituted the name left the first two answering no. A program that asks its
/// dialect with `#ifdef` is the common case, not the exotic one.
///
/// `_WIN32`, `_WIN64`, `__x86_64__` and `_M_X64` are the target's: the first two are what Windows
/// headers and libraries test, and the last two are what code written for this instruction set
/// tests. Without `_WIN32`, a program that selects its platform - sqlite's own `os_setup.h` is one -
/// decides it is on Unix and includes `<pthread.h>`.
/// The limit a `#pragma pack(N)` can name: the five alignments this target has.
fn pack_limit(number: &str) -> Option<u8> {
    match number.parse::<u8>() {
        Ok(limit) if matches!(limit, 1 | 2 | 4 | 8 | 16) => Some(limit),
        _ => None,
    }
}

/// The attributes that ask for nothing this compiler implements, and may therefore be dropped.
///
/// The criterion is "carries no meaning here", not "is well known": this compiler collects no
/// garbage (`used`), links nothing dynamically (`visibility`), has one calling convention, has no
/// optimiser to guide (`always_inline`, `pure`, `cold`), and implements no `format` diagnostics.
/// `noreturn` is in because a promise about control flow is not a layout - and a function that
/// breaks it has undefined behaviour of its own.
///
/// Anything else is refused by name: an unknown attribute could change the program - `vector_size`
/// a type's size, `section` where its bytes go, `cleanup` what runs at the end of a block, `weak`
/// whether a definition wins - and not noticing is silent. `packed` and `aligned` are named
/// separately only because their diagnostic says what the program would have got wrong.
const HARMLESS_ATTRIBUTES: &[&str] = &[
    // A promise about control flow this compiler does not analyse, and the two attributes that ask
    // for a diagnostic: `returns_twice` is `setjmp`'s and `minwindef.h` writes it on Win32's
    // callback entry points.
    "returns_twice",
    "__returns_twice__",
    "naked",
    "__warning__",
    "error",
    "warning",
    // What a diagnostic would say, in a compiler that does not say it.
    "unused",
    "__unused__",
    "warn_unused_result",
    "nodiscard",
    "deprecated",
    "format",
    "__format__",
    "format_arg",
    "nonnull",
    "returns_nonnull",
    "sentinel",
    "fallthrough",
    "artificial",
    // What an optimiser would like to know, in a compiler that does not ask.
    "always_inline",
    "__always_inline__",
    "noinline",
    "__noinline__",
    "flatten",
    "cold",
    "hot",
    "noclone",
    "optimize",
    "target",
    "leaf",
    "pure",
    "__pure__",
    "const",
    "__const__",
    "noreturn",
    "__noreturn__",
    "nothrow",
    "may_alias",
    // Keeping something from a collection that is not done.
    "used",
    "__used__",
    "retain",
    // Linkage a static link makes moot.
    "visibility",
    "gnu_inline",
    "__gnu_inline__",
    "externally_visible",
];

const PREDEFINED: &[(&str, &str)] = &[
    ("__STDC__", "1"),
    ("__STDC_HOSTED__", "1"),
    // `__STDC_VERSION__` is the dialect's, so it is installed from the dialect and not from here.
    ("_WIN32", "1"),
    ("_WIN64", "1"),
    // The architecture, under every name a Windows SDK asks. MSVC's are `_M_X64` and `_M_AMD64`;
    // `mingw-w64`'s `winnt.h` picks its `CONTEXT` definition by testing `__x86_64` - without the
    // trailing underscores `clang` and GCC use - and a target that answers only the GCC spelling
    // gets no `CONTEXT` at all, and then a hundred declarations that name it fail.
    ("__x86_64__", "1"),
    ("__x86_64", "1"),
    ("_M_X64", "100"),
    ("_M_AMD64", "100"),
    // How this target spells its own typedefs. These are facts about `<stddef.h>` - `ptrdiff_t` is
    // `long long` here and `size_t` is `unsigned long long` - and the *spelling* is what a header
    // asks for, because it has to write a cast and cannot include the header to learn a type name:
    // `sqlite3.c` casts an integer to a pointer through `__PTRDIFF_TYPE__` on every compiler that
    // has it, and falls back to `&((char*)0)[X]` on the others - which is a compiler-specific
    // trick it is right to avoid, and which this compiler could not lower until recently.
    ("__PTRDIFF_TYPE__", "long long"),
    ("__SIZE_TYPE__", "unsigned long long"),
    ("__WCHAR_TYPE__", "unsigned short"),
    // `-fno-inline`, in the spelling GCC's headers standardised: this compiler has no inliner, so a
    // header that writes an inline wrapper for the compiler to bake in makes a *symbol* here
    // instead. mingw-w64's `stralign.h` is the case that proved it - its `ua_CharUpperW` calls
    // `uaw_CharUpperW`, which x64 has never had (the file's own TODO says so), so a program that
    // merely included `<windows.h>` could not link.
    ("__NO_INLINE__", "1"),
    ("__XCC__", "1"),
];

/// Preprocess `source` with `options`, reading `#include` through `includes`.
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
        pack: crate::types::Types::DEFAULT_PACK,
        pack_stack: Vec::new(),
        pack_changes: Vec::new(),
        layout_attributes: Vec::new(),
        sites: Vec::new(),
        in_command_line: false,
    };
    preprocessor.read_root();
    let version = match options.standard {
        crate::options::CStandard::C99 => "199901L",
        crate::options::CStandard::C11 => "201112L",
    };
    preprocessor.install_define(&format!("__STDC_VERSION__={version}"));
    for (name, value) in PREDEFINED {
        preprocessor.install_define(&format!("{name}={value}"));
    }
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
    let pack_changes = std::mem::take(&mut preprocessor.pack_changes);
    let layout_attributes = std::mem::take(&mut preprocessor.layout_attributes);
    let sites = std::mem::take(&mut preprocessor.sites);
    drop(preprocessor);
    Preprocessed {
        tokens,
        files,
        diagnostics,
        standard: options.standard,
        names: interner,
        pack_changes,
        layout_attributes,
        sites,
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

/// Where a macro was invoked from: the file, and the span inside it.
///
/// A type rather than a tuple because the two halves are read apart (`site.file` names the file
/// `__FILE__` reports, `site.span` the line `__LINE__` does), and `(u16, Span)` reads as neither.
#[derive(Debug, Clone, Copy)]
pub struct Site {
    /// Which file the invocation is in.
    pub file: u16,
    /// Where in it.
    pub span: Span,
    /// The macro that was invoked. A diagnostic about a token from an expansion points at the macro
    /// *body* - which is right, because that is where the token was written - and this is what lets
    /// it add the one thing the body cannot say: which use it came from.
    pub name: Symbol,
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
    /// Where the macro that produced this token was *invoked*, when it came from one.
    ///
    /// C99 6.10.8p1 makes `__LINE__` "the line number of the current source line", and during an
    /// expansion the current source line is the *invocation's*: a `assert(e)` in a program reports
    /// the program's line, not line 28 of `assert.h`. The token's own span still points into the
    /// macro body, which is what a diagnostic wants.
    invoked_at: Option<Site>,
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
    /// The `#pragma pack` limit in force, and the stack `push`/`pop` walk. The pragma's *grammar*
    /// is this layer's, for the same reason the conditional stack is: the parser is told a number
    /// and never sees a `push`.
    pack: u8,
    pack_stack: Vec<u8>,
    /// Where the limit changed, as `(token index, pack)`: from that token on, records are laid out
    /// with it. That is what `#pragma pack` is - a fact about a *position* in the token stream, not
    /// about a declaration - so the parser reads it by the position it has reached, and no loop that
    /// reads declarations needs to know a pragma exists.
    pack_changes: Vec<(usize, u8)>,
    /// Where the attributes that change a layout were written, as `(token index, attribute)`.
    layout_attributes: Vec<(usize, crate::types::LayoutAttribute)>,
    /// The invocations macro-produced tokens came from.
    sites: Vec<Site>,
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
                    // The lexer is the only thing that knows: a newline it spliced away does not end
                    // the logical line a directive is read from.
                    starts_line: token.starts_line,
                    expansion: None,
                },
                hide: Vec::new(),
                spliced: false,
                invoked_at: None,
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
                    starts_line: token.starts_line,
                    expansion: None,
                },
                hide: Vec::new(),
                spliced: false,
                invoked_at: None,
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
            // `__attribute__((...))` is a token sequence that means nothing this compiler
            // implements, in *any* position - before a declaration, after its declarator, on a
            // member, on a parameter, after a struct's `}`. Reading it here, where tokens are read,
            // covers all of them once; the parser would need a case in each, and the one it missed
            // would be a parse error on a program that is asking for nothing.
            if self.is_spelled(&raw.token, "__attribute__")
                || self.is_spelled(&raw.token, "__attribute")
            {
                self.attribute(raw);
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

    /// Read one `__attribute__((...))` and act on it.
    ///
    /// The list is comma-separated names, some taking an argument -
    /// `((noreturn, aligned(8), visibility("default")))` - so only the names at the *top* of the
    /// inner list matter, and a name inside an argument is the argument's.
    ///
    /// Each name is looked up in `HARMLESS_ATTRIBUTES` and dropped, or recorded in
    /// `layout_attributes` when it decides where bytes go, or refused by name. **The list is what
    /// may be dropped, not what may not**: an attribute this compiler has never heard of could
    /// change the program - `vector_size` a type's size, `section` where its bytes go, `cleanup`
    /// what runs at the end of a block, `weak` whether a definition wins - and the first version of
    /// this function dropped everything except the two names it knew, which quietly accepted all
    /// four. Silence about a thing not understood is the wrong default.
    ///
    /// What it found is recorded against the *position* in the token stream, because that is how the
    /// parser decides what an attribute is on: a declaration asks about the range it parsed.
    fn attribute(&mut self, opening: Raw) {
        let span = opening.token.span;
        let at = self.out.len();
        let mut depth = 0i32;
        // Each name, and the argument its parentheses held in one word - `aligned(8)` is
        // `("aligned", Some("8"))` and `aligned` alone is `("aligned", None)`.
        let mut found: Vec<(String, Option<String>)> = Vec::new();
        let mut expect_name = true;
        // The argument text being collected, when the name before it took parentheses.
        let mut argument: Option<String> = None;
        loop {
            let Some(raw) = self.next() else {
                self.error(span, "`__attribute__` is not closed");
                return;
            };
            if raw.token.kind == TokenKind::Eof {
                self.error(span, "`__attribute__` is not closed");
                return;
            }
            if self.is_punct(&raw.token, "(") {
                depth += 1;
                if depth == 3 {
                    argument = Some(String::new());
                }
                continue;
            }
            if self.is_punct(&raw.token, ")") {
                depth -= 1;
                if depth == 2 {
                    if let Some(text) = argument.take() {
                        if let Some(last) = found.last_mut() {
                            last.1 = Some(text);
                        }
                    }
                }
                // The outer `)` closes the list: `((` opens two levels and the inner holds the
                // names.
                if depth <= 0 {
                    break;
                }
                continue;
            }
            if depth == 2 {
                if self.is_punct(&raw.token, ",") {
                    expect_name = true;
                    continue;
                }
                if raw.token.kind == TokenKind::Ident && expect_name {
                    found.push((self.spell(raw.token.text).to_owned(), None));
                    expect_name = false;
                }
                continue;
            }
            if depth == 3 {
                // The first word of an argument is the whole of it as far as this compiler reads:
                // `aligned(8)` names a number, and an expression there would need a second constant
                // evaluator in the preprocessor.
                if let Some(text) = argument.as_mut() {
                    if text.is_empty() {
                        text.push_str(self.spell(raw.token.text));
                    }
                }
            }
        }
        for (name, argument) in found {
            self.attribute_named(&name, argument.as_deref(), span, at);
        }
    }

    /// One attribute, by the name the list gave it, found at token `at`.
    fn attribute_named(&mut self, name: &str, argument: Option<&str>, span: Span, at: usize) {
        if HARMLESS_ATTRIBUTES.contains(&name) {
            return;
        }
        if matches!(name, "packed" | "__packed__") {
            self.layout_attributes
                .push((at, crate::types::LayoutAttribute::Packed));
            return;
        }
        if matches!(name, "aligned" | "__aligned__") {
            let alignment = match argument {
                // `aligned` with no argument is the largest alignment the target has.
                None => u64::from(crate::types::Types::DEFAULT_PACK),
                Some(text) => match text.parse::<u64>() {
                    Ok(alignment) if alignment.is_power_of_two() && alignment <= 16 => alignment,
                    _ => {
                        self.error(
                            span,
                            format!(
                                "`__attribute__((aligned({text})))`: the alignment is a power of two up to 16, the largest this target has"
                            ),
                        );
                        return;
                    }
                },
            };
            self.layout_attributes
                .push((at, crate::types::LayoutAttribute::Aligned(alignment)));
            return;
        }
        self.error(
            span,
            format!(
                "`__attribute__(({name}))` is not an attribute this compiler knows: an attribute it dropped could change the program, so it is refused by name rather than ignored. If that attribute asks for nothing this compiler does, adding it to `HARMLESS_ATTRIBUTES` in `pp/mod.rs` is the whole change"
            ),
        );
    }

    /// Keep a token, or splice what it expands to    /// Keep a token, or splice what it expands to in front of the tokens still to come.
    fn emit(&mut self, raw: Raw) {
        if self.out.len() >= self.budgets.max_tokens {
            self.too_many_tokens(raw.token.span);
            return;
        }
        // A byte that is no token: reported here and not in the lexer, because this is the first
        // layer that knows whether the text is part of the program at all. `sqlite3.c` keeps a page
        // of Tcl inside `#if 0`, and every `$` in it is a stray byte to a lexer.
        if raw.token.kind == TokenKind::Stray {
            // The byte is the token's whole spelling, and it is one ASCII byte: every byte of a
            // multi-byte character belongs to an identifier, so what reaches here is a printable
            // character that is no token of C, or a control byte.
            let what = match self.spell(raw.token.text).chars().next() {
                Some(byte) if byte.is_ascii_graphic() => format!("stray `{byte}` in program"),
                Some(byte) => format!("stray byte 0x{:02x} in program", byte as u32),
                None => String::from("stray byte in program"),
            };
            self.error(raw.token.span, what);
            return;
        }
        match self.expansion_of(&raw) {
            Some(expansion) => self.splice_front(expansion),
            None => {
                let mut token = raw.token;
                if let Some(site) = raw.invoked_at {
                    token.expansion = Some(self.sites.len() as u32);
                    self.sites.push(site);
                }
                self.out.push(token);
            }
        }
    }

    fn directive(&mut self, _hash: Raw) {
        // The directive runs to the next newline **that survived phase 2**, not to the next line
        // *number*: `#define ADD(a, b) \` and its body are one line, and reading them by number made
        // every continued macro in real C a syntax error - which is how this was found, by compiling
        // something nobody here wrote.
        let mut words = Vec::new();
        while let Some(next) = self.peek() {
            // A token that begins a logical line ends the directive - including the first one, which
            // is what makes `#` alone a null directive: nothing on its own line comes after it.
            if next.token.kind == TokenKind::Eof || next.token.starts_line {
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

        if std::env::var("XCC_DIRS").is_ok() {
            let line = self.files[self.current as usize].logical_line(span.start);
            eprintln!("DIR {} {} active={}", name_text, line, self.active());
        }
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
                invoked_at: None,
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
                    starts_line: false,
                    expansion: None,
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
                starts_line: false,
                expansion: None,
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
        match crate::headers::refusal(&name) {
            // A name from C99's own table: the compiler is declining on purpose, and the message
            // says so. `cannot find` is what a misspelt name gets, and calling `setjmp.h` a typo
            // hides the reason a reader needs.
            Some(reason) => self.error(span, format!("`<{name}>` is not available: {reason}")),
            None => self.error(span, format!("cannot find `{name}`")),
        }
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
        // A pragma's arguments are macros like any other directive's: `#pragma pack(push,
        // _CRT_PACKING)` is `pack(push, 8)` once `_CRT_PACKING` has been replaced, and reading the
        // name as if it were the number refused six headers of the Windows SDK by name.
        let rest = &self.expand_tokens(rest);
        match rest.first() {
            Some(once) if self.is_spelled(&once.token, "once") => {
                let physical = self.file().physical.clone();
                self.once.insert(physical);
            }
            // The one pragma this compiler does something with that is not "ignore it": it decides
            // the layout of every record declared after it.
            Some(pack) if self.is_spelled(&pack.token, "pack") => self.pack_pragma(rest, span),
            // The other pragma that decides something the compiler must get right: a weak
            // definition is one another unit may replace, and this compiler has no weak symbols -
            // so ignoring it means the definition here wins, which is the wrong program whenever a
            // program wrote it to be overridden. Seven files of 95 119 on this machine write it.
            Some(weak) if self.is_spelled(&weak.token, "weak") => {
                self.error(
                    span,
                    "`#pragma weak` makes a definition replaceable, and this compiler has no weak symbols: ignoring it would give the definition here as the one that wins, so it is refused by name",
                );
            }
            Some(known) => {
                let name = self.spell(known.token.text).to_owned();
                self.warning(span, format!("unknown `#pragma {name}` ignored"));
            }
            None => self.warning(span, "empty `#pragma` ignored"),
        }
    }

    /// `#pragma pack(...)`: the five forms MSVC documents.
    ///
    /// `(N)` sets the limit; `()` returns to the default; `(push)` saves the current limit;
    /// `(push, N)` saves and sets; `(pop)` restores. The limit is 1, 2, 4, 8 or 16 - never past the
    /// alignment this target guarantees. Everything else is refused with the list of forms, because
    /// a pragma somebody wrote and this compiler misread is a layout nobody chose.
    ///
    /// `rest` is every token after `#pragma`, so `rest[0]` is `pack` and what follows is the
    /// parenthesised argument list.
    fn pack_pragma(&mut self, rest: &[Raw], span: Span) {
        let mut words: Vec<String> = Vec::new();
        for raw in rest.iter().skip(1) {
            // The parentheses and commas are the form, not the content.
            if raw.token.kind == TokenKind::Punct {
                continue;
            }
            words.push(self.spell(raw.token.text).to_owned());
        }
        // The action first, so the diagnostics do not have to talk to `&mut self` from inside a
        // match that borrows `words`.
        let form: Vec<&str> = words.iter().map(String::as_str).collect();
        let limit = match form.as_slice() {
            [] => Some(crate::types::Types::DEFAULT_PACK),
            ["push"] => {
                self.pack_stack.push(self.pack);
                return;
            }
            // A `pop` with nothing pushed is the default, which is what MSVC does and what a
            // header that pops defensively expects.
            ["pop"] => Some(
                self.pack_stack
                    .pop()
                    .unwrap_or(crate::types::Types::DEFAULT_PACK),
            ),
            ["push", number] => {
                self.pack_stack.push(self.pack);
                pack_limit(number)
            }
            [number] => pack_limit(number),
            _ => {
                self.error(
                    span,
                    "`#pragma pack` takes `(N)`, `()`, `(push)`, `(push, N)` or `(pop)`, where N is 1, 2, 4, 8 or 16",
                );
                return;
            }
        };
        let Some(limit) = limit else {
            self.error(
                span,
                format!(
                    "`#pragma pack({})`: the limit here is 1, 2, 4, 8 or 16, the alignments this target has",
                    words.last().map(String::as_str).unwrap_or_default()
                ),
            );
            return;
        };
        self.pack = limit;
        if self.pack_changes.last().map(|(_, pack)| *pack) != Some(limit) {
            self.pack_changes.push((self.out.len(), limit));
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
        // `__LINE__` and `__FILE__` name the line *being read*, which during an expansion is where the
        // macro was invoked - not where its body was written. Without this a program's `assert(e)`
        // reported line 28 of `assert.h` and the file `"assert.h"`.
        let site = raw.invoked_at.unwrap_or(Site {
            name: raw.token.text,
            file: raw.token.file,
            span: raw.token.span,
        });
        let entry = &self.files[site.file as usize];
        let quoted =
            |text: &str| format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""));
        let (kind, text) = match spelling.as_str() {
            "__FILE__" => (TokenKind::Str, quoted(&entry.logical)),
            "__LINE__" => (
                TokenKind::Number,
                entry.logical_line(site.span.start).to_string(),
            ),
            "__DATE__" => (TokenKind::Str, quoted(&self.stamp.date)),
            "__TIME__" => (TokenKind::Str, quoted(&self.stamp.time)),
            _ => return None,
        };
        let text = self.interner.intern(&text);
        Some(vec![Raw {
            token: PpToken {
                kind,
                text,
                span: raw.token.span,
                file: raw.token.file,
                starts_line: false,
                expansion: None,
            },
            hide: Vec::new(),
            spliced: true,
            invoked_at: None,
        }])
    }

    fn object_like(&self, definition: &Macro, raw: &Raw) -> Vec<Raw> {
        let mut hide = raw.hide.clone();
        hide.push(raw.token.text);
        // The same rule `substitute` follows: these are the body's tokens, but `__LINE__` and
        // `__FILE__` among them report where the macro was *called* (C99 6.10.8p1). Two paths expand a
        // body, and fixing only one left `#define HERE __LINE__` reporting the `#define`'s line.
        let invoked_at = Some(raw.invoked_at.unwrap_or(Site {
            file: raw.token.file,
            span: raw.token.span,
            name: raw.token.text,
        }));
        definition
            .body
            .iter()
            .map(|token| Raw {
                token: *token,
                hide: hide.clone(),
                spliced: true,
                invoked_at,
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
        // `__LINE__` and `__FILE__` name the invocation's line, not the body's (C99 6.10.8p1), so every
        // token this produces is stamped with where the macro was called. An expansion already
        // carrying a site keeps it: the outermost invocation is the "current source line", because a
        // nested expansion's caller is a line inside another macro.
        //
        // Stamped once, at the end, rather than at each place a token is built: the body's tokens are
        // cloned from `definition.body` in several branches, and one of them answered `__LINE__` with
        // the header's own line for exactly that reason.
        let invoked_at = invocation.invoked_at.unwrap_or(Site {
            name: invocation.token.text,
            file: invocation.token.file,
            span: invocation.token.span,
        });

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
                                starts_line: false,
                                expansion: None,
                            },
                            hide: hide.clone(),
                            spliced: true,
                            invoked_at: Some(invoked_at),
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
                    invoked_at: Some(invoked_at),
                    ..raw
                }));
                at += 1;
                continue;
            }

            out.push(Raw {
                token,
                hide: hide.clone(),
                spliced: true,
                invoked_at: Some(invoked_at),
            });
            at += 1;
        }
        for raw in &mut out {
            if raw.invoked_at.is_none() {
                raw.invoked_at = Some(invoked_at);
            }
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
                invoked_at: None,
            }],
        };
        side.into_iter()
            .map(|raw| Raw {
                hide: hide.to_vec(),
                spliced: true,
                invoked_at: None,
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
                    starts_line: false,
                    expansion: None,
                },
                hide: left.hide.clone(),
                spliced: true,
                invoked_at: None,
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
                    invoked_at: None,
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
                starts_line: false,
                expansion: None,
            };
            let mut joined = Vec::new();
            for (position, part) in arguments.iter().enumerate().skip(params.names.len()) {
                if position > params.names.len() {
                    joined.push(Raw {
                        token: comma,
                        hide: Vec::new(),
                        spliced: true,
                        invoked_at: None,
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
        // The expanded tokens keep the `starts_line` the lexer gave them, which is a fact about where
        // the *body* was written and not about where the expansion lands. Nothing reads it here: the
        // only consumer is `directive`, and a macro is expanded after its directive's line has been
        // gathered - so an expansion can never end a line it is not on.
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

    /// The whole preprocessed output, for the tests that ask it a question (`pack_at`) rather than
    /// only what it spells.
    fn preprocessed_of(source: &str) -> Preprocessed {
        let file = SourceFile::new("t.c", source);
        preprocess(&file, &Options::default(), &())
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

    #[test]
    fn a_macro_reports_the_line_it_was_called_on() {
        // C99 6.10.8p1: `__LINE__` is "the line number of the current source line", and during an
        // expansion the current line is the *invocation's*. A program's `assert(e)` used to report
        // line 28 of `assert.h` and the file `"assert.h"`, because the token being expanded carried
        // the span of the macro's body. Found by compiling Lua with `-DLUAI_ASSERT`, which turns every
        // `api_check` into an `assert`, and every `assert` into a report of its own file and line.
        let source = "#define HERE __LINE__
#define WHERE __FILE__
                      int a = HERE;
                      char *name = WHERE;
                      int b = HERE;
";
        let spells = spelled(source);
        // `int a = 3;` and `int b = 5;`: the invocation's lines, not the body's (line 1 and 2).
        assert!(spells.iter().any(|spelling| spelling == "3"), "{spells:?}");
        assert!(spells.iter().any(|spelling| spelling == "5"), "{spells:?}");
        assert!(
            spells.iter().any(|spelling| spelling == "\"t.c\""),
            "`__FILE__` names the file being read: {spells:?}"
        );
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
    fn pack_pragma_forms_and_their_refusals() {
        // The five forms MSVC documents, each a change to the limit the records declared *after* it
        // are laid out with. The table is keyed by token index - what a pragma is a fact about - and
        // the test asks it the way the parser does: where is this declaration, and what limit
        // applies there.
        let pack_of = |source: &str, name: &str| {
            let preprocessed = preprocessed_of(source);
            let spellings = preprocessed.spellings();
            let at = spellings
                .iter()
                .position(|spelling| spelling == name)
                .unwrap_or_else(|| panic!("no `{name}` in {source:?}"));
            preprocessed.pack_at(at)
        };
        assert_eq!(
            pack_of("int x;\n", "x"),
            16,
            "the ABI's alignment, untouched"
        );
        assert_eq!(pack_of("#pragma pack(1)\nint x;\n", "x"), 1);
        let nested = "#pragma pack(push, 2)\nint x;\n#pragma pack(pop)\nint y;\n";
        assert_eq!(pack_of(nested, "x"), 2);
        assert_eq!(pack_of(nested, "y"), 16, "`pop` restores what `push` saved");
        assert_eq!(
            pack_of("#pragma pack(1)\n#pragma pack()\nint x;\n", "x"),
            16
        );
        // A `pop` with nothing pushed is the default, which is what MSVC does and what a header
        // that pops defensively relies on.
        assert_eq!(pack_of("#pragma pack(pop)\nint x;\n", "x"), 16);
        // The limit is one of the target's alignments and the form is one of the five: a pragma
        // this compiler misread is a layout nobody chose.
        assert!(diagnostics("#pragma pack(3)\nint x;\n")[0]
            .message
            .contains("the limit here is 1, 2, 4, 8 or 16"));
        assert!(diagnostics("#pragma pack(push, 2, 3)\nint x;\n")[0]
            .message
            .contains("takes `(N)`, `()`, `(push)`, `(push, N)` or `(pop)`"));
    }

    #[test]
    fn pragmas_are_ignored_with_an_answer() {
        assert!(diagnostics("#pragma once\n").is_empty());
        // `#pragma pack` is no longer one of the ignored ones - it is implemented, and the test
        // above asks it what limit it leaves in force. What stays ignored is what asks for nothing.
        assert!(diagnostics("#pragma pack(push,1)\nstruct S { char c; int i; };\n").is_empty());
        assert_eq!(
            diagnostics("#pragma something_else\n")[0].message,
            "unknown `#pragma something_else` ignored"
        );
        // Except the pragmas that decide something wrongly if dropped: `weak` is one, and
        // ignoring it makes the definition here the one that wins.
        let weak = diagnostics("#pragma weak f\n");
        assert_eq!(weak[0].severity, Severity::Error);
        assert!(weak[0]
            .message
            .contains("`#pragma weak` makes a definition replaceable"));
    }

    #[test]
    fn a_byte_that_is_no_token_matters_only_where_it_is_used() {
        // Translation phase 3 produces tokens and phase 4 decides what is in the program, so a byte
        // that is no token is only a problem where it survives. `sqlite3.c` keeps a page of Tcl
        // inside `#if 0`, and every `$` there is a stray byte to a lexer that judges too early -
        // which refused the file 250 000 lines before the first line that matters.
        assert!(
            diagnostics("#if 0\nforeach c {d s} {\n  scan $c %c x\n}\n#endif\nint x;\n").is_empty()
        );
        let refused = diagnostics("int x = 1 $ 2;\n");
        assert_eq!(refused.len(), 1);
        assert_eq!(refused[0].message, "stray `$` in program");
        // A byte that is not printable is named by its value rather than printed.
        assert_eq!(
            diagnostics("int x = 1 \u{1} 2;\n")[0].message,
            "stray byte 0x01 in program"
        );
    }

    #[test]
    fn attribute_lists_mean_nothing_and_are_dropped() {
        // The attributes that do not change anything this compiler implements are dropped wherever
        // a declaration can carry one, because the parser is not the place that would notice them
        // all: before a declaration, after a declarator, on a member, on a parameter, after a
        // struct's `}` - and through a macro, which is how cJSON writes its public ones.
        assert!(diagnostics("__attribute__((unused)) int g;\n").is_empty());
        assert!(diagnostics("int f(void) __attribute__((noreturn));\n").is_empty());
        assert!(diagnostics("int f(int x __attribute__((unused)));\n").is_empty());
        assert!(diagnostics("__attribute__((noreturn, cold)) int g;\n").is_empty());
        assert_eq!(
            spelled("__attribute__((unused)) int g;"),
            ["int", "g", ";"],
            "the tokens are gone, not merely tolerated"
        );
        assert_eq!(
            spelled("int f(void) __attribute__((format(printf, 1, 2)));"),
            ["int", "f", "(", "void", ")", ";"],
            "an argument list inside the attribute is part of the attribute"
        );
    }

    #[test]
    fn the_two_attributes_that_decide_a_layout_are_recorded_where_they_were_written() {
        // `packed` and `aligned(N)` are not dropped: they are facts about a *position*, because that
        // is how the parser decides what an attribute is on - the range it parsed. The numbers are
        // measured against `clang` in `tests/exe.rs`.
        let attributes = |source: &str| {
            let preprocessed = preprocessed_of(source);
            preprocessed.layout_attributes(0, preprocessed.tokens.len())
        };
        assert_eq!(
            attributes("struct S { int a; } __attribute__((packed));\n"),
            [crate::types::LayoutAttribute::Packed]
        );
        assert_eq!(
            attributes("struct S { int a; } __attribute__((aligned(8), packed));\n"),
            [
                crate::types::LayoutAttribute::Aligned(8),
                crate::types::LayoutAttribute::Packed
            ]
        );
        // `aligned` with no argument is the largest alignment the target has.
        assert_eq!(
            attributes("struct S { int a; } __attribute__((aligned));\n"),
            [crate::types::LayoutAttribute::Aligned(16)]
        );
        // Anything else is refused by name, because an attribute nobody thought about could change
        // the program - and the first version of this reader dropped everything except the two names
        // it knew, which quietly accepted `vector_size`, `section`, `cleanup` and `weak`.
        for attribute in [
            "vector_size(16)",
            "section(\".text.hot\")",
            "cleanup(free)",
            "weak",
        ] {
            let refused = diagnostics(&format!("int g __attribute__(({attribute}));\n"));
            assert_eq!(refused.len(), 1, "{attribute}");
            assert!(
                refused[0]
                    .message
                    .contains("is not an attribute this compiler knows"),
                "{attribute}: {}",
                refused[0].message
            );
        }
        // An alignment past what this target guarantees is refused, as `_Alignas` is.
        assert!(diagnostics("int g __attribute__((aligned(32)));\n")[0]
            .message
            .contains("the alignment is a power of two up to 16"));
        // Nested inside another attribute's arguments it is an argument, not the attribute.
        assert!(diagnostics("__attribute__((warn_unused_result(packed))) int g;\n").is_empty());
        // A malformed one is closed by name rather than eating the file.
        assert!(!diagnostics("__attribute__((unused int g;\n").is_empty());
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
    fn a_macro_can_continue_across_lines() {
        // C99 5.1.1.2 phase 2 removes a backslash-newline, so what follows it is part of the *same*
        // logical line - and a directive ends at a newline, not at a line number. Reading directives
        // by line number made every continued macro a syntax error, and no corpus file has a
        // line-ending backslash: it took compiling a real one (`cJSON.c`) to find it.
        let source = "#define ADD(a, b) \
    ((a) + (b))
int f(void) { return ADD(20, 22); }";
        let file = SourceFile::new("t.c", source);
        let options = Options::default();
        let preprocessed = preprocess(&file, &options, &());
        assert!(!preprocessed.has_errors(), "{:?}", preprocessed.diagnostics);
        // The macro is defined with both parts of its body: the expansion spells them.
        let spelled: Vec<&str> = preprocessed
            .tokens
            .iter()
            .filter(|token| token.kind != TokenKind::Eof)
            .map(|token| preprocessed.text(token))
            .collect();
        let text = spelled.join(" ");
        assert!(text.contains("( 20 ) + ( 22 )"), "{text}");
    }

    #[test]
    fn the_standard_macros_are_defined() {
        let (spells, _) = run_options(
            "__STDC__ __STDC_VERSION__ __STDC_HOSTED__",
            &Options::default(),
            &(),
        );
        assert_eq!(spells, ["1", "199901L", "1"]);
        // And the dialect decides the version, once, in the macro it installs.
        let (spells, _) = run_options(
            "__STDC_VERSION__",
            &Options {
                standard: crate::options::CStandard::C11,
                ..Options::default()
            },
            &(),
        );
        assert_eq!(spells, ["201112L"]);
    }

    /// The three ways a program can ask are the same question, and must agree.
    ///
    /// `__STDC__` was substituted during expansion and never installed as a macro, so
    /// `#if __STDC__` was true while `#ifdef __STDC__` - and `defined(__STDC__)` - were false. C99
    /// 6.10.8p2 makes them macros; a program that asks with `#ifdef` is the common case, not the
    /// exotic one.
    #[test]
    fn the_predefined_macros_answer_the_same_way_everywhere() {
        let names = [
            "__STDC__",
            "__STDC_VERSION__",
            "__STDC_HOSTED__",
            "_WIN32",
            "_WIN64",
            "__x86_64__",
            "_M_X64",
        ];
        for name in names {
            let source = format!(
                "#if defined({name})
counted
#endif
#ifdef {name}
ifdefed
#endif
#if {name}
valued
#endif
"
            );
            let (spells, diagnostics) = run_options(&source, &Options::default(), &());
            assert!(
                diagnostics.is_empty(),
                "{name} has no diagnostics: {diagnostics:?}"
            );
            assert_eq!(
                spells,
                ["counted", "ifdefed", "valued"],
                "{name} must answer the same way to `defined`, `#ifdef` and `#if`"
            );
        }
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
