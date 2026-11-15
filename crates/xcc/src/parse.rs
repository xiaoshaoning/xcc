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
use crate::options::Budgets;
use crate::pp::{PpToken, Preprocessed};
use crate::symtab::{Entry, Kind, Symbols};
use crate::types::{Type, TypeId, Types};

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
    /// The parameter names of the last function declarator, which a definition declares in its
    /// body's scope.
    last_parameters: Vec<(Option<Symbol>, TypeId)>,
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
                    self.skip_until(&[",", ";"], false);
                    if self.eat(",") {
                        continue;
                    }
                    return Some(self.types.error());
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

    /// What follows a declarator: `[count]` and `(parameters)`, in any order.
    fn declarator_suffixes(&mut self, mut ty: TypeId) -> TypeId {
        loop {
            if self.eat("[") {
                // An empty size is an incomplete array (`int a[]`), and its count is filled in
                // by the initialiser once there is one (M3.3).
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
                let (params, variadic) = self.parameter_list();
                ty = self.types.function(ty, params, variadic);
                continue;
            }
            return ty;
        }
    }

    /// The parameters of a function declarator, from inside the parentheses to past the `)`.
    ///
    /// They live in a scope of their own (C99 6.2.1p4), which is why `int f(int a, int a)` is
    /// an error and why these names are gone afterwards.
    fn parameter_list(&mut self) -> (Vec<TypeId>, bool) {
        let mut params = Vec::new();
        self.last_parameters.clear();
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
            let ty = match self.declarator(base) {
                Some(Declarator::Refused(recovered)) => {
                    // Skip what is left of this parameter - `[3]` after a refused `(*p)`, say -
                    // so the loop can find its `,` or `)`.
                    self.skip_until(&[",", ")"], false);
                    if let Some(name) = recovered {
                        // Declared anyway, with the error type: the body's uses of it are then
                        // quiet instead of one refusal becoming five.
                        self.declare_parameter(name, self.types.error());
                    }
                    self.types.error()
                }
                Some(Declarator::Read(name, parameter)) => {
                    // `int a[][3]` as a parameter is a pointer to `int[3]`, not an array.
                    let parameter = self.types.decay(parameter);
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
        self.last_parameters.push((Some(name), ty));
        let span = self.span();
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

    // ---- expressions ----------------------------------------------------------
    //
    // One parser for every expression, whether a constant is wanted or not: the value it
    // produces carries its type, its constant when it has one, and whether it can be assigned
    // to. `constant_expression` is then the same parser plus one question, instead of a second
    // folding pass that would have to agree with this one (ADR-002: at M4 these values become
    // `cir` values, and there is still only one parser).

    /// The value of a constant expression, or `None` after a diagnostic.
    ///
    /// Deliberately not the preprocessor's evaluator: that one works on `#if` tokens with no
    /// types and no symbols. This one knows the type of everything it folds.
    fn constant_expression(&mut self) -> Option<i64> {
        if !self.enter_nesting() {
            return None;
        }
        let value = self.conditional_expression();
        self.leave_nesting();
        match value {
            Some(value) => match value.constant {
                Some(constant) => Some(constant),
                None => {
                    let span = self.span();
                    self.error(span, "expected a constant expression");
                    None
                }
            },
            None => None,
        }
    }

    /// A whole expression, comma operators and all.
    fn expression(&mut self) -> Option<Value> {
        let mut value = self.assignment_expression()?;
        while self.eat(",") {
            value = self.assignment_expression()?;
        }
        Some(value)
    }

    fn assignment_expression(&mut self) -> Option<Value> {
        let left = self.conditional_expression()?;
        const COMPOUND: [&str; 10] = ["+=", "-=", "*=", "/=", "%=", "<<=", ">>=", "&=", "^=", "|="];
        let operator = self.peek().map(|token| self.spell(token).to_owned());
        match operator.as_deref() {
            Some("=") => {
                let span = self.span();
                self.bump();
                let right = self.assignment_expression()?;
                self.assign(left, right, span)
            }
            Some(spelling) if COMPOUND.contains(&spelling) => {
                let span = self.span();
                let spelling = spelling.to_owned();
                self.bump();
                let right = self.assignment_expression()?;
                let base = spelling.trim_end_matches('=').to_owned();
                let result = self.binary_result(&base, left, right, span);
                self.assign(left, result, span)
            }
            _ => Some(left),
        }
    }

    /// The value of `left = right`, with the conversions C allows.
    fn assign(&mut self, left: Value, right: Value, span: Span) -> Option<Value> {
        let right = self.decay(right);
        if self.types.is_error(left.ty) || self.types.is_error(right.ty) {
            return Some(Value::temporary(left.ty));
        }
        if !left.lvalue {
            self.error(span, "cannot assign to this expression");
            return Some(Value::temporary(left.ty));
        }
        let null_pointer = self.types.is_pointer(left.ty) && right.constant == Some(0);
        if !null_pointer && !self.types.is_assignable(left.ty, right.ty) {
            let (to, from) = (self.types.describe(left.ty), self.types.describe(right.ty));
            self.error(span, format!("cannot assign {from} to {to}"));
            return Some(Value::temporary(left.ty));
        }
        Some(Value::temporary(left.ty))
    }

    fn conditional_expression(&mut self) -> Option<Value> {
        let condition = self.binary_expression(0)?;
        if !self.eat("?") {
            return Some(condition);
        }
        self.require_scalar(&condition, "the condition of `?:`");
        let when_true = self.expression()?;
        if !self.eat(":") {
            self.error_at_current("expected `:`");
            return None;
        }
        let when_false = self.conditional_expression()?;
        let ty = self.types.usual_arithmetic(when_true.ty, when_false.ty);
        let constant = match (when_true.constant, when_false.constant) {
            (Some(t), Some(f)) => condition
                .constant
                .map(|condition| if condition != 0 { t } else { f }),
            (Some(t), None) if condition.constant == Some(1) => Some(t),
            (None, Some(f)) if condition.constant == Some(0) => Some(f),
            _ => None,
        };
        Some(Value {
            ty,
            constant,
            lvalue: false,
        })
    }

    /// Binary operators by precedence, left-associative (C99 6.5.5 to 6.5.12).
    fn binary_expression(&mut self, minimum: usize) -> Option<Value> {
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
            let Some(operator) = self.peek().map(|token| self.spell(token).to_owned()) else {
                return Some(left);
            };
            let Some(level) = LEVELS
                .iter()
                .position(|operators| operators.contains(&operator.as_str()))
            else {
                return Some(left);
            };
            if level < minimum {
                return Some(left);
            }
            let span = self.span();
            self.bump();
            let right = self.binary_expression(level + 1)?;
            left = self.binary_result(&operator, left, right, span);
        }
    }

    /// The value of `left <operator> right`, typed the way C says and folded when it can be.
    ///
    /// A type error is *poisoned*: the result is the error type, which is compatible with
    /// everything, so `1 + "text" + 2` is one diagnostic rather than three.
    fn binary_result(&mut self, operator: &str, left: Value, right: Value, span: Span) -> Value {
        let left = self.decay(left);
        let right = self.decay(right);
        if self.types.is_error(left.ty) || self.types.is_error(right.ty) {
            return Value::temporary(self.types.error());
        }
        let arithmetic = self.types.is_arithmetic(left.ty) && self.types.is_arithmetic(right.ty);
        let integer = self.types.is_integer(left.ty) && self.types.is_integer(right.ty);
        let integral = self.types.is_integer(left.ty) && self.types.is_integer(right.ty);
        let folded = |left: Value, right: Value| match (left.constant, right.constant) {
            (Some(a), Some(b)) if integral => fold(operator, a, b),
            _ => None,
        };
        let common = || self.types.usual_arithmetic(left.ty, right.ty);

        match operator {
            "+" | "-" => {
                if arithmetic {
                    return Value {
                        ty: common(),
                        constant: folded(left, right),
                        lvalue: false,
                    };
                }
                // `pointer + integer` and `integer + pointer` are the same expression; adding
                // two pointers is not (and subtracting them is).
                let pointer_side = if self.types.is_pointer(left.ty) {
                    Some((left.ty, self.types.is_integer(right.ty)))
                } else if self.types.is_pointer(right.ty) {
                    Some((right.ty, self.types.is_integer(left.ty)))
                } else {
                    None
                };
                if operator == "+" {
                    if let Some((pointer_ty, other_is_integer)) = pointer_side {
                        if other_is_integer {
                            return Value::temporary(pointer_ty);
                        }
                    }
                }
                if operator == "-"
                    && self.types.is_pointer(left.ty)
                    && self.types.is_integer(right.ty)
                {
                    return Value::temporary(left.ty);
                }
                if operator == "-"
                    && self.types.is_pointer(left.ty)
                    && self.types.is_pointer(right.ty)
                {
                    return Value::temporary(self.types.fundamental("long long").unwrap());
                }
                self.bad_operands(operator, left.ty, right.ty, span)
            }
            "*" | "/" => {
                if arithmetic {
                    return Value {
                        ty: common(),
                        constant: folded(left, right),
                        lvalue: false,
                    };
                }
                self.bad_operands(operator, left.ty, right.ty, span)
            }
            "%" | "<<" | ">>" | "&" | "^" | "|" => {
                if integer {
                    let ty =
                        if operator == "%" || operator == "&" || operator == "^" || operator == "|"
                        {
                            common()
                        } else {
                            self.promote(left.ty)
                        };
                    return Value {
                        ty,
                        constant: folded(left, right),
                        lvalue: false,
                    };
                }
                self.bad_operands(operator, left.ty, right.ty, span)
            }
            "<" | ">" | "<=" | ">=" | "==" | "!=" => {
                let comparable = (arithmetic
                    || self.types.is_pointer(left.ty) && self.types.is_pointer(right.ty))
                    || (self.types.is_pointer(left.ty) && right.constant == Some(0))
                    || (self.types.is_pointer(right.ty) && left.constant == Some(0));
                if comparable {
                    return Value {
                        ty: self.types.fundamental("int").unwrap(),
                        constant: folded(left, right),
                        lvalue: false,
                    };
                }
                self.bad_operands(operator, left.ty, right.ty, span)
            }
            "&&" | "||" => {
                self.require_scalar(&left, "the left side of a logical operator");
                self.require_scalar(&right, "the right side of a logical operator");
                let int = self.types.fundamental("int").unwrap();
                // `0 && anything` is 0 whatever the right side turns out to be at run time.
                let constant = match (operator, left.constant, right.constant) {
                    ("&&", Some(0), _) => Some(0),
                    ("||", Some(c), _) if c != 0 => Some(1),
                    ("&&", Some(_), Some(b)) => Some(i64::from(b != 0)),
                    ("||", Some(_), Some(b)) => Some(i64::from(b != 0)),
                    _ => None,
                };
                Value {
                    ty: int,
                    constant,
                    lvalue: false,
                }
            }
            other => {
                self.error(
                    span,
                    format!("`{other}` is not an operator this front end knows"),
                );
                Value::temporary(self.types.error())
            }
        }
    }

    fn bad_operands(&mut self, operator: &str, left: TypeId, right: TypeId, span: Span) -> Value {
        let (left, right) = (self.types.describe(left), self.types.describe(right));
        self.error(
            span,
            format!("`{operator}` cannot be applied to {left} and {right}"),
        );
        Value::temporary(self.types.error())
    }

    /// Integer promotion (C99 6.3.1.1): anything narrower than `int` becomes `int`.
    fn promote(&self, ty: TypeId) -> TypeId {
        match self.types.kind(ty) {
            Type::Int { bits, .. } if *bits < 32 => self.types.fundamental("int").unwrap(),
            Type::Bool => self.types.fundamental("int").unwrap(),
            _ => ty,
        }
    }

    /// An array or function used as a value is a pointer to it (C99 6.3.2.1). Everything that
    /// *uses* a value calls this; a place that assigns to one does not.
    fn decay(&mut self, value: Value) -> Value {
        let ty = self.types.decay(value.ty);
        if ty == value.ty {
            return value;
        }
        Value::temporary(ty)
    }

    fn require_scalar(&mut self, value: &Value, what: &str) {
        if self.types.is_error(value.ty) {
            return;
        }
        if !(self.types.is_arithmetic(value.ty) || self.types.is_pointer(value.ty)) {
            let ty = self.types.describe(value.ty);
            let span = self.span();
            self.error(span, format!("{what} must be a scalar, not {ty}"));
        }
    }

    fn unary_expression(&mut self) -> Option<Value> {
        let operator = self.peek().map(|token| self.spell(token).to_owned())?;
        let span = self.span();
        match operator.as_str() {
            "-" | "+" | "~" | "!" => {
                self.bump();
                let value = self.unary_expression()?;
                let ty = value.ty;
                match operator.as_str() {
                    "-" => {
                        if !self.types.is_arithmetic(ty) {
                            let ty = self.types.describe(ty);
                            self.error(span, format!("unary `-` needs a number, not {ty}"));
                            return Some(Value::temporary(self.types.error()));
                        }
                        Some(Value {
                            ty: self.promote(ty),
                            constant: value.constant.map(|value| value.wrapping_neg()),
                            lvalue: false,
                        })
                    }
                    "+" => Some(Value {
                        ty: self.promote(ty),
                        constant: value.constant,
                        lvalue: false,
                    }),
                    "~" => {
                        if !self.types.is_integer(ty) {
                            let ty = self.types.describe(ty);
                            self.error(span, format!("`~` needs an integer, not {ty}"));
                            return Some(Value::temporary(self.types.error()));
                        }
                        Some(Value {
                            ty: self.promote(ty),
                            constant: value.constant.map(|value| !value),
                            lvalue: false,
                        })
                    }
                    _ => {
                        self.require_scalar(&value, "`!`");
                        Some(Value {
                            ty: self.types.fundamental("int").unwrap(),
                            constant: value.constant.map(|value| i64::from(value == 0)),
                            lvalue: false,
                        })
                    }
                }
            }
            "&" => {
                self.bump();
                let value = self.unary_expression()?;
                if self.types.is_error(value.ty) {
                    return Some(Value::temporary(self.types.error()));
                }
                if !value.lvalue {
                    self.error(span, "`&` needs something it can take the address of");
                    return Some(Value::temporary(self.types.error()));
                }
                Some(Value::temporary(self.types.pointer(value.ty)))
            }
            "*" => {
                self.bump();
                let value = self.unary_expression().map(|value| self.decay(value))?;
                if self.types.is_error(value.ty) {
                    return Some(Value::temporary(self.types.error()));
                }
                if !self.types.is_pointer(value.ty) {
                    let ty = self.types.describe(value.ty);
                    self.error(span, format!("`*` needs a pointer, not {ty}"));
                    return Some(Value::temporary(self.types.error()));
                }
                Some(Value {
                    ty: self.types.pointee(value.ty),
                    constant: None,
                    lvalue: true,
                })
            }
            "++" | "--" => {
                self.bump();
                let value = self.unary_expression()?;
                Some(self.increment(value, span))
            }
            "sizeof" => {
                self.bump();
                Some(self.sizeof_expression())
            }
            _ => self.postfix_expression(),
        }
    }

    fn increment(&mut self, value: Value, span: Span) -> Value {
        if self.types.is_error(value.ty) {
            return Value::temporary(value.ty);
        }
        let scalar = self.types.is_arithmetic(value.ty) || self.types.is_pointer(value.ty);
        if !value.lvalue || !scalar {
            self.error(span, "`++` and `--` need a variable holding a number");
            return Value::temporary(self.types.error());
        }
        Value::temporary(value.ty)
    }

    /// `sizeof` a type name or an expression, as a constant of type `size_t` (here `unsigned
    /// long long`, the widest integer this front end has).
    fn sizeof_expression(&mut self) -> Value {
        let size_type = self.types.fundamental("unsigned long long").unwrap();
        if self.at_punct("(") {
            self.bump();
            if let Some(ty) = self.try_type_name() {
                if !self.eat(")") {
                    self.error_at_current("expected `)`");
                }
                if self.types.is_error(ty) {
                    return Value::temporary(self.types.error());
                }
                return match self.types.size_of(ty) {
                    Some(size) => Value::constant(size_type, size as i64),
                    None => {
                        let typo = self.types.describe(ty);
                        let span = self.span();
                        self.error(span, format!("`sizeof` needs a complete type, not {typo}"));
                        Value::temporary(self.types.error())
                    }
                };
            }
            // Not a type name: put the parenthesis back, and read it as an expression.
            self.at -= 1;
        }
        let Some(value) = self.unary_expression() else {
            return Value::temporary(self.types.error());
        };
        if self.types.is_error(value.ty) {
            return Value::temporary(self.types.error());
        }
        match self.types.size_of(value.ty) {
            Some(size) => Value::constant(size_type, size as i64),
            None => {
                let ty = self.types.describe(value.ty);
                let span = self.span();
                self.error(span, format!("`sizeof` needs a complete type, not {ty}"));
                Value::temporary(self.types.error())
            }
        }
    }

    fn postfix_expression(&mut self) -> Option<Value> {
        let mut value = self.primary_expression()?;
        loop {
            let span = self.span();
            if self.eat("[") {
                let index = self.expression()?;
                if !self.eat("]") {
                    self.error_at_current("expected `]`");
                }
                value = self.subscript(value, index, span);
                continue;
            }
            if self.at_punct("(") {
                self.bump();
                value = self.call(value, span);
                continue;
            }
            if self.at_punct(".") || self.at_punct("->") {
                self.not_implemented(span, "struct members are not implemented yet (M3.3)");
                self.skip_until(&[";"], false);
                return Some(Value::temporary(self.types.error()));
            }
            if self.at_punct("++") || self.at_punct("--") {
                self.bump();
                value = self.increment(value, span);
                continue;
            }
            return Some(value);
        }
    }

    fn subscript(&mut self, base: Value, index: Value, span: Span) -> Value {
        let base = self.decay(base);
        if self.types.is_error(base.ty) || self.types.is_error(index.ty) {
            return Value::temporary(self.types.error());
        }
        if !self.types.is_integer(index.ty) {
            let ty = self.types.describe(index.ty);
            self.error(span, format!("a subscript must be an integer, not {ty}"));
            return Value::temporary(self.types.error());
        }
        match self.types.kind(base.ty) {
            Type::Pointer(element) => {
                let element = *element;
                Value {
                    ty: element,
                    constant: None,
                    lvalue: true,
                }
            }
            Type::Array { element, .. } => {
                let element = *element;
                Value {
                    ty: element,
                    constant: None,
                    lvalue: true,
                }
            }
            _ => {
                let ty = self.types.describe(base.ty);
                self.error(span, format!("`[]` needs a pointer or an array, not {ty}"));
                Value::temporary(self.types.error())
            }
        }
    }

    /// A call: the arguments are parsed and checked against the declared parameters.
    fn call(&mut self, callee: Value, span: Span) -> Value {
        if self.types.is_error(callee.ty) {
            self.skip_until(&[")"], true);
            return Value::temporary(self.types.error());
        }
        let function = match self.types.kind(callee.ty) {
            Type::Function { .. } => callee.ty,
            Type::Pointer(to) if matches!(self.types.kind(*to), Type::Function { .. }) => *to,
            _ => {
                let ty = self.types.describe(callee.ty);
                self.error(span, format!("cannot call {ty}"));
                self.skip_until(&[")"], true);
                return Value::temporary(self.types.error());
            }
        };
        let (result, params, variadic) = match self.types.kind(function) {
            Type::Function {
                result,
                params,
                variadic,
            } => (*result, params.clone(), *variadic),
            _ => unreachable!("checked above"),
        };

        let mut given = 0usize;
        if !self.eat(")") {
            loop {
                let argument = self.assignment_expression().map(|value| self.decay(value));
                let Some(argument) = argument else { break };
                if self.types.is_error(argument.ty) {
                    given += 1;
                    if !self.eat(",") {
                        break;
                    }
                    continue;
                }
                if given < params.len() {
                    let expected = params[given];
                    let null_pointer =
                        self.types.is_pointer(expected) && argument.constant == Some(0);
                    if !null_pointer && !self.types.is_assignable(expected, argument.ty) {
                        let (from, to) = (
                            self.types.describe(argument.ty),
                            self.types.describe(expected),
                        );
                        self.error(
                            span,
                            format!("argument {} is {from}, expected {to}", given + 1),
                        );
                    }
                } else if !variadic {
                    self.error(
                        span,
                        format!(
                            "this function takes {} arguments, more were given",
                            params.len()
                        ),
                    );
                }
                given += 1;
                if !self.eat(",") {
                    break;
                }
            }
            if !self.eat(")") {
                self.error_at_current("expected `)` to end the arguments");
            }
        }
        if given < params.len() {
            self.error(
                span,
                format!(
                    "this function takes {} arguments, {given} were given",
                    params.len()
                ),
            );
        }
        Value::temporary(result)
    }

    fn primary_expression(&mut self) -> Option<Value> {
        let token = self.peek().copied()?;
        let span = token.span;
        match token.kind {
            crate::lex::TokenKind::Number => {
                self.bump();
                let spelling = self.spell(&token).to_owned();
                Some(self.number_value(&spelling, span))
            }
            crate::lex::TokenKind::Chr => {
                self.bump();
                let value = character_constant(self.spell(&token)).map_or(0, |value| value);
                Some(Value::constant(
                    self.types.fundamental("int").unwrap(),
                    value,
                ))
            }
            crate::lex::TokenKind::Str => {
                self.bump();
                // Adjacent literals concatenate (C99 6.4.5p5); their bytes are M3.3's problem.
                while self
                    .peek()
                    .is_some_and(|next| next.kind == crate::lex::TokenKind::Str)
                {
                    self.bump();
                }
                let char = self.types.fundamental("char").unwrap();
                Some(Value::temporary(self.types.pointer(char)))
            }
            crate::lex::TokenKind::Punct if self.is_punct(&token, "(") => {
                // `(int)x` is a cast; `(x)` groups. Tried in that order, and the cursor is put
                // back when the parenthesis turns out to hold an expression.
                if let Some(ty) = self.cast_type() {
                    return Some(self.cast(ty, span));
                }
                self.bump();
                let value = self.expression()?;
                if !self.eat(")") {
                    self.error_at_current("expected `)`");
                }
                Some(value)
            }
            crate::lex::TokenKind::Ident => {
                self.bump();
                Some(self.named_value(token.text, span))
            }
            _ => {
                self.error(span, "expected an expression");
                None
            }
        }
    }

    /// `(type)` before an expression, or `None` if the parenthesis is not a cast.
    fn cast_type(&mut self) -> Option<TypeId> {
        if !self.at_punct("(") {
            return None;
        }
        self.bump();
        match self.try_type_name() {
            Some(ty) => {
                if !self.eat(")") {
                    self.error_at_current("expected `)` after the type in a cast");
                }
                Some(ty)
            }
            None => {
                self.at -= 1; // put the `(` back: it groups an expression
                None
            }
        }
    }

    fn cast(&mut self, to: TypeId, span: Span) -> Value {
        if self.at_punct("{") {
            // A compound literal, `(struct P){1, 2}`: refused with the rest of M3.3.
            self.not_implemented(span, "compound literals are not implemented yet (M3.3)");
            self.skip_balanced_block();
            return Value {
                ty: self.types.error(),
                constant: None,
                lvalue: true,
            };
        }
        let Some(from) = self.unary_expression() else {
            return Value::temporary(to);
        };
        let converts = self.types.is_assignable(to, from.ty)
            || self.types.is_pointer(to) && self.types.is_integer(from.ty)
            || self.types.is_integer(to) && self.types.is_pointer(from.ty)
            || matches!(self.types.kind(to), Type::Void);
        if !converts {
            let (from, to) = (self.types.describe(from.ty), self.types.describe(to));
            self.error(span, format!("cannot convert {from} to {to}"));
            return Value::temporary(self.types.error());
        }
        Value {
            ty: to,
            constant: from.constant,
            lvalue: false,
        }
    }

    /// A number literal: an integer is folded and typed by its suffix and magnitude (C99
    /// 6.4.4.1), a floating constant is only typed - folding floats is the back end's job.
    fn number_value(&mut self, spelling: &str, span: Span) -> Value {
        let lower = spelling.to_ascii_lowercase();
        let floating = lower.contains('.')
            || (lower.starts_with("0x") && lower.contains('p'))
            || (!lower.starts_with("0x") && (lower.contains('e') || lower.contains('E')));
        if floating {
            let name = if lower.ends_with('f') {
                "float"
            } else if lower.ends_with('l') {
                "long double"
            } else {
                "double"
            };
            return Value::temporary(self.types.fundamental(name).unwrap());
        }
        self.integer_value(spelling, span)
    }

    /// An integer literal, typed by its suffix and magnitude the way C99 6.4.4.1 says.
    fn integer_value(&mut self, spelling: &str, span: Span) -> Value {
        let lower = spelling.to_ascii_lowercase();
        let unsigned = lower.contains('u');
        let longs = lower.matches('l').count();
        let digits = lower.trim_end_matches(['u', 'l']);
        let parsed = if let Some(hex) = digits.strip_prefix("0x") {
            u64::from_str_radix(hex, 16).ok()
        } else if digits.len() > 1 && digits.starts_with('0') {
            u64::from_str_radix(&digits[1..], 8).ok()
        } else {
            digits.parse::<u64>().ok()
        };
        let Some(bits) = parsed else {
            self.error(
                span,
                format!("`{spelling}` is not an integer constant this front end reads"),
            );
            return Value::temporary(self.types.error());
        };
        // A decimal constant too big for `int` becomes unsigned or wider, as C says.
        let ty = if longs >= 2 {
            self.types.integer_with(!unsigned && (bits as i64) >= 0, 64)
        } else if unsigned || bits > i64::from(i32::MAX) as u64 {
            self.types.integer_with(false, 32)
        } else {
            self.types.fundamental("int").unwrap()
        };
        Value::constant(ty, bits as i64)
    }

    /// The value of a name: a variable, a function, or an enumeration constant.
    fn named_value(&mut self, name: Symbol, span: Span) -> Value {
        let Some(entry) = self.symbols.lookup(name) else {
            let spelling = self.spell_of(name).to_owned();
            self.error(span, format!("`{spelling}` is not declared"));
            return Value::temporary(self.types.error());
        };
        match entry.kind {
            Kind::EnumConst(value) => Value::constant(entry.ty, value),
            Kind::Function => Value {
                ty: entry.ty,
                constant: None,
                lvalue: false,
            },
            Kind::Typedef => {
                let spelling = self.spell_of(name).to_owned();
                self.error(span, format!("`{spelling}` is a type, not a value"));
                Value::temporary(self.types.error())
            }
            Kind::Variable => Value {
                ty: entry.ty,
                constant: None,
                lvalue: true,
            },
        }
    }

    /// Read a type name, or put the cursor back and say there was none.
    ///
    /// This is how `sizeof (` tells `sizeof (int)` from `sizeof (x)`: by trying. The
    /// alternative is a lookahead that lists the type keywords again, and a list that has to
    /// agree with `declaration_specifiers` is a list that will stop agreeing.
    fn try_type_name(&mut self) -> Option<TypeId> {
        let before = self.at;
        let specs = self.declaration_specifiers();
        let Some(base) = specs.ty else {
            self.at = before;
            return None;
        };
        Some(match self.declarator(base) {
            Some(Declarator::Read(_, ty)) => ty,
            _ => base,
        })
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
        self.skip_until(&[",", ";"], false);
        let _ = ty;
    }

    /// A function definition's body: the parameters live in its outermost block (C99 6.2.1p4),
    /// so the block is parsed without pushing a scope of its own.
    fn function_body(&mut self, result: TypeId) {
        self.function_result = Some(result);
        self.loops = 0;
        self.symbols.push();
        for (name, ty) in std::mem::take(&mut self.last_parameters) {
            if let Some(name) = name {
                let span = self.span();
                let _ = self.symbols.declare(
                    name,
                    Entry {
                        ty,
                        kind: Kind::Variable,
                        span,
                    },
                );
            }
        }
        self.block_body();
        self.symbols.pop();
        self.function_result = None;
    }

    // ---- statements ------------------------------------------------------------
    //
    // A statement is parsed for what it does to the types: `if (p)` needs a scalar, `return`
    // needs to match the function's result, `break` needs a loop. At M4 each of these becomes
    // `cir` control flow as well; there is no tree in between (ADR-002).

    /// One statement. The cursor is at its first token.
    fn statement(&mut self) {
        if !self.enter_nesting() {
            self.skip_statement();
            return;
        }
        if self.eat(";") {
            self.leave_nesting();
            return;
        }
        if self.at_punct("{") {
            self.block();
            self.leave_nesting();
            return;
        }
        if self
            .peek()
            .is_some_and(|token| token.kind == crate::lex::TokenKind::Ident)
            && self.peek_at(1).is_some_and(|next| self.is_punct(next, ":"))
        {
            let span = self.span();
            self.not_implemented(span, "labels and `goto` are not implemented yet (M3.3)");
            self.bump();
            self.bump();
            self.statement();
            self.leave_nesting();
            return;
        }
        let word = self.peek().map(|token| self.spell(token).to_owned());
        match word.as_deref() {
            Some("if") => {
                self.if_statement();
                self.leave_nesting();
                return;
            }
            Some("while") => {
                self.while_statement();
                self.leave_nesting();
                return;
            }
            Some("do") => {
                self.do_statement();
                self.leave_nesting();
                return;
            }
            Some("for") => {
                self.for_statement();
                self.leave_nesting();
                return;
            }
            Some("return") => {
                self.return_statement();
                self.leave_nesting();
                return;
            }
            Some("break") | Some("continue") => {
                self.loop_control(word.as_deref() == Some("break"));
                self.leave_nesting();
                return;
            }
            Some("switch") | Some("case") | Some("default") | Some("goto") => {
                let span = self.span();
                let keyword = word.unwrap_or_default();
                self.not_implemented(span, format!("`{keyword}` is not implemented yet (M3.3)"));
                self.skip_statement();
                self.leave_nesting();
                return;
            }
            Some(_) if self.starts_specifier() => {
                self.declaration();
                if !self.eat(";") {
                    self.skip_until(&[";"], true);
                }
                self.leave_nesting();
                return;
            }
            _ => {}
        }
        // An expression statement: evaluated for its effect, so its value is discarded.
        if self.expression().is_some() && !self.eat(";") {
            self.error_at_current("expected `;` after an expression");
            self.skip_until(&[";"], true);
        }
        self.leave_nesting();
    }

    fn block(&mut self) {
        self.symbols.push();
        self.block_body();
        self.symbols.pop();
    }

    /// The braces and statements of a block; the caller owns its scope.
    fn block_body(&mut self) {
        if !self.eat("{") {
            return;
        }
        while !self.at_punct("}") && !self.at_eof() {
            let before = self.at;
            self.statement();
            if self.at == before {
                self.bump(); // never spin: a statement that consumed nothing is a bug
            }
        }
        if !self.eat("}") {
            self.error_at_current("expected `}`");
        }
    }

    fn if_statement(&mut self) {
        self.bump();
        let condition = self.parenthesised_condition("`if`");
        if let Some(condition) = condition {
            self.require_scalar(&condition, "the condition of `if`");
        }
        self.statement();
        if self.at_word("else") {
            self.bump();
            self.statement();
        }
    }

    fn while_statement(&mut self) {
        self.bump();
        if let Some(condition) = self.parenthesised_condition("`while`") {
            self.require_scalar(&condition, "the condition of `while`");
        }
        self.loops += 1;
        self.statement();
        self.loops -= 1;
    }

    fn do_statement(&mut self) {
        self.bump();
        self.loops += 1;
        self.statement();
        self.loops -= 1;
        if !self.at_word("while") {
            self.error_at_current("expected `while` after the body of `do`");
            return;
        }
        self.bump();
        if let Some(condition) = self.parenthesised_condition("`do ... while`") {
            self.require_scalar(&condition, "the condition of `do ... while`");
        }
        if !self.eat(";") {
            self.error_at_current("expected `;` after `do ... while`");
        }
    }

    fn for_statement(&mut self) {
        self.bump();
        if !self.eat("(") {
            self.error_at_current("expected `(` after `for`");
            return;
        }
        // C99 6.8.5.3: the `for` head has a scope of its own, so `for (int i = ...)` declares
        // an `i` that is gone after the loop.
        self.symbols.push();
        if !self.at_punct(";") {
            if self.starts_specifier() {
                self.declaration();
            } else {
                self.expression();
            }
        }
        if !self.eat(";") {
            self.error_at_current("expected `;` after the initial part of `for`");
            self.skip_until(&[";"], true);
        }
        if !self.at_punct(";") {
            if let Some(condition) = self.expression() {
                self.require_scalar(&condition, "the condition of `for`");
            }
        }
        if !self.eat(";") {
            self.error_at_current("expected `;` after the condition of `for`");
            self.skip_until(&[")"], true);
        }
        if !self.at_punct(")") {
            self.expression();
        }
        if !self.eat(")") {
            self.error_at_current("expected `)` after the parts of `for`");
        }
        self.loops += 1;
        self.statement();
        self.loops -= 1;
        self.symbols.pop();
    }

    fn loop_control(&mut self, is_break: bool) {
        let span = self.span();
        self.bump();
        if self.loops == 0 {
            let word = if is_break { "break" } else { "continue" };
            self.error(span, format!("`{word}` outside a loop"));
        }
        if !self.eat(";") {
            self.error_at_current("expected `;`");
        }
    }

    fn return_statement(&mut self) {
        let span = self.span();
        self.bump();
        let Some(result) = self.function_result else {
            self.error(span, "`return` outside a function");
            self.skip_until(&[";"], true);
            return;
        };
        let void = matches!(self.types.kind(result), Type::Void);
        if self.eat(";") {
            if !void {
                let ty = self.types.describe(result);
                self.error(
                    span,
                    format!("this function returns {ty}, so `return` needs a value"),
                );
            }
            return;
        }
        let value = self.expression().map(|value| self.decay(value));
        if let Some(value) = value {
            if void {
                self.error(
                    span,
                    "this function returns nothing, so `return` takes no value",
                );
            } else {
                let null_pointer = self.types.is_pointer(result) && value.constant == Some(0);
                if !null_pointer && !self.types.is_assignable(result, value.ty) {
                    let (from, to) = (self.types.describe(value.ty), self.types.describe(result));
                    self.error(span, format!("this function returns {to}, not {from}"));
                }
            }
        }
        if !self.eat(";") {
            self.error_at_current("expected `;` after `return`");
            self.skip_until(&[";"], true);
        }
    }

    /// `( expression )`, as `if`, `while` and `do` take it.
    fn parenthesised_condition(&mut self, what: &str) -> Option<Value> {
        if !self.eat("(") {
            self.error_at_current(format!("expected `(` after {what}"));
            return None;
        }
        let value = self.expression();
        if !self.eat(")") {
            self.error_at_current("expected `)`");
        }
        value
    }

    /// Skip one statement, for the recoveries that cannot parse it.
    fn skip_statement(&mut self) {
        if self.at_punct("{") {
            self.skip_balanced_block();
            return;
        }
        self.skip_until(&[";"], true);
    }

    /// Whether the tokens at the cursor start a declaration: a specifier word, or a name that
    /// is a type. One list, shared with `declaration_specifiers`, so `int x;` inside a block is
    /// a declaration and `x = 1;` is not.
    fn starts_specifier(&self) -> bool {
        let Some(token) = self.peek() else {
            return false;
        };
        if token.kind == crate::lex::TokenKind::Ident && self.symbols.typedef(token.text).is_some()
        {
            return true;
        }
        matches!(
            self.spell(token),
            "typedef"
                | "extern"
                | "static"
                | "auto"
                | "register"
                | "const"
                | "volatile"
                | "restrict"
                | "inline"
                | "void"
                | "_Bool"
                | "char"
                | "short"
                | "int"
                | "long"
                | "float"
                | "double"
                | "signed"
                | "unsigned"
                | "struct"
                | "union"
                | "enum"
        )
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
