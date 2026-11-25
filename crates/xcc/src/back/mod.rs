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

pub mod crt;

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
pub mod x64;

use crate::cir::{self, BinOp, Block, CmpOp, ConvertOp, Func, Inst, Module as CirModule, Term};
use crate::types::{Type, TypeId, Types};

/// Why lowering failed.
///
/// Two kinds, and the difference matters to whoever reads the diagnostic: a **bug** is the
/// compiler's fault and says `internal:`, while a **gap** is a construct the back end has not
/// lowered yet, and is reported like any other refusal - at the expression that asked for it.
#[derive(Debug)]
pub struct LoweringError {
    /// What went wrong, in one sentence.
    pub message: String,
    /// The source expression, when this is a documented gap and not a bug.
    pub not_lowered: Option<Span>,
}

impl LoweringError {
    /// A compiler bug: something the back end was not supposed to be able to fail at.
    fn bug(message: String) -> Self {
        Self {
            message,
            not_lowered: None,
        }
    }

    /// A construct this back end has not learnt yet.
    pub(crate) fn not_lowered(message: String, span: Span) -> Self {
        Self {
            message,
            not_lowered: Some(span),
        }
    }
}

impl std::fmt::Display for LoweringError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<ModuleError> for LoweringError {
    fn from(error: ModuleError) -> Self {
        Self::bug(format!("cranelift module: {error}"))
    }
}

use crate::diag::Span;

/// One function as the compiler saw it and as Cranelift built it: what `--explain` prints.
#[derive(Debug, Clone)]
pub struct Explained {
    /// The function's name in C.
    pub name: String,
    /// Its C-level instructions in program order, each with the source span it came from.
    pub cir: Vec<(Span, String)>,
    /// The Cranelift function after lowering, which is where the virtual registers the back end
    /// chose are visible: `v3 = iadd v1, v2`.
    pub clif: String,
}

/// Every block, each after the blocks that dominate it: preorder of the dominator tree, then any
/// block the entry cannot reach.
pub(super) fn dominator_order(func: &Func) -> Vec<Block> {
    let tree = func.dominator_tree();
    let mut children: HashMap<Block, Vec<Block>> = HashMap::new();
    for (index, parent) in tree.iter().enumerate() {
        if let Some(parent) = parent {
            children
                .entry(*parent)
                .or_default()
                .push(Block(index as u32));
        }
    }
    let mut order = Vec::new();
    let mut stack = vec![func.entry];
    while let Some(block) = stack.pop() {
        order.push(block);
        for child in children.get(&block).into_iter().flatten().rev() {
            stack.push(*child);
        }
    }
    // A block nothing dominates (an orphan the parser made and never filled) still has to be
    // lowered: it is in the function, and the back end fills it with `unreachable`.
    for (index, _) in func.blocks.iter().enumerate() {
        let block = Block(index as u32);
        if !order.contains(&block) {
            order.push(block);
        }
    }
    order
}

/// What lowering a translation unit produced.
#[derive(Debug)]
pub struct Lowered {
    /// The object file.
    pub bytes: Vec<u8>,
    /// Each function, when the caller asked to be told what was made of it. Empty otherwise, and
    /// formatting the Cranelift function is most of the cost of asking.
    pub explained: Vec<Explained>,
}

/// Which levels exist, and why one does not. Level 2 is this compiler's own back end (ADR-008); a
/// number above it is a level nobody wrote, and saying so beats quietly serving another one.
pub(crate) fn check_opt_level(level: u8) -> Result<(), String> {
    match level {
        0..=2 => Ok(()),
        other => Err(format!(
            "opt level {other} is not written: 0 and 1 are Cranelift's, 2 is this compiler's own back end"
        )),
    }
}

