# xcc

A small, embeddable C compiler for Windows x86-64, written in Rust.

`xcc` compiles C to native code in-process: to a COFF object you link, or to a JIT module
you call as a function pointer. It is meant to be embedded in other programs (scripting
hosts, editors, plugin systems, test harnesses), not to replace clang.

- Fast: no optimisation pass by default, compile-and-call in milliseconds.
- Safe to embed: no global state, compiles in parallel, limits on untrusted input, no panics.
- Small: one static binary, one `.dll`, no toolchain needed to compile; the Windows SDK is
  needed only to link a final `.exe`.
- C89 plus parts of C99/C11 that a scripting host actually meets, `long long` included.

It grew out of [borland-reborn](https://github.com/abedegno/borland-reborn), which recovered
Borland C++ 2.0's source from its binaries. Nothing from that project's recovered sources is
used here: that compiler is a byte-exact 16-bit DOS artifact, and this one is a new compiler.
`xcc` can still drive it, through `libbcc20`, as `--target=dos16` - useful for comparing
the two, not for shipping.

**Status: planning.** [PLAN.md](PLAN.md) has the milestones and acceptance criteria;
[docs/ADR-001-architecture.md](docs/ADR-001-architecture.md) records the decisions.

Licence: MIT.
