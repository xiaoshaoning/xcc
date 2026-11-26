//! `cir`: the intermediate form the front end produces and the back ends consume.
//!
//! ADR-002 has the reasoning; this file is the shape. A function is a control-flow graph of
//! blocks that take parameters, and every value is defined in exactly one place. There are no
//! phi nodes: a value a block receives from its predecessors is one of that block's parameters.
//!
//! Four rules hold everywhere here:
//!
//! - Only [`Builder`] mints a [`Value`]. A caller cannot invent one, so a value is defined once.
//! - Memory is explicit: [`Inst::Load`] and [`Inst::Store`], with the C type they move.
//! - Every instruction carries a [`Span`], and every value its C [`TypeId`]: those are what let a
//!   diagnostic point at the expression that produced a bad instruction.
//! - Nothing here names a register, a calling convention or a Cranelift type. Those live in
//!   `back/` (ADR-002, "What `cir` may not contain").

#![allow(dead_code)] // the dump reads some of these, the back end the rest

use crate::diag::Span;
use crate::intern::Symbol;
pub mod dump;
pub mod pass;

use crate::types::{TypeId, Types};

/// A value: an SSA name minted by [`Builder`], defined in exactly one block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Value(pub(crate) u32);

/// Which block defines a value, so a dump and a verifier can find it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValueDef {
    /// The C type of the value.
    pub ty: TypeId,
    /// The block that defines it.
    pub block: Block,
}

/// A basic block: its parameters, its instructions, and how it ends.
#[derive(Debug, Clone)]
pub struct BlockData {
    /// The values the block receives from its predecessors.
    pub params: Vec<Value>,
    /// What happens, in order.
    pub insts: Vec<Inst>,
    /// How the block ends. `None` only while the builder is filling it in.
    pub term: Option<Term>,
    /// Where the block starts in the source, for a diagnostic.
    pub span: Span,
}

/// Which segment a segment-relative read comes from: the SDK's `__readfs*` and `__readgs*`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Segment {
    Fs,
    Gs,
}

/// A block handle: an index into [`Func::blocks`]. Only a builder makes one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Block(pub(crate) u32);

/// A binary operation on two values of one type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `/`, signed or unsigned by the operands' type.
    Div,
    /// `%`, signed or unsigned by the operands' type.
    Rem,
    /// `&`
    And,
    /// `|`
    Or,
    /// `^`
    Xor,
    /// `<<`
    Shl,
    /// `>>`, arithmetic or logical by the operands' type.
    Shr,
}

/// A comparison of two values of one type; the result is an `int`, as C has it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    /// `==`
    Eq,
    /// `!=`
    Ne,
    /// `<`
    Less,
    /// `<=`
    LessEqual,
    /// `>`
    Greater,
    /// `>=`
    GreaterEqual,
}

/// An operation on one value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    /// Unary `-`.
    Neg,
    /// `~`.
    Not,
}

/// A conversion. C's rules decide *which* one applies; this only says which.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConvertOp {
    /// Keep the value, change the type: `int` to `unsigned int`.
    Bitcast,
    /// Keep the low bits.
    Truncate,
    /// Widen, filling with the sign bit.
    SignExtend,
    /// Widen, filling with zeroes.
    ZeroExtend,
    /// An integer to a `float` or `double`.
    IntToFloat,
    /// A `float` or `double` to an integer.
    FloatToInt,
    /// `float` to `double`.
    FloatExtend,
    /// `double` to `float`.
    FloatTruncate,
}

impl ConvertOp {
    /// How a dump writes it.
    pub fn as_str(self) -> &'static str {
        match self {
            ConvertOp::Bitcast => "bitcast",
            ConvertOp::Truncate => "trunc",
            ConvertOp::SignExtend => "sext",
            ConvertOp::ZeroExtend => "zext",
            ConvertOp::IntToFloat => "sitofp",
            ConvertOp::FloatToInt => "fptosi",
            ConvertOp::FloatExtend => "fpext",
            ConvertOp::FloatTruncate => "fptrunc",
        }
    }
}

impl BinOp {
    /// How a dump writes it.
    pub fn as_str(self) -> &'static str {
        match self {
            BinOp::Add => "add",
            BinOp::Sub => "sub",
            BinOp::Mul => "mul",
            BinOp::Div => "div",
            BinOp::Rem => "rem",
            BinOp::And => "and",
            BinOp::Or => "or",
            BinOp::Xor => "xor",
            BinOp::Shl => "shl",
            BinOp::Shr => "shr",
        }
    }
}

impl CmpOp {
    /// How a dump writes it.
    pub fn as_str(self) -> &'static str {
        match self {
            CmpOp::Eq => "eq",
            CmpOp::Ne => "ne",
            CmpOp::Less => "lt",
            CmpOp::LessEqual => "le",
            CmpOp::Greater => "gt",
            CmpOp::GreaterEqual => "ge",
        }
    }
}

impl UnOp {
    /// How a dump writes it.
    pub fn as_str(self) -> &'static str {
        match self {
            UnOp::Neg => "neg",
            UnOp::Not => "not",
        }
    }
}

