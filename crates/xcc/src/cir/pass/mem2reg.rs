//! `mem2reg`: the slots whose address never escapes become values.
//!
//! The front end is deliberately memory-shaped - every local is a stack slot, every read a load,
//! every write a store (ADR-002's invariant, and the reason the parser needs no SSA bookkeeping).
//! That is right for a compiler with one pass and one back end, and wrong for the code: a local
//! stored once and read five times costs six instructions where one would do, and Borland's back
//! end was already avoiding that in 1991.
//!
//! A slot is promotable when **its address never escapes**: every use of the address - or of any
//! bitcast of it, the addresses `cir` computes with - is a load's or a store's `address`, or the
//! next bitcast in the chain. A slot whose address is passed to a function, returned, or added to is
//! memory someone else can see, and it stays memory.
//!
//! Everything else is the classic construction:
//!
//! 1. **Where the values meet.** A slot stored in two blocks needs a block parameter where their
//!    paths join: the iterated dominance frontier of the store blocks names those blocks, and each
//!    gets a parameter of the slot's type.
//! 2. **What reaches where.** A dataflow fixpoint gives, for every block, the value the slot holds
//!    on entry - the block's parameter when it has one, else the value all its predecessors agree
//!    on. Walking a block's instructions with that gives each load what the store before it left,
//!    and each edge what the successor's parameter receives.
//! 3. **Replace and remove.** Each promoted load's result becomes its value (transitively - a load
//!    can be another slot's stored value), then the loads, the stores, the slots and the bitcasts
//!    that named them go.
//!
//! A slot stays memory when the proof does not hold: a type mismatch (a pun through a union is
//! memory's job), or a load with no value on some path (C calls that an indeterminate value, and
//! this pass does not guess).

use std::collections::{HashMap, HashSet};

use super::super::{Block, ConvertOp, Func, Inst, Term, Value};

/// One step of the rename walk: enter a block, or leave it and drop what it pushed.
enum Frame {
    Enter(Block),
    Leave(usize),
}

/// What a slot holds at some point: a value the code computed, or the parameter a block receives.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Current {
    Value(Value),
    Phi(Block),
}

/// Promote what this function allows. Answers how many stack slots were removed.
pub fn run(func: &mut Func, types: &crate::types::Types) -> usize {
    // A load nobody reads is not a read: the front end emits one to evaluate an assignment's
    // target, and treating it as a use vetoes every `int s; s = f();` in the corpus.
    let used = used_values(func);
    let chains = address_chains(func);
    let escaped = escaping_slots(func, &chains);
    let tree = func.dominator_tree();
    let successors = successors_of(func);

    let shape = Shape {
        func,
        types,
        chains: &chains,
        used: &used,
        tree: &tree,
        successors: &successors,
    };
    let mut plans: Vec<Plan> = Vec::new();
    for slot in slots_of(func) {
        if escaped.contains(&slot) {
            continue;
        }
        if let Some(plan) = plan_slot(&shape, slot) {
            if !plan.replace.is_empty() {
                plans.push(plan);
            }
        }
    }
    if plans.is_empty() {
        return 0;
    }
    apply(func, &chains, &plans)
}

/// One slot's promotion: the parameters it needs, what each load becomes, and what each edge feeds.
struct Plan {
    /// The type the slot holds - the pointee of its address.
    ty: crate::types::TypeId,
    /// The stack slot whose address this pass promotes.
    slot: Value,
    /// The block parameters it needs.
    phis: HashSet<Block>,
    /// What each load's result becomes.
    replace: HashMap<Value, Current>,
    /// What an edge feeds a parameter: `(predecessor, successor)` -> the value.
    incoming: HashMap<(Block, Block), Current>,
}

/// Plan one slot's promotion, or `None` when the proof does not hold.
/// What every slot's plan needs from the function: the address chains, the values in use, the
/// dominator tree and the shape of the graph. One borrow, so planning a slot takes two arguments
/// rather than eight.
struct Shape<'a> {
    func: &'a Func,
    types: &'a crate::types::Types,
    chains: &'a HashMap<Value, Value>,
    used: &'a HashSet<Value>,
    tree: &'a [Option<Block>],
    successors: &'a HashMap<Block, Vec<Block>>,
}

