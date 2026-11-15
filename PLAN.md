# Plan

Working plan for `xcc`, in the order it should be built. Each milestone is shippable and
has an acceptance test that a machine can check. Sizes are focused effort for one person:
**S** a day or two, **M** a week, **L** two to four weeks, **XL** more, and are estimates, not
promises.

Targets: **Windows x86-64 only.** No other targets are implemented; the middle end keeps its
layouts and calling conventions behind traits so a second target is added to the back end
without touching the front end.

Not in scope, ever, unless this document changes: C++, other languages, an IDE, a debugger UI,
an optimiser of our own, byte-exactness with any existing compiler.

## Where the work is

Having a back end is the small part. Cranelift is the code generator; the cost of this project
is the C front end - lexing, preprocessing, declarators, types, and the parts of C that
everyone forgets (bitfields, aggregates by value, varargs, initialisers, shadowing). Plan
accordingly: milestones 2 to 4 are 70% of the effort of a usable compiler.

## Where the quality comes from

xcc is meant to take the merits of both projects it grew up next to, and to keep its own
licence while doing it. Which merit lands where:

| merit | from | lands in |
|---|---|---|
| compile speed in microseconds a line, a small binary, no dependency beyond the standard library | TCC | the whole design: nothing has to be optimised for the compiler to be usable |
| embedding: compile and call in process, host symbols callable from C, no filesystem unless asked | TCC (`libtcc`) | M4, M7 - with what xcc adds: no global state, so compiles run on any thread |
| a real optimiser: peephole, jump optimisation, cross-jumping (tail merging), register allocation, flow blocks, jump shortening, jump tables | Borland C++ 2.0's back end, as recovered in borland-reborn | M10: our own SSA, passes and x64 emitter |
| verification by reference: the compiler's output is checked against a compiler that shipped, not against hope | borland-reborn's byte-exactness, tinycc_in_rust's oracle loops | M2, M3, M6 differential harnesses |
| C99, the whole standard | both | M3 |

What is not borrowed: code. xcc is MIT and owns every line it ships (see ADR-001, decision 8).

### Cranelift is not the optimiser, and this is the pivot that makes that all right

Cranelift is a code generator built for compile speed. It gives a working compiler, a JIT and
adequate code; it does not give gcc-class output, and no amount of configuration will change
that. The ambition of one day standing in for gcc/clang therefore rests on two things that are
cheap to design in now and expensive to retrofit:

1. **`cir` is ours and is the boundary.** Front end produces `cir`; back ends lower `cir`. No
   Cranelift type may appear in the front end. A back end is a crate plus one match arm.
2. **The optimiser is ours, over `cir`.** SSA construction, mem2reg, folding, DCE, GVN, inlining,
   loop rotation, strength reduction (M10). Cranelift's own passes are not the plan and are not
   relied on; `--opt-level` chooses which lowering runs.

So the first back end is `back-cranelift`: fast, correct, JIT-capable. The second is `back-x64`:
our own register allocator and emitter, fed by our own optimiser, for when code quality is the
thing being asked for. Swapping either in or out must never touch `cparse`, `cir` or the API.


## M0 - decisions  (S)

- [x] Target `win-x64`; Cranelift as the back end; `long double` is `f64`; no C++.
- [x] Embedding is the product, not a side door: no global state, structured diagnostics, JIT.
- [x] `--target=dos16` drives `libbcc20` through FFI, for comparison.
- [x] Recorded in [docs/ADR-001-architecture.md](docs/ADR-001-architecture.md).
- [ ] Name is provisional; rename before the first release.

## M1 - skeleton, diagnostics, API shape  (S)  **done**

Everything after this milestone depends on the API being right, so it comes before the parser.

- [x] Cargo workspace: `xcc` (library) and `xcc-cli` (binary); `#![deny(unsafe_code)]` in the
      library, to be relaxed only in `jit/`, `capi/` and `bcc20-sys/`.
- [x] `struct Compile` (per-compilation state, owned, `Send`), `Options`, `Diagnostic`
      (severity, message, file, span, notes) - text rendering is a separate step from
      producing them.
- [x] Rendering with carets, hand-written: `--format=human|msvc|gcc` (`miette` was considered
      and is not needed - the three formats are ours to print anyway).
