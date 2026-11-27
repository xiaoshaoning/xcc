//! The parser: declarations, declarators, and the state the other two files work on.
//!
//! Three files, one `Parser`: this one holds the tokens, the type arena, the scopes and the
//! declaration grammar; [`expr`] answers what an expression's value is and may be combined
//! with; [`stmt`] answers what a statement must be true of. They are `impl Parser` blocks
//! rather than separate types because they share that state and nothing else, and splitting
//! the state would mean passing it around.
//!
//! There is no syntax tree, on purpose ([ADR-002](../../../docs/ADR-002-intermediate-form.md)):
//! the parser is written so that the *value* of an expression becomes a value in `cir` at M4,
//! and a parser that built a tree first would be a second encoding of what the type checker
//! already knows. What it produces today is a typed symbol table and diagnostics; what it will
//! produce at M4 is the same, plus `cir`.
//!
//! Two rules shape the whole thing:
//!
//! - **A malformed construct becomes the error type**, not a cascade: after the first
//!   diagnostic the type is [`Type::Error`], which is compatible with everything and never
//!   produces a second complaint. C code with one typo should give one message.
//! - **Recursion is budgeted** (`Budgets::max_nesting`): a declarator or expression nested past
//!   the limit is a diagnostic, not a stack overflow, because the host owns the stack
//!   (CONSTRAINTS 5).

mod bytes;
mod complex;
mod emit;
mod expr;
mod init;
mod stmt;

use std::collections::HashMap;

use crate::cir;
use crate::diag::{Diagnostic, Span};
use crate::intern::Symbol;
use crate::literal::{character_constant, string_length};
use crate::options::Budgets;
use crate::pp::{PpToken, Preprocessed};
use crate::symtab::{Entry, Kind, Symbols};
use crate::types::{Member, RecordId, RecordKind, Type, TypeId, Types};

/// Parse a preprocessed translation unit.
///
/// Returns every diagnostic, in source order. An empty list means the declarations are
/// well formed and typed; it says nothing about the code an expression would generate, because
/// there is no code generation yet.
pub(crate) fn parse(preprocessed: &mut Preprocessed, budgets: &Budgets) -> Parsed {
    let mut parser = Parser {
        preprocessed,
        at: 0,
        types: Types::new(),
        symbols: Symbols::new(),
        diagnostics: Vec::new(),
        dropped: 0,
        errors: 0,
        budgets,
        nesting: 0,
        function_result: None,
        loops: 0,
        current_function: None,
        links: std::collections::HashMap::new(),
        func_literals: std::collections::HashMap::new(),
        switches: Vec::new(),
        labels: HashMap::new(),
        blocks_reported: std::collections::HashSet::new(),
        label_blocks: HashMap::new(),
        gotos: Vec::new(),
        last_parameters: Vec::new(),
        captured_parameters: false,
        last_variadic: false,
        function_params: Vec::new(),
        funcs: Vec::new(),
        globals: Vec::new(),
        globals_by_name: HashMap::new(),
        body: None,
        last_declared: None,
        last_linkage: Linkage::External,
        function_buffer: None,
        bytes: None,
        init_relocations: Vec::new(),
        init_grows: false,
        implicit: std::collections::HashSet::new(),
        implicit_types: std::collections::HashSet::new(),
        implicit_results: std::collections::HashSet::new(),
        offset: 0,
        init_target: None,
        bits: None,
        zero_elements: false,
        unevaluated: false,
        literals: 0,
        targets: Vec::new(),
    };
    parser.translation_unit();
    Parsed {
        dropped: parser.dropped,
        module: cir::Module {
            types: parser.types,
            funcs: parser.funcs,
            globals: parser.globals,
        },
        diagnostics: parser.diagnostics,
    }
}

/// What parsing produced: a module with a body for every function that was lowered, and
/// whatever was wrong with the source.
pub(crate) struct Parsed {
    /// The translation unit as `cir`: `Module::dump` writes it, a back end lowers it.
    pub module: cir::Module,
    /// Errors and warnings, in source order.
    pub diagnostics: Vec<Diagnostic>,
    /// How many were refused by the budget. A list capped at the budget cannot show that it was
    /// capped - its length is the budget, not one more - so the count travels to whoever reports it.
    pub dropped: usize,
}

/// A function whose body is being read: the `cir` function, and the block still being filled.
struct Body {
    func: cir::Func,
    current: cir::Block,
}

impl Body {
    /// A builder for the block being filled.
    fn builder(&mut self) -> cir::Builder<'_> {
        cir::Builder::at(&mut self.func, self.current)
    }
}

/// A value: what an expression evaluates to, as far as the front end knows.
#[derive(Debug, Clone, Copy)]
struct Value {
    ty: TypeId,
    /// The value, when it is known at compile time: an integer's value, or a float's bit
    /// pattern, which is what `cir::Const` carries for either. `Parser::constant_is_zero`
    /// and `Parser::null_pointer_constant` are the two questions asked of it, and they do not
    /// mean the same thing for a float (`-0.0` is zero, and is not a null pointer).
    constant: Option<i64>,
    /// Whether it can be assigned to: a variable, an element, a dereference.
    lvalue: bool,
    /// The `cir` value, when code was generated for it.
    cir: Option<cir::Value>,
    /// The address, when the value is a place that can be assigned to or taken.
    place: Option<cir::Value>,
    /// The address of a symbol, and how many bytes into it, when the value *is* an address and there
    /// is no code to hold it: at file scope there is no `cir` yet, and `char *p = "hi"`,
    /// `hooks = { my_malloc }` and `sqlite3aLTb = &table[256-OP_Ne]` are eight bytes the linker
    /// fills. The second half is the *addend* - what makes `&a[i]` a place in the data section and
    /// not a missing feature - and it travels to the linker inside the relocation.
    address: Option<(Symbol, i64)>,
    /// For a bitfield: the bit it starts at in the storage unit whose address is `place`, and
    /// its width. Reading the value went through it; writing one has to read the unit, clear
    /// these bits and put them back, so the places that write need to know.
    bits: Option<(u8, u8)>,
}

impl Value {
    /// A value with no code behind it: a type the parser worked out, and nothing to run. Error
    /// paths produce these, so one mistake is one diagnostic.
    fn new(ty: TypeId, constant: Option<i64>, lvalue: bool) -> Self {
        Self {
            ty,
            constant,
            lvalue,
            cir: None,
            place: None,
            address: None,
            bits: None,
        }
    }

    fn temporary(ty: TypeId) -> Self {
        Self::new(ty, None, false)
    }

    fn constant(ty: TypeId, constant: i64) -> Self {
        Self::new(ty, Some(constant), false)
    }

    /// The same value, with the constant C folded out of it kept.
    fn with_constant(mut self, constant: Option<i64>) -> Self {
        self.constant = constant;
        self
    }

    /// The same value carrying `cir`, for the places that build one by hand.
    fn with_cir(mut self, cir: Option<cir::Value>) -> Self {
        self.cir = cir;
        self
    }

    /// The same value, carrying the symbol whose address it is: what an address is when there is no
    /// code to hold one.
    fn with_address(mut self, address: Option<(Symbol, i64)>) -> Self {
        self.address = address;
        self
    }

    /// A value that code was generated for.
    fn code(ty: TypeId, cir: cir::Value) -> Self {
        Self {
            ty,
            constant: None,
            lvalue: false,
            cir: Some(cir),
            place: None,
            bits: None,
            address: None,
        }
    }
}

/// The largest alignment this compiler supports: sixteen bytes, which is `max_align_t` on this ABI
/// and what the stack guarantees. C11 6.7.5p6 lets an implementation decline the *extended*
/// alignments above it, and this one does, with a diagnostic naming the number asked for.
/// The Microsoft intrinsics this compiler implements, with the signatures MSVC gives them: the
/// result type and the parameter types. Pointers are spelled with a trailing ` *`.
///
/// A program calls these with no declaration - that is how MSVC works, and its `<intrin.h>` is only
/// one way to get them - and an implicit `int` is *wrong* rather than merely unchecked: the value is
/// truncated. The list is the same fifteen `include/x86intrin.h` declares, because a program that
/// includes the header and a program that does not are the same program.
const INTRINSICS: &[(&str, &str, &[&str])] = &[
    ("_mm_mfence", "void", &[]),
    (
        "_InterlockedCompareExchange",
        "long",
        &["long *", "long", "long"],
    ),
    ("_InterlockedExchange", "long", &["long *", "long"]),
    ("_InterlockedExchangeAdd", "long", &["long *", "long"]),
    ("_InterlockedIncrement", "long", &["long *"]),
    ("_InterlockedDecrement", "long", &["long *"]),
    ("_InterlockedAnd", "long", &["long *", "long"]),
    ("_InterlockedOr", "long", &["long *", "long"]),
    ("_InterlockedXor", "long", &["long *", "long"]),
    (
        "__shiftright128",
        "unsigned long long",
        &["unsigned long long", "unsigned long long", "unsigned char"],
    ),
    (
        "__shiftleft128",
        "unsigned long long",
        &["unsigned long long", "unsigned long long", "unsigned char"],
    ),
    (
        "_umul128",
        "unsigned long long",
        &[
            "unsigned long long",
            "unsigned long long",
            "unsigned long long *",
        ],
    ),
    (
        "_mul128",
        "long long",
        &["long long", "long long", "long long *"],
    ),
    (
        "__stosb",
        "void",
        &["unsigned char *", "unsigned char", "unsigned long long"],
    ),
    (
        "__stosw",
        "void",
        &["unsigned short *", "unsigned short", "unsigned long long"],
    ),
    (
        "__stosd",
        "void",
        &["unsigned long *", "unsigned long", "unsigned long long"],
    ),
    (
        "__stosq",
        "void",
        &[
            "unsigned long long *",
            "unsigned long long",
            "unsigned long long",
        ],
    ),
    (
        "__movsb",
        "void",
        &["unsigned char *", "unsigned char *", "unsigned long long"],
    ),
    (
        "__movsw",
        "void",
        &["unsigned short *", "unsigned short *", "unsigned long long"],
    ),
    (
        "__movsd",
        "void",
        &["unsigned long *", "unsigned long *", "unsigned long long"],
    ),
    (
        "__movsq",
        "void",
        &[
            "unsigned long long *",
            "unsigned long long *",
            "unsigned long long",
        ],
    ),
];

const FUNDAMENTAL_ALIGNMENT: u64 = 16;
/// The largest alignment an object in a data section may ask for. A page is the widest thing
/// a linker places specially, and a number past it is a request nobody makes.
const STATIC_ALIGNMENT_LIMIT: u64 = 4096;

/// The words that can begin a declaration: storage classes, qualifiers, type specifiers and the tag
/// keywords. `starts_specifier` asks this list, and **a word the specifier reader accepts and this
/// list does not is a word that only works at file scope** - `_Alignas(16) int x;` inside a function
/// was read as a call to a function called `_Alignas`, because the list is what decides whether a
/// statement is a declaration. `a_declaration_can_begin_with_every_specifier` runs each of these
/// words at block scope, which is the direction that fails when they drift.
const SPECIFIERS: &[&str] = &[
    "typedef",
    "extern",
    "static",
    "auto",
    "register",
    "const",
    "volatile",
    "restrict",
    "inline",
    "void",
    "_Bool",
    "char",
    "short",
    "int",
    "long",
    "float",
    "double",
    "signed",
    "unsigned",
    "struct",
    "union",
    "enum",
    "_Atomic",
    "_Alignas",
    "_Complex",
    "__int8",
    "__int16",
    "__int32",
    "__int64",
    "__extension__",
    "__inline",
    "__inline__",
    "__forceinline",
    "_forceinline",
    "__restrict",
    "__unaligned",
    "__ptr32",
    "__ptr64",
    "__w64",
    // Refused where the specifiers are read, and it must still be *here*: this list is what decides
    // whether a statement at block scope is a declaration, so a word the reader accepts and this
    // list does not is a word that only works at file scope. `_Imaginary float z;` in a function
    // said "`_Imaginary` is not declared" until it was added.
    "_Imaginary",
    "_Thread_local",
];

/// One `switch` being read: the `case` values seen so far, so a duplicate is caught
/// (C99 6.8.4.2p3), and whether `default` has been seen.
#[derive(Default)]
struct Switch {
    cases: std::collections::HashSet<i64>,
    has_default: bool,
    /// The value tested, and the type the comparisons happen in, when a body is being built.
    condition: Option<cir::Value>,
    condition_type: Option<TypeId>,
    /// Where `break` goes, where the comparison chain is written, and the arms in source order.
    exit: Option<cir::Block>,
    test: Option<cir::Block>,
    arms: Vec<(Option<i64>, cir::Block)>,
}

