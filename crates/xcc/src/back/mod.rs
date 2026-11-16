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
use cranelift_module::{
    default_libcall_names, DataDescription, DataId, FuncId, Linkage, Module, ModuleError,
};
use cranelift_object::{ObjectBuilder, ObjectModule};

pub mod jit;

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
    // A frame larger than a page must touch its guard pages as it allocates them, or a stack
    // overflow silently overwrites whatever is below. Inline, so no `__chkstk` import (ADR-005).
    flags
        .set("enable_probestack", "true")
        .map_err(|error| LoweringError(error.to_string()))?;
    flags
        .set("probestack_strategy", "inline")
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
        functions: HashMap::new(),
        data: HashMap::new(),
    };
    // Every function is declared before any is lowered, so calls can be forward references.
    lower_all(&mut lowerer, module)?;
    object
        .finish()
        .emit()
        .map_err(|error| LoweringError(error.to_string()))
}

/// Declare every function, then lower every body, giving back the ids by name.
///
/// Declaring first is what makes a call to a function defined later resolvable, and it is why a
/// JIT can hand out the address of anything in the unit after one pass.
pub fn lower_into<M: Module>(
    object: &mut M,
    module: &CirModule,
    names: &crate::intern::Interner,
) -> Result<Vec<(String, FuncId)>, LoweringError> {
    let mut lowerer = Lowerer {
        object,
        types: &module.types,
        names,
        functions: HashMap::new(),
        data: HashMap::new(),
    };
    lower_all(&mut lowerer, module)?;
    Ok(lowerer
        .functions
        .iter()
        .map(|(name, id)| (name.clone(), *id))
        .collect())
}

fn lower_all<M: Module>(
    lowerer: &mut Lowerer<'_, M>,
    module: &CirModule,
) -> Result<(), LoweringError> {
    // Objects first, so a body can take the address of one declared later.
    for global in &module.globals {
        lowerer.declare_data(global)?;
    }
    for func in &module.funcs {
        lowerer.declare(func)?;
    }
    for func in &module.funcs {
        lowerer.lower(func)?;
    }
    Ok(())
}

