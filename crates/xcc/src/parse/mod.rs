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
use crate::types::{Member, RecordKind, Type, TypeId, Types};

/// Parse a preprocessed translation unit.
///
/// Returns every diagnostic, in source order. An empty list means the declarations are
/// well formed and typed; it says nothing about the code an expression would generate, because
/// there is no code generation yet.
pub(crate) fn parse(preprocessed: &Preprocessed, budgets: &Budgets) -> Parsed {
    let mut parser = Parser {
        preprocessed,
        at: 0,
        types: Types::new(),
        symbols: Symbols::new(),
        diagnostics: Vec::new(),
        budgets,
        nesting: 0,
        function_result: None,
        loops: 0,
        switches: Vec::new(),
        labels: HashMap::new(),
        gotos: Vec::new(),
        last_parameters: Vec::new(),
        funcs: Vec::new(),
        globals: Vec::new(),
        body: None,
        last_declared: None,
        bytes: None,
        offset: 0,
        init_target: None,
        unevaluated: false,
        literals: 0,
        targets: Vec::new(),
    };
    parser.translation_unit();
    Parsed {
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

    /// A value that code was generated for.
    fn code(ty: TypeId, cir: cir::Value) -> Self {
        Self {
            ty,
            constant: None,
            lvalue: false,
            cir: Some(cir),
            place: None,
        }
    }
}

/// The words that can begin a declaration: storage classes, qualifiers, type specifiers and
/// the tag keywords. `starts_specifier` asks this list, and a test pins it against the
/// specifier reader so the two cannot drift apart.
const SPECIFIERS: &[&str] = &[
    "typedef", "extern", "static", "auto", "register", "const", "volatile", "restrict", "inline",
    "void", "_Bool", "char", "short", "int", "long", "float", "double", "signed", "unsigned",
    "struct", "union", "enum",
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
    preprocessed: &'a Preprocessed,
    /// Position in the token stream.
    at: usize,
    types: Types,
    symbols: Symbols,
    diagnostics: Vec<Diagnostic>,
    budgets: &'a Budgets,
    /// How deep the current recursion is, for the budget.
    nesting: usize,
    /// What the function being read returns, while its body is being read.
    function_result: Option<TypeId>,
    /// How many loops surround the statement being read: `break` and `continue` need one.
    loops: usize,
    /// The `switch`es being read, innermost last, each with the `case` values it already has.
    switches: Vec<Switch>,
    /// The labels defined in the function being read, function-scoped (C99 6.2.1p4), and the
    /// `goto`s that are still waiting for one.
    labels: HashMap<Symbol, Span>,
    gotos: Vec<(Symbol, Span)>,
    /// The parameter names of the last function declarator, which a definition declares in its
    /// body's scope.
    last_parameters: Vec<(Option<Symbol>, TypeId, Span)>,
    /// The functions whose bodies have been read.
    funcs: Vec<cir::Func>,
    /// The globals declared so far.
    globals: Vec<cir::Global>,
    /// The function being read, while its body is being parsed.
    body: Option<Body>,
    /// The name the last declarator declared, for a definition that follows it.
    last_declared: Option<Symbol>,
    /// Where the bytes of a file-scope initialiser are being written, when one is: the same
    /// traversal that checks an initialiser fills this, so the bytes and the diagnostics come
    /// from one reading of the source (ADR-004).
    bytes: Option<Vec<u8>>,
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
    /// Whether the expression being read is not evaluated: `sizeof`'s operand, which C says is
    /// not (C99 6.5.3.4p2), and a `constant_expression`, which is wanted for its value and not
    /// for its code. Nothing parsed in either may emit: `sizeof(x = 3)` must not assign, and
    /// `int a[1 ? 2 : 3]` must not leave a branch in the middle of a declaration.
    unevaluated: bool,
    /// How many string literals have been given an anonymous global: `.str0`, `.str1`, ...
    literals: u32,
    /// Where `break` and `continue` go, innermost last: the block after the loop, and the block
    /// the next iteration starts at.
    targets: Vec<(cir::Block, cir::Block)>,
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

    fn push(&mut self, diagnostic: Diagnostic, span: Span) {
        let file = self.peek().map_or(0, |token| token.file);
        let name = self
            .preprocessed
            .files
            .get(file as usize)
            .map(|source| source.name().to_owned())
            .unwrap_or_default();
        self.diagnostics
            .push(diagnostic.with_file(name).with_span(span));
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
    }

    /// A declaration or function definition, ending at `;` or a body.
    fn external_declaration(&mut self) {
        let Some(declared) = self.declaration() else {
            self.skip_until(&[";"], true);
            return;
        };
        let name = self.last_declared;
        if self.at_punct("{") {
            match self.types.kind(declared) {
                Type::Function { result, .. } => {
                    let result = *result;
                    self.function_body(name, result);
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
        let specs = self.declaration_specifiers();
        let Some(base) = specs.ty else {
            let span = self.span();
            self.error(span, "expected a type specifier");
            self.skip_until(&[";"], true);
            return None;
        };
        let specs = Specifiers {
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
            let ty = self.apply_declarator(tree, base);
            let entry = Entry {
                ty,
                storage: None,
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
            if self.in_body() && !self.lowerable(ty) && !specs.is_typedef {
                let span = self.span();
                self.not_lowered(span, "a variable whose type is an aggregate or a function");
            }
            if self.in_body()
                && !specs.is_typedef
                && !matches!(self.types.kind(ty), Type::Function { .. })
            {
                // A local variable is a slot in the function's frame (ADR-002 invariant 3).
                if let Some(name) = name {
                    let pointer = self.pointer_to(ty);
                    let span = self.span();
                    if let Some(slot) = self.emit(|builder| builder.slot(ty, pointer, span)) {
                        if let Some(entry) = self.symbols.here_mut(name) {
                            entry.storage = Some(slot);
                        }
                    }
                }
            }
            // At file scope an object is data, not a slot: ADR-004. A tentative definition
            // (`int x;`) is defined and zero-filled; an initialiser is the list walker's work
            // and is still refused, because a global with no bytes would be a wrong program.
            if !self.in_body() && !specs.is_typedef && !self.at_punct("=") {
                if let Some(name) = name {
                    if matches!(self.types.kind(ty), Type::Function { .. }) {
                        // A function declaration is a signature, not data.
                    } else {
                        let span = self.span();
                        self.globals.push(cir::Global {
                            name: self.spell_of(name).to_owned(),
                            ty,
                            defined: true,
                            init: None,
                            // `static` is not distinguished yet: the storage class lives in
                            // `Specifiers`, and patch 4 (which makes visibility matter) is the
                            // place to read it. Until then every object claims to be external,
                            // which is wrong for a `static` and harmless only because nothing
                            // reads the flag.
                            external: true,
                            span,
                        });
                    }
                }
            }
            // An initialiser: only a constant expression, for now.
            if self.eat("=") {
                let span = self.span();
                if self.in_body() {
                    // A local has a slot, so its initialiser becomes stores into the frame.
                    // A type with no slot (an aggregate in a body) was refused above; its
                    // initialiser is still read, so its own mistakes are still reported.
                    let target = name
                        .and_then(|name| self.symbols.here(name))
                        .and_then(|entry| entry.storage)
                        .map(|place| (place, ty));
                    self.initialize_local(ty, target);
                } else {
                    let before = self.error_count();
                    match self.initializer_bytes(ty) {
                        Some(bytes) => {
                            if let Some(name) = name {
                                self.globals.push(cir::Global {
                                    name: self.spell_of(name).to_owned(),
                                    ty,
                                    defined: true,
                                    init: Some(bytes),
                                    external: true,
                                    span,
                                });
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
        let mut init: Vec<u8> = Vec::new();
        while let Some(token) = self.peek().copied() {
            if token.kind != crate::lex::TokenKind::Str {
                break;
            }
            let written = self.spell(&token).to_owned();
            self.bump();
            match crate::literal::string_bytes(&written) {
                Some(mut bytes) => init.append(&mut bytes),
                None => {
                    // An escape this compiler does not read is reported and filled with
                    // zeroes: the object keeps the size the source spelled, rather than
                    // silently becoming one byte for a two-byte array.
                    self.error(
                        span,
                        "this string literal has an escape this compiler does not read",
                    );
                    let count = string_length(&written) as usize;
                    init.resize(init.len() + count, 0);
                }
            }
        }
        init.push(0);
        let char = self
            .types
            .fundamental("char")
            .unwrap_or_else(|| self.types.error());
        let ty = self.types.array(char, Some(init.len() as u64));
        // A literal is an object (ADR-004): an anonymous global holding its bytes and its NUL,
        // named with a `.` prefix so it can never collide with a C identifier.
        let name = format!(".str{}", self.literals);
        self.literals += 1;
        self.globals.push(cir::Global {
            name: name.clone(),
            ty,
            defined: true,
            init: Some(init.clone()),
            external: false,
            span,
        });
        (ty, name, init)
    }

    /// The bytes a file-scope initialiser gives an object of type `ty`, or `None` when a value
    /// in it is not something this can write.
    ///
    /// The traversal is `initializer`'s - brace elision, designators, and every diagnostic are
    /// unchanged - with `bytes` set so the leaves land at their offsets. There is no second
    /// reader: that was tried twice and failed both times (ADR-004).
    fn initializer_bytes(&mut self, ty: TypeId) -> Option<Vec<u8>> {
        // `char s[] = "hi"` has no size until the initialiser gives it one, so the buffer grows
        // as the traversal learns; it is cut to the type's size at the end.
        let size = self.types.size_of(ty).map(|size| size as usize);
        self.bytes = Some(vec![0u8; size.unwrap_or(0)]);
        self.offset = 0;
        self.initializer(ty);
        let mut bytes = self.bytes.take()?;
        let size = self.types.size_of(ty).map(|size| size as usize)?;
        bytes.resize(size, 0);
        Some(bytes)
    }

    /// How a type is written in a diagnostic, with its tags: `pointer to struct Point`. The
    /// parser has the names and the type arena does not, so this is where they meet.
    fn describe(&self, ty: TypeId) -> String {
        self.types.describe_with(ty, self.preprocessed.names())
    }

    /// How a name from the symbol table is spelled.
    fn spell_of(&self, name: Symbol) -> &str {
        self.preprocessed.spell(name)
    }

    /// Storage class, qualifiers and type specifiers, in any order.
    fn declaration_specifiers(&mut self) -> Specifiers {
        let mut specs = Specifiers::default();
        let mut spelled: Vec<String> = Vec::new();
        loop {
            let Some(token) = self.peek().copied() else {
                break;
            };
            let word = self.spell(&token).to_owned();
            match word.as_str() {
                "typedef" | "extern" | "static" | "auto" | "register" => {
                    if word == "typedef" {
                        specs.is_typedef = true;
                    }
                    self.at += 1; // a storage class is not part of the type
                }
                "const" | "volatile" | "restrict" | "inline" => {
                    self.at += 1; // qualifiers carry no meaning in M3.1
                }
                "void" | "_Bool" | "char" | "short" | "int" | "long" | "float" | "double"
                | "signed" | "unsigned" => {
                    spelled.push(word);
                    specs.has_type_specifier = true;
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
            .filter(|word| !matches!(*word, "typedef" | "extern" | "static" | "auto" | "register"))
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
        let bits = if count("char") > 0 {
            8
        } else if count("short") > 0 {
            16
        } else if count("long") >= 2 {
            64
        } else {
            32 // `int`, and `long`, which is 32 here
        };
        specs.ty = Some(self.types.integer_with(signed, bits));
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
                ty: int,
                kind: Kind::EnumConst(value),
                span: token.span,
                storage: None,
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
                let members = self.member_list();
                self.types.complete_record(record, members);
                return ty;
            }
        }
        let (ty, record) = self.types.record(kind, tag);
        if let Some(tag) = tag {
            self.symbols.declare_tag(tag, ty);
        }
        let members = self.member_list();
        self.types.complete_record(record, members);
        ty
    }

    /// The members of a struct or union, between its braces.
    fn member_list(&mut self) -> Vec<Member> {
        let mut members = Vec::new();
        if !self.eat("{") {
            return members;
        }
        while !self.at_punct("}") && !self.at_eof() {
            let specs = self.declaration_specifiers();
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
                    members.push(Member {
                        name: None,
                        ty: base,
                        offset: 0,
                        bits: Some((0, width)),
                    });
                } else {
                    let Some(tree) = self.declarator(false) else {
                        break;
                    };
                    let name = tree.name;
                    let ty = self.apply_declarator(tree, base);
                    // `bit_width` takes the `:` itself, so it is not eaten here.
                    let bits = if self.at_punct(":") {
                        Some((0, self.bit_width()))
                    } else {
                        None
                    };
                    members.push(Member {
                        name,
                        ty,
                        offset: 0,
                        bits,
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
        let mut wrappers = Vec::new();
        while self.eat("*") {
            wrappers.push(Wrapper::Pointer);
            while self
                .peek()
                .is_some_and(|token| matches!(self.spell(token), "const" | "volatile" | "restrict"))
            {
                self.bump();
            }
        }
        let inner = if self.eat("(") {
            let inner = self.declarator(false)?;
            if !self.eat(")") {
                self.error_at_current("expected `)`");
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
                let count = if self.at_punct("]") {
                    None
                } else {
                    self.constant_expression().map(|size| size.max(0) as u64)
                };
                if !self.eat("]") {
                    self.error_at_current("expected `]`");
                }
                suffixes.push(Wrapper::Array(count));
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
    fn apply_declarator(&mut self, tree: DeclaratorTree, base: TypeId) -> TypeId {
        let mut ty = base;
        for wrapper in tree.wrappers {
            ty = match wrapper {
                Wrapper::Pointer => self.types.pointer(ty),
                Wrapper::Array(count) => self.types.array(ty, count),
                Wrapper::Function { params, variadic } => self.types.function(ty, params, variadic),
            };
        }
        ty
    }

    /// The parameters of a function declarator, from inside the parentheses to past the `)`.
    ///
    /// They live in a scope of their own (C99 6.2.1p4), which is why `int f(int a, int a)` is
    /// an error and why these names are gone afterwards.
    fn parameter_list(&mut self, capture_names: bool) -> (Vec<TypeId>, bool) {
        let mut params = Vec::new();
        if capture_names {
            self.last_parameters.clear();
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
                break;
            }
            let specs = self.declaration_specifiers();
            let base = specs.ty.unwrap_or_else(|| self.types.error());
            let ty = match self.declarator(false) {
                Some(tree) => {
                    // `int a[][3]` as a parameter is a pointer to `int[3]`, not an array.
                    let name = tree.name;
                    let declared = self.apply_declarator(tree, base);
                    let parameter = self.types.decay(declared);
                    if let Some(name) = name {
                        self.declare_parameter(name, parameter);
                    }
                    parameter
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
            ty,
            kind: Kind::Variable,
            span,
            storage: None,
        };
        if self.symbols.declare(name, entry).is_err() {
            let spelling = self.spell_of(name).to_owned();
            self.error(span, format!("duplicate parameter `{spelling}`"));
        }
    }

    // ---- recovery -------------------------------------------------------------

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
                        }
                        return;
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
}

/// What the specifiers of a declaration said.
#[derive(Debug, Default)]
struct Specifiers {
    ty: Option<TypeId>,
    is_typedef: bool,
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
fn fold(operator: &str, left: i64, right: i64) -> Option<i64> {
    Some(match operator {
        "+" => left.wrapping_add(right),
        "-" => left.wrapping_sub(right),
        "*" => left.wrapping_mul(right),
        "/" if right != 0 => left.wrapping_div(right),
        "%" if right != 0 => left.wrapping_rem(right),
        "<<" if (0..64).contains(&right) => left.wrapping_shl(right as u32),
        ">>" if (0..64).contains(&right) => left.wrapping_shr(right as u32),
        "&" => left & right,
        "|" => left | right,
        "^" => left ^ right,
        "==" => i64::from(left == right),
        "!=" => i64::from(left != right),
        "<" => i64::from(left < right),
        ">" => i64::from(left > right),
        "<=" => i64::from(left <= right),
        ">=" => i64::from(left >= right),
        "&&" => i64::from(left != 0 && right != 0),
        "||" => i64::from(left != 0 || right != 0),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::intern::Interner;
    use crate::pp::preprocess;
    use crate::source::SourceFile;

    /// Preprocess and parse `source`, giving back the module it produced.
    fn module(source: &str) -> (cir::Module, Interner, Vec<Diagnostic>) {
        let file = SourceFile::new("t.c", source);
        let options = crate::options::Options::default();
        let preprocessed = preprocess(&file, &options, &());
        assert!(
            !preprocessed.has_errors(),
            "the test source preprocesses: {:?}",
            preprocessed.diagnostics
        );
        let parsed = parse(&preprocessed, &options.budgets);
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
        assert_eq!(
            ir("int add(int a, int b) { return a + b; }"),
            "func add(v0: int, v1: int) -> int
block0 (entry)(v0: int, v1: int):
  v2 = slot int
  store v0, v2
  v3 = slot int
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

    #[test]
    fn what_the_emitter_cannot_express_is_refused_and_the_body_dropped() {
        let (module, _, diagnostics) =
            module("struct P { int a; }; int f(struct P p) { return p.a; }");
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