struct Parser<'a> {
    preprocessed: &'a mut Preprocessed,
    /// Position in the token stream.
    at: usize,
    types: Types,
    symbols: Symbols,
    diagnostics: Vec<Diagnostic>,
    /// How many diagnostics the budget refused. A list capped at the budget cannot show that it was
    /// capped - its length is the budget, not one more - so whoever reports it needs this number.
    dropped: usize,
    /// How many *errors* have been reported, whether the budget kept them or not.
    ///
    /// Separate from the list because a rule asks "was this body read without an error?", and an
    /// answer that changed with the diagnostic budget would make the parse itself depend on a limit
    /// meant for the report. It did: a capped list stopped growing, the count taken from it stopped
    /// growing with it, and a capped parse produced a hundred and fifteen diagnostics that a
    /// generous one never made.
    errors: usize,
    budgets: &'a Budgets,
    /// How deep the current recursion is, for the budget.
    nesting: usize,
    /// What the function being read returns, while its body is being read.
    function_result: Option<TypeId>,
    /// How many loops surround the statement being read: `continue` needs one.
    loops: usize,
    /// The `__func__` object of each function that asked for one, by the function's name: one object
    /// per function rather than one per use, because a logging macro that prints `__func__` is
    /// expanded a dozen times in one body and twelve copies of one name is twelve copies of bytes.
    func_literals: std::collections::HashMap<Symbol, (TypeId, String)>,
    /// The names a declaration renamed with `__asm__("...")`: what a symbol is *linked* as, when
    /// that is not what the program calls it. mingw-w64's headers rename every CRT function whose
    /// narrow and wide spellings are the same name here - `vsscanf` is `__mingw_vsscanf`, `_wctime`
    /// is `_wctime64` - and the alternative is refusing the Windows SDK's `stdio.h`.
    links: std::collections::HashMap<Symbol, String>,
    /// The function whose body is being read, for the one thing that needs its name: a `static`
    /// local's object is named for the function that declares it.
    current_function: Option<Symbol>,
    /// The `switch`es being read, innermost last, each with the `case` values it already has.
    switches: Vec<Switch>,
    /// The labels defined in the function being read, function-scoped (C99 6.2.1p4), and the
    /// `goto`s that are still waiting for one.
    labels: HashMap<Symbol, Span>,
    /// Whether the block budget of the function being read has already been reported: a source
    /// over it makes hundreds of blocks in a row, and one diagnostic is the answer, not one each.
    blocks_reported: std::collections::HashSet<()>,
    /// Where a label's code starts. Whichever of the label and a `goto` to it comes first
    /// creates the block; the other one switches to it.
    label_blocks: HashMap<Symbol, cir::Block>,
    gotos: Vec<(Symbol, Span)>,
    /// The parameter names of the last function declarator, which a definition declares in its
    /// body's scope.
    last_parameters: Vec<(Option<Symbol>, TypeId, Span)>,
    /// Whether the declarator being read has already handed its parameters over. A declarator can
    /// hold *two* parameter lists - `int (*getter(vfs *p))(struct vfs *)` is a function taking one
    /// `vfs *` and returning a function pointer taking `struct vfs *` - and the one that belongs to
    /// the name being declared is the **first**, the one inside the parentheses. Reading the second
    /// into `last_parameters` too replaced the function's own parameters with its *return type's*,
    /// so `p` was not declared in the body.
    captured_parameters: bool,
    /// Whether the parameter list just read ended in `...`, for the definition that follows.
    last_variadic: bool,
    /// The place of each parameter of the function being defined, in order: what `va_start` needs to
    /// know which one was named as the last.
    function_params: Vec<Option<cir::Value>>,
    /// The functions whose bodies have been read.
    funcs: Vec<cir::Func>,
    /// The globals declared so far: one per name, because C99 6.9p5 gives an identifier one
    /// definition and the back end treats a second as an error rather than as C.
    globals: Vec<cir::Global>,
    /// Where each global's definition sits in `globals`, so a later declaration completes it
    /// instead of appending a duplicate.
    globals_by_name: HashMap<Symbol, usize>,
    /// The function being read, while its body is being parsed.
    body: Option<Body>,
    /// The name the last declarator declared, for a definition that follows it.
    last_declared: Option<Symbol>,
    /// What the declaration just read said about the linkage of `last_declared`.
    ///
    /// One value, because `static`, `extern` and `inline` are three spellings of one question, and
    /// they were three bools read together in one place - which is a fact written down three times
    /// and waiting for a fourth.
    last_linkage: Linkage,
    /// The buffer a record-returning function was handed by its caller, while its body is being
    /// read (Win64: the caller owns the result, and the callee returns its address).
    function_buffer: Option<cir::Value>,
    /// Where the bytes of a file-scope initialiser are being written, when one is: the same
    /// traversal that checks an initialiser fills this, so the bytes and the diagnostics come
    /// from one reading of the source (ADR-004).
    bytes: Option<Vec<u8>>,
    /// The addresses written into those bytes: `char *p = "hello"` at file scope is eight bytes the
    /// linker has to fill, not a value this compiler knows.
    init_relocations: Vec<cir::GlobalRelocation>,
    /// Names taken to be implicitly declared functions, so the warning is one per name rather than
    /// one per call.
    implicit: std::collections::HashSet<Symbol>,
    /// The function *types* invented for those declarations, so a call can be recognised as theirs,
    /// and the `cir` values those calls produced. C89 let such a result be assigned to a pointer -
    /// the two were one width on every ABI of the era - and MSVC still does, with C4047.
    implicit_types: std::collections::HashSet<TypeId>,
    implicit_results: std::collections::HashSet<cir::Value>,
    /// Whether the object being initialised still has no size, so its bytes may grow: `int a[] = {1,2,3}`
    /// learns its length from the list. A *complete* object may not grow - writing past it is an error
    /// the brace walker has already reported, and a designator that reaches past the end is one it
    /// reports by refusing the initialiser.
    init_grows: bool,
    /// The offset into `bytes` that the initialiser being read is at, in bytes from the
    /// object's start - for a local it is the same offset, from the slot's address.
    offset: usize,
    /// The frame slot a local initialiser is writing through, with the type of the object
    /// it is: `Some` while such an initialiser is being read, and `None` for a file-scope
    /// object, whose bytes `bytes` collects (ADR-004).
    ///
    /// These three move together: `initializer_bytes` sets `bytes` and `initialize_local`
    /// sets `init_target`, both start `offset` at zero, and a compound literal suspends all
    /// three while it reads an object of its own (`cast`).
    init_target: Option<(cir::Value, TypeId)>,
    /// The bitfield a leaf of the initialiser being walked is a field of: its start and width in
    /// the storage unit at the leaf's offset, or `None` for a leaf that is a whole object. It is
    /// part of the same flat state `offset` and `init_target` are, and `write_leaf` reads it.
    bits: Option<(u8, u8)>,
    /// Whether the object being initialised was thin on zeroes: true when its size came from the
    /// initialiser itself, so nothing could zero the whole of it before the walk. The array walk
    /// reads it and zeroes each element as it reaches it.
    zero_elements: bool,
    /// Whether the expression being read is not evaluated: `sizeof`'s operand, which C says is
    /// not (C99 6.5.3.4p2), and a `constant_expression`, which is wanted for its value and not
    /// for its code. Nothing parsed in either may emit: `sizeof(x = 3)` must not assign, and
    /// `int a[1 ? 2 : 3]` must not leave a branch in the middle of a declaration.
    unevaluated: bool,
    /// How many string literals have been given an anonymous global: `.str0`, `.str1`, ...
    literals: u32,
    /// Where `break` leaves and, for a loop, where `continue` goes: innermost last.
    ///
    /// One stack, because `break` leaves whichever of a loop and a `switch` is nearer, and two
    /// stacks cannot say which that is - the bug that made `break` inside a `switch` inside a
    /// `for` leave the *loop*. A `switch` pushes its exit with no continue target, and
    /// `continue` finds the nearest entry that has one.
    targets: Vec<(cir::Block, Option<cir::Block>)>,
}

