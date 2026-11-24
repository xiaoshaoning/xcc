# tinyexpr, vendored

`tinyexpr.c` (758 lines) and `tinyexpr.h` (91) from <https://github.com/codeplea/tinyexpr> at
commit `ba2e32bbcc2d1e644a7e132d24c2f542821cabe0`, unmodified, with the project's own `LICENSE` (the
zlib licence, "Copyright (C) 2015-2026 Lewis Van Winkle"). Nothing in them was edited to make this compiler accept them: it did
not have to be, once three bugs were fixed.

`driver.c` is ours: it asks the library to interpret a list of expressions and prints one line per
expression, then compiles `x*y+1` with two variables bound to it and evaluates it twice, the second
time after changing one of them.

## What the expected text is

What `clang` printed for the same two files:

```
clang -O0 -w -I. driver.c tinyexpr.c -o driver -lm && ./driver
```

The text was recorded once, when the fixture was added. The two `nan` lines are expressions the
library itself refuses - `min`, `max` and `if` are not in its builtin table in this version - and
`te_interp` returns `NAN` for them, which is why the *sign* of that constant shows in the output.

## What it found

Three bugs, none of which any of the larger libraries could reach:

- `te_expr` holds an **anonymous union** (C11 6.7.2.1p13). This compiler refused the type
  outright: `x->value` was "not a member". The layout had been right all along - only the view was
  missing, and the fix is one splice of the member list when the record completes;
- `power()` keeps a **`static` table of function pointers** inside a function, and its initialiser
  was evaluated as code: inside a body the address of a function became an instruction, which left
  the data with no value to hold, so the table was all zeros and calling through it crashed. An
  initialiser that writes bytes is not an evaluated context, and now says so;
- `te_variable variables[] = { { "x", &x }, { "y", &y } };` is an **array whose size comes from its
  list**, and its elements' unnamed members held the frame's leftovers - `type` read as
  `-43300501` where `clang` had `0`. Zeroing the whole object needs a count the list has not given
  yet, so the array walk zeroes each element as it reaches it instead.

And one header bug: `NAN` was not declared at all, and the obvious spelling `0.0/0.0` is the
target's *indefinite* NaN, whose sign bit is set - it printed `-nan(ind)` where `clang` printed
`nan`. `NAN` and `INFINITY` are `__builtin_nanf("")` and `__builtin_inff()` now, in `math.h`.
