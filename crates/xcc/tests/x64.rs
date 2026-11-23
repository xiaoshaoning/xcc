//! ADR-008: this compiler's own back end, `--opt-level=2`.
//!
//! Two things are tested, and they are the two claims the slice makes. First, that the constructs
//! it does lower behave: a program through this back end and linked by the host's linker exits with
//! what the same program means. Second, a ratchet over the corpus: every program this back end
//! accepts is built *both* ways - level 2 through this back end, level 0 through Cranelift - linked,
//! run, and compared, so a construct that is subtly wrong shows up as a different exit code instead
//! of as an object nobody ran. The count is printed even when it passes, so a shrinking subset is
//! visible, and the programs it cannot compile yet are named rather than left to be guessed at.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use xcc::{Compile, Options};

/// Corpus programs that cannot be compared, with the reason. `pp_pptr` prints `%p` of a local, and a
/// stack address is a property of the run rather than of the source: two builds of the same program
/// disagree in exactly the way the comparison is looking for. It is named here rather than excluded
/// silently, because "the test cannot see this" and "this disagrees" must not look alike.
const NOT_COMPARABLE: &[&str] = &["pp_pptr"];

/// How many corpus programs agreed, through this back end, when the ratchet was last raised - with
/// Cranelift where it answers and with `clang` where it does not. The 20 the back end still refuses
/// are named by the run, not by this number.
const PROGRAMS_THAT_AGREED: usize = 443;

/// How many corpus programs this back end compiles and Cranelift refuses, and which are therefore
/// compared with `clang` instead. A ratchet like the one above, in the other direction: closing an
/// ABI gap raises it, and losing one lowers it.
const PROGRAMS_ONLY_THIS_BACK_END_COMPILES: usize = 5;

/// A linker for the object this back end writes: `clang` first, because it drives `lld-link` and
/// the object is COFF.
fn linker() -> Option<&'static str> {
    ["clang", "cc", "gcc"]
        .into_iter()
        .find(|candidate| Command::new(candidate).arg("--version").output().is_ok())
}

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus")
}

/// The corpus, sorted, so two tests cannot disagree about which files it holds.
fn corpus_files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(corpus())
        .expect("the corpus is in the repository")
        .map(|entry| entry.expect("an entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "c"))
        .collect();
    files.sort();
    files
}

/// What running a program came to.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Ran {
    status: Option<i32>,
    stdout: Vec<u8>,
}

/// Compile with `opt_level`, link, run, and say what happened; `None` when this level refuses the
/// program, which is how a gap is told from a wrong answer.
fn build_and_run(source: &str, opt_level: u8, dir: &Path, name: &str) -> Option<Ran> {
    let options = Options {
        opt_level,
        ..Options::default()
    };
    let artifact = Compile::new(options).compile(name, source).ok()?;
    let object = dir.join(format!("{name}.o{opt_level}.obj"));
    let exe = dir.join(format!("{name}.o{opt_level}.exe"));
    std::fs::write(&object, artifact.object()).expect("the object is written");
    let link = Command::new(linker()?)
        .arg("-o")
        .arg(&exe)
        .arg(&object)
        .output()
        .expect("the linker runs");
    if !link.status.success() {
        eprintln!(
            "{name} at level {opt_level}: linking failed: {}",
            String::from_utf8_lossy(&link.stderr)
        );
        return None;
    }
    let run = Command::new(&exe).output().expect("the program runs");
    Some(Ran {
        status: run.status.code(),
        stdout: run.stdout,
    })
}

/// The same program through `clang`, which is the oracle for the programs only this back end
/// compiles: a ratchet that counts a program as "compiled here" and never looks at what it answers
/// is a coverage hole wearing a number.
fn run_with_clang(source: &str, dir: &Path, name: &str) -> Option<Ran> {
    let file = dir.join(format!("{name}.ref.c"));
    let exe = dir.join(format!("{name}.ref.exe"));
    std::fs::write(&file, source).expect("the reference source is written");
    let built = Command::new("clang")
        .args(["-O0", "-w", "-o"])
        .arg(&exe)
        .arg(&file)
        .output()
        .ok()?;
    if !built.status.success() {
        return None;
    }
    let run = Command::new(&exe).output().ok()?;
    Some(Ran {
        status: run.status.code(),
        stdout: run.stdout,
    })
}

/// A directory of this test's own, which it cleans up.
fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("xcc-x64-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a temporary directory");
    dir
}

