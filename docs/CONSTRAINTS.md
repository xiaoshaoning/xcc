# Constraints

Not goals - rules. A change is checked against these before it is merged, and a change that
needs one broken needs a new [ADR](ADR-001-architecture.md) first, not a comment explaining why
this once is fine.

Each constraint says what it is, why it is worth the trouble, and how a reviewer can check it
without trusting anybody.

## 1. The front end owns no machine code

`lex`, `pp`, `parse`, `diag` and `cir` may name no Cranelift type, no x86 register, no COFF
record. Lowering happens in `back/`; the JIT is a sink, not a stage.

**Why**: a back end you cannot replace is a back end you are stuck with, and the ambition to
outgrow Cranelift (PLAN.md, M10) depends on this seam existing from the first commit.

**Check**: `cargo tree -p xcc` shows no `cranelift-*`; a CI step greps `crates/xcc/src/{lex,pp,
parse,cir}` for `cranelift`, `iced_x86` and `object`, and fails on a hit. Reviewers ask "which
crate is this in?" before "is this correct?".

## 2. No global state; a `Compile` owns everything

No `static mut`, no `lazy_static` or `OnceLock` holding compiler state, no thread-local that is
not a pure cache keyed by its input. Two `Compile`s on two threads must not see each other.

**Why**: the product is an embeddable compiler (PLAN.md, M7). A host will compile from a
request handler, and "one compile at a time per process" is the thing that makes `libbcc20`
unusable as a library.

**Check**: `#![deny(unsafe_code)]` in the library, relaxed only in `jit/`, `capi/` and
`bcc20-sys/`; the M4 acceptance test compiles 100 sources on 8 threads; review.

## 3. Refuse rather than miscompile

A construct the compiler does not understand is a diagnostic and **no artefact**. Never a
guess, never a silently wrong object, never "probably fine". Unsupported input is an error the
host can show a user; wrong code is a bug the host ships.

**Why**: this is the whole trust proposition of a young compiler. It is also cheap: refusing is
always easier than implementing.

**Check**: every entry under "refused" in [C99-SUPPORT.md](C99-SUPPORT.md) has a test asserting
the diagnostic *and* that no object was written; M3's acceptance is "no false accepts" against a
corpus of sources clang rejects.

## 4. Determinism

Same source, same options, same bytes: identical object files, identical diagnostic order,
across runs, machines and threads. Nothing in an artefact may come from the clock, the
environment, a random seed or the iteration order of a hash map.

**Why**: reproducible builds, build caches that hit, and differential testing that means
something. A compiler whose output wobbles cannot be compared with anything, including its own
previous version.

**Check**: interned symbols get their ids from content and order, never from addresses; symbols,
relocations and sections are sorted before they are written; `Options` carries the clock when a
host needs one. A test compiles the same source twice and compares byte for byte, and it runs on
every platform in CI.

## 5. Bounded work on hostile input

Every recursion has a budget: nesting depth in expressions and declarators, macro expansion
depth, include depth, source and output size, and diagnostics. Exceeding one is a diagnostic,
never a stack overflow, a hang or a process death.

**Why**: an embeddable compiler runs on the *host's* threads, with the host's stack, on input
the host did not write. `((((((...` must not take the host down. The parser is recursive
descent, so this is a decision, not an accident.

**Check**: `Budgets` in `Options` carries every limit; one test per limit asserts the
diagnostic; the fuzz smoke test feeds deeply nested and very large inputs. M2 fixes the stack
strategy (a depth budget, or a worker thread with a stated stack size) and writes it down here
before the parser exists.

## 6. No panics on user input, no `unwrap` on foreign data

A panic means a bug in xcc, not a property of the source. Source text, file contents, host
symbol names and CLI arguments are all foreign data.

**Why**: a panic inside a host's process is a host crash, and the host has no way to catch it.
At the C API boundary (M7) a panic would be undefined behaviour across the FFI anyway.

**Check**: fuzz targets from M1 onward (`cargo-fuzz` at M3) must run without a panic; the C API
(M7) wraps every entry in `catch_unwind`; review rejects `unwrap` on anything parsed from input.

## 7. MIT-clean: no borrowed code, no borrowed corpora

No code, tables, message texts or test sources from borland-reborn's translations (derived from
Borland's binary) or from TinyCC / `tinycc_in_rust` (LGPL). Reference compilers (`clang`, `tcc`,
`libbcc20`) may be **run** as oracles - comparing output is not derivation - and never linked
into a shipped artefact except the optional `--target=dos16` path, which is test-only.

**Why**: MIT is a promise to whoever embeds xcc, and it is easy to break once and impossible to
un-break.

**Check**: `deny.toml` plus a licence check in CI for every dependency; a reviewer asks where a
new corpus file came from; [AUDIT.md](AUDIT.md) records the corpora and their licences.

## 8. One target, one ABI, stated once

Windows x86-64, win64 calling convention, `long double` = `f64`. No `#[cfg]` branching for other
targets in shipped paths, no second ABI in the tests.

**Why**: two targets is twice the work and half the correctness, and PLAN.md's `--target=dos16`
is deliberately a test harness rather than a second back end.

**Check**: the ABI lives in one file (`back/abi/win64.rs`) and everything else calls it; a
`--target` that is not `win64` or `dos16` does not parse.
