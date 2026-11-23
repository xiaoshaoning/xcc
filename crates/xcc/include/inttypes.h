/* inttypes.h - C99 7.8: the integer types wide enough for anything, and how to print them.
 *
 * Clean room: the wording is C99 7.8 and nothing else.
 *
 * The point of this header is not the four functions: it is the macro for every integer type the
 * standard names, so that a program never has to know a width. `%d` for an `intmax_t` prints
 * nonsense on this target; `PRIdMAX` is how it is written instead.
 *
 * **The widths come from `stdint.h` and the two headers must agree.** This target follows MSVC:
 * every `int_leastN_t` and every `int_fastN_t` is exactly N bits wide (there is no wider "fast"
 * type), so all three families of each kind share one set of conversions, and a width that
 * disagreed with `stdint.h` would print the wrong number rather than fail to compile.
 *
 * That makes the conversions *narrower* than glibc's, which name the promoted width (`PRIdLEAST8` is
 * `"d"` there and `"hhd"` here). Both read a value that fits; they differ when a negative value is
 * printed unsigned - `PRIXLEAST8` of `-1` is `FF` here and `FFFFFFFF` there - and the narrow one is
 * the conversion for *the type*, which is what 7.8.1 asks the macro to name.
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

/* 7.8.1: `PRIdN` is the conversion that prints an `intN_t`, and the three families differ only in
 * which type they name - which, on this target, is the same width three times over. */


#define PRId8 "hhd"
#define PRIdLEAST8 PRId8
#define PRIdFAST8 PRId8
#define PRIi8 "hhi"
#define PRIiLEAST8 PRIi8
#define PRIiFAST8 PRIi8
#define PRIo8 "hho"
#define PRIoLEAST8 PRIo8
#define PRIoFAST8 PRIo8
#define PRIu8 "hhu"
#define PRIuLEAST8 PRIu8
#define PRIuFAST8 PRIu8
#define PRIx8 "hhx"
#define PRIxLEAST8 PRIx8
#define PRIxFAST8 PRIx8
#define PRIX8 "hhX"
#define PRIXLEAST8 PRIX8
#define PRIXFAST8 PRIX8
#define PRId16 "hd"
#define PRIdLEAST16 PRId16
#define PRIdFAST16 PRId16
#define PRIi16 "hi"
#define PRIiLEAST16 PRIi16
#define PRIiFAST16 PRIi16
#define PRIo16 "ho"
#define PRIoLEAST16 PRIo16
#define PRIoFAST16 PRIo16
#define PRIu16 "hu"
#define PRIuLEAST16 PRIu16
#define PRIuFAST16 PRIu16
#define PRIx16 "hx"
#define PRIxLEAST16 PRIx16
#define PRIxFAST16 PRIx16
#define PRIX16 "hX"
#define PRIXLEAST16 PRIX16
#define PRIXFAST16 PRIX16
#define PRId32 "d"
#define PRIdLEAST32 PRId32
#define PRIdFAST32 PRId32
#define PRIi32 "i"
#define PRIiLEAST32 PRIi32
#define PRIiFAST32 PRIi32
#define PRIo32 "o"
#define PRIoLEAST32 PRIo32
#define PRIoFAST32 PRIo32
#define PRIu32 "u"
#define PRIuLEAST32 PRIu32
#define PRIuFAST32 PRIu32
#define PRIx32 "x"
#define PRIxLEAST32 PRIx32
#define PRIxFAST32 PRIx32
#define PRIX32 "X"
#define PRIXLEAST32 PRIX32
#define PRIXFAST32 PRIX32
#define PRId64 "lld"
#define PRIdLEAST64 PRId64
#define PRIdFAST64 PRId64
#define PRIi64 "lli"
#define PRIiLEAST64 PRIi64
#define PRIiFAST64 PRIi64
#define PRIo64 "llo"
#define PRIoLEAST64 PRIo64
#define PRIoFAST64 PRIo64
#define PRIu64 "llu"
#define PRIuLEAST64 PRIu64
#define PRIuFAST64 PRIu64
#define PRIx64 "llx"
#define PRIxLEAST64 PRIx64
#define PRIxFAST64 PRIx64
#define PRIX64 "llX"
#define PRIXLEAST64 PRIX64
#define PRIXFAST64 PRIX64
#define PRIdMAX "lld"
#define PRIiMAX "lli"
#define PRIoMAX "llo"
#define PRIuMAX "llu"
#define PRIxMAX "llx"
#define PRIXMAX "llX"
#define PRIdPTR "lld"
#define PRIiPTR "lli"
#define PRIoPTR "llo"
#define PRIuPTR "llu"
#define PRIxPTR "llx"
#define PRIXPTR "llX"
#define PRIdLEASTMAX PRIdMAX
#define PRIdFASTMAX PRIdMAX
#define PRIiLEASTMAX PRIiMAX
#define PRIiFASTMAX PRIiMAX
#define PRIoLEASTMAX PRIoMAX
#define PRIoFASTMAX PRIoMAX
#define PRIuLEASTMAX PRIuMAX
#define PRIuFASTMAX PRIuMAX
#define PRIxLEASTMAX PRIxMAX
#define PRIxFASTMAX PRIxMAX
#define PRIXLEASTMAX PRIXMAX
#define PRIXFASTMAX PRIXMAX
#define PRIdLEASTPTR PRIdPTR
#define PRIdFASTPTR PRIdPTR
#define PRIiLEASTPTR PRIiPTR
#define PRIiFASTPTR PRIiPTR
#define PRIoLEASTPTR PRIoPTR
#define PRIoFASTPTR PRIoPTR
#define PRIuLEASTPTR PRIuPTR
#define PRIuFASTPTR PRIuPTR
#define PRIxLEASTPTR PRIxPTR
#define PRIxFASTPTR PRIxPTR
#define PRIXLEASTPTR PRIXPTR
#define PRIXFASTPTR PRIXPTR

