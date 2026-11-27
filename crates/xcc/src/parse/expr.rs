//! The parser's expressions: what a value is, and what C says it may be combined with.
//!
//! Part of the parser and nothing else: the state is `Parser`'s, which lives in `mod.rs`. This
//! file is the answer to "what does this operator do to these values", which is a question
//! large enough to read on its own.

use super::*;

/// The mask a bitfield of `width` bits has, as a 64-bit pattern.
pub(super) fn field_mask(width: u8) -> u64 {
    if width >= 64 {
        u64::MAX
    } else {
        (1u64 << width) - 1
    }
}

impl Parser<'_> {
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
    pub(super) fn constant_expression(&mut self) -> Option<i64> {
        if !self.enter_nesting() {
            return None;
        }
        let saved = std::mem::replace(&mut self.unevaluated, true);
        let value = self.conditional_expression();
        self.unevaluated = saved;
        self.leave_nesting();
        match value {
            // C99 6.6: what this is asked for - an array's size, a `case` label, an enumerator, a
            // bitfield's width - is an *integer* constant expression. A float has a constant too,
            // its bit pattern, and those bits are not the number.
            Some(value) if !self.types.is_integer(value.ty) => {
                let ty = self.describe(value.ty);
                let span = self.span();
                self.error(
                    span,
                    format!("expected an integer constant expression, not {ty}"),
                );
                None
            }
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
    pub(super) fn expression(&mut self) -> Option<Value> {
        let mut value = self.assignment_expression()?;
        while self.eat(",") {
            value = self.assignment_expression()?;
        }
        Some(value)
    }

    pub(super) fn assignment_expression(&mut self) -> Option<Value> {
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
    pub(super) fn assign(&mut self, left: Value, right: Value, span: Span) -> Option<Value> {
        let right = self.decay(right);
        if self.types.is_error(left.ty) || self.types.is_error(right.ty) {
            return Some(Value::temporary(left.ty));
        }
        if !left.lvalue {
            self.error(span, "cannot assign to this expression");
            return Some(Value::temporary(left.ty));
        }
        // C99 6.5.16: the left operand must be a *modifiable* lvalue, and an array is not one.
        // `s = "hi"` is a constraint violation, not an array copy.
        if matches!(self.types.kind(left.ty), Type::Array { .. }) {
            self.error(span, "cannot assign to an array");
            return Some(Value::temporary(left.ty));
        }
        // C99 6.3.2.1p1: a *modifiable* lvalue is one that is not `const`-qualified, and a struct or
        // union with a const-qualified member is not one either, however deeply. Every compound
        // assignment is this function too, because `a += b` is written as an assignment of `a + b`.
        if !self.types.is_modifiable(left.ty) {
            self.error(span, "cannot assign to a `const` object");
            return Some(Value::temporary(left.ty));
        }
        let null_pointer = self.null_pointer_constant(&right);
        if !null_pointer && !self.types.is_assignable(left.ty, right.ty) {
            if self.types.is_pointer(left.ty) && self.returned_by_an_implicit_call(&right) {
                self.warning(
                    span,
                    "a function nothing declared returns `int`, and its result is assigned to a                      pointer: taken to be the address it was with C89's compilers and is with MSVC                      (C4047)",
                );
            } else {
                let (to, from) = (self.describe(left.ty), self.describe(right.ty));
                self.error(span, format!("cannot assign {from} to {to}"));
                return Some(Value::temporary(left.ty));
            }
        }
        let converted = self.convert(right, left.ty, span);
        if let Some((start, width)) = left.bits {
            return Some(self.write_bitfield(left, converted, (start, width), span));
        }
        match (left.place, converted.cir) {
            (Some(place), Some(value)) => {
                self.store_at(place, left.ty, converted, span);
                if self.types.record_of(left.ty).is_some() {
                    // The result of a record assignment is the object after the copy: a place, so
                    // `(a = b).x` reads it, and not an lvalue.
                    return Some(Value {
                        ty: left.ty,
                        constant: None,
                        lvalue: false,
                        bits: None,
                        address: None,
                        cir: Some(place),
                        place: Some(place),
                    });
                }
                Some(Value::code(left.ty, value))
            }
            // No place and no body: this is a file-scope initialiser, where C99 6.6p7 wants a
            // *constant* expression. `static int a[1] = { x = 5 };` is not one, and it was accepted
            // and written as zero - a wrong answer rather than a diagnostic, which is the one thing
            // this compiler may not do.
            (None, _) if !self.in_body() && !self.types.is_error(left.ty) => {
                self.error(
                    span,
                    "a file-scope initialiser must be a constant expression",
                );
                Some(Value::new(left.ty, None, false))
            }
            (None, _) if self.in_body() && !self.types.is_error(left.ty) => {
                // A place the emitter cannot name yet: a global, a member, an element.
                self.not_lowered(span, "this kind of assignment");
                Some(Value::temporary(left.ty))
            }
            _ => Some(Value::new(left.ty, converted.constant, false)),
        }
    }

    /// A bitfield write: the unit is read, the field's bits are cleared and the new value put
    /// in them. The value of the assignment is the field *after* the write, as C has it.
    fn write_bitfield(&mut self, left: Value, value: Value, bits: (u8, u8), span: Span) -> Value {
        let ty = left.ty;
        let Some(place) = left.place else {
            return Value::new(ty, None, false);
        };
        self.write_bitfield_at(place, ty, value, bits, span);
        // The value of the assignment is the field as it now stands, which is the read path's
        // question - so a signed field is sign-extended there and not in the writer: `(s.c = -7)`
        // is -7, not the four bits that were stored. An assignment is not an lvalue, so the place
        // and the bits stay behind: without that, `(s.a = 1) = 2` would be accepted.
        let mut result = self.read_bitfield(ty, place, bits, span);
        result.lvalue = false;
        result.place = None;
        result.bits = None;
        result
    }

    /// The merge a bitfield write is, at an address rather than at a place: read the unit, clear
    /// the field's bits, put the new ones in. An assignment, `++` and an initialiser all end here,
    /// so the mask arithmetic and the "the neighbours are kept" rule are written once. What the
    /// field is *worth* afterwards is the read path's question, not this one's - which is why a
    /// caller that stores for its own sake pays for no load it would not use.
    pub(super) fn write_bitfield_at(
        &mut self,
        address: cir::Value,
        ty: TypeId,
        value: Value,
        (start, width): (u8, u8),
        span: Span,
    ) {
        // A value with no code is dead code: there is nothing to merge and nothing to store.
        let Value {
            cir: Some(value), ..
        } = value
        else {
            return;
        };
        let mask = field_mask(width) as i64;
        let keep = self.constant_of(ty, !(mask << start), span);
        let clear = self.constant_of(ty, mask, span);
        let shift = self.constant_of(ty, i64::from(start), span);
        let (keep, clear, shift) = (keep.cir, clear.cir, shift.cir);
        let unit = self.emit(|builder| builder.load(address, ty, span));
        let (Some(unit), Some(keep), Some(clear), Some(shift)) = (unit, keep, clear, shift) else {
            return;
        };
        let some = |parser: &mut Self, op: cir::BinOp, a: cir::Value, b: cir::Value| {
            parser.emit(|builder| builder.binary(op, a, b, ty, span))
        };
        let (Some(field), Some(kept)) = (
            some(self, cir::BinOp::And, value, clear),
            some(self, cir::BinOp::And, unit, keep),
        ) else {
            return;
        };
        let Some(shifted) = some(self, cir::BinOp::Shl, field, shift) else {
            return;
        };
        let Some(merged) = some(self, cir::BinOp::Or, kept, shifted) else {
            return;
        };
        self.emit(|builder| builder.store(address, merged, span));
    }

    /// `condition ? when_true : when_false`: two blocks and a join, and the join's parameter is
    /// the expression's value (C99 6.5.15). Only the arm the condition picks is evaluated, so the
    /// arms cannot be one instruction with a `select`: the untaken arm may divide by zero.
    ///
    /// The arms are read in blocks of their own, and their conversions are written back into the
    /// block that computed them - the join's type is not known until both arms have been read.
    /// Both are read the same way whether or not the condition has code, because the folded value
    /// is part of what the expression is: `int *p = 1 ? 0 : 0;` is a null pointer only if the
    /// constant survives.
    pub(super) fn conditional_expression(&mut self) -> Option<Value> {
        // Every way an expression nests inside another one comes through here - a parenthesised
        // group, an argument, a subscript, an initialiser value - so this is where the budget
        // stops a source that would recurse the stack to its end.
        self.within_nesting(Self::conditional_expression_inner)
    }

    fn conditional_expression_inner(&mut self) -> Option<Value> {
        let condition = self.binary_expression(0)?;
        let question = self.span();
        if !self.eat("?") {
            return Some(condition);
        }
        self.require_scalar(&condition, "the condition of `?:`");
        // Code is wanted unless there is none to want: a constant being folded, or an operand
        // nothing evaluates.
        let branch_on = if self.in_body() {
            self.compare_to_zero(condition, cir::CmpOp::Ne, question)
        } else {
            None
        };
        let (then_block, else_block) = if branch_on.is_some() {
            (self.new_block(question), self.new_block(question))
        } else {
            (None, None)
        };
        if let (Some(condition), Some(then_block), Some(else_block)) =
            (branch_on, then_block, else_block)
        {
            self.emit(|builder| {
                builder.branch(
                    condition,
                    (then_block, Vec::new()),
                    (else_block, Vec::new()),
                    question,
                )
            });
            self.switch_to(then_block);
        }
        let when_true = self.expression().map(|value| self.decay(value))?;
        // The cursor after an arm is not always the block the arm started in: `a && b` and a
        // nested `?:` leave it in their own join, and that is where the arm's jump leaves from.
        let then_exit = self.current_block();
        if !self.eat(":") {
            self.error_at_current("expected `:`");
            return None;
        }
        if let Some(else_block) = else_block {
            self.switch_to(else_block);
        }
        let when_false = self
            .conditional_expression()
            .map(|value| self.decay(value))?;
        let else_exit = self.current_block();
        let ty = self.ternary_type(when_true, when_false, question);
        let condition_zero = self.constant_is_zero(&condition);
        let folded = match (when_true.constant, when_false.constant) {
            (Some(t), Some(f)) => condition
                .constant
                .map(|_| if condition_zero { f } else { t }),
            // One arm constant, and the condition known to pick it.
            (Some(t), None) if condition.constant.is_some() && !condition_zero => Some(t),
            (None, Some(f)) if condition.constant.is_some() && condition_zero => Some(f),
            _ => None,
        };
        if branch_on.is_none() || self.types.is_error(ty) {
            return Some(Value::new(ty, folded, false));
        }
        // A condition with code means a body, and a body means each arm ended in one of its
        // blocks: no code path, no body, no exits - and this is the code path.
        let (Some(then_exit), Some(else_exit)) = (then_exit, else_exit) else {
            unreachable!("a branch condition has a body, so both arms end in blocks of it")
        };
        // A `void` arm carries side effects and no value, so the join takes no parameter then.
        let void = matches!(self.types.kind(ty), Type::Void);
        // An *aggregate* arm is a record, and a record is never in a register: its value is the
        // address of the object it was written into. So the join carries that address - which is
        // exactly how a call returning a record hands its result over - and the `?:`'s value is an
        // rvalue of the record type whose place is whichever arm ran. The object is alive for the
        // rest of the full expression, which is as long as the result can be used (C99 6.5.15p5
        // makes it a value, and a value of aggregate type is only ever read by copying it).
        let aggregate = !void && self.types.record_of(ty).is_some();
        if !void && !aggregate && !self.lowerable(ty) {
            self.not_lowered(question, "a conditional expression of this type");
            // Poisoned: nothing downstream complains a second time about a value with no address.
            return Some(Value::temporary(self.types.error()));
        }
        let carried_ty = if aggregate { self.pointer_to(ty) } else { ty };
        let params: Vec<TypeId> = if void { Vec::new() } else { vec![carried_ty] };
        let join = self.emit(|builder| builder.append_block(&params, question))?;
        for (exit, arm) in [(then_exit, when_true), (else_exit, when_false)] {
            self.switch_to(exit);
            let carried: Option<cir::Value> = if void {
                None
            } else if aggregate {
                // The arm's own object, which is where its value already is.
                match arm.cir {
                    Some(address) => Some(address),
                    None => return Some(Value::new(ty, folded, false)),
                }
            } else {
                match self.convert(arm, ty, question).cir {
                    Some(value) => Some(value),
                    // The arm already has a diagnostic of its own; a one-armed jump would only
                    // hide it, and the body is dropped either way.
                    None => return Some(Value::new(ty, folded, false)),
                }
            };
            self.emit(|builder| builder.jump(join, carried.into_iter().collect(), question));
        }
        self.switch_to(join);
        let value = if void {
            None
        } else {
            self.body
                .as_ref()
                .map(|body| body.func.block(join).params[0])
        };
        Some(Value::new(ty, folded, false).with_cir(value))
    }

    /// The type of `a ? b : c` (C99 6.5.15): the common type of the two arms, which for
    /// pointers is the pointer, not the `int` that arithmetic conversions would give.
    pub(super) fn ternary_type(&mut self, left: Value, right: Value, span: Span) -> TypeId {
        if self.types.same(left.ty, right.ty) {
            return left.ty;
        }
        if self.types.is_error(left.ty) || self.types.is_error(right.ty) {
            return self.types.error();
        }
        if matches!(self.types.kind(left.ty), Type::Void)
            || matches!(self.types.kind(right.ty), Type::Void)
        {
            return self.types.fundamental("void").unwrap();
        }
        if self.types.is_pointer(left.ty) && self.types.is_pointer(right.ty) {
            return if self.types.is_assignable(left.ty, right.ty) {
                left.ty
            } else if self.types.is_assignable(right.ty, left.ty) {
                right.ty
            } else {
                let (a, b) = (self.describe(left.ty), self.describe(right.ty));
                self.error(span, format!("the arms of `?:` are {a} and {b}"));
                self.types.error()
            };
        }
        if self.types.is_pointer(left.ty) && self.null_pointer_constant(&right) {
            return left.ty;
        }
        if self.types.is_pointer(right.ty) && self.null_pointer_constant(&left) {
            return right.ty;
        }
        if self.types.is_arithmetic(left.ty) && self.types.is_arithmetic(right.ty) {
            return self.types.usual_arithmetic(left.ty, right.ty);
        }
        let (a, b) = (self.describe(left.ty), self.describe(right.ty));
        self.error(span, format!("the arms of `?:` are {a} and {b}"));
        self.types.error()
    }

    /// Binary operators by precedence, left-associative (C99 6.5.5 to 6.5.12).
    pub(super) fn binary_expression(&mut self, minimum: usize) -> Option<Value> {
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
            if operator == "&&" || operator == "||" {
                // Handled here, not in `binary_result`: short-circuiting means the right operand
                // must be *parsed* somewhere else, and this is the only place the parse order is
                // still ours to choose.
                left = self.logical(&operator, left, level, span)?;
                continue;
            }
            let right = self.binary_expression(level + 1)?;
            left = self.binary_result(&operator, left, right, span);
        }
    }

    /// The value of `left <operator> right`, typed the way C says and folded when it can be.
    ///
    /// A type error is *poisoned*: the result is the error type, which is compatible with
    /// everything, so `1 + "text" + 2` is one diagnostic rather than three.
    pub(super) fn binary_result(
        &mut self,
        operator: &str,
        left: Value,
        right: Value,
        span: Span,
    ) -> Value {
        let left = self.decay(left);
        let right = self.decay(right);
        if self.types.is_error(left.ty) || self.types.is_error(right.ty) {
            return Value::temporary(self.types.error());
        }
        // C99 6.5.5: a complex operand makes the operation a complex one, and none of the rules
        // below applies to a value that is two numbers in a record. One place, before the
        // arithmetic, so `+`, `==` and `<` all get the same answer about what they were given.
        if let Some(complex) = self.complex_binary(operator, left, right, span) {
            return complex;
        }
        let arithmetic = self.types.is_arithmetic(left.ty) && self.types.is_arithmetic(right.ty);
        let integer = self.types.is_integer(left.ty) && self.types.is_integer(right.ty);
        let integral = self.types.is_integer(left.ty) && self.types.is_integer(right.ty);
        // C99 6.3.1.8 first, then the operator: both constants convert to the type the operation
        // is done in, so the fold sees the numbers C sees. `fold_conversion` is the same function
        // that folds a cast, because the conversion is the same one.
        let folded = |ty: TypeId, left: Value, right: Value| match (left.constant, right.constant) {
            (Some(a), Some(b)) if integral => {
                let a = self.fold_conversion(left.ty, ty, a)?;
                let b = self.fold_conversion(right.ty, ty, b)?;
                fold(&self.types, ty, operator, a, b)
            }
            _ => None,
        };
        let common = || self.types.usual_arithmetic(left.ty, right.ty);

        match operator {
            "+" | "-" => {
                if arithmetic {
                    let ty = common();
                    let op = if operator == "+" {
                        cir::BinOp::Add
                    } else {
                        cir::BinOp::Sub
                    };
                    let folded = if self.types.is_float(ty) {
                        self.fold_float(operator, ty, left, right)
                    } else {
                        folded(ty, left, right)
                    };
                    return self.emit_binary(op, ty, ty, [left, right], folded, span);
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
                            return self.pointer_walk(pointer_ty, left, right, true, span);
                        }
                    }
                }
                if operator == "-"
                    && self.types.is_pointer(left.ty)
                    && self.types.is_integer(right.ty)
                {
                    return self.pointer_walk(left.ty, left, right, false, span);
                }
                if operator == "-"
                    && self.types.is_pointer(left.ty)
                    && self.types.is_pointer(right.ty)
                {
                    // The difference of two addresses is a count of elements, not of bytes.
                    let long = self.types.fundamental("long long").unwrap();
                    let element = self.types.pointee(left.ty);
                    let size = self.types.size_of(element).unwrap_or(1).max(1);
                    let left = self.convert(left, long, span);
                    let right = self.convert(right, long, span);
                    match (left.cir, right.cir) {
                        (Some(left), Some(right)) => {
                            let difference =
                                self.emit(|b| b.binary(cir::BinOp::Sub, left, right, long, span));
                            match difference {
                                Some(difference) => {
                                    let divisor = self.constant_of(long, size as i64, span);
                                    match (divisor.cir, difference) {
                                        (Some(divisor), difference) => {
                                            let quotient = self.emit(|b| {
                                                b.binary(
                                                    cir::BinOp::Div,
                                                    difference,
                                                    divisor,
                                                    long,
                                                    span,
                                                )
                                            });
                                            return match quotient {
                                                Some(quotient) => Value::code(long, quotient),
                                                None => Value::new(long, None, false),
                                            };
                                        }
                                        _ => return Value::new(long, None, false),
                                    }
                                }
                                None => return Value::new(long, None, false),
                            }
                        }
                        _ => return Value::new(long, None, false),
                    }
                }
                self.bad_operands(operator, left.ty, right.ty, span)
            }
            "*" | "/" => {
                if arithmetic {
                    let ty = common();
                    let op = if operator == "*" {
                        cir::BinOp::Mul
                    } else {
                        cir::BinOp::Div
                    };
                    let folded = if self.types.is_float(ty) {
                        self.fold_float(operator, ty, left, right)
                    } else {
                        folded(ty, left, right)
                    };
                    return self.emit_binary(op, ty, ty, [left, right], folded, span);
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
                            self.types.promote(left.ty)
                        };
                    let op = match operator {
                        "%" => cir::BinOp::Rem,
                        "<<" => cir::BinOp::Shl,
                        ">>" => cir::BinOp::Shr,
                        "&" => cir::BinOp::And,
                        "^" => cir::BinOp::Xor,
                        _ => cir::BinOp::Or,
                    };
                    let operands = if matches!(op, cir::BinOp::Shl | cir::BinOp::Shr) {
                        self.types.promote(left.ty)
                    } else {
                        ty
                    };
                    let folded = folded(operands, left, right);
                    return self.emit_binary(op, operands, ty, [left, right], folded, span);
                }
                self.bad_operands(operator, left.ty, right.ty, span)
            }
            "<" | ">" | "<=" | ">=" | "==" | "!=" => {
                let comparable = (arithmetic
                    || self.types.is_pointer(left.ty) && self.types.is_pointer(right.ty))
                    || (self.types.is_pointer(left.ty) && self.null_pointer_constant(&right))
                    || (self.types.is_pointer(right.ty) && self.null_pointer_constant(&left));
                if comparable {
                    let int = self.types.fundamental("int").unwrap();
                    let op = match operator {
                        "==" => cir::CmpOp::Eq,
                        "!=" => cir::CmpOp::Ne,
                        "<" => cir::CmpOp::Less,
                        ">" => cir::CmpOp::Greater,
                        "<=" => cir::CmpOp::LessEqual,
                        _ => cir::CmpOp::GreaterEqual,
                    };
                    let operands = if arithmetic {
                        common()
                    } else if self.types.is_pointer(left.ty) {
                        left.ty
                    } else {
                        right.ty
                    };
                    let folded = folded(operands, left, right);
                    return self.emit_compare(op, operands, [left, right], int, folded, span);
                }
                self.bad_operands(operator, left.ty, right.ty, span)
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

    /// `left && right` and `left || right`, short-circuiting: the right side is parsed into its
    /// own block and only reached when the left side does not decide the answer.
    ///
    /// The result is an `int`, 0 or 1, carried out of the join block as its parameter.
    fn logical(&mut self, operator: &str, left: Value, level: usize, span: Span) -> Option<Value> {
        self.require_scalar(&left, "the left side of a logical operator");
        let int = self.types.fundamental("int").unwrap();
        let short_circuits_on = operator == "||";
        // Without code to emit, the parse still has to happen: diagnostics and folding.
        if left.cir.is_none() || !self.in_body() {
            let right = self.binary_expression(level + 1)?;
            let left_zero = self.constant_is_zero(&left);
            let constant = match (short_circuits_on, left.constant, right.constant) {
                (true, Some(_), _) if !left_zero => Some(1),
                (false, Some(_), _) if left_zero => Some(0),
                (_, Some(_), Some(_)) => Some(i64::from(!self.constant_is_zero(&right))),
                _ => None,
            };
            return Some(Value::new(int, constant, false));
        }
        // Each side is compared with zero in its own type: a pointer has no conversion to
        // `int`, and a NaN is a true condition rather than a conversion with no answer.
        let flag = self.compare_to_zero(left, cir::CmpOp::Ne, span)?;
        let take_right = self.new_block(span)?;
        let join = self.emit(|builder| builder.append_block(&[int], span))?;
        // The answer the left side alone settles: 1 for `||`, 0 for `&&`.
        let decided_value = if short_circuits_on { 1 } else { 0 };
        let decided = self.constant_of(int, decided_value, span);
        let Some(decided) = decided.cir else {
            return Some(Value::new(int, None, false));
        };
        if short_circuits_on {
            // `||`: a true left side answers 1 without looking right.
            self.emit(|builder| {
                builder.branch(flag, (join, vec![decided]), (take_right, Vec::new()), span)
            });
        } else {
            // `&&`: a false left side answers 0 without looking right.
            self.emit(|builder| {
                builder.branch(flag, (take_right, Vec::new()), (join, vec![decided]), span)
            });
        }
        self.switch_to(take_right);
        let right = self.binary_expression(level + 1)?;
        let right_flag = match self.compare_to_zero(right, cir::CmpOp::Ne, span) {
            Some(flag) => flag,
            None => return Some(Value::new(int, None, false)),
        };
        self.emit(|builder| builder.jump(join, vec![right_flag], span));
        self.switch_to(join);
        let value = self
            .body
            .as_ref()
            .map(|body| body.func.block(join).params[0])?;
        Some(Value::new(int, None, false).with_cir(Some(value)))
    }

    pub(super) fn bad_operands(
        &mut self,
        operator: &str,
        left: TypeId,
        right: TypeId,
        span: Span,
    ) -> Value {
        let (left, right) = (self.describe(left), self.describe(right));
        self.error(
            span,
            format!("`{operator}` cannot be applied to {left} and {right}"),
        );
        Value::temporary(self.types.error())
    }

    /// Fold `left op right` when both sides are floating constants, in the result's own width.
    /// The back end would compute the same at run time; the front end needs the number because a
    /// file-scope initialiser is bytes (ADR-004), and a global this cannot evaluate must not be
    /// written half-way.
    fn fold_float(&self, operator: &str, ty: TypeId, left: Value, right: Value) -> Option<i64> {
        let single = self.types.size_of(ty) == Some(4);
        // An integer side is converted first, as C's usual arithmetic conversions do; the value
        // is a constant, so the conversion is exact in `f64` and rounded once to `f32`.
        let value_of = |value: Value| -> Option<f64> {
            if self.types.is_float(value.ty) {
                let bits = value.constant?;
                Some(if single {
                    f64::from(f32::from_bits(bits as u32))
                } else {
                    f64::from_bits(bits as u64)
                })
            } else if self.types.is_integer(value.ty) {
                Some(value.constant? as f64)
            } else {
                None
            }
        };
        let (left, right) = (value_of(left)?, value_of(right)?);
        let folded = match operator {
            "+" => left + right,
            "-" => left - right,
            "*" => left * right,
            "/" => left / right,
            _ => return None,
        };
        Some(if single {
            i64::from((folded as f32).to_bits())
        } else {
            folded.to_bits() as i64
        })
    }

    /// An array or function used as a value is a pointer to it (C99 6.3.2.1). Everything that
    /// *uses* a value calls this; a place that assigns to one does not.
    pub(super) fn decay(&mut self, value: Value) -> Value {
        let ty = self.types.decay(value.ty);
        if ty == value.ty {
            return value;
        }
        // Decay is the one place an array becomes a pointer (ADR-004): its value is already the
        // address of the array, typed `pointer to array[N]`, so the conversion C asks for here
        // is a bitcast to `pointer to element` - and every use sees only the result. A record or
        // a pointer returns above: neither decays, and a record is not loaded anywhere.
        let Some(address) = value.cir else {
            // At file scope the array's address is the symbol itself, and the decay is a change of
            // type with nothing to compute - and an array whose base is a *constant* address
            // (`((char *)0)[i]`) keeps its constant through the decay too, which is the half of the
            // value that a subscript's walk reads.
            return Value::new(ty, value.constant, false).with_address(value.address);
        };
        let span = self.span();
        let from = if self.types.value_is_address(value.ty) {
            self.pointer_to(value.ty)
        } else {
            value.ty
        };
        match self.emit(|builder| builder.convert(cir::ConvertOp::Bitcast, address, from, ty, span))
        {
            Some(decayed) => Value::code(ty, decayed),
            None => Value::temporary(ty),
        }
    }

    pub(super) fn require_scalar(&mut self, value: &Value, what: &str) {
        if self.types.is_error(value.ty) {
            return;
        }
        // An array is a scalar here because it does not stay one: C99 6.3.2.1p3 decays it to a
        // pointer wherever a value is wanted, and a condition is such a place - every caller reaches
        // `compare_to_zero`, which decays. Lua's `api_check(L, (idx <= top) && "index")` writes a
        // string literal as a condition, and this check refused it before the decay could happen.
        if !self.types.is_scalar(value.ty)
            && !matches!(self.types.kind(value.ty), Type::Array { .. })
        {
            let ty = self.describe(value.ty);
            let span = self.span();
            self.error(span, format!("{what} must be a scalar, not {ty}"));
        }
    }

    pub(super) fn unary_expression(&mut self) -> Option<Value> {
        // Unary operators recurse into themselves and no further expression rule does: `!!!!x`,
        // `----x`, `(int)(int)(int)x` and their like are this function calling itself.
        self.within_nesting(Self::unary_expression_inner)
    }

    fn unary_expression_inner(&mut self) -> Option<Value> {
        let operator = self.peek().map(|token| self.spell(token).to_owned())?;
        let span = self.span();
        match operator.as_str() {
            "-" | "+" | "~" | "!" => {
                self.bump();
                let value = self.unary_expression()?;
                // A complex value accepts the two signs and `!`; `~` needs an integer (C99 6.5.3.3).
                if matches!(operator.as_str(), "-" | "+") {
                    if let Some(complex) = self.complex_unary(&operator, value, span) {
                        return Some(complex);
                    }
                }
                let ty = value.ty;
                match operator.as_str() {
                    "-" => {
                        if !self.types.is_arithmetic(ty) {
                            let ty = self.describe(ty);
                            self.error(span, format!("unary `-` needs a number, not {ty}"));
                            return Some(Value::temporary(self.types.error()));
                        }
                        let ty = self.types.promote(ty);
                        // A float's constant is a bit pattern, so it is negated as a number.
                        // The type is asked *first*: an `int` is also four bytes, and reading
                        // its constant as an `f32` wrote `-3` as -2147483645.
                        let folded = value.constant.map(|bits| {
                            if !self.types.is_float(ty) {
                                bits.wrapping_neg()
                            } else if self.types.size_of(ty) == Some(4) {
                                i64::from((-f32::from_bits(bits as u32)).to_bits())
                            } else {
                                (-f64::from_bits(bits as u64)).to_bits() as i64
                            }
                        });
                        Some(self.emit_unary(cir::UnOp::Neg, value, ty, folded, span))
                    }
                    // Unary `+` is the identity after promotion, and promotion can be a
                    // conversion (`+c` for a `char` is an `int`), so it is `convert` and not a
                    // relabel: relabelling dropped the value. Like `-`, it needs a number
                    // (C99 6.5.3.3p1): a pointer is not one.
                    "+" => {
                        if !self.types.is_arithmetic(ty) {
                            let ty = self.describe(ty);
                            self.error(span, format!("unary `+` needs a number, not {ty}"));
                            return Some(Value::temporary(self.types.error()));
                        }
                        let ty = self.types.promote(ty);
                        Some(self.convert(value, ty, span))
                    }
                    "~" => {
                        if !self.types.is_integer(ty) {
                            let ty = self.describe(ty);
                            self.error(span, format!("`~` needs an integer, not {ty}"));
                            return Some(Value::temporary(self.types.error()));
                        }
                        let ty = self.types.promote(ty);
                        let folded = value.constant.map(|value| !value);
                        Some(self.emit_unary(cir::UnOp::Not, value, ty, folded, span))
                    }
                    _ => {
                        self.require_scalar(&value, "`!`");
                        let int = self.types.fundamental("int").unwrap();
                        let folded = value
                            .constant
                            .map(|_| i64::from(self.constant_is_zero(&value)));
                        match self.compare_to_zero(value, cir::CmpOp::Eq, span) {
                            Some(cir) => Some(Value::code(int, cir).with_constant(folded)),
                            None => Some(Value::new(int, folded, false)),
                        }
                    }
                }
            }
            "&" => {
                self.bump();
                // C99 6.5.3.2p3: the `&` of a function designator is that designator, because a
                // function name already *is* an address and is never an lvalue - so the rule below
                // refuses it. `lua_atpanic(L, &panic)` is how Lua hands a function over, and the
                // value the name produces is exactly the pointer this operator would return.
                if self.operand_is_a_function_designator() {
                    return self.primary_expression();
                }
                let value = self.unary_expression()?;
                if self.types.is_error(value.ty) {
                    return Some(Value::temporary(self.types.error()));
                }
                if value.bits.is_some() {
                    // C99 6.5.3.2p1: a bitfield has no address of its own - the only address
                    // there is belongs to its whole storage unit.
                    self.error(span, "the address of a bitfield cannot be taken");
                    return Some(Value::temporary(self.types.error()));
                }
                // C99 6.5.3.2p1: the operand must be an lvalue. That is a fact about its *type*,
                // so `sizeof(&x)` is bound by it whether or not the operand is evaluated - which
                // is what separates this check from the place below.
                if !value.lvalue {
                    self.error(span, "`&` needs something it can take the address of");
                    return Some(Value::temporary(self.types.error()));
                }
                let pointer = self.types.pointer(value.ty);
                // `sizeof(&x)` asks for a *type*: the operand is not evaluated (C99 6.5.3.4p2),
                // and a place may need code nothing will run - `b[0]` of an array in a frame,
                // whose address is arithmetic on a slot. So an unevaluated `&` is its pointer
                // type and no value, which is all its caller can ask about.
                if self.unevaluated {
                    return Some(Value::new(pointer, None, false));
                }
                let Some(place) = value.place else {
                    // A place with no code: a file-scope object, whose address is its symbol plus
                    // however far into the object it was reached - `&x`, `&arr`, `&a[i]` and
                    // `&s.member` are all that, because a walk at file scope *is* an offset.
                    if value.address.is_some() {
                        return Some(Value::new(pointer, None, false).with_address(value.address));
                    }
                    // And a *constant* place is a constant address: `&((char*)0)[5]` is 5, which is
                    // how a table at file scope holds integers made into pointers. No relocation,
                    // because there is nothing for the linker to resolve.
                    if let Some(at) = value.constant {
                        return Some(Value::new(pointer, Some(at), false));
                    }
                    self.not_lowered(span, "the address of this kind of place");
                    return Some(Value::temporary(self.types.error()));
                };
                let mut result = Value::new(pointer, None, false);
                result.cir = Some(place);
                result.place = Some(place);
                Some(result)
            }
            "*" => {
                self.bump();
                let value = self.unary_expression().map(|value| self.decay(value))?;
                if self.types.is_error(value.ty) {
                    return Some(Value::temporary(self.types.error()));
                }
                if !self.types.is_pointer(value.ty) {
                    let ty = self.describe(value.ty);
                    self.error(span, format!("`*` needs a pointer, not {ty}"));
                    return Some(Value::temporary(self.types.error()));
                }
                // `*p` is the place `p` points at, and reading it is the load the emitter
                // writes - the same place-and-load `p[i]` is built from.
                let element = self.types.pointee(value.ty);
                match value.cir {
                    Some(place) => Some(self.read_place(element, place, span)),
                    None => Some(Value::new(element, None, true)),
                }
            }
            "++" | "--" => {
                let down = self.at_punct("--");
                self.bump();
                let value = self.unary_expression()?;
                Some(self.step(value, down, false, span))
            }
            "sizeof" => {
                self.bump();
                Some(self.sizeof_expression())
            }
            // The two halves of a complex value, which `<complex.h>` is written in terms of:
            // `creal` and `cimag` are these, and a header cannot define them any other way.
            "__real__" | "__imag__" => {
                self.bump();
                let value = self.unary_expression()?;
                Some(self.complex_real_or_imaginary(value, operator == "__imag__", span))
            }
            "_Generic" => self.generic_selection(),
            "_Alignof" => {
                self.bump();
                Some(self.alignof_expression(span))
            }
            "__builtin_offsetof" => {
                self.bump();
                Some(self.offsetof_expression(span))
            }
            "__builtin_va_start" => {
                self.bump();
                Some(self.va_start_expression(span))
            }
            "__builtin_va_arg" => {
                self.bump();
                Some(self.va_arg_expression(span))
            }
            // The infinities and the quiet NaN, which is how `<math.h>` spells `INFINITY` and
            // `NAN` on every compiler that has them (C99 7.12p4).
            "__builtin_atomic_cas" => {
                self.bump();
                Some(self.atomic_cas_expression(span))
            }
            // C11's generic atomics, answered from the *object's* type instead of from a `_Generic`
            // table. `_Generic` cannot say "any pointer type": the arm would have to name the
            // program's own - `mimalloc` exchanges a `mi_heap_t *volatile *`, and no header can know
            // that name. The compiler can, because the object is an expression it has already typed.
            "__builtin_atomic_exchange" => {
                self.bump();
                Some(self.atomic_generic(true, span))
            }
            "__builtin_atomic_compare_exchange" => {
                self.bump();
                Some(self.atomic_generic(false, span))
            }
            // The SDK's segment reads: `__readgsqword(0x30)` is `NtCurrentTeb()`, and half of
            // `winnt.h`'s inline functions are a field of it. Recognised by name because that is
            // what they *are* - one instruction each, with a constant offset, which is why the SDK
            // declares them rather than writing them.
            "__readfsbyte" | "__readfsword" | "__readfsdword" | "__readfsqword"
            | "__readgsbyte" | "__readgsword" | "__readgsdword" | "__readgsqword" => {
                self.bump();
                Some(self.segment_read(&operator, span))
            }
            "__builtin_inf" | "__builtin_inff" | "__builtin_nan" | "__builtin_nanf" => {
                self.bump();
                Some(self.float_builtin(&operator, span))
            }
            _ => self.postfix_expression(),
        }
    }

    /// Whether the operand at the cursor - which `&` is about to take the address of - is *exactly*
    /// a function designator: parentheses and a function's name, and nothing else.
    ///
    /// A lookahead on the identifier alone answered this while every designator was bare, and it was
    /// wrong in both directions: `&f(x)->m` was taken as a designator and became `(&f)(x)->m`
    /// (lz4frame.c found that), and `&(f)` was not taken as one though it is. The question is about
    /// the token *range* the operand covers, so this one looks at the range: open parens, a name, the
    /// matching closes, end. A call cannot match, because its close paren follows more than a name.
    fn operand_is_a_function_designator(&self) -> bool {
        let mut ahead = 0;
        let mut opens = 0;
        while self
            .peek_at(ahead)
            .is_some_and(|token| self.is_punct(token, "("))
        {
            ahead += 1;
            opens += 1;
        }
        let Some(token) = self.peek_at(ahead).copied() else {
            return false;
        };
        if token.kind != crate::lex::TokenKind::Ident {
            return false;
        }
        let names_a_function = self
            .symbols
            .lookup(token.text)
            .is_some_and(|entry| matches!(entry.kind, Kind::Function));
        if !names_a_function {
            return false;
        }
        ahead += 1;
        for _ in 0..opens {
            let Some(close) = self.peek_at(ahead) else {
                return false;
            };
            if !self.is_punct(close, ")") {
                return false;
            }
            ahead += 1;
        }
        // ...and nothing may follow it: `&f(x)->m` is the address of a member of the call's result,
        // and calling the shortcut there made it `(&f)(x)->m` - the bug lz4frame.c found, and one
        // this rewrite brought back for exactly one commit by asking only the first half.
        !self.peek_at(ahead).is_some_and(|next| {
            ["(", "[", ".", "->"]
                .iter()
                .any(|operator| self.is_punct(next, operator))
        })
    }

    /// `++x` and `x++` (and their `--` forms): read the place, add or subtract one, write it
    /// back, and yield the new value for the prefix form or the old one for the postfix.
    pub(super) fn step(&mut self, value: Value, down: bool, postfix: bool, span: Span) -> Value {
        if self.types.is_error(value.ty) {
            return Value::temporary(value.ty);
        }
        let scalar = self.types.is_arithmetic(value.ty) || self.types.is_pointer(value.ty);
        if !value.lvalue || !scalar {
            self.error(span, "`++` and `--` need a variable holding a number");
            return Value::temporary(self.types.error());
        }
        if !self.types.is_modifiable(value.ty) {
            self.error(span, "cannot modify a `const` object");
            return Value::temporary(value.ty);
        }
        if let Some(bits) = value.bits {
            // `s.a++` steps in the field's own type and then merges through the write path, so a
            // value too wide for the field is truncated by the write - which is exactly what
            // `s.a = s.a + 1` does.
            let ty = value.ty;
            let Some(current) = value.cir else {
                // Dead code: the place emitted nothing, so there is nothing to step either.
                return Value::temporary(ty);
            };
            let one = self.constant_of(ty, 1, span);
            let op = if down {
                cir::BinOp::Sub
            } else {
                cir::BinOp::Add
            };
            let stepped = self.emit_binary(op, ty, ty, [Value::code(ty, current), one], None, span);
            let written = self.write_bitfield(value, stepped, bits, span);
            let result = if postfix { value } else { written };
            return Value::new(ty, result.constant, false).with_cir(result.cir);
        }
        let ty = value.ty;
        let written = if self.types.is_pointer(ty) {
            // `p++` walks one element, which is the same "scale by the pointee" the `+` operator
            // does; a pointer is not "plus one byte".
            let int = self.types.fundamental("int").unwrap();
            let one = self.constant_of(int, 1, span);
            self.pointer_walk(ty, value, one, !down, span)
        } else {
            let one = self.constant_of(ty, 1, span);
            let op = if down {
                cir::BinOp::Sub
            } else {
                cir::BinOp::Add
            };
            self.emit_binary(op, ty, ty, [value, one], None, span)
        };
        // Store the new value back where it came from; the result is that value for a prefix
        // form and the previous one for a postfix form.
        let Some(place) = value.place else {
            self.not_lowered(span, "the address of this kind of place");
            return Value::temporary(ty);
        };
        // The operand is a place and the written value has `ty`, so no code on `written`
        // means the place emitted none: there is nothing to store back either.
        if let Some(cir) = written.cir {
            self.emit(|builder| builder.store(place, cir, span));
        }
        let result = if postfix { value } else { written };
        Value::new(ty, result.constant, false).with_cir(result.cir)
    }

    /// `sizeof` a type name or an expression, as a constant of type `size_t` (here `unsigned
    /// long long`, the widest integer this front end has).
    /// `__builtin_inff()`, `__builtin_nanf("")` and their `double` forms: a floating constant that
    /// is not a number a literal can spell.
    ///
    /// `NAN` cannot be `0.0/0.0` in a header, because the NaN that division produces is the
    /// target's *indefinite* one and its sign bit is set: `printf("%f", NAN)` printed
    /// `-nan(ind)` here where `clang` printed `nan`. The value here is the quiet NaN with a clear
    /// sign bit, which is what `0x7FC00000` and `0x7FF8000000000000` are.
    fn float_builtin(&mut self, word: &str, span: Span) -> Value {
        let single = word.ends_with('f');
        if self.eat("(") {
            // The argument is the NaN's payload, and it is a string this does not read. Every
            // spelling a program writes is `""`, which is the quiet NaN the constants above are;
            // a payload that is *not* empty asks for different bits, and answering with the
            // default ones is a wrong number rather than a smaller one - so it is refused.
            let payload = matches!(
                self.peek().map(|token| token.kind),
                Some(crate::lex::TokenKind::Str)
            ) && self.spell(self.peek().unwrap()) != "\"\"";
            if payload {
                self.error(span, "a NaN payload this compiler does not read yet");
            }
            self.skip_until(&[")"], true);
        } else {
            self.error(span, "this builtin needs `(`");
        }
        let names = match (single, word.contains("nan")) {
            (true, true) => ("float", 0x7FC0_0000u64),
            (false, true) => ("double", 0x7FF8_0000_0000_0000u64),
            (true, false) => ("float", 0x7F80_0000u64),
            (false, false) => ("double", 0x7FF0_0000_0000_0000u64),
        };
        let Some(ty) = self.types.fundamental(names.0) else {
            return Value::temporary(self.types.error());
        };
        // Through the same door as every other constant: inside a body that is an instruction, and
        // in an initialiser that writes bytes it is the bits themselves.
        self.constant_of(ty, names.1 as i64, span)
    }

    /// `__builtin_atomic_cas(pointer, expected, desired)`: swap `*pointer` from `expected` to
    /// `desired` if it holds `expected`, and give the value that was there - which is `expected`
    /// exactly when the swap happened.
    ///
    /// `_Atomic` is a language feature and the *machine* has one operation for it, so this is the
    /// one builtin `stdatomic.h` is written on: every other atomic operation in the header is a loop
    /// of these, in C, which is where the reasoning about them can be read.
    /// `__readgsqword(offset)`: the SDK's segment-relative read.
    ///
    /// The offset has to be a constant, and both SDKs require one - it is a displacement in the
    /// instruction, not an address. A read the compiler cannot place is refused rather than called,
    /// because a call would be a *function* this compiler does not have.
    fn segment_read(&mut self, name: &str, span: Span) -> Value {
        // The result types are the SDK's own declarations of these: `__int64` is `long long` here,
        // and `__LONG32` is `long` - the *spelling* matters because the header declared them that
        // way, and a program that passes the result to `printf("%ld")` is relying on it.
        let (segment, ty) = match name {
            "__readfsbyte" => (cir::Segment::Fs, "unsigned char"),
            "__readfsword" => (cir::Segment::Fs, "unsigned short"),
            "__readfsdword" => (cir::Segment::Fs, "unsigned long"),
            "__readfsqword" => (cir::Segment::Fs, "unsigned long long"),
            "__readgsbyte" => (cir::Segment::Gs, "unsigned char"),
            "__readgsword" => (cir::Segment::Gs, "unsigned short"),
            "__readgsdword" => (cir::Segment::Gs, "unsigned long"),
            _ => (cir::Segment::Gs, "unsigned long long"),
        };
        let result_ty = self
            .types
            .fundamental(ty)
            .unwrap_or_else(|| self.types.error());
        if !self.eat("(") {
            self.error_at_current(format!("`{name}` needs `(`"));
            return Value::temporary(result_ty);
        }
        // The offset is any integer expression: `mov rax, gs:[reg]` takes a register as readily as
        // `gs:[0x30]` takes a displacement, and a library's thread-local slot is a number it
        // computed (`mimalloc`'s is `slot * sizeof(void*)`). It is converted to the width an
        // address has, which is also what the SDK declared the parameter to be.
        let argument = match self.assignment_expression() {
            Some(argument) => argument,
            None => {
                self.error(span, format!("`{name}` needs an offset"));
                return Value::temporary(result_ty);
            }
        };
        if !self.types.is_integer(argument.ty) && !self.types.is_error(argument.ty) {
            let given = self.describe(argument.ty);
            self.error(
                span,
                format!("`{name}` reads at a byte offset, and {given} is not a number"),
            );
            return Value::temporary(result_ty);
        }
        let wide = self
            .types
            .fundamental("unsigned long long")
            .unwrap_or_else(|| self.types.error());
        // An offset is an address, and an address is a *value* here: a constant one is
        // materialised, because the encoder takes a register. One `mov` buys the general form, and
        // `gs:[reg]` is what a library's computed slot needs - `mimalloc`'s is `slot * 8`.
        let converted = self.convert(argument, wide, span);
        let address = self.with_code(converted, span);
        if !self.eat(")") {
            self.error_at_current(format!("expected `)` after the offset of `{name}`"));
        }
        let Some(address) = address.cir else {
            return Value::temporary(result_ty);
        };
        match self.emit(|builder| builder.segment_read(segment, address, result_ty, span)) {
            Some(value) => Value::code(result_ty, value),
            None => Value::temporary(result_ty),
        }
    }

    fn atomic_cas_expression(&mut self, span: Span) -> Value {
        if !self.eat("(") {
            self.error_at_current("`__builtin_atomic_cas` needs `(`");
            return Value::temporary(self.types.error());
        }
        let Some(pointer) = self.assignment_expression() else {
            return Value::temporary(self.types.error());
        };
        let pointer = self.decay(pointer);
        if !self.eat(",") {
            self.error_at_current("expected `,` after the address");
        }
        let expected = self
            .assignment_expression()
            .unwrap_or_else(|| Value::temporary(self.types.error()));
        if !self.eat(",") {
            self.error_at_current("expected `,` after the expected value");
        }
        let desired = self
            .assignment_expression()
            .unwrap_or_else(|| Value::temporary(self.types.error()));
        if !self.eat(")") {
            self.error_at_current("expected `)` after the three arguments");
        }
        if !self.types.is_pointer(pointer.ty) {
            let ty = self.describe(pointer.ty);
            self.error(
                span,
                format!("an atomic operation needs an address, not {ty}"),
            );
            return Value::temporary(self.types.error());
        }
        let object = self.types.pointee(pointer.ty);
        // `lock cmpxchg` has a form for 1, 2, 4 and 8 bytes - the four widths an integer has here -
        // so any integer is a legal object and anything else is refused rather than swapped as the
        // wrong width.
        let integer = self.types.is_integer(object);
        if !integer {
            let ty = self.describe(object);
            self.error(
                span,
                format!("an atomic operation needs an integer, not {ty}"),
            );
            return Value::temporary(self.types.error());
        }
        let expected = self.convert(expected, object, span);
        let desired = self.convert(desired, object, span);
        let (Some(address), Some(expected), Some(desired)) =
            (pointer.cir, expected.cir, desired.cir)
        else {
            return Value::new(object, None, false);
        };
        match self.emit(|builder| builder.atomic_cas(address, expected, desired, object, span)) {
            Some(value) => Value::code(object, value),
            None => Value::new(object, None, false),
        }
    }

    /// `__builtin_offsetof(type-name, member.designator)`: the byte offset of a member.
    ///
    /// C99 7.17p3 defines `offsetof` as `(size_t)&(((type *)0)->member)`, which this compiler
    /// cannot fold: it has no pointer-to-integer conversion, and taking the address of a member of
    /// the null pointer has no address to give. The layout is already known here - that is what
    /// `member.offset` is - so the builtin asks the type table instead of asking the machine.
    ///
    /// `__builtin_va_start(ap, last)`: set the list to the first argument that is not a parameter.
    ///
    /// `last` is the final named parameter, and what the back end needs is how many parameters come
    /// before the `...` - which is `last`'s position. A definition's parameters are in scope here
    /// (C99 6.2.1p4), so each one's place is recorded as it is declared and compared by identity.
    fn va_start_expression(&mut self, span: Span) -> Value {
        let void = self.types.fundamental("void").unwrap();
        if !self.eat("(") {
            self.error_at_current("`__builtin_va_start` needs `(`");
            return Value::temporary(void);
        }
        let Some(list) = self.assignment_expression() else {
            self.error_at_current("`__builtin_va_start` needs a `va_list`");
            return Value::temporary(void);
        };
        if !self.eat(",") {
            self.error_at_current("`__builtin_va_start` needs the last parameter");
            return Value::temporary(void);
        }
        let Some(last) = self.assignment_expression() else {
            return Value::temporary(void);
        };
        if !self.eat(")") {
            self.error_at_current("expected `)`");
            return Value::temporary(void);
        }
        let params = self.function_params.clone();
        let position = params
            .iter()
            .position(|place| place.is_some() && *place == last.place)
            // The count of named parameters, not the index of the last one: `va_start` points at the
            // first argument *after* them.
            .map(|index| index as u8 + 1);
        let Some(skip) = position else {
            self.error(
                span,
                "the second argument of `va_start` must be the last named parameter",
            );
            return Value::temporary(void);
        };
        let char = self.types.fundamental("char").unwrap();
        let bytes = self.pointer_to(char);
        let Some(address) = self.emit(|builder| builder.va_start(bytes, skip, span)) else {
            return Value::temporary(void);
        };
        let value = Value::code(bytes, address);
        match list.place {
            Some(place) => self.store_at(place, list.ty, value, span),
            None => self.not_lowered(span, "a `va_list` that is not a local"),
        }
        Value::temporary(void)
    }

    /// C11 7.17.7.3's exchange and 7.17.7.4's compare-and-exchange, for **any** object type: an
    /// integer of 1, 2, 4 or 8 bytes, or a pointer, which is eight.
    ///
    /// They are here rather than in `<stdatomic.h>` because `_Generic` cannot say "any pointer type".
    /// The object's type decides everything - the width, the signedness, and which runtime function
    /// does the work - and the compiler is the only layer that has it. A failed compare-and-exchange
    /// writes the value it found back through `expected`, which is the standard's own behaviour and
    /// the runtime function's signature.
    fn atomic_generic(&mut self, exchange: bool, span: Span) -> Value {
        // **Parse every argument first, then judge.** An early return from inside the list leaves
        // the cursor between a `,` and a `)`, and the parser then complains about the *rest of the
        // declaration* - "declaration does not declare anything" for a mistake in the call.
        if !self.eat("(") {
            self.error_at_current("an atomic operation needs `(`");
            return Value::temporary(self.types.error());
        }
        let mut given: Vec<Value> = Vec::new();
        while let Some(value) = self.assignment_expression().map(|value| self.decay(value)) {
            given.push(value);
            if !self.eat(",") {
                break;
            }
        }
        if !self.eat(")") {
            self.error_at_current("expected `)` after the arguments");
        }
        let wanted = if exchange { 2 } else { 3 };
        if given.len() != wanted {
            self.error(
                span,
                format!(
                    "`{}` takes {wanted} arguments and was given {}",
                    if exchange {
                        "__builtin_atomic_exchange"
                    } else {
                        "__builtin_atomic_compare_exchange"
                    },
                    given.len()
                ),
            );
            return Value::temporary(self.types.error());
        }
        let object = given[0];
        if !self.types.is_pointer(object.ty) {
            let given = self.describe(object.ty);
            self.error(
                span,
                format!("an atomic operation needs an address, not {given}"),
            );
            return Value::temporary(self.types.error());
        }
        let pointee = self.types.pointee(object.ty);
        // `lock cmpxchg` and `xchg` have forms for 1, 2, 4 and 8 bytes, which is every integer plus
        // a pointer; anything else is refused rather than operated on at the wrong width.
        let Some((kind, parameter)) = self.atomic_kind(pointee) else {
            let given = self.describe(pointee);
            self.error(
                span,
                format!("an atomic operation needs an integer or a pointer, not {given}"),
            );
            return Value::temporary(self.types.error());
        };
        let mut parameters = vec![object.ty];
        let mut arguments = vec![object];
        if !exchange {
            // C11 7.17.7.4: `expected` is a pointer to *an object of the same type*, because a failed
            // exchange writes the value it found through it. Neither half was checked, and both were
            // wrong programs rather than diagnostics: a `char *` where an `int *` belongs let the
            // runtime write four bytes through a one-byte object, and something that was not a
            // pointer at all was accepted and crashed.
            let expected = given[1];
            if !self.types.is_pointer(expected.ty) {
                let it = self.describe(expected.ty);
                self.error(
                    span,
                    format!("a compare-and-exchange needs the address of the expected value, and these arguments give it {it}"),
                );
                return Value::temporary(self.types.error());
            }
            if !self.types.same(self.types.pointee(expected.ty), pointee) {
                let (expected_ty, object_ty) = (
                    self.describe(self.types.pointee(expected.ty)),
                    self.describe(pointee),
                );
                self.error(
                    span,
                    format!("`expected` points at `{expected_ty}` and the object is `{object_ty}`: a failed exchange writes what it found through the first, so the two must be the same type"),
                );
                return Value::temporary(self.types.error());
            }
            parameters.push(expected.ty);
            arguments.push(expected);
        }
        parameters.push(parameter);
        arguments.push(*given.last().expect("the last argument"));
        let result = if exchange {
            pointee
        } else {
            self.types
                .fundamental("_Bool")
                .unwrap_or_else(|| self.types.error())
        };
        let name = format!("__xcc_{}_{kind}", if exchange { "exchange" } else { "cas" });
        let function = self.types.function(result, parameters.clone(), false);
        let pointer = self.types.pointer(function);
        let Some(callee) = self.emit(|builder| builder.global_address(name, pointer, span)) else {
            return Value::new(result, None, false);
        };
        self.emit_call(
            Value::code(pointer, callee),
            &parameters,
            arguments,
            false,
            result,
            span,
        )
    }

    /// The suffix of the runtime function for an object of type `ty`, and the C type its value
    /// argument has. `None` for a type the machine has no atomic operation of that width for.
    fn atomic_kind(&mut self, ty: TypeId) -> Option<(&'static str, TypeId)> {
        let (kind, parameter) = match self.types.kind(ty) {
            crate::types::Type::Int {
                bits: 8,
                signed: true,
            } => ("schar", "signed char"),
            crate::types::Type::Int { bits: 8, .. } | crate::types::Type::Bool => {
                ("uchar", "unsigned char")
            }
            crate::types::Type::Int {
                bits: 16,
                signed: true,
            } => ("short", "short"),
            crate::types::Type::Int { bits: 16, .. } => ("ushort", "unsigned short"),
            crate::types::Type::Int {
                bits: 32,
                signed: true,
            } => ("int", "int"),
            crate::types::Type::Int { bits: 32, .. } => ("uint", "unsigned int"),
            crate::types::Type::Int {
                bits: 64,
                signed: true,
            } => ("llong", "long long"),
            crate::types::Type::Int { bits: 64, .. } => ("ullong", "unsigned long long"),
            crate::types::Type::Pointer(_) => ("ullong", "unsigned long long"),
            _ => return None,
        };
        let parameter = self.types.fundamental(parameter)?;
        Some((kind, parameter))
    }

    /// `__builtin_va_arg(ap, type)`: read one argument and step the list over its slot.
    ///
    /// The step is eight bytes because a Win64 argument slot is eight bytes whatever the type - a
    /// `char` argument still takes one - and it is written back into the list as an ordinary store, so
    /// the `cir` shows the list moving rather than an operation with a side effect of its own.
    fn va_arg_expression(&mut self, span: Span) -> Value {
        if !self.eat("(") {
            self.error_at_current("`__builtin_va_arg` needs `(`");
            return Value::temporary(self.types.error());
        }
        let Some(list) = self.assignment_expression() else {
            self.error_at_current("`__builtin_va_arg` needs a `va_list`");
            return Value::temporary(self.types.error());
        };
        if !self.eat(",") {
            self.error_at_current("`__builtin_va_arg` needs a type");
            return Value::temporary(self.types.error());
        }
        let Some(ty) = self.try_type_name() else {
            self.error_at_current("`__builtin_va_arg` needs a type");
            return Value::temporary(self.types.error());
        };
        if !self.eat(")") {
            self.error_at_current("expected `)`");
            return Value::temporary(self.types.error());
        }
        if !self.in_body() {
            self.not_lowered(span, "`va_arg` outside a function");
            return Value::temporary(ty);
        }
        let (Some(ap), Some(place)) = (list.cir, list.place) else {
            return Value::temporary(ty);
        };
        // C99 7.15.1.1: `va_arg` names the type as it was *passed*, which for a promoted argument is
        // the promoted type - an `int` for a `char`, because that is what the caller wrote.
        let promoted = self.types.promote(ty);
        let Some(value) = self.emit(|builder| builder.va_arg(promoted, ap, ty, span)) else {
            return Value::temporary(promoted);
        };
        // Then the step: eight bytes on, written back into the list.
        let char = self.types.fundamental("char").unwrap();
        let bytes = self.pointer_to(char);
        let list_value = Value::code(list.ty, ap);
        let eight = self.constant_of(
            self.types.fundamental("unsigned long long").unwrap(),
            8,
            span,
        );
        let stepped = self.pointer_walk(bytes, list_value, eight, true, span);
        if let Some(stepped) = stepped.cir {
            self.emit(|builder| builder.store(place, stepped, span));
        }
        Value::code(promoted, value)
    }

    /// `stddef.h` spells `offsetof` in terms of this, the way every compiler's own header does.
    fn offsetof_expression(&mut self, span: Span) -> Value {
        let size_type = self.types.fundamental("unsigned long long").unwrap();
        if !self.eat("(") {
            self.error_at_current("`__builtin_offsetof` needs `(`");
            return Value::temporary(self.types.error());
        }
        let Some(ty) = self.try_type_name() else {
            self.error_at_current("`__builtin_offsetof` needs a type");
            return Value::temporary(self.types.error());
        };
        if !self.eat(",") {
            self.error_at_current("expected `,` before the member");
            return Value::temporary(self.types.error());
        }
        // A member-designator: a name, then deeper names as `a.b.c`. An array subscript is
        // refused rather than computed wrong: it would need the element's size and the index,
        // which no caller in this corpus asks for and a C array member rarely uses.
        let mut offset = 0usize;
        let mut current = ty;
        loop {
            let Some(token) = self.peek().copied() else {
                self.error_at_current("expected a member name");
                return Value::temporary(self.types.error());
            };
            if token.kind != crate::lex::TokenKind::Ident {
                self.error_at_current("expected a member name");
                return Value::temporary(self.types.error());
            }
            self.bump();
            let Some(record) = self.types.record_of(current) else {
                let ty = self.describe(current);
                self.error(span, format!("`{ty}` has no members"));
                return Value::temporary(self.types.error());
            };
            let Some(member) = self.types.member(record, token.text) else {
                let name = self.spell(&token).to_owned();
                let ty = self.describe(current);
                self.error(span, format!("`{ty}` has no member `{name}`"));
                return Value::temporary(self.types.error());
            };
            offset += member.offset as usize;
            current = member.ty;
            if self.at_punct(".") {
                self.bump();
                continue;
            }
            break;
        }
        if !self.eat(")") {
            self.error_at_current("expected `)` after the member");
        }
        self.constant_of(size_type, offset as i64, span)
    }

    pub(super) fn sizeof_expression(&mut self) -> Value {
        let size_type = self.types.fundamental("unsigned long long").unwrap();
        let span = self.span();
        if let Some(ty) = self.parenthesised_type() {
            if self.types.is_error(ty) {
                return Value::temporary(self.types.error());
            }
            return match self.types.size_of(ty) {
                // A constant in a body is a value like any other, and needs its instruction:
                // a literal gets one in `primary_expression` for the same reason.
                Some(size) => self.constant_of(size_type, size as i64, span),
                None => {
                    let typo = self.describe(ty);
                    let span = self.span();
                    self.error(span, format!("`sizeof` needs a complete type, not {typo}"));
                    Value::temporary(self.types.error())
                }
            };
        }
        // C99 6.5.3.4p2: the operand is not evaluated. A variable length array would be the
        // exception, and this compiler has none.
        let saved = std::mem::replace(&mut self.unevaluated, true);
        let value = self.unary_expression();
        self.unevaluated = saved;
        let Some(value) = value else {
            return Value::temporary(self.types.error());
        };
        if self.types.is_error(value.ty) {
            return Value::temporary(self.types.error());
        }
        match self.types.size_of(value.ty) {
            Some(size) => self.constant_of(size_type, size as i64, span),
            None => {
                let ty = self.describe(value.ty);
                let span = self.span();
                self.error(span, format!("`sizeof` needs a complete type, not {ty}"));
                Value::temporary(self.types.error())
            }
        }
    }

    pub(super) fn postfix_expression(&mut self) -> Option<Value> {
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
                let arrow = self.spell(self.peek().expect("checked")) == "->";
                self.bump();
                value = self.member_access(value, arrow, span);
                continue;
            }
            if self.at_punct("++") || self.at_punct("--") {
                let down = self.at_punct("--");
                self.bump();
                value = self.step(value, down, true, span);
                continue;
            }
            return Some(value);
        }
    }

    pub(super) fn subscript(&mut self, base: Value, index: Value, span: Span) -> Value {
        let base = self.decay(base);
        if self.types.is_error(base.ty) || self.types.is_error(index.ty) {
            return Value::temporary(self.types.error());
        }
        if !self.types.is_integer(index.ty) {
            let ty = self.describe(index.ty);
            self.error(span, format!("a subscript must be an integer, not {ty}"));
            return Value::temporary(self.types.error());
        }
        match self.types.kind(base.ty) {
            Type::Pointer(element) | Type::Array { element, .. } => {
                let element = *element;
                // `base[index]` is `*(base + index)`: the address is a place, and reading it is a
                // load the emitter writes.
                let pointer = self.types.pointer(element);
                let address = self.pointer_walk(pointer, base, index, true, span);
                match address.cir {
                    Some(place) => self.read_place(element, place, span),
                    // No code: at file scope the walk answered with a symbol and an offset, or with
                    // a constant. Either way the *place* is that address - reading it here is not a
                    // constant, but `&a[i]` is the address itself, which is what a relocation (or,
                    // for a constant base, the bytes) needs.
                    None => Value::new(element, None, true)
                        .with_address(address.address)
                        .with_constant(address.constant),
                }
            }
            _ => {
                let ty = self.describe(base.ty);
                self.error(span, format!("`[]` needs a pointer or an array, not {ty}"));
                Value::temporary(self.types.error())
            }
        }
    }

    /// A call: the arguments are parsed and checked against the declared parameters.
    pub(super) fn call(&mut self, callee: Value, span: Span) -> Value {
        if self.types.is_error(callee.ty) {
            self.skip_until(&[")"], true);
            return Value::temporary(self.types.error());
        }
        let function = match self.types.kind(callee.ty) {
            Type::Function { .. } => callee.ty,
            Type::Pointer(to) if matches!(self.types.kind(*to), Type::Function { .. }) => *to,
            _ => {
                let ty = self.describe(callee.ty);
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
        let mut surplus = 0usize;
        let mut values: Vec<Value> = Vec::new();
        if !self.eat(")") {
            loop {
                let argument = self.assignment_expression().map(|value| self.decay(value));
                let Some(argument) = argument else { break };
                if self.types.is_error(argument.ty) {
                    given += 1;
                    values.push(argument);
                    if !self.eat(",") {
                        break;
                    }
                    continue;
                }
                if given < params.len() {
                    let expected = params[given];
                    let null_pointer = self.null_pointer_constant(&argument);
                    if !null_pointer && !self.types.is_assignable(expected, argument.ty) {
                        let (from, to) = (self.describe(argument.ty), self.describe(expected));
                        self.error(
                            span,
                            format!("argument {} is {from}, expected {to}", given + 1),
                        );
                    }
                } else if !variadic {
                    surplus += 1;
                }
                values.push(argument);
                given += 1;
                if !self.eat(",") {
                    break;
                }
            }
            if !self.eat(")") {
                self.error_at_current("expected `)` to end the arguments");
            }
        }
        let expected = params.len();
        let wrong = if variadic {
            given < expected
        } else {
            given != expected || surplus > 0
        };
        if wrong {
            let takes = if variadic {
                format!("at least {}", arguments(expected))
            } else {
                arguments(expected)
            };
            self.error(span, format!("this function takes {takes}, {given} given"));
        }
        // C99 6.5.2.2p6: an argument with no parameter type gets the *default argument
        // promotions* - `char`, `short` and `_Bool` to `int`, `float` to `double` - and it is the
        // promoted type that crosses the ABI. This is the one place the variadic form differs
        // from the declared one, which is why it is done here and not in `emit_call`.
        if variadic {
            for slot in values.iter_mut().skip(params.len()) {
                let argument = *slot;
                if self.types.is_error(argument.ty) {
                    continue;
                }
                // A `float` becomes a `double` here, and a `double` crosses the ABI twice: Win64
                // duplicates a variadic floating-point argument into the general register of its
                // own position, which is what lets a callee read the whole list with one 8-byte
                // walk (ADR-006). That duplicate is the back end's business, so nothing is refused
                // here - level 0 and 1 cannot do it and say so themselves.
                let promoted = self.types.default_argument(argument.ty);
                *slot = self.convert(argument, promoted, span);
            }
        }
        let implicit = self.implicit_types.contains(&function);
        let called = self.emit_call(callee, &params, values, variadic, result, span);
        if implicit {
            if let Some(value) = called.cir {
                self.implicit_results.insert(value);
            }
        }
        called
    }

    /// Whether this value is what a call to a function nothing declared returned.
    ///
    /// C89 took such a result to be an `int` and let it be assigned to a pointer, because where C89
    /// compilers ran, the two were one width; MSVC still does, with warning C4047. That is the one
    /// place this compiler follows it - an `int` from anywhere *else* still needs a cast.
    fn returned_by_an_implicit_call(&self, value: &Value) -> bool {
        value
            .cir
            .is_some_and(|cir| self.implicit_results.contains(&cir))
    }

    /// The call itself: arguments converted to the parameter types, and the result value.
    fn emit_call(
        &mut self,
        callee: Value,
        params: &[TypeId],
        values: Vec<Value>,
        variadic: bool,
        result: TypeId,
        span: Span,
    ) -> Value {
        if !self.in_body() {
            return Value::new(result, None, false);
        }
        let Some(callee) = callee.cir else {
            return Value::new(result, None, false);
        };
        // Win64: a record result lives in a buffer the caller owns, so the address goes first;
        // a record small enough for a register comes back in it instead.
        let aggregate = self.types.record_of(result).is_some();
        let buffer = if aggregate {
            let pointer = self.pointer_to(result);
            self.emit(|builder| builder.slot(result, pointer, span))
        } else {
            None
        };
        let mut args: Vec<cir::Value> = match (aggregate, buffer) {
            (true, Some(buffer)) if !self.record_in_register(result) => vec![buffer],
            _ => Vec::new(),
        };
        for (position, argument) in values.into_iter().enumerate() {
            // An argument past the last parameter belongs to a variadic call: it is already
            // converted to its promoted type, so it "wants" to be itself - and the record rules
            // below still apply to it, because a record of eight bytes or fewer crosses the ABI
            // as an integer of its width, which is what `signature_of` has said already. A
            // surplus argument to a non-variadic call was diagnosed, and its body dropped.
            let wanted = match params.get(position).copied() {
                Some(wanted) => wanted,
                None if variadic => argument.ty,
                None => continue,
            };
            let converted = self.convert(argument, wanted, span);
            let Some(value) = converted.cir else {
                return Value::new(result, None, false);
            };
            if self.types.record_of(wanted).is_none() {
                args.push(value);
                continue;
            }
            if self.record_in_register(wanted) {
                // The record's bytes, loaded as one integer of its own width.
                let integer = self.record_integer(wanted);
                let Some(address) = self.address_within(value, wanted, 0, integer, span) else {
                    return Value::new(result, None, false);
                };
                match self.emit(|builder| builder.load(address, integer, span)) {
                    Some(loaded) => args.push(loaded),
                    None => return Value::new(result, None, false),
                }
                continue;
            }
            // By reference: the caller owns the copy the callee may keep, so the argument is
            // copied into a slot of this frame and its address is what travels.
            let pointer = self.pointer_to(wanted);
            let Some(copy) = self.emit(|builder| builder.slot(wanted, pointer, span)) else {
                return Value::new(result, None, false);
            };
            self.copy_object(copy, wanted, value, wanted, wanted, span);
            args.push(copy);
        }
        let returns = if matches!(self.types.kind(result), Type::Void) {
            None
        } else if aggregate {
            Some(self.abi_value(result))
        } else {
            Some(result)
        };
        let called = self.emit(|builder| builder.call(callee, args, variadic, returns, span));
        let Some(called) = called else {
            return Value::new(result, None, false);
        };
        if !aggregate {
            return match called {
                Some(value) => Value::code(result, value),
                None => Value::new(result, None, false),
            };
        }
        let Some(buffer) = buffer else {
            return Value::new(result, None, false);
        };
        if let Some(returned) = called.filter(|_| self.record_in_register(result)) {
            // A register-carried result is stored into the slot the same way a parameter's
            // integer is: the object the expression's value points at.
            let integer = self.record_integer(result);
            if let Some(address) = self.address_within(buffer, result, 0, integer, span) {
                self.emit(|builder| builder.store(address, returned, span));
            }
        }
        // The value of a call returning a record is the object it was written into - and it is a
        // *value* (C99 6.5.2.2p5), not an lvalue: `make().x = 1` writes into an object the program
        // never named, and this said it was writable until the aggregate `?:` made the question
        // easy to ask. The buffer is still where the bytes are, which is what reading it needs.
        Value {
            ty: result,
            constant: None,
            lvalue: false,
            cir: Some(buffer),
            place: None,
            bits: None,
            address: None,
        }
    }

    pub(super) fn primary_expression(&mut self) -> Option<Value> {
        let token = self.peek().copied()?;
        let span = token.span;
        match token.kind {
            crate::lex::TokenKind::Number => {
                self.bump();
                let spelling = self.spell(&token).to_owned();
                let value = self.number_value(&spelling, span);
                // A constant is a value like any other: `int x = 5;` stores one. `constant_of`
                // is the one place that knows a constant keeps its folded value when it gains
                // code, so this branch asks it rather than repeating the rule.
                Some(match (value.constant, self.types.is_integer(value.ty)) {
                    (Some(bits), true) => self.constant_of(value.ty, bits, span),
                    _ => value,
                })
            }
            crate::lex::TokenKind::Chr => {
                self.bump();
                // C99 6.4.4.4p10 and C11 6.4.4.4p12: the prefix picks the type - `wchar_t` for
                // `L` (two bytes here), `char16_t` and `char32_t` for `u` and `U`, `char` for `u8`,
                // and a plain `int` for none of them.
                let int = match crate::literal::prefix_of(self.spell(&token)) {
                    crate::literal::Prefix::Wide | crate::literal::Prefix::Utf16 => self
                        .types
                        .fundamental("unsigned short")
                        .unwrap_or_else(|| self.types.error()),
                    crate::literal::Prefix::Utf32 => self
                        .types
                        .fundamental("unsigned int")
                        .unwrap_or_else(|| self.types.error()),
                    // C11 6.4.4.4 has no `u8` character constant - `u8'a'` is C23's - and accepting
                    // it would be accepting a program the standard this compiler claims does not
                    // have. `u8"..."` *is* C11's, which is why only this one is refused.
                    crate::literal::Prefix::Utf8 => {
                        self.error(span, "a `u8` character constant is C23's, not C11's");
                        self.types.fundamental("int").unwrap()
                    }
                    crate::literal::Prefix::None => self.types.fundamental("int").unwrap(),
                };
                // A constant this does not read is refused, not turned into zero.
                match character_constant(self.spell(&token)) {
                    Some(value) => Some(self.constant_of(int, value, span)),
                    None => {
                        let spelling = self.spell(&token).to_owned();
                        self.error(
                            span,
                            format!("`{spelling}` is not a character constant this compiler reads"),
                        );
                        Some(Value::temporary(int))
                    }
                }
            }
            crate::lex::TokenKind::Str => {
                // A string literal has an array type of its own (C99 6.2.5p20), which is what
                // makes `sizeof "abc"` four and lets an initialiser take its length.
                let (ty, name, _) = self.string_literal();
                Some(self.literal_value(ty, name, span))
            }
            crate::lex::TokenKind::Punct if self.is_punct(&token, "(") => {
                // `(int)x` is a cast; `(x)` groups.
                if let Some(ty) = self.parenthesised_type() {
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
                self.error(
                    span,
                    format!(
                        "expected an expression, found {:?} `{}`",
                        token.kind,
                        self.spell(&token)
                    ),
                );
                None
            }
        }
    }

    /// The value of `__func__`: the array a string literal would be, holding the current function's
    /// name. One object per function, remembered, because a macro that logs it is expanded many times.
    fn function_name_value(&mut self, span: Span) -> Value {
        let function = self.current_function.expect("a function is being read");
        if let Some((ty, name)) = self.func_literals.get(&function) {
            let (ty, name) = (*ty, name.clone());
            return self.literal_value(ty, name, span);
        }
        let spelled = self.spell_of(function).to_owned();
        let mut bytes = spelled.into_bytes();
        bytes.push(0);
        // `static const char __func__[]` (C99 6.4.2.2p1): the qualifier is on the *element*, so
        // `__func__[0] = 'x'` is a write to a `const` object and is refused. A string literal is
        // not const-qualified in C - it is the program's business whether to write to one - and
        // that difference is the reason this is not simply "a literal with a name".
        let character = self
            .types
            .fundamental("char")
            .map(|character| self.types.constant(character))
            .unwrap_or_else(|| self.types.error());
        let ty = self.types.array(character, Some(bytes.len() as u64));
        let name = self.literal_object(ty, &bytes, span);
        self.func_literals.insert(function, (ty, name.clone()));
        self.literal_value(ty, name, span)
    }

    /// The value of a literal object: its type is the array, its value is the address of the object
    /// (ADR-004), and it is an lvalue because a literal is an object the program may look at.
    fn literal_value(&mut self, ty: TypeId, name: String, span: Span) -> Value {
        let pointer = self.types.pointer(ty);
        let symbol = self.preprocessed.intern(&name);
        let mut value = Value::new(ty, None, true).with_address(Some((symbol, 0)));
        if let Some(address) = self.emit(|builder| builder.global_address(name, pointer, span)) {
            value.cir = Some(address);
            value.place = Some(address);
        }
        value
    }

    /// `a.b` and `p->b`: the member's type, and whether it can be assigned to.
    pub(super) fn member_access(&mut self, base: Value, arrow: bool, span: Span) -> Value {
        let Some(token) = self.peek().copied() else {
            return Value::temporary(self.types.error());
        };
        if token.kind != crate::lex::TokenKind::Ident {
            self.error(span, "expected a member name");
            return Value::temporary(self.types.error());
        }
        self.bump();
        if self.types.is_error(base.ty) {
            return Value::temporary(self.types.error());
        }
        // `->` takes a pointer to an aggregate, and C99 6.3.2.1p3 decays an array to one before the
        // operator is applied: `struct P { int x; } p[1]; p->x` is legal, and every flexible-array
        // member in real C is written that way. The decay binds here rather than inside the test,
        // because the field's address is computed from `base` too. `cJSON.c` is what found this.
        let base = if arrow { self.decay(base) } else { base };
        let aggregate = if arrow {
            if !self.types.is_pointer(base.ty) {
                let ty = self.describe(base.ty);
                self.error(
                    span,
                    format!("`->` needs a pointer to a struct or union, not {ty}"),
                );
                return Value::temporary(self.types.error());
            }
            self.types.pointee(base.ty)
        } else {
            base.ty
        };
        let Some(record) = self.types.record_of(aggregate) else {
            let ty = self.describe(aggregate);
            self.error(span, format!("`{ty}` has no members"));
            return Value::temporary(self.types.error());
        };
        if !self.types.record_complete(record) {
            let ty = self.describe(aggregate);
            self.error(span, format!("`{ty}` is not complete yet"));
            return Value::temporary(self.types.error());
        }
        let Some(member) = self.types.member(record, token.text) else {
            let name = self.spell_of(token.text).to_owned();
            self.error(span, format!("`{name}` is not a member"));
            return Value::temporary(self.types.error());
        };
        let (member_ty, offset, bitfield) = (member.ty, member.offset, member.bits);
        let Some(base_address) = base.cir else {
            // No code: at file scope the base is an address in the data section - a symbol and an
            // offset - and a member is that address plus the member's own offset. `&points[1].y`
            // is how sqlite writes a table of pointers into a table of structs.
            if let Some((symbol, at)) = base.address {
                return Value::new(member_ty, None, base.lvalue || arrow)
                    .with_address(Some((symbol, at.wrapping_add(offset as i64))));
            }
            // The object has no address this compiler can name yet: the member is a place all the
            // same, and whatever uses it refuses in its own words rather than this saying the same
            // thing a second time.
            return Value::new(member_ty, None, base.lvalue || arrow);
        };
        let aggregate_ty = if arrow {
            self.types.pointee(base.ty)
        } else {
            base.ty
        };
        let Some(address) =
            self.address_within(base_address, aggregate_ty, offset as usize, member_ty, span)
        else {
            // No address this can emit - nothing is emitting, or it is dead code. The member is
            // still the member: its place and, for a bitfield, its bits are what a reader of the
            // type needs, and `&` refuses a bitfield on the strength of them alone (C99
            // 6.5.3.2p1), evaluated or not.
            return match bitfield {
                Some(bits) => Value {
                    ty: member_ty,
                    constant: None,
                    lvalue: true,
                    cir: None,
                    place: None,
                    bits: Some(bits),
                    address: None,
                },
                None => Value::new(member_ty, None, true),
            };
        };
        let value = match bitfield {
            None => self.read_place(member_ty, address, span),
            Some(bits) => self.read_bitfield(member_ty, address, bits, span),
        };
        // C99 6.5.2.3p3: a member of an *lvalue* is an lvalue, and a member of anything else is not.
        // `->` is always one - the pointer's target is an object - but `make().x` and
        // `(c ? a : b).x` are values, and writing through them (which this accepted until the
        // aggregate `?:` made it easy to hit) writes into an object the program never named.
        Value {
            lvalue: arrow || base.lvalue,
            ..value
        }
    }

    /// A bitfield read: the storage unit loaded as its own type, then the field shifted and
    /// masked out of it. The place stays the *unit's* address, because a write has to put the
    /// field back without disturbing its neighbours - which is why `bits` travels with the
    /// value.
    fn read_bitfield(
        &mut self,
        ty: TypeId,
        address: cir::Value,
        (start, width): (u8, u8),
        span: Span,
    ) -> Value {
        // The place and the bits are facts about the member, not about the code: they are what
        // tells `&` it is looking at a bitfield - which C99 6.5.3.2p1 forbids, evaluated or not -
        // and what a write needs to merge into the right bits. So every way out of this function
        // carries them, and only the value itself is optional.
        let field = |cir: Option<cir::Value>| Value {
            ty,
            constant: None,
            lvalue: true,
            cir,
            place: Some(address),
            bits: Some((start, width)),
            address: None,
        };
        let signed = matches!(self.types.kind(ty), Type::Int { signed: true, .. });
        // Arithmetic shifts for a signed field, logical ones for an unsigned field: the
        // working type's signedness is what decides which `Shr` the back end picks.
        let wide = self.types.integer_with(signed, 64);
        let Some(unit) = self.emit(|builder| builder.load(address, ty, span)) else {
            return field(None);
        };
        let Some(unit) = self.convert(Value::code(ty, unit), wide, span).cir else {
            return field(None);
        };
        let step = |parser: &mut Self, op: cir::BinOp, left: cir::Value, amount: i64| {
            let amount = parser.constant_of(wide, amount, span);
            let amount = amount.cir?;
            parser.emit(|builder| builder.binary(op, left, amount, wide, span))
        };
        let Some(extracted) = (if signed {
            // The sign bit is put at the top and shifted back down: the result is
            // sign-extended from the field's own width.
            step(self, cir::BinOp::Shl, unit, i64::from(64 - start - width))
                .and_then(|up| step(self, cir::BinOp::Shr, up, i64::from(64 - width)))
        } else {
            step(self, cir::BinOp::Shr, unit, i64::from(start))
                .and_then(|down| step(self, cir::BinOp::And, down, field_mask(width) as i64))
        }) else {
            return field(None);
        };
        field(self.convert(Value::code(wide, extracted), ty, span).cir)
    }

    pub(super) fn cast(&mut self, to: TypeId, span: Span) -> Value {
        if self.at_punct("{") {
            // `(struct P){1, 2}`: an unnamed object with the type the cast named. It is an
            // object of its own, so it gets a slot of its own and the walker fills it: the
            // initialiser being read - a local's slot, or a file-scope object's bytes - is
            // suspended while its leaves land in this one.
            let pointer = self.pointer_to(to);
            let slot = self.emit(|builder| builder.slot(to, pointer, span));
            let (bytes, offset, target) = (self.bytes.take(), self.offset, self.init_target.take());
            let literal = match slot {
                Some(slot) => {
                    self.init_target = Some((slot, to));
                    self.offset = 0;
                    self.brace_initializer(to);
                    // The literal's *value* is the object it filled: a load for a scalar, and the
                    // address for anything whose value is its address (ADR-004). Returning the
                    // slot's address as if it were the value typed the value `to` while its code
                    // was a `pointer to` - so `(int){5} + 1` added a pointer, and returning
                    // `(char){65}` from an `int` function converted a pointer.
                    self.read_place(to, slot, span)
                }
                // No body, or no room for one: the value is read for its type and dropped.
                None => {
                    self.brace_initializer(to);
                    Value::new(to, None, true)
                }
            };
            self.bytes = bytes;
            self.offset = offset;
            self.init_target = target;
            return literal;
        }
        // C99 6.3.2.1p3: an array operand of a cast is an expression like any other, so it decays
        // to a pointer first - `(char *)array` is a cast of the pointer, not of the array.
        let Some(from) = self.unary_expression().map(|value| self.decay(value)) else {
            return Value::temporary(to);
        };
        let converts = self.types.is_assignable(to, from.ty)
            || self.types.is_pointer(to) && self.types.is_integer(from.ty)
            || self.types.is_integer(to) && self.types.is_pointer(from.ty)
            || matches!(self.types.kind(to), Type::Void);
        if !converts {
            let (from, to) = (self.describe(from.ty), self.describe(to));
            self.error(span, format!("cannot convert {from} to {to}"));
            return Value::temporary(self.types.error());
        }
        if matches!(self.types.kind(to), Type::Void) {
            return Value::new(to, None, false);
        }
        // A cast is a conversion the C program asked for, so it emits whatever conversion C
        // says it is: returning `Value::new(to, ..)` here dropped the instruction and turned
        // `(int)x` into code that computes nothing.
        self.convert(from, to, span)
    }

    /// A number literal: an integer is folded and typed by its suffix and magnitude (C99
    /// 6.4.4.1), a floating constant is only typed - folding floats is the back end's job.
    pub(super) fn number_value(&mut self, spelling: &str, span: Span) -> Value {
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
            let ty = self.types.fundamental(name).unwrap();
            let Some((bits, value)) = crate::literal::float_value(spelling) else {
                // A hexadecimal constant is C99 and is not read here: refused rather than
                // read as the integer its digits happen to spell.
                self.error(
                    span,
                    format!("`{spelling}` is not a floating constant this compiler reads"),
                );
                return Value::temporary(self.types.error());
            };
            // The reader and the type table must agree about how wide the type is.
            debug_assert_eq!(
                Some(u64::from(bits)),
                self.types.size_of(ty).map(|size| size * 8)
            );
            return self.constant_of(ty, value as i64, span);
        }
        self.integer_value(spelling, span)
    }

    /// An integer literal, typed by its suffix and magnitude the way C99 6.4.4.1 says.
    pub(super) fn integer_value(&mut self, spelling: &str, span: Span) -> Value {
        let lower = spelling.to_ascii_lowercase();
        let unsigned = lower.contains('u');
        let longs = lower.matches('l').count();
        let digits = lower.trim_end_matches(['u', 'l']);
        // Decimal, octal (`0`) and hexadecimal (`0x`) are typed differently by C99 6.4.4.1p5.
        let decimal = !(digits.starts_with("0x") || digits.len() > 1 && digits.starts_with('0'));
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
        // C99 6.4.4.1p5: the first type the value fits, in the order the suffix and the radix
        // choose. A *decimal* constant has no unsigned type until `long long`: `4294967296` is
        // a `long long`, not an `unsigned int` that would drop its top half.
        let ty = self.integer_constant_type(bits, unsigned, longs, decimal);
        let Some(ty) = ty else {
            self.error(
                span,
                format!("`{spelling}` is too large for any integer type"),
            );
            return Value::temporary(self.types.error());
        };
        Value::constant(ty, bits as i64)
    }

    /// The type C99 6.4.4.1p5 gives an integer constant: the first of a list that can hold it,
    /// and the list depends on the suffix and on whether the constant is written in decimal
    /// (a decimal constant never takes an unsigned type before `long long`).
    fn integer_constant_type(
        &mut self,
        value: u64,
        unsigned: bool,
        longs: usize,
        decimal: bool,
    ) -> Option<TypeId> {
        // C99 6.4.4.1p5's table, written the way the standard writes it: **names**, not widths.
        // `int` and `long` are both thirty-two bits on this target and they are two types, which
        // `_Generic` asks about directly - a table keyed on `(signed, bits)` answered `int` for `1L`
        // and `unsigned int` for `1UL`, and did so before this table grew its second axis. The
        // radix is that axis: a decimal constant never takes an unsigned type, and a hexadecimal or
        // octal one does, so `0xfffffffffc000000LL` is an `unsigned long long` where the same digits
        // in decimal would be too large for any type.
        let candidates: &[&str] = match (unsigned, longs, decimal) {
            (false, 0, true) => &["int", "long", "long long"],
            (false, 0, false) => &[
                "int",
                "unsigned int",
                "long",
                "unsigned long",
                "long long",
                "unsigned long long",
            ],
            (true, 0, _) => &["unsigned int", "unsigned long", "unsigned long long"],
            (false, 1, true) => &["long", "long long"],
            (false, 1, false) => &["long", "unsigned long", "long long", "unsigned long long"],
            (true, 1, _) => &["unsigned long", "unsigned long long"],
            (false, _, true) => &["long long"],
            (false, _, false) => &["long long", "unsigned long long"],
            (true, _, _) => &["unsigned long long"],
        };
        // Whether the value fits the type, asked of the type itself: its width and its signedness
        // are already facts there, and writing them out again here is how `long` became `int`.
        let fits = |ty: TypeId| -> bool {
            match self.types.kind(ty) {
                crate::types::Type::Int { bits, signed } => {
                    let width = u32::from(*bits);
                    if width >= 64 {
                        // Every value an integer constant can have fits a 64-bit unsigned type, and
                        // the ceiling of the signed one is `i64::MAX`.
                        !*signed || value <= i64::MAX as u64
                    } else if *signed {
                        value < (1u64 << (width - 1))
                    } else {
                        value < (1u64 << width)
                    }
                }
                _ => false,
            }
        };
        candidates.iter().find_map(|name| {
            let ty = self.types.fundamental(name)?;
            fits(ty).then_some(ty)
        })
    }

    /// The value of a name: a variable, a function, or an enumeration constant.
    pub(super) fn named_value(&mut self, name: Symbol, span: Span) -> Value {
        // C99 6.4.2.2p1: `__func__` is the enclosing function's name, "as if" the body began with
        // `static const char __func__[] = "the name";`. It is not a macro and not a variable the
        // program declared, so it is answered before either lookup - and only inside a function,
        // which is the only place the standard gives it a meaning.
        if self.current_function.is_some() && self.spell_of(name) == "__func__" {
            return self.function_name_value(span);
        }
        if self.symbols.lookup(name).is_none() && self.at_punct("(") {
            // A call to a function nothing declared. C99 6.5.2.2p1 makes that a constraint
            // violation, and every compiler on this platform meets it in real code: MSVC calls it
            // C4013 and takes the callee to be a function returning `int`, GCC's `-std=gnu89` does
            // the same. So it is diagnosed - as a warning, because the translation goes on - and the
            // *call site's* arguments become the signature, which is exactly what this compiler
            // already does for a variadic call (ADR-006). No new lowering, and the object still
            // names the symbol the linker will find.
            let spelling = self.spell_of(name).to_owned();
            // `asm("nop")` looks like a call and is not one: it is a language extension this
            // compiler refuses, and taking it for an implicitly declared function would compile it
            // into a call to a symbol no linker has. Three spellings, because GCC and MSVC each
            // reserve their own.
            if matches!(spelling.as_str(), "asm" | "__asm" | "__asm__") {
                self.error(
                    span,
                    format!(
                        "`{spelling}` is not supported: this compiler generates no inline assembly"
                    ),
                );
                return Value::temporary(self.types.error());
            }
            // **The Microsoft intrinsics are declared by the compiler, because that is what MSVC
            // does.** A program calls `_umul128` with no header - MSVC's `<intrin.h>` is not needed
            // for the intrinsics the compiler knows - and taking one as an implicit `int`
            // *truncates*: `xxhash` lost the high half of a 64x64-to-128 product and produced a
            // different hash from clang's build, quietly. The table below is the answer, and it is
            // the same list `include/x86intrin.h` declares - one for a program that includes the
            // header, one for a program that does not.
            if let Some(function) = self.intrinsic(name) {
                let entry = Entry {
                    ty: function,
                    kind: Kind::Function,
                    span,
                    storage: None,
                    defined: false,
                };
                let _ = self.symbols.declare(name, entry);
            } else {
                if self.implicit.insert(name) {
                    self.warning(
                        span,
                        format!(
                            "`{spelling}` is not declared; it is taken to be a function returning `int`"
                        ),
                    );
                }
                let int = self
                    .types
                    .fundamental("int")
                    .unwrap_or_else(|| self.types.error());
                let function = self.types.function(int, Vec::new(), true);
                self.implicit_types.insert(function);
                let entry = Entry {
                    ty: function,
                    kind: Kind::Function,
                    span,
                    storage: None,
                    defined: false,
                };
                let _ = self.symbols.declare(name, entry);
            }
        }
        let Some(entry) = self.symbols.lookup(name) else {
            let spelling = self.spell_of(name).to_owned();
            self.error(span, format!("`{spelling}` is not declared"));
            return Value::temporary(self.types.error());
        };
        match entry.kind {
            Kind::EnumConst(value) => self.constant_of(entry.ty, value, span),
            Kind::Function => {
                // A function name is its address (C99 6.3.2.1p4): the callee of a call, and a
                // value that can be stored in a function pointer.
                let (function, name) = (entry.ty, name);
                let pointer = self.types.pointer(function);
                let link_name = self.link_name(name);
                match self.emit(|builder| builder.global_address(link_name, pointer, span)) {
                    Some(address) => Value {
                        ty: pointer,
                        constant: None,
                        lvalue: false,
                        bits: None,
                        address: None,
                        cir: Some(address),
                        place: Some(address),
                    },
                    // No code to hold the address - a file-scope initialiser, where it is a
                    // relocation. `static hooks = { my_malloc, my_free };` is how every C program
                    // with a vtable is written, and cJSON is one.
                    None => Value::new(pointer, None, false).with_address(Some((name, 0))),
                }
            }
            Kind::Typedef => {
                let spelling = self.spell_of(name).to_owned();
                self.error(span, format!("`{spelling}` is a type, not a value"));
                Value::temporary(self.types.error())
            }
            Kind::Variable => {
                let ty = entry.ty;
                match entry.storage {
                    // A local has a frame slot, and reading it is a load.
                    Some(place) => self.read_place(ty, place, span),
                    // A file-scope object has no slot: it is data, and its address is the symbol
                    // (ADR-004). Reading *or* writing it is the same place a local would be.
                    None => {
                        let pointer = self.pointer_to(ty);
                        let link_name = self.spell_of(name).to_owned();
                        match self.emit(|builder| builder.global_address(link_name, pointer, span))
                        {
                            Some(address) => self.read_place(ty, address, span),
                            // At file scope there is no code: the value *is* the symbol, which is what
                            // `int *p = &x;` and `char *s = str;` need to say.
                            None => Value::new(ty, None, true).with_address(Some((name, 0))),
                        }
                    }
                }
            }
        }
    }

    /// A type name inside parentheses, with the cursor left at the `(` when there is none -
    /// which is how `(int)x` is told from `(x)`, and `sizeof (int)` from `sizeof (x)`.
    pub(super) fn parenthesised_type(&mut self) -> Option<TypeId> {
        if !self.at_punct("(") {
            return None;
        }
        let before = self.at;
        self.bump();
        match self.try_type_name() {
            Some(ty) => {
                if !self.eat(")") {
                    self.error_at_current("expected `)` after the type");
                }
                Some(ty)
            }
            None => {
                self.at = before;
                None
            }
        }
    }

    /// Read a type name, or put the cursor back and say there was none.
    ///
    /// This is how `sizeof (` tells `sizeof (int)` from `sizeof (x)`: by trying. The
    /// alternative is a lookahead that lists the type keywords again, and a list that has to
    /// agree with `declaration_specifiers` is a list that will stop agreeing.
    pub(super) fn try_type_name(&mut self) -> Option<TypeId> {
        let before = self.at;
        let specs = self.declaration_specifiers();
        let Some(base) = specs.ty else {
            self.at = before;
            return None;
        };
        Some(match self.declarator(false) {
            Some(tree) => self.apply_declarator(tree, base),
            None => base,
        })
    }

    /// A function definition's body: the parameters live in its outermost block (C99 6.2.1p4),
    /// so the block is parsed without pushing a scope of its own.
    pub(super) fn function_body(&mut self, name: Option<Symbol>, result: TypeId, external: bool) {
        self.function_result = Some(result);
        self.current_function = name;
        self.loops = 0;
        self.switches.clear();
        self.labels.clear();
        self.label_blocks.clear();
        self.gotos.clear();
        self.targets.clear();
        self.blocks_reported.clear();
        let span = self.span();
        let params = std::mem::take(&mut self.last_parameters);
        self.captured_parameters = false;
        // Which slot each parameter of *this* function lives in, for `va_start(ap, last)`: it is
        // per function, and it was never cleared - so in a file with two variadic functions the
        // second one's `last` was found at twice its position, and `va_start` pointed past the
        // argument it meant (`outer(out, "ignored", 7)` read a slot of garbage).
        self.function_params.clear();
        let variadic = std::mem::take(&mut self.last_variadic);
        let Some(name) = name else {
            self.block_body();
            self.function_result = None;
            return;
        };
        // C99 6.7.5.3p4 and 6.9.1p3: in a definition every parameter and the result must have
        // a complete type. A declaration may name an incomplete one, so this is asked here.
        for (_, ty, param_span) in &params {
            if self.types.record_of(*ty).is_some() && !self.types.is_complete(*ty) {
                let ty = self.describe(*ty);
                self.error(*param_span, format!("a parameter of type {ty} has no size"));
            }
        }
        if self.types.record_of(result).is_some() && !self.types.is_complete(result) {
            let ty = self.describe(result);
            self.error(span, format!("a result of type {ty} has no size"));
        }
        // Win64: a record too big for a register comes back through a buffer the *caller*
        // owns, handed over as a leading pointer that the callee returns again. The body is
        // written against machine-shaped values, so the hidden pointer and the parameter
        // shapes are decided here, once.
        let buffer = (self.types.record_of(result).is_some() && !self.record_in_register(result))
            .then(|| self.pointer_to(result));
        let mut param_types: Vec<TypeId> = Vec::with_capacity(params.len() + 1);
        if let Some(buffer) = buffer {
            param_types.push(buffer);
        }
        for (_, ty, _) in &params {
            let shaped = if self.types.record_of(*ty).is_some() {
                self.abi_value(*ty)
            } else {
                *ty
            };
            param_types.push(shaped);
        }
        let shaped_result = self.abi_value(result);
        let mut func = cir::Func::new(name, shaped_result, span);
        func.variadic = variadic;
        func.external = external;
        let mut body = Body {
            current: func.entry,
            func,
        };
        body.builder().add_params(&param_types);
        let param_values: Vec<cir::Value> = body.func.params().to_vec();
        let (buffer, param_values) = match buffer {
            Some(_) => (param_values.first().copied(), param_values[1..].to_vec()),
            None => (None, param_values),
        };
        self.function_buffer = buffer;
        self.body = Some(body);

        let errors_before = self.error_count();
        self.symbols.push();
        for (position, (name, ty, span)) in params.into_iter().enumerate() {
            // One entry per parameter, in *source order*: `va_start(ap, last)` finds `last`'s
            // position here, and a parameter that takes no frame slot still takes a position - a
            // record passed by reference is the caller's copy, and a `void` has no slot at all.
            // Pushed before any branch can skip the parameter, and filled in where a slot is made;
            // without the placeholder every later parameter moved one place earlier and `va_start`
            // pointed at the argument *before* the one it meant.
            self.function_params.push(None);
            // C99 6.7.1p2: a parameter is an object, and `void` is not an object type.
            if matches!(self.types.kind(ty), Type::Void) {
                self.error(span, "a parameter cannot have type void");
                continue;
            }
            let incoming = param_values.get(position).copied();
            // A record passed by reference *is* the parameter: the caller made the copy, and
            // the callee may keep it, so the incoming pointer is the variable's storage.
            if self.types.record_of(ty).is_some() && !self.record_in_register(ty) {
                if let (Some(incoming), Some(name)) = (incoming, name) {
                    let _ = self.symbols.declare(
                        name,
                        Entry {
                            ty,
                            kind: Kind::Variable,
                            span,
                            storage: Some(incoming),
                            defined: false,
                        },
                    );
                }
                continue;
            }
            let pointer = self.pointer_to(ty);
            // C's parameter is a variable (C99 6.2.5p20): it gets a slot, and what arrives is
            // stored into it - as an integer, for a record small enough for a register.
            let slot = self.emit(|builder| builder.slot(ty, pointer, span));
            if let Some(entry) = self.function_params.last_mut() {
                *entry = slot;
            }
            if let Some(slot) = slot {
                if let Some(incoming) = incoming {
                    if self.types.record_of(ty).is_some() {
                        let integer = self.record_integer(ty);
                        if let Some(address) = self.address_within(slot, ty, 0, integer, span) {
                            self.emit(|builder| builder.store(address, incoming, span));
                        }
                    } else {
                        self.emit(|builder| builder.store(slot, incoming, span));
                    }
                }
                if let Some(name) = name {
                    let _ = self.symbols.declare(
                        name,
                        Entry {
                            ty,
                            kind: Kind::Variable,
                            span,
                            storage: Some(slot),
                            defined: false,
                        },
                    );
                }
            }
        }
        self.block_body();
        // A block nothing jumps to - the join after an `if` whose arms both return, say - still
        // has to end somehow, and `unreachable` is what it means.
        if let Some(body) = self.body.as_mut() {
            let current = body.current;
            body.func.end_orphans(current);
        }
        // A function that falls off its end returns, and C says what that return is.
        let terminated = self
            .body
            .as_ref()
            .is_none_or(|body| body.func.terminated(body.current));
        if !terminated {
            if matches!(self.types.kind(result), Type::Void) {
                self.emit(|builder| builder.ret(None, span));
            } else if !self
                .body
                .as_ref()
                .is_some_and(|body| body.func.reachable(body.current))
            {
                // Nothing can arrive here: the tail of a `for (;;)` with no `break`, or of a body
                // whose every path returned. It still needs a terminator, and saying so is all this
                // is - the first version of the diagnostic below fired on every one of the runtime's
                // CAS loops.
                self.emit(|builder| builder.unreachable(span));
            } else if self.spell_of(name) == "main" {
                // C99 5.1.2.2.3p1: reaching the `}` that ends `main` returns 0. That is not
                // undefined behaviour, so it is not the trap below - and it was: every program
                // whose `main` had no final `return` died of `ud2` at exit, `example/table.c` of
                // toml-c among them.
                self.emit(|builder| {
                    let zero = builder.constant(result, 0, span);
                    builder.ret(Some(zero), span)
                });
            } else {
                // C99 6.9.1p12: only `main` defines this. The program is legal C, and the *call's*
                // value is undefined if anyone reads it - so this is a warning, as it is in `clang`,
                // and the trap below turns the undefined case into a failure at the point it
                // happens rather than a number that looks like an answer.
                self.warning(
                    span,
                    format!(
                        "control reaches the end of `{}`, which returns {}: the value it returns is undefined",
                        self.spell_of(name),
                        self.describe(result)
                    ),
                );
                // Undefined behaviour, and the program is meant to stop here: a trap says so, and
                // `unreachable` would have said nothing can arrive - which is a different claim, and
                // one the verifier holds the first to.
                self.emit(|builder| builder.trap(span));
            }
        }
        let gotos = std::mem::take(&mut self.gotos);
        for (name, span) in gotos {
            if !self.labels.contains_key(&name) {
                let name = self.spell_of(name).to_owned();
                self.error(span, format!("this function has no label `{name}`"));
            }
        }
        self.symbols.pop();
        self.function_result = None;
        self.function_buffer = None;
        // The body is dropped, not lowered: the emitter does not express C yet. It is taken out
        // of the parser so the next function starts clean.
        let body = self.body.take().expect("this function's body");
        if self.error_count() == errors_before {
            // A body with a mistake in it is thrown away rather than lowered: half a function
            // is not a function, and the verifier is what says so.
            debug_assert_eq!(
                body.func.verify(&self.types),
                Ok(()),
                "an emitted function must verify"
            );
            self.funcs.push(body.func);
        }
    }

    /// How many errors have been reported: enough to tell whether a function body was read
    /// without one.
    ///
    /// A counter, not a count of the stored list: the list is capped by the diagnostic budget, and a
    /// question about the source must not be answered by a number the report's limit decides.
    pub(super) fn error_count(&self) -> usize {
        self.errors
    }
}
