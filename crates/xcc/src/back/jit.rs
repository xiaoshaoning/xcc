//! The JIT: compile C from a string and call it from Rust.
//!
//! This is the product ADR-001 promised - a compiler a host can put in a library, not only a
//! program that writes files. It is `back`'s second consumer, so the lowering is the same code
//! the object writer uses: one translation from `cir`, two destinations.
//!
//! No state is shared between two `Jit`s and none is global: a `Jit` owns its module, so a host
//! may keep one, make a hundred, or hand them to a hundred threads (CONSTRAINTS 2).

use cranelift_codegen::settings::{self, Configurable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{default_libcall_names, FuncId};

use crate::cir::Module as CirModule;
use crate::options::Options;
use crate::pp::preprocess;
use crate::source::SourceFile;

/// A compiled unit that runs.
pub struct Jit {
    module: JITModule,
    /// The name each function was declared by, so a host can ask for it later.
    names: Vec<(String, FuncId)>,
}

impl std::fmt::Debug for Jit {
    /// Cranelift's module has no `Debug` of its own; what a reader wants is which functions are
    /// in it.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Jit")
            .field(
                "functions",
                &self.names.iter().map(|(name, _)| name).collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl Jit {
    /// A JIT that can call the host functions in `symbols`, by name.
    ///
    /// Symbols are given at construction because Cranelift's builder takes them there and
    /// nowhere else; a `Jit` that wants more makes a new one.
    pub fn new(symbols: &[(&str, *const u8)]) -> Result<Self, String> {
        let mut flags = settings::builder();
        flags
            .set("opt_level", "none")
            .map_err(|error| error.to_string())?;
        let flags = settings::Flags::new(flags);
        let isa = cranelift_native::builder()
            .map_err(|error| error.to_string())?
            .finish(flags)
            .map_err(|error| error.to_string())?;
        let mut builder = JITBuilder::with_isa(isa, default_libcall_names());
        for (name, address) in symbols {
            builder.symbol(*name, *address);
        }
        Ok(Self {
            module: JITModule::new(builder),
            names: Vec::new(),
        })
    }

    /// Compile `source` into this JIT. The functions it defines can be asked for afterwards.
    pub fn add_source(&mut self, source: &str) -> Result<(), String> {
        let file = SourceFile::new("<jit>", source);
        let options = Options::default();
        let preprocessed = preprocess(&file, &options, &());
        if preprocessed.has_errors() {
            return Err(describe(&preprocessed.diagnostics));
        }
        let parsed = crate::parse::parse(&preprocessed, &options.budgets);
        let errors: Vec<&crate::diag::Diagnostic> = parsed
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.is_error())
            .collect();
        if let Some(first) = errors.first() {
            return Err(first.message.clone());
        }
        let mut module: CirModule = parsed.module;
        let names = preprocessed.names().clone();
        let functions =
            crate::back::lower_into(&mut self.module, &module, &names).map_err(|error| error.0)?;
        self.names.extend(functions);
        self.module
            .finalize_definitions()
            .map_err(|error| format!("finalizing: {error}"))?;
        Ok(())
    }

    /// The address of a function compiled here.
    ///
    /// The caller is responsible for calling it through the right signature: C has no types at
    /// run time, and this is the boundary where that stops being checkable.
    pub fn function(&self, name: &str) -> Option<*const u8> {
        let (_, id) = self.names.iter().find(|(known, _)| known == name)?;
        Some(self.module.get_finalized_function(*id))
    }
}

fn describe(diagnostics: &[crate::diag::Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|diagnostic| diagnostic.message.clone())
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
// Calling compiled code is unsafe by nature; that is the point of this module.
#[allow(unsafe_code)]
mod tests {
    use super::*;

    /// The acceptance test M4 names: compile a function from a string, call it from Rust.
    #[test]
    fn a_function_compiled_from_a_string_runs() {
        let mut jit = Jit::new(&[]).expect("a JIT");
        jit.add_source("int add(int a, int b) { return a + b; }")
            .expect("it compiles");
        let address = jit.function("add").expect("the function is there");
        let add: extern "C" fn(i32, i32) -> i32 = unsafe { std::mem::transmute(address) };
        assert_eq!(add(2, 3), 5);
        assert_eq!(add(-7, 7), 0);
    }

    /// Two JITs do not see each other's code, which is what "no global state" buys.
    #[test]
    fn two_jits_are_independent() {
        let mut first = Jit::new(&[]).expect("a JIT");
        first
            .add_source("int add(int a, int b) { return a + b; }")
            .expect("it compiles");
        let mut second = Jit::new(&[]).expect("a JIT");
        second
            .add_source("int add(int a, int b) { return a - b; }")
            .expect("it compiles");
        let call = |jit: &Jit| -> i32 {
            let address = jit.function("add").expect("the function is there");
            let add: extern "C" fn(i32, i32) -> i32 = unsafe { std::mem::transmute(address) };
            add(9, 4)
        };
        assert_eq!(call(&first), 13);
        assert_eq!(call(&second), 5);
    }

    /// Ten JITs on eight threads: the property is that there is nothing shared to race on.
    #[test]
    fn many_jits_on_many_threads() {
        let threads: Vec<_> = (0..8)
            .map(|worker| {
                std::thread::spawn(move || {
                    for round in 0..2 {
                        let mut jit = Jit::new(&[]).expect("a JIT");
                        jit.add_source("int add(int a, int b) { return a + b; }")
                            .expect("it compiles");
                        let address = jit.function("add").expect("the function is there");
                        let add: extern "C" fn(i32, i32) -> i32 =
                            unsafe { std::mem::transmute(address) };
                        assert_eq!(add(worker, round), worker + round);
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().expect("no panic");
        }
    }

    #[test]
    fn a_source_with_a_mistake_says_so_instead_of_running() {
        let mut jit = Jit::new(&[]).expect("a JIT");
        let error = jit
            .add_source("int add(int a) { return a + `; }")
            .expect_err("this does not compile");
        assert!(!error.is_empty(), "the error says something");
    }
}
