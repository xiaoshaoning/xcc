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
- [x] `printf`-style calls with integer, character, string and pointer arguments: the call site's
      arguments *are* its signature, so the call is lowered indirectly with one built from them,
      and the default argument promotions are applied first. A floating-point argument stays
      refused - it needs `AL`, a physical register - and so do `va_list`/`va_start`/`va_arg`, which
      are the callee's side of the same convention ([ADR-006](docs/ADR-006-varargs.md), amended).
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

- [x] C API for hosts that are not Rust (`crates/xcc-capi`): `xcc_new` (with the host symbols it
      may call), `xcc_add_source`, `xcc_function`, `xcc_last_error`, `xcc_free` - five functions,
      an opaque handle, and a `xcc.h`. It is the one crate where `unsafe` lives; a failure is a
      message on the handle rather than a return code to decode, and a panic inside the compiler
      reaches the host as one of those messages instead of an abort.
- [x] Sandbox: no filesystem access unless given a resolver (`Includes` is a trait, and `Jit`
      passes none), memory budgets per compile (`Budgets`: source bytes, tokens, object bytes,
      diagnostics), and recursion limits. This item is what turned three of the four holes up:
      expressions were not counted against the nesting budget (a stack overflow), a macro fanning
      out inside an argument was outside the token budget (an unbounded hang), and the renderer
      echoed whole source lines (39 MB of diagnostics from a 200 KB file). Work is bounded by
      tokens rather than by a clock, because a compilation refused on a slow machine and accepted
      on a fast one breaks determinism - [CONSTRAINTS](docs/CONSTRAINTS.md) 4 and 5 record the
      measurements.
- [x] Deterministic output: pinned clock (`Options::stamp` is the epoch unless a host sets it),
      stable symbol order, reproducible objects - `the_same_source_writes_the_same_bytes` compiles
      one program in two *processes* and compares the objects byte for byte, which is what catches
      an order that came from a hash map or an address.
- [x] `--explain`: why this instruction, which registers were used - borrowed from
      borland-reborn's port, where it turned out to be the most useful debugging aid. Each C-level
      instruction sits under the source line it came from, so a conversion the C rules added
      reads as the instruction it is, and the Cranelift function follows with the virtual
      registers in it. `Compile::explain` is the same pipeline as `Compile::compile`, not a
      second one, and the back end only formats the Cranelift function when it is asked.
- [~] Acceptance: `printf`, `snprintf`, `fprintf`, `malloc`, `free`, `qsort` and
      `fopen`/`fwrite`/`fseek`/`fgetc` work in a program compiled by `xcc` and linked against the
      SDK's libraries - `tests/exe.rs` compares such a program's stdout *and* stderr byte for byte
      with clang's build of the same source. A floating-point argument to `printf` is still
      refused (ADR-006). `tests/host.py` is a Python ctypes host that compiles a snippet at run time,
      calls it, calls a host symbol through it, and reads a diagnostic; `tests/hostile.rs` runs
      seven hostile shapes through the binary and asserts a refusal, not a crash or a hang. The
      Lua half is not done: there is no Lua on this machine, and a script nobody can run is worse
      evidence than none - the ABI is the same for any FFI, and the Python host is the proof of it.

## M8 - runtime and headers  (M)

- [x] Clean-room C headers: `stddef`, `stdarg`, `stdbool`, `stdint`, `stdio`, `stdlib`,
      `string`, `ctype`, `math`, `time`, `limits`, `float`. They ship *inside the compiler*
      (`crates/xcc/include`, `include_str!`), so they need no `-I`, no filesystem and no host
      resolver - `--run` finds them too - and a host's own files are consulted first, so `-I`
      can shadow any of them. `stdarg.h` refuses with a reason instead of declaring functions
      nothing can compile ([ADR-006](docs/ADR-006-varargs.md)). `offsetof` is
      `__builtin_offsetof`: C's own definition needs a pointer-to-integer conversion this
      compiler does not do, and the layout is already in the type table.
- [ ] Decide the libc story: import UCRT (`ucrtbase.dll`, `kernel32.dll`) from our own import
      libraries, so no MSYS/mingw and no SDK headers are needed to compile; linking a final
      `.exe` still uses `lld-link` and the SDK's `.lib` files.
      *Partly there*: a program including our headers and calling `malloc`, `free`, `qsort`,
      `fopen` and `printf` compiles with `xcc -c`, links with `clang`, and behaves as clang's own
      build does; and `--run` resolves those same functions from the machine's C runtime, so an
      in-process program calls the library too (`back::crt`). What is missing is the import
      libraries of our own, so that the link needs no toolchain beyond `lld-link`.
- [ ] `-nostdlib` freestanding mode: no headers, no UCRT, our own entry - for embedding and
      for kernels/firmware-shaped code.
- [ ] Acceptance: `printf`, `malloc`, `fopen`/`fread`, `qsort`, `snprintf` work in a program
      built only with `xcc` + `lld-link` + the SDK import libraries.

## M9 - compare with the DOS compiler  (S)

The reason `--target=dos16` stays. It is a measuring stick, not a product.

- [x] `crates/bcc20-sys`: bindgen-free hand-written FFI over `libbcc20`'s `bcc20.h`
      (compile a job from memory, get files back). The library is **loaded at run time**
      (`BCC20_LIB`, else `libbcc20.dll` on `PATH`), so a machine without the port still builds the
      workspace and a missing library is `None` rather than a link error - which also sidesteps
      linking a mingw DLL from an MSVC-targeted crate. It is not reentrant; the tests take a lock
      ([ADR-007](docs/ADR-007-measuring-stick.md)).
- [x] `xcc --target=dos16 -c t.c` compiles a source with the ported Borland compiler (large model,
      optimised) and writes Borland's own OMF object; `-D`/`-U`/`-I` pass through because BCC spells
      them the same, and `-E`, `--run` and `--explain` are refused because they are xcc's own.
- [x] A `compare` example that compiles the same files both ways and prints: time, code size,
      diagnostics. This is how PLAN's successor gets its numbers, and how the front end gets
      cross-checked against a 1991 implementation: `crates/bcc20-sys/examples/compare.rs`, over
      `crates/bcc20-sys/corpus` - six freestanding C89 files, because Borland's own headers are not
      in this repository.
- [x] Acceptance: the comparison runs on the six-file corpus and reports a table -
      `crates/bcc20-sys/tests/acceptance.rs` asserts both compilers build all six and prints the
      sizes. Measured: **4857 bytes of 64-bit code from xcc against 1233 bytes of 16-bit code from
      Borland** on those six files. Some of that gap is the target and some is that Borland has
      register allocation and xcc has none - which is M10's number to move.

## M10 - the quality path: our optimiser and our emitter  (XXL, the long arc)

The milestone that makes "one day stand in for gcc/clang" a statement about code rather than
ambition. Nothing here starts before M6 is solid, and nothing here may change the front end:

- [x] SSA construction over `cir` and `mem2reg`; a verifier and per-pass fixtures.
      `mem2reg` promotes every slot **whose address never escapes** - and builds block parameters
      where values meet, by the iterated dominance frontier of the store blocks, so a local assigned
      in a loop is a value too. Measured on the six-file corpus: **4857 -> 3281 bytes of x64 code at
      `--opt-level=0`, 1981 at `1`**, against Borland's 1233 - a gap of **1.6x**. Per-pass fixtures
      pin what it proves and what it leaves (an escaping address, a pun through a union, a load with
      no value), and the verifier grew the check that caught its first bugs: every value used must
      be one a block still defines. The back end also lowers blocks in dominator order now, which
      SSA needs and the parser's own numbering never promised - and `cir` computes dominators by
      Cooper-Harvey-Kennedy (one array, not a matrix), because the verifier, `mem2reg` and the back
      end each ask. A function past `Budgets::max_blocks` (4096) is refused: everything downstream
      is polynomial in blocks, and measuring a thousand `if`s at sixteen seconds was the alternative.
- [ ] Passes, in the order Borland's own back end had them and this project can measure them:
      constant folding and propagation, DCE, GVN/CSE, jump and branch threading, cross-jumping
      (tail merging, the one borland-reborn documents in detail), loop rotation, strength
      reduction, inlining.
