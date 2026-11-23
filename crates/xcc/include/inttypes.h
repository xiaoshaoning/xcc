/* inttypes.h - C99 7.8: the integer types wide enough for anything, and how to print them.
 *
 * Clean room: the wording is C99 7.8 and nothing else.
 *
 * The point of this header is not the four functions: it is the sixty-odd macros that say which
 * `printf` conversion to use for a type whose width the program is not supposed to know. A program
 * that writes `%d` for an `intmax_t` is wrong on this target, and `PRIdMAX` is how it is right.
 */
#ifndef XCC_INTTYPES_H
#define XCC_INTTYPES_H

#include <stdint.h>

/* 7.8p1: the type `imaxdiv` returns. Two members, both `intmax_t`. */
typedef struct imaxdiv_t {
    intmax_t quot;
    intmax_t rem;
} imaxdiv_t;

intmax_t imaxabs(intmax_t value);
imaxdiv_t imaxdiv(intmax_t numerator, intmax_t denominator);
intmax_t strtoimax(const char *restrict text, char **restrict end, int base);
uintmax_t strtoumax(const char *restrict text, char **restrict end, int base);

/* 7.8.1: `PRIdN` says which conversion prints an `intN_t`. The target's `long long` is 64 bits and
 * its `intmax_t` is the same, so the widest spellings are `ll` - and a `PRI` for a narrower type
 * names that type's own width, never a shorter one, so the printed value is never cut. */
#define PRId8 "d"
#define PRId16 "d"
#define PRId32 "d"
#define PRId64 "lld"
#define PRIdMAX "lld"
#define PRIdPTR "lld"
#define PRIi8 "i"
#define PRIi16 "i"
#define PRIi32 "i"
#define PRIi64 "lli"
#define PRIiMAX "lli"
#define PRIiPTR "lli"
#define PRIo8 "o"
#define PRIo16 "o"
#define PRIo32 "o"
#define PRIo64 "llo"
#define PRIoMAX "llo"
#define PRIoPTR "llo"
#define PRIu8 "u"
#define PRIu16 "u"
#define PRIu32 "u"
#define PRIu64 "llu"
#define PRIuMAX "llu"
#define PRIuPTR "llu"
#define PRIx8 "x"
#define PRIx16 "x"
#define PRIx32 "x"
#define PRIx64 "llx"
#define PRIxMAX "llx"
#define PRIxPTR "llx"
#define PRIX8 "X"
#define PRIX16 "X"
#define PRIX32 "X"
#define PRIX64 "llX"
#define PRIXMAX "llX"
#define PRIXPTR "llX"

/* 7.8.1: the same for `scanf`, where the length modifier is what makes a 64-bit read. */
#define SCNd8 "hhd"
#define SCNd16 "hd"
#define SCNd32 "d"
#define SCNd64 "lld"
#define SCNdMAX "lld"
#define SCNdPTR "lld"
#define SCNi8 "hhi"
#define SCNi16 "hi"
#define SCNi32 "i"
#define SCNi64 "lli"
#define SCNiMAX "lli"
#define SCNiPTR "lli"
#define SCNo8 "hho"
#define SCNo16 "ho"
#define SCNo32 "o"
#define SCNo64 "llo"
#define SCNoMAX "llo"
#define SCNoPTR "llo"
#define SCNu8 "hhu"
#define SCNu16 "hu"
#define SCNu32 "u"
#define SCNu64 "llu"
#define SCNuMAX "llu"
#define SCNuPTR "llu"
#define SCNx8 "hhx"
#define SCNx16 "hx"
#define SCNx32 "x"
#define SCNx64 "llx"
#define SCNxMAX "llx"
#define SCNxPTR "llx"

#endif
