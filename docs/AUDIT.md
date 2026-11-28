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

### M109 - 2026-11-25 - the SDK's own windows.h compiles

Six gaps, each one measured by the same number, and the number reached zero:

```
a wall of missing headers  ->  11 errors  ->  10  ->  4  ->  2  ->  1  ->  0
```

`#include <windows.h>` against `Windows Kits/10/Include/10.0.26100.0` now writes a **31 337-byte object** at level 2.
Linking it is the next frontier (`MapViewOfFileNuma2` is unresolved: `winnt.h`'s SEH helper calls it and this compiler has
no linker-level answer yet). The object is the claim; a `.exe` is not.

**1. A qualifier after a struct tag.** `typedef struct _X const *PCX;` - `winioctl.h` writes four thousand of them - was
refused with "expected `;` after a declaration", because the `struct` arm of the specifier loop **returned**
`finish_specifiers` instead of carrying on to the `const`. The `enum` arm has always carried on; only `struct` and `union`
stopped. One `continue`, plus a guard so that `struct T int x` still cannot swallow the `int`.

**2. And then a `const` that was read and dropped.** The test for (1) failed: `p->x = 2` through a `const struct T *` was
accepted. Two holes, one behind the other - `finish_specifiers` returned early when the type came from a tag, before
applying `is_const`; and `member_access` gave a member its own type, so `p->x` looked modifiable while `*p` did not. C99
6.5.2.3p3 with 6.3.2.1p1: **every member of a const-qualified aggregate is const-qualified**. Both are fixed, and the test
asserts the second through the spelling real code uses.

**3. `__alignof`.** MSVC's spelling of `_Alignof`, and it takes an expression as well as a type: `winioctl.h` asks
`__alignof(DEVICE_DSM_RANGE)` about a *type*. Type first, then an expression, read in an **unevaluated** context - and the
numbers a `gcc` build prints for `8 8 8 1 4`, including the `4` for `__alignof(n++)` that this compiler also says, are the
test. `n` is still 0 afterwards.

**4. `excpt.h`, which the SDK does not ship.** `<um/winnt.h>` writes `#include <excpt.h>` in the middle of itself and
Microsoft's copy lives in the **toolset**. What a machine with the Kits and no toolset has instead is mingw's, whose first
act is `#include <crtdefs.h>` = `<corecrt.h>` - and in that chain `corecrt.h` is the *SDK's*, which defines neither
`__MINGW_EXTENSION` nor `_CRTIMP`. The `-E` dump says it plainly: both names survive as **undefined identifiers**. Two C
runtimes' headers in one translation unit; `clang` refuses the same chain. So `excpt.h` is a **bridge header** now, with
Microsoft's content - the disposition enumeration, the x86-64 `__C_specific_handler`, the accessor macros - and the header
itself records why a shipped copy is right here and wrong for `malloc.h`.

**5. `__has_builtin`, which must answer two different questions.** `winnt.h` tests `#ifdef __has_builtin` and *then*
`__has_builtin(__builtin_offsetof)`: the first asks about macro **definitions** (which no expansion hook can answer) and the
second about this compiler (which a `#define` cannot). The first attempt answered only the second, and `-E` showed the SDK
still writing `(LONG)(LONG_PTR) & ( ( ( T * ) 0 ) -> field )` - the fallback, whose address expression this compiler cannot
fold at file scope, so `C_ASSERT` failed. Now the name is **defined** (`__has_builtin(x)=0`, for `#ifdef`) and **answered**
by a twelve-entry list of the builtins that are really here. Both directions of both questions are a test.

**6. The small ones.** `#pragma deprecated` joins `HARMLESS_PRAGMAS` (it marks a name for the editor). `_stricmp`,
`_strnicmp`, `_wcsicmp`, `_wcsnicmp` are declared in `string.h`/`wchar.h` for the SDK's `stralign.h` - and the
`C_RUNTIME`/`UNAVAILABLE` test caught each one before commit, which is the test doing exactly its job.

**The oracle.** `the_sdks_own_windows_h_compiles` in `tests/real.rs`: the Kits' `windows.h` must compile at level 2, or the
test says which line stopped it. It skips where there is no SDK, and it fails on any of the six. The old pin
`the_sdk_gets_past_the_toolsets_vcruntime_h` is **deleted** - it asserted "it does not have to compile yet - it does not",
and that is now false.

215 unit, 57 parser (+1), 60 exe (+1), 14 real (the pin replaced by the claim), 6 invalid, 1 crt, 33 CLI, 5 hostile,
the differential corpus 417 ran / 0 mismatched, the x64 corpus 445 agree; fmt and clippy clean.

### M108 - 2026-11-25 - the budget that spent itself on warnings

Reviewing the commit that fixed the SDK's first barriers found the reason its diagnosis had taken so long: **the diagnostic budget
was spent on warnings, and the errors were cut**.

```
#include <windows.h>  2 400 warnings about dropped pragmas  ->  the 4 errors were never printed
                      exit 1, and nothing in the log saying why
```

Two places had the same rule wrong, and the fix is one policy expressed twice because the list exists twice: the parser stops
collecting, and `compile::report` cuts what it collected.

**The parser** stops warnings as soon as the list is full, and stops errors only when the *errors* have filled it:

```rust
let errors = self.diagnostics.iter().filter(|held| held.severity == Error).count();
if diagnostic.severity != Error || errors >= self.budgets.max_diagnostics { drop it }
```

That keeps both halves of the budget's job: a hundred warnings cannot keep the next error out, and a hundred errors still stop the
hundred and first - which is what bounds the work on a pathological file, and what the existing test
`a_capped_diagnostic_list_counts_what_it_refused` pins (120 errors, a budget of 10, ten reported).

**`report`** is where the whole list exists, so it is where the *choice* lives: count the errors, let warnings have what is left
of the budget, and never drop an error while a warning is standing.

```rust
let errors = diagnostics.iter().filter(|d| d.severity == Error).count();
let room = budget.saturating_sub(errors);
```
then retain in source order, and truncate only if the errors alone overrun the budget.

The test is a program with 200 unknown pragmas and a mistake on the last line, and it fails without the change with the list of
two hundred warnings - which is exactly what CI's log was.

214 unit, **56 parser** (one more), 59 exe, 14 real, 6 invalid; fmt and clippy clean.

### M107 - 2026-11-25 - the stray backslash, where the layer actually is, and 2 400 warnings that hid an error

Four fixes, and the first one succeeded on the fourth attempt because the third attempt was *read* rather than guessed.

**The stray backslash.** Reading `pp::preprocess` answered what three guesses had not: `read_file` lexes a whole file up front,
`run()` pulls tokens and sends `#`-at-line-start to `directive()` - whose words **never pass through `emit`** - and everything
else to `emit`, which is where the Stray diagnostic already lived. So the only wrong layer was the **lexer**, which diagnosed at
lex time. Two changes there:

- a backslash that begins no universal character name **ends the identifier** and says nothing (`universal_character_name`
  rewinds instead of complaining);
- a token that consumed nothing becomes one **Stray token**, because without that guard the loop asked again at the same byte
  for ever - a twenty-gigabyte allocation rather than a wrong answer.

```
#define M(a) a	b          unused      accepted
                            used        stray `\` in program
int x = 1	2;              program     stray `\` in program
```

Three behaviours, one layer, and the pp and parser edits from the earlier attempts are not needed at all.

**2 400 warnings hid a real error.** With the stray fixed, `#include <windows.h>` reported **zero errors** and exit 1 - because
the diagnostic *budget* was full of warnings about pragmas this compiler drops: `#pragma region`, `#pragma warning`, `#pragma
comment`, `#pragma intrinsic`. Nineteen of them are now in `HARMLESS_PRAGMAS`, dropped in silence, and the same rule as the
attribute list: each is here because dropping it cannot change a byte this compiler emits. The warnings fell to four, and the
four errors appeared.

**And two more of the toolset's facts**: the SDK's `sal.h` is included by MSVC's `vcruntime.h` and by nothing else, so
`_Field_range_(1, 2) int _locale_mb_cur_max;` was a member whose type was an annotation; and `uintptr_t`, `intptr_t` and
`ptrdiff_t` are established by that same header, so a prototype eight lines in reported "expected `)` after the parameters". Both
are in the fallback now.

**The SDK, measured at each step**: a wall of missing headers, then eleven errors, then ten, then four:

```
excpt.h:47        expected a type specifier
winnt.h:189       an array's size at file scope must be a constant expression
winioctl.h:5255   `DEVICE_DSM_RANGE` is a type, not a value     (three times)
```

214 unit (one more), 55 parser, 59 exe, 14 real; fmt and clippy clean.

### M106 - 2026-11-25 - the fix that was not the fix, and the test that proved nothing

Reviewing the pack-pragma commit meant mutating it, and the mutation **passed**: with `single_token_body` deleted, the packing
test still answered 1 and 2, and the SDK still had no packing error. So the function was dead code, and the real fix had been
the other half of the commit.

**The pragma's words arrive already macro-expanded.** The directive reader expands a directive's arguments before the pragma
reader is reached, so `#pragma pack(push, _CRT_PACKING)` had always been `push, 8` *when `_CRT_PACKING` was defined at all*. The
wall was never the expansion: it was that **`_CRT_PACKING` is not in the Windows SDK** - it lives in the toolset's
`vcruntime.h`, which the fallback from M104 stands in for. One `#define` was the whole fix, and thirty lines of expansion
machinery beside it were an invention.

```
without single_token_body:  a { char; long long } packed by a macro is 9 bytes, and 16 after the pop
                            #include <windows.h>: no packing error
```

So the twenty-nine lines are gone, and the comment where they were says what actually happens:

> The words arrive **already macro-expanded** ... What it cannot see is a name that is *undefined*, which is why the SDK needed
> `_CRT_PACKING` in the fallback `vcruntime.h`.

**And the test was worse than dead.** Its two macro cases had also never landed - `git show` found `LIMIT` only in the commit
*message* - so the commit shipped a test that did not contain the thing its message described. They are in now, with a comment
that says what they pin: a property of the *preprocessor*, not of the pragma. Which is the only reason they are worth keeping:
they would have caught a preprocessor that stopped expanding, and they say nothing at all about the code that was deleted.

The lesson is the one this project keeps relearning in new clothes: **a fix that cannot fail its own test is not a fix** - and
the way to find out is to delete it and see whether anything notices.

213 unit, 14 real; fmt and clippy clean.

### M105 - 2026-11-25 - the pragma that names its number, and the last wall before the SDK

Two more of the SDK's barriers, and the second one leaves exactly one error standing.

**`#pragma pack` could not expand its argument.** The table of packing forms compared *spellings* - `"push"`, `"1"`, `"16"` - so
the SDK's own spelling, `#pragma pack(push, _CRT_PACKING)`, matched nothing. MSVC expands the argument and so does every
compiler that reads real Windows headers. The reader resolves an **object-like macro whose body is one token** now, and nothing
more: a pragma's argument is a value, not an expression.

```
#define LIMIT 1
#pragma pack(LIMIT)      ->  a struct of { char; long long } is 9 bytes, and 16 after the pop
```

**`_CRT_PACKING` is not in the SDK at all.** It is defined by the toolset's `vcruntime.h` - the one the fallback stands in for -
so the fallback gained `#define _CRT_PACKING 8`, which is the value MSVC gives it and the packing of every structure the SDK
declares. With that, `excpt.h` gets past line 11 and the error count over `#include <windows.h>` falls from eleven to ten.

**What is left is one gap, ten times**: the stray backslash in `driverspecs.h`. So `#include <windows.h>` against the Windows
Kits alone is now blocked by *one* thing - a preprocessing-token question - rather than by a wall of them. The test in `real.rs`
pins both of today's fixes by asserting their absence from the diagnostics, which is what a test can do while the target still
does not compile.

213 unit, 14 real; fmt and clippy clean.

### M104 - 2026-11-25 - the fallback header that got past the wall, and the two gaps behind it

Starting the SDK work found that the first barrier is not the SDK at all: `corecrt.h` opens with `#include <vcruntime.h>`,
and that header belongs to the **Visual Studio toolset**. A machine with the SDK and no Visual Studio can therefore find
`windows.h` and cannot compile it - which is exactly what CI said.

So the compiler gained a **third tier** of headers. `Headers::read` was `builtin` then `host`, and is now `builtin`, `host`,
**`fallback`**: a fallback is found only when neither of the other two has the name, which is the opposite of the bridge headers
and the lesson `malloc.h` taught. A machine with Visual Studio gets Microsoft's `vcruntime.h` and nothing is shadowed; a machine
without it gets enough to compile instead of `cannot find`.

With the fallback in place, `#include <windows.h>` against the Kits alone now gets **past the wall** - a test pins it, and it
asserts the *absence* of `cannot find \`vcruntime.h\``, because the SDK does not compile yet and this test is about the one thing
that changed.

**The two gaps behind it, measured and not fixed**:

```
excpt.h:11: error: `#pragma pack(_CRT_PACKING)`: the limit here is 1, 2, 4, 8 or 16 ...
driverspecs.h:401: error: stray `\` in program          (ten times)
```

- **`#pragma pack` does not expand its argument.** The SDK writes `#pragma pack(push, _CRT_PACKING)` with `_CRT_PACKING` a
  macro for 8; MSVC expands it, and so does every compiler that reads real Windows headers.
- **A stray byte in a macro body is diagnosed at definition time.** `driverspecs.h` writes `(#kind, param	)`: under C99
  6.10.3p3 a backslash is a *preprocessing token*, legal in a replacement list, and a mistake only if the macro is used. Three
  attempts at this were made and **all three were reverted** - moving the diagnostic from the lexer to the preprocessor to the
  parser, each time finding that the layer could not tell (the scanner lexes the whole file before the preprocessor reads a
  directive) and the last one silently accepting a stray byte in real program text. The code is back at `39e7f06` for those three
  files, and the gap is written down with its reproduction rather than half-fixed.

213 unit, **14 real** (one more), 55 parser; fmt and clippy clean.

### M103 - 2026-11-25 - the compiler cannot read the real Windows SDK

Allowing the SDK fixtures to run on CI found something bigger than a test bug, and the annotation said it in one line:

```
sqlite does not link: windows.h:31:2: warning: unknown `#pragma region` ignored
```

Three fixtures failed there - SQLite, Lua, mimalloc - all of them at the same place. **This compiler has never compiled
against the real Windows SDK.** Every SDK test so far ran on a machine with MSYS2, whose `windows.h` is mingw's: self-contained,
mingw-flavoured, and nothing like Microsoft's. The SDK's `windows.h` opens with `#pragma region`, includes `vcruntime.h` and
`sal.h`, and uses MSVC intrinsics - and the search added in M76 *finds* those headers without any trouble, because finding a
file and reading it are different questions.

That is the honest state of the feature M76 claimed: **the platform headers are found; they are not yet usable.** The claim was
tested only against mingw, so it was a claim about MSYS2.

Two changes, and the second is the one that keeps CI honest:

- **The gate is a compile, not a directory listing.** `windows_sdk()` builds

  ```c
  #include <windows.h>
  int main(void) { return 0; }
  ```

  and answers with the directories only if that works - the one question whose answer means "this installation is one this
  compiler can use". Verified both ways: with MSYS2 the fixtures run; with the Windows Kits alone they skip, and the skip *says*
  `the platform's headers are not usable by this compiler`.
- **The skip is the finding, printed.** A green tick that hides an unusable SDK is how this went unnoticed for ten commits.

The next real item is therefore not on the old list: **make `#include <windows.h>` compile against the SDK** - `#pragma region`,
`vcruntime.h`, `sal.h`, `__declspec`, and whatever else Microsoft's own headers ask for. That is the frontier the corpus pointed
at from a different direction (M75: 87 % of refusals are missing headers), and now it has a one-line reproduction.

213 unit, 13 real, 59 exe, 33 CLI; fmt and clippy clean.

### M102 - 2026-11-25 - the second search for the same place, and the edition it did not know

The sweep of machine-dependent tests found the class in its most damaging form: **`windows_sdk()` spelled three paths
itself** - MSYS2's two prefixes and Visual Studio **Community** - while the compiler's `platform_directories()` had, since M76,
searched `INCLUDE`, both MSYS2 prefixes, all four Visual Studio editions and the Windows SDK.

Two consequences, and the second is the one that matters:

- **Two searches for one place**, which agree only by accident. They had already stopped agreeing: `platform_directories` knows
  Enterprise and Professional, the test's copy did not.
- **Every fixture that needs the SDK skipped on CI.** `windows-latest` has Visual Studio *Enterprise*, so the test's search
  answered `None` and SQLite, Lua, mimalloc and tre were never compiled, linked or run on the runner - the halves that *run* real
  programs were exercised only on the machine that wrote them.

`windows_sdk()` is the compiler's answer now:

```rust
fn windows_sdk() -> Option<Vec<PathBuf>> {
    let directories = xcc::platform::platform_directories();
    directories.iter().any(|path| path.join("windows.h").is_file()).then_some(directories)
}
```

and it returns the whole list rather than one path, because the fixtures need the CRT's headers *and* `windows.h`, which live in
different directories of the same installation.

**What that means for CI, stated before it is observed**: the SDK fixtures now run there. That is four real programs - 265 977
lines of SQLite, Lua, 21 units of mimalloc, tre - compiled, linked and run on every push, where before they skipped. If the
runner cannot do it, the honest outcome is a failing gate and a real bug on a machine nobody develops on, which is what a gate is
for.

Also aligned: `exe.rs` and `x64.rs` each probe for a host linker, in *different orders* (`cc, gcc, clang` and `clang, cc,
gcc`), which is two answers to one question on a machine with more than one. Both ask for `clang` first now.

213 unit, 59 exe, 55 parser, 13 real, 6 invalid, 33 CLI; fmt and clippy clean.

### M101 - 2026-11-25 - the test that passed for the wrong reason, once its neighbour was gone

Removing the machine-dependent test left its sibling with the same flaw, hidden by the neighbour:

```rust
fn an_include_flag_shadows_the_platform() {
    if platform_directories().is_empty() { skip }
    write mine/io.h;  #include <io.h>;  expect mine's marker
}
```

It wrote its own `io.h` in the `-I` directory and then relied on the **machine** having a platform `io.h` for it to shadow. On a
machine without one, our header won because there was nothing else - a pass that proves nothing. The same shape as the test that
was just deleted: assert a property of the compiler by depending on a fact about the machine.

Both directories are the test's own now: `platform/io.h` marked `FROM_PLATFORM`, `mine/io.h` marked `FROM_MY_INCLUDE`, the first
named in `INCLUDE` (which the search reads before anything else) and the second passed as `-I`. The program `#error`s if the
platform's header was read.

**Two mutations, and only the second one is the test:**

- putting the *shipped* headers before the host's (`builtin` first) changes nothing - that invariant belongs to `Headers::read`
  and has its own unit test in `headers.rs`;
- putting the platform directories before the `-I` roots fails it, with the program's own words: *"the platform header was read
  instead"*. That is the invariant this test names, and now it holds it.

213 unit, 59 exe, 55 parser, 13 real, 6 invalid, 33 CLI; fmt and clippy clean.

### M100 - 2026-11-25 - the test that asked the wrong machine

CI failed on three runs in a row, and the annotation named the test:

```
xcc ci:     a_platform_header_is_found_without_an_include_flag
```

It compiled a program including `<unistd.h>` and asserted the platform search found it. `unistd.h` is **mingw's** - it does not
exist in the Windows SDK - so the test passed on the machine that wrote it and failed on `windows-latest`, which has Visual
Studio and no mingw. A test for "the platform's headers are found" that requires one platform's headers.

The fix: `<io.h>`, which the SDK's `ucrt` has. And a second test for the half of the mechanism CI depends on and no test named:
a platform header may `#include` one from a *different* platform directory - the SDK's `io.h` includes `vcruntime.h`, which
lives in the Visual Studio toolset, two directories apart. That one makes the shape from two directories of its own and puts
them in `INCLUDE`, so it is checked on any machine rather than only where Visual Studio is installed.

**How the failure was diagnosed, which is the part worth keeping**: not by reading the workflow again, but by asking GitHub.
Public repositories expose their Actions runs without a token, and the API gives the failing step, the annotation the workflow
already prints (`::error::` lines from the log), and the test's own name:

```
GET /repos/xiaoshaoning/xcc/actions/runs?per_page=3          -> conclusion: failure
GET /repos/xiaoshaoning/xcc/actions/jobs                     -> step 7 "Test" failed
GET /repos/xiaoshaoning/xcc/check-runs/<job>/annotations     -> the test, at crates/xcc-cli/tests/cli.rs:805
```

Two attempts were spent guessing first - the size ceiling (75 396 of 81 950, passing) and the newest fixture - before the
obvious move, which was to **read the failure**. The workflow had been printing the failing test's name all along; the only
thing missing was asking for it.

**Then the fix failed too, and taught the real lesson.** With `<io.h>` the runner still failed the same test, because the
SDK's `io.h` includes `corecrt.h`, which includes `vcruntime.h` from the *toolset* - and the annotation showed only the panic's
location, never its message, because the workflow's `grep` took the panic line and dropped the lines after it. Two repairs,
both worth more than the test:

- **The test is gone.** It compiled a header from whatever SDK the machine has, so it tested the *machine*: whether an
  installation is complete, whether its headers parse. The search is tested by
  `a_platform_header_may_include_one_from_another_platform_directory`, which builds two directories of its own and puts them in
  `INCLUDE` - the same test everywhere, and it passed on the runner while the other failed (33 passed, against 32 before).
- **`grep -A2`** in the CI step that reports failures, so the assertion's own message reaches the annotation. A gate that says
  *which* test failed and not *why* is half a gate, and this was the second time the reason was in the log and not in the
  report.

### M99 - 2026-11-25 - the eleventh fixture, and the first oracle that is not clang

`debug_isqrt.c` is vendored unmodified as `tests/fixtures/fixed_point/`, with `gcc -O2`'s output recorded beside it in
`expected.txt`.

**That is what makes it worth a fixture.** Every other one compares this compiler against a different *invocation* of a compiler
it shares a lineage with - clang, whose `-O0` output the fixtures record. This one compares it against a second
*implementation* of the language, on arithmetic where the answers are numbers rather than layouts: Newton's method on Q30/Q31
values, 64-bit multiplies, shifts, and 47 lines of `%llx`/`%f`/`%08x`.

```
gcc -O2 on Linux    47 lines
xcc --opt-level=2   47 lines, identical
```

It also exercises the pieces this session touched without meaning to: 64-bit multiplies that need the full result, shifts of
values wider than thirty-two bits, and `printf` with three conversion kinds in one line.

The judgement call the survey left open is recorded in the fixture's README rather than buried: this is *this project's own
code*, not a third-party library, so the principle the other ten fixtures were built on - a program nobody wrote for this
compiler - is satisfied in the sense that matters and not in the sense that is easy to state.

213 unit, 58 exe, 55 parser, **13 real** (one more), 6 invalid; fmt and clippy clean.

### M98 - 2026-11-25 - the survey of what the corpus can actually run

The census says 386 of 2 000 sampled files compile. This asked a harder question of the same sample: **how many build an
executable that runs and prints something**, and of those, how many agree with another compiler.

```
2 000 sampled
   27 with a `main` that build and link
   13 print something
    2 agree with gcc on every byte, line endings aside
```

The thirteen are mostly LLVM and ikos test files (`f1: carp`, `a(1) returns 4`) - programs written to be looked at by a
debugger - and scratch code. The two that agree:

- **`rs_encode.c`** (64 lines, `work/dnr_c_codes`): Reed-Solomon over GF(256), two 256-entry tables, one line of output.
  `Parity: 144, 206, 194`, identical to `gcc`.
- **`debug_isqrt.c`** (139 lines, `fixed_point_library/tools`): a fixed-point square root by Newton's method, 47 lines of
  `%llx`/`%f`/`%08x` debug output built on 64-bit multiplies and shifts. Identical to `gcc`.

Both were checked byte for byte, and the only difference is `
` against `
`, which is the C runtime's business: ours writes
through the Windows CRT, `gcc` on Linux does not.

**Neither becomes a fixture without a decision that is not mine to make**: both are this project's own code from other
directories, not a third-party library. The principle the fixtures were built on - *a program nobody wrote for this compiler* -
is satisfied by either, and vendoring one's own worktree is a different question from vendoring xxHash.

What the survey does establish is worth having on its own: **the compiler's output agrees with GCC's on real fixed-point and
Galois-field arithmetic**, compiled by two independent implementations of the same language, with no fixture required.

### M97 - 2026-11-25 - the second spelling that nobody had listed

Reviewing the nine hint attributes found the reason a name added in one spelling works in one spelling: there is **no
normalisation**. GCC accepts every attribute as `__x__` as well as `x`, so the list held eleven names twice - `unused` and
`__unused__`, `pure` and `__pure__` - and everything added since was accepted under only one of its names.

```
__attribute__((__unused__))     accepted before (listed twice)
__attribute__((__device__))     refused   - `device` was listed, `__device__` was not
__attribute__((__no_sanitize__)) refused  - same
```

The fix is one line where the name arrives:

```rust
let name = name.trim_matches('_');
```

and the deletion it enables: **eleven duplicate entries gone** from `HARMLESS_ATTRIBUTES` (54 names to 43), and the two layout
attributes' `matches!(name, "packed" | "__packed__")` collapsed to `name == "packed"`.

**Normalising changes what is looked up, not what is allowed.** `__vector_size__` reduces to `vector_size`, which is not in
the list, so it is still refused - and the test now holds both spellings of a refusal as well as both spellings of an
acceptance, plus `__packed__` still laying a struct out in five bytes and `__aligned__(16)` still aligning to sixteen.

That is the seventh time this session that a name written in two places disagreed with itself, and the first where the
duplication was the *design*: a list of names that has to know every spelling of every name is a list that will not.

213 unit, **59 exe** (one more), 55 parser, 12 real, 6 invalid; fmt and clippy clean.

### M96 - 2026-11-25 - the hints that may be dropped, and the ones that may not

The refused-by-name list had ten entries left that are **hints for something this target does not do**, and they are the same
kind of thing the list already held: an attribute goes in only if dropping it cannot change what the program computes *here*.

```
no_sanitize, no_sanitize_address, no_sanitize_thread, no_sanitize_undefined
target_version, disable_tail_calls, m68k_rtd, mig_server_routine, device
```

A sanitizer this compiler does not have, diagnostics that do not run, function multiversioning in a program with one dispatch,
a tail call this back end does not make, and three calling conventions belonging to m68k, Mach and CUDA. Dropped, and the
program prints what it computes (`28`).

**What is left refused, and it should be**: `vector_size`, `ext_vector_type`, `neon_vector_type`, `mode` and `__ms_struct__` -
the names that decide a type's size, a member's offset or a field's width. The test keeps one of them refused so the boundary
stays stated: an attribute that changes a type is not a hint.

**Measured, and the number is small on purpose**: over the same 2 000-file sample, refusals by attribute fall from 45 to 41 and
accepted files rise from 383 to 386. Four files, because the names that dominate that list are the ones that *cannot* be
dropped - which is the point. A compiler that dropped `vector_size` would compile more files and produce wrong programs, and the
count of files compiled is not the measure that matters.

213 unit, **58 exe** (one more), **55 parser** (one more), 12 real, 6 invalid; fmt and clippy clean.

### M95 - 2026-11-25 - the map inside the new code, and the overflow that is not a gap

Every commit in this range has a review after it, so this pass swept an area again: four separate fixes landed in the stack
machinery (`StackAlloc`, `StackMark`, `StackRestore`, and the `__chkstk` probe), and nothing had checked the one thing a crash
report depends on - that **`--map` still resolves an address inside a function holding a variable-length array**.

It does. A null store after a VLA declaration:

```
--map                          gdb
000000014000f58d mp1.c:8       0x000000014000f58d in main  ->  mov %esi,(%rbx)
```

`mp1.c:8` is the `*p = 2;` that faulted, and the map's entries either side of it are the lines either side of it in the source.
The `call __chkstk` the encoder now emits carries the *declaration's* line, because it is emitted inside the declaration's
operation - so a reader who takes an address out of that region lands on the array, not on an internal helper.

**The one address the map cannot name is a stack overflow**, and that is not a gap:

```
0x00007ffa9896aad7 in ntdll!.chkstk    =>  test %r11b,(%r11)
#1  0x000000014000f541 in main ()
```

The fault is inside the prober, walking down to the guard page - there is no source line there to name, because the code is
ntdll's. What a reader needs is the *caller*, and the backtrace gives `main`, while the map gives `mp2.c:6` for the addresses
around the call. Two tools, one answer, which is what the split between `.pdata`/symbols (gdb) and `.xccmap` (lines) is for.

**The negative result is worth as much as a fix here**: the three new operations emit code with spans like everything else, and
the map has not needed a special case for any of them.

### M94 - 2026-11-25 - the corpus, thirty commits later

The census was re-run with the same corpus, seed and filter as M75/M76, after everything this session added - variable-length
arrays, `typeof`, statement expressions, `alloca`, the platform header search, the aggregate literal pool, the stack probe and
two conformance fixes.

```
                   accepted   before the parser   by the parser
  before (M76)     365 of 2000      1 426              209
  now              383 of 2000      1 392              225
```

**Eighteen more files compile, and thirty-four more get past the preprocessor** - which is why the parser's own count went *up*:
a file that no longer stops at a missing header arrives with its next gap visible. That is the same effect the platform search
produced on its own (nineteen to accepted, fourteen to the parser), and it is the healthiest number here: the frontier moved
from "cannot find" to "cannot compile this", which is a frontier the compiler can do something about.

**And the top of that frontier is no longer the language.** The sample of "expected a type specifier" - still 44 files, still
second - is:

- CLooG output (`for (int c0 = ...)` at file scope), LLVM test fragments (`{` at the top), fragments of programs;
- Altivec `vector bool char`, another architecture's types;
- a C++ file named `.c` (`bool sgp4init(...)`);
- `main () { ... }` - K&R implicit `int`, which C99 removed and this compiler refuses on purpose.

Every one is either not a program or not C. The forty-five `__attribute__` refusals above it are the vector-type attributes that
cannot be honoured without a vector type (M75). So the ranked queue now reads: attributes that *change types*, fragments, and
other architectures - and no language feature in sight.

That is what thirty commits of language work bought: not "more files compile" but "the files that do not compile no longer fail
because of the language". Which is the point at which a compiler stops being measured by a corpus of other people's code and
starts being measured by the programs it says it runs - the ten fixtures, and the one benchmark that cannot be folded into a
constant.

### M93 - 2026-11-25 - the half of the nesting the refusal missed

Reviewing the refusal from `9d5154d` meant asking what a *partial* nesting looks like - one variable dimension, one constant -
and the answer was another shape refused with a sentence about the wrong thing.

```c
int m[3][cols];        /* constant outer, variable inner */

ours:  error: cannot assign to an array
gcc:   accepts it, and m[2][4] is 24
```

The two-variable case was caught by watching the *second* variable wrapper arrive. Here the second wrapper is a **constant**
array, so nothing fired: the declarator produced "an array of three `int *`" - the inner variable-length array having already
become a pointer - and the declaration then tried to store the allocation into an array, which is what the message described.

The guard is now on the **result** rather than on the second wrapper: if a variable-length array was found and the type that came
out is still an array, the outer array's element is the variable-length one and its stride is a value. One condition, both
shapes, and the same message - which is the one that says why.

Flipped both ways: without the guard the test fails with `["cannot assign to an array"]`, the sentence that sent a reader to the
assignment that is not there.

213 unit, 57 exe, 54 parser, 12 real, 6 invalid; fmt and clippy clean.

### M92 - 2026-11-25 - the stride that was a value, and the crash that became a sentence

The second polar_codes bug is the shape of the first, one level down. With the stack probe in place the program got past `main`
and faulted at `L[0][i_N + i_list*N] = LLR_Q[i_N]` with the row pointer NULL - and `L` is

```c
int L[n+1][l*N];      /* both dimensions are values */
```

**A two-dimensional variable-length array with a runtime inner dimension.** Every VLA test this session wrote had a *constant*
inner dimension (`int a[rows][3]`), where the element is still a type. Here the element is `int[l*N]`, whose count is a value,
and the declarator kept a single "this is a variable-length array" slot - so the outer dimension overwrote the inner and `L` was
allocated `(n+1) * sizeof(int *)` bytes with a stride of eight. The program crashed; a program that had not crashed would have
written over its own stack in silence.

Reproduced in five lines (`int m[rows][cols]`, both runtime, print `m[3][4]` and `sizeof m[0]`), against `gcc` on Ubuntu for
the oracle: `0 34 5 4` there, a fault here.

**Refused by name rather than approximated**, because the honest statement is short: a stride is a value, and a stride lives in
a type. The message says exactly that, a test holds it word for word, and `C99-SUPPORT.md` records the limit beside the others.
The one-dimensional cases are untouched - 45 KB, 1 MB, and 256 KB in a loop all still print what `gcc` prints.

What it would take to support it: a pointer-to-array type whose element count is a *value*, which is a variably modified type
one level deeper than this compiler's model - and every index would multiply by a runtime stride. That is a real project, not a
patch, and nothing in the ten fixtures needs it.

213 unit, 57 exe, **54 parser** (one more), 12 real; fmt and clippy clean.

### M91 - 2026-11-25 - the guard that no program could provoke

Reviewing the `__chkstk` commit meant trying to break its own guard: the probe is a call, a call destroys `r8`-`r11`, and the
allocator is allowed to keep values there when a function has two parameters or fewer. So `Inst::StackAlloc` was added to the
predicate that sets `makes_call`.

The mutation says the predicate cannot be *shown* to matter:

```
int f(int a, int b) {          /* two parameters: caller-saved registers allowed */
  int x1..x6 = ...;            /* six live values */
  int arr[a * b];              /* the probe */
  ...
}
```

with the predicate removed, the program still prints the right number - because the allocator spills those values rather than
reaching for `r8`-`r11`, and every other VLA test has the same shape. **No program I can write provokes the clobber.**

That leaves two choices, and the wrong one is tempting: delete the predicate, because "a fix that cannot fail its own test is
not a fix". The rule is right and its *reason* is what decides here - the predicate is not an optimisation whose result can be
measured, it is a **precondition** of the code that follows. Deleting it would leave a latent clobber, armed for the first
change to the allocator's heuristic, and invisible until a program in the wild hit it.

So the check moved to the assertion that can see it:

```rust
debug_assert!(
    self.makes_call,
    "a stack allocation calls `__chkstk` and the allocator was not told"
);
```

With the predicate removed, `a_variable_length_array_is_as_long_as_the_program_says` now fails inside the encoder with exactly
that message. The guard is exercised by every variable-length array in the suite - which is the difference between a change
that cannot fail its test and a change whose test is a different *kind* of test.

213 unit, 57 exe, 53 parser, 12 real; fmt and clippy clean.

### M90 - 2026-11-25 - the probe that was owed, and the crash that was two bugs

The polar_codes crash had no oracle last time. It has one now: **WSL2 is on this machine**, so `gcc -O2 -lm` on Ubuntu builds the
same two files and runs them to completion - `exit 0` - where this compiler faulted. That settles the direction: the fault was
ours, and the program's own arithmetic is fine.

**The first bug was the one already written down as a ceiling.** The crash was at `node_type[i_Q + i_n*N] = 0`, and the array is
a **variable-length array of forty-five kilobytes**:

