//! The assembler: machine operations with their virtual registers resolved to a closed set of
//! x86-64 encodings.
//!
//! Two things worth knowing before reading it:
//!
//! - **The frame is a byte offset from `rsp`, and `rsp` never moves after the prologue.** Locals,
//!   block parameters, spill slots and the staging area for call arguments are one layout decided
//!   here: shadow space first (the callee writes there), then the callee-saved registers this
//!   function uses, then the lowering's frame, then spill slots, then the staging area. A `ret`
//!   undoes exactly that; there is no `rbp` and no `push`.
//! - **Scratch registers are the calling convention's, not the allocator's.** `rax`, `rcx`, `rdx`,
//!   `r10` and `r11` hold nothing that must survive an operation, so an operation may use one as
//!   room to work. Only the seven allocated registers ever hold a value across operations, and a
//!   call cannot destroy them. An operation computes in the destination's own register when an
//!   operand is already there - which the allocator arranges often, by coalescing - so the scratch
//!   registers are needed only for operands that live in memory and for the shapes x86 fixes
//!   (`cl` for a shift count, `rax` and `rdx` for a division).
//!
//! The ceiling this slice has: a frame larger than a page does not probe the stack, so a function
//! with a very large frame could skip past the guard page. Cranelift's back end emits the probe
//! (ADR-005); this one will when the slice grows a frame that large.

use super::regalloc::{Allocation, Location, Reg};
use super::{BinOp, BlockTerm, MachineFunction, Op, VReg};

/// Whether an operation gives the same answer with its operands swapped, which is what lets the
/// destination's register hold either one.
fn commutative(op: BinOp) -> bool {
    matches!(
        op,
        BinOp::Add | BinOp::Mul | BinOp::And | BinOp::Or | BinOp::Xor
    )
}

/// A place in the text that names a symbol, which the object file resolves.
#[derive(Debug)]
pub(crate) struct Relocation {
    /// Where in the text the four-byte displacement is.
    pub offset: u32,
    /// The symbol it points at.
    pub symbol: String,
}

/// What one function became.
#[derive(Debug)]
pub(crate) struct Encoded {
    /// The machine code.
    pub code: Vec<u8>,
    /// The places that need a symbol to finish them.
    pub relocations: Vec<Relocation>,
    /// How many bytes the frame is: the prologue subtracts exactly this.
    pub frame: i32,
}

/// The registers the integer arguments arrive in, in order: rcx, rdx, r8, r9.
const ARGUMENT_REGISTERS: [Reg; super::ARGUMENT_COUNT] = [Reg(1), Reg(2), Reg(8), Reg(9)];

