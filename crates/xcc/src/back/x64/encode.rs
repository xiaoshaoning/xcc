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
use super::{BinOp, BlockTerm, MachineFunction, Op, Operand, VReg};

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

/// The two XMM registers the encoder uses as scratch for floating-point operands; nothing is
/// allocated from them.
const FLOAT_SCRATCH: Reg = Reg(0);
/// Where a spilled floating-point *result* is computed before it is stored: a third register, because
/// an operation with two spilled operands has spent the scratch pair and would otherwise compute its
/// answer in a register it still has to read.
const FLOAT_RESULT: Reg = Reg(2);
const FLOAT_SECOND: Reg = Reg(1);

/// What a floating-point move is doing, which decides its opcode and its direction.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Move {
    /// Register to register: a copy that keeps every bit.
    Register,
    /// Memory to register.
    Load,
    /// Register to memory.
    Store,
    /// Sixteen bytes from memory into a register wherever they are: `movups`, which asks for no
    /// alignment, because a save slot sits wherever the general saves left room.
    LoadUnaligned,
    /// The same the other way.
    StoreUnaligned,
}

/// Where a call argument goes: a register of its class, or the caller's outgoing slots.
#[derive(Debug, Clone, Copy)]
enum Target {
    /// A general-purpose register.
    Integer(Reg),
    /// An XMM register.
    Float(Reg),
    /// A stack slot, at this offset from `rsp`.
    Stack(i32),
}

