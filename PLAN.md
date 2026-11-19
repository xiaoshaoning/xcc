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
- [ ] `back-x64` **floating point across calls**, attempted twice and both times reverted - the state
      of it, so the next attempt starts from the diagnosis instead of from the beginning. The ABI
      mapping is settled and was implemented: float parameters and returns in `xmm0`-`xmm3` by
      *position*, counted separately per class, with a `Target` plan (integer register, XMM register,
      outgoing slot) and a 64-byte staging area for both classes. Two things stopped it.
      **First**: `xmm2`-`xmm5` are caller-saved, so a `double` live across a call was destroyed by it -
      `d2and.c` answered 1 instead of 42. That is fixed and landed (`1361692`): a calling function
      allocates `xmm6`-`xmm15` and saves them with `movups`.
      **Second, not fixed**: with the ABI in place, `d2and.c` still failed, and the disassembly showed a
      comparison reading `xmm0` - the encoder's *scratch* - where its operand had been converted into
      `xmm8`. The fault is operand selection with a spilled float: `float_destination` returns the
      scratch for a spilled result and `float_scratch` hands the same pair out to operands, so the
      result's register, the operands' registers and the scratch are **not** pairwise distinct. The
      integer path holds that invariant by construction (`rax` for a spilled result, `r10`/`r11` for
      operands); the float path has only two spare registers and one of them doubles as the result's
      home.
      **The fix to make first**: reserve a third register - `xmm2` - as the spilled result's home, so
      the pools become `xmm3`-`xmm5` (a function that calls nothing) and `xmm6`-`xmm15` (one that
      calls), the scratch stays `xmm0`/`xmm1`, and the invariant is written beside the pool: *the
      result's register, each operand's, and the scratch pair are pairwise distinct*. Then the ABI work
      on top of it, with `d2and.c` and the mixed-signature hand tests as the first things to run.
      A test that would have caught the second fault at HEAD: a leaf function with more live doubles
      than it has registers, which `fs.c`-shaped programs do not reach because a leaf has four.
- [ ] `back-x64` shrinking, round five: interval splitting; jump shortening and jump tables for
      `switch`; and the passes below, on machine code rather than `cir`. For scale, the remaining
      copies are the return path (partly done), values crossing a call, and spills.
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

