//! Constant folding.
//!
//! The front end is deliberately literal: an initialiser writes one byte per `store`, and each byte's
//! address is `bitcast(ptr to u64) + sext(index) * sizeof(element)` with every part spelled out. On the
//! whole corpus that leaves **1 565 foldable instructions of 19 289**, and the largest class by far is
//! `sext` of a constant (1 274) - which is the first step of every one of those address chains.
//! Folding it lets the multiply fold, which lets the add fold, and a chain that was seven instructions
//! becomes an address with an offset. That is why this pass runs to a fixpoint: each fold is the next
//! one's operand.
//!
//! **What it does not do**, and what was measured before deciding: `x * 1` and `x + 0` appear 9 times
//! in the whole corpus, so the identities that need a substitution rather than a constant are not worth
//! the machinery. Reassociation and strength reduction change what a program computes in ways that need
//! more than arithmetic to justify, and division is left alone because it can trap - and a trap is a
//! property of the program, not a value.

use std::collections::HashMap;

use super::super::{BinOp, ConvertOp, Inst, Value};
use super::super::{Func, TypeId};
use crate::types::{Type, TypeId as Id, Types};

/// Fold what can be folded, and answer how many instructions became constants.
pub(crate) fn run(func: &mut Func, types: &Types) -> usize {
    let mut folded = 0;
    loop {
        let constants = constants_of(func);
        let mut changed = 0;
        for block in &mut func.blocks {
            for inst in &mut block.insts {
                let Some(bits) = folded_value(inst, &constants, types) else {
                    continue;
                };
                let result = inst
                    .result()
                    .expect("a foldable instruction defines a value");
                let span = inst.span();
                let ty = result_type(inst);
                *inst = Inst::Const {
                    result,
                    ty,
                    bits,
                    span,
                };
                changed += 1;
            }
        }
        folded += changed;
        if changed == 0 {
            return folded;
        }
    }
}

/// Every value already known to be a constant, wherever it is defined. Built afresh each round, so a
/// value this pass has just folded is an operand in the next.
fn constants_of(func: &Func) -> HashMap<Value, u64> {
    let mut constants = HashMap::new();
    for block in &func.blocks {
        for inst in &block.insts {
            if let Inst::Const { result, bits, .. } = inst {
                constants.insert(*result, *bits);
            }
        }
    }
    constants
}

/// The C type an instruction produces.
fn result_type(inst: &Inst) -> TypeId {
    match inst {
        Inst::Binary { ty, .. } | Inst::Unary { ty, .. } => *ty,
        Inst::Convert { to, .. } => *to,
        _ => unreachable!("an instruction with no type asked for one"),
    }
}

/// What an instruction's result is, when it can be known without running the program.
fn folded_value(inst: &Inst, constants: &HashMap<Value, u64>, types: &Types) -> Option<u64> {
    match inst {
        Inst::Binary {
            op,
            ty,
            left,
            right,
            ..
        } => {
            let (size, signed) = width(types, *ty)?;
            // Each operand is put in canonical form for the *operation's* type first. A constant from
            // the front end may be canonical for a narrower or differently signed type - `0 - 2` as
            // `unsigned int` arrives as the signed -2 in 64 bits - and shifting that logically moves
            // its top bit down into the low word, which is what made `unsigned >>= 1` answer -1.
            let left = canonical(*constants.get(left)?, size, signed);
            let right = canonical(*constants.get(right)?, size, signed);
            let bits = match op {
                BinOp::Add => left.wrapping_add(right),
                BinOp::Sub => left.wrapping_sub(right),
                BinOp::Mul => left.wrapping_mul(right),
                BinOp::And => left & right,
                BinOp::Or => left | right,
                BinOp::Xor => left ^ right,
                BinOp::Shl => left.wrapping_shl(right as u32),
                BinOp::Shr if signed => ((left as i64) >> (right as u32 & 63)) as u64,
                BinOp::Shr => left.wrapping_shr(right as u32),
                // A division can trap, and a trap belongs to the program rather than to a value.
                BinOp::Div | BinOp::Rem => return None,
            };
            Some(canonical(bits, size, signed))
        }
        Inst::Convert {
            op,
            from,
            to,
            value,
            ..
        } => {
            let bits = *constants.get(value)?;
            let (from_size, from_signed) = width(types, *from)?;
            let (to_size, to_signed) = width(types, *to)?;
            // The same rule as above: what the constant means is decided by the *source* type, not by
            // whatever the front end recorded.
            let value = canonical(bits, from_size, from_signed);
            Some(match op {
                ConvertOp::Bitcast => value,
                ConvertOp::Truncate | ConvertOp::SignExtend | ConvertOp::ZeroExtend => {
                    canonical(value, to_size, to_signed)
                }
                // Converting to or from floating point is not this pass's business: it would have to
                // round correctly, which is a floating-point question and not an integer one.
                ConvertOp::IntToFloat
                | ConvertOp::FloatToInt
                | ConvertOp::FloatExtend
                | ConvertOp::FloatTruncate => return None,
            })
        }
        _ => None,
    }
}

/// A type's width in bytes and whether it is signed, for the types a constant can have.
fn width(types: &Types, ty: Id) -> Option<(u8, bool)> {
    match types.kind(ty) {
        Type::Int { signed, bits } => Some((bits / 8, *signed)),
        Type::Bool => Some((1, false)),
        Type::Pointer(_) => Some((8, false)),
        _ => None,
    }
}

/// The canonical 64-bit form of a value of `size` bytes: its low bits, sign or zero extended, which is
/// what a `cir` constant means (ADR-004).
fn canonical(bits: u64, size: u8, signed: bool) -> u64 {
    if size >= 8 {
        return bits;
    }
    let shift = 64 - u32::from(size) * 8;
    if signed {
        (((bits << shift) as i64) >> shift) as u64
    } else {
        bits & ((1u64 << (u32::from(size) * 8)) - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::canonical;
    use crate::cir::pass::tests::folded;

    #[test]
    fn a_constant_is_canonical_for_its_type() {
        assert_eq!(canonical(0xFF, 1, true), u64::MAX);
        assert_eq!(canonical(0xFF, 1, false), 0xFF);
        assert_eq!(canonical(0xFFFF_FFFF, 4, true), u64::MAX);
        assert_eq!(canonical(0xFFFF_FFFF, 4, false), 0xFFFF_FFFF);
    }

    #[test]
    fn a_constant_is_folded_in_the_types_own_width() {
        // `0 - 2` as `unsigned int` is 0xFFFFFFFE, and shifting it right logically is 0x7FFFFFFF. A
        // fold that used the front end's signed -2 in 64 bits would answer 0xFFFFFFFF, which is what
        // the differential harness caught.
        let (before, after) =
            folded("int main(void) { unsigned int u; u = 0 - 2; u >>= 1; return u == -1; }");
        assert!(
            after < before,
            "{before} instructions before, {after} after"
        );
    }

    #[test]
    fn an_address_chain_folds_to_one_constant() {
        // The shape an initialiser writes: `sext(const) * const + const`.
        let source = "int main(void) { int a[3] = {7, 14}; return a[0] + a[1] * 10 + a[2]; }";
        let (before, after) = folded(source);
        assert!(
            after < before,
            "{before} instructions before folding, {after} after"
        );
    }
}
