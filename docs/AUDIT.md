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
