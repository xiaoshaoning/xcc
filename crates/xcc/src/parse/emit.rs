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

    /// Whether an initialiser is writing an object's bytes rather than emitting stores: true at file
    /// scope, where a `cir` value has nowhere to go and the bytes are the whole answer.
    pub(super) fn at_file_scope(&self) -> bool {
        self.init_target.is_none() && self.bytes.is_some()
    }

    /// Whether code is being generated: false inside a declaration, true inside a body.
    pub(super) fn in_body(&self) -> bool {
        self.body.is_some()
    }

    /// Say that code is being built but this construct cannot be lowered yet, once, and where.
    /// A construct the emitter cannot express is refused rather than quietly left out: an
    /// expression that emits nothing is a program that computes something else.
    ///
    /// No note: the one gap that carried a reason of its own was the varargs refusal, and a back
    /// end that refuses at the call now carries it (ADR-006). A reason a *host* should show apart
    /// from the message belongs on the diagnostic that the back end's own gap produces.
    pub(super) fn not_lowered(&mut self, span: Span, what: &str) {
        // An unevaluated operand has no code to lower, so a construct that needs code is not a
        // construct this refused: `sizeof(p.a)` is a question about the type.
        if !self.in_body() || self.unevaluated {
            return;
        }
        self.push(
            Diagnostic::error(format!("{what} is not lowered yet (M4.2b)")),
            span,
        );
    }

    /// A value read from a place: a load, unless the type's value *is* its address, which is
    /// `value_is_address` - then the place is the value, typed `pointer to array[N]`, `pointer to
    /// function` or `pointer to struct`, and `decay` bitcasts an array's to what a use expects
    /// (ADR-004). An aggregate is never loaded: `cir` has no register for one.
    pub(super) fn read_place(&mut self, ty: TypeId, place: cir::Value, span: Span) -> Value {
        if self.types.value_is_address(ty) {
            return Value {
                ty,
                constant: None,
                lvalue: true,
                cir: Some(place),
                place: Some(place),
                bits: None,
                address: None,
            };
        }
        match self.emit(|builder| builder.load(place, ty, span)) {
            Some(value) => Value {
                ty,
                constant: None,
                lvalue: true,
                cir: Some(value),
                place: Some(place),
                bits: None,
                address: None,
            },
            // No block to load in - dead code, or a construct that ended the block. The place is
            // still a place; what uses it finds no value rather than one that was never computed,
            // and `x = 1` after a `goto` is skipped instead of diagnosed.
            None => Value {
                ty,
                constant: None,
                lvalue: true,
                cir: None,
                place: Some(place),
                bits: None,
                address: None,
            },
        }
    }

    /// Whether a compile-time constant is C's *zero*, which is a question for the type arena:
    /// an integer 0, a null pointer, or a float whose value is 0.0. A value with no constant is
    /// not a known zero.
    pub(super) fn constant_is_zero(&self, value: &Value) -> bool {
        let Some(bits) = value.constant else {
            return false;
        };
        self.types.constant_is_zero(value.ty, bits)
    }

    /// Whether a value is a *null pointer constant*: an *integer* constant expression with the
    /// value zero (C99 6.3.2.3p3). A float `0.0` is zero and is not one, which is why this is
    /// not `constant_is_zero`.
    pub(super) fn null_pointer_constant(&self, value: &Value) -> bool {
        self.types.is_integer(value.ty) && value.constant == Some(0)
    }

    /// `value != 0` as a `_Bool`: the conversion C99 6.3.1.2 asks for, folded when the value is a
    /// constant so that a file-scope `_Bool` gets its byte.
    fn truth_value(&mut self, value: Value, span: Span) -> Value {
        let bool_ty = self
            .types
            .fundamental("_Bool")
            .unwrap_or_else(|| self.types.error());
        let ty = value.ty;
        let folded = value
            .constant
            .map(|_| i64::from(!self.constant_is_zero(&value)));
        let zero = self.constant_of(ty, 0, span);
        match (value.cir, zero.cir) {
            (Some(value), Some(zero)) => {
                match self
                    .emit(|builder| builder.compare(cir::CmpOp::Ne, value, zero, ty, bool_ty, span))
                {
                    Some(cir) => Value::code(bool_ty, cir).with_constant(folded),
                    None => Value::new(bool_ty, folded, false),
                }
            }
            _ => Value::new(bool_ty, folded, false),
        }
    }

    /// `value` compared with zero, as an `int` 0 or 1: `Ne` is what a condition branches on
    /// (C99 6.3.1.2) and what `&&`/`||` hand their join, `Eq` is `!value` (6.5.3.3). A float is
    /// *compared*, never converted: NaN is a true condition, and `fcvt_to_sint` of a NaN is
    /// undefined.
    pub(super) fn compare_to_zero(
        &mut self,
        value: Value,
        op: cir::CmpOp,
        span: Span,
    ) -> Option<cir::Value> {
        // C99 6.3.2.1p3 first: an array is a pointer wherever a value is wanted, and a boolean
        // context is one. Lua writes `api_check(L, (idx <= top) && "unacceptable index")`, whose
        // right operand *is* a string literal; without the decay this compared two arrays with each
        // other - an ill-typed `cmp` the verifier refused, and had it not, a branch on nonsense.
        let value = self.decay(value);
        if self.types.is_error(value.ty) {
            return None;
        }
        let ty = value.ty;
        let zero = self.constant_of(ty, 0, span);
        let int = self.types.fundamental("int").unwrap();
        match (value.cir, zero.cir) {
            (Some(value), Some(zero)) => {
                self.emit(|builder| builder.compare(op, value, zero, ty, int, span))
            }
            _ => None,
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
        // C99 6.3.2.3p3: the null pointer constant converts to a pointer of any type. It is a
        // property of the value, so it belongs here rather than in each caller that stores or
        // returns one.
        if self.types.is_pointer(wanted) && self.null_pointer_constant(&value) {
            return self.constant_of(wanted, 0, span);
        }
        // C99 6.3.1.2: any scalar converted to `_Bool` is 0 or 1, which is a *comparison*, not a
        // narrowing conversion: `(_Bool)2` is 1, `(_Bool)0.5` is 1, and so is any NaN.
        if matches!(self.types.kind(wanted), Type::Bool) && self.types.is_scalar(value.ty) {
            return self.truth_value(value, span);
        }
        let Some(source) = value.cir else {
            // Nothing is being emitted, so nothing will compute this - but a *constant expression* is
            // read in exactly this state, and C99 6.6p6 lets one hold a cast to an integer type. A
            // cast is the one operator that had a constant on each side and produced none, and
            // `char b[((int)(16 * sizeof(void *) * sizeof(double)))]` - how Lua sizes a buffer - is
            // the shape real code writes.
            let folded = value
                .constant
                .and_then(|constant| self.fold_conversion(value.ty, wanted, constant));
            return Value::new(wanted, folded, false);
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

    /// The constant an integer conversion of a constant produces, or `None` when the conversion is
    /// not one a constant expression may hold (C99 6.6p6 allows only arithmetic-to-integer casts).
    ///
    /// The work is the conversion's own, read from [`Self::conversion`] so that the fold and the
    /// instruction cannot disagree about what a `Truncate` means.
    pub(super) fn fold_conversion(&self, from: TypeId, to: TypeId, value: i64) -> Option<i64> {
        let width = |ty: TypeId| match self.types.kind(ty) {
            Type::Int { bits, .. } => Some(*bits),
            Type::Bool => Some(8),
            _ => None,
        };
        let signed = |ty: TypeId| matches!(self.types.kind(ty), Type::Int { signed: true, .. });
        let (from_bits, to_bits) = (width(from)?, width(to)?);
        // The low `bits` bits of `value`, and those bits read as a two's-complement number.
        let truncated = |value: i64, bits: u8| {
            if bits >= 64 {
                value
            } else {
                value & ((1i64 << bits) - 1)
            }
        };
        let extended = |value: i64, bits: u8| {
            if bits >= 64 {
                value
            } else {
                let shift = 64 - bits;
                (value << shift) >> shift
            }
        };
        Some(match self.conversion(from, to)? {
            cir::ConvertOp::Bitcast => value,
            cir::ConvertOp::Truncate => {
                let bits = truncated(value, to_bits);
                if signed(to) {
                    extended(bits, to_bits)
                } else {
                    bits
                }
            }
            cir::ConvertOp::SignExtend => extended(value, from_bits),
            cir::ConvertOp::ZeroExtend => truncated(value, from_bits),
            // A float on either side: narrowing `width` above, and a float's constant is its bit
            // pattern rather than its value - so there is nothing here to fold and 6.6p6 does not
            // ask for one (an integer constant expression holds no float but a cast's operand).
            _ => return None,
        })
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

    /// Whether a record of this type travels in a *register*: Win64 carries one of 1, 2, 4 or 8
    /// bytes as an integer that wide ("as if it had been an integer"), and any other record by
    /// reference to a copy the caller owns.
    pub(super) fn record_in_register(&self, ty: TypeId) -> bool {
        self.types.record_of(ty).is_some() && matches!(self.types.size_of(ty), Some(1 | 2 | 4 | 8))
    }

    /// How a C value of this type crosses a call boundary in `cir`: a register-carried record is
    /// an integer of its own width, a by-reference one is the address of the caller's copy, and
    /// everything else is already its own machine value.
    pub(super) fn abi_value(&mut self, ty: TypeId) -> TypeId {
        if self.types.record_of(ty).is_none() {
            return ty;
        }
        if !self.record_in_register(ty) {
            return self.pointer_to(ty);
        }
        let size = self.types.size_of(ty).unwrap_or(0);
        self.types.integer_with(false, (size * 8) as u8)
    }

    /// The integer type a register-carried record is loaded as and stored as: its own width.
    pub(super) fn record_integer(&mut self, ty: TypeId) -> TypeId {
        let bits = match self.types.size_of(ty).unwrap_or(1) {
            1 => 8,
            2 => 16,
            4 => 32,
            _ => 64,
        };
        self.types.integer_with(false, bits)
    }

    /// The address of the object `offset` bytes into the place `base`, typed `pointer to leaf`.
    /// `object` is the type the place is: a slot's address is `pointer to object`, so walking it
    /// by a byte count is `pointer_walk`'s job with `char` as the element - the scale is then
    /// one - and the last step is a bitcast to `pointer to leaf`. One place for "somewhere
    /// inside a thing": an initialiser's leaf, a member, and an aggregate copy all ask it.
    pub(super) fn address_within(
        &mut self,
        base: cir::Value,
        object: TypeId,
        offset: usize,
        leaf: TypeId,
        span: Span,
    ) -> Option<cir::Value> {
        let char = self.types.fundamental("char").unwrap();
        let int = self.types.fundamental("int").unwrap();
        let bytes = self.pointer_to(char);
        let pointer = self.pointer_to(leaf);
        let from = self.pointer_to(object);
        // Nothing to walk: a leaf at offset zero *is* the object, and its address is the object's
        // address once the type says so. Emitting a walk of zero bytes - a multiply, an add and
        // three bitcasts - was six instructions that meant "the same place", and mem2reg could not
        // see the slot through them (M10).
        if offset == 0 {
            if self.types.same(object, leaf) {
                return Some(base);
            }
            return self.emit(|builder| {
                builder.convert(cir::ConvertOp::Bitcast, base, from, pointer, span)
            });
        }
        let address = Value::new(from, None, false).with_cir(Some(base));
        let index = self.constant_of(int, offset as i64, span);
        let walked = self.pointer_walk(bytes, address, index, true, span);
        let address = walked.cir?;
        self.emit(|builder| builder.convert(cir::ConvertOp::Bitcast, address, bytes, pointer, span))
    }

    /// Put `value` into the object at `place`, whose type is `ty`: a store for a machine value,
    /// and a byte copy for a record, whose value is the address of another object rather than a
    /// register. The one place a value lands in an object, whichever kind it is.
    pub(super) fn store_at(&mut self, place: cir::Value, ty: TypeId, value: Value, span: Span) {
        if self.types.record_of(ty).is_some() {
            if let Some(source) = value.cir {
                self.copy_object(place, ty, source, value.ty, ty, span);
            }
            return;
        }
        if let Some(code) = value.cir {
            self.emit(|builder| builder.store(place, code, span));
        }
    }

    /// Copy an object of `ty` from `source` to `destination`: one byte at a time, because `cir`
    /// has no aggregate register and a byte load is always aligned. A union, a bitfield's unit
    /// and the padding in between come along without this needing to know the layout - which is
    /// the point: `Types` owns it. M10's optimiser is where a wider copy belongs.
    pub(super) fn copy_object(
        &mut self,
        destination: cir::Value,
        destination_ty: TypeId,
        source: cir::Value,
        source_ty: TypeId,
        copy_ty: TypeId,
        span: Span,
    ) {
        let Some(size) = self.types.size_of(copy_ty) else {
            return;
        };
        let char = self.types.fundamental("char").unwrap();
        for offset in 0..size as usize {
            let Some(from) = self.address_within(source, source_ty, offset, char, span) else {
                return;
            };
            let Some(to) = self.address_within(destination, destination_ty, offset, char, span)
            else {
                return;
            };
            let Some(byte) = self.emit(|builder| builder.load(from, char, span)) else {
                return;
            };
            self.emit(|builder| builder.store(to, byte, span));
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
        // A narrower integer to a pointer is implementation-defined (6.3.2.3p5), and the machine
        // has one answer: the value is widened into the register the way it would be widened into
        // a wider integer - sign-extended if it is signed, zero-extended if it is not, which is
        // what clang does with `(void *)-1`.
        if self.types.is_pointer(to) {
            if let Some(from_bits) = width(from) {
                if from_bits < 64 {
                    let signed = signed(from) && !matches!(self.types.kind(from), Type::Bool);
                    return Some(if signed {
                        cir::ConvertOp::SignExtend
                    } else {
                        cir::ConvertOp::ZeroExtend
                    });
                }
            }
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