```c
int node_type[N*(n+1)];      /* N is a kilobyte */
```

`Op::StackAlloc` moved `rsp` with a single `sub` and never probed the pages it stepped over. Windows commits a thread's stack one
page at a time as each access reaches the guard page; the *constant*-size frame has been walked a page at a time since the lz4
fixture found this, and the run-time one was left with a `ponytail:` comment promising a probe loop.

The encoder asks **`__chkstk`** now - the kernel's own prober, exported by `kernel32` - with the size in `rax` where it expects
it and the subtraction after the call, which is MSVC's sequence:

```
mov rax, size        call __chkstk        sub rsp, rax        mov dst, rsp
```

And the probe is a **call**, so the allocator has to know: `Inst::StackAlloc` joins the thread-local address in the predicate
that sets `makes_call`, because a value left in `r8`-`r11` is destroyed by it. That is the same mistake the thread-local
address records, one layer down.

```
45 KB array       faulted            ->  0 11263
1 MB array        faulted            ->  262143
256 KB in a loop  faulted            ->  65535 12
```

Tested both ways: without the `call __chkstk` the tests fail with `left: Some(-1073741819)`, the access violation.

**The second bug is still open, and it is a different one.** With the probe in place polar_codes gets past `main` and faults
inside `decode_unrolled` at `functions_SSCL.c:173`:

```
L[0][i_N + i_list*N] = LLR_Q[i_N];      rbx = 0     <- the row pointer is NULL
```

A NULL row pointer is not a stack problem. It is the next thing, and it has a reproducer, a line number and an instruction.

**And one of my own tests was wrong**: the first version of the loop test used a *megabyte* array outside the loop and another
inside, which is two live megabytes on a one-megabyte stack - a program that overflows it, not a compiler that fails. The probe
turned it into a clean termination, which is what a probe is for; the test now uses 256 KB and checks the numbers.

213 unit, **57 exe** (two more), 53 parser, 12 real; differential 417 ran, 0 mismatched; 445 corpus programs agree; fmt and
clippy clean.

### M89 - 2026-11-25 - the count that was the last index, and the global that lost a value

Reviewing the array-completion fix (M87) checked the shapes it had not mentioned, and found a worse bug underneath it.

```c
int g[] = { [2] = 5, [0] = 1 };

ours:  sizeof g / sizeof g[0] is 1, and g[2] is 622879781
clang: 3, and g[2] is 5
```

C99 6.7.8p22 says the size comes from **the largest subscript**. The walk tracked the index it was *writing*, so the list
ended at index 0, the count came out 1, and the bytes were then cut to the type's size - discarding the `5` that had been
written at offset 8. A wrong global, with no diagnostic: the worst shape a bug can have.

Two lines: an `extent` that keeps the highest index reached, and `complete_array` handed that instead of `written`. Four
programs checked against `clang`, all four now identical:

```
int m[][3] = {{1,2,3},{4,5,6}}      2 6      (was correct)
int local[] = {7,8,9,10}            4 10     (was correct)
int g[] = {[2]=5,[0]=1}             3 1 5    (was 1 1 622879781)
int d[] = {[2]=5,[0]=1}  (local)    3 6      (was 1 6)
```

**The review's own lesson**: the completion from M87 was right, and the *number it was handed* was not. Checking the shape
the fix did not mention - a designator - found that a pre-existing corruption had been sitting under the same walk, invisible
while the count was None because nothing read it.

55 exe (one more), 213 unit, 53 parser, 12 real, 6 invalid; 445 corpus programs agree; fmt and clippy clean.

### M88 - 2026-11-25 - polar_codes: no oracle on this machine, and a crash with its evidence

The third candidate, surveyed properly. `polar_codes/C/SSCL` is the best of the three on paper: a self-contained `main`, a
fixed seed, twenty units, and **variable-length arrays everywhere**

```c
float EbN0dB[num_EbN0dB];   int info_nodes[N];   int ind_ord_l[l];
```

which is exactly what this session added and what no hand-written test covers. It compiles with nothing but the two files and
links with this compiler - and then it **faults**:

```
Thread 1 received signal SIGSEGV
0x000000014000fd9b in main ()     =>  mov %r15d,(%r14)
r14 = 0x5f0260        r15 = 0
```

A store of zero through a pointer the program computed two instructions earlier (`add %r10,%r14`). Whether that address is
wrong because the *program* computed it wrongly or because this compiler computed it wrongly is exactly the question an oracle
answers - and there is none on this machine:

- `clang -O2` and `gcc -O2` both link and both produce binaries that **will not load**: exit 127, no output, and their import
  tables name `api-ms-win-crt-private-l1-1-0.dll`, an API set this Windows does not have. `-static` changes nothing.
- `--opt-level=0` is not an alternative: this program needs variable-length arrays, and the Cranelift back end refuses them
  by design - so the usual "does the other back end agree" check is unavailable for exactly the programs that need the
  feature.

So the crash is **open**, recorded with the instruction, the register values and the address, because that is where the next
attempt starts. What it is not is a claim: a fault inside `main` on a program whose own index arithmetic has not been checked
against a second implementation says nothing yet about which side is wrong.

**The survey's total**: three candidates, three negatives as fixtures -
codec2 needs CMake's generated `version.h`, libosmocore needs configure, and polar_codes cannot be checked on this machine - and
one real conformance bug found and fixed on the way (C99 6.7.8p22, M87). That is what a survey is for: the bug was worth more
than the fixture would have been.

### M87 - 2026-11-25 - the array whose size was in the file, and the program that counted it

The survey of real programs that need this session's features found a conformance bug in the first candidate, before any
fixture was made.

codec2's `src/ldpc_codes.c` counts its LDPC tables the way C has counted tables for fifty years:

```c
/* in the header */  extern struct LDPC ldpc_codes[];
/* in the unit   */  struct LDPC ldpc_codes[] = { ... };
                     int ldpc_codes_num(void) { return sizeof(ldpc_codes) / sizeof(struct LDPC); }
```

and this compiler said *"`sizeof` needs a complete type, not array of struct LDPC"* - **in the file that had the array's size in
front of it**. C99 6.7.8p22 says the initialiser completes the type; the arena's copy of the type *was* completed, and the
**symbol** still held the incomplete array the `extern` declaration had put there, which is the one `sizeof` reads afterwards.

The fix is two lines in the definition's path: complete the array from the bytes the initialiser produced, and write that type
back to the symbol. The minimal case is a test now - a struct table, an `extern` and a definition - and so is a local
`int sizes[] = {1, 2, 3, 4}`, which already worked and is worth holding.

**No hand-written test would have found this.** It needs two declarations of one name: a header and the unit that fills it.

The survey itself, recorded because the answer decides the next round:

- **codec2** - 98 units, 183 files, and `ldpc_enc.c` compiles with nothing but `-Isrc`. It cannot be *built* without CMake:
  `codec2/version.h` is generated from `cmake/version.h.in`, and 56 units include it. The arithmetic its LDPC encoder does is
  also deliberately transcendental-heavy, which makes it a poor runtime benchmark. **The bug, not the fixture.**
- **libosmocore** - 302 units and its own configure step; same class.
- **polar_codes** - 20 units, no generated headers seen yet, and its VLA is in a simulation driver. **The remaining
  candidate**, and the next thing to try.

213 unit, 54 exe, **53 parser** (two more), 12 real, 6 invalid; fmt and clippy clean.

### M86 - 2026-11-25 - the section change that was not one, rejected by its own measurement

The previous entry proposed moving literals to `.rdata`: 61 KB of `.data` that is writable and should not be. The estimate was
"a section-selection change in the data writer plus a test". That estimate was wrong, and the code says so in its own header:

> **Every object is in `.data`**, writable; an `ADDR64` there becomes an absolute address *and* an entry in `.reloc`, so the
> program works at the base it is given and at any other Windows picks.

That is `pe.rs`'s central model, not a flag: one data section, whose start the RIP-relative addressing and the relocation table
are both expressed against, holding the objects and the import address table together (`Image.data: Data`, one field). Splitting
it means a second section in the image, a section table entry, per-section symbol offsets, relocations split in two, and every
`[rip + disp32]` recomputed against a different base - touching the linker's core to save 61 KB of a 2.4 MB image and make a
string literal fault instead of accepting a write.

**Rejected**, and recorded here because a rejected idea with a reason is worth more than an unasked question. What would make
it worth doing is a *reason*: shared memory between processes, a page-count limit, or a program that writes a literal in the
wild. None of those exist today, and the third would be a bug report about the program.

The next thing is the one that matters more anyway: a real program that needs the new features, so that variable-length
arrays, `typeof` and statement expressions are gated by something nobody wrote for this compiler.

### M85 - 2026-11-25 - sqlite, which cannot be folded

M83 ended by saying the corpus could not answer the code-size question, because its programs are decidable before they run.
This is the answer, on the one program in the suite that is not: `sqlite3.c`, 265 977 lines, compiled and linked by both
compilers, then *run* on 20 000 inserts and a grouped query.

```
                             .text        image       the query, 3 runs
xcc  --opt-level=2        2 183 222    2 464 611     140 / 146 / 146 ms
clang -O2                   972 592    1 308 776      97 /  98 / 103 ms
ratio                          2.24x        1.88x     1.46x
```

**Output identical** - `420000` from both - so this is the same work, timed.

Two things this settles and one it opens.

**Settled**: the 3.44x the corpus reported was clang's folder, exactly as M83 concluded. A program that reads and writes real
data comes out at **2.24x** its size and runs **1.46x** as long, against a back end with no optimisation beyond mem2reg,
folding and a linear-scan allocator. Those are numbers a person can defend.

**Settled the other way**: nothing here says the compiler is *slow*. A 46% runtime gap while emitting 2.24x the bytes says
the extra instructions are mostly not on the hot path - which is what a naive allocator does: it spends instructions on copies
and frame traffic in the straight-line code between branches, not inside the loops sqlite spends its time in.

**Opened**: our `.data` is 75 880 bytes against clang's 14 464. Literals - string literals and now the aggregate
initialisers from M82 - are written as initialised globals, and an initialised global goes to `.data`, which is *writable*
and therefore cannot be shared between processes. Nothing is wrong, and 61 KB on a 2.4 MB image is not worth a project; but
`.rdata` is where a constant belongs, and that is the next small thing rather than the next big one.

```
$ ./xcc_bench.exe 20000            $ ./clang_bench.exe 20000
420000                             420000
```

### M84 - 2026-11-25 - the initialiser that ran twice

Reviewing the constant-pool commit (`39d7b36`) found a bug that no test in the suite could see, because no test had a
side effect in an initialiser list:

```c
int one[4] = { bump(), 10, 20, 30 };
int two[8] = { bump(), bump(), 0, ... };

ours:  1 30 7 4      clang:  1 30 5 3
```

`bump` ran **four** times for three calls in the source. The fast path is an *attempt* - it reads the initialiser as bytes,
and when that fails the element walk re-reads the same tokens - and the attempt was **evaluated**, so every expression in a
non-constant list ran twice: once to find out that it was not constant, once to store it.

The fix is the flag three other places in this compiler already use: `typeof`'s operand, a VLA's bound, `sizeof`. Read
**unevaluated** while deciding, evaluate once when the decision is made. With it, `1 30 5 3`, which is what `clang` prints;
without it the test fails with `1 30 7 4`.

That is the third time this session that a decision-then-decide-again shape needed `unevaluated`, and the first time the
absence of it produced a wrong *number* rather than a wrong type: a doubled `f()` in an initialiser is a side effect the
programmer counted on, and the compiler counted it twice.

The rest of the commit held up: the fast path still applies to constants (128 bytes of `.text` and 296 of `.data` for the
two test objects), aggregates that hold addresses still take the element walk, and the whole 445-program corpus agrees.

213 unit, **54 exe** (one more), 51 parser, 12 real; 445 corpus programs agree; fmt and clippy clean.

### M83 - 2026-11-25 - the lever that was not there, and the corpus that flatters an optimiser

The projection before this was "register promotion, not aggregate initialisation", so `mem2reg` was instrumented to say why a
slot stays in memory: escaped, no store, no load, type pun, or promoted. Over the worst program (`cc50_corners`, 3028 bytes,
713 instructions):

```
promoted 17   escaped 2   other 1   nothing to replace 1
```

**The projection was wrong.** Nineteen slots and seventeen of them are already values; there is nothing left to promote in the
program the size profile called the problem. The two escapes are `&one`/`&two`-shaped, which is what escaping means.

So the 248 `mov`s were broken down instead:

```
112  reg-reg copies        37.7% of the bytes
 75  constants
 61  stores to memory
```

Even perfect copy elimination saves a seventh of the instructions - about 11% of the bytes - for a register allocator
rewrite that is several hundred lines and the riskiest kind of change this back end can take. Not worth it.

**And then the real answer, which ends the question.** `clang -O2` compiles that whole program to **24 instructions**: four
for `bump` and twenty for `main`.

```
0000000000000010 <main>:
  10:  55                    push   %rbp
  15:  48 8d 6c 24 20        lea    0x20(%rsp),%rbp
  1f:  48 b8 03 00 00 00 04  movabs $0x400000003,%rax
```

Every condition in `cc50_corners` is a constant - it is a *correctness* test, written so that each construct can be checked
by an exit code - so an optimiser erases the program. The same is true of the other outliers, and probably of most of the
corpus: these files exist to exercise a front end, and their work is decidable before the program runs.

**So "3.44x clang -O2 on the corpus" measures clang's folder, not this compiler's code generation.** The numbers that mean
something are the ones already written down: 1.62x against `-O0`, and a median of 1.54x against `-O2` across programs where
the comparison is like for like. The code-size work is finished, and the right conclusion is to stop rather than to spend a
week on an allocator for a seventh of the instructions.

What this does *not* say: that the back end is good. It says the corpus cannot answer the question. A program with real data
- sqlite reading a file - is where the answer lives, and nobody has measured that.

### M82 - 2026-11-25 - the constant pool reaches the locals, and the tail halves

The size profile (M80) named one construct for the whole tail: a local aggregate with a constant initialiser was stored
element by element. The compiler already knew how to turn such an initialiser into **bytes** - `initializer_bytes`, which
file-scope definitions use - so the fix is to use it, put the bytes in a literal beside the string literals, and emit one
`memcpy`.

Two changes, measured at each step on the same two programs:

```
int a[10] = {1, 2, 3};  char big[256] = {0};
  before                       5008 bytes of .text
  zeros eight at a time         712     (zero_at stored a byte at a time; 8 bytes is the same bytes, a seventh the stores)
  literal + memcpy              128     + 296 of .data, which is the data itself
  clang -O2                      19     + its own literal
```

And over the corpus:

```
                          text+data, 422 programs      median     cc41_mdim    cc34_aggrinit
  before            3.44x                              1.54x      32.76x       27.64x
  after             3.02x                              1.54x      14.95x       14.09x
```

**The rule that keeps it honest**: the fast path answers `false` and puts the token cursor and the error count back whenever
the initialiser is not a constant, or holds a relocation - `int *p[2] = {&a, &b}` names addresses the *linker* owns, and
copying the bytes of a literal would copy the text of an address. Those keep the element walk, which is correct for
everything, just longer, and a second test holds that case. The bytes are built by the same walker globals use, so a local
and a file-scope definition of the same object cannot disagree.

**What is left is a different project.** The worst program now is 32.91x, and it is not aggregate initialisation:

```
cc50_corners   3028 bytes, 713 instructions, of which 248 are `mov`
```

That is register promotion: a `{ int x = 1; ... }` block whose local lives in a frame slot and is loaded and stored around
every use. `mem2reg` promotes a slot whose address never escapes, and these escape it or are never offered to it. The next
lever is there, not in the back end.

213 unit, **53 exe** (two more), 51 parser, 12 real, 6 invalid; 445 corpus programs agree; fmt and clippy clean.

### M81 - 2026-11-25 - the ceiling that was a crash

Reviewing the block-scope fix in `4a7a20b` meant testing the ceiling it wrote down, and the ceiling was wrong about itself:
*"a `goto` or `break` out of such a block leaves the bytes until the function returns"* sounds like a leak. It is a crash.

```c
for (i = 0; i < 20000; i = i + 1) { int a[2048]; if (i % 2 == 0) continue; ... }
```

A `continue` leaves the body block in the middle, so the restore at the block's *end* never ran, and every pass added another
array: a hundred and sixty megabytes, and a fault. `clang` prints `29997`. This shape - an array and a guard clause - is not
exotic; it is most loops that declare a buffer.

Each loop and `switch` now remembers **how deep the block stack was when it opened** (one `usize` in `targets`), and
`break`/`continue` release every block they leave, deepest first because each mark was taken under the previous one. The
`switch` case - `case 1: { int b[n]; ... break; }` - is the same machinery and was checked too (`24002`), since a `switch`
entry has a `break` and no `continue`.

Tested both ways: with the restores, `29997 24002`; without them, `left: Some(-1073741819)` - the access violation, caught by
the test rather than by a user.

What remains a real ceiling is a `goto` out of such a block, and it is now honestly small: a `goto` fires once, so at most one
array's worth of bytes is lost per function. A `goto` *backwards* into a block would repeat it, and that is pathological
enough to leave written down rather than fixed.

213 unit, **51 exe** (one more), 51 parser, 12 real; fmt and clippy clean.

### M80 - 2026-11-25 - where the bytes go, and the tail that is not the average

The standing claim since M10 is "3.44x clang on code size". Nobody had looked at it, so it was profiled two ways: which
operation pays for the bytes, and how the ratio is distributed across programs.

**Which operation.** A per-`Op` byte tally over the 466 corpus programs (225 052 bytes attributed):

```
Binary        33.4%      LoadMemory     10.3%      Call        5.4%
Const         14.2%      StoreLocal      9.2%      LoadLocal   5.3%
                                    StoreMemory 4.9%   Divide 2.8%
```

Memory traffic - the four load and store kinds - is 29.7%, and `Binary` is a third: `add r64, r64` is four bytes and there
is nothing to win in it. Neither is where the problem is.

**How it is distributed**, which is the measurement worth keeping:

```
                          total          median
vs clang -O0    1.62x                     0.93x     <- smaller than clang on the typical program
vs clang -O2    3.44x                     1.54x     <- the standing claim, confirmed
```

The first run of this compared against `-O0` and produced *1.62x* - which read like the compiler had improved by half since
M10. It had not: the standing number was always against `-O2`, and brackets are not a formality. That mistake is the reason
both lines are written down here.

**The tail is the quality problem, and it is one construct.** The worst programs are not 3x the average, they are 26 to 37
times clang:

```
37.09x   cc50_corners    3004 vs 81
32.76x   cc41_mdim       2064 vs 63
27.64x   cc34_aggrinit   1520 vs 55
26.48x   cc40_elision    1668 vs 63
```

Every one is **aggregate initialisation**. A five-line program makes the shape plain:

```c
int a[10] = {1, 2, 3};      /* 5008 bytes with xcc, 19 with clang -O2 */
char big[256] = {0};
```

The zeros are emitted one element at a time, so a zeroed 256-byte array costs a store per byte. The parser already knows how
to turn a constant initialiser into *bytes* - `initializer_bytes`, which globals use - and a local does not.

**And the gate does not measure it.** `the_corpus_is_within_the_code_size_that_was_measured` asserts an absolute ceiling
(80 516 bytes, now 60 544) and only *prints* the ratio. A ratio gate would be machine-dependent - the local clang decides
it - so the ceiling is right to be absolute; but that means the ratio can double and nothing fails. The tail above is what
a gate cannot see and a person has to.

The instrument that produced this was temporary and is gone: per-`Op` bytes at the call site in `encode.rs`, printed when
`XCC_SIZE_PROFILE` is set. Twelve lines to bring back, and worth bringing back the next time this question is asked.

### M79 - 2026-11-25 - the sweep that found the stack growing, and the block that had to give it back

Every commit this session has been reviewed, so this pass swept an **area** instead: the back end took three changes
(frame pointer, `StackAlloc`, the call sequence), each reviewed alone and never together. The interactions:

- variadic **and** VLA in one function - correct (`sum 15`);
- `longjmp` unwinding *through* a frame holding a VLA - correct (`jumped 7`), which is the `UWOP_SET_FPREG` record doing
  its job;
- seven-argument call from a function with a VLA - correct (`spread 28 32`);
- **a VLA declared inside a loop - a crash.**

```c
for (i = 0; i < 20000; i = i + 1) { int scratch[n]; /* ... */ }
```

Twenty thousand passes of eight kilobytes is a hundred and sixty megabytes, and this compiler asked for all of them:
`rsp` moved down at each declaration and came back only when the function returned. `clang` prints `79997`; this compiler
segfaulted. C99 6.2.4p6 is the rule that was missing - the array's lifetime ends with **the block**, not the function.

So a block is a stack scope now. `Inst::StackMark` takes the stack pointer as a value, `Inst::StackRestore` puts it back,
and the parser marks the **first** allocation in a block and restores at the block's end: a block with several arrays
releases them together, and a block with none costs nothing (no mark is emitted). Two tests hold it - the loop that has to
not crash, and an inner block releasing only its own array, which a mark per *function* would have got wrong.

A first attempt at the test used `int scratch[2048]`, a constant bound, which is not a VLA at all and passed; the second
used `scratch[n]` and crashed on the second pass. Worth recording because the first version proved nothing and looked like
proof.

Known ceiling, written into `C99-SUPPORT.md` rather than hidden: a `goto` or `break` that leaves a block *from inside*
skips the restore, so those bytes live until the function returns - the same shape GNU's `alloca` has, and the fix is
landings for jumps, which nothing in the corpus has needed.

213 unit, **50 exe** (two more), 51 parser, 12 real; differential 417 ran, 0 mismatched; 445 corpus programs agree; fmt and
clippy clean.

### M78 - 2026-11-25 - what the platform search was worth, counted

The change in M76 was measured by its *own* quantity - files stopped by a missing header, 1 071 to 1 042 - and not by the
one that matters. So the parent commit was built in a worktree and both binaries answered the same 1 500 files:

```
1 033   missing header -> missing header
  280   accepted       -> accepted
  152   refused        -> refused
   19   missing header -> accepted          <- the win
   14   missing header -> refused           <- the header stopped hiding the next gap
    1   timeout        -> timeout
    1   missing header -> timeout
```

**19 files, 280 to 299, and nothing went backwards.** The fourteen that moved from `missing header` to `refused` are worth as
much as the nineteen: a file that now stops *inside the language* is a file whose next gap is visible, and the previous
census could not see it. Before the change, a missing `<unistd.h>` answered for every fault a file had.

Two of the nineteen are `curl`'s own examples, four are LLVM's tests, and the rest are projects pulled off a shelf - the
shape the corpus is for.

Nothing here is a gate, and that is deliberate: which files on a machine compile is not something CI can hold. The *tests*
the change does carry are the two in the CLI - a platform header found without a flag, and `-I` still shadowing it.

### M77 - 2026-11-25 - the search that stopped one directory short

Reviewing `95cc4b0` found the feature broken for the machine it was written for, and **invisible on this one**.

`add` accepted a directory only if it held `stdio.h`, as a cheap "is this really a header directory" probe. The Windows
SDK keeps `windows.h` in `um` and `shared`, and neither has a `stdio.h` - so both were dropped:

```
ucrt   has stdio.h=true  windows.h=false     <- kept
shared has stdio.h=false windows.h=true      <- dropped
um     has stdio.h=false windows.h=true      <- dropped
```

On a machine with Visual Studio and no mingw, the search would have found the C runtime's headers and stopped one directory
short of `windows.h`, which is the header the whole change exists to find. Here mingw's own `windows.h` masked it, so every
test passed and the feature looked done.

The rule is `is_dir()` now: a search path is a directory, and the *order* decides preference - existence is all the probe
has to ask. The two SDK directories are in the list (`ucrt`, `shared`, `um`), and the test asserts the **relationship**
rather than a machine fact: every SDK directory that holds `windows.h` must be searched. Run against the first version it
fails with `shared holds headers and is not searched`, which is exactly the bug, on the machine that could not show it by
behaviour.

The lesson is the one M75 taught in a different coat: a probe that is *nearly* the right question passes on the machine that
happens to have another answer, and the passing test is what makes the gap invisible.

213 unit (one more), 48 exe, 51 parser, 12 real, 33 CLI; fmt and clippy clean.

### M76 - 2026-11-25 - the headers the machine has, and the flag nobody should have to pass

The census said 87 % of refusals stop at a missing header, so the next question was *which* headers. The answer was a
lesson in reading a measurement: `lean/lean.h` (259), `asn_application.h` (165), `lapacke_utils.h` (72), `ogs-core.h`
(38), `common.h`, `config.h` - **a project's own headers**, the `-I` its build system would supply. And then `unistd.h`
(10), `strings.h` (9), `fcntl.h` (5), `pthread.h` (4), `immintrin.h` (5) - headers this machine *has*, in the SDK that the
census never told the compiler about.

That last part was the real gap, and it is not about the corpus: **`xcc` had no way to find the platform's headers without
`-I`**. Every Windows C compiler finds `windows.h` for its user; this one made the user spell
`-I/c/msys64/ucrt64/include`, and the test harness had been doing exactly that for two years without anyone noticing it
meant the compiler could not.

`xcc::platform::platform_directories()` is now the one place that knows where a Windows C installation keeps its headers:
`INCLUDE` first, because a build system that sets it means it and that is MSVC's own convention; then mingw-w64's two
include directories (`ucrt64` first - it is the runtime this compiler links against); then Visual Studio's toolsets and the
Windows SDK's `ucrt`/`shared`/`um`, newest version first, by listing the directory rather than by spelling a version nobody
should have to spell. The command line puts them **after** the programmer's `-I`s, so `-I` still wins.

It stays out of the library's own path on purpose: the compiler reads nothing by itself, `Headers` wraps whatever
`Includes` the host builds, and this is knowledge the *host* applies. The C API can use it the same way.

Measured: with no flag at all, `#include <unistd.h>`, `<strings.h>` and `<windows.h>` now resolve, `GetCurrentProcessId()`
compiles, links and runs, and the missing-header census goes from 1 071 to 1 042 of 1 500 sampled - with `unistd.h`,
`strings.h`, `fcntl.h`, `pthread.h`, `memory.h` and `immintrin.h` gone from the list entirely. What is left is project
headers and other architectures' vector headers, which is the correct answer for a corpus of other people's build systems.

212 unit (one more), 48 exe, 51 parser, 12 real, **33 CLI** (two more); differential 417 ran, 0 mismatched; 445 corpus
programs agree; fmt and clippy clean.

### M75 - 2026-11-25 - the census that counted file names, and the frontier it found

The queue was re-measured after `typeof` and statement expressions - 52 275 C programs under `D:/Projects`, 2 000 sampled -
and the first result was wrong in an instructive way.

**A `.c` file is not a C program.** The top refusal was "expected a type specifier", 156 times, and every sample of it was a
program's *expected output*: the `acwj` tree keeps its compiler's test results in files named `out.inputNNN.c`, whose text
is `23 H Hello world`. Counting them made a language gap out of a directory naming convention. The corpus is other people's
programs, not other people's stdout - the filter is `tests` in the path, and the number moved from 156 to 39.

What the filtered census says:

```
365 accepted, 1 635 refused
1 426 refused before the parser - a header this machine does not have   (87 %)
  209 refused by the parser:  43 __attribute__, 39 "expected a type specifier",
                              15 not declared, 14 expected an expression, 3 inline assembly
```

**And the top item is mostly correct to refuse.** The attributes asked for are `ext_vector_type` (17), `no_sanitize` (16),
`mode` (16), `neon_vector_type` (15), `__vector_size__` (14), `__ms_struct__` (12), `constructor` (11): the vector ones
*change the type*, `mode` changes the width, `__ms_struct__` changes the layout, and a constructor adds code that must run.
Adding them to the list of attributes that may be dropped would be the exact lie the refusal exists to prevent. Roughly 55
of the asks - `no_sanitize`, `target_version`, `disable_tail_calls`, `device` - are hints that cannot change a program on
this target, and they are worth 2 % of the corpus.

So the honest answer to "what is next" is no longer a language feature. **87 % of the refusals are the header frontier**:
the compiler now gets past `typeof`, statement expressions, VLAs, atomics and intrinsics, and stops at a `windows.h` this
machine does not have.

### M74 - 2026-11-25 - the last statement is not the last expression statement

Reviewing the fixes in `421e753` against `clang` found one more divergence, and it is the kind a
single crafted program hides: the value of `({ ... })` is the block's **last statement**, and only if that statement is an
expression statement. This compiler took the last expression statement *anywhere*:

```c
int v = ({ int t = 1; t; int u = 2; });   clang: error, the block has no value
                                          here:  v is 1
```

The `1` is the value of a statement two statements back, which the block's own end never provided. The fix is a word:
`value = produced` where the code had `if let Some(produced) { value = Some(produced) }` - keeping is what let an earlier
statement answer, and assigning is what makes the last one decide.

The test holds both shapes now, the loop and the trailing declaration, and it was run against the previous rule: it fails
there with an `Artifact` where it expected a diagnostic.

**And the census that came before this review is worth recording**, because it changed the answer to "what is
next" - see M75.

211 unit, 48 exe, 51 parser (same count, two more cases), 12 real; fmt and clippy clean.

### M73 - 2026-11-25 - the operand that was evaluated, and the value that climbed out of a loop

Reviewing `0fbd226` found both halves of GNU's rule missing, each with a measurement that showed it.

**`typeof` evaluated its operand.** `sizeof` had the right shape three lines away - `unevaluated` set around the parse - and
`typeof` did not use it:

```
typeof(x = 99) z = 2;      x is 99 here        clang: x is 5
```

The fix is the same two lines, and the test is that program.

**A statement expression took its value from anywhere in the block, including inside a loop.** The first version let the
expression-statement branch write a field and read it afterwards, so a block whose last statement was a `for` answered with
the value of the assignment *inside* the loop:

```
error: internal: `main` is not valid: after the front end:
       v18 is defined in block 2 and used in block 4 which it does not dominate
```

The verifier caught it as an internal error rather than a wrong number, which is the whole reason it exists. `clang` refuses
that program - a block whose last statement is a `for` has no value - and so does this compiler now.

The fix is the shape M66 taught: **one fact per place, and the answer travels through the return path of the thing that
owns it**. `statement()` saves both the value and a `was_expression_statement` flag, clears them, and keeps the result only
if *this* statement produced one. A statement that contains others puts both fields back the way it found them, so a
child's value cannot answer for its parent.

211 unit, **48 exe** (one more), **51 parser** (one more), 12 real; fmt and clippy clean.

### M72 - 2026-11-25 - typeof and the block that has a value, measured before and after

Two GNU extensions, and the same measurement discipline the VLA round used: count first, then build, then count again
over the same two corpora of the 67 265 C files on this machine.

```
typeof-shaped files:      184 files, accepted   1 ->  46
statement-expression files: 424 files, accepted  72 -> 103
```

**`typeof` is three spellings of one question** - `typeof`, `__typeof__`, `__typeof` - and it takes a type name or an
expression. Telling those apart is the trick `sizeof (` already had: try the type first with `parenthesised_type`, which
puts the cursor back when there is none. It matches `clang` on `typeof(x)`, `typeof(&n)`, `typeof(int)`, `typeof(q)` for
a record, `typeof(c)` keeping a `const`, and `typeof(a[0])` of a variable-length array's element.

It was also the largest *specific* refusal left in the corpus: 84 files stopped at "expected a type specifier", and that
message came from this compiler meeting the word and not knowing it.

**A statement expression is a block whose value is its last expression statement**, and the parser already had every part
of it: the statements are the ordinary ones, read by the ordinary `statement()`. The one new fact is that an expression
statement's value is *kept* rather than discarded - one field, set where the statement is read, taken by
`({ ... })` - which is what keeps one statement grammar instead of two.

The test is the shape real code writes: `MAX`, `CLAMP` and the kernel's `container_of`, with a statement expression in a
loop and another nested inside one. `25 9 10 42 14 42`, the same as `clang`. A block that ends without an expression has
no value, and says so once - GNU treats it as `void`, and a program that uses it as a value has a mistake.

Two smaller things the round turned up: the build ran out of disk mid-gate (5.5 GB of `target/debug`, deleted), and the
`MAX` macro's own test produced the note that M70 added - `= note: in \`MAX\`, used at ty1.c:10:68` - which is the first
time a new feature's first error message arrived with its provenance attached.

211 unit, **47 exe** (one more), **50 parser** (one more), 12 real, 6 invalid; differential 417 ran, 0 mismatched; 445
corpus programs agree; fmt and clippy clean.

### M71 - 2026-11-25 - the scan that was narrower than the window it replaced

Reviewing `cb86b3b` found the search it widened had become, in one direction, *narrower* than the window it replaced.

The window tried `at - 1` **and** `at`. The scan that replaced it ran `0..at`, which is `at - 1` and everything before it,
and not `at`. A diagnostic raised *before* the offending token is consumed points at exactly that token:

```c
#define T (1 + )
int x = T;                       error: expected an expression, found `)`
                                 no note, where the two-token window had one
```

Measured against the commit before, on three programs: `d1` (four tokens back) gained its note, `d2` kept its note on
`at - 1`, and `d3` **lost** the one it had. One fix, one regression, and the regression was invisible to every test in the
suite - which is the shape a review is for.

The range is `0..=at` now: a superset of the window, and the scan a diagnostic needs. Two tests hold both directions - the
note four tokens back, and the note under the cursor - and each was run against the code without its half of the fix. The
second one fails there with `[]`, no notes at all.

A note on the measurement itself: the first attempt to compare against the previous commit used `git stash` on a **clean**
tree, which stashes nothing, so the "before" run was the "after" run. `git show <parent>:path` into a copy, then building
it, is the way; and the difference showed up only when the cases were counted per program instead of eyeballed.

211 unit, 46 exe, **49 parser** (one more), 12 real; fmt and clippy clean.

### M70 - 2026-11-25 - the note four tokens back, and the shape that was never tried

The item said "the note's limit is distance". Two commits had argued about it - M61 widened the search, found nothing,
and reverted; M62 narrowed it back to two tokens while fixing the *use-site* route. Both were reasoning from crafted
programs that happened to be wrong, so this time the answer came from a probe: report, for every diagnostic, the nearest
token carrying its span and whether that token came from a macro.

**The corpus cannot answer the question.** Over 1 200 of the 67 265 C files, the probe fired *zero* times: those files
stop in the preprocessor, at a header this machine does not have, long before a parser raises anything. A measurement whose
subject never runs is not a measurement.

**One shape does answer it**, and it took a minute to write once the question was "which diagnostics point at an *early*
body token":

```c
#define TERN (c ? p : q)
int r = TERN;

error: the arms of `?:` are pointer to int and int     <- caret on the `?`, four tokens back
```

Four is past the window of two, so the note was absent (`= note: in \`TERN\`, used at d1.c:6:11`). Every case M61 tried had
a span that was a *use site* or an operator outside the macro, so the window looked adequate; none of them pointed into the
middle of a body.

The search is a scan back from the cursor now, stopping at the first token with that span, with the cost named where it
lives - the distance, and a diagnostic is not raised in a loop. The test is that program, and it was checked against the
code without the change: no note there, this note here.

211 unit, 46 exe, **48 parser** (one more), 12 real; differential 417 ran, 0 mismatched; fmt and clippy clean.

### M69 - 2026-11-25 - sizeof of an array whose size is a number, and the rounding nobody did