- [x] Exact-string tests for the renderers and the source map; a fuzz smoke test that feeds
      random bytes and requires no panic (2 000 inputs x every offset).
- [x] CI on `windows-latest`: `fmt --check`, `clippy -D warnings`, `test`, release build.
- [x] Acceptance: `xcc --version`, a deliberately broken source gives
      `t.c(1,1): error: <message>` and exit code 1, and random input causes no panic.

## M2 - preprocessor  (L)

- [ ] Lexer with byte spans; identifiers, string and char literals with escapes, numbers as
      C99 has them plus `ll`/`ull` suffixes.
- [ ] `#include` (quoted and angled, in-memory includes for embedding), `#define` with
      parameters, `#`/`##`, variadic macros, `#undef`, `#if`/`#ifdef`/`#elif`/`#else`/`#endif`
      with a real constant-expression evaluator, `defined`, `#line`, `#error`,
      `#pragma once`, `__FILE__`, `__LINE__`, `__DATE__`/`__TIME__` (deterministic, from
      `Options`), `#pragma` ignored with a warning.
- [ ] `-I`, `-D`, `-U`, `--depfile`, `<...>` search that never escapes the configured roots
      (embedding hosts compile untrusted scripts).
- [ ] Limits: include depth, macro expansion depth, total source and output size.
- [ ] Acceptance: preprocesses every `.c` and header in borland-reborn's port corpus without
      crashing; token stream matches `clang -E` on the same files except Borland's extensions.

## M3 - parser and types  (XL)

- [ ] Declarations and declarators in full C order (pointers, arrays, functions, mixed to
      arbitrary depth), `typedef`, `struct`/`union`/`enum`, bitfields, qualifiers, storage
      classes, old-style parameter declarations.
- [ ] Statements and expressions with correct precedence, assignment operators, comma,
      conditional, casts, `sizeof`/`alignof`, compound literals, designated initialisers.
- [ ] C99 as the baseline, not a bonus: declarations after statements, `for (int i...)`,
      `inline`, `restrict`, `//`, `long long` and `unsigned long long`, compound literals,
      designated initialisers, variably modified types, `<stdint.h>`-style typedefs, `__func__`.
- [ ] C11 where a host meets it, behind `--std=c11`: `_Static_assert`, `_Generic`,
      `_Alignas`/`_Alignof`/`alignas`, anonymous members.
- [ ] Semantic analysis: scopes and shadowing, type compatibility, implicit conversions,
      lvalue rules, function prototypes against definitions, constant folding, `sizeof` and
      alignment, static initialisers, tentative definitions.
- [ ] Acceptance: the `c-testsuite` (MIT) corpus and the in-tree corpus
      ([tests/corpus](tests/corpus/README.md), 464 programs) parse and type-check with the same
      accept/reject verdict as clang for every file (diagnostic text may differ); no false
      accepts, which is the test that matters.

## M4 - JIT: the embedding milestone  (L)

The first thing worth showing, and the reason the project exists.

- [ ] Typed middle end (`cir`): expressions lowered to a small, target-independent IR that
      keeps source spans, so bad code paths can be reported later. `cir` may name no Cranelift
      type: the back end is behind it, not inside it.
- [ ] `back-cranelift`: one lowering from `cir`; integers `char`..`long long`, pointers, arrays,
      structs, unions, control flow, calls, globals, string literals, `switch`.
- [ ] JIT: `Jit::new()`, `add_source(&str)`, `add_symbol(name, fn_ptr)`, `get_fn::<F>(name)`,
      `remove`, and redefinition of a function by rebuilding the module.
- [ ] `extern "C"` ABI on x86-64 Windows: shadow space, stack alignment, varargs, `__chkstk`.
- [ ] Acceptance: from a string, `int add(int a, int b) { return a + b; }` is compiled and
      called as `extern "C" fn(i32, i32) -> i32` returning 5 for (2,3); a script calling back
      into a host function through `add_symbol` works; 100 compiles from 8 threads pass (no
      global state).
- [ ] Gate before promising a `.exe` (M5): a Cranelift object for win64 carries `.pdata`/unwind
      information, or is proven not to need it for C. This is the one assumption borrowed from
      nothing, and it is checked before it is built on.

