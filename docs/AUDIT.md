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
  Apache-2.0 (curl), or our own - which includes [tests/corpus](../tests/corpus/README.md), taken
  from another project by this repository's author under MIT. Nothing from TinyCC's or Borland's
  test suites, and nothing derived from a decompiled compiler (CONSTRAINTS 7).
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
| 2026-11-15 | M2 | `tests/corpus` | 426 | 38 (`#include`, no headers until M8) | 0 | 0 | 0 | 2 bugs, listed below |
| 2026-11-15 | M3.1 | `tests/corpus` | 426 read | 0 parsed completely (every program has a body) | 426 refused with "not built yet" | 0 | n/a (no expressions yet) | 0 |
| 2026-11-15 | M3.2 | `tests/corpus` | 464 read | 322 parsed completely | 142 refused (structs, `switch`, function pointers, initialiser lists: M3.3) + 25 correctly rejected as not-C99 | 0 | n/a | 9 checker bugs, below |
| 2026-11-15 | M3.3 | `tests/corpus` | 464 read | 387 parsed completely | 77 refused (`switch`, `goto`, designators) | 0 | n/a | 3 checker bugs, below |
| - | M3.4 (planned) | `c-testsuite` and `tests/corpus` | - | - | - | - | - | - |

### M3 exit - 2026-11-15 - a design review of the whole front end

Not a corpus audit: a pass over every file the earlier review commits had not opened, against
the design principles (deep modules, information hiding, red flags). The files were
`intern.rs`, `lex.rs` and `source.rs` for the first time, `options.rs`, `symtab.rs`,
`pp/eval.rs` and `render.rs` for the second. Nine findings, worst first, each fixed in the same
commit as its neighbours:

1. **Two escape tables that disagreed, and one of them lied.** The preprocessor knew the
   simple, octal and hex forms; the parser knew seven escapes and answered `None` for the
   rest, which its caller turned into **zero without a diagnostic**. `int x = '\x41'` was
   therefore 0, not 65. The knowledge is one module now, `literal.rs`, and what it does not
   read it refuses: `'ab'` and `'\q'` are errors, not zeros. A second bug fell out of writing
   its test: a naive escape counter said `"\x41"` spells three bytes, so a `char` array was
   given the wrong length.
2. **A string literal was a `char *`, not a `char[n]`.** C gives a string literal an array type
   (6.2.5p20), and the difference shows: `sizeof "abc"` was the size of a pointer, and
   `char s[] = "ab" "cd"` counted only the first literal because concatenation threw the rest
   away. It is an array value now, and one `string_literal` helper serves both the expression
   and the initialiser, so `sizeof` and the array's length come from one place.
3. **The epoch lived in three places** - `Stamp::epoch()`, and two `map_or` fallbacks in the
   preprocessor - because `Options::stamp` was an `Option` for a value that has a sensible
   default. `Stamp` implements `Default` as the epoch, `Options::stamp` is a `Stamp`, and the
   two fallbacks are gone.
4. **`Symbols` kept two stacks pushed and popped in lockstep** (`scopes`, `tags`), so the
   invariant "a scope has both" was maintained by hand in four methods. One `Vec<Scope>`, where
   `Scope` holds both maps.
5. **`SourceFile::len` returned `u32`**, which truncates a file above 4 GiB - the number a byte
   budget exists to compare. It returns `usize`, and the budget check compares like with like.
6. **`->*` was in the punctuator table**: a C++ token in a C99 lexer, and one more string to
   compare on every punctuator. Deleted; C99 6.4.6 has 54 punctuators, not 55.
7. **The lexer made a `String` per token** only to hand it to the interner, which copies it
   again. The spelling is interned straight out of the prepared text.
8. **`Interner::len` and `is_empty` had no caller** outside their own tests. Deleted.
9. **`identifier` and `number` returned a `TokenKind`** that was always the same one, which
   their caller passed straight back. They return nothing now.

`render.rs` and `pp/eval.rs` came through with no finding beyond (1); their structure holds.
The parse gate did not move: **394 of the 394** corpus programs that are neither `#include`d
nor deliberately invalid.

### M4.2 - 2026-11-15 - a design review of `cir` and the emitter

The code written for M4.1 and M4.2 had not been read by a review pass. Three findings, all
fixed in the same commit:

1. **The front end reached into `cir` to keep a `cir` invariant.** `terminate_orphans` walked
   `Func::blocks` and built block handles by index, which is the one thing ADR-002 invariant 1
   forbids ("only the builder mints a `Value`") applied one field over. The knowledge - a block
   that will never be filled ends with `unreachable` - is `cir`'s, so it moved there as
   `Func::end_orphans(current)`, and the parser stopped fabricating handles.
2. **The emitter lived in `parse/mod.rs`** next to the token helpers, although it is the one
   place that knows both C's conversion rules and `cir`'s instructions. It is `parse/emit.rs`
   now (182 lines), and `mod.rs` is 1333 lines down to 1162. Same reasoning as `parse/init.rs`
   at M3.3: a grammar or a translation that has its own vocabulary deserves its own file.
