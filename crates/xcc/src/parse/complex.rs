//! Complex numbers: the arithmetic C gives `_Complex` and a record does not have.
//!
//! The type itself is a record of two elements (`Types::complex`), which is what makes layout and
//! the Win64 ABI come out right without a rule of their own. What is left is everything a struct
//! is *not* allowed to do, and that is this file: conversions between a complex value and a real
//! one, the four operators of C99 6.5.5-6.5.7, the two comparisons of 6.5.9, unary minus, truth,
//! and the two parts by name.
//!
//! One decision holds it together: a complex value in an expression is a *temporary holding the
//! two parts*, because that is what the record is everywhere else - an argument, a return, a
//! member of a struct. There is no register form to keep in step, and the back end never learns
//! that the type means anything in particular.
//!
//! Part of the parser and nothing else: the state is `Parser`'s, which lives in `mod.rs`.

use super::*;

impl Parser<'_> {
    /// A complex value's two parts, as element values.
    ///
    /// A real operand of a complex operation is one of the answers here, with a zero imaginary
    /// part - C99 6.3.1.5p2 converts it that way, and doing it in one place is what keeps `z + 1`
    /// and `1 + z` from being two rules.
    pub(super) fn complex_parts(
        &mut self,
        value: Value,
        element: TypeId,
        span: Span,
    ) -> Option<(Value, Value)> {
        let Some(source) = self.types.complex_element(value.ty) else {
            let re = self.convert(value, element, span);
            let zero = self.constant_of(element, 0, span);
            return Some((re, zero));
        };
        let base = value.place.or(value.cir)?;
        let size = self.types.size_of(source)? as usize;
        let re = self.complex_part(base, value.ty, 0, source, span)?;
        let im = self.complex_part(base, value.ty, size, source, span)?;
        let re = if source == element {
            re
        } else {
            self.convert(re, element, span)
        };
        let im = if source == element {
            im
        } else {
            self.convert(im, element, span)
        };
        Some((re, im))
    }

    /// One part of a complex value, read from the object it lives in.
    fn complex_part(
        &mut self,
        base: cir::Value,
        object: TypeId,
        offset: usize,
        element: TypeId,
        span: Span,
    ) -> Option<Value> {
        let address = self.address_within(base, object, offset, element, span)?;
        Some(self.read_place(element, address, span))
    }

    /// A complex value built from its parts: the temporary that is its representation.
    pub(super) fn complex_from_parts(
        &mut self,
        element: TypeId,
        re: Value,
        im: Value,
        span: Span,
    ) -> Value {
        let ty = self.types.complex(element);
        if !self.in_body() {
            return Value::new(ty, None, false);
        }
        let pointer = self.pointer_to(ty);
        let Some(base) = self.emit(|builder| builder.slot(ty, pointer, span)) else {
            return Value::new(ty, None, false);
        };
        let size = self.types.size_of(element).unwrap_or(0) as usize;
        for (offset, part) in [(0, re), (size, im)] {
            let Some(address) = self.address_within(base, ty, offset, element, span) else {
                continue;
            };
            let part = self.convert(part, element, span);
            self.store_at(address, element, part, span);
        }
        let mut value = Value::new(ty, None, false);
        value.cir = Some(base);
        value.place = Some(base);
        value
    }

    /// `creal(z)`, `cimag(z)` and the `__real__`/`__imag__` spellings of them.
    pub(super) fn complex_real_or_imaginary(
        &mut self,
        value: Value,
        imaginary: bool,
        span: Span,
    ) -> Value {
        let Some(element) = self.types.complex_element(value.ty) else {
            // A real operand has no imaginary part and is its own real part (C99 6.13.1).
            return match imaginary {
                true => self.constant_of(value.ty, 0, span),
                false => value,
            };
        };
        let size = self.types.size_of(element).unwrap_or(0) as usize;
        let offset = if imaginary { size } else { 0 };
        let Some(base) = value.place.or(value.cir) else {
            return Value::new(element, None, false);
        };
        match self.complex_part(base, value.ty, offset, element, span) {
            Some(part) => part,
            None => Value::new(element, None, false),
        }
    }

    /// `!z`, and any context that asks whether a complex value is true (C99 6.3.1.2): a complex
    /// number is true when *either* part is, which is not the same question as `creal(z) != 0`.
    pub(super) fn complex_truth(&mut self, value: Value, span: Span) -> Option<Value> {
        let element = self.types.complex_element(value.ty)?;
        let (re, im) = self.complex_parts(value, element, span)?;
        let int = self.types.fundamental("int").unwrap();
        let zero = self.constant_of(element, 0, span);
        let left = self.emit_compare(cir::CmpOp::Ne, element, [re, zero], int, None, span);
        let zero = self.constant_of(element, 0, span);
        let right = self.emit_compare(cir::CmpOp::Ne, element, [im, zero], int, None, span);
        match (left.cir, right.cir) {
            (Some(left), Some(right)) => {
                let truth =
                    self.emit(|builder| builder.binary(cir::BinOp::Or, left, right, int, span))?;
                Some(Value::code(int, truth))
            }
            _ => None,
        }
    }

    /// `left <operator> right` where at least one side is complex, or `None` when neither is.
    ///
    /// The formulas are C99 6.5.5p5's, computed the way the standard writes them: no scaling for
    /// overflow, which is what `clang` does too and what makes the two agree bit for bit.
    pub(super) fn complex_binary(
        &mut self,
        operator: &str,
        left: Value,
        right: Value,
        span: Span,
    ) -> Option<Value> {
        let (left_element, right_element) = (
            self.types.complex_element(left.ty),
            self.types.complex_element(right.ty),
        );
        if left_element.is_none() && right_element.is_none() {
            return None;
        }
        // The element both sides are computed in: the usual arithmetic conversions of the two
        // elements, with a real operand standing for its own element.
        let real_of = |types: &Types, ty: TypeId| types.complex_element(ty).unwrap_or(ty);
        let element = self.types.usual_arithmetic(
            real_of(&self.types, left.ty),
            real_of(&self.types, right.ty),
        );
        match operator {
            "==" | "!=" => {
                let int = self.types.fundamental("int").unwrap();
                let (ar, ai) = self.complex_parts(left, element, span)?;
                let (br, bi) = self.complex_parts(right, element, span)?;
                let re = self.emit_compare(cir::CmpOp::Eq, element, [ar, br], int, None, span);
                let im = self.emit_compare(cir::CmpOp::Eq, element, [ai, bi], int, None, span);
                let different = operator == "!=";
                let combined = match (re.cir, im.cir) {
                    (Some(re), Some(im)) => {
                        let both = self
                            .emit(|builder| builder.binary(cir::BinOp::And, re, im, int, span))?;
                        // `==` is "both parts equal"; `!=` is its negation, which is *not* "both
                        // parts differ" - `(1,2) != (1,3)` is true. The negation is a comparison
                        // with zero and not `UnOp::Not`, which is *bitwise* `~`: it answered -1
                        // where C99 6.5.9p3 says a comparison's result is 0 or 1.
                        match different {
                            false => both,
                            true => {
                                let zero = self.constant_of(int, 0, span);
                                let both = Value::code(int, both);
                                self.emit_compare(
                                    cir::CmpOp::Eq,
                                    int,
                                    [both, zero],
                                    int,
                                    None,
                                    span,
                                )
                                .cir?
                            }
                        }
                    }
                    _ => return None,
                };
                Some(Value::code(int, combined))
            }
            "+" | "-" | "*" | "/" => {
                let (ar, ai) = self.complex_parts(left, element, span)?;
                let (br, bi) = self.complex_parts(right, element, span)?;
                fn op(
                    parser: &mut Parser<'_>,
                    op: cir::BinOp,
                    element: TypeId,
                    span: Span,
                    x: Value,
                    y: Value,
                ) -> Value {
                    parser.emit_binary(op, element, element, [x, y], None, span)
                }
                let part = match operator {
                    "+" => (
                        op(self, cir::BinOp::Add, element, span, ar, br),
                        op(self, cir::BinOp::Add, element, span, ai, bi),
                    ),
                    "-" => (
                        op(self, cir::BinOp::Sub, element, span, ar, br),
                        op(self, cir::BinOp::Sub, element, span, ai, bi),
                    ),
                    "*" => {
                        let ac = op(self, cir::BinOp::Mul, element, span, ar, br);
                        let bd = op(self, cir::BinOp::Mul, element, span, ai, bi);
                        let ad = op(self, cir::BinOp::Mul, element, span, ar, bi);
                        let bc = op(self, cir::BinOp::Mul, element, span, ai, br);
                        (
                            op(self, cir::BinOp::Sub, element, span, ac, bd),
                            op(self, cir::BinOp::Add, element, span, ad, bc),
                        )
                    }
                    // (a + bi) / (c + di), with the denominator as the standard writes it.
                    _ => {
                        let bb = op(self, cir::BinOp::Mul, element, span, br, br);
                        let dd = op(self, cir::BinOp::Mul, element, span, bi, bi);
                        let denominator = op(self, cir::BinOp::Add, element, span, bb, dd);
                        let ac = op(self, cir::BinOp::Mul, element, span, ar, br);
                        let bd = op(self, cir::BinOp::Mul, element, span, ai, bi);
                        let bc = op(self, cir::BinOp::Mul, element, span, ai, br);
                        let ad = op(self, cir::BinOp::Mul, element, span, ar, bi);
                        let re_numerator = op(self, cir::BinOp::Add, element, span, ac, bd);
                        let im_numerator = op(self, cir::BinOp::Sub, element, span, bc, ad);
                        let re = op(
                            self,
                            cir::BinOp::Div,
                            element,
                            span,
                            re_numerator,
                            denominator,
                        );
                        let im = op(
                            self,
                            cir::BinOp::Div,
                            element,
                            span,
                            im_numerator,
                            denominator,
                        );
                        (re, im)
                    }
                };
                Some(self.complex_from_parts(element, part.0, part.1, span))
            }
            "<" | ">" | "<=" | ">=" => {
                // C99 6.5.8p2: the relational operators need *real* operands.
                let ty = self.describe(if left_element.is_some() {
                    left.ty
                } else {
                    right.ty
                });
                self.error(
                    span,
                    format!("{ty} has no order: `<`, `>`, `<=` and `>=` need a real number"),
                );
                Some(Value::temporary(self.types.error()))
            }
            // `%`, the shifts and the bitwise operators need integers (C99 6.5.5p2, 6.5.7p2,
            // 6.5.10p2). Said here rather than left to the scalar rules, which have never seen a
            // value that is two numbers in a record.
            _ => {
                let ty = self.describe(if left_element.is_some() {
                    left.ty
                } else {
                    right.ty
                });
                self.error(
                    span,
                    format!("`{operator}` needs an integer, and {ty} is not one"),
                );
                Some(Value::temporary(self.types.error()))
            }
        }
    }

    /// `-z` and `+z`, which are the two unary operators a complex value accepts.
    pub(super) fn complex_unary(
        &mut self,
        operator: &str,
        value: Value,
        span: Span,
    ) -> Option<Value> {
        let element = self.types.complex_element(value.ty)?;
        let (re, im) = self.complex_parts(value, element, span)?;
        match operator {
            "-" => {
                let negated_re = self.emit_unary(cir::UnOp::Neg, re, element, None, span);
                let negated_im = self.emit_unary(cir::UnOp::Neg, im, element, None, span);
                Some(self.complex_from_parts(element, negated_re, negated_im, span))
            }
            "+" => Some(self.complex_from_parts(element, re, im, span)),
            _ => None,
        }
    }

    /// A conversion *out of* or *into* a complex type, or `None` when neither side is one.
    ///
    /// C99 6.3.1.7: to a real type the imaginary part is dropped; 6.3.1.5p2: to a complex type the
    /// real part is converted and the imaginary one is zero. `_Bool` is neither - 6.3.1.2 asks
    /// whether the value compares unequal to zero, which for a complex number is either part.
    pub(super) fn complex_convert(
        &mut self,
        value: Value,
        wanted: TypeId,
        span: Span,
    ) -> Option<Value> {
        let target = self.types.complex_element(wanted);
        let source = self.types.complex_element(value.ty);
        if target.is_none() && source.is_none() {
            return None;
        }
        if let Some(element) = target {
            let (re, im) = self.complex_parts(value, element, span)?;
            return Some(self.complex_from_parts(element, re, im, span));
        }
        source?;
        if matches!(self.types.kind(wanted), Type::Bool) {
            // The truth is an `int` (0 or 1); the *conversion* was asked for a `_Bool`, and a
            // value whose type is not the one it was converted to is a wrong answer to `_Generic`.
            let truth = self.complex_truth(value, span)?;
            return Some(self.convert(truth, wanted, span));
        }
        let real = self.complex_real_or_imaginary(value, false, span);
        Some(self.convert(real, wanted, span))
    }
}