/// Encode one function.
pub(crate) fn function(func: &MachineFunction, allocation: &Allocation) -> Encoded {
    let body_base = 32 + allocation.saved.len() as i32 * 8;
    let spill_base = body_base + func.frame_bytes;
    let staging_base = spill_base + allocation.spill_bytes;
    let raw = staging_base + if func.makes_call { 32 } else { 0 };
    // Everything `rsp` is moved for, named: a call (which needs the shadow space and the staging
    // area), a value that did not fit in a register, a block parameter, a variable whose address
    // escapes, and a callee-saved register this function had to keep. A function with none of them
    // has no frame - no `sub rsp`, no `add rsp` - which is fourteen bytes of every such function.
    let frameless = !func.makes_call
        && allocation.spill_bytes == 0
        && func.frame_bytes == 0
        && allocation.saved.is_empty();
    // Otherwise, eight extra bytes so that `rsp` is 16-byte aligned inside the function, which is
    // what a call requires: it is 8-aligned on entry, and an odd multiple of eight puts it right.
    let frame = if frameless {
        0
    } else {
        (raw + 15) / 16 * 16 + 8
    };

    let mut encoder = Encoder {
        code: Vec::new(),
        relocations: Vec::new(),
        fixups: Vec::new(),
        labels: Vec::new(),
        allocation,
        body_base,
        spill_base,
        staging_base,
        epilogue: func.blocks.len(),
        scratch: func.registers().scratch,
        scratches: Vec::new(),
        frame,
    };
    encoder.prologue();
    let last = func.blocks.len().saturating_sub(1);
    for (index, block) in func.blocks.iter().enumerate() {
        encoder.label(index);
        for op in &block.ops {
            encoder.op(op);
        }
        match &block.term {
            BlockTerm::Return(value) => {
                if let Some(value) = value {
                    let register = encoder.read(*value);
                    encoder.mov_reg(Reg::RAX, register);
                    encoder.release();
                }
                // The last block is followed by the epilogue: jumping there is jumping to the
                // next instruction.
                if index != last {
                    let epilogue = encoder.epilogue;
                    encoder.jump(epilogue);
                }
            }
            BlockTerm::Jump(target) => {
                // A jump to the block that comes next is the fall-through, which is free.
                if *target != index + 1 {
                    encoder.jump(*target);
                }
            }
            BlockTerm::Branch {
                condition,
                then,
                otherwise,
            } => {
                let register = encoder.read(*condition);
                encoder.cmp_zero(register);
                encoder.release();
                // The common shape is one side falling through, and testing for that side first
                // keeps a comparison and a constant out of the instruction stream.
                if *otherwise == index + 1 {
                    encoder.branch(*then, 5); // jne
                } else if *then == index + 1 {
                    encoder.branch(*otherwise, 4); // je
                } else {
                    encoder.branch(*then, 5);
                    encoder.jump(*otherwise);
                }
            }
            BlockTerm::Unreachable => encoder.bytes(&[0x0F, 0x0B]), // ud2
        }
    }
    // One epilogue, reached by every return: seven copies of the same five instructions would be
    // seven copies of the same five instructions.
    encoder.label(func.blocks.len());
    encoder.epilogue();
    encoder.patch();
    Encoded {
        code: encoder.code,
        relocations: encoder.relocations,
        frame,
    }
}

/// A site whose four-byte displacement is filled in once every label's position is known.
struct Fixup {
    /// Where the displacement starts in the code.
    at: usize,
    /// Which label it points at.
    label: usize,
}

struct Encoder<'a> {
    code: Vec<u8>,
    relocations: Vec<Relocation>,
    fixups: Vec<Fixup>,
    labels: Vec<Option<usize>>,
    allocation: &'a Allocation,
    /// Where the lowering's frame starts, from `rsp`: after the shadow space and the saved
    /// registers. Meaningful whether or not the function has a frame, because a frame of nothing puts
    /// it where the shadow space ends.
    body_base: i32,
    /// Where spill slots start, from `rsp`; no spills, no references.
    spill_base: i32,
    /// Where the argument staging area starts, from `rsp`; unused unless the function calls.
    staging_base: i32,
    /// Which label the epilogue is at.
    epilogue: usize,
    /// The scratch registers an operation may clobber, which belong to the function.
    scratch: [Reg; 2],
    /// The scratch registers the operation being encoded is holding.
    scratches: Vec<Reg>,
    /// How many bytes the prologue subtracts.
    frame: i32,
}