impl Parser<'_> {
    // ---- tokens ---------------------------------------------------------------

    fn tokens(&self) -> &[PpToken] {
        &self.preprocessed.tokens
    }

    fn peek(&self) -> Option<&PpToken> {
        self.tokens().get(self.at)
    }

    fn peek_at(&self, ahead: usize) -> Option<&PpToken> {
        self.tokens().get(self.at + ahead)
    }

    fn bump(&mut self) -> Option<PpToken> {
        let token = self.tokens().get(self.at).copied();
        if token.is_some() {
            self.at += 1;
        }
        token
    }

    /// How a token is spelled.
    fn spell(&self, token: &PpToken) -> &str {
        self.preprocessed.text(token)
    }

    fn is_punct(&self, token: &PpToken, spelling: &str) -> bool {
        token.kind == crate::lex::TokenKind::Punct && self.spell(token) == spelling
    }

    fn is_spelled(&self, token: &PpToken, spelling: &str) -> bool {
        self.spell(token) == spelling
    }

    /// Whether the token at the cursor is a punctuator spelled `spelling`.
    fn at_punct(&self, spelling: &str) -> bool {
        self.peek()
            .is_some_and(|token| self.is_punct(token, spelling))
    }

    /// Whether the token at the cursor is spelled `spelling` (a keyword, usually).
    fn at_word(&self, spelling: &str) -> bool {
        self.peek()
            .is_some_and(|token| self.is_spelled(token, spelling))
    }

    fn at_eof(&self) -> bool {
        self.peek()
            .is_none_or(|token| token.kind == crate::lex::TokenKind::Eof)
    }

    /// Consume the punctuator if it is there, saying so.
    fn eat(&mut self, spelling: &str) -> bool {
        if self.at_punct(spelling) {
            self.at += 1;
            return true;
        }
        false
    }

    /// The span of the token at the cursor, for a diagnostic that has nowhere better to point.
    /// The span of what the parser is standing on, as the *program* wrote it: a token from a macro
    /// expansion reports the invocation, not the body.
    ///
    /// A diagnostic about a token a macro produced points at the definition - right for "this line
    /// is wrong", and wrong for "argument 1 is wrong", where the thing the reader has to fix is the
    /// call site. The site is where both facts meet: it is the invocation's span, so the caret lands
    /// on the use and the note names the macro.
    fn program_span(&self) -> Span {
        self.tokens()
            .get(self.at)
            .and_then(|token| self.preprocessed.site(token))
            .map_or_else(|| self.span(), |site| site.span)
    }

    fn span(&self) -> Span {
        self.peek().map_or_else(
            || {
                self.tokens()
                    .last()
                    .map_or(Span::default(), |token| token.span)
            },
            |token| token.span,
        )
    }

    /// Refuse a construct that M3 has not built yet. Recovers where it was refused, so that one
    /// "not yet" is the whole answer.
    fn not_implemented(&mut self, span: Span, message: impl Into<String>) {
        self.error(span, message);
    }

    fn error(&mut self, span: Span, message: impl Into<String>) {
        self.push(Diagnostic::error(message), span);
    }

    /// Something C asks to be reported and that a translation can carry on past: an excess array
    /// initialiser, a call to a function nothing declared. The distinction is the user's - an error
    /// means "no object", a warning means "built, and you should look".
    fn warning(&mut self, span: Span, message: impl Into<String>) {
        self.push(Diagnostic::warning(message), span);
    }

    fn push(&mut self, diagnostic: Diagnostic, span: Span) {
        // The budget is spent *here*, not by whoever renders the list: a rule that recovers without
        // advancing can produce a diagnostic per iteration, and a vector that grows to hold them all
        // allocated eleven gigabytes on a one-line struct member before the truncation that was
        // supposed to bound it ever ran. A bound applied where the list grows bounds the list.
        if diagnostic.is_error() {
            // Before the budget check: the count is about the source, not about the report.
            self.errors += 1;
        }
        if self.diagnostics.len() >= self.budgets.max_diagnostics {
            self.dropped += 1;
            return;
        }
        let file = self.peek().map_or(0, |token| token.file);
        let name = self
            .preprocessed
            .files
            .get(file as usize)
            .map(|source| source.name().to_owned())
            .unwrap_or_default();
        // A diagnostic about a token a macro produced points at the macro's *body* - which is right,
        // because that is where the token was written - and the body cannot say which use it came
        // from. This does: `14617:40` in sqlite3.c is the definition of `SQLITE_INT_TO_PTR`, and two
        // of its seventeen uses fail, which the diagnostic could not say until now.
        let mut diagnostic = diagnostic.with_file(name).with_span(span);
        // Which macro invocation this diagnostic is about, and there are two routes because it can
        // name one of two things.
        //
        // A token *from* a macro body carries its `expansion`, so the site is read from the token -
        // and that token is one of two: the one just consumed (most diagnostics are reported after
        // the token was read, which is when the name is known) or the one under the cursor. A span
        // alone cannot identify it, because every expansion of one macro produces tokens with the
        // same body spans.
        //
        // A span that names the macro *use* - `f(ARG)`, whose "argument 1 is pointer to char" points
        // at the argument, which is the invocation - has no token at all: the invocation was replaced
        // by its expansion before the parser saw it. That one is looked up by span, in the file the
        // parser is reading.
        // The nearest token at or before the cursor that carries this span, and that is a scan
        // rather than a window: a diagnostic about a macro *body* can point at any of its tokens, and
        // the parse that complains is often several tokens past it. `#define TERN (c ? p : q)` used as
        // `int r = TERN;` complains at the `?` - four tokens back - and a two-token window found no
        // token at all, so the note was missing for a reason that looked like a limit on distance but
        // was a limit on belief.
        //
        // **At** the cursor as well as before it: the window this replaced tried `at - 1` *and* `at`,
        // and a diagnostic raised before the token is consumed - "expected an expression, found `)`"
        // inside `#define T (1 + )` - points at exactly that one. Leaving it out was a narrower search
        // than the one it was meant to widen, and it took a measurement of the three cases to see it.
        let site = (0..=self.at.min(self.tokens().len().saturating_sub(1)))
            .rev()
            .filter_map(|index| self.tokens().get(index))
            .find(|token| token.span == span)
            .and_then(|token| self.preprocessed.site(token))
            .map(|site| (site, true))
            .or_else(|| {
                self.preprocessed
                    .site_at(file, span)
                    .map(|site| (site, false))
            });
        if let Some((site, from_body)) = site {
            let used = self
                .preprocessed
                .files
                .get(site.file as usize)
                .map(|source| {
                    let at = source.position(site.span.start);
                    format!("{}:{}:{}", source.name(), at.line, at.column)
                })
                .unwrap_or_default();
            let macro_name = self.spell_of(site.name).to_owned();
            // A body token needs the use's location, because the caret is under the definition. A
            // span that *is* the use would repeat the caret's own file:line, so it says only which
            // macro the text came from.
            diagnostic = diagnostic.with_note(if from_body {
                format!("in `{macro_name}`, used at {used}")
            } else {
                format!("in `{macro_name}`")
            });
        }
        self.diagnostics.push(diagnostic);
    }

    // ---- the grammar ----------------------------------------------------------

    fn translation_unit(&mut self) {
        while !self.at_eof() {
            let start = self.at;
            self.external_declaration();
            if self.at == start {
                // Nothing was consumed: the token is not the start of a declaration. Say so
                // once and move on, so one stray token is one diagnostic.
                let span = self.span();
                let spelling = self.peek().map(|token| self.spell(token).to_owned());
                self.error(
                    span,
                    format!(
                        "expected a declaration, found `{}`",
                        spelling.unwrap_or_default()
                    ),
                );
                self.bump();
            }
        }
        // C99 6.9.2p2: a tentative definition of incomplete array type is completed at the end of the
        // translation unit as if it had one element. `int open[];` on its own is therefore an object
        // of one `int` - and until this was done, it reached the back end with no size at all and came
        // back as an internal error.
        for global in &mut self.globals {
            if global.defined && global.init.is_none() {
                if let Type::Array {
                    element,
                    count: None,
                } = *self.types.kind(global.ty)
                {
                    global.ty = self.types.array(element, Some(1));
                }
            }
        }
    }

    /// `_Alignas (...)`'s operand: a type name's alignment, or an integer constant expression.
    fn alignment_in_parentheses(&mut self) -> Option<u64> {
        // `parenthesised_type` reads both parentheses, and rewinds when what is inside is not a type
        // - which is exactly the "is this `_Alignas(16)` or `_Alignas(double)`" question.
        if let Some(ty) = self.parenthesised_type() {
            return self.types.align_of(ty);
        }
        if !self.eat("(") {
            self.error_at_current("expected `(` after `_Alignas`");
            return None;
        }
        let saved = std::mem::replace(&mut self.unevaluated, true);
        let value = self.conditional_expression();
        self.unevaluated = saved;
        let span = self.span();
        if !self.eat(")") {
            self.error_at_current("expected `)` after `_Alignas`");
        }
        let value = value?;
        match value.constant {
            // C11 6.7.5p4: "An alignment specification of zero has no effect" - so zero is a legal
            // operand meaning *none*, not a mistake.
            Some(0) => Some(0),
            // p3: an alignment must be a power of two. Three is not a small alignment, it is not an
            // alignment at all, and the frame's allocator would happily round to it.
            Some(constant) if constant > 0 && (constant as u64).is_power_of_two() => {
                Some(constant as u64)
            }
            _ => {
                self.error(
                    span,
                    "`_Alignas` takes a type or a constant expression whose value is zero or a power of two",
                );
                None
            }
        }
    }

    /// `_Alignof ( type-name )` - C11 6.5.3.1. Unlike `sizeof` there is no expression form, so the
    /// operand is a type name or a diagnostic.
    fn alignof_expression(&mut self, span: Span) -> Value {
        if self.preprocessed.standard == crate::options::CStandard::C99 {
            self.error(span, "`_Alignof` is C11: compile with `--std=c11`");
        }
        let size_type = self
            .types
            .fundamental("unsigned long long")
            .unwrap_or_else(|| self.types.error());
        let Some(ty) = self.parenthesised_type() else {
            self.error(span, "`_Alignof` takes a type in parentheses");
            return Value::temporary(size_type);
        };
        // A constant, but one with code behind it: a `return` of a value that only exists at compile
        // time has nothing to hand back, which is a `ud2` and an illegal instruction at run time.
        let align = self.types.align_of(ty).unwrap_or(1);
        self.constant_of(size_type, align as i64, span)
    }

    /// `_Static_assert ( constant-expression , string-literal ) ;` - C11 6.7.10.
    ///
    /// There is nothing to lower: the assertion is a question about the source, answered where it is
    /// asked. A false one is an error with the message the source gave, which is the point of the
    /// feature - `_Static_assert(sizeof(int) == 4, "32-bit int")` fails a build rather than a program.
    fn static_assertion(&mut self) {
        // Where the assertion is, for both diagnostics: a caret under `}` - which is where the
        // cursor is by the time the string has been read - points at nothing a reader can use.
        let assertion = self.span();
        if self.preprocessed.standard == crate::options::CStandard::C99 {
            let span = self.span();
            self.error(span, "`_Static_assert` is C11: compile with `--std=c11`");
        }
        self.bump(); // `_Static_assert`
        if !self.eat("(") {
            self.error_at_current("expected `(` after `_Static_assert`");
            self.skip_until(&[";"], true);
            return;
        }
        let saved = std::mem::replace(&mut self.unevaluated, true);
        let condition = self.conditional_expression();
        self.unevaluated = saved;
        let mut message = String::new();
        if self.eat(",")
            && self
                .peek()
                .is_some_and(|t| t.kind == crate::lex::TokenKind::Str)
        {
            let (_, _, bytes) = self.string_literal();
            message = String::from_utf8_lossy(&bytes[..bytes.len().saturating_sub(1)]).into_owned();
        }
        if !self.eat(")") {
            self.error_at_current("expected `)` to end `_Static_assert`");
        }
        if !self.eat(";") {
            self.error_at_current("expected `;` after `_Static_assert`");
        }
        let span = assertion;
        if condition.and_then(|value| value.constant) == Some(0) {
            let said = if message.is_empty() {
                String::new()
            } else {
                format!(": {message}")
            };
            self.error(span, format!("static assertion failed{said}"));
        }
    }

    /// `_Generic ( assignment-expression , type-name : assignment-expression , ... )` - C11 6.5.1.1.
    ///
    /// Nothing here runs, and only one choice is *made*: the controlling expression is read for its
    /// type, every arm is read for its type and its syntax, and then the winner is read again - this
    /// time for real. Two passes over a token span rather than one pass with a deferred decision,
    /// because the parser is a function over a token slice and a rewind is cheaper than a way to
    /// evaluate an expression later.
    ///
    /// The arms that lose are read with `unevaluated` set, which is the same switch `sizeof` uses:
    /// their values are never built, so an arm that would not compile still has to *parse*, which is
    /// what C asks for.
    fn generic_selection(&mut self) -> Option<Value> {
        if self.preprocessed.standard == crate::options::CStandard::C99 {
            let span = self.span();
            self.error(span, "`_Generic` is C11: compile with `--std=c11`");
        }
        self.bump(); // `_Generic`
        if !self.eat("(") {
            self.error_at_current("expected `(` after `_Generic`");
            return None;
        }
        let saved = std::mem::replace(&mut self.unevaluated, true);
        let control = self.assignment_expression();
        self.unevaluated = saved;
        // C11 6.5.1.1p2: the type is the value's after lvalue conversion, and an array or a function
        // decays - which is what `decay` is for.
        let wanted = control.map(|value| self.decay(value).ty);
        // First pass: which arm matches, and where its expression starts.
        let mut chosen: Option<usize> = None;
        let mut fallback: Option<usize> = None;
        while self.eat(",") {
            let start = self.at;
            let mut arm = None;
            let is_default = self.at_word("default");
            if is_default {
                self.bump();
            } else {
                arm = self.try_type_name();
            }
            if !self.eat(":") {
                self.error_at_current("expected `:` in `_Generic`");
                break;
            }
            let saved = std::mem::replace(&mut self.unevaluated, true);
            self.assignment_expression();
            self.unevaluated = saved;
            let matched = match (is_default, arm, wanted) {
                (true, ..) => {
                    fallback.get_or_insert(start);
                    false
                }
                (false, Some(arm), Some(wanted)) => self.types.identical(arm, wanted),
                _ => false,
            };
            if matched && chosen.is_none() {
                chosen = Some(start);
            }
        }
        if !self.eat(")") {
            self.error_at_current("expected `)` to end `_Generic`");
        }
        // Where the whole construct ends. The second pass reads one arm and comes back here, so the
        // arms after the winner are consumed once - by the pass that made no code.
        let after = self.at;
        let Some(start) = chosen.or(fallback) else {
            let span = self.span();
            self.error(
                span,
                "no `_Generic` arm matches the controlling expression and there is no `default`",
            );
            return None;
        };
        self.at = start;
        if self.at_word("default") {
            self.bump();
        } else {
            self.try_type_name();
        }
        self.eat(":");
        let value = self.assignment_expression();
        self.at = after;
        value
    }

    /// A declaration or function definition, ending at `;` or a body.
    fn external_declaration(&mut self) {
        let start = self.at;
        let Some(declared) = self.declaration() else {
            // Nothing declared. A declaration that declares nothing - `_Static_assert` - has still
            // read its own tokens and its own `;`, so skipping to the next `;` here would swallow the
            // definition after it: `_Static_assert(...); int main(void) {...}` lost `main` entirely
            // and wrote an empty object. Skip only when nothing was consumed at all.
            if self.at == start {
                self.skip_until(&[";"], true);
            }
            return;
        };
        let name = self.last_declared;
        if self.at_punct("{") {
            match self.types.kind(declared) {
                Type::Function { result, .. } => {
                    let result = *result;
                    // A prototype may be followed by the body (`int f(void); int f(void) { }`),
                    // and a body may be followed by a prototype, but a second body is a second
                    // definition (C99 6.9p5).
                    let already = name.is_some_and(|name| {
                        self.symbols.here(name).is_some_and(|entry| entry.defined)
                    });
                    if already {
                        let span = self.span();
                        let spelling = name.map(|name| self.spell_of(name).to_owned());
                        self.error(
                            span,
                            format!("redefinition of `{}`", spelling.unwrap_or_default()),
                        );
                        self.skip_balanced_block();
                        return;
                    }
                    if let Some(entry) = name.and_then(|name| self.symbols.here_mut(name)) {
                        entry.defined = true;
                    }
                    let external = self.last_linkage.is_external();
                    self.function_body(name, result, external);
                }
                _ => {
                    self.error_at_current("expected `;`");
                    self.skip_balanced_block();
                }
            }
            return;
        }
        if !self.eat(";") {
            self.error_at_current("expected `;` after a declaration");
            self.skip_until(&[";"], true);
        }
    }

    /// One declaration: specifiers, then one or more declarators.
    /// One declaration: specifiers, then one or more declarators. Returns the last type it
    /// declared, which is what tells a definition from a declaration.
    fn declaration(&mut self) -> Option<TypeId> {
        // An empty declaration: `;` where a declaration would stand. C99's grammar has no such
        // production, and every Windows SDK header is made of them - `DEFINE_ENUM_FLAG_OPERATORS(X);`
        // is a macro that expands to nothing for a C compiler, leaving its semicolon behind. `clang`
        // accepts it silently (and warns only under `-pedantic`), so refusing it refuses the SDK
        // over punctuation.
        if self.eat(";") {
            return None;
        }
        // C11 6.7.10: a declaration that declares nothing, and says what it checked. It belongs to
        // the declaration grammar rather than the statement grammar, which is why file scope and
        // block scope both reach it here.
        if self.at_word("_Static_assert") {
            self.static_assertion();
            return None;
        }
        let began = self.at;
        let specs = self.declaration_specifiers();
        // C11 6.7.1p3: at block scope `_Thread_local` shall be combined with `static` or `extern`;
        // 6.7.1p4: it shall not appear in a function declaration's specifiers. A permission the
        // standard gives nowhere would be a promise this compiler cannot keep.
        if specs.is_thread_local {
            let span = self.span();
            if self.in_body() && !specs.is_static && !specs.is_extern {
                self.error(
                    span,
                    "`_Thread_local` at block scope needs `static` or `extern`",
                );
            }
            if specs
                .ty
                .is_some_and(|ty| matches!(self.types.kind(ty), Type::Function { .. }))
            {
                self.error(span, "a function cannot be `_Thread_local`");
            }
        }
        let Some(base) = specs.ty else {
            let span = self.span();
            self.error(span, "expected a type specifier");
            self.skip_until(&[";"], true);
            return None;
        };
        let mut specs = Specifiers {
            ty: Some(base),
            ..specs
        };
        let mut last = None;
        loop {
            let base = specs.ty.unwrap_or_else(|| self.types.error());
            let Some(tree) = self.declarator(true) else {
                return last;
            };
            let name = tree.name;
            let (ty, vla) = self.apply_declarator(tree, base);
            // `int _wctime(...) __asm__("_wctime64");` - the SDK renaming a function, read here
            // because this is where a declarator ends and `=`, `,` or `;` may follow.
            self.asm_label(name, ty);
            // `int g __attribute__((aligned(16)))` writes the attribute after the declarator, and
            // `__attribute__((aligned(16))) int g` before the type: one question, "what alignment
            // did this declaration ask for", asked over the whole of it. An object aligned past
            // what it needed is not a wrong program, so a declaration with two declarators sharing
            // one attribute over-aligns the second rather than misaligning anything.
            specs.alignas = self.declaration_alignment(began, specs.alignas);
            let entry = Entry {
                variable_length: false,
                variable_length_size: None,
                ty,
                storage: None,
                defined: false,
                kind: if specs.is_typedef {
                    Kind::Typedef
                } else if matches!(self.types.kind(ty), Type::Function { .. }) {
                    Kind::Function
                } else {
                    Kind::Variable
                },
                span: self.span(),
            };
            if name.is_none() {
                // `enum E { A, B };` declares a tag and nothing else, which C allows.
                if specs.declares_tag && last.is_none() {
                    return Some(ty);
                }
                let span = self.span();
                self.error(span, "declaration does not declare anything");
                return last;
            }
            if let Some(name) = name {
                let span = self.span();
                // C allows a redeclaration with the same type and kind - `extern int x;` twice,
                // or two tentative definitions of one object - and makes everything else an
                // error. An enumerator is never redeclarable: two enumerations in one scope
                // may not share a name.
                let compatible = match self.symbols.here(name) {
                    None => true,
                    Some(previous) => {
                        !matches!(entry.kind, Kind::EnumConst(_))
                            && previous.kind == entry.kind
                            && self.types.same(previous.ty, entry.ty)
                    }
                };
                if compatible {
                    let _ = self.symbols.declare(name, entry);
                } else {
                    let spelling = self.spell_of(name).to_owned();
                    let previous = self
                        .symbols
                        .here(name)
                        .map(|previous| self.describe(previous.ty))
                        .unwrap_or_default();
                    self.error(
                        span,
                        format!("`{spelling}` is already declared in this scope as {previous}"),
                    );
                }
            }
            // What this declarator names, when it is a thing with storage: a function is a
            // signature and a `typedef` is a name for a type, and neither is an object. `void`
            // is not one either (C99 6.7.1p2), a record with no members has no size, and an
            // object past the host's budget is refused rather than truncated by whichever back
            // end asks next (a frame slot is a `u32`).
            let mut object =
                !specs.is_typedef && !matches!(self.types.kind(ty), Type::Function { .. });
            if object && matches!(self.types.kind(ty), Type::Void) {
                let span = self.span();
                self.error(span, "a variable cannot have type void");
                object = false;
            }
            if object && self.types.record_of(ty).is_some() && !self.types.is_complete(ty) {
                let span = self.span();
                let ty = self.describe(ty);
                self.error(span, format!("a variable of {ty} has no size"));
                object = false;
            }
            if object && !self.object_fits(ty) {
                object = false;
            }
            // A `static` local is not in the frame: it has static storage duration, one object
            // for the whole run, initialised once (C99 6.2.4p3), so it is handled with the
            // initialiser below as a global of its own.
            //
            // Neither is an `extern` one: C99 6.2.2p4 makes a block-scope `extern` declaration
            // *refer* to an object that has external linkage somewhere, not to make one of its own.
            // Without this, `int main(void) { extern int g; g = 7; }` gave `g` a slot in the frame
            // and wrote to that - a wrong answer no corpus program could show, because it needs the
            // global to be read from somewhere else afterwards.
            let static_local = object && self.in_body() && specs.is_static;
            let frame_local = object && self.in_body() && !static_local && !specs.is_extern;
            if frame_local {
                // A local variable is a slot in the function's frame (ADR-002 invariant 3).
                if let Some(name) = name {
                    let pointer = self.pointer_to(ty);
                    let span = self.span();
                    let alignas = specs.alignas;
                    if let Some(slot) =
                        self.emit(|builder| builder.aligned_slot(ty, pointer, alignas, span))
                    {
                        if let Some(entry) = self.symbols.here_mut(name) {
                            entry.storage = Some(slot);
                        }
                    }
                }
            }
            // A variable-length array's declaration asked for a number of bytes the program works
            // out: `count * sizeof(element)`, taken off the stack here and stored into the pointer
            // that is the array's name. `emit_stack_alloc` rounds it to sixteen.
            if let Some((count, element)) = vla {
                // Only an automatic object can be one: a variable-length array is bytes taken off
                // the stack at the declaration, so a `typedef`, a global and a `static` local have
                // nowhere to put it. C99 6.7.3.2p2 says the same about the type.
                if !frame_local {
                    let span = self.span();
                    self.error(
                        span,
                        "a variable-length array must be an automatic object: its size is read when the declaration is reached",
                    );
                } else {
                    let span = self.span();
                    match self.types.size_of(element) {
                        Some(size) => {
                            let wide = self
                                .types
                                .fundamental("unsigned long long")
                                .unwrap_or_else(|| self.types.error());
                            let per = self.constant_of(wide, size as i64, span);
                            let total = self.emit_binary(
                                cir::BinOp::Mul,
                                wide,
                                wide,
                                [count, per],
                                None,
                                span,
                            );
                            let address = self.emit_stack_alloc(total, span);
                            // `sizeof` needs the size in bytes, and it must be the one computed
                            // *here*: the count is an expression the program has just evaluated, and
                            // a `sizeof` in a loop may not evaluate it again (C99 6.5.3.4p2). So the
                            // number is stored in a slot of its own, beside the pointer.
                            let wide_pointer = self.pointer_to(wide);
                            let size_slot = self
                                .emit(|builder| builder.slot(wide, wide_pointer, span))
                                .filter(|_| total.cir.is_some());
                            if let (Some(slot), Some(size_value)) = (size_slot, total.cir) {
                                self.emit(|builder| builder.store(slot, size_value, span));
                            }
                            if let (Some(name), Some(_)) = (name, address.cir) {
                                // The name is a variable-length array from here on, which is what
                                // `sizeof` has to know and what the symbol table already scopes.
                                if let Some(entry) = self.symbols.here_mut(name) {
                                    entry.variable_length = true;
                                    entry.variable_length_size = size_slot;
                                }
                                if let Some(place) =
                                    self.symbols.here(name).and_then(|entry| entry.storage)
                                {
                                    let target = Value {
                                        ty,
                                        constant: None,
                                        lvalue: true,
                                        cir: Some(place),
                                        place: Some(place),
                                        bits: None,
                                        address: None,
                                    };
                                    self.assign(target, address, span);
                                }
                            }
                        }
                        None => {
                            let ty = self.describe(element);
                            self.error(
                                span,
                                format!("a variable-length array of {ty} has no size"),
                            );
                        }
                    }
                }
            }
            // At file scope an object is data, not a slot: ADR-004. A tentative definition
            // (`int x;`) is defined and zero-filled; an initialiser is the list walker's work
            // and is still refused, because a global with no bytes would be a wrong program.
            //
            // `extern` with no initialiser is the exception, and it is not a small one: `lua.h`
            // declares `extern const char lua_ident[];`, which has no size until the file that
            // defines it says one - so laying down bytes here invents an object of incomplete type
            // in every file that merely includes the header.
            if object && !self.in_body() && !self.at_punct("=") && !specs.is_extern {
                if let Some(name) = name {
                    let span = self.span();
                    self.define_global(
                        name,
                        ty,
                        None,
                        specs.alignas,
                        !specs.is_static,
                        specs.is_thread_local,
                        span,
                    );
                }
            }
            if static_local {
                if let Some(name) = name {
                    let span = self.span();
                    self.static_local(name, ty, specs.alignas, span);
                } else if self.eat("=") {
                    // No name to give it: read the initialiser for its own mistakes.
                    self.initializer(ty);
                }
            }
            // An initialiser: only a constant expression, for now.
            else if self.eat("=") {
                let span = self.span();
                if self.in_body() || !object {
                    // A local has a slot, so its initialiser becomes stores into the frame. An
                    // object this refused - `void`, too large, an aggregate in a body - has
                    // nowhere to put one; it is still read, so its own mistakes are reported.
                    let target = name
                        .and_then(|name| self.symbols.here(name))
                        .and_then(|entry| entry.storage)
                        .map(|place| (place, ty));
                    self.initialize_local(ty, target);
                } else {
                    let before = self.error_count();
                    match self.initializer_bytes(ty) {
                        Some((bytes, relocations)) => {
                            if let Some(name) = name {
                                self.define_global(
                                    name,
                                    ty,
                                    Some((bytes, relocations)),
                                    specs.alignas,
                                    !specs.is_static,
                                    specs.is_thread_local,
                                    span,
                                );
                            }
                        }
                        // C99 6.7.8p4 wants a constant expression here, and an object this
                        // cannot evaluate would be declared with no bytes at all - a wrong
                        // program, not a smaller one. One diagnostic, and not a second one on
                        // top of whatever the initialiser already said.
                        None => {
                            if self.error_count() == before {
                                self.error(
                                    span,
                                    "a file-scope initialiser this compiler cannot evaluate is not lowered yet",
                                );
                            }
                        }
                    }
                }
            }
            // C99 6.2.5p20: a variable's type has to be complete. This is asked after the
            // initialiser because C lets `char s[] = "abc"` take its size from the literal.
            if self.in_body()
                && !specs.is_typedef
                && matches!(self.types.kind(ty), Type::Array { count: None, .. })
            {
                let span = self.span();
                self.error(span, "an array with no size cannot be a variable");
            }
            last = Some(ty);
            self.last_declared = name;
            self.last_linkage = Linkage::of(&specs);
            if !self.eat(",") {
                return last;
            }
        }
    }

    /// A run of adjacent string literals: the type C gives them - `char[n]`, where `n` counts
    /// the bytes and the NUL - the name of the anonymous global that holds them, and the bytes
    /// themselves, NUL included. Adjacent literals concatenate (C99 6.4.5p5), so `"ab" "cd"`
    /// is one array of five.
    ///
    /// The bytes are decoded here and handed back, because the initialiser that needs them - a
    /// global's buffer or a local's stores - would otherwise be a second reader of the same
    /// literal, and the two would disagree about escapes and adjacent runs.
    fn string_literal(&mut self) -> (TypeId, String, Vec<u8>) {
        let span = self.span();
        // C99 6.4.5p5: adjacent literals concatenate, and they must be of the same *kind* - which
        // in C11 means the same prefix, not merely "both wide": `u"a" U"b"` is a program C has no
        // type for, and joining them as one encoding would be an answer nobody asked for.
        let prefix = self.peek().map_or(crate::literal::Prefix::None, |token| {
            crate::literal::prefix_of(self.spell(token))
        });
        let wide = prefix.width();
        let mut init: Vec<u8> = Vec::new();
        while let Some(token) = self.peek().copied() {
            if token.kind != crate::lex::TokenKind::Str {
                break;
            }
            let written = self.spell(&token).to_owned();
            let own = crate::literal::prefix_of(&written);
            if own != prefix {
                self.error(
                    span,
                    format!(
                        "a `{}` string literal and a `{}` one cannot be joined into one",
                        prefix.spelling(),
                        own.spelling()
                    ),
                );
            }
            self.bump();
            let decoded = crate::literal::bytes_of(&written);
            match decoded {
                Some(mut bytes) => init.append(&mut bytes),
                None => {
                    // An escape this compiler does not read is reported and filled with
                    // zeroes: the object keeps the size the source spelled, rather than
                    // silently becoming one byte for a two-byte array.
                    self.error(
                        span,
                        "this string literal has an escape this compiler does not read",
                    );
                    let units = string_length(&written) as usize;
                    init.resize(init.len() + units * wide, 0);
                }
            }
        }
        // The terminator is one code unit, and the unit's width is the prefix's.
        init.resize(init.len() + wide, 0);
        let element = match prefix {
            crate::literal::Prefix::Wide | crate::literal::Prefix::Utf16 => self
                .types
                .fundamental("unsigned short")
                .unwrap_or_else(|| self.types.error()),
            crate::literal::Prefix::Utf32 => self
                .types
                .fundamental("unsigned int")
                .unwrap_or_else(|| self.types.error()),
            crate::literal::Prefix::None | crate::literal::Prefix::Utf8 => self
                .types
                .fundamental("char")
                .unwrap_or_else(|| self.types.error()),
        };
        // The array's length is in *elements*: the bytes are the characters times the width.
        let ty = self
            .types
            .array(element, Some(init.len() as u64 / wide as u64));
        let name = self.literal_object(ty, &init, span);
        (ty, name, init)
    }

    /// The object a string literal is: an anonymous global holding its bytes and their NUL, named
    /// with a `.` prefix so it can never collide with a C identifier (ADR-004). Two things want one -
    /// a literal in the source, and `__func__`, which C99 6.4.2.2 says is "as if" one was declared -
    /// and this is the one place that writes it.
    fn literal_object(&mut self, ty: TypeId, init: &[u8], span: Span) -> String {
        let name = format!(".str{}", self.literals);
        self.literals += 1;
        self.globals.push(cir::Global {
            name: name.clone(),
            ty,
            defined: true,
            init: Some(init.to_vec()),
            relocations: Vec::new(),
            alignas: 0,
            external: false,
            thread: false,
            span,
        });
        name
    }

    /// Record a file-scope object's definition, completing one already there rather than adding
    /// a second.
    ///
    /// C99 6.9p5 gives an identifier one definition: a later declaration may *complete* it -
    /// `int x;` then `int x = 1;` is one object, and two tentative definitions are one object
    /// too - but a second initialiser is a second definition, which C does not allow. Both
    /// halves matter: the back end refuses a duplicate symbol as an internal error, so a legal
    /// program must never take the second path, and an illegal one must not reach the back end
    /// to be told so in Cranelift's words.
    ///
    /// `static` is not distinguished yet: the storage class lives in `Specifiers`, and patch 4
    /// (which makes visibility matter) is the place to read it. Until then every object claims
    /// to be external, which is wrong for a `static` and harmless only because nothing reads the
    /// flag.
    #[allow(clippy::too_many_arguments)] // every one of them is a fact the caller already has
    fn define_global(
        &mut self,
        name: Symbol,
        ty: TypeId,
        init: Option<(Vec<u8>, Vec<cir::GlobalRelocation>)>,
        alignas: u64,
        external: bool,
        thread: bool,
        span: Span,
    ) {
        // A tentative definition defines nothing yet: `int x;` may be followed by `int x = 1;`.
        let already = self.symbols.here(name).is_some_and(|entry| entry.defined);
        if init.is_some() {
            if already {
                let spelling = self.spell_of(name).to_owned();
                self.error(span, format!("redefinition of `{spelling}`"));
                return;
            }
            if let Some(entry) = self.symbols.here_mut(name) {
                entry.defined = true;
            }
        }
        match (self.globals_by_name.get(&name).copied(), &init) {
            (Some(index), Some(_)) => {
                // The tentative definition becomes the real one: it is what gives the object its
                // bytes, and its size when the initialiser is what completed the type.
                let (bytes, relocations) = init.expect("the arm matched Some");
                self.globals[index].ty = ty;
                self.globals[index].init = Some(bytes);
                self.globals[index].relocations = relocations;
            }
            (Some(_), None) => {
                // `int x;` after anything: a tentative definition says nothing new.
            }
            (None, _) => {
                let (init, relocations) = match init {
                    Some((bytes, relocations)) => (Some(bytes), relocations),
                    None => (None, Vec::new()),
                };
                self.globals_by_name.insert(name, self.globals.len());
                self.globals.push(cir::Global {
                    name: self.spell_of(name).to_owned(),
                    ty,
                    defined: true,
                    init,
                    relocations,
                    alignas,
                    external,
                    thread,
                    span,
                });
            }
        }
    }

    /// A `static` local: one object for the whole run, in the data section.
    ///
    /// C99 6.2.4p3 gives it static storage duration and 6.7.8p10 makes its initialiser a constant
    /// one, so it is a global in everything but name - and its name is `<function>.<variable>`,
    /// which no C identifier can spell, so two functions may each have their own `static int n`
    /// (6.2.2p3). The local name means its *address* inside the body, which is what a frame slot
    /// would have been too, so nothing else in the parser changes.
    fn static_local(&mut self, name: Symbol, ty: TypeId, alignas: u64, span: Span) {
        let mangled = match self.current_function {
            Some(function) => format!("{}.{}", self.spell_of(function), self.spell_of(name)),
            None => self.spell_of(name).to_owned(),
        };
        // The initialiser, or zeroes: a static object with none holds zero (C99 6.7.8p10), and
        // the data section must carry those bytes rather than the frame's leftovers. The object
        // gets its bytes either way - a use of the name afterwards must not become a second,
        // internal complaint about a symbol the compiler never made.
        let initialised = self.eat("=");
        let written = if initialised {
            let before = self.error_count();
            let written = self.initializer_bytes(ty);
            // **An initialiser that could not be written is not the same as no initialiser.** This
            // fell back to zeros silently, and "has already said so" was not true: `(void*)7` in a
            // static table produced no bytes and no diagnostic, so the table was all NULL in the
            // image and the fault was a mile from the cause. C99 6.7.8p4 wants a constant
            // expression, and an object this cannot evaluate is a wrong program, not a smaller one.
            if written.is_none() && self.error_count() == before {
                self.error(
                    span,
                    "a static local\'s initialiser must be a constant expression",
                );
            }
            written
        } else {
            None
        };
        let Some((init, relocations)) = written.or_else(|| {
            self.types
                .size_of(ty)
                .map(|size| (vec![0u8; size as usize], Vec::new()))
        }) else {
            return;
        };
        self.globals.push(cir::Global {
            name: mangled.clone(),
            ty,
            defined: true,
            init: Some(init),
            relocations,
            alignas,
            external: false,
            thread: false,
            span,
        });
        let pointer = self.pointer_to(ty);
        let address = self.emit(|builder| builder.global_address(mangled.clone(), pointer, span));
        if let (Some(address), Some(entry)) = (address, self.symbols.here_mut(name)) {
            entry.storage = Some(address);
        }
    }

    /// The bytes a file-scope initialiser gives an object of type `ty`, or `None` when a value
    /// in it is not something this can write.
    ///
    /// The traversal is `initializer`'s - brace elision, designators, and every diagnostic are
    /// unchanged - with `bytes` set so the leaves land at their offsets. There is no second
    /// reader: that was tried twice and failed both times (ADR-004).
    fn initializer_bytes(&mut self, ty: TypeId) -> Option<(Vec<u8>, Vec<cir::GlobalRelocation>)> {
        // `char s[] = "hi"` has no size until the initialiser gives it one, so the buffer grows
        // as the traversal learns; it is cut to the type's size at the end.
        let size = self.types.size_of(ty).map(|size| size as usize);
        self.bytes = Some(vec![0u8; size.unwrap_or(0)]);
        self.offset = 0;
        self.init_grows = size.is_none();
        self.initializer(ty);
        self.init_grows = false;
        let mut bytes = self.bytes.take()?;
        let relocations = std::mem::take(&mut self.init_relocations);
        let size = self.types.size_of(ty).map(|size| size as usize)?;
        bytes.resize(size, 0);
        Some((bytes, relocations))
    }

    /// How a type is written in a diagnostic, with its tags: `pointer to struct Point`. The
    /// parser has the names and the type arena does not, so this is where they meet.
    fn describe(&self, ty: TypeId) -> String {
        self.types.describe_with(ty, self.preprocessed.names())
    }

    /// Whether an object of this type fits the host's budget, and the one diagnostic when it
    /// does not. A type with no size (an incomplete array) is not this check's business: it has
    /// a diagnostic of its own.
    fn object_fits(&mut self, ty: TypeId) -> bool {
        let Some(size) = self.types.size_of(ty) else {
            return true;
        };
        let limit = self.budgets.max_object_bytes;
        if size <= limit {
            return true;
        }
        let span = self.span();
        self.error(
            span,
            format!("an object of {size} bytes is larger than this compilation's limit of {limit}"),
        );
        false
    }

    /// How a name from the symbol table is spelled.
    /// The type of a Microsoft intrinsic this compiler implements, when `name` is one. Built from
    /// the spellings in [`INTRINSICS`], which are the headers' own, so the table and the declared
    /// list cannot name different types by accident.
    fn intrinsic(&mut self, name: Symbol) -> Option<TypeId> {
        let spelling = self.spell_of(name);
        let (_, result, params) = INTRINSICS
            .iter()
            .find(|(intrinsic, _, _)| *intrinsic == spelling)?;
        let result = self.spelled_type(result)?;
        let mut parameters = Vec::with_capacity(params.len());
        for param in *params {
            parameters.push(self.spelled_type(param)?);
        }
        Some(self.types.function(result, parameters, false))
    }

    /// A type by the name a header writes it, with a trailing ` *` for a pointer: the vocabulary the
    /// intrinsic table is written in.
    fn spelled_type(&mut self, spelling: &str) -> Option<TypeId> {
        // `const` belongs to the type it qualifies, so it is peeled from the pointee and applied
        // there: `const unsigned char *` is a pointer to a const character, which is what the
        // header says and what a caller passing a string literal needs.
        let (spelling, qualified) = match spelling.strip_prefix("const ") {
            Some(rest) => (rest, true),
            None => (spelling, false),
        };
        let mut ty = match spelling.strip_suffix(" *") {
            Some(inner) => {
                let base = match inner.strip_prefix("const ") {
                    Some(inner) => self.types.constant(self.types.fundamental(inner)?),
                    None => self.types.fundamental(inner)?,
                };
                self.pointer_to(base)
            }
            None => self.types.fundamental(spelling)?,
        };
        if qualified {
            ty = self.types.constant(ty);
        }
        Some(ty)
    }

    fn spell_of(&self, name: Symbol) -> &str {
        self.preprocessed.spell(name)
    }

    /// The name a symbol is *linked* by. Almost always the one the program wrote; `__asm__("...")`
    /// after a declarator makes it something else, and every place a name becomes a symbol - a call,
    /// a function used as an address, a relocation in an object's bytes - asks here, so there is one
    /// answer.
    fn link_name(&self, name: Symbol) -> String {
        match self.links.get(&name) {
            Some(renamed) => renamed.clone(),
            None => self.spell_of(name).to_owned(),
        }
    }

    /// `__asm__("name")` after a declarator, which is a *link* name: the name this compiler must
    /// emit and the linker must find, whatever the program calls it. C has no such thing; GCC and
    /// MSVC both spell it this way, and the Windows headers depend on it.
    ///
    /// Only a function may be renamed. GCC allows it on objects too, and nothing in the SDK does -
    /// so an object is refused rather than half-supported.
    fn asm_label(&mut self, name: Option<Symbol>, ty: TypeId) {
        let Some(spelling) = self.peek().map(|token| self.spell(token).to_owned()) else {
            return;
        };
        if !matches!(spelling.as_str(), "asm" | "__asm" | "__asm__") {
            return;
        }
        let span = self.span();
        self.bump();
        if !self.eat("(") {
            self.error(span, "`__asm__` needs a parenthesised name");
            return;
        }
        let Some(literal) = self.peek().map(|token| self.spell(token).to_owned()) else {
            return;
        };
        if !self
            .peek()
            .is_some_and(|token| token.kind == crate::lex::TokenKind::Str)
        {
            self.error(
                span,
                "`__asm__` takes one string literal: the name to link by",
            );
            return;
        }
        self.bump();
        if !self.eat(")") {
            self.error_at_current("expected `)` after the name in `__asm__`");
        }
        let Some(bytes) = crate::literal::bytes_of(&literal) else {
            self.error(
                span,
                "this `__asm__` name has an escape this compiler does not read",
            );
            return;
        };
        let linked = String::from_utf8_lossy(&bytes)
            .trim_end_matches('\0')
            .to_owned();
        // An empty name is not a name: passing it on would reach the linker as a symbol called
        // nothing, which is a stranger diagnostic than this one.
        if linked.is_empty() {
            self.error(span, "`__asm__(\"\")` names nothing to link by");
            return;
        }
        let Some(name) = name else {
            return;
        };
        if !matches!(self.types.kind(ty), Type::Function { .. }) {
            let written = self.spell_of(name).to_owned();
            self.error(
                span,
                format!("`__asm__(\"{linked}\")` renames `{written}`, which is not a function: this compiler links an object by the name it is declared with"),
            );
            return;
        }
        if linked != self.spell_of(name) {
            self.links.insert(name, linked);
        }
    }

    /// Storage class, qualifiers and type specifiers, in any order.
    fn declaration_specifiers(&mut self) -> Specifiers {
        let mut specs = Specifiers::default();
        let mut spelled: Vec<String> = Vec::new();
        while let Some(token) = self.peek().copied() {
            let word = self.spell(&token).to_owned();
            match word.as_str() {
                // `__thread` is GCC's and clang's spelling of `_Thread_local`, and it is the one
                // header libraries reach for on a compiler they cannot identify: mimalloc's
                // `mi_decl_thread` is `__declspec(thread)` for MSVC, `__thread` for GCC, and
                // `__thread` again in the branch for everybody else - which is this one.
                "typedef" | "extern" | "static" | "auto" | "register" | "_Thread_local"
                | "__thread" => {
                    if word == "typedef" {
                        specs.is_typedef = true;
                    }
                    if word == "static" {
                        specs.is_static = true;
                    }
                    if word == "extern" {
                        specs.is_extern = true;
                    }
                    if word == "_Thread_local" || word == "__thread" {
                        specs.is_thread_local = true;
                    }
                    self.at += 1; // a storage class is not part of the type
                }
                "const" => {
                    specs.is_const = true;
                    self.at += 1;
                }
                // `volatile` says the value may change behind the program's back, which this
                // compiler has no need to model: it does not reorder or elide a memory access, so
                // every access is the access the source wrote. `restrict` licenses an optimisation
                // it does not perform. Both are consumed and dropped, and that is stated rather
                // than silently true.
                // `_Noreturn` (C11 6.7.4p4) promises a function does not return. Nothing here
                // needs the promise: a call is emitted where the program wrote it, and control
                // falling off the end of such a function is the programmer's bug rather than the
                // compiler's. Consumed and dropped, like the two above.
                "volatile" | "restrict" | "_Noreturn" => {
                    self.at += 1;
                }
                // `inline` is not dropped: it decides *linkage* (C99 6.7.4p6), so it is a fact
                // about the declaration rather than a hint.
                "inline" => {
                    specs.is_inline = true;
                    self.at += 1;
                }
                // Microsoft's storage-class spelling, which a Windows header emits behind `_WIN32`:
                // `__declspec(dllexport) int f(void);`. Its argument list is skipped and the
                // attribute carries no meaning here - a `dllexport` in a program this compiler
                // links statically exports nothing.
                "__declspec" => {
                    self.at += 1;
                    // `(` is consumed first: `skip_until` stops at a closer at depth zero, and with
                    // the `(` still standing the `)` is at depth one - it stepped *past* the
                    // attribute and ate the rest of the declaration.
                    if self.eat("(") {
                        self.skip_until(&[")"], true);
                    }
                }
                // The calling conventions. On x86-64 there is one, so these are spelling and nothing
                // else - which is what Microsoft's own documentation of `__stdcall` for this target
                // says.
                "__cdecl" | "__stdcall" | "__fastcall" | "__thiscall" | "__vectorcall" => {
                    self.at += 1;
                }
                // C11 6.7.5: `_Alignas(type-name)` or `_Alignas(constant-expression)`. It raises the
                // object's alignment and may not lower it, so a value below the type's own is a
                // diagnostic rather than a smaller object.
                //
                // `__builtin_alignas` is the same specifier without the standard gate, because a
                // *header* cannot ask the program to change its standard: `setjmp.h` needs 16
                // bytes of alignment for the ABI's jump buffer, and a C99 program is entitled to
                // include it.
                "_Alignas" | "__builtin_alignas" => {
                    if word == "_Alignas"
                        && self.preprocessed.standard == crate::options::CStandard::C99
                    {
                        let span = self.span();
                        self.error(span, "`_Alignas` is C11: compile with `--std=c11`");
                    }
                    self.at += 1;
                    if let Some(align) = self.alignment_in_parentheses() {
                        if align == 0 {
                            // No effect, exactly as C11 6.7.5p4 says.
                        } else if align > FUNDAMENTAL_ALIGNMENT && self.in_body() {
                            // **The limit is the frame's, not the data section's.** An *automatic*
                            // object is placed by the prologue, which aligns the frame to sixteen -
                            // the ABI's stack alignment - so asking for more there is a promise this
                            // compiler cannot keep. An object with static storage duration is placed
                            // by the object writer, where sixty-four is an offset like any other.
                            // `xxhash` aligns its accumulators to 64, at file scope, and was refused.
                            let span = self.span();
                            self.error(
                                span,
                                format!(
                                    "`_Alignas({align})` on an automatic object is an extended alignment, and this compiler's frame is {FUNDAMENTAL_ALIGNMENT}-aligned - what the ABI guarantees. Give the object static storage duration"
                                ),
                            );
                        } else if align > STATIC_ALIGNMENT_LIMIT {
                            let span = self.span();
                            self.error(
                                span,
                                format!(
                                    "`_Alignas({align})` is past this compiler's limit of {STATIC_ALIGNMENT_LIMIT} for an object in the data section"
                                ),
                            );
                        } else {
                            specs.alignas = specs.alignas.max(align);
                        }
                    }
                }
                "void" | "_Bool" | "char" | "short" | "int" | "long" | "float" | "double"
                | "signed" | "unsigned" | "_Complex" => {
                    spelled.push(word);
                    specs.has_type_specifier = true;
                    self.at += 1;
                }
                // Microsoft's fixed-width spellings, which the Windows SDK writes a thousand times:
                // `__int64` is `long long`, `__int32` is `int`, and `unsigned` goes in front of
                // them (`unsigned __int64`) as it does in front of the standard ones. They are not
                // C99 and are consumed as what they mean, which is all a program can observe.
                "__int8" | "__int16" | "__int32" | "__int64" => {
                    spelled.push(word);
                    specs.has_type_specifier = true;
                    self.at += 1;
                }
                // GCC's "this is an extension, do not warn about it" marker, which mingw's headers
                // put before declarations a strict C99 compiler would refuse. Nothing follows it
                // that is not already this compiler's business.
                "_" | "__extension__" if word == "__extension__" => {
                    self.at += 1;
                }
                // The *other* compilers' spellings of qualifiers and of `inline`, which a header
                // written for them puts on a declaration this compiler reads the same way. They
                // mean nothing here that the standard word does not: `__inline` is `inline`,
                // `__restrict` is `restrict`, and `__unaligned`/`__ptr32`/`__ptr64`/`__w64` are
                // promises about a *placement* that x86-64 does not need - `UNALIGNED` in `winnt.h`
                // is this word on the target whose pointers are already aligned.
                "__inline" | "__inline__" | "_inline" | "__forceinline" | "_forceinline" => {
                    specs.is_inline = true;
                    self.at += 1;
                }
                "__restrict" | "__restrict__" | "__unaligned" | "__ptr32" | "__ptr64" | "__w64"
                | "__sptr" | "__uptr" => {
                    self.at += 1;
                }

                // C99 6.2.5p11's three imaginary types, refused by name and here, where the word is:
                // C11 removed them, `clang` says "imaginary types are not supported" in every
                // dialect, and a program that asked for one wants a `_Complex` value whose real part
                // is zero. The type is answered as the error type so the rest of the declaration
                // says nothing further, and `finish_specifiers` returns at once for a type already
                // set - one diagnostic, pointing at `_Imaginary` and not at whatever follows it.
                "_Imaginary" => {
                    let span = self.span();
                    self.error(
                        span,
                        "imaginary types are not supported: C11 removed them, and a `_Complex` value with a zero real part is what such a program means",
                    );
                    specs.has_type_specifier = true;
                    specs.ty = Some(self.types.error());
                    self.at += 1;
                }
                "enum" => {
                    if specs.ty.is_some() {
                        break;
                    }
                    spelled.push(word);
                    self.at += 1;
                    specs.ty = Some(self.enum_specifier());
                    specs.declares_tag = true;
                    return self.finish_specifiers(specs, spelled);
                }
                // C11 6.7.2p1: `_Atomic` is a type specifier when a parenthesised type follows it
                // and a qualifier otherwise. Both mean the type C would give it; the atomicity of
                // the *access* is a promise this compiler does not make, and the matrix says so.
                "_Atomic" if specs.ty.is_none() && !specs.has_type_specifier => {
                    if self.preprocessed.standard == crate::options::CStandard::C99 {
                        let span = self.span();
                        self.error(span, "`_Atomic` is C11: compile with `--std=c11`");
                    }
                    self.at += 1;
                    if let Some(ty) = self.parenthesised_type() {
                        specs.ty = Some(ty);
                        specs.has_type_specifier = true;
                        return self.finish_specifiers(specs, spelled);
                    }
                    // A qualifier: keep reading the specifiers.
                    continue;
                }
                "struct" | "union" => {
                    if specs.ty.is_some() {
                        break;
                    }
                    let kind = if word == "struct" {
                        RecordKind::Struct
                    } else {
                        RecordKind::Union
                    };
                    self.at += 1;
                    specs.ty = Some(self.record_specifier(kind, token.span));
                    specs.declares_tag = true;
                    specs.has_type_specifier = true;
                    return self.finish_specifiers(specs, spelled);
                }
                _ => {
                    if specs.ty.is_none() && !specs.has_type_specifier {
                        if let Some(ty) = self.symbols.typedef(token.text) {
                            spelled.push(word);
                            self.at += 1;
                            specs.ty = Some(ty);
                            continue;
                        }
                    }
                    break;
                }
            }
        }
        self.finish_specifiers(specs, spelled)
    }

    /// Turn the words of a type specifier into a type, or the error type with a diagnostic.
    ///
    /// C's type specifiers are a set, not a sequence: `unsigned long long int` and
    /// `long int unsigned long` mean the same thing. So the words are *counted*, not read in
    /// order, and the counts decide the type.
    fn finish_specifiers(&mut self, mut specs: Specifiers, spelled: Vec<String>) -> Specifiers {
        if specs.ty.is_some() {
            return specs;
        }
        let words: Vec<&str> = spelled
            .iter()
            .map(String::as_str)
            .filter(|word| {
                !matches!(
                    *word,
                    "typedef" | "extern" | "static" | "auto" | "register" | "_Thread_local"
                )
            })
            .collect();
        let count = |word: &str| words.iter().filter(|seen| **seen == word).count();
        let signed = count("unsigned") == 0;

        if count("void") > 0 {
            specs.ty = Some(self.types.fundamental("void").unwrap());
            return specs;
        }
        if count("_Bool") > 0 {
            specs.ty = Some(self.types.fundamental("_Bool").unwrap());
            return specs;
        }
        // C99 6.2.5p11: `_Complex` on its own is `double _Complex`, and the three complex types
        // are the three floating ones. Asked before `float` and `double` below, because
        // `float _Complex` spells both and the complex answer is the one that matters.
        if count("_Complex") > 0 {
            let element = if count("float") > 0 {
                self.types.fundamental("float").unwrap()
            } else {
                self.types.fundamental("double").unwrap()
            };
            specs.ty = Some(self.types.complex(element));
            if specs.is_const {
                specs.ty = specs.ty.map(|ty| self.types.constant(ty));
            }
            return specs;
        }
        if count("float") > 0 {
            specs.ty = Some(self.types.fundamental("float").unwrap());
            return specs;
        }
        if count("double") > 0 {
            // `long double` is 64 bits in this ABI, and says so in its spelling.
            let name = if count("long") > 0 {
                "long double"
            } else {
                "double"
            };
            specs.ty = Some(self.types.fundamental(name).unwrap());
            return specs;
        }
        if words.is_empty() {
            // No type specifier at all. Whether that is an error depends on who asked: a
            // declaration must have one, a `sizeof (` may be the start of an expression.
            return specs;
        }
        // Microsoft's fixed-width spellings, before the standard ones: `unsigned __int64` is
        // `unsigned long long` and `__int32` is `int`, and a `short` or `char` in the same
        // declaration is not a spelling C99 has, so the table below is not asked about them.
        for (word, name) in [
            ("__int8", "signed char"),
            ("__int16", "short"),
            ("__int32", "int"),
            ("__int64", "long long"),
        ] {
            if count(word) > 0 {
                let any = if signed { name } else { "" };
                let unsigned_name = match name {
                    "signed char" => "unsigned char",
                    "short" => "unsigned short",
                    "int" => "unsigned int",
                    _ => "unsigned long long",
                };
                specs.ty =
                    self.types
                        .fundamental(if any.is_empty() { unsigned_name } else { name });
                if specs.is_const {
                    specs.ty = specs.ty.map(|ty| self.types.constant(ty));
                }
                return specs;
            }
        }
        // The *spelling* picks the type, not the size. `int` and `long` are both thirty-two bits on
        // this target (LLP64), and they are two types (C99 6.2.5p4) - which `_Generic` asks about
        // directly, and which a table keyed on `(signed, bits)` cannot answer: it gave `long` the
        // `int` type object, so `_Generic(l, int: 1, default: 9)` answered 1 for a `long`.
        let name = match (
            count("char") > 0,
            count("short") > 0,
            count("long") > 0,
            count("long") >= 2,
        ) {
            (true, ..) if count("signed") > 0 => "signed char",
            (true, ..) if !signed => "unsigned char",
            (true, ..) => "char",
            (_, true, ..) if signed => "short",
            (_, true, ..) => "unsigned short",
            // `signed long long` and `signed long` are their own types; `signed int` is `int`.
            (_, _, _, true) if signed => "long long",
            (_, _, _, true) => "unsigned long long",
            (_, _, true, _) if signed => "long",
            (_, _, true, _) => "unsigned long",
            _ if signed => "int",
            _ => "unsigned int",
        };
        specs.ty = self.types.fundamental(name);
        // The qualifier belongs to the declared type, once, here: an object, a member, a typedef and
        // a parameter all read `specs.ty` and all get the `const` C gave them.
        if specs.is_const {
            if let Some(ty) = specs.ty {
                specs.ty = Some(self.types.constant(ty));
            }
        }
        specs
    }

    /// `enum E { A, B = 3 }` and `enum E` and `enum { ... }`.
    fn enum_specifier(&mut self) -> TypeId {
        let int = self.types.fundamental("int").unwrap();
        if self.peek().is_some_and(|token| !self.is_punct(token, "{")) {
            self.bump(); // the tag, which nothing distinguishes yet
        }
        if !self.eat("{") {
            return int; // `enum E` with no body: an incomplete enum is an int here
        }
        let mut next_value: i64 = 0;
        loop {
            if self.eat("}") {
                return int;
            }
            let Some(token) = self.peek().copied() else {
                self.error(self.span(), "expected `}` to end the enumerator list");
                return int;
            };
            if token.kind != crate::lex::TokenKind::Ident {
                self.error(token.span, "expected an enumerator name");
                self.skip_until(&[";"], true);
                return int;
            }
            self.bump();
            let value = if self.eat("=") {
                self.constant_expression().unwrap_or(next_value)
            } else {
                next_value
            };
            next_value = value.wrapping_add(1);
            let entry = Entry {
                variable_length: false,
                variable_length_size: None,
                ty: int,
                kind: Kind::EnumConst(value),
                span: token.span,
                storage: None,
                defined: false,
            };
            if self.symbols.declare(token.text, entry).is_err() {
                let name = self.spell(&token).to_owned();
                self.error(
                    token.span,
                    format!("`{name}` is already declared in this scope"),
                );
            }
            if !self.eat(",") {
                self.eat("}");
                return int;
            }
        }
    }

    /// `struct S`, `struct S { ... }`, `struct { ... }` - and the same for `union`.
    ///
    /// A tag with no body is a forward declaration, which is how two structs can point at each
    /// other; a tag that already has a body is completed rather than redeclared.
    fn record_specifier(&mut self, kind: RecordKind, span: Span) -> TypeId {
        // Where the specifier starts, which is where an attribute can be: `struct __attribute__
        // ((packed)) S { ... }` puts one before the tag and `struct S { ... } __attribute__
        // ((packed))` puts one after the body. Both are "in this record's range".
        let began = self.at;
        let tag = if self
            .peek()
            .is_some_and(|token| token.kind == crate::lex::TokenKind::Ident)
        {
            Some(self.bump().expect("peeked").text)
        } else {
            None
        };
        let has_body = self.at_punct("{");
        if !has_body {
            let Some(tag) = tag else {
                self.error(span, "expected a tag or a body after `struct`");
                return self.types.error();
            };
            if let Some(ty) = self.symbols.tag(tag) {
                return ty;
            }
            let (ty, _) = self.types.record(kind, Some(tag));
            self.symbols.declare_tag(tag, ty);
            return ty;
        }

        // `struct S { ... }` completes `S` if this scope already has an incomplete `S`, and
        // redeclares it otherwise - which for a complete one is the error.
        let existing = tag.and_then(|tag| self.symbols.tag_here(tag));
        if let Some(ty) = existing {
            if let Some(record) = self.types.record_of(ty) {
                if self.types.record_complete(record) {
                    self.error(span, "this struct or union is already complete");
                    self.member_list();
                    return ty;
                }
                let opened = self.at;
                let members = self.member_list();
                let layout = self.record_layout(began, opened, span);
                self.types.complete_record(record, members, layout);
                self.check_member_names(record);
                return ty;
            }
        }
        let (ty, record) = self.types.record(kind, tag);
        if let Some(tag) = tag {
            self.symbols.declare_tag(tag, ty);
        }
        let opened = self.at;
        let members = self.member_list();
        let layout = self.record_layout(began, opened, span);
        self.types.complete_record(record, members, layout);
        self.check_member_names(record);
        ty
    }

    /// Everything a record's declaration said about where its members go.
    ///
    /// `began` is where its specifier started and `opened` is its `{`; the attributes that belong to
    /// it are the ones written in that range and immediately after the body, which is where
    /// `struct S { ... } __attribute__((packed))` puts one.
    ///
    /// A record gets **one** layout, taken where its body ends. A `#pragma pack` *inside* the body
    /// would give different members different limits - which MSVC does accept - and a compiler laying
    /// the record out in one pass cannot express it, so it is refused by name rather than quietly
    /// applied to half the members.
    fn record_layout(&mut self, began: usize, opened: usize, span: Span) -> crate::types::Layout {
        let pack = self.preprocessed.pack_at(self.at);
        if pack != self.preprocessed.pack_at(opened) {
            self.error(
                span,
                "a `#pragma pack` inside a struct or union body is not supported: a record is laid out with the limit in force where it ends",
            );
        }
        let mut layout = crate::types::Layout::new(pack);
        // The record's own attributes are the ones written *outside* its body: before the `{` and
        // immediately after the `}`. A member's attribute is inside and belongs to that member -
        // `struct { char c; int i __attribute__((packed)); short s; }` is 8 bytes with `i` alone
        // packed, and taking the attribute for the record's made it 7.
        let mut attributes = self
            .preprocessed
            .layout_attributes(began, opened.saturating_sub(1));
        attributes.extend(self.preprocessed.layout_attributes(self.at, self.at));
        for attribute in attributes {
            match attribute {
                crate::types::LayoutAttribute::Packed => layout.packed = true,
                crate::types::LayoutAttribute::Aligned(alignment) => {
                    layout.aligned = Some(layout.aligned.unwrap_or(1).max(alignment));
                }
            }
        }
        layout
    }

    /// C99 6.7.2.1p1: no two members of one struct or union may have the same name - and C11
    /// 6.7.2.1p13 puts the members of an anonymous record in that same scope, so the check reads
    /// the *completed* list, where the splice has already happened. Asked here rather than as each
    /// member is parsed, because until the record is complete the list is not the one a lookup,
    /// an initialiser or this check will read: `struct s { int a; union { int a; }; };` names `a`
    /// twice and only the second copy is inside another record.
    fn check_member_names(&mut self, record: RecordId) {
        let mut seen: std::collections::HashSet<Symbol> = std::collections::HashSet::new();
        let mut twice = Vec::new();
        for member in self.types.members(record) {
            let Some(name) = member.name else { continue };
            if !seen.insert(name) {
                twice.push(name);
            }
        }
        for name in twice {
            let spelling = self.spell_of(name).to_owned();
            let span = self.span();
            self.error(
                span,
                format!("`{spelling}` is a member of this struct twice"),
            );
        }
    }

    /// The members of a struct or union, between its braces.
    fn member_list(&mut self) -> Vec<Member> {
        let mut members = Vec::new();
        if !self.eat("{") {
            return members;
        }
        while !self.at_punct("}") && !self.at_eof() {
            // The same empty declaration, in a member list: `int a; MACRO_EXPANDING_TO_NOTHING; int b;`
            // leaves a `;` with no member in front of it, and `winnt.h` writes dozens.
            if self.eat(";") {
                continue;
            }
            // Where this member's declaration starts, so its own attributes can be found by the
            // range it covers - before the type (`__attribute__((aligned(8))) int i`) as well as
            // after the declarator (`int i __attribute__((packed))`).
            let member_from = self.at;
            let specs = self.declaration_specifiers();
            // C11 6.7.3p2 asks for a *variable* declaration, and a member is not one: clang's own
            // words are "type name does not allow storage class to be specified".
            if specs.is_thread_local {
                let span = self.span();
                self.error(span, "a member cannot be `_Thread_local`");
            }
            let Some(base) = specs.ty else {
                let span = self.span();
                self.error(span, "expected a member type");
                self.skip_until(&[";"], true);
                continue;
            };
            loop {
                // `int : 3;` - an anonymous bitfield, used only to pad.
                if self.at_punct(":") {
                    let width = self.bit_width();
                    let (alignas, packed) = self.member_attributes(member_from, specs.alignas);
                    members.push(Member {
                        name: None,
                        ty: base,
                        offset: 0,
                        bits: Some((0, width)),
                        alignas,
                        packed,
                    });
                } else {
                    let Some(tree) = self.declarator(false) else {
                        break;
                    };
                    let name = tree.name;
                    let (ty, vla) = self.apply_declarator(tree, base);
                    if vla.is_some() {
                        // A member's type is the record's shape, and a record is laid out before the
                        // program runs: bytes whose count is a value have no place in one.
                        let span = self.span();
                        self.error(span, "a struct member cannot be a variable-length array");
                    }
                    // `bit_width` takes the `:` itself, so it is not eaten here.
                    let bits = if self.at_punct(":") {
                        Some((0, self.bit_width()))
                    } else {
                        None
                    };
                    let (alignas, packed) = self.member_attributes(member_from, specs.alignas);
                    members.push(Member {
                        name,
                        ty,
                        offset: 0,
                        bits,
                        alignas,
                        packed,
                    });
                }
                if !self.eat(",") {
                    break;
                }
            }
            if !self.eat(";") {
                self.error_at_current("expected `;` after a member");
                self.skip_until(&[";"], true);
            }
        }
        if !self.eat("}") {
            self.error_at_current("expected `}` to end the members");
        }
        members
    }

    /// The alignment a declaration asked for: `__attribute__((aligned(N)))`, which raises, over
    /// whatever `_Alignas` and the attribute list before the type already said.
    fn declaration_alignment(&self, began: usize, alignas: u64) -> u64 {
        let mut alignas = alignas;
        for attribute in self.preprocessed.layout_attributes(began, self.at) {
            if let crate::types::LayoutAttribute::Aligned(alignment) = attribute {
                alignas = alignas.max(alignment);
            }
        }
        alignas
    }

    /// What one member's declaration said about its alignment, ready to spread into its `Member`.
    ///
    /// `__attribute__((packed))` on the member caps its *natural* alignment at 1, and `aligned(N)`
    /// raises it - which is the same question `_Alignas` asks, so `specs.alignas` is the seed.
    fn member_attributes(&self, from: usize, alignas: u64) -> (u64, bool) {
        let mut packed = false;
        let mut alignas = alignas;
        for attribute in self.preprocessed.layout_attributes(from, self.at) {
            match attribute {
                crate::types::LayoutAttribute::Packed => packed = true,
                crate::types::LayoutAttribute::Aligned(alignment) => {
                    alignas = alignas.max(alignment);
                }
            }
        }
        (alignas, packed)
    }

    fn bit_width(&mut self) -> u8 {
        self.bump(); // the `:`
        match self.constant_expression() {
            Some(width) if (0..=64).contains(&width) => width as u8,
            Some(_) => {
                let span = self.span();
                self.error(span, "a bitfield must be between 0 and 64 bits wide");
                0
            }
            None => 0,
        }
    }

    /// A declarator: `*`s, a name or a group, then suffixes - parsed into a tree, because
    /// `int *a[3]` and `int (*a)[3]` differ only in where the parentheses are.
    fn declarator(&mut self, capture_parameters: bool) -> Option<DeclaratorTree> {
        if !self.enter_nesting() {
            return None;
        }
        // A new declaration is a new question about which list is the function's own.
        if capture_parameters {
            self.captured_parameters = false;
        }
        let mut wrappers = Vec::new();
        // A convention may stand on either side of the `*`s - `char * __stdcall f` and
        // `__cdecl * p` are both written in the wild - so it is skipped before the run and after
        // each `*`.
        self.skip_calling_convention();
        while self.eat("*") {
            wrappers.push(Wrapper::Pointer);
            while self.peek().is_some_and(|token| {
                matches!(
                    self.spell(token),
                    "const"
                        | "volatile"
                        | "restrict"
                        | "__restrict"
                        | "__unaligned"
                        | "__ptr32"
                        | "__ptr64"
                        | "__sptr"
                        | "__uptr"
                )
            }) {
                self.bump();
            }
            self.skip_calling_convention();
        }
        let inner = if self.eat("(") {
            let before = self.at;
            // The group carries the name being declared, so its parameter list is the function's
            // own: `int (*getter(vfs *p))(struct vfs *)` takes a `vfs *` and *returns* a function
            // pointer - and passing `false` here captured only the return type's list, so `p` was
            // not declared in the body. `captured_parameters` keeps the second list from being
            // taken for the first.
            let Some(inner) = self.declarator(capture_parameters) else {
                // The recursion gave up without reading anything, which is the same no-progress
                // hole as the missing `)` below: the caller's loop would re-read this `(` for ever.
                if self.at == before {
                    self.skip_until(&[")"], true);
                }
                return None;
            };
            if !self.eat(")") {
                self.error_at_current("expected `)`");
                // A parenthesised declarator with no `*` and no name - `int ( x * p ) ( void ) ;`,
                // which is malformed C and which Windows headers produce by the hundred once
                // `__declspec` is in play - leaves the cursor where it was. The caller's loop then
                // re-reads the same `(` for ever: measured, a one-line struct member allocated
                // eleven gigabytes before the allocator gave up. Skipping to the group's `)` is
                // what makes the recovery *progress*, whatever the caller does next.
                if self.at == before {
                    self.skip_until(&[")"], true);
                }
            }
            inner
        } else {
            DeclaratorTree {
                name: if self
                    .peek()
                    .is_some_and(|token| token.kind == crate::lex::TokenKind::Ident)
                {
                    Some(self.bump().expect("peeked").text)
                } else {
                    None
                },
                wrappers: Vec::new(),
            }
        };

        // Suffixes bind tighter than the `*`s before them, and the *last* suffix is the
        // innermost: `int a[1][2]` is one array of two, so `[2]` is applied to the base first.
        let mut suffixes = Vec::new();
        loop {
            if self.eat("[") {
                if self.at_punct("]") {
                    self.bump();
                    suffixes.push(Wrapper::Array(None));
                    continue;
                }
                // A bound is a constant expression - that is what makes an array's *type* - and C99
                // lets it be a value instead, which is a variable-length array. One parse answers
                // both: a constant has its constant, a computed size has its code. The expression is
                // read *evaluated* inside a function, because that is what a variable-length array's
                // length has to be, and unevaluated outside one, where there is no code to put it in.
                // A constant first, and it has to be first: a cast in a bound - `char b[((int)(16 *
                // sizeof(void *)))]`, which is how Lua sizes a buffer - folds in an unevaluated
                // reading and does not in an evaluated one. Only when no constant comes out is the
                // bound a value the program computes, and reading it again is what gives it code.
                let began = self.at;
                let reported = self.diagnostics.len();
                let errors_before = self.errors;
                let constant = self.constant_expression();
                self.diagnostics.truncate(reported);
                self.errors = errors_before;
                self.at = began;
                let evaluated = !self.in_body();
                let saved = std::mem::replace(&mut self.unevaluated, evaluated);
                let value = self.assignment_expression();
                self.unevaluated = saved;
                let value = value.map(|value| self.decay(value));
                let width = || {
                    self.types
                        .fundamental("unsigned long long")
                        .unwrap_or_else(|| self.types.error())
                };
                match (constant, value) {
                    (Some(size), _) if size > 0 => {
                        suffixes.push(Wrapper::Array(Some(size as u64)));
                    }
                    (Some(_), _) => {
                        // C99 6.7.5.2p1: an array's size is greater than zero. One is used after the
                        // diagnostic because nothing will read it - the declaration is refused - and
                        // a zero-sized object must not be built on the way.
                        let span = self.span();
                        self.error(span, "an array's size must be greater than zero");
                        suffixes.push(Wrapper::Array(Some(1)));
                    }
                    (None, Some(count)) if self.types.is_integer(count.ty) && self.in_body() => {
                        if count.cir.is_none() {
                            // A number with no instruction behind it: an expression this compiler
                            // evaluates but does not emit, standing where the length is read.
                            let span = self.span();
                            self.error(
                                span,
                                "a variable-length array's length must be computable here",
                            );
                            suffixes.push(Wrapper::Array(Some(1)));
                        } else {
                            // The count is a value the program computes, so this is the second kind
                            // of array: the object is still the elements in a row, and the number of
                            // them is read at the declaration. A pointer to the element is what the
                            // name becomes, which is C99's own model of a variably modified type,
                            // minus the type.
                            let wide = width();
                            let count = self.convert(count, wide, self.span());
                            suffixes.push(Wrapper::VariableArray(Box::new(count)));
                        }
                    }
                    (None, Some(_)) if self.captured_parameters => {
                        // A parameter's array bound is *adjusted* away: C99 6.7.6.3p7 makes
                        // `int a[n]` a pointer, and 6.7.6.2p1 says the bound is not evaluated, so
                        // there is nothing to compute and nothing to complain about. `void
                        // f(int n, int a[n])` is `void f(int n, int *a)`, which is what the caller
                        // passes anyway.
                        suffixes.push(Wrapper::Array(None));
                    }
                    (None, Some(count)) => {
                        // Not an integer, or not a number at all: the messages C already has for a
                        // bound that cannot be one, and never a claim that a variable-length array
                        // was refused when nothing was asked for.
                        let described = self.describe(count.ty);
                        let span = self.span();
                        if self.types.is_integer(count.ty) {
                            self.error(
                                span,
                                "an array's size at file scope must be a constant expression",
                            );
                        } else {
                            self.error(
                                span,
                                format!("expected an integer constant expression, not {described}"),
                            );
                        }
                        suffixes.push(Wrapper::Array(Some(1)));
                    }
                    (None, None) => {
                        suffixes.push(Wrapper::Array(Some(1)));
                    }
                }
                if !self.eat("]") {
                    self.error_at_current("expected `]`");
                }
                continue;
            }
            if self.eat("(") {
                let (params, variadic) = self.parameter_list(capture_parameters);
                suffixes.push(Wrapper::Function { params, variadic });
                continue;
            }
            break;
        }
        suffixes.reverse();
        wrappers.extend(suffixes);
        wrappers.extend(inner.wrappers);
        self.leave_nesting();
        Some(DeclaratorTree {
            name: inner.name,
            wrappers,
        })
    }

    /// The type a declarator declares, given the base its specifiers named.
    ///
    /// A variable-length array comes back beside the type, because it is the one declarator whose
    /// result is not a type: its count and its element type are for the caller to turn into a stack
    /// allocation. Returning them is what keeps the answer from being *state* - a field set here and
    /// read by whichever declaration came next, which is how `typedef int Row[n];` handed the
    /// allocation it never asked for to the `int` declared after it.
    fn apply_declarator(
        &mut self,
        tree: DeclaratorTree,
        base: TypeId,
    ) -> (TypeId, Option<(Value, TypeId)>) {
        let mut ty = base;
        let mut vla = None;
        for wrapper in tree.wrappers {
            ty = match wrapper {
                Wrapper::Pointer => self.types.pointer(ty),
                Wrapper::Array(count) => self.types.array(ty, count),
                Wrapper::VariableArray(count) => {
                    // A variable-length array is the one object whose size the type system cannot
                    // hold, so the *name* is a pointer to the first element and the bytes are taken
                    // off the stack where the declaration stands. The element type and the count are
                    // handed to the declaration, which is the place that can emit the allocation.
                    vla = Some((*count, ty));
                    self.types.pointer(ty)
                }
                Wrapper::Function { params, variadic } => self.types.function(ty, params, variadic),
            };
        }
        (ty, vla)
    }

    /// The parameters of a function declarator, from inside the parentheses to past the `)`.
    ///
    /// They live in a scope of their own (C99 6.2.1p4), which is why `int f(int a, int a)` is
    /// an error and why these names are gone afterwards.
    fn parameter_list(&mut self, capture_names: bool) -> (Vec<TypeId>, bool) {
        let mut params = Vec::new();
        // Only the *first* list a declarator meets is the function's own; a second is the return
        // type's (see `captured_parameters`).
        if capture_names && !self.captured_parameters {
            self.last_parameters.clear();
            self.last_variadic = false;
            self.captured_parameters = true;
        }
        let mut variadic = false;
        self.symbols.push();
        if self.eat(")") {
            self.symbols.pop();
            return (params, variadic);
        }
        if self.at_word("void") && self.peek_at(1).is_some_and(|t| self.is_punct(t, ")")) {
            self.bump();
            self.eat(")");
            self.symbols.pop();
            return (params, variadic);
        }
        loop {
            if self.eat("...") {
                variadic = true;
                self.last_variadic = true;
                break;
            }
            let specs = self.declaration_specifiers();
            // 6.7.3p2 again, and clang's wording: "only allowed on variable declarations".
            if specs.is_thread_local {
                let span = self.span();
                self.error(
                    span,
                    "`_Thread_local` is only allowed on variable declarations",
                );
            }
            let base = specs.ty.unwrap_or_else(|| self.types.error());
            let ty = match self.declarator(false) {
                Some(tree) => {
                    // `int a[][3]` as a parameter is a pointer to `int[3]`, not an array.
                    let name = tree.name;
                    let (declared, _) = self.apply_declarator(tree, base);
                    let parameter = self.types.decay(declared);
                    // The parameter *object* keeps its `const`: `int f(const int x) { x = 1; }` is a
                    // write to a const object inside the body. The *function type* drops it
                    // (C99 6.7.5.3p15), so `int f(const int)` and `int f(int)` are one function -
                    // which is what a call through a pointer relies on. Two facts, two places.
                    if let Some(name) = name {
                        self.declare_parameter(name, parameter);
                    }
                    self.types.unqualified(parameter)
                }
                None => base,
            };
            params.push(ty);
            if !self.eat(",") {
                break;
            }
        }
        if !self.eat(")") {
            self.error_at_current("expected `)` after the parameters");
        }
        self.symbols.pop();
        (params, variadic)
    }

    fn declare_parameter(&mut self, name: Symbol, ty: TypeId) {
        let span = self.span();
        self.last_parameters.push((Some(name), ty, span));
        let entry = Entry {
            variable_length: false,
            variable_length_size: None,
            ty,
            kind: Kind::Variable,
            span,
            storage: None,
            defined: false,
        };
        if self.symbols.declare(name, entry).is_err() {
            let spelling = self.spell_of(name).to_owned();
            self.error(span, format!("duplicate parameter `{spelling}`"));
        }
    }

    // ---- recovery -------------------------------------------------------------

    /// The calling-convention words, which on x86-64 are spelling and nothing else: there is one
    /// convention, and Microsoft's own documentation of `__stdcall` for this target says so.
    /// Skip what may stand where a declarator begins and changes nothing this compiler decides: a
    /// calling convention (there is one on this target) and the alignment promises `winnt.h` writes
    /// between a type and its `*` - `} IMAGE_SYMBOL_EX, UNALIGNED *PIMAGE_SYMBOL_EX;`.
    fn skip_calling_convention(&mut self) {
        while self.peek().is_some_and(|token| {
            matches!(
                self.spell(token),
                "__cdecl"
                    | "__stdcall"
                    | "__fastcall"
                    | "__thiscall"
                    | "__vectorcall"
                    | "__unaligned"
                    | "__ptr32"
                    | "__ptr64"
                    | "__w64"
                    | "__sptr"
                    | "__uptr"
            )
        }) {
            self.at += 1;
        }
    }

    fn error_at_current(&mut self, message: impl Into<String>) {
        let span = self.span();
        self.error(span, message);
    }

    /// Skip tokens until one of `delimiters` is reached at depth 0, or a `}` that closes the
    /// enclosing block. `consume` takes the delimiter as well.
    ///
    /// Four recoveries are this function with a different list - to the end of a declaration,
    /// of an initialiser, of a parameter - and they looked different only because each had its
    /// own copy of the depth arithmetic.
    fn skip_until(&mut self, delimiters: &[&str], consume: bool) {
        let mut depth = 0usize;
        while let Some(token) = self.peek() {
            let spelling = self.spell(token).to_owned();
            match spelling.as_str() {
                "(" | "[" | "{" => depth += 1,
                ")" | "]" | "}" => {
                    if depth == 0 {
                        if spelling == "}" {
                            return; // never skip past the block enclosing this recovery
                        }
                        if consume && delimiters.contains(&spelling.as_str()) {
                            self.bump();
                            return;
                        }
                        // A closer with nothing open closes something the *caller* had already
                        // consumed, and the delimiter being looked for is outside it: step over it
                        // and keep looking. Stopping here gave the caller's loop no progress -
                        // measured, as a hang on `struct H { int ( x * p ) ; };`, a member whose
                        // parenthesised declarator has no `*` before its name.
                        self.bump();
                        continue;
                    }
                    depth -= 1;
                }
                _ => {
                    if depth == 0 && delimiters.contains(&spelling.as_str()) {
                        if consume {
                            self.bump();
                        }
                        return;
                    }
                }
            }
            if self.at_eof() {
                return;
            }
            self.bump();
        }
    }

    fn skip_balanced_block(&mut self) {
        if !self.at_punct("{") {
            return;
        }
        let mut depth = 0usize;
        while let Some(token) = self.peek() {
            if self.is_punct(token, "{") {
                depth += 1;
            } else if self.is_punct(token, "}") {
                depth -= 1;
                self.bump();
                if depth == 0 {
                    return;
                }
                continue;
            } else if self.at_eof() {
                return;
            }
            self.bump();
        }
    }

    // ---- nesting budget -------------------------------------------------------

    /// Take one level of the nesting budget.
    ///
    /// Every early return between this and `leave_nesting` leaks a level, and a leak *lowers*
    /// the budget for everything after it in the file, silently. It is one call pair today;
    /// when the parser grows, replace it with a guard whose `Drop` gives the level back.
    fn enter_nesting(&mut self) -> bool {
        if self.nesting >= self.budgets.max_nesting {
            let span = self.span();
            let limit = self.budgets.max_nesting;
            self.error(span, format!("nesting is deeper than {limit}"));
            return false;
        }
        self.nesting += 1;
        true
    }

    fn leave_nesting(&mut self) {
        self.nesting = self.nesting.saturating_sub(1);
    }

    /// Run `parse` one level deeper, and give the level back whatever it answers - the shape a
    /// recursive rule takes so that a source nested past the budget is a diagnostic rather than a
    /// stack overflow, without every early return inside the rule having to remember.
    fn within_nesting<T>(&mut self, parse: impl FnOnce(&mut Self) -> Option<T>) -> Option<T> {
        if !self.enter_nesting() {
            return None;
        }
        let value = parse(self);
        self.leave_nesting();
        value
    }
}

