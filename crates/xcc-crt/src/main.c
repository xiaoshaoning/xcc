/* The C runtime for programs built by xcc (ADR-005).
 *
 * Windows starts a program at the entry point named in the PE header, not at `main`. This is
 * that entry point: it calls `main` and hands the result to the C library's `exit`, which
 * flushes stdio and runs at-exit handlers before calling `ExitProcess`. Calling `ExitProcess`
 * here instead would skip the flush - a program that printf's would print nothing and exit 0.
 *
 * It is written in C and compiled by xcc itself: the first program xcc links is also the first
 * it compiles from its own runtime, so every hello.exe is a regression test of the compiler.
 *
 * `argc`/`argv` are the universal runtime's: `_configure_narrow_argv` splits the command line
 * with Windows' own quoting rule, and the two accessors hand back what it stored. Doing it here
 * would be doing it twice, and the rule about quotes is not one this file should own.
 */

#include <stdarg.h>
#include <stdio.h>

void exit(int code);
int main(int argc, char **argv);

/* The universal runtime's argument setup. The mode is `_crt_argv_unexpanded_arguments`: the command
 * line split into arguments by the platform's own quoting rule, with no wildcard expansion. (The
 * alternative, `_crt_argv_expanded_arguments`, is the `setargv.obj` behaviour, which C does not
 * ask for.) */
int _configure_narrow_argv(int mode);
int *__p___argc(void);
char ***__p___argv(void);
#define ARGV_UNEXPANDED 1

void mainCRTStartup(void) {
    _configure_narrow_argv(ARGV_UNEXPANDED);
    exit(main(*__p___argc(), *__p___argv()));
}

/* The `printf` family, which `ucrtbase.dll` does not export.
 *
 * The UCRT moved every variadic front end into the program: the DLL owns the formatting engine
 * (`__stdio_common_vfprintf` and friends, which take a `va_list` and no `...`) and each program
 * carries the eight-line wrapper that turns its arguments into one. MSVC does this in `ucrt.lib`;
 * this is the same layer for programs this compiler links, and it is compiled by the same compiler
 * that will call it - so `printf` is also a test of `va_arg`.
 *
 * The options word: 0 means C99 semantics, and `_CRT_INTERNAL_PRINTF_LEGACY_VSPRINTF_NULL_TERMINATION`
 * is what makes `sprintf` write the null its name promises.
 */
#define LEGACY_VSPRINTF_NULL_TERMINATION 0x1ull
#define NO_LIMIT ((size_t)-1)

int __stdio_common_vfprintf(unsigned long long options, FILE *stream, const char *format,
                            void *locale, va_list args);
int __stdio_common_vsprintf(unsigned long long options, char *buffer, size_t count,
                            const char *format, void *locale, va_list args);
int __stdio_common_vfscanf(unsigned long long options, FILE *stream, const char *format,
                           void *locale, va_list args);
int __stdio_common_vsscanf(unsigned long long options, const char *input, size_t count,
                           const char *format, void *locale, va_list args);
int _crt_atexit(void (*function)(void));

int vfprintf(FILE *restrict stream, const char *restrict format, va_list args) {
    return __stdio_common_vfprintf(0, stream, format, 0, args);
}

int fprintf(FILE *restrict stream, const char *restrict format, ...) {
    va_list args;
    va_start(args, format);
    int written = __stdio_common_vfprintf(0, stream, format, 0, args);
    va_end(args);
    return written;
}

int vprintf(const char *restrict format, va_list args) {
    return __stdio_common_vfprintf(0, stdout, format, 0, args);
}

int printf(const char *restrict format, ...) {
    va_list args;
    va_start(args, format);
    int written = __stdio_common_vfprintf(0, stdout, format, 0, args);
    va_end(args);
    return written;
}

int vsprintf(char *restrict to, const char *restrict format, va_list args) {
    return __stdio_common_vsprintf(LEGACY_VSPRINTF_NULL_TERMINATION, to, NO_LIMIT, format, 0, args);
}

int sprintf(char *restrict to, const char *restrict format, ...) {
    va_list args;
    va_start(args, format);
    int written =
        __stdio_common_vsprintf(LEGACY_VSPRINTF_NULL_TERMINATION, to, NO_LIMIT, format, 0, args);
    va_end(args);
    return written;
}

int vsnprintf(char *restrict to, size_t count, const char *restrict format, va_list args) {
    return __stdio_common_vsprintf(0, to, count, format, 0, args);
}

int snprintf(char *restrict to, size_t count, const char *restrict format, ...) {
    va_list args;
    va_start(args, format);
    int written = __stdio_common_vsprintf(0, to, count, format, 0, args);
    va_end(args);
    return written;
}

/* The scanning half, the same shape: the library owns the engine, the program owns the `...`. An
 * unlimited count is `(size_t)-1`, which is `sscanf`'s "the string ends where the null does". */

int vfscanf(FILE *restrict stream, const char *restrict format, va_list args) {
    return __stdio_common_vfscanf(0, stream, format, 0, args);
}

int fscanf(FILE *restrict stream, const char *restrict format, ...) {
    va_list args;
    va_start(args, format);
    int read = __stdio_common_vfscanf(0, stream, format, 0, args);
    va_end(args);
    return read;
}

int scanf(const char *restrict format, ...) {
    va_list args;
    va_start(args, format);
    int read = __stdio_common_vfscanf(0, stdin, format, 0, args);
    va_end(args);
    return read;
}

int vscanf(const char *restrict format, va_list args) {
    return __stdio_common_vfscanf(0, stdin, format, 0, args);
}

int vsscanf(const char *restrict from, const char *restrict format, va_list args) {
    return __stdio_common_vsscanf(0, from, NO_LIMIT, format, 0, args);
}

int sscanf(const char *restrict from, const char *restrict format, ...) {
    va_list args;
    va_start(args, format);
    int read = __stdio_common_vsscanf(0, from, NO_LIMIT, format, 0, args);
    va_end(args);
    return read;
}

/* `atexit` is the program's own name for the runtime's: the list is the library's, and the
 * registration call is `_crt_atexit`. */
int atexit(void (*function)(void)) {
    return _crt_atexit(function);
}