`sizeof a` for a variable-length array answers with the number the declaration computed, which is what
C99 6.5.3.4p2 asks for: the size is the one *at the declaration*, so it is stored in a slot beside the pointer and
`sizeof` loads it. A later change to the count does not move it - the new test sets `n = 1` after the declaration and
still reads `20`.

**And it found a bug that shipped in M65.** The first thing the new test did was crash:

```
Thread 1 received signal SIGSEGV
0x00007ffe0bccaa97 in ucrtbase!fputs:   movdqa %xmm0,0x370(%rbp)
```

`movdqa` needs sixteen-byte alignment, so `rsp` was not aligned at the call. The VLA declaration passed
`n * sizeof(int)` - twenty bytes for five ints - straight to `emit_stack_alloc`, under a comment that said *"the encoder
rounds it to sixteen"*. **The encoder does not round; the parser's `alloca` did**, and the two callers disagreed. Every
earlier VLA test had its array in a function that called nothing, and every earlier `alloca` test had a constant size
that the other path had already rounded.

The rounding moved to `emit_stack_alloc`, which is the one place both callers go through, and `alloca` lost its own copy.
A comment describing a check the code does not make is worse than no comment, and here it cost a fault inside the C
runtime rather than a wrong number - the shape a test finds only if it puts the two things in one function.

211 unit, **46 exe** (two more), 47 parser, 12 real; fmt and clippy clean.

### M68 - 2026-11-25 - the header that shadowed the SDK, and the spelling asked on every expression

Reviewing `b811ce8` found one regression I had introduced and one piece of work done in the wrong place.

**`malloc.h` shadowed the SDK's.** The commit shipped `malloc.h` to carry `_alloca` - and a shipped header is found
*before* the SDK's, so a program that includes `<malloc.h>` for `_aligned_malloc` lost it:

```
error: `_aligned_malloc` is not declared; it is taken to be a function returning `int`
```

The first fix was to delete the header, since `_alloca` needs none - the parser answers the name the way GCC does. That
made a common MSVC idiom fail outright (`cannot find malloc.h`), which is worse. The right answer is the one the export
table supplies: **ship the header, and declare what the UCRT exports under it** - `_aligned_malloc`, `_aligned_realloc`,
`_aligned_free`, `_aligned_msize`, `_heapmin`. A shadowing header has to be at least as good as what it shadows, and the
names were in `ucrtbase`'s capture all along. `_aligned_malloc` and `_aligned_free` now link and run (`11 22`), which is
the test the header did not have.

**The four names were compared by allocating a string.** The builtin check ran on *every* primary expression and did
`self.spell(&token).to_owned()` before looking, which is one allocation per literal in every program. It is a `matches!`
over a borrowed spelling now, and only for a token that could be one - the same answer, nothing allocated.

211 unit, 45 exe, 47 parser, 12 real; fmt and clippy clean.

### M67 - 2026-11-25 - the measured win, and the four names a stack allocation answers to

The measurement was owed, so it came first: a fresh census over the **67 265 C files under `D:/Projects`** - the corpus the
old 705 was counted on - and then a **before and after** rather than a grep. Of a random 400 of the 3 862 files whose text
holds a declaration-shaped `name[identifier]`:

```
old -> new
 ('refused','refused')  347
 ('accepted','accepted')  43
 ('refused','accepted')   9     <- all nine: "expected a constant expression"
 ('accepted','refused')    0
```

Nine files, nothing lost, and every one of the nine failed with the VLA refusal. The remaining 347 stop earlier and 298 of
those stop at a header this machine does not have - which is what a corpus of other people's build systems looks like, not
a language gap. The old census counted 705 files that *mention* a VLA-shaped declarator; that number was never the number
of files that needed one, and now the difference is measured rather than assumed.

**Then the names, because the feature was reachable only through a name nobody writes.** `__xcc_alloca` was this
compiler's own spelling; `alloca`, `_alloca` and `__builtin_alloca` are the three a program writes. All four now mean the
same operation, recognised in the parser *before* any lookup - which is what GCC does, and for a reason the census
supplied: seven files call `alloca` having declared nothing, and the implicit `int` they got truncated a stack pointer to
its low 32 bits. `<alloca.h>` (POSIX) and `<malloc.h>` (Microsoft) ship as **bridge** headers mapping the names over the
operation, because neither is a C99 header and the ledger of shipped headers is C99's table.

Corpus effect: of the 114 files that call `alloca(`, 18 compiled and now 24 do; the rest stop at missing headers or at
attributes this compiler refuses by name.

211 unit, **45 exe** (one more), 47 parser, 12 real; fmt and clippy clean.

### M66 - 2026-11-25 - the allocation that went to the next declaration, and the name that outlived its function

Reviewing the two commits that landed VLAs (M64, M65) found both of its bookkeeping decisions wrong, and one of them
produced a wrong message for an innocent declaration.

**The allocation went to the next declaration.** The count and element type came back from the declarator in a parser
field, and the declaration that read it was whichever came next:

```
int main(void) { int n = 4; typedef int Row[n]; int later = 7; }
                error: cannot assign pointer to void to int      <- on `later`
```

The typedef itself was refused - a variable-length array cannot be a typedef - but the field was never taken, so the
declaration after it was allocated bytes it never asked for. A declarator's answer is its **return value** now, and the
field is gone; a variable-length array that is not an automatic object says so where it was written.

**The name outlived its function.** `sizeof a` was refused by asking a list of names nothing pruned:

```
int f(int n) { int a[n]; ... }
int g(void) { int a[3]; return (int)sizeof a; }   error: `sizeof` a variable-length array
```

A sentence about the wrong function, for an array that is twelve bytes. The fact lives in the **symbol entry** now, which
is the thing that is already scoped - one field, sixteen literal sites, and no list to forget.

Two smaller things the same review turned up:

- The Cranelift refusal for a stack allocation carried a run of twenty spaces, from a Rust string continuation that had
  swallowed the following line's indentation. A diagnostic is read by a person; that is what a test asserting on message
  *text* is for, and none did.
- The refusal only appears below `--opt-level=2`, which is now said in the message rather than implied.

211 unit, **47 parser** (two more), 44 exe, 12 real; 445 corpus programs agree; differential 417 ran, 0 mismatched; fmt
and clippy clean.

### M65 - 2026-11-25 - the array whose length is a number, and the two things that ate it

C99 6.7.3.2 lands: `int a[n]` and `int a[n][3]`, compiled, linked and run, agreeing with `clang` on
`30 285 0 18`. It cost two silent failures, both worth more than the feature.

**The counter that was not the list.** A bound is a constant expression first - that is what makes an array's *type* -
and only when that fails is it a value. So the constant reading is attempted, and its complaint withdrawn if the bound
turns out to be a value. Withdrawing the *diagnostic* was not withdrawing the *error*, and `error_count()` reads a
counter, so every function that declared a variable-length array was thrown away by the emitter's own rule - silently,
because the message that would have explained it had been withdrawn a moment earlier. A function that vanishes without a
diagnostic is the worst kind of bug this compiler can have, and one line (`self.errors = errors_before`) is the whole fix.

**The cast that folds only when nothing is emitted.** A bound like `char b[((int)(16 * sizeof(void *) * sizeof(double)))]`
- how Lua sizes a buffer - folds in an unevaluated reading and does not in an evaluated one. Reading the bound *once*,
evaluated, therefore turned a constant array into a variable-length one, and `sizeof b` then refused with a message about
a feature nobody had asked for. So the constant reading comes first, for the reason it is a constant reading, and the
evaluated one only when it has to.

Three smaller decisions:

- The name is a **pointer to the element**, and the bytes come off the stack where the declaration stands
  (`Inst::StackAlloc`); the count multiplies the element size and the encoder rounds to sixteen. `int a[rows][3]` works
  because its element is an array of a constant size, which is still a type.
- **`sizeof` a VLA is refused by name**, with a message that says the size is not in the name. It could return the
  computed size, and that is a later change; what it must not do is answer eight.
