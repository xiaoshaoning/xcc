//! `cir` written down.
//!
//! A textual dump from the first commit, for two reasons ADR-002 gives: a harness can compare
//! two back ends' text, and a person can read what the front end produced without a debugger.
//! The format is stable enough to be pinned by tests, and it is the only thing here that needs
//! the name table.

use std::fmt::Write;

use crate::intern::Interner;
use crate::types::TypeId;

use super::{Block, Func, Inst, Module, Term, Value};

/// Write a whole module: its globals, then its functions.
pub fn dump(module: &Module, names: &Interner) -> String {
    let mut out = String::new();
    for global in &module.globals {
        let ty = module.types.describe_with(global.ty, names);
        let state = if global.defined { "" } else { " extern" };
        let bytes = global.init.as_ref().map_or(0, Vec::len);
        let _ = writeln!(out, "global {}: {ty}{state} ({bytes} bytes)", global.name);
    }
    for func in &module.funcs {
        dump_func(&mut out, &module.types, names, func);
    }
    out
}

/// Write one function.
pub fn dump_func(out: &mut String, types: &crate::types::Types, names: &Interner, func: &Func) {
    let params: Vec<String> = func
        .params()
        .iter()
        .map(|param| {
            format!(
                "v{}: {}",
                param.0,
                type_text(types, names, value_ty(func, *param))
            )
        })
        .collect();
    let result = type_text(types, names, func.result);
    let _ = writeln!(
        out,
        "func {}({}) -> {result}",
        names.resolve(func.name),
        params.join(", ")
    );
    for (index, block) in func.blocks.iter().enumerate() {
        let handle = Block(index as u32);
        let entry = if handle == func.entry { " (entry)" } else { "" };
        let params: Vec<String> = block
            .params
            .iter()
            .map(|param| {
                format!(
                    "v{}: {}",
                    param.0,
                    type_text(types, names, value_ty(func, *param))
                )
            })
            .collect();
        if params.is_empty() {
            let _ = writeln!(out, "block{index}{entry}:");
        } else {
            let _ = writeln!(out, "block{index}{entry}({}):", params.join(", "));
        }
        for inst in &block.insts {
            let _ = writeln!(out, "  {}", instruction(types, names, func, inst));
        }
        if let Some(term) = &block.term {
            let _ = writeln!(out, "  {}", terminator(names, func, term));
        }
    }
}

fn value_ty(func: &Func, value: Value) -> TypeId {
    func.value(value).ty
}

fn type_text(types: &crate::types::Types, names: &Interner, ty: TypeId) -> String {
    types.describe_with(ty, names)
}

fn instruction(types: &crate::types::Types, names: &Interner, func: &Func, inst: &Inst) -> String {
    let v = |value: Value| format!("v{}", value.0);
    let ty = |ty: TypeId| type_text(types, names, ty);
    match inst {
        Inst::Const {
            result,
            ty: of,
            bits,
            ..
        } => {
            format!("{} = const {} {bits}", v(*result), ty(*of))
        }
        Inst::Slot { result, ty: of, .. } => {
            format!("{} = slot {}", v(*result), ty(*of))
        }
        Inst::Load {
            result,
            ty: of,
            address,
            ..
        } => format!("{} = load {}, {}", v(*result), ty(*of), v(*address)),
        Inst::Store { address, value, .. } => format!("store {}, {}", v(*value), v(*address)),
        Inst::Binary {
            result,
            op,
            ty: of,
            left,
            right,
            ..
        } => format!(
            "{} = {} {} {}, {}",
            v(*result),
            op.as_str(),
            ty(*of),
            v(*left),
            v(*right)
        ),
        Inst::Compare {
            result,
            op,
            ty: of,
            left,
            right,
            ..
        } => format!(
            "{} = cmp {} {} {}, {}",
            v(*result),
            op.as_str(),
            ty(*of),
            v(*left),
            v(*right)
        ),
        Inst::Unary {
            result,
            op,
            ty: of,
            value,
            ..
        } => format!("{} = {} {} {}", v(*result), op.as_str(), ty(*of), v(*value)),
        Inst::Convert {
            result,
            op,
            from,
            to,
            value,
            ..
        } => format!(
            "{} = {} {} to {} {}",
            v(*result),
            op.as_str(),
            ty(*from),
            ty(*to),
            v(*value)
        ),
        Inst::Call {
            result,
            callee,
            args,
            ..
        } => {
            let args: Vec<String> = args.iter().map(|arg| v(*arg)).collect();
            match result {
                Some(result) => format!(
                    "{} = call {} {}({})",
                    v(*result),
                    ty(value_ty(func, *result)),
                    v(*callee),
                    args.join(", ")
                ),
                None => format!("call void {}({})", v(*callee), args.join(", ")),
            }
        }
        Inst::GlobalAddress { result, name, .. } => {
            format!("{} = addr {}", v(*result), names.resolve(*name))
        }
    }
}

fn terminator(names: &Interner, func: &Func, term: &Term) -> String {
    let v = |value: Value| format!("v{}", value.0);
    let args = |args: &[Value]| -> String {
        if args.is_empty() {
            String::new()
        } else {
            let spelled: Vec<String> = args.iter().map(|arg| v(*arg)).collect();
            format!("({})", spelled.join(", "))
        }
    };
    match term {
        Term::Return { value, .. } => match value {
            Some(value) => format!("return {}", v(*value)),
            None => "return".to_owned(),
        },
        Term::Jump {
            target, args: to, ..
        } => {
            format!("jump block{}{}", target.0, args(to))
        }
        Term::Branch {
            condition,
            then,
            otherwise,
            ..
        } => format!(
            "br {}, block{}{}, block{}{}",
            v(*condition),
            then.0 .0,
            args(&then.1),
            otherwise.0 .0,
            args(&otherwise.1)
        ),
        Term::Unreachable { .. } => {
            let _ = names;
            let _ = func;
            "unreachable".to_owned()
        }
    }
}
