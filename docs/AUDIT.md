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

### the three-way harness reviewed - 2026-11-16 - three sides, and a doc that still said two

Review of `6cedc70`.

**Found: the module documentation still described a two-sided comparison.** The file's own header said
"xcc --run file.c on one side, clang -O0 on the other" while the code ran three, and a reader who
trusted the doc would have missed the case the change exists for. It says what the three sides are,
why every *pair* is compared rather than each lowering with clang, and that a program one level
compiles and the other refuses counts as a difference. The README's own two rows were stale in the
same way - the optimising row stopped at the Borland comparison and did not know that `clang -O2` is
now in the table, nor that the runner exercises both levels.

**Checked and sound:** the refusal-divergence check runs before the "both refused" path, so a program
one level accepts and the other does not is reported and cannot be counted as a legitimate refusal;
the message names the direction ("compiled it at 0, and not at 1") because the fix depends on which
level moved; `plain`/`optimised` are not moved into the tuple pattern until after that check, so the
three-way comparison has all three answers; a single-sided hang is still a mismatch while a
both-sided one is a timeout, unchanged from before; and the pairwise loop is O(1) in a three-answer
array, so it costs nothing to read or to run.

**Measured after the doc fix:** 412 ran, 8 refused, 44 skipped, 0 timed out, 0 mismatched - the same
numbers, and the test's doc now says what it does.

174 unit, 23 parser, 20 CLI, 3 exe, 4 hostile, 6 C API, 4 bcc20-sys, differential (412 ran, 8
refused, 44 skipped, 0 timed out, 0 mismatched across three compilers), crt, pp gate, fuzz - green;
fmt and clippy clean.

### the support matrix, reviewed - 2026-11-16 - a compiler that said it was freestanding

Nothing in the tree was unreviewed, so the review went after what the last five rounds had made untrue:
the document that answers "do you support X?".

**Found: `__STDC_HOSTED__` was 0, and this compiler has been hosted since M8.** The support matrix said
"`__STDC_HOSTED__` is **0** until M8 ships headers" - and M8 shipped them; the macro did not change. C99
6.10.8p1 says a hosted implementation defines it as 1, and this one ships the standard headers and links
a program against the host's C runtime. A program that tests the macro would take the freestanding branch
of its own code, which is a wrong answer produced by a constant. The value is 1 now, the test that pinned
0 pins 1, and the matrix says why.

**Found: the matrix's own preamble said nothing was implemented.** "Nothing below is implemented yet. M2
is the preprocessor, M3 the parser and types..." - from the M2 era, in the document a user reads first.
It now says what is true: M10, the front end done, two back ends, and what is actually left.

**Found: three rows that had become false in the same way.**
- The freestanding headers were `planned (M8)`; they ship inside the compiler, twelve clean-room headers
  consulted after the host's, and `stdarg.h` is the one that refuses.
- The hosted headers were `planned (M8), over UCRT imports`; they ship, and a linked program uses the
  host's runtime while `--run` resolves the same names out of one loaded runtime - except `snprintf`.
- `va_arg` was refused "because the back end cannot write `AL`". That was true of Cranelift and is now
  false of this project: the own back end names registers and writes `AL` in one instruction. The rows
  (and ADR-006, which said the refusal was "permanent until the back end can write a physical
  register") now say what is actually true - the count is a value `cir` does not carry, which is work,
  and Cranelift still cannot express a variadic *definition*, which is a decision about the two
  lowerings rather than a missing instruction.

**Checked and sound:** the rows that were already right stayed untouched - the variadic *call* row with
its `AL` caveat, the `switch`/jump-table row, `__func__`, `setjmp.h` and the four headers that refuse. No
claim in the matrix is now stronger than a test behind it: the hosted rows name what `tests/exe.rs` and
`--run` demonstrate, and the freestanding row names the header set that ships.

**Measured after the review: 414 agreeing, 50 refused, 0 wrong** - unchanged, and one wrong constant
fixed.

180 unit, 23 parser, 20 CLI, 3 exe, 3 x64, 4 hostile, 6 C API, 4 bcc20-sys, differential (412 ran, 8
refused, 44 skipped, 0 timed out, 0 mismatched across three compilers), crt, pp gate, fuzz - green;
fmt and clippy clean.

### the frame's immediates, reviewed - 2026-11-16 - a plan that could not be read

Review of `afaf881`.

**Found: the plan had three shrinking items and no reader could tell which one was current.** "round five"
(done, the immediates), "round six" (the copies, open), and "round five, the old list" (open - which
described rounds *one to four*, under a number that made it look like the same round twice). The same
change decision - how much shrinking is left - was written in three places with overlapping numbers, and
the third was the measurements of work already finished and already in `AUDIT.md`. That is change
amplification in the document that decides what happens next, which is worse than in code: a reader who
picks the wrong bullet spends a session on measured-out work.

It is now one closed item with the numbers in order and one open item, and the history that lives in the
audit log is no longer copied here. The caution about measuring is kept, in the item where it bites: the
`mov` grep that counted 10 048 copies where the anchored pattern counts 2 760, and the observation that
DCE and folding looked worthless on the six-file corpus and paid 18% on the whole one - a caution that
has now earned its place twice.

**Checked and sound:** the encoding claim is verified rather than assumed - `objdump` shows
`48 83 ec 38` and `48 83 c4 38`, four bytes each, where the four-byte-immediate form was seven; the
`i8` range test falls back to `81 /ext id` for a frame of 128 bytes or more, so a large frame is
unchanged rather than truncated; the extension opcodes (`/5` for `sub`, `/0` for `add`) are named in one
method that documents both; and a frame of zero still emits neither instruction, because the prologue
guards on it before asking.

**Measured after the review: 96 348 bytes over the 414 corpus programs** - unchanged, which is what a
documentation repair should be - with 414 agreeing and none wrong.

180 unit, 23 parser, 20 CLI, 3 exe, 3 x64, 4 hostile, 6 C API, 4 bcc20-sys, differential (412 ran, 8
refused, 44 skipped, 0 timed out, 0 mismatched across three compilers), crt, pp gate, fuzz - green;
fmt and clippy clean.

### folding and dead code, reviewed - 2026-11-16 - a bitcast that kept the wrong meaning of its bits

Review of `e5706b6`.

**Found: folding a `bitcast` returned the source's bits unchanged, which is the wrong meaning for the
destination type.** `int -1` cast to `unsigned int` is `0xFFFFFFFF`; the fold produced the sign-extended
64-bit form. Every other fold in the pass relies on a constant being canonical for its own type - that is
the invariant `folded_value` now enforces on its *operands* - and this was the one arm that could break
it. It is one line: canonicalise to the destination type, like the truncating arms already do. The case
is not reachable from today's front end (it does not fold `-1`, and the two lowerings both use the
*type* rather than the bits, so a non-canonical constant is masked), which is exactly why it needed
finding by reading rather than by running.

**Found: an arm in `result_type` that could never be reached.** It listed `Unary` among the folded
instructions, and the pass never folds a unary operation. Removed, with the `unreachable!` message now
saying which two instructions can arrive.

**Checked and sound, because three passes in one commit is where a design decision can hide:**

- **Dead-code elimination keeps what has an effect**: a store, a call, and a slot - the last because the
  frame's shape comes from saying a slot exists, so removing an unused one would move every offset after
  it. A `Load` *is* removed when nothing reads it, which is the as-if rule (C99 5.1.2.3) and not an
  optimisation of a program that could notice.
- **A division is removed when its result is unused** even though it can trap. C says the trap is
  undefined behaviour, so a program that does not use the result does not have to have done the division;
  the corpus's `LLONG_MIN / -1` case is not affected because its result is used.
- **The address rule looks through at most eight bitcasts** and gives up rather than guessing: a longer
  chain, or an offset that does not fit in `i32`, leaves the addition where it was.
- **The fold loops to a fixpoint and terminates** because each round replaces at least one instruction
  with a constant and a constant is never folded again.