fn plan_slot(shape: &Shape<'_>, slot: Value) -> Option<Plan> {
    let Shape {
        func,
        types,
        chains,
        used,
        tree,
        successors,
    } = *shape;
    // The slot's value is a *pointer* to the object it holds: the type a load produces and a
    // store takes is what the pointee is.
    let ty = types.pointee(func.value(slot).ty);
    let mut stores: HashMap<Block, Value> = HashMap::new();
    let mut loads: Vec<(Block, Value, Value)> = Vec::new(); // block, result, address of the slot
    for (index, block) in func.blocks.iter().enumerate() {
        let here = Block(index as u32);
        for inst in &block.insts {
            match inst {
                Inst::Store { address, value, .. }
                    if chains.get(address).copied() == Some(slot) =>
                {
                    stores.insert(here, *value);
                }
                Inst::Load {
                    result, address, ..
                } if chains.get(address).copied() == Some(slot) => {
                    loads.push((here, *result, *address));
                }
                _ => {}
            }
        }
    }
    if stores.is_empty() || loads.is_empty() {
        return None;
    }
    // A load of another type than the slot is a pun (`u.i = x; return u.c;`), and memory is what
    // makes it work: nothing about SSA would keep it working.
    if loads
        .iter()
        .any(|(_, result, _)| func.value(*result).ty != ty)
    {
        return None;
    }

    // 1. Where the values meet: the iterated dominance frontier of the store blocks.
    let frontiers = dominance_frontiers(func, tree);
    let mut phis: HashSet<Block> = HashSet::new();
    let mut worklist: Vec<Block> = stores.keys().copied().collect();
    let mut queued: HashSet<Block> = worklist.iter().copied().collect();
    while let Some(block) = worklist.pop() {
        for meet in frontiers.get(&block).into_iter().flatten().copied() {
            if !phis.insert(meet) {
                continue;
            }
            if !queued.contains(&meet) {
                queued.insert(meet);
                worklist.push(meet);
            }
        }
    }

    // 2. The rename: walk the dominator tree, keeping a stack of what the slot holds. A block's
    //    parameter is pushed on entry, a store pushes its value, and a load takes the top - which
    //    is the value of the last store that dominates it, exactly what C says. A dataflow
    //    fixpoint over predecessors cannot answer this: a loop's back edge would have to know the
    //    loop's own value to compute it.
    let children = children_of(func, tree);
    let mut values: Vec<Current> = Vec::new();
    let mut replace: HashMap<Value, Current> = HashMap::new();
    let mut incoming: HashMap<(Block, Block), Current> = HashMap::new();
    let mut frames: Vec<Frame> = vec![Frame::Enter(func.entry)];
    while let Some(frame) = frames.pop() {
        match frame {
            Frame::Enter(block) => {
                let base = values.len();
                if phis.contains(&block) {
                    values.push(Current::Phi(block));
                }
                for inst in &func.blocks[block.0 as usize].insts {
                    match inst {
                        Inst::Load {
                            result, address, ..
                        } if chains.get(address).copied() == Some(slot) => {
                            if !used.contains(result) {
                                continue;
                            }
                            // A load with no value on this path is C's indeterminate value, and
                            // this pass does not guess: no plan, and the slot stays memory.
                            replace.insert(*result, values.last().copied()?);
                        }
                        Inst::Store { address, value, .. }
                            if chains.get(address).copied() == Some(slot) =>
                        {
                            values.push(Current::Value(*value));
                        }
                        _ => {}
                    }
                }
                for successor in successors.get(&block).into_iter().flatten().copied() {
                    if phis.contains(&successor) {
                        incoming.insert((block, successor), values.last().copied()?);
                    }
                }
                frames.push(Frame::Leave(base));
                for child in children.get(&block).into_iter().flatten().rev() {
                    frames.push(Frame::Enter(*child));
                }
            }
            Frame::Leave(base) => values.truncate(base),
        }
    }

    Some(Plan {
        ty,
        slot,
        phis,
        replace,
        incoming,
    })
}

/// The value every predecessor of `block` agrees the slot holds, or `None` when they disagree (a
/// join the frontier did not predict) or one of them never stores (a path with no value at all).
fn meet(
    func: &Func,
    block: Block,
    exit: &HashMap<Block, Option<Current>>,
) -> Result<Option<Current>, ()> {
    let predecessors = predecessors_of(func, block);
    if predecessors.is_empty() {
        // The entry block, or a block nothing jumps to: a value starts where a store put it, so
        // there is none on entry.
        return Ok(None);
    }
    let mut agreed: Option<Current> = None;
    for predecessor in predecessors {
        let Some(value) = exit.get(&predecessor).copied().flatten() else {
            // One path has no value, so the block has none: a load that needs one vetoes.
            return Ok(None);
        };
        match agreed {
            None => agreed = Some(value),
            Some(seen) if seen == value => {}
            Some(_) => return Err(()),
        }
    }
    Ok(agreed)
}