## M5 - objects and a real .exe  (M)

- [ ] COFF object writing through `object`; relocations for functions, globals, string
      literals, `.rdata`, `.data`, `.bss`.
- [ ] `xcc-cli -c -o out.obj t.c`; link with `lld-link` or `link.exe` against UCRT.
- [ ] Minimal `crt`: entry point, `GetCommandLineW` to `argc`/`argv`, environment, exit code,
      stack probing.
- [ ] Acceptance: `hello.exe` builds on a clean `windows-latest` runner with only VS Build
      Tools installed, prints, and returns its exit code.

## M6 - enough C to be taken seriously  (XL)

- [ ] Floats and doubles (SSE), conversions, comparisons, constants; `long double` = `f64`,
      documented, `sizeof` 8.
- [ ] Aggregates by value in arguments and returns, struct assignment and initialisation,
      unions with type punning, nested and anonymous members.
- [ ] Bitfields with C's layout rules, packed structs only where the ABI allows.
- [ ] Varargs (`va_list` from `stdarg.h` over the Windows ABI), `printf`-style calls.
- [ ] `long long` arithmetic including division and shifts at the edges (`LLONG_MIN / -1`,
      shifts >= width): defined behaviour, no trap, matching clang's results.
- [ ] Acceptance: differential testing in CI - compile and run a corpus with `xcc` and with
      `clang -O0`, compare stdout and exit codes. The corpus is [tests/corpus](tests/corpus/README.md)
      (which already covers structs, function pointers, doubles, aggregates and `long long`)
      plus cases written for the constructs it does not: varargs, bitfields, unions.

## M7 - embedding polish  (M)

- [ ] C API for hosts that are not Rust (`capi/`): create compiler, add source, add symbol,
      get function pointer, free, last error - the shape `libtcc` users know.
- [ ] Sandbox: no filesystem access unless given a resolver, memory and time budgets per
      compile, recursion limits, `#[forbid(unsafe_code)]` everywhere else.
- [ ] Deterministic output: pinned clock, stable symbol order, reproducible objects.
- [ ] `--explain`: why this instruction, which registers were used - borrowed from
      borland-reborn's port, where it turned out to be the most useful debugging aid.
- [ ] Acceptance: a Python (ctypes) and a Lua (FFI) host each compile a snippet at runtime and
      call it; a compile of hostile input respects its budget and returns an error, not a
      panic or a hang.

## M8 - runtime and headers  (M)

- [ ] Clean-room C headers: `stddef`, `stdarg`, `stdbool`, `stdint`, `stdio`, `stdlib`,
      `string`, `ctype`, `math`, `time`, `limits`, `float`.
- [ ] Decide the libc story: import UCRT (`ucrtbase.dll`, `kernel32.dll`) from our own import
      libraries, so no MSYS/mingw and no SDK headers are needed to compile; linking a final
      `.exe` still uses `lld-link` and the SDK's `.lib` files.
- [ ] `-nostdlib` freestanding mode: no headers, no UCRT, our own entry - for embedding and
      for kernels/firmware-shaped code.
- [ ] Acceptance: `printf`, `malloc`, `fopen`/`fread`, `qsort`, `snprintf` work in a program
      built only with `xcc` + `lld-link` + the SDK import libraries.

## M9 - compare with the DOS compiler  (S)

The reason `--target=dos16` stays. It is a measuring stick, not a product.

- [ ] `crates/bcc20-sys`: bindgen-free hand-written FFI over `libbcc20`'s `bcc20.h`
      (compile a job from memory, get files back).
- [ ] `xcc-cli --target=dos16 -c -ml -O -G t.c` compiles a source with the ported
      Borland compiler, when `libbcc20` is present.
- [ ] A `compare` example that compiles the same files both ways and prints: time, code size,
      diagnostics. This is how PLAN's successor gets its numbers, and how the front end gets
      cross-checked against a 1991 implementation.
- [ ] Acceptance: the comparison runs on the six-file corpus and reports a table.

## M10 - the quality path: our optimiser and our emitter  (XXL, the long arc)