/// What a declaration says about the linkage of the name it declares. C99 6.2.2 has three kinds and
/// this compiler emits two: a definition in this unit is either visible to the other units
/// (external) or private to this one (internal).
///
/// The third is the reason this is a type and not a bool. C99 6.7.4p6: a function *defined* here and
/// declared `inline` but neither `static` nor `extern` is an **inline definition**, which "does not
/// provide an external definition" - so the copy this compiler emits is private to the unit, and the
/// program must have an external definition somewhere else if it calls the function across units.
/// mingw-w64 defines every `FORCEINLINE` helper in `winnt.h` that way, which is how Lua 5.5.1 failed
/// to link before this existed.
#[derive(Clone, Copy, Default)]
enum Linkage {
    /// Visible to the other translation units.
    #[default]
    External,
    /// `static`: private to this one.
    Internal,
    /// `inline` alone: a definition that provides no external one.
    InlineDefinition,
}

impl Linkage {
    fn of(specs: &Specifiers) -> Self {
        if specs.is_static {
            Linkage::Internal
        } else if specs.is_inline && !specs.is_extern {
            Linkage::InlineDefinition
        } else {
            Linkage::External
        }
    }

    /// Whether a definition with this linkage is a symbol the linker should see.
    fn is_external(self) -> bool {
        matches!(self, Linkage::External)
    }
}

