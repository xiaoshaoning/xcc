//! Lowering `cir` to Cranelift, and Cranelift to an object file.
//!
//! This is the only module that may name Cranelift (CONSTRAINTS 8): `cir` is the seam, and
//! replacing this file with a second back end is the point of ADR-001 section 9.
//!
//! Two levels of translation, in order: `cir`'s blocks and values become Cranelift's, then
//! Cranelift's function is compiled for the host and written into a COFF object by
//! `cranelift-object`. Objects, not assembly text: Cranelift has no assembly printer, and the
//! honest thing to emit is what it can emit.

use std::collections::HashMap;

use cranelift_codegen::ir::{
    self, AbiParam, InstBuilder, MemFlags, Signature, UserFuncName, Value as ClValue,
};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_codegen::Context;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{default_libcall_names, FuncId, Linkage, Module, ModuleError};
use cranelift_object::{ObjectBuilder, ObjectModule};

use crate::cir::{self, BinOp, Block, CmpOp, ConvertOp, Func, Inst, Module as CirModule, Term};
use crate::types::{Type, TypeId, Types};

/// Why lowering failed. A lowering failure is a compiler bug or an unsupported target, never a
/// diagnostic about the C source.
#[derive(Debug)]
pub struct LoweringError(pub String);

impl std::fmt::Display for LoweringError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<ModuleError> for LoweringError {
    fn from(error: ModuleError) -> Self {
        LoweringError(format!("cranelift module: {error}"))
    }
}

/// Lower a translation unit to an object file, returning its bytes.
pub fn object(
    module: &CirModule,
    names: &crate::intern::Interner,
) -> Result<Vec<u8>, LoweringError> {
    let mut flags = settings::builder();
    // The host's own feature set, so a JIT and an object cannot disagree about what runs.
    flags
        .set("opt_level", "none")
        .map_err(|error| LoweringError(error.to_string()))?;
    let flags = settings::Flags::new(flags);
    let isa = cranelift_native::builder()
        .map_err(|error| LoweringError(error.to_string()))?
        .finish(flags)
        .map_err(|error| LoweringError(error.to_string()))?;
    let builder = ObjectBuilder::new(isa, "xcc", default_libcall_names())?;
    let mut object = ObjectModule::new(builder);
    let mut lowerer = Lowerer {
        object: &mut object,
        types: &module.types,
        names,
        funcs: HashMap::new(),
    };
    // Every function is declared before any is lowered, so calls can be forward references.
    for func in &module.funcs {
        lowerer.declare(func)?;
    }
    for func in &module.funcs {
        lowerer.lower(func)?;
    }
    object
        .finish()
        .emit()
        .map_err(|error| LoweringError(error.to_string()))
}

/// Holds the object being built and the type table the values name.
struct Lowerer<'a, M: Module> {
    object: &'a mut M,
    types: &'a Types,
    names: &'a crate::intern::Interner,
    /// The function each `cir` name means, for a call.
    funcs: HashMap<cir::Value, FuncId>,
}

