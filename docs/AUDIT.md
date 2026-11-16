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
