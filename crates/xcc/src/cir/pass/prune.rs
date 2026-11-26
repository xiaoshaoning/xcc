//! Blocks the entry cannot reach, and what is in them.
//!
//! C lets a program write statements no path leads to: the code after an `if` whose every arm
//! returned, the tail of a `for (;;)` with no `break`, the body of `if (0)`. The front end still
//! *reads* them - they are programs, and their mistakes are the compiler's to report - so `cir` has
//! blocks nothing arrives at.
//!
//! Nothing useful can be in one. `dce` removes an instruction whose result is unread, and a dead
//! block's instructions are read by dead blocks only, so a definition could go while a use in
//! another dead block stayed behind - which the verifier refuses ("used but no instruction defines
//! it") and which no pass has a reason to fix. So the first pass empties them: no instructions, one
//! terminator that says they are dead. The block *indices* stay, so nothing has to be renumbered.
//!
//! It runs before the others because it makes their questions well-posed: a use in dead code is no
//! value's business, and the back end does not have to encode instructions nothing executes.

use crate::diag::Span;

use super::super::{Func, Term};

/// Empty every block the entry cannot reach, and say how many there were.
pub fn run(func: &mut Func) -> usize {
    let live = func.reachable_blocks();
    let mut emptied = 0;
    for (index, block) in func.blocks.iter_mut().enumerate() {
        // A block that is nothing but a trap already says what this pass has to say.
        let dead = block.insts.is_empty() && matches!(block.term, Some(Term::Unreachable { .. }));
        if live[index] || dead {
            continue;
        }
        let span = block.term.as_ref().map_or(Span::at(0), |term| match term {
            Term::Return { span, .. }
            | Term::Jump { span, .. }
            | Term::Branch { span, .. }
            | Term::Unreachable { span }
            | Term::Trap { span } => *span,
        });
        block.insts.clear();
        block.term = Some(Term::Unreachable { span });
        emptied += 1;
    }
    emptied
}
