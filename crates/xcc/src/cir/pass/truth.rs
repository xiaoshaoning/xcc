//! A truth test of something that has already been tested.
//!
//! The front end is deliberately literal about conditions: `if (a < b)` asks for the comparison, then
//! asks whether its answer is non-zero, because that is what the branch tests. Measured over the corpus,
//! **530 of 1 177 comparisons - 45% - are that wrapper**: `cmp ne (cmp ...), 0`.
//!
//! It is not free. The wrapper hides the comparison from the branch that could have carried it, so the
//! comparison has to be materialised as a value (`cmp`, `setcc`, a widening to 64 bits) and then tested
//! again, where the fused form is `cmp` and a `jcc`. Removing it is a substitution - the wrapper's
//! readers want the comparison's answer, and the comparison already answers 0 or 1 - which is why this is
//! its own pass rather than a rule in `fold`: folding turns instructions into constants, and the module
//! doc of that pass says so.

use std::collections::HashMap;

use super::super::{Func, Inst, Value};

/// Remove truth tests of values that are already 0 or 1, and answer how many were removed.
pub(crate) fn run(func: &mut Func) -> usize {
    let substitution = truth_tests_of_truth_tests(func);
    if substitution.is_empty() {
        return 0;
    }
    func.rewrite_values(&mut |value| {
        // A chain is resolved before it is used, so one rewrite is enough: `((a < b) != 0) != 0` maps
        // through two steps here.
        substitution.get(&value).copied().unwrap_or(value)
    });
    let removed = substitution.len();
    for block in &mut func.blocks {
        block.insts.retain(|inst| {
            !inst
                .result()
                .is_some_and(|value| substitution.contains_key(&value))
        });
    }
    removed
}

/// For every comparison that tests another comparison's answer against zero, what its readers could
/// read instead.
///
/// Only `!= 0`. `(a < b) == 0` is the *negation* of the answer, which is a different value - the branch
/// that reads it can turn its own condition over, and one day that may be worth doing, but it is not this
/// substitution and pretending otherwise would be a wrong answer for a flipped branch.
fn truth_tests_of_truth_tests(func: &Func) -> HashMap<Value, Value> {
    let mut constants: HashMap<Value, u64> = HashMap::new();
    let mut comparisons: HashMap<Value, Value> = HashMap::new();
    for block in &func.blocks {
        for inst in &block.insts {
            match inst {
                Inst::Const { result, bits, .. } => {
                    constants.insert(*result, *bits);
                }
                // Any comparison answers 0 or 1, so its value *is* a truth value.
                Inst::Compare { result, .. } => {
                    comparisons.insert(*result, *result);
                }
                _ => {}
            }
        }
    }
    let mut substitution: HashMap<Value, Value> = HashMap::new();
    for block in &func.blocks {
        for inst in &block.insts {
            let Inst::Compare {
                result,
                op: crate::cir::CmpOp::Ne,
                left,
                right,
                ..
            } = inst
            else {
                continue;
            };
            if constants.get(right) != Some(&0) {
                continue;
            }
            let Some(inner) = comparisons.get(left).copied() else {
                continue;
            };
            substitution.insert(*result, inner);
        }
    }
    // Resolve chains so a single lookup answers the whole question.
    let inner = substitution.clone();
    for value in substitution.values_mut() {
        let mut current = *value;
        while let Some(next) = inner.get(&current) {
            if *next == current {
                break;
            }
            current = *next;
        }
        *value = current;
    }
    substitution
}

#[cfg(test)]
mod tests {
    use super::super::super::dump::dump;
    use crate::options::Options;
    use crate::pp::preprocess;
    use crate::source::SourceFile;

    /// The `cir` of a program with the passes run over it.
    fn optimised(source: &str) -> String {
        let file = SourceFile::new("t.c", source);
        let options = Options::default();
        let preprocessed = preprocess(&file, &options, &());
        let parsed = crate::parse::parse(&preprocessed, &options.budgets);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let mut module = parsed.module;
        super::super::optimise(&mut module)
            .unwrap_or_else(|(_, broken)| panic!("{}", broken.message));
        dump(&module, preprocessed.names())
    }

    #[test]
    fn a_condition_is_its_comparisons_answer() {
        let text = optimised("int f(int a, int b) { if (a < b) return 1; return 0; }");
        // One comparison, not two: the wrapper's `cmp ne ... , 0` is gone.
        assert_eq!(text.matches("= cmp ").count(), 1, "{text}");
    }

    #[test]
    fn a_condition_that_is_not_a_comparison_keeps_its_test() {
        // `if (p)` tests a pointer, which is not 0 or 1: the test has to stay.
        let text = optimised("int f(int *p) { if (p) return 1; return 0; }");
        assert_eq!(text.matches("= cmp ").count(), 1, "{text}");
    }

    #[test]
    fn a_negated_condition_is_not_the_same_answer() {
        // `!(a < b)` is the negation, so both comparisons must remain: substituting would answer
        // the question the wrong way round.
        let text = optimised("int f(int a, int b) { if (!(a < b)) return 1; return 0; }");
        assert_eq!(text.matches("= cmp ").count(), 2, "{text}");
    }
}