- A **parameter's** bound is adjusted to a pointer and never evaluated (C99 6.7.6.2p1, 6.7.6.3p7). Before this, `void
  f(int n, int a[n])` reported "an array's size at file scope must be a constant expression" - a sentence about the wrong
  scope, for a construct where nothing needs computing at all.

211 unit, 44 exe (two more), 45 parser, 12 real; 445 corpus programs agree; differential 417 ran, 0 mismatched; fmt and
clippy clean.

### M64 - 2026-11-25 - the frame that had to move, and the home area that did not

VLAs start here, and the first thing they need is not in the parser. `encode.rs` opens with the sentence this work had to
argue with: *"The frame is a byte offset from `rsp`, and `rsp` never moves after the prologue"*. A variable-length array
moves it, so the frame needs a register that does not.

The frame pointer costs two instructions and one unwind code, and **no offset changes**: the prologue does `sub rsp,
frame` and then `mov rbp, rsp`, so `rbp` and `rsp` name the same byte and every `[rsp+off]` in this file is already
`[rbp+off]`. What changes is that `rsp` may move afterwards. The unwinder is told so (`UNWIND_INFO`'s `FrameRegister`,
`rbp`, offset zero) because a stack walk from inside an allocation cannot add back a fixed amount.

Which function sets one is decided by a single flag: a `cir::Inst::StackAlloc` anywhere in the body, which is
`Op::StackAlloc`, which lowers to `mov r11, n; sub rsp, r11; mov dst, rsp`. The size arrives already rounded to sixteen -
the front end is the only place that knows the element size, and every later call needs `rsp` aligned.

**The bug that made it worth doing carefully.** With `__xcc_alloca` reachable, three of four numbers were right:

```
[0]=0 [1]=3 [2]=6 [39]=24        63 117 1 24
```

Win64 has a callee write the four register arguments into the caller's *home area*, and it finds that area at its own
`rsp` less the return address - the caller's **live** `rsp`. With no frame pointer that is the same byte the frame
reserved, which is why this compiler could treat "home area" as a fixed frame offset for eight versions. With a VLA it is
not: `printf` wrote its arguments into the allocation. A non-variadic call was unaffected, which is what narrowed it. The
call sequence now moves `rsp` down by the reserved amount when the frame has a pointer, places the arguments at the live
`rsp`, and puts it back - and the store has to know which register to measure from, so `mov_memory_from_reg` grew a
`_to(base, ...)` sibling.

Measured: `117 0 3` and `63 117 3`, the same as a `clang` build of the `alloca` equivalent; 211 unit, **43 exe** (one
more), 45 parser, 12 real; 445 corpus programs agree; fmt and clippy clean.

`__xcc_alloca` is the builtin that made this testable before a single line of VLA grammar existed. The array syntax is the
next step, and it is parser work: a bound that is not a constant becomes this operation with the element size applied.

### M63 - 2026-11-25 - three red flags in my own last commit, and the test that proves nothing about the search

Reviewing `3940deb` found three things, all mine.

**Two comments that disagreed.** The route added beside the old one left the old route's paragraph in place, so the code
carried "two candidates is enough" immediately above "there are two routes". One `if` with two arms, one comment.

**Two notions of which file.** `push` already holds `file` - the file the cursor is reading - and the new route computed
its own from the token before the cursor. One question, two answers.

**And a search that depends on an order nothing guarantees.** `site_at` was a binary search by offset, justified in its own
comment by "sites are pushed in expansion order and therefore in source order". That is true *within* a file and false
across them: every file's offsets start at zero, so a header's sites can follow the main file's at smaller offsets. It is a
**scan** from the newest site back now, and the comment says why the order cannot be leaned on.

That last one taught the lesson of the commit before it, in the other direction. Two attempts were made to build a program
that breaks the search - a header included after a main-file expansion, the diagnosed use inside the header - and both
passed. So the new test, which does exactly that, is **not** evidence about the search; it is evidence that a header's site
is not confused with the main file's, which is what it now says it is. A test that cannot fail the way its name suggests is
a test that will be trusted for the wrong reason, and the honest label costs one sentence.

45 parser tests (one more), 211 unit, 42 exe, 12 real; differential 417 ran, 0 mismatched; fmt and clippy clean.

### M62 - 2026-11-25 - the caret on the wrong parenthesis, and the note that follows from it

M61 stopped with one measurement to make: dump the diagnostic's span beside the site's span, with the source text under
each, so my arithmetic stops being part of the answer. It took one run and it settled two things at once.

```
diagnostic span (54,1) text="("        - the call's parenthesis
site       span (55,3) text="ARG"       - correct
```

So the note was missing for a reason nobody had looked for: **"argument 1 is pointer to char, expected int" pointed at the
call's opening parenthesis**, not at argument 1. The span was the whole call's, and the parenthesised start of a call is
`(`. Two of my three earlier readings of that caret column had been wrong - which is exactly why the probe prints text now
and I do not count characters.

The fix has three parts, and the third is why the note needed a second route at all.

**The argument's span.** `call` now records the `program_span()` of each argument - the span of what the *program* wrote -
and the type diagnostic uses it. `program_span` is the site's span when the token came from a macro, and the token's own
otherwise, so it answers "where is this in the source the reader has" for both.

**The second route to a site.** A token from a macro *body* carries `expansion`, which is how the note has always worked.
A span that *is* the invocation matches no token, because the invocation is replaced by its expansion before the parser
sees it - so `site_at` searches the sites by span, which are pushed in expansion order and therefore in source order.
This is what M61 could not state clearly enough to implement; with the span now correct, containment became equality and
the rule is one sentence.

**Two notes from one site.** A body token needs the use's location, because the caret is under the definition. A span that
*is* the use would repeat the caret's own file:line, so that route says only `in \`ARG\``.

```
3 | void g(void) { f(ARG + 1 + 1 + 1); }
  |                  ^^^
= note: in `ARG`
```

The body route is unchanged and still green (`= note: in \`W\`, used at t.c:2:25`).

211 unit, **44 parser** (one more), 42 exe, 12 real, 6 invalid; differential 417 ran, 0 mismatched; 445 corpus programs
agree; fmt and clippy clean.

### M61 - 2026-11-25 - the distance that was not the reason

The next item was the note-distance fix from M52, whose conclusion was that the macro-site lookup tries only two tokens
before the cursor and a diagnostic reported further away gets no note. It was built, measured, and **reverted**: the
distance was not the reason.

The build was the obvious one - scan back from the cursor instead of looking at two tokens - and it changed nothing on any
of nine crafted programs in which a macro's expansion sits three or more tokens before the complaint. Every one of them
printed the same diagnostic with the same missing note, before and after.

What the instrumented run showed: those diagnostics do not point at a token *from a macro body* at all. They point at the
**use**, and a use is not a token. `f(ARG + 1 + 1 + 1)` complains that "argument 1 is pointer to char, expected int" with
a caret under `ARG` - and `ARG` was replaced by its expansion before the parser ever saw it, so no token in the stream has
that span, at any distance. The note's first route reads `token.expansion`, which is why it works for a body token
(`#define W(X) (X + undeclared_thing)` and its note `in \`W\`, used at t.c:2:25`, measured green) and cannot work here.

So the fix needs a **second route**: a span that names a macro use matches no token but does match a `Site`. `Site` already
carries the invocation's span, and sites are pushed in source order, so it is a binary search. That was written, and then
the measurement stopped being clean: the argument's span came back as length one, and the site's span did not contain it,
in a way that survived two rounds of my own arithmetic and no round of certainty. A note is worth having; a note attributed
by a rule I cannot yet state is worse than none, which is the lesson M52 already taught.

Reverted to `0fa6790`'s code. What stands is the correction: **the note's limit is not distance, it is that a use is not a
token**, and the next attempt starts by dumping the diagnostic's span beside the site's span for one case, not by widening
a search.

### M60 - 2026-11-25 - tre becomes the tenth fixture, and the isascii that was a macro

`tests/fixtures/tre/` is thirteen of tre's units unmodified, with a driver that compiles one extended expression and
matches it once with three groups: `2 8 2 5 5 8`, the same as a `clang -O0` build. It is here because it refused to
compile until a pointer converted to a **narrower integer** became one of the conversion table's cases (M42), and it is
the densest preprocessor code in the suite.

Two things had to be got right before it was a fixture at all.

**`-DUSE_LOCAL_TRE_H` is not optional.** Without it `regex.h` includes `<tre/tre.h>` - and **msys2 ships tre**, so the
first build of this fixture compiled the vendored sources against an *installed* header and printed the right answer
anyway. A fixture whose oracle comes from a different library than the one being compiled is the "measurement without an
oracle" failure in a new coat: the numbers agreed because both sides had been told the same thing by someone else. With
the switch, the vendored `tre.h` is used and its Windows branch reads `basetsd.h` - so this fixture needs the SDK, like
mimalloc's and unlike xxhash's.

**`isascii` does not exist as a function in the UCRT.** The link failed with `undefined symbol isascii`, and the
measurement that mattered was one `grep`: `ucrtbase` exports `__isascii` and `__toascii`, and MSVC's own `ctype.h` spells
`isascii` as a macro over the underscored name. So does this one now - the same shape as `_snprintf` versus `snprintf`,
which this project already documented. The first attempt declared a *function* `isascii`, which linked no better than not
declaring it, and the guard written for exactly this - *these names are in no list* - caught the two new declarations by
name within one test run.

Two `real` tests now need the SDK and one needs none; `tre` is the tenth fixture.

211 unit, 43 parser, 42 exe, **12 real** (one more), 31 CLI; fmt and clippy clean.

### M59 - 2026-11-25 - the fixture that fails when the fix is undone

Reviewing `622388f` asked one question - does this test earn its place? - and answered it with a mutation rather than an
opinion: `_umul128` was removed from the intrinsic table and the fixture run.

```
left:  "ed714233c5a9a792 b4b513e2cee8b646 66716377"
right: "ed714233c5a9a792 e4541a9cacf545aa 66716377"
```

The *wrong answer from M54*, exactly - not a compile error, not a skip, the number that made this work necessary. The table
was restored from git and the suite is green.

That is the check a fixture is supposed to pass and most do not: it detects **the specific regression it was written for**.
SQLite's fixture detects "sqlite does not compile"; this one detects "an MSVC intrinsic is an implicit `int` again", which
is a narrower and less obvious failure - and the only symptom in the wild was a hash that differed.

Two things confirmed in the same pass, both worth having written down because they are properties a reader would otherwise
have to assume:

- **It cannot silently skip.** No `cfg`, no `windows_sdk()`, no host linker: `run_fixture_with` uses this compiler's own
  linker end to end, and the expected hashes are inline rather than asked of `clang` at run time. The SDK-free check at
  M40 is what makes this the one new fixture CI can see.
- **It costs 1.6 seconds**, and `real` is now 11 tests in about 105 seconds.

### M58 - 2026-11-25 - xxhash becomes the ninth fixture, and the one CI can see

The win is pinned. `tests/fixtures/xxhash/` is `xxhash.c` and `xxhash.h` unmodified, with a driver that prints three
hashes over one string, and one test that compares them with what `clang -O0`'s build of the same files prints:

```
ed714233c5a9a792 e4541a9cacf545aa 66716377
```

**It is the only one of the four new fixtures that needs no SDK** - SQLite, Lua and mimalloc all reach for `windows.h`
somewhere, so their run halves skip on CI - and that is a difference worth stating plainly: this one *runs* there. What it
checks is exactly the thread that produced it: `_umul128` called with no header, the intrinsic table that answers it, and
the bridge header beside it. Two of the three hashes were right before that work began, which is what makes the test a
good one: a regression shows up as a disagreement in the third, the one that multiplies.

`-DXXH_VECTOR=0` is the library's own switch for its portable scalar path, and it is the same trade `miniz`'s
`MINIZ_NO_STDIO` was: this compiler shadows `emmintrin.h` with an empty bridge header on purpose (M12), so `__m128i` is
not something it can spell.

Eight fixtures were the claim at `v0.1.6`; this is the ninth, and two of them - this one and SQLite - found *wrong
answers* rather than missing features.

211 unit, 43 parser, 42 exe, **11 real** (one more); fmt and clippy clean.

### M57 - 2026-11-25 - the two lists, and the const that was not

Reviewing `57f1185` asked the question its own commit message had to argue away: the intrinsic table and
`include/x86intrin.h` are **two lists of the same fifteen names**, and the argument for keeping both was "both are
transcribed from mingw's `psdk_inc/intrin-impl.h`". That is a claim about care, not a check.

**So the check exists now.** A test walks the table and the header in both directions: every name the parser knows is a
name the header declares, and every function the header declares is a name the parser knows. A name in one and not the
other is a program that compiles differently depending on whether it includes a header, which is the exact complaint that
started this line of work.

**And it found a signature that differed**: the header declares `void __movsb(unsigned char *, const unsigned char *,
unsigned long long)` - the source is `const` - and the table said `unsigned char *`. A bare call passing a string literal
compiled anyway (this compiler lets an argument drop `const` with no diagnostic), so the difference was invisible; the
*types* were still two answers to one question. The table is const-correct now, and `spelled_type` - the small vocabulary
the table is written in - understands `const`, which it did not.

Three times this session the same shape has appeared: two lists, one fact. Twice the answer was a test that compares them;
once it was deletion. This is the first of those that found a real disagreement on the day it was written.

211 unit (one more), 43 parser, 42 exe; fmt and clippy clean.

### M56 - 2026-11-25 - the intrinsics are known names, and xxhash agrees with clang

The gap M54 and M55 both pointed at, closed. MSVC *builds its intrinsics into the compiler*: a program calls `_umul128`
with no header, and MSVC knows it. This compiler took it as an implicit `int`, which is not "unchecked" but **wrong** -
the high half of a 64x64-to-128 product is lost - and `xxhash`'s XXH3 came out as `b4b513e2cee8b646` where clang's build
says `e4541a9cacf545aa`. Two of its three hashes agreed, which is what made it a good witness: the one that was wrong was
the one that multiplies.

`INTRINSICS` is a table of `(name, result, parameters)` in the parser, in the vocabulary a header writes types with
(`unsigned long long *`), consulted in the one place an undeclared name becomes a function. A name in the table gets the
signature MSVC gives it and **no warning**, because nothing is wrong; a name that is not gets the implicit `int` and the
warning it always had.

```
$ xcc -DXXH_VECTOR=0 xxd.c xxhash.c -o xx.exe && ./xx.exe
ed714233c5a9a792 e4541a9cacf545aa 66716377
$ clang ...
ed714233c5a9a792 e4541a9cacf545aa 66716377
```

Three hashes, identical, and the "not declared" warning gone.

**The table is the same fifteen names `include/x86intrin.h` declares**, and that duplication is deliberate rather than
leakage: the header serves a program that *includes* it, the table serves a program that does not, and MSVC has both - a
header for the documented list and built-in knowledge for the ones a program calls bare. What keeps them from disagreeing
is the *spellings*: both are transcribed from mingw's own `psdk_inc/intrin-impl.h`, and the table's parameter strings are
the same C types.

The test is the bare call - **no header at all** - because that is the shape that was broken, and the header path was
already covered.

210 unit, 43 parser, 42 exe, 10 real, 31 CLI; fmt and clippy clean.

### M55 - 2026-11-25 - the bridge header that had no test, and the measurement that proved it anyway

Reviewing `33d0aab` found the `intrin.h` bridge **without a test that would fail without it** - the M51/M52 disease for
the third time in an hour. So the test was written, and then the bridge was removed to prove the test fails: it does.

```
without the bridge:  error: cannot find `intrin.h`
with it:             0 1        (2^32 x 2^32 = 2^64: low half zero, high half one)
```

That is a *narrower* proof than it looks, and the narrowing is worth recording: the test harness serves no SDK, so the
comparison is "the bridge answers `<intrin.h>`" against "nothing does" - not "ours against the SDK's". On a command line
with `-I` on the SDK the difference matters, and there the SDK's `intrin.h` declares nothing this compiler can use either
(its declarations sit behind a compiler identification we do not claim), so the bridge is a strict improvement rather
than a replacement for something that worked.

Two more things from the same pass:

- **The bridge is safe for the SDK's own users, measured**: the four real fixtures pass (30 minutes of them), and
  `mimalloc`'s `windows.h` path still compiles - the only refusal it produced was the linker-owned `_Thread_local`, which
  is what `-c` does to a TLS object.
- **`xxhash` still cannot use it, and that is the point**: its `#include <intrin.h>` is inside
  `#if defined(__GNUC__) || defined(__clang__)`, so it never includes the header at all and `_umul128` stays an implicit
  `int`. The bridge was never going to fix it - M54 says so - and the measurement now agrees with the note.

The root cause from M54 stands unchanged: **this compiler claims `_M_X64` and not `_MSC_VER`, and MSVC's intrinsics are
built into MSVC** rather than declared by a header. A table in the parser is the fix.

210 unit, 43 parser, 42 exe (one more), 10 real; fmt and clippy clean.

### M54 - 2026-11-25 - the assert that was a call, and the multiply that lost its high half

`xxhash` was the program, and it produced one fix, one real miscompile, and one root cause worth more than either.

**`_Static_assert` at block scope was a call to a function.** C11 6.7.10's assertion belongs to the *declaration*
grammar, and `declaration()` answers it before any specifier is read - but the statement grammar decides "is this a
declaration" with `starts_specifier()`, which asks `SPECIFIERS`, and `_Static_assert` is not a specifier. So a
block-scope assertion fell through to the expression grammar, parsed as a call to an implicit function, and the program
failed at **link** with `undefined symbol `_Static_assert``. `xxhash` puts one inside a function. One test, one line, and
the fix is in the predicate that answers the question. Its caret was also wrong - the cursor is on `}` by the time the
string has been read - so it points at the assertion now.

**And a silent wrong number**: with the assert fixed, `xxhash` links and runs, and XXH3_64 disagrees with clang's build -
`b4b513e2cee8b646` against `e4541a9cacf545aa`. XXH64 and XXH32 agree, which is what makes it a good test: two of the three
hashes were right, and the one that was wrong is the one that multiplies 64x64 into 128.

**The cause is a fact about what this compiler claims to be.** `xcc` defines `_M_X64` and `_M_AMD64` - the Windows SDK
picks `CONTEXT` by one of them - and it does *not* define `_MSC_VER`. `xxhash` branches on `_M_X64` alone, takes MSVC's
path, and calls `_umul128` **without a declaration**, because that is what MSVC allows: its intrinsics are built into the
compiler, not declared by a header. Here an undeclared call is an implicit `int` - a warning, by a decision recorded in
M11 for MSVC compatibility - and the high half of the product is gone.

So the fix is neither a header nor `_MSC_VER` (which would switch every MSVC-only branch in every SDK header on). It is the
piece of MSVC this compiler has not copied: **the intrinsics are known names**. `_umul128`, `_mul128`, `__readgsqword` and
the rest are already in the runtime and already have signatures transcribed in `x86intrin.h`; what is missing is a table
in the *parser* - when an undeclared name is one of them, synthesise the declaration instead of an implicit `int`, and say
nothing, because nothing is wrong. That is the next piece of work, and it is the last of the MSVC-shaped gaps this line of
fixtures has found.

**Also landed**: a bridge `intrin.h` that includes `x86intrin.h`, so a program that writes `#include <intrin.h>` - the
spelling a program actually uses - gets the fifteen declarations. It does not fix `xxhash`, which never includes it, and
that is exactly what pointed at the real cause.

210 unit, 43 parser (one more), 41 exe, 10 real; fmt and clippy clean.

### M53 - 2026-11-25 - xxhash, the alignment that belonged to the frame, and the assert that reached the linker

Two real programs tried; one gap fixed; one bug found; one negative.

**`_Alignas` above sixteen is the frame's limit, not the data section's.** `xxhash` aligns its accumulators to 64 at file
scope and was refused with *"this compiler supports 16 and below - what the ABI guarantees"*. That sentence is true about
the *stack*: the prologue aligns the frame to sixteen, which is the ABI's number, so an automatic object asking for more
is a promise this compiler cannot keep. It says nothing about the data section, where the object writer places an object
at an offset - sixty-four is an offset like any other. The check now asks **where** the object lives: file scope is
allowed up to a page, an automatic one keeps the sixteen and the message says why plus what to do about it (*"give the
object static storage duration"*).

`_Alignas(16)` and below were already tested through the layout matrix; the new test is the boundary itself, in both
directions, because the rule now depends on storage duration and a rule with two arms needs two cases.

**And a bug that is not about alignment at all**: with the alignment fixed, `xxhash` compiles, links - and fails at link
with `undefined symbol `_Static_assert``. A `_Static_assert` is being read as an *expression that calls a function* in some
context, so the compiler emits a call to a name no library has. That is one of the two shapes this project treats as a
bug by definition (a wrong program, and a link error is the lucky version of it); it needs the source position, which is
the next session's first ten minutes.

**The negative**: `xxhash`'s vector paths are `__m128i` and `__m256i`, and this compiler shadows `emmintrin.h` with an
empty bridge header on purpose (M12). With `-DXXH_VECTOR=0` - the library's own switch for the portable scalar path - it
compiles to a 102 KB object, and a driver against it prints what clang's build prints:
`ed714233c5a9a792 e4541a9cacf545aa 66716377`. So xxhash is measurable on the scalar path only, which is the same trade
miniz's `MINIZ_NO_STDIO` was.

210 unit, 43 parser (one more), 41 exe, 10 real; fmt and clippy clean.

### M52 - 2026-11-25 - the widening that could never have fired

Reviewing `90055fe` (docs only - the revert was complete) turned the note's "could not be proved" into something better:
the change was **provably a no-op**, and the real limit is a different one.

`Parser::span()` answers with *a token's* span - `self.peek().map_or_else(|| last token, ...)` - and every diagnostic in
this parser passes a span from there. Tokens do not nest: each one's span is its own bytes. So "the expanded token's span
contains the diagnostic's span" holds **only when the two are equal**. Containment and equality are the same predicate
here, which is why the case I was hunting could not be constructed: there is no such case.

That does not mean the note is complete. The lookup tries **two** positions - the token just consumed and the one under
the cursor - and a diagnostic can be reported more than two tokens after the expansion it is about. *That* is the reachable
limit, and it is a different fix: remember which token carried the span, or search further back, rather than widen a
predicate that has nothing to widen.

Written down because the next session would otherwise spend its hour on the same dead end I did: containment is not the
lever, **distance** is. The test to build is a macro expanded three tokens before the diagnostic's span, and the existing
note test (`in `W`, used at t.c:2:25`) already holds the one-token case.

### M51 - 2026-11-25 - the note that could not be proved, and why it was not made wider

The note-vs-location work (M34) has a measured limit: `&((char*)0)[X]` reports with the caret on the `&`, and the site
lookup asks whether the *just-consumed or current token's span equals the diagnostic's span* - so no note. The obvious fix
is to look for an expanded token whose span the caret is **inside**, and I wrote it. Then I tried to prove it, and could
not.

The cases I could construct produce no diagnostic at all: `int a[2] = { ZERO, g };` is a legal file-scope initialiser
(`g`'s address is a constant), and `char a[WIDTH - 5]` with `WIDTH 4` is refused with a span that is not inside the
expansion. The shape that fails is sqlite's: a macro expanding *inside* an expression the caret covers only partly. I have
that measured in M34 and no reproduction of it in a test.

So the change is reverted, and the reason is the one this project keeps relearning: **a rule widened without a failing
case is a rule nobody can keep honest.** The containment rule is *probably* right - a caret inside a token that came from
a macro is that macro's business - and "probably" is what the next session should replace with "measured", by taking
sqlite's own failing line (M34 records it) and reducing it, rather than by inventing a shape that fires nowhere.

The existing note test still holds the current rule: `in `W`, used at t.c:2:25`, for a diagnostic whose caret *is* the
expanded token.

210 unit, 42 parser, 41 exe; fmt and clippy clean.

### M50 - 2026-11-25 - a real crash, resolved with the map and nothing else

The line-number work was built for one purpose, stated in M16 as a complaint: `sqlite3_open` faulted at
`.text+0x129e2d` and naming that function took `objdump`, a symbol table and arithmetic. So the acceptance test is a
crash, and here it is.

```c
static int deref(int *p) {
  return *p;          /* line 4: the fault */
}
```

```
$ xcc --std=c11 --opt-level=2 --map crash.c -o crash.exe > crash.map
$ gdb -batch -ex run ./crash.exe
0x000000014000f743 in deref ()

$ grep 000000014000f740 crash.map
000000014000f740 crash.c:3
000000014000f743 crash.c:4      <- the faulting instruction
```

**`crash.c:4`** is `return *p;` - from the map alone: no objdump, no symbol table lookup, no subtraction. The two tools
are complementary and both are now cheap: gdb says *which function* (`deref`), the map says *which line*, and neither
needed the other to be useful.

And the file field earned its place in the same output:

```
0000000140003d16 math.h:27
000000014000f48d time.h:53
000000014000f740 crash.c:3
```

Two of those are code **from headers** - the runtime's own `inline` functions - attributed to `math.h` and `time.h` rather
than to `xcc-crt/main.c`. Without `Span::file` they would have been reported as lines of whichever file happened to own
offset 27, which is the lie M46 was written to prevent.

### M49 - 2026-11-25 - the map that answered a question nobody asked

Reviewing `fd33300` found the map **unusable for its purpose**, and two smaller things around it.

**The addresses were offsets from `.text`, not the addresses a crash prints.** A crash says `0x14012b05d` and a debugger
shows the same number; the map printed `0000e740`. So the one thing the map exists for - looking up a crash address -
needed the image base and the section's RVA from somewhere else, which is exactly the arithmetic (objdump plus
subtraction) the whole line of work set out to remove. `Linked` now carries `text_address`, the artifact carries it too,
and the map prints `0000000140001007` - the loaded number.

**`-c --map` was accepted and did nothing.** A flag that is read and dropped is the mistake this compiler refuses
everywhere else in its input, and it was in its own command line. Refused by name, with the reason: an *object* carries
its map in its `.xccmap` section; the flag is about a link, where offsets become addresses. `--run`, `-E` and `--explain`
refuse it too - they make no program.

**And the map was printed before the image was written**, so a failed write left a map of a file that did not exist. It
is printed after now, inside the `Ok` arm.

The test grew both halves: an address that starts with the image base, and the `-c --map` refusal (exit 2, naming the
section).

210 unit, 41 exe, 10 real, 31 CLI; fmt and clippy clean.

### M48 - 2026-11-25 - the map reaches the command line, and a crash address has a line

Steps 3 and 4 of M44/M46, and the last piece of the line-number work started in M44.

**Where the file numbering lives**: in the compilation, not the linker. A unit's spans index *that unit's* file table,
so `Compile::link` rebases each unit's line records by the number of files before it as it builds the program's table -
which is the trap M47 recorded, closed in the one place that has both facts. The linker learned only about *offsets*:
`Linked` carries `(address, span)` shifted by where each unit's code landed, which is arithmetic it already did.

**Where it is rendered**: `--map` on the command line, because that is where the sources are - a span is an offset and
only `SourceFile` knows where its lines begin. The artifact carries both the records and the file table, so the flag is
a printer and nothing else.

```
$ xcc --std=c11 --opt-level=2 --map main.c -o main.exe | grep -E 'h[.]h:|main[.]c:'
0000e740 h.h:1        <- `twice`, inlined from the header
0000e755 main.c:2     <- the call site
```

The first entries of any map are the *runtime's*: `xcc-crt/main.c:45`, `:48`, `:52` - because the C runtime is compiled
into every link, and a map that hid it would be a map that lies about where the program starts.

That is the whole line: a symbol table named a fault (`sqlite3InsertBuiltinFuncs`), a span learned which file it was in,
the object carries the records as deltas, and the command line turns them into `file.c:42`. What it cost, measured: 28 %
of `.text` in the object (M47), and nothing at all at run time.

### M47 - 2026-11-25 - the regression that was my own background jobs, and the trap waiting at step four

Reviewing `ae18a33` started with a scare and ended with a warning.

**The scare**: `-c sqlite3.c` measured **28 and 60 seconds** where the baseline is 12. I nearly went looking for a
regression in the line table. Three runs on a quiet machine: 10.8, 12.4, 11.1 - the baseline. The two outliers were the
test suites I had left running in the background plus a virus scanner reading a 3.3 MB object. That is the *third* time
this session a timing has lied (2194 s and 1518 s before it), and the lesson is now a rule: **a timing taken while
something else is running is not a measurement**, and `time`on this machine reports the shell's own CPU as `user`, so the
number that matters is `real` alone and it must be repeated.

**The warning**: file indices are **per translation unit**. `Preprocessed::files` numbers the files of one compilation
from zero, so unit A's `h.h` and unit B's `h.h` are both index 1 - and when the linker merges two units' line tables
(step 4 of M46), it must rebase each unit's indices by the number of files before it. Otherwise a record says "file 1" and
the reader picks the wrong file, silently and plausibly. This is the same class as the `.str0` collision that cost a
milestone in M9: a per-unit name space meeting a program-wide one. It is written down here because step 4 is where it
bites, and nothing in the code says it yet.

**And the number to watch**: `.xccmap` on `sqlite3.c` is 625 272 bytes against 2 137 464 of `.text` - **28 %**, where
`tre` was 19.5 %. A bigger share on code with more header-origin lines, which is the code the map is most valuable for.

210 unit, 41 exe, 10 real, 1 crt, 30 CLI, 5 hostile; fmt and clippy clean.

### M46 - 2026-11-25 - the line number that would have been a lie

Points 4 and 5 of M44 - carry the map through the linker, let the CLI print `file.c:42` - are **blocked on a fact, not on
effort**, and the fact is worth more than the half-feature would have been.

**A `Span` does not say which file it is in.** It is `{ start, len }`: a byte offset and a length, with no file. That is
enough for a diagnostic, because a diagnostic is rendered while the token that produced it still carries `file: u16`
(which is an index into `Preprocessed::files`) - but the `cir` does not keep the token, only the span. So two files that
both have a byte at offset 100 produce spans that cannot be told apart, and a line number computed from a span alone
would be **right for the main file and wrong for every byte that came from a header** - which is most of the interesting
code in the fixtures: `sqlite3.h`, `windows.h`, `mimalloc/internal.h`.

`sqlite3.c` is the case in point: 265 977 lines, of which a large fraction of the *compiled* operations come from
headers. A map that says `sqlite3.c:130549` for an operation that came from `winnt.h` is worse than no map, because it
reads as an answer.

**The fix is cheap and the shape is clear**, which is why this is a note rather than a shrug:

1. `Span` gains `file: u16`, defaulting to 0. There are **8 construction sites** for the struct and 30 for its helpers,
   so the field is a morning's work, not a refactor - and the alternative (a side table keyed by offset) is the same
   information written where nothing can check it.
2. Every site that has a token takes the file from it; a synthesized span keeps 0, which is honest: "this compiler made
   this up, and it belongs to the file being compiled".
3. The file **names and line starts** have to travel with the artifact - `Preprocessed::files` is where they live, and
   `Artifact` does not carry them yet. That is the second half, and it is what makes `--map` render a line instead of a
   number.

**Steps 1 and 2 are in, and the fact that was missing was one line of plumbing.** `Span` has `file: u16`; the tokens the
*lexer* makes cannot know it, so the preprocessor - the layer that decided which file it was reading - stamps it with
`.in_file(file)` as the token is built, and everything downstream keeps the span and therefore the file. The record grew a
fourth varint, the file delta (zigzag, because a macro body and its use are two files), and the object now says which file
each line came from.

Proved where it matters: a `static inline` function in a header, called from a `.c` file, produces code *from the header*
- and the map says so, 6 records with file 0 and **2 with file 1**, the inlined body.

Step 3 is what remains: the **file names and line starts** travel with the artifact (`Preprocessed::files` has them,
`Artifact` does not), and `--map` renders `file.c:42` from them. Until it does, the map is correct as offsets and files
and deliberately not wired to anything that prints a line number.

### M45 - 2026-11-25 - the map that was bigger than the code

Reviewing `fdff9fc` found the section correct in shape and **absurd in size**, and shipped it as a measurement instead of
an opinion.

**The numbers, on `tre`** (177 436 bytes of `.text`):

| encoding | `.xccmap` | share of the code |
|---|---|---|
| one record per operation | 207 148 | 117 % |
| one record per *line* | 122 592 | 69 % |
| deltas as varints | **34 652** | **19.5 %** |

The first was what landed. A map bigger than the code it describes, in *every* object, for a table nothing can read yet -
which is the definition of a cost with no user. The middle one is what a lookup actually needs: consecutive operations of
one instruction share a line, and a lookup wants the greatest offset at or below the address, so a repeat adds nothing.
The last is the record itself: an offset delta (always forward, one byte under 128), a zigzag span delta (a loop goes
backwards, and zigzag makes a small negative as short as a small positive), and a length.

**Verified by decoding it**, because the reader's algorithm is the only thing that proves an encoding. Six records from a
three-line source, each landing on a real token of the right line:

```
code+0   byte 12  line 1 '{'      code+19  byte 39  line 2 '+'
code+6   byte 26  line 1 '3'      code+35  byte 71  line 3 '4'
code+12  byte 24  line 1 '*'      code+40  byte 70  line 3 '('
```

And two findings that came out of the review rather than the writing:

**The entry block's argument moves were labelled with the wrong line.** A block's ops are built in two places - the
entry's parameter moves arrive before the loop - and `resize` gave every pre-existing op the *first instruction's* span.
They now start at the block's own span, which is the function's opening line.

**A span of length zero names no line**, and one was recorded: `Span::at(0)` is what an instruction the front end made up
carries. Skipped now, with the reason beside it.

Points 4 and 5 of M44 still stand: carry the table through the linker, and let the CLI turn a span into `file.c:42`.

210 unit, 41 exe, 30 CLI; fmt and clippy clean.

### M44 - 2026-11-25 - line numbers, scoped and not started

`v0.1.6` gave a fault a **name**: the symbol table turns `.text+0x129e2d` into `sqlite3InsertBuiltinFuncs`. The next
step gives it a **location**, and the design is five touch points rather than a search. Written down because the work is
bigger than the session that found it, and a scope note is what the next one needs.

1. **`Op` does not carry a span, and it does not have to.** `lower_inst` opens with `let span = inst.span()`, so the span
   is in hand exactly where each operation is pushed - `MachineBlock { ops: Vec<Op> }` gains a parallel `spans: Vec<Span>`
   of the same length, one per op, and no `Op` variant changes. (A parallel vector is a fact written twice; the
   alternative is wrapping every variant in `(Span, Op)`, which touches sixty match arms to save one invariant. The
   invariant is checkable in one `debug_assert_eq!(ops.len(), spans.len())`.)
2. **The encoder knows the offset as it goes**: `Encoder::op` is called in a loop over the block's ops, and `code.len()`
   before each call is that operation's address. `Encoded` gains `lines: Vec<(u32, Span)>`, relative to the function's
   start, which is the same shape `relocations` already has.
3. **The object** - `coff::Image` - gains `lines: Vec<(u32, Span)>` in section-relative addresses, beside its
   `functions`, exactly as its symbols are.
4. **The linker** carries them through: `pe::Program` gains them, `Linked` gains them, and they become a section the way
   the symbol table became bytes after the last section - a **`.xccmap`** with one line per operation is the smallest
   thing a reader can use, and nothing maps it.
5. **The CLI renders it**, because that is where the *sources* are: a `Span` is (offset, length) and only `SourceFile`
   knows the lines. `--map` prints `rva  file.c:42` for the linked image, which is what turns a crash address into a
   line.

**Points 1 to 3 are in, with one thing more than the list said.** The spans go into `MachineBlock` beside its ops
(one `debug_assert_eq!(ops.len(), spans.len())`), the encoder records `(offset, span)` per op as `code.len()` before it,
`coff::Image` carries them section-relative - and the object *writes them out*, as a `.xccmap` section of twelve bytes a
record: the offset in `.text`, the span's offset, its length. That was not in the list because the list assumed the table
could sit unread until point 4; it cannot, because a field nothing reads is dead code and a warning on every build. The
object writer is a reader, `objdump -h` shows the section, and a unit test holds the table to its three invariants: every
offset inside `.text`, offsets only increasing, every span inside the source.

Points 4 and 5 are what remains: carry it through the linker, and let the CLI turn a span into `file.c:42` - which is
where the line *number* comes from, because a span is an offset and only the source knows where its lines begin.

What is deliberately *not* in that list: DWARF or CodeView. A debugger reading `.debug_line` would be better, and gdb
does read DWARF in a PE - but the address-to-line mapping has to be *collected* first (points 1-3), and that work is the
same either way. Do the collection, look at the map, and only then decide whether a real `.debug_line` is worth writing.

### M43 - 2026-11-25 - the feature that was tested on one back end, and the corpus that fixed that

Reviewing `419782b` found the fix correct and its **test in the wrong place**. The exe test runs level 2 - the own back
end - while a *language feature* has two back ends to satisfy, and the corpus is where they meet: level 2 through this
one, levels 0 and 1 through Cranelift, in one gate that compares the answers.

`cc52_ptrtoint.c` is that test now, and it measures more than the cast the exe test made:

- the four widths (`unsigned`, `unsigned short`, `unsigned char`, `unsigned long long`) of one non-null pointer;
- the two that are **not** truncations - `_Bool` from a pointer is a truth test, and a *signed* narrow type sign-extends
  the bits it kept (`(short)0x7f3a5c10` is 23568, not 0x5c10);
- a null pointer through every one of them, which is the case a truth test and a truncation happen to agree about and
  therefore says nothing about either;
- the round trip, integer cast to pointer and back, is the same number.

All three levels print what `clang -O0` prints. That is the measurement the review was missing: Cranelift's `ireduce` on a
pointer-typed value was assumed to work and now is known to. The differential went from 416 to 417 programs, and the
corpus ratchet from 444 to 445 agreeing - both by exactly one, which is the new program.

209 unit, 42 parser, 41 exe, 10 real, 1 crt, 6 invalid, 30 CLI, 5 hostile, 8 C API, 4 bcc20-sys; fmt and clippy clean.

### M42 - 2026-11-25 - `tre`, and the cast that was one line of the standard

The next real program was `tre` - a POSIX regex library, 9 041 lines in an amalgamation, with its own DFA builder, its
own allocator and its own stack. It is the most macro-heavy code this compiler has met, and it did not compile.

**One gap, and it is spelled out in C99 6.3.2.3p6**: a pointer may be converted to an *integer narrower than itself*, and
the result is implementation-defined. `conversion` covered pointer-to-pointer, pointer-to-64-bit-integer and
narrow-integer-to-pointer - the three cases where the value is bit-for-bit - and refused the fourth. `tre` hashes a
pointer with `(int)p`, which is exactly that case, and the diagnostic was honest: *"this compiler cannot convert pointer
to char to int yet"*. It is a truncation now, which is what the machine does.

Checked against clang rather than argued: `(unsigned)p`, `(unsigned short)(unsigned)p`, `(unsigned char)(unsigned)p`,
`(unsigned long long)p`, `(int)(p != 0)` and `(int)(short)(unsigned)p` - six answers, identical, including the two that
show the conversion is *not* a truth test: `_Bool` from a pointer is still "is it null", which `convert` answers with a
comparison before `conversion` is ever asked.

`tre` then compiles to a 196 KB object in silence. It is not a fixture yet - it needs a driver and its Windows branch
reads `basetsd.h`, so like mimalloc it would have no SDK-free half - but it is the ninth program this compiler has
compiled and the sixth to name a bug.

209 unit, 42 parser, 41 exe (one more), 10 real, 1 crt, 6 invalid, 30 CLI, 5 hostile, 8 C API, 4 bcc20-sys;
differential 416 ran, 0 mismatched; 444 corpus programs agree, 0 wrong; fmt and clippy clean.

### M41 - 2026-11-25 - the file that was gated for nothing, and the sentence nobody read twice

Reviewing `544a9e5` found one wrong claim and one sentence that did not parse.

**`hostile.rs` did not need gating, and its new comment said it did.** I wrote that "some of these build and run what
survives" and gated the file on Windows. It never links and never executes: every case spawns `xcc -c` and reads the
stderr. So the file was a front-end gate all along - the gate removed coverage on any other host and the comment
justified it with something untrue, which is the worse half. Un-gated; the comment now says what is true
(`-c` only, so it runs anywhere). `cli.rs` was checked the same way and *does* run images (`.exe` at four sites), so
its gate stands, and so do `x64.rs`'s and `differential.rs`'s.

**And the README sentence was broken**: "…have no tests rather than failing ones, and `<code block>` is the check
that…" - a sentence with a code block in the middle of it, which reads as two fragments. Rewritten as one statement
with the command as an example.

Verified while here: the gated files still **have** their tests on this host - `x64` 6, `cli` 30, `differential` 1,
`hostile` 5 - because `--list` counts them without running, and a `#![cfg]` typo would have made all four silently
empty. That is the check the previous commit should have included.

### M40 - 2026-11-25 - the gate that runs anywhere, and the three files that could not

The question was whether this work could move to a Linux host (WSL2 is on this machine). The answer splits, and both
halves are now facts rather than intentions.

**The compiler builds on Linux, today**: `cargo check --workspace --all-targets --target
x86_64-unknown-linux-musl` reports **no errors**. The preprocessor, the parser, the type system, `cir`, the passes and the
x86-64 encoder are host-agnostic, and they always were - nobody had checked.

**What it compiles does not run there**, because the target is Windows x86-64: PE32+, `ucrtbase`, and a C runtime this
compiler writes. Six test files act on that - they link an image and *execute* it - and only two of them said so. The
other three had no `cfg` at all and would have failed on a Linux host rather than skipped:

| file | what it does | was |
|---|---|---|
| `x64.rs` | links and runs the corpus through both back ends | no gate |
| `cli.rs` | compiles, links, runs | no gate |
| `differential.rs` | `--run` in process, and clang's executable | no gate |
| `exe.rs`, `real.rs` | links and runs | thirty-seven and eight per-test `cfg`s |

The three are now `#![cfg(target_os = "windows")]` as whole files, each with a doc comment saying *why* - the target cannot
run here, which is a fact about the target and not about the test - so on another host they are empty rather than red. The
README says the same thing next to the commands, with the `cargo check --target` line that verifies the portable half.

One thing deliberately not done: a *Linux target* - ELF, the System V ABI, a libc, a linker for it. That is what tinyhttpd
would need and it is a different project, not a different host.

209 unit, 42 parser, 6 invalid, 1 preprocessing gate, 2 fuzz, 1 crt; `cargo check --all-targets` for
`x86_64-unknown-linux-musl` clean; fmt and clippy clean.

### M39 - 2026-11-25 - tinyhttpd, and the answer that is "wrong program for this target"

`D:/Projects/github/tinyhttpd` was the next candidate: 549 lines, one file, a real program by anyone's definition. It does
not compile here, and **nothing about that is a compiler bug** - which is worth writing down, because the last five real
programs each named one.

It is a Solaris/Linux program. `<sys/socket.h>`, `<netinet/in.h>`, `<arpa/inet.h>` and `<sys/wait.h>` do not exist on this
target at all: mingw-w64 offers `winsock2.h` instead, and there is no `fork` and no `waitpid` to call - nor any header
that could declare them. Measured by taking the four includes out for a moment: the remaining failures are `u_short`
(a winsock typedef that came from `netinet/in.h`) and `pthread_compat.h`, whose `WINPTHREADS_ALWAYS_INLINE` is defined
only under `__GNUC__` or `_MSC_VER` - the third case is left undefined, so `pthread.h` uses a macro that expands to
nothing and a declaration reads `static WINPTHREADS_ALWAYS_INLINE int ...`.

So: a POSIX program needs a port, this compiler is not a port, and the useful conclusion is that **a fixture has to be a
program written for the target** - which all eight of them are, by accident of how they were chosen rather than by rule.
That is a rule now.

The measurement does leave one thing behind. The diagnostic that started this said
`pthread_compat.h:132:39: error: expected a type specifier`, and line 132 is a `#define`: the real mistake is a *use*
several hundred lines later, and the reader is sent to the definition. That is the note-vs-location work (M34) with a
concrete example to fix against, which is what it was missing.

### M38 - 2026-11-25 - the sweep for the second diagnostic, and the two negatives that close it

Reviewing `f9669bf` asked the question that commit answered for one path: **where else does a poisoned expression get a
second complaint?** Eight sites say "not lowered yet" in the parser; three checked for poison before `f9669bf`, and the
one on the initialiser path was the fourth.

The other five, site by site:

- `assign`'s "this kind of assignment" already refuses when the *target* is the error type, which is the same fact for
  this purpose.
- A conditional expression of a type nothing lowers: a `?:` whose arms are poisoned has the error type, and `lowerable`
  answers for that type like any other - so the site is not reached.
- **`&` of a poisoned expression: tested, and it reports once.** `int *p = &(*1);` gives `* needs a pointer, not int` and
  nothing else, because the `&` path asks what the operand *is* before asking where it lives.
- The two `va_list` sites are about a list that is not a local and about `va_arg` outside a function - neither is a
  question about the type of an expression.

**So the poison sweep is closed**: one path needed the guard, and the rest cannot be reached with a poisoned value. That
is a negative result and it is written down as one, because the next reader's question ("did anyone check the others?")
has an answer now.

`168ae73` was reviewed too: the fixture's include roots are the fixture's own directory, then `include/`, then the SDK -
in that order, which is what makes `#include <mimalloc.h>` resolve; the `standard` parameter is passed **explicitly** at
all four call sites rather than defaulted inside the helper, so a fixture cannot silently change dialect; and the four
older fixtures still compile as C99, which is what they were written as.

209 unit, 42 parser, 40 exe, 10 real, 1 crt, 6 invalid, 30 CLI, 5 hostile, 8 C API, 4 bcc20-sys; fmt and clippy clean.

### M37 - 2026-11-25 - mimalloc becomes a fixture, and it is the first one CI cannot see

`tests/fixtures/mimalloc/` is `src/` and `include/` from Microsoft's allocator, unmodified, with a driver: `mi_malloc`, a
`strcpy` into the block, `mi_version`, `mi_realloc`. One test, and it compares the output with what a `clang -O0` build of
the same two files printed - the two lines are all the driver prints, because a fixture's driver should not be a test
suite of its own.

**Why it took an hour and why the shape is different from the other four.** Every one of mimalloc's 21 units includes
its own OS layer, which includes `windows.h` - so unlike SQLite, Lua and yyjson there is **no SDK-free half**: on a
machine without the SDK there is nothing to compile, and CI is such a machine. That makes it the first fixture whose
verdict is "not run" in CI, and the entry says so rather than pretending otherwise. Its value is where development
happens, which is where the last five conformance bugs were found.

The helper grew a `standard`: every previous fixture is C99 and mimalloc is C11 (`_Atomic`, `_Thread_local`,
`stdatomic.h`), and a fixture that had to be compiled in a dialect it was not written for would be testing the wrong
program. Three call sites pass the default explicitly rather than a hidden one.

209 unit, 42 parser, 40 exe, **10 real** (one more), 1 crt, 6 invalid, 30 CLI, 5 hostile, 8 C API, 4 bcc20-sys;
differential 416 ran, 0 mismatched; 444 corpus programs agree, 0 wrong; `.text` 80 288; fmt and clippy clean.

### M36 - 2026-11-25 - one mistake, one diagnostic - and the parser bug that hid behind it

The double diagnostic is gone, and finding it uncovered a second bug that was mine, from an hour earlier.

**The poison was being laundered.** An expression that has been diagnosed is the *error type*, and the error type is
compatible with everything - that is what it is for. But `convert` only passed poison through when the *target* was the
error type; a poisoned value becoming an `int` came out as an `int` with no code, and the next question on that path
("is this initialiser value lowered?") then said so - a second complaint, about a second thing, for one mistake. Poison
stays poison now, and the initialiser path asks whether the value is poisoned before it asks anything else.

**And the reason a *third* error appeared: the parse stopped mid-call.** `atomic_generic` returned early when an argument
was wrong - between a `,` and a `)` - so the parser read the rest of the call as the rest of the *declaration* and
reported "declaration does not declare anything" for a mistake in the argument. The function now parses every argument
first and judges afterwards, which is the rule the rest of this parser already follows ("the parse still has to happen:
diagnostics and folding") and the one place I had broken it.

The test is the rule rather than the case: `a_poisoned_expression_is_diagnosed_once` feeds two different poisoned
expressions - `int x = *1` and an intrinsic with a wrongly-typed `expected` - and asserts **one** diagnostic each. The
first draft of that test used `1 + "text"`, which is pointer arithmetic and perfectly legal C: a test that asserts a
refusal has to be a program that is wrong.

209 unit, 42 parser (one more), 40 exe, 9 real, 1 crt, 6 invalid, 30 CLI, 5 hostile, 8 C API, 4 bcc20-sys; differential
416 ran, 0 mismatched; 444 corpus programs agree, 0 wrong; fmt and clippy clean.

### M35 - 2026-11-25 - the argument nobody checked, and the two ways it was wrong

Reviewing `bbb8ce7` found one thing, and it was a crash rather than a report.

**`expected` in a compare-and-exchange was never checked.** C11 7.17.7.4 says it is a pointer to *an object of the same
type* as the one being exchanged, because a failed exchange writes the value it found through it. Nothing checked either
half, so both wrong programs compiled:

```c
int x = 1;
char expected = 1;
atomic_compare_exchange(&x, &expected, 2);   /* the runtime writes 4 bytes through a 1-byte object */
atomic_compare_exchange(&x, 5, 3);           /* not a pointer at all: accepted, then SIGSEGV */
```

The first is memory corruption and the second is a segfault, and both were accepted in silence - which is the one thing
this compiler may not do. Each is a diagnostic now, with the reason in it: *"`expected` points at `char` and the object is
`int`: a failed exchange writes what it found through the first, so the two must be the same type"*.

What the review found sound: `atomic_kind`'s table (integer widths, `_Bool` as an unsigned byte, a pointer as eight bytes,
everything else refused), the conversion of the value being stored (`emit_call` converts it to the runtime's parameter
type, which is where a pointer object becomes an `unsigned long long`), and the result typing - the exchange's is the
pointee's type and the compare-and-exchange's is `_Bool`, both of which the call instruction carries correctly.

One small thing left behind: the poisoned value after that diagnostic still produces a second, unrelated one from the
initialiser path. That is the same class as the note-vs-location work in M34 - a diagnostic that has already spoken
should not have a second one land on top of it - and it is written down there rather than fixed here.

209 unit, 40 exe, 9 real, 1 crt, 30 CLI; fmt and clippy clean.

### M34 - 2026-11-25 - the queue, measured, and the one that should not be fixed

The three loose ends, taken in order and each answered with a number or a reason rather than a plan.

**`snprintf` under `--run` cannot be fixed cheaply, and the reason is a decision worth keeping.** The JIT resolves a name
in **one** C runtime: `msvcrt.dll` first, because that is the one that exports `printf` and its family under their own
names, and only that one for the life of the process - two CRTs in one process keep two heaps and two sets of stdio
buffers, so a block from one `malloc` freed by the other's `free`, or a `FILE *` from one handed to the other's
`fprintf`, is the corruption that gives a C runtime its reputation. `snprintf` is C99, so `msvcrt` has it as `_snprintf`
- which does not null-terminate on truncation and returns a negative number instead of the length - and the version the
headers declare is `ucrtbase`'s. A JIT that answered `snprintf` would therefore be mixing two runtimes to save the reader
a linker. The refusal says so, in the words the reader needs: *"link the program instead of running it"*.

The one honest way to have both is to choose the runtime by **what the program calls**, before the first allocation and
never mid-run: a program that needs `snprintf` and nothing from msvcrt's own names could be given `ucrtbase` alone. That
is a real piece of work - the `printf` front ends differ between the two, and the linker already shims them for programs
- and it is written down here rather than done, because a JIT that changes runtimes halfway through is the bug the rule
exists to prevent.

**The remaining refusals are measured, over the 67 181 C files on this machine**: a VLA-shaped declarator in 705,
`__asm__` in 585, a statement expression `({ ... })` in 417, `typeof` in 230, `alloca(` in 138, a computed `goto *` in
36. Those are counts of files that *mention* the construct, not of programs that need it - many sit inside
`#ifdef __GNUC__` - so the next one to implement is still a question for a real program to answer, exactly as mimalloc
answered five of them this week.

**The note-vs-location refactor is not done**: a diagnostic about a macro body carries the invocation index (M15) and
attaches the note when the caret is the token just consumed or the cursor, not when it sits on a `&` three tokens back.

### M33 - 2026-11-25 - the atomics that read the type, and mimalloc runs

**`__builtin_atomic_exchange` and `__builtin_atomic_compare_exchange`**, as designed in M29. The object's *type* is what
decides everything - width, signedness, and which runtime function does the work - and the compiler is the only layer
that has it: `_Generic` cannot say "any pointer type", because the arm would have to name the program's own, and
`mimalloc` exchanges a `mi_heap_t *volatile *`. `<stdatomic.h>`'s `atomic_exchange` and the two compare-and-exchanges are
now these builtins, and the `_Generic` tables behind them are gone.

Checked by hand against the standard's own rules, all seven values on one line: the exchange returns the old value and
stores the new; a compare-and-exchange against the value actually there succeeds and leaves `expected` alone; one that
fails does *not* store, and writes the value it found back through `expected` (C11 7.17.7.4); and all four work for a
pointer object as readily as for an `int`.

**`__movsb` and its three siblings** were the last thing in the way: the `rep movs` block copies the Windows headers
name, and mimalloc copies its heap structure with them. They are four `memcpy` calls in the runtime, with the same
no-overlap rule the intrinsic has.

**Measured, and it is the point of the last four commits**: `mimalloc`'s amalgamation, `static.c`, links against a driver
and runs.

```
$ xcc --std=c11 --opt-level=2 -I include -I <sdk> midrv.c static.c -o mi.exe
$ ./mi.exe
mimalloc under xcc 1
1
```

`mi_malloc`, `mi_realloc` and `mi_free`, through a production allocator's own thread-local heaps. The road here was five
conformance bugs and two intrinsics, and every one of them was found by compiling real code rather than by reading the
standard.

209 unit, 1 crt; fmt and clippy clean.

### M32 - 2026-11-25 - the guard lands, and blames the wrong instruction for three runs

The operand guard from M31 is in: the encoder remembers what the operation being encoded *declared* it reads, and `read`
asserts membership. It fired on the first three tests with three different messages, and every one of them was wrong -
about *my* guard, not about the back end.

`LoadLocal { dst: VReg(6), offset: 16 }` reads `v4`. A load from the frame reads no register at all, so the message was
nonsense - and nonsense with a shape: the operation named was always the *last instruction of the block*, and the read
always belonged to the terminator after it. The branch after a call reads the call's result; that is exactly right, and
it was being blamed on the call. `encoder.operands` was set per *instruction* and never per *terminator*, so every
terminator was checked against the previous instruction's list.

The fix is one line - `encoder.operands = block.term.operands()` before the terminator is encoded - and it makes the
guard cover both halves of a block. Green everywhere.

Two things this leaves: the invariant is now checked on **every test run**, which is what the five-times class needed; and
the first version of the check reported three failures and none of them was real, which is the second time this session
that a diagnostic's own bug was mistaken for a compiler's (the false `_Bool -> unsigned char` from comparing type *ids*
was the first). A check that shouts must be as testable as the code it checks.

209 unit, 40 exe, 9 real, 1 crt, 6 invalid, 30 CLI, 5 hostile, 8 C API, 4 bcc20-sys; fmt and clippy clean.

### M31 - 2026-11-25 - the sweep of the operand lists, and the guard that found something it cannot yet explain

The review of `8cb4833` had nothing unreviewed to read - the tree is clean and every commit before it has had a pass -
so it did the thing that commit's finding asked for: **check every operand list in the back end by hand**, because "a
register the allocator was not told about" has now been found five times in this project and every time by reading code
rather than by a test.

`Op::operands()` swept against what each encoder arm actually reads: `Const`, `Incoming`, `LocalAddress`,
`SymbolAddress`, `ThreadAddress`, `LoadLocal` and `VaStart` read no register; `StoreLocal`, `LoadMemory`,
`StoreMemory`, `LoadFloat`, `StoreFloat`, `FloatConvert`, `Unary`, `Canonical`, `VaArg`, `SegmentRead` and `Call` read
what they declare; `Compare` and `BranchCompare` take one operand that may be an immediate, which their `register()`
handles; `AtomicCas` names all three. The CIR layer was swept the same way - `Inst`'s three match arms (`operands`,
`rewrite_values`, and the verifier) cover the same fourteen variants, and `SegmentRead`'s address is an operand in all
three since M29. Nothing else was wrong.

**Then the sweep became a check.** The encoder now remembers the operation it is encoding and asserts in debug builds that
every virtual register it reads was one the operation declared - which turns the class from "found by reading" into
"found by running any test". It found one immediately: a `Call` to `__stdio_common_vfprintf` with `dst = v5` reads `v5`,
and `v5` is its own destination. Not one of the five shapes already known, and not a case that can be left in.

That is where this stops, deliberately. The assertion is three lines and it works; where the read comes from is not yet
known - `place_arguments` reads only the arguments, `finish_call` only writes, `float_destination` and `canonicalise`
take concrete registers - and a guard landed with an unexplained firing is a guard somebody will weaken at three in the
morning. The change is reverted, and this is the record: **the next session lands the assertion first and lets it name
the function**.

209 unit, 41 parser, 40 exe, 9 real, 1 crt, 6 invalid, 30 CLI, 5 hostile, 8 C API, 4 bcc20-sys; fmt and clippy clean.

### M30 - 2026-11-25 - the register the allocator was not told about, and a number that was noise

Reviewing `a7da05e` found one real bug, and it is the fifth time this project has made the same mistake in a new place.

**`Op::SegmentRead` read a register and told the allocator it read nothing.** Making the offset a *value* gave the
operation a `base` register; `operands()` still answered `Vec::new()` - the arm written when the offset was a constant
and there was nothing to read. So the allocator ended `base`'s live range at the read and was free to hand its register
to `dst`: a read from wherever the destination landed. The test passed anyway, which is the worst property a bug can
have; the measurement that showed it is the code, not a failure. It is `vec![*base]` now, with the reason written beside
it and the `AtomicCas` note above it - which records the *same* mistake about `expected`, found the same way.

The test grew the shape that would expose it: the offset in a register that is live *across* the read, with a value
accumulating while it is - four reads in a loop. That is not a proof, and it is the only kind of guard available for an
allocator invariant: exercise the shape that breaks.

**And the encoder had two matches for one table.** The opcode and the prefix were chosen in separate `match`es, one of
which had two arms doing the same thing (`4` and `_` both `0x8B`). One table, one prefix question, because a 64-bit read
says so in its REX byte and the others need REX only to name an extended base register.

**The 1518 second x64 suite was noise.** Re-measured on a quiet machine: 337 seconds, which is this suite's usual 300
with the machine doing something else. The earlier 2194 was two of my own runs overlapping. Both numbers are recorded
here because a gate whose result depends on the machine is not a gate, and neither is a timing.

209 unit, 41 parser, 40 exe, 9 real, 1 crt, 6 invalid, 30 CLI, 5 hostile, 8 C API, 4 bcc20-sys; differential 416 ran, 0
mismatched; 444 corpus programs agree, 0 wrong; `.text` 80 288 bytes; fmt and clippy clean.

### M29 - 2026-11-25 - `__readgsqword`, and the atomics that cannot be spelled in a header

mimalloc's third blocker was `__readgsqword`, and the interesting part is what it was *not*: the instruction already
existed, with a constant offset, and the parser already had all eight names. What it did not have was a *variable*
offset - and `mimalloc`'s is `slot * sizeof(void*)`, a number it computed.

**An offset is an address, and an address here is a value.** `Inst::SegmentRead` took a `u32`; it takes a `Value` now,
which is the shape `gs:[reg]` and `gs:[0x30]` share in the machine - one addressing mode, one encoding, with the constant
folded into the displacement by the same `address()` that lowers every other load. The verifier sees the offset as an
operand (so dominance and definedness are checked), the passes rewrite it, and the dumper prints it as a value. The
Cranelift back end refuses it as it refused the constant form: Cranelift cannot write a segment prefix, and level 2 can.

The oracle is the machine's own: `gs:[0x30]` is the TEB's self pointer and `gs:[0x48]` is `ClientId.UniqueThread`, so the
test compares the byte it reads against `GetCurrentThreadId()` - through the constant form, the dword form, and a
variable offset. Four answers, all `1`.

**Eight `_explicit` forms were missing from `<stdatomic.h>`.** C11 7.17.7 spells every atomic operation twice, and ours
had only the short spelling - so `atomic_exchange_explicit` was an undeclared function returning `int`, which is how
mimalloc's pointer exchange came back as a thirty-two-bit number widened into a pointer. They are in the header now, as
the standard's own "the others are this one with `seq_cst`", which is what this compiler's atomics are anyway.

**And what is still missing, measured.** mimalloc now stops one line further: `atomic_exchange` of a
`mi_heap_t *volatile *`. C11's generic functions are over *any* atomic type, and `_Generic` cannot say "any pointer" - the
arm would have to name `mi_heap_t *volatile *`, which no header can know. That is why glibc and musl use `__typeof__` for
these and why this is not a header fix: the answer is a `__builtin_atomic_exchange` (and `__builtin_atomic_load`/`store`)
in the compiler, inferring width and sign from the object's type the way `__builtin_atomic_cas` already does, with the
header reduced to a name. That is the next piece of work, and `mimalloc` is the gate for it.

209 unit, 41 parser, 40 exe (one more: the thread environment block), 9 real, 1 crt, 6 invalid, 30 CLI, 5 hostile, 8 C
API, 4 bcc20-sys; differential 416 ran, 0 mismatched; 444 corpus programs agree, 0 wrong; fmt and clippy clean.

### M28 - 2026-11-25 - the review of mimalloc's three, and the one that was still wrong

Reviewing `89ff054` found one real bug in what it added, and one name that said the wrong thing.

**`__func__` was writable.** C99 6.4.2.2p1 declares it `static const char __func__[] = "the name";` - the qualifier is
on the *element* - and the version this session added built a plain `char[]`, so `__func__[0] = 'x'` compiled and wrote
into it. clang refuses that, and a compiler that accepts it is the "accepted and dropped" mistake in its purest form:
nothing warns, and the program has written to an object the standard says is read-only. The element is `const char` now,
and the test asserts the refusal as well as the answers, because the refusal is the half a program could get wrong
silently.

**Two helpers whose names were one word apart.** `literal_object` creates the object; `literal_of` returns the value of
one that exists. They read as one thing and are two, so the second is `literal_value` - the object and the value of it,
which is the distinction ADR-004 makes everywhere else.

What was reviewed and found sound: the `#include` verbatim rule (only p2's written names are unexpanded, p4's `#include
MACRO` still is, and the tests name both), the `__thread` spelling (it joins `_Thread_local` in the storage-class group,
so `static __thread` is one declaration and not two), and the memo of `__func__` objects - one per function, because a
logging macro expands it a dozen times in one body.

209 unit, 41 parser, 39 exe, 9 real, 1 crt, 6 invalid, 30 CLI, 5 hostile, 8 C API, 4 bcc20-sys; differential 416 ran, 0
mismatched; 444 corpus programs agree, 0 wrong; fmt and clippy clean.

### M27 - 2026-11-25 - mimalloc, and the three things it asked for before it could be measured

`mimalloc` is 21 units in `src/` plus an amalgamation (`static.c`) that is the whole library in one object. It compiled
to the third error in under two seconds, and every one of those three was a C99 conformance bug rather than a feature.

**`#include <errno.h>` was macro-expanded.** `include()` expanded the directive's tokens before reading the name, so in
any unit where `<errno.h>` had already been read - and the SDK's header defines `errno` as `(*_errno())` - the second
include of the same file asked for `(*_errno()).h`. C99 6.10.2p2 is not ambiguous: `"the name"` and `<the name>` are the
name *as written*, and only p4's third form, `#include MACRO`, is expanded. The directive now looks at its first token
and takes the name verbatim when it is a string or `<`; mimalloc includes `errno.h` twice.

**`__func__` was not declared.** C99 6.4.2.2p1 makes it the enclosing function's name, "as if" the body began with
`static const char __func__[] = "the name";`. It is answered before either lookup, and only inside a function. The object
is built by the same function a string literal uses - the tail of `string_literal` is now `literal_object`, and a
literal's *value* is now `literal_of` - so there is one writer of "an anonymous global with a NUL, and a value that is
its address". One object per function, memoized: a logging macro that prints `__func__` is expanded a dozen times in one
body.

**`__thread` was a syntax error.** It is GCC's and clang's spelling of `_Thread_local`, and mimalloc reaches for it in
the branch for "some other compiler" - which is this one, because `mi_decl_thread` is `__declspec(thread)` for MSVC,
`__thread` for GCC, and `__thread` again for everybody else. Accepted as the storage class it means, beside
`_Thread_local`.

**What is left, measured.** The amalgamation now stops at `__readgsqword`, an MSVC intrinsic that is one instruction
(`mov rax, gs:[disp32]`) and needs a segment override this back end does not encode - a real piece of work, and a real
feature request, from a library that wants the thread's TEB. And one initialiser in `theap.c` refuses an `int` where a
`mi_heap_t *` belongs, which needs reading before it is called either a bug or a gap.

209 unit (one more: a p2 include name read as written), 41 parser, 39 exe (one more: `__func__` and `__thread`), 9 real,
1 crt, 6 invalid, 30 CLI, 5 hostile, 8 C API, 4 bcc20-sys; differential 416 ran, 0 mismatched; 444 corpus programs agree,
0 wrong; fmt and clippy clean.

### M26 - 2026-11-25 - the review of the paste and the pass, and two things under them

Reviewing `d27ffbc` found the two bugs it fixed were fixed correctly, and two things *around* them were not.

**A comment that said the opposite of the code.** `Func::verify`'s interface comment said it "runs in debug builds and in tests, never on a compilation's path". `optimise` calls it after every pass, on every compilation, release included - and has since the verifier existed. The comment is the wrong side, so the code stays and the comment now says what is true, with the cost measured: `-c sqlite3.c` at `--opt-level=2` takes 12.4 seconds with the verifier and 10.0 without, so a fifth of a large compilation buys a wrong program turned into a diagnostic that names the pass. That is the trade this compiler makes everywhere else.

**Six copies of one error-propagation shape.** Naming the pass in each verifier message (`"after fold: ..."`) had been written as six `if let Err(broken) = verified(...) { return Err((func.name, broken)); }` blocks, which is one fact - "run this pass, then check it under its name" - written six times and a seventh waiting. One `checked` helper and one `fail` closure make it a line per pass, and the pass's name is written once instead of twice.

What was reviewed and found sound: the `##` chain loop's index arithmetic (after the first pair, `at` walks the `##`s themselves, which is the mistake the first attempt made and the test `a_chain_of_pastes` now holds), the type check in `truth` (a comparison's result is `int` or `_Bool`, both fundamental, so comparing ids is exact - and the comment says why), and the `debug_assert` that moved from the builder to the front end, where it asks `types.same` instead of comparing ids that ADR-004 says may differ.

