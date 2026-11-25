/* emmintrin.h - empty, and here on purpose.
 *
 * `winnt.h` includes this and `x86intrin.h` to spell two macros: `CacheLineFlush` is
 * `_mm_clflush(...)` and `YieldProcessor` is `_mm_pause()`. Neither is a declaration the SDK needs
 * at file scope - they are *bodies of macros* a program may never write - so refusing the whole of
 * `windows.h` because this compiler implements no SSE intrinsics would refuse a program that never
 * asks for one.
 *
 * What this header does not do is pretend. It declares nothing, so a program that writes
 * `_mm_clflush(p)` is told the name is not declared, at the line it wrote - which is the truth: this
 * compiler has no intrinsics, and the Win64 ABI needs none of them for the C it compiles.
 *
 * A program that genuinely needs SSE is a program for `clang`, and it will say so when it is
 * compiled.
 */
#ifndef XCC_EMMINTRIN_H
#define XCC_EMMINTRIN_H
#endif
