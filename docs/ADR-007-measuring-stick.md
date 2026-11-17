# ADR-007: the measuring stick is a ported 1991 compiler

Status: accepted, 2026-11-16. Written after M9 was attempted, because the first attempt found that
the library the plan assumed was not there.

## Context

PLAN.md's M9 compares xcc against Borland C++ 2.0 - the compiler that built early-1990s DOS games -
through borland-reborn's port of `BCC.EXE`: 1629 translated functions, each checked call by call
against calls recorded from Borland's binary, and 1874 of its 1880 corpus compile steps identical to
the original. Compiling the same source with it and with xcc is a second oracle: `clang` says what
C means today, and Borland says what a 1991 implementation did with the same source. It is a
measuring stick, not a product: nothing of it ships in xcc's output.

**The first attempt failed, and the reason is worth recording.** The checkout's `build/` held
**Linux** objects (ELF, from `/usr/include/x86_64-linux-gnu`); `make` saw them newer than the
sources and linked them into a "DLL" that was 3.5 MB of zeroes, and `ld` answered `returned 5 exit
status` with no message. A `make clean` and a rebuild on the host's own toolchain fixed it: the
port compiles and links here in about a minute, producing a 13 MB `libbcc20.dll`. The lesson is
the ordinary one - a build tree from another machine is not a build tree - but it cost the first
attempt, and the next person should not pay it again.

**Borland's headers are not in the repository.** They come with Borland C++ 2.0 (`../docs/SETUP.md`
in that project), so any source that `#include`s something cannot be compiled by both compilers
here. That is why M9's corpus is six **freestanding** files: it is the only kind of C both can
build without a product neither of us has.

## Decision

**One crate, `bcc20-sys`, with hand-written FFI over `bcc20.h`, and the library is loaded, not
linked.**

- **Loaded at run time** (`BCC20_LIB`, else `libbcc20.dll` on `PATH`, through `LoadLibraryA` and
  `GetProcAddress`). Three reasons, any one enough: a machine without the port still builds the
  workspace; a missing library is `None` rather than a link error, so the comparison *says* what is
  missing instead of failing to build; and a mingw-built DLL cannot be linked by an MSVC-targeted
  Rust crate without an import library `cc` did not write.
- **Hand-written, not bindgen**: nine fields and two functions is not a code-generation problem, and
  a generated binding would have dragged a build dependency in for it.
- **`--target=dos16 -c` is the CLI's half**: the same command line, another compiler, and an object
  file that is Borland's own OMF down to the checksum byte. `-D`, `-U` and `-I` pass through (BCC
  spells them the same); `-E`, `--run` and `--explain` do not, because they are xcc's own.
- **Two object readers, one question**: `bcc20_sys::object` reads the code size out of each format -
  COFF's `.text` section and OMF's code segment - because the comparison needs one number from each
  and the formats put it in different places.

## Consequences

- The table prints, and it says something worth knowing: on the six-file corpus xcc emits **4857
  bytes of 64-bit code** where Borland emits **1233 bytes of 16-bit code**. Some of that is the
  target (64-bit code is bigger) and some is that Borland has register allocation and peephole
  optimisation and xcc has neither. That ratio is the number M10 exists to move.
- The library is **not reentrant** (`bcc20.h` says so: "one compile at a time per process"), so the
  tests take a lock. A host must serialise too.
- `libbcc20` is licensed from the borland-reborn project and its translation is derived from
  Borland's binary; per CONSTRAINTS 7 it is **run as an oracle and never linked into an artefact**,
  which is what loading it at run time and behind a flag makes true.
- The corpus is six files because Borland's headers are absent. With them, the same harness would
  take Borland's own test corpus (`../bcc20-port/tests`), which is the natural next thing to try.
