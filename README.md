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
| preprocessor | trigraphs, splices, object- and function-like macros, `#`/`##`, variadics, conditionals evaluated in `intmax_t`, includes through a host trait, `#pragma once`, `#line`, the standard macros. 426 corpus programs preprocess token-identical to `clang -E`. |
| headers | twelve clean-room C headers ship inside the compiler (`crates/xcc/include`): `stddef`, `stdbool`, `stdint`, `limits`, `float`, `stdio`, `stdlib`, `string`, `ctype`, `math`, `time`, and a `stdarg` that refuses with a reason. No `-I`, no filesystem, no host resolver; a host's own files come first. |
| C99 front end | scalars, enums, `typedef`, structs/unions with layout and bitfields, declarators including function pointers, all statements including `switch` parsing, initialisers with brace elision and designators, `sizeof`, casts. 394 of 394 corpus programs that are not `#include`d and not deliberately invalid are read completely. |
| code generation | locals and parameters as stack slots, globals and string literals as data (`.bss` for tentative definitions, anonymous `.strN` for literals), arithmetic, comparisons, conversions, casts, floats and doubles, aggregates by value (Win64 ABI), bitfields read/written/stepped/initialised, calls - direct, into host symbols, and through a function pointer - pointers, pointer arithmetic, subscripting, `if`/`else`, `while`, `do`/`while`, `for`, `goto`, `switch`, `break`, `continue`, `return`. |
| embedding | `Jit::new()`, `Jit::add_source`, `Jit::function(name) -> Option<*const u8>`, `Jit::with_symbols` for calls back into the host. Eight threads compiling at once is a test. From C, Python, Lua or anything with `dlopen`: `xcc_new`, `xcc_add_source`, `xcc_function`, `xcc_last_error`, `xcc_free` in `crates/xcc-capi` (`xcc_capi.dll` and `include/xcc.h`) |
| objects | `Compile::compile` returns an `Artifact` whose bytes are a COFF object with `.text`, data sections and a symbol table; `tests/exe.rs` links one with the host linker and runs it. |
| lowering | two back ends behind one `cir` (ADR-001 section 9). `--opt-level=0` and `1` are Cranelift's, at its two levels; `--opt-level=2` is this compiler's own x86-64 back end - instruction selection, linear-scan register allocation, `rsp`-relative frames, and a COFF writer - which covers integers, pointers, frames, control flow and direct calls, and refuses the rest with a source span ([ADR-008](docs/ADR-008-own-back-end.md)). 300 of the 464 corpus programs build, link, run and agree through it, `crates/xcc/tests/x64.rs` keeps that number from shrinking, and it is 5.2x Cranelift's code size until the shrinking work starts |
| optimising | `mem2reg` over `cir` - slots whose address never escapes become values, with block parameters where a loop carries one - then Cranelift's own optimiser behind `--opt-level=1`. On the six-file corpus: 4857 bytes of x64 before the pass, 3281 after it, 1981 at `--opt-level=1` - within 1.9x of `clang -O2`'s 1037 for the same six, and against Borland C++ 2.0's 1233. The differential runner compiles every corpus program at both levels *and* with clang, and every pair must agree |
| measuring | `xcc --target=dos16 -c t.c` hands the source to borland-reborn's port of Borland C++ 2.0 and writes its 16-bit OMF object; `cargo run -p bcc20-sys --example compare` prints time and code size for the same files both ways ([ADR-007](docs/ADR-007-measuring-stick.md)). A measuring stick, not a second product: the library is loaded at run time and never linked in |
| explaining | `xcc --explain t.c` lists each C-level instruction under the source line it came from - conversions and decays included, because they are instructions - then the Cranelift function with the virtual registers in it. `Compile::explain` is the same call for a host. |

## What does not work yet

Stated here rather than discovered later.

- **A floating-point argument to a variadic function cannot be passed** - `printf("%f", 1.5)`.
  Win64 passes it in a vector register and says how many in `AL`, a physical register this back end
  cannot write, so the callee would read nothing. Integer, character, string and pointer
  arguments work ([ADR-006](docs/ADR-006-varargs.md), amended).
- **`va_list`, `va_start`, `va_arg`, and defining a variadic function** are refused: the callee's
  side of the same convention, and the same `AL` question.
- **`--run` calls a subset of the C library.** A JIT is not a linker, so it resolves the C
  runtime's own functions itself - `printf`, `malloc`, `qsort`, `fopen` and the rest - from one
  library (the classic runtime, which exports the `printf` family under its own names). What that
  library does not export is absent: `snprintf`, and the `stdin`/`stdout`/`stderr` accessor, work
  in a linked build and not in process. A name that cannot be resolved is a diagnostic naming it.
- **No unwind data.** A Cranelift win64 object carries `.text` and no `.pdata`/`.xdata` (measured,
  and asserted, in `back::tests`; [ADR-003](docs/ADR-003-win64-unwind.md)). Calling compiled C
  works; *unwinding* through it - SEH, a debugger walking frames - does not.
- **`argc`/`argv` are not there**: the runtime calls `int main(void)` (ADR-005 records the three
  imports the alternative needs).
- **`static` is not distinguished from external** yet: every object claims external linkage, which
  is wrong for a `static` and harmless only because nothing reads the flag.
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
cargo test --workspace           # 135 unit, 17 parser, 11 CLI, preprocessing gate, fuzz, exe
cargo test -- --ignored          # the tests that state what is not built yet
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

Requires Rust 1.82 or newer. `tests/corpus/` holds 464 C programs used as a gate; their
provenance is in [tests/corpus/README.md](tests/corpus/README.md).

## Licence

MIT. The corpus is the author's own, granted under the same terms. No code, table or message text
is taken from any other compiler: reference compilers are run as oracles, never linked.
