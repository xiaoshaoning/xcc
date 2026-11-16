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

## M2 - preprocessor  (L)  **done**, except `--depfile` (which moved to M3)

- [x] Translation phases 1 and 2, then tokens with byte spans: identifiers, pp-numbers
      (C99 6.4.8 - `1e+5` and `0x1p-3` stay one token), string and character literals with
      escapes and `L` prefixes, all 55 punctuators longest-first, digraphs, comments,
      trigraphs, escaped newlines, CRLF. Errors: unterminated comment or literal, stray byte.
- [ ] Numbers become integer and floating constants with `u`/`l`/`ll` suffixes (M3: only the
      parser knows what context a number is in).
- [x] `#define` object-like and `#undef`, with redefinition checking; `#error`/`#warning`;
      `#pragma once`; and a diagnostic, not silence, for every directive not built yet.
- [x] `#define` with parameters, `#`/`##`, variadic `...`/`__VA_ARGS__`, argument prescan,
      rescanning after substitution, and the blue-paint rule that stops a macro expanding
      inside itself. Both budgets it needs are enforced and tested: nesting depth, and total
      tokens produced (a doubling macro nests not at all, so depth cannot bound it).
- [x] `#if`/`#ifdef`/`#ifndef`/`#elif`/`#else`/`#endif`, `defined`, and an integer
      constant-expression evaluator in `intmax_t`/`uintmax_t` semantics: C precedence, `?:`,
      short-circuit `&&`/`||` (so `#if 0 && 1/0` is not an error), and division by zero or
      signed overflow diagnosed. A skipped group is not read: no definitions leak, no errors
      fire, and a nested `#if` inside it is not evaluated.
- [x] `#include` quoted and angled through a host-supplied `Includes` (so an embedder's
      includes stay in memory and the compiler opens nothing), `#pragma once`, the include
      depth budget, `__FILE__`/`__LINE__`/`__DATE__`/`__TIME__`/`__STDC__`/`__STDC_VERSION__`/
      `__STDC_HOSTED__` (date and time from `Options::stamp`, never a clock), `#line` for
      `__LINE__`/`__FILE__`, and `-I`/`-D`/`-U` in the CLI.
- [x] The M2 gate (`crates/xcc/tests/pp_gate.rs`): every freestanding corpus program is
      preprocessed and compared **token for token** with `clang -E`, the reference's output
      being lexed by our own lexer so whitespace and line markers cannot hide a difference.
      426 of 426 agree; none needed to be excused. The test proves it can fail (one probe
      file that seeds a known disagreement makes it report a named failure and exit non-zero).
- [ ] `--depfile`, which needs `Artifact` to carry the files that were read - so it lands with
      the parser, when a compile has dependencies worth listing.
- [ ] `-I`, `-D`, `-U`, `--depfile`, `<...>` search that never escapes the configured roots
      (embedding hosts compile untrusted scripts).
- [ ] Limits: include depth, macro expansion depth, total source and output size.
- [ ] Acceptance: preprocesses every `.c` and header in borland-reborn's port corpus without
      crashing; token stream matches `clang -E` on the same files except Borland's extensions.

## M3 - parser and types  (XL)  **done**: M3.1 to M3.4

- [x] **M3.1**, declarations: types (an arena, structural identity, Win64 sizes), scopes and
      shadowing, declarations and declarators (pointers, arrays, functions, all nesting),
      `typedef`, `enum` with constant enumerators, constant expressions (arithmetic, `sizeof` a
      type, enumeration constants, character constants), parameter scopes, and the nesting
      budget. `struct`/`union`, grouped declarators and initialiser lists are refused with
      "not implemented yet (M3.3)" - and a refused `typedef` still declares its name, with the
      error type, so one refusal is one diagnostic and not a cascade down the file.
- [x] **M3.2**, statements and expressions: function bodies with parameter scopes, `if`,
      `while`, `do`, `for` (with its own scope), `return`, `break`/`continue`, blocks and C99
      declarations in them; one expression parser that types every value and folds the constant
      ones (assignment and compound assignment, `?:`, all binary operators with C's precedence
      and usual arithmetic conversions, unary `- + ~ ! * &`, `++`/`--`, casts, `sizeof` a type
      or an expression, subscripting, calls with argument checking, array-to-pointer decay);
      and the errors that follow: assignment to a non-lvalue, mismatched return, argument count
      and type, `break` outside a loop, calling a non-function. The parse gate went from 0 to
      **322 of 464** corpus programs read completely.