- **The two orderings matter and are stated**: folding runs after `mem2reg` (promotion is what makes a
  local's value a constant), and DCE runs after folding (folding is what makes instructions dead).

**Measured after the review: 97 432 bytes over the 414 corpus programs - unchanged**, with 414 agreeing
and none wrong.

180 unit, 23 parser, 20 CLI, 3 exe, 3 x64, 4 hostile, 6 C API, 4 bcc20-sys, differential (412 ran, 8
refused, 44 skipped, 0 timed out, 0 mismatched across three compilers), crt, pp gate, fuzz - green;
fmt and clippy clean.

### floats across calls, reviewed - 2026-11-16 - an invariant the commit claimed and the code did not hold

Review of `c0ff87c`.

**Found: the register pools did not hold the invariant the commit message and `PLAN.md` said they did.**
The round's account of itself is specific - "the invariant the pools now hold by construction: a result,
its operands and the scratch are pairwise distinct - `xmm0`/`xmm1` scratch, `xmm2` the home of a spilled
result, `xmm3`-`xmm5` (calls nothing) or `xmm6`-`xmm15` (calls)" - and the code had none of the three
pieces: `FLOAT_REGISTERS` was still `[2, 3, 4, 5]`, there was no `FLOAT_RESULT` at all, and
`float_destination` returned `FLOAT_SCRATCH` for a spilled result *without claiming it*, which is exactly
the collision `320e819` had diagnosed. The landed code happened to work - 414 corpus programs and a
dozen hand tests, including two that force nine and fourteen live doubles - because the collision needs
two spilled operands *and* a spilled destination in one operation, and no program in the corpus reaches
it. A latent hazard documented as a guarantee is worse than either: the next reader trusts it.

The fix is the two edits the plan described, landed: `xmm2` is `FLOAT_RESULT`, the leaf pool is
`xmm3`-`xmm5`, and `float_destination` claims the register like every other one an operation holds. The
property now holds where the comment says it does, and the pool comment names the invariant and says what
an overlapping pool would cost.

**Checked and sound:** the REX fixes of this round - `float_binary`, `ucomisd`, `cvtss2sd`/`cvtsd2ss` -
are in the right order relative to the mandatory prefixes (`66`/`F2`/`F3` before REX, never after), and
each was verified against `objdump` output rather than assumed; the argument placement stages both classes
into eight slots and loads them back, with the stack arguments stored before any register move; a float
result is taken from `xmm0` in `finish_call` and a float return writes it there; `Incoming` reads a
stack-passed double with `movss`/`movsd` in the value's own width; and `saved_floats` is still filtered
to the registers the allocation actually used.

**Measured after the review: 414 agreeing, 50 refused, 0 wrong** - unchanged, with two spill-heavy
programs added to the hand tests that would catch the collision if the pool is ever widened again.

177 unit, 23 parser, 20 CLI, 3 exe, 3 x64, 4 hostile, 6 C API, 4 bcc20-sys, differential (412 ran, 8
refused, 44 skipped, 0 timed out, 0 mismatched across three compilers), crt, pp gate, fuzz - green;
fmt and clippy clean.

### function pointers, reviewed - 2026-11-16 - scaffolding left by a deleted refusal

Review of `2510387`.

**Found: the set of function names existed only to feed the refusal this round deleted.** `lower`
built a `HashSet<String>` of every function's link name, allocated a `String` for each, and threaded it
through `FunctionLowering` - all to ask "is this name a function, and therefore a gap?" Now that a
function's address is a symbol like any other, nothing reads it, and the comment above it still said
"taking a function's address as a value needs a call through a pointer, which this back end does not
lower" - a sentence that had become the opposite of the code two lines below it. Both are gone; the
allocation per function goes with them.

**Found: a refusal whose message had stopped being true.** The integer width helper answered "a
floating-point value in this back end", which stopped being true when the float slice landed: floats
*are* in the back end, and what is missing at that point is the caller's expectation of an integer. It
says "a floating-point value where an integer one is needed" now.

**Checked and sound, because a pointer that survives the argument placement is the whole trick:**

- The callee's register is read *after* the arguments are placed, which is safe because a function
  containing any `cir::Call` allocates only callee-saved registers - so the pointer can never be in
  `rcx`, `rdx`, `r8` or `r9`, the four the placement writes. That invariant is written at the call site
  it protects, and the register plan that provides it says so where the pools are defined.
- A spilled callee is loaded after the placement, into a scratch the placement has released; the
  scratch pair and the argument registers are disjoint by construction.
- `place_arguments` was extracted by moving one block, and the integer path it holds is byte-for-byte
  the one the direct call used - which the corpus agrees on, since 411 programs still match Cranelift
  including every call they make.
- An indirect call writes `rax`, so it joins `writes_rax` and the `rax` preference refuses to put a
  returned value there across one; `operands` includes the callee, so liveness and the spill decision
  account for it.
- `SymbolAddress` now names one instruction used for three things - a string literal, a global object
  and a function - and the relocation is the same for all three, which is why the COFF writer needed no
  change for this round.

**Measured after the review: 411 agreeing, 53 refused, 0 wrong** - unchanged.

177 unit, 23 parser, 20 CLI, 3 exe, 3 x64, 4 hostile, 6 C API, 4 bcc20-sys, differential (412 ran, 8
refused, 44 skipped, 0 timed out, 0 mismatched across three compilers), crt, pp gate, fuzz - green;
fmt and clippy clean.

### the volatile XMM registers, reviewed - 2026-11-16 - a wrong answer for a saved value

Review of what `edfdfc4` and its two review commits left behind, found by asking what the *calling*
convention says about the registers the float slice was given.

**Found: a `double` that is live across a call was destroyed by the call.** The float slice allocated
`xmm2`-`xmm5`, which Win64 calls caller-saved, and the integer side's version of this lesson - a calling
function allocates *callee-saved* registers - was not carried over. So this program answered `1`
instead of `63`:

```c
int noisy(void) { double a=1.0, b=2.0, c=3.0, d=4.0; double r = (a+b)*(c+d); return (int)r; }
int main(void) { double keep = 2.5 * -1.25; int x = noisy(); if (keep < 0) return 42 + x; return 1; }
```

`keep` sat in `xmm2`, `noisy` used `xmm2`-`xmm5` for its own arithmetic, and the comparison afterwards
read the callee's leftovers. No diagnostic, no crash - the class of defect the corpus ratchet exists to
find, and it did not: none of the 406 programs it agrees on keeps a float across a call to a
float-using function, which is exactly the blind spot a corpus built for a compiler without floating
point would have.

**The fix is the ABI's own answer, and it is the piece that was already written once this session:**
a function that calls allocates `xmm6`-`xmm15`, which a call preserves, and saves the ones it uses with
`movups` into sixteen-byte frame slots (`movups` because a save slot sits wherever the integer saves
left room, and asking for alignment would buy nothing). A function that calls nothing keeps
`xmm2`-`xmm5` and pays nothing, exactly as it does for integers. `Allocation` gained `saved_floats`
beside `saved`, the frame gained a band between them, and the prologue and epilogue name both lists -
one field each, so the two cannot drift.

**Checked and sound:** the sixteen-byte slots need no alignment because `movups` has none, so the band
can start at whatever offset the general saves end on; `saved_floats` is filtered by which registers the
allocation actually handed out, so a function that saves nothing pays nothing; the frameless rule now
requires both lists empty; and a leaf function's float code is byte-for-byte what it was, which the
406-program ratchet confirms by not moving.

**Measured after the review: 406 agreeing, 58 refused, 0 wrong** - the same numbers, plus one program
that was wrong and is now right.

177 unit, 23 parser, 20 CLI, 3 exe, 4 x64, 4 hostile, 6 C API, 4 bcc20-sys, differential (412 ran, 8
refused, 44 skipped, 0 timed out, 0 mismatched across three compilers), crt, pp gate, fuzz - green;
fmt and clippy clean.

### the float fixes, reviewed - 2026-11-16 - a panic where a diagnostic belonged

Review of `7e6f1e5`.

**Found: a function that returns a floating-point value stopped the compiler.** `double f(void) { return
1.5; }` reached the encoder's return as a value whose class was Float, and the encoder's `read` - which
had just learned to say `unreachable!` rather than use a general register's number as an XMM one - said
so by panicking: exit 101, no diagnostic, no source line. The `unreachable!` did its job, which was to
turn a silent wrong answer into a loud stop; a stop is still not an answer. The lowering refuses it now,
with the span of the `return`, and the same for a floating-point parameter, whose message had been the
left-over "a floating-point value in this back end" - untrue since the previous commit, and unhelpful to
anyone reading it.

**Found: two messages that had become lies.** The refusal for a float parameter came from the integer
width helper, which still said floating point was not lowered at all. Both gap messages now name the ABI
they are missing: `xmm0` for a return, `xmm0`-`xmm3` by position for a parameter.

**Checked and sound, path by path, because "a value's class" is a promise the lowering makes to the
encoder:** every operation that defines a value classifies it - constants, arithmetic, comparisons,
conversions, loads, stores, block parameters, calls, addresses; the only remaining integer-only paths a
float could reach are the ones now refused (parameters, returns, call arguments and results), and a
`Branch`'s condition is always an `int` because C's conversion of a float condition to `!= 0` is the
front end's; the two `unreachable!` arms left in the encoder are the ones that cannot be reached and
whose message says why.

**Tests added for the defect this review fixed:** a `double` in the data section (whose address arrives
in `r8`, the case the ratchet cannot see) and a `double` read through a pointer, beside the eight float
cases already there.

**Measured after the review: 406 agreeing, 58 refused, 0 wrong** - unchanged.

177 unit, 23 parser, 20 CLI, 3 exe, 4 x64, 4 hostile, 6 C API, 4 bcc20-sys, differential (412 ran, 8
refused, 44 skipped, 0 timed out, 0 mismatched across three compilers), crt, pp gate, fuzz - green;
fmt and clippy clean.

### floating point, reviewed - 2026-11-16 - a load that read the wrong address

Review of `edfdfc4`.

**Found: a floating-point load or store through a base in `r8`-`r15` read or wrote `[rax]`.** The move
emitted the mandatory prefix and then the opcode, and never a REX byte - so the base register's high
bit was dropped, and an address computed into `r8` (which is what a global's address arrives in) became
`rax`. A global `double` read as garbage: `double g = 2.5; return (int)(g * 2.0);` answered 0. No
corpus program has a global `double`, so the 406-program ratchet had nothing to say about it, and the
hand tests all used locals whose addresses are `rsp`-relative. The rule is the one the integer path
already follows and this path forgot: **the REX byte comes after the mandatory prefix, and an address
base above `r7` is only nameable through it.**

**Found: the float scratch registers could be asked for a third one.** When a destination is memory its
spare is `xmm0`, and if the left operand was also spilled it had already been loaded into `xmm0` - so
the code asked for a third scratch register, of which there are two, and would have hit `unreachable!`
in a release build. The destination's spare *holding the left operand* is not a collision: it is the
accumulator, and now it is treated as one.

**Found: three arms that would have turned a class mistake into a wrong answer instead of a stop.** The
float helpers had `Location::Register(register) => register` arms - a general register's number used as
an XMM register's - added to satisfy the match checker while the class work was in progress. That is
exactly the shape of the bug that made a `double` reach `xmm2` from `rdx` earlier in the round. All
four helpers now have `unreachable!` arms with the reason, on both sides: a compiler bug should stop
the compiler, and a wrong register is the one outcome that a test with the right answer in it may not
catch.

**Checked and sound:** the two widths' prefixes (`F3` for `float`, `F2` for `double`) are right on every
arithmetic, move and conversion, and `ucomisd`/`ucomiss` - the one pair that takes `66` or nothing -
are the exception they are documented as; `setnp` for equality and `setp` for inequality, with the
ordering comparisons asked from the other side so a NaN fails them as C requires; the four conversions
carry their REX byte; a spilled double takes one eight-byte slot whichever width it holds; and a block
parameter's slot is read and written through the register file its type belongs to, which the earlier
mismatch in `cc50_corners` was.

**Measured after the review: 406 agreeing, 58 refused, 0 wrong** - the same numbers, with one defect
that no corpus program exercises now correct.

177 unit, 23 parser, 20 CLI, 3 exe, 4 x64, 4 hostile, 6 C API, 4 bcc20-sys, differential (412 ran, 8
refused, 44 skipped, 0 timed out, 0 mismatched across three compilers), crt, pp gate, fuzz - green;
fmt and clippy clean.

### stack arguments, reviewed - 2026-11-16 - a count that fitted in a byte, and a dead condition

Review of `39364ff`.

**Found: the outgoing-slot count was a `u8`.** It comes from the widest call in a function, and a call
with two hundred and fifty-three arguments would have wrapped it - a frame too small for the slots the
call writes, which is memory corruption rather than a diagnostic. The front end's token budget makes
that unreachable today, and the field is a `u32` now so that it stays unreachable for a reason the
code states instead of a reason a reader has to derive.

**Found: a condition that could never be false.** The frameless rule gained `outgoing_bytes == 0`, but
a function has outgoing slots only if it has a call, and the same rule already tests that. Removed,
with the invariant it rested on written where the remaining condition is.

**Checked and sound, because the layout is the thing that can go wrong:**

- The frame is now shadow space, outgoing slots, saved registers, the lowering's frame, spills,
  staging - and every offset that moved (`body_base`, `spill_base`, `staging_base`, `saved_base`) moved
  with it, because the prologue and the epilogue both read `saved_base` from one field.
- The two halves of the ABI's frame layout are in one file and each names the other: the caller writes
  its fifth argument at `[rsp + 32]`, and the callee reads it at `[rsp + frame + 40]`, the forty being
  that same shadow space plus the return address the call pushed. `s2` - seven arguments summing the
  first and the last - is the test that would fail if either half drifted.
- Stack arguments are written before the register ones, so a register move cannot destroy a value that
  has not been stored; and the staging path, when a collision forces it, stages the four register
  arguments only, because the others are already in memory.
- `arguments_collide` treats an argument in an argument register at a position of four or more as a
  collision, which is conservative and safe: it can only send a call down the staging path.
- A narrow stack argument is canonicalised on arrival like a register one, and a `long long` on the
  stack keeps its eight bytes; the mixed-size case is a test.

**What is not tested, and why:** a call from this back end *into another compiler's function* with
more than four arguments. The corpus compares whole programs and the CRT's five-argument functions are
variadic or absent, so the agreement rests on the ABI document rather than on a measurement. Recorded
here because it is the one claim in this round that a ratchet cannot check.

**Measured after the review: 396 agreeing, 68 refused, 0 wrong** - unchanged.

177 unit, 23 parser, 20 CLI, 3 exe, 4 x64, 4 hostile, 6 C API, 4 bcc20-sys, differential (412 ran, 8
refused, 44 skipped, 0 timed out, 0 mismatched across three compilers), crt, pp gate, fuzz - green;
fmt and clippy clean.

### the data section, reviewed - 2026-11-16 - a section that was writable and not readable

Review of `5b268bd`.

**Found: the data section's flags left out `IMAGE_SCN_MEM_READ`.** `DATA_FLAGS` was
`0x0050_0040 | 0x8000_0000` - initialized data, aligned to sixteen, writable - and the read flag was
not there. Nothing failed, because a linker maps an object's data section read-write whatever the
object says, and the 383-program ratchet read every global it compiled. That is the shape of defect
this review exists to find: a constant that is wrong, a test suite that cannot see it, and a comment
next to it that had already stopped describing what the value meant. It is now `0xC050_0040`, and the
comment names the flags and says why the missing one is easy to lose.

**Found: "does this file have a `.data` section" was keyed on the bytes rather than on the contents.**
`has_data` tested `!data.bytes.is_empty()`, so a defined object of no bytes with a name would have
produced a symbol pointing at a section the file did not contain - a malformed object rather than a
refusal. C cannot spell such an object (the front end refuses `int a[0]`), which is why the test suite
did not find it either; the condition now covers the symbols, and the reason is written down.

**Found: a defined object of incomplete type was silently demoted to a declaration.** The helper that
laid objects out returned `None` for "no initialiser and no size", which the caller read as "not
defined", so the object would have vanished from the section and appeared as an undefined symbol -
the object compiled, and the linker would have refused it in its own words. It is a `NotYet` with the
object's span now, which is what the rest of this back end does with something it cannot express. The
helper went with it: the two cases - initialiser, or zeroes for its type - are the loop's two arms.

**Checked and sound:** the file's layout arithmetic (`header -> text -> padding -> relocations -> data
-> symbols`) matches the section headers' pointers, which `objdump -h` confirms section by section;
data symbols are `STATIC` for a name the source cannot spell (`.str0`, `f.n`) and `EXTERNAL` for a
file-scope one, and both resolve through the same `REL32`; a declared-only global becomes an undefined
symbol and never a section entry; and a function's address as a value still refuses, with the message
now naming the function rather than just the expression.

**Code size after the review: unchanged**, and the ratchet still at 383 agreeing, none wrong.

177 unit, 23 parser, 20 CLI, 3 exe, 4 x64, 4 hostile, 6 C API, 4 bcc20-sys, differential (412 ran, 8
refused, 44 skipped, 0 timed out, 0 mismatched across three compilers), crt, pp gate, fuzz - green;
fmt and clippy clean.

### round four, reviewed - 2026-11-16 - an invariant that held by luck, and a test whose answer I got wrong

Review of `2fe2efa`.

**Found: the `rax` preference was sound by accident.** A returned value may live in `rax`, and the
check that decided it looked for the three operations that write `rax` - a comparison, a division, a
call - and for a spilled result, which is computed in `rax` before it is stored. It did not look at
**terminators**, and `return x` writes `rax` too. So a value returned late could be given `rax` while
an earlier `return` of something else destroyed it. The check now reads the terminator as well and
allows only the value's *own* return; the mutation that survives in the corpus is that a `Return` of
another value is a clobber.

Why no corpus program found it: every conditional branch is preceded by a comparison, so a value that
is live across a branch has a `Compare` in its interval and the preference refuses. The unsoundness
needs a `Return` reachable with no comparison in the interval, which today's front end cannot emit -
which is exactly the kind of guarantee that a later change to how conditions are lowered would break
silently. Ten lines make it not depend on that.

**Found: my own test was wrong, and the compiler was right.** The program I wrote to demonstrate the
hazard - `int f(int c, int a) { int ok = c == 0; int v = a + 1; if (ok) return 1; return v; }` called
with `f(0, 4)` - returns `1`, not `5`, because `ok` is true when `c` is zero. The reference compiler
said so too, and I had read "ours=1, expect 0" as evidence of a compiler bug when it was arithmetic
in the test. The case is now `f(1, 4)`, which does take the path that returns `v`, and it sits in
`tests/x64.rs` with the other constructs - a test whose expected value came from working the program
out by hand instead of from the thing being tested.

**Checked and sound:** `avoids_rax` is called after the scan, so it sees final spill decisions;
`block_of_position` makes position lookup a table read rather than a scan of every block (the first
version was O(blocks) per position, which on a function with many returns would have been O(n^3));
`intervals_of` is filled by `intervals`, so a returned value always has a range; and the prologue's
`saved` list is filtered after the pass, so a register whose only value moved to `rax` is no longer
saved - the saving the move was supposed to buy.

**Code size after the review: 30 564 bytes, unchanged** - the fix is a correctness guard, not a
change of policy - with the ratchet at 302 agreeing and none wrong.

177 unit, 23 parser, 20 CLI, 3 exe, 4 x64, 4 hostile, 6 C API, 4 bcc20-sys, differential (412 ran, 8
refused, 44 skipped, 0 timed out, 0 mismatched across three compilers), crt, pp gate, fuzz - green;
fmt and clippy clean.

### round three, reviewed - 2026-11-16 - a condition that hid the four things it tested

Review of `43dc3db`.

**Found: the frameless rule was written as an equality a reader had to decode.** `raw == 32` meant
"nothing above the shadow space", which is true exactly when the function has no call, no spill, no
block parameter and no local - four things, none of them named, and the `32` had to be recognised as
the shadow space for the condition to be readable at all. It is now those four things, spelled out
(`frameless`), and the shadow space appears once, where the frame is computed. The behaviour is
identical and the measurement says so: 30 848 bytes either way.

**Checked and sound, because these three changes each touch a data path:**

- *A comparison's answer in place.* `setcc` takes a byte register, so a destination in `spl`-`dil` needs
  a REX prefix, one in `r8`-`r15` needs its high bit in REX.B, and the widen afterwards is `movzx`
  from the same register. The destination can also be a spill slot, in which case it is `rax`, which
  is never allocated. And `setcc` clobbering an operand's register is safe: the comparison is that
  operand's last use, which is why the allocator was free to give the destination that register.
- *Direct argument moves.* The collision question is not "is an argument in another argument's
  register" but "is any argument in a register this call is about to write, other than at its own
  position" - which also catches a spilled argument whose register write would destroy an argument
  still sitting there, because that argument is itself "in someone else's register" and the scan sees
  it from its side. The direct path is a chain of `read_into`s, each at most one move, and a spilled
  argument is read straight out of its slot.
- *No frame at all.* A leaf with no locals and no spills cannot reference `[rsp]`: block parameters
  and escaped variables are the lowering's frame, spills are the allocator's, and the staging area
  belongs to a call. `ret` with `rsp` untouched is what a leaf with nothing on the stack wants, and
  the corpus ratchet ran 302 programs through it.

**Found and removed in passing:** `encode.rs` still computed whether a function makes a call, while
`lower` had decided it a commit earlier - the previous change replaced one use of the local scan and
left the scan itself. The reviewer's own script had failed an assertion before writing that file, and
the assertion failure was the only thing that was noticed at the time.

**Code size after the review: 30 848 bytes, unchanged**, ratchet at 302 agreeing and none wrong.

177 unit, 23 parser, 20 CLI, 3 exe, 4 x64, 4 hostile, 6 C API, 4 bcc20-sys, differential (412 ran, 8
refused, 44 skipped, 0 timed out, 0 mismatched across three compilers), crt, pp gate, fuzz - green;
fmt and clippy clean.

### a leaf function's registers, reviewed - 2026-11-16 - a scratch script in the repository, and two names for one set

Review of `965b394`.

**Found: a 146-line scratch script committed to the repository root.** `.leaf.py` was the edit script
for the change itself - written to a file so the heredoc could not truncate it, then meant to be
deleted. Its first run failed an assertion, `&& rm` did not run, and the next attempt used a different
file name, so this one was still sitting there when `git add -A` swept it in. Two things follow: the
file is gone, and `.gitignore` now says `.*.py` - the habit of writing a script to a file before
running it is a good one, and this is the second time in the project's history that the tidying-up
step was the one that got skipped.

**Found: two names for one set.** `CALLER_SAVED_WIDE` (four registers) and `CALLER_SAVED` (two) read
as if they described different kinds of register, and neither name said what actually separates them:
the same four registers, of which the last two are safe only when the function contains no shift and
no division. There is now one `CALLER_SAVED` whose order means something - `r8` and `r9` first,
because they are safe in every such function - and the narrow case is `[..2]`, which says "the first
two of the same list" rather than naming a second list that could drift.

**Checked and sound:** `rax`, `rcx` and `rdx` are written by the encoder in exactly the places the
register plan accounts for - a comparison's low byte, a shift's count, a division's two halves, the
return value and the spare a spilled result is computed in - and each of those places was read
against the set it can appear in; `rcx`/`rdx` are scratch *only* in the set that allocates neither, so
no value can be clobbered by them; a function of three or more arguments never gets a caller-saved
set, which is the invariant the two corpus failures established, and both shapes are now named tests;
and `Op::Incoming` reads its argument register before writing anything, so the wide set's
`rcx`/`rdx` scratch pair cannot collide with an argument.

**A smaller thing left alone, with the reason:** a leaf function still reserves the 32 bytes of shadow
space it will never use. It costs stack, not instructions, and the frame is rounded to sixteen either
way; changing it would be a line that buys nothing measurable.

**Code size after the review: 34 632 bytes, unchanged**, with the ratchet at 302 agreeing and none
wrong.

177 unit, 23 parser, 20 CLI, 3 exe, 4 x64, 4 hostile, 6 C API, 4 bcc20-sys, differential (412 ran, 8
refused, 44 skipped, 0 timed out, 0 mismatched across three compilers), crt, pp gate, fuzz - green;
fmt and clippy clean.

### back-x64 shrinking, round one, reviewed - 2026-11-16 - a knob nobody measured, and one argument nobody read

Review of `c2cb1e2`.

**Found: two pieces of unrequested generality, one of which I had just written.** `CHEAP_REGISTERS`
was a threshold in the allocator saying a hint may only claim a register that has not been saved yet
while fewer than three registers are in use - a heuristic with a justification in a comment and no
measurement behind it. Deleting it changed the corpus's code size by **zero bytes**, which is the
measurement it never had: a knob nobody turns is dead code with a name. `in_place` was a method on
the encoder taking a `_commutative` parameter it ignored, a leftover from a draft; the callers now
ask `is_in` directly and the commutative decision lives in the one place that acts on it.

**Found: a latent trap in the search for a coalescing partner.** `self_register` read the location
table, which is filled with `rax` before the scan; a value that nothing reads keeps that filler, so
the search could have returned `rax` as "where the hint lives". It cannot match a free register today
because `rax` is not allocatable, but that is a property of a *different* table holding. It now
filters by the allocatable set, and says why. The same reasoning tightened the scratch allocator: it
used to hand back a register it was already holding when an operation asked for a third one, which
would have clobbered a live operand. That case is unreachable by argument - no operation holds more
than two - but it is not unreachable by construction, and `unreachable!` with the reason beats
silently corrupting a register if a later operation is written differently.

**Measured rather than assumed: the liveness pass is quadratic in live values and I left it there.**
A synthetic function with 300 values live across 2 600 blocks takes 2.5 s in a debug build and about
1 s in a release one, and the block budget (4096) bounds it before it can become a cliff. Written
down instead of fixed, because the fix is a `HashSet` in a hot loop that costs readability and buys
nothing measurable at the bound that exists.

**Checked and sound:** the hint can only claim a register whose value is dead there, because an
interval releases its register only when it ends at the position the new one starts, and x86 reads
an instruction's operands before it writes the destination; a commutative operation may compute into
either operand's register, a non-commutative one only into the left operand's, and the case where the
destination's register holds the right operand of a non-commutative operation cannot collide with a
spilled operand's scratch (argued in the review, and the `unreachable!` now states the invariant the
argument rests on); `saved` collects registers reached by either path, so the prologue saves what the
epilogue restores; and the module docs now describe the encoder as it is rather than the naive form
this commit removed.

**Code size after the review: 40 716 bytes, unchanged** - as it should be for a cleanup - with the
302-program ratchet still at 302 agreeing and none wrong.

177 unit, 23 parser, 20 CLI, 3 exe, 4 x64, 4 hostile, 6 C API, 4 bcc20-sys, differential (412 ran, 8
refused, 44 skipped, 0 timed out, 0 mismatched across three compilers), crt, pp gate, fuzz - green;
fmt and clippy clean.

### back-x64, the first slice, reviewed - 2026-11-16 - one real defect, and four copies of one fact

Review of `a62d820`.

**Found: a function's address read by anything besides a call was silently lowered to zero.** The
direct-call path recorded every `GlobalAddress` used as a callee, then emitted a placeholder `0` for
that value so it had a definition. If the same address was also compared, stored, or passed
somewhere, that `0` became the program's address - a call through zero, or `f != 0` answering false,
with no diagnostic. The rule is now "a direct call when *every* read of the address is a call that
names it", and anything else refuses with a span. The first attempt at the fix ("exactly one read")
was too strict and cost a corpus program that calls the same pointer twice; the corpus ratchet said
so in one run, which is the harness paying for itself again. Two tests pin both sides:
`a_function_address_that_never_escapes_is_still_a_direct_call` (a promoted pointer is still direct)
and `a_function_address_read_as_a_value_is_refused` (both escape shapes refuse, and level 0 accepts
them, so the test cannot pass by proving nothing).

**The same fact in four places.** The number of integer argument registers was a table in `lower.rs`,
a table in `encode.rs`, a `len()` check, and a field on `MachineFunction` that nothing read (with an
`#[allow(dead_code)]` explaining that it "keeps the table in one place" - it kept it in two). There
is now one `ARGUMENT_COUNT` in the module that owns the ABI shape, the encoder's table is sized by
it, and the lowering's check asks it. The dead field, the dead `Placed::code_len`, and the dead
`params` field went with it.

**A quadratic in the allocator.** `Positions::new` asked "did this block define this operand?" by
scanning the block's instructions, once per operand: O(ops²) per block, on input a hostile program
controls. It is one `HashSet` per block now. This is the same shape of bug the previous review found
in `mem2reg`'s escape set; it is worth naming twice because a compiler's own data structures are
where the budgets do not reach.

**A comment that said the opposite of the code.** The module header claimed "the encoder never sees a
virtual register at all". It does - through `Allocation::location` - and what it never sees is an
*unallocated* one. Corrected, along with `Allocation`'s own doc, which now says that a value nothing
reads (a call target's address) has no interval and its entry there must not be read.

**Checked and sound:** the entry block's parameters no longer reserve frame slots that no edge can
write (nothing can jump to the entry block); `term_operands` is now `cir`'s one definition of "the
values a terminator reads", used by the verifier, the passes and this back end rather than copied a
fourth time; the frame's alignment rule (`round_up(...) + 8`) is right for a call from any frame
size; and the eleven encoder sites that name a register field were re-read against the REX bit each
one sets.

**Measured after the review: 302 corpus programs agree through both back ends, 162 refused, 0
wrong** - the same numbers, with one defect that no corpus program happened to exercise now refused
instead of miscompiled.

177 unit, 23 parser, 20 CLI, 3 exe, 4 x64, 4 hostile, 6 C API, 4 bcc20-sys, differential (412 ran, 8
refused, 44 skipped, 0 timed out, 0 mismatched across three compilers), crt, pp gate, fuzz - green;
fmt and clippy clean.

### M10 reviewed - 2026-11-22 - a table that outlives the memory it describes

The frame-table commit, reviewed. Its unwind format is right and its oracle passes; what the review found is
one malformed table and one dangling registration.

**`UWOP_SAVE_XMM128_FAR` was emitted with one slot and the wrong units.** The far form takes *two* slots and its
offset is scaled by sixteen, like the small form; the encoder wrote one slot holding a raw byte offset. It fired
whenever a float save landed at 8 mod 16 - which is whenever a function's outgoing slot count was odd - so a
function that both calls with many arguments and keeps an XMM register across the call had an unwind table the
unwinder would misread. **The fix is not the far form: it is the alignment.** The float save area is now sixteen
bytes aligned, so the *small* form always applies, the far branch and its constant are gone, and the invariant is
"the offset is a multiple of sixteen" rather than "…unless". Cost: **8 bytes over 445 corpus objects** (202 230 →
202 238), measured by `git stash`.

**And `Loaded` freed memory that its frame table still pointed at.** `Drop` called `VirtualFree` and never
`RtlDeleteFunctionTable`, so after a `Jit` was dropped the process's unwind tables kept entries naming released
pages - read by the *next* exception anywhere in the process, which is the worst possible place to find out.
`Loaded` now holds the table it registered and deletes it before the release; the field's doc says why the
pointer is kept.

**Two small ones in the same commit.** `compile`'s single-path change left a redundant brace block around a
`match` (`let object = { match … }`), and the `Loaded` doc still described an allocation of two sections when it
now holds four.

**Measured after the review:** the oracle still passes on all three paths (a `longjmp` out of three compiled
frames returns 42 through the object writer at levels 0 and 2 and through the JIT); 443 corpus programs agree, 20
refused, 0 wrong; cJSON byte-identical to clang.

190 unit, 27 parser, 22 CLI, 4 exe, 5 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran, 5 refused,
44 skipped, 0 timed out, 0 mismatched) - green; fmt and clippy clean.

### M10 reviewed - 2026-11-22 - the release note was wrong about the one number nobody can edit later

The switch fix, reviewed, and the release it was published in.

**The switch fix holds, and it is now the same shape as every other construct.** `if`, `while`, `for` and `do` all
ask `current_is_open()` at the moment they need to link; the `switch` was the one that kept a block from earlier
and asked about *it* instead. Five shapes that the change touches were checked against clang and agree: a label at
the end of an arm reached by `goto` from an earlier arm, an arm that ends in a nested `switch` and falls through,
`continue` inside a `switch` inside a loop, an arm that ends in `?:`, and one that ends in `if`.

**And the tag was wrong.** Its message said "188 unit" while this commit has 190 - copied from an older audit entry
rather than measured. A tag message is the one place a number cannot be corrected quietly: everything else is a
commit away from being fixed, and the tag is what a reader quotes a year later. It was re-annotated and force-pushed
within the hour, and the number now comes from a test run at that commit.

**The version was the same class of fault, one level down**: `Cargo.toml` said `0.0.1` while the tags said `0.1.2`,
so `xcc --version` and `git describe` disagreed about which release the binary was. Both now say 0.1.3, and the tag
series resumes after five milestones of silence - a tag per milestone is the only map of which state was verified
when.

190 unit, 27 parser, 22 CLI, 4 exe, 5 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran, 5 refused,
44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong; fmt and clippy
clean.

### M10 - 2026-11-22 - the real code is a gate now

cJSON and miniz are vendored, unmodified, and both run on every test run: `tests/real.rs` compiles each with this
compiler's front end and own back end at `--opt-level=2`, links with this compiler's own linker, runs the result, and
compares its output with what a `clang` build of the same sources printed. **2.9 seconds for the pair** - cheap for the
only test that catches what 464 small programs cannot.

**Unmodified is the point, and the fixtures README says so**: a file that had to be patched for this compiler would be
a compiler bug, not a fixture. The one exception is stated where it lives - `miniz_export.h`, which upstream generates
from a CMake template and which is six lines of macros here, because a fixture that needs CMake is a fixture that needs
CMake. The licence notice at the top of every file is the upstream one, and the README records the upstream URL and the
date taken.

**The expected output is recorded, not compared against a live `clang`.** A test that shells out to another compiler
is a test that fails on a machine without one, and this suite skips the host linker by name already; the fixture README
says how the eight lines were captured, so a future reader can re-record rather than guess.

**Two things the test also covers by accident, and should**: `#include "cJSON.h"` needed the fixture directory through
`Compile::with_includes` - the documented embedder API, which the tests had never exercised - and the miniz driver
needs `-DMINIZ_NO_STDIO`, which goes through `Options::defines`, so the vendored code tests the API a host uses rather
than only the command line.

195 unit, 28 parser, 29 CLI, 10 exe, 2 real, 1 crt, 5 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran,
5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong, refusal
oracle 19/0; fmt and clippy clean.

### M10 - 2026-11-22 - wide literals were a wrong answer, and the linker had no alignment

Three of the five refused headers, and two bugs that had to come first.

**Wide literals were silently wrong.** `L"ab"` compiled and was emitted as its *narrow* bytes: `sizeof L"ab"` was 3
where the target's `wchar_t` makes it 6, `L'a'` was an `int` and not a `wchar_t`, and `wchar_t *s = L"ab"` read 25185
and then two bytes past the object. No test caught it because no header shipped it and no corpus program writes one.
Now `L'a'` has type `wchar_t`, and a wide literal is stored as UTF-16 code units - escapes give *code unit* values
(`L"F"` is one character), source text is decoded from UTF-8 (so a literal `é` is one unit, not the two bytes it
is written in), and a character above the basic plane is a surrogate pair. Measured against `clang` on five shapes:
`sizeof` of a string, of a character, of a wide character, an escape, and UTF-8 source - all identical.

**And the linker never aligned a unit's data.** Adding eight bytes to the C runtime broke `setjmp`: the program's
`jmp_buf`, `_Alignas(16)`, started at `.data + 8` and `__intrinsic_setjmp`'s `movdqa` faulted. The linker concatenated
units' data with no padding, and it worked until the bytes before it happened to leave the alignment right. A unit now
carries the strictest alignment any object in it asks for, and is placed there - and *the padding comes before the
base is computed*, which my first attempt got wrong: the symbols were placed eight bytes above their bytes.

**`fenv.h` ships**, and it needed one measured fact and one design decision. The fact: the eleven functions and their
values against the UCRT (`FE_UPWARD` is 0x200, `fenv_t` is eight bytes). The decision: `FE_DFL_ENV` is a *pointer to
an object*, not a `-1` sentinel, because this runtime's `fesetenv` dereferences what it is handed - so the runtime
captures the default with `fegetenv` before `main` runs, and `fesetenv(FE_DFL_ENV)` restores what the program started
in. A `clang` build of the same program restores `FE_TOWARDZERO`, because mingw's default object is not what C99 7.6p6
says it is; ours is, and the test says why the two differ. `feraiseexcept` and `feupdateenv` are the runtime's own -
no Windows runtime exports them - and are listed as unavailable to a *JIT*, with the reason.

**`wctype.h` ships**, all eighteen of its functions exported by the library under their own names, with `wint_t`,
`WEOF`, `wctype_t` and `wctrans_t` measured at the target's widths. `wctype("alpha")` and `wctrans("toupper")` work,
which is what those two opaque types are for.

**`wchar.h` does not, yet, and the measurement says why**: of its seventy-five C99 names, fifty-seven resolve and
eighteen do not - the nine wide `stdio` front ends (the UCRT keeps those in the program, as it does the narrow ones),
`fwide`, `mbsinit`, and the four `wmem*` functions. The next step is the runtime's, and PLAN.md carries the list.

195 unit, 28 parser, 29 CLI, 13 exe, 2 real, 1 crt, 5 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran,
5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong, refusal
oracle 19/0; fmt and clippy clean.

### M10 reviewed - 2026-11-22 - the strongest claim was the one not written down

The `mem2reg` fix, reviewed. The fix is right and the sweep around it came up empty - which is what made the finding
worth writing.

**Checked, and complete**: an escape is now any use of a slot's address that is not a load, a store, or the next
bitcast - instructions *and* terminators. `address_chains` follows only `Slot` and `Bitcast`, so `&s.member` is not in
a chain at all: a member's loads and stores go through a `Field`, never through the slot, and the slot is left alone.
Four shapes were measured against `clang` rather than argued about: a conditional over two locals' addresses, over two
members' addresses (`struct S` with `*p += 10`), over array elements, and a `switch` whose arms take an address. All
agree.

**And the README still stopped at the previous library.** The real-code evidence in it ended with cJSON, one paragraph
after a session that made miniz - nine thousand five hundred lines - deflate, inflate, write a zip and read it back.
The strongest evidence the project has was the claim not written down, so the linking row now names both, with what each
one measurably does: cJSON byte-identical to a `clang` build, miniz's stream readable by real zlib and its archive
extracting byte-identical.

**No instrumentation left behind**: the `XCC_TIME` and `XCC_DUMP_CIR` hooks that found the hang and the
broken pass were temporary, and `grep` says so - a debug switch left in a shipping compiler is a second, undocumented
interface.

195 unit, 28 parser, 29 CLI, 10 exe, 1 crt, 5 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran, 5 refused,
44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong, refusal oracle 19/0;
cJSON identical, miniz zips and round-trips; fmt and clippy clean.

### M10 - 2026-11-22 - promotion looked at instructions and not at edges

`miniz_zip.c` - the remaining 4 911 lines of miniz - needed one include and found one pass bug.

**The include**: `<sys/stat.h>`, which is POSIX and which this compiler does not have. `MINIZ_NO_STDIO` is the
library's own switch for a build with no file APIs, and with it the zip code works entirely in memory - which is the
honest configuration for a compiler whose headers are C99's.

**The pass bug** was `mem2reg`: `int *p = c ? &a : &b;` threads those addresses through the join as **block
arguments**, and the escape analysis that decides which slots may be promoted walked the *instructions* of each block
and never its **terminator**. So a slot whose address flowed into a join was promoted anyway, its `Slot` instruction was
deleted, and the block argument went on naming a value nothing defined:

```
internal: `mz_zip_writer_add_mem_ex_v2` is not valid: v21 is used but no instruction defines it
```

Finding it was a measurement, not a reading: the verifier's five hooks were labelled to say which pass broke the
function (`mem2reg`), and the cir dumps before and after showed `Value(21)` as a `Slot` that two jumps passed as an
argument before the pass and that nothing defined after it. Six lines reproduce it - `int *p = c ? &x : 0;` - and that
is the test now. `miniz_zip.c` compiles, and a driver writes a zip archive in memory and extracts the file back
byte-identical: 176 bytes of archive, 39 bytes of file, `1 file(s) ... identical`.

**What the fix says**: an escape is a use, and a terminator is a use. The check reads `term_operands` - the same
function the verifier and both back ends read - so the answer cannot disagree with what the rest of the compiler thinks
an operand is.

195 unit, 28 parser, 29 CLI, 10 exe, 1 crt, 5 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran, 5 refused,
44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong, refusal oracle 19/0;
cJSON identical, miniz round-trips and zips; fmt and clippy clean.

### M10 - 2026-11-22 - the hang was a list pretending to be a set

The level-2 hang on miniz's inflater, found and fixed. It was not the back end's size limit and not the frame-slot fix:
it was **liveness**.

**Where it was**, measured rather than guessed: `lower` finishes in 9 ms for the file's 26 functions; inside
`coff::assemble` every function allocates in microseconds except `tinfl_decompress`, which names 2 992 values; inside
`allocate`, `Positions::new` takes 1.8 ms and then `Positions::intervals` never returns.

**What it was**: `liveness` - the textbook backwards fixpoint - held its live sets in `Vec<VReg>` and compared them with
`contains`. Union of successors, difference of definitions, and the change test were all linear scans, twice per value
per block per iteration: quadratic in the number of values, on a function with three thousand of them and a hundred
and forty blocks. The sets are `BTreeSet<VReg>` now, which is what the algorithm says they are. `miniz_tinfl.c`
compiles in **1990 ms**, 82 ms of it the back end, against **over 30 minutes** before - and no output at all.

**The fix that was waiting behind it**: with liveness fast, the frame slot can finally be emitted where it belongs -
the entry block, because a slot's address is a property of the frame and not of the block the declaration stood in.
`tinfl_decompress`'s `case` labels sit inside nested `do { } while (0)` blocks, so a slot created in one case's block
was read in another, and the verifier refused the function as not dominating. That refusal was the honest error
standing in for a hang; both are gone, and the dump's slot order is declaration-reversed as a consequence, which the
`cir` test now says out loud.

**Measured end to end**: `miniz.c`, `miniz_tdef.c` and `miniz_tinfl.c` compile and link with our linker, and
`roundtrip.exe` compresses a 209 793-byte file to 127 795 bytes and inflates it back **byte-identical**, in one
process; the same compressed stream is read correctly by real zlib. A test pins the state-machine shape that broke it.

195 unit, 28 parser, 29 CLI, 9 exe, 1 crt, 5 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran, 5 refused,
44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong, refusal oracle 19/0;
cJSON identical, miniz round-trips; fmt and clippy clean.

### M10 reviewed - 2026-11-22 - a result without its method

The miniz commit, reviewed. The subscript fix holds and the `-E -o` fix holds; the review found one thing that made a
published result irreproducible, and one leftover.

**The miniz result did not say how to reproduce it.** "`miniz.c` and `miniz_tdef.c` compile and compress a 209 793-byte
file to 127 795 bytes that zlib reads back exactly" is true - with three defines that turn off every part of the library
that calls `tinfl_decompress`, the function the verifier refuses. Without them the link fails at
`undefined symbol tinfl_decompress`, and a reader following the sentence would conclude the claim was wrong. A result
recorded without its method is a result that expires the moment somebody tries it.

**`print_spellings` was a name around a `print!`.** The `-E -o` work split the text-building out of the printing so both
paths share one writer - and left the one-line wrapper that calls the new function. One caller, one statement: the call
site says `print!("{}", spellings(...))` now, and the wrapper is gone.

**Checked, and already right**: `xcc -E a.c b.c -o x.i` refuses rather than writing the last source over the first -
the "one `-o`, one source" rule is earlier than the `-E` branch; `-E` with no `-o` still prints; a preprocess that
fails writes no partial file.

195 unit, 28 parser, 29 CLI, 8 exe, 1 crt, 5 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran, 5 refused,
44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong, refusal oracle 19/0;
cJSON identical, miniz deflate verified by zlib again; fmt and clippy clean.

### M10 - 2026-11-22 - miniz: one bug fixed, one diagnosed, one found

The next real program: miniz's deflate and inflate (9 500 lines across six files, MIT). It found three front-end bugs in
ten minutes, which is the argument for real code.

**Fixed: a subscripted string literal was taken for an initialiser.** `initializer()` looked at the first token, saw a
string literal, and called the character-array path - so `unsigned a = ""[code - 16];` lost its subscript
and then complained that a `char *` did not fit an `unsigned`. miniz's deflate is written exactly that way, in
`TDEFL_PUT_BITS(..., ""[code - 16])`. The literal path now applies when the literal is *not* followed by
`[`: an expression beginning with a literal is an expression. `char *p = "abc"` and `char s[] = "abc"` are unchanged,
and a test pins all four shapes.

**Diagnosed, not fixed: a frame slot is a value with a block position, and it should not be.** `tinfl_decompress` is a
state machine whose `case` labels live *inside* nested `do { } while (0)` blocks; a `Slot` created in one case's block
is loaded in another, and the verifier refuses the function: `v739 is defined in block 78 and used in block 83 which it
does not dominate`. Moving the `Slot` instruction to the entry block - which is where a frame slot really lives - makes
the dominance valid and then **hangs our level-2 back end** (over 30 minutes, against 2 seconds at levels 0 and 1). A
synthetic function with 400 locals in one block compiles in under a second, so the hang is not "many slots in one
block" but something about that function's shape at level 2. Reverted to the honest error; PLAN.md carries the
reproducer, the measurement and the two candidate fixes.

**Found: `-E` ignored `-o`.** `xcc -E t.c -o t.i` printed to stdout and wrote no file, so a build system saw success
and no output. `-o` names the file every other mode's `-o` names; it does now, and `-E` with no `-o` still prints.

**What does work**: `miniz.c` and `miniz_tdef.c` compile at `--opt-level=2`, link with our own linker, and **compress a
209 793-byte file to 127 795 bytes that real zlib reads back exactly** - with three defines that turn off the parts of
the library that call the inflater (`-DMINIZ_NO_INFLATE_APIS -DMINIZ_NO_ZLIB_APIS -DMINIZ_NO_ARCHIVE_APIS`), because
`tinfl_decompress` is the function the verifier refuses - `uncompress` returns `Z_OK`, the length
matches, and the bytes match. That is a third-party compressor built end to end by this compiler and checked by a
compressor written by somebody else.

195 unit, 28 parser, 29 CLI, 8 exe, 1 crt, 5 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran, 5 refused,
44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong, refusal oracle 19/0;
cJSON identical; miniz deflate verified by zlib; fmt and clippy clean.

### M10 reviewed - 2026-11-22 - a depfile that named two files with one space

The defines and the depfile, reviewed. Both work; four things about the second were wrong.

**`dedup` without a `sort` is not a set.** The link path built its dependency list, appended the files read, and called
`dedup` - which removes *adjacent* duplicates only, so the same header read by two units stayed in the list twice.
The `-c` path sorted first and did not have the bug. Two callers, two answers, one of them wrong: the helper now takes
an iterator and does the sort, the dedup and the shipped-header filter for whoever asks.

**A link silently ignored `--depfile`.** The flag was checked against `--run`, `-E` and `--explain`, and the link path
runs *before* the per-file loop that writes the rule - so `xcc a.c b.c --depfile t.d` wrote `t.exe` and no rule, and
said nothing. An option that is accepted and ignored is worse than one that is refused. A link names a file a build
system makes as much as `-c` does, so it writes the rule: `t.exe: a.c b.c`, with the C runtime excluded because it
lives inside the compiler.

**`-c` with two sources and one `--depfile` wrote the last rule over the first.** gcc writes one `.d` per object for a
reason. Refused with the reason, rather than quietly keeping whichever source came last.

**Two comments describing a compiler that no longer exists**: `--run`'s help still said "`#include` is not resolved
here (there are no headers until M8)", two milestones after the headers started shipping inside the binary; and
`is_shipped` said "a header this compiler ships", while the table it reads also holds the five it refuses - neither is
a file a build system can rebuild, which is the property the depfile cares about.

195 unit, 27 parser, 28 CLI, 8 exe, 1 crt, 5 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran, 5 refused,
44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong, refusal oracle 19/0;
cJSON plus its driver still byte-identical; fmt and clippy clean.

### M10 - 2026-11-22 - the JIT was already right, and the README was not

Two things the command line promised and did not keep, and one it denied that it could do at all.

**`--run` refused `-D` because of a comment that had stopped being true.** The CLI said the JIT "compiles with
the defaults", and the message ended in "yet" - but `Jit` has taken [`Options`] all along, through the `unsafe`
constructor that also wants host symbols. A host that only wanted `--std` and `-D` to mean what they mean in a compile
had to reach for `unsafe` to get it, so the safe way in is now `Jit::configured(&options)`, and the CLI's refusal is
narrowed to the one thing that is still true: `-I` needs host files, and the JIT reads the headers inside the binary.

**And the README claimed a bug the JIT does not have.** The last commit wrote that two sources in one `Jit` that each
define `static int helper(void)` "name the first one". Measured through the API: at `--opt-level=2` each keeps its own
(`a() = 2`, `b() = 200`), and at levels 0 and 1 `add_source` refuses the second with
`DuplicateDefinition("helper")`. Correct where it can be, loud where it cannot - the line was a limitation I asserted
instead of one I measured.

**`--depfile` names what a build system can rebuild.** `xcc -c t.c --depfile t.d` writes `t.obj: t.c own.h`, with the
files read that exist as files: the shipped headers are filtered out, because a dependency inside the compiler binary is
not a dependency `make` can rebuild - naming one sends `make` looking for a `stdio.h` that is on no disk. The rule
lives with the headers (`is_shipped`), the filtering with the compilation (`Artifact::dependencies`), and the CLI only
prints what it is handed. Spaces and `$` are escaped, since a depfile with an unescaped space in a path is a depfile
that silently names two files.

**Review of my own patch caught three things**: a `README` insertion that landed in the *linking* row and described the
depfile as part of linking; a `dependencies()` whose loop a string-slice edit had swallowed, leaving `target:` and a
newline as the whole rule; and the two escape sequences that the editing shell ate on the way in. The suite caught the
second; the first and third were caught by reading what was written.

195 unit, 27 parser, 27 CLI, 8 exe, 1 crt, 5 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran, 5 refused,
44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong, refusal oracle 19/0;
cJSON plus its driver still byte-identical; fmt and clippy clean.

### M10 reviewed - 2026-11-22 - the other three quarters of the same bug

The literal fix only fixed literals. The commit's own README note said a `static` *function* would still collide, and
the review asked the obvious follow-up question - what about a `static` *object*? Measured, both were live silent wrong
answers:

```
two units, each `static int helper(int)`      ours: a: 2   7    clang: a: 2 500
two units, each `static int counter = ...`    ours: b=11       clang: b=500
```

The second unit called the first unit's helper and read the first unit's counter. The knowledge was in `cir` for one
kind and missing for the other: `cir::Global` had an `external` field that the parser always set to `true`, and
`cir::Func` had no field at all. Both now carry the fact from the declaration's `static` specifier, the object writer
marks those symbols with static storage class, and the linker keys them per unit - one `UnitLocals` per unit holding
its functions and its objects, which replaced the `(usize, String)` tuple key and with it an allocation per lookup.

**Three quarters of one bug were three separate quiet failures**, which is the argument for fixing a class and not an
instance: the string-literal case was found by a real program, and the other two by asking "what else is keyed by a
name that is only unique in its unit?"

**And the fix broke `.pdata`, which is how the panic surfaced.** A `static` function is not in the program's global
function map, so `exception_table`'s `expect("a described function is a function of the program")` panicked the moment
cJSON - full of statics - was linked again. The unwind entry now records the unit it came from, because that is what
makes its name resolvable; the JIT's single image fills it with zero and says why.

`--run` still has the old behaviour - several sources in one `Jit` share one symbol table, so two of them may not each
define `static int helper` - and the README says so instead of the line that had claimed the linker could not do it.

The CLI test now links the two `static` cases and asserts clang's numbers; cJSON plus the driver is byte-identical
again, which is the check that mattered.

195 unit, 27 parser, 26 CLI, 8 exe, 1 crt, 5 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran, 5 refused,
44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong, refusal oracle 19/0;
fmt and clippy clean.

### M10 - 2026-11-22 - two units, one literal

The first real program built and run by `xcc` alone: cJSON (3 206 lines, MIT) and a driver over it, compiled and
linked with no host tool. It parses, walks and round-trips a JSON file - and printed **`"version":,`**, a number
missing from the output, where `clang` printed `0.1`.

The bug was not in cJSON and not in the number formatting. `cJSON_PrintUnformatted` produced `[y]` for a number whose
`valuedouble` was correct, and the instrumented build showed the *format string itself* was wrong - it printed the
driver's format, not its own. **A program's string literals are named `.str0`, `.str1`, ... and every unit numbers them
from zero**, so their names collide across units while meaning different bytes; the linker keyed every object by name
alone and `or_insert` kept the first, so the second unit's literals all resolved into the first unit's data.

Twenty lines reproduced it, and now pin it: `a.c` printing `"alpha"` and calling a function in `b.c` that prints
`"bravo"` printed `a=alpha` twice.

**The fix is the flag that was already there.** The object writer has always marked a unit's own objects with static
storage class - `DataSymbol::external` - and *nothing read it*: the README even said "every object claims external
linkage, which is wrong for a `static` and harmless only because nothing reads the flag". The linker reads it now: a
non-external object is keyed by `(unit, name)`, and a relocation is resolved from inside the unit that has it, its own
objects first. Two units can each have a `.str0` that means their own bytes. (Functions are still keyed by name alone:
`Placed` carries no such flag, so two units with a `static` helper of the same name would still collide - the same
class of bug, one layer up, and stated rather than left to be discovered.)

**Measured after the fix**: cJSON plus the driver, byte-identical to `clang` on both a small JSON file (13 lines) and a
nested one (1 328 bytes of output), and the same `not JSON` answer on a malformed input. A no-argument run says so and
exits 2.

195 unit, 27 parser, 25 CLI, 8 exe, 1 crt, 5 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran, 5 refused,
44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong, refusal oracle 19/0;
fmt and clippy clean.

### M10 reviewed - 2026-11-22 - the comment I "corrected" was the load-bearing one

The ten-names commit, reviewed - and the review's first act was to break `setjmp`.

**`setjmp`'s second argument is not decoration.** The header's comment said the C library's entry point takes
"the frame it should stop at" as its second argument, and the header passed `0` there. I read that as a leftover of a
different library and simplified the declaration to C's own one-parameter shape - `_setjmp(environment)`. It compiles,
it links, and it faults inside `RtlUnwindEx` with `0xC00000FF`, because the unwinder is handed whatever the second
register happened to hold. Restoring the argument made the program print `back with 7` again. So the original comment
was right, the "correction" was an assumption dressed as a review, and the comment now carries the measurement that
settles it - a reader who wonders, as I did, will find the answer and the experiment rather than a fixed conclusion.

**The header was missing all seven `v*` functions.** `vprintf`, `vfprintf`, `vsprintf`, `vsnprintf`, `vscanf`,
`vfscanf` and `vsscanf` are C99 7.19.6.8, 7.19.6.9 and 7.19.6.12-14, and the runtime source already defined six of
them - but `stdio.h` declared none, so a program could not call the function whose example the standard gives. They are
declared now, `vscanf` is written, and the new test is that example: a program's own `error(...)` forwarding its
`va_list` into `vfprintf`, and its own `shout(...)` over `vsnprintf`. Two layers of `...` in one program, checked
against `clang`'s output - identical on stdout and stderr.

**One dead branch in the new gate.** `let Some(...) = import_for(name) else { continue }`, with the comment "the
program defines it" - but the loop walks the runtime's own list, whose every name `import_for` answers for. A branch
that cannot be taken is a branch that cannot be tested; it is an `expect` now, which says the invariant out loud.

**And a stale row in `C99-SUPPORT.md`**: stdio was still `partial (M8)` and still said `snprintf` was resolved by no
runtime, two milestones after the runtime started carrying it. It says `yes` and names the layer that makes it so.

195 unit, 27 parser, 24 CLI, 8 exe, 1 crt, 5 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran, 5 refused,
44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong, refusal oracle 19/0;
fmt and clippy clean.

### M10 - 2026-11-22 - every declared name links, and the gate that first said yes to everything

Ten of the 178 names the headers declare had no home in a link. Measured against `ucrtbase.dll`'s export table and
closed, in four shapes:

- **`scanf`, `fscanf`, `sscanf`** - the scanning front ends are the *program's* half of the UCRT, exactly as `printf`'s
  were, so the runtime carries them over `__stdio_common_vfscanf` and `__stdio_common_vsscanf`.
- **`atexit`** - the library exports `_crt_atexit`, which takes the same function pointer.
- **`_setjmp`** - the library's own name is `__intrinsic_setjmp`, and it has to be the library's function: `setjmp`
  saves the *caller's* frame, so a wrapper written here would save the wrapper's.
- **`time`, `difftime`, `gmtime`, `localtime`, `mktime`** - the target's `time_t` is 64-bit and the library says so:
  `_time64`, `_difftime64`, `_gmtime64`, `_localtime64`, `_mktime64`. None of the five has a `time`-less spelling in
  the library at all, which is also why a `clang`-built program imports `_time64` from the time api-set.

That last group made the linker's mapping deeper than it was: `library_of(name) -> &str` became
`import_for(name) -> (library, name-the-library-exports)`, because a program refers to `time` while the import table
must look up `_time64`. `Imported` carries both, so the *program's* name is what a relocation matches and the
*library's* name is what the hint/name entry says. One table, `RENAMED_IN_THE_LIBRARY`, holds the eight spellings.

**And this is where the review of my own work earned its keep twice.**

First, the mapping's first draft reached a `'static` lifetime by leaking a `String` per lookup - dismissed, because a
caller that wants a name does not want a leak, and `&str` in, `&str` out is the same function without it.

Second, the gate that was supposed to keep all this honest **passed after I deleted a rename from the table**. It asked
`CRT_SOURCE.contains("time")`, and the runtime source contains the word `runtime`. A gate satisfied by prose is not a
gate: the check is now the name as a *whole word followed by a `(`*, and deleting `("time", "_time64")` makes it fail
with `[("time", "time")]`. The older ledger test had the same `contains` weakness and now shares the helper.

**Measured, not assumed**: `sscanf("42 words", "%d %15s", ...)` reads both fields, `time(0)` returns a real time,
`setjmp`/`longjmp` round-trips with the right value, and the `atexit` handler prints *after* everything else - so it
ran at exit, through `_crt_atexit`. `tests/exe.rs` links that program and asserts the four lines and the exit status;
`back::crt`'s test asks `ucrtbase.dll` itself for every declared name, so a name that drifts out of the library fails
in the suite instead of as `ERROR_PROC_NOT_FOUND` in a user's program.

195 unit, 27 parser, 24 CLI, 7 exe, 1 crt, 5 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran, 5 refused,
44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong, refusal oracle 19/0;
fmt and clippy clean.

### M10 reviewed - 2026-11-22 - the assertion the refactor ate

The `argc`/`argv` commit, reviewed. The feature works - a linked program reads real arguments, a quoted one stays
one, `int main(void)` is unaffected - and the review's main finding was against the review's own tidy-up.

**A slicing edit removed an assertion and left the variable that fed it.** Two tests that each linked a program, ran
it and checked the result had the same eight-line setup, so I extracted a `run_linked` helper. In doing that by string
indices, the `assert_eq!(code, Some(11))` from the first test fell inside the replaced span, and what remained was
`let code = ...` with nothing reading it. The suite still passed - it checked the program's *output* and no longer its
*exit status*, which is half of what a program did. Restored, with a comment saying what it is for. A test that loses
an assertion does not fail; it just stops asking.

**A comment claimed what `link.exe` does by default.** The argument-mode constant was explained as "what a program
that is not a setargv program gets from `link.exe`" - plausible, unverified, and not something this repository can
check. The comment now names the mode and what it means, and leaves the other linker out of it.

**ADR-005 argued both sides of one alternative.** The bullet rejecting `_configure_narrow_argv` had grown a clause
saying it was measured and chosen anyway, which reads as a decision that disagrees with itself. The decision section
says it now, and the rejected-alternative bullet says what changed the answer.

**Two things were measured rather than assumed.** The environment: `getenv` works in a linked program, which is not
obvious - the UCRT populates it alongside `argv`, and if it had not, every linked program calling `getenv` would read
a null. And the ceiling: a narrow `argv` is converted through the ANSI code page, so `café` arrives as `caf¨¦`
- stated in the ADR, because a cost that is only observable with a non-ASCII argument is a cost a reader deserves to
read rather than discover. `--run` passes exactly one argument, the source's name, and the flag's help says so.

194 unit, 27 parser, 24 CLI, 6 exe, 1 crt, 5 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran, 5 refused,
44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong, refusal oracle 19/0;
fmt and clippy clean.

### M10 reviewed - 2026-11-22 - a link threw its warnings away

The linker commit, reviewed. The binding fix holds - `xcc hello.c` produces a program Windows runs - and the review
found one defect that mattered and a handful that did not.

**A link discarded the front end's warnings.** `Compile::link` read `let (module, preprocessed, _) =
self.front_end(...)`, and the third value is the diagnostic list: `xcc t.c` was silent about
`int a[2] = {1, 2, 3};` where `xcc -c t.c` warned about it. A warning a mode does not print is a warning the user
does not get, and the *mode* is not a reason to lose it. A link now returns the same `Artifact` a compilation does -
the bytes and the warnings - so there is one type for what the compiler produced and one place its diagnostics are
rendered. That also let the `object()` name go: there is one name for the payload, `bytes()`, because a link's payload
is not an object and two names for one thing is two places to keep in step.

**The level error still carried its CRLF continuation** - twenty-two spaces inside a sentence, from the same edit that
was cleaned up once before in the CLI and not here. It is a sentence again.

**`ImportTables.address_table: Vec<u64>` was the wrong name and the wrong type**: it holds hint/name RVAs, which are
32-bit, and the *address table* is a different thing in the same file. It is `lookup_rvas: Vec<u32>`.

**Two comments described a compiler that no longer exists.** `C_RUNTIME`'s module doc said every name in it came from
a header - two now come from the runtime source's `printf` front ends - and `snprintf`'s reason for being unavailable
to a JIT said "the universal runtime's header spells it as an inline wrapper this compiler cannot reproduce", which
stopped being true the hour this compiler started reproducing it. The reason now says what is true: the layer that
carries it is the one a *link* compiles in, so link the program instead of running it.

**The runtime test compiled the runtime at a level a link cannot use.** `tests/crt.rs` built `main.c` at the default
level, which worked until the file gained a `va_arg`; the test now compiles at `--opt-level=2`, which is the level the
only caller of that source uses, and says why.

**And a path nothing tested: two sources in one link.** `xcc a.c b.c -o two.exe` was run by hand and never by the
suite, though per-unit text and data offsets are exactly the class of bug that links fine with one source and wrong
with two. It is a CLI test now.

194 unit, 27 parser, 24 CLI, 5 exe, 1 crt, 5 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran, 5 refused,
44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong, refusal oracle 19/0;
fmt and clippy clean.

### M10 - 2026-11-22 - the import address table has to start full

`xcc hello.c` writes `hello.exe` now, and the operating system runs it. The linker's last blocker was not the
loader; it was one sentence of the format that nothing I had written said.

**The import address table must begin as a copy of the import lookup table.** With an all-zero table the loader
binds nothing and does not read the import directory at all: a bad DLL name changes nothing, and the first call
through a thunk jumps to `rip = 0`. Everything else in the image was already right - `pefile`, `objdump` and gdb all
parsed the descriptors, the hint/name entry, `ucrtbase.dll`, the thunks' `RIP`-relative displacements and the IAT
directory - so I varied the things that were left: DLL characteristics, the exception table, the IAT directory, the
import directory's size, the section's name and its flags, the number of sections. **Not one of them mattered.**
What settled it was a hand-built 4 KB image that imported `ExitProcess` and did nothing else: it failed the same way,
which said the fault was in the table and not in the rest of the image. Filling the slots from the lookup table turned
the fault into a program that exits 7.

**Two more bugs fell out of the same walk.** The import table's size was counted without the even-alignment the
writer gives its hint/name entries, so `resize` cut the table down and the library's name lost its last letter -
`ucrtbase.d`, which Windows reports as a missing DLL and says nothing about the name it could not read. The size is no
longer recomputed: the writer **asserts** that what it wrote is what the header declares. And `ucrtbase.dll` exports
`__stdio_common_vfprintf`, not `printf` - the UCRT keeps every variadic front end in the program, in `ucrt.lib`'s
layer - so the runtime this compiler links in now carries those eight wrappers itself, compiled by xcc, which makes
every `printf` in a linked program a test of `va_arg`.

**What the review of my own reasoning found**: I had twice concluded "the loader never reads the directory" from
`LoadLibrary`, which resolves nothing even for an image that works; and two of my hand-built fixtures were broken
in ways that mimicked the bug (a section table that disagreed with the file's layout, a table placed one byte past a
section's virtual size - gdb says so, in a warning I had not been reading). A fixture that is broken in the same way
as the patient is not a control.

**Measured, then wired up**: `xcc hello.c -o hello.exe` produces a 20 992-byte image that prints and returns its
`main`'s value; `tests/exe.rs` writes one and asserts both; `crates/xcc-cli/tests/cli.rs` does it through the binary.
A link goes through this compiler's own back end, so a link at another level is refused with the reason and `-c`
suggested - the same rule as `va_arg`, stated in the same place.

194 unit, 27 parser, 23 CLI, 5 exe, 5 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran, 5 refused,
44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong, refusal oracle 19/0;
fmt and clippy clean.

### M10 reviewed - 2026-11-22 - the sixty macros the commit said it had added

The `isinf` fix, reviewed. Its own fix holds - a computed infinity is classified correctly now - and the review found
that **the same commit claimed something it had not done**.

**`inttypes.h` still had no `LEAST`/`FAST` families**, though the commit message said it did: the script that added
them asserted on a `math.h` block first, exited there, and never reached the second file. The claim and the code went
out together, and only a probe - `printf("%" PRIdFAST16, …)` - said so, with an error message about a missing
parenthesis where the truth was a missing macro. Both families are there now, **generated from one table** so the six
kinds and three families cannot disagree with each other.

**And the widths behind them were a second opportunity to disagree.** This target's `stdint.h` makes every
`int_leastN_t` and `int_fastN_t` exactly N bits wide (MSVC's choice, no wider "fast" types), so `PRIdFAST16` is
`"hd"`, not `"lld"` - the first draft of that table said `"lld"` and would have printed garbage for a `short`. The
header states the invariant where the macros are, because a disagreement between two headers is a wrong number
rather than a compile error.

**One divergence from clang is deliberate, and now documented.** mingw's `PRIdLEAST8` is `"d"` - the *promoted*
width - while ours is `"hhd"`, the type's own, which is what 7.8.1 asks the macro to name. They print the same value
for anything that fits; they differ when a negative value is printed unsigned, `PRIXLEAST8` of `-1` being `FF` here
and `FFFFFFFF` there. Left as it is, with the reason in the header, so the next reader does not "fix" it to match a
compiler whose choice is only defensible as a choice.

`HUGE_VAL` and `HUGE_VALF` are `DBL_MAX`/`FLT_MAX` now that `<float.h>` is included - the same argument the header
already made about `DBL_MIN`, applied to the two constants above it.

194 unit, 27 parser, 22 CLI, 4 exe, 5 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran, 5 refused,
44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong; fmt and clippy
clean.

### M10 reviewed - 2026-11-22 - the header I had just written called an infinity finite

The conformance-header commit, reviewed. Both headers are right in the main; the review found one *wrong answer* in
the four hours it had existed, and it was mine.

**`isinf` compared against `HUGE_VAL`, which is finite here.** `HUGE_VAL` is the largest finite double (MSVC's
choice, and documented as such in the header I wrote), so `isinf(1e308 * 10)` answered **0** and `isfinite` of a real
infinity answered **1** - a program that tests for overflow would never see it. The test is now against the *range of
the type*: `(x) > DBL_MAX || (x) < -DBL_MAX`, which is true for an infinity and false for every finite value, NaN
included. `isnormal` and `fpclassify` had the same hard-coded digits and now read `DBL_MIN` from `<float.h>` - the
header includes it, because a header that spells out the ABI's digits disagrees with the ABI the day someone reads it.

**And the probe that found it was wrong in the same way**: it asserted `isinf(HUGE_VAL)`, which is true on glibc
(where `HUGE_VAL` *is* an infinity) and false here - so the first version passed for the wrong reason on clang and
would have passed for the wrong reason here too, had the macro been consistent with itself. A probe whose expected
value comes from the other compiler's *choice of constant* is not an oracle.

**Two smaller ones.** The classification macros evaluate their argument **more than once** (`isnan(f())` calls `f`
twice); the standard does not forbid it, and the header now says it in the place someone would look. And `inttypes.h`
was missing the `LEASTN`/`FASTN` families 7.8.1 lists alongside the fixed widths - sixty macros that follow from the
ones already there.

**Measured again against clang, with a *computed* infinity on both sides**: byte-identical, and the construct now
pins `isinf(1e308 * 10)` and `isinf(-1e308 * 10)` and says in a comment why `HUGE_VAL` is not the thing to test.

194 unit, 27 parser, 22 CLI, 4 exe, 5 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran, 5 refused,
44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong; fmt and clippy
clean.

### M10 - 2026-11-22 - `math.h` was a third of C99, and `inttypes.h` was refused for no reason

Two headers, and both were lying by omission in a way a real program notices.

**`math.h` declared 22 of C99 7.12's 36 functions.** The missing fourteen are the ones modern code reaches
for - `log2`, `cbrt`, `hypot`, `round`, `trunc`, `copysign`, `fmin`/`fmax`, `nextafter`, `fma` - and Lua's
`lmathlib` warned *"`log2` is not declared"* on every build of it, which is what "undefined" looks like when the
declaration is the only thing missing. All 36 are exported by the universal runtime, so all 36 are declared now,
with the five classification macros (`isnan`, `isinf`, `isfinite`, `isnormal`, `signbit`, `fpclassify`) written as
macros over them: a program that only asks *whether* a number is a number links no call to find out.

**`inttypes.h` was the one C99 header refused for "not implemented"** rather than "cannot" - and it is four
declarations and sixty macro spellings, all of which the runtime exports. `PRIdMAX`, `PRIu64`, `SCNx32`: the header
exists so a program never has to know that this target's `long long` is 64 bits.

**The two ratchets did their job while I was writing it.** The ledger test that compares the headers against the
runtime's symbol list refused the build until every new name was in `C_RUNTIME`, and the README test refused it
until `inttypes` was named there. The ledger also caught a flaw in *itself*: it counted `isnan` - a macro this
header defines - as a call into the library, so it now ignores names the shipped headers define, while
`#define stdout __acrt_iob_func(1)` still counts the accessor it really calls.

**Measured against clang, call for call**: twenty-one functions and twelve macros give byte-identical output, and
the construct in `tests/x64.rs` pins the arithmetic rather than the printing (`log2(8) = 3`, `hypot(3,4) = 5`,
`trunc(-2.7) = -2`, `PRIdMAX` of `-2` is `-2` while `PRIxMAX` of 255 is `ff`).

**And `--run` says which runtime it is.** `log2` works in a linked program and not under `--run`, because the
classic runtime that a JIT loads does not export the C99 additions. The message said only *"a JIT can only call what
it has"*; it now says that this runtime lacks the name, that the headers declare it, and to link the program - a
`missing_reason` in one place, next to the reason table it extends.

194 unit, 27 parser, 22 CLI, 4 exe, 5 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran, 5 refused,
44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong; fmt and clippy
clean.

### M10 reviewed - 2026-11-22 - eight things nothing called, hidden by one `allow` that had outlived its reason

The codebase-read commit, reviewed. Its two fixes hold; what the review found is a *blind spot* of exactly the kind
the commit was about.

**`lib.rs` carried `#[allow(dead_code)] mod back;`** under a comment that said the module was built before anything
filled it, and ended "**the allow leaves with that commit**". That commit was M4.2. The allow stayed, and it silenced
dead-code warnings across roughly 6 700 lines - the whole back end. Removing it, the compiler answered at once with
**eight** items nothing reads:

| item | what it was |
|---|---|
| `coff::write` | the object writer's old entry point, superseded by `write_image` |
| `pe::base_relocation_size` | the size helper this very commit had made redundant - hidden by the allow *in the same pass* |
| `MachineFunction::param_offsets` | a table of block-parameter offsets the encoder computes itself |
| `Encoded::frame` | the frame size, already in `coder` and in `MachineFunction::frame_bytes` |
| `Encoder::outgoing_bytes` | stored, never read; the one caller computes it before the struct exists |
| `Loaded::size` | written, never read: `VirtualFree` with `MEM_RELEASE` takes zero |
| `FunctionLowering::names` | the interner, kept because a field existed for it |
| `regalloc::location_register` | a helper with no caller |

**And the mechanical part of that needed care**: deleting `param_offsets` from the *output* struct was right, and
deleting it from the *lowering* struct broke the build - it is read twice there. Same for `Encoder::frame` (used by
every offset in the file) against `Encoded::frame` (dead). The lesson is the usual one: delete one item, build,
repeat. Three files were reverted and redone that way.

**A comment that grants a permission outlives the reason for it, and then the permission is the thing that rots.**
The allow is gone; every module now has to justify the ones it keeps, in place.

194 unit, 27 parser, 22 CLI, 4 exe, 5 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran, 5 refused,
44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong; fmt and clippy
clean.

### M10 - 2026-11-22 - the linker, as far as it goes: an image that loads and does not bind

The linker's own module is written (`back/x64/pe.rs`): sections placed from sizes, `REL32` and `ADDR64`
relocations filled in, `.pdata`/`.xdata` regenerated for the whole program, an import table with a thunk per
imported function, base relocations, and a PE32+ header. `Compile::link` compiles the units *and* the C runtime and
hands the images to it.

**Measured: the image loads and runs.** Windows maps our sections and jumps to `mainCRTStartup`; the disassembly
shows the calls going to the right thunks and the `lea` reaching the right format string. **And the loader does not
bind the import table**: a `jmp [rip+slot]` finds a zero, so a program faults on its first library call. Three
things were fixed on the way - a section's *virtual* size was the file-padded one, a file that stops at the last
byte of the last section is refused (`ERROR_BAD_EXE_FORMAT`, nothing said), and relocations were written at a
*unit-relative* offset so a second unit's displacement landed inside the first's instructions - and the import
directory is demonstrably not read: **a deliberately invalid DLL name in the descriptor changes nothing.**

So it stays behind `-c`, and the message says exactly this. The next step wants an oracle this machine does not
have: `dumpbin`/`pefile`, or a diff of our `.idata` against what `link.exe` writes for the same program. Guessing
further would be measuring the compiler's opinion of itself.

193 unit, 27 parser, 22 CLI, 4 exe, 5 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran, 5 refused,
44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong; fmt and clippy
clean.

### M10 reviewed - 2026-11-22 - the same index, wrong for a second reason

The `va_start` commit, reviewed. Its fix holds - Lua's every message is now clang's - and the review found the *other*
way the same index could be wrong, one branch over.

**A parameter that takes no frame slot still takes a position.** `function_params` is what `va_start(ap, last)`
searches for `last`'s position, and it records a *slot* per parameter - but a record passed by reference is the
caller's copy and gets no slot, and the loop `continue`s for it. So every parameter after it moved one place
earlier, and `va_start` pointed at the argument *before* the one it meant:

```c
static long long f(struct Big s, int last, ...) { ... va_arg(ap, int) ... }
f(b, 0, 2);        /* 40 before - s.a + 0, the wrong slot - and 42 after */
```

The list is now pushed **once per parameter, before any branch can skip one**, and filled in where a slot is made: the
invariant "one entry per parameter, in source order" is structural rather than something three `continue`s have to
remember.

**Checked on every probe the previous commit used**, since the change sits under all of them: `va_start` in one
function, in two, after a record by reference, with a forwarded `va_list`, and Lua's own build - whose error messages
and `hello.lua` output still match clang's byte for byte.

191 unit, 27 parser, 22 CLI, 4 exe, 5 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran, 5 refused,
44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong; fmt and clippy
clean.

### M10 - 2026-11-22 - every error message Lua printed was built from the wrong slots

Lua now runs and its messages are clang's - but until this commit, *every* message it printed was garbage: an error
in a program read `:1905176596: (null)` where clang says `err.lua:2: attempt to call a nil value (local 'f')`. The
cause was `va_start`.

**`function_params` was never cleared between functions.** It records which slot each parameter lives in, so that
`va_start(ap, last)` can find `last`'s position; `function_body` pushes into it and nothing emptied it. In a file
with several variadic functions the second one's `last` was found at *twice* its position - measured as a clean
pattern, `skip = 2n - 1` for `n` named parameters - and `va_start` pointed past the argument it meant. Six lines in,
`f2(int a, int b, ...)` said `va_start 3 parameters`; it says 2 now. Lua has dozens of variadic functions and builds
every diagnostic through `luaO_pushfstring`, so the whole error surface was wrong while every ordinary program
worked - which is exactly why the previous oracles (hello.lua, the corpus, 443 programs) never saw it.

**Found by giving the harness a second question.** The bisect compared one *successful* program per file; a
compiler can be wrong only on an error path and pass that. Compiling a script that *raises* an error found the
difference in one run, and the reduction went from "an interpreter prints nonsense" to twelve lines: a variadic
function forwarding its `va_list` to another function, in a file that also has a second variadic function.

**And the acceptance test's stopping point is not ours.** Lua's own suite (`all.lua`) runs into `main.lua` and
stops at `getoutput`, which needs the file its shell redirection was supposed to write - and **a clang build of the
same interpreter stops at exactly the same line with the same error**. `main.lua` says so at the top: "most (all?)
tests here assume a reasonable Unix-like shell". A control that fails the same way is not a bug report.

191 unit, 27 parser, 22 CLI, 4 exe, 5 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran, 5 refused,
44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong; fmt and clippy
clean.

### M10 reviewed - 2026-11-22 - a wrong answer is the one thing that may not be accepted

The commit that made Lua run, reviewed. Its two fixes hold - the whole corpus and 33 of Lua's 33 objects agree - and
the review found the *other* half of the initialiser bug, still open: a form of it that produced a wrong answer
instead of a diagnostic.

**`static int a[1] = { x = 5 };` compiled into zero.** C99 6.6p7 wants a *constant* expression in a file-scope
initialiser, and an assignment is not one; clang refuses it. This compiler accepted it, wrote nothing, and the
program read zero. The `assign` path that returns a value for a file-scope initialiser had no error for "not in a
body" - the arm existed for block scope only, so the file-scope case fell through to a silent value. It is a
diagnostic now, and block scope still assigns happily.

**And the plan had grown a habit of keeping its dead bullets.** Three bullets said what Lua's state was - one
closed, one new, one an "older note" describing seven failing files that no longer exist - and two said `.pdata` was
both done and "kept for the record". A plan that keeps every superseded paragraph stops being a plan; the history is
what the AUDIT is for. The duplicates are gone and the surviving bullet says what is true now: Lua runs, and its
test suite is the next thread.

**Checked while there**, because the fix touched the same function: the brace-list path already parsed an
assignment-expression (`initializer_value` calls `assignment_expression`), so the two paths now agree and the bug was
only ever on the non-braced one; and a bare `return;` behaves the same at all three optimisation levels, which the
Lua build (level 2 only) could not have shown.

191 unit, 27 parser, 22 CLI, 4 exe, 5 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran, 5 refused,
44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong; fmt and clippy
clean.

### M10 - 2026-11-22 - Lua runs: a `return;` that emitted nothing, and an initialiser that was not one

M7's acceptance item is closed. Lua 5.4.7 compiles, links and **runs**: every one of its 33 objects is correct on
its own, `lua hello.lua` prints what a clang build prints, and the interpreter walks into its own test suite. Two
bugs stood between the six failing files and this, and both are C, not Lua.

**A bare `return;` in a `void` function emitted nothing at all.** No instruction, no terminator: the block simply
fell through to whatever came next. `luaK_posfix` folds a constant and returns, so the missing return made it fold
*and* emit the operation - a program computing `2^10` printed `2^100`, and one computing `#t` printed the wrong
length. That single bug was four of the six files (`lcode`, `lparser`, `ltable`, `lvm`), which is why they failed
in four different functions: each of them returns early somewhere.

**An initialiser may be an assignment, and ours was not.** C99 6.7.8p1 says an initialiser is a brace list or an
*assignment*-expression; the parser read a conditional-expression and let the rest be swallowed by its error
recovery. So `CallInfo *ci = L->ci = next_ci(L);` parsed as `ci = L->ci` - no call to `next_ci`, no store - and
every call frame was built on the *current* one. Lua's `prepCallInfo` is written exactly that way, and `lauxlib`
lost an initialiser in the same shape.

**Both were found with a better oracle, not with a better guess.** Compiling Lua with `-DLUAI_ASSERT` makes every
internal invariant an `assert` and every `assert` report its own file and line - and the two earlier fixes in this
area (the decay in a boolean context, and `__FILE__`/`__LINE__` naming the invocation) are what made that build
compile at all. The bisect harness also had to be fixed: it counted exit 0 as success, so a build that printed
nothing "passed"; it now compares *output* against a clang-built reference, which dropped `lcode` from "fine" to
"computes 2^100" and turned six files into six honest failures.

**Measured: 33 of 33 objects correct alone; the interpreter's output is identical to clang's; the test suite runs
into `main.lua` before it meets its first difference** - `getoutput`'s `os.execute` round trip, which is the next
thread to pull.

191 unit, 27 parser, 22 CLI, 4 exe, 5 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran, 5 refused,
44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong; fmt and clippy
clean.

### M10 reviewed - 2026-11-22 - the decay belongs where the value is used, and a pair is not a fact

The `LUAI_ASSERT` commit, reviewed. Its two fixes hold on every shape I could think of, and the review found one
design fault of the kind this project names again and again: a fact with no name.

**`(u16, Span)` for "where the macro was invoked".** The two halves are read apart - the file is what `__FILE__`
reports, the span is the line `__LINE__` does - so a tuple made every use site say which half it meant. It is a
`Site` now, with a doc comment saying what each field answers.

**What the fixes were checked against**, since a fix that only makes one program work is a coincidence: `if`, `while`,
`do ... while`, `for`, `?:`, `!`, `&&` and `||` with a string literal all agree with clang; `_Bool b = "abc"` agrees;
a two-level macro chain (`#define A B`, `#define B __LINE__`) reports the *outermost* invocation's line, which is
what the standard means by "the current source line"; and `assert(1 == 1)` in a file named `asrt.c` reports
`"asrt.c", 2` where it used to report `"assert.h", 28`.

**One thing checked and found already right**: every `require_scalar` caller ends at `compare_to_zero`, which is
where the decay now happens - so widening the check to accept arrays could not open a hole, and the reasoning is
written where the widening is.

191 unit, 27 parser, 22 CLI, 4 exe, 5 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran, 5 refused,
44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong; fmt and clippy
clean.

### M10 - 2026-11-22 - a `switch` arm that ends in a `?:` fell off its own end

The front-end bug behind two of Lua's seven failing files, found by reducing a `SIGILL` in `llex`'s `read_string`
to twelve lines and then reading the `cir`.

**The fall-through link was made from the wrong block.** A `switch` arm that reaches the next `case` without a
`break` must be linked from wherever control *is* when the arm ends. The old code deferred that to after the whole
body was read, and linked from the block the arm *started* in - which is right only if the arm's last block is its
first. A `?:` (and equally an `if`, or any construct with a join) leaves the cursor in a join block of its own, so
the join got no terminator at all and the arm fell off its end into the `unreachable` block the lowering emits
there. The program then executed `ud2`: `SIGILL`, from code whose `cir` the verifier had passed.

**The fix is smaller than the bug**: the link is made in `open_arm`, when the next arm is opened, and at the end of
the body for the last arm. The deferred loop is gone, and with it the question "which block ended this arm?".

**Measured: two of Lua's seven failing files now pass** - `lcode` and `llex` compile and run in the real
interpreter - and the reduction is pinned by two constructs: an arm ending in a `?:` inside a `while`, and an arm
ending in an `if`. Five files still fail: `lauxlib`, `ldo`, `lparser` (a parse error raised into the unwinder),
`ltable` (a hang sampled inside `luaH_newkey`), `lvm`.

190 unit, 27 parser, 22 CLI, 4 exe, 5 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran, 5 refused,
44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong; fmt and clippy
clean.

### M10 reviewed - 2026-11-22 - an `extern` object was a compiler bug at one level and a feature at the other

The narrowing commit, reviewed. Its text holds, and its evidence had a defect in it: the line that said `lcode` could
not be tested at `--opt-level=1` was a lie the commit had not noticed, because the error it got back was an
**internal** one.

**`extern int table[];` in one file and the definition in another** is the most ordinary thing in C, and Lua is
full of it - `lcode.c` reads `luaP_opmodes`, which `lopcodes.c` defines. Our own back end names the symbol in a
relocation and works. The Cranelift path looked the name up in a map of the globals *this unit defines*, found
nothing, and reported `internal: `luaP_opmodes` is not declared here` - **a construct that one level compiles
reported as a compiler bug by the other**, which is the worst shape a diagnostic can take: it says "the compiler
is broken" about a program that is not. The path now declares an *imported* data symbol - no bytes of its own,
the linker supplies the address - which is what an external declaration means.

Verified: a two-file program through level 1 returns 20 where it used to be refused, and **`lcode.c` now compiles at
`--opt-level=1`** - where it still fails to run, which is the point of the exercise: it moves `lcode` from *untested*
to *tested and failing*, and the failure is at both levels, so it is the front end's.

**Two probes cleared while looking for it**, both worth keeping: Lua's own instruction-encoding macros
(`MASK1`/`SETARG_A`/`GETARG_sBx`, all shifts and masks on `unsigned int`) agree with clang over a 256-iteration
sweep, and the `Node`/`Table`/`TValue` layouts agree byte for byte - so the bit manipulation and the aggregate
layouts in the failing files are not the bug.

190 unit, 27 parser, 22 CLI, 4 exe, 5 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran, 5 refused,
44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong; fmt and clippy
clean.

### M10 - 2026-11-22 - the frame table: `.pdata` and `.xdata`, and the three bugs it uncovered

The unwinder's tables, written the way the measurement said they had to be. The oracle was already written:
a `longjmp` out of three of this compiler's own frames, which faulted before and prints `42` now - on the
object path at both levels and in the JIT.

**The codes are recorded where the prologue is emitted.** `encode::prologue` writes pushes, one `sub rsp`, and
the XMM saves; each call now records a `UNWIND_CODE` - the end offset of the instruction, the operation, the
register or size - so no second function has to reconstruct what the first one did. `coff::unwind_info` turns
them into the format's header and slots, in the *decreasing* order the format requires, and the frame table is
laid out once (`unwind_layout`) for both consumers: the object writer turns entries into `ADDR32NB` relocations,
the loader into addresses, because `.pdata` holds image-relative addresses and `RtlAddFunctionTable` reads them
against the base.

**And the object writer became a loop over sections.** It had two hard-coded sections; `.pdata` and `.xdata`
would have been two more copies of the same fourteen lines. A section is now bytes, named symbols, and
relocations, written by one loop - and the test that counts the bytes of an empty object still holds.

**Three bugs the work uncovered, none of them in the frame table.**

*A translation unit with no functions wrote a zero-byte object, silently.* `compile` skipped the back end
whenever the function list was empty, on the theory that there was "nothing to lower" - so `lopcodes.c` and
`lctype.c`, two files in Lua that are a table and nothing else, produced empty objects and their symbols "did
not exist" at link time. The comment said an object was still what the caller asked for, and the code handed
back nothing.

*A tentative definition of an incomplete array was never completed.* C99 6.9.2p2 completes `int open[];` to one
element at the end of the translation unit; the parser left it incomplete, and the back end refused it as an
object with no size. Invisible until the short-circuit above was removed, which is what a bug behind another
bug looks like.

*The loader subtracted the image base from something that was already an offset* - `xdata_start - base` - which
only a unit *with* unwind data could reach, and the first such unit panicked.

**And a false diagnostic.** `int a[3] = {1, 2, 3,}` warned "this array has 3 elements, more were given", because
the check saw the trailing comma and not what followed it. A trailing comma introduces nothing; the warning now
asks whether anything follows before it fires. It was found by the `--opt-level=0` reading of Lua, which spells
its opcode table with leading commas.

**Measured: Lua 5.4.7 is 34 files, all 34 compile at `--opt-level=2`, and they link into an interpreter - which
then faults.** The fault is reproducible in a twenty-line driver and lands in *clang's* `luaV_finishget`,
called from *clang's* `init_registry`, while only `lauxlib.c` is this compiler's: so it is a mismatch our
`lauxlib` produces rather than a bad instruction of ours, and it is where this milestone stops. It is the next
item in `PLAN.md`, with the driver already written.

190 unit, 27 parser, 22 CLI, 4 exe, 5 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran, 5
refused, 44 skipped, 0 timed out, 0 mismatched) - green; fmt and clippy clean.

### M10 reviewed - 2026-11-22 - a private copy of somebody else's object

Two commits reviewed: the four Lua fixes and the frame-table work. The second one's defect was a *wrong
answer*, which is the only kind that matters.

**A block-scope `extern` declaration made a frame slot.** C99 6.2.2p4 makes `extern int g;` inside a function
*refer* to the object that has external linkage - it is a declaration and not a definition, which the previous
commit had just taught the file-scope path. The block-scope path was not taught: it allocated a slot, and
`g = 7` wrote the slot. Nothing in 443 corpus programs could show it, because the check needs the global read
from somewhere else afterwards:

```c
int g = 5; int read(void) { return g; }
int main(void) { extern int g; g = 7; return read(); }   /* 5 before, 7 now; clang says 7 */
```

**A comment describing a check the code does not make, found again** - and this time in the docs:
*"There is no `gnu89` mode, **no extensions beyond what C99 defines**, and no C++"*, in a compiler whose own
headers are spelled in `__builtin_offsetof`, `__builtin_va_start`, `__builtin_va_arg` and now
`__builtin_alignas`. The sentence now names them and says why each exists: a `__builtin_` is there when a
header *cannot* be written in C, which is exactly the reason a keyword is defensible.

**And the matrix row for `setjmp.h` still cited the reason the commit had just measured to be wrong** -
"`jmp_buf`'s layout is the ABI's" - when the layout was written and the jump faulted for an unrelated cause.
It now names the frame table, which is where the milestone is.

**Verified, not assumed:** six casts in integer constant expressions (`(char)300`, `(unsigned char)-1`,
`(long)(signed char)-1`, `(int)(unsigned long long)-1`, `(short)70000`, `(unsigned short)-1`) all agree with
clang; `signal`/`raise` behaves as clang's does, which is a header that ships being a header a program can
use; the aligned member's `offsetof`, the locale's decimal point, the block-scope `extern` and the constant
casts are four constructs now pinned. 443 corpus programs agree, 20 refused, 0 wrong; cJSON byte-identical to
clang; 33 of Lua's 34 files compile.

188 unit, 27 parser, 22 CLI, 4 exe, 5 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran, 5
refused, 44 skipped, 0 timed out, 0 mismatched) - green; fmt and clippy clean.

### M10 - 2026-11-22 - the frame table, found by trying to write the header it is missing

`signal.h` and `locale.h` are real now, so Lua was down to `ldo.c` and `setjmp.h`. Writing that header
meant writing the ABI's `jmp_buf` down - 256 bytes, 16-byte aligned, `_setjmp(buffer, 0)` - which
surfaced two things, one fixed and one measured into the plan.

**A member's `_Alignas` was silently ignored.** C11 6.7.5 lets a member raise its own alignment, and the
layout code only ever asked for the *type's*. `struct { _Alignas(16) unsigned long long p[2]; }` was 16
bytes with align 8, so `offsetof` of it in a larger struct was 8, not 16. It compiled, it was wrong, and
nothing failed - which is why it took a header that needs the alignment for a `movaps` to find it.
`Member` now carries its own `alignas` and the layout honours it.

**And a C99 program cannot spell that.** `_Alignas` is gated on `--std=c11` by design, and a *header* may
not ask the program to change its standard. Our headers are already written in `__builtin_` terms
(`offsetof`, `va_arg`, `va_start`), so `__builtin_alignas(n)` is the same specifier without the gate, and
`setjmp.h` used it.

**Then `longjmp` faulted.** The buffer was written correctly - `frame` and `rip` both plausible - and the
jump crashed even *inside one frame*, because the C library's `longjmp` calls `RtlUnwindEx`, which demands
a `RUNTIME_FUNCTION` for every frame it passes. clang emits `.pdata` + `.xdata` (12 bytes each for a
one-function file); this compiler emits neither. **So the header was deleted and the refusal rewritten to
name the missing thing rather than the shape of a struct**: *"`longjmp` walks the frame table, and this
compiler emits no `.pdata` yet"* - a fact that was measured by trying, not assumed from ADR-003.

That is the next milestone, and it now has an oracle rather than a description: the same program either
prints `same frame: 42` or faults, and the prologue it must describe is already known in one place. It is
also the difference between "a debugger can walk our frames, C++ exceptions pass through, Lua's `pcall`
works" and none of those.

**Measured: 33 of Lua's 34 files compile at `--opt-level=2`**, the last waiting on the frame table. Two new
constructs pin the work: an aligned member's `offsetof` and the locale's decimal point. 443 corpus programs
agree, 20 refused, 0 wrong; cJSON byte-identical to clang.

188 unit, 27 parser, 22 CLI, 4 exe, 5 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran, 5
refused, 44 skipped, 0 timed out, 0 mismatched) - green; fmt and clippy clean.

