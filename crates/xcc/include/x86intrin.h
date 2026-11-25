/* x86intrin.h - empty, and here on purpose.
 *
 * `winnt.h` includes this to spell two macros, `CacheLineFlush` and `YieldProcessor`, whose bodies
 * name intrinsics (`_mm_clflush`, `_mm_pause`). This compiler implements none, and declares none:
 * a program that writes one is told the name is not declared, at the line it wrote. What the empty
 * header buys is `#include <windows.h>` working for the program that never writes one - which is
 * nearly every program. The whole argument is in `emmintrin.h`.
 */
#ifndef XCC_X86INTRIN_H
#define XCC_X86INTRIN_H
#endif
