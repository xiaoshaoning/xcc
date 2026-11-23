# ADR-005: the `crt`, and what an xcc `.exe` is allowed to need

Status: accepted, 2026-11-15. The last piece of M5 that is decisions rather than code.

## Context

`xcc` produces a COFF object. Something has to turn the process's entry point into a call to
`main`, seed the C library, and end the process - and on Windows that "something" is the C
runtime's startup, which every MSVC-built program inherits and which `xcc` does not have.

Four questions have to be answered together, because each one constrains the others:

1. what the entry point is, and who writes it;
2. where `argc`/`argv` come from;
3. how the process ends;
4. how much stack probing the prologue needs.

And a fifth that decides what a user must install:

5. what libraries the link needs, and therefore whether "build a `.exe` with xcc" requires Visual
   Studio.

## Decision

**1. `mainCRTStartup` is written in C, compiled by `xcc`.**

```c
void exit(int);
int main(void);

void mainCRTStartup(void) { exit(main()); }
```

Nothing here needs assembly. The linker's `/ENTRY:mainCRTStartup` points at it; Win64 gives a
function that takes no arguments and never returns the shape above. Writing it in C means the
first program `xcc` links is also the first program it compiles from its own runtime - and it is
a test of the compiler rather than a second implementation to keep in step.

**2. `argc`/`argv` are the runtime's, and not this file's.**

`GetCommandLineW` returns one wide string; C's `main` wants a `char **`. Between them sit
`CommandLineToArgvW` (shell32), `WideCharToMultiByte` (kernel32), and a heap for the result -
three imports and a quoting rule that C's standard describes in a paragraph and Windows
implements its own way. Doing it now would put an argument-parsing subsystem in the only place
in the project that cannot be tested in isolation.

The decision was to defer, and then the library turned out to export the whole answer:
`_configure_narrow_argv` splits the command line with Windows' own quoting rule and the two
accessors hand back what it stored - the same three imports `clang`'s own startup uses. So the
entry point is three lines and no parser:

```c
_configure_narrow_argv(_crt_argv_unexpanded_arguments);
exit(main(*__p___argc(), *__p___argv()));
```

A program whose `main` is `int main(void)` is unaffected: Win64 ignores an argument register the
callee does not name. Measured - `xcc args.c -o args.exe` then `args.exe one "two words" ""`
prints four arguments, keeps the quoted one whole, and returns `4` from a `main` that returns
`argc`.

**3. The process ends with UCRT's `exit`, not `ExitProcess`.**

`ExitProcess(main())` looks simpler and is wrong: it skips the at-exit handlers, which is where
`printf`'s buffer is flushed. A `hello.exe` built that way prints nothing and exits 0 - the worst
kind of wrong, because the exit status looks like success. `exit` flushes, runs `atexit`
handlers, and calls `ExitProcess` itself.

**4. Stack probing is Cranelift's, inline.**

`enable_probestack = true` with the inline strategy, so a large frame touches its guard pages as
it allocates them and no `__chkstk` import is needed. This is a backend setting, not a `crt`
one, and it lives in `back` where the frame is built.

**5. The link needs the Windows SDK's import libraries, and the acceptance test says so.**

`lld-link` (or MSVC `link.exe`) plus `ucrt.lib`, `kernel32.lib`: that is where `exit`, `printf`
and everything else lives, and no amount of compiler work removes it. What *can* be chosen is
which linker, and the answer is **`lld-link`** - `rust-lld` ships with every Rust toolchain, so
the acceptance test needs no Visual Studio, only the SDK libraries that a Windows machine with a
C toolchain already has.

A hand-written import table (a `.edata` section naming `ucrtbase.dll`'s functions) would remove
even the SDK dependency. That is a later milestone's option, and it buys a smaller requirement
for a much larger piece of work; it is recorded here so the choice is visible rather than
accidental.

## Measured on the development machine, and the correction it forces

Decision 5 said `lld-link`, on the reasoning that `rust-lld` ships with every Rust toolchain.
Measured:

    rustc --print sysroot      -> stable-x86_64-pc-windows-gnu
    <sysroot>/lib/rustlib/x86_64-pc-windows-gnu/bin/gcc-ld/ld.lld.exe   ELF linker, not COFF/PE
    C:/Program Files/Microsoft Visual Studio/2022                       no Hostx64/x64/link.exe
    cc --version               -> cc.exe (Rev8, Built by MSYS2 project) 15.2.0
    /c/msys64/ucrt64/lib/libucrt.a                                      UCRT import library

So there is **no `lld-link` on this machine**, and the Rust toolchain installed is the *GNU* one,
whose `ld.lld` links ELF and not PE. Decision 5's premise - "no Visual Studio needed" - is true of
a toolchain that is not installed here, which makes it a claim about someone else's machine.

Three ways out, and the choice is the next decision rather than this one:

1. **MSYS2's `cc` with `-nostartfiles -Wl,-e,mainCRTStartup`**, linking `-lucrt`. Present, produces
   PE, works today - at the cost of requiring MSYS2, a heavier ask than the SDK for this
   project's audience.
2. **Require VS Build Tools** (or the MSVC Rust toolchain, whose sysroot carries `lld-link`).
   Honest and standard; makes the acceptance test need a toolchain the compiler does not.
3. **A hand-written import table** (`.edata` naming `ucrtbase.dll`'s functions), which removes the
   linker's *library* needs and leaves only the linker.

What stands regardless: the entry point is `mainCRTStartup` written in C, the process ends with
`exit`, and probing is inline. What changes: **the linking story is not settled by this ADR**. It
is settled by which of the three the project is willing to require - a question about the
project's users, not about the compiler.

## Rejected

- **Assembly `_start`.** Faster to write for someone who knows the registers, and a second
  assembler dialect to maintain. C is testable with the compiler under test.
- **MSVC `link.exe` as the only linker.** It works, and it makes "build a `.exe` with xcc"
  require Visual Studio, which the project's own premise - embeddable, small, no toolchain
  ceremony - argues against.
- **`ExitProcess` directly.** Wrong for anything buffered; see (3).
- **`__getmainargs`/`_configure_narrow_argv`.** UCRT-internal names, undocumented, and versioned
  by the runtime rather than by C - so `CommandLineToArgvW` was preferred in this document, and
  then measured: `ucrtbase.dll` exports all three, `clang`'s own startup imports all three, and
  the alternative was a parser of our own to keep in step with the platform's quoting rule. The
  undocumentation is the cost, and it is one function's worth.
- **Writing the `.pdata` as part of this work.** ADR-003 measured its absence; a `crt` does not
  need it, and mixing an unwind-data project into the linking one would hide which failed.

## Consequences

- M5's acceptance test is: `int main(void) { return 42; }` linked with `lld-link` against the
  SDK and run, plus a program that `printf`s and exits 0. Both are checkable in `tests/exe.rs`.
- `argc`/`argv` arrive later than M5, through the runtime's own three imports rather than through
  a `CommandLineToArgvW` of our own; `tests/exe.rs` passes a program its arguments.
- The `crt` is one C file in the repository, compiled by `xcc` during the build, which makes
  every `hello.exe` a regression test of the compiler itself.
- If the SDK libraries are missing, the failure is the linker's and it names the missing `.lib` -
  which is a better error than a hand-rolled import table would give.
