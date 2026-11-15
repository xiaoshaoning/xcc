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
        /// The arguments, already converted to the parameter types.
        args: Vec<Value>,
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
    /// The value this instruction defines, if it defines one.
    pub fn result(&self) -> Option<Value> {
        match self {
            Inst::Load { result, .. }
            | Inst::Binary { result, .. }
            | Inst::Compare { result, .. }
            | Inst::Unary { result, .. }
            | Inst::Convert { result, .. }
            | Inst::Slot { result, .. }
            | Inst::Const { result, .. }
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
            | Inst::Unary { span, .. }
            | Inst::Convert { span, .. }
            | Inst::Slot { span, .. }
            | Inst::Const { span, .. }
            | Inst::Call { span, .. }
            | Inst::GlobalAddress { span, .. } => *span,
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
    /// Nothing follows; the front end uses this where C says the behaviour is undefined.
    Unreachable {
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

    /// Whether a block already ends.
    pub fn terminated(&self, block: Block) -> bool {
        self.blocks[block.0 as usize].term.is_some()
    }

    /// Where the entry block is.
    pub fn entry_span(&self) -> Span {
        self.blocks[self.entry.0 as usize].span
    }

    fn mint(&mut self, ty: TypeId, block: Block) -> Value {
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
        result: Option<TypeId>,
        span: Span,
    ) -> Option<Value> {
        let value = result.map(|ty| self.func.mint(ty, self.current));
        self.push(Inst::Call {
            result: value,
            callee,
            args,
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
        let result = self.func.mint(pointer, self.current);
        self.push(Inst::Slot { result, ty, span });
        result
    }

    /// The address of a global object or function.
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

    /// End without a successor.
    pub fn unreachable(&mut self, span: Span) {
        self.set_term(Term::Unreachable { span });
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
    /// Whether the name is visible outside this translation unit.
    pub external: bool,
    /// Where in the source.
    pub span: Span,
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
    pub fn verify(&self, types: &Types) -> Result<(), Broken> {
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
        // through. Computed once, then used by every check below.
        let dominators = self.dominators();
        for (index, block) in self.blocks.iter().enumerate() {
            let here = Block(index as u32);
            for inst in &block.insts {
                for used in used_values(inst) {
                    self.check_dominates(&dominators, here, used, types)?;
                }
            }
            match block.term.as_ref().expect("checked above") {
                Term::Return { value, .. } => {
                    if let Some(value) = value {
                        self.check_dominates(&dominators, here, *value, types)?;
                    }
                }
                Term::Jump { target, args, .. } => {
                    self.check_arguments(&dominators, here, *target, args, types)?;
                    for arg in args {
                        self.check_dominates(&dominators, here, *arg, types)?;
                    }
                }
                Term::Branch {
                    condition,
                    then,
                    otherwise,
                    ..
                } => {
                    self.check_dominates(&dominators, here, *condition, types)?;
                    if !types.is_integer(self.value(*condition).ty)
                        && !types.is_pointer(self.value(*condition).ty)
                    {
                        return Err(Broken::new(format!(
                            "block {index} branches on a value that is not a number or a pointer"
                        )));
                    }
                    for (target, args) in [then, otherwise] {
                        self.check_arguments(&dominators, here, *target, args, types)?;
                        for arg in args {
                            self.check_dominates(&dominators, here, *arg, types)?;
                        }
                    }
                }
                Term::Unreachable { .. } => {}
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
        match inst {
            Inst::Load {
                result,
                ty,
                address,
                ..
            } => {
                if self.value(*result).ty != *ty || self.value(*result).block != here {
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
                if self.value(*result).ty != *ty {
                    return wrong(format!(
                        "v{} is not of the type it is computed in",
                        result.0
                    ));
                }
                for operand in [left, right] {
                    let seen = self.value(*operand).ty;
                    if seen != *ty && !types.is_assignable(*ty, seen) {
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
                if self.value(*left).ty != *ty || self.value(*right).ty != *ty {
                    return wrong(format!("block {index} compares values of another type"));
                }
                Ok(())
            }
            Inst::Unary {
                result, ty, value, ..
            } => {
                if self.value(*result).ty != *ty || self.value(*value).ty != *ty {
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
                if self.value(*value).ty != *from || self.value(*result).ty != *to {
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
                if self.value(*result).block != here || self.value(*result).ty != *ty {
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

    /// The blocks that every path from the entry to a block must pass through.
    fn dominators(&self) -> Vec<Vec<bool>> {
        let count = self.blocks.len();
        let mut dominators = vec![vec![true; count]; count];
        dominators[self.entry.0 as usize] = (0..count).map(|b| b == 0).collect();
        let mut changed = true;
        while changed {
            changed = false;
            for block in 0..count {
                if block == self.entry.0 as usize {
                    continue;
                }
                let mut next = vec![true; count];
                for (index, _) in self.blocks.iter().enumerate().filter(|(index, _)| {
                    self.successors(Block(*index as u32))
                        .contains(&Block(block as u32))
                }) {
                    for (target, seen) in next.iter_mut().enumerate() {
                        *seen = *seen && dominators[index][target];
                    }
                }
                // A block always dominates itself (the usual formulation is
                // `{b} | intersection of its predecessors`).
                next[block] = true;
                if next != dominators[block] {
                    dominators[block] = next;
                    changed = true;
                }
            }
        }
        dominators
    }

    fn successors(&self, block: Block) -> Vec<Block> {
        match self.blocks[block.0 as usize].term.as_ref() {
            Some(Term::Jump { target, .. }) => vec![*target],
            Some(Term::Branch {
                then, otherwise, ..
            }) => vec![then.0, otherwise.0],
            _ => Vec::new(),
        }
    }

    fn check_dominates(
        &self,
        dominators: &[Vec<bool>],
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
        let definition = self.value(value).block;
        if !dominators[here.0 as usize][definition.0 as usize] {
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

    fn check_arguments(
        &self,
        dominators: &[Vec<bool>],
        here: Block,
        target: Block,
        args: &[Value],
        types: &Types,
    ) -> Result<(), Broken> {
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
        let _ = dominators;
        let _ = here;
        Ok(())
    }
}

fn used_values(inst: &Inst) -> Vec<Value> {
    match inst {
        Inst::Load { address, .. } => vec![*address],
        Inst::Store { address, value, .. } => vec![*address, *value],
        Inst::Binary { left, right, .. } => vec![*left, *right],
        Inst::Compare { left, right, .. } => vec![*left, *right],
        Inst::Unary { value, .. } => vec![*value],
        Inst::Convert { value, .. } => vec![*value],
        Inst::Call { callee, args, .. } => {
            let mut used = vec![*callee];
            used.extend(args.iter().copied());
            used
        }
        Inst::Slot { .. } | Inst::Const { .. } | Inst::GlobalAddress { .. } => Vec::new(),
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
