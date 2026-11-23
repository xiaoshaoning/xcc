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
 * `argc`/`argv` are M6 (ADR-005): they need CommandLineToArgvW, WideCharToMultiByte, a heap and
 * Windows' own quoting rule, none of which belongs in the twelve lines that make a program run.
 */

#include <stdarg.h>
#include <stdio.h>

void exit(int code);
int main(void);

void mainCRTStartup(void) {
    exit(main());
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
