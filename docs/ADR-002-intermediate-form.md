# ADR-002: the intermediate form

Status: accepted, 2026-11-15. Follows ADR-001 §9 ("our IR is the pivot; Cranelift is one
lowering"), which said *that* `cir` exists; this says *what it is*.

## Context

The front end will start producing an intermediate representation at M4, and its shape is the
most expensive thing in the project to change afterwards: every parser rule that feeds it, every
pass that reads it and both back ends are written against it.

Three implementations have already answered the question, differently:

- **Borland C++ 2.0** (recovered in `borland-reborn`) has no middle end at all: expressions are
  parsed into "code nodes" that are already x86-16 - registers, segments, operand kinds - and
  the generator walks them while it emits. Fast, small, and impossible to retarget: it took a
  second compiler (Borland's own 32-bit one) to reuse the front end.
- **TinyCC**, and the `tinycc_in_rust` port, make the parser the code generator: values live in
  "a register plus a stack temp", and Phase 7 grows an IR by making every value an IR value
  instead (their D7a). That decision was reached by rejecting the two alternatives below, for
  reasons that hold here too.
- **The book track** (`Writing-a-C-Compiler-in-Matlab`) emits TACKY: a flat list of
  three-address instructions with explicit temporaries and labels, then lowers it. Easy to write
  and to read, and a dead end for optimisation for the same reason a quad list always is.

## Decision

`cir` is a **control-flow graph of basic blocks with parameters, whose values are SSA by
construction**. No phi nodes: a value that a block receives from its predecessors is a parameter
of that block, exactly as Cranelift's IR states it.

Four invariants, in the order they constrain the design:

1. **Values are minted, never invented.** Only `Func::builder` mints a `Value`; a user cannot
   construct one. A value is defined in exactly one place, and definition dominates use.
2. **Joins are block parameters.** A branch carries the values its target's parameters receive,
   so a merge point needs no separate representation and there is no phi-insertion problem for
   values. (C *variables* are a different matter: they live in memory.)
3. **Memory is explicit.** `load` and `store` carry a width and a C type; the address of a
   variable is an ordinary value. C variables are memory until `mem2reg` (M10) proves a
   variable's address never escapes, at which point its loads and stores become values.
4. **Every instruction carries a `Span`, and every value carries its C type.** Those two are what
   make a diagnostic able to point at the line that produced a bad instruction, and what lets
   the M6/M10 differential harness say which expression disagreed.

`cir` also carries, per function: params, blocks, an entry and terminator rules, and calls (the
callee as a value, so function pointers need nothing special).

And one thing that is not an invariant but has paid for itself in every compiler that has it:
**a textual dump from the first commit** (`cir::dump`). You cannot diff a data structure, and the
harness that compares our two back ends at M10 compares text.

## What `cir` may not contain

- Registers, instruction encodings, calling conventions, alignment of the stack, `.pdata`: those
  are `back/`'s business (CONSTRAINTS 1 and 8).
- Cranelift types, `iced_x86` types, COFF sections. A back end names them; `cir` does not.
- C's conversion rules. `cir` knows a value's width and signedness; *when* to convert, promote or
  truncate is the front end's decision, made once, in the type checker.
- Global state. A `Func` is owned like everything else (CONSTRAINTS 2).

## Rejected

**A. A quad list (TACKY).** Cheapest to build, easiest to print, and the wrong shape for the two
things this project promised: M10's optimizer would have to construct SSA - dominators, phi
placement, renaming - before any pass could run, and `back-cranelift` would have to *convert*
rather than translate, because Cranelift's blocks take parameters. Paying the SSA cost once, in
the builder, is cheaper than paying it in every pass.

**B. Cranelift's IR as the middle end.** Rejected already in ADR-001 §9, and it stays rejected:
every optimisation becomes Cranelift's to grant, and "replace the back end" becomes "rewrite what
the front end emits".

**C. An AST, then lowering.** The textbook shape, and a second representation of knowledge the
front end already holds: C's types, scopes, storage classes, implicit conversions, initialisers.
The front end's knowledge lives in its symbol table and type table; a parallel tree would have to
be kept in step with them, and the bug that results is a *silent* disagreement between two
encodings of the same fact. The cost of the builder being fussy (every expression must yield a
value, every join needs parameters) is paid once; the cost of two encodings is paid forever.

## Consequences

- The parser cannot simply "emit as it goes" into registers; it must yield a value for every
  expression. That is the discipline that makes the front end retargetable, and it is the main
  reason M3 is XL rather than L.
- Until `mem2reg` exists (M10), `cir` looks naive: a local variable is a stack slot and every
  access is a load or store. That is honest - it is what the C says - and the passes that make it
  fast come later, on this shape rather than on a rewrite of it.
- `Func::verify()` is part of the interface from the first commit: values defined before use,
  blocks terminated, branch arguments matching parameters, types consistent. A verifier failure
  is a compiler bug, not a diagnostic, and it runs in debug builds and in tests.
- Debug information (DWARF/PDB) later maps through `cir`'s spans; nothing here prevents it.
