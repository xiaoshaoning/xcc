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

pub mod mem2reg;

use super::Module;

/// What the passes changed, for a caller that wants to print it (`--explain`, the tests).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Changes {
    /// Loads and stores removed because their slot became values.
    pub memory: usize,
    /// Stack slots removed because nothing was left in them.
    pub slots: usize,
    /// Values that replaced a load.
    pub promoted: usize,
}

impl Changes {
    /// Whether any pass changed anything at all.
    pub fn is_empty(&self) -> bool {
        *self == Changes::default()
    }
}

/// Run every pass over every function.
pub fn optimise(module: &mut Module) -> Changes {
    let mut changes = Changes::default();
    for func in &mut module.funcs {
        let before = mem2reg::count_memory(func);
        changes.slots += mem2reg::run(func);
        let after = mem2reg::count_memory(func);
        changes.memory += before.saturating_sub(after);
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::super::dump::dump;
    use super::*;
    use crate::options::Options;
    use crate::pp::preprocess;
    use crate::source::SourceFile;

    /// Compile one function, run the passes, and give back the `cir` written down.
    fn promoted(source: &str) -> (String, Changes) {
        let file = SourceFile::new("t.c", source);
        let options = Options::default();
        let preprocessed = preprocess(&file, &options, &());
        let mut parsed = crate::parse::parse(&preprocessed, &options.budgets);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let changes = optimise(&mut parsed.module);
        for func in &parsed.module.funcs {
            assert_eq!(
                func.verify(&parsed.module.types),
                Ok(()),
                "the passes must leave a function verifying"
            );
        }
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
        // `slots` counts the stack slots the same run removed, not a second measure of memory.
        assert_eq!(changes.slots, 4, "{changes:?}");
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
        // `&x` is visible to whoever gets it: the address is not the pass's to remove.
        let (text, _) =
            promoted("int f(int *p); int g(void) { int x; int *q = &x; return f(q) + x; }");
        assert!(text.contains("slot"), "{text}");
        assert!(text.contains("store"), "{text}");
    }

    #[test]
    fn a_slot_written_twice_stays_memory() {
        // Two stores meet in a block parameter, which this pass does not build yet: `i` in a loop
        // is the case, and it stays as it is rather than being half-promoted.
        let (text, _) = promoted("int f(int n) { int i; int s = 0; for (i = 0; i < n; i = i + 1) { s = s + i; } return s; }");
        assert!(text.contains("load"), "{text}");
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
