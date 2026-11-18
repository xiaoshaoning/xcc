//! This compiler's own x86-64 back end: `cir` to machine instructions, an allocator, and a COFF
//! object, with no Cranelift under it.
//!
//! ADR-001 section 9 asked for a seam that a second back end could be dropped into; this is it,
//! and ADR-008 is the record of why it is shaped this way. What is here is a **slice**: integer
//! code with frames, calls and control flow. Everything else - floating point, function pointers,
//! aggregates by value, globals - refuses with the reason and the source span, so a program this
//! back end cannot compile says so instead of running differently from the same program at level 0.
//!
//! Five files, each hiding one decision:
//!
//! - [`lower`] turns `cir` into three-address operations on virtual registers, and decides the
//!   frame's shape: what lives in memory (block parameters, locals whose address escapes) and what
//!   is merely canonical in a register.
//! - [`regalloc`] is linear scan over live intervals computed from the control-flow graph. Only
//!   callee-saved registers are allocated, so a value is safe across a call for free; the ceiling
//!   that buys (seven registers, and a spill for the eighth) is written down rather than hidden.
//! - [`encode`] is the assembler: x86-64 machine code for the closed set of operations above, with
//!   the two passes a jump needs.
//! - [`coff`] is the object file: one `.text`, function symbols, and the relocations a call needs.
//! - this file is the seam: refusal type, the entry point, and the tests that say the seam fits.

pub(crate) mod coff;
pub(crate) mod encode;
pub(crate) mod lower;
pub(crate) mod regalloc;

use crate::cir::BinOp;
use crate::cir::CmpOp;
use crate::diag::Span;

/// Why this back end cannot compile something yet. Not a compiler bug: a documented gap, refused
/// with the expression that asked for it, so the caller fixes the program rather than the compiler.
#[derive(Debug)]
pub(crate) struct NotYet {
    /// What is not lowered.
    pub message: String,
    /// Where in the source it is.
    pub span: Span,
}

impl std::fmt::Display for NotYet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// A virtual register: one per value, and one per temporary this back end needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct VReg(pub u32);

/// One machine operation. Every operand is a virtual register; the allocator gives each a register
/// or a frame slot, and the encoder never sees a virtual register at all.
#[derive(Debug, Clone)]
pub(crate) enum Op {
    /// A 64-bit constant, already canonical for its C type.
    Const { dst: VReg, value: u64 },
    /// An argument, as it arrives in the register Win64 gives that position.
    Incoming {
        dst: VReg,
        index: u8,
        size: u8,
        signed: bool,
    },
    /// The address of a frame slot: `lea`.
    LocalAddress { dst: VReg, offset: i32 },
    /// Read eight bytes of a frame slot, which is how a block parameter arrives.
    LoadLocal { dst: VReg, offset: i32 },
    /// Write eight bytes of a frame slot, which is how an edge passes them.
    StoreLocal { offset: i32, src: VReg },
    /// Read `size` bytes at `base + offset`, widened to 64 bits.
    LoadMemory {
        dst: VReg,
        base: VReg,
        offset: i32,
        size: u8,
        signed: bool,
    },
    /// Write the low `size` bytes of `src` at `base + offset`.
    StoreMemory {
        base: VReg,
        offset: i32,
        src: VReg,
        size: u8,
    },
    /// Two values, one operation, in 64 bits; the result is canonicalised to `size`.
    Binary {
        dst: VReg,
        op: BinOp,
        left: VReg,
        right: VReg,
        size: u8,
        signed: bool,
    },
    /// `<<`, or a right shift that is arithmetic or logical by the operand's type.
    Shift {
        dst: VReg,
        left: VReg,
        right: VReg,
        right_signed: bool,
        left_shift: bool,
        size: u8,
        signed: bool,
    },
    /// `/` or `%`, which x86 splits between `rax` and `rdx`.
    Divide {
        dst: VReg,
        left: VReg,
        right: VReg,
        remainder: bool,
        signed: bool,
        size: u8,
    },
    /// Unary `-` or `~`.
    Unary {
        dst: VReg,
        src: VReg,
        negate: bool,
        size: u8,
        signed: bool,
    },
    /// A comparison, giving 1 or 0 in an `int`: `setcc` and a move.
    Compare {
        dst: VReg,
        op: CmpOp,
        left: VReg,
        right: VReg,
        signed: bool,
    },
    /// Truncate to `size` and extend back to 64 bits, which is all a C conversion of integers is
    /// once values are canonical.
    Canonical {
        dst: VReg,
        src: VReg,
        size: u8,
        signed: bool,
    },
    /// Call a function by name: `call rel32`, with the arguments staged through the frame because
    /// `rcx` may hold one of them while another has to go there.
    Call {
        dst: Option<VReg>,
        name: String,
        args: Vec<VReg>,
        size: u8,
        signed: bool,
    },
}

