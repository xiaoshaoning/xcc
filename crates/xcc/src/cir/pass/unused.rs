//! Functions nothing can reach.
//!
//! `static` is internal linkage (C99 6.1.2.2p7): no other translation unit can name the function,
//! so a name no operand in this module mentions is unreachable - not by a call and not by an address
//! in an initialiser. Removing it is therefore a fact, not a heuristic.
//!
//! It matters because of what an *inline* definition is. C99 6.7.4p6 gives it no external
//! definition, so a header's `FORCEINLINE` body becomes a private function in every unit that
//! includes the header, and an unused one is dead code. `winnt.h`'s `RtlFillVolatileMemory` calls
//! `RtlSetVolatileMemory`, which this machine's `ntdll` does not export: MSVC never emits either
//! body, and this compiler's linker refused to link a program that only returned zero because of a
//! call nothing made. The JIT refused the same program with the same reason, which is why the rule
//! lives here - one place, above both back ends - and not in the lowering of one of them.
//!
//! The reachability walk is what makes a *chain* die: scanning every body for names instead kept
//! each dead function alive in turn, because the last of them named the next.

use std::collections::{HashMap, HashSet, VecDeque};

use super::{Func, Module};
use crate::cir::Inst;
use crate::intern::Interner;

/// Remove every function that is not reachable from something another unit can see.
///
/// The roots are every name with external linkage - the only ones a linker can reach - and every
/// name an initialiser holds, since a table of function pointers outlives every call. Returns how
/// many functions went, for `Changes`.
pub fn remove(module: &mut Module, names: &Interner) -> usize {
    let mut uses: HashMap<String, Vec<String>> = HashMap::new();
    for function in &module.funcs {
        uses.insert(
            names.resolve(function.name).to_owned(),
            mentioned_by(function)
                .into_iter()
                .map(str::to_owned)
                .collect(),
        );
    }
    let mut reachable: HashSet<String> = HashSet::new();
    let mut pending: VecDeque<String> = VecDeque::new();
    for function in &module.funcs {
        if function.external {
            let name = names.resolve(function.name).to_owned();
            if reachable.insert(name.clone()) {
                pending.push_back(name);
            }
        }
    }
    for global in &module.globals {
        for relocation in &global.relocations {
            let name = relocation.symbol.clone();
            if reachable.insert(name.clone()) {
                pending.push_back(name);
            }
        }
    }
    while let Some(name) = pending.pop_front() {
        for used in uses.get(&name).map(Vec::as_slice).unwrap_or_default() {
            if reachable.insert(used.clone()) {
                pending.push_back(used.clone());
            }
        }
    }
    let before = module.funcs.len();
    module
        .funcs
        .retain(|function| function.external || reachable.contains(names.resolve(function.name)));
    before - module.funcs.len()
}

/// The names one function mentions. `GlobalAddress` is the only instruction that carries a link
/// name, and a call is a call through the value that instruction produced.
fn mentioned_by(function: &Func) -> Vec<&str> {
    let mut used = Vec::new();
    for block in &function.blocks {
        for instruction in &block.insts {
            if let Inst::GlobalAddress { name, .. } = instruction {
                used.push(name.as_str());
            }
        }
    }
    used
}
