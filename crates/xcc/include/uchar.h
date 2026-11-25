/* uchar.h - C11 7.28.
 *
 * The two character types the standard adds and the four functions that convert between multibyte
 * characters and them. The *literals* are the compiler's: `u"..."` and `u'...'` are UTF-16 code
 * units, `U"..."` and `U'...'` are code points, `u8"..."` and `u8'...'` are UTF-8 bytes - which is
 * the encoding this compiler reads its source in, so `u8` and no prefix are the same bytes with
 * different types.
 *
 * The four functions are the universal C runtime's own, exported under these names - measured
 * against `ucrtbase.dll`'s export table before writing a line of this header - so they are
 * declarations and nothing else. The conversions are the *locale's*: with the "C" locale a program
 * has, `mbrtoc16` accepts ASCII and refuses anything above it, which is what the standard says and
 * what `clang`'s build does here too.
 */
#ifndef __XCC_UCHAR_H
#define __XCC_UCHAR_H

#include <stddef.h>
#include <stdint.h>
#include <wchar.h>

/* 7.28p2: two unsigned integer types, `uint_least16_t` and `uint_least32_t` by the standard's own
 * wording - which on this target are 16 and 32 bits exactly. */
typedef uint_least16_t char16_t;
typedef uint_least32_t char32_t;

/* 7.28.1: from a multibyte character to `char16_t`, and back. A `\0` byte in the input is a
 * "null wide character", not an end of string: `s` may be null, and the count is what decides. */
size_t mbrtoc16(char16_t *restrict out, const char *restrict s, size_t count,
                mbstate_t *restrict state);
size_t c16rtomb(char *restrict s, char16_t character, mbstate_t *restrict state);

/* 7.28.2: the same two for `char32_t`. On a target whose `char32_t` holds every code point these
 * are the plain conversions, and a surrogate pair is not a character. */
size_t mbrtoc32(char32_t *restrict out, const char *restrict s, size_t count,
                mbstate_t *restrict state);
size_t c32rtomb(char *restrict s, char32_t character, mbstate_t *restrict state);

#endif
