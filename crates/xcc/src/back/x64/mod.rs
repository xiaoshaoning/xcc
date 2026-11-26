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
pub(crate) mod load;
pub(crate) mod lower;
pub(crate) mod pe;
pub(crate) mod regalloc;

use crate::cir::BinOp;
use crate::cir::CmpOp;
use crate::cir::Segment;
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
    /// The general registers the allocator may give to integer values, in the order it prefers them.
    pub integer: Vec<Reg>,
    /// The same for floating-point values. A separate field, not a separate decision: the reasons to
    /// exclude a register are the ones below, applied to both files at once.
    pub float: Vec<Reg>,
    /// Two registers the encoder may clobber while it works, never given to a value.
    pub scratch: [Reg; 2],
}

/// The general registers the first four arguments arrive in, by position.
pub(crate) const ARGUMENT_REGISTERS: [Reg; ARGUMENT_COUNT] = [Reg(1), Reg(2), Reg(8), Reg(9)];

/// The vector registers the first four arguments arrive in, by the same position: the two files are
/// counted separately, so position 1 puts an `int` in `rdx` and a `double` in `xmm1`.
pub(crate) const FLOAT_ARGUMENTS: [Reg; ARGUMENT_COUNT] = [Reg(0), Reg(1), Reg(2), Reg(3)];

/// A caller-saved pool minus the registers a parameter arrives in. A pool that loses every register
/// to the arguments falls back to the callee-saved one: two `mov`s are cheaper than the spill that
/// would otherwise follow, and no argument arrives in a callee-saved register.
fn left_to_lend(pool: &[Reg], arriving: &[Reg], saved: &[Reg]) -> Vec<Reg> {
    let mut registers: Vec<Reg> = pool
        .iter()
        .copied()
        .filter(|register| !arriving.contains(register))
        .collect();
    if registers.is_empty() {
        registers.extend_from_slice(saved);
    }
    registers
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
        // So is every register a parameter arrives in, and for a sharper reason: the prologue moves a
        // parameter into the register the allocator chose, and that move can destroy a *later*
        // parameter that arrived in the same register and has not been read yet. `f(int a, double b,
        // int c, double d)` is the case that found this - `b` given `xmm3`, where `d` was waiting.
        // Counting the arguments that arrive and dropping those registers covers both files with one
        // rule; two per-class thresholds is how the float half of it was missed twice.
        let arriving = self.param_count.min(ARGUMENT_COUNT);
        if self.makes_call {
            // A call destroys every caller-saved register, so only the callee-saved ones are usable -
            // and no argument arrives in one, so nothing has to be filtered. The price is a `mov` to
            // save and another to restore, once per register the function actually touches.
            return Registers {
                integer: regalloc::CALLEE_SAVED.to_vec(),
                float: regalloc::CALLEE_SAVED_FLOATS.to_vec(),
                scratch: [Reg(10), Reg(11)],
            };
        }
        // A function that calls nothing keeps the caller-saved registers instead, because the saves
        // they would cost are `mov`s nothing needs. `rcx` and `rdx` are lent out only when nothing
        // fixes them: a shift's count is `rcx` and a division's high half is `rdx`, and then they are
        // the scratch pair instead.
        let (integer_pool, scratch) = if shifts_or_divides {
            (&regalloc::CALLER_SAVED[..2], [Reg(10), Reg(11)])
        } else {
            (&regalloc::CALLER_SAVED[..], [Reg(1), Reg(2)])
        };
        Registers {
            integer: left_to_lend(
                integer_pool,
                &ARGUMENT_REGISTERS[..arriving],
                &regalloc::CALLEE_SAVED,
            ),
            float: left_to_lend(
                &regalloc::FLOAT_REGISTERS,
                &FLOAT_ARGUMENTS[..arriving],
                &regalloc::CALLEE_SAVED_FLOATS,
            ),
            scratch,
        }
    }
}

/// How many integer arguments Win64 passes in registers. One place, because every module needs the
/// number: the encoder names the registers and reads the rest from the stack, and the register plan
/// keeps the argument registers out of the allocatable set for a function that takes more.
pub(crate) const ARGUMENT_COUNT: usize = 4;

