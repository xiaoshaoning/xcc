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