impl Op {
    /// The virtual register this operation defines, if it defines one.
    pub(crate) fn defined(&self) -> Option<VReg> {
        match self {
            Op::Const { dst, .. }
            | Op::Incoming { dst, .. }
            | Op::LocalAddress { dst, .. }
            | Op::LoadLocal { dst, .. }
            | Op::LoadMemory { dst, .. }
            | Op::Binary { dst, .. }
            | Op::Shift { dst, .. }
            | Op::Divide { dst, .. }
            | Op::Unary { dst, .. }
            | Op::Compare { dst, .. }
            | Op::Canonical { dst, .. } => Some(*dst),
            Op::Call { dst, .. } => *dst,
            Op::StoreLocal { .. } | Op::StoreMemory { .. } => None,
        }
    }

    /// The virtual registers this operation reads, in order.
    pub(crate) fn operands(&self) -> Vec<VReg> {
        match self {
            Op::Const { .. }
            | Op::Incoming { .. }
            | Op::LocalAddress { .. }
            | Op::LoadLocal { .. } => Vec::new(),
            Op::StoreLocal { src, .. } => vec![*src],
            Op::LoadMemory { base, .. } => vec![*base],
            Op::StoreMemory { base, src, .. } => vec![*base, *src],
            Op::Binary { left, right, .. }
            | Op::Shift { left, right, .. }
            | Op::Divide { left, right, .. }
            | Op::Compare { left, right, .. } => vec![*left, *right],
            Op::Unary { src, .. } | Op::Canonical { src, .. } => vec![*src],
            Op::Call { args, .. } => args.clone(),
        }
    }
}

/// How a block ends.
#[derive(Debug, Clone)]
pub(crate) enum BlockTerm {
    /// Leave the function, with the value in `rax`.
    Return(Option<VReg>),
    /// Continue at another block, by index in layout order.
    Jump(usize),
    /// Two ways out; the condition is non-zero or zero.
    Branch {
        condition: VReg,
        then: usize,
        otherwise: usize,
    },
    /// Nothing follows.
    Unreachable,
}

impl BlockTerm {
    /// The virtual registers the terminator reads.
    pub(crate) fn operands(&self) -> Vec<VReg> {
        match self {
            BlockTerm::Return(Some(value)) => vec![*value],
            BlockTerm::Return(None) | BlockTerm::Jump(_) | BlockTerm::Unreachable => Vec::new(),
            BlockTerm::Branch { condition, .. } => vec![*condition],
        }
    }
}

/// One block in machine form.
#[derive(Debug)]
pub(crate) struct MachineBlock {
    /// What happens, in order.
    pub ops: Vec<Op>,
    /// How it ends.
    pub term: BlockTerm,
}

/// One function in machine form, before allocation.
#[derive(Debug)]
pub(crate) struct MachineFunction {
    /// Its link name.
    pub name: String,
    /// How many arguments it takes, which the encoder uses to save what the caller expects.
    pub params: usize,
    /// Bytes of frame the lowering needed, block parameters included; spills are added after.
    pub frame_bytes: i32,
    /// The blocks, in layout order.
    pub blocks: Vec<MachineBlock>,
    /// Where each block's parameters start, in the same order.
    pub param_offsets: Vec<i32>,
    /// Which register a parameter arrives in, by position.
    #[allow(dead_code)]
    // the encoder spells the four registers; this keeps the table in one place
    pub arg_register: fn(u8) -> u8,
}

/// Lower a whole translation unit to a COFF object, or say what is not lowered.
pub(crate) fn object(
    module: &crate::cir::Module,
    names: &crate::intern::Interner,
) -> Result<Vec<u8>, NotYet> {
    if !module.globals.is_empty() {
        let span = module
            .funcs
            .first()
            .map(|func| func.entry_span())
            .unwrap_or(Span::at(0));
        return Err(NotYet {
            message: "a translation unit with global objects".into(),
            span,
        });
    }
    let functions = lower::lower(module, names)?;
    Ok(coff::write(&functions))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_operation_knows_what_it_reads_and_writes() {
        let add = Op::Binary {
            dst: VReg(2),
            op: BinOp::Add,
            left: VReg(0),
            right: VReg(1),
            size: 4,
            signed: true,
        };
        assert_eq!(add.defined(), Some(VReg(2)));
        assert_eq!(add.operands(), vec![VReg(0), VReg(1)]);
        let store = Op::StoreMemory {
            base: VReg(3),
            offset: 0,
            src: VReg(4),
            size: 8,
        };
        assert_eq!(store.defined(), None);
        assert_eq!(store.operands(), vec![VReg(3), VReg(4)]);
    }
}
