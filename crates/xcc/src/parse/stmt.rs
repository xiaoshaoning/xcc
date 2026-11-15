//! The parser's statements: what a statement must be true of, and what it checks.
//!
//! Part of the parser and nothing else: the state is `Parser`'s, which lives in `mod.rs`.

use super::*;

impl Parser<'_> {
    // ---- statements ------------------------------------------------------------
    //
    // A statement is parsed for what it does to the types: `if (p)` needs a scalar, `return`
    // needs to match the function's result, `break` needs a loop. At M4 each of these becomes
    // `cir` control flow as well; there is no tree in between (ADR-002).

    /// One statement. The cursor is at its first token.
    pub(super) fn statement(&mut self) {
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

    pub(super) fn block(&mut self) {
        self.symbols.push();
        self.block_body();
        self.symbols.pop();
    }

    /// The braces and statements of a block; the caller owns its scope.
    pub(super) fn block_body(&mut self) {
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

    pub(super) fn if_statement(&mut self) {
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

    pub(super) fn while_statement(&mut self) {
        self.bump();
        if let Some(condition) = self.parenthesised_condition("`while`") {
            self.require_scalar(&condition, "the condition of `while`");
        }
        self.loops += 1;
        self.statement();
        self.loops -= 1;
    }

    pub(super) fn do_statement(&mut self) {
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

    pub(super) fn for_statement(&mut self) {
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

    pub(super) fn loop_control(&mut self, is_break: bool) {
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

    pub(super) fn return_statement(&mut self) {
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
    pub(super) fn parenthesised_condition(&mut self, what: &str) -> Option<Value> {
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
    pub(super) fn skip_statement(&mut self) {
        if self.at_punct("{") {
            self.skip_balanced_block();
            return;
        }
        self.skip_until(&[";"], true);
    }

    /// Whether the tokens at the cursor start a declaration: a specifier word, or a name that
    /// is a type. One list, shared with `declaration_specifiers`, so `int x;` inside a block is
    /// a declaration and `x = 1;` is not.
    pub(super) fn starts_specifier(&self) -> bool {
        let Some(token) = self.peek() else {
            return false;
        };
        if token.kind == crate::lex::TokenKind::Ident && self.symbols.typedef(token.text).is_some()
        {
            return true;
        }
        SPECIFIERS.contains(&self.spell(token))
    }
}