/// What the specifiers of a declaration said.
#[derive(Debug, Default)]
struct Specifiers {
    ty: Option<TypeId>,
    /// Whether `const` was one of them: the declared type is qualified, and the write rules ask.
    is_const: bool,
    /// The alignment `_Alignas` asked for, when it did: zero means the type's own.
    alignas: u64,
    is_typedef: bool,
    /// Whether `static` was one of the specifiers: inside a function that means storage duration
    /// and not linkage, which is the one place the word changes what a declaration *is*.
    is_static: bool,
    /// Whether `inline` was one of them. C99 6.7.4p6: a function defined `inline` and neither
    /// `static` nor `extern` is an *inline definition*, which "does not provide an external
    /// definition" - so the copy this compiler emits is internal to the unit, like a `static` one.
    is_inline: bool,
    /// Whether `extern` was one of them: with no initialiser that makes a declaration and not a
    /// definition (C99 6.9.2p2), so the object belongs to another translation unit and this one must
    /// not lay down bytes for it.
    is_extern: bool,
    /// Whether `_Thread_local` (C11 6.7.3) was one of them: the object has thread storage duration -
    /// one copy per thread, initialised as many times as there are threads.
    is_thread_local: bool,
    /// Whether the specifiers named a tag (`enum E { ... }`), which C lets stand alone as a
    /// declaration that declares nothing.
    declares_tag: bool,
    /// Whether a type specifier has been read. `int T;` declares a variable called `T` even
    /// when `T` is a typedef, so the loop must not treat a typedef name as a specifier once a
    /// type has been named (C99 6.7.2p2).
    has_type_specifier: bool,
}