/// One instruction. Each variant that has a result says which value it defined, so a value's
/// definition is recorded in one place and a dump can print `v3 = add v0, v1`.
#[derive(Debug, Clone)]
pub enum Inst {
    /// Read a value of type `ty` from `address`.
    Load {
        /// The value read.
        result: Value,
        /// The type read.
        ty: TypeId,
        /// The address: a pointer.
        address: Value,
        /// Where in the source.
        span: Span,
    },
    /// Write `value` at `address`.
    Store {
        /// The address: a pointer to the value's type.
        address: Value,
        /// What to write.
        value: Value,
        /// Where in the source.
        span: Span,
    },
    /// Two values, one operation.
    Binary {
        /// The value produced.
        result: Value,
        /// Which operation.
        op: BinOp,
        /// The type of the operands and of the result.
        ty: TypeId,
        /// The left operand.
        left: Value,
        /// The right operand.
        right: Value,
        /// Where in the source.
        span: Span,
    },
    /// A comparison, which produces an `int`: 1 or 0.
    Compare {
        /// The value produced.
        result: Value,
        /// Which comparison.
        op: CmpOp,
        /// The type of the operands.
        ty: TypeId,
        /// The left operand.
        left: Value,
        /// The right operand.
        right: Value,
        /// Where in the source.
        span: Span,
    },
    /// A read of a segment-relative word: `mov rax, gs:[offset]`. See [`Builder::segment_read`].
    SegmentRead {
        result: Value,
        segment: Segment,
        offset: u32,
        ty: TypeId,
        span: Span,
    },
    /// A compare-and-swap: the one machine operation atomic arithmetic is built from.
    ///
    /// `pointer` is the address of an object of `ty`; if its value equals `expected` it becomes
    /// `desired`, and the value *read* is the result - which is `expected` when the swap happened.
    /// Every other atomic operation is a loop of these, written in C, so this is the only place the
    /// machine has to be asked for something atomic.
    AtomicCas {
        /// The old value of the object.
        result: Value,
        /// Where the object is.
        pointer: Value,
        /// What it is expected to hold.
        expected: Value,
        /// What it should hold afterwards.
        desired: Value,
        /// The type of the object, which is what width the operation has.
        ty: TypeId,
        /// Where in the source.
        span: Span,
    },
    /// One value, one operation.
    Unary {
        /// The value produced.
        result: Value,
        /// Which operation.
        op: UnOp,
        /// The type of the operand and of the result.
        ty: TypeId,
        /// The operand.
        value: Value,
        /// Where in the source.
        span: Span,
    },
    /// A conversion between C types, decided by the front end.
    Convert {
        /// The value produced.
        result: Value,
        /// Which conversion.
        op: ConvertOp,
        /// The type converted from.
        from: TypeId,
        /// The type converted to.
        to: TypeId,
        /// The value converted.
        value: Value,
        /// Where in the source.
        span: Span,
    },
    /// Call `callee` (a pointer to a function) with `args`.
    Call {
        /// The value produced, when the function returns one.
        result: Option<Value>,
        /// The function to call.
        callee: Value,
        /// The arguments: the ones the signature declares, converted to their parameter types,
        /// then any the variadic form adds, which `call` has already promoted.
        args: Vec<Value>,
        /// Whether the call passes more arguments than the function declares. A variadic call has
        /// no single signature - each call site has its own - so the back end builds one from the
        /// arguments here rather than from the declaration (ADR-006).
        variadic: bool,
        /// Where in the source.
        span: Span,
    },
    /// A constant of type `ty`: `bits` interpreted by the type, as a C literal is.
    Const {
        /// The value produced.
        result: Value,
        /// The type of the constant.
        ty: TypeId,
        /// The bits: the integer value, or the float's bit pattern.
        bits: u64,
        /// Where in the source.
        span: Span,
    },
    /// Space on the stack for a C variable; the result is a pointer to it. Local variables are
    /// memory until `mem2reg` proves their address never escapes (ADR-002 invariant 3).
    Slot {
        /// The pointer produced.
        result: Value,
        /// The type of what the slot holds.
        ty: TypeId,
        /// The alignment C11's `_Alignas` asked for, when it asked for one: zero means the type's
        /// own. It is the object's alignment, not its type's - `_Alignas(16) int x` is a sixteen-byte
        /// object holding an `int` - so it cannot live in the type.
        alignas: u64,
        /// Where in the source.
        span: Span,
    },
    /// The address of the first *unnamed* argument of a variadic function: `skip` names the
    /// parameters before the `...`, so the address is that many argument slots past the home area.
    ///
    /// This is the one `cir` operation that does not describe a C construct. C's `va_start` is a macro
    /// over an implementation-defined type, and what it has to say is "where the arguments that are not
    /// parameters begin", which is a fact about the calling convention. A back end with no home area to
    /// point at - Cranelift, whose arguments live in whatever register its allocator chose - cannot
    /// implement it and says so rather than guessing (ADR-006).
    VaStart {
        /// The pointer produced: a `char *` that walks the argument slots.
        result: Value,
        /// How many named parameters come before the `...`.
        skip: u8,
        /// Where in the source.
        span: Span,
    },
    /// Read one argument through a `va_list`, by the type the macro was given. It does not advance the
    /// list: C's `va_arg` is an expression that yields a value, and the front end writes the step back
    /// into the `va_list` lvalue, so the advance is a store like any other.
    VaArg {
        /// The value read.
        result: Value,
        /// The list to read through: a `char *`.
        list: Value,
        /// The type read, which decides the width and the sign extension.
        ty: TypeId,
        /// Where in the source.
        span: Span,
    },
    /// The address of a global object or a function, by the name its object file gives it.
    GlobalAddress {
        /// The pointer produced.
        result: Value,
        /// The name of the global: the link name, which for an anonymous object the compiler
        /// generated and the source never spelled (ADR-004: a name from the source is a
        /// `Symbol`, a name the compiler makes is a `String`).
        name: String,
        /// Where in the source.
        span: Span,
    },
}

impl Inst {
    /// The values this instruction reads, in order: the same list [`used_values`] gives the
    /// verifier, so a pass and the checker cannot disagree about what an operand is.
    pub(crate) fn operands(&self) -> Vec<Value> {
        used_values(self)
    }

    /// Rewrite the values this instruction reads; its own result is untouched.
    fn rewrite_values(&mut self, rewrite: &mut impl FnMut(Value) -> Value) {
        match self {
            Inst::Load { address, .. } => *address = rewrite(*address),
            Inst::Store { address, value, .. } => {
                *address = rewrite(*address);
                *value = rewrite(*value);
            }
            Inst::Binary { left, right, .. } => {
                *left = rewrite(*left);
                *right = rewrite(*right);
            }
            Inst::Compare { left, right, .. } => {
                *left = rewrite(*left);
                *right = rewrite(*right);
            }
            // A segment read names an *offset*, not a value, so there is nothing to rewrite.
            Inst::SegmentRead { .. } => {}
            Inst::AtomicCas {
                pointer,
                expected,
                desired,
                ..
            } => {
                *pointer = rewrite(*pointer);
                *expected = rewrite(*expected);
                *desired = rewrite(*desired);
            }
            Inst::Unary { value, .. } | Inst::Convert { value, .. } => *value = rewrite(*value),
            Inst::Call { callee, args, .. } => {
                *callee = rewrite(*callee);
                for arg in args {
                    *arg = rewrite(*arg);
                }
            }
            Inst::Slot { .. }
            | Inst::Const { .. }
            | Inst::VaStart { .. }
            | Inst::GlobalAddress { .. } => {}
            Inst::VaArg { list, .. } => *list = rewrite(*list),
        }
    }

    /// The value this instruction defines, if it defines one.
    pub fn result(&self) -> Option<Value> {
        match self {
            Inst::Load { result, .. }
            | Inst::Binary { result, .. }
            | Inst::Compare { result, .. }
            | Inst::AtomicCas { result, .. }
            | Inst::SegmentRead { result, .. }
            | Inst::Unary { result, .. }
            | Inst::Convert { result, .. }
            | Inst::Slot { result, .. }
            | Inst::Const { result, .. }
            | Inst::VaStart { result, .. }
            | Inst::VaArg { result, .. }
            | Inst::GlobalAddress { result, .. } => Some(*result),
            Inst::Call { result, .. } => *result,
            Inst::Store { .. } => None,
        }
    }

    /// Where in the source this instruction came from.
    pub fn span(&self) -> Span {
        match self {
            Inst::Load { span, .. }
            | Inst::Store { span, .. }
            | Inst::Binary { span, .. }
            | Inst::Compare { span, .. }
            | Inst::AtomicCas { span, .. }
            | Inst::SegmentRead { span, .. }
            | Inst::Unary { span, .. }
            | Inst::Convert { span, .. }
            | Inst::Slot { span, .. }
            | Inst::Const { span, .. }
            | Inst::Call { span, .. }
            | Inst::VaStart { span, .. }
            | Inst::VaArg { span, .. }
            | Inst::GlobalAddress { span, .. } => *span,
        }
    }
}

impl Term {
    /// Rewrite the values this terminator reads; a target's parameters are untouched.
    fn rewrite_values(&mut self, rewrite: &mut impl FnMut(Value) -> Value) {
        match self {
            Term::Return { value, .. } => {
                if let Some(value) = value {
                    *value = rewrite(*value);
                }
            }
            Term::Jump { args, .. } => {
                for arg in args {
                    *arg = rewrite(*arg);
                }
            }
            Term::Branch {
                condition,
                then,
                otherwise,
                ..
            } => {
                *condition = rewrite(*condition);
                for arg in &mut then.1 {
                    *arg = rewrite(*arg);
                }
                for arg in &mut otherwise.1 {
                    *arg = rewrite(*arg);
                }
            }
            Term::Unreachable { .. } | Term::Trap { .. } => {}
        }
    }
}