### M10 - 2026-11-22 - Lua, measured: four causes, and only one of them a missing header

The plan's last product claim is M7's acceptance, which needed an interpreter. `lua-5.4.7/src` is 34 files; **two
compiled.** The 32 refusals had exactly four causes, and the count is what made the order obvious:
17 `<signal.h>`, 9 `expected a constant expression`, 5 `<locale.h>`, 1 `<setjmp.h>`. Nothing else - no fifth
thing hiding behind the first four.

**The nine were a bug, not a gap.** C99 6.6p6 lets an integer constant expression hold a cast to an integer type,
and `convert` - the one function that asks "is this conversion legal" - emitted an instruction and kept no
constant, which is right in a body and wrong while a constant expression is being read. A cast was the only
operator that had a constant on each side and produced none. `fold_conversion` now folds it from the *same*
`conversion` table the instruction comes from, so the fold and the code cannot disagree about what a `Truncate`
means. `char b[((int)(16 * sizeof(void *) * sizeof(double)))]` - how Lua sizes a buffer - is 1024 bytes.

**`&f` was refused.** C99 6.5.3.2p3: the `&` of a function designator *is* that designator, because a function
name is already an address and is never an lvalue - so the lvalue rule that guards `&x` refused `&panic`, and
`lua_atpanic(L, &panic)` is how Lua hands a function over.

