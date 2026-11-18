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

use std::collections::HashSet;

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
    /// In a register for its whole life.
    Register(Reg),
    /// In a frame slot at this offset from the frame body: the encoder moves it in and out.
    Spill(i32),
}

/// How many registers a function may start using without the saves outweighing the copies they
/// coalesce.
const CHEAP_REGISTERS: usize = 3;

/// The registers this allocator may use: the callee-saved ones.
pub(crate) const ALLOCATABLE: [Reg; 7] = [
    Reg(3),  // rbx
    Reg(6),  // rsi
    Reg(7),  // rdi
    Reg(12), // r12
    Reg(13), // r13
    Reg(14), // r14
    Reg(15), // r15
];

/// What allocation decided for one function.
#[derive(Debug)]
pub(crate) struct Allocation {
    /// Where each virtual register lives, by its index. A value that nothing reads - the address of
    /// a function whose only use is the call that names it - has no interval, so its entry here is
    /// meaningless and must not be read.
    pub locations: Vec<Location>,
    /// The callee-saved registers used, in the order the prologue saves them.
    pub saved: Vec<Reg>,
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
    let positions = Positions::new(func);
    let intervals = positions.intervals(func);
    let mut locations = vec![Location::Register(Reg(0)); positions.value_count];
    let mut spills = 0;

    // Intervals by start, which is the order the scan needs; ties keep the earlier definition
    // first so a value is not expired before the value it defines has been given a register.
    let mut order: Vec<Interval> = intervals;
    order.sort_by_key(|interval| (interval.start, interval.reg));

    let mut active: Vec<Interval> = Vec::new();
    let mut free: Vec<Reg> = ALLOCATABLE.to_vec();
    let mut saved: Vec<Reg> = Vec::new();
    for interval in order {
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
                free.push(location_register(locations[other.reg.0 as usize]));
            } else {
                index += 1;
            }
        }
        free.sort();
        // Coalescing: a value computed from another one takes the same register when it is free, so
        // the `mov` that would copy it is not emitted at all.
        if let Some(hint) = interval.hint {
            // A register that has not been saved yet costs two `mov`s in the prologue and epilogue,
            // and the copy it removes is one: it is worth taking only while the set is small.
            if let Some(register) = free.iter().find(|register| {
                self_register(&locations, hint) == Some(**register)
                    && (saved.contains(register) || saved.len() < CHEAP_REGISTERS)
            }) {
                let register = *register;
                free.retain(|other| *other != register);
                locations[interval.reg.0 as usize] = Location::Register(register);
                if !saved.contains(&register) {
                    saved.push(register);
                }
                active.push(interval);
                active.sort_by_key(|other| other.end);
                continue;
            }
        }
        if free.is_empty() {
            // Nothing is free: the interval that runs longest loses its register, because that
            // register then covers the most instructions. A tie spills the new one, which keeps
            // the scan's decision stable.
            let victim = active
                .iter()
                .enumerate()
                .max_by_key(|(_, other)| other.end)
                .map(|(index, other)| (index, *other));
            match victim {
                Some((index, other)) if other.end > interval.end => {
                    let register = location_register(locations[other.reg.0 as usize]);
                    active.remove(index);
                    locations[other.reg.0 as usize] = Location::Spill(spills * 8);
                    spills += 1;
                    locations[interval.reg.0 as usize] = Location::Register(register);
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
            locations[interval.reg.0 as usize] = Location::Register(register);
            if !saved.contains(&register) {
                saved.push(register);
            }
            active.push(interval);
            active.sort_by_key(|other| other.end);
        }
    }
    // The prologue saves in a fixed order, so two runs of the compiler save in the same order.
    saved.sort();
    Allocation {
        locations,
        saved,
        spill_bytes: spills * 8,
    }
}

/// Where a value already lives, if it has been given a register.
fn self_register(locations: &[Location], value: VReg) -> Option<Reg> {
    match locations.get(value.0 as usize) {
        Some(Location::Register(register)) => Some(*register),
        _ => None,
    }
}

fn location_register(location: Location) -> Reg {
    match location {
        Location::Register(register) => register,
        Location::Spill(_) => unreachable!("a spilled value has no register to give back"),
    }
}

/// How the instruction stream is numbered, and what is live where.
struct Positions {
    /// Where each block's operations start; a block with no operations still has a position.
    block_start: Vec<usize>,
    /// One past each block's last instruction, terminator included.
    block_end: Vec<usize>,
    /// For each block: the values it reads before defining, and the values it defines.
    uses: Vec<Vec<VReg>>,
    defined: Vec<Vec<VReg>>,
    /// How many virtual registers the function names.
    value_count: usize,
    /// What each block can go to, as block indices.
    successors: Vec<Vec<usize>>,
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
        let block_end = func
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
                let mut seen: HashSet<VReg> = HashSet::new();
                let mut uses = Vec::new();
                for operand in block
                    .ops
                    .iter()
                    .flat_map(|op| op.operands())
                    .chain(block.term.operands())
                {
                    if !defined.contains(&operand) && seen.insert(operand) {
                        uses.push(operand);
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
                } => vec![*then, *otherwise],
            })
            .collect();
        Self {
            block_start,
            block_end,
            uses,
            defined,
            value_count,
            successors,
        }
    }

    /// A live interval per value: from its definition to its last use, or to the end of the block
    /// it is live out of.
    fn intervals(&self, func: &MachineFunction) -> Vec<Interval> {
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
        intervals
    }

    /// Live-in and live-out sets per block, by the textbook backwards fixpoint.
    fn liveness(&self) -> (Vec<Vec<VReg>>, Vec<Vec<VReg>>) {
        let blocks = self.uses.len();
        let mut live_in = vec![Vec::new(); blocks];
        let mut live_out = vec![Vec::new(); blocks];
        loop {
            let mut changed = false;
            for block in (0..blocks).rev() {
                let mut out: Vec<VReg> = Vec::new();
                for successor in &self.successors[block] {
                    for value in &live_in[*successor] {
                        if !out.contains(value) {
                            out.push(*value);
                        }
                    }
                }
                let mut into: Vec<VReg> = self.uses[block].clone();
                for value in &out {
                    if !self.defined[block].contains(value) && !into.contains(value) {
                        into.push(*value);
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