/// How a block ends. A jump carries the values its target's parameters receive, so a merge point
/// needs nothing but the block's parameter list.
#[derive(Debug, Clone)]
pub enum Term {
    /// Leave the function, with a value unless it returns `void`.
    Return {
        /// The value returned.
        value: Option<Value>,
        /// Where in the source.
        span: Span,
    },
    /// Go to `target`, giving it these arguments.
    Jump {
        /// Where to go.
        target: Block,
        /// What its parameters receive, in order.
        args: Vec<Value>,
        /// Where in the source.
        span: Span,
    },
    /// Two ways out, chosen by `condition`: non-zero is true, as in C.
    Branch {
        /// The value tested: an integer or a pointer.
        condition: Value,
        /// Where a non-zero condition goes.
        then: (Block, Vec<Value>),
        /// Where a zero condition goes.
        otherwise: (Block, Vec<Value>),
        /// Where in the source.
        span: Span,
    },
    /// Nothing follows: no path leads here. A block the front end opened and did not fill - the
    /// code after an `if` whose every arm returned - ends this way.
    ///
    /// This and [`Term::Trap`] both encode as `ud2`, and they are not the same fact. `Unreachable`
    /// says *nothing can arrive*; `Trap` says *arriving is undefined behaviour*, and it is meant to
    /// be reached - falling off the end of a non-void function is the case. So the verifier holds
    /// the first to "the entry cannot reach it" and asks the second for nothing.
    Unreachable {
        /// Where in the source.
        span: Span,
    },
    /// C says the behaviour is undefined here, so the program stops: a trap rather than a number
    /// that looks like an answer.
    Trap {
        /// Where in the source.
        span: Span,
    },
}

/// One function: its parameters, its result, and its blocks.
#[derive(Debug, Clone)]
pub struct Func {
    /// The name it is called by.
    pub name: Symbol,
    /// What it returns.
    pub result: TypeId,
    /// Whether the parameter list ended in `...`. A variadic function's prologue writes its register
    /// arguments into the caller's home area, which is what a `va_list` walks (ADR-006).
    pub variadic: bool,
    /// Whether the name is visible outside this translation unit. A `static` function is not, and
    /// two units may each have one of the same name - so a linker that keys functions by name alone
    /// gives the second unit the first unit's code.
    pub external: bool,
    /// The entry block, which receives the parameters.
    pub entry: Block,
    /// Every block, in the order the builder made them.
    pub blocks: Vec<BlockData>,
    /// Every value, in the order the builder minted them: `v0` is the first.
    values: Vec<ValueDef>,
}

impl Func {
    /// A function with one empty entry block.
    pub fn new(name: Symbol, result: TypeId, span: Span) -> Self {
        let mut func = Func {
            name,
            result,
            variadic: false,
            external: true,
            entry: Block(0),
            blocks: Vec::new(),
            values: Vec::new(),
        };
        func.blocks.push(BlockData {
            params: Vec::new(),
            insts: Vec::new(),
            term: None,
            span,
        });
        func
    }

    /// The parameters, which are the entry block's parameters.
    pub fn params(&self) -> &[Value] {
        &self.blocks[self.entry.0 as usize].params
    }

    /// What a value is: its type, and the block that defines it.
    pub fn value(&self, value: Value) -> ValueDef {
        self.values[value.0 as usize]
    }

    /// Every value, in definition order.
    pub fn values(&self) -> impl Iterator<Item = (Value, ValueDef)> + '_ {
        self.values
            .iter()
            .enumerate()
            .map(|(index, def)| (Value(index as u32), *def))
    }

    /// A block's data.
    pub fn block(&self, block: Block) -> &BlockData {
        &self.blocks[block.0 as usize]
    }

    /// End every block that will never be filled with `unreachable`, leaving `current` alone.
    ///
    /// Some functions have them: the join after an `if` whose arms both return, or the block a
    /// front end made and then found it had nothing to put in. Where the block's emptiness is
    /// allowed to be known is here, not in the front end, which otherwise reaches for a block
    /// handle it was told it cannot make (ADR-002 invariant 1).
    pub fn end_orphans(&mut self, current: Block) {
        for index in 0..self.blocks.len() {
            let block = Block(index as u32);
            if block == current || self.terminated(block) {
                continue;
            }
            let span = self.blocks[index].span;
            Builder::at(self, block).unreachable(span);
        }
    }

    /// Append an empty block. Opening one does not need a block to be current, which is how
    /// `switch` opens every arm body after an earlier arm has already ended.
    pub fn append_empty(&mut self, span: Span) -> Block {
        let block = Block(self.blocks.len() as u32);
        self.blocks.push(BlockData {
            params: Vec::new(),
            insts: Vec::new(),
            term: None,
            span,
        });
        block
    }

    /// Whether a block already ends.
    pub fn terminated(&self, block: Block) -> bool {
        self.blocks[block.0 as usize].term.is_some()
    }

    /// Where the entry block is.
    pub fn entry_span(&self) -> Span {
        self.blocks[self.entry.0 as usize].span
    }

    /// A fresh value of `ty` defined in `block`: what a pass makes when it needs one (the builder
    /// is the usual caller, and a pass is the other).
    pub(crate) fn mint(&mut self, ty: TypeId, block: Block) -> Value {
        let value = Value(self.values.len() as u32);
        self.values.push(ValueDef { ty, block });
        value
    }
}

/// Fills one function. The only way to make a [`Value`] is to ask the builder for one.
pub struct Builder<'a> {
    func: &'a mut Func,
    current: Block,
}

impl<'a> Builder<'a> {
    /// A builder that appends to `func`, starting in its entry block.
    pub fn new(func: &'a mut Func) -> Self {
        let current = func.entry;
        Self { func, current }
    }

    /// A builder that continues `block`, which must not end yet. This is how a front end that
    /// keeps ownership of the function between statements comes back to it.
    pub fn at(func: &'a mut Func, block: Block) -> Self {
        assert!(
            func.blocks[block.0 as usize].term.is_none(),
            "block {} is already terminated",
            block.0
        );
        Self {
            func,
            current: block,
        }
    }

    /// The block being filled.
    pub fn current(&self) -> Block {
        self.current
    }

    /// A value's type.
    pub fn ty(&self, value: Value) -> TypeId {
        self.func.value(value).ty
    }

    /// The function being built.
    pub fn func(&mut self) -> &mut Func {
        self.func
    }

    /// Give the current block the parameters a function takes: only the entry block may.
    pub fn add_params(&mut self, types: &[TypeId]) {
        let block = self.current;
        let params: Vec<Value> = types.iter().map(|&ty| self.func.mint(ty, block)).collect();
        self.func.blocks[block.0 as usize].params = params;
    }

    /// A new block with parameters of the given types, appended to the function.
    pub fn append_block(&mut self, types: &[TypeId], span: Span) -> Block {
        let block = Block(self.func.blocks.len() as u32);
        self.func.blocks.push(BlockData {
            params: Vec::new(),
            insts: Vec::new(),
            term: None,
            span,
        });
        let params: Vec<Value> = types.iter().map(|&ty| self.func.mint(ty, block)).collect();
        self.func.blocks[block.0 as usize].params = params;
        block
    }

    /// Start filling `block`.
    ///
    /// # Panics
    ///
    /// If `block` already ends: a block is filled once (ADR-002 invariant 1).
    pub fn switch_to(&mut self, block: Block) {
        assert!(
            self.func.blocks[block.0 as usize].term.is_none(),
            "block {} is already terminated",
            block.0
        );
        self.current = block;
    }

    /// Whether the current block already ends.
    pub fn is_terminated(&self) -> bool {
        self.func.blocks[self.current.0 as usize].term.is_some()
    }

    fn push(&mut self, inst: Inst) {
        assert!(
            !self.is_terminated(),
            "an instruction after the end of block {}",
            self.current.0
        );
        self.func.blocks[self.current.0 as usize].insts.push(inst);
    }

