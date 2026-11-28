//! The passes that run on `cir` between the front end and the back end.
//!
//! M10's first step, and the one the measuring stick says matters most: the front end writes every
//! local into a stack slot and reads it back, so a three-line function is a dozen loads and stores
//! of values that never needed memory. [`mem2reg`] promotes the slots whose address never escapes
//! into SSA values, which is fewer instructions for both lowerings and the traffic Borland's
//! register allocator was already avoiding in 1991.
//!
//! The passes are optional in the sense that a caller may skip them; a `Compile` runs them with
//! `cir::optimise`, and the verifier runs after each one in debug builds, so a pass that breaks a
//! function is caught where it broke rather than where it was used.

pub mod dce;
pub mod fold;
pub mod mem2reg;
pub mod prune;
pub mod truth;
pub mod unused;

use super::{Broken, Func, Module};
use crate::types::Types;

/// What the passes changed. The tests read it; nothing on the command line prints it, which this
/// comment claimed `--explain` did for as long as the field existed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Changes {
    /// Loads and stores removed because their slot became values.
    pub memory: usize,
    /// Stack slots removed because nothing was left in them.
    pub slots: usize,
    /// Instructions whose result became a constant.
    pub folded: usize,
    /// Instructions removed because nothing read their result.
    pub dead: usize,
    /// Functions removed because nothing could reach them.
    pub functions: usize,
    /// Truth tests removed because the value they tested was already 0 or 1.
    pub simplified: usize,
}

impl Changes {
    /// Whether any pass changed anything at all.
    pub fn is_empty(&self) -> bool {
        *self == Changes::default()
    }
}

/// The verifier, with the name of whoever should have been the last to touch the function: a pass
/// that breaks a function is a compiler bug, and "a pass broke it" without the pass is a search.
fn verified(func: &super::Func, types: &Types, by: &str) -> Result<(), super::Broken> {
    func.verify(types).map_err(|broken| super::Broken {
        message: format!("after {by}: {}", broken.message),
    })
}

/// Run every pass over every function, and check that each one left a function the verifier
/// accepts.
///
/// The check is here, not at the call sites, because a pass that breaks a function is a compiler
/// bug wherever it is run from - and a caller that forgot to look would get a wrong program
/// instead of a message. `Broken` names the function and the value, which is what makes a broken
/// pass debuggable at all.
pub fn optimise(
    module: &mut Module,
    names: &crate::intern::Interner,
) -> Result<Changes, (crate::intern::Symbol, Broken)> {
    // First, because it decides *which* functions the rest of this loop sees: a `static` nothing can
    // reach is dead C (C99 6.1.2.2p7), and dropping it here is the one place both back ends agree -
    // a header's unused `FORCEINLINE` body cannot fail a link and cannot make the JIT refuse a
    // program the linker accepts. See [`unused`].
    let mut changes = Changes {
        functions: unused::remove(module, names),
        ..Changes::default()
    };
    for func in &mut module.funcs {
        // A pass that breaks a function is a compiler bug, so each one is verified under its own
        // name: the message says "after `fold`: ..." rather than "a pass broke it", which was a
        // search through five passes every time. One `fail` closure, because every one of them
        // reports the same way afterwards.
        let name = func.name;
        let fail = |broken: Broken| (name, broken);
        // First, the cir the *front end* wrote. A function that is already ill-typed here has a
        // front-end bug, and verifying before the first pass is what tells the two layers apart:
        // without it, a `switch` on an `unsigned char` that compared itself with `int` case labels
        // was reported as "a pass broke `parse_number`", and the pass had done nothing wrong.
        verified(func, &module.types, "the front end").map_err(fail)?;
        // Dead first: `dce` would otherwise remove a definition in one dead block and leave its use
        // in another, which is a broken function by the verifier's lights and nobody's fault.
        checked(func, &module.types, "prune", prune::run).map_err(fail)?;
        let before = mem2reg::count_memory(func);
        changes.slots += checked(func, &module.types, "mem2reg", |func| {
            mem2reg::run(func, &module.types)
        })
        .map_err(fail)?;
        let after = mem2reg::count_memory(func);
        changes.memory += before.saturating_sub(after);
        // A condition's truth test goes first: it is the one pass that *removes* a comparison rather
        // than folding it, and what it leaves is what the branch can carry.
        changes.simplified += checked(func, &module.types, "truth", truth::run).map_err(fail)?;
        // Folding runs after `mem2reg`, because promotion is what turns a local's value into a
        // constant in the first place.
        changes.folded += checked(func, &module.types, "fold", |func| {
            fold::run(func, &module.types)
        })
        .map_err(fail)?;
        // Folding is what makes instructions dead, so this runs last and to a fixpoint of its own.
        changes.dead += checked(func, &module.types, "dce", dce::run).map_err(fail)?;
    }
    Ok(changes)
}

