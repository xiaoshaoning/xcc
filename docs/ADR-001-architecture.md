# ADR-001: architecture

Status: accepted, 2026-11-15. Supersedes nothing; this is the first record.

## Context

`borland-reborn` recovered Borland C++ 2.0's source from `BCC.EXE` and now has a native,
in-process C compiler (`bcc20-port`, `libbcc20`) that reproduces Borland's 16-bit DOS objects
byte for byte. That artifact cannot produce anything that runs on a modern Windows machine:
its output is OMF for an 8086, its data model lives in a simulated 640 KB DOS image, and it
needs Borland's own headers, which are not redistributable.

The goal here is different: a C compiler that is *useful on Windows today* and *embeddable* -
compiled at runtime by a host program, called as a function pointer, shipped as a small
library.

## Decision 1: a new compiler, not a rescued one

Rejected: retargeting `bcc20-port`'s front end. Three reasons, any one of them fatal.

1. **Its state lives in a simulated 640 KB DOS image.** The abstract syntax trees are `u16`
   offsets into a fake segment (`farptr`, `MK_FP`); the parser allocates from Borland's own
   pools. From Rust that is not an AST, it is `unsafe` FFI into a machine model.
2. **There is no middle end to hook.** Code generation is fused with expression parsing
   (`CGEXPR.C` emits instructions while it walks), and the "code nodes" are x86-16 flavoured:
   near and far pointers, segment overrides, an 8-register world. Retargeting means replacing
   it, not reusing it.
3. **Licensing.** Those sources are translations of Borland's binary. A compiler built on them
   could never be MIT.

Rejected: porting TCC, chibicc or QBE. Each is a fine compiler, but the point is a Rust
library with a Rust API, structured diagnostics, no global state and no `unsafe` in the parser
- none of which a C code base gives cheaply.

## Decision 2: Rust

The compiler is a large mutable pointer graph with lifetimes; Rust removes a class of bugs that
are expensive to find in compilers, and the ecosystem is unusually complete for this specific
job:

| need | crate |
|---|---|
| COFF objects, relocations | `object` |
| instruction encoding, decoding, listings | `iced-x86` (as `--emit=asm`) |
| code generation | `cranelift-codegen`, `cranelift-jit`, `cranelift-object` |
| diagnostics with spans and carets | `miette` |
| tests | `insta` snapshots, `proptest`, `cargo-fuzz`, `criterion` |

`unsafe` is confined to `jit/`, `capi/` and `bcc20-sys/`; the rest of the workspace forbids it.

## Decision 3: Cranelift

| | Cranelift | LLVM (`inkwell`) | own emitter (`iced-x86`) |
|---|---|---|---|
| first working JIT | days | weeks | months |
| dependency | pure Rust | LLVM dev libs | none |
| code quality | adequate, no optimisation by default | best | ours to write |
| 80-bit `long double` | no | yes | yes |
| debug info | limited | DWARF for free | no |

Chosen: Cranelift. Embedding wants compile-and-call latency, not a code generator to maintain;
a scripting host that cares about speed uses clang, not this. `long double` therefore means
`f64` - which is what MSVC does for x86-64 in any case - and is documented rather than faked.

## Decision 4: Windows x86-64 only

One target, the ABI made explicit (`abi/win64.rs`: shadow space, 16-byte stack alignment,
varargs register/stack split, `__chkstk`), and no target abstraction until a second target
exists. The middle end keeps `Layout` and `CallingConvention` behind traits so that adding a
target stays a back-end job.

## Decision 5: embedding shapes the API, not just the packaging

Consequences that are cheapest to honour now and expensive later:

- **No global state.** Every compile owns a `Compile`; parallel compiles in threads are a test,
  not an aspiration. This is the opposite of `bcc20-port` (one compile at a time per process).
- **Diagnostics are data first.** `Vec<Diagnostic>` with spans; rendering is a separate step so
  a host can show them in its own editor.
- **The JIT is the point.** `Jit::add_source`, `add_symbol`, `get_fn` are exported before the
  object writer is, because "compile this string and call it" is the use case.
- **Untrusted input is expected.** Budgets for source size, expansion depth, compile time;
  includes resolvable only through a host-provided resolver.
- **A C API exists** (`libtcc`'s shape) so non-Rust hosts can embed it too.

## Decision 6: `--target=dos16` stays, as a measuring stick

`libbcc20` exposes a clean C API, so a thin FFI crate can compile the same source with a 1991
compiler and with this one, and print time, code size and diagnostics side by side. It is not
a supported output target; it is how the new front end gets cross-checked against an
implementation that shipped, and how progress gets measured.

## Consequences

- A usable C compiler is 3 to 6 months of work, mostly front end; the JIT demo at M4 is the
  first honest checkpoint.
- `long double` is 8 bytes. Code that needs 80-bit precision is out of scope.
- No optimisation pass: `-O` selects Cranelift's level and nothing more. An optimiser is a
  separate project.
- Signatures on target: x86-64 Windows and nothing else, so no `#ifdef _WIN64` variants, no
  second ABI in the tests.
