/* limits.h - C99 5.2.4.2.1, for a 8/16/32/64-bit target whose `char` is signed. */
#ifndef XCC_LIMITS_H
#define XCC_LIMITS_H

#define CHAR_BIT 8
#define SCHAR_MIN (-128)
#define SCHAR_MAX 127
#define UCHAR_MAX 255
#define CHAR_MIN SCHAR_MIN
#define CHAR_MAX SCHAR_MAX
#define MB_LEN_MAX 1
#define SHRT_MIN (-32767 - 1)
#define SHRT_MAX 32767
#define USHRT_MAX 65535
#define INT_MIN (-2147483647 - 1)
#define INT_MAX 2147483647
#define UINT_MAX 4294967295U
/* `long` is 32 bits on this ABI (LLP64), which is what makes the C library's own `long`-shaped
 * functions agree with it: see `fseek` and `clock_t`. */
#define LONG_MIN (-2147483647 - 1)
#define LONG_MAX 2147483647
#define ULONG_MAX 4294967295U
#define LLONG_MIN (-9223372036854775807LL - 1)
#define LLONG_MAX 9223372036854775807LL
#define ULLONG_MAX 18446744073709551615ULL

#endif