/// The dominance frontier of every block: where a value defined in it may meet another from a
/// different path.
fn dominance_frontiers(func: &Func, tree: &[Option<Block>]) -> HashMap<Block, HashSet<Block>> {
    let mut frontiers: HashMap<Block, HashSet<Block>> = HashMap::new();
    for (index, _) in func.blocks.iter().enumerate() {
        let block = Block(index as u32);
        let predecessors = predecessors_of(func, block);
        if predecessors.len() < 2 {
            continue;
        }
        let stop = tree[block.0 as usize];
        for predecessor in predecessors {
            let mut runner = Some(predecessor);
            while let Some(here) = runner {
                if Some(here) == stop {
                    break;
                }
                frontiers.entry(here).or_default().insert(block);
                runner = tree[here.0 as usize];
            }
        }
    }
    frontiers
}

/// The dominator tree's children, by parent.
fn children_of(func: &Func, tree: &[Option<Block>]) -> HashMap<Block, Vec<Block>> {
    let mut children: HashMap<Block, Vec<Block>> = HashMap::new();
    for (index, parent) in tree.iter().enumerate() {
        if let Some(parent) = parent {
            children
                .entry(*parent)
                .or_default()
                .push(Block(index as u32));
        }
    }
    let _ = func;
    children
}

/// The blocks that jump to `block`.
fn predecessors_of(func: &Func, block: Block) -> Vec<Block> {
    let mut found = Vec::new();
    for (index, other) in func.blocks.iter().enumerate() {
        let into = match &other.term {
            Some(Term::Jump { target, .. }) => *target == block,
            Some(Term::Branch {
                then, otherwise, ..
            }) => then.0 == block || otherwise.0 == block,
            _ => false,
        };
        if into {
            found.push(Block(index as u32));
        }
    }
    found
}

/// Where each block's terminator goes.
fn successors_of(func: &Func) -> HashMap<Block, Vec<Block>> {
    let mut successors = HashMap::new();
    for (index, block) in func.blocks.iter().enumerate() {
        let out = match &block.term {
            Some(Term::Jump { target, .. }) => vec![*target],
            Some(Term::Branch {
                then, otherwise, ..
            }) => vec![then.0, otherwise.0],
            _ => Vec::new(),
        };
        successors.insert(Block(index as u32), out);
    }
    successors
}

/// How many loads and stores a function has: what a caller measures a pass by.
pub fn count_memory(func: &Func) -> usize {
    func.blocks
        .iter()
        .flat_map(|block| block.insts.iter())
        .filter(|inst| matches!(inst, Inst::Load { .. } | Inst::Store { .. }))
        .count()
}

/// The stack slots of the function.
fn slots_of(func: &Func) -> Vec<Value> {
    func.blocks
        .iter()
        .flat_map(|block| block.insts.iter())
        .filter_map(|inst| match inst {
            Inst::Slot { result, .. } => Some(*result),
            _ => None,
        })
        .collect()
}

