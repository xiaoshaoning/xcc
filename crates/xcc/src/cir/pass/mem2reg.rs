//! `mem2reg`: the slots whose address never escapes become values.
//!
//! The front end is deliberately memory-shaped - every local is a stack slot, every read a load,
//! every write a store (ADR-002's invariant, and the reason the parser needs no SSA bookkeeping).
//! That is right for a compiler with one pass and one back end, and wrong for the code: a local
//! stored once and read five times costs six instructions where one would do, and Borland's back
//! end was already avoiding that in 1991.
//!
//! This pass promotes the slots it can prove, and only those:
//!
//! - **the address never escapes**: every use of the slot's address - or of any bitcast of it, the
//!   addresses `cir` computes with - is a load's or a store's `address`, or the next bitcast in
//!   the chain. A slot whose address is passed to a function, returned, or added to is memory
//!   someone else can see, and it stays memory;
//! - **the slot has exactly one store**, whose block dominates every load - and precedes the load
//!   when both are in the same block.
//!
//! One store is the case that pays: parameters (stored into their slots at entry), `int x = ...;`
//! read afterwards, and a value a function computes once. The case it does *not* take - a local
//! assigned in a loop, which needs a block parameter at the join - is the next step, not a
//! different design: dominance frontiers, the classic algorithm. Writing the ceiling down keeps it
//! from being mistaken for the whole of mem2reg.

use std::collections::HashMap;

use super::super::{Block, Func, Inst, Value};

/// Promote what this function allows. Answers how many loads and stores went away.
pub fn run(func: &mut Func) -> usize {
    // A load nobody reads is not a read: the front end emits one to evaluate an assignment's
    // target, and treating it as a use vetoes every `int s; s = f();` in the corpus.
    let used = used_values(func);
    let chains = address_chains(func);
    let escapes = escaping_slots(func, &chains);
    let stores = single_stores(func, &chains, &escapes);
    if stores.is_empty() {
        return 0;
    }

    // Every load of a promoted slot must be reachable from its one store, in the store's own
    // type: a load before the store needs the memory the store filled, and a load of another
    // type is a pun (`u.i = x; return u.c;`) that memory is exactly what makes work. Either one
    // vetoes the slot, and the whole slot stays - a half-promoted slot leaves a use of an
    // address that no longer has an instruction to name it.
    let mut stores = stores;
    let mut vetoed: Vec<Value> = Vec::new();
    for (index, block) in func.blocks.iter().enumerate() {
        for (at, inst) in block.insts.iter().enumerate() {
            let Inst::Load {
                result, address, ..
            } = inst
            else {
                continue;
            };
            let Some(slot) = chains.get(address).copied() else {
                continue;
            };
            if !used.contains(result) {
                continue;
            }
            let Some((value, store_block, store_at)) = stores.get(&slot).copied() else {
                continue;
            };
            let here = Block(index as u32);
            let reaches = if store_block == here {
                store_at < at
            } else {
                func.dominates(store_block, here)
            };
            if !reaches || func.value(*result).ty != func.value(value).ty {
                vetoed.push(slot);
            }
        }
    }
    for slot in vetoed {
        stores.remove(&slot);
    }
    if stores.is_empty() {
        return 0;
    }

    // The second walk sees only slots that survived, so every load it matches is one the store
    // reaches in its own type.
    let mut replace: HashMap<Value, Value> = HashMap::new();
    for (index, block) in func.blocks.iter().enumerate() {
        for (at, inst) in block.insts.iter().enumerate() {
            let Inst::Load {
                result, address, ..
            } = inst
            else {
                continue;
            };
            let Some(slot) = chains.get(address).copied() else {
                continue;
            };
            if !used.contains(result) {
                continue;
            }
            let Some((value, store_block, store_at)) = stores.get(&slot).copied() else {
                continue;
            };
            let here = Block(index as u32);
            let reaches = if store_block == here {
                store_at < at
            } else {
                func.dominates(store_block, here)
            };
            if reaches {
                replace.insert(*result, value);
            }
        }
    }
    if replace.is_empty() {
        return 0;
    }

    // A replaced load can be the value another slot's store writes - `int x = 42; int y = x;` -
    // so the map has to be resolved to what each value finally becomes. Handing a use the load's
    // value would leave it naming an instruction this pass is about to remove.
    let resolve = |mut value: Value| {
        let mut steps = 0;
        while let Some(next) = replace.get(&value).copied() {
            value = next;
            steps += 1;
            if steps > 64 {
                break; // not a DAG: give up on the chain rather than loop
            }
        }
        value
    };
    let final_map: HashMap<Value, Value> =
        replace.keys().map(|load| (*load, resolve(*load))).collect();
    let before = count_memory(func);
    func.rewrite_values(&mut |value| final_map.get(&value).copied().unwrap_or(value));
    for block in &mut func.blocks {
        // A load whose result was replaced computes nothing anyone reads; a store to a promoted
        // slot writes memory nobody will read; the slot and the bitcasts that lead to it have no
        // address left to name. Everything else stays.
        block.insts.retain(|inst| match inst {
            Inst::Load { result, .. } => !replace.contains_key(result) && used.contains(result),
            Inst::Store { address, .. } => chains
                .get(address)
                .is_none_or(|slot| !stores.contains_key(slot)),
            Inst::Slot { result, .. } => !stores.contains_key(result),
            Inst::Convert { result, .. } => !chains
                .get(result)
                .is_some_and(|slot| stores.contains_key(slot)),
            _ => true,
        });
    }
    before - count_memory(func)
}

