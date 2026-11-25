//! Dead code: an instruction whose result nothing reads, and which has no effect of its own.
//!
//! The pass exists because the two before it *make* dead code. Folding turns `sext`/`mul` chains into
//! constants, and the back end folds a `base + k` address into a memory operand's displacement - so the
//! addition and the bitcasts around it stop being read. Measured without this pass, that made the object
//! *larger* (118 848 -> 129 812 bytes: the folded constants were emitted and the arithmetic was still
//! there), which is the clearest argument for it there is.
//!
//! **What it keeps**: anything whose effect is not its result - a store, a call, a slot (the frame's
//! layout is a side effect of saying it exists). A load is removed when nothing reads it, because a
//! program that does not read memory does not have to have read it (C99 5.1.2.3, the as-if rule).

use std::collections::HashMap;

use super::super::Func;
use super::super::{Inst, Value};

/// Remove what nothing reads, and answer how many instructions went.
pub(crate) fn run(func: &mut Func) -> usize {
    let mut removed = 0;
    loop {
        let uses = use_counts(func);
        let mut this_round = 0;
        for block in &mut func.blocks {
            block.insts.retain(|inst| {
                let useful = match inst.result() {
                    Some(result) => uses.get(&result).copied().unwrap_or(0) > 0,
                    None => true,
                };
                if !useful && is_pure(inst) {
                    this_round += 1;
                    return false;
                }
                true
            });
        }
        removed += this_round;
        if this_round == 0 {
            return removed;
        }
    }
}

/// Whether an instruction's only contribution is the value it defines.
fn is_pure(inst: &Inst) -> bool {
    match inst {
        // A store writes memory and a call does something; a slot is what gives the frame its shape.
        Inst::Store { .. } | Inst::Slot { .. } | Inst::Call { .. } => false,
        // A compare-and-swap writes memory and a thread-local lookup *is* a call this back end emits
        // itself. The first is the reason `call_once` hung: `(void)__builtin_atomic_cas(...)` is how
        // a program says "swap and ignore what was there", and this pass was deleting the swap -
        // leaving the flag at "running" for ever, which every other thread then waits on.
        Inst::AtomicCas { .. } => false,
        _ => true,
    }
}

/// How many times each value is read, instructions and terminators together.
fn use_counts(func: &Func) -> HashMap<Value, usize> {
    let mut counts: HashMap<Value, usize> = HashMap::new();
    for block in &func.blocks {
        for inst in &block.insts {
            for operand in inst.operands() {
                *counts.entry(operand).or_default() += 1;
            }
        }
        if let Some(term) = &block.term {
            for operand in super::super::term_operands(term) {
                *counts.entry(operand).or_default() += 1;
            }
        }
    }
    counts
}