3. **`emit_binary` and `emit_compare` took eight parameters** - enough that clippy's threshold
   caught them, and the fix had been to bundle the two operands into `[Value; 2]`, which reads
   as a trick. The parameters that are genuinely one thing (`operands`, `result`, `sides`,
   `folded`) are still four; the honest repair is to drop `folded` and let the caller keep the
   constant, which is a change to make when a second caller needs it rather than now. Recorded
   here as a known wart rather than fixed in a hurry.

What the review did *not* find is as useful: the refusal discipline held. Every construct the
emitter cannot express is a diagnostic at the construct, the body is dropped rather than
partially lowered, and the verifier runs on every body kept - which is what caught three real
bugs during M4.2b rather than after it.

### M3.4 - 2026-11-15 - the corpus, read to the end

The parse gate went from 387 to **395 of 464**; the 69 that are not counted are the ones with
`#include` (this front end has no headers until M8) and the 26 in `EXPECTED_INVALID`, which are
programs a conforming C99 compiler must refuse. Nothing in the corpus is refused for being
unbuilt any more.

Two things the corpus found, both about ordering rather than grammar:

1. **`default:` was read as a label.** A label is `name :`, and `default` ends in a colon too,
   so the label branch ran before the keyword match. The keyword match now comes first.
2. **A designator followed by a comma was mistaken for a full aggregate.** `{.y = 2, .x = 1}`
   reached the "too many initialisers" check because the check ran on the comma, before knowing
   a designator came after it.

Deliberate ceiling: a designator that names a sub-object - `[1].x = `, `.a.b = ` - is refused
with one diagnostic instead of half-read, because carrying the cursor back out through the chain
is not written yet. The GNU `[a ... b]` range is refused for the same reason: it is not C99.

### M3.3 - 2026-11-15 - aggregates and declarators

The parse gate went from 322 to 387 of 464. Three real bugs, all in the parts where C's
grammar and C's layout rules disagree with the obvious reading:

1. **Suffix order was backwards.** `int a[1][2]` was typed as an array of two arrays of one:
   the suffixes have to be applied innermost-last, so the *last* one touches the base first.
   It took `char s[4] = "abc"` and a nested initialiser to expose it.
2. **Commas in initialiser lists belonged to the wrong list.** A nested list consumed the
   separator that belonged to the list around it, so `{{1,2,3},{4,5,6}}` failed on the second
   row. Each level now takes a separator only when it is going to continue.
3. **Bitfield widths were swallowed twice**: `bit_width` takes the `:` itself, and its caller
   did too, so `unsigned a : 3` read the `;` as the width expression.

### M3.2 - 2026-11-15 - the corpus, parsed for meaning

322 of 464 corpus programs now parse completely, up from 0. The test that made this possible
also kept it honest: every refusal of every program must be a "not built yet" or a *correct*
C99 rejection, and the correct ones are listed in the test with their reasons (implicit
`printf`, Borland's `word`, `&` of a non-lvalue, a call with the wrong count). Anything else
fails the test.

The checker was wrong nine times before the corpus stopped arguing, each a real C rule:

1. Array arguments did not decay: `f(a)` with `int a[10]` was refused. Decay is now one
   function applied where a value is *used*, not where it is assigned to.
2. Array *parameters* did not decay either: `int a[][3]` must be `int (*)[3]` (C99 6.7.5.3p7).
3. `++`/`--` rejected pointers.
4. `int + pointer` was rejected while `pointer + int` was accepted.
5. `(&x)` complaints after an already-refused construct: a poisoned value must never produce a
   second diagnostic, and several checks did not test for it.
6. `sizeof` of a poisoned type complained, twice, with two different messages.
7. `(int)x` was read as a parenthesised *expression*: the cast was tried only on an identifier.
8. A label was read as an expression (`done: ;`).
9. A refused declarator ended the whole declaration, so `typedef int (*A)(), (*B)();` left `B`
   undeclared and cascaded.

### M2 - 2026-11-15 - `tests/corpus`, preprocessing only

Compared with `clang -E`, token for token, the reference's text lexed by our own lexer
(`crates/xcc/tests/pp_gate.rs`): **426 of 426 agree**, nothing excused, 38 programs skipped
because they include a header and there are no headers until M8. One disagreement is known and
documented rather than excused: `__STDC_HOSTED__` is 0 here and 1 for a hosted compiler.

The harness was proved able to fail before it was trusted: a probe file whose
`#if __STDC_HOSTED__` disagreement made the gate report the file and the first differing tokens.

Findings, worst first:

1. **`#if 1 ? 1 : 1/0` was rejected** and is valid C - the untaken arm of `?:` was being
   evaluated. Found by asking `clang -E` the same question the corpus does not ask.
2. **A malformed `-D` was reported in the source file**, at an offset past its end, because the
   command line had no file of its own. Now: `<command line>: error: ...`.

Neither was reachable from the corpus, which is the point of writing them down.

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
