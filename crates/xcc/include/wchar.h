/* wchar.h - C99 7.24: wide characters, and everything that reads and writes them.  Part of xcc.
 *
 * Clean room: the wording is C99 7.24 and nothing else.  The *widths* and the `mbstate_t` layout
 * are the target's: `wchar_t` and `wint_t` are 16 bits, and a conversion state is eight bytes.
 *
 * Twelve of the functions here are the wide `stdio` front ends, which the UCRT keeps in the
 * program rather than the library - the same division as the narrow ones - so the runtime this
 * compiler compiles into every link carries them: `crates/xcc-crt/src/main.c`.  Four more
 * (`wmemcpy`, `wmemmove`, `wmemcmp`, `wmemchr`), `mbsinit` and `fwide` are there for the same
 * reason: no Windows runtime exports them under those names.
 *
 * `fwide` is the platform's stub, and the header says so where it is declared: this C runtime has
 * one orientation per stream, the byte one, so asking is 0 and setting is the value asked for.
 */
#ifndef XCC_WCHAR_H
#define XCC_WCHAR_H

#include <stdarg.h>
#include <stddef.h>
#include <stdio.h>
#include <time.h>
#include <wctype.h>

/* 7.24.1: the conversion state. A multi-byte character may not fit the bytes read so far, and this
 * carries what is left between calls. The layout is the UCRT's own: the library writes it. */
typedef struct _xcc_mbstate {
    unsigned long _Wchar;
    unsigned short _Byte;
    unsigned short _State;
} mbstate_t;

wchar_t *wmemset(wchar_t *to, wchar_t character, size_t count);

/* 7.24.2.1: formatted wide output and input, and the `va_list` forms of each. */
int fwprintf(FILE *restrict stream, const wchar_t *restrict format, ...);
int swprintf(wchar_t *restrict to, size_t count, const wchar_t *restrict format, ...);
int vfwprintf(FILE *restrict stream, const wchar_t *restrict format, va_list args);
int vswprintf(wchar_t *restrict to, size_t count, const wchar_t *restrict format, va_list args);
int vwprintf(const wchar_t *restrict format, va_list args);
int wprintf(const wchar_t *restrict format, ...);
int fwscanf(FILE *restrict stream, const wchar_t *restrict format, ...);
int swscanf(const wchar_t *restrict from, const wchar_t *restrict format, ...);
int vfwscanf(FILE *restrict stream, const wchar_t *restrict format, va_list args);
int vswscanf(const wchar_t *restrict from, const wchar_t *restrict format, va_list args);
int vwscanf(const wchar_t *restrict format, va_list args);
int wscanf(const wchar_t *restrict format, ...);

/* 7.24.2.2 and 7.24.2.3: a stream's orientation, and wide characters through it. */
int fwide(FILE *stream, int mode);
wint_t fgetwc(FILE *stream);
wchar_t *fgetws(wchar_t *restrict to, int count, FILE *restrict stream);
wint_t fputwc(wchar_t character, FILE *stream);
int fputws(const wchar_t *restrict text, FILE *restrict stream);
wint_t getwc(FILE *stream);
wint_t getwchar(void);
wint_t putwc(wchar_t character, FILE *stream);
wint_t putwchar(wchar_t character);
wint_t ungetwc(wint_t character, FILE *stream);

/* 7.24.2.4: two thousand and ninety-nine's time formatting for a wide string. */
size_t wcsftime(wchar_t *restrict to, size_t count, const wchar_t *restrict format,
                const struct tm *restrict when);

/* 7.24.2.5: one wide character to a multi-byte character and back. */
wint_t btowc(int character);
int wctob(wint_t character);

/* 7.24.2.6: multi-byte characters, one at a time and in runs, through a conversion state. */
int mbsinit(const mbstate_t *state);
size_t mbrlen(const char *restrict text, size_t count, mbstate_t *restrict state);
size_t mbrtowc(wchar_t *restrict to, const char *restrict text, size_t count,
               mbstate_t *restrict state);
size_t wcrtomb(char *restrict to, wchar_t character, mbstate_t *restrict state);
size_t mbsrtowcs(wchar_t *restrict to, const char **restrict from, size_t count,
                 mbstate_t *restrict state);
size_t wcsrtombs(char *restrict to, const wchar_t **restrict from, size_t count,
                 mbstate_t *restrict state);

/* 7.24.2.7: wide strings to numbers. */
double wcstod(const wchar_t *restrict text, wchar_t **restrict end);
float wcstof(const wchar_t *restrict text, wchar_t **restrict end);
long double wcstold(const wchar_t *restrict text, wchar_t **restrict end);
long wcstol(const wchar_t *restrict text, wchar_t **restrict end, int base);
long long wcstoll(const wchar_t *restrict text, wchar_t **restrict end, int base);
unsigned long wcstoul(const wchar_t *restrict text, wchar_t **restrict end, int base);
unsigned long long wcstoull(const wchar_t *restrict text, wchar_t **restrict end, int base);

/* 7.24.4.1: copies, including the ones that work on any wide array rather than a string. */
wchar_t *wcscpy(wchar_t *restrict to, const wchar_t *restrict from);
wchar_t *wcsncpy(wchar_t *restrict to, const wchar_t *restrict from, size_t count);
wchar_t *wmemcpy(wchar_t *restrict to, const wchar_t *restrict from, size_t count);
wchar_t *wmemmove(wchar_t *to, const wchar_t *from, size_t count);

/* 7.24.4.2: concatenation. */
wchar_t *wcscat(wchar_t *restrict to, const wchar_t *restrict from);
wchar_t *wcsncat(wchar_t *restrict to, const wchar_t *restrict from, size_t count);

/* 7.24.4.3: comparison. */
int wcscmp(const wchar_t *left, const wchar_t *right);
int wcscoll(const wchar_t *left, const wchar_t *right);
int wcsncmp(const wchar_t *left, const wchar_t *right, size_t count);

/* The case-insensitive pair, in the SDK's spelling, for the same reason as `_stricmp`. */
int _wcsicmp(const wchar_t *left, const wchar_t *right);
int _wcsnicmp(const wchar_t *left, const wchar_t *right, size_t count);
size_t wcsxfrm(wchar_t *restrict to, const wchar_t *restrict from, size_t count);
int wmemcmp(const wchar_t *left, const wchar_t *right, size_t count);

/* 7.24.4.4 and 7.24.4.5: searching a wide string, and a wide array. */
wchar_t *wcschr(const wchar_t *text, wchar_t character);
size_t wcscspn(const wchar_t *text, const wchar_t *reject);
wchar_t *wcspbrk(const wchar_t *text, const wchar_t *accept);
wchar_t *wcsrchr(const wchar_t *text, wchar_t character);
size_t wcsspn(const wchar_t *text, const wchar_t *accept);
wchar_t *wcsstr(const wchar_t *haystack, const wchar_t *needle);
wchar_t *wcstok(wchar_t *restrict text, const wchar_t *restrict separators,
                wchar_t **restrict state);
wchar_t *wmemchr(const wchar_t *text, wchar_t character, size_t count);

/* 7.24.4.6: the length of a wide string, in characters. */
size_t wcslen(const wchar_t *text);

#endif
