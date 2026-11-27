//! `cir` to this back end's machine form: one linear list of three-address operations per block,
//! with frame slots where a value must live in memory.
//!
//! Three decisions, and the reasoning, because neither is obvious from the code:
//!
//! - **A value in a register is canonical for its C type**: a signed `int` is sign-extended to 64
//!   bits, an unsigned one zero-extended. Every operation is therefore a 64-bit operation, and the
//!   only instructions that look at a width are the ones that touch memory or compare. This is what
//!   keeps `neg`, `not`, `shl` and the rest from needing a per-width case.
//! - **A block parameter lives in a frame slot**, and an edge into the block stores its arguments
//!   there before it jumps. That is the whole of the "phi" problem here: a slot has one name, so
//!   these stores never need to be formed into a parallel move.
//! - **The frame is addressed from `rsp` without ever moving it.** No pushes, no `rbp`: every
//!   slot has one offset for the whole function, which is why an offset in this file is relative
//!   to the frame body and the encoder adds the shadow space and the saved registers once.

use std::collections::{HashMap, HashSet};

use super::{
    BlockTerm, Class, MachineBlock, MachineFunction, NotYet, Op, Operand, VReg, ARGUMENT_COUNT,
};
use crate::cir::{BinOp, Block, ConvertOp, Func, Inst, Module, Term, Value};
use crate::diag::Span;
use crate::types::{Type, TypeId, Types};

/// Lower one translation unit. A function this slice cannot lower refuses with the reason and the
/// place, rather than producing something that runs differently from the same code at level 0.
pub(crate) fn lower(
    module: &Module,
    names: &crate::intern::Interner,
) -> Result<Vec<MachineFunction>, NotYet> {
    // Which names are thread-local, and how big each is: the two facts an address of one needs and
    // a `lea` cannot express. Built here because the module is what knows them, and passed down
    // because the lowering of a function is where the address is.
    let mut thread_locals: std::collections::HashMap<String, u32> =
        std::collections::HashMap::new();
    for global in &module.globals {
        if global.thread {
            let size = module.types.size_of(global.ty).unwrap_or(0) as u32;
            thread_locals.insert(global.name.clone(), size);
        }
    }
    let mut lowered = Vec::new();
    for func in &module.funcs {
        lowered.push(lower_function(func, &module.types, names, &thread_locals)?);
    }
    Ok(lowered)
}

fn lower_function(
    func: &Func,
    types: &Types,
    names: &crate::intern::Interner,
    thread_locals: &std::collections::HashMap<String, u32>,
) -> Result<MachineFunction, NotYet> {
    let mut lowerer = FunctionLowering {
        types,
        thread_locals,
        regs: vec![None; func.values().count()],
        next_reg: 0,
        blocks: Vec::new(),
        dynamic_stack: false,
        order: HashMap::new(),
        param_offsets: Vec::new(),
        frame_bytes: 0,
        returned: Vec::new(),
        classes: Vec::new(),
        direct_calls: HashMap::new(),
        reads: HashMap::new(),
        immediates: HashMap::new(),
        addresses: HashMap::new(),
    };
    lowerer.run(func)?;
    let makes_call = func.blocks.iter().any(|block| {
        block.insts.iter().any(|inst| {
            matches!(inst, Inst::Call { .. })
                // A thread-local address is a call too: the runtime owns the block. The allocator
                // has to know, or a value it left in `r8` is destroyed by the argument that goes
                // there - which is what a segmentation fault in a *worker thread* turned out to be.
                || matches!(inst, Inst::GlobalAddress { name, .. }
                    if thread_locals.contains_key(name))
                // A run-time stack allocation is a call too: the encoder asks `__chkstk` to probe
                // the pages it is about to commit, and a probe is a call like any other - a value the
                // allocator left in `r8`-`r11` is destroyed by it. That is the same mistake the
                // thread-local address above records, one layer down.
                || matches!(inst, Inst::StackAlloc { .. })
        })
    });
    // The widest call decides how much outgoing room the frame needs; a call with four arguments
    // needs none, because they all go in registers.
    let widest_call = func
        .blocks
        .iter()
        .flat_map(|block| &block.insts)
        .filter_map(|inst| match inst {
            Inst::Call { args, .. } => Some(args.len()),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    let outgoing_slots = widest_call.saturating_sub(ARGUMENT_COUNT);
    Ok(MachineFunction {
        name: names.resolve(func.name).to_string(),
        external: func.external,
        frame_bytes: lowerer.frame_bytes,
        variadic: func.variadic,
        classes: lowerer.classes,
        returned: lowerer.returned,
        outgoing_slots: outgoing_slots as u32,
        param_count: func.params().len(),
        makes_call,
        blocks: lowerer.blocks,
        dynamic_stack: lowerer.dynamic_stack,
    })
}

/// An address the lowering can make once and reuse.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Address {
    /// The address of a named object or function.
    Symbol(String),
    /// The address of a frame slot, by its offset: a slot's offset is unique in the frame, and nothing
    /// moves the frame.
    Local(i32),
}

/// A comparison that a branch carries instead of a value: the instruction it replaces, and what the
/// terminator needs to make the same decision.
struct FusedCompare {
    /// The instruction to leave out of the op list.
    index: usize,
    op: crate::cir::CmpOp,
    left: Value,
    right: Value,
}

/// The values of one function, the frame slots they use, and the blocks being filled.
struct FunctionLowering<'a> {
    types: &'a Types,
    /// The thread-local objects of this module, by link name, with their sizes.
    thread_locals: &'a std::collections::HashMap<String, u32>,
    /// A machine register per `cir` value, mints on first use.
    regs: Vec<Option<VReg>>,
    next_reg: u32,
    /// Whether anything here allocates at run time, which the encoder turns into a frame pointer.
    dynamic_stack: bool,
    blocks: Vec<MachineBlock>,
    /// Where each `cir` block went in `blocks`.
    order: HashMap<Block, usize>,
    /// Where each block's parameters start in the frame.
    param_offsets: Vec<i32>,
    /// Bytes of frame this function uses, block parameters included.
    frame_bytes: i32,
    /// The values a `return` hands back.
    returned: Vec<VReg>,
    /// Which register file each value belongs to, by virtual register.
    classes: Vec<Class>,
    /// Values that are the function a call names directly, so no address is ever materialised for
    /// them: `call rel32` needs a symbol, not a pointer.
    direct_calls: HashMap<Value, String>,
    /// How many times each value is read, instructions and terminators together, as `cir` wrote it:
    /// a comparison read once is a branch that can compare for itself, a constant read once is an
    /// immediate, and either read twice is a value that has to exist.
    reads: HashMap<Value, usize>,
    /// The addresses this *block* has already made, so the same one is made once. **Nothing can
    /// invalidate an address**: a symbol's address is the same for the whole function, and so is a
    /// frame slot's - what changes is the bytes *at* it.
    ///
    /// Per block, not per function, and the measurement is why: sharing the register across a whole
    /// function removes 232 `lea`s and then pays back 1 200 bytes in spills, because the register now
    /// lives from the first mention to the last. A block is short enough that the reuse is close to
    /// free, and a repeated address inside one block is the common case - `p->a` and `p->b` off the
    /// same base, a global read twice in a row.
    addresses: HashMap<Address, VReg>,
    /// The constants that never materialise: their value travels in the instruction that reads them.
    /// A constant read once, by an operation whose encoding has an immediate field, needs neither a
    /// register nor a `mov`, and a third of the back end's bytes went into that `mov`.
    immediates: HashMap<Value, u64>,
}