- [x] **M3.3**, aggregates and declarators in full: `struct`/`union` with tags, forward
      declarations, anonymous records, members with layout (sizes, alignment, bitfields),
      `.` and `->`; declarators parsed as a tree, so grouping rebinds and `int (*fp)(int)`,
      `int (*a)[3]` and `int (*table[2])(int)` all work; initialiser lists with C's brace
      elision (`int a[2][2] = {1,2,3,4}`), string initialisers that complete an array's size,
      and compound literals. The parse gate went from 322 to **387 of 464** corpus programs.
- [x] **M3.4**, control flow and designators: `switch`/`case`/`default` with the constraints
      C99 6.8.4.2p3 states (integer controlling expression, no duplicate `case`, one `default`,
      `break` legal inside a `switch`), `goto` and labels resolved at the end of the function so
      a forward jump works, and designated initialisers (`[3] = ...`, `.x = ...`) that move the
      cursor and let the list carry on from there (6.7.8p17). The parse gate went from 387 to
      **395 of 464**; every program the corpus can offer is now read, so nothing in it is
      refused for being unbuilt.
- [x] **M3 exit**, a review of every file the earlier passes had not opened (`intern.rs`,
      `lex.rs`, `source.rs`, and again `options.rs`, `symtab.rs`, `pp/eval.rs`, `render.rs`).
      The gate reads **394 of the 394** corpus programs that are not `#include`d and not
      deliberately invalid; the one refusal that changed is `char s[2] = "abc"`, which a
      conforming compiler must refuse and which the list had not named yet.
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

- [x] `cir`'s shape is fixed and its verifier passes on hand-built functions before any lowering
      exists: CFG of blocks with parameters, SSA values minted by the builder, explicit
      `load`/`store`/`slot`, a span and a C type on every value, constants, calls with the callee
      as a value, globals by name. The verifier checks block termination, dominance, branch
      arguments against block parameters, and the types an instruction claims. Decided in
      [ADR-002](docs/ADR-002-intermediate-form.md); `cir` names no Cranelift type.
- [x] `cir::dump`, because the M10 harness compares text, not memory: one line per instruction,
      one block per section, pinned by tests.
- [x] **M4.2a/b**, the parser mints values as it parses: scalars, locals and parameters as
      slots, constants, arithmetic, shifts, bitwise operators, comparisons, assignment, casts,
      `sizeof`, direct calls with C's argument conversions, and the control flow of C -
      `if`/`else`, `while`, `do`/`while`, `for` (the increment in a block of its own), `break`,
      `continue`, `return` - as a CFG of blocks. Refused rather than dropped, each with one
      diagnostic: `&&`, `||` (short-circuit needs block splitting), `++`/`--`, pointer
      arithmetic, subscripts, members, `switch`, `goto`, and a body that mixes them is dropped
      whole. Three unit tests pin the IR text of `add`, of a loop, and of a refusal.
- [ ] `back-cranelift`: one lowering from `cir`; integers `char`..`long long`, pointers, arrays,
      structs, unions, control flow, calls, globals, string literals, `switch`.
- [x] JIT: `Jit::new()`, `Jit::with_symbols` (unsafe: the addresses outlive the JIT),
      `add_source(&str)`, `function(name) -> Option<*const u8>`. Calling one compiled function
      from another works, and so does a C function calling back into Rust.
- [ ] JIT: `remove`, and redefinition of a function by rebuilding the module.
- [ ] `extern "C"` ABI on x86-64 Windows: shadow space, stack alignment, varargs, `__chkstk`.
- [ ] Acceptance: from a string, `int add(int a, int b) { return a + b; }` is compiled and
      called as `extern "C" fn(i32, i32) -> i32` returning 5 for (2,3); a script calling back
      into a host function through `add_symbol` works; 100 compiles from 8 threads pass (no
      global state).
- [x] Gate before promising a `.exe` (M5): **answered - it does not carry it.** A Cranelift win64
      object has `.text` and no `.pdata`/`.xdata` (measured, and now asserted, in
      `back::tests`). M5 therefore promises "a `.exe` that runs and calls C functions", not
      exception throughput; see [ADR-003](docs/ADR-003-win64-unwind.md).

