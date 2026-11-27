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

    /// A block to jump to, when code is being built.
    ///
    /// The count is a budget because everything after the parser is quadratic in it: the dominator
    /// matrix is `blocks x blocks`, and the verifier, `mem2reg` and the back end each ask it.
    pub(super) fn new_block(&mut self, span: Span) -> Option<cir::Block> {
        let body = self.body.as_mut()?;
        let limit = self.budgets.max_blocks;
        if body.func.blocks.len() >= limit {
            if self.blocks_reported.insert(()) {
                self.error(
                    span,
                    format!("a function of more than {limit} basic blocks is past what this compiler analyses: its dominator work grows with the square of the count"),
                );
            }
            return None;
        }
        Some(body.func.append_empty(span))
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
    pub(super) fn current_block(&self) -> Option<cir::Block> {
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
        let test =
            condition.and_then(|condition| self.compare_to_zero(condition, cir::CmpOp::Ne, span));
        let (then_block, else_block, join) = match (test, self.in_body()) {
            (Some(_), true) => (
                self.new_block(span),
                self.new_block(span),
                self.new_block(span),
            ),
            _ => (None, None, None),
        };
        if let (Some(test), Some(then_block), Some(else_block)) = (test, then_block, else_block) {
            match condition.and_then(|condition| self.constant_truth(&condition)) {
                Some(true) => self.jump(then_block, span),
                Some(false) => self.jump(else_block, span),
                None => {
                    self.emit(|builder| {
                        builder.branch(
                            test,
                            (then_block, Vec::new()),
                            (else_block, Vec::new()),
                            span,
                        );
                    });
                }
            }
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
            let test = self.compare_to_zero(condition, cir::CmpOp::Ne, span);
            if let (Some(test), Some(body_block), Some(exit)) = (test, body_block, exit) {
                match self.constant_truth(&condition) {
                    Some(true) => self.jump(body_block, span),
                    Some(false) => self.jump(exit, span),
                    None => {
                        self.emit(|builder| {
                            builder.branch(
                                test,
                                (body_block, Vec::new()),
                                (exit, Vec::new()),
                                span,
                            );
                        });
                    }
                }
            }
        }
        let (Some(body_block), Some(test), Some(exit)) = (body_block, test, exit) else {
            self.loops += 1;
            self.statement();
            self.loops -= 1;
            return;
        };
        self.switch_to(body_block);
        self.targets.push((exit, Some(test)));
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
        self.targets.push((exit, Some(test)));
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
            Some((condition, test)) => match self.constant_truth(&condition) {
                Some(true) => self.jump(body_block, span),
                Some(false) => self.jump(exit, span),
                None => {
                    self.emit(|builder| {
                        builder.branch(test, (body_block, Vec::new()), (exit, Vec::new()), span)
                    });
                }
            },
            // Nothing to test: the test block leaves the loop. A condition that emitted no
            // value has a diagnostic of its own, and the function is dropped.
            None => self.jump(exit, span),
        }
        self.switch_to(exit);
    }

    /// The `while (condition);` half of a `do`, read after the body because that is where it is.
    fn do_tail(&mut self) -> Option<(Value, cir::Value)> {
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
        let span = self.span();
        // The condition and its test: the *value* is what says a constant has only one side, and
        // `do { } while(1)` is the loop that has one.
        condition.and_then(|condition| {
            let test = self.compare_to_zero(condition, cir::CmpOp::Ne, span)?;
            Some((condition, test))
        })
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
                let test = self.compare_to_zero(condition, cir::CmpOp::Ne, span);
                if let (Some(test), Some(body_block), Some(exit)) = (test, body_block, exit) {
                    match self.constant_truth(&condition) {
                        Some(true) => self.jump(body_block, span),
                        Some(false) => self.jump(exit, span),
                        None => {
                            self.emit(|builder| {
                                builder.branch(
                                    test,
                                    (body_block, Vec::new()),
                                    (exit, Vec::new()),
                                    span,
                                );
                            });
                        }
                    }
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
        // **The cursor, not the block the update started in.** An update expression that ends in a
        // join - `p = c ? 0 : p->pNext` is sqlite's loop over indices - leaves the cursor in the
        // ternary's join block, and the step block is terminated by the branch into the arms. Asking
        // about `step` answered "already ended" and left the join with no way back to the test: the
        // loop ran once and then fell into a trap, which is a `ud2` inside `whereLoopAddBtree` with
        // nothing to say which line it was. Every other loop asks `current_is_open`, and so does this
        // one now.
        if self.current_is_open() {
            self.jump(test, span);
        }
        self.switch_to(body_block);
        self.targets.push((exit, Some(step)));
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
            // `break` leaves the innermost loop *or* `switch`; `continue` skips to the nearest
            // loop, which is why a switch's entry has no continue target.
            let target = if is_break {
                self.targets.last().map(|(stop, _)| *stop)
            } else {
                self.targets.iter().rev().find_map(|(_, next)| *next)
            };
            if let Some(target) = target {
                self.jump(target, span);
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
            // `return;` is still a return: it ends the block, whether or not it carries a value. A
            // `void` function's bare `return` used to emit *nothing at all* - no instruction, no
            // terminator - so control fell through to whatever came next. Lua's `luaK_posfix` folds a
            // constant and returns, and the missing return made it fold *and* emit the operation: a
            // program computing `2^10` printed `2^100`.
            if void {
                self.emit(|builder| builder.ret(None, span));
            } else {
                let ty = self.describe(result);
                self.error(
                    span,
                    format!("this function returns {ty}, so `return` needs a value"),
                );
                // Something has to end the block, and the diagnostic has already said the program is
                // wrong: a trap keeps the rest of the function checking.
                self.emit(|builder| builder.trap(span));
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
                let null_pointer = self.null_pointer_constant(&value);
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
            if self.types.record_of(result).is_some() {
                // The value is the address of the object to return: copied into the buffer the
                // caller passed for a large one, and loaded as its integer for a small one.
                if let Some(source) = value.cir {
                    if let Some(buffer) = self.function_buffer {
                        self.copy_object(buffer, result, source, result, result, span);
                        self.emit(|builder| builder.ret(Some(buffer), span));
                    } else {
                        let integer = self.record_integer(result);
                        if let Some(address) = self.address_within(source, result, 0, integer, span)
                        {
                            let loaded = self.emit(|builder| builder.load(address, integer, span));
                            if let Some(loaded) = loaded {
                                self.emit(|builder| builder.ret(Some(loaded), span));
                            }
                        }
                    }
                }
                return;
            }
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
        // C99 6.8.4.2p5: the controlling expression is promoted - a `switch` on an `unsigned char`
        // compares `unsigned char`s with `int` case labels otherwise, and `cir` is typed. The value is
        // converted *here*, once, because the compare against each case label is built from it.
        let (value, condition_type) = match condition {
            Some(condition) => {
                let promoted = self.types.promote(condition.ty);
                let converted = if promoted == condition.ty {
                    condition
                } else {
                    self.convert(condition, promoted, span)
                };
                (converted.cir, Some(promoted))
            }
            None => (None, None),
        };
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
        let (Some(exit), Some(test)) = (exit, test) else {
            self.statement();
            self.switches.pop();
            return;
        };
        // A `break` inside the switch leaves through this block, and it is nearer than any loop
        // around the switch - which is what pushing it on the same stack says.
        self.targets.push((exit, None));
        self.jump(test, span);
        // Statements before the first `case` are unreachable in C, and get a block to be
        // unreachable in rather than the comparison block.
        if let Some(stray) = self.new_block(span) {
            self.switch_to(stray);
        }
        self.statement();
        self.targets.pop();
        let switch = self.switches.pop().expect("the switch being read");
        // The statements that ended the body fall through to the exit, from wherever they left the
        // cursor - the last arm is not special, it is the arm with no arm after it.
        if self.current_is_open() {
            let exit = switch.exit.expect("the switch's exit block");
            self.jump(exit, span);
        }
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
    }

    /// Start a `case` or `default` body and remember it for the chain.
    fn open_arm(&mut self, value: Option<i64>, span: Span) -> Option<cir::Block> {
        let body = self.new_block(span)?;
        // The arm that just ended falls through into this one, and the link belongs where control
        // actually *is* - not to the block the arm started in. A `?:` or an `if` leaves the cursor in
        // a join block of its own, so linking from the arm's first block left that join with no
        // terminator at all, and a `switch` whose arm ended in `?:` fell off the end of the arm and
        // trapped. Nothing is linked into the first arm: statements before the first `case` are
        // unreachable in C.
        let first = self
            .switches
            .last()
            .is_none_or(|switch| switch.arms.is_empty());
        if !first && self.current_is_open() {
            self.jump(body, span);
        }
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
        let name = token.text;
        if self.labels.insert(name, span).is_some() {
            let spelled = self.spell_of(name).to_owned();
            self.error(
                span,
                format!("this function already has a label `{spelled}`"),
            );
        }
        // Where the label's code starts: the block a `goto` ahead of it made, else a new one that
        // the code above falls into.
        //
        // A new block even when the code above is still going, because a label must be the *first*
        // instruction of its block: a `goto` back to a label in the middle of a block re-runs
        // everything before it, and `i = 0; again: i = i + 1; goto again;` then never ends. That
        // was a real loop, not a hypothetical one (corpus `cc50_corners.c`).
        let target = match self.label_blocks.get(&name).copied() {
            Some(block) => block,
            None => match self.new_block(span) {
                Some(block) => block,
                // No body: nothing to label.
                None => return,
            },
        };
        self.label_blocks.insert(name, target);
        if self.current_block() != Some(target) {
            // The code above the label leaves to it; a block that already ended has nothing
            // to leave and emits nothing.
            self.jump(target, span);
            self.switch_to(target);
        }
        self.statement();
    }

    /// `goto name ;` - the label may come later, so a block that has to is made here and
    /// filled when the label arrives.
    pub(super) fn goto_statement(&mut self) {
        let span = self.span();
        self.bump();
        let mut name = None;
        match self.peek().copied() {
            Some(token) if token.kind == crate::lex::TokenKind::Ident => {
                self.bump();
                self.gotos.push((token.text, span));
                name = Some(token.text);
            }
            _ => self.error_at_current("expected a label name after `goto`"),
        }
        if !self.eat(";") {
            self.error_at_current("expected `;` after `goto`");
        }
        if let Some(name) = name {
            let target = match self.label_blocks.get(&name).copied() {
                Some(block) => Some(block),
                None => match self.new_block(span) {
                    Some(block) => {
                        self.label_blocks.insert(name, block);
                        Some(block)
                    }
                    None => None,
                },
            };
            if let Some(target) = target {
                self.jump(target, span);
            }
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
        // C11 6.7.10's `_Static_assert` belongs to the *declaration* grammar, and `declaration`
        // answers it before any specifier is read. It is not a specifier, so it was not in the list
        // and a block-scope assertion fell through to the statement grammar - where it parsed as a
        // call to an implicit function, and the program failed at *link* with `undefined symbol
        // `_Static_assert``. `xxhash` puts one inside a function and that is how it was found.
        if self.spell(token) == "_Static_assert" {
            return true;
        }
        SPECIFIERS.contains(&self.spell(token))
    }
}