/// Which register file a value lives in. x86-64 has two, and a value belongs to exactly one: a float
/// is never in a general register, an integer never in an XMM one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Class {
    /// An integer or a pointer, in a general-purpose register.
    Integer,
    /// A `float` or a `double`, in an XMM register.
    Float,
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
    /// An argument. Win64 gives the first four *positions* a register each - the position picks the
    /// register, and the argument's class picks which of the two files it is in, so `f(int, double)`
    /// has its `int` in `rcx` and its `double` in `xmm1`. The fifth position onwards goes in the
    /// caller's outgoing area whatever its class.
    Incoming {
        dst: VReg,
        /// The argument's position among the parameters, which is its index into *both* argument
        /// files at once - 0 is `rcx` or `xmm0`, 1 is `rdx` or `xmm1` - or, when `on_stack`, the
        /// outgoing slot it was written to.
        slot: u8,
        /// The register file it belongs to, which decides the register and how it is moved.
        class: Class,
        /// Whether the caller pushed it.
        on_stack: bool,
        size: u8,
        signed: bool,
    },
    /// The address of a frame slot: `lea`.
    LocalAddress { dst: VReg, offset: i32 },
    /// The address of a symbol, by its link name: `lea` with a displacement the object file's
    /// relocation fills in. A string literal, a global object and - since function pointers - a
    /// function are all a name the linker resolves to an address.
    SymbolAddress { dst: VReg, symbol: String },
    /// A read of a segment-relative word: `mov rax, gs:[offset]`. The machine has one instruction
    /// for it, and the SDK's `__readgsqword` family is that instruction - which is how the thread
    /// environment block is reached, and therefore how `GetLastError` and `NtCurrentTeb` work.
    SegmentRead {
        dst: VReg,
        segment: Segment,
        /// The register holding the byte offset into the segment, and the constant part of it: a
        /// constant offset folds into the displacement, which is what `gs:[0x30]` writes.
        base: VReg,
        offset: i32,
        size: u8,
    },
    /// A compare-and-swap at `[pointer]`, of `size` bytes: if the object equals `expected` it
    /// becomes `desired`, and `dst` is the value read - which equals `expected` exactly when the
    /// swap happened. The one machine operation atomic arithmetic is built from; everything else is
    /// a loop of these in C (`crates/xcc-crt/src/main.c`).
    AtomicCas {
        dst: VReg,
        pointer: Operand,
        expected: Operand,
        desired: Operand,
        size: u8,
    },
    /// The address of a *thread-local* object: not a `lea`, because there is no single address to
    /// load. The runtime keeps one block per thread and hands out this object's cell in it; the
    /// linker decides where in that block the object sits, and writes that offset into a word the
    /// sequence below reads. Nothing here sizes the object: the block's template does, once, for
    /// all of them.
    ThreadAddress { dst: VReg, symbol: String },
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
        right: Operand,
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
    /// `va_start`: the address of the first argument that is not a parameter. `skip` counts those
    /// parameters, and the caller's home area holds the first four arguments, so it is one `lea`.
    VaStart { dst: VReg, skip: u8 },
    /// `va_arg`: read the type at the list's address. The step is the front end's store to the list,
    /// so this is a load and nothing else.
    VaArg {
        dst: VReg,
        list: VReg,
        size: u8,
        signed: bool,
        /// Which register file the value lands in. Win64 duplicates a variadic `double` into a
        /// general-purpose register precisely so a walk like this one can find it, so the read is the
        /// same eight bytes and only the file differs.
        class: Class,
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
        right: Operand,
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
    /// Two floating-point values, one operation: `+`, `-`, `*` or `/`, in the type's own width.
    FloatBinary {
        dst: VReg,
        op: BinOp,
        left: VReg,
        right: VReg,
        /// 4 for `float`, 8 for `double`.
        size: u8,
    },
    /// A floating-point comparison, giving 1 or 0 in an `int` - so the answer lands in a general
    /// register, because that is the register C's answer belongs in.
    FloatCompare {
        dst: VReg,
        op: CmpOp,
        left: VReg,
        right: VReg,
        size: u8,
    },
    /// Load a floating-point constant into a register; its bits travel through a general register.
    FloatConst { dst: VReg, bits: u64, size: u8 },
    /// Convert between the two floating widths, or between a float and an integer.
    FloatConvert {
        dst: VReg,
        src: VReg,
        from: Class,
        to: Class,
        /// The source's width and the destination's, in bytes.
        from_size: u8,
        to_size: u8,
        /// Whether the source is signed: an integer's sign for `int`-to-float, the float's for
        /// `float`-to-`int`, where it chooses between the signed and unsigned conversions.
        signed: bool,
    },
    /// Read a floating-point value from memory.
    LoadFloat {
        dst: VReg,
        base: VReg,
        offset: i32,
        size: u8,
    },
    /// Write one.
    StoreFloat {
        base: VReg,
        offset: i32,
        src: VReg,
        size: u8,
    },
    /// Call through a pointer: `call r/m`, with the same argument placement as a direct call and the
    /// target in a register. This is how a function pointer is called (C99 6.5.2.2p1).
    CallIndirect {
        dst: Option<VReg>,
        /// The pointer being called.
        callee: VReg,
        args: Vec<VReg>,
        size: u8,
        signed: bool,
        /// See `Call`: a variadic call through a pointer duplicates its float arguments the same way.
        variadic: bool,
    },
    /// Call a function by name: `call rel32`. The first four arguments go in registers, moved there
    /// directly - see `place_arguments` for why two of them can never be in each other's way - and
    /// the rest go in the function's outgoing slots, which is what the frame reserves them for.
    Call {
        dst: Option<VReg>,
        name: String,
        args: Vec<VReg>,
        size: u8,
        signed: bool,
        /// Whether the call passes more arguments than the callee declares. A variadic call also
        /// duplicates each floating-point argument into the general register of its position, and
        /// answers `AL` - Win64's count of the vector registers the call used - with zero, so that a
        /// callee reads the general registers, where the copies are.
        variadic: bool,
    },
}

