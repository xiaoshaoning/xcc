//! Turning typed expressions into `cir`: the one seam between the parser and the intermediate
//! form.
//!
//! Part of the parser and nothing else: the state is `Parser`'s, which lives in `mod.rs`. The
//! file is separate because this is the one place that knows both C's conversion rules and
//! `cir`'s instructions, and having it next to the token helpers hid that.

use super::*;

impl Parser<'_> {
    /// Run `emit` on the block being filled, if a function body is being read. `None` means
    /// there is no body: a declaration at file scope, or an expression outside any function.
    pub(super) fn emit<T>(&mut self, emit: impl FnOnce(&mut cir::Builder<'_>) -> T) -> Option<T> {
        // An unevaluated operand's code would land in the middle of whatever asked for it - a
        // declarator, a `case` label, or the `sizeof` that does not evaluate it.
        if self.unevaluated {
            return None;
        }
        let body = self.body.as_mut()?;
        // Code after a `return` or a `break` is unreachable, which C allows: it emits nothing
        // rather than a second terminator.
        if body.func.terminated(body.current) {
            return None;
        }
        Some(emit(&mut body.builder()))
    }

    /// Whether code is being generated: false inside a declaration, true inside a body.
    pub(super) fn in_body(&self) -> bool {
        self.body.is_some()
    }

    /// Say that code is being built but this construct cannot be lowered yet, once, and where.
    /// A construct the emitter cannot express is refused rather than quietly left out: an
    /// expression that emits nothing is a program that computes something else.
    pub(super) fn not_lowered(&mut self, span: Span, what: &str) {
        // An unevaluated operand has no code to lower, so a construct that needs code is not a
        // construct this refused: `sizeof(p.a)` is a question about the type.
        if self.in_body() && !self.unevaluated {
            self.not_implemented(span, format!("{what} is not lowered yet (M4.2b)"));
        }
    }

    /// A value read from a place: a load, unless the type's value *is* its address, which is
    /// `value_is_address` - then the place is the value, typed `pointer to array[N]` or
    /// `pointer to function`, and `decay` bitcasts it to what the use expects (ADR-004).
    pub(super) fn read_place(&mut self, ty: TypeId, place: cir::Value, span: Span) -> Value {
        if self.types.value_is_address(ty) {
            return Value {
                ty,
                constant: None,
                lvalue: true,
                cir: Some(place),
                place: Some(place),
            };
        }
        match self.emit(|builder| builder.load(place, ty, span)) {
            Some(value) => Value {
                ty,
                constant: None,
                lvalue: true,
                cir: Some(value),
                place: Some(place),
            },
            None => Value::new(ty, None, true),
        }
    }

    /// The `cir` type of a value's type, for a conversion: how wide it is, and whether it is
    /// signed, decide which conversion C needs.
    pub(super) fn convert(&mut self, value: Value, wanted: TypeId, span: Span) -> Value {
        if self.types.same(value.ty, wanted) {
            return value;
        }
        if self.types.is_error(wanted) {
            return Value::new(wanted, value.constant, false);
        }
        // C99 6.3.2.3p3: the integer constant zero converts to a null pointer of any pointer
        // type, whichever integer type it was written as. It is a property of the value, so
        // it belongs here rather than in each caller that stores or returns one.
        if self.types.is_pointer(wanted) && value.constant == Some(0) {
            return self.constant_of(wanted, 0, span);
        }
        let Some(source) = value.cir else {
            return Value::new(wanted, None, false);
        };
        let Some(op) = self.conversion(value.ty, wanted) else {
            let (from, to) = (self.describe(value.ty), self.describe(wanted));
            self.error(
                span,
                format!("this compiler cannot convert {from} to {to} yet"),
            );
            return Value::new(self.types.error(), None, false);
        };
        let converted = self.emit(|builder| builder.convert(op, source, value.ty, wanted, span));
        match converted {
            Some(cir) => Value::code(wanted, cir),
            None => Value::new(wanted, None, false),
        }
    }

    /// An integer constant of type `ty`, with code when a body is being built.
    pub(super) fn constant_of(&mut self, ty: TypeId, value: i64, span: Span) -> Value {
        match self.emit(|builder| builder.constant(ty, value as u64, span)) {
            Some(cir) => Value::code(ty, cir).with_constant(Some(value)),
            None => Value::constant(ty, value),
        }
    }

    /// `pointer +/- integer`: the index is scaled by the pointee's size, and the arithmetic
    /// happens in the address's own width, because `cir` compares and computes like with like.
    pub(super) fn pointer_walk(
        &mut self,
        pointer_ty: TypeId,
        left: Value,
        right: Value,
        add: bool,
        span: Span,
    ) -> Value {
        let result = Value::new(pointer_ty, None, false);
        if !self.in_body() {
            return result;
        }
        let element = self.types.pointee(pointer_ty);
        let size = self.types.size_of(element).unwrap_or(1).max(1) as i64;
        let address = self.types.fundamental("unsigned long long").unwrap();
        // `p + n` scales `n`; `n + p` is the same expression with the sides swapped.
        let (pointer, index) = if self.types.is_pointer(left.ty) {
            (left, right)
        } else {
            (right, left)
        };
        let pointer = self.convert(pointer, address, span);
        let index = self.convert(index, address, span);
        let scale = self.constant_of(address, size, span);
        let (Some(pointer), Some(index), Some(scale)) = (pointer.cir, index.cir, scale.cir) else {
            return result;
        };
        let scaled = self.emit(|b| b.binary(cir::BinOp::Mul, index, scale, address, span));
        let Some(scaled) = scaled else {
            return result;
        };
        let op = if add {
            cir::BinOp::Add
        } else {
            cir::BinOp::Sub
        };
        let walked = self.emit(|b| b.binary(op, pointer, scaled, address, span));
        let Some(walked) = walked else {
            return result;
        };
        let back =
            self.emit(|b| b.convert(cir::ConvertOp::Bitcast, walked, address, pointer_ty, span));
        match back {
            Some(back) => Value::code(pointer_ty, back),
            None => result,
        }
    }

    /// `left op right`, with both operands converted to `operands` as C requires.
    pub(super) fn emit_binary(
        &mut self,
        op: cir::BinOp,
        operands: TypeId,
        result: TypeId,
        sides: [Value; 2],
        folded: Option<i64>,
        span: Span,
    ) -> Value {
        let [left, right] = sides;
        if !self.in_body() {
            return Value::new(result, folded, false);
        }
        let left = self.convert(left, operands, span);
        let right = self.convert(right, operands, span);
        match (left.cir, right.cir) {
            (Some(left), Some(right)) => {
                match self.emit(|b| b.binary(op, left, right, operands, span)) {
                    Some(value) => Value::code(result, value).with_constant(folded),
                    None => Value::new(result, folded, false),
                }
            }
            // An operand that emitted nothing already has a diagnostic of its own; a second
            // one here would just repeat it.
            _ => Value::new(result, folded, false),
        }
    }

    /// `op value` on a promoted operand.
    pub(super) fn emit_unary(
        &mut self,
        op: cir::UnOp,
        value: Value,
        ty: TypeId,
        folded: Option<i64>,
        span: Span,
    ) -> Value {
        if !self.in_body() {
            return Value::new(ty, folded, false);
        }
        let value = self.convert(value, ty, span);
        match value.cir {
            Some(value) => match self.emit(|b| b.unary(op, value, ty, span)) {
                Some(result) => Value::code(ty, result).with_constant(folded),
                None => Value::new(ty, folded, false),
            },
            None => Value::new(ty, folded, false),
        }
    }

    /// A comparison, whose result is an `int`: 1 or 0, as C has it.
    pub(super) fn emit_compare(
        &mut self,
        op: cir::CmpOp,
        operands: TypeId,
        sides: [Value; 2],
        result: TypeId,
        folded: Option<i64>,
        span: Span,
    ) -> Value {
        let [left, right] = sides;
        if !self.in_body() {
            return Value::new(result, folded, false);
        }
        let left = self.convert(left, operands, span);
        let right = self.convert(right, operands, span);
        match (left.cir, right.cir) {
            (Some(left), Some(right)) => {
                match self.emit(|b| b.compare(op, left, right, operands, result, span)) {
                    Some(value) => Value::code(result, value).with_constant(folded),
                    None => Value::new(result, folded, false),
                }
            }
            _ => Value::new(result, folded, false),
        }
    }

    /// Which conversion turns `from` into `to`, by C99 6.3.1.
    pub(super) fn conversion(&self, from: TypeId, to: TypeId) -> Option<cir::ConvertOp> {
        let width = |ty: TypeId| match self.types.kind(ty) {
            Type::Int { bits, .. } => Some(*bits),
            Type::Bool => Some(8),
            _ => None,
        };
        let signed = |ty: TypeId| matches!(self.types.kind(ty), Type::Int { signed: true, .. });
        if self.types.is_pointer(from) && self.types.is_pointer(to) {
            return Some(cir::ConvertOp::Bitcast);
        }
        // A pointer and an integer of the target's width are the same bits, so C's "a pointer
        // may be converted to an integer large enough to hold it" is a bitcast (6.3.2.3p6).
        let wide = |ty: TypeId| matches!(self.types.kind(ty), Type::Int { bits: 64, .. });
        if (self.types.is_pointer(from) && wide(to)) || (wide(from) && self.types.is_pointer(to)) {
            return Some(cir::ConvertOp::Bitcast);
        }
        if let (Some(from_bits), Some(to_bits)) = (width(from), width(to)) {
            return Some(if to_bits == from_bits {
                cir::ConvertOp::Bitcast
            } else if to_bits < from_bits {
                cir::ConvertOp::Truncate
            } else if signed(from) && !matches!(self.types.kind(from), Type::Bool) {
                cir::ConvertOp::SignExtend
            } else {
                cir::ConvertOp::ZeroExtend
            });
        }
        let from_float = self.types.is_float(from);
        let to_float = self.types.is_float(to);
        match (from_float, to_float) {
            (true, true) => Some(if self.types.size_of(to)? >= self.types.size_of(from)? {
                cir::ConvertOp::FloatExtend
            } else {
                cir::ConvertOp::FloatTruncate
            }),
            (false, true) => Some(cir::ConvertOp::IntToFloat),
            (true, false) => Some(cir::ConvertOp::FloatToInt),
            (false, false) => None,
        }
    }

    /// The type a pointer to `ty` has.
    pub(super) fn pointer_to(&mut self, ty: TypeId) -> TypeId {
        self.types.pointer(ty)
    }

    /// Whether a type has a machine representation yet: scalars, pointers and arrays of them do;
    /// aggregates wait until their layout is lowered.
    pub(super) fn lowerable(&self, ty: TypeId) -> bool {
        match self.types.kind(ty) {
            Type::Void | Type::Bool | Type::Int { .. } | Type::Float { .. } | Type::Pointer(_) => {
                true
            }
            Type::Array { element, .. } => self.lowerable(*element),
            _ => false,
        }
    }
}
