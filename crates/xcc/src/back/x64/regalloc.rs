//! Linear-scan register allocation.
//!
//! The intervals come from ordinary liveness over the control-flow graph, and the scan is Poletto
//! and Sarkar's: keep the intervals that are live in an ordered list, expire the ones that ended,
//! and when no register is free spill the interval that runs longest, which is the one whose
//! register buys the most.
//!
//! **Only callee-saved registers are allocated** - `rbx`, `rsi`, `rdi`, `r12` to `r15`. A call then
//! cannot destroy a live value, so there is no need for the second pass a caller-saved set would
//! require (allocate again around every call, or move what crosses one to memory). The price is
//! seven registers instead of fifteen and a spill for the eighth value that is live at once; the
//! rest of the machine is scratch, and the calling convention owns it.
//!
//! A spilled value gets one frame slot and no register: the encoder loads it into a scratch
//! register for each use and stores it after each definition. Spilling a whole interval rather than
//! splitting it is the ceiling here - a split would keep the hot part in a register - and it is the
//! next thing to do when the code size this produces starts to matter.
//!
//! A value computed from another asks for that value's register, and gets it when it is free by the
//! time the computation happens: the copy that would otherwise be emitted never exists. That is the
//! whole of the coalescing here - no interference graph, no iteration - and most of the bytes it
//! removed were copies.

use std::collections::{BTreeSet, HashSet};

use super::{BlockTerm, MachineFunction, VReg};

/// A machine register, by the number x86-64 encoding gives it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct Reg(pub u8);

impl Reg {
    /// The address of the frame pointer, never allocated.
    pub(crate) const RSP: Reg = Reg(4);
    /// The accumulator, which returns values and holds the high half of a division.
    pub(crate) const RAX: Reg = Reg(0);
    /// The count register, which shifts read and the first argument arrives in.
    pub(crate) const RCX: Reg = Reg(1);
    /// The high half of a division, and the second argument.
    pub(crate) const RDX: Reg = Reg(2);
}

/// Where a value is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Location {
    /// In a general-purpose register for its whole life.
    Register(Reg),
    /// In an XMM register for its whole life.
    Float(Reg),
    /// In a frame slot at this offset from the frame body: the encoder moves it in and out.
    Spill(i32),
}

/// The XMM registers a floating-point value may live in, for a function that calls nothing. The three
/// below them are spoken for, and the reason is the invariant the encoder relies on - **an operation's
/// result, each of its operands and the scratch pair are pairwise distinct**, exactly as
/// `rax`/`r10`/`r11` are for integers: `xmm0`/`xmm1` are the scratch, `xmm2` is where a spilled result
/// is computed, and a pool that overlapped either would let an operation destroy a value it has still
/// to read.
pub(crate) const FLOAT_REGISTERS: [Reg; 3] = [Reg(3), Reg(4), Reg(5)];

/// The XMM registers a *calling* function may keep a value in: `xmm6` to `xmm15`, which Win64 says a
/// call preserves. `xmm0`-`xmm5` are caller-saved, so a `double` in one of them is destroyed by the
/// next call - a wrong answer with no diagnostic, and the reason this pool exists. The price is
/// sixteen bytes of frame and two `movups` per register used, exactly as the integer registers cost a
/// `mov` each.
pub(crate) const CALLEE_SAVED_FLOATS: [Reg; 10] = [
    Reg(6),
    Reg(7),
    Reg(8),
    Reg(9),
    Reg(10),
    Reg(11),
    Reg(12),
    Reg(13),
    Reg(14),
    Reg(15),
];

/// What a function that calls another may allocate: the callee-saved registers. A call cannot
/// destroy them, so a value in one is safe across the call for free; the price is a `mov` to save and
/// another to restore it, once each per function.
pub(crate) const CALLEE_SAVED: [Reg; 7] = [
    Reg(3),  // rbx
    Reg(6),  // rsi
    Reg(7),  // rdi
    Reg(12), // r12
    Reg(13), // r13
    Reg(14), // r14
    Reg(15), // r15
];