    fn set_term(&mut self, term: Term) {
        assert!(!self.is_terminated(), "a block ends once");
        self.func.blocks[self.current.0 as usize].term = Some(term);
    }

    /// A value of `ty` read from `address`.
    pub fn load(&mut self, address: Value, ty: TypeId, span: Span) -> Value {
        let result = self.func.mint(ty, self.current);
        self.push(Inst::Load {
            result,
            ty,
            address,
            span,
        });
        result
    }

    /// Write `value` at `address`.
    pub fn store(&mut self, address: Value, value: Value, span: Span) {
        self.push(Inst::Store {
            address,
            value,
            span,
        });
    }

    /// `left op right`, both of type `ty`.
    pub fn binary(
        &mut self,
        op: BinOp,
        left: Value,
        right: Value,
        ty: TypeId,
        span: Span,
    ) -> Value {
        let result = self.func.mint(ty, self.current);
        self.push(Inst::Binary {
            result,
            op,
            ty,
            left,
            right,
            span,
        });
        result
    }

    /// `left op right`, where the operands are `ty` and the result is an `int`.
    pub fn compare(
        &mut self,
        op: CmpOp,
        left: Value,
        right: Value,
        ty: TypeId,
        result: TypeId,
        span: Span,
    ) -> Value {
        let value = self.func.mint(result, self.current);
        self.push(Inst::Compare {
            result: value,
            op,
            ty,
            left,
            right,
            span,
        });
        value
    }

    /// A read of a segment-relative word: `mov rax, gs:[offset]`.
    ///
    /// This is how a Windows SDK reaches the thread environment block - `__readgsqword(0x30)` is
    /// `NtCurrentTeb()`, and `GetLastError` is a field of it. The offset must be a constant, which
    /// both SDKs require of these intrinsics; `size` is how many bytes are read, 1, 2, 4 or 8, and
    /// is the size of the result's type.
    pub fn segment_read(&mut self, segment: Segment, offset: u32, ty: TypeId, span: Span) -> Value {
        let value = self.func.mint(ty, self.current);
        self.push(Inst::SegmentRead {
            result: value,
            segment,
            offset,
            ty,
            span,
        });
        value
    }

    /// A compare-and-swap, giving the value it read. See [`Inst::AtomicCas`].
    pub fn atomic_cas(
        &mut self,
        pointer: Value,
        expected: Value,
        desired: Value,
        ty: TypeId,
        span: Span,
    ) -> Value {
        let value = self.func.mint(ty, self.current);
        self.push(Inst::AtomicCas {
            result: value,
            pointer,
            expected,
            desired,
            ty,
            span,
        });
        value
    }

    /// `op value`, giving a value of type `ty`.
    pub fn unary(&mut self, op: UnOp, value: Value, ty: TypeId, span: Span) -> Value {
        let result = self.func.mint(ty, self.current);
        self.push(Inst::Unary {
            result,
            op,
            ty,
            value,
            span,
        });
        result
    }

    /// Convert `value` from `from` to `to`, by `op`.
    pub fn convert(
        &mut self,
        op: ConvertOp,
        value: Value,
        from: TypeId,
        to: TypeId,
        span: Span,
    ) -> Value {
        let result = self.func.mint(to, self.current);
        self.push(Inst::Convert {
            result,
            op,
            from,
            to,
            value,
            span,
        });
        result
    }

    /// Call `callee` with `args`; `result` is `None` when the function returns nothing.
    pub fn call(
        &mut self,
        callee: Value,
        args: Vec<Value>,
        variadic: bool,
        result: Option<TypeId>,
        span: Span,
    ) -> Option<Value> {
        let value = result.map(|ty| self.func.mint(ty, self.current));
        self.push(Inst::Call {
            result: value,
            callee,
            args,
            variadic,
            span,
        });
        value
    }

    /// A constant of type `ty` whose bits are `bits`.
    pub fn constant(&mut self, ty: TypeId, bits: u64, span: Span) -> Value {
        let result = self.func.mint(ty, self.current);
        self.push(Inst::Const {
            result,
            ty,
            bits,
            span,
        });
        result
    }

    /// Stack space for a C variable of type `ty`; the value is a `pointer` to it. The caller
    /// has the type arena, so it says what a pointer to `ty` is.
    pub fn slot(&mut self, ty: TypeId, pointer: TypeId, span: Span) -> Value {
        self.aligned_slot(ty, pointer, 0, span)
    }

    /// The same, with the alignment `_Alignas` asked for; zero means the type's own.
    ///
    /// The `Slot` goes to the **entry block**, not to the block the declaration is in: a frame slot
    /// belongs to the frame, and its address is the same on every path through the function.
    /// Emitting it where the declaration stood made it a value defined in one block and used in
    /// another that need not dominate it - which is what a `switch` whose `case` labels sit *inside*
    /// nested `do { } while (0)` blocks does, and miniz's inflater is exactly that: a slot created
    /// in one case's block was loaded from another and the verifier refused the function.
    ///
    /// This waited for the register allocator's liveness to stop being quadratic. With every slot in
    /// the entry block `tinfl_decompress` names nearly three thousand values, and the old liveness -
    /// `contains` on a `Vec` twice per value per iteration - did not finish in half an hour against
    /// 82 milliseconds now.
    pub fn aligned_slot(&mut self, ty: TypeId, pointer: TypeId, alignas: u64, span: Span) -> Value {
        let entry = self.func.entry;
        let result = self.func.mint(pointer, entry);
        // At the front: the entry block is already terminated by the time a nested declaration is
        // read, and appending would put an instruction after its terminator.
        self.func.blocks[entry.0 as usize].insts.insert(
            0,
            Inst::Slot {
                result,
                ty,
                alignas,
                span,
            },
        );
        result
    }

    /// The address of a global object or function.
    /// `va_start`: the address of the first argument that is not a parameter (`skip` counts them).
    pub fn va_start(&mut self, ty: TypeId, skip: u8, span: Span) -> Value {
        let result = self.func.mint(ty, self.current);
        self.push(Inst::VaStart { result, skip, span });
        result
    }

    /// `va_arg`: read one argument of `ty` through the list.
    pub fn va_arg(&mut self, ty: TypeId, list: Value, read: TypeId, span: Span) -> Value {
        let result = self.func.mint(ty, self.current);
        self.push(Inst::VaArg {
            result,
            list,
            ty: read,
            span,
        });
        result
    }

    pub fn global_address(&mut self, name: String, ty: TypeId, span: Span) -> Value {
        let result = self.func.mint(ty, self.current);
        self.push(Inst::GlobalAddress { result, name, span });
        result
    }

    /// Go to `target` with `args`.
    pub fn jump(&mut self, target: Block, args: Vec<Value>, span: Span) {
        self.set_term(Term::Jump { target, args, span });
    }

    /// Go to `then` when `condition` is non-zero, `otherwise` when it is zero.
    pub fn branch(
        &mut self,
        condition: Value,
        then: (Block, Vec<Value>),
        otherwise: (Block, Vec<Value>),
        span: Span,
    ) {
        self.set_term(Term::Branch {
            condition,
            then,
            otherwise,
            span,
        });
    }

    /// Leave the function.
    pub fn ret(&mut self, value: Option<Value>, span: Span) {
        self.set_term(Term::Return { value, span });
    }

    /// End without a successor, because nothing can arrive.
    pub fn unreachable(&mut self, span: Span) {
        self.set_term(Term::Unreachable { span });
    }

    /// End in a trap, because arriving here is undefined behaviour.
    pub fn trap(&mut self, span: Span) {
        self.set_term(Term::Trap { span });
    }
}