The milestone that makes "one day stand in for gcc/clang" a statement about code rather than
ambition. Nothing here starts before M6 is solid, and nothing here may change the front end:

- [ ] SSA construction over `cir` and `mem2reg`; a verifier and per-pass fixtures.
- [ ] Passes, in the order Borland's own back end had them and this project can measure them:
      constant folding and propagation, DCE, GVN/CSE, jump and branch threading, cross-jumping
      (tail merging, the one borland-reborn documents in detail), loop rotation, strength
      reduction, inlining.
- [ ] `back-x64`: our own instruction selection, register allocation (start linear scan, the
      Borland-style score-based allocator later), jump shortening, jump tables for `switch`.
- [ ] A differential harness that compiles with both lowerings and compares behaviour, so moving
      work between them is safe.
- [ ] `--opt-level` picks the lowering: `0`/`1` Cranelift (compile speed), `2`/`3` ours (code
      quality), with numbers in `docs/COMPARISON.md` style rather than claims.
- [ ] Acceptance: on a named benchmark set, `--opt-level=2` beats `back-cranelift` on runtime and
      stays within a stated factor of `clang -O2`, with every result checked against the same
      program built by clang.

Once `back-x64` is real, Cranelift becomes what it is best at: the fast path and the JIT.

## Layout

```
Cargo.toml                 workspace
crates/xcc/             the library: no CLI, no globals
  src/lex.rs  pp.rs        M2
  src/parse/  ast.rs  sem/ M3 (types.rs, scope.rs, consts.rs, check.rs)
  src/cir/                 M4 (lower.rs, ir.rs, layout.rs)
  src/back/                M4/M5 (cranelift.rs, coff.rs, abi/win64.rs)
  src/jit/                 M4, the only unsafe of the code path
  src/diag.rs              M1
  src/capi.rs              M7
crates/xcc-cli/         M1, M5 (main.rs, args.rs, render.rs)
crates/bcc20-sys/          M9
tests/                     conformance, differential, snapshots
fuzz/                      M1, M3
docs/                      ADR, this plan
```

## Risks, and what to do about them

- **The front end is the project.** If M3 overruns, the compiler is not usable no matter how
  good the back end is. Mitigation: acceptance tests are conformance-based from the start, and
  the C99 syntax list is fixed (nothing gets added mid-flight).
- **C's corner cases are where compilers lose trust**: bitfield layout, aggregates by value,
  varargs, `long long` edges. Mitigation: differential testing against clang in CI from M4 on,
  not at the end.
- **Cranelift's fluid API**: pin the version, keep lowering behind one module, and keep
  `--emit=asm` as an escape hatch so a back end swap is possible.
- **Mistaking Cranelift for an optimiser.** It is a code generator tuned for compile speed. Any
  plan that reaches gcc-class output by configuring Cranelift is wrong; hence M10 owns the
  optimiser, and `cir` keeps Cranelift behind a seam.
- **Licence creep from the neighbours.** borland-reborn's translations are Borland-derived and
  tinycc_in_rust is LGPL: neither contributes code here. Oracles may be *run* (comparing
  against a binary is not deriving from it); corpora must be MIT or public domain. See ADR-001,
  decision 8.
- **Embedding expectations**: hosts want sub-millisecond compiles and no surprises. Mitigation:
  benchmark from M4 (`criterion`), and treat every `panic` path as a bug with a fuzz test.
- **Scope creep into optimisation.** Until M10, Cranelift's default is fine: an optimiser is a
  later milestone with its own acceptance test, and the answer to "make it faster" *this quarter*
  is still "use clang".

## Next slice

M2, in this order, because CONSTRAINTS 4 and 5 demand the decisions before the code:

1. **The stack strategy** (constraint 5): what the recursion budget is, where it is enforced,
   and the number. Written into CONSTRAINTS.md before the preprocessor exists.
2. **The interner** (constraint 4): identifiers get ids in first-seen order, never from
   addresses or hash order, so preprocessing twice gives the same bytes.
3. **The lexer**, then macros, then conditionals, then includes - each with its tests, and the
   `clang -E` differential on `tests/corpus` as the gate at the end.

After M2: M3, M4 (the JIT demo), M5 (`hello.exe`), M6, then M7 to M10 in whatever order users
ask for.