impl Encoder<'_> {
    // ---------------------------------------------------------------- frame

    fn prologue(&mut self) {
        if self.frame != 0 {
            self.sub_rsp(self.frame);
        }
        let saved = self.allocation.saved.clone();
        for (index, register) in saved.iter().enumerate() {
            self.mov_memory_from_reg(32 + index as i32 * 8, *register);
        }
    }

    fn epilogue(&mut self) {
        let saved = self.allocation.saved.clone();
        for (index, register) in saved.iter().enumerate() {
            self.mov_reg_from_memory(*register, 32 + index as i32 * 8);
        }
        if self.frame != 0 {
            self.add_rsp(self.frame);
        }
        self.byte(0xC3); // ret
    }

    // ---------------------------------------------------------------- operations

    fn op(&mut self, op: &Op) {
        match op {
            Op::Const { dst, value } => {
                let register = self.destination(*dst);
                self.mov_imm(register, *value);
                self.write(*dst, register);
            }
            Op::Incoming {
                dst,
                index,
                size,
                signed,
            } => {
                // Straight into the register (or spill) the value lives in: the argument arrives in
                // one register and is canonicalised where it is going, not in `rax` and then moved.
                let register = self.destination(*dst);
                self.mov_reg(register, ARGUMENT_REGISTERS[usize::from(*index)]);
                self.canonicalise(register, *size, *signed);
                self.write(*dst, register);
            }
            Op::DataAddress { dst, symbol } => {
                // `lea dst, [rip + disp32]`: the displacement is a relocation, because the linker
                // decides where the data section goes.
                let register = self.destination(*dst);
                self.prefix_w(register, Reg(0));
                self.byte(0x8D);
                // mod = 00, r/m = 101 says "relative to the next instruction" with no base register.
                self.byte(((register.0 & 7) << 3) | 0b101);
                self.relocations.push(Relocation {
                    offset: self.code.len() as u32,
                    symbol: symbol.clone(),
                });
                self.bytes(&[0, 0, 0, 0]);
                self.write(*dst, register);
            }
            Op::LocalAddress { dst, offset } => {
                let register = self.destination(*dst);
                self.lea(register, self.body_base + offset);
                self.write(*dst, register);
            }
            Op::LoadLocal { dst, offset } => {
                let register = self.destination(*dst);
                self.mov_reg_from_memory(register, self.body_base + offset);
                self.write(*dst, register);
            }
            Op::StoreLocal { offset, src } => {
                let register = self.read(*src);
                self.mov_memory_from_reg(self.body_base + offset, register);
                self.release();
            }
            Op::LoadMemory {
                dst,
                base,
                offset,
                size,
                signed,
            } => {
                let address = self.read(*base);
                let register = self.destination(*dst);
                self.load_widened(register, address, *offset, *size, *signed);
                self.write(*dst, register);
                self.release();
            }
            Op::StoreMemory {
                base,
                offset,
                src,
                size,
            } => {
                let address = self.read(*base);
                let value = self.read(*src);
                self.store_narrowed(address, *offset, value, *size);
                self.release();
            }
            Op::Binary {
                dst,
                op,
                left,
                right,
                size,
                signed,
            } => self.binary(*dst, *op, *left, *right, *size, *signed),
            Op::Shift {
                dst,
                left,
                right,
                right_signed,
                left_shift,
                size,
                signed,
            } => self.shift(
                *dst,
                *left,
                *right,
                *left_shift,
                *right_signed,
                *size,
                *signed,
            ),
            Op::Divide {
                dst,
                left,
                right,
                remainder,
                signed,
                size,
            } => self.divide(*dst, *left, *right, *remainder, *signed, *size),
            Op::Unary {
                dst,
                src,
                negate,
                size,
                signed,
            } => self.unary(*dst, *src, *negate, *size, *signed),
            Op::Compare {
                dst,
                op,
                left,
                right,
                signed,
            } => self.compare(*dst, *op, *left, *right, *signed),
            Op::Canonical {
                dst,
                src,
                size,
                signed,
            } => self.canonical_op(*dst, *src, *size, *signed),
            Op::Call {
                dst,
                name,
                args,
                size,
                signed,
            } => {
                // An argument that is already in another argument's register has to be got out of
                // the way first, which the staging area does; when no two of them collide - the
                // common case, and the only one for a call with one argument - each goes straight
                // to its register.
                if self.arguments_collide(args) {
                    for (index, arg) in args.iter().enumerate() {
                        let register = self.read(*arg);
                        self.mov_memory_from_reg(self.staging_base + index as i32 * 8, register);
                        self.release();
                    }
                    for (index, target) in ARGUMENT_REGISTERS.iter().enumerate().take(args.len()) {
                        self.mov_reg_from_memory(*target, self.staging_base + index as i32 * 8);
                    }
                } else {
                    for (index, arg) in args.iter().enumerate() {
                        self.read_into(*arg, ARGUMENT_REGISTERS[index]);
                    }
                }
                self.call(name);
                if let Some(dst) = dst {
                    // The upper bits of `rax` are unspecified after a call, so a narrow result is
                    // canonicalised here rather than trusted.
                    self.canonicalise(Reg::RAX, *size, *signed);
                    self.write(*dst, Reg::RAX);
                }
            }
        }
    }

    // ---------------------------------------------------------------- operations with a shape

    /// `dst = left OP right`, in the destination's own register where one of the operands is
    /// already there. This is where most of the code size is won: the naive form moves both operands
    /// into scratch registers and the result out again, three moves around one `add`.
    fn binary(&mut self, dst: VReg, op: BinOp, left: VReg, right: VReg, size: u8, signed: bool) {
        let left = self.read(left);
        let right = self.read(right);
        let swaps = commutative(op);
        let target = self.destination(dst);
        let target = if self.is_in(dst, left) {
            left
        } else if swaps && self.is_in(dst, right) {
            right
        } else if target != left && target != right {
            self.mov_reg(target, left);
            target
        } else {
            // The destination's own register holds the other operand and this operation does not
            // commute, so the answer is computed elsewhere and moved back.
            let scratch = self.scratch();
            self.mov_reg(scratch, left);
            scratch
        };
        let (accumulator, operand) = if swaps && target == right {
            (right, left)
        } else {
            (target, right)
        };
        match op {
            BinOp::Mul => {
                // imul puts one operand in the register field: `imul accumulator, operand`.
                self.prefix_w(accumulator, operand);
                self.bytes(&[0x0F, 0xAF]);
                self.modrm(3, accumulator.0, operand);
            }
            other => {
                let opcode = match other {
                    BinOp::Add => 0x01,
                    BinOp::Sub => 0x29,
                    BinOp::And => 0x21,
                    BinOp::Or => 0x09,
                    BinOp::Xor => 0x31,
                    unreachable => unreachable!("{unreachable:?} is not a binary operation"),
                };
                // `op accumulator, operand`: the accumulator is the r/m side.
                self.prefix_w(operand, accumulator);
                self.byte(opcode);
                self.modrm(3, operand.0, accumulator);
            }
        }
        self.canonicalise(target, size, signed);
        self.write(dst, target);
        self.release();
    }

    /// `dst = left << right`, or a right shift whose kind the operand's type decides. The count is
    /// only expressible in `cl`, which is not allocated, so one move is unavoidable.
    #[allow(clippy::too_many_arguments)] // the operation's shape *is* this long
    fn shift(
        &mut self,
        dst: VReg,
        left: VReg,
        right: VReg,
        left_shift: bool,
        arithmetic: bool,
        size: u8,
        signed: bool,
    ) {
        let source = self.read(left);
        let count = self.read(right);
        self.mov_reg(Reg::RCX, count);
        let target = if self.is_in(dst, source) {
            source
        } else {
            let scratch = self.scratch();
            self.mov_reg(scratch, source);
            scratch
        };
        self.prefix_w(Reg(0), target);
        self.byte(0xD3);
        let extension = match (left_shift, arithmetic) {
            (true, _) => 4,      // shl
            (false, true) => 7,  // sar
            (false, false) => 5, // shr
        };
        self.modrm(3, extension, target);
        self.canonicalise(target, size, signed);
        self.write(dst, target);
        self.release();
    }

    /// `/` or `%`: `rax` holds the dividend and `rdx` the extension, so the operands are moved into
    /// the registers x86 names rather than into ones this allocator chose.
    fn divide(
        &mut self,
        dst: VReg,
        left: VReg,
        right: VReg,
        remainder: bool,
        signed: bool,
        size: u8,
    ) {
        self.read_into(left, Reg::RAX);
        let divisor = self.read_into(right, self.scratch[0]);
        if signed {
            self.bytes(&[0x48, 0x99]); // cqo
        } else {
            self.bytes(&[0x31, 0xD2]); // xor edx, edx
        }
        self.prefix_w(Reg(0), divisor);
        self.byte(0xF7);
        self.modrm(3, if signed { 7 } else { 6 }, divisor);
        let result = if remainder { Reg::RDX } else { Reg::RAX };
        self.canonicalise(result, size, signed);
        self.write(dst, result);
        self.release();
    }

    /// Unary `-` or `~`, in place when the value is already where the answer goes.
    fn unary(&mut self, dst: VReg, src: VReg, negate: bool, size: u8, signed: bool) {
        let source = self.read(src);
        let target = if self.scratches.contains(&source) || self.is_in(dst, source) {
            source
        } else {
            let target = self.destination(dst);
            self.mov_reg(target, source);
            target
        };
        self.prefix_w(Reg(0), target);
        self.byte(0xF7);
        self.modrm(3, if negate { 3 } else { 2 }, target);
        self.canonicalise(target, size, signed);
        self.write(dst, target);
        self.release();
    }

    /// A comparison into a 1 or 0: `cmp` the two registers the operands already live in, then `setcc`
    /// into the destination's own low byte and widen it there. The answer has no reason to pass
    /// through `rax`, and 317 of the corpus's copies were it doing so.
    fn compare(&mut self, dst: VReg, op: crate::cir::CmpOp, left: VReg, right: VReg, signed: bool) {
        let left = self.read(left);
        let right = self.read(right);
        self.prefix_w(right, left);
        self.byte(0x39); // cmp left, right
        self.modrm(3, right.0, left);
        let target = self.destination(dst);
        // `setcc` has no 64-bit form, and its operand is the r/m byte: a REX prefix is needed to
        // name `spl`, `bpl`, `sil` and `dil`, and for an extended register's high bit.
        if target.0 >= 4 {
            self.rex_byte(Reg(0), target);
        }
        self.bytes(&[0x0F, 0x90 | condition_code(op, signed)]);
        self.modrm(3, 0, target);
        self.canonicalise(target, 1, false);
        self.write(dst, target);
        self.release();
    }

    /// Truncate to the type's width and extend back: in place when the value is where it is going.
    fn canonical_op(&mut self, dst: VReg, src: VReg, size: u8, signed: bool) {
        let source = self.read(src);
        let target = if self.scratches.contains(&source) || self.is_in(dst, source) {
            source
        } else {
            let target = self.destination(dst);
            self.mov_reg(target, source);
            target
        };
        self.canonicalise(target, size, signed);
        self.write(dst, target);
        self.release();
    }

    /// Whether a value lives in this register.
    fn is_in(&self, value: VReg, register: Reg) -> bool {
        matches!(self.allocation.location(value), Location::Register(other) if other == register)
    }

    /// The register an operation computes its result in when the destination is memory: `rax` is
    /// never allocated, so it is always free.
    fn destination(&self, dst: VReg) -> Reg {
        match self.allocation.location(dst) {
            Location::Register(register) => register,
            Location::Spill(_) => Reg::RAX,
        }
    }

    /// Where a value lives, in a register: a spilled one is loaded into a scratch for this use.
    fn read(&mut self, value: VReg) -> Reg {
        match self.allocation.location(value) {
            Location::Register(register) => register,
            Location::Spill(offset) => {
                let scratch = self.scratch();
                self.mov_reg_from_memory(scratch, self.spill_base + offset);
                scratch
            }
        }
    }

    /// Read a value into a specific register, which division and comparison need: `rax` holds the
    /// dividend, and `r10`/`r11` are the operands' homes.
    fn read_into(&mut self, value: VReg, wanted: Reg) -> Reg {
        match self.allocation.location(value) {
            Location::Register(register) => {
                if register != wanted {
                    self.mov_reg(wanted, register);
                }
            }
            Location::Spill(offset) => self.mov_reg_from_memory(wanted, self.spill_base + offset),
        }
        if wanted.0 >= 10 && !self.scratches.contains(&wanted) {
            self.scratches.push(wanted);
        }
        wanted
    }

    /// Whether moving the arguments into their registers, in order, can destroy one that has not
    /// been read yet: an argument whose register is a *later* argument's register.
    fn arguments_collide(&self, args: &[VReg]) -> bool {
        args.iter().enumerate().any(|(index, arg)| {
            match self.allocation.location(*arg) {
                Location::Register(register) => ARGUMENT_REGISTERS
                    .iter()
                    .position(|target| *target == register)
                    .is_some_and(|position| position != index),
                // A spilled argument is read straight out of its slot into the register that needs
                // it, so it cannot be in the way.
                Location::Spill(_) => false,
            }
        })
    }

    /// A register for a spilled operand: the function's two scratch registers, and no third. An
    /// operation holds at most two, and asking for a third means it would clobber one it is still
    /// using.
    fn scratch(&mut self) -> Reg {
        let [first, second] = self.scratch;
        let register = if !self.scratches.contains(&first) {
            first
        } else if !self.scratches.contains(&second) {
            second
        } else {
            unreachable!("an operation is holding both scratch registers already")
        };
        self.scratches.push(register);
        register
    }

    fn release(&mut self) {
        self.scratches.clear();
    }

    /// Write a computed value back to where the destination lives.
    fn write(&mut self, dst: VReg, register: Reg) {
        match self.allocation.location(dst) {
            Location::Register(target) => {
                if target != register {
                    self.mov_reg(target, register);
                }
            }
            Location::Spill(offset) => self.mov_memory_from_reg(self.spill_base + offset, register),
        }
    }

    // ---------------------------------------------------------------- encoding

    fn byte(&mut self, byte: u8) {
        self.code.push(byte);
    }

    fn bytes(&mut self, bytes: &[u8]) {
        self.code.extend_from_slice(bytes);
    }

    fn label(&mut self, index: usize) {
        if self.labels.len() <= index {
            self.labels.resize(index + 1, None);
        }
        self.labels[index] = Some(self.code.len());
    }

    fn jump(&mut self, target: usize) {
        self.byte(0xE9);
        self.fixup(target);
    }

    fn branch(&mut self, target: usize, condition: u8) {
        self.bytes(&[0x0F, 0x80 | condition]);
        self.fixup(target);
    }

    fn fixup(&mut self, label: usize) {
        self.fixups.push(Fixup {
            at: self.code.len(),
            label,
        });
        self.bytes(&[0, 0, 0, 0]);
    }

    fn call(&mut self, name: &str) {
        self.byte(0xE8);
        self.relocations.push(Relocation {
            offset: self.code.len() as u32,
            symbol: name.to_string(),
        });
        self.bytes(&[0, 0, 0, 0]);
    }

    /// Resolve every jump. A relative displacement is measured from the byte after it, which is
    /// what `rel32` means and the only place that fact appears.
    fn patch(&mut self) {
        for fixup in &self.fixups {
            let target = self.labels[fixup.label].unwrap_or(self.code.len());
            let displacement = target as i64 - (fixup.at as i64 + 4);
            self.code[fixup.at..fixup.at + 4].copy_from_slice(&(displacement as i32).to_le_bytes());
        }
    }

    fn modrm(&mut self, mode: u8, reg: u8, rm: Reg) {
        self.byte((mode << 6) | ((reg & 7) << 3) | (rm.0 & 7));
        // Only a memory operand needs a SIB byte: `mod = 3` names a register, and a register that
        // happens to be number four (`rsp`) has no addressing mode at all.
        if mode != 3 && rm.0 & 7 == 4 {
            // A base of `rsp` or `r12` needs a SIB byte to say "no index, base is rsp".
            self.byte(0x24);
        }
    }

    fn mov_reg(&mut self, to: Reg, from: Reg) {
        if to == from {
            return;
        }
        self.prefix_w(from, to);
        self.byte(0x89);
        self.modrm(3, from.0, to);
    }

    fn mov_imm(&mut self, to: Reg, value: u64) {
        // The 32-bit form sign-extends to 64, so it holds anything whose 64-bit pattern survives
        // that extension: every constant that came from a C `int`, and `-1`, and small pointers.
        if let Ok(immediate) = i32::try_from(value as i64) {
            self.prefix_w(Reg(0), to);
            self.byte(0xC7);
            self.modrm(3, 0, to);
            self.bytes(&immediate.to_le_bytes());
        } else {
            self.prefix_w(Reg(0), to);
            self.byte(0xB8 | (to.0 & 7));
            self.bytes(&value.to_le_bytes());
        }
    }

    fn lea(&mut self, to: Reg, offset: i32) {
        self.prefix_w(to, Reg::RSP);
        self.byte(0x8D);
        self.memory_operand(to.0, Reg::RSP, offset);
    }

    fn mov_reg_from_memory(&mut self, to: Reg, offset: i32) {
        self.prefix_w(to, Reg::RSP);
        self.byte(0x8B);
        self.memory_operand(to.0, Reg::RSP, offset);
    }

    fn mov_memory_from_reg(&mut self, offset: i32, from: Reg) {
        self.prefix_w(from, Reg::RSP);
        self.byte(0x89);
        self.memory_operand(from.0, Reg::RSP, offset);
    }

    /// A `[base + offset]` operand: the smallest displacement form that reaches the offset.
    fn memory_operand(&mut self, reg: u8, base: Reg, offset: i32) {
        let mode = if offset == 0 && base.0 & 7 != 5 {
            0
        } else if i8::try_from(offset).is_ok() {
            1
        } else {
            2
        };
        self.modrm(mode, reg, base);
        if mode == 1 {
            self.byte(offset as i8 as u8);
        } else if mode == 2 {
            self.bytes(&offset.to_le_bytes());
        }
    }

    fn cmp_zero(&mut self, register: Reg) {
        self.prefix_w(Reg(0), register);
        self.byte(0x83);
        self.modrm(3, 7, register);
        self.byte(0);
    }

    fn sub_rsp(&mut self, bytes: i32) {
        self.prefix_w(Reg(0), Reg::RSP);
        self.byte(0x81);
        self.modrm(3, 5, Reg::RSP);
        self.bytes(&bytes.to_le_bytes());
    }

    fn add_rsp(&mut self, bytes: i32) {
        self.prefix_w(Reg(0), Reg::RSP);
        self.byte(0x81);
        self.modrm(3, 0, Reg::RSP);
        self.bytes(&bytes.to_le_bytes());
    }

    /// Read `size` bytes at `base + offset`, widened to 64 bits, with the sign the C type has.
    fn load_widened(&mut self, to: Reg, base: Reg, offset: i32, size: u8, signed: bool) {
        match (size, signed) {
            (1, false) => self.widen(to, base, offset, &[0x0F, 0xB6]),
            (1, true) => self.widen(to, base, offset, &[0x0F, 0xBE]),
            (2, false) => self.widen(to, base, offset, &[0x0F, 0xB7]),
            (2, true) => self.widen(to, base, offset, &[0x0F, 0xBF]),
            (4, true) => self.widen(to, base, offset, &[0x63]),
            (4, false) => {
                // A 32-bit load zeroes the upper half, which is the canonical unsigned form.
                self.prefix(to, base);
                self.byte(0x8B);
                self.memory_operand(to.0, base, offset);
            }
            (8, _) => {
                self.prefix_w(to, base);
                self.byte(0x8B);
                self.memory_operand(to.0, base, offset);
            }
            (other, _) => unreachable!("a {other}-byte load"),
        }
    }

    fn widen(&mut self, to: Reg, base: Reg, offset: i32, opcode: &[u8]) {
        self.prefix_w(to, base);
        self.bytes(opcode);
        self.memory_operand(to.0, base, offset);
    }

    /// Write the low `size` bytes of `value` at `base + offset`.
    ///
    /// A one-byte store needs a REX prefix more often than the others: without one, register
    /// numbers four to seven are `ah`, `ch`, `dh` and `bh`, and only with one are they `spl`,
    /// `bpl`, `sil` and `dil`. Writing `sil` without it stores the wrong byte, silently.
    fn store_narrowed(&mut self, base: Reg, offset: i32, value: Reg, size: u8) {
        match size {
            2 => {
                self.byte(0x66);
                self.prefix(value, base);
            }
            1 => {
                if value.0 >= 4 || base.0 >= 8 {
                    self.rex_byte(value, base);
                }
            }
            4 => self.prefix(value, base),
            _ => self.prefix_w(value, base),
        }
        self.byte(if size == 1 { 0x88 } else { 0x89 });
        self.memory_operand(value.0, base, offset);
    }

    /// Truncate to `size` bytes and extend back to 64 bits, with the type's sign.
    fn canonicalise(&mut self, register: Reg, size: u8, signed: bool) {
        match (size, signed) {
            (8, _) => {}
            (4, false) => {
                // `mov eax, eax` zeroes the upper 32 bits and touches nothing else.
                self.prefix(register, register);
                self.byte(0x89);
                self.modrm(3, register.0, register);
            }
            (4, true) => self.widen_reg(register, &[0x63]),
            (2, false) => self.widen_reg(register, &[0x0F, 0xB7]),
            (2, true) => self.widen_reg(register, &[0x0F, 0xBF]),
            (1, false) => self.widen_reg(register, &[0x0F, 0xB6]),
            (1, true) => self.widen_reg(register, &[0x0F, 0xBE]),
            (other, _) => unreachable!("canonicalising {other} bytes"),
        }
    }

    fn widen_reg(&mut self, register: Reg, opcode: &[u8]) {
        self.prefix_w(register, register);
        self.bytes(opcode);
        self.modrm(3, register.0, register);
    }

    /// The REX prefix for a 64-bit operation whose result goes in `reg` and whose memory operand
    /// uses `base`. The high bits of an extended register are the only other thing in it.
    fn prefix_w(&mut self, reg: Reg, base: Reg) {
        self.byte(0x48 | ((reg.0 >> 3) << 2) | (base.0 >> 3));
    }

    /// The same for a 32-bit operation, which needs a prefix only to name an extended register.
    fn prefix(&mut self, reg: Reg, base: Reg) {
        if reg.0 >= 8 || base.0 >= 8 {
            self.rex_byte(reg, base);
        }
    }

    /// A REX prefix with no 64-bit bit: the only way to name `spl`, `bpl`, `sil` or `dil`.
    fn rex_byte(&mut self, reg: Reg, base: Reg) {
        self.byte(0x40 | ((reg.0 >> 3) << 2) | (base.0 >> 3));
    }
}

/// The condition code `setcc` and `jcc` use for a comparison.
fn condition_code(op: crate::cir::CmpOp, signed: bool) -> u8 {
    use crate::cir::CmpOp;
    match (op, signed) {
        (CmpOp::Eq, _) => 4,
        (CmpOp::Ne, _) => 5,
        (CmpOp::Less, true) => 12,
        (CmpOp::Less, false) => 2,
        (CmpOp::LessEqual, true) => 14,
        (CmpOp::LessEqual, false) => 6,
        (CmpOp::Greater, true) => 15,
        (CmpOp::Greater, false) => 7,
        (CmpOp::GreaterEqual, true) => 13,
        (CmpOp::GreaterEqual, false) => 3,
    }
}