- [x] `back-x64`, **as a slice**: our own instruction selection, linear-scan register allocation
      over callee-saved registers, `rsp`-relative frames with no `rbp` and no `push`, block
      parameters through frame slots, direct calls, and the COFF writer. `--opt-level=2` selects it
      and Cranelift is not in the path (ADR-008). **300 of the 464 corpus programs** build through
      it, link, run and agree with the same program built by Cranelift's path; the other 164 are
      refused with a span and named (`crates/xcc/tests/x64.rs`). It refuses floating point, indirect
      calls, aggregates by value, globals, more than four arguments and `va_arg`.
      **Code size, measured, before any shrinking: 60 108 bytes across the 302 programs that
      compile, against Cranelift's 11 626 and `clang -O2`'s 9 255 - 5.2x and 6.5x.** Correct and
      large, which is what a first slice is.
- [x] `back-x64` shrinking, round one: **60 108 -> 40 716 bytes (-32%)**, 3.5x Cranelift and 4.4x
      `clang -O2`, with all 302 programs still agreeing. The measurement said where it was: 73.4% of
      the instructions this back end emitted were `mov`. Two changes took a third of them. *Compute
      in place*: an operation whose destination's register already holds an operand emits the
      operation alone, not three moves around it (and `Incoming` canonicalises in the register the
      value lives in rather than in `rax` and then a move). *Coalesce copies*: a value computed from
      another asks the allocator for that value's register and takes it when it is free, so the copy
      never exists; an interval that ends where another begins releases its register first, because
      x86 reads an instruction's operands before it writes the destination. Instructions 11 028 ->
      8 798, `mov` 7 235 -> 4 985.
      **Measured and rejected:** coalescing block parameters was 2.6% of the instruction stream (94
      parameter loads and 192 stores across the whole corpus), so it is not worth the machinery.
- [x] `back-x64` shrinking, round two: **40 716 -> 34 632 bytes (-15%)**, now **3.0x Cranelift and
      3.7x `clang -O2`** from 5.2x and 6.5x at the slice, with all 302 programs agreeing. A leaf
      function has no call to survive, so it allocates **caller-saved** registers and saves none:
      the prologue and epilogue's frame discipline fell from **19.6% of the instruction stream to
      3.7%** (881 saves to 129). The set is chosen by what the function contains, and that is the
      whole design: `rax` is never allocatable (a comparison writes its low byte, a division its
      whole width, the return value goes there), `rcx`/`rdx` only stay out when the function shifts
      or divides, and four registers are lent out when it does neither. **Two corpus programs
      disagreed the first time** - three and four argument functions - because a value moved into
      `r8` destroyed the third argument still sitting in it; caller-saved registers are therefore
      only allocated to functions of two arguments or fewer, which is the count whose registers
      cannot collide, and both shapes are now hand-written tests.
- [x] `back-x64` shrinking, round three: **34 632 -> 30 848 bytes (-11%)**, now **2.7x Cranelift and
      3.3x `clang -O2`** from 5.2x and 6.5x at the slice, 302 programs still agreeing. The round
      began by measuring what spills cost and finding **254 spill instructions in the whole corpus** -
      the guess in the plan ("spills are what the corpus pays for") was wrong, and a pass to split
      intervals would have bought almost nothing. Counting the copies instead, and which encoder path
      emitted each, put the bytes where they were: a comparison's answer was computed in `rax` and
      moved out (317 copies), call arguments were staged through memory even when no two could
      collide, and a function with an empty frame still paid `sub rsp`/`add rsp` (fourteen bytes
      each). All three are gone: `setcc` writes the destination's own low byte, arguments go
      straight to their registers unless the move can destroy one that has not been read, and a
      function with nothing in its frame has no frame.
- [x] `back-x64` shrinking, round four: **30 848 -> 30 564 bytes**, all 302 programs still agreeing.
      Small, and one half of the round was a loss rather than a win, which is worth recording:
      **canonicalisation at observation points was measured and rejected.** The plan's reasoning was
      that 914 corpus instructions widen a 32-bit result that nothing looks at, so moving the
      canonicalisation from the definition to the few uses that read the upper bits (a comparison, a
      division, a right shift) should remove most of them. Built, measured, reverted: **1009
      extensions against 914, 30 940 bytes against 30 848**. Uses outnumber definitions - one value
      feeds several consumers - so canonicalising where the value is made is cheaper than
      canonicalising it wherever it is read, and the front end's converts are semantic rather than
      noise. What the attempt did produce is a sharper statement of where canonical form is needed:
      the conversion's *source* signedness decides a widening extension, the destination's a
      truncation, and both are now written down where the encoder needs them.
      What did land is the other half: **a returned value lives in `rax`** when nothing in its life
      writes that register (a comparison, a division, a call, or a spilled result), so the copy at
      the return does not exist; a register that ends up empty is no longer saved either.
- [x] `back-x64` **globals**: a `.data` section in the object, each object laid out at its type's
      alignment, and `lea reg, [rip + disp32]` with a relocation for its address. **302 -> 383 of the
      464 corpus programs** now compile through this back end, link, run and agree with Cranelift's
      lowering; the 81 left are 13 floating-point programs, 13 that pass more than four arguments, and
      the rest are gaps in the front end or in ADR-004 (a pointer initialised from a string literal),
      not in the back end. A global, a `static` local and an initialised array are hand-written cases
      in `tests/x64.rs` beside the corpus ratchet.
      What is *not* here: a function's address as a value (a call through a pointer), and a `.data`
      section that is separate from a read-only one - string literals sit in `.data`, writable, which
      is correct and not what a linker's `--dynamicbase` would prefer.
- [x] `back-x64` **stack arguments**: the fifth argument onwards goes in the caller's outgoing area,
      above the shadow space, where a callee reads it at `[rsp + frame + 40]` - the forty being the
      shadow space and the return address. The frame's layout is now shadow, outgoing slots, saved
      registers, the lowering's frame, spills, staging, and the widest call in a function decides how
      many slots there are. **383 -> 396 of the 464 corpus programs** compile, link, run and agree.
      **The work found an older bug**, which is the part worth keeping: a leaf function with three or
      more arguments cannot use caller-saved registers (moving the first argument into `r8` would
      destroy a later one), so it takes the callee-saved set - and the allocator only recorded saves
      when a function *called something*, which a leaf does not. It clobbered `rbx`, `rsi`, `rdi` and
      `r12`-`r15` without preserving any, and its caller's values died at the call. The save list is
      now "every callee-saved register touched", which is what the ABI asks for and needs no
      knowledge of whether a call happens; a leaf usually touches none because its set is
      caller-saved.
