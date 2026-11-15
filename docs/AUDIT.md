# Audits

The ritual that keeps "it compiles our tests" from being mistaken for "it compiles C": every
milestone ends with a run over code nobody here wrote, and the numbers are written down, bad ones
included.

An audit answers three questions:

1. How much of a real corpus does xcc get through (translation units compiled, files refused,
   crashes)?
2. Where it disagrees with a reference compiler, what exactly is the disagreement - our bug,
   their extension, or undefined behaviour?
3. What did fixing it cost, and what did it teach? A front-end fidelity bug found by an audit is
   worth more than a hundred unit tests, because it is the class of bug that ships.

## Rules

- **Corpora must be permissive**: public domain (sqlite amalgamation), MIT/BSD (zlib, lua),
  Apache-2.0 (curl), or our own. Nothing from TinyCC's or Borland's test suites, and nothing
  derived from a decompiled compiler (CONSTRAINTS 7).
- **Reference compilers are oracles, never dependencies**: `clang` for verdicts and behaviour,
  `tcc` for a second opinion on C99 corners, `libbcc20` for the 16-bit comparison. Compare
  outputs; never link.
- **Counts, not adjectives.** "Compiles zlib" is not a result; "15 of 15 TUs, 3 diagnostics, 0
  mismatches against clang -O0, 2 bugs found and fixed" is.
- **Reproduce before believing**: a mismatch is recorded with the file, the snippet, and the
  command that shows it.

## Log

| date | milestone | corpus | TUs | compiled | refused | crashes | mismatches vs clang | found and fixed |
|---|---|---|---|---|---|---|---|---|
| - | M3 (planned) | `c-testsuite` | - | - | - | - | - | - |

## Template for a new audit

```md
## <date> - <milestone> - <corpus> <version>

Built with `--std=c99`, default options. Reference: `clang -O0` (and `tcc` where it disagrees).

| file/TU | result | note |
|---|---|---|
| ... | compiled / refused / crash / mismatch | the diagnostic, or the differing behaviour |

Totals: N TUs, N compiled, N refused (grouped by cause), N crashes, N behaviour mismatches.

Findings, worst first, each with the failing snippet and the fix:

1. **<one line>** - the construct, what xcc did, what it should have done, and the commit that
   fixed it.
```

## What an audit is not

- It is not a benchmark: compile time and code quality are measured elsewhere (PLAN.md M4, M10).
- It is not a conformance claim: passing a corpus means the corpus passes, and the corpus is
  chosen for the bugs it provokes.
- It is not a gate that blocks a release by itself: a refusal is a legitimate outcome, so long as
  it is in [C99-SUPPORT.md](C99-SUPPORT.md) and it is deliberate.