**And `extern` was defining objects.** `lua.h` declares `extern const char lua_ident[];`, which has no size until
the file that defines it says one. The parser laid down bytes for every file-scope object with no initialiser -
so merely *including the header* invented an object of incomplete type, and the back end said so in its own
words. `extern` without an initialiser is a declaration (6.9.2p2) and now is one.

**`signal.h` and `locale.h` became real** - the two the previous commit had written a reason for, both
declarations over exports both C runtimes have. `stdio.h` also gained `setvbuf` with its three modes and
`tmpnam`, which was a declared-nothing function returning `int` compared against `NULL`.

**Measured: 33 of Lua's 34 files compile at `--opt-level=2`.** The last one, `ldo.c`, needs `setjmp.h` - the
header whose refusal reason ("`jmp_buf`'s layout is the ABI's") is a statement about *writing the layout down*,
which is the next step and not a blocker of a different kind. `cJSON.c` still compiles and still matches clang
byte for byte; 443 corpus programs agree, 20 refused, 0 wrong.

188 unit, 27 parser, 22 CLI, 4 exe, 5 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran, 5
refused, 44 skipped, 0 timed out, 0 mismatched) - green; fmt and clippy clean.

### M10 reviewed - 2026-11-22 - two lists that had to agree, and nothing that compared them

The header ledger, reviewed. The ledger holds; next door sat the same defect it had just fixed.

**`errno.h` declared `_errno`, and `--run`'s symbol list had never heard of it.** `back/crt.rs` resolves a
*hand-written list* of names from the loaded C runtime, and the headers are a second list of the same names -
so `--run` could not run the `errno` example the previous commit's own test used, and nothing said so until
someone tried. The two lists now have a test that reads the shipped header text, extracts every name followed
by `(`, and requires each to be in the runtime list or in a named exception. `_errno` was the whole gap.

**And `UNAVAILABLE` is the exceptions, with reasons.** Two names a header reaches for and no Windows runtime
offers: `snprintf` (the universal runtime spells it as an inline wrapper), and **`__acrt_iob_func`** - which
the review found *live*: `assert.h` pulls `stdio.h`, `stdio.h` spells `stderr` with the universal runtime's
accessor, and `--run` loads the classic runtime, which exports `_iob`. So a program that `#include <assert.h>`
could no longer be run, and the message named a symbol the program never wrote. It now quotes the reason, and
the matrix says which streams `--run` cannot reach and why one runtime per process is deliberate.

**Two smaller faults in the same commit.** The refusal message wrapped the name in `<>` even when the program
wrote `"..."`; and `headers::c99_headers` was a test-only accessor without `#[cfg(test)]`, which is a warning
the release build only escapes because the test module uses it.

**A comment in `crt.rs` cited the `snprintf` fact that now lives in `UNAVAILABLE`** - the same fact in two
places, which is where this project keeps finding its bugs.

**Measured after the review: `--run --opt-level=2` reads `errno == ERANGE` from the C runtime's own `strtol`
(42), with a CLI test that would have failed before `_errno` was added; 126 header-declared names against 126
runtime names, exactly; 443 corpus programs agree, 20 refused, 0 wrong; cJSON byte-identical to clang.**

188 unit, 27 parser, 22 CLI, 4 exe, 5 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran, 5
refused, 44 skipped, 0 timed out, 0 mismatched) - green; fmt and clippy clean.

### M10 - 2026-11-22 - the ledger of headers, and the two C99 requires that were in no list

A real library failed on `#include <assert.h>` - C99 7.2, the header every library includes before it
asserts anything - and finding out why uncovered a worse fault than the missing header.

**Which headers exist was written in three places**: `builtin`'s match arm, the shipped list in a test, and the
*absent* list in another test. `assert.h` and `errno.h` were in none of them, so both fell through the gaps
between the lists and a program including either was told `cannot find` - the message a **typo** gets.
`src/headers.rs` now holds **one table**: every header in C99 7.1.2's table, each `Shipped(text)` or
`Refused(reason)`. Both behaviours derive from it, and a test walks it against the standard's own list, so a
header cannot be added to one and forgotten in the other.

**And the refusal message the docs promised did not exist.** The refused list was covered by a test that only
asked whether *something* failed, under a comment saying the message names the reason "(CONSTRAINTS 3)" - it
did not, and the test could not tell. `<setjmp.h>` now says *"not available: `jmp_buf`'s layout is the ABI's,
not the C library's"*, and the test checks the reason reaches the reader. **A comment describing a check the
code does not make** is this project's most-repeated finding, and here it had hidden a missing header for two
milestones.

**Three headers became real.** `assert.h` is 7.2p2's macro over the C library's `fprintf` and `abort`, so it
needs no runtime of its own - which is why it can exist where `setjmp.h` cannot; `NDEBUG` gives 7.2p1's void
expression. `errno.h` is `#define errno (*_errno())`, because the C library keeps one per thread, and the `E*`
values are the target's. `iso646.h` was eight lines nothing in the corpus used and nothing in real code should
have to lack.

**Measured:** `assert` both ways, `errno`/`ERANGE` from a real `strtol` overflow, and every `iso646` spelling,
each against clang; the refusal message names its reason for the nine headers that refuse. **sds.c (BSD, 1 328
lines) now gets past `assert.h`** and fails at `sys/types.h` - POSIX, not C99, so that refusal is ours to
explain and not to fix. cJSON still compiles and still matches clang byte for byte; 443 corpus programs agree,
20 refused, 0 wrong.

**The README said "twelve clean-room C headers" while fifteen shipped.** Its row is now compared to the table
by name - every header that ships is named there, and nothing that refuses is - which is a fact the build
checks instead of a number a reader has to trust.

185 unit, 27 parser, 21 CLI, 4 exe, 5 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran, 5
refused, 44 skipped, 0 timed out, 0 mismatched) - green; fmt and clippy clean.

### M10 reviewed - 2026-11-22 - a fact that was true in one of the two places it lived

The cJSON commit, reviewed. Its three fixes hold; what the review found is the same defect repeated in the
place the commit forgot, and one claim that was true only by luck.

**"The passes broke it" was still the message in the JIT.** The commit fixed that wording in `compile` - the
front end's `cir` is now verified before the first pass, so a type error is the front end's - and left the
identical sentence in `back/jit.rs`, which runs the same `optimise` and would have blamed the same innocent
pass, in `--run`, with the same hour of bisecting to pay. **Two copies of one message is how one of them gets
fixed.**

**`ADDR64` wrote the symbol and dropped the addend.** The loader replaced the eight bytes at a data
relocation with the symbol's address, which is right exactly as long as every addend is zero - true today,
because the front end writes only plain addresses, and *false* by construction the moment `&s.member` or
`a + 2` reaches an initialiser. It now reads what the field holds and adds, which is what the relocation
means, so the assumption cannot become a silent wrong answer.

**And a flag that is true in the wrong place.** An expanded token keeps the `starts_line` the lexer gave the
macro's *body*, which describes where the body was written, not where the expansion lands. Nothing reads it
there - `directive` is the only consumer, and a macro is expanded after its directive's line has been
gathered - but "nothing reads it there" was undocumented, which is the shape of an invariant that decays. It
has a comment now, next to the splice that creates it.

**Measured after the review: cJSON still compiles and its output still matches clang byte for byte; 443
corpus programs agree, 20 refused, 0 wrong, refusal oracle 19 against clang.** The differential is at 415
ran, 5 refused, 0 mismatched.

185 unit, 27 parser, 21 CLI, 4 exe, 5 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran, 5
refused, 44 skipped, 0 timed out, 0 mismatched), crt, pp gate, fuzz - green; fmt and clippy clean.

### M10 reviewed - 2026-11-21 - the two operands C11 names, and neither of the ones built

The `_Alignof`/`_Alignas`/`_Atomic` commit, reviewed. It works on the cases that were tried, and the two
cases C11 names explicitly are the two it got wrong.

**`_Alignas(0)` was an error.** C11 6.7.5p4 says *"An alignment specification of zero has no effect"* - so
zero is a legal operand of the feature, meaning *none*, and a compiler that refuses it refuses something the
standard permits. Every other number was accepted, including the ones that are not alignments at all.

**`_Alignas(3)` was accepted.** C11 6.7.5p3 wants a power of two; three is not a small alignment, it is not
an alignment, and the frame's allocator would have rounded to it without complaint. clang's own words are
*"requested alignment is not a power of 2"*, and a compiler that accepts it is a compiler that agrees with a
program no other compiler builds.

Both are the shape a boundary case takes when the implementation was written from the examples rather than
from the clause: **the two values the standard bothers to name are exactly the two that were missed.**

**And a `static` local dropped the alignment in silence.** `static _Alignas(16) int s;` inside a function is
an object with static storage duration - the specification travels with the object, not with the storage
class - and `static_local` was passed no alignment, so the request vanished. A silently ignored attribute is
worse than a refused one: the program compiles, runs, and is not aligned.

The verification is now behavioural in all three cases: a program that *checks its own addresses* and
returns 0 only if they are what was asked for, run at both levels. That is the only kind of test an
alignment can have - the addresses are the feature.

**Measured after the review: 443 corpus programs agree, 20 refused, 0 wrong.** Differential at 415 ran, 5
refused, 0 mismatched.

184 unit, **27 parser**, 21 CLI, 4 exe, 4 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran, 5
refused, 44 skipped, 0 timed out, 0 mismatched), crt, pp gate, fuzz - green; fmt and clippy clean.

### M10 reviewed - 2026-11-21 - two options that were quietly rounded

The C API's options, reviewed. The code compiled and every test passed; two lines in it were still wrong,
and one of them said so in a comment.

**`standard = 7` meant C99, and `opt_level = -1` meant zero.** The mapping was `1 => C11, _ => C99` and
`clamp(0, 255)`. A host that asked for a dialect this compiler does not have, or a negative level, got the
default and no word about it - the failure discovered from a bug report rather than from the API. Worse, the
comment above the clamp said the value was *"passed through: `build` refuses it, so the refusal names the
level the host asked for"*. Nothing refused it, nothing named it: `clamp` had already turned -1 into 0
before `check_opt_level` could see it. **A comment describing a check the code does not make** is the exact
fault a review found here three commits ago, in the branch fusion - the same mistake in a new place, which
is the argument for looking for it by shape rather than by memory.

Both are refused now, with the number the host gave: *"standard 7 is not one this compiler has: 0 is C99
and 1 is C11"*, *"opt level -1 is not one this compiler has: 0, 1 and 2"*. They come out of the one error
path the API has - the handle carries the reason and the first `xcc_add_source` reports it - so a host that
checks one return value checks everything, which is what the API's own documentation promises.

**And the test is the shape the rule needs**: not "level 3 fails" but *every* level in 0..=2 is accepted
and both ways of asking for nothing sensible are refused with the value named. A test written against the
one case that was thought of is a test that agrees with the mistake.

**Measured after the review: 443 corpus programs agree, 20 refused, 0 wrong; 8 C API tests.** The
differential is at 415 ran, 5 refused, 0 mismatched.

184 unit, 25 parser, 21 CLI, 4 exe, 4 x64, 4 hostile, **8 C API**, 4 bcc20-sys, differential (415 ran, 5
refused, 44 skipped, 0 timed out, 0 mismatched), crt, pp gate, fuzz - green; fmt and clippy clean.

### M10 reviewed - 2026-11-21 - the page that could not be written

The level-2 JIT, reviewed. It found a live fault in the first minute, and then found that two of the
measurements taken while writing it were measuring nothing.

**`.data` was made executable and read-only along with `.text`.** The loader protects its one allocation
after the relocations are applied - code must be executable, data must be writable - and the first version
applied one protection to both, page granularity being what it is. So `static int n; n++` faulted: the
program was running code it had just made read-only from code that had just written to it. The two sections
now start on separate pages (`data_start` rounds up to 4096) and each gets the protection its contents
need. Found by asking the question the corpus measurement had not: *does a program that writes a global
work?* - thirty seconds of test, one program.

**And two measurements were wrong in the same way.** The first said "444 corpus programs run at
`--opt-level=2`", counting a program that segfaults as one that ran, because the check was "the compiler
did not refuse" rather than "the program did what it should". The second classified every exit status above
128 as a crash, so `return 321;` was reported as 39 crashes. Both are the same mistake: **a measurement
without an oracle is a measurement of the compiler's opinion of itself.** With clang as the oracle and
*disagreement* as the only signal: **439 agreed, 24 refused, 1 difference.**

**The one difference is not ours.** `%e` printed `1.250000e+000` where clang's program printed
`1.250000e+00` - and the *same object*, linked by clang against the modern runtime, prints `e+00`, while the
JIT resolves `printf` from the classic one. The code is identical; the library that formats a float is not.
That is exactly what the README's `--run` caveat already said, now with a number behind it.

The smaller ones: `Loaded::names` had no caller - an accessor written for a use that never came - and is
gone.

**Measured after the review: 443 corpus programs agree through the object path, 20 refused, 0 wrong; 439
agree through `--run --opt-level=2` against clang, 24 refused (the four `open` programs among them, which no
loaded runtime exports), 1 difference that is the C runtime's.**

184 unit, 25 parser, 21 CLI, 4 exe, 4 x64, 4 hostile, 6 C API, 4 bcc20-sys, differential (415 ran, 5
refused, 44 skipped, 0 timed out, 0 mismatched), crt, pp gate, fuzz - green; fmt and clippy clean.

### M10 reviewed - 2026-11-21 - the headline that lied

The refusal-solving commit, reviewed. No defect in the code; the defect was in the file a reader meets
first.

**`README.md` said two things that were no longer true, and one number four milestones old.** Its "What
does not work yet" section opened with *"a floating-point argument to a variadic function cannot be passed -
`printf("%f", 1.5)`"* and *"`va_list`, `va_start`, `va_arg`, and defining a variadic function are refused"*.
All of it has worked at `--opt-level=2` for several commits. And the lowering row said **"414 of the 464
corpus programs"** with **"2.6x Cranelift after four rounds of shrinking"** - the number is 439, and the
size story has been told against `clang -O2` since the seventh round: 7.11x to 3.46x.

A stale line in a table is untidy. **A refusal list that refuses what works is worse than no list**: a
reader decides the compiler cannot do something it can, and nothing in the build contradicts them, because
nothing reads the README.

**The fix has two halves, and the second is the interesting one.** The text now says what is true - the two
false bullets replaced by the real asymmetries (`--opt-level=2` only for variadic calls, definitions and
`va_arg`; a file-scope initialiser may hold an address but not an address *expression*) - and a test
asserts that the README quotes `{PROGRAMS_THAT_AGREED} of the 464`. The number lives in one place now, and
the two cannot drift apart again without the suite saying so.

That is the pattern this project keeps relearning: **a fact that is written down twice is a fact that will
disagree with itself**, and the way out is not to be careful but to make one copy checked by the build.

**Measured after the review: 439 corpus programs agree, 24 refused, 0 wrong**; the differential at 415 ran,
5 refused, 0 mismatched.

184 unit, 24 parser, 20 CLI, 4 exe, **4 x64**, 4 hostile, 6 C API, 4 bcc20-sys, differential (415 ran, 5
refused, 44 skipped, 0 timed out, 0 mismatched), crt, pp gate, fuzz - green; fmt and clippy clean.

### M10 reviewed - 2026-11-20 - the two types that were one type

The `--std=c11` commit, reviewed. It had already found one identity fault while being written - a declared
`long` *was* the `int` type object, which is why `_Generic(l, int: 1, default: 9)` answered 1 - and the
review found the second of the same shape.

**`signed char` was `char`.** C99 6.2.5p15 makes `char`, `signed char` and `unsigned char` three types; the
fundamentals table had only two, and the specifier path folded `signed char` into `char` because it keyed on
`(signed, bits)` and both are a signed byte. A `_Generic` on a `signed char` therefore matched the `char`
arm. Fixed by adding the third fundamental and by letting the *spelling* choose, as `long` now does: the
table is a table of types, and a lookup is the only thing that can tell two same-width types apart.

**What is left of the same fault, and why it stays.** `usual_arithmetic` builds its result from the operand
shapes, so `long + long` is typed `int`. The two are the same width here, so nothing a program *does* can
observe it - only a generic selection on an arithmetic result, which is a corner of a corner, and fixing it
means a rank in the type object. That is written at the function now rather than discovered later.

**And one gap this commit makes visible.** The C API has no options at all - a host compiles as C99 at level
0, and `--std=c11` is a CLI-only flag. That was the right shape for M7's minimal API and it is no longer
obviously right: a feature the command line can select should be one a host can select. Recorded here rather
than built, because an options struct is an API decision with a lifetime, not a flag.

**Measured after the review: 419 corpus programs agree, 45 refused, 0 wrong, 59 660 bytes.**

184 unit, 24 parser, 20 CLI, 4 exe, 3 x64, 4 hostile, 6 C API, 4 bcc20-sys, differential (412 ran, 8
refused, 44 skipped, 0 timed out, 0 mismatched), crt, pp gate, fuzz - green; fmt and clippy clean.

### M10 reviewed - 2026-11-20 - a number carried from another question

The last commit was two lines of documentation about a dropped optimisation, so the review went looking
for what the plan *says* - and found a contradiction that had survived several turns of work.

**`interval splitting` was in two lists at once.** One entry lists it under "measured and dropped, with the
numbers ... (254 spill instructions corpus-wide)"; another, twenty lines later, is an open item whose first
task is "interval splitting". A reader can follow either and be wrong. It is the worst kind of
documentation defect: not a stale comment in code the compiler would contradict, but two instructions in
the same file, neither of which anything tests.

**And the number under it was measured for a different question.** The 6% ceiling that the plan and the
last several recommendations have carried came from `2 119 copies that liveness blocks` - which is a
*coalescing* ceiling, not a splitting one. A copy that cannot be coalesced is a `mov`; shortening a live
range does not remove it. Measured properly, what interval splitting itself can remove is spill traffic:
**125 accesses over 419 programs, 81 loads and 44 stores, about 625 bytes (1.0%)**. That is less than the
1.5% the address dedupe won and less than the 2% items already dropped for not paying, so it is recorded
as dropped with that number, and the round-six item no longer names it.

**What is left in round six is what nobody has measured**: jump tables for `switch`. That is the honest
state of the list - one unmeasured idea, and five ideas with numbers that say no.

The "254 spill instructions" figure is replaced rather than deleted, because the entry is a record of what
was believed: it now says what was measured and when it was re-measured.

**Measured after the review: 419 corpus programs agree, 45 refused, 0 wrong, 59 660 bytes** - unchanged,
since only documentation moved.

184 unit, 23 parser, 20 CLI, 4 exe, 3 x64, 4 hostile, 6 C API, 4 bcc20-sys, differential (412 ran, 8
refused, 44 skipped, 0 timed out, 0 mismatched), crt, pp gate, fuzz - green; fmt and clippy clean.

### M10 reviewed - 2026-11-20 - the scope of a fact, and the place it is written

The address-dedupe commit, reviewed. No defect: the two questions it raises have answers that hold by
construction, and the review's work was to *ask* them and then to fix what the answers made wrong in the
comments.

**Is the key sound?** `Address::Local(i32)` keys on a frame offset, and that is only a key if two slots
cannot share one - `allocate` bumps `frame_bytes` and never reuses it, so every slot has its own offset,
and reusing a slot's address value is therefore reusing that slot's address. `Address::Symbol(String)` keys
on a link name, unique per object in a translation unit. And the aliasing itself is the same shape as the
free widening: two `cir` values, one machine value, safe because `cir` defines each value once.

**Is exactly one place making them?** Yes - `address_of` is the only construction site for
`SymbolAddress` and `LocalAddress` in the lowering, so the map cannot be bypassed by a second path. That is
worth checking rather than assuming: a second `ops.push(Op::SymbolAddress { .. })` somewhere would dedupe
nothing and nobody would notice.

What the review did fix is where the facts are *written*:

- The clear that scopes the map per block had been inserted between the comparison-fusion comment and
  `fusable_compare`, so the comment describing the fusion sat on top of a line about addresses. A comment
  is attached to the code below it; three lines of prose above an unrelated statement is how a reader
  learns to skim.
