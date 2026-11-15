# The corpus

464 C programs, the conformance ladder for M3 and M6. They came from another project by the
same author (`Writing-a-C-Compiler-in-Matlab`), which compiles the same language for the same
ISA and checks its output by running it, so the programs have already earned their keep once.

Licence: **MIT**, like the rest of xcc. The author of both projects is the same person and the
sole author of these files (`git log` over the source project: 311 commits, one author), so the
grant is his to give.

## What is here, and what is not

| taken | why |
|---|---|
| the compiler-track programs: `cc*`, `p3_*`, `pp_*`, `dreg*`, `dmore*`, `stress*`, `return_*` | exercises the language xcc targets: expressions, control flow, functions, `char` and globals, pointers and arrays, structs, `switch`, `sizeof`, typedefs and enums, multi-dimensional arrays, function pointers, casts, `void`, the comma operator, doubles |

| left behind | why |
|---|---|
| `tests/programs/hello.c` | copied from `lotabout/write-a-C-interpreter`, GPL-2, and it is an acceptance program for that project's interpreter, not for a compiler |
| `xc.m`, `cc_int.m`, `ma_asm.m`, `x86sim.m`, `peephole_pass.m` | MATLAB, and a different design (an interpreter, a simulator, an assembler). Read for method, never copied: `xc.m` is a GPL-2 derivative of `xc.c` |
| `tmp_*` | scratch files from the source project's runs |

## How the harness must use them

There are no stored expectations, on purpose. The source project's authority was the host
compiler: every program is compiled with `gcc`, run, and its exit code and stdout compared with
its own compiler's. xcc does the same, with `clang -O0` as the reference:

1. compile the program with xcc, link it, run it - or, once M4 exists, JIT it and call `main`;
2. compile the same file with `clang -O0` (Windows x64), link, run;
3. compare exit code and stdout byte for byte.

That is a differential test, so a program needs no annotation to be useful, and a *new* program
is added by dropping a file in.

**426 of the 464 need no header at all** (`grep -L '#include' *.c`), so they compile, link, run
and JIT as soon as the compiler can emit code - no runtime, no headers, no M8. 38 include
headers and 62 print, and those wait for M8. That split is the reason this corpus arrived before
the parser: the majority of it is usable at the first milestone that can produce a callable
function.

Two groups are treated differently:

- **`cc_bad_*.c` must be refused.** They are inputs C does not accept (assigning through `&x`,
  and similar). xcc must report a diagnostic and write no object: [CONSTRAINTS](../docs/CONSTRAINTS.md)
  3. Today there are two; more come from `clang`'s own rejections.
- **Programs the reference compilers accept only as extensions.** Some of these programs poke at
  Borland-or-gcc-ish dialect corners. If `clang -O0` accepts a file only with an extension flag,
  the file is not evidence about C99; the harness skips it and the reason goes in the audit log
  ([AUDIT](../docs/AUDIT.md)). Triage happens at M3, file by file, and the list of skips belongs
  in the audit, not in this file.

## Name groups

The names came from the source project's suite and are indicative, not load-bearing: the harness
treats every file the same way except for `cc_bad_*`.

| group | roughly |
|---|---|
| `cc2_*` .. `cc18_*` | one program per feature of the tutorial the source project followed, in its order: constants, unary and binary operators, comparisons, arithmetic, locals, control flow, functions, `char`/globals/compound assignment, pointers/arrays/strings, structs, `switch`/`sizeof`/typedef/enum, function pointers/`goto`/casts |
| `cc19_*` .. `cc60_*` | extensions beyond that tutorial: dialect corners, regressions, double printing, aggregate initialisers |
| `cc_bad_*` | inputs a compiler must refuse |
| `p3_*` | small expression and statement programs (`return 10-4-3;`), from an earlier pass at the tutorial |
| `pp_*` | array and pointer address-of paths, brace and string initialisers, `sizeof`, multi-dimensional arrays - the code-generation corner cases, not the preprocessor (none of the 55 uses `#define`) |
| `dreg*`, `dmore*` | doubles: formatting, dense literals, globals |
| `stress*` | longer programs meant to be run, not read |

## Provenance

- Source: `Writing-a-C-Compiler-in-Matlab`, `tests/programs/*.c`, taken 2026-11-15.
- Excluded from that directory: `hello.c` (GPL-2, third party), `tmp_*` (scratch).
- Every file taken is the author's own work; none carried a third-party notice (`grep -il
  'GNU General Public'` over the source directory returns only `hello.c`).