/// One step of a declarator: what it does to the type of what it encloses.
#[derive(Debug, Clone)]
enum Wrapper {
    /// `*`: a pointer to the type enclosed.
    Pointer,
    /// `[n]`: an array of the type enclosed.
    Array(Option<u64>),
    /// `[e]` where `e` is not a constant: a variable-length array (C99 6.7.3.2). The value is the
    /// element *count*, computed where the declaration stands.
    VariableArray(Box<crate::parse::Value>),
    /// `(params)`: a function returning the type enclosed.
    Function { params: Vec<TypeId>, variadic: bool },
}

/// A declarator, unapplied: its name and the wrappers to put around a base type, in order.
///
/// `int *a[3]` is `[Pointer, Array(3)]` - a pointer first, then an array *of* that pointer -
/// while `int (*a)[3]` is `[Array(3), Pointer]`, which is why grouping has to be parsed into a
/// tree rather than read left to right (C99 6.7.5).
#[derive(Debug)]
struct DeclaratorTree {
    /// The name, if the declarator has one (an abstract declarator does not).
    name: Option<Symbol>,
    /// What to wrap around the base type, outermost first.
    wrappers: Vec<Wrapper>,
}

/// "1 argument", "2 arguments": the message reads as a sentence.
fn arguments(count: usize) -> String {
    if count == 1 {
        "1 argument".to_owned()
    } else {
        format!("{count} arguments")
    }
}

