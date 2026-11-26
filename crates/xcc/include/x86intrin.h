/* x86intrin.h - the intrinsics this compiler *does* implement, declared; and the rest absent.
 *
 * `winnt.h` includes this to spell macros whose bodies name intrinsics: `MemoryBarrier` and
 * `MemoryFence` are `_mm_mfence`, `InterlockedCompareExchange` is `_InterlockedCompareExchange`,
 * `ShiftRight128` is `__shiftright128`, and so on. mingw-w64 declares those in `intrin.h` behind
 * `__MINGW_EXTENSION` and a compiler identification this compiler does not claim, so a program that
 * used one was told the name was not declared - at the line it wrote, which was honest but made
 * `windows.h` unusable in practice: every declaration and every use of those macros printed a
 * warning about a function returning `int`.
 *
 * What is declared here is exactly what `crates/xcc-crt/src/main.c` defines, with the signatures
 * transcribed from `psdk_inc/intrin-impl.h` and `intrin.h` of mingw-w64 12.0 - the same library the
 * Windows SDK ships - because a declaration that disagreed with the body would be a wrong call with
 * a signature.
 *
 * Everything else stays undeclared, and deliberately: a program that writes `_mm_clflush` is told
 * the name is not declared rather than silently given `int`. The argument for an empty header is
 * `emmintrin.h`'s, and it still holds for every intrinsic not in this list.
 */
#ifndef XCC_X86INTRIN_H
#define XCC_X86INTRIN_H

/* The three the Windows headers spell `MemoryBarrier`, `MemoryFence` and `YieldProcessor`. Only the
 * first two are implemented: `YieldProcessor` is a scheduling hint, and a hint this compiler cannot
 * spell is better refused than approximated. */

void _mm_mfence(void);

/* `psdk_inc/intrin-impl.h`'s `_Interlocked*` family, 32-bit `long`, which is `__LONG32` there. */
long _InterlockedCompareExchange(long volatile *Destination, long ExChange, long Comperand);
long _InterlockedExchange(long volatile *Target, long Value);
long _InterlockedExchangeAdd(long volatile *Addend, long Value);
long _InterlockedIncrement(long volatile *Addend);
long _InterlockedDecrement(long volatile *Addend);
long _InterlockedAnd(long volatile *Value, long Mask);
long _InterlockedOr(long volatile *Value, long Mask);
long _InterlockedXor(long volatile *Value, long Mask);

/* The 64-bit double-width shifts and multiplies `winnt.h` names `ShiftRight128`, `ShiftLeft128`,
 * `UnsignedMultiply128` and `SignedMultiply128`. */
unsigned long long __shiftright128(unsigned long long LowPart, unsigned long long HighPart,
                                   unsigned char Shift);
unsigned long long __shiftleft128(unsigned long long LowPart, unsigned long long HighPart,
                                  unsigned char Shift);
unsigned long long _umul128(unsigned long long Multiplier, unsigned long long Multiplicand,
                            unsigned long long *HighProduct);
long long _mul128(long long Multiplier, long long Multiplicand, long long *HighProduct);

/* The `stos` family, which the Windows headers use to fill and copy blocks. */
void __stosb(unsigned char *Destination, unsigned char Value, unsigned long long Count);
void __stosw(unsigned short *Destination, unsigned short Value, unsigned long long Count);
void __stosd(unsigned long *Destination, unsigned long Value, unsigned long long Count);
void __stosq(unsigned long long *Destination, unsigned long long Value, unsigned long long Count);

#endif
