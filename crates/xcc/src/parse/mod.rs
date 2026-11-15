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

mod expr;
mod stmt;

use std::collections::HashMap;

use crate::diag::{Diagnostic, Span};
use crate::intern::Symbol;
use crate::options::Budgets;
use crate::pp::{PpToken, Preprocessed};
use crate::symtab::{Entry, Kind, Symbols};
use crate::types::{Member, RecordKind, Type, TypeId, Types};

/// Parse a preprocessed translation unit.
///
/// Returns every diagnostic, in source order. An empty list means the declarations are
/// well formed and typed; it says nothing about the code an expression would generate, because
/// there is no code generation yet.
pub(crate) fn parse(preprocessed: &Preprocessed, budgets: &Budgets) -> Vec<Diagnostic> {
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
    };
    parser.translation_unit();
    parser.diagnostics
}

/// A value: what an expression evaluates to, as far as the front end knows.
#[derive(Debug, Clone, Copy)]
struct Value {
    ty: TypeId,
    /// The value, when it is known at compile time.
    constant: Option<i64>,
    /// Whether it can be assigned to: a variable, an element, a dereference.
    lvalue: bool,
}

impl Value {
    fn temporary(ty: TypeId) -> Self {
        Self {
            ty,
            constant: None,
            lvalue: false,
        }
    }

    fn constant(ty: TypeId, constant: i64) -> Self {
        Self {
            ty,
            constant: Some(constant),
            lvalue: false,
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
        if self.at_punct("{") {
            match self.types.kind(declared) {
                Type::Function { result, .. } => {
                    let result = *result;
                    self.function_body(result);
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
                        .map(|previous| self.types.describe(previous.ty))
                        .unwrap_or_default();
                    self.error(
                        span,
                        format!("`{spelling}` is already declared in this scope as {previous}"),
                    );
                }
            }
            // An initialiser: only a constant expression, for now.
            if self.eat("=") {
                self.initializer(ty);
            }
            last = Some(ty);
            if !self.eat(",") {
                return last;
            }
        }
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

/// How many bytes a string literal spells, not counting its quotes or the NUL.
fn string_length(spelling: &str) -> u64 {
    let body = spelling.strip_prefix('L').unwrap_or(spelling);
    let body = body
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(body);
    let mut bytes = 0u64;
    let mut escaped = false;
    for character in body.chars() {
        if escaped {
            escaped = false;
            bytes += 1;
        } else if character == '\\' {
            escaped = true;
        } else {
            bytes += 1;
        }
    }
    bytes
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

/// A character constant's value.
fn character_constant(spelling: &str) -> Option<i64> {
    let body = spelling.strip_prefix('L').unwrap_or(spelling);
    let body = body.strip_prefix('\'')?.strip_suffix('\'')?;
    let unescaped = match body {
        "\\n" => 10,
        "\\t" => 9,
        "\\r" => 13,
        "\\0" => 0,
        "\\\\" => 92,
        "\\'" => 39,
        "\\\"" => 34,
        "\\?" => 63,
        _ => {
            let mut characters = body.chars();
            let first = characters.next()?;
            if characters.next().is_some() {
                return None; // multi-character constants are not answered
            }
            first as i64
        }
    };
    Some(unescaped)
}
