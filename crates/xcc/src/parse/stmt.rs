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
                let span = self.span();
                self.not_lowered(span, "`goto`");
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

    /// A block to jump to, when code is being built.
    pub(super) fn new_block(&mut self, span: Span) -> Option<cir::Block> {
        Some(self.body.as_mut()?.func.append_empty(span))
    }

    /// Leave the current block for `target`.
    pub(super) fn jump(&mut self, target: cir::Block, span: Span) {
        self.emit(|builder| builder.jump(target, Vec::new(), span));
    }

    /// Start filling `block`.
    pub(super) fn switch_to(&mut self, block: cir::Block) {
        if let Some(body) = self.body.as_mut() {
            body.current = block;
        }
    }

    /// The block being filled, which is not always the one a construct opened: a nested `switch`
    /// leaves the cursor in its exit block, so "jump back from where we are" must ask here.
    fn current_block(&self) -> Option<cir::Block> {
        self.body.as_ref().map(|body| body.current)
    }

    /// Whether the block being filled can still take instructions: a construct jumps back to
    /// its test or forward to its join only from an open block, and after a nested `switch` the
    /// cursor sits in that switch's exit, not in the block the construct opened.
    fn current_is_open(&self) -> bool {
        self.current_block()
            .is_some_and(|now| !self.terminated(now))
    }

    /// Whether a block already ends, so it is not given a second terminator.
    fn terminated(&self, block: cir::Block) -> bool {
        self.body
            .as_ref()
            .is_none_or(|body| body.func.terminated(block))
    }

    /// `if (condition) statement else statement`: three blocks and a join. The join takes no
    /// parameters because C's `if` has no value.
    pub(super) fn if_statement(&mut self) {
        let span = self.span();
        self.bump();
        let condition = self.parenthesised_condition("`if`");
        if let Some(condition) = condition {
            self.require_scalar(&condition, "the condition of `if`");
        }
        let test = condition.and_then(|condition| condition.cir);
        let (then_block, else_block, join) = match (test, self.in_body()) {
            (Some(_), true) => (
                self.new_block(span),
                self.new_block(span),
                self.new_block(span),
            ),
            _ => (None, None, None),
        };
        if let (Some(test), Some(then_block), Some(else_block)) = (test, then_block, else_block) {
            self.emit(|builder| {
                builder.branch(
                    test,
                    (then_block, Vec::new()),
                    (else_block, Vec::new()),
                    span,
                )
            });
            self.switch_to(then_block);
        }
        self.statement();
        if let (Some(_then), Some(else_block), Some(join)) = (then_block, else_block, join) {
            if self.current_is_open() {
                self.jump(join, span);
            }
            self.switch_to(else_block);
        }
        if self.at_word("else") {
            self.bump();
            self.statement();
        }
        if let (Some(_else), Some(join)) = (else_block, join) {
            if self.current_is_open() {
                self.jump(join, span);
            }
            self.switch_to(join);
        }
    }

    /// `while (condition) statement`: the test is a block of its own, so the body can come back
    /// to it.
    pub(super) fn while_statement(&mut self) {
        let span = self.span();
        self.bump();
        let test = self.new_block(span);
        let body_block = self.new_block(span);
        let exit = self.new_block(span);
        if let Some(test) = test {
            self.jump(test, span);
            self.switch_to(test);
        }
        if let Some(condition) = self.parenthesised_condition("`while`") {
            self.require_scalar(&condition, "the condition of `while`");
            if let (Some(condition), Some(body_block), Some(exit)) =
                (condition.cir, body_block, exit)
            {
                self.emit(|builder| {
                    builder.branch(
                        condition,
                        (body_block, Vec::new()),
                        (exit, Vec::new()),
                        span,
                    )
                });
            }
        }
        let (Some(body_block), Some(test), Some(exit)) = (body_block, test, exit) else {
            self.loops += 1;
            self.statement();
            self.loops -= 1;
            return;
        };
        self.switch_to(body_block);
        self.targets.push((exit, test));
        self.loops += 1;
        self.statement();
        self.loops -= 1;
        self.targets.pop();
        // A body that ends in `break` or `return` already ends; a second terminator would be a
        // block with two exits. "Where we are" is the block the last statement filled, which a
        // nested `switch` may have moved to its own exit.
        if self.current_is_open() {
            self.jump(test, span);
        }
        self.switch_to(exit);
    }

    pub(super) fn do_statement(&mut self) {
        let span = self.span();
        self.bump();
        let body_block = self.new_block(span);
        let test = self.new_block(span);
        let exit = self.new_block(span);
        let (Some(body_block), Some(test), Some(exit)) = (body_block, test, exit) else {
            self.loops += 1;
            self.statement();
            self.loops -= 1;
            self.do_tail();
            return;
        };
        self.jump(body_block, span);
        self.switch_to(body_block);
        self.targets.push((exit, test));
        self.loops += 1;
        self.statement();
        self.loops -= 1;
        self.targets.pop();
        if self.current_is_open() {
            self.jump(test, span);
        }
        self.switch_to(test);
        let condition = self.do_tail();
        match condition {
            Some(condition) => {
                self.emit(|builder| {
                    builder.branch(
                        condition,
                        (body_block, Vec::new()),
                        (exit, Vec::new()),
                        span,
                    )
                });
            }
            // Nothing to test: the test block leaves the loop. A condition that emitted no
            // value has a diagnostic of its own, and the function is dropped.
            None => self.jump(exit, span),
        }
        self.switch_to(exit);
    }

    /// The `while (condition);` half of a `do`, read after the body because that is where it is.
    fn do_tail(&mut self) -> Option<cir::Value> {
        if !self.at_word("while") {
            self.error_at_current("expected `while` after the body of `do`");
            return None;
        }
        self.bump();
        let condition = self.parenthesised_condition("`do ... while`");
        if let Some(condition) = condition.as_ref() {
            self.require_scalar(condition, "the condition of `do ... while`");
        }
        if !self.eat(";") {
            self.error_at_current("expected `;` after `do ... while`");
        }
        condition.and_then(|condition| condition.cir)
    }

    /// `for`: the increment lives in a block of its own, which is parsed (and filled) before the
    /// body because that is the order the source is in. A block may be filled in any order.
    pub(super) fn for_statement(&mut self) {
        let span = self.span();
        self.bump();
        if !self.eat("(") {
            self.error_at_current("expected `(` after `for`");
            return;
        }
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
        let test = self.new_block(span);
        let body_block = self.new_block(span);
        let step = self.new_block(span);
        let exit = self.new_block(span);
        if let Some(test) = test {
            self.jump(test, span);
            self.switch_to(test);
        }
        if !self.at_punct(";") {
            if let Some(condition) = self.expression() {
                self.require_scalar(&condition, "the condition of `for`");
                if let (Some(condition), Some(body_block), Some(exit)) =
                    (condition.cir, body_block, exit)
                {
                    self.emit(|builder| {
                        builder.branch(
                            condition,
                            (body_block, Vec::new()),
                            (exit, Vec::new()),
                            span,
                        )
                    });
                }
            }
        } else if let Some(body_block) = body_block {
            // `for (;;)` never tests, so nothing can leave it but `break` or `return`.
            self.jump(body_block, span);
        }
        if !self.eat(";") {
            self.error_at_current("expected `;` after the condition of `for`");
            self.skip_until(&[")"], true);
        }
        if let Some(step) = step {
            self.switch_to(step);
        }
        if !self.at_punct(")") {
            self.expression();
        }
        if !self.eat(")") {
            self.error_at_current("expected `)` after the parts of `for`");
        }
        let (Some(body_block), Some(step), Some(test), Some(exit)) = (body_block, step, test, exit)
        else {
            self.loops += 1;
            self.statement();
            self.loops -= 1;
            self.symbols.pop();
            return;
        };
        if !self.terminated(step) {
            self.jump(test, span);
        }
        self.switch_to(body_block);
        self.targets.push((exit, step));
        self.loops += 1;
        self.statement();
        self.loops -= 1;
        self.targets.pop();
        if self.current_is_open() {
            self.jump(step, span);
        }
        self.switch_to(exit);
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
        if self.in_body() {
            if let Some((stop, next)) = self.targets.last().copied() {
                let target = if is_break { stop } else { next };
                self.jump(target, span);
            } else if is_break {
                // No loop around it, so `break` belongs to the innermost `switch` - and
                // `continue` does not, which is why a switch pushes no loop target.
                if let Some(exit) = self.switches.last().and_then(|switch| switch.exit) {
                    self.jump(exit, span);
                }
            }
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
                let ty = self.describe(result);
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
                    let (from, to) = (self.describe(value.ty), self.describe(result));
                    self.error(span, format!("this function returns {to}, not {from}"));
                }
            }
        }
        if !self.eat(";") {
            self.error_at_current("expected `;` after `return`");
            self.skip_until(&[";"], true);
        }
        let span = self.span();
        if let Some(value) = value {
            let value = self.convert(value, result, span);
            if let Some(cir) = value.cir {
                self.emit(|builder| builder.ret(Some(cir), span));
            }
        }
    }

    /// `switch ( expression ) statement` - the body is read once; `case` and `default` are
    /// labels inside it, and where each of them lands is a question for the code generator.
    /// `switch (e) body`: one comparison chain, written *after* the body is read, because the
    /// `case` values appear inside it. Each arm gets a block of its own, and the bodies are
    /// linked to each other in source order so C's fall-through falls through.
    pub(super) fn switch_statement(&mut self) {
        let span = self.span();
        self.bump();
        let condition = self.parenthesised_condition("`switch`");
        if let Some(condition) = condition.as_ref() {
            if !self.types.is_error(condition.ty) && !self.types.is_integer(condition.ty) {
                let ty = self.describe(condition.ty);
                self.error(span, format!("`switch` needs an integer, not {ty}"));
            }
        }
        let value = condition.and_then(|condition| condition.cir);
        let condition_type = condition.map(|condition| self.types.promote(condition.ty));
        let (exit, test) = match (value, self.in_body()) {
            (Some(_), true) => (self.new_block(span), self.new_block(span)),
            _ => (None, None),
        };
        self.switches.push(Switch {
            condition: value,
            condition_type,
            exit,
            test,
            ..Switch::default()
        });
        let (Some(_exit), Some(test)) = (exit, test) else {
            self.statement();
            self.switches.pop();
            return;
        };
        self.jump(test, span);
        // Statements before the first `case` are unreachable in C, and get a block to be
        // unreachable in rather than the comparison block.
        if let Some(stray) = self.new_block(span) {
            self.switch_to(stray);
        }
        self.statement();
        let switch = self.switches.pop().expect("the switch being read");
        self.emit_switch_chain(&switch, span);
        // Whatever comes after the switch belongs to the exit block: without this, trailing
        // statements land in the last arm's body, `exit` is never filled, and a `break` jumps
        // into a block the orphan pass turns into a trap.
        self.switch_to(switch.exit.expect("the switch's exit block"));
    }

    /// The comparison chain, then the fall-through links. Both are written after the body,
    /// which is why blocks are filled in an order of their own choosing (`for` does the same).
    fn emit_switch_chain(&mut self, switch: &Switch, span: Span) {
        let (Some(condition), Some(condition_type), Some(test), Some(exit)) = (
            switch.condition,
            switch.condition_type,
            switch.test,
            switch.exit,
        ) else {
            return;
        };
        self.switch_to(test);
        let mut default = None;
        for (value, body) in &switch.arms {
            let Some(value) = value else {
                default = Some(*body);
                continue;
            };
            let next = match self.new_block(span) {
                Some(next) => next,
                None => continue,
            };
            let literal = self.constant_of(condition_type, *value, span);
            if let Some(literal) = literal.cir {
                if let Some(flag) = self.emit(|builder| {
                    builder.compare(
                        cir::CmpOp::Eq,
                        condition,
                        literal,
                        condition_type,
                        condition_type,
                        span,
                    )
                }) {
                    self.emit(|builder| {
                        builder.branch(flag, (*body, Vec::new()), (next, Vec::new()), span)
                    });
                }
            }
            self.switch_to(next);
        }
        let last = default.unwrap_or(exit);
        self.jump(last, span);
        for (position, (_, body)) in switch.arms.iter().enumerate() {
            let next = switch
                .arms
                .get(position + 1)
                .map_or(exit, |(_, next)| *next);
            if !self.terminated(*body) {
                self.switch_to(*body);
                self.jump(next, span);
            }
        }
    }

    /// Start a `case` or `default` body and remember it for the chain.
    fn open_arm(&mut self, value: Option<i64>, span: Span) -> Option<cir::Block> {
        let body = self.new_block(span)?;
        if let Some(switch) = self.switches.last_mut() {
            switch.arms.push((value, body));
        }
        self.switch_to(body);
        Some(body)
    }

    /// `case constant :` - C99 6.8.4.2p3: inside a `switch`, and no two the same.
    pub(super) fn case_label(&mut self) {
        let span = self.span();
        self.bump();
        let value = self.constant_expression();
        if !self.eat(":") {
            self.error_at_current("expected `:` after the `case` value");
        }
        // The body is opened before the duplicate check: a duplicate `case` still has one.
        let body = self.open_arm(value, span);
        let _ = body;
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
        let arm = self.open_arm(None, span);
        let _ = arm;
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