/// What an operation computes with: a value in a register, or a constant it can carry itself.
///
/// The corpus puts a third of its bytes into materialising constants - `mov r32, imm32` for a value a
/// `cmp` or an `add` could hold in the instruction - and 62% of those constants are read exactly once,
/// by exactly such an operation. Saying "an operand is a register **or** a constant" makes that
/// expressible once, and makes every use site say which it handles: the alternative, a register that
/// nothing ever defines, is a wrong answer waiting for a reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Operand {
    /// A value in a register.
    Value(VReg),
    /// A constant, as it would be materialised: canonical for its C type.
    Immediate(u64),
}

impl Operand {
    /// The register this operand reads, if it reads one.
    pub(crate) fn register(self) -> Option<VReg> {
        match self {
            Operand::Value(value) => Some(value),
            Operand::Immediate(_) => None,
        }
    }
}

impl Op {
    /// The virtual register this operation defines, if it defines one.
    pub(crate) fn defined(&self) -> Option<VReg> {
        match self {
            Op::Const { dst, .. }
            | Op::Incoming { dst, .. }
            | Op::LocalAddress { dst, .. }
            | Op::SymbolAddress { dst, .. }
            | Op::ThreadAddress { dst, .. }
            | Op::AtomicCas { dst, .. }
            | Op::SegmentRead { dst, .. }
            | Op::LoadLocal { dst, .. }
            | Op::LoadMemory { dst, .. }
            | Op::Binary { dst, .. }
            | Op::Shift { dst, .. }
            | Op::Divide { dst, .. }
            | Op::Unary { dst, .. }
            | Op::VaStart { dst, .. }
            | Op::VaArg { dst, .. }
            | Op::Compare { dst, .. }
            | Op::FloatBinary { dst, .. }
            | Op::FloatCompare { dst, .. }
            | Op::FloatConst { dst, .. }
            | Op::FloatConvert { dst, .. }
            | Op::LoadFloat { dst, .. }
            | Op::Canonical { dst, .. } => Some(*dst),
            Op::Call { dst, .. } | Op::CallIndirect { dst, .. } => *dst,
            Op::StoreLocal { .. } | Op::StoreMemory { .. } | Op::StoreFloat { .. } => None,
        }
    }