/// What a function that calls nothing may allocate, in the order it should prefer them: the
/// caller-saved registers that nothing in the ABI claims. `rax` is not here - a comparison writes its
/// low byte, a division its whole width, and the return value goes there - and neither are `rcx` and
/// `rdx`, because a shift's count is `rcx` and a division's high half is `rdx`.
///
/// The first two are safe in every such function; the last two are safe only when it contains no
/// shift and no division, and are then the pair the encoder uses as scratch instead.
pub(crate) const CALLER_SAVED: [Reg; 4] = [
    Reg(8),  // r8
    Reg(9),  // r9
    Reg(10), // r10
    Reg(11), // r11
];

/// What allocation decided for one function.
#[derive(Debug)]
pub(crate) struct Allocation {
    /// Where each virtual register lives, by its index. A value that nothing reads - the address of
    /// a function whose only use is the call that names it - has no interval, so its entry here is
    /// meaningless and must not be read.
    pub locations: Vec<Location>,
    /// The callee-saved general registers used, in the order the prologue saves them.
    pub saved: Vec<Reg>,
    /// The callee-saved XMM registers used, likewise: only these survive a call.
    pub saved_floats: Vec<Reg>,
    /// Bytes of spill slots, at the end of the frame.
    pub spill_bytes: i32,
}

/// A value's live range, in positions of the linearised instruction stream.
#[derive(Debug, Clone, Copy)]
struct Interval {
    reg: VReg,
    start: usize,
    end: usize,
    /// The value this one is computed from, if it is: an operation that writes a copy of an operand
    /// costs a `mov` unless the two get the same register, and the interval can ask for it here.
    hint: Option<VReg>,
}

