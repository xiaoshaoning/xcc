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

#include <fenv.h>
#include <stdarg.h>
#include <wchar.h>
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
    // The default floating-point environment, captured while it still is one: `fesetenv(FE_DFL_ENV)`
    // restores whatever this holds, and the process starts in the default.
    fegetenv(&_Fenv_default);
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

/* The wide engines, which take and produce `wchar_t` and are otherwise the same shape. */
int __stdio_common_vfwprintf(unsigned long long options, FILE *stream, const wchar_t *format,
                             void *locale, va_list args);
int __stdio_common_vswprintf(unsigned long long options, wchar_t *buffer, size_t count,
                             const wchar_t *format, void *locale, va_list args);
int __stdio_common_vfwscanf(unsigned long long options, FILE *stream, const wchar_t *format,
                            void *locale, va_list args);
int __stdio_common_vswscanf(unsigned long long options, const wchar_t *input, size_t count,
                            const wchar_t *format, void *locale, va_list args);
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

/* fenv.h, the two functions no Windows runtime exports.
 *
 * `feraiseexcept` raises each exception by performing the operation that causes it: they are masked
 * by default, so the operation sets the status flag rather than trapping, which is what the standard
 * asks for. The values are `volatile` so the compiler computes them rather than folding the flags
 * away. `feupdateenv` is the standard's own recipe - save what is raised, restore the environment,
 * raise it again in the new one. */
fenv_t _Fenv_default;

int feraiseexcept(int excepts) {
    volatile double zero = 0.0;
    volatile double one = 1.0;
    volatile double big = 1e308;
    volatile double tiny = 1e-308;
    volatile double sink = 0.0;

    if (excepts & FE_INVALID) sink = zero / zero;
    if (excepts & FE_DIVBYZERO) sink = one / zero;
    if (excepts & FE_OVERFLOW) sink = big * big;
    if (excepts & FE_UNDERFLOW) sink = tiny * tiny;
    if (excepts & FE_INEXACT) sink = one / 3.0;
    (void)sink;
    return 0;
}

int feupdateenv(const fenv_t *environment) {
    int raised = fetestexcept(FE_ALL_EXCEPT);
    int status = fesetenv(environment);
    if (status != 0) {
        return status;
    }
    return feraiseexcept(raised);
}

/* wchar.h, the functions no Windows runtime exports.
 *
 * The twelve wide `stdio` front ends are the same division as the narrow ones: the UCRT owns the
 * engine - `__stdio_common_vfwprintf` and the three beside it, each taking a `va_list` - and the
 * program carries the `...` that turns its arguments into one. The four `wmem*` functions and
 * `mbsinit` are small enough to be written here.
 *
 * `fwide` is this platform's stub, and it is the *platform's*: the UCRT has one orientation, the
 * byte one, and its own header answers with the mode it was asked for. A program that asks gets 0;
 * one that sets gets what it set; both are true statements about the only orientation there is.
 */

int fwide(FILE *stream, int mode) {
    (void)stream;
    return mode;
}

int mbsinit(const mbstate_t *state) {
    /* 7.24.2.6: a state that describes the initial conversion - no character held over, no bytes
     * consumed in the middle of one. */
    return state == 0 || (state->_Wchar == 0 && state->_Byte == 0 && state->_State == 0);
}

wchar_t *wmemset(wchar_t *to, wchar_t character, size_t count) {
    size_t i;
    for (i = 0; i < count; i++) {
        to[i] = character;
    }
    return to;
}

wchar_t *wmemcpy(wchar_t *restrict to, const wchar_t *restrict from, size_t count) {
    size_t i;
    for (i = 0; i < count; i++) {
        to[i] = from[i];
    }
    return to;
}

wchar_t *wmemmove(wchar_t *to, const wchar_t *from, size_t count) {
    size_t i;
    if (to < from) {
        for (i = 0; i < count; i++) {
            to[i] = from[i];
        }
    } else if (to > from) {
        for (i = count; i > 0; i--) {
            to[i - 1] = from[i - 1];
        }
    }
    return to;
}

int wmemcmp(const wchar_t *left, const wchar_t *right, size_t count) {
    size_t i;
    for (i = 0; i < count; i++) {
        if (left[i] != right[i]) {
            return left[i] < right[i] ? -1 : 1;
        }
    }
    return 0;
}

wchar_t *wmemchr(const wchar_t *text, wchar_t character, size_t count) {
    size_t i;
    for (i = 0; i < count; i++) {
        if (text[i] == character) {
            return (wchar_t *)text + i;
        }
    }
    return 0;
}

int vfwprintf(FILE *restrict stream, const wchar_t *restrict format, va_list args) {
    return __stdio_common_vfwprintf(0, stream, format, 0, args);
}

int fwprintf(FILE *restrict stream, const wchar_t *restrict format, ...) {
    va_list args;
    va_start(args, format);
    int written = __stdio_common_vfwprintf(0, stream, format, 0, args);
    va_end(args);
    return written;
}

int vwprintf(const wchar_t *restrict format, va_list args) {
    return __stdio_common_vfwprintf(0, stdout, format, 0, args);
}

int wprintf(const wchar_t *restrict format, ...) {
    va_list args;
    va_start(args, format);
    int written = __stdio_common_vfwprintf(0, stdout, format, 0, args);
    va_end(args);
    return written;
}

int vswprintf(wchar_t *restrict to, size_t count, const wchar_t *restrict format, va_list args) {
    return __stdio_common_vswprintf(0, to, count, format, 0, args);
}

int swprintf(wchar_t *restrict to, size_t count, const wchar_t *restrict format, ...) {
    va_list args;
    va_start(args, format);
    int written = __stdio_common_vswprintf(0, to, count, format, 0, args);
    va_end(args);
    return written;
}

int vfwscanf(FILE *restrict stream, const wchar_t *restrict format, va_list args) {
    return __stdio_common_vfwscanf(0, stream, format, 0, args);
}

int fwscanf(FILE *restrict stream, const wchar_t *restrict format, ...) {
    va_list args;
    va_start(args, format);
    int read = __stdio_common_vfwscanf(0, stream, format, 0, args);
    va_end(args);
    return read;
}

int vwscanf(const wchar_t *restrict format, va_list args) {
    return __stdio_common_vfwscanf(0, stdin, format, 0, args);
}

int wscanf(const wchar_t *restrict format, ...) {
    va_list args;
    va_start(args, format);
    int read = __stdio_common_vfwscanf(0, stdin, format, 0, args);
    va_end(args);
    return read;
}

int vswscanf(const wchar_t *restrict from, const wchar_t *restrict format, va_list args) {
    return __stdio_common_vswscanf(0, from, NO_LIMIT, format, 0, args);
}

int swscanf(const wchar_t *restrict from, const wchar_t *restrict format, ...) {
    va_list args;
    va_start(args, format);
    int read = __stdio_common_vswscanf(0, from, NO_LIMIT, format, 0, args);
    va_end(args);
    return read;
}
