# ADR-008: this compiler's own back end

Date: 2026-11-16

## Status

Accepted. Level 2 is implemented as a slice; the rest of the language refuses with a source span.

## Context

Cranelift is what compiled everything before this: it is a good back end, it is not this project's
back end, and three things were waiting on that difference.

1. **The plan's levels.** PLAN.md defines level 2 as "this compiler's own code", and levels 3 as
   more of it. Until now `--opt-level=2` was refused, so "the compiler optimises" meant "Cranelift
   optimises something the front end handed it".
2. **Two lowerings.** M10's fourth item is a differential harness that compiles with both lowerings
   and compares behaviour. With one lowering there is nothing to compare: the harness compares the
   compiler against clang, which is a different question.
3. **`va_arg` and `%f`.** ADR-006 refused them because Cranelift cannot write `AL` or move a value
   through a named physical register. A back end that names its own registers can.

The measuring stick (ADR-007) put a number on the gap: 1981 bytes of x64 for the six-file corpus at
level 1, against `clang -O2`'s 1037 and Borland C++ 2.0's 1233. Cranelift at `opt_level=speed` is
not going to close that.

## Decision

Write an x86-64 back end, in a slice: **integer code with frames, calls and control flow**. Level 2
selects it and Cranelift is not in the path at all, so a number cannot quietly mean somebody else's
compiler.

Five decisions shape it.

**`cir` is enough.** The back end consumes the same `cir` that Cranelift does, including block
parameters and the front end's ABI lowering. It contains no type enquiry Cranelift also does; the
few `cir` facts it needs (a type's width and signedness) are read from the type table with the C
type's meaning, not a machine one.

**A value in a register is canonical for its C type**: a signed `int` is sign-extended to 64 bits,
an unsigned one zero-extended, on the way in. Every operation is then a 64-bit operation, and only
the instructions that touch memory, compare, or narrow on the way out look at a width. This is one
rule in one place (`lower::canonical` and the encoder's `canonicalise`) instead of a per-width case
in nine operations.

**Block parameters live in frame slots.** An edge stores its arguments in the target's slots and
jumps; the block reads them back at its start. A phi node's central difficulty is that its incoming
values must arrive at once, and a slot has one name, so the difficulty is gone. The cost is memory
traffic on every edge, which the next round of work (shrinking, coalescing) can remove where it
matters.

**Only callee-saved registers are allocated**: `rbx`, `rsi`, `rdi`, `r12` to `r15`. A call then
cannot destroy a live value, so there is no second allocation around calls and no spilling of
everything live across one. The price is seven registers instead of fifteen. `rax`, `rcx`, `rdx`,
`r10` and `r11` are scratch, owned by the encoder: an operation moves an operand in, computes, and
moves the result back. Linear scan (Poletto and Sarkar) with whole-interval spilling is the
allocator; interval *splitting* is the next improvement, not this one.

**The frame is `rsp`-relative and `rsp` never moves after the prologue.** Shadow space, the saved
registers, the lowering's frame, spill slots, and the staging area for call arguments are one
layout, in that order, decided when the function is encoded. No `rbp`, no `push`, no unwinding
information to get wrong. Arguments are staged through memory before the four argument registers are
loaded, because two arguments can swap positions and a staging area has no cycle in it.

**A gap is refused where it is found.** The back end returns a `NotYet` with the source span of the
instruction that asked, and `compile` renders it as an ordinary refusal - `not lowered yet at opt
level 2: a floating-point value in this back end` - not as `internal:`. A compiler bug is still a
bug and says so; a construct nobody has written yet is a gap and says that instead.

The object file is written here too (`coff.rs`): a 20-byte header, one `.text`, function symbols,
undefined externals, a string table, and `IMAGE_REL_AMD64_REL32` relocations for calls. `lld-link`
links what it produces; the host's linker is still the linker.

## What the slice does

Integer and pointer scalars of every width; arithmetic, division, remainder, shifts, comparisons
signed and unsigned; conversions between integer types; `_Bool`; loads and stores of 1, 2, 4 and 8
bytes; frame slots for variables whose address escapes; all control flow the front end emits
(`return`, `jump`, `branch`, `switch` as branches, `goto`); direct calls with up to four integer
arguments and an integer or `void` result; recursion.

## What it refuses, and why that is the design

Floating point; indirect calls and taking a function's address; aggregates by value; global objects
(no `.data` yet); more than four arguments; `va_arg`. Each is a `NotYet` with a span, so the
refusal reads like the two hundred other refusals in this compiler and a program never runs
*differently* from the same program at level 0 - it either agrees or does not compile.

## Evidence

- **300 of the 464 corpus programs** build through this back end, link, run, and agree with the same
  program built by Cranelift's path. `crates/xcc/tests/x64.rs` is the ratchet; the 164 refusals are
  counted and named.
- The hand-written constructs the slice covers are in the same test: arguments, recursion, loops
  through block parameters, `switch`, `break`/`continue`, arrays, pointer walks, narrow-type
  sign-extension, 64-bit arithmetic, signed and unsigned shifts.
- Bugs the differential harness found while the slice was being written, each of which would have
  been a silent wrong answer: a SIB byte emitted for a register-direct operand (every immediate
  shifted by one byte), the REX prefix missing from `F7`/`D3` forms (division by the wrong
  register), `rsp`-relative operands needing no SIB when the mode is register-direct, byte stores to
  `sil`/`dil` without a REX prefix (wrong byte stored), and the entry block's parameters being read
  from slots that nothing had written.

**Code size, measured, before any shrinking:** 60 108 bytes across the 302 corpus programs that
compile, against Cranelift's 11 626 for the same programs and `clang -O2`'s 9 255. The slice is
**5.2x Cranelift and 6.5x clang**. That is the honest state of a first slice: correct, and large,
because every operation goes through scratch moves, every edge through memory, and nothing is
coalesced or peepholed.

**The first shrinking round took it to 40 716 bytes** - 3.5x and 4.4x - and the measurement said
what to do before any of it was written: 73.4% of the instructions were `mov`. Computing in the
destination's own register when an operand is already there, and letting a computed value coalesce
with the value it is computed from, removed a third of the bytes (11 028 instructions to 8 798, 7 235
`mov` to 4 985) with the 302-program ratchet unchanged. Coalescing block parameters, the next item on
the list, turned out to be 2.6% of the instruction stream and was dropped rather than written: the
list is a prediction, and the measurement gets the last word.

## Consequences

- `--opt-level=2` compiles a subset and refuses the rest, with a span. A host that asks for level 2
  gets this compiler or a diagnostic, never Cranelift under another name.
- `--run` still takes levels 0 and 1: this back end writes objects, and mapping a frame into memory
  to run it is a separate piece of work.
- A second lowering exists, so M10's cross-lowering harness can finally compare two back ends rather
  than one back end against clang. The corpus ratchet in `tests/x64.rs` is that comparison, one
  level deeper.
- ADR-006's two blockers (`AL`, physical registers) now have a back end that can address them; the
  `va_arg` work is unblocked, not done.