/// Holds the object being built and the type table the values name.
struct Lowerer<'a, M: Module> {
    object: &'a mut M,
    types: &'a Types,
    names: &'a crate::intern::Interner,
    /// The function each declared name means, so lowering a call can name it.
    functions: HashMap<String, FuncId>,
    /// The objects each declared name means, so lowering an address can name one.
    data: HashMap<String, DataId>,
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
    ///
    /// One place, because a body's own signature and the signature of a function it calls are
    /// the same question about the same C type.
    fn signature_of(&self, params: &[TypeId], result: TypeId) -> Result<Signature, LoweringError> {
        let mut signature = self.object.make_signature();
        // A function this unit does not define is declared from its *C* type, so the ABI's
        // aggregate rules have to be applied here: a record too big for a register is
        // returned through a leading pointer the caller owns, and passed the same way.
        let (hidden, returned) = self.abi_result(result)?;
        if let Some(hidden) = hidden {
            signature.params.push(AbiParam::new(hidden));
        }
        for param in params {
            let ty = self.abi_value(*param)?;
            signature.params.push(AbiParam::new(ty));
        }
        if let Some(returned) = returned {
            signature.returns.push(AbiParam::new(returned));
        }
        Ok(signature)
    }

    /// The machine type a C value crosses the ABI as: a record of 1, 2, 4 or 8 bytes is an
    /// integer that wide, any other record is a pointer to a caller-owned copy, and everything
    /// else is its own machine type.
    fn abi_value(&self, ty: TypeId) -> Result<ir::Type, LoweringError> {
        if self.types.record_of(ty).is_none() {
            return self.machine_type(ty);
        }
        Ok(match self.types.size_of(ty) {
            Some(1) => ir::types::I8,
            Some(2) => ir::types::I16,
            Some(4) => ir::types::I32,
            _ => ir::types::I64,
        })
    }

    /// A record too big for a register comes back through a buffer the caller owns: the hidden
    /// leading pointer, and the address the callee returns in `rax`.
    fn abi_result(
        &self,
        result: TypeId,
    ) -> Result<(Option<ir::Type>, Option<ir::Type>), LoweringError> {
        if matches!(self.types.kind(result), Type::Void) {
            return Ok((None, None));
        }
        if self.types.record_of(result).is_none() {
            return Ok((None, Some(self.machine_type(result)?)));
        }
        Ok(match self.types.size_of(result) {
            Some(1 | 2 | 4 | 8) => (None, Some(self.abi_value(result)?)),
            _ => (Some(ir::types::I64), Some(ir::types::I64)),
        })
    }

    /// The signature of a function this module defines.
    fn signature(&self, func: &Func) -> Result<Signature, LoweringError> {
        let params: Vec<TypeId> = func
            .params()
            .iter()
            .map(|param| func.value(*param).ty)
            .collect();
        self.signature_of(&params, func.result)
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
        self.functions.insert(self.name(func).to_owned(), id);
        Ok(())
    }

    /// Put a global object in the object file: its size and alignment from the C type, its
    /// bytes from the front end (ADR-004), or zeroes when the source said nothing.
    fn declare_data(&mut self, global: &crate::cir::Global) -> Result<(), LoweringError> {
        let size = self
            .types
            .size_of(global.ty)
            .ok_or_else(|| LoweringError(format!("the object `{}` has no size", global.name)))?;
        let linkage = if global.external {
            Linkage::Export
        } else {
            Linkage::Local
        };
        let id = self
            .object
            .declare_data(&global.name, linkage, true, false)?;
        let mut description = DataDescription::new();
        match &global.init {
            Some(bytes) => {
                description.define(bytes.clone().into_boxed_slice());
            }
            // A tentative definition holds zeroes: `.bss`, which costs nothing in the file.
            None => description.define_zeroinit(size as usize),
        }
        description.align = Some(self.types.align_of(global.ty).unwrap_or(1).max(1));
        self.object.define_data(id, &description)?;
        self.data.insert(global.name.clone(), id);
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
        // The functions this body can call, by the value its address was minted as.
        let mut callees: HashMap<cir::Value, ir::FuncRef> = HashMap::new();
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
                self.lower_inst(&mut builder, func, inst, &mut values, &mut callees)?;
            }
            if let Some(term) = &block.term {
                self.lower_term(&mut builder, term, &values, &blocks)?;
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
        func: &Func,
        inst: &Inst,
        values: &mut HashMap<cir::Value, ClValue>,
        callees: &mut HashMap<cir::Value, ir::FuncRef>,
    ) -> Result<(), LoweringError> {
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
                // A float is compared with `fcmp` for every operator: `icmp.f64` is not an
                // instruction, and C's `!=` is the *unordered* one, which is true for a NaN.
                if self.types.is_float(*ty) {
                    let code = match op {
                        CmpOp::Eq => ir::condcodes::FloatCC::Equal,
                        CmpOp::Ne => ir::condcodes::FloatCC::NotEqual,
                        CmpOp::Less => ir::condcodes::FloatCC::LessThan,
                        CmpOp::LessEqual => ir::condcodes::FloatCC::LessThanOrEqual,
                        CmpOp::Greater => ir::condcodes::FloatCC::GreaterThan,
                        CmpOp::GreaterEqual => ir::condcodes::FloatCC::GreaterThanOrEqual,
                    };
                    let flag = builder.ins().fcmp(code, left, right);
                    let result_ty = self.machine_type(func.value(*result).ty)?;
                    Self::widen_flag(builder, values, *result, result_ty, flag);
                    return Ok(());
                }
                let comparison = match (op, signed) {
                    (CmpOp::Eq, _) => ir::condcodes::IntCC::Equal,
                    (CmpOp::Ne, _) => ir::condcodes::IntCC::NotEqual,
                    (CmpOp::Less, true) => ir::condcodes::IntCC::SignedLessThan,
                    (CmpOp::Less, false) => ir::condcodes::IntCC::UnsignedLessThan,
                    (CmpOp::LessEqual, true) => ir::condcodes::IntCC::SignedLessThanOrEqual,
                    (CmpOp::LessEqual, false) => ir::condcodes::IntCC::UnsignedLessThanOrEqual,
                    (CmpOp::Greater, true) => ir::condcodes::IntCC::SignedGreaterThan,
                    (CmpOp::Greater, false) => ir::condcodes::IntCC::UnsignedGreaterThan,
                    (CmpOp::GreaterEqual, true) => ir::condcodes::IntCC::SignedGreaterThanOrEqual,
                    (CmpOp::GreaterEqual, false) => {
                        ir::condcodes::IntCC::UnsignedGreaterThanOrEqual
                    }
                };
                let flag = builder.ins().icmp(comparison, left, right);
                let result_ty = self.machine_type(func.value(*result).ty)?;
                Self::widen_flag(builder, values, *result, result_ty, flag);
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
                // The C types as well as the machine ones: an unsigned integer converts from and
                // to unsigned, and only the C type says which it is.
                let (from_ty, to_ty) = (*from, *to);
                let (from, to) = (self.machine_type(from_ty)?, self.machine_type(to_ty)?);
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
                    // An unsigned integer converts from and to unsigned: `fcvt_from_sint` of
                    // 4e9 held in 32 bits would come out negative. x64's convert takes a 32- or
                    // 64-bit integer, so a `char` or a `short` is widened on the way in and the
                    // result is narrowed on the way out.
                    ConvertOp::IntToFloat => {
                        let unsigned =
                            matches!(self.types.kind(from_ty), Type::Int { signed: false, .. });
                        let operand = match from {
                            ir::types::I8 | ir::types::I16 if unsigned => {
                                builder.ins().uextend(ir::types::I32, operand)
                            }
                            ir::types::I8 | ir::types::I16 => {
                                builder.ins().sextend(ir::types::I32, operand)
                            }
                            _ => operand,
                        };
                        if unsigned {
                            builder.ins().fcvt_from_uint(to, operand)
                        } else {
                            builder.ins().fcvt_from_sint(to, operand)
                        }
                    }
                    ConvertOp::FloatToInt => {
                        let unsigned =
                            matches!(self.types.kind(to_ty), Type::Int { signed: false, .. });
                        let wide = match to {
                            ir::types::I8 | ir::types::I16 => ir::types::I32,
                            _ => to,
                        };
                        let converted = if unsigned {
                            builder.ins().fcvt_to_uint(wide, operand)
                        } else {
                            builder.ins().fcvt_to_sint(wide, operand)
                        };
                        if wide == to {
                            converted
                        } else {
                            builder.ins().ireduce(to, converted)
                        }
                    }
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
                // The callee's address was minted by a `GlobalAddress` in this body; a call
                // through a function *pointer* is M4.5's problem, and says so.
                let target = callees.get(callee).copied().ok_or_else(|| {
                    LoweringError("a call through a function pointer is not lowered yet".to_owned())
                })?;
                let args: Vec<ClValue> = args
                    .iter()
                    .map(|arg| value(values, *arg))
                    .collect::<Result<_, _>>()?;
                let call = builder.ins().call(target, &args);
                if let Some(result) = result {
                    let produced = builder.inst_results(call)[0];
                    values.insert(*result, produced);
                }
            }
            Inst::GlobalAddress { result, name, .. } => {
                // The C type of the address says what the function's signature is, so the
                // signature is read from the type rather than carried twice.
                let pointer = func.value(*result).ty;
                let function = self.types.pointee(pointer);
                let (params, returns) = match self.types.kind(function) {
                    Type::Function { result, params, .. } => (params.clone(), *result),
                    // An object: its address is the symbol value the linker will resolve.
                    _ => {
                        let id = self.data.get(name.as_str()).copied().ok_or_else(|| {
                            LoweringError(format!("`{name}` is not declared here"))
                        })?;
                        let address = self.object.declare_data_in_func(id, builder.func);
                        let value = builder.ins().symbol_value(ir::types::I64, address);
                        values.insert(*result, value);
                        return Ok(());
                    }
                };
                // A function of this unit was declared before any body was lowered, so its id
                // is already known; only something outside the unit is an import.
                let id = match self.functions.get(name.as_str()) {
                    Some(id) => *id,
                    None => {
                        let signature = self.signature_of(&params, returns)?;
                        self.object
                            .declare_function(name.as_str(), Linkage::Import, &signature)?
                    }
                };
                let target = self.object.declare_func_in_func(id, builder.func);
                let address = builder.ins().func_addr(ir::types::I64, target);
                values.insert(*result, address);
                callees.insert(*result, target);
            }
        }
        Ok(())
    }

    /// A comparison produces a flag; the C type of the result says how wide the 0 or 1 is - `int`
    /// for a condition, `_Bool` for a conversion (which is already one byte).
    fn widen_flag(
        builder: &mut FunctionBuilder<'_>,
        values: &mut HashMap<cir::Value, ClValue>,
        result: cir::Value,
        result_ty: ir::Type,
        flag: ClValue,
    ) {
        let widened = if result_ty == ir::types::I8 {
            flag
        } else {
            builder.ins().uextend(result_ty, flag)
        };
        values.insert(result, widened);
    }

    fn lower_term(
        &mut self,
        builder: &mut FunctionBuilder<'_>,
        term: &Term,
        values: &HashMap<cir::Value, ClValue>,
        blocks: &HashMap<Block, ir::Block>,
    ) -> Result<(), LoweringError> {
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

    /// The gate PLAN.md puts before M5: does a win64 object from Cranelift carry the unwind
    /// information Windows expects, or must M5 supply it?
    #[test]
    fn a_win64_object_says_which_unwind_data_it_has() {
        let bytes = object_of("int add(int a, int b) { return a + b; }");
        let has = |section: &[u8]| bytes.windows(section.len()).any(|window| window == section);
        // COFF keeps a section name of eight bytes or fewer in its header, so a plain scan sees
        // it. The assertions *are* the verdict ADR-003 records: if Cranelift starts emitting
        // unwind data, this test fails and the ADR is revisited rather than left to rot.
        assert!(has(b".text"), "there is code");
        assert!(
            !has(b".pdata") && !has(b".xdata"),
            "a Cranelift win64 object now carries unwind data: see ADR-003 and M5"
        );
    }

    /// Data in the object: a string literal is an anonymous global (ADR-004), so an object for
    /// a function that returns one has a section beside `.text` and a symbol for the literal.
    #[test]
    fn a_string_literal_becomes_data_in_the_object() {
        let bytes = object_of("char *g(void) { return \"hi\"; }");
        let has = |needle: &[u8]| bytes.windows(needle.len()).any(|window| window == needle);
        assert!(has(b".str0"), "the literal has a link name");
        assert!(has(b"hi"), "and its bytes are in the file");
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
