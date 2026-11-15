# ADR-003: win64 unwind data, and what M5 may promise

Status: accepted, 2026-11-15. Answers the gate PLAN.md put before M5 ("a Cranelift object for
win64 carries `.pdata`/unwind information, or is proven not to need it for C").

## What was measured

`back/tests::a_win64_object_says_which_unwind_data_it_has` compiles
`int add(int a, int b) { return a + b; }` with `cranelift-object 0.117.2` and scans the COFF
section headers:

```
.pdata: false, .xdata: false, .text: true, .reloc: false
```

Cranelift's win64 object emission writes `.text` and the symbol table, and **no unwind data**.
The test asserts that fact rather than printing it, so the day the dependency changes, the gate
fails loudly instead of quietly invalidating this record.

## Decision

M5 emits and links objects as planned, and does **not** claim Windows exception dispatch.

- Plain calls, returns and stack frames work: Windows does not require `.pdata` to *call* a
  function, only to *unwind* through it (SEH, `RaiseException`, debuggers walking the stack).
- What is lost is: exceptions thrown through compiled C, `/EHsc`-style library callbacks that
  unwind past it, and a debugger's ability to show frames below a compiled function.
- So M5's promise is "a `.exe` that runs and calls C functions", and the `.pdata` gap is named in
  the release notes rather than papered over. `xcc` compiles C, and C has no exceptions: the gap
  only bites when a host's library unwinds through xcc-compiled code.

Unwind data becomes xcc's own work when something needs it - M5's `.exe` test on a host with an
installed exception handler, or `setjmp`/`longjmp`, whichever comes first - because generating
`.pdata`/`.xdata` for a function whose frame we control is a matter of writing them into the
object after Cranelift has produced the code and the frame layout.

## Alternatives, and why not

- **Emit no objects, only a JIT.** The JIT is lossless in this respect (it never unwinds through
  Win32 SEH), and it is what M4 shipped. But "a `.exe`" is the point of M5 and the project's
  second promise to a user who does not embed.
- **Use `emit_unwind_info` from `cranelift-codegen`.** 0.117 exposes it for the SystemV kind;
  the win64 kind is not in the version resolved here. Revisit when a newer Cranelift is taken,
  and this ADR says so.
- **Refuse to produce a `.exe`.** Honest, and useless: nothing about a `.exe` needs unwind data
  until something unwinds.
