# Vendored real code

Two libraries written by other people, checked in so that the claims the README makes about real
code are made by a test rather than by a paragraph. Each is unmodified: the licence notice at the
top of every file is the upstream one, and nothing here is patched to suit this compiler. A file
that had to be patched would be a compiler bug, not a fixture.

They are here because they found bugs a corpus of small programs could not: cJSON found three
front-end bugs and a linker bug, and miniz found three front-end bugs, a pass bug and a hang.

| fixture | upstream | licence | what it exercises |
|---|---|---|---|
| `cjson/` | [DaveGamble/cJSON](https://github.com/DaveGamble/cJSON), master, 2026-11-22 | MIT | the parser, the tree walk, and printing |
| `miniz/` | [richgel999/miniz](https://github.com/richgel999/miniz), master, 2026-11-22 | MIT / public domain | `tdefl`, `tinfl`, the zip writer and the zip reader |
| `sqlite/` | [SQLite](https://sqlite.org) 3.51.3, the amalgamation, 2026-11-22 | public domain | everything: 265 977 lines in one translation unit |
| `lua/` | [Lua](https://lua.org) 5.5.1, `src/` of the release tarball | MIT | the interpreter, the compiler, the GC, patterns, coroutines, `string.pack` |

## cjson

`cJSON.c`, `cJSON.h` from [DaveGamble/cJSON](https://github.com/DaveGamble/cJSON), master as of
2026-11-22. Copyright (c) 2009-2017 Dave Gamble and cJSON contributors, MIT — the full text is at the
top of `cJSON.c` and `cJSON.h`.

## miniz

`miniz.c`, `miniz_tdef.c`, `miniz_tinfl.c`, `miniz_zip.c` and their headers from
[richgel999/miniz](https://github.com/richgel999/miniz), master as of 2026-11-22. MIT and public
domain; the licences are in the files themselves.

**`miniz_export.h` is ours, not upstream's.** Upstream generates it from a CMake template, and the
two macros it defines are the whole of it. It is here rather than generated because a fixture that
needs CMake is a fixture that needs CMake.

## lua

`src/*.c` and `src/*.h` from [Lua](https://lua.org) 5.5.1, the release tarball, unmodified. MIT; the
notice is in `lua.h`'s header comment and in `README.upstream`, which is the tarball's own README.

It is here because an interpreter is a different program from a database: one long-lived loop
dispatching bytecode, a garbage collector that traces its own stack, a pattern matcher written in C,
coroutines as separate machine stacks, and `string.pack`'s format compiler. Lua 5.5 also has the
newest things in the language - integer division, bitwise operators, `goto`, integer subtypes.

`driver.lua` is ours: the script a test runs the interpreter on, whose output is compared with what a
`clang -O2` build of the same sources printed. It touches the whole language, and one of the lines it
prints is `string.dump`'s bytecode read back by `load`, so the dumper is covered too.

Two of its 34 units read `windows.h` - `loadlib.c` loads DLLs, `lua.c` asks whether stdin is a
terminal - and `luaconf.h` defines `LUA_USE_WINDOWS` for `_WIN32` whatever a program wants, so the
compile-only test compiles the other 32 - `luac.c` among them, the bytecode compiler the release
ships - on every machine, while the run test needs the SDK and says so when it is not there.

## sqlite

`sqlite3.c` and `sqlite3.h`, the amalgamation from [SQLite](https://sqlite.org) 3.51.3, 2026-11-22.
Public domain: the library's own statement is in `LICENSE.md`, and nothing here is patched.

It is here because it is the largest single file of real C there is - 9.6 MB, 265 977 lines, one
translation unit - and because a handful of things live in it that no small program has: function
tables of forty members with pointer initialisers, `SQLITE_INT_TO_PTR`, `do { } while(1)`, and a
query planner with a thousand-line function.

It is checked **twice**, because it needs the host's Windows headers to *run* and this compiler's
whole point is not to:

- `sqlite3.c` with `-DSQLITE_OS_OTHER=1` compiles and writes a 2.6 MB object on any machine. That
  test never skips.
- With its Windows layer, the driver opens an in-memory database, creates a table, inserts three
  rows with one `INSERT`, and reads them back through a prepared statement: `6 one,two,three
  3.51.3` and `0 -1`. That one needs `windows.h`, so it says it is skipping when the host has no
  SDK - which is what CI is.

`driver.c` in each directory is ours too: the program a test compiles against the library, whose
output is compared with what a `clang` build of the same files printed.