/// Allocate registers for one function.
pub(crate) fn allocate(func: &MachineFunction) -> Allocation {
    let registers = func.registers();
    let mut positions = Positions::new(func);
    let intervals = positions.intervals(func);
    let mut locations = vec![Location::Register(Reg(0)); positions.value_count];
    let mut spills = 0;

    // Intervals by start, which is the order the scan needs; ties keep the earlier definition
    // first so a value is not expired before the value it defines has been given a register.
    let mut order: Vec<Interval> = intervals;
    order.sort_by_key(|interval| (interval.start, interval.reg));

    // Two register files, so two pools. A value's class decides which one it draws from, and an
    // interval only ever competes with the ones in its own file.
    let class_of = |value: VReg| {
        func.classes
            .get(value.0 as usize)
            .copied()
            .unwrap_or(super::Class::Integer)
    };
    let mut free_integer: Vec<Reg> = registers.integer;
    let mut free_float: Vec<Reg> = registers.float;
    let mut active: Vec<Interval> = Vec::new();
    let mut saved: Vec<Reg> = Vec::new();
    let mut saved_floats: Vec<Reg> = Vec::new();
    for interval in order {
        let float = class_of(interval.reg) == super::Class::Float;
        // Everything that ended before this one starts has finished with its register. The
        // register is free again, and it stays in `saved`: the prologue has to save every register
        // the function touches, not every one it holds at the end.
        let mut index = 0;
        while index < active.len() {
            // An interval ending at this position is read by this operation and not after it, so its
            // register is free for the value this operation writes: x86 reads an instruction's
            // operands before it writes the destination.
            if active[index].end <= interval.start {
                let other = active.remove(index);
                match locations[other.reg.0 as usize] {
                    Location::Register(register) => free_integer.push(register),
                    Location::Float(register) => free_float.push(register),
                    Location::Spill(_) => {}
                }
            } else {
                index += 1;
            }
        }
        free_integer.sort();
        free_float.sort();
        let free = if float {
            &mut free_float
        } else {
            &mut free_integer
        };
        // Coalescing: a value computed from another one takes the same register when it is free, so
        // the `mov` that would copy it is not emitted at all.
        if let Some(hint) = interval.hint {
            let wanted = self_register(&locations, hint);
            if let Some(register) = wanted.filter(|register| free.contains(register)) {
                free.retain(|other| *other != register);
                locations[interval.reg.0 as usize] = if float {
                    Location::Float(register)
                } else {
                    Location::Register(register)
                };
                // Every callee-saved register this function touches has to be preserved, whether it
                // calls anything or not. A leaf usually allocates none of them - its set is the
                // caller-saved one - but it does when it takes more than two arguments, and then the
                // saves are what keep its caller's values alive.
                if CALLEE_SAVED.contains(&register) && !saved.contains(&register) {
                    saved.push(register);
                }
                if float
                    && CALLEE_SAVED_FLOATS.contains(&register)
                    && !saved_floats.contains(&register)
                {
                    saved_floats.push(register);
                }
                active.push(interval);
                active.sort_by_key(|other| other.end);
                continue;
            }
        }
        if free.is_empty() {
            // Nothing is free: the interval that runs longest loses its register, because that
            // register then covers the most instructions. A tie spills the new one, which keeps the
            // scan's decision stable. Only an interval of the same class is a candidate: a float
            // cannot take a general register.
            let victim = active
                .iter()
                .enumerate()
                .filter(|(_, other)| (class_of(other.reg) == super::Class::Float) == float)
                .max_by_key(|(_, other)| other.end)
                .map(|(index, other)| (index, *other));
            match victim {
                Some((index, other)) if other.end > interval.end => {
                    let register = match locations[other.reg.0 as usize] {
                        Location::Register(register) | Location::Float(register) => register,
                        Location::Spill(_) => unreachable!("an active interval has a register"),
                    };
                    active.remove(index);
                    locations[other.reg.0 as usize] = Location::Spill(spills * 8);
                    spills += 1;
                    locations[interval.reg.0 as usize] = if float {
                        Location::Float(register)
                    } else {
                        Location::Register(register)
                    };
                    active.push(interval);
                    active.sort_by_key(|other| other.end);
                }
                _ => {
                    locations[interval.reg.0 as usize] = Location::Spill(spills * 8);
                    spills += 1;
                }
            }
        } else {
            let register = free.remove(0);
            locations[interval.reg.0 as usize] = if float {
                Location::Float(register)
            } else {
                Location::Register(register)
            };
            if CALLEE_SAVED.contains(&register) && !saved.contains(&register) {
                saved.push(register);
            }
            if float && CALLEE_SAVED_FLOATS.contains(&register) && !saved_floats.contains(&register)
            {
                saved_floats.push(register);
            }
            active.push(interval);
            active.sort_by_key(|other| other.end);
        }
    }
    // A returned value has to be in `rax` at the end, so a value that is returned and whose whole
    // life avoids the operations that write `rax` may as well live there: the copy at the return
    // then does not exist. `rax` is not one of the registers the scan hands out, so it is always
    // free for this.
    for value in &func.returned {
        let Some((start, end)) = positions
            .intervals_of
            .get(value.0 as usize)
            .copied()
            .flatten()
        else {
            continue;
        };
        if !matches!(locations[value.0 as usize], Location::Register(_)) {
            continue;
        }
        if positions.avoids_rax(func, *value, (start, end), &locations) {
            locations[value.0 as usize] = Location::Register(Reg::RAX);
        }
    }
    // A register that no value ended up in - because its owner moved to `rax` - must not be saved.
    saved.retain(|register| {
        locations
            .iter()
            .any(|location| matches!(location, Location::Register(other) if other == register))
    });
    // The prologue saves in a fixed order, so two runs of the compiler save in the same order.
    saved.sort();
    saved_floats.sort();
    Allocation {
        locations,
        saved,
        saved_floats,
        spill_bytes: spills * 8,
    }
}

/// Where a value already lives, if it has been given a register at all.
///
/// The table is filled with `rax` before the scan and a value that nothing reads keeps that filler;
/// the caller asks whether the answer is in the pool it is drawing from, which is what makes the
/// filler harmless.
fn self_register(locations: &[Location], value: VReg) -> Option<Reg> {
    match locations.get(value.0 as usize) {
        Some(Location::Register(register)) | Some(Location::Float(register)) => Some(*register),
        _ => None,
    }
}

