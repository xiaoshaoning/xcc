//! The parser: declarations, declarators, and the constant expressions they need.
//!
//! This is the first slice of M3. It reads a translation unit of *declarations* -
//! `int x;`, `int *p = 0;`, `int f(char *s, long n);`, `typedef struct`-free shapes for now,
//! `enum E { A, B = 3 };`, `int a[10];` - builds their types, and puts the names in scopes.
//! Statements and non-constant expressions arrive next (M3.2), then `struct`/`union` and
//! initialiser lists (M3.3).
//!
//! There is no syntax tree, on purpose ([ADR-002](../../../docs/ADR-002-intermediate-form.md)):
//! the parser is written so that the *value* of an expression becomes a value in `cir` at M4,
//! and a parser that built a tree first would be a second encoding of what the type checker
//! already knows. What it produces today is a typed symbol table and diagnostics; what it will
//! produce at M4 is the same, plus `cir`.
//!
//! Two rules shape the whole file:
//!
//! - **A malformed declaration becomes the error type**, not a cascade: after the first
//!   diagnostic, the type is [`Type::Error`], which is compatible with everything, so the rest
//!   of the declaration stays quiet. C code with one typo should give one message.
//! - **Recursion is budgeted** (`Budgets::max_nesting`): a declarator or parenthesised
//!   expression nested past the limit is a diagnostic, not a stack overflow, because the host
//!   owns the stack (CONSTRAINTS 5).

use crate::diag::{Diagnostic, Span};
use crate::intern::Symbol;
use crate::options::{Budgets, CStandard};
use crate::pp::{PpToken, Preprocessed};
use crate::symtab::{Entry, Kind, Symbols};
use crate::types::{Type, TypeId, Types};

/// Parse a preprocessed translation unit.
///
/// Returns every diagnostic, in source order. An empty list means the declarations are
/// well formed and typed; it says nothing about the code an expression would generate, because
/// there is no code generation yet.
pub(crate) fn parse(
    preprocessed: &Preprocessed,
    budgets: &Budgets,
    _std: CStandard,
) -> ParseResult {
    let mut parser = Parser {
        preprocessed,
        at: 0,
        types: Types::new(),
        symbols: Symbols::new(),
        diagnostics: Vec::new(),
        budgets,
        nesting: 0,
    };
    parser.translation_unit();
    ParseResult {
        diagnostics: parser.diagnostics,
    }
}

/// What parsing produced: everything that was wrong with it.
#[derive(Debug, Default)]
pub(crate) struct ParseResult {
    /// Errors and warnings, in source order.
    pub diagnostics: Vec<Diagnostic>,
}