- [x] `back-x64` **floating point, inside a function**: a second register file (`xmm2`-`xmm5`, with
      `xmm0`/`xmm1` as the encoder's scratch), `addsd`/`subss`/… in both widths, constants through a
      general register, `ucomisd` comparisons whose answer is an `int`, `cvtsi2sd` and its
      counterparts both ways, and `cvtss2sd`/`cvtsd2ss`. **396 -> 406 of the 464 corpus programs**
      compile, link, run and agree. The class of a value - a general register or an XMM one - comes
      from its C type and is carried in `MachineFunction::classes`; the allocator scans each class
      against its own pool and a floated interval can only spill against a float.
      **Not here, and the next round of it:** a `double` parameter, a `double` return, and a float
      argument to a call - the ABI puts those in `xmm0`-`xmm3` *by argument position*, and three
      corpus programs refuse for exactly that. Four encoding faults were found by hand tests before
      the corpus saw any of them: SSE's two-operand form puts the destination in the *register* field
      (the opposite of the integer form); a float destination whose register holds a still-live
      operand needs a scratch, as the integer path already knew; `ucomisd` takes `66` and not `F2`,
      and a wrong prefix is not a wrong answer but an illegal instruction; the two conversions need
      their REX byte or they read `rax`; and `==` needs `setnp`, not `setp`, or every equality is
      quietly false.
- [x] `back-x64` **function pointers and indirect calls**: a function's address is a symbol like any
      other, so `lea reg, [rip + disp32]` with a relocation materialises it; a call through one is
      `call r/m` with the same argument placement as a direct call, in one function used by both. A
      callee whose only reader is the call stays a direct `call rel32`, so a promoted pointer costs
      nothing. **406 -> 411 of the 464 corpus programs** compile, link, run and agree; the refusals are
      now 53, and every one of them is the front end or ADR-004 except three float-ABI programs.
      **`Op::DataAddress` became `Op::SymbolAddress`**, because the address of a function is the same
      instruction and the same relocation as the address of an object, and one name for one thing is
      what keeps a reader from looking for the other. The callee's register survives the argument
      placement for a reason the register plan guarantees - a function with any call allocates only
      callee-saved registers - and that is written where it is relied on.
      **A test whose premise expired was replaced, not kept**: `a_function_address_read_as_a_value_is_refused`
      asserted the gap this round closed. Its two programs are now cases in the constructs table with
      their right answers (1 and 7), and `a_function_address_that_never_escapes_is_still_a_direct_call`
      still holds the other half.
- [x] `back-x64` **floating point across calls**: parameters, returns and float arguments in
      `xmm0`-`xmm3` *by position*, counted separately per class, with a `Target` plan (integer register,
      XMM register, outgoing slot) and a 64-byte staging area for both. **411 -> 414 of the 464 corpus
      programs** compile, link, run and agree; 50 refuse, and every one is the front end's or ADR-004's.
      It took three attempts, and the fault that stopped the first two was **a missing REX byte**: every
      SSE instruction's operand above `xmm7` is only nameable through it, and `ucomisd`, the arithmetic
      operations and the width conversions all emitted their registers without one. That was invisible
      while the pools were `xmm2`-`xmm5` and became a wrong answer the moment a calling function
      allocated `xmm6`-`xmm15` - `d2and.c` read an operand out of `xmm0`, the register holding the
      callee's leftovers. Three faults in this feature have now been the same fault: a REX byte that
      only matters for a high register (the float moves, the conversions, and now the comparisons).
      **The lesson kept**: the third attempt's debug dump - allocation correct, instruction reading a
      register no value occupied - is what turned "somewhere in the float path" into one instruction's
      encoding, so the dump stays in the review log rather than being thrown away.
      The invariant the pools now hold by construction: *a result, its operands and the scratch are
      pairwise distinct* - `xmm0`/`xmm1` scratch, `xmm2` a spilled result's home, `xmm3`-`xmm5` (calls
      nothing) or `xmm6`-`xmm15` (calls) for values.
- [x] `cir` **folding and dead-code elimination**, measured first and then three passes deep:
      `--opt-level=2` over the 414 corpus programs went **118 848 -> 97 432 bytes (-18%)**, from 7.11x
      `clang -O2` to **5.83x**, and from 2.62x Cranelift's size to **2.19x**. The chain that made it so:
      the front end writes an initialiser one byte per `store`, each byte's address being
      `bitcast + sext(index) * sizeof` and then `+ const`; **folding** (1 565 foldable instructions of
      19 289, 1 274 of them `sext` of a constant) collapses the arithmetic to constants, the *back end's*
      new address rule turns `base + k` into a memory operand's displacement, and **dead-code
      elimination** removes the arithmetic nothing reads any more. Each alone was measured and each
      alone *lost*: folding without the address rule was +9% (the constants were emitted and the
      arithmetic stayed), and the address rule without DCE was +1% (the same, one level down). That
      three-pass dependency is the finding worth keeping.
      **And the differential harness caught a semantic bug in the fold**: `unsigned int u = 0 - 2; u >>= 1;`
      answered 0xFFFFFFFF instead of 0x7FFFFFFF, because the front end's constant for `0 - 2` is the
      *signed* -2 in 64 bits and a logical shift of that drags bit 63 into the low word. Folding now
      canonicalises each operand for the operation's own type first, which is the rule the whole pass
      rested on and did not state.
- [x] `back-x64` shrinking, rounds one to five, each measured before it was written. Code size over the
      414 corpus programs: **118 848 -> 96 348 bytes**, from 7.11x `clang -O2` to **5.76x**, and from
      2.62x Cranelift's size to **2.17x**. In order: operands computed in place and copies coalesced
      (-32%), a leaf function saving nothing (-11%, the callee-saved set is for functions that call),
      comparisons answering in their own register, arguments placed directly, an empty frame not
      emitted, the frame's immediates in one byte (-1%), and the largest of all - `cir` folding with an
      address rule and dead-code elimination (-18%, and the three only pay together).
      **Measured and dropped, with the numbers**: jump shortening (405 bytes, 0.3%), block-parameter
      coalescing (2.6% of the stream), interval splitting (125 spill accesses over 419 programs, about
      625 bytes - and this entry used to say "254 spill instructions", a number nobody re-measured until
      a review did), DCE and
      folding *on the old six-file corpus* (zero opportunities each - they mattered only once the whole
      corpus was measured, which is the caution below in its most useful form).
      **A caution for whoever measures next**: a quick grep of `mov` operands counted 10 048 copies where
      the anchored pattern counts 2 760, and the wrong number was briefly believed. A pattern must anchor
      at the end of the line, and a measurement that disagrees with instrumented counting by a factor of
      four is the measurement that is wrong.
- [x] Measured the copies, and the measurement **closed one door**. Tagged by source over the 414
      programs: 2 234 copies, of which ~50% are conversions, 401 argument placement, 351 writes, 191
      returns. The conversions looked like the win - `pointer -> unsigned long long -> pointer` around
      every address - and the pass to drop them is illegal: substituting a same-*width* conversion away
      breaks `cir`'s typing, and the verifier says so ("block 0 converts other types than it says"),
      because a `pointer` and an `unsigned long long` are different `cir` types even when their machine
      form agrees. Dropping only conversions between *identical* types is legal and finds nothing. So
      those copies are the price of a typed IR, and they are only removable by giving the destination and
      the source the same register - an allocator question, whose own measurement is hint *attempts*
      against hint *fires*.
- [x] `back-x64` shrinking: the register-copy question, **measured and closed**. The coalescing hint was
      instrumented at the allocator: over the 414 programs it makes **6 297 attempts, fires on 4 128
      (65.6%), and is blocked on 2 119 (33.7%) because the value it would share a register with is still
      live at that point** - a copy that two live values require, not a missed opportunity - and on 50
      (0.8%) because that value is in memory. The blocked 2 119 are, to within a hundred, the whole of the
      copies the object code still contains, and the rest are the ABI's own moves: an argument arrives in
      `rcx` and lands in `rbx`, a result arrives in `rax`.
      So the allocator is doing what coalescing can do, and what is left of it needs interval *splitting*
      with live-range reordering - a real design change whose ceiling is those 6% - rather than another
      measurement. Written down rather than attempted, because the measurement says the ceiling is small
      and two allocator changes this session were written before their measurement.
      The frame's 12.8% is saves, which is the ABI's price for a value that outlives a call.
- [x] **`va_arg`** - and it was a slice after all, once the shape was seen. `va_list` is a `char *`;
      Win64's home area holds arguments 0-3 at `[rsp + 8 .. +40)` with the stack arguments following at
      `+40`, so *one pointer walks both*; a variadic function's prologue writes its four register
      arguments there, `va_start` is one `lea`, and `va_arg` is a load plus an eight-byte step the front
      end writes back into the list. `cir` gained `VaStart` and `VaArg`, the parser two builtins beside
      `__builtin_offsetof`, and `stdarg.h` is a real header. Cranelift cannot lower either operation - it
      has no home area to point at - so it refuses with a span, which makes this **the first construct
      where level 2 accepts what 0 and 1 refuse**: the corpus ratchet cannot hold such a program, and
      `tests/x64.rs` compares them against clang instead. `va_arg` of a floating-point type is still
      refused, because the vector half of Win64's duplicate needs `AL` and `cir` does not carry the
      count.
