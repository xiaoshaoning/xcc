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

### `&&` and `||` - 2026-11-15 - review of the short-circuit commit

1. **The old `&&`/`||` arm in `binary_result` was unreachable and wrong.** `binary_expression`
   now intercepts those operators before `binary_result` is ever called, so the arm that remained
   there - complete with a `not_lowered` refusal from the previous milestone - was dead code that
   said the opposite of what the compiler now does. Deleted. A dead branch that describes the old
   behaviour is worse than a comment: the next reader cannot tell which is true.
2. **`left_int.cir.or(Some(left_cir))` was always `Some`** - a pattern that reads as a fallible
   choice and cannot fail. It says what it means now: the converted left side when the conversion
   emitted something, else the original.

The interesting part of this commit was not the lowering but where the lowering had to go: parse
order, not type rules, is what makes short-circuit possible. That is why the operator is handled
in `binary_expression` and why the arm above had to die - `binary_result` sees both operands
already evaluated, and that is precisely what short-circuit forbids.

### the crt - 2026-11-15 - review of the runtime file and the probing setting

1. **The crt had no caller.** `crates/xcc-crt/src/main.c` was written and nothing compiled it:
   a file that cannot fail, cannot pass, and will drift away from the subset the compiler
   supports - the exact shape of the `#[ignore]` hazard the switch test had. It has a caller now
   (`tests/crt.rs`), and the test states the claim ADR-005 makes: xcc compiles its own runtime,
   so a program that runs proves something about the compiler. **It passes** - `exit`, `main` and
   a call are inside the supported subset today.
2. **The probing setting is unverified by measurement.** `enable_probestack`/`probestack_strategy`
   are string settings, accepted at construction; a frame larger than a page is the only thing
   that would prove them, and no test builds one. Recorded rather than guessed: a function with a
   large local array is the test, and it needs arrays as locals, which the emitter still refuses.

### the cursor fix - 2026-11-15 - review of "jump back from where we are"

1. **One question, written five times.** `if`, `while`, `do` and `for` each spelled out
   `self.current_block().is_some_and(|now| !self.terminated(now))` - the rule that a construct
   jumps back only from an open block - and it is the rule the trap fix is *about*, so it belongs
   behind a name: `current_is_open()`. Now one definition and four call sites that read as the
   sentence they are.
2. **Four `let _ = block;` lines** existed only to keep a destructured binding alive after the
   block stopped being consulted. Dead bindings that look deliberate are worse than unused
   parameters: the `_then`/`_else` patterns say "not used" in the language's own words.

The fix itself was right - the failing dump proved it - and this is the cleanup that makes the
next reader see the rule rather than reconstruct it.

### switch - 2026-11-15 - review of the switch commit

The test that shipped with it proves arms, `break`, `default`, fall-through and negative case
labels. The review found what the test could not reach, because it needs a switch inside a loop:

    int f(int n) { int r = 0; while (n > 0) { switch (n) { case 1: continue; default: r = r + 1; } n = n - 1; } return r; }

1. **`continue` was sent to the switch's exit.** `switch` pushed `(exit, exit)` onto the
   loop-target stack, so `continue` left the loop by the wrong door and skipped the loop's step.
   A switch pushes no loop target now; `break` consults its own exit when no loop target exists,
   and `continue` keeps seeing the loop around it. The `(exit, exit)` pair was a loop target that
   was not a loop - one fact in a place it did not belong, the tenth of these.
2. **Still open: that program ends in a trap** (`STATUS_ILLEGAL_INSTRUCTION`), so a block in this
   shape reaches `unreachable`. The first suspect is `emit_switch_chain`'s fall-through loop,
   which pairs arm *i* with arm *i+1* while `default` may sit anywhere in `arms`. Not fixed here:
   a guess at the pairing would be a second wrong shape rather than one, and the reproduce line
   above is short enough to keep.

### pointer arithmetic - 2026-11-15 - review of the subscript commit

1. **"A constant keeps its folded value when it gains code" was written twice.** The `Number`
   branch of `primary_expression` emitted a `cir` constant and rebuilt the `Value` with
   `.with_constant(..)`, while `constant_of` - added in the same commit for pointer scaling - did
   exactly that. One now calls the other; the rule lives in `constant_of`, where a pointer's
   scale factor also needs it.