impl<M: Module> Lowerer<'_, M> {
    /// A Cranelift type for a C type: its width, and whether it is a float.
    fn machine_type(&self, ty: TypeId) -> Result<ir::Type, LoweringError> {
        Ok(match self.types.kind(ty) {
            Type::Bool => ir::types::I8,
            Type::Int { bits, .. } => match bits {
                8 => ir::types::I8,
                16 => ir::types::I16,
                32 => ir::types::I32,
                64 => ir::types::I64,
                other => {
                    return Err(LoweringError(format!(
                        "no Cranelift type for {other}-bit int"
                    )))
                }
            },
            Type::Float { bits } => match bits {
                32 => ir::types::F32,
                64 => ir::types::F64,
                other => {
                    return Err(LoweringError(format!(
                        "no Cranelift type for {other}-bit float"
                    )))
                }
            },
            Type::Pointer(_) => ir::types::I64, // the target's word (ADR-001 decision 5)
            _other => {
                return Err(LoweringError(format!(
                    "a value of type `{}` has no machine type yet",
                    self.types.describe_with(ty, self.names)
                )))
            }
        })
    }

    /// The signature a C function type has, as the platform's C ABI states it.
    fn signature(&self, func: &Func) -> Result<Signature, LoweringError> {
        let mut signature = self.object.make_signature();
        for param in func.params() {
            let ty = self.machine_type(func.value(*param).ty)?;
            signature.params.push(AbiParam::new(ty));
        }
        if !matches!(self.types.kind(func.result), Type::Void) {
            let ty = self.machine_type(func.result)?;
            signature.returns.push(AbiParam::new(ty));
        }
        Ok(signature)
    }

    fn name(&self, func: &Func) -> &str {
        self.names.resolve(func.name)
    }

    fn declare(&mut self, func: &Func) -> Result<(), LoweringError> {
        let signature = self.signature(func)?;
        let name = self.name(func).to_owned();
        let id = self
            .object
            .declare_function(&name, Linkage::Export, &signature)?;
        let _ = id;
        Ok(())
    }

    fn lower(&mut self, func: &Func) -> Result<(), LoweringError> {
        let signature = self.signature(func)?;
        let name = self.name(func).to_owned();
        let id = self
            .object
            .declare_function(&name, Linkage::Export, &signature)?;
        let mut context = Context::new();
        context.func.signature = signature;
        context.func.name = UserFuncName::user(0, id.as_u32());

        let mut builder_context = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(&mut context.func, &mut builder_context);

        // Blocks first, so a jump can name one that is filled later.
        let mut blocks: HashMap<Block, ir::Block> = HashMap::new();
        for (index, block) in func.blocks.iter().enumerate() {
            let handle = Block(index as u32);
            let translated = builder.create_block();
            let params: Vec<ir::Type> = block
                .params
                .iter()
                .map(|param| func.value(*param).ty)
                .map(|ty| self.machine_type(ty))
                .collect::<Result<_, _>>()?;
            for ty in params {
                builder.append_block_param(translated, ty);
            }
            blocks.insert(handle, translated);
        }

        let mut values: HashMap<cir::Value, ClValue> = HashMap::new();
        builder.switch_to_block(blocks[&func.entry]);
        for (position, param) in func.params().iter().enumerate() {
            let translated = blocks[&func.entry];
            values.insert(*param, builder.block_params(translated)[position]);
        }
        // Values start in the entry block; each block's own parameters are its first values.
        for (index, block) in func.blocks.iter().enumerate() {
            let handle = Block(index as u32);
            if handle == func.entry {
                continue;
            }
            for (position, param) in block.params.iter().enumerate() {
                let translated = blocks[&handle];
                values.insert(*param, builder.block_params(translated)[position]);
            }
        }

        for (index, block) in func.blocks.iter().enumerate() {
            let handle = Block(index as u32);
            if handle != func.entry {
                builder.switch_to_block(blocks[&handle]);
            }
            for inst in &block.insts {
                self.lower_inst(&mut builder, func, inst, &mut values, &blocks)?;
            }
            if let Some(term) = &block.term {
                self.lower_term(&mut builder, func, term, &values, &blocks)?;
            }
        }
        // Every block is filled by now, so every block is sealed: nothing will add a
        // predecessor after this point.
        for block in blocks.values() {
            builder.seal_block(*block);
        }
        builder.finalize();

        let name = self
            .object
            .declare_function(&name, Linkage::Export, &context.func.signature)?;
        self.object
            .define_function(name, &mut context)
            .map_err(|error| LoweringError(format!("cranelift: {error:?}")))?;
        Ok(())
    }

    fn lower_inst(
        &mut self,
        builder: &mut FunctionBuilder<'_>,
        _func: &Func,
        inst: &Inst,
        values: &mut HashMap<cir::Value, ClValue>,
        blocks: &HashMap<Block, ir::Block>,
    ) -> Result<(), LoweringError> {
        let _ = blocks;
        let value = |values: &HashMap<cir::Value, ClValue>, v: cir::Value| {
            values
                .get(&v)
                .copied()
                .ok_or_else(|| LoweringError(String::from("a value has no Cranelift value")))
        };
        let pointer = ir::types::I64;
        match inst {
            Inst::Const {
                result, ty, bits, ..
            } => {
                let machine = self.machine_type(*ty)?;
                let constant = if self.types.is_float(*ty) {
                    if machine == ir::types::F32 {
                        builder.ins().f32const(f32::from_bits(*bits as u32))
                    } else {
                        builder.ins().f64const(f64::from_bits(*bits))
                    }
                } else {
                    builder.ins().iconst(machine, *bits as i64)
                };
                values.insert(*result, constant);
            }
            Inst::Slot { result, ty, .. } => {
                let size = self
                    .types
                    .size_of(*ty)
                    .ok_or_else(|| LoweringError("a slot for an incomplete type".to_owned()))?;
                let slot = builder.create_sized_stack_slot(ir::StackSlotData::new(
                    ir::StackSlotKind::ExplicitSlot,
                    size as u32,
                    0,
                ));
                let address = builder.ins().stack_addr(pointer, slot, 0);
                values.insert(*result, address);
            }
            Inst::Load {
                result,
                ty,
                address,
                ..
            } => {
                let machine = self.machine_type(*ty)?;
                let address = value(values, *address)?;
                let loaded = builder.ins().load(machine, MemFlags::trusted(), address, 0);
                values.insert(*result, loaded);
            }
            Inst::Store {
                address,
                value: what,
                ..
            } => {
                let address = value(values, *address)?;
                let what = value(values, *what)?;
                builder.ins().store(MemFlags::trusted(), what, address, 0);
            }
            Inst::Binary {
                result,
                op,
                ty,
                left,
                right,
                ..
            } => {
                let (left, right) = (value(values, *left)?, value(values, *right)?);
                let signed = matches!(self.types.kind(*ty), Type::Int { signed: true, .. });
                let float = self.types.is_float(*ty);
                let computed = match (op, float, signed) {
                    (BinOp::Add, true, _) => builder.ins().fadd(left, right),
                    (BinOp::Sub, true, _) => builder.ins().fsub(left, right),
                    (BinOp::Mul, true, _) => builder.ins().fmul(left, right),
                    (BinOp::Div, true, _) => builder.ins().fdiv(left, right),
                    (BinOp::Add, false, _) => builder.ins().iadd(left, right),
                    (BinOp::Sub, false, _) => builder.ins().isub(left, right),
                    (BinOp::Mul, false, _) => builder.ins().imul(left, right),
                    (BinOp::Div, false, true) => builder.ins().sdiv(left, right),
                    (BinOp::Div, false, false) => builder.ins().udiv(left, right),
                    (BinOp::Rem, false, true) => builder.ins().srem(left, right),
                    (BinOp::Rem, false, false) => builder.ins().urem(left, right),
                    (BinOp::And, false, _) => builder.ins().band(left, right),
                    (BinOp::Or, false, _) => builder.ins().bor(left, right),
                    (BinOp::Xor, false, _) => builder.ins().bxor(left, right),
                    (BinOp::Shl, false, _) => builder.ins().ishl(left, right),
                    (BinOp::Shr, false, true) => builder.ins().sshr(left, right),
                    (BinOp::Shr, false, false) => builder.ins().ushr(left, right),
                    (op, ..) => {
                        return Err(LoweringError(format!(
                            "`{}` has no lowering for this type",
                            op.as_str()
                        )))
                    }
                };
                values.insert(*result, computed);
            }
            Inst::Compare {
                result,
                op,
                ty,
                left,
                right,
                ..
            } => {
                let (left, right) = (value(values, *left)?, value(values, *right)?);
                let signed = matches!(self.types.kind(*ty), Type::Int { signed: true, .. });
                let float = self.types.is_float(*ty);
                let comparison = match (op, float, signed) {
                    (CmpOp::Eq, _, _) => ir::condcodes::IntCC::Equal,
                    (CmpOp::Ne, _, _) => ir::condcodes::IntCC::NotEqual,
                    (CmpOp::Less, false, true) => ir::condcodes::IntCC::SignedLessThan,
                    (CmpOp::Less, false, false) => ir::condcodes::IntCC::UnsignedLessThan,
                    (CmpOp::LessEqual, false, true) => ir::condcodes::IntCC::SignedLessThanOrEqual,
                    (CmpOp::LessEqual, false, false) => {
                        ir::condcodes::IntCC::UnsignedLessThanOrEqual
                    }
                    (CmpOp::Greater, false, true) => ir::condcodes::IntCC::SignedGreaterThan,
                    (CmpOp::Greater, false, false) => ir::condcodes::IntCC::UnsignedGreaterThan,
                    (CmpOp::GreaterEqual, false, true) => {
                        ir::condcodes::IntCC::SignedGreaterThanOrEqual
                    }
                    (CmpOp::GreaterEqual, false, false) => {
                        ir::condcodes::IntCC::UnsignedGreaterThanOrEqual
                    }
                    (_, true, _) => {
                        let code = match op {
                            CmpOp::Eq => ir::condcodes::FloatCC::Equal,
                            CmpOp::Ne => ir::condcodes::FloatCC::NotEqual,
                            CmpOp::Less => ir::condcodes::FloatCC::LessThan,
                            CmpOp::LessEqual => ir::condcodes::FloatCC::LessThanOrEqual,
                            CmpOp::Greater => ir::condcodes::FloatCC::GreaterThan,
                            CmpOp::GreaterEqual => ir::condcodes::FloatCC::GreaterThanOrEqual,
                        };
                        let flag = builder.ins().fcmp(code, left, right);
                        return self.widen_flag(builder, values, *result, flag);
                    }
                };
                let flag = builder.ins().icmp(comparison, left, right);
                self.widen_flag(builder, values, *result, flag)?;
            }
            Inst::Unary {
                result,
                op,
                ty,
                value: operand,
                ..
            } => {
                let operand = value(values, *operand)?;
                let computed = match (op, self.types.is_float(*ty)) {
                    (cir::UnOp::Neg, true) => builder.ins().fneg(operand),
                    (cir::UnOp::Neg, false) => builder.ins().ineg(operand),
                    (cir::UnOp::Not, _) => builder.ins().bnot(operand),
                };
                values.insert(*result, computed);
            }
            Inst::Convert {
                result,
                op,
                from,
                to,
                value: operand,
                ..
            } => {
                let operand = value(values, *operand)?;
                let (from, to) = (self.machine_type(*from)?, self.machine_type(*to)?);
                let converted = match op {
                    ConvertOp::Bitcast => {
                        if from == to {
                            operand
                        } else {
                            builder.ins().bitcast(to, MemFlags::new(), operand)
                        }
                    }
                    ConvertOp::Truncate => builder.ins().ireduce(to, operand),
                    ConvertOp::SignExtend => builder.ins().sextend(to, operand),
                    ConvertOp::ZeroExtend => builder.ins().uextend(to, operand),
                    ConvertOp::IntToFloat => builder.ins().fcvt_from_sint(to, operand),
                    ConvertOp::FloatToInt => builder.ins().fcvt_to_sint(to, operand),
                    ConvertOp::FloatExtend => builder.ins().fpromote(to, operand),
                    ConvertOp::FloatTruncate => builder.ins().fdemote(to, operand),
                };
                values.insert(*result, converted);
            }
            Inst::Call {
                result,
                callee,
                args,
                ..
            } => {
                let _ = callee;
                let _ = args;
                let _ = result;
                return Err(LoweringError(
                    "calls are lowered when the JIT needs them (M4.4)".to_owned(),
                ));
            }
            Inst::GlobalAddress { .. } => {
                return Err(LoweringError(
                    "global addresses are lowered with globals (M5)".to_owned(),
                ));
            }
        }
        Ok(())
    }

    /// A comparison produces a C `int`, so the flag becomes 1 or 0 in the result's width.
    fn widen_flag(
        &mut self,
        builder: &mut FunctionBuilder<'_>,
        values: &mut HashMap<cir::Value, ClValue>,
        result: cir::Value,
        flag: ClValue,
    ) -> Result<(), LoweringError> {
        let widened = builder.ins().uextend(ir::types::I32, flag);
        values.insert(result, widened);
        Ok(())
    }

    fn lower_term(
        &mut self,
        builder: &mut FunctionBuilder<'_>,
        func: &Func,
        term: &Term,
        values: &HashMap<cir::Value, ClValue>,
        blocks: &HashMap<Block, ir::Block>,
    ) -> Result<(), LoweringError> {
        let _ = func;
        let value = |v: cir::Value| {
            values
                .get(&v)
                .copied()
                .ok_or_else(|| LoweringError(String::from("a value has no Cranelift value")))
        };
        match term {
            Term::Return { value: what, .. } => match what {
                Some(what) => {
                    let what = value(*what)?;
                    builder.ins().return_(&[what]);
                }
                None => {
                    builder.ins().return_(&[]);
                }
            },
            Term::Jump { target, args, .. } => {
                let args: Vec<ClValue> = args
                    .iter()
                    .map(|arg| value(*arg))
                    .collect::<Result<_, _>>()?;
                builder.ins().jump(blocks[target], &args);
            }
            Term::Branch {
                condition,
                then,
                otherwise,
                ..
            } => {
                let condition = value(*condition)?;
                // The zero has to be the condition's own width: `icmp` compares like with like.
                let width = builder.func.dfg.value_type(condition);
                let zero = builder.ins().iconst(width, 0);
                let truth = builder
                    .ins()
                    .icmp(ir::condcodes::IntCC::NotEqual, condition, zero);
                let then_args: Vec<ClValue> = then
                    .1
                    .iter()
                    .map(|arg| value(*arg))
                    .collect::<Result<_, _>>()?;
                let else_args: Vec<ClValue> = otherwise
                    .1
                    .iter()
                    .map(|arg| value(*arg))
                    .collect::<Result<_, _>>()?;
                builder.ins().brif(
                    truth,
                    blocks[&then.0],
                    &then_args,
                    blocks[&otherwise.0],
                    &else_args,
                );
            }
            Term::Unreachable { .. } => {
                builder.ins().trap(ir::TrapCode::unwrap_user(1_u8));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::Options;
    use crate::pp::preprocess;
    use crate::source::SourceFile;

    /// Compile `source` and give back its object file.
    fn object_of(source: &str) -> Vec<u8> {
        let file = SourceFile::new("t.c", source);
        let options = Options::default();
        let preprocessed = preprocess(&file, &options, &());
        let parsed = crate::parse::parse(&preprocessed, &options.budgets);
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        object(&parsed.module, preprocessed.names()).expect("lowering works")
    }

    #[test]
    fn a_function_becomes_a_coff_object() {
        let bytes = object_of("int add(int a, int b) { return a + b; }");
        // IMAGE_FILE_MACHINE_AMD64, then the section count: a COFF object, not nothing.
        assert!(bytes.len() > 100, "{} bytes", bytes.len());
        assert_eq!(&bytes[0..2], &[0x64, 0x86], "the machine is x86-64");
        // The function's name is in the symbol table, which is how a linker finds it.
        assert!(
            bytes.windows(3).any(|window| window == b"add"),
            "the object names the function"
        );
    }
}