/// A global object: what a translation unit declares outside every function.
#[derive(Debug, Clone)]
pub struct Global {
    /// The name, as the linker will see it. A `String` because a compiler-generated name
    /// (`.str0`, and later `__func__`) is not a token the source contained: an interner hands
    /// out symbols, and the front end does not own one (ADR-004, "the front end cannot mint a
    /// name").
    pub name: String,
    /// Its type.
    pub ty: TypeId,
    /// Whether it is defined here rather than only declared.
    pub defined: bool,
    /// The bytes it is initialised with, when it is defined and its value is known.
    pub init: Option<Vec<u8>>,
    /// The alignment `_Alignas` asked for, when it asked for one: zero means the type's own.
    pub alignas: u64,
    /// The places in those bytes that are the *address* of a symbol rather than a value: eight bytes
    /// each, filled in by the linker. ADR-004 said a global's initialiser is bytes, and a C
    /// initialiser may also be `&something` - `char *p = "hello"` being the one every program has.
    pub relocations: Vec<GlobalRelocation>,
    /// Whether the name is visible outside this translation unit.
    pub external: bool,
    /// Whether the object has thread storage duration (C11 6.7.3): one copy per thread, and its
    /// *address* is not a link-time constant - which is the whole of what makes it different.
    pub thread: bool,
    /// Where in the source.
    pub span: Span,
}

/// An address inside a global object's initialiser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalRelocation {
    /// Where the address goes, in bytes from the object's start.
    pub offset: u32,
    /// What it is the address of.
    pub symbol: String,
    /// How many bytes into that symbol the address is. `&table[3]` is the symbol plus three
    /// elements; the linker adds this to the address it resolves, and the object writer puts it in
    /// the field because that is what a COFF relocation means (`S + A`, `A` already written).
    pub addend: i64,
}

/// A translation unit.
#[derive(Debug)]
pub struct Module {
    /// The types the values and instructions name.
    pub types: Types,
    /// The functions defined here.
    pub funcs: Vec<Func>,
    /// The globals defined or declared here.
    pub globals: Vec<Global>,
}

impl Module {
    /// An empty module over `types`.
    pub fn new(types: Types) -> Self {
        Self {
            types,
            funcs: Vec::new(),
            globals: Vec::new(),
        }
    }

    /// The function called `name`, if this module defines one.
    pub fn func(&self, name: Symbol) -> Option<&Func> {
        self.funcs.iter().find(|func| func.name == name)
    }
}

/// Why a function does not verify. A verifier failure is a compiler bug, not a diagnostic
/// (ADR-002).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Broken {
    /// What is wrong, in one line, naming the block or value.
    pub message: String,
}

impl Broken {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl Func {
    /// Check the invariants the builder is supposed to keep: every block ends, every value used
    /// exists and is defined in a block that dominates the use, branch arguments match the
    /// target's parameters, and instruction types agree.
    ///
    /// This is a compiler-bug detector: it runs in debug builds and in tests, never on a
    /// compilation's path.
    /// Rewrite every value this function uses: `rewrite` sees each operand of each instruction
    /// and each terminator and answers what it should be instead. A pass's whole job, in one call.
    pub(crate) fn rewrite_values(&mut self, rewrite: &mut impl FnMut(Value) -> Value) {
        for block in &mut self.blocks {
            for inst in &mut block.insts {
                inst.rewrite_values(rewrite);
            }
            if let Some(term) = &mut block.term {
                term.rewrite_values(rewrite);
            }
        }
    }

    pub fn verify(&self, types: &Types) -> Result<(), Broken> {
        // Every value used must be one a block still defines. A pass can remove an instruction and
        // leave a use behind, and nothing else notices until the back end asks `values` for it.
        let mut defined: Vec<bool> = vec![false; self.values.len()];
        for block in &self.blocks {
            for param in &block.params {
                defined[param.0 as usize] = true;
            }
            for inst in &block.insts {
                if let Some(result) = inst.result() {
                    defined[result.0 as usize] = true;
                }
            }
        }
        for block in &self.blocks {
            let mut uses: Vec<Value> = block.insts.iter().flat_map(Inst::operands).collect();
            if let Some(term) = &block.term {
                uses.extend(term_operands(term));
            }
            for value in uses {
                if !defined.get(value.0 as usize).copied().unwrap_or(false) {
                    return Err(Broken::new(format!(
                        "v{} is used but no instruction defines it",
                        value.0
                    )));
                }
            }
        }
        for (index, block) in self.blocks.iter().enumerate() {
            let here = Block(index as u32);
            if block.term.is_none() {
                return Err(Broken::new(format!("block {index} does not end")));
            }
            for param in &block.params {
                if self.value(*param).block != here {
                    return Err(Broken::new(format!(
                        "v{} is a parameter of block {index} but is defined elsewhere",
                        param.0
                    )));
                }
            }
            for inst in &block.insts {
                self.verify_inst(types, here, index, inst)?;
            }
        }
        // Dominance: a value must be defined in a block that every path to the use goes
        // through. The tree is computed once for the whole function and walked by each check - and
        // so is reachability, which `check_dominates` asks about on every operand. Asking per use
        // was a depth-first walk per use: 7 seconds of `sqlite3.c` became 40.
        let idom = self.dominator_tree();
        let live = self.reachable_blocks();
        for (index, block) in self.blocks.iter().enumerate() {
            let here = Block(index as u32);
            for inst in &block.insts {
                for used in used_values(inst) {
                    self.check_dominates(&idom, &live, here, used, types)?;
                }
            }
            match block.term.as_ref().expect("checked above") {
                Term::Return { value, .. } => {
                    if let Some(value) = value {
                        self.check_dominates(&idom, &live, here, *value, types)?;
                    }
                }
                Term::Jump { target, args, .. } => {
                    self.check_arguments(*target, args, types)?;
                    for arg in args {
                        self.check_dominates(&idom, &live, here, *arg, types)?;
                    }
                }
                Term::Branch {
                    condition,
                    then,
                    otherwise,
                    ..
                } => {
                    self.check_dominates(&idom, &live, here, *condition, types)?;
                    if !types.is_integer(self.value(*condition).ty)
                        && !types.is_pointer(self.value(*condition).ty)
                    {
                        return Err(Broken::new(format!(
                            "block {index} branches on a value that is not a number or a pointer"
                        )));
                    }
                    for (target, args) in [then, otherwise] {
                        self.check_arguments(*target, args, types)?;
                        for arg in args {
                            self.check_dominates(&idom, &live, here, *arg, types)?;
                        }
                    }
                }
                // `unreachable` is not an instruction that traps on purpose: it is the terminator a
                // block gets when the *front end* believes nothing can arrive at it. A branch into
                // one is therefore a front-end bug that a program finds by executing the trap - which
                // is how `sqlite3.c` died of a `ud2` inside the query planner with no span to point
                // at.
                //
                // The invariant is reachability, not "has no edges": a dead arm that jumped to a
                // dead join is an edge nobody follows, and asking it that way failed `sqlite3.c` on
                // a block no path leads to.
                Term::Unreachable { span } => {
                    if live[index] {
                        return Err(Broken::new(format!(
                            "block {index} is unreachable (at byte {}) and the entry reaches it",
                            span.start
                        )));
                    }
                }
                // A trap is *meant* to be reached - that is what makes it a trap - so it is the one
                // terminator with no reachability to check.
                Term::Trap { .. } => {}
            }
        }
        Ok(())
    }

