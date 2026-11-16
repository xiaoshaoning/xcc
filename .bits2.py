def edit(path, pairs):
    s = open(path, encoding='utf-8', newline='').read()
    nl = '\r\n' if '\r\n' in s else '\n'
    for old, new in pairs:
        old = old.replace('\n', nl)
        new = new.replace('\n', nl)
        assert old in s, (path, old[:90])
        s = s.replace(old, new, 1)
    open(path, 'w', encoding='utf-8', newline='').write(s)


# --- reading a bitfield: load the unit, then shift and mask it out -------------------------
edit('crates/xcc/src/parse/expr.rs', [(
    "        let (member_ty, offset, bitfield) = (member.ty, member.offset, member.bits);\n"
    "        if bitfield.is_some() {\n"
    "            // A bitfield shares its storage unit with its neighbours: it has no address, and\n"
    "            // reading one is a shift the emitter does not do yet.\n"
    "            self.not_lowered(span, \"a bitfield in code\");\n"
    "            return Value::temporary(self.types.error());\n"
    "        }\n"
    "        let Some(base_address) = base.cir else {\n"
    "            // The object has no address this compiler cannot name yet (a global, as `named_value`\n"
    "            // says): the member is a place all the same, and whatever uses it refuses in its own\n"
    "            // words rather than this saying the same thing a second time.\n"
    "            return Value::new(member_ty, None, base.lvalue || arrow);\n"
    "        };",
    "        let (member_ty, offset, bitfield) = (member.ty, member.offset, member.bits);\n"
    "        let Some(base_address) = base.cir else {\n"
    "            // The object has no address this compiler can name yet (a global, as `named_value`\n"
    "            // says): the member is a place all the same, and whatever uses it refuses in its own\n"
    "            // words rather than this saying the same thing a second time.\n"
    "            return Value::new(member_ty, None, base.lvalue || arrow);\n"
    "        };",
), (
    "        let Some(address) =\n"
    "            self.address_within(base_address, aggregate_ty, offset as usize, member_ty, span)\n"
    "        else {\n"
    "            return Value::new(member_ty, None, true);\n"
    "        };\n"
    "        self.read_place(member_ty, address, span)\n"
    "    }",
    "        // The *unit* the field lives in, loaded as its own type.\n"
    "        let Some(address) =\n"
    "            self.address_within(base_address, aggregate_ty, offset as usize, member_ty, span)\n"
    "        else {\n"
    "            return Value::new(member_ty, None, true);\n"
    "        };\n"
    "        let Some(bits) = bitfield else {\n"
    "            return self.read_place(member_ty, address, span);\n"
    "        };\n"
    "        self.read_bitfield(member_ty, address, bits, span)\n"
    "    }\n"
    "\n"
    "    /// A bitfield read: the storage unit loaded as its own type, then the field shifted and\n"
    "    /// masked out of it. The place stays the *unit's* address, because a write has to put the\n"
    "    /// field back without disturbing its neighbours - which is why `bits` travels with the\n"
    "    /// value.\n"
    "    fn read_bitfield(\n"
    "        &mut self,\n"
    "        ty: TypeId,\n"
    "        address: cir::Value,\n"
    "        (start, width): (u8, u8),\n"
    "        span: Span,\n"
    "    ) -> Value {\n"
    "        let signed = matches!(self.types.kind(ty), Type::Int { signed: true, .. });\n"
    "        // Arithmetic for a signed field, logical for an unsigned one: the width of the working\n"
    "        // type is what makes `(64 - start - width)` shifts meaningful.\n"
    "        let wide = self.types.integer_with(signed, 64);\n"
    "        let unit = self.emit(|builder| builder.load(address, ty, span));\n"
    "        let unit = match unit {\n"
    "            Some(unit) => unit,\n"
    "            None => return Value::new(ty, None, true),\n"
    "        };\n"
    "        let unit = self.convert(Value::code(ty, unit), wide, span);\n"
    "        let Some(unit) = unit.cir else {\n"
    "            return Value::new(ty, None, true);\n"
    "        };\n"
    "        let extracted = if signed {\n"
    "            // Put the sign bit at the top and shift it back down, so the result is\n"
    "            // sign-extended from the field's own width.\n"
    "            let up = self.constant_of(wide, i64::from(64 - start - width), span);\n"
    "            let up = up.cir.map(|up| {\n"
    "                self.emit(|builder| builder.binary(cir::BinOp::Shl, unit, up, wide, span))\n"
    "            });\n"
    "            let Some(Some(up)) = up else {\n"
    "                return Value::new(ty, None, true);\n"
    "            };\n"
    "            let down = self.constant_of(wide, i64::from(64 - width), span);\n"
    "            let down = down.cir.map(|down| {\n"
    "                self.emit(|builder| builder.binary(cir::BinOp::Shr, up, down, wide, span))\n"
    "            });\n"
    "            let Some(Some(down)) = down else {\n"
    "                return Value::new(ty, None, true);\n"
    "            };\n"
    "            down\n"
    "        } else {\n"
    "            let down = self.constant_of(wide, i64::from(start), span);\n"
    "            let down = down.cir.map(|down| {\n"
    "                self.emit(|builder| builder.binary(cir::BinOp::Shr, unit, down, wide, span))\n"
    "            });\n"
    "            let Some(Some(down)) = down else {\n"
    "                return Value::new(ty, None, true);\n"
    "            };\n"
    "            let mask = self.constant_of(wide, i64::from(field_mask(width)), span);\n"
    "            let mask = mask.cir.map(|mask| {\n"
    "                self.emit(|builder| builder.binary(cir::BinOp::And, down, mask, wide, span))\n"
    "            });\n"
    "            let Some(Some(masked)) = mask else {\n"
    "                return Value::new(ty, None, true);\n"
    "            };\n"
    "            masked\n"
    "        };\n"
    "        let extracted = self.convert(Value::code(wide, extracted), ty, span);\n"
    "        Value {\n"
    "            ty,\n"
    "            constant: None,\n"
    "            lvalue: true,\n"
    "            cir: extracted.cir,\n"
    "            place: Some(address),\n"
    "            bits: Some((start, width)),\n"
    "        }\n"
    "    }",
)])