    /// The virtual registers this operation reads, in order.
    pub(crate) fn operands(&self) -> Vec<VReg> {
        match self {
            Op::Const { .. }
            | Op::Incoming { .. }
            | Op::LocalAddress { .. }
            | Op::SymbolAddress { .. }
            | Op::ThreadAddress { .. }
            | Op::LoadLocal { .. } => Vec::new(),
            // The offset is a register the read *uses*, and leaving it out is the mistake the
            // `AtomicCas` note below records: an operand the allocator does not know about ends its
            // live range here, and the register is handed to `dst` - a read from wherever the
            // destination landed.
            Op::SegmentRead { base, .. } => vec![*base],
            Op::AtomicCas {
                pointer,
                expected,
                desired,
                ..
            } => {
                // All three, and `expected` is the one that is easy to forget: leaving it out ended
                // its live range here, so the allocator handed its frame slot to another value - and
                // the loop compared the value it had just read with itself, which always looked like
                // a successful swap. Single-threaded that is invisible, because the first swap *does*
                // succeed; two threads lost half their increments.
                let mut read = vec![];
                read.extend(pointer.register());
                read.extend(expected.register());
                read.extend(desired.register());
                read
            }
            Op::StoreLocal { src, .. } => vec![*src],
            Op::LoadMemory { base, .. } => vec![*base],
            Op::StoreMemory { base, src, .. } => vec![*base, *src],
            Op::Binary { left, right, .. } | Op::Compare { left, right, .. } => {
                let mut read = vec![*left];
                read.extend(right.register());
                read
            }
            Op::Shift { left, right, .. }
            | Op::Divide { left, right, .. }
            | Op::FloatBinary { left, right, .. }
            | Op::FloatCompare { left, right, .. } => vec![*left, *right],
            Op::LoadFloat { base, .. } => vec![*base],
            Op::StoreFloat { base, src, .. } => vec![*base, *src],
            Op::FloatConvert { src, .. } => vec![*src],
            Op::FloatConst { .. } => Vec::new(),
            Op::VaArg { list, .. } => vec![*list],
            Op::VaStart { .. } => Vec::new(),
            Op::Unary { src, .. } | Op::Canonical { src, .. } => vec![*src],
            Op::Call { args, .. } => args.clone(),
            Op::CallIndirect { callee, args, .. } => {
                let mut operands = vec![*callee];
                operands.extend(args.iter().copied());
                operands
            }
        }
    }
}

/// Whether an operation writes `rax`, which is where a function returns its value, and the one
/// register a value may not share. The three that do are the ones the instruction set fixes: a
/// comparison's low byte, a division's dividend and high half, and a call's result.
fn writes_rax(op: &Op) -> bool {
    matches!(
        op,
        // The four that need the accumulator by the instruction set's choice...
        Op::Compare { .. } | Op::Divide { .. } | Op::Call { .. } | Op::CallIndirect { .. }
        // ...and the three that need it because this compiler wrote them: `lock cmpxchg` compares
        // against `rax` and leaves the value it read there; a segment read is `mov rax, gs:[..]`;
        // and the thread-local lookup is a call this back end emits rather than an `Op::Call`.
        | Op::AtomicCas { .. }
        | Op::SegmentRead { .. }
        | Op::ThreadAddress { .. }
    )
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
    /// Two ways out, decided by comparing two values. The same thing as `Branch` on a comparison's
    /// answer, but the answer is never materialised: 45% of the comparisons in the corpus are read by
    /// nothing but a branch, and computing `setcc`, widening it and testing it again costs about seven
    /// bytes each where `cmp` and a `jcc` cost two instructions between them.
    BranchCompare {
        op: crate::cir::CmpOp,
        left: VReg,
        right: Operand,
        signed: bool,
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
            BlockTerm::BranchCompare { left, right, .. } => {
                let mut read = vec![*left];
                read.extend(right.register());
                read
            }
        }
    }
}

/// One block in machine form.
#[derive(Debug)]
pub(crate) struct MachineBlock {
    /// What happens, in order.
    pub ops: Vec<Op>,
    /// Where in the source each of those came from, one per operation.
    ///
    /// A parallel vector rather than a span inside every `Op`: the alternative is wrapping sixty
    /// variants in `(Span, Op)` to protect one invariant, and the invariant is one assertion. The
    /// encoder turns these into a line table - the address of each operation and the line it was
    /// written on - which is what lets a fault say `sqlite3.c:130549` instead of `+0x129e2d`.
    pub spans: Vec<Span>,
    /// How it ends.
    pub term: BlockTerm,
}