    fn verify_inst(
        &self,
        types: &Types,
        here: Block,
        index: usize,
        inst: &Inst,
    ) -> Result<(), Broken> {
        let wrong = |message: String| Err(Broken::new(message));
        // Every type comparison here is structural, as the arena's identity is: the same C type
        // may be two `TypeId`s, and `==` would reject correct code (ADR-004's corollary).
        match inst {
            Inst::SegmentRead { result, ty, .. } => {
                // The same rule every other defining instruction answers: the value is defined here,
                // and it is of the type the instruction names.
                if self.value(*result).block != here {
                    return wrong(format!("v{} is not defined where it is used", result.0));
                }
                if !types.same(self.value(*result).ty, *ty) {
                    return wrong(format!("v{} is not read as its own type", result.0));
                }
                if !matches!(
                    types.kind(*ty),
                    crate::types::Type::Int {
                        bits: 8 | 16 | 32 | 64,
                        ..
                    }
                ) {
                    return wrong(format!(
                        "v{} is a segment read of something not an integer",
                        result.0
                    ));
                }
                Ok(())
            }
            Inst::AtomicCas {
                result,
                pointer,
                expected,
                desired,
                ty,
                ..
            } => {
                if self.value(*result).block != here {
                    return wrong(format!("v{} is not defined where it is used", result.0));
                }
                if !types.same(self.value(*result).ty, *ty) {
                    return wrong(format!("v{} is not the type it was swapped as", result.0));
                }
                for operand in [expected, desired] {
                    if !types.same(self.value(*operand).ty, *ty) {
                        return wrong(format!(
                            "v{} is swapped into an object of another type",
                            operand.0
                        ));
                    }
                }
                if !types.is_pointer(self.value(*pointer).ty) {
                    return wrong(format!("v{} is not a pointer to swap through", pointer.0));
                }
                Ok(())
            }
            Inst::VaStart { result, .. } | Inst::VaArg { result, .. } => {
                // The same rule every other defining instruction answers: the value is defined here,
                // and it is of the type the instruction names.
                if self.value(*result).block != here {
                    return wrong(format!("v{} is not defined where it is used", result.0));
                }
                if let Inst::VaArg { ty, list, .. } = inst {
                    if !types.same(self.value(*result).ty, *ty) {
                        return wrong(format!("v{} is not read as its own type", result.0));
                    }
                    if !types.is_pointer(self.value(*list).ty) {
                        return wrong(format!(
                            "block {index} reads an argument through a non-pointer"
                        ));
                    }
                }
                Ok(())
            }
            Inst::Load {
                result,
                ty,
                address,
                ..
            } => {
                if !types.same(self.value(*result).ty, *ty) || self.value(*result).block != here {
                    return wrong(format!("v{} is not a load of its own type", result.0));
                }
                if !types.is_pointer(self.value(*address).ty) {
                    return wrong(format!(
                        "block {index} loads through a value that is not an address"
                    ));
                }
                if !types.same(types.pointee(self.value(*address).ty), *ty) {
                    return wrong(format!(
                        "block {index} loads a value of another type than its address points to"
                    ));
                }
                Ok(())
            }
            Inst::Store { address, value, .. } => {
                let target = self.value(*address).ty;
                if !types.is_pointer(target) {
                    return wrong(format!(
                        "block {index} stores through a value that is not an address"
                    ));
                }
                if !types.is_assignable(types.pointee(target), self.value(*value).ty) {
                    return wrong(format!(
                        "block {index} stores a value of another type than its address points to"
                    ));
                }
                Ok(())
            }
            Inst::Binary {
                result,
                ty,
                left,
                right,
                ..
            } => {
                if !types.same(self.value(*result).ty, *ty) {
                    return wrong(format!(
                        "v{} is not of the type it is computed in",
                        result.0
                    ));
                }
                for operand in [left, right] {
                    let seen = self.value(*operand).ty;
                    if !types.same(seen, *ty) && !types.is_assignable(*ty, seen) {
                        return wrong(format!(
                            "block {index} computes on v{} of another type",
                            operand.0
                        ));
                    }
                }
                Ok(())
            }
            Inst::Compare {
                result,
                ty,
                left,
                right,
                ..
            } => {
                if !types.is_integer(self.value(*result).ty) {
                    return wrong(format!(
                        "v{} is a comparison that is not an integer",
                        result.0
                    ));
                }
                if !types.same(self.value(*left).ty, *ty) || !types.same(self.value(*right).ty, *ty)
                {
                    return wrong(format!("block {index} compares values of another type"));
                }
                Ok(())
            }
            Inst::Unary {
                result, ty, value, ..
            } => {
                if !types.same(self.value(*result).ty, *ty)
                    || !types.same(self.value(*value).ty, *ty)
                {
                    return wrong(format!("block {index} applies one type to another"));
                }
                Ok(())
            }
            Inst::Convert {
                result,
                from,
                to,
                value,
                ..
            } => {
                // Structural identity, not the id: the arena mints a fresh id for every
                // `pointer(...)` it is asked for, so `==` rejects correct conversions (ADR-004).
                if !types.same(self.value(*value).ty, *from)
                    || !types.same(self.value(*result).ty, *to)
                {
                    return wrong(format!("block {index} converts other types than it says"));
                }
                Ok(())
            }
            Inst::Call { result, callee, .. } => {
                if let Some(result) = result {
                    if self.value(*result).block != here {
                        return wrong(format!("v{} is not defined by its call", result.0));
                    }
                }
                if !types.is_pointer(self.value(*callee).ty) {
                    return wrong(format!(
                        "block {index} calls a value that is not a function"
                    ));
                }
                Ok(())
            }
            Inst::Const { result, ty, .. } => {
                if self.value(*result).block != here || !types.same(self.value(*result).ty, *ty) {
                    return wrong(format!("v{} is not a constant of its own type", result.0));
                }
                Ok(())
            }
            Inst::Slot { result, ty, .. } => {
                let pointer = self.value(*result);
                if pointer.block != here || !types.is_pointer(pointer.ty) {
                    return wrong(format!("v{} is not a slot address", result.0));
                }
                if !types.same(types.pointee(pointer.ty), *ty) {
                    return wrong(format!("v{} is a slot for another type", result.0));
                }
                Ok(())
            }
            Inst::GlobalAddress { result, .. } => {
                if self.value(*result).block != here {
                    return wrong(format!("v{} is not defined by its address", result.0));
                }
                Ok(())
            }
        }
    }

    /// The immediate dominator of every block: the closest block that dominates it, or `None` for
    /// the entry.
    ///
    /// Cooper, Harvey and Kennedy's iterative algorithm: walk the blocks in reverse postorder,
    /// and for each ask what its predecessors' dominators have in common. It needs one `Vec<Option>`
    /// where the textbook formulation needs an `n x n` matrix, and it converges in a couple of
    /// passes for real code - which matters, because this is asked by the verifier, by `mem2reg` and
    /// by the back end, on every block of every function.
    pub(crate) fn dominator_tree(&self) -> Vec<Option<Block>> {
        let order = self.reverse_postorder();
        // Postorder numbers: a block's number is smaller than its predecessors' while the tree is
        // being built, which is what makes "the common dominator of two blocks" a walk upwards.
        let mut postorder = vec![0usize; self.blocks.len()];
        for (number, block) in order.iter().rev().enumerate() {
            postorder[block.0 as usize] = number;
        }
        let mut idom: Vec<Option<Block>> = vec![None; self.blocks.len()];
        idom[self.entry.0 as usize] = Some(self.entry);
        let intersect = |idom: &Vec<Option<Block>>, mut a: Block, mut b: Block| -> Block {
            while a != b {
                while postorder[a.0 as usize] < postorder[b.0 as usize] {
                    a = idom[a.0 as usize].unwrap_or(a);
                }
                while postorder[b.0 as usize] < postorder[a.0 as usize] {
                    b = idom[b.0 as usize].unwrap_or(b);
                }
            }
            a
        };
        let mut changed = true;
        while changed {
            changed = false;
            for block in &order {
                if *block == self.entry {
                    continue;
                }
                let mut predecessors = self
                    .predecessors(*block)
                    .into_iter()
                    .filter(|other| idom[other.0 as usize].is_some());
                let Some(first) = predecessors.next() else {
                    continue;
                };
                let mut new = first;
                for other in predecessors {
                    new = intersect(&idom, other, new);
                }
                if idom[block.0 as usize] != Some(new) {
                    idom[block.0 as usize] = Some(new);
                    changed = true;
                }
            }
        }
        // The entry's own dominator is nothing, not itself: its callers ask "what dominates this
        // block" and the entry is the end of the chain.
        idom[self.entry.0 as usize] = None;
        idom
    }

