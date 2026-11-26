# xcc

A small, embeddable C compiler for Windows x86-64, written in Rust. MIT.

`xcc` compiles C to native code in-process: to a COFF object you link, or to a JIT module whose
functions you call as function pointers. It is meant to be embedded in programs that need to run
C they did not ship with - a game's mod scripts, a plugin host, a teaching tool - so there is no
global state: every compilation owns everything it touches, and two can run on two threads.

## What works today

This is M6 of the plan in [PLAN.md](PLAN.md), with M7's C API and sandbox in place. The status
below is what `cargo test --workspace` proves, not what is intended.

```
source -> preprocess -> parse -> cir (verified SSA CFG) -> Cranelift -> COFF object
                                                        -> cranelift-jit -> callable from Rust
```

| | |
|---|---|
| preprocessor | trigraphs, splices, object- and function-like macros, `#`/`##`, variadics, conditionals evaluated in `intmax_t`, includes through a host trait, `#pragma once`, `#line`, and the macros an implementation owes a program: `__STDC__`, `__STDC_VERSION__` (the dialect's), `__STDC_HOSTED__`, `__FILE__`, `__LINE__`, `__DATE__`, `__TIME__`, and the target's own - **`_WIN32`, `_WIN64`, `__x86_64__`, `_M_X64`** - so `#ifdef _WIN32` answers the same way to `#if`, `#ifdef` and `defined()`. 426 corpus programs preprocess token-identical to `clang -E`. |
| headers | twenty-nine clean-room C headers ship inside the compiler (`crates/xcc/include`): all twenty-four of C99 7.1.2 - `assert`, `complex`, `ctype`, `errno`, `fenv`, `float`, `inttypes`, `iso646`, `limits`, `locale`, `math`, `setjmp`, `signal`, `stdarg` (whose `va_list`, `va_start` and `va_arg` are real at `--opt-level=2`), `stdbool`, `stddef`, `stdint`, `stdio`, `stdlib`, `string`, `tgmath`, `time`, `wchar`, `wctype` - and three of C11's: `stdalign`, `stdatomic` (real atomics: one `lock cmpxchg` in the back end, every other operation a loop of it in C, and two threads that must total exactly), `stdnoreturn`, and `threads` (mutexes and condition variables over the target's own critical sections, `thrd_create` over `_beginthreadex`, and `tss_t` over the runtime's thread-local slot), and
`uchar` (with `u"..."`, `U"..."` and `u8"..."` literals in the compiler itself: UTF-16 code units, code points, and
UTF-8 bytes). Wide characters work end to end: `L'a'` is a `wchar_t`, `L"ab"` an array of them stored as UTF-16, and the wide `printf`/`scanf` front ends are in the runtime like the narrow ones. `math.h` declares all three families 7.12 asks for - `sin`, `sinf` and `sinl`, where the last is a macro because this target's `long double` is its `double` - and `tgmath.h` picks between them with `_Generic`, so the argument's type decides and nothing is promoted - which is why that header needs `--std=c11`, and says so. `complex.h` ships the part of C99 7.3 that can be honoured: the `_Complex` types and their arithmetic are the compiler's, `creal`/`cimag` are its two halves, and `conj`, `cabs`, `carg` and `cproj` are the C runtime source's own - the universal runtime exports **none** of 7.3's sixty-six spellings, measured one link at a time, so all twenty bases are the runtime source's own - the four exact ones and the sixteen transcendentals - and a test compares each of them with `clang` at six points in every quadrant. Every other header of C99 7.1.2's table ships; a header that ever has to refuse does it by name with its reason, never `cannot find`. No `-I`, no filesystem, no host resolver; a host's own files come first. |
| C99 front end | scalars, enums, `typedef`, structs/unions with layout and bitfields, declarators including function pointers, all statements including `switch` parsing, initialisers with brace elision and designators, `sizeof`, casts. 394 of 394 corpus programs that are not `#include`d and not deliberately invalid are read completely. |
| code generation | locals and parameters as stack slots, globals and string literals as data (`.bss` for tentative definitions, anonymous `.strN` for literals), arithmetic, comparisons, conversions, casts, floats and doubles, aggregates by value (Win64 ABI), bitfields read/written/stepped/initialised, calls - direct, into host symbols, and through a function pointer - pointers, pointer arithmetic, subscripting, `if`/`else`, `while`, `do`/`while`, `for`, `goto`, `switch`, `break`, `continue`, `return`. |
| embedding | `Jit::new()`, `Jit::configured` for the dialect and the back end, `Jit::with_options` for those and calls back into the host, `Jit::add_source`, `Jit::function(name) -> Option<*const u8>`. Eight threads compiling at once is a test. From C, Python, Lua or anything with `dlopen`: `xcc_new`, `xcc_new_with` (an `xcc_options` with the dialect and the opt level), `xcc_add_source`, `xcc_function`, `xcc_last_error`, `xcc_free` in `crates/xcc-capi` (`xcc_capi.dll` and `include/xcc.h`) |
| objects | `Compile::compile` returns an `Artifact` whose bytes are a COFF object with `.text`, data sections and a symbol table; `tests/exe.rs` links one with the host linker and runs it. `xcc -c t.c --depfile t.d` writes the makefile rule too, naming the source and the host headers it read and *not* the headers that ship inside the compiler |
| linking | `xcc hello.c` writes `hello.exe` with no host tool: `back/x64/pe.rs` places the sections, patches `REL32`/`ADDR64`, writes a thunk per import, regenerates `.pdata`/`.xdata`, writes the import table and base relocations, and lays out a PE32+ image - and the C runtime that makes `main` a program (`crates/xcc-crt/src/main.c`) is compiled by xcc on every link - `printf` and `scanf` front ends, `atexit` and `argc`/`argv` included - and **every function the headers declare links**: the ten names with no home the measurement found (`scanf`, `fscanf`, `sscanf`, `atexit`, `_setjmp`, and the five 64-bit `time` names the library spells `_time64`, `_gmtime64`, ...) are placed, and a test asks `ucrtbase.dll`'s own export table for the rest. `tests/exe.rs` writes such an image, runs it, and checks both its output and its exit status. Real programs build this way, with no host tool anywhere: **cJSON** (3 206 lines, MIT) plus a driver parses, walks and round-trips a JSON file and prints byte-for-byte what a `clang` build prints, and **miniz** (9 500 lines, MIT) deflates, inflates, writes a zip archive and reads it back - the compressed stream is one real zlib can decompress, and the archive extracts byte-identical; **tinyexpr** (758 lines, zlib) parses and evaluates a list of expressions and compiles one with variables bound to it, byte for byte what a `clang` build prints; **lz4** (10 500 lines, BSD 2-Clause) compresses a hundred thousand bytes to 583 and back, through the block, streaming, high-compression, frame and stdio APIs, with **every compressed byte identical to a `clang` build's** - which LZ4 being deterministic makes a claim about bytes and not about behaviour. **All four are vendored under `crates/xcc/tests/fixtures` and all run on every test run**: `tests/real.rs` compiles and links them with this compiler alone and compares their output with the recorded `clang` build's, so the claim is a gate and not a paragraph |
| lowering | two back ends behind one `cir` (ADR-001 section 9). `--opt-level=0` and `1` are Cranelift's, at its two levels; `--opt-level=2` is this compiler's own x86-64 back end - instruction selection, linear-scan register allocation, `rsp`-relative frames, and a COFF writer - which covers everything the front end accepts except a file-scope initialiser that holds an address: integers, pointers, frames, control flow, direct and indirect calls, variadic definitions and `va_arg`, floating point inside a function and across calls and into variadic calls, aggregates by value, global objects with relocated addresses, and stack-passed arguments ([ADR-008](docs/ADR-008-own-back-end.md)). 443 of the 464 corpus programs build, link, run and agree through it - the other 21 are programs `clang` refuses too, or ones needing a header or another compiler's extension this machine does not have - and `crates/xcc/tests/x64.rs` keeps that number from shrinking. **It is 3.49x `clang -O2`'s code size** - `.text` summed over the 420 corpus programs both compilers accept - measured here. `the_corpus_is_within_the_code_size_that_was_measured` holds the *bytes this compiler emits* under a ceiling - 80 336 of them, over every corpus program it compiles - and reports the ratio as context. It is not the assertion: the ratio depends on which `clang` is installed, and CI proved it by failing on a runner whose `clang` is a different build while every byte this compiler wrote was identical. The 3.46x this row used to quote had drifted by the time anything checked it |
| optimising | four passes over `cir`, each measured: `mem2reg` (slots whose address never escapes become values, with block parameters where a loop carries one), a truth test of a truth value removed, constant folding to a fixpoint, and dead-code elimination. Then, in the back end, constants that travel inside the instruction that reads them, a comparison a branch carries instead of a value, and an address made once per block. **5.76x `clang -O2`'s code size down to 3.46x** when it was measured; the running total is 3.49x and is now a test rather than a sentence. The rest of the optimiser list - interval splitting, jump shortening, branch threading, cross-jumping - was measured and dropped with the numbers in [PLAN.md](PLAN.md). The differential runner compiles every corpus program at both levels *and* with clang, and every pair must agree |
| measuring | `xcc --target=dos16 -c t.c` hands the source to borland-reborn's port of Borland C++ 2.0 and writes its 16-bit OMF object; `cargo run -p bcc20-sys --example compare` prints time and code size for the same files both ways ([ADR-007](docs/ADR-007-measuring-stick.md)). A measuring stick, not a second product: the library is loaded at run time and never linked in |
| explaining | `xcc --explain t.c` lists each C-level instruction under the source line it came from - conversions and decays included, because they are instructions - then the Cranelift function with the virtual registers in it. `Compile::explain` is the same call for a host. |

## What does not work yet

Stated here rather than discovered later.

- **Variadic calls and definitions are `--opt-level=2` only.** `printf("%f", 1.5)`, `va_list`,
  `va_start`, `va_arg` and defining a variadic function all work there; levels 0 and 1 are
  Cranelift's, which has no home area to walk and writes a floating-point argument in one register
  file instead of two, so they refuse the construct with a span rather than print zeros
  ([ADR-006](docs/ADR-006-varargs.md)). A file-scope initialiser that holds an address -
  `char *p = "hello"` - is the same shape of asymmetry.
- **Three things are refused on purpose, and no compiler accepts them.** A file-scope initialiser
  that is not constant (`int h = g + 2;` at file scope - a job for a startup prologue this compiler
  does not generate), `word` as a 32-bit type (another compiler's extension), and `<mex.h>` (MATLAB).
  Everything else the corpus refuses is not C: assigning to an array or an rvalue, `&` of an rvalue,
  a wrong argument count, `<<` on a `double`.
- **A file-scope initialiser may not be an address expression.** `char *p = "hello"` and
  `char *names[] = {"a", "b"}` work (a relocation in the data section); `int *p = &x;` does not,
  because the parser has no symbolic constant to carry it ([ADR-004](docs/ADR-004-data.md)).
- **A link goes through this compiler's own back end, so it is `--opt-level=2`.** Levels 0 and 1 are
  Cranelift's, which keeps its sections to itself and cannot hand an image to a linker; a link at
  another level says so and suggests `-c` rather than quietly compiling at the level nobody asked for.
- **`--run` runs at every level, including this compiler's own back end.** `--opt-level=2 --run` loads
  the code the encoder made, applies its relocations in memory, and calls it - so the constructs that only
  level 2 compiles (`va_arg`, a floating-point variadic argument, a file-scope address) run in process
  too. The differential harness still compares the two Cranelift levels; putting the third code generator
  into it is the next step, and the corpus gate already compares its output through linked objects.
- **`--run` calls a subset of the C library.** A JIT is not a linker, so it resolves the C
  runtime's own functions itself - `printf`, `malloc`, `qsort`, `fopen` and the rest - from one
  library (the classic runtime, which exports the `printf` family under its own names). What that
  library does not export is absent: `snprintf`, and the `stdin`/`stdout`/`stderr` accessor, work
  in a linked build and not in process. A name that cannot be resolved is a diagnostic naming it.
- **No unwind data.** A Cranelift win64 object carries `.text` and no `.pdata`/`.xdata` (measured,
  and asserted, in `back::tests`; [ADR-003](docs/ADR-003-win64-unwind.md)). Calling compiled C
  works; *unwinding* through it - SEH, a debugger walking frames - does not.
- **Internal linkage is scoped everywhere, by two different rules.** The linker keys a `static`
  name - a function, an object, a string literal - by the unit it belongs to, because it sees each
  unit's own symbols. A `Jit` compiles each source separately: at `--opt-level=2` two sources may
  each define `static int helper(void)` and each keeps its own, and at levels 0 and 1 `add_source`
  refuses the second with `DuplicateDefinition` rather than resolving to the first. Both were
  measured; the linker's version was three bugs, and cJSON found the first of them.
- **Refused with one diagnostic each**: `_Complex`, `#pragma pack`, `#include_next`, a file-scope
  pointer initialised from a literal, and a struct value inside a brace list.
  [docs/C99-SUPPORT.md](docs/C99-SUPPORT.md) is the line-by-line answer, including the refusals
  that C itself asks for - the address of a bitfield, an array assigned to another array.
- **No optimiser.** `cir` is SSA from the first instruction, so `mem2reg` and friends are passes
  on this shape rather than a rewrite of it (M10).

## Using it

```rust
use xcc::Jit;

let mut jit = Jit::new()?;
jit.add_source("int add(int a, int b) { return a + b; }")?;
let address = jit.function("add").expect("compiled");
let add: extern "C" fn(i32, i32) -> i32 = unsafe { std::mem::transmute(address) };
assert_eq!(add(2, 3), 5);
```

From the command line, `xcc --run file.c` compiles the source in process, calls `main`, and
leaves with its value - the embedding API used the way a host would:

```
$ xcc --run hello.c; echo $?
```

For a host function callable from C, register it at construction - the addresses must outlive the
`Jit`:

```rust
let mut jit = unsafe { Jit::with_symbols(&[("host_double", host_double as *const u8)]) }?;
jit.add_source("int host_double(int); int twice(int x) { return host_double(x); }")?;
```

To produce an object instead:

```rust
let artifact = xcc::Compile::new(xcc::Options::default()).compile("t.c", source)?;
std::fs::write("t.obj", artifact.object())?;
```

## How it is built

The middle end is `cir`: a control-flow graph of blocks with parameters, SSA by construction, with
an explicit `load`/`store` for every C variable and a span and C type on every value. It is the
one seam between the front end and every back end; Cranelift is the first lowering, not the
architecture. [ADR-002](docs/ADR-002-intermediate-form.md) argues that choice against a quad list,
against Cranelift's IR as the middle end, and against an AST.

Decisions that were expensive to get wrong are recorded as they were made:
[ADR-001](docs/ADR-001-architecture.md) (architecture, and the MIT boundary),
[ADR-002](docs/ADR-002-intermediate-form.md) (the intermediate form),
[ADR-003](docs/ADR-003-win64-unwind.md) (unwind data: absent, and what that costs),
[ADR-004](docs/ADR-004-data.md) (where data lives, and the naming rule behind `Symbol` vs
`String`). [docs/AUDIT.md](docs/AUDIT.md) keeps the record of what each milestone's review found.

## Building and testing

```
cargo build --workspace          # library and the `xcc` binary
cargo test --workspace           # unit, parser, CLI, preprocessing gate, fuzz, exe, the fixtures
cargo test -- --ignored          # the tests that state what is not built yet
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

**The compiler builds on a Linux host; what it compiles does not run there.** The target is Windows
x86-64 - PE32+, `ucrtbase`, and a C runtime this compiler writes - so the test files that *run* what
they build are `#![cfg(target_os = "windows")]` as a whole: `x64`, `cli`, `differential`, `hostile`,
and the `exe` and `real` tests that run a program. On another host those files have no tests rather
than failing ones, and

```
cargo check --workspace --all-targets --target x86_64-unknown-linux-musl
```

is the check that the rest - the preprocessor, the parser, the type system, `cir`, the passes and the
x86-64 encoder - is host-agnostic. It reports no errors.

Requires Rust 1.82 or newer. `tests/corpus/` holds 464 C programs used as a gate; their
provenance is in [tests/corpus/README.md](tests/corpus/README.md).

## Licence

MIT. The corpus is the author's own, granted under the same terms. No code, table or message text
is taken from any other compiler: reference compilers are run as oracles, never linked.