/// One function in machine form, before allocation.
#[derive(Debug)]
pub(crate) struct MachineFunction {
    /// Its link name.
    pub name: String,
    /// Whether the name is visible outside this translation unit: what a `static` function is not.
    pub external: bool,
    /// Bytes of frame the lowering needed, block parameters included; spills are added after.
    pub frame_bytes: i32,
    /// How many argument *slots* the widest call in this function pushes on the stack: everything
    /// beyond the fourth, eight bytes each. The frame has to make room for them above the shadow
    /// space, because a callee reads its fifth argument at `[rsp + 40]` once its own frame is
    /// subtracted - the shadow space and the return address above it.
    pub outgoing_slots: u32,
    /// The register file each value belongs to, by virtual register. A value's class comes from its C
    /// type, not from the allocation, so the lowering records it and the allocator reads it.
    pub classes: Vec<Class>,
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
    /// Whether this function takes `...`: its prologue writes the register arguments into the
    /// caller's home area, because that is what a `va_list` walks - the four registers, then the stack
    /// arguments, contiguously (ADR-006).
    pub variadic: bool,
    /// The blocks, in layout order.
    pub blocks: Vec<MachineBlock>,
}

/// Lower a whole translation unit to a COFF object, or say what is not lowered.
pub(crate) fn object(
    module: &crate::cir::Module,
    names: &crate::intern::Interner,
) -> Result<Vec<u8>, NotYet> {
    Ok(coff::write_image(&image(module, names)?))
}

/// The same lowering, stopping at the image: a *linker* needs the sections, their symbols and their
/// relocations, and the object writer's job is to put them in a file it will not be linking.
pub(crate) fn image(
    module: &crate::cir::Module,
    names: &crate::intern::Interner,
) -> Result<coff::Image, NotYet> {
    let functions = lower::lower(module, names)?;
    let data = data_section(module)?;
    Ok(coff::assemble(&functions, data))
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
        let align = module
            .types
            .align_of(global.ty)
            .unwrap_or(1)
            .max(1)
            .max(global.alignas) as usize;
        data.align = data.align.max(align as u32);
        while data.bytes.len() % align != 0 {
            data.bytes.push(0);
        }
        data.symbols.push(coff::DataSymbol {
            name: global.name.clone(),
            offset: data.bytes.len() as u32,
            align: align as u32,
            size: module
                .types
                .size_of(global.ty)
                .unwrap_or(0)
                .max(global.init.as_ref().map_or(0, |init| init.len() as u64))
                as u32,
            thread: global.thread,
            external: global.external,
            // `int shared;` with no initialiser is a *tentative* definition, and one in two units is
            // still one object; `int shared = 1;` is a definition, and two of those are a duplicate.
            tentative: global.external && global.init.is_none(),
        });
        // The object's own start in the section, so the addresses inside it can be named from there.
        let start = data.bytes.len() as u32;
        for relocation in &global.relocations {
            data.relocations.push(coff::DataRelocation {
                offset: start + relocation.offset,
                symbol: relocation.symbol.clone(),
                addend: relocation.addend,
            });
        }
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
            right: Operand::Value(VReg(1)),
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

    /// The address-to-line table, held to the three things a map needs: every entry points inside
    /// the code, the offsets only ever go forward, and every line it names is a line of the source.
    ///
    /// It has no other reader yet - the image does not write them out - so this test is what keeps
    /// the collection honest until it does. A table nothing reads is a table that is wrong.
    #[test]
    fn every_operation_records_where_it_came_from() {
        use crate::options::Options;
        use crate::pp::preprocess;
        use crate::source::SourceFile;

        let source = "int f(int a, int b) {
  int c = a + b;
  c = c * 2;
  return c;
}
                      int main(void) { return f(3, 4); }
";
        let file = SourceFile::new("t.c", source);
        let options = Options::default();
        let mut preprocessed = preprocess(&file, &options, &());
        let parsed = crate::parse::parse(&mut preprocessed, &options.budgets);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let built = image(&parsed.module, preprocessed.names()).expect("it lowers");
        assert!(!built.lines.is_empty(), "no lines were collected at all");
        assert!(
            built.lines.len() >= 8,
            "two functions of a dozen operations produced {} lines",
            built.lines.len()
        );
        let mut previous = 0;
        for (at, span) in &built.lines {
            assert!(
                (*at as usize) < built.text.len(),
                "a line begins at {at}, past the end of {} bytes of code",
                built.text.len()
            );
            assert!(
                *at >= previous,
                "the offsets are not in order: {at} after {previous}"
            );
            previous = *at;
            assert!(
                (span.start as usize) + (span.len as usize) <= source.len(),
                "a span runs past the end of the source"
            );
            assert!(span.len > 0, "an empty span names no line");
        }
    }
}