- [x] **`cJSON.c` compiles, and its output is byte-identical to clang's.** The three bugs real code found,
      all in constructs the corpus never uses:
      **One symbol, and every address works.** A file-scope initialiser holding an address had been
      limited to a *string literal*: `static struct hooks hooks = { my_malloc, my_free };` - a table of
      callbacks, which is how every C program with a vtable is written - was refused, and so were `&x` and
      an array's name. The `cir`-less paths threw the symbol away instead of keeping it; `Value` now carries
      the symbol whose address it is, and the initialiser writes a relocation. That is the ADR-004 hole
      closed for every address *expression* C allows at file scope, not just the literal case.
      **A `switch` on a narrow type was ill-typed.** C99 6.8.4.2p5 promotes the controlling expression;
      `switch (c)` with `unsigned char c` compared an `unsigned char` with `int` case labels, and `cir` is
      typed. cJSON's number parser is written exactly that way.
      **And the verifier said the wrong thing about it.** The front end's `cir` was never verified before
      the first pass, so a type error the *front end* wrote was reported as *"a pass broke `parse_number`"*
      - and the pass had done nothing wrong. `optimise` now verifies what the front end wrote before it runs
      anything, and the message says what is true (`internal: `parse_number` is not valid: ...`) rather than
      guessing which half of the compiler did it. That cost an hour of bisecting to find out, which is what
      a message that names the wrong layer always costs.
      **Measured: `cJSON.c` (3 206 lines, MIT) compiles, links, and parses/prints JSON with output
      byte-identical to a clang build of the same source.** 443 corpus programs still agree, 20 refused, and
      the refusal oracle is now 19 - the function-address initialiser became a construct that works.
- [x] **Real code, and the two bugs 443 corpus programs could not find.** The audit discipline says a
      milestone ends with a run over code nobody here wrote, and it had not been done since M3: every
      corpus program is this project's own. `cJSON.c` - 3 206 lines, MIT, nobody here wrote it - was
      fetched and compiled. **Two bugs, both in constructs the corpus never uses:**
      **A `\`-continued macro was a syntax error.** C99 5.1.1.2 phase 2 splices backslash-newline, so a
      directive ends at a newline, not at a *line number* - and `directive()` grouped a directive's tokens
      by `span` line, so every continued macro in real C was cut in half. **No corpus file has a
      line-ending backslash** (checked: zero of 464), which is why 443 compiled programs, 415 differential
      programs and a fuzz target never saw it. The lexer now says which side of a *surviving* newline each
      token is on (`PpToken::starts_line`), because only phase 2 knows.
      **`p->x` on an array of structs was refused.** C99 6.3.2.1p3 decays the array before `->` applies,
      and every flexible-array member in real C is written that way. The first fix decayed only for the
      type test and computed the field's address from the array - which the verifier caught by name
      ("loads a value of another type than its address points to"), the second time this milestone that a
      verifier message turned a wrong answer into a sentence.
      **What is left in `cJSON.c` is one blocker**: a file-scope initialiser holding *function* addresses
      (`static internal_hooks hooks = { my_malloc, my_free };`) - the same ADR-004 relocation that works for
      a string literal, one symbolic constant away.
- [x] **The last twenty refusals are asked of an oracle, not assumed.** "Solve the 20 refused" has a
      truthful answer only if the number means something, and it did not: a *count* of refusals hides the
      only thing worth knowing - whether the reason is the program's or the compiler's. `clang -std=c99`
      was run over all twenty: **it refuses every one**, for the same reasons the corpus's own comments
      give - `// Negative: too many array initializers must error`, `<mex.h>` (MATLAB's), and `word` (a
      comment in `cc26_word.c` says *"`word` is a cc_int extension, so gcc cannot be the oracle"*).
      So the count is not a defect list and "solving" it would mean compiling C that no compiler accepts.
      **What was worth doing is pinning the invariant that the count was hiding**:
      `every_program_refused_here_is_refused_by_clang_too` runs both compilers over the corpus and fails on
      a program clang accepts and this does not - printed as *"refusal oracle: 20 refused, 0 of them only
      here"*. A gap now has nowhere to hide in the arithmetic.
- [x] **The C11 row finished: `_Alignof`, `_Alignas`, `_Atomic`.** `--std=c11` had two constructs; the
      matrix promised five. `_Alignof` is `sizeof` for alignment; `_Atomic T` and `_Atomic(T)` declare an
      object of type `T` - the atomicity of the accesses is not implemented, and the matrix says so rather
      than implying it; `_Alignas` raises an object's alignment in the frame and in the data section.
      **Three things had to be true for `_Alignas` to be honest rather than decorative.** The alignment is
      the *object's*, not its type's, so it travels with the slot and the global rather than being inferred
      from the type - `_Alignas(16) int x` is a sixteen-byte object holding an `int`. The frame's body base
      had to be rounded to sixteen, because `saved_float_base` sits after an odd number of eight-byte
      pushes as often as an even one and a sixteen-aligned *offset* from an eight-aligned base is not a
      sixteen-aligned object. And above sixteen is an *extended* alignment, which C11 6.7.5p6 lets an
      implementation decline: `_Alignas(32)` is refused with the number named instead of quietly
      under-aligning.
      **And it found a list that had drifted.** `_Alignas(16) int x;` inside a *function* was read as a call
      to a function called `_Alignas`, because `SPECIFIERS` - the words that start a declaration - decides
      whether a statement is a declaration, and the specifier reader had learned the word without the list.
      The comment above that list claimed *"a test pins it against the specifier reader so the two cannot
      drift apart"*. **There was no such test.** There is now:
      `a_declaration_can_begin_with_every_specifier` runs all twenty-four words at block scope, which is
      the direction that fails when the two drift.
      **Measured: 443 corpus programs agree, 20 refused, 0 wrong, 3.45x `clang -O2`.** The frame rounding
      costs **60 bytes over 444 programs** (79 592 -> 79 652) - measured by stashing the commit and
      rebuilding, because the corpus had also grown by twenty-five programs in between and the two numbers
      were not comparable.
- [x] **The embedding API can ask for what the command line can.** The C API was five functions and no
      options, so a host compiled as C99 at level 0 - and the two things this milestone added, `--std=c11`
      and `--opt-level=2`, were reachable only from the command line. That is a capability that exists and
      cannot be used, which is the same defect as a README that lies.
      One struct, `xcc_options { standard, opt_level }`, and one function, `xcc_new_with` - with
      `xcc_new(..)` still meaning "the defaults", so nothing that worked stopped working. In the library
      the `Jit` now *holds* an [`Options`], which is the front end's own struct rather than a second one for
      the same three questions, and `add_source` uses it: a host embedding this compiler is asking what the
      command line asks. `Jit::with_options(symbols, options)` replaces `with_symbols`, which was the same
      function with a dial fixed at zero.
      **Measured by the tests that now exist**: C11 source compiles through the C API and the same source is
      refused at C99 naming `--std=c11`; a `va_arg` program with floating-point variadic arguments compiles
      at level 2 and is refused at level 0; and level 3 - a level nobody wrote - is refused rather than
      quietly served, which is the rule the whole option set follows.
- [x] **The level-2 JIT: the back end that could not be run, runs.** The gap was measured first - **9
      corpus programs compiled at `--opt-level=2` and could not be run at all**, and `--run
      --opt-level=2` refused outright, so a host calling `Jit::with_opt_level(2)` was told the compiler's
      own code generator could not be JITted. Both halves of that are now false: **`--run` works at every
      level**, and the embedding API gains the back end that makes the smallest code.
      The work was a join, not a code generator: the encoder already produced two sections and a list of
      places naming a symbol, and the object writer handed those places to a linker. `coff::assemble` now
      answers an `Image` - text, functions, data - and has two consumers, `write` for a file and a new
      `x64::load` for memory, which allocates, copies, resolves each relocation against the C runtime and
      the units already loaded, and makes it executable. Nothing in code generation learned that a JIT
      exists.
      **Two faults, both found by running it.** A symbol address was the offset inside the allocation
      rather than the address, so the first `call` went to page zero of the process. And a `REL32` reaches
      two gigabytes, while the allocator put the region sixteen hundred *gigabytes* from the C runtime -
      so `VirtualAlloc` is now given an address hint that walks outwards from the first symbol the unit may
      call, which is what a JIT has to do on an ISA whose calls are relative.
      **Measured: 444 corpus programs run at `--run --opt-level=2`, 20 refused** - the same 20 as the
      object path, all of them not C. `va_arg`, a floating-point variadic argument and a file-scope address
      now run in process, which is what the three constructs had in common: they could be compiled and not
      run.
