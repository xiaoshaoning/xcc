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
            Some("switch") => {
                self.switch_statement();
                self.leave_nesting();
                return;
            }
            Some("case") => {
                self.case_label();
                self.leave_nesting();
                return;
            }
            Some("default") => {
                self.default_label();
                self.leave_nesting();
                return;
            }
            Some("goto") => {
                self.goto_statement();
                self.leave_nesting();
                return;
            }
            // `name :` - a label. Checked after the keyword match, because `case` and
            // `default` also end in a `:` and are not names.
            Some(_)
                if self.peek_at(1).is_some_and(|next| self.is_punct(next, ":"))
                    && self
                        .peek()
                        .is_some_and(|token| token.kind == crate::lex::TokenKind::Ident) =>
            {
                self.label_statement();
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
        // `break` leaves a loop or a `switch`; `continue` needs a real loop.
        if is_break && self.loops == 0 && self.switches.is_empty() {
            self.error(span, "`break` outside a loop or `switch`");
        }
        if !is_break && self.loops == 0 {
            self.error(span, "`continue` outside a loop");
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

    /// `switch ( expression ) statement` - the body is read once; `case` and `default` are
    /// labels inside it, and where each of them lands is a question for the code generator.
    pub(super) fn switch_statement(&mut self) {
        let span = self.span();
        self.bump();
        if let Some(condition) = self.parenthesised_condition("`switch`") {
            if !self.types.is_error(condition.ty) && !self.types.is_integer(condition.ty) {
                let ty = self.types.describe(condition.ty);
                self.error(span, format!("`switch` needs an integer, not {ty}"));
            }
        }
        self.switches.push(Switch::default());
        self.statement();
        self.switches.pop();
    }

    /// `case constant :` - C99 6.8.4.2p3: inside a `switch`, and no two the same.
    pub(super) fn case_label(&mut self) {
        let span = self.span();
        self.bump();
        let value = self.constant_expression();
        if !self.eat(":") {
            self.error_at_current("expected `:` after the `case` value");
        }
        if let Some(value) = value {
            let message = match self.switches.last_mut() {
                Some(current) => {
                    if current.cases.insert(value) {
                        None
                    } else {
                        Some(format!("this `switch` already has a case for {value}"))
                    }
                }
                None => Some("`case` outside a `switch`".to_owned()),
            };
            if let Some(message) = message {
                self.error(span, message);
            }
        }
        self.statement();
    }

    /// `default :` - at most one per `switch` (C99 6.8.4.2p3).
    pub(super) fn default_label(&mut self) {
        let span = self.span();
        self.bump();
        if !self.eat(":") {
            self.error_at_current("expected `:` after `default`");
        }
        let message = match self.switches.last_mut() {
            Some(current) if current.has_default => {
                Some("this `switch` already has a `default`".to_owned())
            }
            Some(current) => {
                current.has_default = true;
                None
            }
            None => Some("`default` outside a `switch`".to_owned()),
        };
        if let Some(message) = message {
            self.error(span, message);
        }
        self.statement();
    }

    /// `name :` - a label, which a `goto` anywhere in the function may name (C99 6.2.1p4).
    pub(super) fn label_statement(&mut self) {
        let Some(token) = self.peek().copied() else {
            return;
        };
        let span = self.span();
        self.bump(); // the name
        self.bump(); // the `:`
        if self.labels.insert(token.text, span).is_some() {
            let name = self.spell_of(token.text).to_owned();
            self.error(span, format!("this function already has a label `{name}`"));
        }
        self.statement();
    }

    /// `goto name ;` - resolved when the function ends, because the label may come later.
    pub(super) fn goto_statement(&mut self) {
        let span = self.span();
        self.bump();
        match self.peek().copied() {
            Some(token) if token.kind == crate::lex::TokenKind::Ident => {
                self.bump();
                self.gotos.push((token.text, span));
            }
            _ => self.error_at_current("expected a label name after `goto`"),
        }
        if !self.eat(";") {
            self.error_at_current("expected `;` after `goto`");
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