- `address_of`'s doc said "if this *function* has not made it already", which was true of the first version
  of this change and false of the one that landed - the scope is the block, and the reason it is the block
  is a measurement. A comment that keeps the superseded scope of a fact is exactly the kind that survives a
  review of the *code* and misleads the next person.

**Measured after the review: 419 corpus programs agree, 45 refused, 0 wrong, 59 660 bytes** - unchanged.

184 unit, 23 parser, 20 CLI, 4 exe, 3 x64, 4 hostile, 6 C API, 4 bcc20-sys, differential (412 ran, 8
refused, 44 skipped, 0 timed out, 0 mismatched), crt, pp gate, fuzz - green; fmt and clippy clean.

### M10 reviewed - 2026-11-20 - the string that was put where the previous one went

The file-scope-address commit, reviewed - and the review turned up the *neighbouring* initialiser shapes,
which had never been tried.

**A string literal in a brace list initialised one `char`.** `char grid[2][4] = {"ab", "cd"}` descended
from the array to the inner array to its four `char`s, and tried to put a `char[3]` into one of them:
"cannot initialise char with pointer to char". It had always done that, inside a body as well as at file
scope - a table of `char` arrays, which is how every C program spells a list of short strings, was simply
not a thing this initialiser reader could do. The literal now initialises the object at the level it
belongs to, which is a `char` array: `char_array && literal` in `initializer_body`, and one
`literal_initialiser` that both the top-level and the element path reach instead of two copies of the same
twenty lines.

**The first attempt at that guard was too wide, and the tests said so within the minute.** "The next token
is a literal" is not the question; "the next token is a literal *and this level is a `char` array*" is.
The wide version fired on `char *names[] = {"a", "b"}` - whose elements each take a literal - and on
`struct entry t[] = {{"x", 1}}`, and refused both with "a string literal initialises an array of `char`,
not pointer to char". A guard is a statement about the shape it accepts; this one had been written about
the shape it hoped for.

**And a store at the wrong offset, found by making the case reachable.** The local path wrote each byte of
a literal at `offset`, the literal's offset from zero, rather than at the *object's* offset plus it - so in
`char grid[2][4] = {"ab", "cd"}` the second string landed on top of the first, and a program whose answer
should have been `'a'` answered `'c'`. Nothing could reach that code before, because the element case never
got there; the fix is `at + offset` and a `self.offset` advance that the file-scope path already did.

**The corpus says the neighbour is not rare.** `char *p = "hello"`, the table of strings, the table of
`char` arrays, a struct array whose members are strings, and the static pointer inside a function - five
shapes, and the construct now runs all five through one function whose answer is checked against nothing
but arithmetic.

**Measured after the review: 419 corpus programs agree, 45 refused, 0 wrong, 60 556 bytes** - unchanged,
because the corpus had no `char` array of arrays to gain.

184 unit, 23 parser, 20 CLI, 4 exe, 3 x64, 4 hostile, 6 C API, 4 bcc20-sys, differential (412 ran, 8
refused, 44 skipped, 0 timed out, 0 mismatched), crt, pp gate, fuzz - green; fmt and clippy clean.

### M10 reviewed - 2026-11-20 - a module comment that outlived its subject

The staging-area commit, reviewed. No defect this time - the two it exposed were fixed in the commit
itself - so the finding is about what the code says rather than what it does.

**Five places still described the staging area as a live mechanism.** The module comment at the top of
`encode.rs` - the first thing a reader of this file meets - said the layout was "shadow space, the
callee-saved registers, the lowering's frame, spill slots, then the staging area", with "no `push`". Both
halves were false by the time this commit landed: the staging area is gone, the integer registers *are*
pushed, and the floats are what stays in the frame. `Op::Call`'s doc said the arguments are "staged through
the frame when one of them would destroy another", which is the fallback that no longer exists.
`place_arguments` said "only the staged path has a copy to make", about a path that is gone. And ADR-008
described the same layout and the same staging, with a rationale - "a staging area has no cycle in it" -
that is still true and no longer describes the code.

A comment that is wrong is worse than an absent one, and these were wrong in the first paragraph a new
reader sees. All five are rewritten, and ADR-008 now records the two decisions that were taken back
*purposefully*, with the measurements that took them back, rather than reading as though the design had
always been the one that survived.

The smaller ones:

- The `debug_assert` guarding the collision invariant did not say it was a debug-only check, which matters
  here: there is no longer a fallback to select, so a release build *trusts* the invariant rather than
  testing it. The comment says so now.
- `integer_from_float`'s comment claimed "the fifth time this back end has paid for that order", a number
  nobody can check at the call site. It names the actual failure shape instead - `movq %xmm15, %rax` where
  `movq %xmm7, %r8` was asked for, an instruction that names two registers the operation never mentioned -
  and states the rule: the prefix and `modrm` must be told the same thing in the same order.

**Measured after the review: 418 corpus programs agree, 46 refused, 0 wrong, 60 476 bytes** - unchanged,
which is what a comment review should cost.

184 unit, 23 parser, 20 CLI, 4 exe, 3 x64, 4 hostile, 6 C API, 4 bcc20-sys, differential (412 ran, 8
refused, 44 skipped, 0 timed out, 0 mismatched), crt, pp gate, fuzz - green; fmt and clippy clean.

### M10 reviewed - 2026-11-20 - a number that meant two things, and the two out of sixty-six

The push/pop commit, reviewed - and the review found a **wrong answer**, not a smell.

**`frame` meant two different things and the code assumed one.** It had always been "how far `rsp` is
below its value on entry", and five places read it: the `sub`/`add rsp`, the variadic home area, the
incoming stack arguments, and `va_start`'s `lea`. Pushing callee-saved registers moved `rsp` *before* the
subtraction, so the subtraction had to shrink by the pushed bytes - and the patch shrank the field itself
instead of only the thing subtracted. The patch built `let frame = total - pushed` and stored that, which
is right for the two `sub`/`add` sites and wrong for the other three: `va_start` pointed 48 bytes below
the home area in a function that pushed six registers, and `sum(4, 10, 20, 30, 2)` answered **72 where
clang answers 70**. Silent, and no corpus program puts a `va_list` and a pushed integer in one function
(`dmath` and its three neighbours save only XMM registers, so `pushed` is zero and the arithmetic
coincidentally held).

The field keeps its one meaning and the two subtraction sites subtract `self.pushed()`. That is fewer
lines than the version that was wrong, and it makes the invariant true for every reader instead of only
the two that were edited.

**Why nothing caught it.** The construct table had a variadic *reader* and a variadic *definition*, and the
one place they met was a function that calls nothing - so no pushes. The new construct is a variadic
function that also calls something, which is exactly where the two mechanisms cross; checked the way a
test should be, by putting the wrong subtraction back and watching it answer 72.

**Measured after the review: 418 corpus programs agree, 46 refused, 0 wrong, 60 964 bytes** - the same
bytes as the commit, one construct richer and one wrong answer gone.

184 unit, 23 parser, 20 CLI, 4 exe, 3 x64, 4 hostile, 6 C API, 4 bcc20-sys, differential (412 ran, 8
refused, 44 skipped, 0 timed out, 0 mismatched), crt, pp gate, fuzz - green; fmt and clippy clean.

### M10 reviewed - 2026-11-20 - the width that was not a widening, and a panic the corpus could not see

Two commits reviewed: the truth-test pass, and the free widening.

**The widening shortcut took a conversion that is not a widening.** The guard was
`to_class == Class::Integer && to_size == 8`, and `(long long)1.5` satisfies it - destination an integer,
size eight - while its *source* is a `double`. Aliasing the two values therefore handed an integer reader
a register holding a float, and the back end **panicked**: `a floating-point result in a general register`.
Not a wrong answer, but not a diagnostic either: a compiler that exits 101 on a working C program. Both
files have to be the integer one, and the comment now says why the destination alone is not the question -
the value in the register is a `double`, and reading it as an integer is a reinterpretation.

**Why nothing caught it.** The corpus never converts a double to a 64-bit integer, and the construct table
had float conversions only to `int`. Both were green. The construct is now there - 3 750 000 000 needs all
sixty-four bits, so a 32-bit path would have hidden the fault rather than shown it - and it was checked the
way a test should be: with the guard removed again, it fails with the panic; with the guard, it answers 7
where clang answers 7.

The rest of the review:

- The shortcut also called `classify(src, Class::Integer)`, which with both files named is provably the
  same value it already had. Deleted, and the comment says what makes it unnecessary instead of the code
  repeating it.
- `Changes::simplified` had no reader: the new pass reported a count nothing printed or asserted. Three
  tests now assert it, which is better than either printing it or dropping it - the field is a claim about
  what the pass did, and a claim needs a witness.
- The negated-condition test asserted the wrong number, and the reason is interesting: `!(a < b)` in `cir`
  is `(a < b) == 0` *and then* a truth test of the result, so the pass removes one comparison (the truth
  test) and keeps two (the comparison and its negation). The assertion now says which one, which is a
  sharper statement of the rule than "nothing happened".

**Measured after the review: 418 corpus programs agree, 46 refused, 0 wrong, 64 156 bytes** - the same
bytes, one construct richer.

184 unit, 23 parser, 20 CLI, 4 exe, 3 x64, 4 hostile, 6 C API, 4 bcc20-sys, differential (412 ran, 8
refused, 44 skipped, 0 timed out, 0 mismatched), crt, pp gate, fuzz - green; fmt and clippy clean.

### M10 reviewed - 2026-11-20 - a count one decision had edited, and a scan that asked twice

The immediate-operand commit, reviewed.

**The read counts were no longer read counts.** `run` built them, and then *edited* them - a function
address whose only reader is the call that names it was removed from the map so that nothing would
materialise it. That was already load-bearing before this commit; what changed is that two more decisions
began asking the same map "is this read exactly once", and one of them (fusing a comparison into its
branch) would now be told a count that had been adjusted to make a different decision come out right. The
field keeps the counts `cir` wrote and the direct-call pass edits a copy. A measurement that has been
tuned is not a measurement.

**The op list was in two places.** The lowering carried `matches!(op, Add | Sub | And | Or | Xor)` to
decide what may hold an immediate, and the encoder carried the same five in its `/ext` table with an
`unreachable!` for anything else. Add a sixth operation and one of the two answers is wrong - and the
wrong one is the silent one. The table is now `encode::immediate_extension(op) -> Option<u8>`, the
encoder's own answer, and the lowering asks it.

**The scan was quadratic.** For each constant the code asked "is anything reading this one as an
immediate operand" by walking every instruction of the function: 4 000 instructions and 1 000 constants is
four million comparisons, per function. It is two linear passes now - one to note where an immediate field
can hold a constant, one to collect them. This back end has already paid for one review of a quadratic
scan (`Positions::new`, and a position-to-block table that was cubic), and neither was hard to see from
the shape of the loop.

The smaller ones:

- A closure `|lowering: &mut Self| lowering.reg_for(*right)` existed only to hand `self` past the borrow
  checker; it was a pass-through with a parameter nobody needed, and the arms call the method directly.
- `binary_immediate`'s target choice was three branches of which two did nothing different: when the
  destination's register *is* the left operand, `target != left` is already false. One `if`.
- Moving `immediate_extension` out of the `impl` block cost three attempts and briefly left the file
  unbalanced - `src/back/x64/encode.rs` would not compile between them. Worth writing down because the
  tooling that found it was `cargo build`, not a test: a patch applied by script to a file whose shape
  the script does not know is a patch that needs the compiler's answer before the next one.

**Measured after the review: 418 corpus programs agree, 46 refused, 0 wrong, 73 432 bytes** - byte for
byte what the commit produced, which is the standard a refactor is held to.

181 unit, 23 parser, 20 CLI, 4 exe, 3 x64, 4 hostile, 6 C API, 4 bcc20-sys, differential (412 ran, 8
refused, 44 skipped, 0 timed out, 0 mismatched), crt, pp gate, fuzz - green; fmt and clippy clean.

### M10 reviewed - 2026-11-20 - the skip and the decision, and three comments that lied

Two commits reviewed: the constant forms, and the compare-and-branch fusion.

**The finding is a latent wrong answer.** `fusable_compare` decided *which instruction to skip*, and
`lower_term` decided *whether to fuse* - separately, with the floating-point test in the second one. The
instruction was already left out of the op list by the time the second decision was made, so a
floating-point comparison read only by its branch would have branched on a value nothing defines. It is
not a live bug, and the reason is luck worth writing down: the front end wraps every condition in
`!= 0`, so what the branch carries is always an *integer* comparison and the float one is still lowered as
a value. A pass that folded `cmp ne (cmp x, y), 0` into `cmp x, y` would have made it live. The test is
now in `fusable_compare` and nothing is skipped that is not also fused - the two decisions are one
`Option<FusedCompare>`, and the doc says why.

**Three comments asserted checks that do not exist.** The old `fusable_compare` claimed "the comparison
must be ... the last of them: a branch cannot carry a comparison whose operands may have been changed
since", and then "every instruction after it writes something the comparison does not read, or the fusion
would change what the comparison sees". Neither was checked, and neither was needed: a `cir` value is
defined once and never written again, so nothing between a comparison and its branch can change what it
reads. A comment that describes a check the code does not make is worse than no comment - the next reader
either trusts it or re-derives the truth. Both are replaced by the invariant that actually holds.

The rest:

- `fusable_compare` returned a four-tuple that two call sites re-split with `map(|(index, _, _, _)| ...)`
  and `map(|(_, op, left, right)| ...)`. A named `FusedCompare { index, op, left, right }` says what the
  four are, and `lower_term` takes one instead of a second tuple shape derived from the first.
- `compare` and `compare_flags` each encoded `cmp` themselves - four duplicated lines of the one thing a
  comparison always does. `compare` now calls `compare_flags`, which hands back the two registers, and
  each caller releases when it is done with them.
- A construct now pins all three shapes at once: a comparison read once by a branch (fused), a
  floating-point one read once (not fused, and the reason is the ABI's unordered cases), and one read
  twice (a value, and it must not be fused). `30` from three paths through one function.

**Measured after the review: 418 corpus programs agree, 46 refused, 0 wrong, 80 936 bytes** - the same
bytes as before the refactor, which is the point: a review that changes behaviour is a feature.

181 unit, 23 parser, 20 CLI, 4 exe, 3 x64, 4 hostile, 6 C API, 4 bcc20-sys, differential (412 ran, 8
refused, 44 skipped, 0 timed out, 0 mismatched), crt, pp gate, fuzz - green; fmt and clippy clean.

### M10 reviewed - 2026-11-19 - the direction nobody had tested, and a knob nobody turned

The `printf("%f")` commit, reviewed. Its feature was three lines; what the review found is that the
*test* for it was half a test.

**A convention has two sides, and only one of them was foreign.** The interop test put a clang-compiled
caller against our callee (variadic, doubles) and, in the other direction, a clang-compiled *`g`* against
our caller - but `g` takes fixed arguments, so the one thing our caller had just learned to do, write a
variadic `double` into **both** register files, was never checked against a callee that did not come from
this repository. That case is now in the table: our caller, clang's `pick` reading `va_arg(ap, double)`.
It is the only test that fails when the duplicate is removed - the other two pass without it, because a
clang *caller* duplicates for us and our own callee reads what we wrote. Measured by deleting the two
lines: `exited with Some(0), expected 28`.

**`not_lowered_with(span, what, note)` had no caller left.** The `note` parameter was added for the varargs
refusal - the one gap whose reason was a fact about the ABI rather than about the source - and the same
commit that closed the gap left the parameter behind, called from exactly one place with `None`. A knob
nobody turns is dead code with a name, so it is gone; `Diagnostic::notes` stays, because it is the host's
API and `render.rs` shows it.

The rest, small:

- `no_vector_arguments` became `no_vector_registers_used`. The old name described the *policy* and was
  false about the call: we do pass vector arguments - in both files - and what we say in `AL` is that a
  callee should read none of them. A name that contradicts what the code does is worse than a vague one.
- The Cranelift refusal was the only message in `back/mod.rs` using `concat!` and `.to_owned()`; its
  neighbours use one continued literal and `.into()`. Now it matches, and it ends in the same words as
  the `va_arg` refusal above it - "Level 2 passes both".
- `place_arguments`'s new doc said a variadic call copies each float into both files without saying that
  only the *staged* path has a copy to make. The direct path is all-integer by its own test, so it has
  none; a reader should not have to derive that from a condition forty lines below.
- The support matrix's M8 row still said a callee that reads `AL` "finds it at whatever it was", which the
  new M10 row directly contradicts. Fixed, and it now says what is true: a call with no floating-point
  argument leaves `AL` alone, which is harmless because the callee reads no vector registers.

**Measured after the review: 418 corpus programs agree through the own back end, 46 refused, 0 wrong**,
98 776 bytes of `.text` (5.74x `clang -O2`), and the ABI test now covers three directions instead of two.

181 unit, 23 parser, 20 CLI, 4 exe, 3 x64, 4 hostile, 6 C API, 4 bcc20-sys, differential (412 ran, 8
refused, 44 skipped, 0 timed out, 0 mismatched across three compilers), crt, pp gate, fuzz - green;
fmt and clippy clean.

### M10 reviewed - 2026-11-19 - a rule written twice is a rule broken once, and 60 bytes that proved it

Four commits reviewed together: `va_arg` of a `double`, and the ABI fixes its test forced.

**The finding that matters is structural.** The last commit fixed the argument register rule twice -
once in `encode.rs` for the caller, once in `lower.rs` for the callee - and got it wrong once more, in
`regalloc.rs`, because *which registers a function may use* was two decisions in two modules: integers
in `mod.rs` behind `param_count <= 2`, floats in `regalloc.rs` behind `param_count > 3`. Two thresholds,
neither derivable from the other, guarding one invariant - **a register a parameter arrives in is not
allocatable** - and the bug that shipped was the half whose threshold nobody had revisited. The fix is
not a third threshold: `Registers` now carries both pools, and `left_to_lend` subtracts the arriving
argument registers from a candidate pool in one place. The number that pays for the refactor instead of
the other way round: **96 348 -> 96 288 bytes**, because the old thresholds were over-conservative as
well as wrong - a three-parameter leaf could not use `r9`, which nothing arrives in, and now can.

The rest of the review, smaller:

- `Op::VaArg { float: bool }` became `class: Class`, matching `Op::Incoming`. A boolean that restates
  what a class already says is a second encoding of one fact, and the encoder branches on it either way.
- The argument tables moved from `encode.rs` to `mod.rs`, beside `ARGUMENT_COUNT` and the rule that
  reads them - a fact about the ABI filed under the encoder is how the encoder came to own half of it.
  Their old doc comment still claimed `f(int, double)` puts the `double` in `xmm0`, which is the bug
  this milestone fixed; deleted rather than corrected, since the tables now say it once.
- `Op::Incoming`'s `slot` doc said "which register of its class", true before the position rule and
  misleading after: it is the index into *both* files at once.
- The interop test lost its boilerplate to a `linked_with_clang` helper, and its sources became raw
  strings instead of `concat!` with embedded newlines.

**Measured after the review: 414 corpus programs agree through the own back end, 50 refused, 0 wrong**,
and 96 288 bytes of `.text` (was 96 348; `clang -O2` 16 715, 5.76x).

181 unit, 23 parser, 20 CLI, 4 exe, 3 x64, 4 hostile, 6 C API, 4 bcc20-sys, differential (412 ran, 8
refused, 44 skipped, 0 timed out, 0 mismatched across three compilers), crt, pp gate, fuzz - green;
fmt and clippy clean.

### M10: the harness compiles both lowerings - 2026-11-16 - and the plan's number against clang

M10's fourth item: "a differential harness that compiles with both lowerings and compares behaviour,
so moving work between them is safe". It is the *same* test as the corpus gate, not a second one:
`crates/xcc-cli/tests/differential.rs` now compiles and runs every corpus program at
`--opt-level=0`, at `--opt-level=1`, and with `clang -O0`, and every *pair* of the three answers must
agree on the exit code and on stdout.

Three details that decided the shape:

- **Three answers, every pair compared.** Comparing each lowering with clang and stopping there would
  miss the case the item exists for: two lowerings that agree with each other and disagree with C.
  The pairwise loop says "opt-level=0 against opt-level=1" in the message when that pair is the one
  that moved, which is the sentence a reader needs.
- **A refusal counts as an answer.** A program one level compiles and the other refuses is reported
  as a mismatch naming which way round it is - the failure a compiler is most likely to have first,
  and the one an exit-code comparison alone would hide as "both fine".
- **It replaced work rather than adding it.** The extra lowering costs 18% of the test's time (184 s
  against 156 s) because the corpus is already spread over cores; a second copy of the loop would
  have cost the full time again and drifted from the first.

**Measured: 412 programs agree across all three, 8 are refused at both levels, 0 mismatched.**

**And the number the plan's acceptance asks for.** With the stick in place, the same six files:

| | x64 bytes | against clang -O2 |
|---|---:|---:|
| Borland C++ 2.0 (16-bit) | 1233 | - |
| `clang -O2` | 1037 | 1.00x |
| xcc `--opt-level=1` | 1981 | **1.91x** |
| xcc `--opt-level=0` | 3281 | 3.16x |

So the level-1 path is within **1.9x** of `clang -O2` on this corpus, with every result checked
against the same program built by clang - the plan's acceptance has a number to state now, and it
does not flatter the compiler.

174 unit, 23 parser, 20 CLI, 3 exe, 4 hostile, 6 C API, 4 bcc20-sys, differential (412 ran, 8
refused, 44 skipped, 0 timed out, 0 mismatched across three compilers), crt, pp gate, fuzz - green;
fmt and clippy clean.

### mem2reg with parameters reviewed - 2026-11-16 - the quadratic that was hiding behind a comment

Review of `915dd96`: block parameters in `mem2reg`, and the dominator order the back end needed.

**Found: the verifier's dominator analysis was `O(blocks^3)` and ran on every compile.** It built an
`n x n` matrix by iterating to a fixpoint, and since the passes started verifying in the compile path
it ran for every function. Measured on 20000 `if` statements: **longer than two minutes**, i.e. a
denial of service in one line of input. Two separate faults, both mine: the algorithm, and the fact
that nothing bounds how many blocks a function may have.

**Fixed two ways, because they are two problems.** `cir` computes dominators by Cooper-Harvey-Kennedy
now - one `Vec<Option<Block>>`, a handful of passes over the blocks in reverse postorder - and
`dominates` walks that chain instead of consulting a matrix. And `Budgets::max_blocks` (4096) refuses
a function past the size where the polynomial matters, with a diagnostic that says what the limit is
for. Measured after: the 20000-`if` function is refused in a second, and a thousand `if`s (2000
blocks, inside the budget) compiles in 3.4 s where it took 16.2 s.

**Found in passing: `dominates` recomputed the tree per call.** My first version of the new API did
`self.dominator_tree()` inside `dominates`, and the verifier calls it once per use - so the fix above
would have been quadratic in *uses* instead of in *blocks*. The verifier threads one tree through
every check now (`dominates_with`), and the public shape keeps the convenient one-call version for
callers that ask once.