/// Run one pass, then verify what it left, with the pass's name for the message.
fn checked(
    func: &mut Func,
    types: &crate::types::Types,
    by: &str,
    run: impl FnOnce(&mut Func) -> usize,
) -> Result<usize, Broken> {
    let changed = run(func);
    verified(func, types, by)?;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::super::dump::dump;
    use super::*;
    use crate::options::Options;
    use crate::pp::preprocess;
    use crate::source::SourceFile;

    /// Count the instructions a program's `cir` holds, before and after the passes: what folding a
    /// chain of address arithmetic is worth, in one number.
    pub(crate) fn folded(source: &str) -> (usize, usize) {
        let file = SourceFile::new("t.c", source);
        let options = Options::default();
        let mut preprocessed = preprocess(&file, &options, &());
        let parsed = crate::parse::parse(&mut preprocessed, &options.budgets);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let mut module = parsed.module;
        // Counting instructions cannot see a fold - one instruction replaces another - so count the
        // ones that still do something: everything but a constant.
        let count = |module: &crate::cir::Module| {
            module
                .funcs
                .iter()
                .flat_map(|f| &f.blocks)
                .flat_map(|b| &b.insts)
                .filter(|inst| !matches!(inst, crate::cir::Inst::Const { .. }))
                .count()
        };
        let before = count(&module);
        let names = preprocessed.names().clone();
        optimise(&mut module, &names).unwrap_or_else(|(_, broken)| panic!("{}", broken.message));
        (before, count(&module))
    }

    /// Compile one function, run the passes, and give back the `cir` written down.
    fn promoted(source: &str) -> (String, Changes) {
        let file = SourceFile::new("t.c", source);
        let options = Options::default();
        let mut preprocessed = preprocess(&file, &options, &());
        let mut parsed = crate::parse::parse(&mut preprocessed, &options.budgets);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let changes = optimise(&mut parsed.module, preprocessed.names()).unwrap_or_else(
            |(function, broken)| {
                panic!(
                    "the passes broke `{}`: {}",
                    preprocessed.names().resolve(function),
                    broken.message
                )
            },
        );
        (dump(&parsed.module, preprocessed.names()), changes)
    }

    #[test]
    fn a_parameter_stored_into_a_slot_becomes_the_value_it_holds() {
        // The front end writes every parameter into a slot at entry and reads it back; with one
        // store that dominates every load, there is nothing memory was adding.
        let (text, changes) = promoted("int add(int a, int b) { return a + b; }");
        assert!(!text.contains("slot"), "{text}");
        assert!(!text.contains("load"), "{text}");
        assert!(!text.contains("store"), "{text}");
        assert!(text.contains("add int"), "{text}");
        // Two slots, one per parameter, each with a store and a load: four instructions.
        assert_eq!(changes.memory, 4, "{changes:?}");
        // Two slots, one per parameter: the field counts slots, and `memory` the loads and
        // stores that went with them.
        assert_eq!(changes.slots, 2, "{changes:?}");
    }

    #[test]
    fn a_local_written_once_and_read_afterwards_becomes_a_value() {
        let (text, _) = promoted("int f(int a, int b, int c) { int s; s = a * b + c; return s; }");
        assert!(!text.contains("slot"), "{text}");
        assert!(!text.contains("load"), "{text}");
        assert!(text.contains("mul int"), "{text}");
        assert!(text.contains("add int"), "{text}");
    }

    #[test]
    fn a_slot_whose_address_escapes_stays_memory() {
        // `&x` is visible to whoever gets it - `f` may write through it - so x's store and load
        // stay. (`q` is a different slot, and promoting it is what turns `f(q)` into `f(&x)`.)
        let (text, _) =
            promoted("int f(int *p); int g(void) { int x; x = 1; int *q = &x; return f(q) + x; }");
        assert!(text.contains("slot"), "{text}");
        assert!(text.contains("store"), "{text}");
        assert!(text.contains("load"), "{text}");
    }

    #[test]
    fn a_slot_written_in_a_loop_becomes_a_block_parameter() {
        // The case the first version of this pass left alone: two stores meet at the loop's
        // header, and the classic construction puts a parameter there. Nothing of the loop's
        // counter or accumulator is memory afterwards.
        let (text, _) = promoted(
            "int f(int n) { int i; int s = 0; for (i = 0; i < n; i = i + 1) { s = s + i; } return s; }",
        );
        assert!(!text.contains("slot"), "{text}");
        assert!(!text.contains("load"), "{text}");
        assert!(text.contains("add int"), "{text}");
    }

    #[test]
    fn a_union_punned_through_two_types_stays_memory() {
        // `u.i = x; return u.c;` is a load of a different type than the store: memory is the
        // mechanism, so it stays.
        let (text, _) = promoted(
            "union U { int i; char c; }; int f(void) { union U u; u.i = 0x41424344; return u.c; }",
        );
        assert!(text.contains("load"), "{text}");
    }
}