/// The values something reads: the set a load must be in to be a read at all.
fn used_values(func: &Func) -> HashSet<Value> {
    let mut used: HashSet<Value> = HashSet::new();
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

/// For every value that is a slot's address or a bitcast chain from one: the slot it names.
fn address_chains(func: &Func) -> HashMap<Value, Value> {
    let mut chains: HashMap<Value, Value> = HashMap::new();
    for block in &func.blocks {
        for inst in &block.insts {
            match inst {
                Inst::Slot { result, .. } => {
                    chains.insert(*result, *result);
                }
                Inst::Convert {
                    op: ConvertOp::Bitcast,
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
fn escaping_slots(func: &Func, chains: &HashMap<Value, Value>) -> HashSet<Value> {
    let mut escaped: HashSet<Value> = HashSet::new();
    for block in &func.blocks {
        for inst in &block.insts {
            // Which operands of what may name a slot's address: a load's address, a store's
            // address (its first operand), or the bitcast that continues the chain. A store's
            // *value* is not one of them - `int *p; p = &literal;` puts the address where someone
            // else can read it.
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
                if !fine {
                    escaped.insert(slot);
                }
            }
        }
    }
    escaped
}

/// Put the plans into the function: parameters on blocks, arguments on edges, values in place of
/// loads, and the memory gone.
fn apply(func: &mut Func, chains: &HashMap<Value, Value>, plans: &[Plan]) -> usize {
    // The parameters first, in a fixed order (slot, then block), so every predecessor appends its
    // arguments in the same order the parameters were added.
    let mut minted: HashMap<(usize, Block), Value> = HashMap::new();
    for (position, plan) in plans.iter().enumerate() {
        let ty = plan.ty;
        let mut blocks: Vec<Block> = plan.phis.iter().copied().collect();
        blocks.sort();
        for block in blocks {
            let value = func.mint(ty, block);
            func.blocks[block.0 as usize].params.push(value);
            minted.insert((position, block), value);
        }
    }
    // Then the arguments, one per parameter, on every edge that enters the block.
    for index in 0..func.blocks.len() {
        let here = Block(index as u32);
        let Some(term) = func.blocks[index].term.take() else {
            continue;
        };
        let term = match term {
            Term::Jump { target, args, span } => {
                let args = feed(plans, &minted, here, target, args);
                Term::Jump { target, args, span }
            }
            Term::Branch {
                condition,
                then,
                otherwise,
                span,
            } => {
                let then = (then.0, feed(plans, &minted, here, then.0, then.1));
                let otherwise = (
                    otherwise.0,
                    feed(plans, &minted, here, otherwise.0, otherwise.1),
                );
                Term::Branch {
                    condition,
                    then,
                    otherwise,
                    span,
                }
            }
            other => other,
        };
        func.blocks[index].term = Some(term);
    }
    // Then the values in place of the loads, resolved transitively, and the memory removed.
    let mut final_map: HashMap<Value, Value> = HashMap::new();
    for (position, plan) in plans.iter().enumerate() {
        for (load, value) in &plan.replace {
            let resolved = resolve(plans, &minted, *value, position);
            final_map.insert(*load, resolved);
        }
    }
    func.rewrite_values(&mut |value| final_map.get(&value).copied().unwrap_or(value));
    let promoted: HashSet<Value> = plans.iter().map(|plan| plan.slot).collect();
    for block in &mut func.blocks {
        block.insts.retain(|inst| match inst {
            // A load of a promoted slot goes whether its value was used (replaced) or not (dead):
            // the second kind is what the front end emits to evaluate an assignment's target, and
            // its address is about to stop existing.
            Inst::Load { address, .. } => chains
                .get(address)
                .is_none_or(|slot| !promoted.contains(slot)),
            Inst::Store { address, .. } => chains
                .get(address)
                .is_none_or(|slot| !promoted.contains(slot)),
            Inst::Slot { result, .. } => !promoted.contains(result),
            Inst::Convert { result, .. } => chains
                .get(result)
                .is_none_or(|slot| !promoted.contains(slot)),
            _ => true,
        });
    }
    plans.len()
}

/// The arguments an edge into `target` adds to a terminator's list, in parameter order.
fn feed(
    plans: &[Plan],
    minted: &HashMap<(usize, Block), Value>,
    predecessor: Block,
    target: Block,
    mut args: Vec<Value>,
) -> Vec<Value> {
    for (position, plan) in plans.iter().enumerate() {
        if !plan.phis.contains(&target) {
            continue;
        }
        let Some(current) = plan.incoming.get(&(predecessor, target)).copied() else {
            // The fixpoint promised every edge a value; a missing one leaves the argument out and
            // the verifier says so, which is the shape a compiler bug should take.
            continue;
        };
        args.push(resolve(plans, minted, current, position));
    }
    args
}

/// What a `Current` finally is: a computed value, with replaced loads followed to the end, or the
/// parameter of the block that owns it.
fn resolve(
    plans: &[Plan],
    minted: &HashMap<(usize, Block), Value>,
    current: Current,
    owner: usize,
) -> Value {
    match current {
        Current::Phi(block) => minted[&(owner, block)],
        Current::Value(mut value) => {
            let mut steps = 0;
            loop {
                // A value can be another slot's load result - `int y = x;` - and that load's
                // replacement belongs to *its* plan, including the parameter it may name.
                let Some(position) = plans
                    .iter()
                    .position(|plan| plan.replace.contains_key(&value))
                else {
                    return value;
                };
                match plans[position].replace[&value] {
                    Current::Phi(block) => return minted[&(position, block)],
                    Current::Value(next) => value = next,
                }
                steps += 1;
                if steps > 64 {
                    return value; // not a DAG: stop following rather than loop
                }
            }
        }
    }
}
