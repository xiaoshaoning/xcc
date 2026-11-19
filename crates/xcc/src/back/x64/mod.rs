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
//!   the two passes a jump needs. It reads an operation's virtual registers through the allocation,
//!   so the only registers it names are the allocation's and its own scratch.
//! - [`coff`] is the object file: one `.text`, function symbols, and the relocations a call needs.
//! - this file is the seam: refusal type, the entry point, and the tests that say the seam fits.

pub(crate) mod coff;
pub(crate) mod encode;
pub(crate) mod lower;
pub(crate) mod regalloc;

use crate::cir::BinOp;
use crate::cir::CmpOp;
use crate::diag::Span;

use regalloc::Reg;

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

/// The registers one function's code may use, decided by what the function contains.
///
/// This is the one place that knows which registers the instruction set fixes for what - `rcx` is a
/// shift's count and the first argument, `rdx` a division's high half and the second, `rax` a
/// comparison's result and the return value - so a function that does none of those things can lend
/// the registers out instead of leaving them idle, and one that does keeps them to itself.
pub(crate) struct Registers {
    /// What the allocator may give to values, in the order it prefers them.
    pub allocatable: &'static [Reg],
    /// Two registers the encoder may clobber while it works, never given to a value.
    pub scratch: [Reg; 2],
}

impl MachineFunction {
    /// The registers this function's code may use. A call makes the caller-saved registers useless
    /// (a call destroys them) and the callee-saved ones free (a call preserves them); a function
    /// that calls nothing has the opposite trade, and it is worth taking because the saves are then
    /// two `mov`s each that nothing needs.
    pub(crate) fn registers(&self) -> Registers {
        let shifts_or_divides = self
            .blocks
            .iter()
            .flat_map(|block| &block.ops)
            .any(|op| matches!(op, Op::Shift { .. } | Op::Divide { .. }));
        // `rax` is never allocatable: a comparison writes its low byte, a division its whole width,
        // and the return value goes there. It is the spare the encoder computes a spilled result in.
        //
        // The argument registers are excluded with the same reasoning: a value moved into `r8` while
        // the fourth argument is still there destroys it. Two arguments arrive in `rcx` and `rdx`,
        // which no caller-saved set allocates, so two is the count under which the sets are safe.
        let arguments_are_safe = self.param_count <= 2;
        if !self.makes_call && arguments_are_safe && !shifts_or_divides {
            Registers {
                allocatable: &regalloc::CALLER_SAVED,
                // This function fixes nothing to `rcx` or `rdx`, so they are the scratch pair and
                // the last two caller-saved registers are lent out.
                scratch: [Reg(1), Reg(2)],
            }
        } else if !self.makes_call && arguments_are_safe {
            Registers {
                allocatable: &regalloc::CALLER_SAVED[..2],
                scratch: [Reg(10), Reg(11)],
            }
        } else {
            Registers {
                allocatable: &regalloc::CALLEE_SAVED,
                scratch: [Reg(10), Reg(11)],
            }
        }
    }
}

/// How many integer arguments Win64 passes in registers. One place, because every module needs the
/// number: the encoder names the registers and reads the rest from the stack, and the register plan
/// keeps the argument registers out of the allocatable set for a function that takes more.
pub(crate) const ARGUMENT_COUNT: usize = 4;

/// A virtual register: one per value, and one per temporary this back end needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct VReg(pub u32);

