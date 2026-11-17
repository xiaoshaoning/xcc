# ADR-006: varargs, and the two things the back end cannot say

Status: accepted 2026-11-16, amended 2026-11-16. Written before the code, because the first thing
varargs needs is a decision about where the missing expressiveness lives. **Amended the same day**
when the second of its two blockers turned out to be expressible after all: see
[The amendment](#the-amendment-per-call-site-signatures-via-an-indirect-call) at the end. The
original decision is kept as written, because the reasoning that was wrong is worth remembering.

## Context

A variadic call is the last common C construct xcc refuses, and `printf` is the reason it matters:
most of the corpus calls it, the runtime prints with it, and a host embedding xcc will pass one to
code it compiles. The plan puts varargs in M6, with `va_list` from `stdarg.h` over the Windows
ABI.

Win64's rule for a variadic call is two rules, and both are the *caller's*:

1. **The duplicate.** A `float` or `double` that is a variadic argument is passed twice - in the
   next XMM register and in the next general-purpose register - so a callee can `va_arg` it out of
   whichever register file it saved. (The fixed parameters are passed normally.)
2. **`AL`.** The caller sets `AL` to the number of vector registers the call used, so the callee's
   prologue knows how many `xmm` registers to spill into its register-save area.

Both are expressible in the front end's model - the duplicate is two `cir` values, a `double` and
its bitcast, in the call's argument list; the count is a number the parser knows - and neither is
expressible in the back end Cranelift gives us:

- **`AL` is a physical register, and `cir` has none.** Cranelift is a register allocator: a value
  is whatever register the allocator picks, and there is no instruction that writes `rax` before a
  call. `cranelift-codegen` 0.117 has no mention of `variadic` or `varargs` anywhere - it does not
  model the rule either. `printf` reads `AL` to decide whether to spill `xmm0`-`xmm3`; because the
  duplicate puts every variadic `double` in a general-purpose register as well, the spill is not
  what a `va_arg(double)` reads - but that is a property of one callee, not of the ABI, and
  "printf worked on my machine" is not a rule to compile by.
- **A variadic call has a different signature at every call site**, and
  `cranelift-module`'s `declare_function` refuses two signatures for one name
  (`FunctionDeclaration::merge` returns `IncompatibleSignature` when the signature differs). So
  `printf("%d\n", 1)` and `printf("%s %f\n", s, d)` cannot both be declared against the same
  symbol, and there is no per-call-site signature to be had.

Neither is a missing line of code. The first needs `cir` (or the back end) to grow a notion of a
call-site register - an instruction the allocator must not touch - which is target knowledge with
no home in the current architecture. The second needs the object module to key declarations by
`(name, signature)`, which is a change to a dependency.

## Decision

**Variadic calls stay refused, and the refusal says what is missing.** The message keeps its
"not lowered yet" wording (a host or a user can recognise it as a gap) and the two constraints go
into the diagnostic's *note*, which the renderer already prints and a host can show apart from the
message - `Diagnostic::message` is documented as one sentence, and this reason is not:

```
t.c:1:58: error: a call to a variadic function is not lowered yet (M4.2b)
  = note: Win64 sets `AL` to the vector-register count and gives every call site its own
          signature; this back end can express neither (docs/ADR-006-varargs.md)
```

That made `not_lowered` a special case of a new `not_lowered_with(span, what, Option<&str>)` - the
ordinary refusal passes no note, and this one does.
**The front-end half is not written early.** The duplicate and the promotion belong in
`emit_call`, next to the aggregate rules; adding them now would be code no test can run, and the
shape of the call will change the moment `AL` is expressible. The knowledge lives in this file
until then.

**`va_list`, `va_start` and `va_arg` are the same question from the callee's side** and are not
started either: a callee that cannot spill `xmm` registers cannot implement `va_arg` over them.
M8's headers will need this ADR to be overturned, not extended.

## Consequences

- `printf` remains unusable, and so remains the most common refusal in the corpus. M6's
  differential runner (compile *and run* corpus files with xcc and with clang) must therefore
  choose its programs from those that print nothing, until this is lifted.
- The call-side work that *is* independent of this - aggregate arguments, floats promoted to
  `double`, the fixed parameters - is already done; a variadic call needs only the two rules above
  added to `emit_call`.
- The two upstream facts to re-check when picking this up: whether `cranelift-module` has learned
  to key declarations by signature, and whether `cranelift-codegen` has grown a way to set a call
  register (`AL`). Both are one grep each; both are the whole reason this ADR exists.
- If neither changes, the path is M10's own emitter, which owns its call sequences and can say
  `mov al, N` because it is writing the instructions.

## The amendment: per-call-site signatures via an indirect call

The second blocker was not a blocker. `cranelift-module` refuses to declare one name with two
signatures - but a **`call_indirect` needs no declaration at all**: it takes a signature and an
address. So a variadic call is lowered as:

1. a signature built from the *arguments of this call site* (`signature_of`, the same function a
   pointer call already used, ABI rules included);
2. the address of the callee - `func_addr` of the `FuncRef` a direct call would have used, or the
   pointer value when the callee is already one;
3. `call_indirect`, which passes exactly those arguments and returns what the signature says.

The front-end half is the default argument promotions (C99 6.5.2.2p6): `char`, `short` and `_Bool`
to `int`, `float` to `double`, applied in `call` to the arguments past the declared parameters.
`_Bool` is already 0/1 by comparison, and a promoted argument is one `Convert`, so nothing else
changed.

**`AL` remains out of reach, and it now bounds what works instead of blocking everything.** What
the callee does with `AL` is decide whether to spill its vector registers into the register-save
area. If no floating-point argument is passed, either decision is harmless: the callee spills
registers nobody asked it for, or spills nothing and reads nothing. So:

- **integer, character, string and pointer arguments work** - `printf("%d %s %x")`, `snprintf`,
  `fprintf`, `sscanf`'s input side - and are the overwhelming majority of calls;
- **a floating-point argument is refused** with a diagnostic that is *not* "not lowered yet": it
  cannot be passed, because a callee that finds `AL` at zero reads nothing where the argument
  should be. Passing it in both files (the ABI's duplicate) does not help: which of the two
  `va_arg` reads is the callee's choice, and it is the vector one for a `double`.

That refusal is honest and permanent until the back end can write a physical register. It is also
small: a `%f` in a format string is the only thing it costs a `printf` user.

**What the amendment costs.** A variadic call is an indirect call - a `func_addr` and a
`call_indirect` instead of a direct `call` - so its callee cannot be inlined and it pays one
indirection. At `opt_level = none` that is indistinguishable from what a direct call costs, and it
is what every other compiler does at a call site whose signature only it knows.

**Still refused: `va_list`, `va_start` and `va_arg`.** These are the callee's side of the same
convention, and the same `AL` question decides whether a variadic function can exist in C compiled
by xcc. `stdarg.h` refuses with a pointer to this ADR; a program that defines its own variadic
function cannot be compiled yet, and one that calls `printf` can.

**A JIT cannot be the linker.** `--run` resolves symbols when it finalizes, and a call to `printf`
is a call to something the JIT does not have; that is a diagnostic now rather than a panic inside
`cranelift-jit` (`back::jit` checks every address a unit mints against what it defines and what it
was given). A program that prints is built with `-c` and linked, which is M8's acceptance path.
