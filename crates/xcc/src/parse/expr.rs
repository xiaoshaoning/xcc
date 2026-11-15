//! The parser's expressions: what a value is, and what C says it may be combined with.
//!
//! Part of the parser and nothing else: the state is `Parser`'s, which lives in `mod.rs`. This
//! file is the answer to "what does this operator do to these values", which is a question
//! large enough to read on its own.

use super::*;

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
        let null_pointer = self.types.is_pointer(left.ty) && right.constant == Some(0);
        if !null_pointer && !self.types.is_assignable(left.ty, right.ty) {
            let (to, from) = (self.describe(left.ty), self.describe(right.ty));
            self.error(span, format!("cannot assign {from} to {to}"));
            return Some(Value::temporary(left.ty));
        }
        let converted = if self.types.is_pointer(left.ty) && right.constant == Some(0) {
            // `p = 0` is the null pointer, whatever `int` the constant was written as.
            match self.emit(|builder| builder.constant(left.ty, 0, span)) {
                Some(cir) => Value::code(left.ty, cir).with_constant(Some(0)),
                None => Value::new(left.ty, Some(0), false),
            }
        } else {
            self.convert(right, left.ty, span)
        };
        match (left.place, converted.cir) {
            (Some(place), Some(value)) => {
                self.emit(|builder| builder.store(place, value, span));
                Some(Value::code(left.ty, value))
            }
            (None, _) if self.in_body() && !self.types.is_error(left.ty) => {
                // A place the emitter cannot name yet: a global, a member, an element.
                self.not_lowered(span, "this kind of assignment");
                Some(Value::temporary(left.ty))
            }
            _ => Some(Value::new(left.ty, converted.constant, false)),
        }
    }

    pub(super) fn conditional_expression(&mut self) -> Option<Value> {
        let condition = self.binary_expression(0)?;
        let question = self.span();
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
        let ty = self.ternary_type(when_true, when_false, question);
        let constant = match (when_true.constant, when_false.constant) {
            (Some(t), Some(f)) => condition
                .constant
                .map(|condition| if condition != 0 { t } else { f }),
            (Some(t), None) if condition.constant == Some(1) => Some(t),
            (None, Some(f)) if condition.constant == Some(0) => Some(f),
            _ => None,
        };
        Some(Value::new(ty, constant, false))
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
        if self.types.is_pointer(left.ty) && right.constant == Some(0) {
            return left.ty;
        }
        if self.types.is_pointer(right.ty) && left.constant == Some(0) {
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
                    let ty = common();
                    let op = if operator == "+" {
                        cir::BinOp::Add
                    } else {
                        cir::BinOp::Sub
                    };
                    let folded = folded(left, right);
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
                    let folded = folded(left, right);
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
                            self.promote(left.ty)
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
                        self.promote(left.ty)
                    } else {
                        ty
                    };
                    let folded = folded(left, right);
                    return self.emit_binary(op, operands, ty, [left, right], folded, span);
                }
                self.bad_operands(operator, left.ty, right.ty, span)
            }
            "<" | ">" | "<=" | ">=" | "==" | "!=" => {
                let comparable = (arithmetic
                    || self.types.is_pointer(left.ty) && self.types.is_pointer(right.ty))
                    || (self.types.is_pointer(left.ty) && right.constant == Some(0))
                    || (self.types.is_pointer(right.ty) && left.constant == Some(0));
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
                    let folded = folded(left, right);
                    return self.emit_compare(op, operands, [left, right], int, folded, span);
                }
                self.bad_operands(operator, left.ty, right.ty, span)
            }
            "&&" | "||" => {
                // Short-circuit needs a block per side, which lands with M4.2b's control flow;
                // until then an eagerly evaluated `&&` would read memory the program does not.
                self.not_lowered(span, "`&&` and `||` in code");
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
                Value::new(int, constant, false)
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

    /// Integer promotion (C99 6.3.1.1): anything narrower than `int` becomes `int`.
    pub(super) fn promote(&self, ty: TypeId) -> TypeId {
        match self.types.kind(ty) {
            Type::Int { bits, .. } if *bits < 32 => self.types.fundamental("int").unwrap(),
            Type::Bool => self.types.fundamental("int").unwrap(),
            _ => ty,
        }
    }

    /// An array or function used as a value is a pointer to it (C99 6.3.2.1). Everything that
    /// *uses* a value calls this; a place that assigns to one does not.
    pub(super) fn decay(&mut self, value: Value) -> Value {
        let ty = self.types.decay(value.ty);
        if ty == value.ty {
            return value;
        }
        Value::temporary(ty)
    }

    pub(super) fn require_scalar(&mut self, value: &Value, what: &str) {
        if self.types.is_error(value.ty) {
            return;
        }
        if !(self.types.is_arithmetic(value.ty) || self.types.is_pointer(value.ty)) {
            let ty = self.describe(value.ty);
            let span = self.span();
            self.error(span, format!("{what} must be a scalar, not {ty}"));
        }
    }

    pub(super) fn unary_expression(&mut self) -> Option<Value> {
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
                            let ty = self.describe(ty);
                            self.error(span, format!("unary `-` needs a number, not {ty}"));
                            return Some(Value::temporary(self.types.error()));
                        }
                        let ty = self.promote(ty);
                        let folded = value.constant.map(|value| value.wrapping_neg());
                        Some(self.emit_unary(cir::UnOp::Neg, value, ty, folded, span))
                    }
                    "+" => Some(Value::new(self.promote(ty), value.constant, false)),
                    "~" => {
                        if !self.types.is_integer(ty) {
                            let ty = self.describe(ty);
                            self.error(span, format!("`~` needs an integer, not {ty}"));
                            return Some(Value::temporary(self.types.error()));
                        }
                        let ty = self.promote(ty);
                        let folded = value.constant.map(|value| !value);
                        Some(self.emit_unary(cir::UnOp::Not, value, ty, folded, span))
                    }
                    _ => {
                        self.require_scalar(&value, "`!`");
                        let int = self.types.fundamental("int").unwrap();
                        let folded = value.constant.map(|value| i64::from(value == 0));
                        let zero = Value::new(value.ty, Some(0), false);
                        let zero = if self.in_body() {
                            match self.emit(|builder| builder.constant(value.ty, 0, span)) {
                                Some(cir) => Value::code(value.ty, cir),
                                None => zero,
                            }
                        } else {
                            zero
                        };
                        Some(self.emit_compare(
                            cir::CmpOp::Eq,
                            value.ty,
                            [value, zero],
                            int,
                            folded,
                            span,
                        ))
                    }
                }
            }
            "&" => {
                self.bump();
                let value = self.unary_expression()?;
                if self.types.is_error(value.ty) {
                    return Some(Value::temporary(self.types.error()));
                }
                let Some(place) = value.place else {
                    if value.lvalue {
                        self.not_lowered(span, "the address of this kind of place");
                    } else {
                        self.error(span, "`&` needs something it can take the address of");
                    }
                    return Some(Value::temporary(self.types.error()));
                };
                let pointer = self.types.pointer(value.ty);
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
                Some(Value::new(self.types.pointee(value.ty), None, true))
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

    pub(super) fn increment(&mut self, value: Value, span: Span) -> Value {
        if self.types.is_error(value.ty) {
            return Value::temporary(value.ty);
        }
        self.not_lowered(span, "`++` and `--`");
        let scalar = self.types.is_arithmetic(value.ty) || self.types.is_pointer(value.ty);
        if !value.lvalue || !scalar {
            self.error(span, "`++` and `--` need a variable holding a number");
            return Value::temporary(self.types.error());
        }
        Value::temporary(value.ty)
    }

    /// `sizeof` a type name or an expression, as a constant of type `size_t` (here `unsigned
    /// long long`, the widest integer this front end has).
    pub(super) fn sizeof_expression(&mut self) -> Value {
        let size_type = self.types.fundamental("unsigned long long").unwrap();
        if let Some(ty) = self.parenthesised_type() {
            if self.types.is_error(ty) {
                return Value::temporary(self.types.error());
            }
            return match self.types.size_of(ty) {
                Some(size) => Value::constant(size_type, size as i64),
                None => {
                    let typo = self.describe(ty);
                    let span = self.span();
                    self.error(span, format!("`sizeof` needs a complete type, not {typo}"));
                    Value::temporary(self.types.error())
                }
            };
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
                self.bump();
                value = self.increment(value, span);
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
                let mut value = Value::new(element, None, true);
                let address = self.pointer_walk(pointer, base, index, true, span);
                if let Some(place) = address.cir {
                    value.place = Some(place);
                    if let Some(loaded) = self.emit(|b| b.load(place, element, span)) {
                        value.cir = Some(loaded);
                    }
                }
                value
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
                    let null_pointer =
                        self.types.is_pointer(expected) && argument.constant == Some(0);
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
        self.emit_call(callee, &params, values, result, variadic, span)
    }

    /// The call itself: arguments converted to the parameter types, and the result value.
    fn emit_call(
        &mut self,
        callee: Value,
        params: &[TypeId],
        values: Vec<Value>,
        result: TypeId,
        variadic: bool,
        span: Span,
    ) -> Value {
        if !self.in_body() {
            return Value::new(result, None, false);
        }
        let Some(callee) = callee.cir else {
            return Value::new(result, None, false);
        };
        let mut args = Vec::new();
        for (position, argument) in values.into_iter().enumerate() {
            // A variadic argument is promoted, not converted: C99 6.5.2.2p6.
            let wanted = match params.get(position) {
                Some(wanted) => *wanted,
                None if variadic => self.promote(argument.ty),
                None => continue,
            };
            let converted = self.convert(argument, wanted, span);
            match converted.cir {
                Some(value) => args.push(value),
                None => return Value::new(result, None, false),
            }
        }
        let returns = if matches!(self.types.kind(result), Type::Void) {
            None
        } else {
            Some(result)
        };
        match self.emit(|builder| builder.call(callee, args, returns, span)) {
            Some(Some(value)) => Value::code(result, value),
            Some(None) => Value::new(result, None, false),
            None => Value::new(result, None, false),
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
                let int = self.types.fundamental("int").unwrap();
                // A constant this does not read is refused, not turned into zero.
                match character_constant(self.spell(&token)) {
                    Some(value) => Some(Value::constant(int, value)),
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
                let (ty, _, name) = self.string_literal();
                let pointer = self.types.pointer(ty);
                let mut value = Value::new(ty, None, true);
                if let Some(address) =
                    self.emit(|builder| builder.global_address(name, pointer, span))
                {
                    value.cir = Some(address);
                    value.place = Some(address);
                }
                Some(value)
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

    /// `a.b` and `p->b`: the member's type, and whether it can be assigned to.
    pub(super) fn member_access(&mut self, base: Value, arrow: bool, span: Span) -> Value {
        // The layout exists, but lowering a field needs address arithmetic on the aggregate,
        // which the emitter does not do yet.
        self.not_lowered(span, "a member access in code");
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
        match self.types.member(record, token.text) {
            Some(member) => Value::new(member.ty, None, base.lvalue || arrow),
            None => {
                let name = self.spell_of(token.text).to_owned();
                self.error(span, format!("`{name}` is not a member"));
                Value::temporary(self.types.error())
            }
        }
    }

    pub(super) fn cast(&mut self, to: TypeId, span: Span) -> Value {
        if self.at_punct("{") {
            // `(struct P){1, 2}`: an unnamed object with the type the cast named.
            self.brace_initializer(to);
            return Value::new(to, None, true);
        }
        let Some(from) = self.unary_expression() else {
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
        Value::new(to, from.constant, false)
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
            return Value::temporary(self.types.fundamental(name).unwrap());
        }
        self.integer_value(spelling, span)
    }

    /// An integer literal, typed by its suffix and magnitude the way C99 6.4.4.1 says.
    pub(super) fn integer_value(&mut self, spelling: &str, span: Span) -> Value {
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
    pub(super) fn named_value(&mut self, name: Symbol, span: Span) -> Value {
        let Some(entry) = self.symbols.lookup(name) else {
            let spelling = self.spell_of(name).to_owned();
            self.error(span, format!("`{spelling}` is not declared"));
            return Value::temporary(self.types.error());
        };
        let lowerable = self.lowerable(entry.ty);
        let _ = lowerable;
        match entry.kind {
            Kind::EnumConst(value) => Value::constant(entry.ty, value),
            Kind::Function => {
                // A function name is its address (C99 6.3.2.1p4): the callee of a call, and a
                // value that can be stored in a function pointer.
                let (function, name) = (entry.ty, name);
                let pointer = self.types.pointer(function);
                let link_name = self.spell_of(name).to_owned();
                match self.emit(|builder| builder.global_address(link_name, pointer, span)) {
                    Some(address) => Value {
                        ty: pointer,
                        constant: None,
                        lvalue: false,
                        cir: Some(address),
                        place: Some(address),
                    },
                    None => Value::new(pointer, None, false),
                }
            }
            Kind::Typedef => {
                let spelling = self.spell_of(name).to_owned();
                self.error(span, format!("`{spelling}` is a type, not a value"));
                Value::temporary(self.types.error())
            }
            Kind::Variable => match entry.storage {
                // A variable with a slot is a load from it; one without is a place the emitter
                // cannot name yet (a global, or a member), which its own statement will refuse.
                Some(slot) => {
                    let (ty, slot) = (entry.ty, slot);
                    self.load_variable(ty, slot, span)
                }
                None => Value::new(entry.ty, None, true),
            },
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
    pub(super) fn function_body(&mut self, name: Option<Symbol>, result: TypeId) {
        self.function_result = Some(result);
        self.loops = 0;
        self.switches.clear();
        self.labels.clear();
        self.gotos.clear();
        self.targets.clear();
        let span = self.span();
        let params = std::mem::take(&mut self.last_parameters);
        let Some(name) = name else {
            self.block_body();
            self.function_result = None;
            return;
        };
        let func = cir::Func::new(name, result, span);
        if !params.iter().all(|(_, ty, _)| self.lowerable(*ty)) || !self.lowerable(result) {
            self.not_lowered(span, "a function whose parameters or result are aggregates");
        }
        let param_types: Vec<TypeId> = params.iter().map(|(_, ty, _)| *ty).collect();
        let mut body = Body {
            current: func.entry,
            func,
        };
        body.builder().add_params(&param_types);
        let param_values: Vec<cir::Value> = body.func.params().to_vec();
        self.body = Some(body);

        let errors_before = self.error_count();
        self.symbols.push();
        for (position, (name, ty, span)) in params.into_iter().enumerate() {
            let pointer = self.pointer_to(ty);
            // C's parameter is a variable (C99 6.2.5p20): it gets a slot, and the value that
            // arrives is stored into it.
            if let Some(slot) = self.emit(|builder| builder.slot(ty, pointer, span)) {
                if let Some(value) = param_values.get(position) {
                    self.emit(|builder| builder.store(slot, *value, span));
                }
                if let Some(name) = name {
                    let _ = self.symbols.declare(
                        name,
                        Entry {
                            ty,
                            kind: Kind::Variable,
                            span,
                            storage: Some(slot),
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
            } else {
                self.emit(|builder| builder.unreachable(span));
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
    fn error_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.is_error())
            .count()
    }
}
