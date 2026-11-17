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
    /// A JIT with no host symbols: the common case.
    pub fn new() -> Result<Self, String> {
        // No symbols, so no contract to keep: this is the safe way in.
        #[allow(unsafe_code)]
        unsafe {
            Self::with_symbols(&[])
        }
    }

    /// A JIT that can call the host functions in `symbols`, by name.
    ///
    /// # Safety
    ///
    /// Every address must stay valid and keep the signature C would give it for as long as this
    /// `Jit` lives: compiled code calls through them, and C has nothing to check with. Symbols
    /// are taken here because Cranelift's builder takes them at construction and nowhere else.
    #[allow(unsafe_code)]
    pub unsafe fn with_symbols(symbols: &[(&str, *const u8)]) -> Result<Self, String> {
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
        // Both stages report the same way, and every error is reported: a host fixing one
        // mistake at a time is a host that recompiles once per mistake.
        let mut errors: Vec<String> = Vec::new();
        collect_errors(&preprocessed.diagnostics, &mut errors);
        let parsed = crate::parse::parse(&preprocessed, &options.budgets);
        collect_errors(&parsed.diagnostics, &mut errors);
        if !errors.is_empty() {
            return Err(errors.join("; "));
        }
        let module: CirModule = parsed.module;
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

/// Every error in `diagnostics`, in order, as one line each.
fn collect_errors(diagnostics: &[crate::diag::Diagnostic], into: &mut Vec<String>) {
    for diagnostic in diagnostics {
        if diagnostic.is_error() {
            into.push(diagnostic.message.clone());
        }
    }
}

#[cfg(test)]
// Calling compiled code is unsafe by nature; that is the point of this module.
#[allow(unsafe_code)]
mod tests {
    use super::*;

    /// The acceptance test M4 names: compile a function from a string, call it from Rust.
    #[test]
    fn a_function_compiled_from_a_string_runs() {
        let mut jit = Jit::new().expect("a JIT");
        jit.add_source("int add(int a, int b) { return a + b; }")
            .expect("it compiles");
        let address = jit.function("add").expect("the function is there");
        let add: extern "C" fn(i32, i32) -> i32 = unsafe { std::mem::transmute(address) };
        assert_eq!(add(2, 3), 5);
        assert_eq!(add(-7, 7), 0);
    }

    /// C99 6.9p5 from the other side: a tentative definition and a real one are *one* object, in
    /// either order, and the bytes are the ones the real definition gave it.
    #[test]
    fn a_tentative_definition_and_a_real_one_are_one_object() {
        let mut jit = Jit::new().expect("a JIT");
        jit.add_source(
            "int first;
             int first = 5;
             int second = 7;
             int second;
             int read_first(void) { return first; }
             int read_second(void) { return second; }",
        )
        .expect("it compiles");
        let call = |name: &str| -> i32 {
            let address = jit.function(name).expect("the function is there");
            let function: extern "C" fn() -> i32 = unsafe { std::mem::transmute(address) };
            function()
        };
        assert_eq!(call("read_first"), 5);
        assert_eq!(call("read_second"), 7);
    }

    /// A function calling another one in the same unit, through the address the emitter mints.
    #[test]
    fn one_function_calls_another() {
        let mut jit = Jit::new().expect("a JIT");
        jit.add_source(
            "int add(int a, int b) { return a + b; }
int twice(int a) { return add(a, a); }",
        )
        .expect("it compiles");
        let address = jit.function("twice").expect("the function is there");
        let twice: extern "C" fn(i32) -> i32 = unsafe { std::mem::transmute(address) };
        assert_eq!(twice(21), 42);
    }

    /// C code calling back into Rust: the point of `add_symbol`.
    #[test]
    fn compiled_code_calls_a_host_function() {
        extern "C" fn host_double(value: i32) -> i32 {
            value * 2
        }
        let mut jit = unsafe { Jit::with_symbols(&[("host_double", host_double as *const u8)]) }
            .expect("a JIT");
        jit.add_source(
            "int host_double(int x);
int twice(int x) { return host_double(x); }",
        )
        .expect("it compiles");
        let address = jit.function("twice").expect("the function is there");
        let twice: extern "C" fn(i32) -> i32 = unsafe { std::mem::transmute(address) };
        assert_eq!(twice(21), 42);
    }

    /// `switch`, as the values it must produce: the arms, `break`, `default`, and C's
    /// fall-through. The case that made it hard is `case 1: r = 5;` - a *store* inside a case
    /// body, which reported "this kind of assignment is not lowered yet" while the block the arm
    /// opened was already terminated.
    #[test]
    fn switch_arms_break_default_and_fall_through() {
        let mut jit = Jit::new().expect("a JIT");
        jit.add_source(
            "int pick(int x) {
             switch (x) { case 1: return 10; case 2: break; default: return 30; }
             return 0;
             }
             int fall(int x) {
             int r;
             r = 0;
             switch (x) { case 1: r = r + 1; case 2: r = r + 2; break; default: r = 9; }
             return r;
             }",
        )
        .expect("it compiles");
        let call = |name: &str, argument: i32| -> i32 {
            let address = jit.function(name).expect("the function is there");
            let function: extern "C" fn(i32) -> i32 = unsafe { std::mem::transmute(address) };
            function(argument)
        };
        // An arm, `break` out to the code after the switch, and the default.
        assert_eq!(call("pick", 1), 10);
        assert_eq!(call("pick", 2), 0);
        assert_eq!(call("pick", 3), 30);
        // Fall-through: case 1 runs its own body and case 2's, so r is 3; case 2 alone is 2.
        assert_eq!(call("fall", 1), 3);
        assert_eq!(call("fall", 2), 2);
        assert_eq!(call("fall", 9), 9);
    }

    /// `continue` inside a `switch` inside a loop goes to the loop, not out of the switch - and
    /// the code after the switch still returns to the loop, even though the switch left the
    /// cursor in its own exit block.
    #[test]
    fn continue_inside_a_switch_continues_the_loop() {
        let mut jit = Jit::new().expect("a JIT");
        jit.add_source(
            "int count(int n) {
             int r; r = 0;
             while (n > 0) {
             switch (n) { case 1: n = n - 1; continue; default: r = r + 1; }
             n = n - 1;
             }
             return r;
             }",
        )
        .expect("it compiles");
        let address = jit.function("count").expect("the function is there");
        let count: extern "C" fn(i32) -> i32 = unsafe { std::mem::transmute(address) };
        // 3 and 2 reach the default arm; 1 continues out of the switch but not out of the loop.
        assert_eq!(count(3), 2);
        assert_eq!(count(1), 0);
        assert_eq!(count(0), 0);
    }

    /// `++`/`--`: the prefix form yields the new value, the postfix form the old one, and both
    /// write back.
    #[test]
    fn increment_and_decrement_write_back_and_differ_by_which_value_they_yield() {
        let mut jit = Jit::new().expect("a JIT");
        jit.add_source(
            "int post(int n) { int i; i = n; return i++; }
             int pre(int n) { int i; i = n; return ++i; }
             int down(int n) { int i; i = n; return i--; }",
        )
        .expect("it compiles");
        let call = |name: &str, argument: i32| -> i32 {
            let address = jit.function(name).expect("the function is there");
            let function: extern "C" fn(i32) -> i32 = unsafe { std::mem::transmute(address) };
            function(argument)
        };
        assert_eq!(call("post", 7), 7, "postfix yields the old value");
        assert_eq!(call("pre", 7), 8, "prefix yields the new one");
        assert_eq!(call("down", 7), 7);
    }

    /// `&&` and `||` short-circuit, which is observable: the arm that is not taken must not
    /// divide by zero. A plain `and`/`or` of two eager operands traps here.
    ///
    /// Written before the emitter can lower them (M6's first item) and ignored until it can.
    #[test]
    fn logical_operators_do_not_evaluate_what_they_do_not_need() {
        let mut jit = Jit::new().expect("a JIT");
        jit.add_source(
            "int and_guard(int n) { return n && (10 / n); }
             int or_guard(int n) { return n || (10 / n); }",
        )
        .expect("it compiles");
        let call = |name: &str, argument: i32| -> i32 {
            let address = jit.function(name).expect("the function is there");
            let function: extern "C" fn(i32) -> i32 = unsafe { std::mem::transmute(address) };
            function(argument)
        };
        // `0 && (10 / 0)` is 0 without dividing: an eager `and` traps here.
        assert_eq!(call("and_guard", 0), 0);
        assert_eq!(call("and_guard", 5), 1);
        // `5 || (10 / 5)` short-circuits to 1. The other side of `||` is *not* asserted: seeing
        // `0 || (10 / 0)` return 1 means the division ran, and C calls that undefined.
        assert_eq!(call("or_guard", 5), 1);
    }

    /// An array local lives in the frame, and its value is its address: no load, and `a[i]`
    /// addresses the slot. `&a + 1` steps a whole array only if the address is typed
    /// `pointer to array[N]`, which is the question ADR-004 answers.
    #[test]
    fn array_locals_are_their_own_address_and_decay_by_bitcast() {
        let mut jit = Jit::new().expect("a JIT");
        jit.add_source(
            "int sum3(int n) { int a[3]; a[0] = n; a[1] = 2; a[2] = 3; return a[0] + a[1] + a[2]; }
             int walk(void) { int a[3]; int *p; p = a; p[1] = 7; return *(p + 1) + a[1]; }
             int whole(void) { int a[3]; return sizeof *(&a + 1) + sizeof a; }
             int nested(void) { int a[2][3]; a[1][2] = 5; return a[1][2]; }
             int stepped(void) { int a[3]; int *p; p = a; p[1] = 7; p++; return p[0]; }",
        )
        .expect("it compiles");
        let call = |name: &str| -> i32 {
            let address = jit.function(name).expect("the function is there");
            let function: extern "C" fn() -> i32 = unsafe { std::mem::transmute(address) };
            function()
        };
        let address = jit.function("sum3").expect("the function is there");
        let sum3: extern "C" fn(i32) -> i32 = unsafe { std::mem::transmute(address) };
        assert_eq!(sum3(4), 9);
        assert_eq!(call("walk"), 14);
        // `*(&a + 1)` is the array after `a`, so 12 bytes; a flat `int *` would make it 4.
        assert_eq!(call("whole"), 24);
        assert_eq!(call("nested"), 5);
        // `p++` steps one int, not one byte: a byte step would not land on the 7.
        assert_eq!(call("stepped"), 7);
    }

    /// A local's initialiser becomes stores into its slot: a scalar, the null pointer, an
    /// address, and the brace lists and string literals that fill an array - with C's zeroes in
    /// what the list does not name (C99 6.7.8p21).
    #[test]
    fn local_initialisers_store_into_the_frame() {
        let mut jit = Jit::new().expect("a JIT");
        jit.add_source(
            "int scalar(void) { int x = 3; return x; }
             int null_pointer(void) { int *p = 0; return p == 0; }
             int address_of(void) { int y; int *p = &y; *p = 7; return y; }
             int braces(void) { int a[3] = {1, 2, 3}; return a[0] * 100 + a[1] * 10 + a[2]; }
             int partial(void) { int a[3] = {5}; return a[0] * 100 + a[1] * 10 + a[2]; }
             int nested(void) { int m[2][2] = {{1, 2}, {3, 4}}; return m[0][1] * 10 + m[1][0]; }
             int flat(void) { int m[2][2] = {1, 2, 3, 4}; return m[1][1]; }
             int designator(void) { int a[5] = {[2] = 7}; return a[2] * 10 + a[0]; }
             int sized_by_text(void) { char s[] = \"abc\"; return s[0] + s[3]; }
             int text_tail(void) { char s[8] = \"hi\"; return s[1] * 10 + s[2]; }
             int pointer_to_text(void) { char *p = \"abc\"; return p[1]; }
             int joined_text(void) { char s[] = \"a\" \"b\"; return s[0] + s[1]; }
             int joined_length(void) { char s[] = \"ab\" \"cd\"; return sizeof s; }",
        )
        .expect("it compiles");
        for (name, wanted) in [
            ("scalar", 3),
            ("null_pointer", 1),
            ("address_of", 7),
            ("braces", 123),
            // The elements the list does not name are zero, not whatever the frame held.
            ("partial", 500),
            ("nested", 23),
            ("flat", 4),
            ("designator", 70),
            // 97 + the NUL the literal's size includes.
            ("sized_by_text", 97),
            // 'i' * 10 + the NUL of the sized array's tail.
            ("text_tail", 1050),
            ("pointer_to_text", 98),
            // Adjacent literals are one run (C99 6.4.5p5), before and after the NUL.
            ("joined_text", 195),
            ("joined_length", 5),
        ] {
            let address = jit.function(name).expect("the function is there");
            let function: extern "C" fn() -> i32 = unsafe { std::mem::transmute(address) };
            assert_eq!(function(), wanted, "{name}");
        }
    }

    /// `condition ? a : b` is two blocks and a join (C99 6.5.15). The untaken arm must not be
    /// evaluated - `n ? 10 / n : -1` with zero is not a division - so it cannot be one
    /// instruction with a `select`. The arms' common type is the join's parameter type, and an
    /// arm that is itself `&&` or another `?:` leaves the cursor in its own join, which is where
    /// that arm's jump has to leave from.
    #[test]
    fn a_conditional_expression_evaluates_only_the_arm_it_picks() {
        let mut jit = Jit::new().expect("a JIT");
        jit.add_source(
            "int pick(int c) { return c ? 10 : 20; }
             int guarded(int n) { return n ? 10 / n : -1; }
             int chain(int a) { return a == 1 ? 10 : a == 2 ? 20 : 30; }
             int logical(int a, int b) { return (a && b) ? 1 : 2; }
             int pointer(int c) { int a[2]; int *p = c ? a : 0; return p == 0; }
             int merged(void) { return 1 ? 5 : 6; }
             int middle(int a) { return a ? (a == 1 ? 10 : 20) : 30; }
             int null_pointer(void) { int *p = 1 ? 0 : 0; return p == 0; }
             int side(int x) { int r = 0; x ? (r = 1) : (r = 2); return r; }
             void nothing(void) { }
             int no_value(int c) { c ? nothing() : nothing(); return c; }
             int wider(int c) { long l = c ? 1 : 2L; return (int)l; }
             int sized(void) { int a[1 ? 2 : 3]; return sizeof a; }
             int unevaluated(void) { int x = 0; int s = sizeof(x = 3); return x * 10 + s; }
             int texts(int c) { char *p = c ? \"a\" : \"b\"; return p[0]; }
             int addresses(int c) { int a; int b; int *p = c ? &a : &b; return p == &a; }
             int helper(int v) { return v * 10; }
             int called(int c) { return c ? helper(1) : helper(2); }",
        )
        .expect("it compiles");
        // Two closures: calling a `void` function through `fn(i32)` would pass a register its
        // signature does not have, and a signature mistake is exactly what a test must not hide.
        let call = |name: &str, argument: i32| -> i32 {
            let address = jit.function(name).expect("the function is there");
            let function: extern "C" fn(i32) -> i32 = unsafe { std::mem::transmute(address) };
            function(argument)
        };
        let call_void = |name: &str| -> i32 {
            let address = jit.function(name).expect("the function is there");
            let function: extern "C" fn() -> i32 = unsafe { std::mem::transmute(address) };
            function()
        };
        assert_eq!(call("pick", 1), 10);
        assert_eq!(call("pick", 0), 20);
        // `10 / 0` in the arm that is not taken would trap.
        assert_eq!(call("guarded", 0), -1);
        assert_eq!(call("guarded", 5), 2);
        assert_eq!(call("chain", 1), 10);
        assert_eq!(call("chain", 2), 20);
        assert_eq!(call("chain", 9), 30);
        // The nested `?:` is the middle arm: its join is where the middle arm's jump leaves.
        assert_eq!(call("middle", 1), 10);
        assert_eq!(call("middle", 0), 30);
        assert_eq!(call("logical", 1), 1);
        assert_eq!(call("logical", 0), 2);
        assert_eq!(call("pointer", 1), 0);
        assert_eq!(call("pointer", 0), 1);
        // The folded value survives the lowering, so a constant `?:` is still a null pointer
        // constant in a body, as it is at file scope.
        assert_eq!(call_void("null_pointer"), 1);
        assert_eq!(call("side", 1), 1);
        assert_eq!(call("side", 0), 2);
        assert_eq!(call("no_value", 7), 7);
        assert_eq!(call("wider", 0), 2);
        // A conditional expression in a declarator's size is read for its value only: no
        // branch and no blocks in the middle of the declaration, and two ints of storage.
        assert_eq!(call_void("sized"), 8);
        // `sizeof`'s operand is not evaluated (C99 6.5.3.4p2): `x` is still 0, and `sizeof`
        // is 4 whether or not the assignment in it looks like it ran.
        assert_eq!(call_void("unevaluated"), 4);
        // Arms of pointer type: a literal's address, and the address of a local.
        assert_eq!(call("texts", 1), i32::from(b'a'));
        assert_eq!(call("texts", 0), i32::from(b'b'));
        assert_eq!(call("addresses", 1), 1);
        assert_eq!(call("addresses", 0), 0);
        assert_eq!(call("called", 1), 10);
        assert_eq!(call("called", 0), 20);
        assert_eq!(call_void("merged"), 5);
    }

    /// Floats and doubles (M6): literals, arithmetic, comparisons, conversions both ways, a
    /// float condition, and `sizeof`. A float constant is a bit pattern in `Value::constant`,
    /// which is why `null_pointer_constant` asks about the type and not just the value.
    #[test]
    fn floats_have_values_and_arithmetic() {
        let mut jit = Jit::new().expect("a JIT");
        jit.add_source(
            "double add(double a, double b) { return a + b; }
             double scaled(int n) { return n * 1.5; }
             int truncate(double d) { return (int)d; }
             int narrow_to_short(double d) { return (short)d; }
             double from_char(void) { char c = 7; return c; }
             int less(double a, double b) { return a < b; }
             int equal(double a, double b) { return a == b; }
             int not_equal(double a, double b) { return a != b; }
             double widened(float f) { return f * 2.0; }
             float narrowed(double d) { return d; }
             int condition(double d) { if (d) { return 1; } return 0; }
             int logical(double d) { return d && 1; }
             int chosen(double d) { return d ? 10 : 20; }
             double negative(void) { return -1.5; }
             int sizes(void) { return sizeof(float) * 100 + sizeof(long double); }
             int from_double(double d) { int i = d; return i; }
             int unsigned_from(double d) { unsigned u = d; return (int)u; }",
        )
        .expect("it compiles");
        let two = |name: &str, a: f64, b: f64| -> f64 {
            let address = jit.function(name).expect("the function is there");
            let function: extern "C" fn(f64, f64) -> f64 = unsafe { std::mem::transmute(address) };
            function(a, b)
        };
        let compare = |name: &str, a: f64, b: f64| -> i32 {
            let address = jit.function(name).expect("the function is there");
            let function: extern "C" fn(f64, f64) -> i32 = unsafe { std::mem::transmute(address) };
            function(a, b)
        };
        let one = |name: &str, a: f64| -> i32 {
            let address = jit.function(name).expect("the function is there");
            let function: extern "C" fn(f64) -> i32 = unsafe { std::mem::transmute(address) };
            function(a)
        };
        let call_int = |name: &str, a: i32| -> f64 {
            let address = jit.function(name).expect("the function is there");
            let function: extern "C" fn(i32) -> f64 = unsafe { std::mem::transmute(address) };
            function(a)
        };
        let call_float = |name: &str, a: f32| -> f64 {
            let address = jit.function(name).expect("the function is there");
            let function: extern "C" fn(f32) -> f64 = unsafe { std::mem::transmute(address) };
            function(a)
        };
        let to_float = |name: &str, a: f64| -> f32 {
            let address = jit.function(name).expect("the function is there");
            let function: extern "C" fn(f64) -> f32 = unsafe { std::mem::transmute(address) };
            function(a)
        };
        let no_argument = |name: &str| -> f64 {
            let address = jit.function(name).expect("the function is there");
            let function: extern "C" fn() -> f64 = unsafe { std::mem::transmute(address) };
            function()
        };
        let sizes = jit.function("sizes").expect("the function is there");
        let sizes: extern "C" fn() -> i32 = unsafe { std::mem::transmute(sizes) };

        assert_eq!(two("add", 1.25, 2.5), 3.75);
        assert_eq!(call_int("scaled", 4), 6.0);
        assert_eq!(one("truncate", 3.9), 3);
        // A conversion to a `char` or a `short` is not an x64 instruction: it goes through 32 bits.
        assert_eq!(one("narrow_to_short", 3.9), 3);
        // ...and one from a narrow integer is widened first.
        assert_eq!(no_argument("from_char"), 7.0);
        assert_eq!(compare("less", 1.0, 2.0), 1);
        assert_eq!(compare("less", 2.0, 1.0), 0);
        assert_eq!(compare("equal", 2.5, 2.5), 1);
        assert_eq!(compare("not_equal", 2.5, 2.5), 0);
        assert_eq!(compare("not_equal", 2.5, 2.0), 1);
        assert_eq!(call_float("widened", 1.5), 3.0);
        assert_eq!(to_float("narrowed", 1.5), 1.5f32);
        // A float condition is a comparison, not a conversion: NaN is true, and 0.0 is false.
        assert_eq!(one("condition", 0.5), 1);
        assert_eq!(one("condition", 0.0), 0);
        assert_eq!(one("logical", 0.0), 0);
        assert_eq!(one("logical", 2.0), 1);
        assert_eq!(one("chosen", 0.0), 20);
        assert_eq!(one("chosen", -1.0), 10);
        assert_eq!(no_argument("negative"), -1.5);
        assert_eq!(sizes(), 408);
        assert_eq!(one("from_double", 42.9), 42);
        assert_eq!(
            one("unsigned_from", 4_000_000_000.0),
            4_000_000_000u32 as i32
        );
    }

    /// A record crosses a call on the Win64 ABI's terms: 1, 2, 4 and 8 bytes travel in an
    /// integer register, anything else by reference to a copy the *caller* owns, and a result
    /// too big for a register comes back through a buffer the caller passes first and gets in
    /// `rax` again. `#[repr(C)]` structs and `extern "C"` in Rust are the same ABI, so Rust is
    /// the oracle here, in both directions.
    #[test]
    fn aggregates_cross_a_call_on_the_abi() {
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct Tiny {
            a: i8,
            b: i8,
        }
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct Odd {
            a: i8,
            b: i8,
            c: i8,
        }
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct Small {
            x: i32,
        }
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct Pair {
            a: i32,
            b: i32,
        }
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct Wide {
            a: i32,
            b: i32,
            c: i32,
        }
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct Doubled {
            d: f64,
        }
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct Sixteen {
            d: f64,
            e: f64,
        }
        #[repr(C)]
        #[derive(Clone, Copy)]
        union Union {
            i: u32,
            c: [u8; 8],
        }

        extern "C" fn host_wide(w: Wide) -> i32 {
            w.a * 100 + w.b * 10 + w.c
        }
        extern "C" fn host_make_wide(v: i32) -> Wide {
            Wide {
                a: v,
                b: v + 1,
                c: v + 2,
            }
        }
        extern "C" fn host_pair_after(p: Pair, n: i32) -> i32 {
            p.a * 10 + p.b + n
        }

        let symbols: &[(&str, *const u8)] = &[
            ("host_wide", host_wide as *const u8),
            ("host_make_wide", host_make_wide as *const u8),
            ("host_pair_after", host_pair_after as *const u8),
        ];
        let mut jit = unsafe { Jit::with_symbols(symbols) }.expect("a JIT");
        jit.add_source(
            "struct Tiny { char a; char b; };
             struct Odd { char a; char b; char c; };
             struct Small { int x; };
             struct Pair { int a; int b; };
             struct Wide { int a; int b; int c; };
             struct Doubled { double d; };
             struct Sixteen { double d; double e; };
             union Union { int i; char c[8]; };
             int take_tiny(struct Tiny t) { return t.a * 10 + t.b; }
             int take_odd(struct Odd o) { return o.a * 100 + o.b * 10 + o.c; }
             int take_small(struct Small s) { return s.x; }
             int take_pair(struct Pair p) { return p.a * 10 + p.b; }
             int take_wide(struct Wide w) { return w.a * 100 + w.b * 10 + w.c; }
             double take_doubled(struct Doubled d) { return d.d; }
             struct Tiny give_tiny(int v) { struct Tiny t; t.a = v; t.b = v + 1; return t; }
             struct Odd give_odd(int v) { struct Odd o; o.a = v; o.b = v + 1; o.c = v + 2; return o; }
             struct Small give_small(int v) { struct Small s; s.x = v; return s; }
             struct Pair give_pair(int a, int b) { struct Pair p; p.a = a; p.b = b; return p; }
             struct Wide give_wide(int v) { struct Wide w; w.a = v; w.b = v + 1; w.c = v + 2; return w; }
             int round_trip(int v) { struct Wide w = give_wide(v); return take_wide(w); }
             int mixed(int a, struct Wide w, int b) { return a * 1000 + w.a * 100 + w.b * 10 + w.c + b; }
             int two_smalls(struct Pair x, struct Pair y) {
                 return x.a * 10 + x.b + y.a * 100 + y.b * 1000;
             }
             int host_wide(struct Wide w);
             struct Wide host_make_wide(int v);
             int host_pair_after(struct Pair p, int n);
             int call_host(void) { struct Wide w = {1, 2, 3}; return host_wide(w); }
             int call_host_result(void) { struct Wide w = host_make_wide(7); return w.a * 100 + w.b * 10 + w.c; }
             int call_host_after(void) { struct Pair p = {2, 3}; return host_pair_after(p, 10); }
             int take_union(union Union u) { return u.i; }
             union Union give_union(int v) { union Union u; u.i = v; return u; }
             double take_sixteen(struct Sixteen s) { return s.d + s.e; }
             struct Sixteen give_sixteen(double v) { struct Sixteen s; s.d = v; s.e = v * 2.0; return s; }
             int modifies(struct Wide w) { w.a = 99; return w.a; }
             int caller_keeps(void) { struct Wide w = {1, 2, 3}; int r = modifies(w); return w.a * 100 + r; }
             int crowded(struct Wide a, struct Wide b, struct Wide c, int n)
             { return a.a + b.b * 10 + c.c * 100 + n * 1000; }
             struct Wide crowded_result(struct Wide a, struct Wide b, int n)
             { struct Wide w; w.a = a.a + n; w.b = b.b + n; w.c = n; return w; }",
        )
        .expect("it compiles");
        let call = |name: &str| -> i32 {
            let address = jit.function(name).expect("the function is there");
            let function: extern "C" fn() -> i32 = unsafe { std::mem::transmute(address) };
            function()
        };

        // xcc as the callee: Rust builds the arguments the ABI's way.
        let take_tiny: extern "C" fn(Tiny) -> i32 =
            unsafe { std::mem::transmute(jit.function("take_tiny").unwrap()) };
        assert_eq!(take_tiny(Tiny { a: 1, b: 2 }), 12);
        let take_odd: extern "C" fn(Odd) -> i32 =
            unsafe { std::mem::transmute(jit.function("take_odd").unwrap()) };
        // Three bytes is not a register size: passed by reference.
        assert_eq!(take_odd(Odd { a: 1, b: 2, c: 3 }), 123);
        let take_small: extern "C" fn(Small) -> i32 =
            unsafe { std::mem::transmute(jit.function("take_small").unwrap()) };
        assert_eq!(take_small(Small { x: 42 }), 42);
        let take_pair: extern "C" fn(Pair) -> i32 =
            unsafe { std::mem::transmute(jit.function("take_pair").unwrap()) };
        assert_eq!(take_pair(Pair { a: 3, b: 4 }), 34);
        let take_wide: extern "C" fn(Wide) -> i32 =
            unsafe { std::mem::transmute(jit.function("take_wide").unwrap()) };
        assert_eq!(take_wide(Wide { a: 1, b: 2, c: 3 }), 123);
        // Eight bytes of a `double` still travels as an integer: the bits, not an XMM register.
        let take_doubled: extern "C" fn(Doubled) -> f64 =
            unsafe { std::mem::transmute(jit.function("take_doubled").unwrap()) };
        assert_eq!(take_doubled(Doubled { d: 2.5 }), 2.5);

        // ...and xcc as the caller of a Rust function and the callee of one at once.
        let mixed: extern "C" fn(i32, Wide, i32) -> i32 =
            unsafe { std::mem::transmute(jit.function("mixed").unwrap()) };
        assert_eq!(mixed(1, Wide { a: 2, b: 3, c: 4 }, 5), 1234 + 5);
        let two: extern "C" fn(Pair, Pair) -> i32 =
            unsafe { std::mem::transmute(jit.function("two_smalls").unwrap()) };
        assert_eq!(
            two(Pair { a: 1, b: 2 }, Pair { a: 3, b: 4 }),
            12 + 300 + 4000
        );
        assert_eq!(call("call_host"), 123);
        assert_eq!(call("call_host_result"), 789);
        assert_eq!(call("call_host_after"), 33);

        // Results: a register width comes back in `rax`, anything else through a buffer.
        let give_tiny: extern "C" fn(i32) -> Tiny =
            unsafe { std::mem::transmute(jit.function("give_tiny").unwrap()) };
        let tiny = give_tiny(4);
        assert_eq!((tiny.a, tiny.b), (4, 5));
        let give_odd: extern "C" fn(i32) -> Odd =
            unsafe { std::mem::transmute(jit.function("give_odd").unwrap()) };
        let odd = give_odd(4);
        assert_eq!((odd.a, odd.b, odd.c), (4, 5, 6));
        let give_small: extern "C" fn(i32) -> Small =
            unsafe { std::mem::transmute(jit.function("give_small").unwrap()) };
        assert_eq!(give_small(7).x, 7);
        let give_pair: extern "C" fn(i32, i32) -> Pair =
            unsafe { std::mem::transmute(jit.function("give_pair").unwrap()) };
        let pair = give_pair(5, 6);
        assert_eq!((pair.a, pair.b), (5, 6));
        let give_wide: extern "C" fn(i32) -> Wide =
            unsafe { std::mem::transmute(jit.function("give_wide").unwrap()) };
        let wide = give_wide(10);
        assert_eq!((wide.a, wide.b, wide.c), (10, 11, 12));
        assert_eq!(call("round_trip"), 123);
        // A union travels like the struct of its size it is. Only the member written is a
        // value: the other bytes of the eight are as indeterminate as C's padding, so the
        // reading is of `i` and not of the whole register.
        let take_union: extern "C" fn(Union) -> i32 =
            unsafe { std::mem::transmute(jit.function("take_union").unwrap()) };
        assert_eq!(
            take_union(Union {
                c: [0x42, 0x41, 0, 0, 0, 0, 0, 0]
            }),
            0x4142
        );
        let give_union: extern "C" fn(i32) -> Union =
            unsafe { std::mem::transmute(jit.function("give_union").unwrap()) };
        let given = give_union(0x5152);
        assert_eq!(unsafe { given.i }, 0x5152);
        // Sixteen bytes is two doubles: by reference in, through a buffer out.
        let take_sixteen: extern "C" fn(Sixteen) -> f64 =
            unsafe { std::mem::transmute(jit.function("take_sixteen").unwrap()) };
        assert_eq!(take_sixteen(Sixteen { d: 1.5, e: 2.5 }), 4.0);
        let give_sixteen: extern "C" fn(f64) -> Sixteen =
            unsafe { std::mem::transmute(jit.function("give_sixteen").unwrap()) };
        let sixteen = give_sixteen(1.5);
        assert_eq!((sixteen.d, sixteen.e), (1.5, 3.0));
        // The by-reference copy is real: what the callee writes is not the caller's object.
        assert_eq!(call("caller_keeps"), 100 + 99);
        // Three by-reference records and an integer fill the four registers exactly.
        let crowded: extern "C" fn(Wide, Wide, Wide, i32) -> i32 =
            unsafe { std::mem::transmute(jit.function("crowded").unwrap()) };
        assert_eq!(
            crowded(
                Wide { a: 1, b: 0, c: 0 },
                Wide { a: 0, b: 2, c: 0 },
                Wide { a: 0, b: 0, c: 3 },
                4
            ),
            1 + 20 + 300 + 4000
        );
        // ...and a hidden result pointer takes the first of them, shifting everything along.
        let crowded_result: extern "C" fn(Wide, Wide, i32) -> Wide =
            unsafe { std::mem::transmute(jit.function("crowded_result").unwrap()) };
        let shifted = crowded_result(Wide { a: 5, b: 0, c: 0 }, Wide { a: 0, b: 6, c: 0 }, 10);
        assert_eq!((shifted.a, shifted.b, shifted.c), (15, 16, 10));
    }

    /// A record lays out the way the ABI does, measured against clang: the struct sizes and
    /// offsets here are `clang -o t.exe t.c` on this machine (win64), which is the oracle the
    /// plan names. A bitfield's *unit* is spoken for once a bitfield is packed into it, so a
    /// member that is not a bitfield starts after that unit rather than inside it - the mistake
    /// that made `struct { int a; int :3; int b; }` eight bytes instead of twelve.
    #[test]
    fn a_record_lays_out_like_the_abi() {
        let mut jit = Jit::new().expect("a JIT");
        jit.add_source(
            "struct S1 { int a; int : 3; int b; };
             struct S2 { int a : 3; int b : 5; char c; };
             struct S3 { int a : 20; int b : 20; };
             struct S4 { char a; double d; };
             struct S5 { char a; int b : 3; char c; };
             struct S6 { int : 0; char c; };
             struct S7 { char a; short b; char c; };
             struct S8 { char a : 4; char b : 4; };
             struct S9 { int a : 3; int : 0; int b : 3; };
             union U1 { char a; double d; };
             int s1(void) { return sizeof(struct S1); }
             int s2(void) { int n = sizeof(struct S2); struct S2 v; return n * 100 + (int)((char *)&v.c - (char *)&v); }
             int s3(void) { return sizeof(struct S3); }
             int s4(void) { int n = sizeof(struct S4); struct S4 v; return n * 100 + (int)((char *)&v.d - (char *)&v); }
             int s5(void) { int n = sizeof(struct S5); struct S5 v; return n * 100 + (int)((char *)&v.c - (char *)&v); }
             int s6(void) { int n = sizeof(struct S6); struct S6 v; return n * 100 + (int)((char *)&v.c - (char *)&v); }
             int s7(void) { int n = sizeof(struct S7); struct S7 v; return n * 100 + (int)((char *)&v.b - (char *)&v) * 10 + (int)((char *)&v.c - (char *)&v); }
             int s8(void) { return sizeof(struct S8); }
             int s9(void) { return sizeof(struct S9); }
             int u1(void) { return sizeof(union U1); }",
        )
        .expect("it compiles");
        let call = |name: &str| -> i32 {
            let address = jit.function(name).expect("the function is there");
            let function: extern "C" fn() -> i32 = unsafe { std::mem::transmute(address) };
            function()
        };
        // clang: S1 12, S2 8 (c at 4), S3 8, S4 16 (d at 8), S5 12 (c at 8), S6 1 (c at 0),
        // S7 6 (b at 2, c at 4), S8 1, S9 8, U1 8.
        assert_eq!(call("s1"), 12);
        assert_eq!(call("s2"), 8 * 100 + 4);
        assert_eq!(call("s3"), 8);
        assert_eq!(call("s4"), 16 * 100 + 8);
        assert_eq!(call("s5"), 12 * 100 + 8);
        // The zero-width bitfield raises nothing: `c` is at zero and the struct is one byte.
        assert_eq!(call("s6"), 100);
        assert_eq!(call("s7"), 6 * 100 + 2 * 10 + 4);
        assert_eq!(call("s8"), 1);
        assert_eq!(call("s9"), 8);
        assert_eq!(call("u1"), 8);
    }

    /// Aggregates in the frame (M6): a struct local is a slot, a member is its address plus an
    /// offset, and a struct's *value* is the address of its bytes - so assignment copies, and a
    /// `cir` register never has to hold one. Unions come along by the same layout, and a bitfield
    /// has no address, so it is refused in code.
    #[test]
    fn aggregates_live_in_the_frame() {
        let mut jit = Jit::new().expect("a JIT");
        jit.add_source(
            "struct P { int x; int y; };
             struct Q { struct P inner; int z; };
             struct A { int v[3]; };
             struct N { struct N *next; int v; };
             union U { int i; char c; };
             struct D { double d; int i; };
             int members(void) { struct P p; p.x = 3; p.y = 4; return p.x * 10 + p.y; }
             int braces(void) { struct P p = {1, 2}; return p.x * 10 + p.y; }
             int designator(void) { struct P p = {.y = 7}; return p.x * 10 + p.y; }
             int copy(void) { struct P p = {1, 2}; struct P q = p; return q.x * 10 + q.y; }
             int assigned(void) { struct P a = {1, 2}; struct P b = {3, 4}; a = b; return a.x * 10 + a.y; }
             int through_pointer(void) { struct P a; struct P *p = &a; p->x = 5; p->y = 6; return p->x * 10 + p->y; }
             int address_of_member(void) { struct P a = {1, 0}; int *q = &a.x; *q = 9; return a.x; }
             int stepped(void) { struct P a = {1, 0}; a.x += 4; a.x++; return a.x; }
             int by_size(void) { struct P p; return sizeof(p.y); }
             int nested(void) { struct P p = {7, 8}; struct Q q; q.inner = p; q.z = 9; return q.inner.x * 10 + q.inner.y; }
             int array_member(void) { struct A a; a.v[1] = 5; return a.v[1]; }
             int array_of_structs(void) { struct P a[2] = {{1, 2}, {3, 4}}; return a[0].x * 10 + a[1].y; }
             int list(void) { struct N a; struct N b; a.next = &b; a.v = 1; b.v = 2; return a.next->v; }
             int punning(void) { union U u; u.i = 0x41424344; return u.c; }
             int copied_union(void) { union U a; union U b; a.i = 0x41424344; b = a; return b.c; }
             int of_assignment(void) { struct P a = {1, 0}; return (a = a).x; }
             int doubles(void) { struct D a; struct D b; b.d = 3.5; b.i = 4; a = b; return (int)(a.d * 10.0) * 10 + a.i; }",
        )
        .expect("it compiles");
        let call = |name: &str| -> i32 {
            let address = jit.function(name).expect("the function is there");
            let function: extern "C" fn() -> i32 = unsafe { std::mem::transmute(address) };
            function()
        };
        assert_eq!(call("members"), 34);
        assert_eq!(call("braces"), 12);
        // The members a list does not name are zero.
        assert_eq!(call("designator"), 7);
        assert_eq!(call("copy"), 12);
        assert_eq!(call("assigned"), 34);
        assert_eq!(call("through_pointer"), 56);
        assert_eq!(call("address_of_member"), 9);
        assert_eq!(call("stepped"), 6);
        assert_eq!(call("by_size"), 4);
        assert_eq!(call("nested"), 78);
        assert_eq!(call("array_member"), 5);
        assert_eq!(call("array_of_structs"), 14);
        assert_eq!(call("list"), 2);
        // A union's members all start at zero: the same bytes read as an `int` and as a `char`.
        assert_eq!(call("punning"), 0x44);
        assert_eq!(call("copied_union"), 0x44);
        // An assignment's value is the object after the copy, so `.x` reads it.
        assert_eq!(call("of_assignment"), 1);
        // The copy is of bytes, so a `double` member comes along too.
        assert_eq!(call("doubles"), 354);
    }

    /// A bitfield read and written in code: the storage unit is loaded once, the field's bits are
    /// shifted and masked out of it, and a write puts them back without touching its neighbours.
    /// The layout these rely on is pinned by `a_record_lays_out_like_the_abi`.
    #[test]
    fn bitfields_are_masked_into_their_unit() {
        let mut jit = Jit::new().expect("a JIT");
        jit.add_source(
            "struct S { unsigned a : 3; unsigned b : 5; int c : 4; unsigned d : 20; };
             struct S a_global = { 5, 17, -3 };
             int write_and_read(void) {
                 struct S s; s.a = 5; s.b = 17; s.c = -3; s.d = 1000000;
                 return (int)(s.a * 1000 + s.b * 10) + (int)(s.d / 1000) + s.c;
             }
             int truncated(void) { struct S s; s.a = 9; return s.a; }
             int sign_extended(void) { struct S s; s.c = -7; return s.c; }
             int neighbours(void) { struct S s; s.a = 7; s.b = 31; s.c = 7; s.d = 0xFFFFF; s.a = 1;
                 return (int)(s.b * 100 + s.c) + (int)(s.d >> 16); }
             struct T { unsigned char x : 3; unsigned char y : 2; };
             int in_a_char(void) { struct T t; t.x = 5; t.y = 3; return t.x * 10 + t.y; }
             int compound(void) { struct S s; s.a = 1; s.a += 3; return s.a; }
             int assignment_value(void) { struct S s; s.c = -7; return (s.c = -3) == -3; }
             int stepped(void) { struct S s; s.a = 6; int after = ++s.a; s.a = 6; int before = s.a++;
                 return after * 1000000 + s.a * 10000 + before * 100; }
             int wraps(void) { struct S s; s.a = 7; s.a++; return s.a; }
             int signed_step(void) { struct S s; s.c = -1; int after = s.c--; return (after + 2) * 100 + (s.c + 8); }
             int initialised(void) { struct S one = { 5 }; struct S three = { 5, 17, -3 };
                 struct S named = { .c = -3 };
                 return one.a * 100000 + one.b * 10000 + three.a * 1000 + three.b * 10 + named.c; }
             int initialised_at_file_scope(void) {
                 return (int)(a_global.a * 1000 + a_global.b * 10) + a_global.c; }",
        )
        .expect("it compiles");
        let call = |name: &str| -> i32 {
            let address = jit.function(name).expect("the function is there");
            let function: extern "C" fn() -> i32 = unsafe { std::mem::transmute(address) };
            function()
        };
        assert_eq!(call("write_and_read"), 5 * 1000 + 170 + 1000 - 3);
        // Nine does not fit three bits: C stores the low three, so nine is one.
        assert_eq!(call("truncated"), 1);
        // A four-bit signed field holds -8..7, sign-extended to `int`.
        assert_eq!(call("sign_extended"), -7);
        // Writing one field leaves its neighbours alone.
        assert_eq!(call("neighbours"), 31 * 100 + 7 + 0xF);
        assert_eq!(call("in_a_char"), 53);
        assert_eq!(call("compound"), 4);
        // The value of the assignment is the field after the write, sign-extended: `(s.c = -3)`
        // is -3, not the masked bits that were stored.
        assert_eq!(call("assignment_value"), 1);
        // `++` on a field steps in the field's type and merges back in: a prefix form yields the
        // new value, a postfix form the old one, and seven plus one is zero in three bits.
        assert_eq!(call("stepped"), 7 * 1000000 + 7 * 10000 + 6 * 100);
        assert_eq!(call("wraps"), 0);
        // `s.c--` at -1 hands back the old value, -1, and leaves -2: (1 * 100) + 6.
        assert_eq!(call("signed_step"), 106);
        // An initialiser merges a field's bits into its unit's bytes too - in a frame and in a
        // global, where the bytes are built up without any code to run.
        // `one` is {5, 0, 0} as {a, b, c}; `three` is {5, 17, -3}, `named` is {0, 0, -3}.
        assert_eq!(call("initialised"), 500_000 + 5_000 + 170 - 3);
        assert_eq!(call("initialised_at_file_scope"), 5 * 1000 + 170 - 3);
    }

    /// `goto` and labels: a forward jump into code not yet read, a backward one that is a loop,
    /// a jump over dead code, and a label on a block that a `goto` may not target (the entry
    /// block, or a join, which takes parameters a `goto` has no values for) getting a block of
    /// its own.
    #[test]
    fn a_goto_jumps_to_its_label() {
        let mut jit = Jit::new().expect("a JIT");
        jit.add_source(
            "int forward(void) { int x = 0; goto skip; x = 100; skip: return x + 1; }
             int loop(void) { int n = 0; int s = 0; again: s = s + n; n = n + 1; if (n < 4) { goto again; } return s; }
             int dead(void) { int x = 0; goto end; x = 100; end: return x; }
             int at_the_top(int n) { int s = 0; top: if (n <= 0) { goto done; } s = s + n; n = n - 1; goto top; done: return s; }
             int shared(int x) { if (x) { goto out; } x = 2; out: return x; }",
        )
        .expect("it compiles");
        let call = |name: &str| -> i32 {
            let address = jit.function(name).expect("the function is there");
            let function: extern "C" fn() -> i32 = unsafe { std::mem::transmute(address) };
            function()
        };
        assert_eq!(call("forward"), 1);
        assert_eq!(call("loop"), 6);
        assert_eq!(call("dead"), 0);
        assert_eq!(call("shared"), 1);
        // A label at the top of a function that *has* parameters: the label needs a block of its
        // own, because a `goto` cannot jump to the entry block.
        let at_the_top: extern "C" fn(i32) -> i32 =
            unsafe { std::mem::transmute(jit.function("at_the_top").unwrap()) };
        assert_eq!(at_the_top(3), 6);
        assert_eq!(at_the_top(0), 0);
    }

    /// C99 6.3.1.2: any scalar converted to `_Bool` is 0 or 1. It used to be a narrowing
    /// conversion, so `(_Bool)2` stored 2, `(_Bool)0.5` was refused, and a pointer was not
    /// accepted at all - the three ways a narrowing conversion is the wrong rule.
    #[test]
    fn a_bool_holds_only_zero_or_one() {
        let mut jit = Jit::new().expect("a JIT");
        jit.add_source(
            "int from_int(void) { _Bool b = 2; return b; }
             int from_zero(void) { _Bool b = 0; return b; }
             int from_float(void) { _Bool b = 0.5; return b; }
             int from_zero_float(void) { _Bool b = 0.0; return b; }
             int from_nan(void) { double nan = 0.0 / 0.0; _Bool b = nan; return b; }
             int from_pointer(void) { int x; _Bool b = &x; return b; }
             int from_null(void) { int *p = 0; _Bool b = p; return b; }
             int cast(int x) { return (_Bool)x; }
             int added(void) { _Bool b = 1; return b + b; }
             int negated(void) { _Bool b = 0; return !b; }
             int size(void) { return sizeof(_Bool); }",
        )
        .expect("it compiles");
        let call = |name: &str| -> i32 {
            let address = jit.function(name).expect("the function is there");
            let function: extern "C" fn() -> i32 = unsafe { std::mem::transmute(address) };
            function()
        };
        assert_eq!(call("from_int"), 1);
        assert_eq!(call("from_zero"), 0);
        assert_eq!(call("from_float"), 1);
        assert_eq!(call("from_zero_float"), 0);
        // NaN is not zero, so it is true - and `fcvt_to_sint` of it would be undefined.
        assert_eq!(call("from_nan"), 1);
        assert_eq!(call("from_pointer"), 1);
        assert_eq!(call("from_null"), 0);
        let address = jit.function("cast").expect("the function is there");
        let cast: extern "C" fn(i32) -> i32 = unsafe { std::mem::transmute(address) };
        assert_eq!(cast(300), 1);
        assert_eq!(cast(0), 0);
        // A `_Bool` is promoted like any other narrow integer, so the arithmetic is `int`s.
        assert_eq!(call("added"), 2);
        assert_eq!(call("negated"), 1);
        assert_eq!(call("size"), 1);
    }

    /// C99 6.3.1.8: the integer promotions come before the usual arithmetic conversions, so
    /// `char + char` is `int + int` and cannot wrap at eight bits. It did: 120 + 121 came back
    /// as -15, because `usual_arithmetic` compared the two types without promoting either.
    #[test]
    fn arithmetic_promotes_what_is_narrower_than_int() {
        let mut jit = Jit::new().expect("a JIT");
        jit.add_source(
            "int chars(void) { char a = 120; char b = 121; return a + b; }
             int unsigned_chars(void) { unsigned char a = 200; return a * 2; }
             int negative(void) { char a = -1; return a < 0; }
             int promoted_size(void) { char a = 1; return sizeof(a + a); }",
        )
        .expect("it compiles");
        for (name, wanted) in [
            ("chars", 241),
            ("unsigned_chars", 400),
            ("negative", 1),
            ("promoted_size", 4),
        ] {
            let address = jit.function(name).expect("the function is there");
            let function: extern "C" fn() -> i32 = unsafe { std::mem::transmute(address) };
            assert_eq!(function(), wanted, "{name}");
        }
    }

    /// Two JITs do not see each other's code, which is what "no global state" buys.
    #[test]
    fn two_jits_are_independent() {
        let mut first = Jit::new().expect("a JIT");
        first
            .add_source("int add(int a, int b) { return a + b; }")
            .expect("it compiles");
        let mut second = Jit::new().expect("a JIT");
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
                        let mut jit = Jit::new().expect("a JIT");
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
        let mut jit = Jit::new().expect("a JIT");
        let error = jit
            .add_source("int add(int a) { return a + `; }")
            .expect_err("this does not compile");
        assert!(!error.is_empty(), "the error says something");
    }
}