/// Fold one binary operation on two constants. `None` when it cannot be folded - a division by
/// zero, or a shift C leaves undefined, is not folded rather than guessed.
/// Fold a binary operation on two integer constants, **in the type the operation is done in**.
///
/// C99 6.3.1.8 puts the usual arithmetic conversions before the operator, and this compared the two
/// spellings instead: `-1 > 0u` was `-1 > 0`, which is false, where C converts -1 to 4294967295 and
/// the answer is true. The `cir` folder had the rule and this copy did not - one fact written down
/// twice, disagreeing - and the corpus says so: `-1 / 2u` is 2147483647, not 0.
///
/// So the operands arrive already converted to `ty` (that is `fold_conversion`'s job at the call
/// site), and what is left here is the *width and sign* of `ty`: an unsigned shift is logical, an
/// unsigned division is unsigned, and comparisons see numbers rather than bit patterns.
fn fold(types: &Types, ty: TypeId, operator: &str, left: i64, right: i64) -> Option<i64> {
    // The type's own width and sign, which is what decides how each operation behaves. A pointer or
    // anything else that reaches this is not a constant expression.
    let (bits, signed) = match types.kind(ty) {
        Type::Int { bits, signed } => (*bits, *signed),
        Type::Bool => (8, false),
        _ => return None,
    };
    let widen = |value: i64| -> i64 {
        if bits >= 64 {
            return value;
        }
        let mask = (1i64 << bits) - 1;
        let narrowed = value & mask;
        if signed && narrowed >> (bits - 1) != 0 {
            narrowed | !mask
        } else {
            narrowed
        }
    };
    // The same value seen as an unsigned number of `bits` bits, which is what an unsigned
    // comparison, division or shift asks about.
    let unsigned = |value: i64| -> u64 {
        if bits >= 64 {
            value as u64
        } else {
            (value as u64) & ((1u64 << bits) - 1)
        }
    };
    let answer = match operator {
        // Comparisons see numbers: two different bit patterns of `-1` and `4294967295` are one
        // number in an unsigned type, and the conversion at the call site already made them one.
        "<" | ">" | "<=" | ">=" | "==" | "!=" => {
            let (a, b) = if signed {
                (left, right)
            } else {
                // Both are already non-negative after the conversion, but a 64-bit unsigned value
                // is negative as `i64` and must compare as the larger number it is.
                let (a, b) = (unsigned(left), unsigned(right));
                return Some(match operator {
                    "<" => i64::from(a < b),
                    ">" => i64::from(a > b),
                    "<=" => i64::from(a <= b),
                    ">=" => i64::from(a >= b),
                    "==" => i64::from(a == b),
                    _ => i64::from(a != b),
                });
            };
            return Some(match operator {
                "<" => i64::from(a < b),
                ">" => i64::from(a > b),
                "<=" => i64::from(a <= b),
                ">=" => i64::from(a >= b),
                "==" => i64::from(a == b),
                _ => i64::from(a != b),
            });
        }
        "/" if right != 0 => {
            if signed {
                left.wrapping_div(right)
            } else {
                (unsigned(left) / unsigned(right)) as i64
            }
        }
        "%" if right != 0 => {
            if signed {
                left.wrapping_rem(right)
            } else {
                (unsigned(left) % unsigned(right)) as i64
            }
        }
        // A shift is logical on an unsigned type and arithmetic on a signed one, and it is done in
        // the type's width - which is the one operation where the *sign* alone is not enough.
        ">>" if (0..bits as i64).contains(&right) => {
            if signed {
                return Some(widen(left >> right));
            }
            return Some((unsigned(left) >> right) as i64);
        }
        "<<" if (0..bits as i64).contains(&right) => {
            let shifted = unsigned(left) << right;
            return Some(widen(if bits >= 64 {
                shifted as i64
            } else {
                (shifted & ((1u64 << bits) - 1)) as i64
            }));
        }
        "+" => left.wrapping_add(right),
        "-" => left.wrapping_sub(right),
        "*" => left.wrapping_mul(right),
        "&" => left & right,
        "|" => left | right,
        "^" => left ^ right,
        "&&" => i64::from(left != 0 && right != 0),
        "||" => i64::from(left != 0 || right != 0),
        _ => return None,
    };
    Some(widen(answer))
}