208 unit, 41 parser, 38 exe, 9 real, 1 crt, 6 invalid, 30 CLI, 5 hostile, 8 C API, 4 bcc20-sys; differential 416 ran, 0 mismatched; 444 corpus programs agree, 0 wrong; fmt and clippy clean.

### M25 - 2026-11-25 - the paste that pasted once, and the pass that changed a type

Two real programs, two bugs, both found in the first ten minutes - which is the argument for real programs over
invented ones, made again.

**`a##b##c` pasted one pair.** C99 6.10.3.3p3 pastes *every* `##` in a replacement list, in turn, the result of one
paste being a token the next one uses. This pasted the first pair and advanced past it, so the second `##` found nothing
on its left and said so. `yyjson` writes `0x##d##c##b##a##UL` - a four-way paste - twenty-eight times in one file, from
one macro; the whole library was blocked by it. The replacement list now walks its `##`s, each paste feeding the next.

**A pass that changed a value's type.** `truth` removes `cmp ne v, 0` when it already knows `v` is 0 or 1, which is 45%
of the corpus's comparisons and worth doing. But a comparison's answer is `int` (C99 6.5.9p3) *or* `_Bool` - the latter
when `truth_value` converts to `_Bool` - and substituting one for the other changes a value's *type*. `yyjson`'s
`val->tag = (u8)4 | (u64)((u8)sign << 3)` then read a `_Bool`-typed `convert` whose operand was an `int`, and the
verifier said so in the only words it had: "block 15 says it converts `_Bool` -> `unsigned char`, and it converts
`int` -> `unsigned char`". The substitution now asks whether the two values have the same type first - and comparing the
*ids* is exact here, because a comparison's result is one of two fundamental types and a fundamental type has one id.

**And two diagnostics that made the search shorter.** The verifier's message named a block and not the types, so it now
names all four; and `optimise`'s checks said "a pass broke it" without saying which, so each now names the pass it just
ran. Neither is a bug fixed, and both are what turned "something is wrong in 11 224 lines" into one line.

A third finding came and went: a `debug_assert` I added to the builder compared *type ids* where ADR-004 says two ids
can be one type, and fired falsely on a pointer-heavy program. The assertion moved to the front end and asks
`types.same`, which is the question the rest of the compiler asks.

**yyjson is now a fixture**: 11 224 lines, and the only one that is compiled *and run* on every machine - it needs no
Windows SDK - with its output compared line for line against clang's. It reads a document, prints what it found, then
builds and serialises one, which covers both allocation paths.

208 unit (one more: a chain of pastes), 41 parser, 38 exe (one more: a `_Bool` from a comparison, converted), **9 real**
(one more), 1 crt, 6 invalid, 30 CLI, 5 hostile, 8 C API, 4 bcc20-sys; differential 416 ran, 0 mismatched; 444 corpus
programs agree, 0 wrong; fmt and clippy clean.

### M24 - 2026-11-25 - the review of seven commits, and the five things it found

The unreviewed stretch was `a8485b5` (the symbol table) through `8914a53` (the Lua fixture). Five findings, and two of
them are the kind this project keeps rediscovering.

**One missing section blanked the whole symbol table.** `symbols()` asked for `.text` *and* `.data` at once and returned
nothing if either was absent - two independent facts answered by one `else`. A program whose objects happen to be empty
would have had no names at all. The two families are now added independently.

**Three bools answering one question.** `inline` linkage was implemented by threading `last_declared_static`,
`last_declared_inline` and `last_declared_extern` from the declaration loop to the body - three copies of one fact, read
together in one place, waiting for a fourth (`_Thread_local` will be next). They are one `Linkage` value now, with
`Linkage::of(&specs)` and one call to `is_external()`, which is also where C99 6.7.4p6's rule can be read in one piece.

**An empty name is not a name.** `__asm__("")` was recorded as a link name, and would have reached the linker as a symbol
called nothing. Refused at the word, in the same breath as the rest of that check.

**`?` where `continue` belonged.** `windows_sdk()`'s Visual Studio branch used `read_dir(..).ok()?` inside a loop over
candidate roots: a directory that could not be listed ended the search instead of moving to the next candidate. That is
the "an unwrap turns cannot-know into a value" mistake wearing a ticket to the same place.

**A counter nothing reads, and a comment that named a caller that does not exist.** `Changes` said "for a caller that
wants to print it (`--explain`, the tests)" - and `--explain` has never printed one - and the `dead_blocks` field added
this session had no reader at all. The field is gone, and the comment says what is true.

Two smaller ones: `dump::terminator` kept two parameters it stopped using when the dumper gained `trap` (removed), and
Lua's `luac.c` was shipped in the fixture but never compiled - it is now in the compile-only set, so the bytecode
compiler the release ships is covered by a test rather than by a measurement made once by hand.

**A measurement worth recording, because it was wrong.** The hostile suite was reported at 2194 seconds against its usual
twenty, and no code had changed: two of my own test runs were overlapping on one machine. The suite is 19.9 seconds when
it runs alone. A gate whose result depends on the machine is not a gate, and neither is a measurement.

207 unit, 41 parser, 37 exe, 8 real, 1 crt, 6 invalid, 30 CLI, 5 hostile, 8 C API, 4 bcc20-sys; differential 416 ran, 0
mismatched; 444 corpus programs agree, 0 wrong; fmt and clippy clean.

### M23 - 2026-11-25 - Lua becomes a gate too

The other half of the same argument: `tests/fixtures/lua/` is `src/` from the 5.5.1 release tarball - 34 files,
unmodified - with `driver.lua`, a script this project wrote, and three tests.

**Why an interpreter and not just SQLite.** One is a 265 977-line parser that opens a database and runs a query; the
other is a long-lived loop dispatching bytecode, a garbage collector that traces its own machine stack, a pattern
matcher, coroutines as separate stacks, and `string.pack`'s format compiler. Lua 5.5 also walks the newest corner of the
language - integer division, bitwise operators, `goto`, integer subtypes - and its driver script prints every answer, so
two builds are compared line for line: numbers, error text, `tostring` of a metatable, `utf8.codepoint`, an `io` round
trip, and `string.dump`'s bytecode read back by `load`.

**Two of the 34 units read `windows.h`**, and `luaconf.h` defines `LUA_USE_WINDOWS` for `_WIN32` whether a program wants
it or not - `loadlib.c` loads DLLs, `lua.c` asks whether stdin is a terminal. So the compile-only test leaves those two
out (31 units, every machine) and the run test needs a host with an SDK and says so. The compile-only half is not a
weaker test: it is the front end and the back end over the VM, the parser, the GC and the string library, which is where
a miscompile shows as a refusal or a verifier failure.

**What it cost, before it was a test**: `__asm__("name")` as a link name and `inline` deciding linkage (M22), both found
by compiling it, and neither visible in a corpus of 464 small programs or in SQLite.

`run_fixture_with` grew an argument list, because an interpreter is run as `lua script.lua` and a C fixture is run with
no arguments at all. The `real` suite is now 8 tests and 122 seconds in a debug build.

207 unit, 41 parser, 37 exe, **8 real** (two more), 1 crt, 6 invalid, 30 CLI, 5 hostile, 8 C API, 4 bcc20-sys; differential
416 ran, 0 mismatched; 444 corpus programs agree, 0 wrong; fmt and clippy clean.

### M22 - 2026-11-25 - Lua 5.5.1, and the two declarations the Windows headers spell

Lua 5.5.1, from the release tarball, compiled by this compiler into a 642 KB `lua.exe`: it runs a script that touches
the whole language - integers and bitwise operators, `//`, patterns, `string.pack`, metamethods, coroutines,
`pcall`/`xpcall` with error objects, `goto`, `repeat`, `for` with a negative step, utf8, an `io` round trip, `load` - and
prints **byte for byte** what a `clang -O2` build of the same 33 files prints. `luac` builds too, and the bytecode it
writes is byte-identical to clang's.

Two bugs stood in the way, and both are in the class this compiler keeps finding: something the Windows headers say that
C99 does not.

**`__asm__("name")` after a declarator is a *link* name.** mingw-w64 renames every CRT function whose wide and narrow
spellings are one name on this target - `vsscanf` is `__mingw_vsscanf`, `_wctime` is `_wctime64`, `_fileno` on a 64-bit
file offset is `_fileno64` - through `__MINGW_ASM_CALL`, which is `__asm__("...")` in a declaration. The parser stopped at
the `__asm__` and said "expected `;`". It is now read where a declarator ends: the symbol keeps the name the program
writes, and `links` records what it is linked by, asked at the two places a name becomes a symbol - a call, and a
relocation in an object's bytes. Renaming a non-function is refused rather than half-supported: nothing in the SDK does
it, and an object renamed would need the relocation to carry a name rather than a symbol.

**`inline` decides linkage.** It was consumed and dropped, so every `FORCEINLINE` helper that `winnt.h` defines - and
there are dozens - was emitted as an *external* definition in every unit that included `windows.h`, and the linker's
one-definition rule (M11) refused the second unit. C99 6.7.4p6 is plain about it: a definition declared `inline` and
neither `static` nor `extern` is an *inline definition*, which "does not provide an external definition". It is emitted
with internal linkage now, like a `static` one, which is what a compiler that does not inline one has to do.

And a third, smaller: our `stdio.h` is the copy `<stdio.h>` resolves to, so the SDK's `_fileno`, `_fdopen`, `_popen` and
`_pclose` were never declared. An undeclared function is an `int`, and these return a `FILE *` - Lua's `_popen` came back
as a thirty-two-bit number widened into a pointer, which works until the heap is above four gigabytes. They are declared
in `include/stdio.h` now, in a block that says they are the target's additions and not C99's, and `C_RUNTIME` names them,
which the header/runtime-list test found before I did.

207 unit, 41 parser, 37 exe (one more: a rename that has to link, and one header with one `inline` definition in two
units), 4 real, 1 crt, 6 invalid, 30 CLI, 5 hostile, 8 C API, 4 bcc20-sys; fmt and clippy clean.

### M21 - 2026-11-25 - SQLite becomes a gate

A measurement that is not a test is a measurement that expires. The session's biggest result - 265 977 lines of SQLite
compiled silently, linked and answered correctly - was a paragraph and a temp directory; it is now `sqlite3.c` in
`tests/fixtures/sqlite/` with two tests.

**Two, because the fixture needs a Windows SDK to run and this compiler's whole point is not to need one to compile.**
`-DSQLITE_OS_OTHER=1` takes out the OS layer - the only part that includes `windows.h` - so that build compiles and
writes a 2.6 MB object on **any** machine, and that test never skips. With its Windows layer on, the driver opens an
in-memory database, creates a table, inserts three rows in one `INSERT`, and reads them back through a prepared
statement: `6 one,two,three 3.51.3` and `0 -1`, byte for byte what `clang` printed when the fixture was added. That one
says it is skipping when the host has no SDK, which is what CI is - and the compile-only test is the one that holds
there.

What the fixture cost when it arrived, over the two days it took: three bugs in the type of an integer constant and its
two axes, the initialiser that was a constant cast to a pointer, the initialiser that became zeroes, a `for` loop whose
update ended in a conditional, and a constant folder that compared two operands before converting them. None of the
five was visible in a corpus of 464 small programs, and every one of them is now what a single test would catch.

`run_fixture` grew a `windows_sdk()` and a list of include roots, because this is the first fixture that names a header
nobody's C library has. `Budgets::for_a_compiler()` - whose doc comment has said "sqlite's amalgamation is one" since it
was written - is now what the fixture tests use, which is what the command line already did.

Cost: the `real` suite goes from 5 seconds to 70 in a debug build, against 8 minutes for the x64 suite. The vendored
amalgamation is 9.9 MB.

207 unit, 41 parser, 36 exe, 6 real (two more), 1 crt, 6 invalid, 30 CLI, 5 hostile, 8 C API, 4 bcc20-sys; fmt and clippy
clean.

### M20 - 2026-11-25 - the pragma that saved a definition, and the intrinsics that had no declaration

With the loop bugs fixed, `sqlite3.c` ran - and printed the right answer while printing *warnings* on every line of
every `windows.h` build. Both kinds were the same class: something this compiler ignored that changes what a program
means.

**`#pragma push_macro` / `pop_macro`.** mingw-w64 wraps every Windows declaration in them: save the name, redefine it,
put it back. They were in the "unknown pragma, ignored" bucket, so the redefinition stayed - which is a wrong program, not
just a warning, and the warning was on every line of every build that read `winnt.h`. They are now implemented: a stack
of saved definitions per name, `None` for "this was not a macro then" (which is what `minwindef.h` relies on when it
pushes `min` and `max`), and a `pop` with nothing pushed is refused rather than guessed at. The name is a string literal,
because that is how both compilers spell it.

**The intrinsics had no declarations.** `winnt.h` spells `MemoryBarrier` as `_mm_mfence`, `UnsignedMultiply128` as
`_umul128`, `InterlockedCompareExchange` as `_InterlockedCompareExchange`, and their prototypes live in mingw's
`intrin.h` behind `__MINGW_EXTENSION` and a compiler identification this compiler does not claim - so every use was
"not declared, taken to be a function returning `int`". The runtime implements fifteen of them; the bridge header
`x86intrin.h` now declares those fifteen, with signatures transcribed from `psdk_inc/intrin-impl.h` and `intrin.h` of
mingw-w64 12.0, the library the Windows SDK ships. Everything else stays undeclared and refused at the line it is
written, which is the `emmintrin.h` argument unchanged.

Measured: `xcc --std=c11 --opt-level=2 sqlite_driver.c sqlite3.c -o sq.exe` with `-I` on the SDK's headers now produces
**no output at all** - 265 977 lines compiled in 7 seconds with nothing to say - and the image prints
`6 one,two,three 3.51.3`, byte for byte what `clang -O2` prints.

207 unit (one more), 41 parser, 36 exe, 4 real, 1 crt, 6 invalid, 30 CLI, 5 hostile, 8 C API, 4 bcc20-sys; differential
**416** ran (cc51_loops.c), 0 mismatched; 444 corpus programs agree through the own back end, 0 wrong; `.text` 80 288
bytes over 445 programs; fmt and clippy clean.

### M19 - 2026-11-25 - sqlite3.c runs, and the four bugs between the symbol table and the answer

The symbol table named `sqlite3InsertBuiltinFuncs + 163`, the two initialiser bugs under it were fixed, and the fault came
back *named*: a `ud2` in `whereLoopAddBtree`. What follows is that chase, and the four bugs it found - one of them a
miscompiled loop, and three of them things this compiler had been doing wrong all along, hidden because a wrong constant
was branched on rather than believed.

**A `for` loop whose update ends in a join lost its way back.** `p = c ? 0 : p->pNext` is sqlite's loop over indices.
The update is read starting in the step block, and a conditional leaves the cursor in *its own* join block - so the step
block ends (in the branch into the arms) while the cursor is elsewhere. `for_statement` asked `terminated(step)` and got
"already ended", emitted no jump back to the test, and the join ended in a trap: the loop ran once and then `ud2`. It was
the only one of six loop questions in `stmt.rs` that asked about a named block instead of the cursor; every other one
already asked `current_is_open`.

**A block nothing can reach is not a block that traps on purpose.** `Term::Unreachable` meant both "the front end
abandoned this block" and "arriving here is undefined behaviour" (the fall-off of a non-void function), so no rule could
hold one without forbidding the other. They are now `Unreachable` and `Trap`, both encoded `ud2`, and the verifier holds
the first to "the entry cannot reach it" - which is the invariant that makes a reachable trap a compiler bug instead of a
crash. It found the `for` bug above by itself, with a span.

**Dead code is emptied by its own pass.** C code after an `if` whose every arm returned is legal, is read, and cannot
run; `dce` could then remove a definition in one dead block and leave its use in another, which the verifier refuses as
"used but no instruction defines it". `cir::pass::prune` empties unreachable blocks first - no instructions, one trap -
and the dominance check skips dead blocks, because dominance is a statement about paths and a dead block has none. Measured
side effect: `.text` over the 444 corpus programs fell from 80 336 bytes to 79 032.

**A constant condition has one side, and saying so removed a lie.** `do { } while(1)` branched to an exit no path takes,
which made the exit *reachable* to every reader of the graph - including the "control reaches the end of ..." warning,
which fired on `yy_find_shift_action` in `sqlite3.c`, a function that cannot reach its end. `constant_truth` decides one
branch at compile time; `1` jumps to the body, `0` to the exit. Two warnings from that build became none.

**And the fold that decided wrongly.** C99 6.3.1.8 puts the usual arithmetic conversions *before* the operator; the front
end's constant folder compared the two spellings instead, so `-1 > 0u` folded to false. That was invisible while a wrong
constant was branched on and the back end's own (correct) folder answered the runtime comparison - and it became a wrong
*answer* the moment a constant condition decided the branch. The fold now converts both operands to the common type
(`fold_conversion`, the same function that folds a cast) and does the operation in that type's width and sign: an unsigned
division is unsigned, an unsigned shift is logical, and a comparison sees numbers. The corpus had the cases - `-1 / 2u`
is 2147483647 - and cc50_corners.c failed on one of them, which is how this was found rather than argued.

**sqlite3.c, measured.** 265 977 lines, `xcc --std=c11 --opt-level=2 sqlite_driver.c sqlite3.c -o sq.exe` takes 7 seconds
and writes a 2.4 MB image; it opens an in-memory database, creates a table, inserts three rows, prepares a query with
`sum`, `group_concat` and `sqlite_version`, and prints `6 one,two,three 3.51.3` - byte for byte what `clang -O2`'s build of
the same two files prints.

206 unit, 41 parser, 36 exe, 4 real, 1 crt, 6 invalid, 30 CLI, 5 hostile, 8 C API, 4 bcc20-sys; differential 415 ran, 0
mismatched (cc51_loops.c is new); 443 corpus programs agree through the own back end, 0 wrong; `.text` 79 032 bytes over
444 programs; fmt and clippy clean.

### M18 - 2026-11-25 - the initialiser that was a constant, and the one that was zeroes

The symbol table named the fault: `sqlite3InsertBuiltinFuncs + 163`, a `movsbq` of `zName[0]` where `zName` was NULL. Its
table of 31 function definitions is a `static FuncDef aJsonFunc[]` inside `sqlite3RegisterJsonFunctions`, and **all 2496
bytes of it were zero in the image**. Three bugs were behind that, and only the first was the one being chased.