2. **Recorded, not fixed: pointer *difference* is not part of `pointer_walk`.** `p - q` (a count
   of elements) is twenty lines inline in `binary_result`, beside a `pointer_walk` that owns
   `p +/- n`. Two halves of one subject in two places. It wants folding into one function that
   answers "what is the address arithmetic here?", and it is left as it is because the inline
   block is tested and the shape of the merged function is not yet obvious - the third time a
   finding has been recorded rather than guessed at.

After the review, the ninth instance of "one fact in two places" in this project. Two of the nine
produced wrong programs (`'A'` as 0, a one-byte object for a two-byte array); the rest were
duplication that would have drifted.

### data patches 3-4 - 2026-11-15 - review of the string literals and the data section

1. **A literal whose escape nobody reads produced a one-byte object for a two-byte array.**
   `string_literal` computed the array's size from the *spelling* and its bytes from
   `string_bytes`, and on `None` fell back to `unwrap_or_default()` - an empty vector, then a NUL
   pushed onto it. `char *s = "\q";` therefore returned a pointer to an array of one byte whose
   type said two: a wrong program, and the third time this project's own rule ("a guess is worse
   than a refusal") has been broken by a convenience call. The bytes are now checked against the
   count the type promised, refused with a diagnostic, and zero-filled to the promised length so
   the type and the data cannot disagree even in the broken case.
2. **Recorded, not fixed: the back end spells "link name" two ways.** `functions` is keyed by
   `String` (from `GlobalAddress`, per the naming rule), while `Lowerer::name(func)` resolves
   `Func::name` through the name table. Two spellings of one concept, one conversion apart. The
   repair is `Func` carrying its link name as well - the third shape the ADR deferred - and it
   waits for a caller that needs it, which M5's `crt` will be.

### the walker - 2026-11-15 - review of the traversal that writes bytes

1. **The leaf-write block appeared twice**: once for a leaf inside a brace list
   (`initializer_value`) and once for a leaf that is the whole initialiser (`initializer`'s
   scalar path) - the same "write here, or clear everything" decision, copied. Now one
   `write_leaf(ty, value)`, which is also where the rule lives that a partly-filled object is a
   wrong program and not a smaller one.
2. **Recorded, not fixed: the literal's spelling is collected by a lookahead.** The string branch
   walks ahead over the adjacent `Str` tokens to build the concatenated spelling before
   `string_literal()` consumes them, which is the same walk `string_literal` does internally. The
   repair is for `string_literal` to return the spelling it already assembles - a signature change
   with two call sites - and it waits for patch 3, which is when the second caller arrives and
   makes the right shape obvious.

After seven reviews in this project, the count of "one fact in two places" findings is now seven.
It has never once been a false alarm, and finding it by asking "where else is this true?" has been
cheaper than reading for style in every case.

### data patch 1-2 - 2026-11-15 - review of the bytes writers and the globals

Three findings from the first data code; two fixed, one recorded because fixing it needs patch 4.

1. **A check that could never fire.** `constant_bytes` refused a value with no constant up front
   and then asked `value.constant?` two lines later - the same question twice, with the second
   one making the first redundant. Deleted; the `?` is the one that reads the value.
2. **`static` objects claim to be external.** `patch 2` pushes every file-scope object with
   `external: true`, because the storage class lives in `Specifiers` and nothing reads the flag
   yet. Left as a comment on the field rather than guessed at: a `static` marked exported is a
   bug the linker would find at patch 4, and guessing the flag now would be a second guess to
   undo. Recorded here so patch 4 removes the comment instead of inheriting the lie.
3. **One decision asked twice.** `declaration` decides "is there an initialiser?" by peeking at
   `=` before pushing the global, and then eats the `=` further down. Two readings of one fact:
   the day an initialiser form arrives that is not `=`, the peek and the eat disagree. Not fixed
   in this commit because the repair is the walker's arrival (patch 1's other half turns both
   readings into one `initializer_bytes` call), and a half-repair would move the duplication
   rather than remove it.

The pattern the last four reviews keep finding - one fact in two places - is now the first thing
to look for in this codebase, and these are two more instances of it.

### M5.1 - 2026-11-15 - a design review of the only new code: one test

Everything written since the last review is documentation (ADR-003, ADR-004) and one integration
test, `tests/exe.rs`, so that is what got read. Two findings, both small and both real:

1. **The test leaked a temp directory per run.** `temp_dir()` created one and nothing removed it:
   a slow leak in a drive nobody looks at, and the kind of thing that is nobody's job until it
   fills a CI disk. It cleans up before it asserts now - cleanup first, so a failed assertion
   still cannot skip it.
