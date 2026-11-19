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

use std::collections::HashMap;

use super::{BlockTerm, Class, MachineBlock, MachineFunction, NotYet, Op, VReg, ARGUMENT_COUNT};
use crate::cir::{BinOp, Block, ConvertOp, Func, Inst, Module, Term, Value};
use crate::types::{Type, TypeId, Types};

/// Lower one translation unit. A function this slice cannot lower refuses with the reason and the
/// place, rather than producing something that runs differently from the same code at level 0.
pub(crate) fn lower(
    module: &Module,
    names: &crate::intern::Interner,
) -> Result<Vec<MachineFunction>, NotYet> {
    let mut lowered = Vec::new();
    for func in &module.funcs {
        lowered.push(lower_function(func, &module.types, names)?);
    }
    Ok(lowered)
}

fn lower_function(
    func: &Func,
    types: &Types,
    names: &crate::intern::Interner,
) -> Result<MachineFunction, NotYet> {
    let mut lowerer = FunctionLowering {
        types,
        names,
        regs: vec![None; func.values().count()],
        next_reg: 0,
        blocks: Vec::new(),
        order: HashMap::new(),
        param_offsets: Vec::new(),
        frame_bytes: 0,
        returned: Vec::new(),
        classes: Vec::new(),
        direct_calls: HashMap::new(),
    };
    lowerer.run(func)?;
    let makes_call = func.blocks.iter().any(|block| {
        block
            .insts
            .iter()
            .any(|inst| matches!(inst, Inst::Call { .. }))
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
        frame_bytes: lowerer.frame_bytes,
        variadic: func.variadic,
        classes: lowerer.classes,
        returned: lowerer.returned,
        outgoing_slots: outgoing_slots as u32,
        param_count: func.params().len(),
        makes_call,
        blocks: lowerer.blocks,
        param_offsets: lowerer.param_offsets,
    })
}

/// The values of one function, the frame slots they use, and the blocks being filled.
struct FunctionLowering<'a> {
    types: &'a Types,
    names: &'a crate::intern::Interner,
    /// A machine register per `cir` value, mints on first use.
    regs: Vec<Option<VReg>>,
    next_reg: u32,
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
}

impl FunctionLowering<'_> {
    fn run(&mut self, func: &Func) -> Result<(), NotYet> {
        // A call whose callee is the address of a global function is a direct call, and the address
        // itself is then left out of the machine form - which is sound only when *every* read of it
        // is a call naming it. A value read anywhere else needs an address this back end has not
        // learnt to make, so it is refused rather than quietly made zero.
        let mut reads = self.read_counts(func);
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
        // The first four *positions* take a register each, counted separately per class: a fifth
        // integer argument does not take `r10`, it goes on the stack.
        let mut integer_slot = 0u8;
        let mut float_slot = 0u8;
        for (position, param) in func.params().iter().enumerate() {
            let ty = func.value(*param).ty;
            let (size, signed, class) = self.scalar(ty, func.entry_span())?;
            let (slot, on_stack) = if position < ARGUMENT_COUNT {
                if class == Class::Float {
                    let slot = float_slot;
                    float_slot += 1;
                    (slot, false)
                } else {
                    let slot = integer_slot;
                    integer_slot += 1;
                    (slot, false)
                }
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
            for inst in &func.block(block).insts {
                self.lower_inst(func, inst, &mut block_ops)?;
            }
            let term = self.lower_term(func, block, &mut block_ops)?;
            self.blocks.push(MachineBlock {
                ops: block_ops,
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
                let (left, right) = (self.reg_for(*left), self.reg_for(*right));
                if class == Class::Float {
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
                        right,
                        right_signed: signed,
                        left_shift: *op == BinOp::Shl,
                        size,
                        signed,
                    },
                    BinOp::Div | BinOp::Rem => Op::Divide {
                        dst,
                        left,
                        right,
                        remainder: *op == BinOp::Rem,
                        signed,
                        size,
                    },
                    _ => Op::Binary {
                        dst,
                        op: *op,
                        left,
                        right,
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
                let (left, right) = (self.reg_for(*left), self.reg_for(*right));
                ops.push(match class {
                    Class::Float => Op::FloatCompare {
                        dst,
                        op: *op,
                        left,
                        right,
                        size,
                    },
                    Class::Integer => Op::Compare {
                        dst,
                        op: *op,
                        left,
                        right,
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
            Inst::Slot { result, ty, .. } => {
                let size = self.types.size_of(*ty).ok_or_else(|| NotYet {
                    message: format!("a local of incomplete type {}", self.types.describe(*ty)),
                    span,
                })?;
                let align = self.types.align_of(*ty).unwrap_or(1);
                let offset = self.allocate(size, align);
                let dst = self.reg_for(*result);
                self.classify(dst, Class::Integer);
                ops.push(Op::LocalAddress { dst, offset });
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
                if class == Class::Float {
                    return Err(NotYet {
                        message:
                            "a `va_arg` of a floating-point type: Win64 passes a variadic `double` \
                                  in a vector register as well as a general one, and this back end \
                                  writes only the general one"
                                .into(),
                        span,
                    });
                }
                let dst = self.reg_for(*result);
                self.classify(dst, Class::Integer);
                ops.push(Op::VaArg {
                    dst,
                    list: self.reg_for(*list),
                    size,
                    signed,
                });
            }
            Inst::GlobalAddress { result, name, .. } => {
                if self.direct_calls.contains_key(result) {
                    // Read only as the callee of a call: the call names the symbol and no address
                    // is ever materialised, so this value is defined by being left out. Nothing
                    // reads it, so it never needs a register or a slot.
                } else {
                    let dst = self.reg_for(*result);
                    self.classify(dst, Class::Integer);
                    ops.push(Op::SymbolAddress {
                        dst,
                        symbol: name.clone(),
                    });
                }
            }
            Inst::Call {
                result,
                callee,
                args,
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
                    }),
                    None => ops.push(Op::CallIndirect {
                        dst,
                        callee: self.reg_for(*callee),
                        args,
                        size,
                        signed,
                    }),
                }
            }
        }
        Ok(())
    }

    fn lower_term(
        &mut self,
        func: &Func,
        block: Block,
        ops: &mut Vec<Op>,
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
                BlockTerm::Branch {
                    condition: self.reg_for(condition),
                    then: self.order[&then.0],
                    otherwise: self.order[&otherwise.0],
                }
            }
            Term::Unreachable { .. } => BlockTerm::Unreachable,
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