/// How the instruction stream is numbered, and what is live where.
struct Positions {
    /// Where each block's operations start; a block with no operations still has a position.
    block_start: Vec<usize>,
    /// One past each block's last instruction, terminator included.
    block_end: Vec<usize>,
    /// For each block: the values it reads before defining, and the values it defines.
    ///
    /// Sets, because the liveness fixpoint is union and difference. Built as `Vec`s and deduplicated
    /// on the way in, they made the fixpoint quadratic in the number of values - `contains` on a list
    /// of three thousand - and a function the size of miniz's inflater never finished: the whole
    /// function's allocate call is still running after thirty minutes.
    uses: Vec<BTreeSet<VReg>>,
    defined: Vec<BTreeSet<VReg>>,
    /// How many virtual registers the function names.
    value_count: usize,
    /// What each block can go to, as block indices.
    successors: Vec<Vec<usize>>,
    /// Which block a position belongs to, so "what happens at this position" is a lookup.
    block_of_position: Vec<usize>,
    /// Each value's live range, for the pass that puts returned values in `rax`.
    intervals_of: Vec<Option<(usize, usize)>>,
}

impl Positions {
    fn new(func: &MachineFunction) -> Self {
        let mut block_start = Vec::with_capacity(func.blocks.len());
        let mut position = 0;
        for block in &func.blocks {
            block_start.push(position);
            // Every operation is one position; the terminator is one more.
            position += block.ops.len() + 1;
        }
        let block_end: Vec<usize> = func
            .blocks
            .iter()
            .enumerate()
            .map(|(index, _)| block_start[index] + func.blocks[index].ops.len() + 1)
            .collect();
        let mut value_count = 0;
        for block in &func.blocks {
            for op in &block.ops {
                if let Some(dst) = op.defined() {
                    value_count = value_count.max(dst.0 as usize + 1);
                }
                for operand in op.operands() {
                    value_count = value_count.max(operand.0 as usize + 1);
                }
            }
            for operand in block.term.operands() {
                value_count = value_count.max(operand.0 as usize + 1);
            }
        }
        // A block's uses are the values it reads before defining them. Every value is defined once,
        // so "defined anywhere in this block" is enough - and it is asked once per block rather than
        // once per operand, which is the difference between linear and quadratic in the size of a
        // block.
        let uses = func
            .blocks
            .iter()
            .map(|block| {
                let defined: HashSet<VReg> =
                    block.ops.iter().filter_map(|op| op.defined()).collect();
                let mut uses: BTreeSet<VReg> = BTreeSet::new();
                for operand in block
                    .ops
                    .iter()
                    .flat_map(|op| op.operands())
                    .chain(block.term.operands())
                {
                    if !defined.contains(&operand) {
                        uses.insert(operand);
                    }
                }
                uses
            })
            .collect();
        let defined = func
            .blocks
            .iter()
            .map(|block| block.ops.iter().filter_map(|op| op.defined()).collect())
            .collect();
        let successors = func
            .blocks
            .iter()
            .map(|block| match &block.term {
                BlockTerm::Return(_) | BlockTerm::Unreachable => Vec::new(),
                BlockTerm::Jump(target) => vec![*target],
                BlockTerm::Branch {
                    then, otherwise, ..
                }
                | BlockTerm::BranchCompare {
                    then, otherwise, ..
                } => vec![*then, *otherwise],
            })
            .collect();
        let mut block_of_position = vec![0; block_end.last().copied().unwrap_or(1)];
        for (index, block) in func.blocks.iter().enumerate() {
            let span = block_start[index]..block_start[index] + block.ops.len() + 1;
            for entry in &mut block_of_position[span] {
                *entry = index;
            }
        }
        Self {
            block_start,
            block_end,
            uses,
            defined,
            value_count,
            successors,
            block_of_position,
            // filled by `intervals`, which needs the whole function
            intervals_of: Vec::new(),
        }
    }