**Checked and sound:** the CHK implementation against the old matrix for every function in the corpus
- the differential runner would have caught a wrong dominator (it changes what `mem2reg` may
promote), and it is unchanged at 412 ran, 8 refused, 0 mismatched; the code sizes are unchanged too
(3281 bytes at level 0, 1981 at level 1, Borland's 1233), which says the rewrite is a change of
algorithm and not of answers; the entry block can never receive a parameter, because a block the
entry dominates has the entry as its immediate dominator and the frontier walk stops there; and
`feed` leaves an argument out rather than inventing one, so a missing edge argument is a verifier
error and not a wrong program.

174 unit, 23 parser, 20 CLI, 3 exe, 4 hostile, 6 C API, 4 bcc20-sys, differential (412 ran, 8
refused, 44 skipped, 0 timed out, 0 mismatched), crt, pp gate, fuzz - green; fmt and clippy clean.

### M10: mem2reg with block parameters - 2026-11-16 - the dataflow that could not see a loop, and the lowering order that had never been tested

`mem2reg` promoted only slots written once, which left every loop counter and accumulator in memory -
`arrays.c` spent 43% of its instructions on loads and stores, `control.c` 42%. The classic
construction is in now: block parameters where the iterated dominance frontier of the store blocks
says values meet, and a rename walk that gives each load the value of the last store that dominates
it.

**The first attempt was a dataflow fixpoint, and a loop killed it.** For each block, "the value on
entry" was the value all its predecessors agree on - which cannot compute a loop header: the header's
value depends on the back edge's, which depends on the header's. It converged, to the answer "no
value anywhere", and `int f(int n) { for (i = 0; i < n; i = i + 1) ... }` ended up with its parameter
still in memory. Renaming along the dominator tree is the answer, and the reason the classic
algorithm is written that way; the walk is iterative (an explicit `Enter`/`Leave` stack) because a
function with ten thousand blocks is a real function and the host's stack is not the compiler's.

**The second attempt found a bug that predates it.** With parameters feeding themselves around a
loop, the lowering failed with "a value has no Cranelift value": the back end lowered blocks in the
order the parser made them, and the parser makes a loop's join block *before* its body - so a use
could name a value whose defining block had not been lowered yet. Nothing in `cir` promised
creation order, and nothing had tested it until block parameters made a use cross a block boundary
backwards. The back end lowers in **dominator-tree preorder** now, which is the order SSA guarantees.

**Measured on the six-file corpus, after:**

| | x64 bytes | vs Borland (1233) |
|---|---:|---:|
| before mem2reg | 4857 | 3.9x |
| mem2reg, one store | 3822 | 3.1x |
| mem2reg with parameters, `--opt-level=0` | 3281 | 2.7x |
| the same at `--opt-level=1` | 1981 | **1.6x** |

The differential runner is unchanged at 412 ran, 8 refused, 0 mismatched - including the loop and
switch programs whose parameters are new. 174 unit, 23 parser, 20 CLI, 3 exe, 4 hostile, 6 C API,
4 bcc20-sys, crt, pp gate, fuzz - green; fmt and clippy clean.

### `--opt-level` reviewed - 2026-11-16 - one decision printed three times

Review of `4ec8352`: the opt-level flag and the numbers behind it.

**Found: "2 and 3 are not written" was said in three places.** The object path mapped `0`/`1` to
Cranelift's spellings and its own message; the JIT mapped the same two numbers again, with a
different message; and the front end checked the range a third time so the refusal would not arrive
as an `internal:` error. Three copies of one decision, each free to drift. There is one function now
(`back::cranelift_opt_level`) that answers either the spelling or the reason, and the three callers
use it: the object path maps the reason to its error type, the JIT returns it, and the front end
turns it into the diagnostic.

**Found: the check ran after the source was parsed.** A wrong opt level is not a fact about the
source, so it is now asked first - before the preprocessor, before the parser - and the diagnostic
does not depend on the source compiling at all. (It also stopped constructing a `SourceFile` twice
in the error path.)

**Checked and sound:** the default is still `0`, so `cargo test`, the differential runner and every
existing behaviour are on the path they were; `Jit::with_symbols` keeps level 0 explicitly, so a
host that passes symbols does not silently get an optimiser it did not ask for; a level the JIT is
given is validated where its flags are set, which is the only place it could take effect; and the
measurement in the help text and the audit is the one the compare example prints.

174 unit, 23 parser, 20 CLI, 3 exe, 4 hostile, 6 C API, 4 bcc20-sys, differential (412 ran, 8
refused, 44 skipped, 0 timed out, 0 mismatched), crt, pp gate, fuzz - green; fmt and clippy clean.

### M10: `--opt-level`, and the measurement that made it worth a flag - 2026-11-16

M10's `--opt-level` line, and the lesson that came before writing it.

**First I measured what a pass would buy, and the answer was nothing.** After `mem2reg` the six-file
corpus has **zero** instructions whose result is unused, so a dead-code pass had nothing to remove -
a fact worth having before building one. The plan's next item could have been DCE and it would have
been forty lines of measured nothing.

**Then I flipped one string.** The back end has always lowered with Cranelift's `opt_level=none`,
chosen in M4 for compile speed. Setting it to `speed` and re-measuring took the six files from 3822
to **2459 bytes** and the gap to Borland from 3.1x to **2.0x**, for 55 ms -> 180 ms of compile time.
That is the largest single result in M10 so far and it is one line, which is exactly why it is a
*flag* and not a new default: `0` compiles fastest and is what the differential runner and CI use,
`1` buys the code size, and the number a caller passes means what it says.

**`2` and `3` are refused, as the caller's mistake.** They are this compiler's own back end in the
plan, which is not written. Serving them from Cranelift would make one number mean two things, and
routing the refusal through the back end's error made it arrive as `internal:` - a compiler bug
message for a number the caller chose. It is a source diagnostic now, with no `internal` in it.

**Soundness, measured rather than assumed:** all **414** corpus programs that run under one level run
under the other with the same exit code and the same output, and the same 50 are refused at both. A
JIT test compiles three programs at both levels and compares their answers, so the property is
checked where a change is noticed first.

173 unit, 23 parser, 20 CLI, 3 exe, 4 hostile, 6 C API, 4 bcc20-sys, differential (412 ran, 8
refused, 44 skipped, 0 timed out, 0 mismatched), crt, pp gate, fuzz - green; fmt and clippy clean.

### mem2reg reviewed - 2026-11-16 - a field that counted the wrong thing, and a pass that could be slower than the compiler

Review of `cc49f70`: the first M10 pass.

**Found: `Changes.slots` counted loads and stores, not slots.** `mem2reg::run` answered how many
memory instructions it removed and the caller stored that in a field called `slots`; the test then
asserted the wrong number (4 slots for two slots), which is how a lie in a statistic becomes a
expectation. `run` answers the slot count now, `memory` is derived from the before/after it always
was, and `Changes.promoted` - never written by anything - is gone.

**Found: the pass could be quadratic where the compiler must not be.** `used_values` and the escape
set were `Vec`s searched with `contains` per operand: a 100k-instruction function - which hostile
input can produce and the token budget allows - would have been 10^10 comparisons inside a pass that
runs on *every* compile. Both are `HashSet`s now. This is the kind of thing that only shows up on
input nobody writes by hand, which is the reason CONSTRAINTS 5 exists.

**Found: two call sites ran the passes and checked the result themselves.** `compile.rs` and
`jit.rs` each called `optimise` and then looped over functions to verify - the same six lines twice,
with different wording, and a third future caller to keep in step. `optimise` answers
`Result<Changes, (Symbol, Broken)>` now: it runs the passes, verifies after each function, and names
what broke. A caller maps the error to its own words and nothing else. The check also stopped being
a `debug_assert` in the compile path: a pass that breaks a function in a release build produced a
*wrong program*, which is the one thing a pass must never do.

**Checked and sound:** the promoted value always *dominates* every use it replaces (that is the
premise of the substitution, and the verifier checks it independently); a slot's removal takes its
bitcast chain, its one store and every load with it, and only when every load was replaced - the
bug that made `v3` undefined was exactly a slot removed while a load survived; the offset-0 fast
path in `address_within` keeps the *type* of the address it returns, and every caller passes a base
typed `pointer to <object>`; and the six corpus files still agree with clang, as do all 412 programs
the differential runner executes.

Measured again after the changes: **3822 bytes** of x64 on the six files (4857 before the pass),
55 ms to compile them (72 before), Borland's 1233 unchanged. 173 unit, 23 parser, 19 CLI, 3 exe,
4 hostile, 6 C API, 4 bcc20-sys, differential (412 ran, 8 refused, 0 mismatched), crt, pp gate,
fuzz - green; fmt and clippy clean.

### M10: mem2reg - 2026-11-16 - 21% smaller, and the four bugs that got it there

M10's first item, and the one the measuring stick said mattered most: the front end writes every
local into a stack slot and reads it back, so a three-line function was a dozen loads and stores of
values that never needed memory. `cir::pass::mem2reg` promotes the slots it can prove.

**What it proves, and no more.** A slot is promoted when its address never escapes (every use of the
address, or of any bitcast of it, is a load's or a store's `address`, or the next bitcast), when it
has exactly one store, and when every load of it is dominated by that store *in the store's own
type*. Anything else stays memory - including the case that needs a block parameter, a local
assigned in a loop. Writing that ceiling down matters: half of the four bugs below were the pass
promoting something it had not proven.

**The four bugs, in the order they were found, all by measurement rather than review:**

1. **A store whose address was a bitcast of an escaped slot was recorded as "the one store"** and
   then removed, while the loads stayed: `struct P p; p.x = 3;` returned 10 where C says 34. The
   escape analysis looked for the slot *value* in operands, and a bitcast of it is a different
   value. It follows the chain now.
2. **A store's value operand was treated as an address.** `int *p; p = &literal;` puts the slot's
   address where someone else can read it - the definition of escaping - and the escape check
   passed over it because it only asked whether the *instruction* was a load or a store.
3. **A load of a different type was promoted.** `union U u; u.i = x; return u.c;` read the stored
   `int` as the `char` it was loaded as: 101 where the byte is 68. Memory is exactly the mechanism
   that makes a pun work, so a type mismatch vetoes the slot.
4. **A replaced load could replace another load.** `int x = 42; int y = x;` mapped `y`'s load to
   `x`'s load and then removed `x`'s: a use naming an instruction that no longer existed. The map
   is resolved transitively before it is applied.

**Two things came out of it that are theirs, not the pass's.** The verifier now checks that every
value used is one a block still defines - the check that turned bugs 1 and 4 from a wrong answer
into a message naming the value - and `address_within` no longer emits a walk of *zero* bytes for a
leaf at offset 0: it was a multiply, an add and three bitcasts that meant "the same place", and
mem2reg could not see the slot through them.

**Measured, on the six-file corpus both ways:** 4857 -> **3822 bytes** of x64 code (-21%), and
63 ms to compile the six instead of 72. Borland's 16-bit code stays at 1233, so the gap the stick
measures narrowed from 3.9x to 3.1x - and the remaining difference is mostly register allocation,
which is what the next pass is. The differential runner is unchanged at 412 ran, 8 refused, 0
mismatched: the pass is sound on all 412 programs that run.

173 unit, 23 parser, 19 CLI, 3 exe, 4 hostile, 6 C API, 4 bcc20-sys, differential (412 ran, 8
refused, 44 skipped, 0 timed out, 0 mismatched), crt, pp gate, fuzz - green; fmt and clippy clean.

### M9 reviewed - 2026-11-16 - a fixture nobody read, and a path the constraints said should not ship

Review of `add5a70`: the measuring stick.

**Found: the OMF fixture was committed with nothing reading it.** The test that used
`tests/aggregates.obj` - the 401-byte object the port actually wrote, which is where the checksum
byte and Borland's segment names live - was written, then removed in the same turn while I renamed
it, leaving a binary in the repository and the lesson untested. It is `tests/omf.rs` now, and asserts
both readers on the same bytes: the code segment is 107 bytes, and the file is not a COFF object.

**Found: CONSTRAINTS 7 said the `--target=dos16` path is "test-only", and the code was not.** The
constraint is about licensing - libbcc20 is derived from Borland's binary and must never be linked
into a shipped artefact - and we do load it at run time and never link it, so the promise held. But
the *FFI crate* was an unconditional dependency of the CLI, so a released `xcc.exe` carried the
whole path. It is a cargo feature now (`dos16`, on by default): `--no-default-features` builds a CLI
with no `--target` flag, no FFI and no OMF reader, which is what "test-only" means when a build has
to be able to leave it behind. Measured both ways: `--target=dos16` works in the default build,
and a no-default build answers "unexpected argument '--target'".

**Found: `Outcome.status` preferred the wrong of two equal values.** `bcc20_compile` returns the
status *and* stores it, and the code took `result.status` when it was non-zero - but when the library
refuses to run at all (a job over its file limit) it returns -1 and never touches the result, which
we zeroed: that would have read as success. The return value is the one to trust, and the comment
says why.

**Checked and sound:** the FFI's layouts against `bcc20.h` field by field, including the function
pointer and the `Option<fn>` null; the job's pointers all outlive the call, and the result is freed
through the library's own `bcc20_free`; `GetProcAddress`'s null becomes `None` rather than a
transmuted zero; the corpus is C89, freestanding and deterministic as its README says; and the
comparison's two readers answer `None` on a file they do not understand instead of guessing.

169 unit, 23 parser, 19 CLI, 3 exe, 4 hostile, 6 C API, 4 bcc20-sys, differential (412 ran, 8
refused, 44 skipped, 0 timed out, 0 mismatched), crt, pp gate, fuzz - green; fmt and clippy clean
(with and without the `dos16` feature).

### M9: the measuring stick - 2026-11-16 - a 1991 compiler as an oracle, and the build tree that lied

M9's item was to compare xcc against the compiler that built early-1990s DOS games, through
borland-reborn's port of `BCC.EXE`. It is now a table.

**The first attempt failed, and the failure is the interesting part.** The port's `build/` held
objects from a **Linux** checkout - ELF, with `/usr/include/x86_64-linux-gnu` in their debug paths -
and `make` judged them newer than the sources. It linked them into a "DLL" of 3.5 MB of zeroes and
`ld` answered `returned 5 exit status`, with no message, twice. A `make clean` and a rebuild on this
host's own toolchain produced a 13 MB `libbcc20.dll` in a minute. A build tree from another machine
is not a build tree; the next person should not pay that hour again, so ADR-007 records it.

**The FFI is loaded, not linked, and that is three decisions at once.** `BCC20_LIB`, else
`libbcc20.dll` on `PATH`, through `LoadLibraryA` and `GetProcAddress`: a machine without the port
still builds the workspace; a missing library is `None` so the comparison *says* what is missing
instead of failing to build; and a mingw DLL cannot be linked by an MSVC-targeted crate without an
import library that `cc` did not write. `bcc20.h` also says what the header knows and the caller
must: "not reentrant: one compile at a time per process" - the tests take a lock, and so must a
host.

**Two object readers, one question.** The comparison needs the code size from each format: COFF's
`.text` section for xcc, OMF's code segment for Borland. Reading OMF found a fact worth writing
down - every record ends with a **checksum byte** whose inclusion makes the record sum to zero -
which showed up as a `0xFC` that the LNAMES reader took for a 252-byte name. The reader stops at an
entry that would overrun the record now, and a fixture of a real Borland object (401 bytes, 107 of
them code) pins it, because a hand-built record set does not have a checksum in the right place.

**What the stick says.** On six freestanding C89 files: **4857 bytes of 64-bit code from xcc against
1233 bytes of 16-bit code from Borland** (aggregates 936/107, arith 1000/306, arrays 944/299,
control 824/201, globals 240/84, switch 913/236). Some of that is the target - 64-bit code is
bigger - and some is that Borland has register allocation and peephole optimisation and xcc has
neither. That is M10's number to move, and it is now measured rather than assumed. Compile time is
comparable: 72 ms against 49 ms for the six (one xcc compile each, including Cranelift).

**And the corpus had to be small for an honest reason:** Borland C++ 2.0's headers are not in this
repository (they come with the product), so a file that `#include`s anything cannot be built by
both. Six freestanding files are what both can compile here; the corpus README says so, and says
the rules (C89, no I/O, deterministic) that make it a comparison rather than a demo.

169 unit, 23 parser, 19 CLI, 3 exe, 4 hostile, 6 C API, 5 bcc20-sys, differential (412 ran, 8
refused, 44 skipped, 0 timed out, 0 mismatched), crt, pp gate, fuzz - green; fmt and clippy clean.

### M8 `sizeof` and the refusal count, reviewed - 2026-11-16 - two constraints skipped by one shortcut

Review of `8fb9c42`: the honest refusal count, and the `sizeof(&b[0])` fix.

**Found: the fix skipped `&`'s constraints along with its code.** Making an unevaluated `&` answer
its pointer type was right for the *place* - which may need arithmetic nothing will run - but the
early return came before the lvalue check, so `sizeof(&5)` was accepted in silence where clang says
"cannot take the address of an rvalue". Two different questions had been asked in one place: *is
this the right kind of operand* (a fact about its type, which holds whether or not the operand is
evaluated) and *can I find its address* (a fact about the code, which does not). The first is
checked first now, always; the second is skipped when nothing is evaluating.

**Found: a bitfield's descriptor was dropped whenever no address could be emitted.** `read_bitfield`
and `member_access` both returned a code-less value with `bits: None` on the way out of a path where
`emit` had nothing to say - so `sizeof(&s.a)` was accepted as well, where clang says "address of
bit-field requested" (C99 6.5.3.2p1). The place and the bits are *facts about the member*, not about
the code, which is what a write needs to merge into the right bits and what `&` needs to refuse; both
now travel with the value on every path, and only the code is optional. `read_bitfield` writes the
value once instead of five times, which is how it had drifted.

**Measured after: 412 ran, 8 refused** (was 411/9). `cc53_ptrrow2.c` - the program made entirely of
`&a[0]` expressions - moved from refused to running and agreeing. The eight that are left are: a
file-scope pointer to a string literal (ADR-004), four floating-point arguments to variadic calls
(ADR-006), and three *deliberately invalid* initialiser programs that clang merely warns about. The
runner prints their names now, so the shrinking set is visible rather than a number.

168 unit, 23 parser, 18 CLI, 3 exe, 4 hostile, 6 C API, differential (412 ran, 8 refused, 44
skipped, 0 timed out, 0 mismatched), crt, pp gate, fuzz - green; fmt and clippy clean.

### M8: the refusal count was flattering the compiler - 2026-11-16 - and a `sizeof` that crashed

The differential runner asked xcc first and counted a refusal without asking whether `clang` could
build the program at all. So "51 refused" was really "51 programs xcc would not compile", of which
most are programs *nobody* can compile with these headers - `mex.h`, POSIX `open`/`read`, a `printf`
called with no declaration (C99 has none), Borland's `word`. Both compilers are asked now, and only
a program `clang` builds and xcc refuses counts as a gap.

**The honest number is 9.** Of 464 corpus programs: 411 run and agree, **9 are refused that clang
builds**, 44 are outside the subset for anyone, 0 mismatch. The nine have reasons, and the reasons
are recorded in `crates/xcc/tests/parse.rs`'s table: four pass a `double` to a variadic function
(ADR-006's `AL`), one takes the address of a string literal at file scope (ADR-004's bytes cannot
express a relocation), one is the `&`-of-a-row case below, and three are *deliberately invalid* C
that clang merely warns about. A new ratchet says the count may only shrink.

**Found: `sizeof(&b[0])` compiled to an illegal instruction.** Valid C, no diagnostic, and the
.emitted function was `slot; trap` - a `return` whose expression produced nothing at all. The chain:
`sizeof`'s operand is unevaluated (C99 6.5.3.4p2), and `&` of a row of an array in a frame needs
arithmetic on a slot to find its place - which emits nothing while unevaluated, so `place` was
`None`, so `&` made it an error, so `sizeof` answered with the error type, so `return` emitted
nothing, so the block got an `unreachable`. A `sizeof` operand asks for a *type*, not a value:
`&` in an unevaluated context now answers its pointer type and no value, and says nothing about a
place it was never going to need. `cc53_ptrrow2.c` - which is entirely about that expression -
passes now, and a JIT test pins the global, local and row-arithmetic forms.

169 unit, 22 parser, 18 CLI, 3 exe, 4 hostile, 6 C API, differential (411 ran, 9 refused, 44
skipped, 0 timed out, 0 mismatched), crt, pp gate, fuzz - green; fmt and clippy clean.

### M8 `--run` and the C library, reviewed - 2026-11-16 - a JIT that could no longer be moved

Review of `59c439f`: the C runtime for `--run`, static locals, the label-block fix, the `break`
stack, and the exit through the runtime.

**Found: `Jit` stopped being `Send`.** Storing the runtime's addresses as `*const u8` removed the
auto `Send`, so a host that built a JIT at startup and moved it to a worker no longer compiled -
the "one `Jit` per thread, no global state" property the module has advertised since M4. The
addresses are process-global and immutable and carry no ownership, so `unsafe impl Send` says what
is true, with a note on why it is sound and why it is not `Sync`; a test is now a compile-time
guard, so a future field cannot quietly take the property away.

**Found: a diagnosed static initialiser produced a second, internal error.** When a `static`
local's initialiser could not be written, `static_local` gave up before creating the object, so the
name had no storage and every later use of it fell into the file-scope path and minted a
`GlobalAddress` for a symbol the compiler had never made: `internal: `n` is not declared here`, on
top of the real diagnostic. The object is created with zeroes now - the initialiser's own error is
the only one - which is what "one diagnostic per mistake" means.

**Found: the README said `--run` cannot call the C library**, which stopped being true in the same
commit that made it able to. It says what it can and cannot: one library, the `printf` family
included, `snprintf` and the stream accessor excluded (the classic runtime exports neither), and a
missing name is a diagnostic that names it.

**Checked and sound:** the `crt` list against the headers, mechanically - every function the
twelve headers declare is in it, with no name left behind; the `targets` stack is pushed and popped
on every path a switch can leave by, so a `break` outside a body is still refused rather than
aimed at nothing; `continue` finds the nearest entry with a continue target, which is what a
`do`-`while` needs (`test`, not `body`); a label now always gets a block, including the label at the
top of a function, where the entry block cannot be a target; and the `init.rs` change - accepting a
constant that also has code - is only reachable where `self.bytes` is set, which is the
file-scope and static-local paths.

167 unit, 22 parser, 18 CLI, 3 exe, 4 hostile, 6 C API, differential (411 ran, 51 refused, 2
skipped, 0 timed out, 0 mismatched), crt, pp gate, fuzz - green; fmt and clippy clean.

### M8: `--run` gets the C library, and four bugs the ratchet had been hiding - 2026-11-16

The JIT resolves symbols when it finalizes, so `--run` could not call the C library - and the
differential runner, which runs programs in process, could not see the 35 programs that `printf`
had just unblocked. `back::crt` looks the C runtime's own functions up by name, and the ratchet went
**376 -> 411 ran, 86 -> 51 refused**. Four real bugs came out of it, all older than this commit,
none reachable while those programs were refused.

**Static locals were ordinary locals.** `static int n = 0;` inside a function got a frame slot, so
it was re-initialised on every call and two functions' `static int n` were one variable. C99 6.2.4p3
gives it static storage duration: it is a global named `<function>.<variable>` (a name no C
identifier can spell), initialised from its constant initialiser or zero-filled, and the local name
means its *address*. One branch in `declaration`, one helper, and the initialiser walker learned
that a constant inside a body *also* emits an instruction - which is no reason to refuse its bytes
(that was the second half: the bytes path refused every value with code, which only file-scope
values never had).

**A label in the middle of a block re-ran the block.** `label_statement` reused the block the code
above was in when it was still open, so in `i = 0; again: i = i + 1; if (i < 3) goto again;` the
back-jump restarted at `i = 0` - an infinite loop. A label must be the *first* instruction of its
block; every label now gets a block of its own and the code above falls into it.

**`break` inside a `switch` inside a loop left the loop.** Two stacks (`targets` for loops,
`switches` for switches) cannot say which construct is nearer, and `loop_control` asked the loop
first. There is one stack now: a loop pushes `(exit, Some(continue target))` and a switch pushes
`(exit, None)`, so `break` takes the top and `continue` finds the nearest entry with a continue
target. The `Switch` stack keeps the arms and the case values, which is a different question.

**And `--run` lost buffered output.** `std::process::exit` calls `ExitProcess`, which discards
every byte `printf` has buffered - a program whose output goes to a pipe printed nothing, which is
also why three corpus programs' mismatches read `output "" against "6 6 6"`. The CLI now leaves
through the *same* runtime's `exit`, which flushes stdio and runs at-exit handlers: the reason
ADR-005's crt exists for a linked `.exe`, in five lines for an in-process run.

**One CRT, not two.** The first version of `crt` took each name from whichever of `ucrtbase` and
`msvcrt` answered first - which loads two heaps and two sets of stdio buffers into one process, so a
block from one `malloc` freed by the other's `free` is the corruption that gives C runtimes their
reputation. It loads exactly one library now (the classic one, which exports `printf` under its own
name; the UCRT offers it as an inline wrapper), and a name it lacks is absent, which `--run` says
plainly. `snprintf` is one such name - no Windows C runtime exports it - and a linked build has it.

166 unit, 22 parser, 18 CLI, 3 exe, 4 hostile, 6 C API, differential (411 ran, 51 refused, 2
skipped, 0 timed out, 0 mismatched), crt, pp gate, fuzz - green; fmt and clippy clean.

### M8 varargs reviewed - 2026-11-16 - the record that crossed the ABI as its own address

Review of `cd0da32`: variadic calls lowered as indirect calls, and the two bugs it exposed.

**Found: a small record passed to a variadic function went in as its address.** The call-site
signature said what the ABI says - a record of eight bytes or fewer crosses in an integer of its
width - but the argument pushed was the record's *address*, and those agree in type only by luck:
both are 64-bit. `printf("%d", p)` for `struct P { int a, b; }` printed the low half of a stack
address where clang prints 1. The surplus-argument path now takes the argument's own (promoted)
type as its "wanted" type, so the same record rules that apply to a declared parameter apply to a
variadic one: a small record is loaded as an integer, a big one travels by reference.

**Found: `(void *)-1` and `(char *)array` were refused.** Neither is varargs; both were on a test
of `sscanf` and `%p` that the previous commit made worth writing. An array operand of a cast must
decay first (C99 6.3.2.1p3) - `cast` called `unary_expression` and stopped, so an array arrived at
the conversion table as an array. And a *narrow* integer to a pointer had no conversion at all: the
table widened integers to integers and bitcast pointers to 64-bit integers, and treated everything
else as "cannot yet". C leaves the narrowing implementation-defined (6.3.2.3p5); the machine's
answer is to widen the value into the register as an integer would be widened - sign-extended when
signed, which is what clang does with `(void *)-1` (`== (void *)(long long)-1`, measured).

**Found: the differential runner's ratchet could fail on a busy machine.** A full `cargo test
--workspace` had one program time out on both sides and `ran` came out one below the ratchet, which
reads as "something refuses code that ran". A timeout on both sides is not a disagreement about a
value: it is the machine, or a corpus program that does not terminate under either compiler. It is
now counted as `timed_out`, printed, and added to the ratchet's side of the comparison - while a
hang on *one* side is still a mismatch, because that is a program this compiler made diverge.

Checked and sound: the promotions are applied to exactly the arguments past the declared
parameters, and only for a variadic call; a non-variadic surplus argument still pushes nothing (its
error already dropped the body); the call-site signature is built from the same `signature_of` a
declared function uses, so aggregates and the hidden result pointer behave identically; `func_addr`
on an imported `FuncRef` produces a relocation the object writer resolves, which is what makes
`printf` link; and the JIT's symbol check runs before lowering, so a missing symbol leaves the JIT
usable - the C API's second source still calls the first one's functions, which it now has to
remember across sources.

163 unit, 22 parser, 18 CLI, 3 exe, 4 hostile, 6 C API, differential (376 ran, 86 refused, 2
skipped, 0 timed out, 0 mismatched), crt, pp gate, fuzz - green; fmt and clippy clean.

### M8: `printf` - 2026-11-16 - the blocker that was not one, and the bug it had hidden

ADR-006 recorded two things Cranelift could not say, and refused every variadic call because of
them. One of the two was wrong.

**The signature per call site was already expressible.** `cranelift-module` refuses to declare one
name with two signatures - but a `call_indirect` needs no declaration at all: it takes a signature
and an address. So a variadic call is now an indirect call whose signature is built from *this call
site's* arguments (`signature_of`, the ABI rules included), to the address of the function, plus
the default argument promotions C99 6.5.2.2p6 asks for - `char`, `short` and `_Bool` to `int`,
`float` to `double`. The front end's half is one loop and one `Convert`.

**`AL` is genuinely out of reach, and now bounds instead of blocks.** What a callee does with `AL`
is decide whether to spill its vector registers. With no floating-point argument, either decision
is harmless: it spills registers nobody asked for, or spills nothing and reads nothing. So
integer, character, string and pointer arguments work - which is what `printf("%d %s %x")`,
`snprintf`, `fprintf` and `sscanf`'s input side do - and a floating-point argument is refused with
a diagnostic that says *cannot be passed*, not *not lowered yet*, because it is the ABI rather than
a missing step. `va_list`/`va_start`/`va_arg` stay refused: the callee's side of the same question.

**Measured, not assumed.** `tests/exe.rs` compiles a program that `snprintf`s, `printf`s and
`fprintf`s to stderr, links it against the C runtime, and compares its stdout *and* stderr byte for
byte with clang's build of the same source through the same linker: nine bytes, `answer=42`, and
the same line on both streams. That is M8's acceptance item, minus `%f`.

**Found: the JIT panicked on a symbol it could not resolve.** `--run` on a program calling `printf`
hit `can't resolve symbol printf` *inside cranelift-jit* - a panic in a dependency, in the host's
process, which is exactly what CONSTRAINTS 6 forbids. The JIT now checks every address a unit
mints against what it defines and what it was given, so the same program gets "`printf` is not
defined in this compilation and was not given as a host symbol" before anything is compiled. The
check has to know about *earlier* sources too - the C API's second source calls the first one's
functions - which the API test found immediately.

**Found: a scalar compound literal's value was its address.** With `printf` calls unblocked, corpus
programs that had been refused for printing began to compile, and one of them hit a `debug_assert`
in the parser: `(int){5} + 1` was `add` on a `pointer to int` typed as `int`, and `(char){65}`
returned from an `int` function was a pointer conversion. The literal filled a slot and handed back
the slot's address with the literal's type - right for an aggregate, where a value *is* its address
(ADR-004), and wrong for a scalar, where the value is the object. `read_place` knows the difference,
and is now what the compound-literal path returns. Four cases pinned by a JIT test (6, 65, 11, 12,
each matching clang), and `cc60_scalar_literal.c` compiles clean. The bug is older than this
commit: it was invisible because the programs that contain a compound literal also printed, so
their bodies were dropped before the verifier saw them.

161 unit, 22 parser, 18 CLI, 3 exe, 4 hostile, 6 C API, differential (376 ran, 86 refused, 0
mismatched), crt, pp gate, fuzz - green; fmt and clippy clean.

### M8 headers reviewed - 2026-11-16 - the LLP64 trap, in the one file that states the ABI's numbers

Review of `7ed1353`: the headers that ship with the compiler.

**Found: `limits.h` was wrong about `long`.** This ABI is LLP64 - `long` is 32 bits while a pointer
and `long long` are 64 - and I wrote `LONG_MAX` and `ULONG_MAX` as 64-bit values because that is
what a compiler on Linux would say. A program comparing them disagreed with clang: 27 against 57.
`stdint.h` and `stddef.h` were right, because they say `long long`; `limits.h` is the one header
whose whole job is to state the sizes, so a wrong number there is wrong in every program that reads
it. Fixed and re-measured: 57 against 57.

**Found: the same trap twice more, in declarations of real functions.** `stdio.h` declared
`fseek(FILE *, long long, int)` and `long long ftell(FILE *)`, and `time.h` declared
`clock_t` as `long long`. The C library's `fseek` takes a 32-bit offset, `ftell` returns one, and
`clock()` returns a 32-bit `clock_t`. Declaring a real function wider than it is compiles and then
truncates silently: the callee reads the low half of a register, and a large offset becomes a small
one. Both are `long` now, with a comment naming the trap, and the fix is what makes `fseek(in, 4,
SEEK_SET)` land on the byte it should.

**Found by fixing it: `strtol` and `strtoul` were missing.** A header that declares `strtoll` and
not `strtol` is a header that invites a program to use `long long` where the library means `long` -
the same mistake, made by the program instead of by me. Both are declared now, at their real
widths.

Two tests came out of it, because a measurement that is not a test is a measurement that expires: a
JIT test pins `LONG_MAX == 2147483647`, `ULONG_MAX == 4294967295`, `sizeof(long) == 4` and
`sizeof(size_t) == 8` - the ABI's shape, in the header's own words - and `tests/exe.rs` links a
program that `fopen`s, `fwrite`s, `fseek`s to byte four, `fgetc`s and `remove`s, built by xcc and
linked by the host's linker. That last one is M8's libc claim minus the variadic half, and it is
the first test in this repo where a program compiled here talks to the C runtime's own files.

Also added: `#include <stdarg.h>` now has a test that it refuses *with the ADR in the message* -
the one header whose value is that it says no.

Checked and sound: the host's files come before ours in every path (`Compile::with_includes` wraps
the host resolver, and a test shadows `stddef.h` from a host to prove it); `__builtin_offsetof`
walks a dotted designator and refuses an array one with a diagnostic; `again_if_hung` retries only
a timeout, so a real hang is still a mismatch.

160 unit, 21 parser, 18 CLI, 2 exe, 4 hostile, 6 C API, differential (376 ran, 86 refused, 0
mismatched), crt, pp gate, fuzz - green; fmt and clippy clean.

### M8: the headers ship with the compiler - 2026-11-16 - and `offsetof` neither C nor we could write

M8's first item: twelve clean-room C headers.

**They live inside the compiler, not on a filesystem.** `crates/xcc/include/*.h` are compiled in
with `include_str!` and answered by a `Headers` resolver that asks the host's files first and the
compiler's second. That order is one decision in one place, and it gives three things at once: no
`-I` for `#include <stddef.h>`, no filesystem for an embedder - `--run`, which has no host files at
all, compiles headers now - and `-I` still shadows any header we ship, which is what a program
carrying its own `stdint.h` expects. The "no filesystem access unless given a resolver" property is
intact, because these are not files: they are the compiler.

**`offsetof` needed a builtin.** C99 7.17p3 defines it as `(size_t)&(((type *)0)->member)`, which
this compiler cannot fold twice over: it refuses pointer-to-integer conversions, and a member of the
null pointer has no address to give. So `stddef.h` spells it `__builtin_offsetof(type, member)`, and
the parser answers from `member.offset` - the layout it already has - for a dotted designator. One
extension, in the one place that owns layout, for a macro whose standard definition is C the
compiler cannot compile.

**The headers are checked by two compilers.** The differential runner now gives clang `-isystem
crates/xcc/include` instead of skipping every `#include`, so a corpus program with a header is
compared against the same declarations - and clang has to accept our headers, which makes it a
clean-room audit on every run. The blanket "skip anything with #include" is gone: 40 skips became
2, and 376 programs now run and agree (was 375 with 40 skipped).

**Verified against clang, not against ourselves:** a program using `size_t`, `uint8_t`, `int16_t`,
`bool`, `CHAR_BIT`, `INT_MAX` and `offsetof` returns 21 under both compilers; a JIT test pins
`sizeof(struct S) == 16` and `offsetof(struct S, i) == 4`; and `malloc`/`free`/`qsort` compile with
`xcc -c`, link with clang against the SDK's libraries, and exit 11 - what clang's own build exits
with. `printf` still refuses, which is 50 of the 86 corpus refusals.

**Found while measuring: the differential runner is flaky under load.** One run reported 30
mismatches, every one of them "hung" on one side - the machine was running forty programs at once
and the ten-second limit caught ordinary programs. Two clean runs followed, so it was load, not a
bug; the runner now retries a side that timed out, because a real hang hangs twice. A ratchet test
that fails on a busy machine is a test people learn to ignore.

156 unit (159 with the header tests), 21 parser, 18 CLI, 4 hostile, 6 C API, differential, crt,
pp gate, fuzz, exe - green; fmt and clippy clean.

### `--explain` reviewed - 2026-11-16 - a mode that quietly ignored what it was given

Review of `9a3e4ff`: the explanation, and the README rewrite that came with it.

**Found: `--explain -o out.obj` wrote nothing and said nothing.** The mode conflict check covered
`-c`, `-E` and `--run`, and `-o` slipped past it: a host that asked for an explanation *and* an
object got an explanation, exit 0, and no file. Accepting a request and silently not doing it is the
one thing a seven-function command line should never do, and this file refuses the same shape three
lines above (`--run` with `-D`/`-U`/`-I`). It says so now: `--explain writes no object, so -o has
nothing to name`.

**Found: `run` returned a tuple whose halves each caller discarded.** `compile` wanted the artefact
and `explain` wanted the text, and both read `result.0`/`result.1`. A named `Outcome` says which
half is which at both call sites and costs one struct.