**An integer constant cast to a pointer is an address constant** (C99 6.6p9: an address constant "shall be created
explicitly using the unary `&` operator or an integer constant cast to pointer type"). `convert`'s constant path kept a
constant pointer *to* a pointer, and dropped one going *from* an integer - so `(void*)7` was a value with nothing in it.
sqlite writes every element of that table through `SQLITE_INT_TO_PTR`, which is `(void*)(intptr_t)`, so every element's
`pUserData` and `zName` was dropped. One condition, widened: a constant becoming a pointer stays the constant it was.

**A static local's initialiser that cannot be written is not the same as no initialiser.** `static_local` fell back to
zeroes with a comment saying the initialiser "has already said so" - and for `(void*)7` it had said nothing at all. Now a
present initialiser that produces no bytes is refused, in the file-scope path's own words (`a static local's initialiser
must be a constant expression`). This is the guard that turns the next silent zero into a diagnostic.

**A string literal is an object, so its value is its address** - the same shape a file-scope object's value has (ADR-004).
The expression reader gave a literal an address only when it was emitting code, so `(u8*)" "` - how sqlite builds its
one-character set - was a cast of a value that was nothing. `static const unsigned char * const azOne[] = { (u8*)" " }`
had no bytes either. The parser may now mint the literal's name in the *preprocessor's* interner (`Preprocessed::intern`,
`&mut Preprocessed` through `parse`), because one name table is the whole point: a compiler-made object needs a symbol
like any other, and `Value.address` stays "a symbol and an addend".

Measured: `sqlite_driver.c` + `sqlite3.c` now get **past `sqlite3_open`** and reach the query planner, where they stop at
a `ud2` in `whereLoopAddBtree` - a different bug, named by the same symbol table. Before: a SIGSEGV inside `open`.

206 unit, 41 parser, 36 exe (one more: the three shapes, run and compared), 4 real, 1 crt, 6 invalid (one more: the
refusals), 30 CLI, 5 hostile, 8 C API, 4 bcc20-sys; differential 415 ran, 0 mismatched; 443 corpus programs agree, 0
wrong; fmt and clippy clean.

### M17 - 2026-11-25 - the image names its own functions

`sqlite3_open` faulted at `.text+0x129e2d` and there was no way to say which function that is: the linked image had no
symbol table, so naming one function took `objdump -t`, arithmetic, and a guess. A compiler whose output cannot be
debugged is one whose bugs cannot be found, and this project had spent a milestone chasing a fault it could not name.

So the image now carries a COFF symbol table: `PointerToSymbolTable` and `NumberOfSymbols` in the file header, eighteen
bytes a record, a string table for names longer than eight, and every function and object the program holds, in address
order. Nothing maps it, the loader never reads it - it sits after the last section's bytes, which is why the header's
pointer and the writer's position are held together by one `debug_assert`.

Four decisions, once each:

**Derived, never stored.** `Program` already holds every name against the place it was given - `functions`, `objects`,
and each unit's `local` - so `Writer::symbols` reads those three and stores nothing. A second list would be a second
thing to keep right, and this project has already paid for that (the `&(f)` token range, the two ways of reaching an
offset).

**Local names count.** Almost every function in `sqlite3.c` is `static`. A table of external names alone would have
named almost nothing in the program this exists for. Two `static` functions of one name in two units are two symbols
with the same name at different addresses, which the format allows and which `objdump -t` prints.

**The compiler's own names do not.** `.str0` and the rest of the assembler's dotted names are bookkeeping: a table of
three thousand `.strN` entries buries the two thousand functions it was built for.

**No `--map`.** A text map would be the same fact in a second rendering, and its only consumer would be these eyes.
`objdump -t` reads the table and `gdb` uses it - an outside reader, which is this project's whole testing philosophy -
and both were checked on a real program before this entry was written.

The first use paid for it. On a 2.4 MB image of `sqlite3.c`, `gdb` said:

```
#0  sqlite3InsertBuiltinFuncs + 163
#1  sqlite3RegisterJsonFunctions ()
#2  sqlite3RegisterBuiltinFunctions ()
#3  sqlite3_initialize ()
#4  openDatabase ()
```

which is a call chain, not an offset. The test links a program with a `static` function whose name is longer than eight
bytes, a global object, and `main`, and reads the image's table back: section, type and storage class for each, the
string table's second case, and the address order the reader relies on.

206 unit, 35 exe (one more), 4 real, 1 crt, 30 CLI, 5 hostile, 8 C API, 41 parser, 5 invalid, 4 bcc20-sys; fmt and
clippy clean.

### M16 reviewed - 2026-11-25 - the warning on every link, and the lvalue that was not one

The aggregate `?:` landed and the review found two things, and the second is older than the feature that exposed it.

**A warning on every link.** `_mm_mfence` calls `__xcc_atomic_fence`, which the runtime defines two hundred lines
below it - so every linked program printed "`__xcc_atomic_fence` is not declared; it is taken to be a function returning
`int`". A call to a function with an `int` return that is actually `void` is the shape that breaks on a target where
return values travel differently; and a warning that appears on every single run is one a reader stops seeing, which is
how the three `kernel32` names in `b36a6f6` hid for a commit. The definition moved below the fence it calls.

**Two values that claimed to be lvalues.** `member_access` set `lvalue: true` on every member it produced, and a call
returning a record claimed the same - so `make().x = 1` and `(c ? a : b).x = 1` were accepted and wrote into an object
the program never named. C99 6.5.2.3p3 answers the first ("a member of an lvalue is an lvalue and a member of anything
else is not") and 6.5.2.2p5 the second. `make().x = 1` was wrong before the aggregate `?:` existed; the `?:` is simply
the first thing that made the question easy to ask, which is the usual reason an old bug gets found by new code.

**And the aggregate `?:` itself is right where it matters**: two arms that are calls, a compound literal, a variable;
chained and nested conditionals; the result passed by value twice from one expression and read through - six lines,
identical to `clang`. It carries the arms' addresses because a record's value already is its address (ADR-004), so the
join has something to carry after all; the refusal it replaces was right about `cir` and wrong about there being
nothing else.

sqlite3.c still compiles with 0 errors and still faults inside `sqlite3_open` - unchanged, and the reason it is hard to
chase is unchanged too: the linked image has no symbol table.

206 unit, 41 parser, 30 CLI, 5 invalid, 34 exe, 4 real, 1 crt, 6 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong;
`.text` 80 336 bytes over 444 programs; four fixtures byte-identical to `clang`; fmt and clippy clean.

### M16 - 2026-11-25 - sqlite3.c compiles, links, and faults inside `open`

**Yes, it compiles.** 265 977 lines, the Windows path, through the SDK headers: **0 errors**, a 2.7 MB object in 7.1
seconds. Getting the last three took two things.

**The `__PTRDIFF_TYPE__` family.** `sqlite3.c` casts an integer to a pointer through `__PTRDIFF_TYPE__` on any compiler
that has it, and falls back to `&((char*)0)[X]` on the others - so two errors were sqlite saying "I do not know what kind
of compiler this is, so here is the trick that works everywhere". `__PTRDIFF_TYPE__`, `__SIZE_TYPE__` and
`__WCHAR_TYPE__` are *true statements about this target's typedefs* (`long long`, `unsigned long long`,
`unsigned short`), and defining them is the same class of fix as `__NO_INLINE__`: a header asks, and the answer is a fact
rather than a claim.

**And the aggregate `?:`, which was a real feature.** `a ? b : c` where the arms are structs. The refusal was right that
`cir` has no register for an aggregate and wrong that there is nothing else to carry: **a record's value is already its
address** (ADR-004), so the join carries the `pointer to record` and the result is an rvalue of the record type whose
place is whichever arm ran. The object is alive for the rest of the full expression, which is as long as a value of
aggregate type can be used. Two tests asserted the old refusal; both now assert the answer, and the one that needed an
example of "what the emitter cannot express" uses `va_arg` of a record instead.

**It links** - after two more MSVC intrinsics in the runtime: `_InterlockedCompareExchange` (and the seven beside it,
which are the `lock cmpxchg` loop this runtime already had under other names) and `_mm_mfence` (`winnt.h`'s
`MemoryBarrier`, which is this runtime's own fence).

**It does not run.** `sqlite3_open(":memory:")` faults with a NULL dereference, while `clang`'s build of the same two
files runs the whole driver and answers `sum = 6`. `-DSQLITE_THREADSAFE=0` faults the same way, so it is not the mutex
path. The fault is at `.text+0x129e2d`, and naming the function there took an `objdump` of the object, a symbol table,
and arithmetic on the linked image - **because our linked image carries no symbol table at all**. That is the finding
this measurement leaves behind: a compiler whose output cannot be debugged makes every miscompilation a search, and the
next piece of work is a symbol table (or a `--map`) in the image, not another guess at sqlite.

205 unit, 41 parser, 30 CLI, 5 invalid, 34 exe, 4 real, 1 crt, 6 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong;
`.text` 80 336 bytes over 444 programs; four fixtures byte-identical to `clang`; fmt and clippy clean.

### M15 - 2026-11-25 - the first use is not the right use

A diagnostic about a token from a macro has nowhere to point but the macro's *body* - which is where the token was
written, and which cannot say which of seventeen uses is at fault. `sqlite3.c` is the case: `SQLITE_INT_TO_PTR` is
defined once at 14617 and used seventeen times, two of those uses fail, and the caret went to the definition twice with
nothing to tell them apart.

`PpToken::expansion` is now an index into a table of invocations, stamped as a token leaves an expansion, and the parser
adds `in `MACRO`, used at file:line:col` where every diagnostic passes. **The first version keyed the table by span and
was wrong in the way that matters**: every expansion of one macro produces tokens carrying the same body spans, so the
lookup answered with the first use for all of them - and the measurement showed it, both errors reported at use number
one. The key had to be the token, because the token is the only thing that knows which expansion made it.

**And it does not reach the case that started it, which is measured rather than assumed.** The note attaches when the
diagnostic's span belongs to the token just consumed or the one under the cursor - two candidates, which is enough for
"`undeclared_thing` is not declared" inside a macro body, and not enough for the `&` refusal: `&((char*)0)[X]` is
refused by the `&` after `X` has been read, so the caret is on the `&` and the cursor is two tokens past it. A wider
search would be a guess; the honest fix is for the *site* to travel with the span, which is a location type and a
refactor of every `error(span, ..)` call. That is the next step, and it is written down instead of approximated - the
seventeen uses cost a wrong answer once already.

206 unit, 41 parser, 30 CLI, 5 invalid, 34 exe, 4 real, 1 crt, 6 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong;
`.text` 80 336 bytes over 444 programs; four fixtures byte-identical to `clang`; fmt and clippy clean.

### M14 reviewed - 2026-11-25 - the addend that reached the linker and not the object

The addend went in and the review found that it went in **one of two places**. `GlobalRelocation`'s doc says the object
writer puts `A` in the field "because that is what an `IMAGE_REL_AMD64_ADDR64` means" - and nothing did. This compiler's
own linker read the struct and was right; every other linker read the bytes and got `S + 0`, which resolves `&table[4]` to
the *start* of `table`. The proof is a one-line experiment the review ran and the tests did not: `clang at.o -o x.exe`,
linking this compiler's object with clang's CRT. It printed `4 a 10 def` - the same as clang's own build - and with the
field at zero it would have printed `0 a 0 000`. **A feature that is only exercised by its own consumer is a feature
tested against itself**, and the test is now a unit test on the object bytes, so the two consumers cannot disagree again.

**And a silent wrong answer next to it.** `pointer_walk`'s constant branch folded a *non-constant* index to zero -
`index.constant.unwrap_or(0)` - so `void *p = &((char *)0)[f()];` at file scope compiled and wrote a null pointer where
the program meant an address. `clang` refuses it ("initializer element is not a compile-time constant"). The fold now
needs both halves to be constants, and the refusal comes from the initialiser, which is where the standard puts it. That
is the second time in two commits that an `unwrap_or` in a new branch turned "cannot know" into a value.

Checked and found sound: the negative addend path (`wrapping_add` of a negative `i64` as `u64`) is what `arr - n` needs
and wraps correctly; `member_access`'s new branch is behind `base.cir == None` so a body's member access is untouched;
`decay` keeps the constant only when the value had one, so an array's normal decay is unchanged; and sqlite3.c is still
3 errors - the addend work did not move it further, because the three left are the diagnostic problem named in PLAN.

206 unit, 40 parser, 30 CLI, 5 invalid, 34 exe, 4 real, 1 crt, 6 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong;
`.text` 80 336 bytes over 444 programs; four fixtures byte-identical to `clang`; fmt and clippy clean.

### M14 - 2026-11-25 - the addend, and the two halves of an address

The relocation had one thing to say about an address: the symbol. That is enough for `&x` and for `char *p = "hi"`,
and it is why `&sqlite3UpperToLower[256-OP_Ne]` was four errors in sqlite3.c: the address is a symbol **and how far into
it**. `GlobalRelocation` and `DataRelocation` now carry `addend: i64`; the linker answers `S + A`, and the object writer
puts `A` in the field because that is what an `IMAGE_REL_AMD64_ADDR64` means.

The parser half is the more interesting one, because *four* ways of reaching an offset all had to agree: a walk at file
scope (`&table[4]`), a member (`&points[1].y`), a cast on the way (`(const char *)&table[4]`), and a **constant base**,
where there is no symbol at all. That last one is sqlite's `SQLITE_INT_TO_PTR`, `&((char*)0)[5]` - an integer made into a
pointer, by C99 6.5.3.2p3's rule that the address of a dereference is the pointer behind it - and with a constant base the
walk folds to a constant, so the bytes go in directly and nothing is relocated. A `_Static` table of them is sqlite's
function registry.

That fold uncovered a rule that had been true only of this compiler's reach: `constant_bytes` said **"only the null
pointer is a constant"**. It now writes the number, and the test that asserted the old rule says why it changed - the
same shape as the `1L` fix a review ago, where a rule was right about what the compiler *could* do and wrong about C.

Three things were wrong on the way and each was found by running a program rather than reading: `subscript`'s no-code
arm kept the address and dropped the constant; `decay` kept neither the constant nor the symbol for a constant base; and
the cast's fold refused a pointer-to-pointer conversion because it only folds arithmetic to integer. Each is one arm of
one match, and the shape of the bug is the same every time - *a value has two halves and one of them was dropped on the
way through*.

sqlite3.c is at **3 errors**, and the measurement has stopped being able to help: two of the three point at the macro's
*definition*, not its use, so the diagnostic cannot say which of seventeen uses fails. That is the next piece of work,
and it is named in PLAN rather than guessed at.

205 unit, 40 parser, 30 CLI, 5 invalid, 34 exe, 4 real, 1 crt, 6 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong;
`.text` 80 336 bytes over 444 programs; four fixtures byte-identical to `clang`; fmt and clippy clean.

### M13 reviewed - 2026-11-25 - the other axis of a table, and the one it had never had

The hex-constant fix added the second axis to C99 6.4.4.1p5's table and the review found that **the first axis was
wrong too**: `1L` was typed `int` and `1UL` was `unsigned int`, because the candidates were written as `(signed, width)`
and the answer was mapped back through `integer_with(signed, width)` - which cannot tell `int` from `long` on a target
where both are thirty-two bits. That is the *same* fact a comment three hundred lines away in `finish_specifiers`
already spells out for declarations: "The *spelling* picks the type, not the size... a table keyed on `(signed, bits)`
cannot answer: it gave `long` the `int` type object." The lesson was written down in one place and repeated as a bug in
another.

The table is now the standard's, verbatim, as **names**: `1L` → `["long", "long long"]`, `0x10UL` →
`["unsigned long", "unsigned long long"]`, and so on down the nine rows. Whether a value fits is asked of the type
itself (`is `value` less than what a type of this width and signedness can hold`), so the widths are in one place - the
type table - instead of two. `_Generic` on seven spellings now answers what `clang` answers, and four of them are
`_Static_assert`ed in the test, including the two that were wrong.

**The comments were wrong with it.** The doc comment above the table said "`long` is 32 bits on this target, so `L` and
no suffix have the same shape" - true of the *width* and precisely the mistake: they have the same width and are
different types. It says that now.

Checked and found sound this time: the `captured_parameters` flag is reset by the outermost `declarator` *and* by the
group's recursive call, which is what makes `int (*f(vfs *p))(struct vfs *)` capture `p` and not the return type's
parameters; `TokenKind::Stray` is one ASCII byte by construction (every byte of a multi-byte character belongs to an
identifier), so the slice `push_scanned` interns cannot be a partial character; and a stray byte inside a macro body is
reported where the macro is *used*, which is where the standard puts it.

205 unit, 40 parser, 30 CLI, 5 invalid, 33 exe, 4 real, 1 crt, 6 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong;
`.text` 80 336 bytes over 444 programs; four fixtures byte-identical to `clang`; fmt and clippy clean.

### M13 - 2026-11-25 - sqlite3.c, and the three things it asked for in the first ten minutes

265 977 lines of SQLite, the Windows path, through the SDK headers: **8 errors, then 7** once the fourth was fixed -
where before this it could not be read at all. Each error was a fact about C this compiler had wrong, and the first
three took less than an hour between them.

**A stray byte is a token.** `sqlite3.c` keeps a page of Tcl inside `#if 0` - `foreach c {d s} { scan $c %c x }` - and
the lexer reported every `$` as a stray, 250 000 lines before the first line that mattered. That is translation phase 3
judging before phase 4 has decided what is in the program: the lexer now makes a `TokenKind::Stray` and keeps travelling,
and the preprocessor reports one it is about to hand on. `clang` accepts the file, and so do we. It is the same mistake
in miniature as every "the check lives in the wrong layer" bug in this codebase: the judgement was attached to the
cheapest place to make it, not the place that knows.

**The radix is part of the type of a constant.** C99 6.4.4.1p5's table has two axes and only one of them was implemented:
a *decimal* constant never takes an unsigned type, and a hexadecimal or octal one does - so
`0xfffffffffc000000LL` is an `unsigned long long`, and was refused as "too large for any integer type". The `decimal`
flag was already computed and already used for the no-suffix row; the `L` and `LL` rows ignored it.

**A declarator can hold two parameter lists.** `int (*getter(vfs *p))(struct vfs *)` takes a `vfs *` and *returns* a
function pointer. The group's recursive declarator was read with `capture_parameters: false`, so the function's own list
was never captured and only the return type's was - leaving `p` undeclared in the body, which is what the error said.
sqlite writes that shape for every VFS entry point. The fix is two lines and a flag: the group keeps capturing, and the
*first* list a declarator meets is the function's own.

**And one header, one struct.** Our `time.h` defines `struct timespec` and `struct tm`; the SDK's `sys/types.h` and
`time.h` define the same two under `_TIMESPEC_DEFINED` and `_TM_DEFINED`. Ours now says those words, so whichever is read
first, the other stays out - the same bridge idea as `vadefs.h`, in the other direction.

What is left is three, and they are the *address* and the *conditional*: `&array[index]` at file scope (sqlite's four
`sqlite3aLTb`-style tables), `&((char*)0)[X]` (its `SQLITE_INT_TO_PTR` for a compiler that is neither GCC nor MSVC), and
`?:` whose arms are aggregates (its parser's token select). All three are named in PLAN with their counts; the first two
need a relocation *addend*, which is the one thing ADR-004's design has never had.

205 unit, 40 parser, 30 CLI, 5 invalid, 33 exe, 4 real, 1 crt, 6 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong;
`.text` 80 336 bytes over 444 programs; four fixtures byte-identical to `clang`; fmt and clippy clean.

### M12 reviewed - 2026-11-25 - the reason that stopped being true in one commit

The intrinsics went in and the review measured something uncomfortable: **with `__NO_INLINE__` defined, no SDK intrinsic
is referenced at all**. A `windows.h` program's object has no `__shiftright128`, no `__stosb`, no `_umul128` in its
relocations - `__NO_INLINE__` removed the inline wrappers that used to name them, which is the same commit's other half.

So the block comment over those eight functions - "a compiler emits every one it reads, so a link needs them whether or
not the program calls one" - was **false the moment it was committed**, and it was the *justification* for the code
below it. The functions stay, because the SDK *declares* them and a program may write one (`_umul128` is how every
bignum and hash in C for Windows multiplies), but the comment now says that instead: what they are, that the signatures
are transcribed from `psdk_inc/intrin-impl.h` **and that nothing checks the transcription** - a wrong width there is a
call that disagrees with its caller's registers with no diagnostic anywhere - and that inclusion alone no longer needs
them.

Two of the same family's comments said the same thing twice, and the other one was the truer: "`memset` with an element
size" tells a reader what `__stosb` is; "the SDK's `RtlZeroMemory` and the threadpool helpers are made of them" was the
justification that had just expired. Merged into the shorter one.

**And the arithmetic got a harder test.** The first version checked four expressions; the review found sixteen edge
cases identical to `clang` - `_umul128(0xFFFFFFFFFFFFFFFF, 0xFFFFFFFFFFFFFFFF)`, `_umul128(0x100000000, 0x100000000)`
(the case whose *high* word a naive implementation gets right while its low word is wrong), the four sign combinations of
`_mul128`, `INT64_MIN * -1`, `INT64_MIN * INT64_MIN`, and both shifts at counts 1 and 63. Those sixteen lines are the
test now, so the two mistakes this arithmetic made once cannot come back quietly.

Checked and found sound: `CALLER_SAVED` is `r8`–`r11`, so `rcx` and `rdx` are never allocated and reading the shift
count into `rcx` clobbers nothing - the "fixed register the allocator does not know" hazard does not apply here; the
`__stos*` signatures match `psdk_inc/intrin-impl.h` (`__LONG32` is `long` here, so `__stosd` takes `unsigned long *`);
and `__NO_INLINE__` is read only by headers, so no library behaviour changes with it.

204 unit, 39 parser, 30 CLI, 5 invalid, 33 exe, 4 real, 1 crt, 6 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong;
`.text` 80 336 bytes over 444 programs; four fixtures byte-identical to `clang`; fmt and clippy clean.

### M12 - 2026-11-25 - `windows.h` runs, and the two arithmetic mistakes inside it

`#include <windows.h>` links and runs. Getting there took three things, and the largest was not the one I had planned.

**One macro.** `__NO_INLINE__`, predefined, and it is *true*: this compiler has no inliner, so a header's inline
wrapper becomes a symbol rather than being baked in. mingw-w64's `stralign.h` is the case that proved it -
`ua_CharUpperW` is `__CRT_INLINE` and calls `uaw_CharUpperW`, which **x64 has never had** (the file's own TODO says
so) - so a program that merely *included* `<windows.h>` could not link, twice over: once for the intrinsic
`__shiftright128`, then for a helper that does not exist. The reachability work I had named in PLAN would also have
fixed it; a True statement about the compiler was cheaper and does not need the linker to know which code is dead.

**Eight intrinsics, in C.** `__shiftright128`, `__shiftleft128`, `__stosb`/`w`/`d`/`q`, `_umul128`, `_mul128`. They are
*arithmetic*, not instruction encodings - the compilers that own them make them `shrd`, `mul` and `stos` for speed, and
a program cannot tell a call from an instruction except by how long it takes. Every one is checked against `clang`
line for line, and two were wrong first:

- `__shiftleft128` returned the **low** half's expression; it is the *high* half of `(high:low) << n`, so
  `0x1111…:0x2222…` by 4 came out `0x1111…12` where `clang` says `0x2222…21`.
- `_umul128` dropped the `al*bh` term from its **low** word - the high word uses the same `t3` and came out right,
  which is the shape of bug a plausible-looking test misses and a line-by-line comparison does not.

**And a back-end bug they found.** `(low >> n) | (high << (64 - n))` in a function with parameters panicked the
encoder: *an operation is holding both scratch registers already*. A function with parameters *and* a shift has an
empty register pool - that is the design, everything spills - so both operands of the shift are in memory, and `shift`
read the count into a scratch as well and then wanted a third one for the destination. The count's scratch was never
needed: the instruction reads the count in `rcx`, so it is read *into* `rcx`. Two operands, two scratches, and the
panic is gone. That is the fourth time a fixed register and the allocator have disagreed, and the first time the
compiler's own runtime was the program that found it.

204 unit, 39 parser, 30 CLI, 5 invalid, 33 exe, 4 real, 1 crt, 6 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong;
`.text` 80 336 bytes over 444 programs; four fixtures byte-identical to `clang`; fmt and clippy clean.

### M12 - 2026-11-25 - the SDK's headers, read as C

`#include <windows.h>` compiles. It was 86 errors, then 66, then 14, then 5, then 2, then none - and **every step was a
fact about C this compiler had wrong**, not a compromise with the SDK:

| what the SDK writes | what it is | what this compiler did |
|---|---|---|
| `MACRO_EXPANDS_TO_NOTHING;` | an empty declaration | refused it: `expected a type specifier` |
| `unsigned __int64` | `unsigned long long` | did not know the word |
| `extern __inline__` | `extern inline` | did not know the word |
| `struct DECLSPEC_ALIGN(16) _M128A` | a 16-aligned struct | the attribute was already dropped; the spacing was fine |
| `#pragma pack(push, _CRT_PACKING)` | `pack(push, 8)` | read the *name* as the limit |
| `__attribute__((__returns_twice__))` | a promise about control flow | refused it by name |
| `winnt.h`'s `#if defined(__x86_64)` | which architecture | we define `__x86_64__`, and it asks the other spelling |

That last one is why the list is worth reading twice: an SDK picks its `CONTEXT` definition by testing a macro
*spelling*, `__x86_64` without the trailing underscores that GCC and clang use, and a target that answers only the GCC
spelling gets no `CONTEXT` - and then a hundred declarations that name it fail, which is what "86 errors" partly was.

**Two bridge headers.** A system SDK has to know how the compiler spells `va_list` before it can declare a variadic
function, and mingw-w64's `vadefs.h` asks whether it is GCC, clang or MSVC and says `#error VARARGS not implemented for
this compiler` about anything else. Neither answer is true, so this compiler ships a `vadefs.h` of its own - which
shadows the SDK's, because its include guard *is* the SDK's - that says what is true: `va_list` is a `char *` on this
target, and the SDK's four `_crt_va_*` names are this compiler's `stdarg.h` macros. The empty `emmintrin.h` and
`x86intrin.h` are the same idea with a duller edge: `winnt.h` includes them to spell two macros, this compiler declares
no intrinsics, and an empty header means a program that *writes* one is told the name is not declared rather than being
refused for a header it never used.

**And the include order, which had to change.** The compiler's own headers are now read *before* the host's `-I`
directories. That is the opposite of what a C++ compiler does, and it is the standard's reading of `<...>` - the places
the *implementation* defines, then any the programmer adds. Without it, `-I <the SDK's include directory>` shadows
`string.h` and `stdarg.h` with mingw's own, which are written for a compiler with a CRT it does not have; with it, one
directory of SDK headers is usable beside one runtime.

**Linking to the SDK is capturing and asking.** Nine libraries (`kernel32`, `user32`, `gdi32`, `advapi32`, `shell32`,
`ole32`, `oleaut32`, `comctl32`, `ws2_32`) are captured the way `ucrtbase` was - 6 364 names - and the linker asks the
export tables **first**. That order is the `TlsAlloc` lesson twice over: `C_RUNTIME` lists the names this compiler's own
runtime *calls*, so answering "which library exports this" from it sent `TlsAlloc` to `ucrtbase`, and every linked
program failed to load with `0xC0000139` until the tables were asked first. A name and a library are two facts, and only
the DLL owns the second.

**The segment read is an instruction, so it got an instruction.** `__readgsqword(0x30)` is `NtCurrentTeb()`, and half of
`winnt.h`'s inline functions are a field of it, so the eight spellings are one CIR instruction, one `Op`, four widths in
the encoder (`mov rax, gs:[offset]`), and the narrow ones canonicalised exactly as a `cmpxchg`'s result is. The proof
has an oracle instead of a plausible number: the TEB's client id, 0x48 bytes into what `gs:0x30` points at, equals the
thread id `GetCurrentThreadId` returns - and that function is imported from `kernel32` through the captured exports by a
program that declared it by hand, so the test needs no SDK and no machine-specific path.

**What is left is one symbol and one idea.** `windows.h` compiles but does not link: `__shiftright128` is called by an
SDK inline function nothing reaches, because this compiler emits every `extern __inline__` definition a header contains
and a link then requires every intrinsic any of them mentions. The fix is the one a linker with whole-program vision
should have - require an import only for a name reachable from the entry point - and it is named in PLAN rather than
papered over.

204 unit, 39 parser, 30 CLI, 5 invalid, 32 exe, 4 real, 1 crt, 6 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong;
`.text` 80 336 bytes over 444 programs; four fixtures byte-identical to `clang`; fmt and clippy clean.

### M11 reviewed - 2026-11-25 - the object two units agree about, and the one they do not

The duplicate check went in and the review found the case it had left open: **two tentative definitions of different
sizes**. C99 6.9.2p2 lets `char buf[8];` appear in every unit, and the one object they mean has one size - so a unit
that declared `char buf[64];` writes 64 bytes into an 8-byte object. The linker kept the first one's bytes and said
nothing, and `clang -fcommon` (the behaviour the standard describes) resizes the object to the largest, which is the one
thing this linker cannot do: the address is already placed against the first unit's bytes.

Whether such a write lands on another object is a *layout* question - the two attempts to demonstrate corruption on a
neighbouring global both landed on bytes nothing used, because `.data` order is not declaration order - which is exactly
why it must be refused rather than measured: a program whose correctness depends on where the linker put an object is
one this compiler must not build. Two tentatives are one object only when they agree about its size, and disagreeing is
now a diagnostic naming both sizes.

That is also the difference between the two exemptions, and it took the review to see it: a tentative may appear in
every unit *because* the standard says the units describe one object; they cannot describe two different ones. The
merge stays, and it got stricter in the only place where merging is a lie.

Checked and found sound: the same-size merge still links and runs (`shared` written in one unit, read in another), the
order-independence of the four duplicate cases is untouched, `tentative` is still derived rather than stored (`external
&& init.is_none()`), and nothing about it reaches the JIT or `-c`.

201 unit, 38 parser, 30 CLI, 5 invalid, 31 exe, 4 real, 1 crt, 6 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong;
`.text` 80 336 bytes over 444 programs; four fixtures byte-identical to `clang`; fmt and clippy clean.

### M11 - 2026-11-25 - the first body wins, and the two cases that are not duplicates

The linker kept the first of two definitions and said nothing. `clang` says "multiple definition of `f`" and refuses,
and the difference is not stylistic: whichever unit came first wins, so the program is the one written only when the two
definitions are identical. Found by accident while building toml-c, whose single-header distribution is meant to be
compiled alone - `toml.c` and `example/table.c` together defined 28 names twice and produced a working executable, which
is the worst kind of evidence.

The check itself is four cases, and **the first version of it was wrong in the way order-dependent checks are**: it
tracked "real definitions" and "tentative definitions" in two sets and asked each set separately, so a tentative in unit
A beside a real definition in unit B was refused one way round and accepted the other. The sets are asked together now -
a name is a duplicate if it is defined for real twice, or if a real definition meets a tentative one - and the state
lives outside both loops, because every unit's functions are gathered before any unit's objects and a name checked
against half of them would be checked wrongly.

**The one exemption is the standard's, and it needed no new field.** C99 6.9.2p2: `int shared;` with no initialiser is
a *tentative* definition, not an external definition, and a program may have one in every unit. In this compiler that
predicate is already spelled `external && init.is_none()`, so the object writer reads "tentative" off the global it
already had rather than the parser growing a flag. Two tentatives merge, and the program runs.

That is also the one place this linker is now **more permissive than the `clang` beside it**: `-fno-common` makes each
tentative a real definition, so clang refuses two of them. Refusing valid C would be the gap, so the merge stays - and
because a divergence nobody wrote down is a trap, PLAN's closed row and this entry both say where and why.

The message names the rule and the usual cause, because the usual cause is structural: a function defined in a header,
or a `.c` file compiled beside a unit that already includes it - which is exactly what toml-c's single-header build had
been doing silently.

201 unit, 38 parser, 30 CLI, 5 invalid, 31 exe, 4 real, 1 crt, 6 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong;
`.text` 80 336 bytes over 444 programs; four fixtures byte-identical to `clang`; fmt and clippy clean.

### M11 reviewed - 2026-11-25 - the attribute that became the record's, and the one the range began after

The matrix from the previous commit matched `clang` on all eight of its rows, and the review found two bugs that the
matrix could not see - both in the same place, both silent.

**A member's `packed` became the record's.** `record_layout` asked for every attribute in the record's range, and a
member's attribute is inside that range. `struct { char c; int i __attribute__((packed)); short s; }` came out 7/1/5/1 -
packed throughout, alignment 1 - where `clang` says 8/1/6/2: only `i` is packed. The eight measured rows could not show
it because the one row with a member `packed` had nothing after it, where both readings agree. The range is now the
record's *own* fence: before the `{` and immediately after the `}`, which is where both spellings of a record attribute
are written, and never inside.

**An attribute before the member's type was dropped.** `__attribute__((aligned(8))) int i;` inside a struct is 16 bytes
with `i` at 8 in `clang`; the member's range started *after* the specifiers, so the attribute was outside it and
`sizeof` came back 8 with `i` at 4 - silently the natural layout. The range now starts before the specifiers, and the
per-declarator *end* stays where it was, which keeps `int a, b __attribute__((packed))` meaning what it says: the
attribute is after `b`, so `a` never sees it.

Both are the same failure of the same idea, and it is worth naming: **a range is only as good as its ends**, and this
commit's ends were "the whole record" and "from the declarator". The positions were right - an attribute is a fact about
where it is written - and the fence around them was not.

The test could not catch either, which is the more important finding. Its matrix had one row per *kind* of attribute and
none for the *fence*: a member attribute with a member after it, and an attribute before a type. Both rows are in the
program now, with `clang`'s numbers - `one_member 8 1 6 2` and `attribute_first 16 8 8` - so the two readings are
distinguishable from the inside.

Checked and found sound: `#pragma pack` is untouched by any of it (five matrices re-run against `clang`, identical),
`aligned` on an object still reaches `_Alignas` from either side of the declarator, and the pp's index convention - an
attribute is recorded at the position of the *next* token - is what makes the inclusive range end work.

201 unit, 38 parser, 30 CLI, 5 invalid, 30 exe, 4 real, 1 crt, 6 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong;
`.text` 80 336 bytes over 444 programs; four fixtures byte-identical to `clang`; fmt and clippy clean.

### M11 - 2026-11-25 - the two spellings that are not the same decision

`__attribute__((packed))` and `((aligned(N)))` are implemented on a record, on one member and on an object, against a
matrix from `clang` - all eight rows identical - and the interesting part of it is that **they are not `#pragma pack`
with another spelling**:

| | size | offset | align |
|---|---|---|---|
| `struct {char,int} __attribute__((packed))` | 5 | 1 | 1 |
| `struct {char; int i __attribute__((packed));}` | 5 | 1 | 1 |
| `struct {char} __attribute__((aligned(16)))` | 16 | — | 16 |
| `struct {int} __attribute__((packed, aligned(8)))` | 8 | — | 8 |
| `struct {char; int i __attribute__((aligned(8)));}` | 16 | 8 | 8 |
| `struct {_Alignas(16) char; int} __attribute__((packed))` | **16** | 1 | **16** |

The last row is the whole difference: `#pragma pack(1)` caps a `_Alignas(16)` member to 1 (measured in the previous
commit, 5 bytes, alignment 1) and `__attribute__((packed))` leaves it at 16. So the layout takes the explicit alignment
*after* the packed cap and *before* the pragma's - `max(natural-capped, explicit).min(pack)` - which is one expression
that reproduces both spellings, and `aligned` is a floor applied after it (`packed, aligned(8)` is 8 bytes, alignment
8).

**How it reaches layout** is the design question, and the answer is the same one `#pragma pack` got: a fact about a
*position* in the token stream. The preprocessor records `(token index, attribute)`; the parser asks about the range it
parsed. That is what makes `struct S { ... } __attribute__((packed))` work at all, because the attribute comes *after*
the `}` and the record is completed *at* it - a parser that met attributes where it expects them would need a case in
every position a declaration can carry one, which is the class of bug this compiler keeps finding. The argument reader
(`aligned(8)` rather than `aligned(8 + 8)`) runs in the same pass, because the parentheses belong to the name.

Two smaller decisions: `aligned` on an object is `_Alignas` (16 is the ceiling, refused above it with the same
reasoning and now the same wording), and a declaration's attributes are asked for over the *whole* declaration - so
`int g __attribute__((aligned(16)))` and `__attribute__((aligned(16))) int g` are one question. Over-aligning an object
is not a wrong program, which is why a declaration with two declarators sharing one attribute is left over-applied
rather than tracked per declarator.

201 unit, 38 parser, 30 CLI, 5 invalid, 30 exe, 4 real, 1 crt, 6 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong;
`.text` 80 336 bytes over 444 programs; four fixtures byte-identical to `clang`; fmt and clippy clean.

### M11 - 2026-11-25 - packing is one line, and the note that had measured the wrong struct

`#pragma pack` is implemented: all five forms MSVC documents, against a matrix from `clang`. The whole of packing is
**one line** - `complete_record` caps a member's alignment at the limit, `.min(pack)`, and every offset, the record's
own alignment and its size follow from it. `{char,int,short}` is 12/4/8 unpacked, 7/1/5 under `pack(1)` and 8/2/6 under
`pack(2)`; a `double` member moves from 8 to 4 under `pack(4)`; a bitfield unit's alignment is capped the same way, so
one struct is 5 bytes packed and 6 under `pack(2)` against 8 unpacked. `pack` caps an explicit `_Alignas(16)` as well
(6 bytes, align 2 under `pack(2)`), which MSVC's own `align` attribute would not - measured, not assumed.

**The design question was how the limit reaches layout, and the answer is that a pragma is a fact about a position in
the token stream.** The preprocessor owns the pragma's grammar - the `push`/`pop` stack, exactly as it owns the
conditional stack - and records `(token index, limit)`; `Preprocessed::pack_at(index)` answers for a position. So the
parser asks "what limit applies where this body ends" and no loop that reads a declaration has to know a pragma exists,
which is a class of *unknown unknowns* removed rather than three call sites guarded. The one thing this cannot express
is a pragma *inside* a struct body, where different members would get different limits: refused by name.

**And the note in PLAN was wrong, in the way that matters most - it was a measurement.** It said packing "reaches
nested records", and gave a number: a `struct inner` of `{char,int}` inside a `pack(2)` struct is 6 bytes, so the outer
is 8. The truth is that `struct inner` was *declared inside* the `#pragma pack(2)` region in that program, so its own
declaration was packed; the measurement that separates the two readings is `offsetof(outer2, in.i)`, which is 2+4 and
not 2+2. A record is laid out once, with the limit in force where it is declared. Had the note been trusted, the
implementation would have re-laid out nested records - work that is not only unnecessary but wrong.

The proof is not a `sizeof` table alone: the test's last three lines pass a packed struct **by value** through the ABI,
index an array of them, and read one back as raw bytes (`memcpy`), which is where a wrong offset stops being a number
and becomes wrong memory. Eleven lines, byte-identical to `clang`.

201 unit, 38 parser, 30 CLI, 5 invalid, 29 exe, 4 real, 1 crt, 6 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong;
`.text` 80 336 bytes over 444 programs; four fixtures byte-identical to `clang`; fmt and clippy clean.

### M11 reviewed - 2026-11-25 - the list that decided "unknown means harmless"

Reading the attribute reader a second time found the shape this codebase keeps finding, in its most dangerous form.

**It was a denylist of two.** `__attribute__((packed))` and `((aligned))` were refused and *everything else was dropped* -
so `vector_size(16)`, which decides a type's size; `section(".text.hot")`, which decides where its bytes go;
`cleanup(free)`, which decides what runs at the end of a block; and `weak`, which decides whether a definition wins, all
became nothing, silently. Measured on the built compiler: all four accepted. The default for an unknown attribute was
"harmless", and an attribute nobody had thought about is precisely the case a compiler cannot answer.

The list is now what *may* be dropped - `HARMLESS_ATTRIBUTES`, whose criterion is "carries no meaning here" and whose
comment says why each group qualifies: no garbage collection (`used`), no dynamic linking (`visibility`), one calling
convention, no optimiser to guide (`always_inline`, `pure`, `cold`), no `format` diagnostics. Anything else is refused
*by name*, with a message that says where the list is. That is the honest default: a diagnostic costs the writer one
line and cannot be wrong about a program.

**And the same question for pragmas found a second one.** `#pragma pack` was refused in the previous commit; `#pragma
weak` - seven files in the local corpus - had the same fault: ignoring it makes the definition here the one that wins,
which is the wrong program whenever it was written to be overridden. Refused by name, and the two are now the whole list
of pragmas whose ignorance is a wrong binary rather than a missing feature. `#pragma GCC diagnostic` (443 files) and
`#pragma comment(lib, ...)` (33) stay warnings, correctly: one is diagnostics and the other is an import a static link
would refuse anyway.

**Two of my own documents described the rule I had just replaced** - PLAN's closed row and the `C99-SUPPORT` cell both
still said "dropped except `packed`/`aligned`" - and both say what the code does now.

Also checked and found sound this time: `cargo package --list` ships `ucrtbase-exports.txt` (an `include_str!` that is
not in the package is a publish-time failure, not a build-time one), the capture is 2 484 names sorted and unique as the
tool writes it, and `Func::reachable` is one allocation per function definition, asked once.

201 unit, 37 parser, 30 CLI, 5 invalid, 28 exe, 4 real, 1 crt, 6 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong;
`.text` 80 336 bytes over 444 programs; four fixtures byte-identical to `clang`; fmt and clippy clean.

### M11 - 2026-11-25 - the pragma that was a warning, and the attribute that was a parse error

`#pragma pack` was ignored with a warning, and the layout underneath changed. Measured: `struct { char c; int i; }` is
8 bytes naturally and 5 under `pack(1)` - so every packed struct in a program had the wrong offsets, which is a
program that reads and writes the wrong bytes, with one line of output saying so. **A warning on a program that then
miscompiles is the same silent wrongness as no warning at all**, and it is now an error naming the reason.

`__attribute__` was worse in a smaller way: refused outright, with "expected a type specifier", and **2 304 of the
95 119 C files on this machine use it** - most of them for `unused`, `noreturn`, `always_inline`, `visibility`, which
change nothing this compiler implements. Reading it is now a *token* decision, taken in the preprocessor: it is a token
sequence that means nothing in every position a declaration can carry one - before it, after its declarator, on a
member, on a parameter, after a struct's `}` - and the parser would need a case in each, the one it forgot being a
parse error on a program that asks for nothing. Two of the attributes do change a layout this compiler computes, and
they are refused by name: `packed` and `aligned`, 528 files of the 2 304.

The reason packing is refused rather than implemented in this commit is the measurement that goes in PLAN: it reaches
**nested** records (`struct inner` re-laid out with its members capped at 2 inside a `pack(2)` struct: 6 bytes, outer
8, not 10) and **bitfields** pack differently (5 bytes under `pack(1)`, 6 under `pack(2)`, 8 natural). A layout that
depends on global state at the point of a declaration is not a fact `Types` can cache once per record, so the honest
refusal is the right shape until that is designed.

A synthetic program with all four ignorable attributes prints what `clang` prints (`2 42`, then `exit(1)` from a
`noreturn` function's `exit`). The four fixtures are unchanged, which is the check that the attributes they guard behind
`__GNUC__` stayed guarded.

201 unit, 37 parser, 30 CLI, 5 invalid, 28 exe, 4 real, 1 crt, 6 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong;
`.text` 80 336 bytes over 444 programs; four fixtures byte-identical to `clang`; fmt and clippy clean.

### M11 - 2026-11-25 - the import list that was a copy, and the `main` that trapped at exit

`strnlen` is exported by `ucrtbase.dll` and C99's `<string.h>` does not declare it. toml-c declares it itself,
because every POSIX platform does - and this compiler answered "undefined symbol `strnlen`", which blames the
program for a name the runtime has. `clang` links the same source and prints 3.

`C_RUNTIME` is *what the shipped headers declare* - the JIT binds exactly those, and a test walks it - so it was
never going to grow by a name nobody's header mentions. The list the linker needed is the library's own: 2 484
exports, captured once by `tools/pe_exports.py` (which reads a PE export table, and had the same class of bug the
linker did - it read `AddressOfFunctions` where `AddressOfNames` belongs, so every "name" was code bytes) and checked
in as `src/back/ucrtbase-exports.txt` with the version and size of the DLL it came from. A link still reads no DLL:
that is the design, and a capture is how a fact the DLL owns crosses into the source without the link depending on
the machine. The headers stay the first answer, so nothing that worked changes; the export table is the second.

Then toml-c's example ran, and crashed - `ud2`, an illegal instruction, at exit. `example/table.c`'s `int main()` has
no `return`, and **C99 5.1.2.2.3p1 says reaching the `}` that ends `main` returns 0** - it is the one function for
which falling off the end is defined. Every other non-void function got `unreachable` for the undefined value it
would return (a defensible choice, since C99 6.9.1p12 makes the value undefined), and `main` was swept up with them:
`int main(void) { }` alone aborting with SIGILL was the minimal reproduction.

Two things came out of fixing it. The diagnostic that now warns about a real fall-off fired on fifty of the runtime's
own atomic loops, whose tails sit after a `for (;;)` - reachable is not the same as terminated, so the question is
asked of the CIR (`Func::reachable`, which is `reverse_postorder` given a name) and an unreachable tail is silent. And
the warning is a *warning*: the function is legal C, only the call's value is undefined, exactly as `clang` says.

Building the example also exposed a third thing, recorded not fixed: compiling `toml.c` and `example/table.c` together
defines 28 names twice and the link succeeds, taking the first of each - `clang` says "multiple definition" and
refuses. The comment in `pe.rs` claimed a `-c` build reported it, which was never true.

toml-c's example is byte-identical to `clang`'s: `example.com:80`, `key #0: key`, `key #1: sub`.

201 unit, 37 parser, 30 CLI, 5 invalid, 28 exe, 4 real, 1 crt, 6 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong;
`.text` 80 336 bytes over 444 programs; four fixtures byte-identical to `clang`; fmt and clippy clean.

### M10 reviewed - 2026-11-25 - the predicate twice, and the number that was already a character

Reading the identifier scanner a second time found two design smells and no bugs - and one of the two smells was the
kind this codebase keeps finding.

**"Can begin an identifier" was written twice.** The token loop said `is_ident_start(byte) || byte == b'\\' || byte
>= 0x80`, and the scanner said `is_ident_continue(byte) || byte >= 0x80` - the same three ideas, one with digits, and
the high-byte clause stated in both. There is one predicate now, `starts_identifier`, and the continuation rule is
`starts_identifier(byte) || byte.is_ascii_digit()`, which is the only place that asks. A second list would have had to
be told what `\` and `>= 0x80` mean all over again.

**The reader answered a number and the caller kept asking whether it was a character.** `universal_character_name`
returned `u32`, checked against a surrogate range and `0x10FFFF` by hand, and then every caller asked
`char::from_u32` anyway - one caller with `map_or("", ..)`, which is a *silent skip* of a character if the range check
ever disagreed with the library. It returns `char` now: the two clamps are gone because `char::from_u32` is the
rejection, and one impossible branch went with them. The function is named `universal_character_name`; its type now
says the same thing.

**Two comments described checks the code does not make.** The diagnostic's span covers four digits when they were hex
and wrong, and only `\u` when a digit was not one - the reader stops at the first digit it would have had to read -
and the comment claimed it covered the digits. The literal reader's comment listed all three rules under a check that
makes one of them. Both say what is true now.

What the review also measured, and found sound: an identifier with a UCN works as a macro name, as both halves of a
`##` paste, as a struct tag, as a label, and as a `static` name long enough to reach the COFF string table - 13 bytes
of `caf` and four `é` - with the same output as `clang`. And the diagnostic that says a UCN may not spell `$` cites
the standard, not this compiler.

201 unit, 36 parser, 30 CLI, 5 invalid, 26 exe, 4 real, 1 crt, 6 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong;
`.text` 80 336 bytes over 444 programs; four fixtures byte-identical to `clang`; fmt and clippy clean.

### M10 - 2026-11-25 - the name that decodes, and the bytes after it

C99 6.4.2.1 puts a universal character name inside an identifier and 6.4.3p2 makes it the *same* identifier as the
character it names. So the lexer decodes one: the token's spelling becomes the character rather than the six bytes of
the escape, which is why `caf\u00e9` and `café` intern to one symbol. The literal reader is shared, so the four rules -
four or eight hex digits, at or above `0xA0`, not a surrogate, inside the code space - are in one place; the three
characters below `0xA0` that a name *may* spell (`$`, `@`, the backtick) are refused here as not part of an identifier.

Three things came out of writing it.

**A decoded name stopped at the escape.** `caf\u00e9` worked, so the first measurement looked like success - and it is
the shape that hides the bug, because nothing follows the name. `a\u00e9b` declared `aé` and then looked for `aéb`, so
the two spellings disagreed and the *declaration* was the one that lost bytes. The scanner appended the decoded character
and then kept advancing `self.at` without appending what it passed.

**A lexical refusal could not be tested.** `refuses` went through `check`, whose first assertion is that the source
*reaches the parser* - which a lexer error never does. The helper now asks the compiler directly, because the question
is what it said, not how far it got.

**And the guard was worth keeping**: a source that does not preprocess is a test that is wrong, and that assertion is
still there for `parses`.

Honest about the one divergence: any code point at or above `0xA0` is an identifier character here, where `clang` asks
Unicode's XID tables and refuses an emoji. The complete rule is several thousand ranges; a name `clang` would refuse is a
portability trap worth a sentence in PLAN, not a dependency. The *reverse* - refusing a valid name - would be a gap, and
that is why the boundary went where it did.

201 unit, 36 parser, 30 CLI, 5 invalid, 26 exe, 4 real, 1 crt, 6 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong;
`.text` 80 336 bytes over 444 programs; four fixtures byte-identical to `clang`; fmt and clippy clean.

### M10 reviewed - 2026-11-25 - the rule the standard has three times, and the list that decides what a declaration is

The four gaps were closed with a full gate behind them and four findings came out of reading the diff.

**C99 6.4.3p2 has a third rule** - a character whose short identifier is below `0xA0` is spelled as itself, so a
universal character name may not name one, except `$`, `@` and the backtick. The new reader had the range and the
surrogate rule and not this one, so `"\u0041"` compiled to `"A"` here and is an error in `clang` ("character 'A' cannot
be specified by a universal character name"). Measured in all four kinds of literal - narrow, UTF-16, UTF-32 and a
character constant - refused in the one place all four read from. **A rule with three clauses is where a fourth
usually waits**, so the "what is a character" question now has one answer with all of it in it.

**`_Imaginary` was refused at the right moment and pointed at the wrong word.** The diagnostic used `self.span()`,
which by then is the token *after* the specifiers, so `double _Imaginary z` blamed `z`. Refusing where the word is
read - as `_Alignas` next door already did - fixes the span and deletes the block in `finish_specifiers` with it.
**And at block scope it never got that far**: `_Imaginary float z;` said "`_Imaginary` is not declared", because
`SPECIFIERS` is the list that decides whether a statement is a declaration, and it did not have the word. Two lists
knowing one thing, which the comment above `SPECIFIERS` already warns about. The test that guards it walks *that*
list, so a word missing from the list is a word the test never tries - and my first version asserted only that
compiling *failed*, which "not declared" also does. The test asserts the reason now, which is the half that fails on
the interesting mistake.

**The 8-bit `cmpxchg` emitted its REX byte only for an extended register.** That is right for 32- and 64-bit forms
and wrong for the byte form: without a REX prefix `r4` to `r7` name `ah`, `ch`, `dh` and `bh`, which are different
registers. Nothing miscompiles today - the scratch pair is `r10`/`r11`, or `rcx`/`rdx` with a pool that starts at
`r8` - and that is the whole problem: the encoder would depend on an invariant about the register plan that neither
file states. The byte form now always emits it.

**A comment that described the branch two below it.** The new universal-character-name branch was inserted between
"A numeric escape is one or more digits and nothing else" and the numeric escape it describes. Moved back.

Honest about what a reader will still see: a universal character name that names a character below `0xA0` is refused
with the same message as an escape that is not read at all, because the four readers get one `None` and cannot tell
the two apart. The reason is in the code and in PLAN; a third answer through four call sites is not worth a message
for a spelling that means "write the character instead".

201 unit, 35 parser, 30 CLI, 5 invalid, 25 exe, 4 real, 1 crt, 6 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong;
`.text` 80 336 bytes over 444 programs; four fixtures byte-identical to `clang`; fmt and clippy clean.

### M10 - 2026-11-25 - the four gaps, and the one that had to be closed twice

The four things the last report listed as open are closed. Three were small; the first was small and the second half
of it was not.

**`&(f)` was the shape the lookahead could not see**, and the fix is a question about the operand's *token range* -
parentheses, a function's name, the matching closes - rather than about the token in front of it. **And the rewrite
brought the old bug back for one commit**: a range check that asks only what the range *is* accepts `&mk` in
`&mk(1)->d`, because the call's `(` belongs to nothing the check looked at. What must also be true is that *nothing
follows* the range, and the lz4 test failed exactly as it did in July. Both halves are in now, with `&(g)`, `&((g))`
and `&mk(1)->d` in the parse suite.

**`_Imaginary` is refused by name.** C99 6.2.5p11 has three imaginary types, C11 removed them, and `clang` says
"imaginary types are not supported" in every dialect - so refusing is the oracle's answer, and the message says what
such a program means (`_Complex` with a zero real part). The old behaviour was a parse error two tokens later.

**Universal character names** - `\uXXXX` and `\UXXXXXXXX` - are read where a character is: a narrow literal holds the
character's UTF-8 bytes (so `"caf\u00e9"` is five bytes, not six), a `u` literal its UTF-16 units, a `U` literal its
code point, and a surrogate or a value past `0x10FFFF` is refused rather than becoming something else. Four readers
needed it - narrow, UTF-16, UTF-32 and the character constant - which is now one `universal_character_name`.

**And the narrow atomics**: `lock cmpxchg` has a form for one byte and for two, so `_Atomic char` and `_Atomic short`
are real atomics. The interesting half is the *result*: `cmpxchg` writes only `al` or `ax` and leaves the rest of `rax`
as it was, so a narrow result must be extended back to its type - `Op::Canonical` after the swap - or a signed
`_Atomic short` holding -300 reads as a large positive number. The narrow forms are in the encoder, the lowering, five
runtime loops and seven tables in the header, and `tests/exe.rs` compares seven lines with `clang`, two of them negative.

201 unit, 34 parser, 30 CLI, 6 invalid, 25 exe, 4 real, 1 crt, 6 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong;
`.text` 80 336 bytes over 444 programs (3.490x the local clang over the 420 both accept); four fixtures byte-identical
to `clang`; fmt and clippy clean.

### M10 reviewed - 2026-11-25 - the width that was missing, and a diagnostic that spoke Rust

The `uchar.h` commit, reviewed. The literals encode right for all three families and the conversions match `clang`;
the review found that a `U` literal could not *initialise* anything, that a message printed Rust at a C programmer, and
two smaller repetitions.

**`char32_t a[] = U"ab"` was refused, and the reason read as a type error in the program.** Two helpers decide what a
string literal may fill - one in `init.rs`, one in `bytes.rs` - and both knew 8 and 16 bits, because 16 was the widest
literal this compiler had. A 32-bit element answered "not a character type". Then, once the width was added, `a[0]` came
out right and `a[1]` zero: an *unsized* array takes its length from the literal and its bytes from the second helper,
which was still refusing.

**And the fix that looked obvious was wrong in the way the old test said it would be.** Accepting every 32-bit element
made `int a[] = "hi"` legal - and the existing test for exactly that failed, which is the test doing its job. `char32_t`
is `unsigned int` and `int` is `int`: the width alone cannot tell them apart, and the unsignedness can. Both helpers say
so now, and `int a[] = "hi"` is still refused in the same words.

**A diagnostic printed `Utf16` and `Utf32`.** Joining `u"a"` with `U"b"` is an error - C joins *identically prefixed*
literals and nothing else - and the message said `a `Utf16` string literal and a `Utf32` one`, which is Rust talking to
a C programmer. `Prefix::spelling` gives the source's own `u` and `U`.

**And `is_literal_start` had two clauses for one question**: "the byte after the prefix is a quote" is the whole rule,
with or without a prefix.

201 unit, 32 parser, 30 CLI, 5 invalid, 24 exe, 4 real, 1 crt, 6 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong;
`.text` 80 336 bytes over 444 programs (3.490x the local clang over the 420 both accept); four fixtures byte-identical
to `clang`; fmt and clippy clean.

### M10 - 2026-11-25 - `uchar.h`, and the character constant C11 does not have

`uchar.h` ships, and with it **all five of C11's headers**. The header itself is short - two typedefs and the four
multibyte conversions, all four of which the universal runtime exports under exactly those names, measured before a
line was written. The work was the *compiler* half: C11 6.4.5's `u`, `U` and `u8` prefixes on string and character
literals, which no header can give.

**The prefix was a boolean, and that is where the change went.** `literal::is_wide` returned "starts with `L`", which
was the whole truth while `L` was the only prefix. It is a `Prefix` now - `None`, `Wide`, `Utf16`, `Utf32`, `Utf8` - with
its width and its element type beside it, and one `bytes_of` that encodes for all five: UTF-16 for `L` and `u`, code
points for `U`, UTF-8 for `u8` and for no prefix at all (this compiler reads its source as UTF-8, so those two are the
same bytes and different types). The lexer's two entry points became one, because a literal is a prefix and a quote and
the quote says which kind it is.

**And a program this compiler was accepting that C11 does not have.** `u8'a'` - a UTF-8 *character* constant - is C23's,
not C11's, and `clang -std=c11` refuses it. This compiler took it and gave it type `char`. It is refused by name now,
next to the note saying why `u8"..."` is not: one is a string and one is not a character, in the standard's own
partition.

**Then the ledger test found two things of its own.** The runtime's declared names were collected from C99's table only,
so with `uchar.h` shipping, `mbrtoc16` looked homeless - fixed by walking both tables, which is what the rest of the
ledger already did. And the thirty functions `threads.h` and `stdatomic.h` declare for the runtime to define were called
homeless too, because the runtime defines many of them *through a macro* (`XCC_ATOMIC(int, int)` makes
`__xcc_add_int`) and no text scan can see that: the `__xcc_` prefix is the compiler's reservation and is now the promise,
the way `__builtin_` already was.

201 unit, 32 parser, 30 CLI, 5 invalid, 24 exe, 4 real, 1 crt, 6 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong;
`.text` 80 336 bytes over 444 programs (3.490x the local clang over the 420 both accept); four fixtures byte-identical
to `clang`; fmt and clippy clean.

### M10 reviewed - 2026-11-25 - three kernel calls nothing declared, and a dialect left switched on

The `threads.h` commit, reviewed. The header and the producer/consumer hold; the review found a state leak on an error
path, a structure written twice, a typedef spelled through to its underlying type, and three functions the runtime
called without declaring.

**`CloseHandle`, `WaitForSingleObject` and `GetSystemTimeAsFileTime` were called as implicitly declared functions** - the
compiler said so on *every* link, and I had been filtering its output to the word `error`. That is the finding worth
keeping: a warning that appears on every run is one a reader stops seeing, and a filter is how it stopped being seen. The
declarations are in the runtime now, and a link prints nothing.

**The dialect was left switched on.** A link swaps in C11 for the runtime and back for each program unit - and the `?`
on a failed front end skipped the swap back, so a failed link left the whole `Compile` building C99 programs as C11.
`std::mem::replace` around exactly the call that needs it, and the restore happens whatever the call answers.

**The runtime had written the header's two structures a second time**, as `xcc_critical_section` and
`xcc_condition_variable`, and asserted *their* sizes against 40 and 8 - so the assertion guarded the copy and the pair
could still disagree. One structure now: the assertions are on `sizeof(mtx_t)` and `sizeof(cnd_t)`, and the Win32
declarations take the header's types with no casts at the call.

**And `struct timespec`'s `tv_sec` was spelled `long long`**, which is what `time_t` happens to be. C11 7.27.1p3 names
the typedef, and a header that writes through to the underlying type breaks the day it changes width.

201 unit, 32 parser, 30 CLI, 5 invalid, 23 exe, 4 real, 1 crt, 6 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong;
`.text` 80 336 bytes over 444 programs (3.490x the local clang over the 420 both accept); four fixtures byte-identical
to `clang`; fmt and clippy clean.

### M10 - 2026-11-25 - `threads.h`, and the store the dead-code pass deleted

`threads.h` ships (C11 7.26): a whole threading interface over what the target already has - `_beginthreadex` for a
thread, the critical section for a mutex, the Win32 condition variable for `cnd_t`, the runtime's thread-local slot for
`tss_t`, and `__builtin_atomic_cas` for `call_once`. **It was 48 files of measured demand**, third behind `stdatomic.h`
and ahead of everything else.

**A producer and a consumer proved it**, and three things the header cannot promise are written down rather than
implied: `mtx_timed` is refused by `mtx_init`, because a critical section cannot be entered with a timeout; a `tss_t`
destructor is not run, because Windows' thread-local storage has none to run; and a `cnd_timedwait` timeout has
millisecond resolution, which is what the call underneath has. mingw ships no `threads.h` at all - measured, `clang`
says "file not found" - so the test's three expected lines are the standard's, not another compiler's.

**And it found the fourth instance of one bug class.** `call_once` sets its flag with `(void)__builtin_atomic_cas(...)`
- swap, and ignore what was there - and the dead-code pass *deleted the swap*, because its list of instructions with an
effect beyond their result was `Store`, `Slot` and `Call`, and a compare-and-swap is none of those. The flag then stayed
at "running" for ever and the second `call_once` spun. The fix is one entry where `dce` decides, and the note there says
what the first version cost.

**Then `C99` broke everywhere, and the runtime was the reason.** `threads.h` uses `_Static_assert`, the runtime includes
it, and a link compiled the runtime in the *caller's* dialect - so a C99 program built a C99 runtime and could no longer
link. The runtime is this compiler's own source, so its dialect is the compiler's choice: a link now compiles it as C11
whatever the program asked for, and `tests/crt.rs` says the same thing where it compiles the runtime on its own.

201 unit, 32 parser, 30 CLI, 5 invalid, 23 exe, 4 real, 1 crt, 6 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong;
`.text` 80 336 bytes over 444 programs (3.490x the local clang over the 420 both accept); four fixtures byte-identical
to `clang`; fmt and clippy clean.

### M10 reviewed - 2026-11-25 - the list the swap was missing from, and a mode that printed nothing

The atomics commit, reviewed. The review's first job was the bug that commit had *dodged* - the CAS loop that lost a
sixth of its increments - and it is now found, fixed, and the natural shape of the loop is back in the runtime.

**`writes_rax` is a list of the operations that leave a value in `rax`, and `Op::AtomicCas` was not on it.** The
allocator moves a function's *return* value into `rax` by design, guarded by that list. A CAS loop returns the value it
had carried across iterations, so that value lived in `rax` - and `lock cmpxchg` writes `rax`, which nothing had told
the allocator. The value was therefore destroyed by the swap it was the witness of, and `seen == old` compared the
value just read with itself: the loop exited on the first attempt whether or not the swap had happened. Threads then
lost increments, a sixth of them. One entry in the list, and the loop that reads once and carries the value across the
back edge is **exact three runs running** - which is the shape the runtime has again, the in-loop load having been an
experiment rather than a fix.

**`Op::ThreadAddress` was missing for exactly the same reason**: the thread-local lookup is a call this back end emits
itself, so it is not an `Op::Call`, and it clobbers `rax` too - which would have broken any function that *returns* a
thread-local value. Measured after the fix: `int get(void) { return counter; }` with `counter` thread-local is right,
at both values tried.

**And `--explain` printed nothing at `--opt-level=2` and exited 0.** The level-2 path returned an empty explanation,
because the listing is Cranelift's and this back end does not write one. Found by reaching for it while chasing the
allocator bug and getting silence - a mode that silently does nothing is worse than one that says it cannot, so it
refuses now with the reason and points at `--opt-level=0`, where it works. The sentence is one line on purpose: written
through a shell heredoc first, its continuations became a run of spaces in the diagnostic.

201 unit, 32 parser, 30 CLI, 5 invalid, 22 exe, 4 real, 1 crt, 6 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong;
`.text` 80 336 bytes over 444 programs (3.490x the local clang over the 420 both accept); four fixtures byte-identical
to `clang`; fmt and clippy clean.

### M10 - 2026-11-25 - atomic operations, and the loop the allocator miscompiled

`stdatomic.h` ships, with real atomics. The demand was measured first, across the ~200 repositories in
`D:/Projects/github`: **124 files** include it - six times the next C11 header - and `stdalign.h` (23 files) and
`stdnoreturn.h` (11) came along in the same commit for five lines each.

**The machine has one atomic operation and it is now in the back end.** `windows.h`'s `InterlockedExchangeAdd` and
friends are *intrinsics*, not exports - measured, none of them is in `kernel32.dll` or `kernelbase.dll` - so there was
no library to call. `__builtin_atomic_cas` is `lock cmpxchg`, with the comparand in `rax` (which nothing is allocated
to), the result the value the object held, and the `lock` prefix emitted because without it the operation is a read, a
comparison and a write that two threads can interleave. **Every other operation is a loop of that in C**, in the runtime
source: five arithmetic operations and an exchange, per width - 32 bits and 64 - because that is what the machine
swaps. The header is then a table: one `_Generic` per operation, one arm per width.

**And the loop is where the compiler was wrong.** The first version read the object once and carried the last value
across the loop's back edge, the obvious way to write a CAS loop. Two threads, two hundred thousand increments each,
and the counter read **335 999 of 400 000** - and a spinlock built on the same sequence lost the same way, which is
what said "the loop" rather than "the swap". Moving the load *inside* the loop makes it exact, three runs running. The
disassembly of the first form shows the comparison comparing the value it just read with itself - an allocator
problem, most likely the coalescing hint on the `old = seen` copy, legitimate straight-line and not across a back edge.

**It is not an atomics bug**, and that is why it is in PLAN.md with the function that reproduces it rather than
patched over: the same shape would miscompile a user's CAS loop. The shapes tried so far in plain C do not reproduce it,
so the reproducer stays the atomic loop and the bisect is the next step.

**What is proved.** `tests/exe.rs` starts two threads that fetch-add to an `atomic_int`, to an `atomic_ullong` and
through an `atomic_flag` spinlock, and requires **exactly** 2N on all three - the assertion a non-atomic implementation
fails and nothing else catches. `tests/invalid.rs` holds the width refusal: `atomic_char` is declared as C11 asks and
its operations are not, because a `char` swap is not one of the machine's operations. Every operation prints what
`clang` prints.

201 unit, 32 parser, 30 CLI, 5 invalid, 22 exe, 4 real, 1 crt, 6 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong;
`.text` 80 336 bytes over 444 programs; four fixtures byte-identical to `clang`; fmt and clippy clean.

### M10 reviewed - 2026-11-25 - two things clang refused and this compiler did not

The `_Thread_local` commit, reviewed. The thread test still passes, and the review found one out-of-bounds read, two
programs this compiler accepted that clang refuses, a race in the runtime, and three small pieces of my own sloppiness.

**The template could be shorter than the block the runtime copies from it.** The linker grew the template with
`resize(block, 0)` and then appended an object's bytes if `.data` had any - but an object with *no initialiser* has no
bytes in `.data` at all (`_Thread_local int z;` is the common case), so the template stayed short and `memcpy(block,
initial, total)` read past its end. It is grown to each object's whole size now, and the bytes are copied into the
slice that size reserved.

**`a member cannot be `_Thread_local``, and neither can a parameter** - C11 6.7.3p2, in clang's own words. Both were
accepted silently, which for the member meant a `_Thread_local` member behaving as an ordinary one: a storage class
promised and not delivered. The two refusals are where the two lists are read, and each says what clang says.

**And the runtime's `TlsAlloc` key was allocated lazily**, which is a race two threads can lose: each allocates a key,
one overwrites the other's, and the loser's block - already filled - is unreachable from then on, so its
`_Thread_local` objects silently reset. The key is made in `mainCRTStartup`, where there is exactly one thread. The
same edit moved the three kernel declarations to the top of the file, where the reason they are there is written down.

**Three smaller ones.** `Op::ThreadAddress` carried a `size` nothing read; the offset word was computed by adding the
size and subtracting it back; and the comment on `made_by_the_linker` said "the two words" when the lookup reads three.
The first is deleted, the second is one `let` before the increment, and the third counts to three.

201 unit, 32 parser, 30 CLI, 4 invalid, 21 exe, 4 real, 1 crt, 6 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong;
`.text` 80 336 bytes over 444 programs (3.490x the local clang over the 420 both accept); four fixtures byte-identical
to `clang`; fmt and clippy clean.

### M10 - 2026-11-25 - `_Thread_local`, and the register the allocator did not know about

The last C11 storage class ships. `_Thread_local` (C11 6.7.3) gives an object thread storage duration: one copy per
thread, initialised as many times as there are threads. `tests/exe.rs` proves it with two real threads - each adds its
own id to a copy that starts at 100 and sees 101 and 102, while the main thread's is still 100 - and the program's
output does not depend on which thread ran first, because each records what it saw in a shared array that main prints
after joining.

**The shape of the thing, and the trade it makes.** A thread-local object has no link-time address, so the *address* is
a computation. The linker lays out one block per thread - it is the only place every unit's objects are in one place -
and writes each object's offset, the block's size and a dense template into `.data`. The compiler emits
`__xcc_tls(offset, total, template)`, which allocates this thread's block on first use, copies the template into it and
returns the object's cell. Five instructions and three relocations the first time a block touches the object, and a
register after that: **the PE TLS directory is the fast route - four instructions, no call - and the loader fills it.
It needs a `.tls` section, a 24-byte directory with three relocated addresses and a data-directory entry, which is a
bigger piece of the image writer than this took on.** It was measured afterwards, which the claim deserved: a tight
loop over a `_Thread_local` counter runs **1.34x** a plain static's, against clang's 1.17x - so the directory had
0.17x to win, and the call was the right trade rather than merely the quick one.

**Then the crash, and it is the same class as two before it.** The first thread test segfaulted inside a *worker*: the
sequence writes its arguments into `rcx`, `rdx` and `r8`, and `r8` and `r9` are *allocatable* - the allocator's
caller-saved set is `r8`-`r11`. Nothing spilled, because the operation was not a `Call` and the lowering's "does this
function make a call" test did not see it. The fix is in the lowering, where that question is asked: a `GlobalAddress`
naming a thread-local object makes a call. This is the third time a hardcoded register met an allocator that did not
know about it - `float_const` and `RAX` was the first, and the note in `thread_address` says so.

**And one bug the design had all along: the template was copied per object.** `__xcc_tls` copied an object's own bytes
when it made the block, so the *first* object a thread touched was initialised and every other one read zero -
`a=11 b=0` in a two-line program. The template is one dense run covering the whole block now, copied whole.

**The trap was the library, not the code.** Every program stopped loading with `0xC0000139` (ENTRYPOINT_NOT_FOUND) -
including ones with no thread-local object, because the runtime source calls `TlsAlloc` for the block. `TlsAlloc`,
`TlsGetValue` and `TlsSetValue` appear in `ucrtbase.dll`'s dump as **its imports of the kernel's**, not as its exports,
so the import table asked the wrong DLL for them. They are `kernel32` names, in the table next to `ExitProcess`, and
the note there says how the dump misled.

**Refused where it cannot be honoured.** `-c` and `--run` have no linker to lay a block out, so both refuse by name
with the reason, and `tests/cli.rs` holds both messages. `_beginthreadex` - the C runtime's thread entry point, which
the test needs - is declared in `process.h`, a header this compiler does not ship, so it lives in a one-entry table
saying exactly that rather than in `C_RUNTIME`, whose own test would otherwise fail: it did, which is why the table
exists.

201 unit, 32 parser, 30 CLI, 4 invalid, 21 exe, 4 real, 1 crt, 6 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong;
`.text` 80 336 bytes over 444 programs (3.490x the local clang over the 420 both accept); four fixtures byte-identical
to `clang`; fmt and clippy clean.

### M10 - 2026-11-23 - the tag moved one commit, and why

The release tag went up at `3ae2f6d` and CI failed there on the one test new since the last release - the code-size
gate, which asserted a ratio against whichever `clang` the machine had. The fix is `ad5f4a9`, so the tag was re-pointed
at it: a release tag should name a commit whose whole gate ran, and this one did not. The message was re-annotated in
the same move, because it still said the test holds the ratio when it now holds a byte total.

That is the second time a tag has been re-made in this repository, and the reason is the same both times: the tag is
what a reader quotes, so a number in it has to come from a run at the commit it names. The rule is now applied in the
right order - measure, fix, re-measure, then tag - which is cheaper than the move.

### M10 - 2026-11-23 - a gate that measured whatever clang the machine had

The tag was pushed and CI failed on the one test that was new since the last release: `the_corpus_is_within_the_code_size_that_was_measured`. Every byte this compiler wrote was identical to the local run; what moved was the *denominator*. The check asserted a **ratio** against `clang -O2`, and the runner's `clang` is a different build from the one measured here.

That is the failure the audit keeps recording under other names - a gate whose result depends on the machine is not a gate - and the fix is the one that rule implies: assert the fact that is this compiler's own. The check now sums the `.text` bytes **it** emits over every corpus program it compiles (80 336 of them, ceiling 81 950, 2% of headroom) and reports the ratio as context, with `clang` optional: where it is missing or refuses a program, the ceiling still holds.

Two smaller things fell out of making it optional. The ratio's denominator now counts only the programs `clang` also accepted - the set the ratio is about - which is a second counter rather than a second meaning of one. And the numbers in README.md say which is which: 80 336 bytes over 444 programs as the assertion, 3.49x over the 420 both compilers accept as the context.

201 unit, 32 parser, 29 CLI, 4 invalid, 20 exe, 4 real, 1 crt, 6 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong;
`.text` 80 336 bytes over 444 programs (3.490x the local clang over the 420 both accept); four fixtures
byte-identical to `clang`; fmt and clippy clean.

### M10 reviewed - 2026-11-23 - the cuts and the singular points, where the formulas were wrong

The transcendental commit, reviewed. The hundred lines of interior values held; the review went looking for the
*other* points - the ones where a formula divides by zero or sits on a branch cut - and almost every one of them was
wrong. Twelve of seventeen edge cases were a NaN, a conjugate or a flipped sign, and none of them is exotic: `cacosh(-1)`,
`catanh(2)`, `ctan(i)`, `ctanh` of anything large, `csqrt(0)`.

**The rule the review settled on: a formula with a branch to choose has been chosen wrongly once already.** Three of
them had chosen:

- `csqrt(0 + 0i)` was `0 - nan`, because the formula divides by `2t` and `t` is zero exactly there;
- `cacosh` computed `i w` as `Re w + i Im w` where `i(a + bi)` is `-b + ai` - so `cacosh(-1)` came back as the
  *conjugate* of π i. The prose next to it said "the sign of the imaginary part follows the argument's", which is true
  and was not what the code did;
- `cacosh`'s rotation and `catanh`'s rotation both pick a side of the cut with the *signed zero* of an intermediate,
  and `catanh(2)` came back as `-π/2 i` where Annex G says `+π/2`.

So the four inverse functions are the real-arithmetic forms now - `asin(p)` of a `p` that cannot leave `[-1, 1]`,
`atan2` over `1 - x² - y²`, a quarter of a log of squared distances - which have no branch to choose, and the two
rotations (`casinh = -i asin(iz)`, `cacosh = ±i acos(z)`) inherit from those. `ctan` and `ctanh` are the double-angle
quotients, and `ctanh` asks whether `tanh x` has *reached* 1 rather than guessing a bound: `ctanh(1e308)` is 1 and
`inf/inf` cannot happen.

**Two disagreements are `clang`'s, and are recorded rather than matched.** `ctanh(1e308)` is the limit, 1; `clang` returns
a NaN there while returning 1 for `ctanh(inf)` - its own formula overflowing. And `cpow(0, -2)` is infinite by Annex
G.6.4: this compiler writes `inf 0`, `clang` writes `inf nan`. Neither is in the compared program, and both are stated
where the code is.

**`ctanh`'s saturation and `csqrt`'s zero are in the test now**, along with seven points on the cuts and singularities -
165 lines against `clang`, up from 101. The comparison also adds `+ 0.0` to every number it prints: the sign of a zero
is not a value, and two implementations that agree on every number disagreed about `-0.000000000`.

**And the `l` spellings.** Sixteen of the twenty bases had no `long double _Complex` form at all: `cexpl`, `clog`,
`csqrtl` and the rest were undeclared names in a header that promises 7.3 in three widths. They are macros to the
`double` ones, like the four that already were, because this target's `long double _Complex` *is* its `double _Complex`.

201 unit, 32 parser, 29 CLI, 4 invalid, 20 exe, 4 real, 1 crt, 6 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong,
`.text` 3.490x `clang -O2`; four fixtures byte-identical to `clang`; fmt and clippy clean.

### M10 - 2026-11-23 - the nineteen complex transcendentals, and the two signs that were wrong

`complex.h` now ships all of C99 7.3: the sixteen transcendental functions of 7.3.5 to 7.3.8 in `double` and in
`float`, over `math.h` and over the compiler's own complex arithmetic. The `float` forms compute in `double` and narrow
once, which is what the four `math.h` wrappers already did and costs nothing on this target; their last bit can differ
from the runtime's own float routines, so the test prints them to four decimals and says why.

**Every one of them agrees with `clang` at six points in every quadrant** - 96 lines of nine decimals, plus the four
identities that say the branches are Annex G's (`sin^2 + cos^2 = 1`, `exp(log z) = z`, `sqrt(z)^2 = z`, `sin(asin z) =
z`). That is a strong claim for two independent implementations, and it came from the test failing first: five of the
101 lines differed, and three of the five were one bug.

**`csqrt`'s real part was `y/(2t)` where it is `|y|/(2t)`.** The principal root's real part is never negative, so the
signed value negated it for every negative imaginary part - `csqrt(-0.5 - 1.5i)` came back as `-0.735 - 1.020i` where
`clang` has `+0.735`. Everything built on `csqrt` inherited it, which is why `cacosh` differed too: it was the *same*
wrong number, seen through a logarithm.

**And `casin` used `-i log(iz + sqrt(1 - z^2))`, which has a sign to choose.** For `|x| > 1` the principal root is the
wrong one: `asin(3 + i)` came back as `3.07 - 0.72i`, a real part outside `[-pi/2, pi/2]`. It is the real arithmetic of
Abramowitz and Stegun 4.4.39 now - `asin(p)` for a `p` that cannot leave `[-1, 1]`, and `acosh(q)` with the sign of
`y` - which has no choice in it. `cacos` inherited the fix through `pi/2 - asin(z)`.

**`catan` had its ratio upside down.** `(i/2) log((1 - iz)/(1 + iz))`, not the other way round: on the real line the
direction decides between `pi/4` and `-pi/4`, and the test caught it on the first run as a negated result. The comment
now says what the direction decides, next to the line that decides it.

201 unit, 32 parser, 29 CLI, 4 invalid, 20 exe, 4 real, 1 crt, 6 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong,
`.text` 3.490x `clang -O2`; four fixtures byte-identical to `clang`; fmt and clippy clean.

### M10 reviewed - 2026-11-23 - four promises the header could not keep, and the scalar the standard says it is

The `_Complex` commit, reviewed. The type, the arithmetic and the ABI hold - the exe test still prints what `clang`
prints - and the review found four things wrong at the edges, three of them promises rather than code.

**`cabsl`, `cargl`, `conjl` and `cprojl` were declared and defined nowhere.** `long double _Complex` *is*
`double _Complex` on this target, so writing the definitions would have been redefining `cabs` three times; leaving them
declared was worse - `cabsl(z)` was "undefined symbol", a link failure for a name the header promised. They are macros
now, the way `math.h` already writes its own `l` family, and the reason is in the header where the four lines are.
Spotted by writing the program the review wanted to test and watching the *linker* complain, which is the only way a
missing definition shows.

**`(_Bool)z` was refused**, and `is_scalar` was the cause: it said "a number or a pointer", and C99 6.2.5p11 puts the
complex types among the floating ones and 6.2.5p18 among the arithmetic - so a complex value **is** a scalar (6.2.5p21).
Making it one meant four callers to look at. `convert` had to ask the complex rules *before* the `_Bool` rule, because
`_Bool` is a scalar question and `truth_value` compares one number; the conversion now answers in the type it was asked
for, so `_Generic((_Bool)z, _Bool: 1)` is 1 rather than `int`. And the initialiser walk was using "not a scalar" to mean
"has parts a list may not fill" - which a complex value has, scalar or not - so it asks a new
`Types::is_aggregate` instead. That is the one place the old predicate was standing in for a different question.

**The compound-assignment arm in `complex_binary` was unreachable.** `binary_result` is asked about operators, and
`z += w` is an assignment: the arm could not run, and a branch that cannot run is a reader's question with no answer.

**And the ledger's reasons had drifted.** Twelve near-identical strings for the eight names that remain, and the two
`fenv` entries dropped by the edit that added them. One wording now, eight entries, and `feraiseexcept`/`feupdateenv`
back where they were - a script that rewrote a block between two markers and took the neighbours with it.

201 unit, 32 parser, 29 CLI, 4 invalid, 19 exe, 4 real, 1 crt, 6 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong,
`.text` 3.490x `clang -O2`; four fixtures byte-identical to `clang`; fmt and clippy clean.

### M10 - 2026-11-23 - `_Complex`: two numbers in a record, and the three bugs underneath it

`complex.h` ships. It was the last header of C99 7.1.2 that refused, and the reason it refused was a *type*:
`_Complex` over `float` or `double`, the arithmetic C gives it, and the calling convention - none of which a header
can supply.

**The type is a record of two, and that is the measurement rather than a shortcut.** `sizeof (double _Complex)` is 16
and its alignment is 8, `{re, im}` is how a program initialises one, and the Win64 ABI carries an eight-byte one in an
integer register and a sixteen-byte one behind a pointer - which is exactly what `record_in_register` already did for a
record of the same size. Measured off `clang`: `mul(double _Complex, double _Complex)` receives a hidden return pointer
in `rcx` and pointers to both arguments in `rdx`/`r8`, while `fadd(float _Complex, float _Complex)` packs each argument
into one integer register. So `Types::complex` builds the record once per element type, records which records are
complex, and layout and the ABI need no rule of their own.

**`parse/complex.rs` owns everything a struct may not do**: the four operators of 6.5.5-6.5.7 (the formulas as the
standard writes them, which is also how `clang` computes them), both comparisons of 6.5.9, unary minus, truth, the two
conversions of 6.3.1.5/6.3.1.7, and the two halves by name. A complex value in an expression is a temporary holding the
parts, so the back end never learns the type means anything. `<`/`>` on one and `%`/`&`/shifts say what they need, with
the type named - which is why `describe` had to learn to say `double _Complex` rather than `struct (untagged)`.

**Three bugs came out of the way, and two were older than this work.** The first is in the default argument promotions:
`Types::promote` is *integer* promotion, and the variadic call site asked it for a `float` - so `printf("%.1f", 4.5f)`
put thirty-two bits in a register the callee read sixty-four from and printed `0.0`. That is why a struct of two floats
also came back as zeros, and it had been there since variadic calls were. The second is in the back end: `float_const`
staged a constant's bits through `Reg::RAX` without asking the allocator, and in a function returning a struct it
overwrote the hidden return pointer - `return (struct two){1.5, 2.5};` crashed the moment a constant was materialised
inside a live range. Both are fixed, and the second is why a `static` table of function pointers in a returned struct
worked before and a compound literal did not.

The third was mine, twice: `!=` and `!` on a complex value were computed with `cir::UnOp::Not`, which is *bitwise* `~`
and not logical negation - `(1,0) != (1,0)` answered `-1` where 6.5.9p3 says a comparison's result is 0 or 1. A
comparison with zero is what it is, in both places.

**What the header can and cannot promise was measured, not assumed.** The universal C runtime exports **none** of C99
7.3's sixty-six spellings - `cacos` to `cprojl`, one link at a time, every one "undefined symbol"; an earlier note
claiming otherwise was wrong. So `creal`/`cimag` are macros over the two halves, `I` is a macro over the type, and the
four exact functions (`conj`, `cabs`, `carg`, `cproj` in three widths) are the C runtime source's own. The nineteen
transcendentals are **not declared at all**: a program that calls `csin` is told it is not declared here, which is a
better error than a link failure for a name the platform does not have.

`tests/exe.rs` compiles, links and runs a program that prints its `sizeof`, its four operators, both comparisons, both
conversions, truth, `I`, a value across a call boundary in each of the two ABI shapes, and the four runtime functions -
and compares every line with what `clang` printed for the same program.

201 unit, 32 parser, 29 CLI, 4 invalid, 19 exe, 4 real, 1 crt, 6 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong,
`.text` 3.490x `clang -O2`; four fixtures byte-identical to `clang`; fmt and clippy clean.

### M10 reviewed - 2026-11-23 - the ratio in the README, and the zeroes a scalar does not need

The anonymous-member commit, reviewed. The splice holds, tinyexpr still matches `clang`, and the review found one
performance regression, two rough edges in the new code, and one sentence in the README that had quietly stopped being
true.

**The zeroing pass ran for scalar elements too.** `int a[] = {1, 2, 3}` was zeroed byte by byte and then written over,
because the walk had learned to zero every element of an array whose size comes from its list. Only an *aggregate*
element can have a member the list does not name; a scalar's leaf writes every byte of it. On a five-line program the
object went from 804 bytes to 588, and nothing else changed. The gate is `!is_scalar(element)`, one condition and one
sentence saying why.

**`flatten` had two exits for one question.** A member that is not an anonymous record was pushed and `continue`d in two
places - once for "its type is not a record" and once for "it has a name or is a bitfield". One `match` that answers
"which record, if any" and one place that pushes it reads better and is one branch shorter.

**And a NaN payload was silently ignored.** `__builtin_nan("0x1")` asks for a different NaN than `__builtin_nan("")`,
and the builtin was returning the quiet one either way - a wrong number rather than a smaller program. The payload is
now refused by name. Only `""` reaches the constant, which is what `NAN` is.

**The README's "3.46x `clang -O2`'s code size" had moved to 3.49x and nothing failed.** Measuring it needs `.text` and
not the object file - a COFF file carries a symbol table, and `clang`'s is longer for reasons that have nothing to do
with instructions; the first attempt measured objects and got 0.45x, which is the kind of number that tells you the
measurement is wrong rather than the compiler. The measurement is now a test:
`the_corpus_is_within_the_code_size_that_was_measured` compiles every comparable corpus program both ways, sums `.text`,
prints the ratio and fails above 3.55x. It reads 3.490x over 420 programs and costs 40 seconds. The two sentences in
README.md now say 3.49x and point at the test, and PLAN.md's older numbers say "measured at the time", because they were.

**The fixture's provenance was "commit `master` when the fixture was added"**, which is not a thing anyone can check.
It names `ba2e32bbcc2d1e644a7e132d24c2f542821cabe0` and the line counts now.

**And the `.gitignore` rule I added last commit to keep commit messages out of the tree did not work.** git has no
trailing comments: `.msg_*   # commit messages, ...` is a pattern that ends in the comment, so it matched no file at
all, and the scratch file was still there in `git status`. `git check-ignore -v` says so in one command, which is how
this was found - and the comment now says that, next to the pattern, so the next person who wants to annotate one knows
where the annotation may go.

201 unit, 32 parser, 29 CLI, 3 invalid, 18 exe, 4 real, 1 crt, 6 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong,
`.text` 3.490x `clang -O2`; four fixtures byte-identical to `clang`; fmt and clippy clean.

### M10 - 2026-11-23 - anonymous members, and the three bugs behind them

`tinyexpr.c` (758 lines, zlib) compiles, links and runs, and prints byte for byte what a `clang` build of the same
files prints. It was blocked on an **anonymous union** - `te_expr` holds `union { double value; const double *bound;
const void *function; };` - and fixing it uncovered two more, in code no earlier library reached.

**The member list, flat.** The layout of an anonymous member was already right: it was laid out like any other member,
and `sizeof` of the whole record counted it, which is why the measurement that "found" this found a *lookup* failure
and not a size. What was missing was the view. The fix is one splice in `Types::complete_record`, after layout: a
member with no name whose type is a record contributes *its* members, offsets shifted by its own. Everything else -
lookup, the initialiser walk, `describe`, the duplicate-name check - reads that one list and needed no change, because
there is no second view to disagree with it. That last one is not a detail: `struct s { int a; union { int a; }; }`
names one member twice, and C11 6.7.2.1p13 puts both in the same scope.

**The duplicate-name check moved to where the truth is.** It used to run as each member was parsed, which cannot see
across the splice. It runs on the completed list now - one place, and `member_list` lost a `HashSet` and a helper for
it.

**tinyexpr's second bug: a `static` local table of function pointers was all zeros.** `power()` keeps
`static double (*const unary[2][3])(double) = {{NULL, ...}, {negate, ...}};` in a body. `emit` refused to make code
only when there was no body; inside one, the address of `negate` became an instruction, and the initialiser that was
*writing bytes* had no value to write - so it wrote zeros, and `te_interp("-3")` returned `3`. The predicate already
existed (`at_file_scope`, "writing bytes rather than emitting stores") and `emit` simply never asked it. It does now,
under the name of the thing it tests: `writing_bytes`. Then `-3+10/4` is `-0.5` again.

**Its third: an array whose size comes from its list left its elements' unnamed members holding the frame's
leftovers.** `te_variable variables[] = { { "x", &x }, { "y", &y } };` printed `type` as `-43300501`. C99 6.7.8p21
zeroes what the list does not name, and the zeroing pass ran only when the object's type was already complete - which
an array whose count comes from the list is not, until the walk has read it. The array walk zeroes each element as it
reaches it now: the same bytes, one level down, and in the only place that knows the count.

**And `NAN` was not declared in `math.h` at all**, and the obvious spelling would have been wrong. `0.0/0.0` is this
target's *indefinite* NaN and its sign bit is set, so `printf("%f", NAN)` printed `-nan(ind)` where `clang` printed
`nan`. `__builtin_nanf("")`, `__builtin_nan("")`, `__builtin_inff()` and `__builtin_inf()` are constants the front end
now makes, and `NAN`/`INFINITY` are them - the same spelling every other compiler's header uses. `_Noreturn` is
consumed and dropped, next to `inline`.

**tinyexpr is the fourth vendored fixture** (`tests/fixtures/tinyexpr`, unmodified, zlib licence, its README saying
what the expected text is and where it came from) and runs on every test run.

201 unit, 32 parser, 29 CLI, 3 invalid, 18 exe, 4 real, 1 crt, 5 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong,
refusal oracle 19/0; four fixtures byte-identical to `clang`; fmt and clippy clean.

### M10 reviewed - 2026-11-22 - the count and the list under it, and a frame sized to a constant

The stack-probe commit, reviewed. The probe holds and lz4 still matches `clang` byte for byte; the review found four
things I had written wrong around it, one of them in a comment that describes a byte layout.

**"Four bugs came out of this fixture", followed by three.** The count was in two places - the test's doc comment and
the fixture's README - and both said four while both listed three: two parser bugs and the prologue. The same class as
the README that said "twenty-four headers" while twenty-three shipped, and it will keep happening until counts are
mechanical. This one is a word next to a hand-written list, and the list is the truth; both now say three.

**"`jbe` is at 19 and jumps 24 bytes to the tail at 43"** - the arithmetic is right and the nouns are wrong. `rel32`
is counted from the *end* of the branch, so 19 is where `jbe` ends, not where it is (it begins at 13, which is what
`objdump` prints). A comment about byte layout that names the wrong offsets is worse than none, because it is the kind
you check once and then trust. It says "ends at" now.

**The largest frame in the new test was one megabyte, which is half the stack reserve.** It was the number I had
measured, but the test is about the *walk*, and a size that close to a header constant makes the test fail for a reason
that has nothing to do with probing the day someone changes `STACK_RESERVE`. It takes three hundred thousand bytes now
- seventy-three pages, which proves the loop and leaves the constant out of it.

**And `<clock.h>` for `clock()`**, in the fixture's README: the header is `<time.h>`, which is what `fuzzer.c` includes.

**Measured, because the fix left two combinations untested by the fixture:** a variadic function with a 65 536-byte
frame and ten arguments, and one with a 50 000-byte frame around a `double` argument, so the probe and the XMM saves
share a prologue - both print what `clang` prints. And `--run` with a 100 000-byte local works: Cranelift probes the
stack itself, so the JIT never had this bug and the fix does not apply to it.

201 unit, 32 parser, 29 CLI, 3 invalid, 17 exe, 3 real, 1 crt, 5 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong,
refusal oracle 19/0; fmt and clippy clean.

### M10 - 2026-11-22 - a stack page Windows grows through, and the frame that stepped over it

lz4's own `lib/` - the block compressor, the high-compression one, the frame format, the stdio wrapper and xxhash,
10 540 lines - now compiles, links and runs against this compiler, and its compressed output is **byte-identical to a
`clang` build's** through every API: the block API at four accelerations, `LZ4_compress_destSize`, a caller-supplied
state, partial decompression, both compressors' `extState` forms, the streaming API with `LZ4_saveDict`, compression
against a loaded dictionary, the high-compression stream, frames written and read back in odd-sized chunks, a frame
built from a prepared `CDict`, and the stdio wrapper over a `FILE *`. The driver prints a digest of every compressed
byte, so "identical" is a claim about bytes. It is vendored under `crates/xcc/tests/fixtures/lz4` and runs on every
test run.

**The bug it found is the worst kind: a wrong answer that only appears when the program is big.** A function whose
frame is larger than one stack page was a crash - `char a[65536]` was an access violation, `char a[32768]` too, and
`char a[24576]` was not, which is the shape of a guard page rather than of an arithmetic mistake. Windows commits a
thread's stack one page at a time, as accesses reach the single guard page below what is committed: one `sub rsp` of a
whole large frame steps *over* that page and the next write is an access violation the kernel cannot grow through. The
prologue now walks down a page at a time, touching each one, and describes the whole walk with the one `ALLOC_LARGE`
it adds up to. The image also asked for a one-megabyte stack where `clang` asks for two, measured off a binary it
linked; it asks for two now.

**The first attempt at the probe was wrong in a way that is worth keeping.** It used a label for the loop, taken from
the encoder's own label vector - and the *lowering* numbers the labels it will use, so the index it took was handed to
a block later: `jbe` jumped into the middle of the function and `jmp` went nowhere. Eight lines of decoding the emitted
object found it. The loop is written with its two displacements as constants now, with a `debug_assert` on the length
that makes the constants and the bytes disagree loudly if either moves - and the same `objdump` reading that caught it.

**Both gates were checked by breaking the fix**: with the probe disabled, `char a[1 000 000]` dies with `0xC0000005`
and both the new `tests/exe.rs` frame test and the lz4 fixture fail. A gate that would pass either way is not a gate.

Not attempted, and measured rather than assumed: lz4's `programs/` (`lz4cli.c`, `lz4io.c`, `bench.c`) needs
`windows.h`, `io.h`, `sys/stat.h` and `winioctl.h` - the Win32 SDK, the same boundary sqlite3.c stops at - and
`tests/fuzz/fuzzer.c` needs only `clock()` itself but reaches those headers through `programs/util.h`. A depth-100
recursion of 5 000-byte frames runs, and a 3 MB frame reports a stack overflow, which is the reserve answering
correctly rather than a crash.

201 unit, 32 parser, 29 CLI, 3 invalid, 17 exe, 3 real, 1 crt, 5 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong,
refusal oracle 19/0; lz4's digest and the two older fixtures byte-identical to `clang`; fmt and clippy clean.

### M10 reviewed - 2026-11-22 - the deeper rule that was shallower

The lz4 commit, reviewed. Both fixes hold and lz4 still compresses byte-identically to `clang`; the review found that
the first fix left a shape behind, and that my first attempt at it made things worse.

**`&(g)` is refused while `&g` is accepted.** C99 6.5.3.2p3 makes the address of a function designator *be* the
designator, and the shortcut reads the token in front of the operand - where a `(` hides it. Found by sweeping the
neighbourhood of the fix rather than the fix itself, which is the lesson: a test that checks what was broken does not
check what was not.

**And the "better" rule broke `&g`.** I replaced the lookahead with a check of the parsed operand's type - "a function
designator's type is a function, so return it unchanged" - which reads like the standard and is wrong here: by the time
a function's name is a value it has *decayed* to a pointer to function, the same type a call returning one has. So
every `&g` was refused. Reverted, and the reason is now in the comment where the rule lives, because the next person to
think of it will think of it too.

**`&(f)` is a gap and is recorded as one** rather than guessed at: a fix has to count parentheses, and `&f` is the
idiom - lz4, Lua, miniz, cJSON and stb_image all write that, and none of them writes `&(f)`. PLAN.md says what it would
take and why it waits.

201 unit, 32 parser, 29 CLI, 3 invalid, 16 exe, 2 real, 1 crt, 5 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong,
refusal oracle 19/0; lz4's frame stream still byte-identical; fmt and clippy clean.

### M10 - 2026-11-22 - lz4, and two parses that looked right until a real file said otherwise

The local lz4 checkout (3 035 lines in `lib/lz4.c`, plus the HC compressor, the frame format and xxhash) compiles and
works: a hundred thousand bytes compress to 583 and back, and **the compressed stream is byte-identical to a `clang`
build's** - which LZ4 being deterministic makes a strong oracle. The same holds for the high-compression compressor and
for a framed stream with a content checksum. Getting there took two fixes, both in the parser.

**An array initialiser could not end with a comma, if its size came from the list.** `lz4frame.c` declares its error
strings as `{ LZ4F_LIST_ERRORS(LZ4F_GENERATE_STRING) }`, and that macro puts a comma after *every* element, the last
one included: C99 6.7.8p1 allows it and every real table is written that way. A *sized* array was fine - the capacity
check stopped the loop at the comma and the surplus arm knew a trailing comma from a surplus value - but an unsized one
has no capacity, so after the comma the parser demanded a value: "expected an expression". The check for a comma before
the closing brace is now where the separator is eaten, which is the only place both cases pass through.

**And `&f(x)->m` was parsed as `(&f)(x)->m`.** C99 6.5.3.2p3 makes the address of a function designator *be* the
designator, and the shortcut for it fired on the identifier alone - so a call, a subscript, or a member access after it
was applied to the *address* instead of to the call's result. `lz4frame.c` writes
`LZ4F_pool_waitDone(mt->pool, &LZ4F_mt_job(mt, n)->done)`, and the compiler said "cannot call pointer to function".
The shortcut now needs the designator to be the whole operand: no `(`, `[`, `.` or `->` behind it.

**Both are the same shape of mistake**, and worth naming: a rule that is right about the expression it was written for
and wrong about the one that follows it. The tests hold the four shapes of the first and the four of the second,
including the two that must still be refused - a surplus array value, and the address of a call's result.

201 unit, 32 parser, 29 CLI, 3 invalid, 16 exe, 2 real, 1 crt, 5 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong,
refusal oracle 19/0; lz4 and lz4hc and lz4frame byte-identical to `clang`; fmt and clippy clean.

### M10 reviewed - 2026-11-22 - the test that would have passed for the wrong reason

The lexer and initialiser commit, reviewed. Both fixes hold and the sweep around them came up empty; the finding is
against the test that was supposed to keep the first one fixed.

**The lexer's regression test asserted `contains("int")`.** A `//` comment losing its newline is a bug about *which
tokens start a logical line*, and "int is somewhere in that set" passes for a lexer that marks **every** token as
starting one - the failure the flag exists to prevent, in the other direction. The set is the assertion now:
`["#", "int"]`, exactly the `#` of the directive and the `int` of the line after it. Writing it that way immediately
failed, correctly, on the end-of-file token: the file's end starts a line too, and a test that means "the tokens"
has to say so.

**Checked, because the fix is one line in a hot path**: a `//` comment before a directive, at end of file with no
newline, a `/* */` spanning lines (where the newline must *not* count - C99 replaces a comment with one space, so a
directive runs through it), a block comment inside a `#define`'s body, nested trailing commas in a two-dimensional
array, and a trailing comma after designators (6.7.8p7). All correct.

201 unit, 30 parser, 29 CLI, 3 invalid, 16 exe, 2 real, 1 crt, 5 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong,
refusal oracle 19/0; fmt and clippy clean.

### M10 - 2026-11-22 - `#define A 1 // c` took the rest of the file as its body

stb_image.h - 7 812 lines, one of the most used single-file libraries in C - was the next real code, and it found
three bugs in an hour. The first is the worst this compiler has had.

**A `//` comment ate the newline that ends its line and did not say so.** The lexer's line-comment branch walked to
the newline, consumed it, and broke - while the branch that handles a newline on its own sets `starts_line`, the flag
that tells the next token it begins a logical line. A directive reads until a token that starts one, so
`#define A 1 // c` read to the **end of the file** as its body: every line after it vanished, silently, and the
compiled program was not the program written. Any C file with a trailing comment on a `#define` - which is most of
them - was miscompiled. Measured: with the flag set, `stb_image.h` preprocesses as itself and clang's `-E` and ours
agree on the file's own text.

**And a struct initialiser could not end with a comma.** The capacity check in the brace walker ran *before* the loop
ate the separator, so `{ r, e, }` - a struct filled exactly, with the comma everyone writes (C99 6.7.8p1) - looked
like a third value: "too many initialisers for this struct or union". stb's callbacks table is written that way.
The check now asks whether the comma stands before the closing brace, which is what a trailing comma is.

**And the array-of-function-pointers path under it** was only reached once those two were fixed: with them,
`stb_image.h` compiles with `-DSTBI_NO_SIMD` (its own switch; x86-64 has SSE2 and stb uses `<emmintrin.h>`, which is
a compiler feature and not a header this compiler ships) and **decodes a 2×2 PNG byte-identically to a `clang`
build** - 231 KB of object from 7 812 lines.

Three tests: the lexer's flag after a comment, five preprocessor shapes around it, and the trailing comma with its
counter-case (a struct given *more* values is still refused).

201 unit, 30 parser, 29 CLI, 3 invalid, 16 exe, 2 real, 1 crt, 5 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong,
refusal oracle 19/0; cJSON and miniz unchanged, stb_image decodes identically; fmt and clippy clean.

### M10 reviewed - 2026-11-22 - const stopped at the object and not at its members

The `const` commit, reviewed. The qualifier reaches a typedef, a member, a pointer's target and an array
element - and the check that used it asked one question where C asks two.

**A struct with a `const` member is not a modifiable lvalue either**, and C99 6.3.2.1p1 says so *recursively*: "does
not have any member (including, recursively, any member or element of all contained aggregates or unions) with a
const-qualified type". `assign` tested `is_const(left.ty)`, which is true only when the *whole type* carries the
qualifier - so `struct S { const int a; }; y = x;` compiled, and a member buried one struct deeper did too. Measured
against `clang`, which refuses both.

The question is one function now - `Types::is_modifiable` - which walks the members the same way C says to, and both
write sites ask it. That is the shape the review was looking for: the rule lives where the type is known, not in the
two places that happen to need it. The test gained the two shapes and the one that must still compile, because a rule
that refuses everything is not a rule.

200 unit, 28 parser, 29 CLI, 3 invalid, 16 exe, 2 real, 1 crt, 5 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential
(415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong,
refusal oracle 19/0; fmt and clippy clean.

### M10 - 2026-11-22 - const was consumed and dropped

Seven programs C forbids compiled, and the reason was one line: `"const" | "volatile" | "restrict" |
"inline" => { self.at += 1; }`. The qualifiers were read and thrown away, so `const int c; c = 2;` compiled - and
several of its spellings did: through a typedef, through a struct member, through a pointer, on an array element, and
with `++` and `+=`.

**`const` is a type now, and `kind()` peels it.** `Type::Const(inner)` is a wrapper, and `Types::kind` returns the
*unqualified* view: every rule that asks what a type is - `sizeof`, arithmetic, layout, every conversion - sees the
type it already knew, and only the three that ask "may this be written?" look at the wrapper. The alternative, a
qualifier flag on every variant, would have touched every `match` in the compiler; this touched two, and both needed an
`unreachable!` arm to tell the type checker what the peeling already guarantees. `const int *p` is then
`Pointer(const int)`: the qualifier lives where C puts it, so a member, a typedef and an array element carry it without
anyone remembering a parallel fact.

**Two check sites, because there are two writes**: `assign` - which every compound assignment routes through, since
`a += b` is written as an assignment of `a + b` - and `step`, for `++` and `--`. Each is one clause.

**And one subtlety that the measurement caught**: dropping a parameter's qualifier, as C99 6.7.5.3p15 requires for the
*function type*, also removed it from the parameter *object* inside the body - so `int f(const int x) { x = 1; }`
still compiled. Two facts, two places: the symbol keeps the qualified type, the function signature takes
`unqualified`. Before that fix the sweep said one program still passed; after it, none.

**Duplicate member names** were the last of the eight: C99 6.7.2.1p1, one hash set per member list and a diagnostic.

**The sweep is a test now** (`tests/invalid.rs`): seventeen programs C forbids, each measured against `clang` when it
was written, plus the three that clang accepts as well - a forward-declared incomplete struct, a non-void function with
no `return`, and an unnamed member - which this compiler accepts for the same reason. It says what each case is for,
so the next "we accept what C forbids" is found by the suite rather than by a person reading the parser.

200 unit, 28 parser, 29 CLI, 3 invalid, 16 exe, 2 real, 1 crt, 5 x64, 5 hostile, 8 C API, 4 bcc20-sys,
differential (415 ran, 5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree,
20 refused, 0 wrong, refusal oracle 19/0; cJSON, miniz and the pg gate unchanged; fmt and clippy clean.

### M10 reviewed - 2026-11-22 - the cap changed the parse

The predefined-macros and budgets commit, reviewed. Both findings are against the *caps* it added, and the second is
the worst kind of bug this project has a name for.

**The truncation note could no longer fire.** `truncate_diagnostics` added "N further diagnostics not shown" when the
list was *longer* than the budget - and the parser now caps the list where it grows, so its length is exactly the budget
and the condition is never met. A check whose result the code cannot reach. The count the parser refused comes in beside
the list now (`Parsed::dropped`), and the arithmetic happens in one place - `report(diagnostics, refused, options)` -
instead of two that could disagree, which they immediately did: the first version printed "1 further" for a list that
had refused fifty.

**And the cap changed the parse.** With the list capped, `error_count` - which counted the *stored* errors, to answer
"was this body read without one?" - stopped growing, so rules that ask that question took a different path: a source
capped at ten diagnostics produced **355** diagnostics' worth of work where a generous budget produced 240. A limit meant
for the *report* was deciding what the *parser* did, which is worse than a wrong count: it makes the compile depend on
how much of its own output it was allowed to keep. The answer is a counter incremented before the cap check, so the fact
is about the source and the list is about the report, and a test asserts the invariant across two budgets:
*shown + refused is what the source earned, whatever the budget.*

**Two things added because the review could not check them**: the README said "the standard macros" without naming the
target's, which is what a program tests to learn where it is; and nothing pinned the point of `Budgets::for_a_compiler`,
so a test now compiles a nine-megabyte source - sqlite's amalgamation is 9 018 109 bytes - with it.

200 unit, 28 parser, 29 CLI, 16 exe, 2 real, 1 crt, 5 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential (415 ran,
5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong, refusal
oracle 19/0; fmt and clippy clean.

### M10 - 2026-11-22 - the target's own names, and the eleven gigabytes they uncovered

Three things, found by pointing the compiler at a program the size of sqlite's amalgamation.

**The predefined macros were half-defined.** `__STDC__`, `__STDC_VERSION__` and `__STDC_HOSTED__` were substituted
during expansion and never installed as macros, so `#if __STDC__` was true while `#ifdef __STDC__` and
`defined(__STDC__)` were false - three answers to one question in one file. C99 6.10.8p2 makes them macros, and
asking with `#ifdef` is the common case. They are installed as macros now, in one table, beside the target's own names
which were missing entirely: `_WIN32`, `_WIN64`, `__x86_64__`, `_M_X64`, `__XCC__`. `__STDC_VERSION__` comes from the
dialect, so it is installed from the dialect.

**What that cost, measured.** sqlite3.c (255 680 lines, nine megabytes) was refused by this compiler's own eight-
megabyte source budget, and the command line had no way to raise it - so `Budgets::for_a_compiler` is what the CLI
uses now, with the three limits that bound *the machine* (macro depth, include depth, nesting) left exactly where they
were. With the budget lifted, sqlite asked for `<pthread.h>` and `<sys/types.h>`: `os_setup.h` had decided the target
was Unix, because `_WIN32` did not exist. With `_WIN32` defined it asks for `<windows.h>` instead, which is the Win32
SDK - a boundary this compiler states rather than crosses.

**And then the eleven gigabytes.** With `_WIN32` defined, cJSON takes the branch every Windows header takes -
`__declspec(dllexport) type __stdcall name(...)` - and the fixture test died with `memory allocation of 11811160064
bytes failed`. Bisected to one line: `typedef struct H { int ( x * p ) ( void ) ; } H;`. Two bugs, both real:

- `skip_until` *stopped* at a `)` at depth zero that no `(` in its own scan had opened, leaving the caller's member
  loop with no progress: an infinite loop that, before the second fix, grew a `Vec<Diagnostic>` until the allocator
  gave up. It now steps over a stray closer and keeps looking for the delimiter it was sent for - the delimiter is
  outside it by construction.
- the diagnostic list was truncated *after* the parse, so nothing bounded it *during*: `Parser::push` now spends the
  budget where the list grows. A bound applied where the list grows bounds the list; one applied to the list
  afterwards bounds the answer, not the work.

Both reproducers are now a diagnostic in about 150 milliseconds, and the commit is also the parser learning the two
spellings a Windows header emits: `__declspec(...)` is skipped wherever a specifier may stand, and
`__cdecl`/`__stdcall`/`__fastcall`/`__thiscall`/`__vectorcall` are skipped on either side of the `*`s, because on
x86-64 there is one convention and Microsoft's own documentation says so.

**Found and not fixed**: a nameless struct member - `struct H { void * ( __cdecl * ) ( unsigned long long ) ; };` -
is accepted silently, where C99 6.7.2.1p1 wants a declarator with an identifier. It is a diagnostic gap, not a
correctness one for well-formed code, and it is the next thing to look at rather than something to hide.

198 unit, 28 parser, 29 CLI, 16 exe, 2 real, 1 crt, 5 x64, 5 hostile, 8 C API, 4 bcc20-sys, differential (415 ran,
5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong, refusal
oracle 19/0; cJSON, miniz and the pg gate as before; fmt and clippy clean.

### M10 reviewed - 2026-11-22 - the noun was checked and the number was not

The math-families and `tgmath.h` commit, reviewed. One finding, and it is the third time this project has made it: a
count in prose that no test reads.

**The README said "twenty-four" headers ship; twenty-three do.** It listed twenty-three names, and the test that
compares the list to the ledger passed - because it compares *names*, and the number was prose beside them. The same
line said "twenty" while nineteen shipped, and "twelve" while fifteen did, before it. A fact written down twice
disagrees with itself, and the copy nothing reads is the copy that drifts.

The count is now a test: `the_readme_counts_the_headers_that_ship` reads the word after `| headers |`, maps it through
a table of numerals, and compares it to the number of `Shipped` entries. Proved both ways - "twenty-two" fails with "the
README says `twenty-two` headers ship; 23 do", and a word past the table fails as a word the test does not read.

**And a sentence in the same row still described the attempt that was measured wrong**: `tgmath.h` "picks between them
by the argument's type with `sizeof`". That was the first design, which returned the right value with the wrong type;
the header uses `_Generic` now and needs `--std=c11`, and the README says so.

196 unit, 28 parser, 29 CLI, 16 exe, 2 real, 1 crt, 5 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran,
5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong, refusal
oracle 19/0; fmt and clippy clean.

### M10 - 2026-11-22 - the header that needed the whole of math.h first

`tgmath.h` ships, `complex.h` stays refused with a measured reason, and the work in between was `math.h`'s other two
families - which had never been there.

**`math.h` declared only the `double` family.** C99 7.12 asks for three of every function: `sin`, `sinf` and `sinl`,
fifty-two functions each. The `f` family is the library's own for all but four (`fabsf`, `frexpf`, `ldexpf`, `hypotf`,
which the runtime source wraps), and the `l` family is a **macro** mapping to the `double` one: this target's
`long double` *is* its `double`, so `sinl` and `sin` are one function with two spellings - which is what the target's
own header says, and what this header already said about `HUGE_VALL`.

**`tgmath.h` needed `_Generic`, and finding that out took two attempts.** The first version dispatched on `sizeof`,
the classic C99 trick. Measured against `clang`, the *values* were right and the *types* were wrong: a conditional
over `sqrtf(x)` and `sqrt((double)x)` has type `double`, so `sqrt(4.0f)` returned a double - and `_Generic` in the
test said so (`2 2 2` where `clang` said `1 2 2`). The second attempt used `_Generic` and could not compile:
`_Generic((x), float: sqrtf, default: sqrt)(x)` - a *function designator* selected and then called - is not something
this parser accepts. The third works, and is the one that is exact: the call is written into each arm, the unchosen
arms are never emitted, and a `float` argument produces a `float` result with the argument evaluated exactly once
(`calls=1`, identical to `clang`).

**So `tgmath.h` requires `--std=c11`, and says so by using `_Generic`**: at C99 it fails with the compiler's own
"`_Generic` is C11: compile with `--std=c11`", which is a better answer than a macro that returns the wrong type. The
header states that, and the two reasons - the wrong type from `sizeof`, the absent `_Complex` half.

**`complex.h` stays refused, and the reason is now measured**: all twenty-one of its functions (`creal`, `cimag`,
`conj`, `cabs`, `cexp`, `cpow`, ...) are exported by the runtime, so nothing is missing from the library. What is
missing is `_Complex` - a type, its arithmetic and its calling convention. The refusal says exactly that, so the next
person knows the work is the type system's and not a header's.

**And the ledger's own extractor needed two fixes** to survive a header of macro bodies: a macro's *parameters* are not
functions (`_TG1(name, x)` names neither), a *pasted* name is not a call (`name##f(x)` names no `f`), and the keywords
that look like calls (`_Generic`, `_Alignof`, ...) are keywords. Each was found by the test failing on the new header,
which is the test doing its job.

195 unit, 28 parser, 29 CLI, 16 exe, 2 real, 1 crt, 5 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran,
5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong, refusal
oracle 19/0; fmt and clippy clean.

### M10 - 2026-11-22 - wchar.h, and the eighteen names it was missing

`wchar.h` ships: the last of the five headers that used to refuse is one of twenty-three. What it needed was measured
before it was written - fifty-seven of its seventy-five C99 names were already exported by the runtime, and the
eighteen that were not are the runtime source's now.

**Twelve wide `stdio` front ends**, the same division as the narrow ones: the UCRT owns
`__stdio_common_vfwprintf`/`vswprintf`/`vfwscanf`/`vswscanf` - each taking a `va_list` - and the program carries the
`...` that turns its arguments into one. **Four `wmem*` functions, `mbsinit` and `wmemset`** are four-line loops.
`mbstate_t` is the UCRT's own layout (`unsigned long`, two `unsigned short`s, eight bytes), because the library writes
through the pointer a program hands it.

**`fwide` was the design question, and the platform answered it.** This runtime tracks no orientation: mingw's own
header defines `fwide` as an inline stub that ignores the stream and returns the mode it was given. So ours does the
same, and the header says so where it is declared - asking is 0, setting is the mode, and both are true statements
about the one orientation there is. A program that relies on `fwide` to *detect* a stream's orientation was relying on
something no Windows C runtime offers.

**Measured end to end** against a `clang` build: `swprintf(L"%d-%ls-%c", …)` writes 9 characters, `wprintf` writes them
to stdout, `swscanf(L"%d %ls", …)` reads two conversions, `wmemset`/`wmemcpy`/`wmemcmp` agree, `mbsinit` answers for
both a zeroed state and a null pointer, and `fwide(stdout, 0)` is 0.

195 unit, 28 parser, 29 CLI, 15 exe, 2 real, 1 crt, 5 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran,
5 refused, 44 skipped, 0 timed out, 0 mismatched) - green; 443 corpus programs agree, 20 refused, 0 wrong, refusal
oracle 19/0; fmt and clippy clean.

### M10 reviewed - 2026-11-22 - the wide literal worked everywhere but where it is written

The wide-literal and header commit, reviewed. The commit's own tests passed and covered five shapes; the review found
the sixth, which is the one a program is most likely to write.

**`wchar_t s[] = L"ab";` - a wide array at file scope - was refused**: "a string literal initialises an array of
`char`, not unsigned short". The *literal* was widened correctly by the last commit; the *initialiser* path still
checked for an 8-bit element and counted bytes where it meant elements, so the one form a real program uses to write a
wide string constant did not compile. The check is now "the array's element has the literal's own character width" -
`char` fills `char`, `wchar_t` fills `wchar_t`, `int` fills neither - and the byte writer takes the element's width
into account, so `wchar_t h[4] = L"xy"` is eight bytes with the tail zeroed. Four shapes measured against `clang`:
unbounded, oversized, exact (`wchar_t e[2] = L"ab"`, the NUL dropped as 6.7.8p14 says), and a pointer.

**And a message that said the wrong unit twice**: "a 1-byte array cannot hold this 2-byte string" for
`wchar_t tight[1] = L"ab"` - both numbers are *elements*, and the string is *characters*. It reads "a 1-element array
cannot hold this 2-character string" now, and the narrow case (`char s[2] = "abc"`) reads the same way rather than
differently for no reason.

**Two tests held the old wording**: one expected "3-byte string" and one expected the old refusal text. Both were
*updated*, not deleted - they are the tests that would have caught this had they been written from the standard's
words rather than from the compiler's.

195 unit, 28 parser, 29 CLI, 14 exe, 2 real, 1 crt, 5 x64, 4 hostile, 8 C API, 4 bcc20-sys, differential (415 ran,
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