# --- writing a bitfield: read the unit, clear the field, or the new value in -----------------
edit('crates/xcc/src/parse/expr.rs', [(
    "        let converted = self.convert(right, left.ty, span);\n"
    "        match (left.place, converted.cir) {",
    "        let converted = self.convert(right, left.ty, span);\n"
    "        if let Some((start, width)) = left.bits {\n"
    "            return Some(self.write_bitfield(left, converted, (start, width), span));\n"
    "        }\n"
    "        match (left.place, converted.cir) {",
), (
    "    /// `condition ? when_true : when_false`: two blocks and a join, and the join's parameter is",
    "    /// A bitfield write: the unit is read, the field's bits are cleared and the new value put in\n"
    "    /// them. The value of the assignment is the field *after* the write, masked to its width, as\n"
    "    /// C says a bitfield's value is.\n"
    "    fn write_bitfield(\n"
    "        &mut self,\n"
    "        left: Value,\n"
    "        value: Value,\n"
    "        (start, width): (u8, u8),\n"
    "        span: Span,\n"
    "    ) -> Value {\n"
    "        let ty = left.ty;\n"
    "        let (Some(place), Some(value)) = (left.place, value.cir) else {\n"
    "            return Value::new(ty, None, false);\n"
    "        };\n"
    "        let mask = field_mask(width) as i64;\n"
    "        let keep = self.constant_of(ty, !(mask << start), span);\n"
    "        let clear = self.constant_of(ty, mask, span);\n"
    "        let shift = self.constant_of(ty, i64::from(start), span);\n"
    "        let unit = self.emit(|builder| builder.load(place, ty, span));\n"
    "        let (Some(unit), Some(keep), Some(clear), Some(shift)) =\n"
    "            (unit, keep.cir, clear.cir, shift.cir)\n"
    "        else {\n"
    "            return Value::new(ty, None, false);\n"
    "        };\n"
    "        let field = self.emit(|builder| builder.binary(cir::BinOp::And, value, clear, ty, span));\n"
    "        let Some(field) = field else {\n"
    "            return Value::new(ty, None, false);\n"
    "        };\n"
    "        let kept = self.emit(|builder| builder.binary(cir::BinOp::And, unit, keep, ty, span));\n"
    "        let Some(kept) = kept else {\n"
    "            return Value::new(ty, None, false);\n"
    "        };\n"
    "        let shifted = self.emit(|builder| builder.binary(cir::BinOp::Shl, field, shift, ty, span));\n"
    "        let Some(shifted) = shifted else {\n"
    "            return Value::new(ty, None, false);\n"
    "        };\n"
    "        let merged = self.emit(|builder| builder.binary(cir::BinOp::Or, kept, shifted, ty, span));\n"
    "        let Some(merged) = merged else {\n"
    "            return Value::new(ty, None, false);\n"
    "        };\n"
    "        self.emit(|builder| builder.store(place, merged, span));\n"
    "        Value::code(ty, field)\n"
    "    }\n"
    "\n"
    "    /// `condition ? when_true : when_false`: two blocks and a join, and the join's parameter is",
)])

# `++`/`--` on a bitfield would store the whole unit: refused where it is seen.
edit('crates/xcc/src/parse/expr.rs', [(
    "        let ty = value.ty;\n"
    "        let written = if self.types.is_pointer(ty) {",
    "        if value.bits.is_some() {\n"
    "            // The store `step` makes would write the field's value over its whole unit.\n"
    "            self.not_lowered(span, \"`++` on a bitfield\");\n"
    "            return Value::temporary(self.types.error());\n"
    "        }\n"
    "        let ty = value.ty;\n"
    "        let written = if self.types.is_pointer(ty) {",
), (
    "/// \"count\" is the wrong name for it, and the arithmetic is C's.",
    "/// \"count\" is the wrong name for it, and the arithmetic is C's.",
)])

# The mask a field of `width` bits has, in the widest working type.
edit('crates/xcc/src/parse/expr.rs', [(
    "/// Fold one binary operation on two constants.",
    "/// The mask a bitfield of `width` bits has, as a 64-bit pattern.\n"
    "fn field_mask(width: u8) -> u64 {\n"
    "    if width >= 64 {\n"
    "        u64::MAX\n"
    "    } else {\n"
    "        (1u64 << width) - 1\n"
    "    }\n"
    "}\n"
    "\n"
    "/// Fold one binary operation on two constants.",
)])
print("ok")
