/* intrin.h - the MSVC intrinsic header, which this compiler answers with its own list.
 *
 * mingw-w64 declares the Microsoft intrinsics here, behind `__MINGW_EXTENSION` and a compiler
 * identification this compiler does not claim - so a program that does `#include <intrin.h>` got no
 * declarations at all, and every intrinsic it used was an implicit `int`. `xxhash` was the case that
 * mattered: it multiplies 64x64 into 128 with `_umul128`, the implicit `int` truncated the result,
 * and XXH3 gave a *different number* from clang's build - quietly, because an implicit declaration
 * of a function that returns `unsigned __int64` is not an error without a warning that stops.
 *
 * The list is `x86intrin.h`'s, which is the fifteen the runtime implements, transcribed from
 * mingw's own `psdk_inc/intrin-impl.h`. This header exists so that `<intrin.h>` - the spelling a
 * program actually writes - reaches it.
 */
#ifndef XCC_INTRIN_H
#define XCC_INTRIN_H

#include <x86intrin.h>

#endif