/// One machine operation. Every operand is a virtual register; the allocator gives each a register
/// or a frame slot, and the encoder never sees a virtual register at all.
#[derive(Debug, Clone)]
pub(crate) enum Op {
    /// A 64-bit constant, already canonical for its C type.
    Const { dst: VReg, value: u64 },
    /// An argument: the first four arrive in the registers Win64 gives those positions, and the rest
    /// in the caller's outgoing area, above the shadow space.
    Incoming {
        dst: VReg,
        /// Its position in the parameter list, from zero.
        index: u8,
        size: u8,
        signed: bool,
    },
    /// The address of a frame slot: `lea`.
    LocalAddress { dst: VReg, offset: i32 },
    /// The address of an object in the data section, by its link name: `lea` with a displacement the
    /// object file's relocation fills in. Also how a string literal's address is taken.
    DataAddress { dst: VReg, symbol: String },
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
    /// Call a function by name: `call rel32`. The first four arguments go in registers, staged
    /// through the frame when one of them would destroy another; the rest go in the function's
    /// outgoing slots, which is what the frame reserves them for.
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
            | Op::DataAddress { dst, .. }
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
            | Op::DataAddress { .. }
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

/// Whether an operation writes `rax`, which is where a function returns its value, and the one
/// register a value may not share. The three that do are the ones the instruction set fixes: a
/// comparison's low byte, a division's dividend and high half, and a call's result.
fn writes_rax(op: &Op) -> bool {
    matches!(op, Op::Compare { .. } | Op::Divide { .. } | Op::Call { .. })
}

/// The value an operation copies, if it copies one: the operand its result is computed from. The
/// allocator uses this to put the two in one register and emit no `mov` at all.
fn hint_source(op: &Op) -> Option<VReg> {
    match op {
        Op::Binary { left, .. } | Op::Shift { left, .. } | Op::Unary { src: left, .. } => {
            Some(*left)
        }
        Op::Canonical { src, .. } => Some(*src),
        _ => None,
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
    /// Bytes of frame the lowering needed, block parameters included; spills are added after.
    pub frame_bytes: i32,
    /// How many argument *slots* the widest call in this function pushes on the stack: everything
    /// beyond the fourth, eight bytes each. The frame has to make room for them above the shadow
    /// space, because a callee reads its fifth argument at `[rsp + 40]` once its own frame is
    /// subtracted - the shadow space and the return address above it.
    pub outgoing_slots: u32,
    /// The values this function returns: the ones a `return` hands back, which have to end up in
    /// `rax` and are worth keeping there.
    pub returned: Vec<VReg>,
    /// How many arguments it takes. A caller-saved register may only be allocated when this is two
    /// or fewer: arguments arrive in `rcx`, `rdx`, `r8` and `r9`, and moving the first one into `r8`
    /// would destroy the third before anything had read it.
    pub param_count: usize,
    /// Whether anything in this function calls another. A function that calls has to keep the
    /// callee-saved registers intact for its caller and can keep its own values in them; a function
    /// that calls nothing has neither obligation, and can use caller-saved registers instead - which
    /// are destroyed by a call, and there is no call.
    pub makes_call: bool,
    /// The blocks, in layout order.
    pub blocks: Vec<MachineBlock>,
    /// Where each block's parameters start, in the same order.
    pub param_offsets: Vec<i32>,
}

/// Lower a whole translation unit to a COFF object, or say what is not lowered.
pub(crate) fn object(
    module: &crate::cir::Module,
    names: &crate::intern::Interner,
) -> Result<Vec<u8>, NotYet> {
    let functions = lower::lower(module, names)?;
    let data = data_section(module)?;
    Ok(coff::write(&functions, &data))
}

/// The objects in the data section: their bytes, laid out in the order the module holds them, each
/// at its type's alignment, with the name of every one and where it starts.
///
/// A global that is only *declared* is not laid out: it is a name the linker resolves, and the object
/// says so with an undefined symbol. A global that is *defined* always has bytes - the initialiser the
/// front end wrote, or as many zeroes as its type takes, because a defined object with no initialiser
/// holds zero (C99 6.7.8p10) - so a defined object whose type has no size is a gap to report rather
/// than an object to leave out.
fn data_section(module: &crate::cir::Module) -> Result<coff::Data, NotYet> {
    let mut data = coff::Data::default();
    for global in &module.globals {
        if !global.defined {
            data.undefined.push(global.name.clone());
            continue;
        }
        let align = module.types.align_of(global.ty).unwrap_or(1).max(1) as usize;
        while data.bytes.len() % align != 0 {
            data.bytes.push(0);
        }
        data.symbols.push(coff::DataSymbol {
            name: global.name.clone(),
            offset: data.bytes.len() as u32,
            external: global.external,
        });
        match &global.init {
            Some(init) => data.bytes.extend_from_slice(init),
            None => {
                let size = module.types.size_of(global.ty).ok_or_else(|| NotYet {
                    message: format!(
                        "a defined object of incomplete type {}",
                        module.types.describe(global.ty)
                    ),
                    span: global.span,
                })? as usize;
                data.bytes.resize(data.bytes.len() + size, 0);
            }
        }
    }
    Ok(data)
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
