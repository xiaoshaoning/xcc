# lz4, vendored

`lib/lz4.c`, `lib/lz4hc.c`, `lib/lz4frame.c`, `lib/lz4file.c` and `lib/xxhash.c` from the local
checkout of <https://github.com/lz4/lz4> at commit `d04dab31`, unmodified, with their headers and
the library's own `LICENSE` (BSD 2-Clause, "Copyright (c) 2011-2020, Yann Collet"). Nothing in them
was edited to make this compiler accept them, and nothing may be: a fixture that has been adjusted
is a fixture that tests the adjustment.

`driver.c` is ours. It exercises the library's public surface - the block API at four
accelerations, `LZ4_compress_destSize`, a caller-supplied state, partial decompression, both
compressors' `extState` forms, the streaming block API with `LZ4_saveDict`, compression against a
loaded dictionary, the high-compression stream, the frame API written and read back in odd-sized
chunks, a frame built from a prepared `CDict`, and the stdio wrapper over `tmpfile()` - and prints
one line per result. The last line is an FNV-1a digest of every byte of every compressed output, so
two builds that print the same number emitted the same streams.

## What the expected text is

The expected output in `tests/real.rs` is what `clang` printed for the same files:

```
clang -O0 -w -I. driver.c lz4.c lz4hc.c lz4frame.c lz4file.c xxhash.c -o driver && ./driver
```

LZ4 is deterministic, so an identical digest is a strong claim: the same compressed bytes, from two
independent compilations. The numbers were recorded once, when the fixture was added; the test
compares this compiler's run with them, not with another run of `clang`.

## Why these five files and not the whole repository

`programs/` (`lz4cli.c`, `lz4io.c`, `bench.c`) needs `windows.h`, `io.h`, `sys/stat.h` and
`winioctl.h` - the Win32 SDK, which is the boundary this compiler stops at and not something worth
vendoring. `tests/fuzz/fuzzer.c` needs only `<clock.h>`'s `clock()`, but reaches `programs/util.h`
and `platform.h` through its include list, and those want the same SDK headers.

## What it found

Four bugs, none of which the small-program corpus could see:

- an array initialiser whose size comes from its list could not end with a comma, which is how
  lz4frame.c writes its error-string table (C99 6.7.8p1);
- `&f(x)->m` was parsed as `(&f)(x)->m`, because the shortcut for "the address of a function
  designator is the designator" fired on the name alone;
- a frame larger than one stack page was taken with a single `sub rsp`, which steps over the guard
  page Windows grows the stack through: `char a[65536]` was an access violation. This one crashed
  the streaming decoder here, which keeps a 64 KiB dictionary on the stack.