## M5 - objects and a real .exe  (M)

- [x] COFF object writing: `cranelift-object` writes `.text` and the symbol table, with
      relocations for the functions a unit defines and the symbols it imports. `.rdata`,
      `.data` and `.bss` wait for globals and string literals, which the emitter still refuses.
- [x] **An object links into a `.exe` that runs**: `tests/exe.rs` writes the object, links it
      with the host's `cc` (MinGW gcc driving GNU ld here) and runs the result, asserting the
      process exit status is the C `return` value (`int main(void) { return 42; }` -> 42). This
      is the first end-to-end claim of M5 and it is a test, not a paragraph.
- [ ] Globals and string literals, to the design in [ADR-004](docs/ADR-004-data.md): the front
      end fills `cir::Global.init` with bytes it computes from `Types`' layout, a string literal
      is an anonymous `Global`, and the back end declares data symbols and writes the bytes.
- [ ] `xcc-cli -c -o out.obj t.c` and `link.exe` against UCRT (the CLI writes objects, the
      linking is still the host linker's job).
- [x] `switch`: the condition is evaluated once, every arm gets a block, the comparison chain is
      written *after* the body is read (a block may be filled in any order), and the arms are
      linked in source order for C's fall-through. `pick(1)==10`, `pick(2)==0`, `pick(3)==30`,
      `fall(1)==3`, `fall(2)==2`, `fall(9)==9` - run-time assertions, in
      `back::jit::tests::switch_arms_break_default_and_fall_through`. Two bugs found by the
      test: opening a block cannot go through `emit` (which refuses when the current block
      ended), and the code after a `switch` belongs to the exit block or `break` jumps into an
      orphan that the orphan pass turns into a trap.
- [ ] Minimal `crt`, as decided in [ADR-005](docs/ADR-005-crt.md): `mainCRTStartup` written in C
      and compiled by xcc itself, ending the process with UCRT's `exit` (not `ExitProcess`, which
      skips the stdio flush), stack probing by Cranelift's inline strategy so no `__chkstk`
      import is needed, and `int main(void)` only - `argc`/`argv` move to M6 with their three
      imports and Windows' quoting rule recorded there.
- [ ] Acceptance: `hello.exe` builds on a clean `windows-latest` runner with only VS Build
      Tools installed, prints, and returns its exit code.

## M6 - enough C to be taken seriously  (XL)

- [x] Floats and doubles (SSE), conversions, comparisons, constants; `long double` = `f64`,
      documented, `sizeof` 8.
- [x] Aggregates by value in arguments and returns, struct assignment and initialisation,
      unions with type punning, nested members. (C11 anonymous members are still `planned`: they
      are a parse-and-layout question, not an M6 one.)
- [x] Bitfields with C's layout rules - read, written, stepped, initialised; packed structs are
      refused (`#pragma pack`), which is "only where the ABI allows" and this ABI allows none.
- [ ] Varargs (`va_list` from `stdarg.h` over the Windows ABI), `printf`-style calls.
      Blocked on the back end, not on the front end: [ADR-006](docs/ADR-006-varargs.md) records the
      two things Cranelift cannot say (`AL`, and a signature per call site), so this waits for
      M10's own emitter unless the dependency grows the ability first.
- [x] `long long` arithmetic including division and shifts at the edges. Settled by measurement
      rather than by decision: against `clang -O0` on this machine, a shift count at or beyond the
      width masks to the hardware's six bits in both (`(1LL << 70)` is 64 in each), and
      `LLONG_MIN / -1` raises the divide error in both. C leaves it undefined, so "no trap" was
      never reachable: the two agree by doing what the machine does.
- [x] Acceptance: differential testing in CI - compile and run a corpus with `xcc` and with
      `clang -O0`, compare stdout and exit codes: `crates/xcc-cli/tests/differential.rs`, over
      the [tests/corpus](tests/corpus/README.md) programs both compilers can build - 375 of
      464 run and agree, 49 refused and 40 skipped (`#include`, until M8). The cases M6 names but
      the compiler cannot run yet (varargs) are refused, so they wait for that feature rather than
      for the runner.

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
  src/cir/                 M4 (ir.rs blocks and values, build.rs the builder, dump.rs, verify.rs)
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

