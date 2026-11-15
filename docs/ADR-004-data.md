# ADR-004: where globals and string literals live before lowering

Status: accepted, 2026-11-15. The design half of M5's first blocker; the code follows it.

## The problem

`back` and the JIT can name a function (`GlobalAddress` -> `func_addr`), but nothing names an
*object*: a file-scope variable, a `static`, a string literal. The emitter refuses all three,
which is why `hello.exe` cannot exist yet. Two facts have to be decided once:

1. what a translation unit's data is, in `cir`;
2. who computes its bytes, and when.

## Decision

**`cir::Global` is the data, and the front end fills it in.** The struct already exists
(`name`, `ty`, `defined`, `init: Option<Vec<u8>>`, `external`, `span`); M5 makes the parser use
it:

- A file-scope declaration with an initialiser produces `Global { defined: true, init: Some(..) }`;
  one without produces `defined: false` (a declaration, resolved by the linker).
- **The bytes are computed by one function**, `init::bytes(ty, initialiser)`, in the front end,
  because only the front end has the type layout (`Types::size_of`, member offsets, bitfields)
  and the constants. A back end that recomputed them would be a second implementation of C's
  layout rules - the exact duplication ADR-002 rejected.
- A **string literal becomes an anonymous `Global`** with a generated name (`.` + a counter, so
  it can never collide with a C identifier) and `init` = its bytes, `external: false`,
  `defined: true`. Its value in an expression is a `GlobalAddress` of that name, with type
  `pointer to char`. Two adjacent literals are already concatenated by the emitter, so one
  literal run is one global - no merging pass, and no deduplication until M10 measures it.
- `GlobalAddress` in `back` grows the object case: `declare_data` with the C type's size and
  alignment, `Linkage::Export` when `external` and `Local` otherwise, `write_data` for the
  initialiser (or `write_zeroes` for `.bss`), and `symbol_value` for the address. Relocation
  between globals (a `char *p = "x";` at file scope) points at the anonymous global like any
  other.

## Rejected

- **Bytes computed in the back end.** Layout would then live in two places: `Types` for `sizeof`
  and the back end for data. When they disagreed, the program would compile and read the wrong
  bytes.
- **String literals as a special instruction.** `cir` would grow a second way to name memory
  beside `GlobalAddress`, and every pass would have to know both. A literal *is* an object.
- **Deduplicating equal literals now.** It needs a map keyed by bytes, a policy on when two
  literals are "equal" (a suffix `L` changes the type), and a reason. M10 can measure the win.
- **`.rdata` as a separate section hint.** One read-only flag is not worth a `cir` field until
  something needs the distinction; `Linkage::Local` plus no writes is what the linker sees.

## What the first attempt taught (recorded after it was reverted)

The list walker was first written as **a second initialiser parser** - its own `{`/`,`/`}` loop,
its own indexing, its own surplus check - and reverted. Every failing test named the same fault:

    int a[2][2] = {{1,2},{3,4}};   expected an expression, found `{`   (no brace elision)
    int a[5] = {[2] = 7};          a designator in a file-scope initialiser
    char s[2] = "abc";             a generic refusal, not "cannot hold this 3-byte string"

`initialize_array` and `initialize_record` already do all of that, and the corpus tests them. A
second reader cannot match them without becoming them, so the walker is not a parser at all:

    fn initialize_array(&mut self, ty, element, declared, braced, out: &mut Option<Vec<u8>>)

The traversal stays single. A leaf that can write bytes copies them into `out` at
`member.offset` / `index * size_of(element)`; a leaf that cannot (a non-constant, a pointer to a
literal) clears `out`, and the existing type checker keeps reporting its existing diagnostics.
`initializer_bytes(ty)` becomes a wrapper: `out = Some(vec![0; size])`, call the existing
`brace_initializer`/`initializer`, return `out`. One traversal, one set of messages, bytes as a
by-product - which is what "the bytes are computed by one function" in the decision above meant
all along.

This is the sixth appearance in this project of the same red flag (one fact in two places): the
escape tables, `cir`'s block invariant, the C-to-Cranelift signature, the escape table again, the
"is there an initialiser?" peek, and this. Look for it first.

## Blocker found starting patch 3: the front end cannot mint a name

The decision above says a string literal is an anonymous `Global` with a generated name and
`external: false`. Writing that hit a wall in the first hour: `cir::Global.name` is a `Symbol`,
symbols only come from an `Interner`, and the parser's interner is `&'a Preprocessed` - owned by
the preprocessor and **immutable once preprocessing has finished**. The parser can read
`spell_of(name)`; it cannot make `.str0`.

Three ways out, and the choice is a real one:

1. **Own the interner in the parser.** `Preprocessed` hands its table over once preprocessing
   ends, and the parser keeps it mutable. Cheap, and it changes who owns a fact that currently
   has exactly one owner - the reason the parser is a pure function of the tokens.
2. **`cir::Global.name` becomes a `String`.** Then a synthetic name needs no interning, and a C
   name costs one allocation at the point the global is pushed. The dump stops needing the name
   table for globals, which is also true of nothing else in `cir`.
3. **A second interner for synthetic names, with a marker** (`Symbol` gains a namespace bit, or
   the back end is told which table a symbol belongs to). Every reader of a `Symbol` then has to
   know which table minted it - the "unknown unknowns" the principles warn about most.

Chosen: **(2)**, and applied to `cir::Global.name` in the same commit that wrote this record. The
grounds that a global's name is data, not a token: it is written
once, read by a linker, and never compared with another `Symbol`. (1) trades a clean ownership
boundary for a convenience; (3) buys a whole class of subtle bugs for nothing.

`Inst::GlobalAddress` still carries a `Symbol`, and patch 3 needs that to change too: an anonymous
global has no symbol. It is deliberately *not* changed in this commit, because the two names
answer the same question and must answer it the same way - `Func::name` (a `Symbol`, minted while
reading a declaration) is the third member of that family, and the rule "a name that comes from
the source is a symbol, a name the compiler generates is a string" needs writing down before the
third shape arrives. That rule is the next decision, not a patch.

The question is recorded before the code because it comes back for `static` counters, `__func__`,
and every compiler-generated label - and because the wrong answer here is expensive to reverse,
which is exactly what an ADR is for.

## Consequences

- `cir::Global` gains no fields, which is the test of whether the shape was right at M4.
- The parser's file-scope path currently refuses (`not_lowered`) - that refusal moves to the
  initialiser reader, which knows whether a constant initialiser it cannot evaluate is the
  reason.
- `hello.exe` needs `puts` from UCRT as well; that is a linker story, not this one.