    /// Whether every path from the entry to `of` passes through `through`.
    ///
    /// The verifier asks this about every use, so it walks the dominator chain - short for real
    /// code - rather than consulting a matrix.
    pub(crate) fn dominates(&self, through: Block, of: Block) -> bool {
        self.dominates_with(&self.dominator_tree(), through, of)
    }

    /// The same, with the tree already computed: the verifier asks this for every use of every
    /// value, and computing the tree per question would be quadratic in the number of uses.
    fn dominates_with(&self, idom: &[Option<Block>], through: Block, of: Block) -> bool {
        if through == of {
            return true;
        }
        let mut block = of;
        let mut steps = 0;
        loop {
            match idom[block.0 as usize] {
                Some(next) if next == through => return true,
                Some(next) if next != block => {
                    block = next;
                    steps += 1;
                    if steps > self.blocks.len() {
                        return false;
                    }
                }
                _ => return false,
            }
        }
    }

    /// Whether control can get to `block` from the entry at all.
    ///
    /// The block after a `for (;;)` that never breaks is not reachable, and the fact matters where a
    /// block's terminator is chosen: "this function falls off its end" is only true of a block
    /// something can arrive at. Unreachable blocks still need *a* terminator, and `unreachable` is
    /// the honest one.
    pub fn reachable(&self, block: Block) -> bool {
        self.reverse_postorder().contains(&block)
    }

    /// Every block the entry can reach, one depth-first walk. The verifier asks this about every
    /// operand it checks, so the walk happens once and the answer is a lookup.
    pub(crate) fn reachable_blocks(&self) -> Vec<bool> {
        let mut live = vec![false; self.blocks.len()];
        let mut stack = vec![self.entry];
        while let Some(block) = stack.pop() {
            if live[block.0 as usize] {
                continue;
            }
            live[block.0 as usize] = true;
            stack.extend(self.successors(block));
        }
        live
    }

    /// Every block reachable from the entry, in an order where each comes after the blocks that
    /// dominate it.
    fn reverse_postorder(&self) -> Vec<Block> {
        let mut visited = vec![false; self.blocks.len()];
        let mut postorder = Vec::new();
        let mut stack: Vec<(Block, bool)> = vec![(self.entry, false)];
        while let Some((block, done)) = stack.pop() {
            if done {
                postorder.push(block);
                continue;
            }
            if visited[block.0 as usize] {
                continue;
            }
            visited[block.0 as usize] = true;
            stack.push((block, true));
            for successor in self.successors(block).into_iter().rev() {
                if !visited[successor.0 as usize] {
                    stack.push((successor, false));
                }
            }
        }
        postorder.reverse();
        postorder
    }

    /// The blocks that jump to `block`.
    fn predecessors(&self, block: Block) -> Vec<Block> {
        let mut found = Vec::new();
        for (index, other) in self.blocks.iter().enumerate() {
            let into = match &other.term {
                Some(Term::Jump { target, .. }) => *target == block,
                Some(Term::Branch {
                    then, otherwise, ..
                }) => then.0 == block || otherwise.0 == block,
                _ => false,
            };
            if into {
                found.push(Block(index as u32));
            }
        }
        found
    }

    pub(crate) fn successors(&self, block: Block) -> Vec<Block> {
        match self.blocks[block.0 as usize].term.as_ref() {
            Some(Term::Jump { target, .. }) => vec![*target],
            Some(Term::Branch {
                then, otherwise, ..
            }) => vec![then.0, otherwise.0],
            Some(Term::Return { .. } | Term::Unreachable { .. } | Term::Trap { .. }) | None => {
                Vec::new()
            }
        }
    }

    fn check_dominates(
        &self,
        idom: &[Option<Block>],
        live: &[bool],
        here: Block,
        value: Value,
        types: &Types,
    ) -> Result<(), Broken> {
        if (value.0 as usize) >= self.values.len() {
            return Err(Broken::new(format!(
                "v{} is used but never defined",
                value.0
            )));
        }
        // Dominance is a statement about *paths*, and a block the entry cannot reach has none: C
        // code after an `if` whose every arm returned is emitted - the statements are there to be
        // checked - and nothing can arrive at it. Asking for dominance there refuses a legal
        // program, and refusing to emit dead code is not this compiler's business.
        if !live[here.0 as usize] {
            return Ok(());
        }
        let definition = self.value(value).block;
        if !self.dominates_with(idom, definition, here) {
            return Err(Broken::new(format!(
                "v{} is defined in block {} and used in block {} which it does not dominate",
                value.0, definition.0, here.0
            )));
        }
        if types.is_error(self.value(value).ty) {
            return Err(Broken::new(format!(
                "v{} has the error type, which must not reach a module",
                value.0
            )));
        }
        Ok(())
    }