impl ParseResult {
    /// Whether anything went wrong badly enough to stop.
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(Diagnostic::is_error)
    }
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
            .cloned()
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
        let declaration = self.declaration();
        let Some(declaration) = declaration else {
            self.skip_to_semicolon();
            return;
        };
        if self.at_punct("{") {
            // M3.2: statements.
            if matches!(self.types.kind(declaration.ty), Type::Function { .. }) {
                self.not_implemented(
                    self.span(),
                    "function bodies are not implemented yet (M3.2)",
                );
                self.skip_balanced_block();
            } else {
                self.error_at_current("expected `;`");
                self.skip_balanced_block();
            }
            return;
        }
        if !self.eat(";") {
            self.error_at_current("expected `;` after a declaration");
            self.skip_to_semicolon();
        }
    }

    /// One declaration: specifiers, then one or more declarators.
    fn declaration(&mut self) -> Option<Declared> {
        let specs = self.declaration_specifiers()?;
        let mut last = None;
        loop {
            let base = specs.ty.unwrap_or_else(|| self.types.error());
            let (name, ty) = match self.declarator(base) {
                Some(Declarator::Read(name, ty)) => (name, ty),
                Some(Declarator::Refused(recovered)) => {
                    if let Some(name) = recovered {
                        let entry = Entry {
                            ty: self.types.error(),
                            kind: if specs.is_typedef {
                                Kind::Typedef
                            } else {
                                Kind::Variable
                            },
                            span: self.span(),
                        };
                        let _ = self.symbols.declare(name, entry);
                    }
                    self.skip_before_semicolon();
                    return Some(Declared {
                        ty: self.types.error(),
                    });
                }
                None => return last,
            };
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
                    return Some(Declared { ty });
                }
                let span = self.span();
                self.error(span, "declaration does not declare anything");
                return last;
            }
            if let Some(name) = name {
                let span = self.span();
                if let Err(previous) = self.symbols.declare(name, entry) {
                    let spelling = self.spell_of(name).to_owned();
                    self.error(
                        span,
                        format!("`{spelling}` is already declared in this scope"),
                    );
                    let _ = previous;
                }
            }
            // An initialiser: only a constant expression, for now.
            if self.eat("=") {
                self.initializer(ty);
            }
            last = Some(Declared { ty });
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
    fn declaration_specifiers(&mut self) -> Option<Specifiers> {
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
                    spelled.push(word);
                    self.at += 1;
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
                    let span = token.span;
                    self.not_implemented(
                        span,
                        "`struct` and `union` are not implemented yet (M3.3)",
                    );
                    self.skip_struct_specifier();
                    specs.ty = Some(self.types.error());
                    specs.declares_tag = true;
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
    fn finish_specifiers(
        &mut self,
        mut specs: Specifiers,
        spelled: Vec<String>,
    ) -> Option<Specifiers> {
        if specs.ty.is_some() {
            return Some(specs);
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
            return Some(specs);
        }
        if count("_Bool") > 0 {
            specs.ty = Some(self.types.fundamental("_Bool").unwrap());
            return Some(specs);
        }
        if count("float") > 0 {
            specs.ty = Some(self.types.fundamental("float").unwrap());
            return Some(specs);
        }
        if count("double") > 0 {
            // `long double` is 64 bits in this ABI, and says so in its spelling.
            let name = if count("long") > 0 {
                "long double"
            } else {
                "double"
            };
            specs.ty = Some(self.types.fundamental(name).unwrap());
            return Some(specs);
        }
        if words.is_empty() {
            let span = self.span();
            self.error(span, "expected a type specifier");
            specs.ty = Some(self.types.error());
            return Some(specs);
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
        Some(specs)
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
                self.skip_to_semicolon();
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

    /// A declarator: leading `*`s, a name, then suffixes.
    ///
    /// Grouped declarators - `int (*fp)(int)`, `int (*a)[3]` - are not read yet (M3.3), and
    /// are diagnosed rather than mis-parsed: the type they declare depends on which side of the
    /// parentheses each suffix binds to, and getting that wrong silently is worse than saying
    /// "not yet".
    fn declarator(&mut self, base: TypeId) -> Option<Declarator> {
        if !self.enter_nesting() {
            return None;
        }
        let mut ty = base;
        while self.eat("*") {
            ty = self.types.pointer(ty);
            while self
                .peek()
                .is_some_and(|token| matches!(self.spell(token), "const" | "volatile" | "restrict"))
            {
                self.bump();
            }
        }
        let name = if self.at_punct("(") {
            let span = self.span();
            self.not_implemented(span, "grouped declarators are not implemented yet (M3.3)");
            let recovered = self.skip_balanced_parens();
            self.leave_nesting();
            return Some(Declarator::Refused(recovered));
        } else if self
            .peek()
            .is_some_and(|token| token.kind == crate::lex::TokenKind::Ident)
        {
            Some(self.bump().expect("peeked").text)
        } else {
            None
        };
        let ty = self.declarator_suffixes(ty);
        self.leave_nesting();
        Some(Declarator::Read(name, ty))
    }

    fn declarator_suffixes(&mut self, mut ty: TypeId) -> TypeId {
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
                ty = self.types.array(ty, count);
                continue;
            }
            if self.eat("(") {
                let mut params = Vec::new();
                let mut variadic = false;
                if self.eat(")") {
                    ty = self.types.function(ty, params, variadic);
                    continue;
                }
                // Parameters live in a scope of their own (C99 6.2.1p4), which is why
                // `int f(int a, int a)` is an error and why these names vanish afterwards.
                self.symbols.push();
                if self.at_word("void") && self.peek_at(1).is_some_and(|t| self.is_punct(t, ")")) {
                    self.bump();
                    self.eat(")");
                    self.symbols.pop();
                    ty = self.types.function(ty, params, variadic);
                    continue;
                }
                loop {
                    if self.eat("...") {
                        variadic = true;
                        break;
                    }
                    let Some(specs) = self.declaration_specifiers() else {
                        break;
                    };
                    let base = specs.ty.unwrap_or_else(|| self.types.error());
                    let ty = match self.declarator(base) {
                        Some(Declarator::Refused(_)) => {
                            // Skip what is left of this parameter - `[3]` after a refused
                            // `(*p)`, say - so the loop can find its `,` or `)`.
                            self.skip_to_parameter_boundary();
                            base
                        }
                        Some(Declarator::Read(name, parameter)) => {
                            if let Some(name) = name {
                                let span = self.span();
                                let entry = Entry {
                                    ty: parameter,
                                    kind: Kind::Variable,
                                    span,
                                };
                                if self.symbols.declare(name, entry).is_err() {
                                    let spelling = self.spell_of(name).to_owned();
                                    self.error(span, format!("duplicate parameter `{spelling}`"));
                                }
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
                ty = self.types.function(ty, params, variadic);
                continue;
            }
            return ty;
        }
    }

    /// The value of a constant expression, or `None` after a diagnostic.
    ///
    /// This is deliberately *not* the preprocessor's evaluator: that one works on `#if` tokens
    /// with no types, while this one is typed, knows `sizeof` and enumeration constants, and
    /// will grow casts. Two evaluators with different knowledge, not one with a flag.
    fn constant_expression(&mut self) -> Option<i64> {
        if !self.enter_nesting() {
            return None;
        }
        let value = self.conditional_expression();
        self.leave_nesting();
        value.and_then(|value| match value {
            Constant::Integer(value) => Some(value),
            Constant::NotConstant => {
                let span = self.span();
                self.error(span, "expected a constant expression");
                None
            }
        })
    }

    fn conditional_expression(&mut self) -> Option<Constant> {
        let condition = self.binary_expression(0)?;
        if !self.eat("?") {
            return Some(condition);
        }
        let taken = condition.truth();
        let left = self.conditional_expression();
        if !self.eat(":") {
            self.error_at_current("expected `:`");
            return None;
        }
        let right = self.conditional_expression();
        Some(if taken == Some(true) {
            left.unwrap_or(Constant::NotConstant)
        } else if taken == Some(false) {
            right.unwrap_or(Constant::NotConstant)
        } else {
            Constant::NotConstant
        })
    }

    /// Binary operators by precedence, left-associative (C99 6.5.5 to 6.5.12).
    fn binary_expression(&mut self, minimum: usize) -> Option<Constant> {
        const LEVELS: [&[&str]; 10] = [
            &["||"],
            &["&&"],
            &["|"],
            &["^"],
            &["&"],
            &["==", "!="],
            &["<", ">", "<=", ">="],
            &["<<", ">>"],
            &["+", "-"],
            &["*", "/", "%"],
        ];
        let mut left = self.unary_expression()?;
        loop {
            let Some(operator) = self.peek().copied() else {
                return Some(left);
            };
            let spelling = self.spell(&operator).to_owned();
            let Some(level) = LEVELS
                .iter()
                .position(|operators| operators.contains(&spelling.as_str()))
            else {
                return Some(left);
            };
            if level < minimum {
                return Some(left);
            }
            self.bump();
            let right = self.binary_expression(level + 1)?;
            left = apply(&spelling, left, right).unwrap_or(Constant::NotConstant);
        }
    }

    fn unary_expression(&mut self) -> Option<Constant> {
        if self.eat("-") {
            return Some(self.unary_expression()?.negate());
        }
        if self.eat("+") {
            return self.unary_expression();
        }
        if self.eat("~") {
            return Some(self.unary_expression()?.bit_not());
        }
        if self.eat("!") {
            return Some(self.unary_expression()?.logical_not());
        }
        if self.at_word("sizeof") {
            self.bump();
            if self.at_punct("(") && self.looks_like_type_name() {
                self.bump();
                let ty = self.type_name();
                if !self.eat(")") {
                    self.error_at_current("expected `)`");
                }
                return Some(match self.types.size_of(ty) {
                    Some(size) => Constant::Integer(size as i64),
                    None => {
                        let span = self.span();
                        self.error(span, "`sizeof` needs a complete type");
                        Constant::NotConstant
                    }
                });
            }
            // `sizeof` an expression: the expression checker arrives at M3.2.
            self.not_implemented(
                self.span(),
                "`sizeof` an expression is not implemented yet (M3.2)",
            );
            if self.at_punct("(") {
                self.skip_balanced_parens();
            } else {
                self.bump();
            }
            return Some(Constant::NotConstant);
        }
        self.primary_expression()
    }

    fn primary_expression(&mut self) -> Option<Constant> {
        let token = self.peek().copied()?;
        if self.is_punct(&token, "(") {
            self.bump();
            let value = self.conditional_expression();
            if !self.eat(")") {
                self.error_at_current("expected `)`");
            }
            return value;
        }
        match token.kind {
            crate::lex::TokenKind::Number => {
                self.bump();
                Some(integer_constant(self.spell(&token)))
            }
            crate::lex::TokenKind::Chr => {
                self.bump();
                Some(match character_constant(self.spell(&token)) {
                    Some(value) => Constant::Integer(value),
                    None => Constant::NotConstant,
                })
            }
            crate::lex::TokenKind::Ident => {
                let entry = self.symbols.lookup(token.text);
                match entry {
                    Some(Entry {
                        kind: Kind::EnumConst(value),
                        ..
                    }) => {
                        let value = *value;
                        self.bump();
                        Some(Constant::Integer(value))
                    }
                    _ => {
                        self.bump();
                        Some(Constant::NotConstant)
                    }
                }
            }
            _ => {
                // Not the start of an expression: do not consume it, so the caller reports the
                // real problem at the token that caused it.
                None
            }
        }
    }

    /// Whether the tokens at the cursor start a type name (`(int)`, `(unsigned char *)`).
    fn looks_like_type_name(&self) -> bool {
        let mut ahead = 1; // past the `(`
        while let Some(token) = self.peek_at(ahead) {
            let word = self.spell(token);
            let known = matches!(
                word,
                "void"
                    | "_Bool"
                    | "char"
                    | "short"
                    | "int"
                    | "long"
                    | "float"
                    | "double"
                    | "signed"
                    | "unsigned"
                    | "const"
                    | "volatile"
                    | "restrict"
            ) || self.symbols.typedef(token.text).is_some();
            if !known {
                return false;
            }
            ahead += 1;
            if self.peek_at(ahead).is_some_and(|t| self.is_punct(t, "*")) {
                return true;
            }
            if self.peek_at(ahead).is_some_and(|t| self.is_punct(t, ")")) {
                return true;
            }
        }
        false
    }

    /// A type name, for `sizeof(...)`: specifiers and an abstract declarator.
    fn type_name(&mut self) -> TypeId {
        match self.declaration_specifiers() {
            Some(specs) => {
                let base = specs.ty.unwrap_or_else(|| self.types.error());
                match self.declarator(base) {
                    Some(Declarator::Read(_, ty)) => ty,
                    _ => base,
                }
            }
            None => self.types.error(),
        }
    }

    /// `= <constant>` for a scalar, with a diagnostic for what is not supported yet.
    fn initializer(&mut self, ty: TypeId) {
        if self.at_punct("{") {
            self.not_implemented(
                self.span(),
                "initialiser lists are not implemented yet (M3.3)",
            );
            self.skip_balanced_block();
            return;
        }
        let token = self.peek().copied();
        let Some(token) = token else { return };
        if token.kind == crate::lex::TokenKind::Str {
            self.bump();
            return; // a string literal initialising an array or a pointer
        }
        // Read, not checked: whether a value fits the declared type is a question about the
        // expression's type, which the expression checker answers at M3.2. Whatever is left of
        // the expression - a call's arguments, say - is skipped so that the declaration still
        // ends at its `;`.
        if self.conditional_expression().is_none() {
            let span = self.span();
            self.error(span, "expected an initialiser");
        }
        self.skip_expression_remainder();
        let _ = ty;
    }

    // ---- recovery -------------------------------------------------------------

    fn error_at_current(&mut self, message: impl Into<String>) {
        let span = self.span();
        self.error(span, message);
    }

    fn skip_before_semicolon(&mut self) {
        while let Some(token) = self.peek() {
            if self.is_punct(token, ";") || self.is_punct(token, "}") || self.at_eof() {
                return;
            }
            self.bump();
        }
    }

    fn skip_to_semicolon(&mut self) {
        while let Some(token) = self.peek() {
            if self.is_punct(token, ";") || self.at_eof() {
                self.bump();
                return;
            }
            if self.is_punct(token, "}") {
                return;
            }
            self.bump();
        }
    }

    /// Skip the rest of an expression up to the `,` or `;` that ends it.
    fn skip_expression_remainder(&mut self) {
        let mut depth = 0usize;
        while let Some(token) = self.peek() {
            match self.spell(token) {
                "(" | "[" | "{" => depth += 1,
                ")" | "]" | "}" => {
                    if depth == 0 {
                        return;
                    }
                    depth -= 1;
                }
                "," | ";" if depth == 0 => return,
                _ => {}
            }
            if self.at_eof() {
                return;
            }
            self.bump();
        }
    }

    /// Skip to the `,` or `)` that ends a parameter, without consuming it.
    fn skip_to_parameter_boundary(&mut self) {
        let mut depth = 0usize;
        while let Some(token) = self.peek() {
            if self.is_punct(token, "(") || self.is_punct(token, "[") || self.is_punct(token, "{") {
                depth += 1;
            } else if self.is_punct(token, ")")
                || self.is_punct(token, "]")
                || self.is_punct(token, "}")
            {
                if depth == 0 {
                    return;
                }
                depth -= 1;
            } else if (self.is_punct(token, ",") && depth == 0) || self.at_eof() {
                return;
            }
            self.bump();
        }
    }

    /// Skip a balanced `( ... )` group, and return the one name inside it, if there is one:
    /// `(*FP)` names `FP`, which the caller can then declare so later uses stay quiet.
    fn skip_balanced_parens(&mut self) -> Option<Symbol> {
        let mut depth = 0usize;
        let mut name = None;
        let mut names = 0;
        while let Some(token) = self.peek().copied() {
            if self.is_punct(&token, "(") {
                depth += 1;
            } else if self.is_punct(&token, ")") {
                depth = depth.saturating_sub(1);
                self.bump();
                if depth == 0 {
                    return if names == 1 { name } else { None };
                }
                continue;
            } else if self.at_eof() {
                return None;
            } else if token.kind == crate::lex::TokenKind::Ident {
                names += 1;
                name = Some(token.text);
            }
            self.bump();
        }
        None
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

    fn skip_struct_specifier(&mut self) {
        // `struct` `union` then a tag or a body: skip to the semicolon if there is a body.
        self.bump();
        if self
            .peek()
            .is_some_and(|token| token.kind == crate::lex::TokenKind::Ident)
        {
            self.bump();
        }
        if self.at_punct("{") {
            self.skip_balanced_block();
        }
    }

    // ---- nesting budget -------------------------------------------------------

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

/// What one declarator declared.
#[derive(Debug)]
struct Declared {
    ty: TypeId,
}

/// What parsing one declarator produced.
#[derive(Debug)]
enum Declarator {
    /// A declarator was read: maybe with a name (an abstract declarator has none).
    Read(Option<Symbol>, TypeId),
    /// The declarator used a construct M3 has not built, and was refused.
    ///
    /// The name is recovered when it can be, and declared with the error type: a later
    /// `FP gp;` that uses an alias we could not read is then quiet, instead of one refusal
    /// becoming a cascade down the file.
    Refused(Option<Symbol>),
}

/// The value of a constant expression, or the knowledge that it is not one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Constant {
    Integer(i64),
    /// An expression that would only have a value at run time, or could not be read.
    NotConstant,
}

impl Constant {
    fn truth(self) -> Option<bool> {
        match self {
            Constant::Integer(value) => Some(value != 0),
            Constant::NotConstant => None,
        }
    }

    fn integer(self) -> Option<i64> {
        match self {
            Constant::Integer(value) => Some(value),
            Constant::NotConstant => None,
        }
    }

    fn negate(self) -> Self {
        match self.integer() {
            Some(value) => Constant::Integer(value.wrapping_neg()),
            None => Constant::NotConstant,
        }
    }

    fn bit_not(self) -> Self {
        match self.integer() {
            Some(value) => Constant::Integer(!value),
            None => Constant::NotConstant,
        }
    }

    fn logical_not(self) -> Self {
        match self.truth() {
            Some(value) => Constant::Integer(i64::from(!value)),
            None => Constant::NotConstant,
        }
    }
}

fn apply(operator: &str, left: Constant, right: Constant) -> Option<Constant> {
    let (Some(left), Some(right)) = (left.integer(), right.integer()) else {
        return Some(Constant::NotConstant);
    };
    let value = match operator {
        "+" => left.wrapping_add(right),
        "-" => left.wrapping_sub(right),
        "*" => left.wrapping_mul(right),
        "/" => {
            if right == 0 {
                return Some(Constant::NotConstant);
            }
            left.wrapping_div(right)
        }
        "%" => {
            if right == 0 {
                return Some(Constant::NotConstant);
            }
            left.wrapping_rem(right)
        }
        "<<" => left.wrapping_shl(right as u32),
        ">>" => left.wrapping_shr(right as u32),
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
    };
    Some(Constant::Integer(value))
}

/// An integer constant from its spelling, or `NotConstant` if it is not one this front end
/// reads yet (floats, suffixes it does not know).
fn integer_constant(spelling: &str) -> Constant {
    let lower = spelling.to_ascii_lowercase();
    let digits = lower.trim_end_matches(['u', 'l']);
    let value = if let Some(hex) = digits.strip_prefix("0x") {
        u64::from_str_radix(hex, 16).ok()
    } else if digits.len() > 1 && digits.starts_with('0') {
        u64::from_str_radix(&digits[1..], 8).ok()
    } else {
        digits.parse::<u64>().ok()
    };
    match value {
        Some(value) => Constant::Integer(value as i64),
        None => Constant::NotConstant,
    }
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