/// Encode one function.
pub(crate) fn function(func: &MachineFunction, allocation: &Allocation) -> Encoded {
    // An `i32` frame is what the prologue can subtract, and the front end's token budget bounds the
    // argument count long before that: a call site cannot spend more tokens than the whole source.
    let outgoing_base = 32;
    let outgoing_bytes = func.outgoing_slots as i32 * 8;
    // The integer registers a function keeps are *pushed*, not stored into slots: `push rbx` is one
    // byte where `mov [rsp+disp8], rbx` is five, and the pop restores it without an address. They sit
    // in the eight bytes each that the frame still reserves for them, at the top of it, which is where
    // pushes put things - so the layout keeps its shape and nothing addresses them.
    let pushed = 8 * allocation.saved.len() as i32;
    let saved_float_base = outgoing_base + outgoing_bytes;
    let body_base = saved_float_base + allocation.saved_floats.len() as i32 * 16;
    let spill_base = body_base + func.frame_bytes;
    // The frame ends where the spills do: the staging area this used to hold - eight slots, for four
    // integer arguments and four floating ones - was for a case that cannot occur (see
    // `place_arguments`).
    let raw = spill_base + allocation.spill_bytes;
    // Everything `rsp` is moved for, named: a call (which needs the shadow space and the staging
    // area), a value that did not fit in a register, a block parameter, a variable whose address
    // escapes, and a callee-saved register this function had to keep. A function with none of them
    // has no frame - no `sub rsp`, no `add rsp` - which is fourteen bytes of every such function.
    // A function with no call has no outgoing slots by construction - the count comes from call
    // sites - so the call is the only thing that needs naming here.
    let frameless = !func.makes_call
        && allocation.spill_bytes == 0
        && func.frame_bytes == 0
        && allocation.saved.is_empty()
        && allocation.saved_floats.is_empty();
    // Otherwise, eight extra bytes so that `rsp` is 16-byte aligned inside the function, which is
    // what a call requires: it is 8-aligned on entry, and an odd multiple of eight puts it right.
    // The pushes sit *above* everything the body uses, not beside it: the body grows up from `rsp` and
    // the pushed registers end where `rsp` began, so the total the prologue moves is the two added
    // together, rounded so that `rsp` stays 16-byte aligned inside the function. Adding is what makes
    // the pushed area and the body disjoint; taking the larger of the two looked right and put a
    // function's locals on top of its saved registers the moment the body grew past the pushes - a
    // fault that 64 bytes of now-removed padding had been hiding.
    let frame = if frameless {
        0
    } else {
        (raw + pushed + 15) / 16 * 16 + 8
    };

    let mut encoder = Encoder {
        code: Vec::new(),
        relocations: Vec::new(),
        fixups: Vec::new(),
        labels: Vec::new(),
        allocation,
        body_base,
        spill_base,
        outgoing_base,
        outgoing_bytes,
        saved_float_base,
        epilogue: func.blocks.len(),
        scratch: func.registers().scratch,
        classes: &func.classes,
        scratches: Vec::new(),
        frame,
    };
    encoder.prologue(func.variadic);
    let last = func.blocks.len().saturating_sub(1);
    for (index, block) in func.blocks.iter().enumerate() {
        encoder.label(index);
        for op in &block.ops {
            encoder.op(op);
        }
        match &block.term {
            BlockTerm::Return(value) => {
                if let Some(value) = value {
                    // A float goes back in `xmm0` and an integer in `rax`.
                    if encoder.class_of(*value) == super::Class::Float {
                        let register = encoder.read_float(*value);
                        encoder.float_move(
                            super::FLOAT_ARGUMENTS[0],
                            register,
                            0,
                            8,
                            Move::Register,
                        );
                    } else {
                        let register = encoder.read(*value);
                        encoder.mov_reg(Reg::RAX, register);
                    }
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
                encoder.branch_either(5, 4, *then, *otherwise, index);
            }
            BlockTerm::BranchCompare {
                op,
                left,
                right,
                signed,
                then,
                otherwise,
            } => {
                encoder.compare_flags(*left, *right);
                encoder.release();
                let taken = condition_code(*op, *signed);
                // The inverse of a condition code is its low bit: the sixteen of them are eight
                // pairs, `e`/`ne`, `b`/`ae`, `be`/`a`, and so on.
                encoder.branch_either(taken, taken ^ 1, *then, *otherwise, index);
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
    /// Where the outgoing argument slots start, from `rsp`: above the shadow space, where a callee
    /// reads its fifth argument.
    outgoing_base: i32,
    /// How many bytes those slots take, which everything after them is displaced by.
    outgoing_bytes: i32,

    /// Where the saved XMM registers start, from `rsp`: after those, sixteen bytes each.
    saved_float_base: i32,
    /// Where the lowering's frame starts, from `rsp`: after the shadow space and the saved
    /// registers. Meaningful whether or not the function has a frame, because a frame of nothing puts
    /// it where the shadow space ends.
    body_base: i32,
    /// Where spill slots start, from `rsp`; no spills, no references.
    spill_base: i32,
    /// Which label the epilogue is at.
    epilogue: usize,
    /// The scratch registers an operation may clobber, which belong to the function.
    scratch: [Reg; 2],
    /// Which register file each value belongs to, from the lowering.
    classes: &'a [super::Class],
    /// The scratch registers the operation being encoded is holding.
    scratches: Vec<Reg>,
    /// How far `rsp` is below its value on entry: the frame's own bytes plus the bytes the pushes
    /// moved. **Every offset in the function is measured from there**, which is why the offsets use
    /// this number and the `sub`/`add rsp` use it minus the pushes - the two add up to the same place.
    frame: i32,
}

impl Encoder<'_> {
    // ---------------------------------------------------------------- frame

    fn prologue(&mut self, variadic: bool) {
        // The pushes come first, so they are outside the frame and need no address: the subtraction
        // is `frame` shorter instead, which is exactly what the layout reserved for them.
        let saved = self.allocation.saved.clone();
        for register in &saved {
            self.push(*register);
        }
        // What is left to subtract after the pushes have moved `rsp` by their own bytes.
        let remaining = self.frame - self.pushed();
        if remaining != 0 {
            self.sub_rsp(remaining);
        }
        if variadic {
            // Win64's home area, which is where `va_start` points: the caller reserved it, and a
            // variadic callee is the only function that has to fill it in. Four integer registers and
            // the stack arguments after them are contiguous from here.
            for (index, register) in super::ARGUMENT_REGISTERS.iter().enumerate() {
                let offset = self.frame + 8 + 8 * index as i32;
                self.mov_memory_from_reg(offset, *register);
            }
        }
        let saved_floats = self.allocation.saved_floats.clone();
        for (index, register) in saved_floats.iter().enumerate() {
            let offset = self.saved_float_base + index as i32 * 16;
            self.float_move(*register, Reg::RSP, offset, 16, Move::StoreUnaligned);
        }
    }

    fn epilogue(&mut self) {
        let saved_floats = self.allocation.saved_floats.clone();
        for (index, register) in saved_floats.iter().enumerate() {
            let offset = self.saved_float_base + index as i32 * 16;
            self.float_move(*register, Reg::RSP, offset, 16, Move::LoadUnaligned);
        }
        let remaining = self.frame - self.pushed();
        if remaining != 0 {
            self.add_rsp(remaining);
        }
        // Popped in the reverse of the order they were pushed, which is all the addressing they need.
        let saved = self.allocation.saved.clone();
        for register in saved.iter().rev() {
            self.pop(*register);
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
                slot,
                class,
                on_stack,
                size,
                signed,
            } => {
                // The fifth position onwards was pushed by the caller: it sits above the shadow
                // space, with the return address above that, so it is `rsp + frame + 40` here.
                let stack_offset = self.frame + 40 + 8 * i32::from(*slot);
                match (class, on_stack) {
                    (super::Class::Integer, false) => {
                        // Straight into the register (or spill) the value lives in: the argument
                        // arrives in one register and is canonicalised where it is going.
                        let register = self.destination(*dst);
                        self.mov_reg(register, super::ARGUMENT_REGISTERS[usize::from(*slot)]);
                        self.canonicalise(register, *size, *signed);
                        self.write(*dst, register);
                    }
                    (super::Class::Integer, true) => {
                        let register = self.destination(*dst);
                        self.mov_reg_from_memory(register, stack_offset);
                        self.canonicalise(register, *size, *signed);
                        self.write(*dst, register);
                    }
                    (super::Class::Float, _) => {
                        let register = self.float_destination(*dst);
                        if *on_stack {
                            self.float_move(register, Reg::RSP, stack_offset, *size, Move::Load);
                        } else {
                            self.float_move(
                                register,
                                super::FLOAT_ARGUMENTS[usize::from(*slot)],
                                0,
                                *size,
                                Move::Register,
                            );
                        }
                        self.write_float(*dst, register);
                    }
                }
            }
            Op::SymbolAddress { dst, symbol } => {
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
            Op::VaStart { dst, skip } => {
                // The caller's home area holds arguments 0 to 3 at `[rsp + frame + 8 ..+]`, and the
                // stack arguments follow at `+40`, so the first unnamed argument is `skip` slots into
                // it and one pointer walks both. This is the whole of Win64's `va_start`.
                let register = self.destination(*dst);
                let offset = self.frame + 8 + 8 * i32::from(*skip);
                self.lea(register, offset);
                self.write(*dst, register);
            }
            Op::VaArg {
                dst,
                list,
                size,
                signed,
                class,
            } => {
                // A load through the list; the front end advances the list itself, so `va_arg` is one
                // instruction and the step is a store the reader can see in the `cir`. A `double` is
                // read from the same eight bytes as anything else: Win64 duplicates a variadic
                // floating-point argument into a general-purpose register for exactly this, so the
                // only difference is which register file the value lands in.
                let address = self.read(*list);
                if *class == super::Class::Float {
                    let register = self.float_destination(*dst);
                    self.float_move(register, address, 0, *size, Move::Load);
                    self.write_float(*dst, register);
                } else {
                    let register = self.destination(*dst);
                    self.load_widened(register, address, 0, *size, *signed);
                    self.write(*dst, register);
                }
                self.release();
            }
            Op::LoadLocal { dst, offset } => {
                // A block parameter's slot holds eight bytes whichever register file the value
                // belongs to, so the move has to be the one that file understands: an integer load
                // would read a double's bits into a general register and lose them.
                let offset = self.body_base + offset;
                if matches!(self.allocation.location(*dst), Location::Float(_)) {
                    let register = self.float_destination(*dst);
                    self.float_move(register, Reg::RSP, offset, 8, Move::Load);
                    self.write_float(*dst, register);
                } else {
                    let register = self.destination(*dst);
                    self.mov_reg_from_memory(register, offset);
                    self.write(*dst, register);
                }
            }
            Op::StoreLocal { offset, src } => {
                let offset = self.body_base + offset;
                if matches!(self.allocation.location(*src), Location::Float(_)) {
                    let register = self.read_float(*src);
                    self.float_move(register, Reg::RSP, offset, 8, Move::Store);
                } else {
                    let register = self.read(*src);
                    self.mov_memory_from_reg(offset, register);
                }
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
            Op::FloatBinary {
                dst,
                op,
                left,
                right,
                size,
            } => self.float_binary(*dst, *op, *left, *right, *size),
            Op::FloatCompare {
                dst,
                op,
                left,
                right,
                size,
            } => self.float_compare(*dst, *op, *left, *right, *size),
            Op::FloatConst { dst, bits, size } => self.float_const(*dst, *bits, *size),
            Op::FloatConvert {
                dst,
                src,
                from,
                to,
                from_size,
                to_size,
                signed,
            } => self.float_convert(*dst, *src, *from, *to, *from_size, *to_size, *signed),
            Op::LoadFloat {
                dst,
                base,
                offset,
                size,
            } => {
                let address = self.read(*base);
                let register = self.float_destination(*dst);
                self.float_move(register, address, *offset, *size, Move::Load);
                self.write_float(*dst, register);
                self.release();
            }
            Op::StoreFloat {
                base,
                offset,
                src,
                size,
            } => {
                let address = self.read(*base);
                let value = self.read_float(*src);
                self.float_move(value, address, *offset, *size, Move::Store);
                self.release();
            }
            Op::Canonical {
                dst,
                src,
                size,
                signed,
            } => self.canonical_op(*dst, *src, *size, *signed),
            Op::CallIndirect {
                dst,
                callee,
                args,
                size,
                signed,
                variadic,
            } => {
                self.place_arguments(args, *variadic);
                // The callee's register survives the placement: a function with any call allocates
                // only callee-saved registers, so it is never one of the argument registers the
                // placement writes. A spilled callee is loaded after, into a scratch.
                let target = self.read(*callee);
                if *variadic {
                    self.no_vector_registers_used();
                }
                self.call_through(target);
                self.finish_call(*dst, *size, *signed);
            }
            Op::Call {
                dst,
                name,
                args,
                size,
                signed,
                variadic,
            } => {
                self.place_arguments(args, *variadic);
                if *variadic {
                    self.no_vector_registers_used();
                }
                self.call(name);
                self.finish_call(*dst, *size, *signed);
            }
        }
    }

    // ---------------------------------------------------------------- operations with a shape

    /// `dst = left OP right`, in the destination's own register where one of the operands is
    /// already there. This is where most of the code size is won: the naive form moves both operands
    /// into scratch registers and the result out again, three moves around one `add`.
    fn binary(&mut self, dst: VReg, op: BinOp, left: VReg, right: Operand, size: u8, signed: bool) {
        // A constant operand is written into the instruction's immediate field: `81 /ext id`, or
        // `83 /ext ib` when a byte is enough, which is how `x + 1` stops paying for a `mov` first.
        if let Operand::Immediate(value) = right {
            return self.binary_immediate(dst, op, left, value, size, signed);
        }
        let Operand::Value(right) = right else {
            unreachable!("an immediate returned above")
        };
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
    /// `cmp left, right`, leaving the answer in the flags. The half a comparison has whether or not
    /// its answer becomes a value, so both shapes share it.
    fn compare_flags(&mut self, left: VReg, right: Operand) {
        let left = self.read(left);
        match right {
            Operand::Value(right) => {
                let right = self.read(right);
                self.prefix_w(right, left);
                self.byte(0x39); // cmp left, right
                self.modrm(3, right.0, left);
            }
            // Comparing with zero does not need a `cmp`: `test x, x` sets the same flags for every
            // condition this back end emits, in fewer bytes, and costs no immediate at all.
            Operand::Immediate(0) => {
                self.prefix_w(left, left);
                self.byte(0x85); // test left, left
                self.modrm(3, left.0, left);
            }
            Operand::Immediate(value) => {
                self.prefix_w(Reg(0), left);
                if let Ok(small) = i8::try_from(value as i64) {
                    self.byte(0x83);
                    self.modrm(3, 7, left);
                    self.byte(small as u8);
                } else {
                    self.byte(0x81);
                    self.modrm(3, 7, left);
                    self.bytes(&(value as i32).to_le_bytes());
                }
            }
        }
    }

    fn compare(
        &mut self,
        dst: VReg,
        op: crate::cir::CmpOp,
        left: VReg,
        right: Operand,
        signed: bool,
    ) {
        self.compare_flags(left, right);
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

    /// `dst = left OP constant`, with the constant in the instruction. Only the operations whose
    /// encoding has that field: `mul` takes three operands and a register, and a shift's count is a
    /// register, so neither comes here.
    fn binary_immediate(
        &mut self,
        dst: VReg,
        op: BinOp,
        left: VReg,
        value: u64,
        size: u8,
        signed: bool,
    ) {
        let extension =
            immediate_extension(op).expect("only an operation with an immediate field gets here");
        // Three cases collapse into one: the destination's register is the left operand's (compute
        // there), or it is another register (move the operand in), or it is a spilled destination
        // whose spare is `rax` (move in as well). A constant has no other operand to collide with, so
        // unlike `binary` there is no scratch case.
        let left = self.read(left);
        let target = self.destination(dst);
        if target != left {
            self.mov_reg(target, left);
        }
        self.prefix_w(Reg(0), target);
        if let Ok(small) = i8::try_from(value as i64) {
            self.byte(0x83);
            self.modrm(3, extension, target);
            self.byte(small as u8);
        } else {
            self.byte(0x81);
            self.modrm(3, extension, target);
            self.bytes(&(value as i32).to_le_bytes());
        }
        self.canonicalise(target, size, signed);
        self.write(dst, target);
        self.release();
    }

    // ---------------------------------------------------------------- floating point

    /// Move a floating-point value: a register copy, a load, or a store, in the type's width.
    ///
    /// The REX byte comes after the mandatory prefix and is not optional: an XMM register above
    /// `xmm7` and an address base above `r7` are only nameable through it, and without it the
    /// instruction silently addresses `rax` instead of `r8`.
    fn float_move(&mut self, to: Reg, base: Reg, offset: i32, size: u8, what: Move) {
        match what {
            Move::Register => {
                self.prefix(to, base);
                self.bytes(&[0x0F, 0x28]); // movaps: every bit, whatever the type
                self.modrm(3, to.0, base);
            }
            Move::Load => {
                self.float_prefix(size);
                self.prefix(to, base);
                self.bytes(&[0x0F, 0x10]);
                self.memory_operand(to.0, base, offset);
            }
            Move::Store => {
                self.float_prefix(size);
                self.prefix(to, base);
                self.bytes(&[0x0F, 0x11]);
                self.memory_operand(to.0, base, offset);
            }
            Move::LoadUnaligned => {
                self.prefix(to, base);
                self.bytes(&[0x0F, 0x10]); // movups
                self.memory_operand(to.0, base, offset);
            }
            Move::StoreUnaligned => {
                self.prefix(to, base);
                self.bytes(&[0x0F, 0x11]); // movups
                self.memory_operand(to.0, base, offset);
            }
        }
    }

    /// The mandatory prefix a floating-point instruction carries: `F2` for `double`, `F3` for
    /// `float`.
    fn float_prefix(&mut self, size: u8) {
        self.byte(if size == 8 { 0xF2 } else { 0xF3 });
    }

    /// `dst = left OP right` on XMM registers, in the type's width.
    fn float_binary(&mut self, dst: VReg, op: BinOp, left: VReg, right: VReg, size: u8) {
        let left_register = self.read_float(left);
        let right_register = self.read_float(right);
        // SSE's two-operand form is `op xmm1, xmm2/m`: the *register* field is the destination and
        // the r/m field the source, which is the opposite way round from the integer operations.
        let destination = self.float_destination(dst);
        // When the destination is memory its spare is `xmm0`, and `left` may already be in it: then
        // it *is* the accumulator, and asking for a third scratch register - which does not exist -
        // would be the alternative.
        let spare_holds_left =
            self.scratches.contains(&destination) && destination == left_register;
        let (target, source) = if self.is_in_float(dst, left_register) || spare_holds_left {
            (left_register, right_register)
        } else if matches!(op, BinOp::Add | BinOp::Mul) && self.is_in_float(dst, right_register) {
            // Adding and multiplying commute, so the destination's register can hold either operand.
            (right_register, left_register)
        } else if destination != left_register && destination != right_register {
            self.float_move(destination, left_register, 0, size, Move::Register);
            (destination, right_register)
        } else {
            // The destination's register holds an operand that is still live - the register was
            // freed by *another* interval ending here - so copying into it would destroy that
            // operand before the operation reads it.
            let scratch = self.float_scratch();
            self.float_move(scratch, left_register, 0, size, Move::Register);
            (scratch, right_register)
        };
        self.float_prefix(size);
        self.prefix(target, source);
        self.bytes(&[0x0F, float_opcode(op)]);
        self.modrm(3, target.0, source);
        self.write_float(dst, target);
        self.release();
    }

    /// A floating-point comparison, answering 1 or 0 in a general register.
    ///
    /// `ucomisd` sets the flags as an unsigned comparison does, and a NaN sets carry and parity
    /// together. C says a comparison involving a NaN is false except `!=`, so the two orders a NaN
    /// would otherwise pass - `setb` and `setbe` - are avoided by asking whether the *second* operand
    /// is the greater one; `==` and `!=` are asked directly and corrected with the parity flag.
    fn float_compare(
        &mut self,
        dst: VReg,
        op: crate::cir::CmpOp,
        left: VReg,
        right: VReg,
        size: u8,
    ) {
        let left = self.read_float(left);
        let right = self.read_float(right);
        use crate::cir::CmpOp;
        let (first, second, condition) = match op {
            CmpOp::Less => (right, left, 7),         // seta
            CmpOp::LessEqual => (right, left, 3),    // setae
            CmpOp::Greater => (left, right, 7),      // seta
            CmpOp::GreaterEqual => (left, right, 3), // setae
            CmpOp::Eq => (left, right, 4),           // sete, with parity
            CmpOp::Ne => (left, right, 5),           // setne, with parity
        };
        // `ucomisd` carries `66` and `ucomiss` carries nothing: this is the one floating-point
        // instruction whose prefix is not the arithmetic one, and `F2 0F 2E` is not an instruction
        // at all - which the processor says by refusing to run it.
        if size == 8 {
            self.byte(0x66);
        }
        // The REX byte names a register above `xmm7`, and without it the operand is read as
        // `xmm0`-`xmm7`: a silent wrong answer rather than a fault, because both are valid.
        self.prefix(first, second);
        self.bytes(&[0x0F, 0x2E]); // ucomisd / ucomiss
        self.modrm(3, first.0, second);
        self.byte(0x40);
        self.bytes(&[0x0F, 0x90 | condition]);
        self.modrm(3, 0, Reg::RAX);
        if matches!(op, CmpOp::Eq | CmpOp::Ne) {
            // Equality is "equal *and* not unordered", so it asks for `setnp`; inequality is
            // "unequal *or* unordered", so it asks for `setp`. Using `setp` for both would make every
            // equal comparison false, which is a quiet way to lose an `if`.
            let parity = if op == CmpOp::Eq { 0x9B } else { 0x9A }; // setnp / setp
            self.bytes(&[0x0F, parity]);
            self.modrm(3, 1, Reg::RCX);
            let opcode = if op == CmpOp::Eq { 0x20 } else { 0x08 }; // and al, cl / or al, cl
            self.bytes(&[opcode, 0xC8]);
        }
        self.bytes(&[0x48, 0x0F, 0xB6, 0xC0]); // movzx rax, al
        self.write(dst, Reg::RAX);
        self.release();
    }

    /// A floating-point constant, whose bits travel through a general register.
    fn float_const(&mut self, dst: VReg, bits: u64, size: u8) {
        let target = self.float_destination(dst);
        self.mov_imm(Reg::RAX, bits);
        self.byte(0x66); // the mandatory prefix, before the REX byte
        if size == 8 {
            self.prefix_w(target, Reg::RAX);
            self.bytes(&[0x0F, 0x6E]); // movq xmm, rax
        } else {
            self.prefix(target, Reg::RAX);
            self.bytes(&[0x0F, 0x6E]); // movd xmm, eax
        }
        self.modrm(3, target.0, Reg::RAX);
        self.write_float(dst, target);
        self.release();
    }

    /// A conversion involving a floating-point type: between widths, from an integer, or to one.
    #[allow(clippy::too_many_arguments)] // the shape of a conversion is this long
    fn float_convert(
        &mut self,
        dst: VReg,
        src: VReg,
        from: super::Class,
        to: super::Class,
        from_size: u8,
        to_size: u8,
        signed: bool,
    ) {
        match (from, to) {
            (super::Class::Float, super::Class::Float) => {
                let target = self.float_destination(dst);
                let source = self.read_float(src);
                if target != source {
                    self.float_move(target, source, 0, 8, Move::Register);
                }
                if from_size == 4 && to_size == 8 {
                    self.byte(0xF3);
                    self.prefix(target, target);
                    self.bytes(&[0x0F, 0x5A]); // cvtss2sd
                    self.modrm(3, target.0, target);
                } else if from_size == 8 && to_size == 4 {
                    self.byte(0xF2);
                    self.prefix(target, target);
                    self.bytes(&[0x0F, 0x5A]); // cvtsd2ss
                    self.modrm(3, target.0, target);
                }
                // The source and destination are the same register here, and a register above
                // `xmm7` is only nameable through a REX byte.
                self.write_float(dst, target);
            }
            (super::Class::Integer, super::Class::Float) => {
                let target = self.float_destination(dst);
                // A 32-bit integer is extended to 64 first, so one conversion serves both widths,
                // and an unsigned 32-bit one is already a positive 64-bit value.
                let source = self.read(src);
                self.extend_to_whole(source, from_size, signed);
                self.float_prefix(to_size);
                // The REX byte carries the 64-bit operand size, without which the instruction would
                // read the 32-bit half of a register, and the high bits of both registers.
                self.prefix_w(target, source);
                self.bytes(&[0x0F, 0x2A]); // cvtsi2sd / cvtsi2ss
                self.modrm(3, target.0, source);
                self.write_float(dst, target);
            }
            (super::Class::Integer, super::Class::Integer) => {
                unreachable!("an integer conversion is not a floating-point one")
            }
            (super::Class::Float, super::Class::Integer) => {
                let target = self.destination(dst);
                let source = self.read_float(src);
                self.float_prefix(from_size);
                // A 64-bit destination, for the same reason as the conversion the other way.
                self.prefix_w(target, source);
                self.bytes(&[0x0F, 0x2C]); // cvttsd2si / cvttss2si
                self.modrm(3, target.0, source);
                // The conversion fills a whole 64-bit register; a narrower C type keeps its low
                // bytes, which is what the canonical form does next.
                self.canonicalise(target, to_size, signed);
                self.write(dst, target);
            }
        }
        self.release();
    }

    /// Whether a value lives in this XMM register.
    fn is_in_float(&self, value: VReg, register: Reg) -> bool {
        matches!(self.allocation.location(value), Location::Float(other) if other == register)
    }

    /// Extend a value of `width` bytes to the whole register, with the type's sign; nothing at all
    /// when it is already whole.
    fn extend_to_whole(&mut self, register: Reg, width: u8, signed: bool) {
        match (width, signed) {
            (8, _) => {}
            (4, true) => self.widen_reg(register, &[0x63]),
            (4, false) => {
                self.prefix(register, register);
                self.byte(0x89);
                self.modrm(3, register.0, register);
            }
            (2, true) => self.widen_reg(register, &[0x0F, 0xBF]),
            (2, false) => self.widen_reg(register, &[0x0F, 0xB7]),
            (1, true) => self.widen_reg(register, &[0x0F, 0xBE]),
            (1, false) => self.widen_reg(register, &[0x0F, 0xB6]),
            (other, _) => unreachable!("extending a {other}-byte value"),
        }
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
            Location::Float(_) => unreachable!("an integer result in an XMM register"),
        }
    }

    /// The same for a floating-point result, whose spare is `xmm0`.
    ///
    /// A value in a *general* register here is a mistake in the lowering's classes, and using that
    /// register's number as an XMM number would address a different register quietly - which is how
    /// a float load read the wrong one once already.
    fn float_destination(&mut self, dst: VReg) -> Reg {
        match self.allocation.location(dst) {
            Location::Float(register) => register,
            Location::Spill(_) => {
                // Claimed like any other register the operation is holding: no operand read for the
                // same operation may be handed the register the answer is computed in.
                if !self.scratches.contains(&FLOAT_RESULT) {
                    self.scratches.push(FLOAT_RESULT);
                }
                FLOAT_RESULT
            }
            Location::Register(_) => unreachable!("a floating-point result in a general register"),
        }
    }

    /// Where a floating-point value lives, in an XMM register: a spilled one is loaded for this use.
    fn read_float(&mut self, value: VReg) -> Reg {
        match self.allocation.location(value) {
            Location::Float(register) => register,
            Location::Register(_) => unreachable!("an integer value read as floating point"),
            Location::Spill(offset) => {
                let scratch = self.float_scratch();
                self.float_move(scratch, Reg::RSP, self.spill_base + offset, 8, Move::Load);
                scratch
            }
        }
    }

    /// Write a floating-point result back to where it lives.
    fn write_float(&mut self, dst: VReg, register: Reg) {
        match self.allocation.location(dst) {
            Location::Float(target) => {
                if target != register {
                    self.float_move(target, register, 0, 8, Move::Register);
                }
            }
            Location::Register(_) => {
                unreachable!("a floating-point value written to a general one")
            }
            Location::Spill(offset) => {
                self.float_move(register, Reg::RSP, self.spill_base + offset, 8, Move::Store);
            }
        }
    }

    /// The spare XMM register an operation uses for a spilled operand: `xmm0` first, then `xmm1`.
    fn float_scratch(&mut self) -> Reg {
        let register = if !self.scratches.contains(&FLOAT_SCRATCH) {
            FLOAT_SCRATCH
        } else if !self.scratches.contains(&FLOAT_SECOND) {
            FLOAT_SECOND
        } else {
            unreachable!("an operation is holding both floating-point scratch registers")
        };
        self.scratches.push(register);
        register
    }

    /// Where a value lives, in a register: a spilled one is loaded into a scratch for this use.
    fn read(&mut self, value: VReg) -> Reg {
        match self.allocation.location(value) {
            Location::Register(register) => register,
            Location::Float(_) => unreachable!("a floating-point value read as an integer"),
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
            // A general-register path never receives a floating-point value; the float paths read
            // their own operands. Reaching here with one is a bug in the lowering's classes.
            Location::Float(_) => {
                unreachable!("an integer operation reads a floating-point value")
            }
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

    /// Put the arguments where the call will find them: the first four in `rcx`, `rdx`, `r8` and
    /// `r9`, the rest in the outgoing slots. One place, because a direct call and a call through a
    /// pointer pass their arguments the same way, and two copies of this would drift.
    ///
    /// A *variadic* call also copies each floating-point argument into the general register of its
    /// own position: Win64 requires it, and it is what makes one eight-byte-per-argument walk read a
    /// list that mixes the two classes (ADR-006). Only the staged path has a copy to make - the direct
    /// one is all-integer by its own test - and the copy is the same eight bytes the double was just
    /// loaded from, so it costs a load and never a store.
    fn place_arguments(&mut self, args: &[VReg], variadic: bool) {
        // Where each argument goes: **the position picks the register**, and the argument's class
        // picks which file it is in. `f(int a, double b, double c)` has `b` in `xmm1` and `c` in
        // `xmm2`, not `xmm0` and `xmm1` - which is what clang's own output says, and what a per-class
        // counter gets wrong while every test that compiles both sides itself still passes.
        let mut plan = Vec::with_capacity(args.len());
        for (position, argument) in args.iter().enumerate() {
            let class = self.class_of(*argument);
            let target = if position < super::ARGUMENT_REGISTERS.len() {
                match class {
                    super::Class::Integer => Target::Integer(super::ARGUMENT_REGISTERS[position]),
                    super::Class::Float => Target::Float(super::FLOAT_ARGUMENTS[position]),
                }
            } else {
                let slot = position - super::ARGUMENT_REGISTERS.len();
                Target::Stack(self.outgoing_base + 8 * slot as i32)
            };
            plan.push((*argument, class, target));
        }
        // The stack arguments go first: a store cannot disturb a register, while a register move
        // could disturb a value that has not been stored yet.
        for (argument, class, target) in &plan {
            let Target::Stack(offset) = target else {
                continue;
            };
            match class {
                super::Class::Integer => {
                    let register = self.read(*argument);
                    self.mov_memory_from_reg(*offset, register);
                }
                super::Class::Float => {
                    let register = self.read_float(*argument);
                    self.float_move(register, Reg::RSP, *offset, 8, Move::Store);
                }
            }
            self.release();
        }
        // Nothing collides, and it cannot: a function that makes a call allocates only callee-saved
        // registers - `Registers` gives the caller-saved ones to functions that call nothing - so no
        // argument's value is ever in one of the registers the arguments have to go to. The staging
        // area this used to need, and the second pass that filled it, are gone; the assertion is here
        // so that a future pool which allows an argument register says so instead of quietly
        // clobbering an argument.
        debug_assert!(
            !self.arguments_collide(&plan),
            "an argument's value is in another argument's register: the staging area is needed again"
        );
        for (position, (argument, class, target)) in plan.iter().enumerate() {
            match (class, target) {
                (super::Class::Integer, Target::Integer(register)) => {
                    self.read_into(*argument, *register);
                }
                (super::Class::Float, Target::Float(register)) => {
                    let source = self.read_float(*argument);
                    if source != *register {
                        self.float_move(*register, source, 0, 8, Move::Register);
                    }
                    if variadic {
                        // The same sixty-four bits, into the general register of the same position:
                        // Win64's duplicate, which is what lets a callee read the list one eight-byte
                        // slot at a time (ADR-006).
                        self.integer_from_float(super::ARGUMENT_REGISTERS[position], source);
                    }
                }
                // A stack argument is already where it belongs.
                (_, Target::Stack(_)) => {}
                (class, target) => unreachable!("a {class:?} argument at {target:?}"),
            }
            self.release();
        }
    }

    /// What a call leaves behind: its result in `rax`, with the upper bits of a narrow one
    /// unspecified, so they are canonicalised rather than trusted. A floating-point result comes back
    /// in `xmm0`, which is the ABI round this back end has not finished.
    fn finish_call(&mut self, dst: Option<VReg>, size: u8, signed: bool) {
        let Some(dst) = dst else { return };
        if self.class_of(dst) == super::Class::Float {
            let register = self.float_destination(dst);
            self.float_move(register, super::FLOAT_ARGUMENTS[0], 0, size, Move::Register);
            self.write_float(dst, register);
        } else {
            self.canonicalise(Reg::RAX, size, signed);
            self.write(dst, Reg::RAX);
        }
    }

    /// `call r/m64`: the same call as a direct one, with the target in a register. `/2` is the
    /// indirect form, and the register is the r/m field.
    fn call_through(&mut self, target: Reg) {
        self.prefix(Reg(0), target);
        self.byte(0xFF);
        self.modrm(3, 2, target);
    }

    /// Which register file a value belongs to, from the function's table.
    fn class_of(&self, value: VReg) -> super::Class {
        self.classes
            .get(value.0 as usize)
            .copied()
            .unwrap_or(super::Class::Integer)
    }

    /// Whether moving the arguments into their registers, in order, can destroy one that has not
    /// been read yet: an argument whose register is a *later* argument's register.
    /// Whether putting the arguments in their registers would destroy one another: an argument whose
    /// value already sits in the register that *another* position needs is the case, and the answer
    /// decides whether the staging area is used at all.
    ///
    /// Each file is checked against its own, because they are disjoint - an integer in `rdx` is not
    /// disturbed by a `double` going to `xmm1` - and the corpus has no call in which either file
    /// collides, so the staging area is what pays when one does. A spilled argument is read straight
    /// from its slot into the register that needs it, so it is never in the way.
    fn arguments_collide(&self, plan: &[(VReg, super::Class, Target)]) -> bool {
        plan.iter()
            .enumerate()
            .any(|(position, (argument, class, target))| {
                let held = match self.allocation.location(*argument) {
                    Location::Register(register) => register,
                    Location::Float(register) => register,
                    Location::Spill(_) => return false,
                };
                let file: &[Reg] = match (class, target) {
                    (super::Class::Integer, Target::Integer(_)) => &super::ARGUMENT_REGISTERS,
                    (super::Class::Float, Target::Float(_)) => &super::FLOAT_ARGUMENTS,
                    // A stack argument is written before any register is touched, so it is read only.
                    _ => return false,
                };
                file.iter()
                    .position(|register| *register == held)
                    .is_some_and(|other| other != position)
            })
    }

    /// `movq r64, xmm`: the value's sixty-four bits, into a general register. Used for the variadic
    /// duplicate, where the same `double` has to reach a callee that reads the general file.
    fn integer_from_float(&mut self, to: Reg, from: Reg) {
        self.byte(0x66);
        // The prefix takes the same two registers `modrm` does, in the same order: `0F 7E` names the
        // XMM register in the *reg* field and the general one in the *r/m* field, so `R` belongs to
        // `from` and `B` to `to`. Swapping them here emits `movq %xmm7, %r???` - a register the
        // instruction never named - and it is the fifth time this back end has paid for that order.
        self.prefix_w(from, to);
        self.bytes(&[0x0F, 0x7E]);
        self.modrm(3, from.0, to);
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
            Location::Float(_) => unreachable!("a floating-point value written by an integer op"),
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

    /// One of two blocks, whichever falls through. The common shape is one side falling through, and
    /// testing for that side first keeps an extra jump out of the instruction stream.
    fn branch_either(
        &mut self,
        taken: u8,
        not_taken: u8,
        then: usize,
        otherwise: usize,
        index: usize,
    ) {
        if otherwise == index + 1 {
            self.branch(then, taken);
        } else if then == index + 1 {
            self.branch(otherwise, not_taken);
        } else {
            self.branch(then, taken);
            self.jump(otherwise);
        }
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

    /// A constant, in the cheapest form that is *exactly* the value asked for.
    ///
    /// This is the largest single use of code in the whole back end - 37.7% of the bytes over the
    /// corpus, because folding turns initialisers and their address arithmetic into constants - and
    /// 97% of those constants are zero or a byte, so the choice of form is worth more than any pass.
    /// A value comes here canonical for its C type, which is what makes the short forms safe: a `u64`
    /// whose top half is clear is the zero-extension of its low half, so a 32-bit move writes it whole.
    fn mov_imm(&mut self, to: Reg, value: u64) {
        if value == 0 {
            // `xor r32, r32` is two bytes and sets all sixty-four of them. Nothing holds flags across
            // an operation - a comparison computes its answer inside its own encoding - so clearing
            // them here costs nothing. Both halves name `to`, and in `31 /r` the register is in the
            // *modrm* field, so the REX prefix has to be told about it twice: naming an extended
            // register through `B` alone gives `xor r8d, eax`, which is a wrong answer that runs.
            self.prefix(to, to);
            self.byte(0x31);
            self.modrm(3, to.0, to);
        } else if let Ok(immediate) = u32::try_from(value) {
            // `mov r32, imm32` zero-extends: five bytes instead of seven, and the value is unchanged
            // because a non-negative constant's canonical form *is* its zero-extension.
            self.prefix(Reg(0), to);
            self.byte(0xB8 | (to.0 & 7));
            self.bytes(&immediate.to_le_bytes());
        } else if let Ok(immediate) = i32::try_from(value as i64) {
            // `mov r64, imm32` sign-extends, so it holds anything whose pattern survives that
            // extension: every constant that came from a C `int`, and `-1`, and small pointers.
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

    /// Say that a variadic call used no vector registers. This back end passes each floating-point
    /// argument in *both* files - the vector register the ABI names and the general register of its
    /// position - so the count in `AL` that makes a callee read the copies is zero, and no vector save
    /// area has to exist at all. `clang` leaves `AL` unspecified at such a call and relies on the same
    /// duplicate; a zero is the same answer made deterministic, which is the difference between
    /// working by luck and working.
    fn no_vector_registers_used(&mut self) {
        self.byte(0xB0); // mov al, imm8
        self.byte(0);
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

    /// `sub rsp, bytes`, in the four-byte form or the one-byte form: most frames are small, and six
    /// bytes a function is six bytes over five hundred functions.
    fn sub_rsp(&mut self, bytes: i32) {
        self.adjust_rsp(5, bytes);
    }

    /// What the pushes move `rsp` by, which is the part of `frame` the subtraction does not have to.
    fn pushed(&self) -> i32 {
        8 * self.allocation.saved.len() as i32
    }

    /// `push r64`: one byte for the first eight registers, two for the rest, and it does the
    /// bookkeeping - `rsp` moves, which is why the frame's subtraction is that much shorter.
    fn push(&mut self, register: Reg) {
        self.prefix(Reg(0), register);
        self.byte(0x50 | (register.0 & 7));
    }

    /// `pop r64`, the same in reverse.
    fn pop(&mut self, register: Reg) {
        self.prefix(Reg(0), register);
        self.byte(0x58 | (register.0 & 7));
    }

    /// `add rsp, bytes`, likewise.
    fn add_rsp(&mut self, bytes: i32) {
        self.adjust_rsp(0, bytes);
    }

    /// `83 /ext ib` when the amount fits in a signed byte, `81 /ext id` otherwise: the same
    /// instruction, and the assembler this back end writes for itself takes the shorter form.
    fn adjust_rsp(&mut self, extension: u8, bytes: i32) {
        self.prefix_w(Reg(0), Reg::RSP);
        if let Ok(small) = i8::try_from(bytes) {
            self.byte(0x83);
            self.modrm(3, extension, Reg::RSP);
            self.byte(small as u8);
        } else {
            self.byte(0x81);
            self.modrm(3, extension, Reg::RSP);
            self.bytes(&bytes.to_le_bytes());
        }
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

/// The `/ext` field of the `81`/`83` group for an operation that has one, or `None` for one whose
/// encoding has no immediate field: `mul` is the two-operand `0F AF`, and a shift's count is a
/// register.
///
/// The lowering asks this same question when it decides whether a constant may travel inside the
/// instruction instead of being materialised. One answer, so the two cannot disagree about which
/// operations the encoder can be asked for.
pub(crate) fn immediate_extension(op: BinOp) -> Option<u8> {
    match op {
        BinOp::Add => Some(0),
        BinOp::Or => Some(1),
        BinOp::And => Some(4),
        BinOp::Sub => Some(5),
        BinOp::Xor => Some(6),
        _ => None,
    }
}

/// The `0F 5x` opcode of a floating-point operation: the same for both widths, which the mandatory
/// prefix chooses.
fn float_opcode(op: BinOp) -> u8 {
    match op {
        BinOp::Add => 0x58,
        BinOp::Mul => 0x59,
        BinOp::Sub => 0x5C,
        BinOp::Div => 0x5E,
        other => unreachable!("{other:?} is not a floating-point operation"),
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