- [x] **The refusals, finished: 45 -> 20, and the 20 are not C.** After the implicit-declaration work, the
      last solvable group was the four programs that assign an implicit call's `int` result to a pointer -
      `char *p = malloc(10)` with nothing declared. C89 let that through because the two were one width
      where C89 compilers ran, and MSVC still does with C4047, so it is a **warning** now and the
      assignment is made. The leniency is *narrow*: it applies only to a value a call to an implicit
      declaration returned - tracked by the invented function type and the `cir` values those calls
      produced - so an `int` from anywhere else still needs a cast, which a test pins in both directions.
      **Measured: 20 refused, 443 of the 464 corpus programs compile, link, run and agree.** The 20 that
      remain are 6 that need MATLAB's `<mex.h>`, 11 that are not C at all (an array or an rvalue assigned
      to, `&` of an rvalue, a wrong argument count, `<<` on a `double`), and 3 that want a file-scope
      initialiser evaluated by a startup prologue - a feature every one of them is also refused by `clang`
      for, checked program by program rather than assumed.
- [x] **The 45 refusals, sorted into "not ours" and "ours", and 21 of them solved.** The corpus gate
      counted 45 refused programs; a script ran `clang -std=c99` over every one of them and found that
      clang refuses 42 - so the number was never a measure of the compiler. What it hid was two places
      where this compiler was *stricter than the platform*:
      **A call to a function nothing declared** was an error. C99 6.5.2.2p1 makes it a constraint
      violation, and MSVC's C4013, GCC's `-std=gnu89` and every 1990s program on Windows take the callee
      to be a function returning `int`. It is now the same: **a warning**, and the call site's arguments
      become the signature - which is the *existing* variadic lowering (ADR-006), so the feature cost no
      new mechanism at all. **24 corpus programs** compiled by that alone.
      **Excess array initialisers and a too-long string for its array** were errors. C99 6.7.8p2 and
      6.7.8p14 make them constraint violations too, and every compiler reports and carries on: they are
      warnings now, and what fits is what is written. That needed a second fix underneath: `char s[2] =
      "ab"` is *valid* C - 6.7.8p14 drops the terminating NUL when there is no room - and it was being
      refused. **3 more programs.**
      **Measured: 45 -> 24 refusals, 419 -> 439 corpus programs that compile, link, run and agree.**
      What remains is 6 that need MATLAB's header, 11 that are not C at all (assigning to an array or an
      rvalue, `&` of an rvalue, a wrong argument count, `<<` on a `double`), 3 that want a file-scope
      initialiser evaluated at startup (a feature, not a diagnostic), and 4 that want C89's habit of
      treating an implicit `int` result as a pointer.
      **And one program had to be named as uncomparable**: `pp_pptr` prints `%p` of a local, and a stack
      address is a property of the run rather than of the source - two builds disagree in exactly the way
      the comparison looks for. It is excluded by name, with the reason, because "the test cannot see
      this" and "this disagrees" must never look alike.
- [x] **`--std=c11`, and the two constructs the matrix had been promising behind it.** The flag did not
      exist, so every "planned behind `--std=c11`" row was unreachable - a support matrix is a promise,
      and this one had a promise with no mechanism for four milestones. `--std=c99` (the default) and
      `--std=c11` now exist, `__STDC_VERSION__` follows them (`199901L` / `201112L`), and the dialect
      travels with the tokens so the parser can answer for it.
      `_Static_assert` is a declaration that declares nothing - checked where it is asked, reported with
      the source's own message - and `_Generic` reads every arm for its type and its syntax, then reads
      the winner again for real: two passes over a token span rather than one pass with a deferred
      decision, because the parser is a function over a token slice and a rewind is cheaper than a way to
      evaluate an expression later. The arms that lose are read with the same `unevaluated` switch
      `sizeof` uses, so they emit nothing - which is what C asks for and what makes
      `_Generic(x, int: 1, char: x = 2)` legal.
      **And it needed a front-end fix underneath.** `_Generic(l, int: 1, default: 9)` answered 1 for a
      `long`: the specifier path chose a type from `(signed, bits)`, so a declared `long` *was* the `int`
      type object - the LLP64 trap this project recorded in M8, this time in the type system rather than
      in a header. C99 6.2.5p4 says `int` and `long` are two types even when the target makes them the same
      width; the spelling now picks the type, and `Types::identical` asks the question `_Generic` actually
      asks (identity, recursively through pointers and arrays) rather than the structural `same` that
      answered it wrongly.
- [ ] **Branch threading, measured and dropped: 136 bytes of 59 660 (0.2%).** The count that suggested it
      was real: **415 of the corpus's 2 203 `cir` blocks are "hallways"** - no instructions, no
      parameters, and a jump for a terminator, so a jump to one is a jump to somewhere else. Threading
      them is sound (the parameter half of the condition is what makes it so) and the pass found all 415;
      it saved **136 bytes**, because the encoder already emits *no* jump when the target is the next block
      and the layout puts a hallway immediately before its destination almost every time. The upper bound
      had assumed a `jmp` per hallway; the fall-through rule had already collected them.
      **Cross-jumping, closed by the same measurement**: 7 blocks in the whole corpus duplicate another
      block. That is not a pass, it is a rounding error, and it does not go on the list.
      Both numbers are here rather than in a deleted comment, because "we thought about it" without a
      number is what this list is for.
- [x] **An address is made once per block.** Eighth measurement, and the first one whose *prediction* was
      wrong in an instructive way. Counting address materialisations in the encoder: **584, with 292 of them
      repeats of an address the same function had already made (50%)**, and nothing can invalidate an
      address - a symbol's is fixed, a frame slot's is fixed, what changes is the bytes *at* it. So the
      lowering keeps a map and aliases the second `cir` value to the first register, like the free
      widening.
      **Measured: -296 bytes (0.5%)**, against a prediction of ~2 044. The histogram said why: the address
      ops fell by ~1 400 bytes and the `store` and `load` buckets *grew* by 1 200, because a register
      shared across a whole function lives from the first mention to the last and the allocator spilled
      the difference. The reuse was paying for itself in pressure.
      **So the map is cleared per block** - a block is a straight line, so a repeated address there is
      close to free, and it is the common case: `p->a` and `p->b` off one base, a global read twice in a
      row. Measured again: **60 556 -> 59 660 (-1.5%)**, 3.52x -> **3.46x `clang -O2`**. The lesson is the
      one this back end keeps teaching: an optimisation's *scope* is as much a measurement as its
      mechanism, and the first version of this one was right about the mechanism and wrong about the
      scope.
- [x] **An address in a file-scope initialiser.** The last hole in ADR-004, and the last corpus program
      refused for a compiler gap: `char *p = "hello"` and `char *names[] = {"alpha", "beta"}` - the pointer
      and the table that every C program has, and the one thing the data format could not say. An address
      is eight bytes the *linker* fills, so a `cir::Global` now carries relocations beside its bytes and
      the COFF writer turns each into `IMAGE_REL_AMD64_ADDR64`. Levels 0 and 1 refuse it with the span,
      because `cranelift-object` cannot name a data symbol the way its relocation type wants - the third
      deliberate asymmetry, recorded in ADR-004 with the two others.
      **It needed a second fix that the string table found**: `int a[] = {1,2,3}` at file scope was refused
      too, and had been all along. An object with no size yet learns its length from the brace list, but
      the byte writer refused to grow past a buffer of zero bytes and abandoned the whole initialiser -
      `self.bytes = None`, and the caller's "cannot evaluate" - so a file-scope array whose length comes
      from its initialiser was a gap nobody had looked at. The writer grows the buffer only when the
      object's size is *unknown*; a complete object may not grow, which is the error the brace walker
      already reports.
      **Measured 418 -> 419 corpus programs compile and agree at `--opt-level=2`, 45 refused** - and every
      one of those 45 is a program `clang -std=c99` refuses too, or one that needs a header this machine
      does not have. That is the milestone worth recording: the corpus no longer contains a program this
      compiler cannot compile for a reason of its own.