**Found: two tests were compiling C containing literal newlines.** Four `temp.write` calls had been
written by a patch script that turned `
` into a real line break inside the string. Rust accepts
it, so nothing failed - but a test whose C source is three lines of half-indented Rust string is a
test that will be misread, and one of them had already lost a `;` when I retyped it (the
determinism test's `struct Point` declaration), which the suite caught immediately. The escapes are
what they should be now.

**Checked and sound:** the `explained` vector is only built when asked (a normal compile formats no
Cranelift function); `Lowered::bytes` and the explanation come out of one lowering pass, so the two
views cannot disagree about what was compiled; a source with no functions explains to nothing, which
is what it should be, and a source that does not compile explains nothing and reports its
diagnostics *and* exits 1; the module's stated limit - a `Span` has no file, so an included file's
instructions are listed without a line - is a limit of `Span`, not of this code, and M8 owns it.

156 unit, 21 parser, 17 CLI, 4 hostile, 6 C API, differential, crt, pp gate, fuzz, exe - green;
fmt and clippy clean.

### M7: `--explain`, and a README that had stopped being true - 2026-11-16 - the front page said M4

M7's last item, and the documentation audit that came with it.

**`--explain`** prints, for each function, the C-level instructions in program order - each under
the source line it came from - and then the Cranelift function, which is where the virtual
registers appear. Two questions, two answers: *why is this instruction here* is the line above it
(a `sext char to int` is there because `char + char` is an `int` addition, which is the conversion
showing itself), and *which registers* is `v6 = iadd v4, v5`.

The implementation is one call into the existing pipeline rather than a second one:
`Compile::explain` and `Compile::compile` share `run`, and the only difference is a `bool` that
reaches the `Lowerer`, which keeps a `Vec<Explained>` instead of dropping each function. Formatting
the Cranelift function is most of the cost of asking, so a compile that did not ask pays nothing.
The two views are **not** paired instruction by instruction, and the module says so: one C
instruction becomes zero, one or several machine instructions, and a mapping the back end does not
keep would be a lie told by a debugger.

The one thing it cannot do yet: a `Span` is an offset without the file it is an offset into, so a
line number is shown for the unit's own file and an instruction from an included file is listed
without one. M8 gives the span its file, because headers make the question real.

**The README's front page had stopped being true.** It said "This is M4", that `switch` does not
lower and its test is `#[ignore]`d, that there is no `crt` and no `.exe` that prints, and that
`&&`, `||`, `++`/`--`, member access and calls are refused. Every one of those was done - the
`switch` test passes unignored, `crates/xcc-crt` exists, `tests/exe.rs` links and runs a program,
and a probe using all of them agrees with `clang -O0` (both 250). What actually does not work is
variadic calls, headers, unwind data, `argc`/`argv`, `static` linkage and a short list of refusals;
that is what it says now, with tests/corpus numbers attached.

A wrong front page is worse than a wrong corner: it is the one page a reader trusts before they
have any way to check it.

156 unit, 21 parser, 16 CLI, 4 hostile, 6 C API, differential, crt, pp gate, fuzz, exe - green;
fmt and clippy clean.

### M7 reviewed: one definition per name, and numbers from the right build - 2026-11-16 - legal C that reached Cranelift's words

Review of the three M7 commits (`e4c8ab7`, `5c18111`, `ef3beff`).

**Found: `int x; int x;` did not compile.** Two tentative definitions of one object are legal C99
and there is nothing to choose between them, but the parser pushed a `cir::Global` per declaration,
so the back end saw two symbols and answered `cranelift module: Duplicate definition of identifier:
x`. The legal pair `int x; int x = 1;` failed the same way, and so did `int f(void) { } int f(void)
{ }` - the one case where C does say no, but our message was Cranelift's rather than C's.

Two facts were missing, and they are different facts. *Where is this object's record*, so a later
declaration completes it instead of appending - that is `globals_by_name`, a map to the index in
`globals`. And *has this name a definition already*, which is the question C99 6.9p5 asks - that
belongs on the name, so it is `Entry.defined`, set by a file-scope initialiser and by a function
body, and asked by both. `define_global` now either completes the tentative record or refuses with
`redefinition of x`; the function path refuses a second body in the same words and skips it, which
also keeps the emitter from seeing two bodies.

Measured against clang, five cases: `int x = 1; int x = 2;` and two bodies are refusals, and
`int x; int x;`, `int x; int x = 1;`, `int x = 1; int x;` all compile to one object with the bytes
the real definition gave it (5 and 7 read back from a JIT test). That the corpus never caught it is
fair: 375 programs, and none of them declared a name twice.

**Found: the C API trusted the host's symbol count with an allocation.** `Vec::with_capacity(count)`
allocates before a single symbol is read, so a host that passed a nonsense count made the compiler
abort - in the one place a host's mistakes are supposed to come back as a message. `Vec::new()`, and
the list grows with the symbols that are actually checked.

**Found: my own numbers were from the wrong build.** The sandbox entry said a million tokens is
"about a minute" and 60 µs a token to lower. Every one of those measurements was `target/debug`. In
release the same inputs are ten to thirty times faster - a million tokens in 4.2 s, not 60 - so the
documented cost of the budget was wrong by an order of magnitude in the direction that makes the
compiler look worse than it is. CONSTRAINTS 5 carries release numbers now and says which build they
came from. The lesson is one I had written down and then ignored: measure the build the host ships.

Checked and sound: `within_nesting` gives its level back on every path (the numbers in the hostile
tests moved by nothing when the corpus ran); the prescan budget stops the fan-out bomb and the
`reported` guard stops the work rather than merely the output; the render window keeps the caret
under its character; the determinism test's two processes really do produce one byte stream.

153 unit, 21 parser, 14 CLI, 4 hostile, 6 C API, differential, crt, pp gate, fuzz, exe - green;
fmt and clippy clean.

### M7: the same source writes the same bytes - 2026-11-16 - the test CONSTRAINTS promised

CONSTRAINTS 4 has promised since M1 that "a test compiles the same source twice and compares byte
for byte, and it runs on every platform in CI". There was no such test; there is now, and it found
not a bug but the absence of evidence for a claim the repo had been making for five milestones.

Two *processes*, not two calls in one: Rust randomises a `HashMap`'s iteration order per process,
so a same-process test would pass while the object written by the next run differed. The program
compiled is deliberately broad - a struct, a string array, a tentative definition, a float, a
function taking and returning an aggregate - because each of those goes through a different table
in the object writer, and it is the tables a hash order would leak out of.

It passes, which is the useful part: symbol order, section order and relocation order were already
sorted, `Options::stamp` already defaults to the epoch so `__DATE__` cannot read a clock, and the
claim now has a check behind it rather than a sentence.

M7's acceptance is half done and says so: the Python host runs (`tests/host.py`), the hostile shapes
run (`tests/hostile.rs`), and the Lua half is not written, because there is no Lua on this machine
and a script nobody can run is worse evidence than none.

152 unit, 20 parser, 14 CLI, 4 hostile, 6 C API, differential, crt, pp gate, fuzz, exe - green;
fmt and clippy clean.

### M7: the sandbox, and three holes it named - 2026-11-16 - hostile input measured, not assumed

M7's sandbox item says a compile of hostile input "respects its budget and returns an error, not a
panic or a hang". The budgets existed (source bytes, tokens, macro depth, include depth, object
bytes, diagnostics). What did not exist was the evidence, so I wrote seven hostile sources and
compiled them.

**Found: `((((((...` was a stack overflow.** A hundred thousand parentheses killed the process
(exit 127, "has overflowed its stack"). `max_nesting` was counted in declarators, statements and
constant expressions - not in expressions, which is exactly where parentheses, unary operators and
casts recurse. `conditional_expression` and `unary_expression` now count too, through
`within_nesting`, which gives the level back whatever the rule answers: the alternative, an
`enter`/`leave` pair around rules with early returns, leaks a level per early return and silently
lowers the budget for the rest of the file - the comment on `enter_nesting` already warned about
that shape. Measured after: parentheses, `!!!!`, `(int)(int)`, `g(g(g(`, `if (x) if (x)`,
`{ { {` and `int ((((x))))` are all "nesting is deeper than 256" in under a second each.

**Found: a macro that fans out inside an argument ran forever.** `#define D0(x) x x x x x x x x x x`
and `#define Dn(x) D(n-1)(D(n-1)(x))` nested ten deep is ten-to-the-hundredth tokens of work. The
token budget was checked on the queue of tokens *waiting* to expand and on the output, but a
function-like macro prescans its argument first - and there the queue stays short while the work
explodes. Now a prescan spends the same budget, and once it is spent `expansion_of` stops
expanding: the one diagnostic stands and the remaining tokens pass through untouched. Measured:
120 s of nothing became a refusal in 18 s (the 18 s is a million tokens of prescan work at the
budget's own price, not a hang).

**Found: 39 MB of diagnostics from a 200 KB file.** One diagnostic per level was not the problem -
there were a hundred, the budget's worth. Each echoed the *whole* source line, and that line was
200 KB: 400 KB per diagnostic, twice (the line and its caret row). The renderer now shows a window
of at most 120 characters around the caret, with `…` where the line continues; the caret's column
is what a reader needs, and the line number is already in the header. Measured after: 25 KB, and a
test pins both the width and that the caret still sits under the character it is about.

**What the measurements say about a time budget.** Adding one would break CONSTRAINTS 4: the same
source refused on a slow machine and accepted on a fast one is not deterministic. So work is
bounded by tokens, and the price per token is measured rather than guessed - about 2.5 µs to
preprocess and 60 µs to lower, i.e. roughly a minute for the default million tokens, and a host
that cannot afford that lowers `max_tokens`. The 60 µs is mostly Cranelift building one enormous
block, which is M10's problem to earn back.

`crates/xcc-cli/tests/hostile.rs` runs each shape through the compiled binary as a *child
process* - a regression in the nesting budget is a stack overflow, and a test that ran the compiler
in-process would take the harness with it instead of failing with a name. It asserts the exit code,
the message naming the budget, and that the refusal is under a megabyte.

152 unit, 20 parser, 13 CLI, 4 hostile, 6 C API, differential, crt, pp gate, fuzz, exe - green;
fmt and clippy clean.

### M7: the C API - 2026-11-16 - five functions, and one place where `unsafe` is the job

M7's headline item: `crates/xcc-capi`, a `cdylib` + `staticlib` + `rlib` that exports
`xcc_new`, `xcc_add_source`, `xcc_function`, `xcc_last_error` and `xcc_free`, with `include/xcc.h`
for a C compiler to read. Nothing in it compiles anything: each function is one call into
`Jit` and one message kept on the handle.

**The interface is not `libtcc`'s, and the reason is a dependency.** tcc lets a host add symbols
at any time; Cranelift takes them at builder construction and nowhere else. Honouring that would
mean a "not built yet" handle state, a rule ("symbols precede the first compile") the caller has to
remember, and a failure mode for breaking it. So `xcc_new(symbols, count)` takes them up front: the
one ordering rule the dependency imposes is expressed by the signature instead of by a doc
sentence. A host that discovers a symbol later makes a new handle.

**Failures are messages, not codes to decode.** `xcc_new` never answers NULL - a symbol list it
cannot read, or an instruction set it cannot build, leaves a handle whose first `xcc_add_source`
refuses with the reason in `xcc_last_error`. One error path, so a host that checks the one call it
already checks checks everything; there is no second place to look. `xcc_last_error` is never NULL
and empty after a call that worked, so a host may print it unconditionally.

**A panic is a message too.** `xcc_add_source` wraps the compile in `catch_unwind`: a bug in this
compiler reaches the host as `internal error compiling: ...` instead of unwinding through a
language that cannot catch it. One `catch_unwind`, at the one entry point that does work.

**What the tests found.** Writing the Python host's second unit as `int twice(...) { return
add(...); }` failed with "`add` is not declared" - and that is C, not a bug: each `xcc_add_source`
is a translation unit of its own, so a second one declares what it uses from the first, exactly as
one `.c` file declares what another defines. The JIT is the linker, not a shared header. The first
draft of the header claimed the functions of every source could call each other, which was wrong in
exactly the way that matters to a host: the answer is in the docs now, and both hosts declare.

Verified: the six Rust-side tests (compile and call, a host symbol called from compiled C - the
host's own `triple(14)` is 42, a refused source that leaves the earlier unit working and clears on
the next good call, a missing name, and a NULL handle that is refused rather than chased); and
`tests/host.py`, which is a real Python program with ctypes: it compiles C at run time, calls it,
calls the C runtime's `abs` through the symbol table, adds a second unit, reads a diagnostic, and
frees. The crate-root docs still described M2 as the present; they describe M6 now.

150 unit, 20 parser, 13 CLI, 6 C API, differential, crt, pp gate, fuzz, exe - green; fmt and
clippy clean.

### bitfields stepped and initialised, reviewed - 2026-11-16 - a writer that answered a question nobody asked

One commit since the last review (`94862ce`): `++`/`--` on a bitfield, bitfield initialisers in a
frame and in a global's bytes, and the milestone paperwork.

**Found: the merge returned a value its caller had to throw away.** `write_bitfield_at` ended with
a read-back of the field, because an assignment's value *is* the field after the write. But an
initialiser stores for its own sake: every `struct S s = {1, 2, 3}` emitted three loads and six
shifts whose results nothing used. The fix is the one the split already implied: the writer writes
and answers nothing, and `write_bitfield` asks the reader when it needs a value. Two responsibilities
that had been sharing a return type - "merge these bits" and "what is this field worth" - are now
one function each, and the initialiser path pays for no load it does not use.

**Found: a comment that explained the wrong thing.** `self.bits = None` at the end of a member's
write was commented as bookkeeping for the union check that follows it. Its real job is that the
state means "the leaf just walked": stale, it would merge the *next* value - a plain member, or a
global declared after this one - into bits nobody asked about. The clear was in the right place for
the wrong stated reason, which is how it survives a future refactor.

Checked and sound, no change needed: `self.bits` restored by `initialize_local` and cleared by
every member's write (so a record followed by `int x = 200;` at file scope writes 200, not 200's
low bits, measured against clang); `++` in dead code (no place, no step, no diagnostic); the same
on a union member and a nested record; prefix versus postfix on a field that wraps (7 + 1 is 0 in
three bits) and on a signed one (-1-- leaves -2); `bytes::merge_bits` on two fields sharing a unit
in a global's bytes. Nine programs' worth of behaviour is pinned by the JIT test, whose expected
values came from `clang -O0` printing its own, not from reasoning about ours.

150 unit, 20 parser, 13 CLI, differential, crt, pp gate, fuzz, exe - green; fmt and clippy clean.

### M6 closed: bitfields stepped and initialised, and the rest of the paperwork - 2026-11-16 - two refusals that were only a missing call

The last two bitfield refusals, and the milestone paperwork that had drifted behind the code.

**`s.a++`** now steps in the field's own type and merges through the write path, so a value too
wide for the field is truncated by the write - which is exactly what `s.a = s.a + 1` does, and
what C stores. A prefix form yields the new value, a postfix form the old one.

**A bitfield in an initialiser** merges the same way, in both places an object can live: a frame
slot (code) and a global's bytes (no code). The code side was one call away - the mask arithmetic
moved out of `write_bitfield` into `write_bitfield_at`, which takes an *address* rather than a
`Value`, because that is what an initialiser has; `store_leaf` now decides between a whole store
and a merge. The byte side is `bytes::merge_bits`, the same arithmetic over the object's bytes,
taking its mask from the same `field_mask`. Two domains, one rule.

**The paperwork.** `PLAN.md` still had floats, aggregates, bitfields and `long long` unticked, and
the acceptance count at 359. The `long long` item is now settled by measurement, not by decision:
against `clang -O0` on this machine a shift count at or beyond the width masks to the hardware's
six bits in both (`(1LL << 70)` is 64 in each), and `LLONG_MIN / -1` raises the divide error in
both. C leaves that undefined, so "defined behaviour, no trap" was never reachable - the two agree
by doing what the machine does. `docs/C99-SUPPORT.md` had lost a row: it still said "a bitfield read
in code ... refused" three lines above the row saying it was `sound`, which is worse than saying
nothing, because a stale row is a confident wrong answer to "can this compiler do X".

What is left of M6 is varargs ([ADR-006](ADR-006-varargs.md), blocked on the back end), and the
rest is M7's C API and M8's headers.

150 unit, 20 parser, 13 CLI, differential, crt, pp gate, fuzz, exe - green; fmt and clippy clean.

### function pointers, `goto` and bitfields, reviewed - 2026-11-16 - the edges the three opened

The review of the three commits that cleared the differential runner's work list. The three
mechanisms held; what the review found were the edges each one opened.

1. **`&s.a` on a bitfield was accepted, and meant the whole storage unit.** Reading and writing a
   bitfield went through `place`, which is the *unit's* address - so `&` saw a place and handed back
   a pointer to the unit, and `*(int *)&s.a = 5` would have written a neighbour's bits. C99
   6.5.3.2p1 says a bitfield has no address; it is refused now, which is what clang says too.
2. **The value of a signed bitfield assignment was the masked bits, not the field.** `(s.c = -3)`
   was 13 (`-3 & 0xF`) where C says the assignment's value is the field after the write, which is
   -3 sign-extended from four bits. The writer reads the field back after storing - one load, which
   M10 can fold - and clears its place and bits, because an assignment is not an lvalue.

Checked and sound, no change needed: a variadic call through a pointer (the refusal sees the
pointee's flags), an aggregate result through a pointer (the same `signature_of`, hidden buffer
included), `(*f)()` against `f()`, a `goto` into a loop or an `if` body, a `goto` to a label at the
end of a function, dead code after a `goto` in a declaration with an initialiser, and a bitfield in
a union.

150 unit, 20 parser, 13 CLI, differential, crt, pp gate, fuzz, exe - green; fmt and clippy clean.

### M6: bitfields read and written in code - 2026-11-16 - the unit, and the bits in it

The last M6 item the differential runner could reach. The layout was already right (measured
against clang two commits ago); what was missing was the code: a bitfield has no address of its
own, only a bit range inside a storage unit.

**Reading** is the unit loaded as its own type, converted to a 64-bit working type - signed or
unsigned by the field, which is what decides whether the back end's `Shr` is arithmetic or logical
- and then two shifts: `shl(64 - start - width)` then `shr(64 - width)` for a signed field (which
sign-extends from the field's own width), `shr(start)` then `and(mask)` for an unsigned one. The
result converts back to the field's declared type, because that is the expression's type.

**Writing** is read-modify-write on the unit: `(unit & ~(mask << start)) | ((value & mask) <<
start)`. The value of the assignment is the field *after* the write, as C has it.

The one design cost is in `Value`: a bitfield value carries `bits: Option<(u8, u8)>`, because
`place` alone would say "the unit's address" and the writer would then put the field's value over
its neighbours. Ten struct literals gained the field; the constructors answer `None`.

Two things stay refused, each with a diagnostic: **`++`/`--` on a bitfield** (`step` reads a place,
steps it and stores - the store would write the whole unit) and **a bitfield in an initialiser
list** (the walker writes the unit; masking and shifting belong where the value is written). The
corpus's differential count did not move - 375 still - because the bitfield programs it holds are
either invalid C99 or refused for one of those two reasons.

150 unit, 20 parser, 13 CLI, differential, crt, pp gate, fuzz, exe - green; fmt and clippy clean.

### M6: `goto` and labels - 2026-11-16 - a label needs a block a `goto` may target

`goto` had been parsed since M3.4 and refused by the emitter since M4. Three corpus programs, and it
turned out to need one idea rather than machinery: **which block a label's code starts in**, and who
creates it.

The label and the `goto` to it can come in either order, so whichever comes first creates the block
and the other switches to it. A label the code above falls into is the block the code is already
in; a label a `goto` reached first was already given a block by that `goto`.

Two cases forced a third rule, **a label cannot *be* a block that a `goto` may not target** - found
by the verifier, one after the other:

1. **The entry block.** `int f(int n) { top: ... goto top; }` jumped to the entry block, and
   Cranelift says `invalid reference to entry block block0`. The label gets a block of its own and
   the entry jumps there.
2. **A block with parameters.** `int f(int n) { top: ... goto top; }` with the label on the entry is
   case one, but a label on a *join* had the same shape: the join's parameters are the values its
   construct passes it, and a `goto` has none - `block 0 is given 0 values and takes 1`.

A third thing fell out of writing the test: **dead code after a `goto` was diagnosed**.
`goto skip; x = 100;` reported "this kind of assignment is not lowered yet", because the load of
`x` emitted nothing (no block to emit into) and `read_place` dropped the *place* with it. A place is
a place even when no block is there to load from; the value is missing and the store is skipped,
which is what dead code should do.

15 corpus programs were already at the finish line for a different reason and this moved the count
to 375 running and agreeing (was 372), 49 refused (was 52).

149 unit, 20 parser, 13 CLI, differential, crt, pp gate, fuzz, exe - green; fmt and clippy clean.

### M6: calls through a function pointer - 2026-11-16 - the last refusal in the call path

The largest single gap the differential runner measured: thirteen corpus programs refused at "a call
through a function pointer is not lowered yet". The front end had been ready for it for milestones -
`call` accepts a function type or a pointer to one, converts the arguments to the declared
parameters, and hands the emitter a callee *value* - so the gap was one branch in the back end.

`Inst::Call` now asks whether the callee's address came from a `GlobalAddress` in this body. When it
did, that is the function and the call is direct. When it did not, the callee is a pointer, and the
call is `call_indirect` against the signature of **the function type the pointer points at** -
`signature_of_pointer`, which is `signature_of` one dereference further along, so aggregates cross
the ABI's way and `(*f)()` and `f()` lower to the same thing.

The measurement moved with it: 372 corpus programs now run and agree with `clang -O0` (was 359), 52
refused (was 65), and the ratchet is at 372.

148 unit, 20 parser, 13 CLI, differential, crt, pp gate, fuzz, exe - green; fmt and clippy clean.

### the differential runner, reviewed - 2026-11-16 - what the harness could not see, and two of its own

The review of the runner and of the four bugs it found. The four fixes held up (each is now a
corpus program that runs), and the review's findings were in the harness and at its edges.

1. **`--run` accepted `-D`/`-U`/`-I` and ignored them.** `Jit::add_source` compiles with
   `Options::default()` - the embedding API has no way to pass options yet (M7) - so the flag was
   built and then discarded with a `let _ = options;`. Two lies in three lines: a flag that does
   nothing, and a variable saying so quietly. The flag is refused with a message naming the
   reason, which is the honest state until the JIT takes options.
2. **The harness could not see a program that writes more than a pipe buffer.** The output was
   read *after* the process exited, so a program printing 64 KiB would block on its own output,
   miss the time limit, and be reported as a hang. The pipes are drained by threads while the
   program runs. Nothing in the current subset prints anything (no `printf`), which is exactly why
   this had to be fixed before the subset grows.
3. **The counts were a `(usize, usize, usize)`** with `.0`, `.1`, `.2` meaning ran, refused,
   skipped - the one thing in the test that a reader had to look up. It is a `Counts` struct with
   three named fields.
4. **`ran > 300` was a floor with no meaning.** It would have passed while fifty programs silently
   stopped running. It is a ratchet at the measured number (`PROGRAMS_THAT_RAN = 359`): raising it
   is what closing a gap looks like, and lowering it is the regression the test exists to catch.

Two more found by probing the edges the runner had opened:

5. **The unsigned-and-wider integer types in the new literal table were wrong.** `u` without
   `ll` ended in a *signed* 64-bit type, so `unsigned long long a = 4294967296u;` was a signed
   comparison and `18446744073709551615u` was refused as too large. Both lists end unsigned now.
6. **Unary `+` accepted a pointer** (`+p` is not C, C99 6.5.3.3p1) where `-` already refused one,
   and an incomplete record's initialiser said "too many initialisers" *after* the declaration had
   said it had no size, then left the cursor inside the braces and cascaded into four diagnostics.

148 unit, 20 parser, 13 CLI, differential, crt, pp gate, fuzz, exe - green; fmt and clippy clean.

### M6 acceptance: the differential runner - 2026-11-16 - and the four bugs it found first

M6's last item asks that the corpus be *run*, not just compiled, against a reference. This is that
runner, and it earned its keep on the first run: 28 of 332 programs disagreed with `clang -O0`,
every one of them a silent wrong answer rather than a refusal.

**The shape.** `xcc --run file.c` compiles the source in process, calls `main`, and exits with what
it returns - the embedding API used the way a host would, and one new command-line flag. The test
then runs each corpus program twice, as a *process*: through `xcc --run`, and through
`clang -O0 -nostdinc` and its executable. Exit status and stdout are compared; a time limit turns a
hang into a result; a condition code above `0xC0000000` is a crash, which is also a result. A
program xcc refuses is counted, not failed - a refusal is a recorded gap - and so is one `clang`
itself will not build.

**The numbers now**: of 464 corpus programs, **359 run and agree**, 65 are refused (variadic
`printf`, undeclared library calls, deliberately invalid programs), 40 are skipped (`#include`, no
headers until M8, plus the few the reference rejects), 0 disagree. It costs about two minutes,
parallelised across the machine's workers; `clang` is 0.66 s per file and our own `--run` 0.12 s.
That is the price of isolation: a crash in one program is a result, not the end of the run.

**Four bugs, all of them the same family - something emitted nothing:**

1. **Reading a file-scope object produced a function with no return.** `return g;` emitted nothing,
   because a global had no address to load from, so the function fell into `unreachable` and the
   program died with `ud2`. Twenty-eight programs, all of the `cc1x_*init` family. Fixed by
   extending ADR-004's rule to file scope: an object's value is the symbol's address, so a global
   is a place like a local - loaded, stored, subscripted, a member's base.
2. **Unary `+` dropped its operand.** `return +42;` was a trap. It is
   `convert(value, promote(ty))` now, because promotion can be a conversion (`+c` for a `char`).
3. **A decimal constant too big for `int` was typed `unsigned int`.** `unsigned long long a =
   4294967296;` stored a 32-bit zero, so `a > b` was false where C says true. C99 6.4.4.1p5's table
   is implemented: a *decimal* constant takes `int`, then `long long` - never an unsigned type
   first - while octal and hexadecimal take the unsigned-and-wider order.
4. **A compound literal had no storage.** `(struct P){3, 4}` was a value with no place, so `.x`
   computed nothing and the program trapped. It gets a slot of its own now, filled by the same
   walker `initialize_local` already used.

The runner is `#[test]` and lives with the command line's tests, because it drives the command
line. What it cannot cover yet is recorded above it: `printf` (ADR-006), headers (M8), and the
bitfield cases M6 asks for, which the compiler refuses rather than runs.

148 unit, 20 parser, 13 CLI, differential, crt, pp gate, fuzz, exe - green; fmt and clippy clean.

### varargs, reviewed - 2026-11-16 - a reason is a note, not a longer sentence

The review of the varargs commit. Its one code change was a diagnostic, and the diagnostic was
written in the wrong field.

**The refusal's reason went into `message`.** `Diagnostic::message` is documented as "one sentence,
without a full stop", and the varargs message became one sentence with a colon, two clauses and a
parenthetical. The struct has `notes: Vec<String>` for exactly this - the renderer prints them,
`render_all` prints them, and a host can show them apart - and the parser had never used one. So
`not_lowered` is now the no-note case of `not_lowered_with(span, what, Option<&str>)`, and the
variadic refusal passes its reason as a note. The message is back to `a call to a variadic function
is not lowered yet (M4.2b)`, with the milestone suffix that every other refusal carries and this one
had dropped by hand-writing its own text.

The test followed the reason to its new home: `tests/parse.rs` gained a `notes` helper, because
`check` has always collected only messages and a note is part of what a refusal says.

148 unit, 20 parser, 11 CLI, crt, pp gate, fuzz, exe - green; fmt and clippy clean.

### M6: varargs - 2026-11-16 - refused with the reason, and an ADR instead of an afternoon

Varargs turned out to be a back-end question, not a front-end one, and the front end's half is
cheap: a variadic `float` is promoted to `double` (already done by the parser) and then passed
*twice*, in the next XMM register and in the next general-purpose register, so a callee can
`va_arg` it out of whichever register file it saved. Two `cir` values - the `double` and its
bitcast - express that.

The two rules that cannot be expressed are both the caller's, and both were measured in the
dependency's source rather than assumed:

1. **`AL`.** Win64 sets `AL` to the number of vector registers the call used; the callee's
   prologue spills `xmm0`-`xmm3` on the strength of it. `AL` is a physical register and `cir` has
   none: Cranelift is a register allocator, and `cranelift-codegen` 0.117 has no mention of
   `variadic`, `varargs` or a call-site register anywhere in it.
2. **A signature per call site.** `cranelift-module`'s `FunctionDeclaration::merge` returns
   `IncompatibleSignature` when the same name is declared with a second signature, so
   `printf("%d", 1)` and `printf("%s %f", s, d)` cannot both be declared against the symbol.

Neither is a missing line. So [ADR-006](ADR-006-varargs.md) records the decision - refuse, and
make the refusal say why - with the two constraints in the diagnostic's note:

```
t.c:1:58: error: a call to a variadic function is not lowered yet (M4.2b)
  = note: Win64 sets `AL` to the vector-register count and gives every call site its own
          signature; this back end can express neither (docs/ADR-006-varargs.md)
```

The front-end half is deliberately *not* written: the duplicate and the promotion belong in
`emit_call` and would be code no test can run, and the shape of the call changes the moment `AL`
becomes expressible. What is lost by waiting is nothing; what is gained is that the next reader
does not spend the afternoon this note did.

Consequence for M6's acceptance item: the differential runner (compile *and run* corpus files with
xcc and with clang) has to pick programs that print nothing until this is lifted - which is why it
is the next thing worth doing, on a corpus subset rather than the whole thing.

148 unit, 20 parser, 11 CLI, crt, pp gate, fuzz, exe - green; fmt and clippy clean.

### the aggregate ABI, reviewed - 2026-11-16 - what the probes got wrong, and one check that was a guess