    /// A live interval per value: from its definition to its last use, or to the end of the block
    /// it is live out of.
    fn intervals(&mut self, func: &MachineFunction) -> Vec<Interval> {
        let mut hints: Vec<Option<VReg>> = vec![None; self.value_count];
        for block in &func.blocks {
            for op in &block.ops {
                if let (Some(dst), Some(source)) = (op.defined(), super::hint_source(op)) {
                    hints[dst.0 as usize] = Some(source);
                }
            }
        }
        let mut intervals: Vec<Option<Interval>> = vec![None; self.value_count];
        let touch =
            |reg: VReg, position: usize, defined: bool, intervals: &mut Vec<Option<Interval>>| {
                let slot = &mut intervals[reg.0 as usize];
                match slot {
                    Some(interval) => {
                        if defined {
                            interval.start = interval.start.min(position);
                        }
                        interval.end = interval.end.max(position);
                    }
                    None => {
                        *slot = Some(Interval {
                            reg,
                            start: position,
                            end: position,
                            hint: None,
                        });
                    }
                }
            };
        for (index, block) in func.blocks.iter().enumerate() {
            for (offset, op) in block.ops.iter().enumerate() {
                let position = self.block_start[index] + offset;
                if let Some(dst) = op.defined() {
                    touch(dst, position, true, &mut intervals);
                }
                for operand in op.operands() {
                    touch(operand, position, false, &mut intervals);
                }
            }
            let terminator = self.block_start[index] + block.ops.len();
            for operand in block.term.operands() {
                touch(operand, terminator, false, &mut intervals);
            }
        }
        // A value live out of a block lives until the block's edge, which is where its register
        // has to still be holding it for a successor's first instruction.
        let (live_in, live_out) = self.liveness();
        for (index, _) in func.blocks.iter().enumerate() {
            let end = self.block_end[index].saturating_sub(1);
            for value in &live_out[index] {
                touch(*value, end, false, &mut intervals);
            }
            for value in &live_in[index] {
                touch(*value, self.block_start[index], false, &mut intervals);
            }
        }
        let mut intervals: Vec<Interval> = intervals.into_iter().flatten().collect();
        for interval in &mut intervals {
            interval.hint = hints[interval.reg.0 as usize];
        }
        self.intervals_of = vec![None; self.value_count];
        for interval in &intervals {
            self.intervals_of[interval.reg.0 as usize] = Some((interval.start, interval.end));
        }
        intervals
    }

    /// Whether everything between two positions leaves `rax` alone. Three kinds of operation write
    /// it - a comparison, a division and a call - as does any operation whose result had to be
    /// spilled, and as does **a return of some other value**: `return x` moves `x` into `rax`, which
    /// would destroy a value living there. Only `value`'s own return is allowed.
    fn avoids_rax(
        &self,
        func: &MachineFunction,
        value: VReg,
        interval: (usize, usize),
        locations: &[Location],
    ) -> bool {
        let (start, end) = interval;
        (start..=end).all(|position| {
            let block = self.block_of_position[position];
            let instructions = &func.blocks[block].ops;
            let offset = position - self.block_start[block];
            let Some(op) = instructions.get(offset) else {
                // The position is the block's terminator.
                return match &func.blocks[block].term {
                    super::BlockTerm::Return(Some(returned)) => *returned == value,
                    _ => true,
                };
            };
            !super::writes_rax(op)
                && op.defined().is_none_or(|result| {
                    !matches!(locations[result.0 as usize], Location::Spill(_))
                })
        })
    }

    /// Live-in and live-out sets per block, by the textbook backwards fixpoint.
    fn liveness(&self) -> (Vec<BTreeSet<VReg>>, Vec<BTreeSet<VReg>>) {
        let blocks = self.uses.len();
        let mut live_in = vec![BTreeSet::new(); blocks];
        let mut live_out = vec![BTreeSet::new(); blocks];
        loop {
            let mut changed = false;
            for block in (0..blocks).rev() {
                let mut out: BTreeSet<VReg> = BTreeSet::new();
                for successor in &self.successors[block] {
                    out.extend(live_in[*successor].iter().copied());
                }
                let mut into: BTreeSet<VReg> = self.uses[block].clone();
                for value in &out {
                    if !self.defined[block].contains(value) {
                        into.insert(*value);
                    }
                }
                if out != live_out[block] || into != live_in[block] {
                    live_out[block] = out;
                    live_in[block] = into;
                    changed = true;
                }
            }
            if !changed {
                return (live_in, live_out);
            }
        }
    }
}

/// A value's location, which the tests and the encoder both ask for.
impl Allocation {
    /// Where `reg` lives.
    pub(crate) fn location(&self, reg: VReg) -> Location {
        self.locations[reg.0 as usize]
    }
}
