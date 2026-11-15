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

## M0 - decisions  (S)

- [x] Target `win-x64`; Cranelift as the back end; `long double` is `f64`; no C++.
- [x] Embedding is the product, not a side door: no global state, structured diagnostics, JIT.
- [x] `--target=dos16` drives `libbcc20` through FFI, for comparison.
- [x] Recorded in [docs/ADR-001-architecture.md](docs/ADR-001-architecture.md).
- [ ] Name is provisional; rename before the first release.

## M1 - skeleton, diagnostics, API shape  (S)

Everything after this milestone depends on the API being right, so it comes before the parser.

- [ ] Cargo workspace: `xcc` (library) and `xcc-cli` (binary); no `unsafe` outside
      `jit/`, `capi/` and `bcc20-sys/`.
- [ ] `struct Compile` (per-compilation state, owned, `Send`), `Options`, `Diagnostic`
      (severity, message, file, span, notes) - text rendering is a separate step from
      producing them.
- [ ] `miette` rendering with carets; `--format=msvc|gcc|human`.
- [ ] Snapshot tests (`insta`) for diagnostics; a fuzz smoke test that feeds random bytes and
      requires no panic.
- [ ] CI on `windows-latest`: `fmt --check`, `clippy -D warnings`, `test`, `cargo-fuzz` smoke,
      release build of `xcc-cli`.
- [ ] Acceptance: `xcc-cli --version`, a deliberately broken source gives
      `t.c(3,7): error: <message>` and exit code 1, and 10 000 fuzz inputs cause no panic.

## M2 - preprocessor  (L)

- [ ] Lexer with byte spans; identifiers, string and char literals with escapes, numbers in
      C89 forms plus `ll`/`ull` suffixes.
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
- [ ] C99 syntax that costs nothing to accept: declarations after statements, `for (int i...)`,
      `inline`, `restrict`, `//`, `long long`, `stdint.h`-style typedefs.
- [ ] Semantic analysis: scopes and shadowing, type compatibility, implicit conversions,
      lvalue rules, function prototypes against definitions, constant folding, `sizeof` and
      alignment, static initialisers, tentative definitions.
- [ ] Acceptance: the `c-testsuite` (MIT) corpus parses and type-checks with the same
      accept/reject verdict as clang for every file (diagnostic text may differ); no false
      accepts, which is the test that matters.

## M4 - JIT: the embedding milestone  (L)

The first thing worth showing, and the reason the project exists.

- [ ] Typed middle end (`cir`): expressions lowered to a small, target-independent IR that
      keeps source spans, so bad code paths can be reported later.
- [ ] Cranelift lowering: integers `char`..`long long`, pointers, arrays, structs, unions,
      control flow, calls, globals, string literals, `switch`.
- [ ] JIT: `Jit::new()`, `add_source(&str)`, `add_symbol(name, fn_ptr)`, `get_fn::<F>(name)`,
      `remove`, and redefinition of a function by rebuilding the module.
- [ ] `extern "C"` ABI on x86-64 Windows: shadow space, stack alignment, varargs, `__chkstk`.
- [ ] Acceptance: from a string, `int add(int a, int b) { return a + b; }` is compiled and
      called as `extern "C" fn(i32, i32) -> i32` returning 5 for (2,3); a script calling back
      into a host function through `add_symbol` works; 100 compiles from 8 threads pass (no
      global state).

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
      `clang -O0`, compare stdout and exit codes; the corpus includes the usual suspects
      (fizzbuzz, struct/union/bitfield spray, varargs, recursion, `long long` corners).

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
- [ ] `xcc-cli --target=dos16 -c -ml -O -G t.c` compiles a C89 source with the ported
      Borland compiler, when `libbcc20` is present.
- [ ] A `compare` example that compiles the same files both ways and prints: time, code size,
      diagnostics. This is how PLAN's successor gets its numbers, and how the front end gets
      cross-checked against a 1991 implementation.
- [ ] Acceptance: the comparison runs on the six-file corpus and reports a table.

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
- **Embedding expectations**: hosts want sub-millisecond compiles and no surprises. Mitigation:
  benchmark from M4 (`criterion`), and treat every `panic` path as a bug with a fuzz test.
- **Scope creep into optimisation.** Cranelift's default is fine. An optimiser is a different
  project; the answer to "make it faster" is "use clang".

## First slice, ready to start

M1, as one pull request: workspace, library skeleton with `Options`/`Diagnostic`/`Compile`, CLI
with `--version`/`--help`/`--format`, `miette` rendering, three snapshot tests, one fuzz smoke
test, CI job on `windows-latest`, `README` polish. Acceptance is M1's, and it does not touch a
line of parsing, so it can be reviewed quickly.

After that, in order: M2, M3, M4 (JIT demo), M5 (hello.exe), M6, then M7 to M9 in whatever
order users ask for.
