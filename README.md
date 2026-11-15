# xcc

A small, embeddable C compiler for Windows x86-64, written in Rust.

`xcc` compiles C to native code in-process: to a COFF object you link, or to a JIT module
you call as a function pointer. It is meant to be embedded in other programs (scripting
hosts, editors, plugin systems, test harnesses), not to replace clang.

- Fast: no optimisation pass by default, compile-and-call in milliseconds.
- Safe to embed: no global state, compiles in parallel, limits on untrusted input, no panics.
- Small: one static binary, one `.dll`, no toolchain needed to compile; the Windows SDK is
  needed only to link a final `.exe`.
- C99, the whole standard, plus the C11 pieces a scripting host meets; `long long` included.

It grew out of [borland-reborn](https://github.com/abedegno/borland-reborn), which recovered
Borland C++ 2.0's source from its binaries, and it takes its method from `tinycc_in_rust`, a
local, unpublished Rust port of TinyCC. Nothing from either is used here: the first is a
byte-exact 16-bit DOS compiler derived from Borland's binary, the second is LGPL, and this
compiler is MIT and owns its code. `xcc` can still drive the 16-bit one, through `libbcc20`,
as `--target=dos16` - for comparing the two, never for shipping.

The ambition is to keep what each is good at: TCC's speed, size and habit of being embedded,
Borland's optimiser, and a licence and an API that a modern host can adopt. See
[PLAN.md](PLAN.md), "where the quality comes from".

**Status: M1.** The library, the diagnostics and the command line work; there is no code
generation yet, so `xcc` reports "the C front end is not implemented yet" and exits 1.
[PLAN.md](PLAN.md) has the milestones and their acceptance criteria;
[docs/ADR-001-architecture.md](docs/ADR-001-architecture.md) records the decisions.

```sh
cargo run -p xcc-cli -- --version
cargo run -p xcc-cli -- -c --format msvc t.c     # diagnostics as MSBuild expects them
cargo test --workspace
```

Licence: MIT.
