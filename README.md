# xcc

A small, embeddable C compiler for Windows x86-64, written in Rust. MIT.

`xcc` compiles C to native code in-process: to a COFF object you link, or to a JIT module whose
functions you call as function pointers. It is meant to be embedded in programs that need to run
C they did not ship with - a game's mod scripts, a plugin host, a teaching tool - so there is no
global state: every compilation owns everything it touches, and two can run on two threads.

## What works today

This is M4 of the plan in [PLAN.md](PLAN.md), plus the data and pointer work that M5 needed
first. The status below is what `cargo test --workspace` proves, not what is intended.

```
source -> preprocess -> parse -> cir (verified SSA CFG) -> Cranelift -> COFF object
                                                        -> cranelift-jit -> callable from Rust
```

| | |
|---|---|
| preprocessor | trigraphs, splices, object- and function-like macros, `#`/`##`, variadics, conditionals evaluated in `intmax_t`, includes through a host trait, `#pragma once`, `#line`, the standard macros. 426 corpus programs preprocess token-identical to `clang -E`. |
| C99 front end | scalars, enums, `typedef`, structs/unions with layout and bitfields, declarators including function pointers, all statements including `switch` parsing, initialisers with brace elision and designators, `sizeof`, casts. 394 of 394 corpus programs that are not `#include`d and not deliberately invalid are read completely. |
| code generation | locals and parameters as stack slots, globals and string literals as data (`.bss` for tentative definitions, anonymous `.strN` for literals), arithmetic, comparisons, conversions, casts, direct calls (unit-internal and into host symbols), pointers, pointer arithmetic, subscripting, `if`/`else`, `while`, `do`/`while`, `for`, `break`, `continue`, `return`. |
| embedding | `Jit::new()`, `Jit::add_source`, `Jit::function(name) -> Option<*const u8>`, `Jit::with_symbols` for calls back into the host. Eight threads compiling at once is a test. |
| objects | `Compile::compile` returns an `Artifact` whose bytes are a COFF object with `.text`, data sections and a symbol table; `tests/exe.rs` links one with the host linker and runs it. |

## What does not work yet

Stated here rather than discovered later.

- **`switch` does not lower.** It parses and type-checks; the emitter's comparison chain is not
  finished. `back::jit::tests::switch_arms...` is written, `#[ignore]`d, and holds the six
  run-time assertions it must satisfy.
- **No `crt` and no `.exe` that prints.** There is no entry point, no `argc`/`argv`, no exit path;
  linking is the host linker's job today.
- **No unwind data.** A Cranelift win64 object carries `.text` and no `.pdata`/`.xdata` (measured,
  and asserted, in `back::tests`; [ADR-003](docs/ADR-003-win64-unwind.md)). Calling compiled C
  works; *unwinding* through it - SEH, a debugger walking frames - does not. M5 does not claim it.
- **Refused with one diagnostic each**: `&&`, `||`, `++`/`--`, member access in code, calls
  through function pointers, and records or pointers-to-literals in file-scope initialisers.
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