- [x] **The argument staging area was for a case that cannot happen.** Seventh measurement: with the
      frame's operations accounted for, the call path was next. Instrumented per call, **no call in the
      corpus collides in either register file** - an argument's value is never in a register another
      position needs - so every call was paying for a staging area it did not use: four stores and four
      loads, in a 64-byte frame reservation, on the *chance* of a collision.
      The chance is nil by construction, which is the part worth writing down: `Registers` gives the
      **caller-saved** set - which contains `r8` and `r9`, two of the four integer argument registers - to
      functions that call **nothing**, and a function that calls nothing places no arguments. So the
      staging path, its second pass and its 64 bytes are gone, replaced by a `debug_assert` that names the
      invariant so a future pool which allows an argument register says so instead of quietly clobbering
      one.
      **Measured 60 964 -> 60 476 (-0.8%)** - small, and smaller than the 4 KB I estimated, because the
      estimate counted *calls with register arguments* rather than *calls that stage*; all-integer calls
      already took the direct path. The commits that pay for themselves in size are not always the ones
      that pay for themselves in clarity, and this one is the second kind.
      **And it exposed two layout faults**, which is the real story: removing the padding made
      `cc32_byval` crash. The total the prologue moved was `max(body, pushes)` where it had to be
      `body + pushes` - the pushed registers sit *above* the body, so taking the larger of the two put a
      function's locals on top of its saved registers as soon as the body outgrew them. The 64 bytes of
      staging had been hiding that since the pushes landed, in the previous commit. One wrong answer
      (`va_start` pointing into the frame) was found by the last review; this one was found by a corpus
      program that copies a struct into a local.
- [x] **A callee-saved register is pushed, not stored.** Sixth measurement, and the first on the frame
      rather than on an operation: the prologue and epilogue were 12.2% of the bytes between them, and
      **351 integer saves** across 514 functions were costing ten bytes each - `mov [rsp+disp8], r64` to
      keep it and the same to restore. `push r64` and `pop r64` are one byte each. Measured
      **64 156 -> 60 964 (-5.0%)**, 3.73x -> **3.54x `clang -O2`**.
      What makes it safe is arithmetic rather than care: the pushes go *outside* the frame, so the two
      of them together move `rsp` by the same total that the frame alone used to, and **every
      `rsp`-relative offset in the function is unchanged**. The total is rounded up from whichever is
      larger - the frame's own body or the pushes - so the alignment rule (`rsp` 16-byte aligned inside
      the function) still holds and the subtraction is never negative; a function with seven saved
      registers and nothing else in its frame is the case that made `max` necessary.
      The registers are restored by pop order, so nothing ever addresses them - which is why the
      `saved_base` slot they used to own is gone from the encoder.
- [x] **A widening conversion is free, and 96% of conversions are widenings.** Fifth measurement, and the
      first one that cost nothing to act on: `canonical` was 6.5% of the bytes, so a breakdown of those
      operations by width was next. **4 239 of 4 427 canonicalise to sixty-four bits** - and a
      canonicalisation to sixty-four bits is a no-op by construction, because a value in a register is
      already canonical for its own C type and every type of size eight *is* its own sixty-four bits.
      `(long)x` is `x`: sign-extended for an `int`, zero-extended for an `unsigned`, and neither needs an
      instruction here because the register already holds that form. So the lowering *aliases* the two
      `cir` values to one machine value instead of emitting an instruction that copies a register to
      itself, and narrowing is the only conversion left with work to do.
      **Measured 67 868 -> 64 156 (-5.5%)**, 3.94x -> **3.73x `clang -O2`**; 418 programs still agree.
      The saving is larger than the bytes the instruction printed (3 801), because a no-op operation still
      owns an interval: the two values had to be given registers and sometimes paid a `mov` to be made
      equal. Saying they are one value removes the question.
      **The risk is the one this back end keeps meeting** - a `cir` value with no definition is a wrong
      answer waiting for a reader, which is what the branch-fusion review found. Here the definition is
      not removed: both names point at the *same* `VReg`, whose definition is the source's. The corpus
      ratchet and the construct tests are what say so.
- [x] **The truth test of a truth value is the value.** Fourth measurement: `binary` led at 18.5%, but the
      question behind `compare`'s 13.1% was bigger. Counting the corpus's `cir`: **530 of 1 177 comparisons
      (45%) are `cmp ne (cmp ...), 0`** - the front end's literal truth test, because `if (a < b)` asks for
      the comparison and *then* asks whether its answer is non-zero. That wrapper is not free: it hides the
      comparison from the branch that could have carried it, so the comparison is materialised (`cmp`,
      `setcc`, a widening to 64 bits) and then tested again, where the fused form is `cmp` and a `jcc`.
      A new pass - `cir::pass::truth`, between `mem2reg` and `fold` - substitutes the comparison for its
      wrapper and removes it. It is a *substitution*, not a fold, which is why it is its own pass: folding
      turns instructions into constants, and that pass's module doc says so.
      **Measured 73 432 -> 67 868 (-7.6%)**, 4.27x -> **3.94x `clang -O2`**, and the ratio has passed a
      milestone of its own: the back end is under four times clang's -O2 for the first time. 418 programs
      still agree. Both lowerings gain - the wrapper was an `icmp` and a `brif` in Cranelift too.
      **Only `!= 0`.** `(a < b) == 0` is the *negation*: substituting would answer a flipped branch the
      wrong way round, so it is left alone, with a test that pins it (`!(a < b)` keeps both comparisons).
      That is a decision a future change can revisit - the branch can turn its own condition over - but it
      is not this substitution and the code says which it is.