impl FunctionLowering<'_> {
    fn run(&mut self, func: &Func) -> Result<(), NotYet> {
        // A call whose callee is the address of a global function is a direct call, and the address
        // itself is then left out of the machine form - which is sound only when *every* read of it
        // is a call naming it. A value read anywhere else needs an address this back end has not
        // learnt to make, so it is refused rather than quietly made zero.
        //
        // The counts themselves are kept as they are, and the removal below works on a copy: two
        // later decisions ask "is this read exactly once", and a count that has been edited to make
        // one decision come out right is a count the next one cannot trust.
        self.reads = self.read_counts(func);
        let mut reads = self.reads.clone();
        let mut callee_reads: HashMap<Value, usize> = HashMap::new();
        for block in &func.blocks {
            for inst in &block.insts {
                if let Inst::Call { callee, .. } = inst {
                    *callee_reads.entry(*callee).or_default() += 1;
                }
            }
        }
        for (callee, count) in callee_reads {
            if reads.get(&callee).copied().unwrap_or(0) != count {
                continue; // read by something besides the call
            }
            if let Some(Inst::GlobalAddress { name, .. }) = self.defining(func, callee) {
                self.direct_calls.insert(callee, name.clone());
                reads.remove(&callee);
            }
        }
        self.immediates = self.carried_constants(func);
        // The layout: blocks after the ones that dominate them, so a branch's target is in the
        // future about as often as it is in the past. Parameters first in the frame, locals after.
        for (index, block) in super::super::dominator_order(func).into_iter().enumerate() {
            self.order.insert(block, index);
        }
        self.param_offsets = Vec::with_capacity(self.order.len());
        let mut offset = 0;
        for (block, _) in self.ordered_blocks() {
            self.param_offsets.push(offset);
            // The entry block's parameters came from registers, and nothing can jump to the entry
            // block, so its slots would be written by no edge and read by no instruction.
            if block != func.entry {
                offset += 8 * func.block(block).params.len() as i32;
            }
        }
        self.frame_bytes = offset;

        // The entry block's parameters are the arguments.
        let entry = func.entry;
        let mut ops = Vec::new();
        // The first four *positions* take a register each: a fifth argument goes on the stack,
        // whatever its class.
        for (position, param) in func.params().iter().enumerate() {
            let ty = func.value(*param).ty;
            let (size, signed, class) = self.scalar(ty, func.entry_span())?;
            // The position picks the register and the class picks the file - the same rule the
            // caller writes, because it is the same ABI: `f(int, double)` has its `double` in
            // `xmm1`, beside the `int` in `rcx`.
            let (slot, on_stack) = if position < ARGUMENT_COUNT {
                (position as u8, false)
            } else {
                ((position - ARGUMENT_COUNT) as u8, true)
            };
            let dst = self.reg_for(*param);
            self.classify(dst, class);
            ops.push(Op::Incoming {
                dst,
                slot,
                class,
                on_stack,
                size,
                signed,
            });
        }
        for (block, _) in self.ordered_blocks() {
            let mut block_ops = if block == entry {
                std::mem::take(&mut ops)
            } else {
                Vec::new()
            };
            // The entry block arrives with its arguments already moved into slots, and those ops
            // were pushed before this loop: their line is the function's, not the first
            // instruction's. `resize` below would have said otherwise.
            let mut block_spans: Vec<Span> = vec![func.block(block).span; block_ops.len()];
            let position = self.order[&block];
            // A block's parameters arrive in its slots: reading them here is the "phi". The entry
            // block is the exception - its parameters arrived in registers, and it has already read
            // them.
            if block != entry {
                for (index, param) in func.block(block).params.iter().enumerate() {
                    let offset = self.param_offsets[position] + 8 * index as i32;
                    let dst = self.reg_for(*param);
                    // A parameter's register file is its type's, like any other value's: the slot
                    // holds eight bytes and only the reader knows what they mean.
                    let ty = func.value(*param).ty;
                    let (_, _, class) = self.scalar(ty, func.block(block).span)?;
                    self.classify(dst, class);
                    block_ops.push(Op::LoadLocal { dst, offset });
                }
            }
            // A new block, so the addresses the last one made are not in scope for this one's reuse.
            self.addresses.clear();
            // A comparison whose only reader is this block's branch never has to exist as a value:
            // the branch can compare and jump. That is 45% of the comparisons in the corpus, and it
            // saves the `setcc`, the widening that follows it, and the test the branch would do.
            let fused = self.fusable_compare(func, block);
            for (index, inst) in func.block(block).insts.iter().enumerate() {
                if Some(index) == fused.as_ref().map(|fused| fused.index) {
                    continue;
                }
                // A constant the instruction after it carries needs no register and no `mov`.
                if let Inst::Const { result, .. } = inst {
                    if self.immediates.contains_key(result) {
                        continue;
                    }
                }
                self.lower_inst(func, inst, &mut block_ops)?;
                // Every op this instruction emitted is on this instruction's line. One instruction
                // can be several ops - a load, a shift, a store - and all of them are the source
                // line the program wrote.
                block_spans.resize(block_ops.len(), inst.span());
            }
            let term = self.lower_term(func, block, &mut block_ops, fused)?;
            block_spans.resize(block_ops.len(), func.block(block).span);
            debug_assert_eq!(block_ops.len(), block_spans.len());
            self.blocks.push(MachineBlock {
                ops: block_ops,
                spans: block_spans,
                term,
            });
        }
        Ok(())
    }

    /// The blocks in layout order, with their handles.
    fn ordered_blocks(&self) -> Vec<(Block, usize)> {
        let mut pairs: Vec<(Block, usize)> = self.order.iter().map(|(b, i)| (*b, *i)).collect();
        pairs.sort_by_key(|(_, index)| *index);
        pairs
    }

    /// The constants that can travel inside the instruction that reads them.
    ///
    /// Three conditions, and each rules out a way of being wrong: the constant is read exactly once,
    /// so nothing else can want its register; that reader is an operation whose encoding *has* an
    /// immediate field - `add`, `sub`, `and`, `or`, `xor` and a comparison - and it reads the constant
    /// on the right, where x86 puts the field; and the value survives being written there, which for
    /// every one of those forms means fitting a signed 32-bit immediate, since that is what the
    /// instruction sign-extends. `mul` and the shifts are left out: their encodings take a register.
    fn carried_constants(&self, func: &Func) -> HashMap<Value, u64> {
        // One pass to find where an immediate field can hold a constant, one to collect the
        // constants: asking "does anything read this one there" inside the loop over constants would
        // be quadratic in the size of the function, and the last quadratic scan in this back end cost
        // a review of its own.
        let mut carryable: HashSet<Value> = HashSet::new();
        for block in &func.blocks {
            for inst in &block.insts {
                match inst {
                    // `Inst::Binary`'s operation is the *owning instruction's* question: the encoder
                    // answers which operations have an immediate field, and this asks it rather than
                    // keeping a second list to drift.
                    Inst::Binary { op, right, .. }
                        if super::encode::immediate_extension(*op).is_some() =>
                    {
                        carryable.insert(*right);
                    }
                    Inst::Compare { right, .. } => {
                        carryable.insert(*right);
                    }
                    _ => {}
                }
            }
        }
        let mut carried = HashMap::new();
        for block in &func.blocks {
            for inst in &block.insts {
                let Inst::Const {
                    result, ty, bits, ..
                } = inst
                else {
                    continue;
                };
                if self.reads.get(result).copied().unwrap_or(0) != 1
                    || self.types.is_float(*ty)
                    || i32::try_from(*bits as i64).is_err()
                    || !carryable.contains(result)
                {
                    continue;
                }
                carried.insert(*result, *bits);
            }
        }
        carried
    }

    /// The register a value has, making the address if this *block* has not made it already. Two
    /// `cir` values that name the same address name the same register, which is what a second `lea`
    /// would have computed anyway - and the block is the scope because a register shared further than
    /// that lives long enough to be spilled.
    fn address_of(&mut self, what: Address, result: Value, ops: &mut Vec<Op>) -> VReg {
        if let Some(register) = self.addresses.get(&what) {
            self.regs[result.0 as usize] = Some(*register);
            return *register;
        }
        let dst = self.reg_for(result);
        let op = match &what {
            // A thread-local object has no address to `lea`: the runtime owns the block and only it
            // knows where this thread's copy begins.
            Address::Symbol(symbol) => match self.thread_locals.get(symbol) {
                Some(_) => Op::ThreadAddress {
                    dst,
                    symbol: symbol.clone(),
                },
                None => Op::SymbolAddress {
                    dst,
                    symbol: symbol.clone(),
                },
            },

            Address::Local(offset) => Op::LocalAddress {
                dst,
                offset: *offset,
            },
        };
        ops.push(op);
        self.addresses.insert(what, dst);
        dst
    }

    /// How the operation reads a value: from a register, or as the constant it was made from.
    fn operand(&mut self, value: Value) -> Operand {
        match self.immediates.get(&value) {
            Some(bits) => Operand::Immediate(*bits),
            None => Operand::Value(self.reg_for(value)),
        }
    }

    /// How many times each value is read, instructions and terminators together.
    fn read_counts(&self, func: &Func) -> HashMap<Value, usize> {
        let mut counts: HashMap<Value, usize> = HashMap::new();
        let count = |value: Value, counts: &mut HashMap<Value, usize>| {
            *counts.entry(value).or_default() += 1;
        };
        for block in &func.blocks {
            for inst in &block.insts {
                for operand in inst.operands() {
                    count(operand, &mut counts);
                }
            }
            if let Some(term) = &block.term {
                for operand in crate::cir::term_operands(term) {
                    count(operand, &mut counts);
                }
            }
        }
        counts
    }

    /// The instruction that defines `value`, if this function defines it.
    fn defining<'f>(&self, func: &'f Func, value: Value) -> Option<&'f Inst> {
        let block = func.value(value).block;
        func.block(block)
            .insts
            .iter()
            .find(|inst| inst.result() == Some(value))
    }

    fn reg_for(&mut self, value: Value) -> VReg {
        let index = value.0 as usize;
        if let Some(Some(reg)) = self.regs.get(index) {
            return *reg;
        }
        let reg = VReg(self.next_reg);
        self.next_reg += 1;
        self.regs[index] = Some(reg);
        reg
    }

    fn fresh(&mut self) -> VReg {
        let reg = VReg(self.next_reg);
        self.next_reg += 1;
        reg
    }

    /// A C type's width in bytes and whether it is signed, for a type that fits in a register.
    fn width(&self, ty: TypeId, span: crate::diag::Span) -> Result<(u8, bool), NotYet> {
        match self.types.kind(ty) {
            Type::Int { signed, bits } => Ok((bits / 8, *signed)),
            Type::Bool => Ok((1, false)),
            Type::Pointer(_) => Ok((8, false)),
            Type::Void => Err(NotYet {
                message: "a value of type void".into(),
                span,
            }),
            Type::Float { .. } => Err(NotYet {
                message: "a floating-point value where an integer one is needed".into(),
                span,
            }),
            _other => Err(NotYet {
                message: format!("a value of type {}", self.types.describe(ty)),
                span,
            }),
        }
    }

    /// An address, as the base a memory operand needs and the displacement it can carry. The front
    /// end writes `base + k` for every element of an array, and x86 has a place for the `k`: folding it
    /// out of the instruction stream is what turns a byte of an initialiser from a multiply, an add
    /// and three bitcasts into one store.
    fn address(&mut self, func: &Func, address: Value, _ops: &mut Vec<Op>) -> (Value, i32) {
        // The front end wraps the arithmetic in bitcasts - `pointer -> u64 -> pointer -> pointer` -
        // and every one of them is the same bits. Peeling them is what lets the addition be seen.
        let mut peeled = address;
        for _ in 0..8 {
            match self.defining(func, peeled) {
                Some(Inst::Convert {
                    op: ConvertOp::Bitcast,
                    value,
                    ..
                }) => peeled = *value,
                _ => break,
            }
        }
        if let Some(Inst::Binary {
            op: BinOp::Add,
            left,
            right,
            ..
        }) = self.defining(func, peeled)
        {
            // `base + k`, or `k + base`.
            for (pointer, offset) in [(*left, *right), (*right, *left)] {
                if let Some(Inst::Const { bits, .. }) = self.defining(func, offset) {
                    if let Ok(offset) = i32::try_from(*bits as i64) {
                        return (pointer, offset);
                    }
                }
            }
        }
        (address, 0)
    }

    /// Which register file a type belongs to.
    fn class_of(&self, ty: TypeId) -> Class {
        if self.types.is_float(ty) {
            Class::Float
        } else {
            Class::Integer
        }
    }

    /// A floating type's width in bytes.
    fn float_size(&self, ty: TypeId) -> u8 {
        match self.types.kind(ty) {
            Type::Float { bits } => bits / 8,
            _ => unreachable!("a float width asked of a type that is not one"),
        }
    }

    /// Record which register file a value belongs to, from its C type.
    fn classify(&mut self, value: VReg, class: Class) {
        let index = value.0 as usize;
        if self.classes.len() <= index {
            self.classes.resize(index + 1, Class::Integer);
        }
        self.classes[index] = class;
    }

    /// The width, sign and register file of a scalar, which is everything an operation needs to know
    /// about the type it works on.
    fn scalar(&self, ty: TypeId, span: crate::diag::Span) -> Result<(u8, bool, Class), NotYet> {
        match self.types.kind(ty) {
            Type::Float { bits } => Ok((bits / 8, false, Class::Float)),
            _ => {
                let (size, signed) = self.width(ty, span)?;
                Ok((size, signed, Class::Integer))
            }
        }
    }

    /// Reserve `size` bytes of frame at `align`, giving an offset from the frame body.
    fn allocate(&mut self, size: u64, align: u64) -> i32 {
        let align = align.max(1) as i32;
        let size = (size.max(1)) as i32;
        self.frame_bytes = (self.frame_bytes + align - 1) / align * align;
        let offset = self.frame_bytes;
        self.frame_bytes += size;
        offset
    }

    fn lower_inst(&mut self, func: &Func, inst: &Inst, ops: &mut Vec<Op>) -> Result<(), NotYet> {
        let span = inst.span();
        match inst {
            Inst::SegmentRead {
                result,
                segment,
                address,
                ty,
                ..
            } => {
                let (size, signed, class) = self.scalar(*ty, span)?;
                if class != Class::Integer || !matches!(size, 1 | 2 | 4 | 8) {
                    return Err(NotYet {
                        message: "a segment read of something that is not an integer of 1, 2, 4 or 8                                   bytes"
                            .into(),
                        span,
                    });
                }
                let dst = self.reg_for(*result);
                self.classify(dst, Class::Integer);
                // The offset is an address in the segment, so it lowers like one: a register, and
                // whatever constant the front end folded into it as a displacement - which is how
                // `gs:[0x30]` stays one instruction.
                let (address, offset) = self.address(func, *address, ops);
                let base = self.reg_for(address);
                ops.push(Op::SegmentRead {
                    dst,
                    segment: *segment,
                    base,
                    offset,
                    size,
                });
                if size < 4 {
                    // The read leaves the rest of `rax` as it was, exactly as `cmpxchg` does.
                    ops.push(Op::Canonical {
                        dst,
                        src: dst,
                        size,
                        signed,
                    });
                }
                return Ok(());
            }
            Inst::AtomicCas {
                result,
                pointer,
                expected,
                desired,
                ty,
                ..
            } => {
                // An integer of 1, 2, 4 or 8 bytes: `cmpxchg` has a form for each, and `_Atomic` of
                // a float is not something this compiler accepts, because a float swap is not one
                // instruction.
                let (size, signed, class) = self.scalar(*ty, span)?;
                if class != Class::Integer || !matches!(size, 1 | 2 | 4 | 8) {
                    return Err(NotYet {
                        message: "an atomic operation on this type: it needs an integer of 1, 2, 4                                   or 8 bytes"
                            .into(),
                        span,
                    });
                }
                let dst = self.reg_for(*result);
                self.classify(dst, Class::Integer);
                let pointer = Operand::Value(self.reg_for(*pointer));
                let expected = self.operand(*expected);
                let desired = self.operand(*desired);
                ops.push(Op::AtomicCas {
                    dst,
                    pointer,
                    expected,
                    desired,
                    size,
                });
                // `cmpxchg` writes `al` or `ax` and leaves the rest of `rax` alone, so a value of one
                // or two bytes is not canonical until it is extended - and every later operation
                // assumes the canonical form. Four and eight bytes arrive whole.
                if size < 4 {
                    ops.push(Op::Canonical {
                        dst,
                        src: dst,
                        size,
                        signed,
                    });
                }
            }
            Inst::Const {
                result, ty, bits, ..
            } => {
                let (size, signed, class) = self.scalar(*ty, span)?;
                let dst = self.reg_for(*result);
                self.classify(dst, class);
                ops.push(match class {
                    // A floating constant travels as its bit pattern; where those bits go is the
                    // encoder's business.
                    Class::Float => Op::FloatConst {
                        dst,
                        bits: *bits,
                        size,
                    },
                    Class::Integer => Op::Const {
                        dst,
                        value: canonical(*bits, size, signed),
                    },
                });
            }
            Inst::Binary {
                result,
                op,
                ty,
                left,
                right,
                ..
            } => {
                let (size, signed, class) = self.scalar(*ty, span)?;
                let dst = self.reg_for(*result);
                self.classify(dst, class);
                let left = self.reg_for(*left);
                if class == Class::Float {
                    let right = self.reg_for(*right);
                    if !matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div) {
                        return Err(NotYet {
                            message: "a bitwise operation on a floating-point value".into(),
                            span,
                        });
                    }
                    ops.push(Op::FloatBinary {
                        dst,
                        op: *op,
                        left,
                        right,
                        size,
                    });
                    return Ok(());
                }
                ops.push(match op {
                    BinOp::Shl | BinOp::Shr => Op::Shift {
                        dst,
                        left,
                        right: self.reg_for(*right),
                        right_signed: signed,
                        left_shift: *op == BinOp::Shl,
                        size,
                        signed,
                    },
                    BinOp::Div | BinOp::Rem => Op::Divide {
                        dst,
                        left,
                        right: self.reg_for(*right),
                        remainder: *op == BinOp::Rem,
                        signed,
                        size,
                    },
                    _ => Op::Binary {
                        dst,
                        op: *op,
                        left,
                        right: self.operand(*right),
                        size,
                        signed,
                    },
                });
            }
            Inst::Unary {
                result,
                op,
                ty,
                value,
                ..
            } => {
                let (size, signed, class) = self.scalar(*ty, span)?;
                let dst = self.reg_for(*result);
                self.classify(dst, class);
                let src = self.reg_for(*value);
                if class == Class::Float {
                    if *op != crate::cir::UnOp::Neg {
                        return Err(NotYet {
                            message: "`~` on a floating-point value".into(),
                            span,
                        });
                    }
                    // `-x` is `0 - x`: a zero arrives as the bits of one, so no constant of its own
                    // is needed in the data section.
                    let zero = self.fresh();
                    self.classify(zero, Class::Float);
                    ops.push(Op::FloatConst {
                        dst: zero,
                        bits: 0,
                        size,
                    });
                    ops.push(Op::FloatBinary {
                        dst,
                        op: BinOp::Sub,
                        left: zero,
                        right: src,
                        size,
                    });
                    return Ok(());
                }
                ops.push(match op {
                    crate::cir::UnOp::Neg => Op::Unary {
                        dst,
                        src,
                        size,
                        signed,
                        negate: true,
                    },
                    crate::cir::UnOp::Not => Op::Unary {
                        dst,
                        src,
                        size,
                        signed,
                        negate: false,
                    },
                });
            }
            Inst::Compare {
                result,
                op,
                ty,
                left,
                right,
                ..
            } => {
                let (size, signed, class) = self.scalar(*ty, span)?;
                let dst = self.reg_for(*result);
                // A comparison answers in an `int`, whatever its operands were.
                self.classify(dst, Class::Integer);
                let left = self.reg_for(*left);
                ops.push(match class {
                    Class::Float => Op::FloatCompare {
                        dst,
                        op: *op,
                        left,
                        right: self.reg_for(*right),
                        size,
                    },
                    Class::Integer => Op::Compare {
                        dst,
                        op: *op,
                        left,
                        right: self.operand(*right),
                        signed,
                    },
                });
            }
            Inst::Convert {
                result,
                op,
                from,
                to,
                value,
                ..
            } => {
                let (from_size, from_signed, from_class) = self.scalar(*from, span)?;
                let (to_size, to_signed, to_class) = self.scalar(*to, span)?;
                let floats = matches!(
                    op,
                    ConvertOp::IntToFloat
                        | ConvertOp::FloatToInt
                        | ConvertOp::FloatExtend
                        | ConvertOp::FloatTruncate
                );
                if floats != (from_class == Class::Float || to_class == Class::Float) {
                    return Err(NotYet {
                        message: format!(
                            "a conversion the front end named `{}` between {} and {}",
                            op.as_str(),
                            self.types.describe(*from),
                            self.types.describe(*to)
                        ),
                        span,
                    });
                }
                // Widening to sixty-four bits of type is free, and is the largest single class of
                // conversion in the corpus: 4 239 of 4 427. A value in a register is canonical for its
                // own C type already, and every type of size eight *is* its own sixty-four bits, so
                // `(long)x` is `x` - sign-extended for `int`, zero-extended for `unsigned`, and
                // neither needed here because the register holds that form. Narrowing is the only
                // conversion with work to do.
                //
                // **Both files have to be the integer one.** `(long long)1.5` is also a conversion to
                // a type of size eight, and it is `cvttsd2si`: the value in the register is a `double`
                // and reading it as an integer is not a widening, it is a reinterpretation. Testing
                // only the destination let that case take this shortcut and panic where the double
                // met an integer reader - found by an audit of the review, not by the corpus.
                if from_class == Class::Integer && to_class == Class::Integer && to_size == 8 {
                    // The two `cir` values are one machine value - and it is already in the integer
                    // file, and already canonical, so there is nothing to classify either. Saying they
                    // are one value is cheaper than an instruction that copies a register to itself.
                    let src = self.reg_for(*value);
                    self.regs[result.0 as usize] = Some(src);
                    return Ok(());
                }
                let dst = self.reg_for(*result);
                self.classify(dst, to_class);
                let src = self.reg_for(*value);
                ops.push(match (from_class, to_class) {
                    (Class::Float, Class::Float) | (Class::Integer, Class::Float) => {
                        Op::FloatConvert {
                            dst,
                            src,
                            from: from_class,
                            to: to_class,
                            from_size,
                            to_size,
                            signed: from_signed,
                        }
                    }
                    (Class::Float, Class::Integer) => Op::FloatConvert {
                        dst,
                        src,
                        from: from_class,
                        to: to_class,
                        from_size,
                        to_size,
                        signed: to_signed,
                    },
                    (Class::Integer, Class::Integer) => Op::Canonical {
                        dst,
                        src,
                        size: to_size,
                        signed: to_signed,
                    },
                });
            }
            Inst::Load {
                result,
                ty,
                address,
                ..
            } => {
                let (size, signed, class) = self.scalar(*ty, span)?;
                let dst = self.reg_for(*result);
                self.classify(dst, class);
                let (address, offset) = self.address(func, *address, ops);
                let base = self.reg_for(address);
                ops.push(match class {
                    Class::Float => Op::LoadFloat {
                        dst,
                        base,
                        offset,
                        size,
                    },
                    Class::Integer => Op::LoadMemory {
                        dst,
                        base,
                        offset,
                        size,
                        signed,
                    },
                });
            }
            Inst::Store { address, value, .. } => {
                let ty = func.value(*value).ty;
                let (size, _, class) = self.scalar(ty, span)?;
                let (address, offset) = self.address(func, *address, ops);
                let base = self.reg_for(address);
                let src = self.reg_for(*value);
                ops.push(match class {
                    Class::Float => Op::StoreFloat {
                        base,
                        offset,
                        src,
                        size,
                    },
                    Class::Integer => Op::StoreMemory {
                        base,
                        offset,
                        src,
                        size,
                    },
                });
            }
            Inst::Slot {
                result,
                ty,
                alignas,
                ..
            } => {
                let size = self.types.size_of(*ty).ok_or_else(|| NotYet {
                    message: format!("a local of incomplete type {}", self.types.describe(*ty)),
                    span,
                })?;
                // `_Alignas` raises an object's alignment and never lowers it (C11 6.7.5p6).
                let align = self.types.align_of(*ty).unwrap_or(1).max(*alignas);
                let offset = self.allocate(size, align);
                let dst = self.address_of(Address::Local(offset), *result, ops);
                self.classify(dst, Class::Integer);
            }
            Inst::StackMark { result, .. } => {
                let dst = self.reg_for(*result);
                self.classify(dst, Class::Integer);
                ops.push(Op::StackMark { dst });
                self.dynamic_stack = true;
            }
            Inst::StackRestore { mark, .. } => {
                ops.push(Op::StackRestore {
                    mark: self.reg_for(*mark),
                });
            }
            Inst::StackAlloc { result, size, .. } => {
                let dst = self.reg_for(*result);
                self.classify(dst, Class::Integer);
                ops.push(Op::StackAlloc {
                    dst,
                    size: self.reg_for(*size),
                });
                // The frame is addressed from `rbp` from here on: everything else in this function
                // keeps its offset, but `rsp` is no longer the base they are measured from.
                self.dynamic_stack = true;
            }
            Inst::VaStart { result, skip, .. } => {
                let dst = self.reg_for(*result);
                self.classify(dst, Class::Integer);
                ops.push(Op::VaStart { dst, skip: *skip });
            }
            Inst::VaArg {
                result, list, ty, ..
            } => {
                let (size, signed, class) = self.scalar(*ty, span)?;
                let dst = self.reg_for(*result);
                self.classify(dst, class);
                ops.push(Op::VaArg {
                    dst,
                    list: self.reg_for(*list),
                    size,
                    signed,
                    class,
                });
            }
            Inst::GlobalAddress { result, name, .. } => {
                if self.direct_calls.contains_key(result) {
                    // Read only as the callee of a call: the call names the symbol and no address
                    // is ever materialised, so this value is defined by being left out. Nothing
                    // reads it, so it never needs a register or a slot.
                } else {
                    let dst = self.address_of(Address::Symbol(name.clone()), *result, ops);
                    self.classify(dst, Class::Integer);
                }
            }
            Inst::Call {
                result,
                callee,
                args,
                variadic,
                ..
            } => {
                let arguments = args.clone();
                let args: Vec<VReg> = args.iter().map(|arg| self.reg_for(*arg)).collect();
                for (register, argument) in args.iter().zip(arguments.iter()) {
                    let class = self.class_of(func.value(*argument).ty);
                    self.classify(*register, class);
                }
                let dst = result.map(|result| {
                    let register = self.reg_for(result);
                    let class = self.class_of(func.value(result).ty);
                    self.classify(register, class);
                    register
                });
                let (size, signed) = match result {
                    Some(result) => {
                        let ty = func.value(*result).ty;
                        if self.class_of(ty) == Class::Float {
                            (self.float_size(ty), false)
                        } else {
                            self.width(ty, span)?
                        }
                    }
                    // A call that returns nothing has neither, and the encoder never asks.
                    None => (0, false),
                };
                // A callee whose only use is this call is a direct call: `call rel32` names the
                // symbol and no address is materialised. Anything else - a pointer held in a
                // variable, passed in, read from a table - is called through a register.
                match self.direct_calls.get(callee).cloned() {
                    Some(name) => ops.push(Op::Call {
                        dst,
                        name,
                        args,
                        size,
                        signed,
                        variadic: *variadic,
                    }),
                    None => ops.push(Op::CallIndirect {
                        dst,
                        callee: self.reg_for(*callee),
                        args,
                        size,
                        signed,
                        variadic: *variadic,
                    }),
                }
            }
        }
        Ok(())
    }

    /// The comparison this block's branch can carry, if it can.
    ///
    /// Two conditions, and both are checked here rather than half here and half at the terminator:
    /// **nothing but this branch reads the comparison** - one that is also stored or returned has to
    /// exist as a value anyway, and a fused copy would compute it twice - and it compares integers.
    /// A floating-point one is left alone: `ucomisd` settles its unordered cases with condition codes
    /// `setcc` and the branch do not share.
    ///
    /// Returning the instruction *and* the decision to skip it is the point: the caller leaves that
    /// instruction out of the op list only when this answers, so the skip and the fusion cannot come
    /// apart. They could when the class was tested after the skip, and the reason it was harmless is
    /// that the front end wraps every condition in `!= 0` - a wrapper a pass could one day fold away,
    /// which would have turned this into a branch on a value nothing defines.
    fn fusable_compare(&self, func: &Func, block: Block) -> Option<FusedCompare> {
        let Some(Term::Branch { condition, .. }) = &func.block(block).term else {
            return None;
        };
        if self.reads.get(condition).copied().unwrap_or(0) != 1 {
            return None;
        }
        // Any of this block's instructions, not just the last one: a `cir` value is defined once and
        // never written again, so nothing between the comparison and the branch can change what it
        // reads.
        let insts = &func.block(block).insts;
        let (index, inst) = insts.iter().enumerate().find(
            |(_, inst)| matches!(inst, Inst::Compare { result, .. } if result == condition),
        )?;
        let Inst::Compare {
            op,
            ty,
            left,
            right,
            ..
        } = inst
        else {
            unreachable!("the search matched a comparison")
        };
        if self.class_of(*ty) == Class::Float {
            return None;
        }
        Some(FusedCompare {
            index,
            op: *op,
            left: *left,
            right: *right,
        })
    }

    fn lower_term(
        &mut self,
        func: &Func,
        block: Block,
        ops: &mut Vec<Op>,
        fused: Option<FusedCompare>,
    ) -> Result<BlockTerm, NotYet> {
        let term = func
            .block(block)
            .term
            .clone()
            .expect("the parser ends every block");
        Ok(match term {
            Term::Return { value, .. } => match value {
                Some(value) => {
                    let register = self.reg_for(value);
                    let float = self.types.is_float(func.value(value).ty);
                    self.classify(register, self.class_of(func.value(value).ty));
                    // Only an integer return is a candidate for the `rax` preference: a float comes
                    // back in `xmm0`.
                    if !float {
                        self.returned.push(register);
                    }
                    BlockTerm::Return(Some(register))
                }
                None => BlockTerm::Return(None),
            },
            Term::Jump { target, args, .. } => {
                self.store_parameters(func, target, &args, ops);
                BlockTerm::Jump(self.order[&target])
            }
            Term::Branch {
                condition,
                then,
                otherwise,
                ..
            } => {
                self.store_parameters(func, then.0, &then.1, ops);
                self.store_parameters(func, otherwise.0, &otherwise.1, ops);
                let then = self.order[&then.0];
                let otherwise = self.order[&otherwise.0];
                if let Some(fused) = fused {
                    let ty = func.value(fused.left).ty;
                    let (_, signed, _) = self.scalar(ty, func.block(block).span)?;
                    return Ok(BlockTerm::BranchCompare {
                        op: fused.op,
                        left: self.reg_for(fused.left),
                        right: self.operand(fused.right),
                        signed,
                        then,
                        otherwise,
                    });
                }
                BlockTerm::Branch {
                    condition: self.reg_for(condition),
                    then,
                    otherwise,
                }
            }
            Term::Unreachable { .. } | Term::Trap { .. } => BlockTerm::Unreachable,
        })
    }

    fn store_parameters(&mut self, _func: &Func, target: Block, args: &[Value], ops: &mut Vec<Op>) {
        let position = self.order[&target];
        for (index, arg) in args.iter().enumerate() {
            let offset = self.param_offsets[position] + 8 * index as i32;
            let src = self.reg_for(*arg);
            ops.push(Op::StoreLocal { offset, src });
        }
    }
}

/// A constant as a canonical 64-bit value: sign-extended when its type is signed.
fn canonical(bits: u64, size: u8, signed: bool) -> u64 {
    let shift = 64 - u32::from(size) * 8;
    if !signed {
        return bits & mask(size);
    }
    // Sign-extend from the type's top bit.
    (((bits & mask(size)) << shift) as i64 >> shift) as u64
}

/// The low `size` bytes of a value.
pub(crate) fn mask(size: u8) -> u64 {
    if size >= 8 {
        u64::MAX
    } else {
        (1u64 << (u32::from(size) * 8)) - 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_constant_is_canonical_for_its_type() {
        // An `int` holding 0xFFFFFFFF is -1, so it is stored as all ones in 64 bits.
        assert_eq!(canonical(0xFFFF_FFFF, 4, true), u64::MAX);
        assert_eq!(canonical(0xFFFF_FFFF, 4, false), 0xFFFF_FFFF);
        assert_eq!(canonical(0xFF, 1, true), u64::MAX);
        assert_eq!(canonical(0x80, 1, true), 0xFFFF_FFFF_FFFF_FF80);
        assert_eq!(canonical(0x80, 1, false), 0x80);
        assert_eq!(canonical(7, 8, true), 7);
    }
}
