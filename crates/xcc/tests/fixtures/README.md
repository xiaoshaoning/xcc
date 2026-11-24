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

`driver.c` in each directory is ours too: the program a test compiles against the library, whose
output is compared with what a `clang` build of the same files printed.