#[cfg(test)]
mod tests {
    /// **Two lists of the same thing, and they must name the same names.** The intrinsic table serves
    /// a program that calls one bare; `include/x86intrin.h` serves a program that includes the header.
    /// MSVC has both, so this compiler has both - and the only thing that keeps them from drifting is
    /// this assertion, because a name in one and not the other is a program that compiles one way
    /// depending on whether it includes a header.
    #[test]
    fn the_intrinsic_table_names_what_the_header_declares() {
        let header = include_str!("../../include/x86intrin.h");
        for (name, _, _) in super::INTRINSICS {
            assert!(
                header.contains(name),
                "`{name}` is in the parser's table and not in x86intrin.h"
            );
        }
        // And the other direction, for the fifteen the header declares as *functions*: every
        // declaration line whose name starts with `_` or `__` is one the table must know too.
        for line in header.lines() {
            let Some(open) = line.find('(') else { continue };
            if !line.trim_end().ends_with(';') {
                continue;
            }
            let before = &line[..open];
            let Some(name) = before.split_whitespace().last() else {
                continue;
            };
            if !name.starts_with('_') || name == "_mm_mfence_declared_nowhere" {
                continue;
            }
            assert!(
                super::INTRINSICS.iter().any(|(known, _, _)| known == &name),
                "`{name}` is declared in x86intrin.h and not in the parser's table"
            );
        }
    }

    use super::*;
    use crate::intern::Interner;
    use crate::pp::preprocess;
    use crate::source::SourceFile;

    /// Preprocess and parse `source`, giving back the module it produced.
    fn module(source: &str) -> (cir::Module, Interner, Vec<Diagnostic>) {
        let file = SourceFile::new("t.c", source);
        let options = crate::options::Options::default();
        let mut preprocessed = preprocess(&file, &options, &());
        assert!(
            !preprocessed.has_errors(),
            "the test source preprocesses: {:?}",
            preprocessed.diagnostics
        );
        let parsed = parse(&mut preprocessed, &options.budgets);
        let names = preprocessed.names().clone();
        (parsed.module, names, parsed.diagnostics)
    }

    fn ir(source: &str) -> String {
        let (module, names, diagnostics) = module(source);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        cir::dump::dump(&module, &names)
    }

    #[test]
    fn a_function_body_becomes_cir() {
        // The slots come out in the reverse of declaration order. A `Slot` names the address of a
        // local's frame space, which is a fact about the *function* - so every slot is put at the
        // front of the entry block, where it dominates every use. Emitting them where the
        // declarations stood gave a `switch` case block a value created in another case's block
        // (miniz's inflater), which is invalid SSA.
        assert_eq!(
            ir("int add(int a, int b) { return a + b; }"),
            "func add(v0: int, v1: int) -> int
block0 (entry)(v0: int, v1: int):
  v3 = slot int
  v2 = slot int
  store v0, v2
  store v1, v3
  v4 = load int, v2
  v5 = load int, v3
  v6 = add int v4, v5
  return v6
"
        );
    }

    #[test]
    fn a_file_scope_object_becomes_a_global() {
        let (unit, names, diagnostics) =
            module("int x; static long long y; int f(void) { return x; }");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let text = cir::dump::dump(&unit, &names);
        assert!(text.contains("global x: int"), "{text}");
        assert!(text.contains("global y: long long"), "{text}");
        // A tentative definition is defined with no bytes; an initialiser fills them, and the
        // dump says how many the object has.
        assert_eq!(unit.globals.len(), 2);
        let (unit, names, diagnostics) =
            module("int x = 5; char s[] = \"hi\"; int a[3] = {1, 2, 3};");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let text = cir::dump::dump(&unit, &names);
        assert!(text.contains("global x: int (4 bytes)"), "{text}");
        assert!(
            text.contains("global s: array[3] of char (3 bytes)"),
            "{text}"
        );
        assert!(
            text.contains("global a: array[3] of int (12 bytes)"),
            "{text}"
        );
        // Each element at its own offset. An array that walked no offset wrote all three at the
        // start - `3` alone - which a count of bytes could not see.
        let a = unit
            .globals
            .iter()
            .find(|global| global.name == "a")
            .expect("the array is a global");
        assert_eq!(
            a.init.as_deref(),
            Some(&[1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0][..])
        );
        // Adjacent literals are one run (C99 6.4.5p5): decoded once, and their bytes are the
        // object's. A second reader of the spelling refused this with "an escape this compiler
        // does not read".
        let (unit, _, diagnostics) = module("char s[] = \"ab\" \"cd\";");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let s = unit
            .globals
            .iter()
            .find(|global| global.name == "s")
            .expect("the array is a global");
        assert_eq!(s.init.as_deref(), Some(&b"abcd\0"[..]));
        // A float initialiser is its bit pattern, and a folded one is folded in its own width.
        let (unit, _, diagnostics) = module("double d = 1.5; double e = 1.5 * 2; float f = 0.5f;");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        for (name, wanted) in [
            ("d", 1.5f64.to_bits().to_le_bytes().to_vec()),
            ("e", 3.0f64.to_bits().to_le_bytes().to_vec()),
            ("f", 0.5f32.to_bits().to_le_bytes().to_vec()),
        ] {
            let global = unit
                .globals
                .iter()
                .find(|global| global.name == name)
                .expect("the object is a global");
            assert_eq!(global.init.as_deref(), Some(&wanted[..]), "{name}");
        }
        // A negative constant is negated as its *own* type. Reading an `int`'s constant as an
        // `f32` because it is four bytes wide wrote `-3` as -2147483645.
        let (unit, _, diagnostics) = module("int x = -3; long y = -4; double z = -1.5;");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        for (name, wanted) in [
            ("x", (-3i32).to_le_bytes().to_vec()),
            // `long` is four bytes on this target.
            ("y", (-4i32).to_le_bytes().to_vec()),
            ("z", (-1.5f64).to_bits().to_le_bytes().to_vec()),
        ] {
            let global = unit
                .globals
                .iter()
                .find(|global| global.name == name)
                .expect("the object is a global");
            assert_eq!(global.init.as_deref(), Some(&wanted[..]), "{name}");
        }
        // A `_Bool` at file scope holds 0 or 1, whatever constant it was given.
        let (unit, _, diagnostics) = module("_Bool yes = 2; _Bool no = 0.0;");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        for (name, wanted) in [("yes", 1u8), ("no", 0u8)] {
            let global = unit
                .globals
                .iter()
                .find(|global| global.name == name)
                .expect("the object is a global");
            assert_eq!(global.init.as_deref(), Some(&[wanted][..]), "{name}");
        }
        // A value that is not a constant is refused, not declared with no bytes at all.
        let (unit, names, _) = module("int f(void) { return 1; } int y = f();");
        let text = cir::dump::dump(&unit, &names);
        assert!(!text.contains("global y:"), "{text}");
    }

    #[test]
    fn a_loop_becomes_blocks_and_a_branch() {
        let text =
            ir("int f(int n) { int s; s = 0; while (n > 0) { s = s + n; n = n - 1; } return s; }");
        assert!(text.contains("br "), "{text}");
        assert!(text.contains("jump block1"), "{text}");
        assert!(text.contains("cmp gt int"), "{text}");
    }

    /// A capped list says how many it refused, and the count is the difference: whatever the
    /// budget, *shown + refused* is the same number - the diagnostics the source earned.
    #[test]
    fn a_capped_diagnostic_list_counts_what_it_refused() {
        let source: String = (0..120)
            .map(|i| {
                format!(
                    "int q{i} = ;
"
                )
            })
            .collect();
        let file = SourceFile::new("many.c", &source);
        let options = crate::options::Options::default();
        let mut preprocessed = preprocess(&file, &options, &());
        let generous = parse(
            &mut preprocessed,
            &crate::options::Budgets {
                max_diagnostics: 10_000,
                ..crate::options::Budgets::default()
            },
        );
        let total = generous.diagnostics.len();
        assert!(total > 100, "the source earns more than a hundred: {total}");

        let capped = parse(
            &mut preprocessed,
            &crate::options::Budgets {
                max_diagnostics: 10,
                ..crate::options::Budgets::default()
            },
        );
        assert_eq!(capped.diagnostics.len(), 10, "the cap is the cap");
        assert_eq!(
            capped.diagnostics.len() + capped.dropped,
            total,
            "shown plus refused is what the source earned, whatever the budget"
        );
    }

    #[test]
    fn what_the_emitter_cannot_express_is_refused_and_the_body_dropped() {
        // A `va_arg` of a *record* is the shape: the value has to come back in a register the back
        // end chose, and an aggregate has no register. (A record-valued `?:` used to be the example
        // here - it is lowered now, because the arms are already addresses and the join carries
        // one.)
        let (module, _, diagnostics) = module(
            "typedef struct { int a; } S;
             int f(int n, ...) { __builtin_va_list ap; __builtin_va_start(ap, n); S s = __builtin_va_arg(ap, S); return s.a; }",
        );
        let messages: Vec<&str> = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .collect();
        assert!(
            messages
                .iter()
                .any(|message| message.contains("not lowered yet")),
            "{messages:?}"
        );
        assert!(module.funcs.is_empty(), "a body with a gap is not lowered");
    }
}