The review of the ABI commit. The rule itself held up: the probes that mattered - a by-reference
callee modifying "its" parameter (the caller's object must not change), three by-reference records
and an integer filling the four registers exactly, a hidden result pointer shifting all of them, a
union by value, a sixteen-byte struct in and out - all pass. Three of those are now in the test
rather than in a scratch file, because they are the cases implementations get wrong.

Two findings:

1. **A *definition* with an incomplete record parameter or result was accepted.** `struct S; int
   f(struct S s) { ... }` compiled: the ABI turns an incomplete record into a by-reference
   parameter, so nothing ever asked for its size, and `sizeof(s)` inside would have been the only
   complaint. C99 6.7.5.3p4 and 6.9.1p3 require a complete type in a *definition* (a declaration
   may name an incomplete one), so the check is asked where the definition is, and each is one
   diagnostic with the type's name in it.
2. **Nothing was wrong with the stack slot's alignment**, and that was worth checking rather than
   assuming: a by-reference aggregate's copy is read by the callee, and the Win64 rule wants it
   8-byte aligned. Cranelift aligns every stack slot to at least a word (`machinst/abi.rs:
   max(word_bytes, 1 << align_shift)`), so the copies are 8-aligned and no change was needed.

One test bug found by the new cases, and it is a real C fact rather than a compiler fault: reading
a union returned by value *as an integer* reads the bytes of the members never written. The test
passes and reads a `#[repr(C)] union`'s `i` instead; a union's value is the member that was
stored.

148 unit, 20 parser, 11 CLI, crt, pp gate, fuzz, exe - green; fmt and clippy clean.

### M6: the Win64 aggregate ABI - 2026-11-16 - where an aggregate crosses a call

Structs and unions pass and return by value now, and the rules are MSVC's:

- **1, 2, 4 or 8 bytes** travel in an integer register - the *bits*, so a struct holding a `double`
  goes in `rcx` rather than an XMM register.
- **Any other size** travels by reference, to a copy the **caller** owns; the callee may keep it,
  which is exactly C's call-by-value.
- **A result** of 1, 2, 4 or 8 bytes comes back in `rax`; any other comes back through a buffer the
  caller passes as the leading argument and the callee returns again.

The decision that made this tractable: **the ABI is lowered in the front end, and `cir` carries
machine-shaped values.** A callee's `cir` parameters are integers and pointers, never records, so
`signature(func)` is still just `machine_type` over what the body sees. The one place the *C* type
is read instead is `signature_of`, which declares a function from its type - a call into the host,
say - and so applies the same rules itself. Two implementations of one rule is a smell, and the
test that matters is that they agree: `call_host` in the ABI test calls a Rust function declared
from its type and gets the same answer a Rust caller gets from the xcc-compiled `take_wide`.

Where the values live is unchanged from the aggregates commit: an aggregate's value is the address
of its object. So a by-reference parameter's storage *is* the incoming pointer, a by-reference
argument is copied into a slot of the caller's frame first, and a call's result is the slot the
caller made for it - which is why `struct Wide w = host_make_wide(7);` needs no special case at the
assignment.

Both refusals this replaces are gone from `C99-SUPPORT`, and the corpus's `cc14_bv*` and
`cc16_sretfptr*` files compile instead of refusing. Rust's `#[repr(C)]` plus `extern "C"` is the
oracle: it implements the same ABI, so a wrong rule shows up as a wrong number rather than as a
crash. That test covers every size class, arguments after an aggregate, two aggregates, a host
function returning one, and a round trip through both directions.

148 unit, 20 parser, 11 CLI, crt, pp gate, fuzz, exe - green; fmt and clippy clean.

### aggregates in the frame, reviewed - 2026-11-16 - the initialiser walker, and a layout measured against clang

The review of the aggregates commit. Three silent wrongs in the initialiser, and the first
differential measurement the plan asks for.

1. **An unnamed bitfield took a value.** `struct S { int a; int :3; int b; } s = {1, 2};` wrote `2`
   into the padding's storage instead of `b`'s: C99 6.7.8p9 says an unnamed member is not
   initialised. The walker steps over padding now.
2. **A named bitfield was written over its whole unit.** `struct S { unsigned a :3; } s = {9};`
   stored 9 in the unit where C's value is `9 & 7` = 1, and nothing would have masked it on the way
   out either. Refused with a diagnostic, since a bitfield cannot even be read in code yet.
3. **A union took more than one initialiser.** `union U u = {1, 2};` wrote both into the same
   bytes; C allows one. It is one error now.

Then the layout, which had never been measured: the initialiser fixes made bitfield structs
reachable, so their *sizes* started to matter, and they disagreed with the ABI.
`struct { int a; int :3; int b; }` was 8 bytes and put `b` inside the bitfield's unit; a struct of
bitfields and a `char` was 4 where the ABI says 8; `int :0` raised the alignment of the whole
struct, which the ABI does not do. The rule is one sentence - **the unit a bitfield is packed into
is spoken for, so whatever comes next starts after it** - and `int :0` closes a unit and is
otherwise nothing.

`a_record_lays_out_like_the_abi` is the measurement, with the oracle named: ten structs whose
sizes and offsets are `clang`'s on win64, compared field by field. It failed on four of them before
the fix and passes on all ten after. Layout is the first part of the front end with numbers from
another compiler pinned in a test rather than in prose.

147 unit, 20 parser, 11 CLI, crt, pp gate, fuzz, exe - green; fmt and clippy clean.

### M6: aggregates in the frame - 2026-11-16 - a struct's value is the address of its bytes

Struct and union locals, member access, struct assignment and brace initialisation are lowered. The
decision that made it small is ADR-004's rule extended: `cir` has no register for an aggregate, so
**an aggregate expression's value is the address of the object holding it** - the same shape an
array already had. `value_is_address` says so, `read_place` therefore never loads one, and the
kernel of the change is one new sentence in `assign`: a record is put into an object by a *copy*,
not a store.

1. **One place a value lands in an object.** `store_at` decides between a store (a machine value)
   and a byte copy (`copy_object`), and both the assignment and the initialiser's leaf writer call
   it. Without it, `struct P q = p;` stored the *address* of `p` into `q` - eight bytes into a
   struct that may be smaller - and the compiler would have been quietly wrong.
2. **The copy is byte-wise.** A union, a bitfield's storage unit and the padding between members
   come along without the copier knowing the layout, which is `Types`' knowledge (ADR-004). M10's
   optimiser is where a wider copy belongs.
3. **`address_within` is the one byte-arithmetic helper.** It was `leaf_address` in the initialiser
   code; member access and the aggregate copy need the same walk, so it moved next to
   `pointer_walk` and all three ask it.
4. **The initialiser's top-level elision branch was wrong, and this found it.** `struct P q = p;`
   read as "a brace-elided list for a record" and descended into `p`'s first member, so a struct
   *copy* was refused as "cannot initialise int with struct P". C99 6.7.8p1 says an initialiser is
   a brace list or an assignment-expression: at the top level there is no elision, and the branch
   is gone. Elision stays inside a list, where it belongs.

Refused with a diagnostic rather than half-lowered, each found by probing: a **bitfield read in
code** (it has no address of its own), a **conditional expression whose arms are aggregates** (the
join would need a register), an **aggregate argument** and an **aggregate returned by value** (the
Win64 aggregate ABI - the next slice), and a **struct value inside a brace list** (`{v}`), where
elision would have to know the expression's type before reading it. A `void` object, a record with
no members and an object past the budget were already refused.

Two cascades the corpus gate caught and this closed: a call returning a struct poisoned the value
so the assignment does not complain a second time, and a member of an object with no address (a
global) is a place-less lvalue - the consumer refuses in its own words, as `named_value` already
did for the global itself.

146 unit, 20 parser, 11 CLI, crt, pp gate, fuzz, exe - green; fmt and clippy clean.

### the budget and `_Bool`, reviewed - 2026-11-16 - `void` was a slot the back end could not make

The review of the object-budget and `_Bool` commits. Two real findings, one of them older.

1. **A `_Bool` value was assignable to a pointer.** The `_Bool` commit's `is_assignable` arm said
   "any scalar comes out of a `_Bool`", so `p = b` passed the type checker and the emitter then
   reported `this compiler cannot convert _Bool to pointer yet` - a compiler bug blamed for a
   program that is simply not C. The arm is gone: a `_Bool` is an integer, the arithmetic arms
   already take it, and a `_Bool` *value* is not a null pointer constant.
2. **`void` objects were internal errors.** `void v;` in a function said `internal: a slot for an
   incomplete type`, at file scope `internal: the object `v` has no size`, and `void v` as a
   parameter `internal: a value of type void has no machine type yet`. `void` is not an object
   type (C99 6.7.1p2) and each of the three is one diagnostic now. The declarator asks its
   questions once - is this an object at all, is it `void`, does it fit the budget - instead of the
   three places that each spelled out `!typedef && !function`.

Also, both small enough to be part of the same pass: the budget check moved out of `emit.rs` (it
is a fact about a declaration, not about `cir`) into the declaration code that owns it, and
`convert`'s `_Bool` guard asks `is_scalar` rather than respelling "arithmetic or pointer".

145 unit, 18 parser, 11 CLI, crt, pp gate, fuzz, exe - green; fmt and clippy clean.

### `_Bool` - 2026-11-16 - 0 or 1, by comparison rather than by truncation

`_Bool` was converted to by *narrowing*: `_Bool b = 2;` stored 2, `_Bool b = 0.5;` was refused (no
instruction conversion of a float to a one-byte integer gives 1), and a pointer was not assignable
to one at all - three faces of the same wrong rule. C99 6.3.1.2 is a comparison: any scalar
converted to `_Bool` is 0 if it equals zero and 1 otherwise.

There is one `truth_value` now, a compare against zero with a `_Bool` result, and the back end's
`widen_flag` takes the width of the *result's* type instead of assuming `int` - which is what makes
a comparison produce a `_Bool` rather than an `int` the parser then lies about. The same
comparison answers a float source (`(_Bool)0.5` is 1) and a NaN one (also 1, which is why a
conversion was never the right shape: `fcvt_to_sint` of a NaN is undefined).

Three smaller things fell out of it:

- **`is_assignable` refused a pointer for `_Bool`**, because its `Bool` arm asked for *arithmetic*
  on both sides. C99 6.3.1.2 says any *scalar*, so a pointer converts; `Types::is_scalar` is now
  the one predicate for that, and `require_scalar` asks it too.
- **The file-scope byte writer ignored the conversion.** `_Bool yes = 2;` was written as `02`, and
  `int x = 1.5;` as the low four bytes of a double - the bytes path wrote a constant the store path
  would have converted first. A `_Bool` leaf is 0 or 1 in `constant_bytes` now; the general float
  case is refused, which it already was by the "cannot evaluate" rule.
- **`constant_is_zero` moved to `Types`**, because "is this constant C's zero" is a fact about a
  type and some bits, and the byte writer needs it as much as the parser does.

145 unit, 18 parser, 11 CLI, crt, pp gate, fuzz, exe - green; fmt and clippy clean.

### the object budget - 2026-11-16 - a truncated frame is not a refusal

The review of the floats commit left one finding unfixed: `int a[100000000000];` is a legal
constant expression and a 400 GB object, and the back end builds the frame slot with
`size as u32` - about 542 MB of frame, silently, rather than refusing.

The limit is a **budget**, not a constant, because it is the host's decision: xcc is embedded in
programs that compile code they did not write, and `Budgets` is where this project already puts
"what one compilation may spend". `max_object_bytes` defaults to 64 MiB - past any frame Windows
will give a thread, and past the globals a JIT'ed module plausibly holds - and a host that knows
its own limits raises it.

One check, at the one place an object is declared (`object_fits`), covers a local's slot and a
global's data object: both are made from the same type's size, and neither is more trusted. An
oversized object is not silently dropped either - its initialiser is still read, so its own
mistakes are reported, and nothing is written.

144 unit, 18 parser, 11 CLI, crt, pp gate, fuzz, exe - green; fmt and clippy clean.

### M6 floats, reviewed - 2026-11-16 - the bits were read as the number, twice

The review of the floats commit. Two of its bugs were the same mistake in different clothes: a
constant's *bits* used where its *value* was meant.

1. **A float was accepted as an integer constant expression.** `Value.constant` now holds a float's
   bit pattern, and `constant_expression` only asked whether a constant existed - so
   `int a[1.5];` took 0x3FF8000000000000 as the array's size (and compiled), `case 1.5:` became a
   case label, and `enum { A = 1.5 }` an enumerator. C99 6.6 wants an *integer* constant
   expression, and the bits of a float are not the float. One type check refuses all three:
   `expected an integer constant expression, not double`.
2. **Unary minus read an `int` constant as an `f32`.** The fold asked `size_of(ty) == 4` before it
   asked whether the type was a float at all, so every four-byte integer took the `f32` branch:
   `int x = -3;` at file scope was written as -2147483645, and `int a[3] = {-1, 2, 3};` had the same
   fault. The type is asked first now, and the byte test pins `int x = -3; long y = -4;
   double z = -1.5;`.

Found while probing the same corner, and fixed with it: **an array size of zero or less was
accepted** and clamped to zero (`size.max(0)`), which C99 6.7.5.2p1 forbids - `int a[-1]` became a
zero-element array, silently. It is one diagnostic now.

Recorded, not fixed - it needs a decision, not a patch: **an absurdly large array is accepted, and
the back end truncates its size**. `int a[100000000000];` is a legal constant expression and a
400 GB object; `StackSlotData` takes a `u32`, so the slot is built with `size as u32` - about
542 MB of frame, not 400 GB. A limit belongs in `Budgets` (where the other implementation limits
live) with its reason written down; picking the number is the decision.

144 unit, 18 parser, 11 CLI, crt, pp gate, fuzz, exe - green; fmt and clippy clean.

### M6: floats and doubles - 2026-11-16 - the front end was the missing half

The back end already had the instructions: `fadd`/`fsub`/`fmul`/`fdiv`, `fneg`, `icmp`/`fcmp`,
`fcvt_*`, `fpromote`/`fdemote`, and `Const` already wrote float bits. What was missing was the
front end, and it was missing in a way that made the back end's work unreachable: a floating
constant was *typed* and given no value (`Value::temporary`), so no float expression could emit.

1. **A float constant is now its bits.** `literal::float_value` reads the text once - `1.5`, `1.5f`
   rounded to `float`, `1.5L` (64-bit, as documented), `1.`, `.5`, exponents - and hands back the
   bit pattern; `Value::constant` holds it, exactly as `cir::Const` does. That makes the *two
   questions* asked of a constant different, so they are now asked in one place each:
   `constant_is_zero` is C's zero and includes `-0.0`; `null_pointer_constant` is an *integer*
   constant 0 (6.3.2.3p3) and deliberately does not include `0.0`. Eight ad-hoc
   `constant == Some(0)` tests became those two predicates.
2. **Conditions are comparisons.** `if (d)`, `while (d)`, `d ? a : b`, `d && e`, `!d` were all
   "branch on the value", which the verifier rightly refused for an `f64` - and `p && q` did not
   work at all, because `logical` converted its operands to `int` and a pointer has no such
   conversion. One helper, `compare_to_zero(value, Ne | Eq)`, is now the answer for every
   condition: it compares in the value's own type, so a pointer compares with a null pointer and
   a float with 0.0 - which is also the only way NaN is handled correctly (`fcvt_to_sint` of a NaN
   is undefined, and NaN is a true condition).
3. **Folding for file scope.** A file-scope initialiser is bytes (ADR-004), so `double d = 1.5 * 2;`
   needs the number, not the instruction. `fold_float` folds `+ - * /` in the result's width, with
   an integer side converted first. When it cannot fold, the object is **refused**:
   `a file-scope initialiser this compiler cannot evaluate is not lowered yet`. Before, it was
   *silently dropped* - the global simply did not exist - which is a wrong program, not a smaller
   one, and it was reachable for every non-constant file-scope initialiser.

Two back-end faults the tests found, both an assertion or a verifier error rather than a wrong
answer:

- **`icmp.f64`.** The compare lowering matched `Eq`/`Ne` before it looked at the type, so float
  equality asked Cranelift for an integer comparison of an `f64`. Every float comparison goes
  through `fcmp` now; `Ne` is the unordered one, which is what C's `!=` means for a NaN.
- **A conversion to or from a narrow integer.** x64's `fcvt_*` takes a 32- or 64-bit integer, so
  `(short)3.9` and `char c; (double)c` panicked the instruction selector. The conversion widens on
  the way in and narrows on the way out; and because the signedness is a property of the *C* type,
  `unsigned` now converts with `fcvt_*_uint` instead of reinterpreting 4e9 as negative.

Still refused, with a diagnostic: hexadecimal floating constants, and a float converted to
`_Bool`. Still not lowered and now recorded: `_Bool` holds whatever a narrowing conversion leaves
(`_Bool b = 2;` stores 2, and truthiness saves it) - C says 0 or 1, and nothing implements that
yet.

144 unit, 18 parser, 11 CLI, crt, pp gate, fuzz, exe - green; fmt and clippy clean.

### `?:`, reviewed - 2026-11-16 - the folded value was dropped exactly where there was code

The review of the `?:` commit. Two findings, one of them a real hole in the new lowering.

1. **A constant `?:` stopped being a constant in a body.** `int *p = 1 ? 0 : 0;` was refused with
   "cannot assign int to pointer", while the same spelling at file scope became the null pointer:
   the lowering path returned `constant: None`, so the null pointer rule had nothing to see. C99
   6.3.2.3p3's null pointer constant is *any* integer constant expression with the value zero, and
   `1 ? 0 : 0` is one. The arms were read twice - once per path - and the fold had been written in
   only one of them, which is how the two copies diverged. There is one flow now: the arms are read
   in one place, the folded value is computed once, and only the lowering is gated on whether the
   condition has code.
2. **The test called `void` functions through `fn(i32)`.** `sized`, `unevaluated` and `merged` take
   no parameters, and the test passed one anyway - harmless on Win64, and exactly the kind of
   signature mistake a test exists to catch. Two closures now, one per signature.

Checked and sound, no change needed: a condition of every width (`char`, `short`, `_Bool`,
`unsigned`, `long long`), pointer arms (`&a`, `&b`, a literal's address), calls in both arms, a
`void` arm, a `long` merge, `?:` in a declarator's size, a `?:` inside `sizeof`'s unevaluated
operand, code after `return`, and a nested `?:` in the middle arm - the cursor rule applied to the
arm the first test did not cover.

142 unit, 18 parser, 11 CLI, crt, pp gate, fuzz, exe - green; fmt and clippy clean.

### `?:` - 2026-11-16 - two blocks and a join, and an operand that was being evaluated

`?:` was read, typed and folded, and emitted nothing: `return a ? 1 : 2;` compiled to a trap. It is
lowered now as two blocks and a join whose parameter is the expression's value - not one `Select`,
because the untaken arm must not be evaluated at all: `n ? 10 / n : -1` with `n` zero must not
divide. An arm that is itself `&&` or another `?:` leaves the cursor in its own join, so each
arm's jump leaves from *where the cursor is*, not from the block the arm started in - the trap the
switch commit already recorded, in a second construct.

Four findings on the way:

1. **`sizeof` evaluated its operand.** `int s = sizeof(x = 3)` set `x` to 3, `sizeof(x++)`
   incremented it, and `sizeof(a ? (b = 3) : a)` set `b`: C99 6.5.3.4p2 says the operand is not
   evaluated. There is one parser flag for this now (`unevaluated`), set by `sizeof`'s operand and
   by `constant_expression`; `emit` refuses to emit while it is set, and `not_lowered` stays
   quiet - so `sizeof(p->a)` no longer reports "a member access in code is not lowered yet".
2. **A constant expression in a body emitted code.** `int a[1 ? 2 : 3]` left a branch and two
   blocks in the middle of the declaration *and* lost its folded value ("expected a constant
   expression"); `&&`/`||` had the same fault since M6.1. The flag is what makes a declarator's
   size and a `case` label read for their value.
3. **The verifier compared types by `TypeId`.** `int *p = c ? &a : &b; p == &a` failed as
   "compares values of another type", because every `pointer(...)` is a fresh id. ADR-004 fixed
   that in the `Convert` check only; Load, Binary, Compare, Unary, Const and Slot compare
   structurally now, which is what the arena's identity means.
4. **An aggregate parameter or result was an internal error.** `struct P f(struct P p)` reported
   `internal: a value of type struct P has no machine type yet`: the refusal was written before
   the body existed, and `not_lowered` asks whether one is being read, so it said nothing. It
   runs after the body is opened and inside `errors_before`'s count, so it is a diagnostic and the
   body is dropped.

The test runs: pick, a guarded division, a right-associative chain, `&&` in the condition, pointer
arms (`&a`, `&b`), a literal's address, calls in both arms, a `void` arm, a `long` merge, a `?:`
in an array's size, and `sizeof`'s unevaluated operand.

142 unit, 18 parser, 11 CLI, crt, pp gate, fuzz, exe - green; fmt and clippy clean.

### local initialisation, reviewed - 2026-11-16 - `char + char` was eight bits

The review of the local-initialisation commit, plus what probing the new path turned up. Two of the
findings were older than the commit and silent.

1. **`char + char` was computed in eight bits.** `usual_arithmetic` compared the two types and
   returned the wider one, without the integer promotions C99 6.3.1.8 puts *first*: 120 + 121 came
   back as -15. It is pre-existing and reachable from the first `char` sum anyone writes; nothing in
   the suite ran arithmetic on a type narrower than `int`. `promote` moved from the parser into
   `Types`, where the conversion rules live, and `usual_arithmetic` promotes both sides before
   comparing them; the parser's copy is gone. `chars`, `unsigned_chars`, `negative` and
   `promoted_size` (the `sizeof` is 4) are the test.
2. **The initialiser wrote a value before checking it.** `int a[2] = {1, "x"}` produced three
   diagnostics: a failed conversion, "this initialiser value is not lowered yet" for the store that
   then could not be made, and finally the real "cannot initialise int with pointer to char". The
   check runs first now, in the one leaf writer, and a test asserts the count is one. `leaf` and
   `write_leaf` were two functions with the same doc comment and one caller between them; they are
   one function.
3. **Adjacent string literals were refused as an unreadable escape.** `string_bytes` decodes one
   literal, but its callers concatenated the *spellings* of the run (`"ab" "cd"`) and handed that
   over, so the decode saw stray quotes and the length check failed. The run is decoded once now,
   token by token, in `string_literal`, which hands the bytes back: the initialiser no longer
   re-scans the source for the spelling (twelve lines of pre-scan deleted), and
   `string_literal_bytes` takes the bytes instead of a spelling. `char s[] = "ab" "cd"` is five
   bytes at file scope and in a frame.
4. **A compound literal's leaves went into the enclosing object.** `int y = (int[2]){7, 8}[0];`
   walked the literal's brace list with the *outer* initialiser's destination and offset, so the
   second int was stored four bytes past `y`'s slot. Nothing wrong reached the user - the enclosing
   initialiser then fails to produce a value, and the body is dropped - but the state leak is the
   bug: `cast` suspends the initialisation (bytes, offset, and slot together) while it reads an
   object of its own.

The three fields an initialiser runs on - `bytes`, `offset`, `init_target` - now say in one place
that they move together. They were not merged into a struct: the parser's state is flat throughout,
and the invariant fits in a comment; the compound-literal case is the second place that saves and
restores them, and the second place is what would have justified the struct.

141 unit, 18 parser, 11 CLI, crt, pp gate, fuzz, exe - green; fmt and clippy clean.

### local initialisation - 2026-11-16 - the hole the review found, closed

The review above found that an initialiser in a function was read, checked and then dropped. It is
lowered now: a local's leaves become stores through its frame slot, and what the list does not name
is zero (C99 6.7.8p21).

The design was already half there. `write_leaf` is the one place a leaf lands in an object, and it
had one destination - the buffer ADR-004 fills for a file-scope object. A local is the same leaf
with a different destination, so the parser remembers the slot it is writing through
(`init_target`, next to `bytes` and `offset`), and `write_leaf` stores instead of copying. The
address of a leaf is a walk by *bytes* from the slot: `pointer_walk` with `char` as the element,
whose scale is one, then a bitcast to the leaf's type - no second address-arithmetic.

What the work turned up, worst first:

1. **A file-scope array's initialiser wrote every element at offset zero.** `int a[3] = {1,2,3}`
   gave `{3,0,0}`: `initialize_array` never advanced `self.offset`, while `initialize_record`
   did. The unit test asserted the object's *size* (12 bytes) and nothing looked inside, so the
   bug survived the data patches and the review that followed them. The walker now sets
   `base + index * size_of(element)`, and the unit test asserts the bytes.
2. **`char *p = "abc"` at file scope was a null pointer.** The string became `.str0`, the pointer
   was left out of the initialiser, and the object was declared with zeroes. ADR-004's bytes
   cannot express a relocation, so this is refused now with a diagnostic instead of being wrong;
   lowering it means teaching a global's data description about addresses, which is an ADR-level
   change, not a patch.
3. **The null pointer constant rule lived in `assign`.** `int *p = 0;` needs it, `return 0;` from
   a pointer-returning function still does not have it, and a third caller was coming. It is in
   `convert` now (C99 6.3.2.3p3), where the conversion happens, and `assign`'s copy is gone.
4. **A float literal has no code.** `double d = 1.5;` is typed and then stored as nothing, so a
   local which had no cir and no constant is refused ("this initialiser value is not lowered
   yet") rather than reading the frame. The refusal leaves when M6's float constants land.

Also settled while writing it: the diagnostic spelling. `init.rs` said "cannot initialise" in one
place and "cannot initialize" in another, and two tests pinned one each. Both say "initialise",
like the rest of the file's prose.

Still not lowered, with a diagnostic each: a record or array-of-record local (its initialiser is
read and its mistakes reported, but the declaration itself is refused); a pointer to a literal at
file scope; local aggregate initialisation beyond arrays of scalars.

140 unit, 18 parser, 11 CLI, crt, pp gate, fuzz, exe - green; fmt and clippy clean.

### array locals and `++`/`--` - 2026-11-16 - a review of the last four code commits

Not a corpus audit: a pass over everything committed since the previous review that it did not
cover - `++`/`--` (153e29f), its clippy follow-up (ece00bd), the M6 test (bfbfc85), and array
locals (70d2b1b). The test that found most of this was written for the array commit and then
asked for `sizeof`, a cast, `*p` and an initialiser.

Findings, worst first:

1. **A local initialiser was read, checked, and never stored.** At file scope the initialiser
   writes bytes; in a body it wrote nothing, so `int f(void) { int x = 3; return x; }` returned
   whatever the stack slot held (21, when asked). That is every `int x = 3;` in every function -
   the most ordinary C there is, miscompiled and silent. Arrays made it worse: `int a[3] = {1,2,3}`
   and `char s[] = "abc"` parse, size and check correctly and then read uninitialised memory.
   Refused now (`initialising a variable in a function is not lowered yet`), which is the honest
   state until a slot can be written; the work is one function and belongs with M4.2b's frame
   writes, not with this review.
2. **An array with no size became an internal error.** `int f(void) { int a[]; ... }` reached the
   back end and reported `internal: a slot for an incomplete type`, because `lowerable` had just
   learned to accept arrays. A source diagnostic now (C99 6.2.5p20), asked *after* the initialiser
   so that `char s[] = "abc"` is not refused for the size it is about to be given.
3. **`is_assignable` was carrying the string-initialiser rule.** `(array of T, pointer to T)` was
   accepted "so a string literal can initialise a char array", which put an initialisation rule
   inside the predicate that assignment, casts, calls, the ternary and `cir`'s verifier all ask.
   It bought the array commit a special case in `assign` to keep `s = "hi"` out. No caller needed
   it: removed, and the corpus is unchanged (`cc16_strarray*.c` stay rejected, as correct C).
4. **"A value that is its address" had no name and three spellings** - `read_place`'s
   `Array | Function` match, `decay`'s `matches!(... Array)`, and a comment in `subscript` - and
   the one function implementing it lived in `stmt.rs` although all three callers are expressions.
   It is `Types::value_is_address`, used by both, and `read_place` sits next to `convert` and
   `constant_of` in `emit.rs`, the seam that knows `cir`.
5. **A refusal in front of the arguments it was about.** The variadic refusal added with the array
   commit ran before the argument loop, so `printf(1)` reported only "not lowered" and never
   "argument 1 is int, expected pointer to char" - and the arity check never ran either. Moved
   behind both; `emit_call` lost the `variadic` parameter, which then had only one possible value.
6. **A branch in `step` that could not be taken.** The written value always has the operand's type,
   so the `None` arm converted it to the type it already had and then stored nothing. Deleted.
   `self.spell(self.peek().expect("checked")) == "--"`, twice, is `self.at_punct("--")`.
7. **A doc comment that said the same thing twice**, above `widen_flag`.
8. **`p++` had no test.** `++`'s test only exercised `int i`, and the pointer form had been passing
   a *pointer* where the index goes; nothing would have caught it. The array test steps to a
   written element now (`stepped`), and the review's own three emitters (`sizeof`, a cast, `*p`)
   were found the same way.

Recorded, not fixed - each needs a decision rather than a patch:

- **`int a[];` at file scope is an internal error.** `the object `a` has no size`. C99 6.9.2p5
  completes it to one element at the end of the translation unit, and nothing does.
- **Two tentative definitions of one object are two data objects.** `int x; int x;` is accepted
  deliberately (the parser allows it) and then `cranelift module: Duplicate definition of
  identifier: x`. One object, one `cir::Global`, is the fix.
- **`int a[2][4] = {"ab", "cd"};`** - a string where a row belongs is descended to the `char`
  element and refused ("cannot initialize char with pointer to char"). Pre-existing, and a
  brace-elision question for M6's aggregate work.
- **`?:` emits nothing**: `int f(int a) { return a ? 1 : 2; }` compiles to a trap, because `cir`
  has no `Select`. Recorded in the array-locals entry above; unchanged.

139 unit, 18 parser, 11 CLI, crt, pp gate, fuzz, exe - green; fmt and clippy clean.

### array locals, the fifth attempt - 2026-11-16 - ADR-004's answer, implemented

The answer (the slot's address is `pointer to array[N]`; `decay` is the one `Bitcast`) made the
three failed patches collapse into three lines: `lowerable` accepts a sized array, `named_value`
returns the slot without a load, and `decay` bitcasts. The verifier's `Convert` check compares
with `Types::same` now, which the attempt needed and `==` could not give.

Reverting the fourth attempt was worth it, because what it left behind was not array work: **four
places emitted nothing for a value the parser believed it had**, and each one showed up only as
`ud2` - a compiled function falling into `unreachable` because no `return` was ever emitted. Found
by asking the new test for `sizeof`, a cast, and `*p`:

1. **A cast emitted no conversion.** `cast` ended in `Value::new(to, from.constant, false)`, which
   dropped the source value: `(int)x` computed nothing. It is `self.convert(from, to, span)` now.
2. **Constants that were not literals had no instruction.** `sizeof`, character constants and
   enumerators produced `Value::constant`, while `primary_expression` gave number literals
   `constant_of`; only the literal survived to code. All three go through `constant_of` now,
   which is the point of its existence.
3. **`*p` was a value with no place**, so a dereference computed nothing. It is the place-and-load
   `p[i]` is built from - the same `load_place`.
4. **`p++` passed a `pointer` where an index belongs**, so the scale never ran. One `constant_of`.

Two refusals were added rather than left half-emitted, both pending M6: **a call to a variadic
function** (a signature built from the declared arity does not verify against five arguments;
Win64 varargs is the missing piece) and **assignment to an array**, which is a constraint
violation (C99 6.5.16) that the string-initialiser rule in `is_assignable` had been letting
through - three corpus files (`cc16_strarray*.c`) are listed as correctly rejected now.

Not fixed, and the same class: **`?:` emits nothing**. `int f(int a) { return a ? 1 : 2; }`
compiles to a trap, because the arms' values are dropped and `cir` has no `Select` to hold them.
That needs an instruction, so it is a design question like the array one was, not a patch.

### array locals - 2026-11-15 - an attempt, reverted, and the question it exposed

Nothing was committed: the attempt failed `cir`'s verifier and was reverted rather than shipped
half-working. Recorded because the failure is a *design question*, and four patches in a row
failed it identically:

    int f(void) { int a[3]; a[0] = 1; return a[0]; }
    Broken { message: "block 0 converts other types than it says" }

What the attempt established, which is worth keeping:

1. **An array local needs no load.** Its value is its address (C99 6.3.2.1p3), so `named_value`
   returns the slot as both `cir` and `place`, and `lowerable` must accept a sized array.
2. **`decay` cannot relabel a value.** Turning `array[N] of int` into `pointer to int` changes the
   *type of the address*, and `cir` types its values, so the change is a `Bitcast` - and the
   verifier is right to refuse a relabelled value.
3. **`cir`'s verifier compared `TypeId`s with `==` in the `Convert` check** while the type arena
   has structural identity: two pointers of the same meaning may be different ids. That is a real
   weakness, found by this attempt, and it belongs in the commit that makes arrays work - with the
   array as its test, not before it.

The open question, which the next attempt should answer before writing code: **which type does an
array local's address have in `cir`** - `pointer to array[N]` (what the slot is) or
`pointer to element` (what C's decay means) - and where that conversion happens once, so that
`a[i]`, `&a`, `a + 1` and passing `a` to a function all agree. Four patches each answered it
differently in a different place; that is the definition of the fact having no home.

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