#[test]
fn the_constructs_the_slice_lowers_behave() {
    let Some(_) = linker() else {
        eprintln!("no linker on this machine: the own back end's tests are skipped");
        return;
    };
    let dir = temp_dir("constructs");
    // Each program returns what it means, and the exit status is the return value of `main`.
    let programs: &[(&str, &str, i32)] = &[
        (
            "a constant",
            "int main(void) { return 6 * 7; }",
            42,
        ),
        (
            "arguments and a direct call",
            "int add(int a, int b) { return a + b; } int main(void) { return add(20, 22); }",
            42,
        ),
        (
            // The argument registers are also candidates for allocation, so moving the first
            // argument into `r8` must not happen while the third is still there.
            "four arguments, which is all the registers hold",
            "int f(int a, int b, int c, int d) { return a * 1000 + b * 100 + c * 10 + d; } int main(void) { return f(1, 2, 4, 4); }",
            1244,
        ),
        (
            "three arguments, and a call",
            "int g(int a, int b, int c) { return a - b - c; } int main(void) { return g(10, 2, 3); }",
            5,
        ),
        (
            // A returned value may live in `rax`, the register the return uses; an earlier `return`
            // of something else writes `rax`, and must not be allowed to destroy it.
            "an early return before the value returned later",
            "int f(int c, int a) { int ok = c == 0; int v = a + 1; if (ok) return 1; return v; } int main(void) { return f(1, 4) == 5 ? 0 : 1; }",
            0,
        ),
        (
            // The data section: a global object, read through a relocation.
            "a global object",
            "int g = 5; int main(void) { g = g + 37; return g; }",
            42,
        ),
        (
            "a static local, which is a global in everything but name",
            "int bump(void) { static int n; n = n + 20; return n; } int main(void) { bump(); return bump(); }",
            40,
        ),
        (
            "an initialised array in the data section",
            "int a[4] = {10, 20, 30, 40}; int main(void) { return a[0] + a[3]; }",
            50,
        ),
        (
            "a string literal's bytes",
            "int main(void) { char *s = \"hello\"; return s[1]; }",
            101,
        ),
        (
            // Arguments past the fourth go on the stack: the caller leaves room above the shadow
            // space and the callee reads them there.
            "five arguments, so the fifth is on the stack",
            "int five(int a, int b, int c, int d, int e) { return e - a; } int main(void) { return five(1, 2, 3, 4, 9) + 0; }",
            8,
        ),
        (
            // A stack argument of a wider type, and a narrow one in a register that has to be
            // canonicalised on arrival.
            "mixed sizes across the register and stack boundary",
            "long long f(long long a, int b, char c, int d, long long e) { return a + e + b + c + d; } int main(void) { return (int)(f(1, 2, 3, 4, 5) - 15); }",
            0,
        ),
        (
            "seven arguments",
            "int seven(int a, int b, int c, int d, int e, int f, int g) { return a + g; } int main(void) { return seven(10, 1, 2, 3, 4, 5, 32); }",
            42,
        ),
        (
            // A leaf that cannot use caller-saved registers (three arguments is more than two) has
            // to preserve the callee-saved ones it touches, or the caller's values die at the call.
            "a three-argument leaf called twice, with values kept across both",
            "int three(int a, int b, int c) { return a + b + c; } int main(void) { int x = three(1, 2, 3); int y = three(4, 5, 6); return x + y; }",
            21,
        ),
        (
            // Floating point: its own register file, its own widths, and a comparison whose answer
            // is an `int` again.
            "doubles in arithmetic, narrowed on the way out",
            "int main(void) { double x = 2.5; double y = 4.0; return (int)(x * y); }",
            10,
        ),
        (
            "a double division and subtraction",
            "int main(void) { double a = 7.0, b = 2.0; return (int)(a / b) + (int)(a - b); }",
            8,
        ),
        (
            "a double comparison",
            "int main(void) { double z = 3.75; if (z > 1.5) return 42; return 1; }",
            42,
        ),
        (
            "negative doubles, which are a subtraction from zero",
            "int main(void) { double c = -3.125; double d = -1.25; if (c < 0 && d > -2) return 42; return 1; }",
            42,
        ),
        (
            "float widened to double",
            "int main(void) { float f = 1.5f; double d = f; return (int)(d * 2.0); }",
            3,
        ),
        (
            "double equality, which is not the same question as ordering",
            "int main(void) { double a = 7.0; return (a == 7.0 ? 100 : 0) + (int)(a / 2.0); }",
            103,
        ),
        (
            // A global's address arrives in `r8`, so a float load through it needs the REX byte that
            // names that base: without it the load read `[rax]` and answered nonsense.
            "a double in the data section",
            "double g = 2.5; int main(void) { return (int)(g * 2.0); }",
            5,
        ),
        (
            "a double through a pointer",
            "int main(void) { double d = 4.5; double *p = &d; return (int)(*p * 2.0); }",
            9,
        ),
        (
            // A call preserves `xmm6`-`xmm15` and destroys the rest, so a `double` that is live
            // across a call must be in one of those, and the caller must save it: this value was
            // read back as garbage because it lived in a volatile register.
            "a double that survives a call which uses XMM registers",
            "int noisy(void) { double a = 1.0, b = 2.0, c = 3.0, d = 4.0; double r = (a + b) * (c + d); return (int)r; } int main(void) { double keep = 2.5 * -1.25; int x = noisy(); if (keep < 0) return 42 + x; return 1; }",
            63,
        ),
        (
            // A function pointer: the callee's address is a symbol, and the call is `call r/m`.
            "a call through a function pointer",
            "int add(int a, int b) { return a + b; } int main(void) { int (*p)(int, int) = add; return p(20, 22); }",
            42,
        ),
        (
            "a function pointer passed to another function",
            "int add(int a, int b) { return a + b; } int mul(int a, int b) { return a * b; } int apply(int (*f)(int, int), int a, int b) { return f(a, b); } int main(void) { return apply(mul, 6, 7); }",
            42,
        ),
        (
            "a function pointer called twice",
            "int twice(int x) { return x * 2; } int main(void) { int (*p)(int) = twice; int r = p(p(10)); return r; }",
            40,
        ),
        (
            // An address read by something other than the call that uses it: the two programs that
            // used to be refused here are correct now.
            "a function's address compared with zero",
            "int f(void) { return 7; } int main(void) { return f != 0; }",
            1,
        ),
        (
            "a function's address passed to another function",
            "int f(void) { return 7; } int g(int (*p)(void)) { return p(); } int main(void) { return g(f); }",
            7,
        ),
        (
            // Floating point across a call: doubles in `xmm0`-`xmm3` by position and back in `xmm0`,
            // with the registers above `xmm7` that the callee-saved pool uses named by a REX byte.
            "a double passed to a function and returned",
            "double add2(double a, double b) { return a + b; } int main(void) { return (int)(add2(2.5, 1.25) * 2.0); }",
            7,
        ),
        (
            "integers and doubles mixed in one signature",
            "double mix(int a, double b, int c, double d) { return a + b + c + d; } int main(void) { return (int)mix(1, 2.5, 3, 4.25); }",
            10,
        ),
        (
            "a double fifth, which is on the stack",
            "double later(double a, int b, double c, int d, double e) { return a + e + b + c + d; } int main(void) { return (int)(later(1.0, 2, 3.0, 4, 5.0) - 15.0); }",
            0,
        ),
        (
            "a float parameter and return",
            "float half(float x) { return x / 2.0f; } int main(void) { return (int)(half(9.0f) * 2.0f); }",
            9,
        ),
        (
            // The program that failed three attempts: a value live across a call to a function that
            // uses XMM registers itself.
            "a double computed, then compared, around a call",
            "double f2(double a, double b) { return a - b; } int main(void) { double a = 2.5; double b = -1.25; double c = a * b; double e = f2(a, 10); if (c < 0 && e < 0) return 42; return 1; }",
            42,
        ),
        (
            // More live doubles than the leaf pool has registers, so operands and results spill: the
            // case the three-register invariant exists for.
            "twelve doubles, more than the registers hold",
            "int main(void) { double d0=0.5; double d1=1.5; double d2=2.5; double d3=3.5; double d4=4.5; double d5=5.5; double d6=6.5; double d7=7.5; double d8=8.5; double d9=9.5; double d10=10.5; double d11=11.5; double r = (d0+d1)*(d2+d3)+(d4+d5)*(d6+d7)-(d8+d9)*(d10+d11)+(d0*d11-d1*d10); return (int)(r + 1000.0) % 251; }",
            244,
        ),
        (
            // `va_arg`: the first four arguments arrive in registers, and a variadic function's
            // prologue writes them into the caller's home area, which is what the list walks.
            "a variadic function reading its arguments",
            "#include <stdarg.h>
int sum(int count, ...) { va_list ap; int total = 0; int i; va_start(ap, count); for (i = 0; i < count; i++) total += va_arg(ap, int); va_end(ap); return total; } int main(void) { return sum(4, 10, 20, 30, 2); }",
            62,
        ),
        (
            // A floating-point to 64-bit integer conversion is `cvttsd2si`, not a widening: the
            // register holds a `double` and reading it as an integer is a reinterpretation. This is
            // the case that a widening shortcut took by mistake, and it panicked rather than
            // answering - 3 750 000 000 needs all sixty-four bits, so a 32-bit path would have
            // hidden the fault instead of showing it.
            "a double converted to a 64-bit integer",
            "int main(void) { double d = 3.75e9; long long a = (long long)d; long long b = (long long)1.5; return (int)(a / 1000000000 + b + 3); }",
            7,
        ),
        (
            // A cast in an integer constant expression: C99 6.6p6 allows one, and Lua sizes a
            // buffer exactly this way - `char b[((int)(16 * sizeof(void *) * sizeof(double)))]`.
            "a cast in an integer constant expression",
            "int main(void) { char b[((int)(16 * sizeof(void *) * sizeof(double)))];                 return (int)sizeof b == 1024 ? 42 : 1; }",
            42,
        ),
        (
            // `longjmp` out of three of this compiler's own frames: the C library's `longjmp` calls
            // `RtlUnwindEx`, which walks `.pdata` - and without a frame table this faulted rather
            // than jumped. The deepest check there is for the frame table.
            "longjmp out of three frames",
            "#include <setjmp.h>

             static jmp_buf env;

             static void deep(int n) { if (n == 0) longjmp(env, 42); else deep(n - 1); }

             int main(void) { int jumped = setjmp(env); if (jumped == 0) deep(3); return jumped; }",
            42,
        ),
        (
            // `signal.h` in anger: the declarator that returns a handler, the handler itself, and
            // `raise` calling it. A header that ships is one a program can use.
            "signal and raise",
            "#include <signal.h>

             static int got;

             static void handler(int number) { got = number; }

             int main(void) {

                 void (*previous)(int) = signal(SIGINT, handler);

                 if (previous == SIG_ERR) return 1;

                 raise(SIGINT);

                 return got == SIGINT ? 42 : 2; }",
            42,
        ),
        (
            // A block-scope `extern` declaration refers to the object with external linkage
            // (C99 6.2.2p4) and does not make one. The check needs the global read from somewhere
            // else afterwards: a private frame slot would still return the value just written.
            "a block-scope extern declaration",
            "int g = 5; int read(void) { return g; }

             int main(void) { extern int g; g = 7; return read(); }",
            7,
        ),
        (
            // A member's `_Alignas` (C11 6.7.5) is the *member's* alignment: `offsetof` moves, and
            // so does the struct's own alignment - which is what the ABI's `movaps` needs and what a
            // struct of two 8-byte integers does not have.
            "an aligned struct member",
            "#include <stddef.h>

             struct unit { __builtin_alignas(16) unsigned long long part[2]; };

             struct outer { char c; struct unit u; };

             int main(void) { return (int)offsetof(struct outer, u) == 16 && (int)sizeof(struct unit) == 16 ? 42 : 1; }",
            42,
        ),
        (
            // `localeconv`'s decimal point: the reason `locale.h` matters to a program that never
            // calls `setlocale`, and a header is only real if a program can read through it.
            "the locale's decimal point",
            "#include <locale.h>

             int main(void) { char point = localeconv()->decimal_point[0];
                 return point == '.' ? 42 : 1; }",
            42,
        ),
        (
            // `&f` where `f` names a function: C99 6.5.3.2p3 says the result is the designator,
            // because a function name already is an address and is never an lvalue. Lua hands a
            // handler over as `lua_atpanic(L, &panic)`.
            "the address of a function",
            "int panic(int n) { return n * 2; }

             int call(int (*handler)(int)) { return handler(21); }

             int main(void) { return call(&panic); }",
            42,
        ),
        (
            // An `extern` declaration of an incomplete array is a declaration and not a definition
            // (C99 6.9.2p2): `lua.h` says `extern const char lua_ident[];`, and laying down bytes
            // for it would invent an object of incomplete type in every file that includes the
            // header.
            "an extern declaration of an incomplete array",
            "extern const char lua_ident[];

             int main(void) { return 42; }",
            42,
        ),
        (
            // `errno` is the C library's, reached through a function because it is one per thread:
            // the header is `#define errno (*_errno())` and the overflow is `strtol`'s own.
            "errno is the C library's",
            "#include <errno.h>
#include <stdlib.h>

             int main(void) { errno = 0; strtol(\"99999999999999999999\", 0, 10);
                 return errno == ERANGE ? 42 : 1; }",
            42,
        ),
        (
            // `assert` with `NDEBUG`: the same header, the other arm of its `#ifdef`, and the
            // failing condition is never evaluated - so a program that would abort does not.
            "assert with NDEBUG",
            "#define NDEBUG
#include <assert.h>

             int main(void) { assert(0); return 7; }",
            7,
        ),
        (
            // The spellings `iso646.h` asks for, in an expression rather than a statement.
            "the iso646 spellings",
            "#include <iso646.h>

             int main(void) { int a = 1; int b = 0;
                 if (a and not b) return ((a bitand 3) == 1) and (((a bitor b) xor 0) == 1) ? 42 : 1;
                 return 0; }",
            42,
        ),
        (
            // C99 7.12's additions to `math.h`, and 7.8's `inttypes.h`: the functions are the
            // library's, the classification macros are the header's own, and the `PRI` macros are
            // what a program uses to print a type whose width it does not know.
            "the C99 additions to math.h and inttypes.h",
            "#include <inttypes.h>

             #include <math.h>

             #include <stdio.h>

             #include <string.h>

             int main(void) {

                 char text[32];

                 imaxdiv_t parts = imaxdiv(17, 5);

                 snprintf(text, sizeof text, \"%\" PRIdMAX \":%\" PRIxMAX, (intmax_t)-2, (intmax_t)255);

                 if (strcmp(text, \"-2:ff\") != 0) return 1;

                 if (parts.quot != 3 || parts.rem != 2) return 2;

                 if (imaxabs((intmax_t)-7) != 7) return 3;

                 if (log2(8.0) != 3.0 || exp2(5.0) != 32.0) return 4;

                 if (cbrt(27.0) != 3.0 || hypot(3.0, 4.0) != 5.0) return 5;

                 if (round(2.5) != 3.0 || trunc(-2.7) != -2.0) return 6;

                 if (copysign(3.0, -1.0) != -3.0) return 7;

                 if (fmin(2.0, 3.0) != 2.0 || fmax(2.0, 3.0) != 3.0) return 8;

                 if (!(nextafter(1.0, 2.0) > 1.0)) return 9;

                 /* An infinity *computed*, not `HUGE_VAL`: on this target `HUGE_VAL` is the largest
                    finite double (MSVC's choice), so `isinf(HUGE_VAL)` is false and a test that
                    expected otherwise would be testing the constant rather than the classification. */
                 if (!(isnan(0.0 / 0.0) && isinf(1e308 * 10) && isinf(-1e308 * 10) && !isinf(1.0)))                      return 10;

                 if (!(signbit(-1.0) && isnormal(1.0) && fpclassify(1.0) == FP_NORMAL)) return 11;

                 return 42; }",
            42,
        ),
        (
            // A parameter that takes no frame slot still takes a *position*: a record passed by
            // reference is the caller's copy, so it has no slot - and without a placeholder the
            // parameters after it moved one place earlier, which made `va_start` point at the
            // argument before the one it meant.
            "va_start after a record passed by reference",
            "#include <stdarg.h>

             struct Big { long long a, b, c; };

             static long long f(struct Big s, int last, ...) { va_list ap; va_start(ap, last);

                 return s.a + va_arg(ap, int); }

             int main(void) { struct Big b = {40, 0, 0}; return f(b, 0, 2) == 42 ? 42 : 1; }",
            42,
        ),
        (
            // `va_start` with more than one variadic function in the file: the record of which slot
            // each parameter lives in is per function, and it was never cleared, so the second
            // function's `last` was found at twice its position and `va_start` pointed past the
            // argument it meant. Lua has dozens of variadic functions, and every error message it
            // printed was built from the wrong slots.
            "va_start in two functions of one file",
            "#include <stdarg.h>

             static int first(int a, ...) { va_list ap; va_start(ap, a); return va_arg(ap, int); }

             static int second(int a, int b, ...) { va_list ap; va_start(ap, b);

                 return va_arg(ap, int) * 10 + va_arg(ap, int); }

             int main(void) { return first(0, 3) + second(0, 0, 4, 2) == 45 ? 42 : 1; }",
            42,
        ),
        (
            // A bare `return;` in a void function ends the block. It used to emit nothing at all, so
            // control fell through to whatever came next - and a program that folded a constant and
            // then returned folded it *and* emitted the operation, so `2^10` printed `2^100`.
            "a bare return in a void function",
            "static int marker;

             static void nothing(int a) { if (a) return; marker += 100; }

             int main(void) { nothing(1); return marker == 0 ? 42 : marker; }",
            42,
        ),
        (
            // A declaration whose initialiser is an assignment: C99 6.7.8p1 says an initialiser is an
            // *assignment*-expression. Parsing a conditional-expression instead left the assignment to
            // be swallowed by error recovery, so `int a = *p = f();` called nothing and stored `*p`'s
            // old value into `a`.
            "a declaration initialised by an assignment",
            "static int seven(void) { return 7; }

             int main(void) { int x = 1; int a = x = seven(); return a == 7 && x == 7 ? 42 : 1; }",
            42,
        ),
        (
            // A string literal in a boolean context: C99 6.3.2.1p3 decays an array wherever a value is
            // wanted, and `&&`'s operand is one. Lua writes `api_check(L, (idx <= top) && "index")`,
            // whose right operand *is* a string - without the decay this compared two arrays, an
            // ill-typed `cmp` the verifier refused.
            "an array in a boolean context",
            "int f(int x) { return x && \"index\"; }

             int g(int x) { return x || \"index\"; }

             int h(void) { return !\"non-empty\"; }

             int main(void) {

                 return f(0) == 0 && f(1) == 1 && g(0) == 1 && g(1) == 1 && h() == 0 ? 42 : 1; }",
            42,
        ),
        (
            // A `switch` arm that ends in a `?:` and falls through to the next arm: the fall-through
            // link has to be made from wherever control actually is, and the `?:` leaves it in a join
            // block of its own. Linking from the arm's first block instead left the join with no
            // terminator, and the arm fell off its own end into a trap.
            "a switch arm that ends in a conditional",
            "struct Z { int n; int *p; }; struct LS { int current; struct Z *z; };

             static int fill(void) { return 0; }

             #define zgetc(z) (((z)->n--)>0 ? *(z)->p++ : fill())

             static int walk(struct LS *ls) {

                 int seen = 0;

                 while (ls->current != 0) {

                     switch (ls->current) { default: ls->current = zgetc(ls->z); seen += 1; }

                 }

                 return seen; }

             int main(void) { int d[3] = {1, 2, 0}; struct Z z = {3, d};

                 struct LS ls = {9, &z}; return walk(&ls) == 3 ? 42 : 1; }",
            42,
        ),
        (
            // The same shape one construct over: an arm that ends in an `if` join.
            "a switch arm that ends in an if",
            "int f(int c) { int r = 0; switch (c) { case 1: if (c) r = 5; r += 1; } return r; }

             int main(void) { return f(1) == 6 ? 42 : 1; }",
            42,
        ),
        (
            // A `switch` on a narrow type: C99 6.8.4.2p5 promotes the controlling expression, and a
            // `switch` on an `unsigned char` that compared itself with `int` case labels was a
            // `cir` the verifier refused. `cJSON.c`'s number parser is written exactly this way.
            "a switch on an unsigned char",
            "int classify(unsigned char c) { switch (c) { case '0': case '1': return 7;
                       case 'x': return 11; default: return 0; } }
             int main(void) { return classify('1') + classify((unsigned char)'x') + classify('q'); }",
            18,
        ),
        (
            // An address in a global's initialiser: a function's, which is how every C program with a
            // table of callbacks is written.
            "a function address in a global's initialiser",
            "int my_malloc(int n) { return n; } int my_free(int n) { return n; }
             struct hooks { int (*allocate)(int); int (*release)(int); };
             static struct hooks global = { my_malloc, my_free };
             int main(void) { return global.allocate(40) + global.release(2); }",
            42,
        ),
        (
            // `->` on an array of structs: C99 6.3.2.1p3 decays the array to a pointer before the
            // operator applies, and every flexible-array member in real C is written this way. The
            // first version decayed only for the type test and computed the address from the array,
            // which the verifier caught as "loads a value of another type than its address points to".
            "a struct's member through an array of one",
            "struct P { int x; int y; }; struct Q { int n; struct P p[1]; };
             int main(void) { struct Q q; q.n = 1; q.p->x = 20; q.p->y = 22;
                              return q.p->x + q.p->y + q.n - 1; }",
            42,
        ),
        (
            // An address inside a global's initialiser: eight bytes the *linker* fills, which the
            // data section now carries a relocation for. Both shapes every C program has - a pointer
            // to a string, and a table of them.
            "a file-scope pointer to a string, and a table of them",
            // A `char` array of arrays is the other table: the literal initialises the element, not
            // its first `char`, and a local's copy has to land at the element's offset in the frame.
            "char *p = \"hello\"; char *names[] = {\"alpha\", \"beta\"}; int table[] = {3, 4, 5};
             char grid[2][4] = {\"ab\", \"cd\"};
             struct entry { char *name; int value; };
             struct entry entries[] = {{\"x\", 7}, {\"y\", 8}};
             int local(void) { char g[2][4] = {\"ef\", \"gh\"}; return g[1][0]; }
             int main(void) { return p[1] + names[1][0] + table[2] + grid[0][1] + entries[1].name[0]
                                    + entries[0].value + local() - 400; }",
            // 101 + 98 + 5 + 'b'(98) + 'y'(121) + 7 + 'g'(103) - 400 = 133 - ... see below.
            133,
        ),
        (
            // A variadic function that *also* calls something: the call makes it keep its integers in
            // callee-saved registers, so the prologue pushes - and the home area `va_start` points at
            // is above the pushes as well as above the frame. This is the case a pushed register and a
            // `va_list` met in, and the answer was two out of sixty-six before it was.
            "a variadic function that also calls, reading its arguments",
            "#include <stdarg.h>
             int helper(void) { return 1; }
             int sum(int count, ...) { va_list ap; int total = 0; int i; va_start(ap, count); for (i = 0; i < count; i++) { total += va_arg(ap, int); total += helper(); } va_end(ap); return total + i; }
             int main(void) { return sum(4, 10, 20, 30, 2); }",
            70,
        ),
        (
            // A branch that compares for itself, in the three shapes that tell fusion from a wrong
            // fusion: an integer comparison read once, a *floating-point* one read once (which is
            // not fused - `ucomisd` settles unordered cases with other condition codes), and one
            // read twice, which has to exist as a value and must not be fused either.
            "a comparison carried by its branch, and two that are not",
            "int main(void) { double a = 1.5, b = 2.5; int x = 4, r = 0; if (x > 10) r += 1; if (a < b) r += 2; int both = x < a * 8.0; r += both * 4 + (both ? 8 : 0); return r + (a < b ? 16 : 32); }",
            30,
        ),

        (
            // A floating-point argument to a variadic function: the caller writes it to a vector
            // register *and* to the general register of its position, which is what lets the
            // callee's eight-byte-per-argument walk find a `double` where it finds an `int`.
            "a variadic function reading a double, and a call that mixes the two",
            "#include <stdarg.h>
double pick(int count, ...) { va_list ap; double total = 0.0; int i; va_start(ap, count); for (i = 0; i < count; i++) total = total + va_arg(ap, double); va_end(ap); return total; }
double tangled(int tag, ...) { va_list ap; double total = 0.0; va_start(ap, tag); total = total + va_arg(ap, double); total = total + va_arg(ap, int); total = total + va_arg(ap, double); va_end(ap); return total + tag; }
int main(void) { return (int)(pick(3, 1.5, 2.25, 3.25) * 4.0 + tangled(1, 10.5, 5, 2.5)); }",
            47,
        ),
        (
            "a variadic function reading past the fourth argument",
            "#include <stdarg.h>
long long pick(int first, ...) { va_list ap; va_start(ap, first); char c = (char)va_arg(ap, int); int i = va_arg(ap, int); long long l = va_arg(ap, long long); va_end(ap); return first + c + i + l; } int main(void) { return (int)(pick(1, 'A', 1000, 100000LL) % 251); }",
            164,
        ),
        (
            "a loop, which becomes block parameters",
            "int main(void) { int i; int s = 0; for (i = 0; i < 10; i++) s += i; return s; }",
            45,
        ),
        (
            "a branch",
            "int main(void) { int x = 100; if (x > 50) return 1; return 2; }",
            1,
        ),
        (
            "division and remainder, signed",
            "int f(int a) { return a / 3 + a % 3; } int main(void) { return f(100); }",
            34,
        ),
        (
            "an array in the frame, read and written",
            "int main(void) { int a[3]; a[0] = 7; a[1] = 8; a[2] = 9; return a[0] + a[2]; }",
            16,
        ),
        (
            "an unsigned comparison",
            "int main(void) { unsigned u = 4294967295u; return u > 1 ? 3 : 4; }",
            3,
        ),
        (
            "narrow types, sign-extended",
            "int main(void) { return (int)(char)-1 + (int)(unsigned char)200; }",
            199,
        ),
        (
            "64-bit arithmetic",
            // 3815 is the whole return value: an exit status is 32 bits, not the 8 a shell shows.
            "int main(void) { long long x = 4000000000LL; return (int)(x + 3000 > 0) + (int)(x >> 20); }",
            3815,
        ),
        (
            "recursion",
            "int fact(int n) { if (n < 2) return 1; return n * fact(n - 1); } int main(void) { return fact(5); }",
            120,
        ),
        (
            "a switch",
            "int main(void) { int x = 3, r = 0; switch (x) { case 1: r = 10; break; case 3: r = 30; break; default: r = 99; } return r; }",
            30,
        ),
        (
            "a pointer into a local",
            "int main(void) { int a[4]; int *p = a; int i; for (i = 0; i < 4; i++) a[i] = i * 3; return *(p + 1) + *(p + 3); }",
            12,
        ),
        (
            "break and continue",
            "int main(void) { int i = 0, s = 0; while (1) { i++; if (i > 5) break; if (i == 2) continue; s += i; } return s; }",
            13,
        ),
        (
            "signed and unsigned right shifts",
            "int main(void) { int a = 7, b = -7; unsigned c = 0x80; return (a >> 1) + (b >> 1) + (c >> 6) + 6; }",
            7,
        ),
        (
            "a 64-bit result narrowed by the return",
            "long long f(long long a, long long b) { return a * b - 7; } int main(void) { return (int)(f(1000, 1000) % 251); }",
            9,
        ),
    ];
    let mut wrong = Vec::new();
    for (what, source, expected) in programs {
        let name = format!("t{}", wrong.len());
        let Some(ran) = build_and_run(source, 2, &dir, &name) else {
            wrong.push(format!("{what}: this back end refused it"));
            continue;
        };
        if ran.status != Some(*expected) {
            wrong.push(format!(
                "{what}: exited with {:?}, expected {expected}",
                ran.status
            ));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// A call through a function pointer whose value never escapes is not a gap: `mem2reg` promotes the
/// variable, the callee becomes the function's own address, and the call is direct. It is worth a
/// test because the direct-call path is the one that could drop the address.
#[test]
fn a_function_address_that_never_escapes_is_still_a_direct_call() {
    let Some(_) = linker() else {
        eprintln!("no linker on this machine: the own back end's tests are skipped");
        return;
    };
    let dir = temp_dir("address-direct");
    let source = "int f(void) { return 7; } int main(void) { int (*p)(void) = f; return p(); }";
    let ran = build_and_run(source, 2, &dir, "direct");
    let _ = std::fs::remove_dir_all(&dir);
    let ran = ran.expect("a call the lowering can see through compiles");
    assert_eq!(
        ran.status,
        Some(7),
        "the call went to the function it names"
    );
}

/// Every program this compiler refuses is one `clang -std=c99` refuses too.
///
/// The corpus gate reports a **count** of refusals, and a count of refusals hides the only thing worth
/// knowing about them: whether the reason is the program's or the compiler's. Twenty are refused today,
/// and every one of them is a program no C compiler accepts - a deliberate negative like `// Negative:
/// too many array initializers must error`, a program that needs MATLAB's `<mex.h>`, or a program that
/// uses another compiler's `word` extension. **A refusal only this compiler makes would look exactly
/// like those in a count**, so what is pinned here is not how many are refused but that none of them is
/// ours: clang is the oracle, and a gap is a program clang accepts and this does not.
#[test]
fn every_program_refused_here_is_refused_by_clang_too() {
    let Ok(clang) = std::process::Command::new("clang")
        .arg("--version")
        .output()
    else {
        eprintln!("no clang on this machine: the refusal oracle is skipped");
        return;
    };
    assert!(clang.status.success(), "clang answers");
    let mut refused = 0usize;
    let mut gaps: Vec<String> = Vec::new();
    for path in corpus_files() {
        let source = std::fs::read_to_string(&path).expect("the source is readable");
        let options = Options {
            opt_level: 2,
            ..Options::default()
        };
        if Compile::new(options).compile("t.c", &source).is_ok() {
            continue;
        }
        refused += 1;
        let theirs = std::process::Command::new("clang")
            .args(["-std=c99", "-fsyntax-only"])
            .arg(&path)
            .output()
            .expect("clang runs");
        if theirs.status.success() {
            gaps.push(
                path.file_name()
                    .expect("a name")
                    .to_string_lossy()
                    .into_owned(),
            );
        }
    }
    eprintln!(
        "refusal oracle: {refused} refused, {} of them only here",
        gaps.len()
    );
    assert!(
        gaps.is_empty(),
        "refused here and accepted by clang -std=c99: {}",
        gaps.join(", ")
    );
}

/// The README quotes the ratchet, so the two move together or this test says so. The number drifted
/// once - four milestones of shrinking left the README claiming 414 - and a headline that lies is the
/// documentation equivalent of a wrong answer.
#[test]
fn the_readme_quotes_the_ratchet() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../README.md");
    let readme = std::fs::read_to_string(path).expect("the README is readable");
    let quoted = format!("{PROGRAMS_THAT_AGREED} of the 464");
    assert!(
        readme.contains(&quoted),
        "the README must quote `{quoted}` - it claims a different number"
    );
}

#[test]
fn the_corpus_agrees_through_both_back_ends() {
    let Some(_) = linker() else {
        eprintln!("no linker on this machine: the own back end's corpus test is skipped");
        return;
    };
    let mut files: Vec<PathBuf> = std::fs::read_dir(corpus())
        .expect("the corpus is in the repository")
        .map(|entry| entry.expect("an entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "c"))
        .collect();
    files.sort();

    let workers = std::thread::available_parallelism().map_or(4, |count| count.get());
    let next = AtomicUsize::new(0);
    let agreed = AtomicUsize::new(0);
    // Programs this back end compiles and Cranelift does not: `va_arg` and a floating-point argument
    // to a variadic call are the two, and counting them as refusals would report a gap we have closed
    // as one we have not.
    let beyond: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let refused: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let wrong: Mutex<Vec<String>> = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for worker in 0..workers {
            let (files, next, agreed, beyond, refused, wrong) =
                (&files, &next, &agreed, &beyond, &refused, &wrong);
            scope.spawn(move || {
                let dir = std::env::temp_dir()
                    .join(format!("xcc-x64-corpus-{}-{worker}", std::process::id()));
                let _ = std::fs::create_dir_all(&dir);
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(path) = files.get(index) else { break };
                    let name = path
                        .file_stem()
                        .expect("a file stem")
                        .to_string_lossy()
                        .into_owned();
                    if NOT_COMPARABLE.contains(&name.as_str()) {
                        continue;
                    }
                    let source = std::fs::read_to_string(path).expect("the source is readable");
                    let Some(optimised) = build_and_run(&source, 2, &dir, &name) else {
                        refused.lock().expect("the refusals").push(name);
                        continue;
                    };
                    // Level 0 is Cranelift's lowering of the same `cir`: the two back ends are two
                    // answers to one question, and they must agree where both answer. Where only one
                    // does, that is the ABI asymmetry ADR-006 records - level 2 duplicates a
                    // variadic float argument into the general register of its position and level 0
                    // refuses to - so it is counted, not called a failure.
                    let plain = build_and_run(&source, 0, &dir, &name);
                    let cranelift_answered = plain.is_some();
                    // Where Cranelift refuses, the oracle is `clang`: a program counted as compiled
                    // here and compared with nothing is a coverage hole wearing a number.
                    let Some(otherwise) = plain.or_else(|| run_with_clang(&source, &dir, &name))
                    else {
                        beyond
                            .lock()
                            .expect("the programs only level 2 compiles")
                            .push(name);
                        continue;
                    };
                    if !cranelift_answered {
                        beyond
                            .lock()
                            .expect("the programs only level 2 compiles")
                            .push(name.clone());
                    }
                    if otherwise == optimised {
                        agreed.fetch_add(1, Ordering::Relaxed);
                    } else {
                        let reference = if cranelift_answered {
                            format!("Cranelift {otherwise:?}")
                        } else {
                            format!("clang {otherwise:?}")
                        };
                        wrong
                            .lock()
                            .expect("the mismatches")
                            .push(format!("{name}: own back end {:?}, {reference}", optimised));
                    }
                }
                let _ = std::fs::remove_dir_all(&dir);
            });
        }
    });

    let agreed = agreed.into_inner();
    let mut wrong = wrong.into_inner().expect("the mismatches");
    let mut refused = refused.into_inner().expect("the refusals");
    let mut beyond = beyond
        .into_inner()
        .expect("the programs only level 2 compiles");
    wrong.sort();
    refused.sort();
    beyond.sort();
    eprintln!(
        "own back end: {agreed} corpus programs agreed, {} refused, {} wrong, {} compiled only          here",
        refused.len(),
        wrong.len(),
        beyond.len()
    );
    if !beyond.is_empty() {
        eprintln!("own back end: only here: {}", beyond.join(", "));
    }
    if !wrong.is_empty() {
        eprintln!("own back end: wrong: {}", wrong.join(", "));
    }
    assert!(wrong.is_empty(), "{} disagreed", wrong.len());
    assert!(
        agreed >= PROGRAMS_THAT_AGREED,
        "{agreed} programs agreed and {PROGRAMS_THAT_AGREED} used to: something this back end \
         lowered it no longer does"
    );
    assert!(
        beyond.len() >= PROGRAMS_ONLY_THIS_BACK_END_COMPILES,
        "{} programs are compiled only here and {PROGRAMS_ONLY_THIS_BACK_END_COMPILES} used to \
         be: one of them has become something this back end refuses",
        beyond.len()
    );
}