- [x] **A constant travels inside the instruction that reads it.** Third measurement, same method: after
      the fusion, `const` was **31.5%** of the bytes (22 684) and `compare` had fallen from 18.2% to 11.0%,
      so the constants were the whole story. The reader table said where they go: **binary 31.5%, compare
      30.7%, store 18.0%, return 10.8%**, and **96% of constants have exactly one reader**.
      So the machine form now says what it always meant: an operand is **`Operand::Value(vreg)` or
      `Operand::Immediate(u64)`**. The lowering carries a constant inside `add`, `sub`, `and`, `or`, `xor`
      and any comparison when it has one reader, is an integer, and fits a signed 32-bit immediate; the
      `Const` instruction is then *not lowered at all*, so the value has no register and no `mov`. A
      comparison against zero needs no constant either - `test x, x` sets the same flags for every
      condition this back end emits, in three bytes.
      **Measured 80 936 -> 73 432 (-9.3%)**, 4.70x -> **4.27x `clang -O2`**; 418 programs still agree.
      The type change is the point of the exercise, not an accident: with `right: VReg` the constant had to
      exist as a value, and a register nothing defines is a wrong answer waiting for a reader - which is
      exactly the fault the review found in the branch fusion. `Operand` makes the compiler ask at every
      use site whether it handled the constant.
      **Left out, and said so in the code**: `mul` and the shifts (their encodings take a register, and
      `imul`'s three-operand form is a different family), a constant on the *left* of a non-commutative
      operation (x86 puts the immediate field on the right, and swapping is a separate decision worth its
      own measurement), and stores, which would need the immediate-store form.
- [x] **A comparison that only a branch reads never becomes a value.** Re-measured after the constant
      change: `const` still led at 28.9%, then `compare` at **18.2%** and `canonical` at 5.0% - and those
      two are one decision. A comparison materialised `cmp`, `setcc`, a widening to 64 bits, and then the
      branch tested it against zero: about eleven bytes to answer a question the branch had to ask
      anyway. Counting the readers of every comparison: **1 306 comparisons, 593 of them (45.4%) read by
      nothing but a branch.** Those now become `BlockTerm::BranchCompare`, and the branch emits `cmp` and
      a `jcc` - eight bytes of `setcc`, widening and test gone, and a value that no longer exists.
      **Measured 87 768 -> 80 936 (-7.8%)**, 5.10x -> **4.70x `clang -O2`**. The estimate was 4 150 bytes
      and the result 6 832, because the fused branch also stops materialising the constant the
      comparison compared against.
      Three things made it cheap enough to be worth it, and all three were already there: `cir` values are
      defined once, so a comparison's operands cannot change between it and the branch; the lowering
      already built a read-count map for direct calls, so "read exactly once" cost one field; and a value
      with no definition needs no interval, which `regalloc` already assumed for an unused function
      address. Floating-point comparisons are left alone - `ucomisd` and the unordered cases are settled
      by different condition codes than `setcc` and this branch code use, and 45% of the integer ones is
      where the money is.
- [x] **The cheapest form of a constant, which was 37.7% of the code.** Measured first: a histogram of
      the emitted bytes by operation kind over the 418 corpus programs put `const` at **37.7%** - twice
      anything else, ahead of `compare` (16.0%), `binary` (8.2%), `store` (6.9%) and the prologue and
      epilogue together (8.0%). The reason is the fold pass's own success: folding initialisers and their
      address arithmetic makes constants, and each constant was materialised as `mov r64, imm32` (seven
      bytes) however small it was. A second histogram of the *values*: 4 790 constants, **29.5% of them
      zero** and **67.9% between one and 255**, 1.9% fitting `u32`, 0.7% signed 32-bit, 10 wide.
      So two forms: `xor r32, r32` for zero (two bytes, sets all sixty-four) and `mov r32, imm32` for
      anything non-negative that fits `u32` (five bytes instead of seven). Both are *exactly* the value,
      never a truncation, because a value arrives canonical for its C type and a non-negative constant's
      canonical form **is** its zero-extension - which is the invariant the whole encoder rests on, used
      here to save bytes instead of to explain a load. Predicted **13 755 bytes**; measured
      **98 776 -> 87 768 (-11.1%)**, 5.74x -> **5.10x `clang -O2`**. The gap between prediction and
      measurement is the REX prefix on `r8`-`r15`: three of the four short forms are one byte longer there.
      **And the first attempt was a wrong answer that ran**: in `31 /r` the register sits in the *modrm*
      field, so `xor r8d, r8d` needs the REX bit for it twice, and naming the extended register through
      `B` alone emits `xor r8d, eax` - which assembles, executes, and computes something else. It cost four
      minutes and one segfaulting array program; the construct test's `a[4]` and the corpus ratchet found
      it, exactly as they found the seven faults of the first slice.
- [x] **`printf("%f")`: a floating-point argument *to* a variadic function.** The last row of ADR-006,
      and the premise behind its refusal was wrong. Win64 asks the caller to write each variadic
      floating-point argument to a vector register **and** to the general register of its position, and
      to say how many vector registers it used in `AL`. The duplicate is what lets one
      eight-byte-per-argument walk read a list that mixes the classes - the same walk `va_arg` already
      did - and a callee told *zero* reads the general registers, where the copies are. `clang` does
      exactly that and leaves `AL` unspecified at such a call; writing zero is the same answer made
      deterministic. So this is not "the ABI we cannot express": it is one extra load from a staging
      slot the caller had already written, plus two bytes.
      **Where it is**: `--opt-level=2` only, because Cranelift's lowerer writes the vector register
      alone and would print zeros - it now refuses with the span that asked, at the call, which is the
      second construct where the two lowerings differ on purpose and the first where the difference is a
      call rather than a definition. **Measured**: 414 -> **418 of 464 corpus programs** compile and agree
      through our back end (the four: `dmath`, `dmore`, `dreg_denseprint`, `dsmoke`), and the acceptance
      test's `printf("%f|%.2f|%e|%g|%d|%f")` is byte-identical to clang's output from the real C runtime.
      **And the review of the accounting found a hole worth more than the feature**: the corpus ratchet
      counted a program as "refused" when *either* lowering refused, so four programs we now compile were
      reported as gaps we had not closed - and, once that was fixed, as "compiled only here" and compared
      with *nothing*. They are now compared with `clang` where Cranelift refuses. A ratchet that counts a
      program and never looks at what it answers is a coverage hole wearing a number.
- [x] **`va_arg` of a `double`**, and the duplicate is what makes it easy: Win64 passes a variadic
      `double` in a vector register *and* a general one, so a callee walking the general slots - which is
      what our `va_list` is - finds it with no `AL` and no vector save area. Verified against a
      **clang-built caller**, which is the only way: a test that compiles both sides itself cannot see a
      convention they share.
      **That test found two real ABI bugs in what we had already shipped.** First, the argument register
      is chosen by the argument's *position*, not by a per-class counter: `f(int, double, double)` has its
      doubles in `xmm1` and `xmm2`, and we were putting them in `xmm0` and `xmm1`. Second - found by the
      same test after the first fix - a callee whose fourth parameter arrives in `xmm3` could not keep a
      value in `xmm3`, because the float pool's caller-saved set starts there: moving the first parameter
      into it destroyed the fourth before anything read it. Both are invisible to any test where one
      compiler builds both sides, which every test we had did.
- [ ] **`va_arg`, the older plan, kept for what it got wrong:** It is not a back-end feature at all:
      `stdarg.h` is a stub that `#error`s, the front end has no builtin for `va_start`/`va_arg`, and
      `cir` has no operation for either. The work, in order: a `__builtin_va_list` type (`char *` on
      Win64) and the three builtins; the parser's special case for `__builtin_va_arg(ap, T)`, whose
      second argument is a *type* and not an expression; a `cir` operation that says "the address of
      the first unnamed argument" and one that reads and steps; and on this back end the variadic
      prologue - the four register arguments and the float ones written into the caller's 32-byte home
      space, which is already below the stack arguments, so one pointer walks both.
      **The structural consequence to decide first**: Cranelift cannot express a variadic definition at
      all, so levels 0 and 1 will refuse what level 2 accepts. The corpus ratchet compares the two
      lowerings and would call that a mismatch; a variadic program therefore needs the *clang*
      comparison in `tests/x64.rs` instead, and ADR-006 needs a sentence saying the two lowerings
      deliberately differ in what they accept.
- [x] **`.pdata` / `.xdata`: the frame table.** Done: the codes are recorded as the prologue emits them,
      `.pdata` is written with `ADDR32NB` relocations, and the JIT registers the table with
      `RtlAddFunctionTable`. A `longjmp` out of three compiled frames returns 42 at both levels and under
      `--run`; Lua's 34 files compile and link, and the interpreter runs.
- [x] **A `//` comment lost the newline that ends its line.** The lexer consumed the newline and never set
      `starts_line`, so the line after any `//` comment was invisible to directives - and a directive reads until a
      token that starts a logical line, so `#define A 1 // c` took the rest of the file as the macro body. Found by
      `stb_image.h`, which was then made to compile (and decode a PNG byte-identically to `clang` with its own
      `STBI_NO_SIMD`). The same file found a second bug beside it: a struct initialiser could not end with a comma.
- [x] **`const` was consumed and dropped.** `Type::Const(inner)` with a peeling `kind()`, so the qualifier reaches a
      typedef, a member, a pointer's target and an array element without a parallel fact anywhere; two write sites
      (`assign`, which every compound assignment routes through, and `step`) and one rule: a modifiable lvalue is not
      `const`-qualified (C99 6.3.2.1p1). A parameter keeps its qualifier as an object and loses it in the function type
      (6.7.5.3p15) - two facts, and the sweep caught the first version losing the first while doing the second.
      Duplicate member names too (6.7.2.1p1), and `tests/invalid.rs` holds seventeen forbidden programs measured
      against `clang`, so the next one is found by the suite.
- [ ] **`&(f)` of a parenthesised function designator is refused.** `&g` is the designator (C99 6.5.3.2p3) and
      `&(g)` is the same designator, but the shortcut reads the token *in front of* the operand, and a `(`
      there hides it. The operand cannot answer instead: by the time a function name is a value it has decayed
      to a pointer to function, which is the type a call returning one also has. A fix has to look through the
      parentheses by counting them, and that rule is only worth writing when something needs it - `&f` is the
      idiom, and lz4, Lua, miniz, cJSON and stb_image all write that.
- [ ] **A nameless struct member is accepted.** `struct H { void * ( __cdecl * ) ( unsigned long long ) ; };`
      compiles silently; C99 6.7.2.1p1 wants a declarator with an identifier. Found while bisecting the
      eleven-gigabyte member loop, which is fixed - this is the diagnostic that was missing beside it.
- [ ] **`windows.h`.** sqlite3.c now takes its Windows path and asks for the Win32 SDK. That is a boundary, not a
      bug: this compiler ships C99's headers, and the SDK is a different library of declarations. Crossing it
      would mean vendoring the SDK, which is a product decision and not a next step.
- [x] **`wchar.h`.** Closed: fifty-seven of its seventy-five C99 names were already exported, and the eighteen that
      were not are the runtime source's - the twelve wide `stdio` front ends over `__stdio_common_vfwprintf` and its
      three siblings, four `wmem*`, `wmemset` and `mbsinit`. `fwide` is the platform's stub: this runtime tracks no
      stream orientation and mingw's own header answers with the mode it is asked for, which the header says where it
      is declared. `mbstate_t` is the UCRT's layout, because the library writes through the pointer. Wide I/O measured
      identical to a `clang` build. Four of the five headers that used to refuse now ship; `complex.h` and `tgmath.h`
      are left, and `complex.h` needs `_Complex` in the type system rather than a header.
- [x] **The real code is a fixture, not a claim.** cJSON and miniz are vendored unmodified under
      `crates/xcc/tests/fixtures`, with their upstream licence notices and a README that says what each one
      found and how the expected output was recorded. `tests/real.rs` compiles both with this compiler's own
      back end, links with its own linker, runs them, and compares stdout with a `clang` build's - 4.3 seconds
      for the pair, which is cheap for the only test that catches what small programs cannot.
- [x] **miniz, all of it.** `miniz_zip.c` compiles with `-DMINIZ_NO_STDIO` (its `<sys/stat.h>` is POSIX and this
      compiler has no POSIX headers; the library's own switch turns the file APIs off) and a driver writes a zip
      archive in memory and extracts the file back byte-identical. Building it found the last `mem2reg` hole:
      `int *p = c ? &a : &b;` threads those addresses through the join as block arguments, and the escape analysis
      read instructions and never terminators - so a slot was promoted, its `Slot` deleted, and the argument named a
      value nothing defined. The check now asks `term_operands`, the same list the verifier and both back ends use.
- [x] **A frame slot is not an instruction.** Closed, and the blocker was not the slot. A slot's `Slot` instruction
      was emitted where the declaration stood, so a `switch` whose `case` labels sit inside nested `do { } while (0)`
      blocks (miniz's inflater) read a slot from a block that did not dominate it, and the verifier refused the
      function. The slot goes to the entry block now. What stood in the way was `liveness`: the backwards fixpoint
      kept its live sets in `Vec<VReg>` and compared them with `contains`, which is quadratic in the number of values
      - 2 992 of them in `tinfl_decompress`, and the function never finished (over 30 minutes, measured; `lower` had
      taken 9 ms and every other function's allocation microseconds). With `BTreeSet<VReg>` the file compiles in
      1 990 ms, 82 ms of it the back end, and miniz round-trips a 209 793-byte file through its own deflate and
      inflate, byte-identical, with zlib agreeing on the stream.
- [x] **A real program, built and run by xcc alone.** cJSON - 3 206 lines of MIT C - and a driver over it,
      compiled and linked with no host tool: `xcc jd.c cJSON.c -o jd.exe` parses a JSON file, walks it, round-trips
      it, and prints byte-identical output to the same sources built by `clang` (13 lines on one input, 1 328 bytes on
      another, and the same `not JSON` answer on a broken one). The driver is what found the next bug.
- [x] **Every declared name links.** Measured against `ucrtbase.dll`'s export table: ten of the 178 names
      the headers declare had no home - `scanf`/`fscanf`/`sscanf` (the scanning front ends are the program's
      half of the UCRT, like `printf`'s, so the runtime carries them over `__stdio_common_vfscanf`),
      `atexit` (a wrapper over `_crt_atexit`), `_setjmp` (the library exports `__intrinsic_setjmp`), and
      `time`/`difftime`/`gmtime`/`localtime`/`mktime` (the target's 64-bit `time_t` names them `_time64` and
      friends). The rename lives in one table, and the measurement is a gate: a test asks the library for
      every name, so a name that drifts fails there rather than as `ERROR_PROC_NOT_FOUND` in a user's program.
      The gate was first written with `contains(name)`, which passes for `time` because `runtime` contains it -
      a gate satisfied by prose is not a gate.
- [x] **A linked program is a command-line tool.** `argc`/`argv` come from the runtime the link
      compiles in - `_configure_narrow_argv` plus the two accessors, the same three imports `clang`'s
      startup uses - so the entry point is three lines and no quoting rule of ours. Measured first:
      a real tool (argv, `fopen`, `fread`, `malloc`, `printf`) linked cleanly and segfaulted, because
      the runtime called `main()` with nothing and the program read register garbage. `int main(void)`
      is unaffected; narrow arguments are code-page limited, which the ADR now says.
- [x] **Lua runs.** Closed: two C bugs stood in the way - a bare `return;` in a `void` function emitted
      nothing, and an initialiser was parsed as a conditional-expression instead of an assignment-expression.
      Every one of Lua's 33 objects is now correct alone and the interpreter's output matches a clang build's.
- [x] **The linker.** Closed: `back/x64/pe.rs` writes a PE32+ image that Windows loads, binds and runs, and
      `xcc hello.c` writes `hello.exe` with no host tool. The blocker was not the loader: **the import address
      table must start out as a copy of the import lookup table.** With an all-zero IAT the loader binds nothing
      and reads no imports at all - the first call through a thunk jumps to zero - and no header field in the
      image changes that (dll characteristics, the exception table, the IAT directory, the import directory's
      size and the section name were each varied against the running image and none of them mattered). Filling
      the slots from the lookup table turned a fault at `rip = 0` into a program that exits 7. Two more bugs
      came out of the same walk: the import table's size was counted without the even-alignment of its hint/name
      entries, so the table was *resized down* and the library name lost its last letter (`ucrtbase.d`, which
      the loader reports as a missing DLL); and `ucrtbase.dll` exports `__stdio_common_vfprintf`, not `printf`,
      because the UCRT keeps every variadic front end in the program - so the C runtime carries them now, eight
      short wrappers compiled by xcc and tested by every `printf` in a linked program.
- [x] **Lua's messages, and its suite's stopping point.** `va_start` pointed at the wrong slot in any file with
      more than one variadic function (`skip = 2n - 1`), so every diagnostic Lua printed was built from the wrong
      arguments; it now matches clang's. The suite stops in `main.lua` at `getoutput` - **and a clang build stops at
      the same line with the same error**, because those tests need a Unix-like shell. Not ours to fix.
- [ ] `back-x64` shrinking, round six: **jump tables for `switch`**, which is the one item in this
      list nobody has measured, and the passes below on machine code rather than `cir`. The other two
      things this item used to name are settled and must not be re-proposed: jump shortening was
      measured at **405 bytes (0.3%)** and interval splitting at **125 spill accesses, 81 loads and 44
      stores, about 625 bytes (1.0%)** - see the dropped list below, where both now live with their
      numbers. The 2 119 copies that liveness blocks are a *coalescing* ceiling and not a splitting
      one: a value that could not be coalesced is a `mov`, not a spill, and shortening its live range
      does not remove it.
- [x] A differential harness that compiles with both lowerings and compares behaviour, so moving
      work between them is safe. It is the *same* test rather than a second one: each corpus program
      is compiled and run at `--opt-level=0` and at `1` and by `clang -O0`, and every pair of the
      three answers must agree - exit code, stdout, and refusal. It also catches a program one
      level compiles and the other does not, which is the failure a compiler is most likely to have
      first. 412 programs agree; 8 are refused at both levels with reasons; running the extra
      lowering costs 18% of the test's time because the work is already parallel.
- [~] `--opt-level` picks the lowering: `0`/`1` Cranelift (compile speed), `2`/`3` ours (code
      quality), with numbers in `docs/COMPARISON.md` style rather than claims.
      **`0`, `1` and `2` are there**: `0` leaves Cranelift at `opt_level=none`, `1` turns its
      optimiser on, and `2` is this compiler's own back end, Cranelift not in the path at all. `3`
      is refused as the caller's mistake: a number never means two things. Measured on the six-file corpus: **3822 bytes of x64 at `0`,
      2459 at `1`** (55 ms against 180), which takes the gap the measuring stick reports from 3.1x
      to **2.0x**. All 414 corpus programs that run agree between the two levels, and a JIT test
      keeps it that way.
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

