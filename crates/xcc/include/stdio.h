/* stdio.h - C99 7.19, declarations only: every function here is the C runtime's, which the
 * linker brings in.  A variadic one (`printf`) is called by the rules ADR-006 records - the call
 * site's arguments are its signature - and a floating-point argument to one needs `--opt-level=2`,
 * the only level that writes the general-register duplicate Win64 asks for.
 */
#ifndef XCC_STDIO_H
#define XCC_STDIO_H

#include <stdarg.h>
#include <stddef.h>

typedef struct _iobuf FILE;

/* The three streams are not variables in this C library: it hands out the descriptor for an
 * index, and the standard names are macros over that. Declaring them as `extern FILE *stdin`
 * would compile and then fail to link - a promise the target cannot keep. */
FILE *__acrt_iob_func(unsigned index);
#define stdin __acrt_iob_func(0)
#define stdout __acrt_iob_func(1)
#define stderr __acrt_iob_func(2)

#define EOF (-1)
#define BUFSIZ 512
#define FOPEN_MAX 20
#define FILENAME_MAX 260
#define L_tmpnam 260
#define TMP_MAX 32767
#define SEEK_SET 0
#define SEEK_CUR 1
#define SEEK_END 2
/* 7.19.5.6: the three buffering modes `setvbuf` takes, and the values are the target's. */
#define _IOFBF 0
#define _IOLBF 64
#define _IONBF 4

int remove(const char *path);
int rename(const char *from, const char *to);
FILE *tmpfile(void);
int fclose(FILE *stream);
int fflush(FILE *stream);
FILE *fopen(const char *restrict path, const char *restrict mode);
FILE *freopen(const char *restrict path, const char *restrict mode, FILE *restrict stream);
void setbuf(FILE *restrict stream, char *restrict buffer);
int setvbuf(FILE *restrict stream, char *restrict buffer, int mode, size_t size);

int fprintf(FILE *restrict stream, const char *restrict format, ...);
int fscanf(FILE *restrict stream, const char *restrict format, ...);
int printf(const char *restrict format, ...);
int scanf(const char *restrict format, ...);
int snprintf(char *restrict to, size_t count, const char *restrict format, ...);
int sprintf(char *restrict to, const char *restrict format, ...);
int sscanf(const char *restrict from, const char *restrict format, ...);

/* 7.19.6.8, 7.19.6.9, 7.19.6.12-14: the `va_list` forms, which are how a program writes a
 * variadic function of its own - the standard's own example of `error` is one of these calls. They
 * are functions, not macros, so their arguments are checked like any other. */
int vfprintf(FILE *restrict stream, const char *restrict format, va_list args);
int vfscanf(FILE *restrict stream, const char *restrict format, va_list args);
int vprintf(const char *restrict format, va_list args);
int vscanf(const char *restrict format, va_list args);
int vsnprintf(char *restrict to, size_t count, const char *restrict format, va_list args);
int vsprintf(char *restrict to, const char *restrict format, va_list args);
int vsscanf(const char *restrict from, const char *restrict format, va_list args);

int fgetc(FILE *stream);
char *fgets(char *restrict to, int count, FILE *restrict stream);
int fputc(int c, FILE *stream);
int fputs(const char *restrict text, FILE *restrict stream);
int getc(FILE *stream);
int getchar(void);
int putc(int c, FILE *stream);
int putchar(int c);
int puts(const char *text);
int ungetc(int c, FILE *stream);

size_t fread(void *restrict to, size_t size, size_t count, FILE *restrict stream);
size_t fwrite(const void *restrict from, size_t size, size_t count, FILE *restrict stream);
/* `long`, not `long long`: the C library's `fseek` takes a 32-bit offset on this ABI (MSVC's
 * `_fseeki64` is the 64-bit one, and not C99). Declaring it wider would compile and truncate. */
int fseek(FILE *stream, long offset, int origin);
long ftell(FILE *stream);
void rewind(FILE *stream);
void clearerr(FILE *stream);
int feof(FILE *stream);
int ferror(FILE *stream);
void perror(const char *text);
/* **The target's own additions, which C99 does not have.** They are here because this header is
 * the one `<stdio.h>` resolves to, so the SDK's copy - which declares them - is never read, and a
 * program that calls one was told only that the name was undeclared. An undeclared function is an
 * `int`, and these return a `FILE *`: Lua's `_popen` came back as a thirty-two-bit number widened
 * into a pointer, which works until the heap is above four gigabytes. Each is a documented
 * Microsoft form of a POSIX function, and each is what `ucrtbase` exports under that name. */
int _fileno(FILE *stream);
FILE *_fdopen(int descriptor, const char *mode);
FILE *_popen(const char *command, const char *mode);
int _pclose(FILE *stream);

/* 7.19.4.4: a name for a file that did not exist a moment ago. C99 deprecated it, and C11 replaced
 * it with `tmpnam_s`; it is what real C written against C99 calls, and the declaration is the whole
 * difference between a call and an implicit `int` returned into a pointer comparison. */
char *tmpnam(char *name);

#endif