/// How hard *Cranelift* works, as it spells it.
///
/// Level 2 is not Cranelift's: it is this compiler's own back end, which lowers the same `cir` and
/// loads its own code (ADR-008), so asking this function about it is asking the wrong back end - the
/// JIT decides first and only comes here for levels 0 and 1.
pub(crate) fn cranelift_opt_level(level: u8) -> Result<&'static str, String> {
    match level {
        0 => Ok("none"),
        1 => Ok("speed"),
        2 => Err(String::from(
            "opt level 2 is this compiler's own back end and does not go through Cranelift",
        )),
        other => Err(format!(
            "opt level {other} is not written: 0 and 1 are Cranelift's, 2 is this compiler's own back end"
        )),
    }
}

/// Lower a translation unit to an object file, keeping what each function became when `explain`
/// is set.
/// The C runtime's entry point, compiled with every program a link produces.
pub(crate) const CRT_SOURCE: &str = include_str!("../../../xcc-crt/src/main.c");

/// Where a name a program does not define comes from: the library, and the name it has there.
///
/// Two answers, because a Windows console program imports from two libraries: the C runtime, whose
/// names are the headers' own list, and `kernel32`, which has the one function that ends a process.
/// A name neither has is `None` - the caller reports it as undefined rather than guessing.
///
/// **A name and the name the library exports are not always the same string.** `time_t` is 64-bit on
/// this target, and the C runtime exports `_time64`, `_gmtime64` and the rest under those names -
/// which is also why a `clang`-built program imports `_time64` from the time api-set rather than
/// `time` from `ucrtbase`. This function is the only place that knows the difference, so no caller
/// and no part of the runtime has to spell a library's own name.
/// Whether the *linker* makes this name, rather than a unit or a library: the three things the
/// thread-local lookup reads - one offset word per object, the block's size, and the template every
/// thread's copy starts from.
///
/// They exist because a `_Thread_local` object's place in a thread's block is a fact only the
/// linker has - it is the one that lays the block out, after every unit is compiled - so the code
/// that reads it is compiled against a word the linker will fill. `compile.rs` asks here before it
/// calls a name undefined, and the block layout in `pe.rs` is what defines them.
pub(crate) fn made_by_the_linker(name: &str) -> bool {
    name == "__xcc_tls_total"
        || name == "__xcc_tls_template"
        || name.starts_with("__xcc_tls_offset_")
}

pub(crate) fn import_for(name: &str) -> Option<(&'static str, String)> {
    // The four the kernel exports rather than the C runtime. `TlsAlloc` is the trap: it appears in
    // `ucrtbase.dll`'s dump, but as an *import* of the kernel's, not as one of its own exports - the
    // loader said so with `0xC0000139` on every program that linked, which is how it was found.
    const KERNEL32: [&str; 6] = [
        "ExitProcess",
        "TlsAlloc",
        "TlsGetValue",
        "TlsSetValue",
        // The two a *test* waits on a thread with: `tests/exe.rs` starts a second thread to show
        // that a `_Thread_local` object has a copy per thread, and that needs the wait.
        "WaitForSingleObject",
        "CloseHandle",
    ];
    if let Some(name) = KERNEL32.iter().find(|kernel| **kernel == name) {
        return Some(("kernel32.dll", (*name).to_owned()));
    }
    if let Some((name, _)) = crt::BEYOND_THE_HEADERS
        .iter()
        .find(|(known, _)| *known == name)
    {
        return Some(("ucrtbase.dll", (*name).to_owned()));
    }
    if !crt::C_RUNTIME.contains(&name) {
        return None;
    }
    Some(("ucrtbase.dll", exported_as(name).to_owned()))
}

/// The names the C runtime exports under a spelling of its own: the 64-bit `time_t` ABI.
///
/// Each was measured against `ucrtbase.dll`'s export table before it was written here - `difftime`,
/// `gmtime`, `localtime` and `mktime` have no `time`-less spelling in the library at all.
const RENAMED_IN_THE_LIBRARY: &[(&str, &str)] = &[
    ("time", "_time64"),
    ("difftime", "_difftime64"),
    ("gmtime", "_gmtime64"),
    ("localtime", "_localtime64"),
    ("mktime", "_mktime64"),
    // `setjmp` has to be the library's own function, because it saves the *caller's* frame; the
    // name in the library is the intrinsic one, which is also what a `clang`-built program imports.
    // It takes the environment *and* the frame to unwind to - `setjmp.h` passes both, and the
    // measurement behind that is in its comment.
    ("_setjmp", "__intrinsic_setjmp"),
];

