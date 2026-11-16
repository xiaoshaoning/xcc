# ADR-006: varargs, and the two things the back end cannot say

Status: accepted, 2026-11-16. Written before the code, because the first thing varargs needs is a
decision about where the missing expressiveness lives.

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