/* SCN*: what reads one. */
#define SCNd8 "hhd"
#define SCNdLEAST8 SCNd8
#define SCNdFAST8 SCNd8
#define SCNi8 "hhi"
#define SCNiLEAST8 SCNi8
#define SCNiFAST8 SCNi8
#define SCNo8 "hho"
#define SCNoLEAST8 SCNo8
#define SCNoFAST8 SCNo8
#define SCNu8 "hhu"
#define SCNuLEAST8 SCNu8
#define SCNuFAST8 SCNu8
#define SCNx8 "hhx"
#define SCNxLEAST8 SCNx8
#define SCNxFAST8 SCNx8
#define SCNX8 "hhX"
#define SCNXLEAST8 SCNX8
#define SCNXFAST8 SCNX8
#define SCNd16 "hd"
#define SCNdLEAST16 SCNd16
#define SCNdFAST16 SCNd16
#define SCNi16 "hi"
#define SCNiLEAST16 SCNi16
#define SCNiFAST16 SCNi16
#define SCNo16 "ho"
#define SCNoLEAST16 SCNo16
#define SCNoFAST16 SCNo16
#define SCNu16 "hu"
#define SCNuLEAST16 SCNu16
#define SCNuFAST16 SCNu16
#define SCNx16 "hx"
#define SCNxLEAST16 SCNx16
#define SCNxFAST16 SCNx16
#define SCNX16 "hX"
#define SCNXLEAST16 SCNX16
#define SCNXFAST16 SCNX16
#define SCNd32 "d"
#define SCNdLEAST32 SCNd32
#define SCNdFAST32 SCNd32
#define SCNi32 "i"
#define SCNiLEAST32 SCNi32
#define SCNiFAST32 SCNi32
#define SCNo32 "o"
#define SCNoLEAST32 SCNo32
#define SCNoFAST32 SCNo32
#define SCNu32 "u"
#define SCNuLEAST32 SCNu32
#define SCNuFAST32 SCNu32
#define SCNx32 "x"
#define SCNxLEAST32 SCNx32
#define SCNxFAST32 SCNx32
#define SCNX32 "X"
#define SCNXLEAST32 SCNX32
#define SCNXFAST32 SCNX32
#define SCNd64 "lld"
#define SCNdLEAST64 SCNd64
#define SCNdFAST64 SCNd64
#define SCNi64 "lli"
#define SCNiLEAST64 SCNi64
#define SCNiFAST64 SCNi64
#define SCNo64 "llo"
#define SCNoLEAST64 SCNo64
#define SCNoFAST64 SCNo64
#define SCNu64 "llu"
#define SCNuLEAST64 SCNu64
#define SCNuFAST64 SCNu64
#define SCNx64 "llx"
#define SCNxLEAST64 SCNx64
#define SCNxFAST64 SCNx64
#define SCNX64 "llX"
#define SCNXLEAST64 SCNX64
#define SCNXFAST64 SCNX64
#define SCNdMAX "lld"
#define SCNiMAX "lli"
#define SCNoMAX "llo"
#define SCNuMAX "llu"
#define SCNxMAX "llx"
#define SCNXMAX "llX"
#define SCNdPTR "lld"
#define SCNiPTR "lli"
#define SCNoPTR "llo"
#define SCNuPTR "llu"
#define SCNxPTR "llx"
#define SCNXPTR "llX"
#define SCNdLEASTMAX SCNdMAX
#define SCNdFASTMAX SCNdMAX
#define SCNiLEASTMAX SCNiMAX
#define SCNiFASTMAX SCNiMAX
#define SCNoLEASTMAX SCNoMAX
#define SCNoFASTMAX SCNoMAX
#define SCNuLEASTMAX SCNuMAX
#define SCNuFASTMAX SCNuMAX
#define SCNxLEASTMAX SCNxMAX
#define SCNxFASTMAX SCNxMAX
#define SCNXLEASTMAX SCNXMAX
#define SCNXFASTMAX SCNXMAX
#define SCNdLEASTPTR SCNdPTR
#define SCNdFASTPTR SCNdPTR
#define SCNiLEASTPTR SCNiPTR
#define SCNiFASTPTR SCNiPTR
#define SCNoLEASTPTR SCNoPTR
#define SCNoFASTPTR SCNoPTR
#define SCNuLEASTPTR SCNuPTR
#define SCNuFASTPTR SCNuPTR
#define SCNxLEASTPTR SCNxPTR
#define SCNxFASTPTR SCNxPTR
#define SCNXLEASTPTR SCNXPTR
#define SCNXFASTPTR SCNXPTR

#endif