/// What `name` is called in the library it comes from.
fn exported_as(name: &str) -> &str {
    match RENAMED_IN_THE_LIBRARY
        .iter()
        .find(|(ours, _)| *ours == name)
    {
        Some((_, theirs)) => theirs,
        None => name,
    }
}

/// The image of one translation unit, for a linker: code, data, symbols, relocations and the
/// unwinder's description of every function.
///
/// Only level 2 has one. Levels 0 and 1 are Cranelift, which owns its own object writer and never
/// shows the sections it made, so a program that is to be *linked* has to be lowered by this
/// compiler's own back end - the same rule as `va_arg`, for the same reason, and stated once here.
pub(crate) fn image(
    module: &CirModule,
    names: &crate::intern::Interner,
    opt_level: u8,
) -> Result<Option<x64::coff::Image>, LoweringError> {
    if opt_level < 2 {
        return Ok(None);
    }
    let image = x64::image(module, names)
        .map_err(|gap| LoweringError::not_lowered(format!("{gap}"), gap.span))?;
    Ok(Some(image))
}

pub fn object(
    module: &CirModule,
    names: &crate::intern::Interner,
    explain: bool,
    opt_level: u8,
) -> Result<Lowered, LoweringError> {
    // Level 2 is this compiler's own back end (ADR-008): Cranelift is not in the path at all, so a
    // number that says "the compiler's own code" cannot quietly mean somebody else's.
    if opt_level >= 2 {
        let bytes = x64::object(module, names)
            .map_err(|gap| LoweringError::not_lowered(format!("{gap}"), gap.span))?;
        return Ok(Lowered {
            bytes,
            explained: Vec::new(),
        });
    }
    let mut flags = settings::builder();
    // The host's own feature set, so a JIT and an object cannot disagree about what runs.
    flags
        .set(
            "opt_level",
            cranelift_opt_level(opt_level).map_err(LoweringError::bug)?,
        )
        .map_err(|error| LoweringError::bug(error.to_string()))?;
    // A frame larger than a page must touch its guard pages as it allocates them, or a stack
    // overflow silently overwrites whatever is below. Inline, so no `__chkstk` import (ADR-005).
    flags
        .set("enable_probestack", "true")
        .map_err(|error| LoweringError::bug(error.to_string()))?;
    flags
        .set("probestack_strategy", "inline")
        .map_err(|error| LoweringError::bug(error.to_string()))?;
    let flags = settings::Flags::new(flags);
    let isa = cranelift_native::builder()
        .map_err(|error| LoweringError::bug(error.to_string()))?
        .finish(flags)
        .map_err(|error| LoweringError::bug(error.to_string()))?;
    let builder = ObjectBuilder::new(isa, "xcc", default_libcall_names())?;
    let mut object = ObjectModule::new(builder);
    let mut lowerer = Lowerer {
        object: &mut object,
        types: &module.types,
        names,
        functions: HashMap::new(),
        data: HashMap::new(),
        explained: explain.then(Vec::new),
    };
    // Every function is declared before any is lowered, so calls can be forward references.
    lower_all(&mut lowerer, module)?;
    let explained = lowerer.explained.take().unwrap_or_default();
    drop(lowerer); // the object cannot be finished while the lowerer still holds it
    let bytes = object
        .finish()
        .emit()
        .map_err(|error| LoweringError::bug(error.to_string()))?;
    Ok(Lowered { bytes, explained })
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
        explained: None,
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
    /// Every function as it was made, when the caller asked to be told - `None` otherwise, which
    /// is why a normal compile pays nothing for the feature.
    explained: Option<Vec<Explained>>,
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
                    return Err(LoweringError::bug(format!(
                        "no Cranelift type for {other}-bit int"
                    )))
                }
            },
            Type::Float { bits } => match bits {
                32 => ir::types::F32,
                64 => ir::types::F64,
                other => {
                    return Err(LoweringError::bug(format!(
                        "no Cranelift type for {other}-bit float"
                    )))
                }
            },
            Type::Pointer(_) => ir::types::I64, // the target's word (ADR-001 decision 5)
            _other => {
                return Err(LoweringError::bug(format!(
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

    /// The signature a *pointer* to a function calls with: the function type it points at, read
    /// the same way a declaration's is.
    fn signature_of_pointer(&self, pointer: TypeId) -> Result<Signature, LoweringError> {
        let function = self.types.pointee(pointer);
        match self.types.kind(function) {
            Type::Function { result, params, .. } => self.signature_of(params, *result),
            _ => Err(LoweringError::bug(
                "a call of something that is not a function".to_owned(),
            )),
        }
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
        let size = self.types.size_of(global.ty).ok_or_else(|| {
            LoweringError::bug(format!("the object `{}` has no size", global.name))
        })?;
        let linkage = if global.external {
            Linkage::Export
        } else {
            Linkage::Local
        };
        // An initialiser that holds an address needs a relocation in the data section. Our own back
        // end writes one; this one cannot name the symbol the way `cranelift-object` wants it, so it
        // refuses instead of writing eight zero bytes and calling that a pointer (ADR-004).
        if !global.relocations.is_empty() {
            return Err(LoweringError::not_lowered(
                "an initialiser that holds an address of another object - `char *p = \"hello\"` at \
                 file scope. Level 2 writes the relocation"
                    .into(),
                global.span,
            ));
        }
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
        // Blocks in dominator-tree order, not in the order the parser made them: a use may name a
        // value defined in a block that comes *later* in that order and still dominates it - a
        // loop's join block is made before its body - and this lowering maps a value when it
        // lowers the block that defines it. Preorder of the dominator tree is the order SSA
        // guarantees: every block after the ones that dominate it.
        for handle in dominator_order(func) {
            let block = &func.blocks[handle.0 as usize];
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

        // Same order as the block creation above: a use is lowered after the block that defines
        // it, which the parser's own order does not promise (a loop's join block is made first).
        for handle in dominator_order(func) {
            let block = &func.blocks[handle.0 as usize];
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

        // After `finalize` every instruction is in the function and every value has the virtual
        // register the back end gave it, which is the point of showing this at all.
        if let Some(explained) = &mut self.explained {
            explained.push(Explained {
                name: name.clone(),
                cir: func
                    .blocks
                    .iter()
                    .flat_map(|block| block.insts.iter())
                    .map(|inst| {
                        (
                            inst.span(),
                            crate::cir::dump::instruction(self.types, self.names, func, inst),
                        )
                    })
                    .collect(),
                clif: context.func.display().to_string(),
            });
        }

        let name = self
            .object
            .declare_function(&name, Linkage::Export, &context.func.signature)?;
        self.object
            .define_function(name, &mut context)
            .map_err(|error| LoweringError::bug(format!("cranelift: {error:?}")))?;
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
                .ok_or_else(|| LoweringError::bug(String::from("a value has no Cranelift value")))
        };
        let pointer = ir::types::I64;
        match inst {
            // The two operations that only a back end with a home area can lower (ADR-006).
            Inst::VaStart { span, .. } | Inst::VaArg { span, .. } => {
                return Err(LoweringError::not_lowered(
                    "`va_arg` in the Cranelift back end: its arguments are in registers it chose, and \
                     there is no home area to walk. Level 2 lowers it"
                        .into(),
                    *span,
                ))
            }
            // Nothing in Cranelift's instruction set is a compare-and-swap at a *chosen* encoding,
            // and atomics have to be exact: they are what two threads agree by.
            Inst::AtomicCas { span, .. } => {
                return Err(LoweringError::not_lowered(
                    "`_Atomic` in the Cranelift back end: the loop back ends need a `lock cmpxgh` \
                     this compiler writes itself, and it writes one at level 2"
                        .into(),
                    *span,
                ))
            }
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
            Inst::Slot {
                result,
                ty,
                alignas,
                ..
            } => {
                let size = self.types.size_of(*ty).ok_or_else(|| {
                    LoweringError::bug("a slot for an incomplete type".to_owned())
                })?;
                // The third field is the slot's alignment shift: `_Alignas(16)` on a `char` has to
                // be a sixteen-byte object, and C11 6.7.5p6 says it may raise, never lower, one.
                let align = self.types.align_of(*ty).unwrap_or(1).max(*alignas).max(1);
                let slot = builder.create_sized_stack_slot(ir::StackSlotData::new(
                    ir::StackSlotKind::ExplicitSlot,
                    size as u32,
                    align.trailing_zeros() as u8,
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
                        return Err(LoweringError::bug(format!(
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
                variadic,
                span,
            } => {
                let machine: Vec<ClValue> = args
                    .iter()
                    .map(|arg| value(values, *arg))
                    .collect::<Result<_, _>>()?;
                // A variadic call has no single signature: its arguments this time *are* its
                // signature, so one is built from them and the call is indirect - a `FuncRef`
                // carries the declaration's signature, and a second one for the same name is an
                // error in `cranelift-module` (ADR-006).
                if *variadic {
                    // Win64 duplicates each floating-point argument into the general register of its
                    // position, and says so in `AL`. Our own back end does both; this one writes the
                    // vector register alone, and a callee that reads the general ones - which is what
                    // every variadic callee does, `printf` included - answers zero. Refused rather
                    // than miscompiled: level 2 implements it (ADR-006, ADR-008).
                    if args
                        .iter()
                        .any(|arg| self.types.is_float(func.value(*arg).ty))
                    {
                        return Err(LoweringError::not_lowered(
                            "a floating-point argument to a variadic function: Win64 passes it in a \
                             vector register and in the general register of its position, and this \
                             lowerer writes only the vector one. Level 2 passes both"
                                .into(),
                            *span,
                        ));
                    }
                    let params: Vec<TypeId> = args.iter().map(|arg| func.value(*arg).ty).collect();
                    let result_ty = match result {
                        Some(result) => func.value(*result).ty,
                        None => self
                            .types
                            .fundamental("void")
                            .ok_or_else(|| LoweringError::bug(String::from("no void type")))?,
                    };
                    let signature = self.signature_of(&params, result_ty)?;
                    let signature = builder.import_signature(signature);
                    let address = match callees.get(callee).copied() {
                        Some(target) => builder.ins().func_addr(ir::types::I64, target),
                        None => value(values, *callee)?,
                    };
                    let call = builder.ins().call_indirect(signature, address, &machine);
                    if let Some(result) = result {
                        let produced = builder.inst_results(call)[0];
                        values.insert(*result, produced);
                    }
                    return Ok(());
                }
                // A direct call: the callee's address was minted by a `GlobalAddress` in this
                // body, so the function itself is known. Anything else is a call through a
                // pointer, and its signature is the function type the pointer points at - the
                // same `signature_of` the direct path uses, ABI rules and all.
                let call = match callees.get(callee).copied() {
                    Some(target) => builder.ins().call(target, &machine),
                    None => {
                        let address = value(values, *callee)?;
                        let signature = self.signature_of_pointer(func.value(*callee).ty)?;
                        let signature = builder.import_signature(signature);
                        builder.ins().call_indirect(signature, address, &machine)
                    }
                };
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
                        let id = match self.data.get(name.as_str()).copied() {
                            Some(id) => id,
                            // A name this unit only *declares* - `extern int table[];` here and the
                            // object in another file. Our own back end names it in a relocation; this
                            // one needs a data declaration, and an imported one is exactly that: no
                            // bytes of its own, and the linker supplies the address. Without this the
                            // reference was reported as a compiler bug, which it was not.
                            None => {
                                let id = self
                                    .object
                                    .declare_data(name.as_str(), Linkage::Import, true, false)?;
                                self.data.insert(name.clone(), id);
                                id
                            }
                        };
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
                .ok_or_else(|| LoweringError::bug(String::from("a value has no Cranelift value")))
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
        object(&parsed.module, preprocessed.names(), false, 0)
            .expect("lowering works")
            .bytes
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