/// The values something reads: a load whose result is not here is dead.
fn used_values(func: &Func) -> Vec<Value> {
    let mut used: Vec<Value> = Vec::new();
    for block in &func.blocks {
        for inst in &block.insts {
            used.extend(inst.operands());
        }
        if let Some(term) = &block.term {
            used.extend(super::super::term_operands(term));
        }
    }
    used
}

/// How many loads and stores a function has: what a caller measures a pass by.
pub fn count_memory(func: &Func) -> usize {
    func.blocks
        .iter()
        .flat_map(|block| block.insts.iter())
        .filter(|inst| matches!(inst, Inst::Load { .. } | Inst::Store { .. }))
        .count()
}

/// For every value that is a slot's address or a bitcast chain from one: the slot it names.
///
/// `cir` has no "this is the slot" marker on a computed address, so the chain is what makes
/// `v1 = bitcast ptr-to-struct -> ptr-to-int v0` still *be* `v0`. Only bitcasts: an `add` moves the
/// address, and a slot whose address moves is memory with several names.
fn address_chains(func: &Func) -> HashMap<Value, Value> {
    let mut chains: HashMap<Value, Value> = HashMap::new();
    for block in &func.blocks {
        for inst in &block.insts {
            match inst {
                Inst::Slot { result, .. } => {
                    chains.insert(*result, *result);
                }
                Inst::Convert {
                    op: super::super::ConvertOp::Bitcast,
                    result,
                    value,
                    ..
                } => {
                    if let Some(slot) = chains.get(value).copied() {
                        chains.insert(*result, slot);
                    }
                }
                _ => {}
            }
        }
    }
    chains
}

/// The slots whose address is used for anything but a load, a store, or the next bitcast.
fn escaping_slots(func: &Func, chains: &HashMap<Value, Value>) -> Vec<Value> {
    let mut escaped: Vec<Value> = Vec::new();
    for block in &func.blocks {
        for inst in &block.insts {
            // Which operands of what may name a slot's address: a load's address, a store's
            // address (its first operand), or the bitcast that continues the chain. A store's
            // *value* is not one of them - `int *p; p = &literal;` puts the address where someone
            // else can read it, which is the definition of escaping.
            for (at, value) in inst.operands().iter().enumerate() {
                let Some(slot) = chains.get(value).copied() else {
                    continue;
                };
                let fine = match inst {
                    Inst::Load { .. } => true,
                    Inst::Store { .. } => at == 0,
                    Inst::Convert { result, .. } => chains.get(result).copied() == Some(slot),
                    _ => false,
                };
                if !fine && !escaped.contains(&slot) {
                    escaped.push(slot);
                }
            }
        }
    }
    escaped
}

/// The store that defines each promotable slot, with the block and index it sits at.
fn single_stores(
    func: &Func,
    chains: &HashMap<Value, Value>,
    escapes: &[Value],
) -> HashMap<Value, (Value, Block, usize)> {
    let mut stores: HashMap<Value, (Value, Block, usize)> = HashMap::new();
    let mut doubled: Vec<Value> = Vec::new();
    for (index, block) in func.blocks.iter().enumerate() {
        for (at, inst) in block.insts.iter().enumerate() {
            let Inst::Store { address, value, .. } = inst else {
                continue;
            };
            let Some(slot) = chains.get(address).copied() else {
                continue;
            };
            if escapes.contains(&slot) {
                continue;
            }
            if stores
                .insert(slot, (*value, Block(index as u32), at))
                .is_some()
            {
                doubled.push(slot);
            }
        }
    }
    for slot in doubled {
        // A second store is a join this pass cannot do yet: it needs a block parameter where the
        // two meet. The slot stays memory, which is what leaving it out of the map means.
        stores.remove(&slot);
    }
    stores
}