    fn check_arguments(&self, target: Block, args: &[Value], types: &Types) -> Result<(), Broken> {
        let params = &self.blocks[target.0 as usize].params;
        if args.len() != params.len() {
            return Err(Broken::new(format!(
                "block {} is given {} values and takes {}",
                target.0,
                args.len(),
                params.len()
            )));
        }
        for (arg, param) in args.iter().zip(params) {
            let given = self.value(*arg).ty;
            let wanted = self.value(*param).ty;
            if !types.same(given, wanted) && !types.is_assignable(wanted, given) {
                return Err(Broken::new(format!(
                    "block {} takes a value of another type in v{}",
                    target.0, param.0
                )));
            }
        }
        Ok(())
    }
}

/// Every value a terminator reads. One definition, because the verifier, the passes and the back
/// ends all have to agree on what "the operands of a terminator" are.
pub(crate) fn term_operands(term: &Term) -> Vec<Value> {
    match term {
        Term::Return { value, .. } => value.iter().copied().collect(),
        Term::Jump { args, .. } => args.clone(),
        Term::Branch {
            condition,
            then,
            otherwise,
            ..
        } => {
            let mut values = vec![*condition];
            values.extend(then.1.iter().copied());
            values.extend(otherwise.1.iter().copied());
            values
        }
        Term::Unreachable { .. } | Term::Trap { .. } => Vec::new(),
    }
}

/// Every value an instruction reads, in a fixed order: the verifier walks them, and a pass
/// rewrites them, and both must agree on what "the operands" are.
fn used_values(inst: &Inst) -> Vec<Value> {
    match inst {
        Inst::Load { address, .. } => vec![*address],
        Inst::Store { address, value, .. } => vec![*address, *value],
        Inst::Binary { left, right, .. } => vec![*left, *right],
        Inst::Compare { left, right, .. } => vec![*left, *right],
        Inst::AtomicCas {
            pointer,
            expected,
            desired,
            ..
        } => vec![*pointer, *expected, *desired],
        Inst::SegmentRead { .. } => Vec::new(),
        Inst::Unary { value, .. } => vec![*value],
        Inst::Convert { value, .. } => vec![*value],
        Inst::Call { callee, args, .. } => {
            let mut used = vec![*callee];
            used.extend(args.iter().copied());
            used
        }
        Inst::VaArg { list, .. } => vec![*list],
        Inst::Slot { .. }
        | Inst::Const { .. }
        | Inst::VaStart { .. }
        | Inst::GlobalAddress { .. } => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::dump::{dump, dump_func};
    use super::*;
    use crate::intern::Interner;

    struct Fixture {
        types: Types,
        names: Interner,
        int: TypeId,
        pointer: TypeId,
    }

    fn setup() -> Fixture {
        let mut types = Types::new();
        let int = types.fundamental("int").unwrap();
        let pointer = types.pointer(int);
        Fixture {
            types,
            names: Interner::new(),
            int,
            pointer,
        }
    }

    fn dump_text(types: &Types, names: &Interner, func: &Func) -> String {
        let mut out = String::new();
        dump_func(&mut out, types, names, func);
        out
    }

    #[test]
    fn a_hand_built_function_dumps_and_verifies() {
        let mut f = setup();
        let name = f.names.intern("f");
        let mut func = Func::new(name, f.int, Span::default());
        let (a, b);
        {
            let mut builder = Builder::new(&mut func);
            builder.add_params(&[f.int, f.int]);
            let params = builder.func().params().to_vec();
            a = params[0];
            b = params[1];
            let sum = builder.binary(BinOp::Add, a, b, f.int, Span::default());
            builder.ret(Some(sum), Span::default());
        }
        assert_eq!(func.verify(&f.types), Ok(()));
        // The module dump is the entry point a harness compares, so a test pins it too.
        let module = Module::new(f.types);
        let mut module = module;
        module.funcs.push(func.clone());
        assert_eq!(
            dump(&module, &f.names),
            "func f(v0: int, v1: int) -> int
block0 (entry)(v0: int, v1: int):
  v2 = add int v0, v1
  return v2
"
        );
        f.types = module.types;
        assert_eq!(
            dump_text(&f.types, &f.names, &func),
            "func f(v0: int, v1: int) -> int
block0 (entry)(v0: int, v1: int):
  v2 = add int v0, v1
  return v2
"
        );
    }

    #[test]
    fn a_local_variable_is_a_slot_and_memory_is_explicit() {
        let mut f = setup();
        let name = f.names.intern("f");
        let mut func = Func::new(name, f.int, Span::default());
        {
            let mut builder = Builder::new(&mut func);
            builder.add_params(&[]);
            let slot = builder.slot(f.int, f.pointer, Span::default());
            let seven = builder.constant(f.int, 7, Span::default());
            builder.store(slot, seven, Span::default());
            let read = builder.load(slot, f.int, Span::default());
            builder.ret(Some(read), Span::default());
        }
        assert_eq!(func.verify(&f.types), Ok(()));
        let text = dump_text(&f.types, &f.names, &func);
        assert!(text.contains("v0 = slot int"), "{text}");
        assert!(text.contains("v1 = const int 7"), "{text}");
        assert!(text.contains("store v1, v0"), "{text}");
        assert!(text.contains("v2 = load int, v0"), "{text}");
    }

    #[test]
    fn a_join_point_is_a_block_parameter() {
        let mut f = setup();
        let name = f.names.intern("f");
        let mut func = Func::new(name, f.int, Span::default());
        {
            let mut builder = Builder::new(&mut func);
            builder.add_params(&[f.int]);
            let condition = builder.func().params()[0];
            let then = builder.append_block(&[], Span::default());
            let otherwise = builder.append_block(&[], Span::default());
            let join = builder.append_block(&[f.int], Span::default());
            builder.branch(
                condition,
                (then, Vec::new()),
                (otherwise, Vec::new()),
                Span::default(),
            );
            builder.switch_to(then);
            let one = builder.constant(f.int, 1, Span::default());
            builder.jump(join, vec![one], Span::default());
            builder.switch_to(otherwise);
            let two = builder.constant(f.int, 2, Span::default());
            builder.jump(join, vec![two], Span::default());
            builder.switch_to(join);
            let value = builder.func().block(join).params[0];
            builder.ret(Some(value), Span::default());
        }
        assert_eq!(func.verify(&f.types), Ok(()));
        let text = dump_text(&f.types, &f.names, &func);
        assert!(text.contains("block3(v1: int):"), "{text}");
        assert!(text.contains("jump block3(v2)"), "{text}");
        assert!(text.contains("br v0, block1, block2"), "{text}");
    }

    #[test]
    fn the_verifier_catches_a_value_its_use_does_not_dominate() {
        let mut f = setup();
        let name = f.names.intern("f");
        let mut func = Func::new(name, f.int, Span::default());
        {
            let mut builder = Builder::new(&mut func);
            builder.add_params(&[f.int]);
            let condition = builder.func().params()[0];
            let then = builder.append_block(&[], Span::default());
            let otherwise = builder.append_block(&[], Span::default());
            let join = builder.append_block(&[], Span::default());
            builder.branch(
                condition,
                (then, Vec::new()),
                (otherwise, Vec::new()),
                Span::default(),
            );
            builder.switch_to(then);
            let hidden = builder.constant(f.int, 7, Span::default());
            builder.jump(join, Vec::new(), Span::default());
            builder.switch_to(otherwise);
            builder.jump(join, Vec::new(), Span::default());
            builder.switch_to(join);
            builder.ret(Some(hidden), Span::default());
        }
        let broken = func.verify(&f.types).expect_err("this must not verify");
        assert!(
            broken.message.contains("does not dominate"),
            "{}",
            broken.message
        );
    }

    #[test]
    fn the_verifier_catches_a_block_that_does_not_end() {
        let mut f = setup();
        let name = f.names.intern("f");
        let mut func = Func::new(name, f.int, Span::default());
        {
            let mut builder = Builder::new(&mut func);
            builder.add_params(&[]);
            builder.append_block(&[], Span::default());
        }
        assert!(func
            .verify(&f.types)
            .expect_err("no terminator")
            .message
            .contains("does not end"));
    }

    #[test]
    fn the_verifier_catches_arguments_that_do_not_match_the_parameters() {
        let mut f = setup();
        let name = f.names.intern("f");
        let mut func = Func::new(name, f.int, Span::default());
        {
            let mut builder = Builder::new(&mut func);
            builder.add_params(&[]);
            let target = builder.append_block(&[f.int], Span::default());
            builder.jump(target, Vec::new(), Span::default());
            builder.switch_to(target);
            let param = builder.func().block(target).params[0];
            builder.ret(Some(param), Span::default());
        }
        assert!(func
            .verify(&f.types)
            .expect_err("missing argument")
            .message
            .contains("takes 1"));
    }

    #[test]
    fn a_branch_needs_a_condition_that_is_a_number_or_an_address() {
        let mut f = setup();
        let void = f.types.fundamental("void").unwrap();
        let name = f.names.intern("f");
        let mut func = Func::new(name, f.int, Span::default());
        {
            let mut builder = Builder::new(&mut func);
            builder.add_params(&[void]);
            let condition = builder.func().params()[0];
            let then = builder.append_block(&[], Span::default());
            let otherwise = builder.append_block(&[], Span::default());
            builder.branch(
                condition,
                (then, Vec::new()),
                (otherwise, Vec::new()),
                Span::default(),
            );
            builder.switch_to(then);
            let one = builder.constant(f.int, 1, Span::default());
            builder.ret(Some(one), Span::default());
            builder.switch_to(otherwise);
            let zero = builder.constant(f.int, 0, Span::default());
            builder.ret(Some(zero), Span::default());
        }
        assert!(func
            .verify(&f.types)
            .expect_err("a void condition")
            .message
            .contains("not a number"));
    }
}