2. **A hand-written loop over three linker names** where `Iterator::find` says it in one line -
   clippy pointed at it, which is the argument for running clippy on tests and not only on the
   library.

One thing checked and left alone: the test **passes when no linker is installed**, with a printed
message. That is masking information, which the principles warn about - but the alternative
(failing) would blame the compiler for a missing toolchain. The rule it settles on: skip loudly,
never silently, and the message names the reason.

### calls - 2026-11-15 - a design review of call lowering

One finding, and one thing checked because the code made it easy to be wrong.

1. **Two builds of one signature.** Lowering a body asked `signature()`; lowering a function
   *address* built a `Signature` inline from the C type - the same question about the same C
   type answered in two places, which is how the two drift. One `signature_of(&[TypeId], result)`
   serves both, and `signature()` is now three lines that gather a body's parameter types.
2. **A call to a function of this unit declared itself an `Import`.** It happened to work -
   Cranelift kept the earlier declaration - but the code said something false about a function
   the module defines, and "happens to work" is what a test stops catching. The id is looked up
   in `functions` first, and only a name from outside the unit is imported.

The rest held up: `GlobalAddress` reads the signature from the function *type* rather than
carrying it twice, and a call through a function pointer is refused where it is seen.

### M4.4 follow-up - the JIT's public surface

`Jit` is the first thing a host touches, so its interface was read on its own. Two findings:

1. **A safe constructor took raw pointers.** `Jit::new(&[(&str, *const u8)])` accepted addresses
   compiled code would call, under a contract nothing checked - and the common case (no host
   symbols) had to write `&[]` to say so. The rule that applies is "make the common case simple,
   keep rare options out of the way": `Jit::new()` is safe, and `unsafe fn with_symbols` carries
   the contract in its doc comment where a caller cannot miss it.
2. **Two error paths with two shapes.** A preprocessor failure returned every diagnostic joined;
   a parser failure returned only the first. Same kind of failure, different answer, and a host
   fixing one mistake per compile is a host that compiles once per mistake. One helper collects
   every error from both stages; `describe` is gone.

Behaviour unchanged: 124 unit (five for the back end and the JIT), 17 parser, 11 CLI, pp gate,
fuzz - green; fmt and clippy clean.

### M4.4 - 2026-11-15 - the JIT, and a review that did not look

The JIT is small: `Jit::new(symbols)`, `add_source`, `function(name)`, one lowering shared with
the object writer. Four tests: `add(2,3) == 5`, two JITs not seeing each other's code, eight
threads compiling two each, and a bad source returning its diagnostic.

The review that preceded this commit teaches the more useful lesson. It claimed to fix
`declare` throwing away the function id, and it did not: the patch was written against the text
as I remembered it, rustfmt had reformatted that call onto three lines, the replacement matched
nothing, and the "fix" committed an unchanged `let _ = id`. The JIT then found it immediately -
`function("add")` returned `None` because nothing was ever recorded - which is the argument for
an acceptance test over a review comment, and for checking that a patch applied instead of
assuming a silent `str.replace` did its job.



`back/mod.rs` was written in one sitting to get an object out, and three things in it showed.

1. **The declaration pass threw away what it was for.** `declare` called `declare_function` and
   then discarded the id (`let _ = id`), `lower` declared the function a *second* time, and the
   `funcs` map that was supposed to hold the ids was never written to. The pass exists so a call
   can name a function declared later, so it now fills `functions`, `lower` looks the id up, and
   the duplicate declaration is gone. The `let _` was the tell: a value nobody wanted means the
   step has no purpose.
2. **`widen_flag` returned `Result` and could not fail.** `?` on an infallible call, and a
   function that answers `Ok(())` in every path, is error handling for an error that does not
   exist; it is a plain function now.
3. **Two parameters nobody read**, threaded through `lower_inst` and `lower_term` because the
   signature looked symmetric: a pass-through variable in function-argument form. Dropped.

Kept, deliberately: `LoweringError` is a newtype over a string, which is a thin interface - but
it names an outcome that must not be confused with a C diagnostic ("a compiler bug" versus "your
source is wrong"), and the `From<ModuleError>` keeps Cranelift's error type from leaking into
`compile.rs`. That one earns its keep; the review argues with itself in the comment.

Known ceiling, recorded rather than fixed: `signature()` builds every parameter as a plain
argument and ignores C's `...`. Varargs work only because the emitter refuses calls until M4.4,
and the parameter has to be honoured when they land.

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
