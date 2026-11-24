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
#include <math.h>
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

/* math.h's four `float` variants that the library does not export: each is its double function with
 * a narrower value, and each is exact - a `float` converts to a `double` with no loss, so the
 * computation is the same one the `f` function would do. */
float fabsf(float x) {
    return (float)fabs((double)x);
}

float frexpf(float value, int *exponent) {
    return (float)frexp((double)value, exponent);
}

float ldexpf(float value, int exponent) {
    return (float)ldexp((double)value, exponent);
}

float hypotf(float x, float y) {
    return (float)hypot((double)x, (double)y);
}

/* complex.h's six functions that are exact: three are the parts themselves, three are one math.h
 * call. The transcendental nineteen are not here and not declared - the universal C runtime exports
 * none of C99 7.3's sixty-six spellings, measured one name at a time. */
double _Complex conj(double _Complex z) {
    double re = __real__(z), im = __imag__(z);
    return (double _Complex){ re, -im };
}

float _Complex conjf(float _Complex z) {
    float re = __real__(z), im = __imag__(z);
    return (float _Complex){ re, -im };
}

long double _Complex conjl(long double _Complex z) {
    return (long double _Complex){ __real__(z), -__imag__(z) };
}

double cabs(double _Complex z) {
    return hypot(__real__(z), __imag__(z));
}

float cabsf(float _Complex z) {
    return hypotf(__real__(z), __imag__(z));
}

double carg(double _Complex z) {
    return atan2(__imag__(z), __real__(z));
}

float cargf(float _Complex z) {
    return atan2f(__imag__(z), __real__(z));
}

double _Complex cproj(double _Complex z) {
    /* C99 7.3.9.4: an infinite part makes the whole value the positive infinity on the real axis,
     * with the sign of the imaginary part kept on the zero. */
    double re = __real__(z), im = __imag__(z);
    if (isinf(re) || isinf(im)) {
        return (double _Complex){ INFINITY, copysign(0.0, im) };
    }
    return z;
}

float _Complex cprojf(float _Complex z) {
    float re = __real__(z), im = __imag__(z);
    if (isinf(re) || isinf(im)) {
        return (float _Complex){ INFINITY, copysignf(0.0F, im) };
    }
    return z;
}

/* ---------------------------------------------------------------- C99 7.3, the transcendentals
 *
 * The universal C runtime exports none of them, measured one link at a time, so they are here -
 * over `math.h` and over the compiler's own complex arithmetic. Each `double` version is the
 * textbook formula C99 6.3.1.5 and Annex G describe; each `float` version computes in `double` and
 * narrows once, which is what the four `math.h` wrappers above do and costs nothing on this target.
 *
 * Two formulas are written out rather than composed: `cexp` (which is where a complex exponential
 * starts) and `csqrt`, whose branch is chosen by the sign of the real part so that the principal
 * root lands on the right side of the cut. Everything else is those two plus `+ - * /`, so there is
 * one place for each branch and not one per function.
 */

#define XCC_COMPLEX_PI 3.14159265358979323846
#define XCC_PAIR(z) (double _Complex){ __real__(z), __imag__(z) }
#define XCC_NARROW(w) (float _Complex){ (float)__real__(w), (float)__imag__(w) }

double _Complex cexp(double _Complex z) {
    double scale = exp(__real__(z));
    return (double _Complex){ scale * cos(__imag__(z)), scale * sin(__imag__(z)) };
}

double _Complex clog(double _Complex z) {
    return (double _Complex){ log(cabs(z)), carg(z) };
}

double _Complex csqrt(double _Complex z) {
    double re = __real__(z), im = __imag__(z);
    double magnitude = hypot(re, im);
    /* The principal root is the one with a non-negative real part, and `(r + x) / 2` loses
     * nothing on that side; on the other side `(r - x) / 2` is the one that survives, and the
     * imaginary part takes its sign from `y`. */
    if (re >= 0.0) {
        double t = sqrt((magnitude + re) / 2.0);
        return (double _Complex){ t, im / (2.0 * t) };
    }
    // The real part of the principal root is never negative, so it is `|y|/(2t)` and not `y/(2t)`:
    // with the signed value, `csqrt(-0.5 - 1.5i)` came back with its real part negated - and so did
    // every function built on this one, which is how it was found.
    double t = sqrt((magnitude - re) / 2.0);
    return (double _Complex){ fabs(im) / (2.0 * t), copysign(t, im) };
}

double _Complex cpow(double _Complex base, double _Complex exponent) {
    /* 7.3.8.1 defines it as `cexp(y * clog(x))`, and that is what real code relies on; the case
     * it cannot express - a zero base - is the one special case worth writing. */
    if (__real__(base) == 0.0 && __imag__(base) == 0.0) {
        double power = __real__(exponent);
        if (__imag__(exponent) == 0.0 && power > 0.0) {
            return (double _Complex){ 0.0, 0.0 };
        }
        return (double _Complex){ 0.0 / 0.0, 0.0 / 0.0 };
    }
    return cexp(exponent * clog(base));
}

double _Complex csin(double _Complex z) {
    return (double _Complex){ sin(__real__(z)) * cosh(__imag__(z)),
                              cos(__real__(z)) * sinh(__imag__(z)) };
}

double _Complex ccos(double _Complex z) {
    return (double _Complex){ cos(__real__(z)) * cosh(__imag__(z)),
                              -sin(__real__(z)) * sinh(__imag__(z)) };
}

double _Complex ctan(double _Complex z) {
    return csin(z) / ccos(z);
}

double _Complex csinh(double _Complex z) {
    return (double _Complex){ sinh(__real__(z)) * cos(__imag__(z)),
                              cosh(__real__(z)) * sin(__imag__(z)) };
}

double _Complex ccosh(double _Complex z) {
    return (double _Complex){ cosh(__real__(z)) * cos(__imag__(z)),
                              sinh(__real__(z)) * sin(__imag__(z)) };
}

double _Complex ctanh(double _Complex z) {
    return csinh(z) / ccosh(z);
}

double _Complex casin(double _Complex z) {
    /* `-i log(iz + sqrt(1 - z^2))` is the textbook form and it has a sign to choose: `sqrt` here
     * does not want the principal root for |x| > 1, and with it `asin(3 + i)` came back as
     * 3.07 - 0.72i, a real part outside [-pi/2, pi/2]. So the real arithmetic of 4.4.39 in
     * Abramowitz and Stegun, which has no choice in it: the real part is `asin(p)` for a `p` that
     * is always in [-1, 1], and the imaginary part is `acosh(q)` with the sign of `y`. */
    double x = __real__(z), y = __imag__(z);
    double near = hypot(x + 1.0, y), far = hypot(x - 1.0, y);
    double p = (near - far) / 2.0;
    double q = (near + far) / 2.0;
    double imaginary = log(q + sqrt(q * q - 1.0));
    return (double _Complex){ asin(p), y < 0.0 ? -imaginary : imaginary };
}

double _Complex cacos(double _Complex z) {
    double _Complex w = casin(z);
    return (double _Complex){ XCC_COMPLEX_PI / 2.0 - __real__(w), -__imag__(w) };
}

double _Complex catan(double _Complex z) {
    /* (i/2) log((1 - iz) / (1 + iz)). The ratio's *direction* is not a matter of taste: on the
     * real line it decides between pi/4 and -pi/4, and written the other way round this answered
     * catan(1.5 - 0.75i) with the sign of its own result flipped. Then (i/2)(a + bi) is
     * -b/2 + (a/2)i. */
    double _Complex iz = (double _Complex){ -__imag__(z), __real__(z) };
    double _Complex quotient = ((double _Complex){ 1.0, 0.0 } - iz) / ((double _Complex){ 1.0, 0.0 } + iz);
    double _Complex w = clog(quotient);
    // (i/2)(a + bi) = (-b/2) + (a/2)i
    return (double _Complex){ -__imag__(w) / 2.0, __real__(w) / 2.0 };
}

double _Complex casinh(double _Complex z) {
    return clog(z + csqrt(z * z + (double _Complex){ 1.0, 0.0 }));
}

double _Complex cacosh(double _Complex z) {
    /* The form that keeps the branch right on both sides of the cut: 2 log(sqrt((z+1)/2) +
     * sqrt((z-1)/2)), which is `clog(z + sqrt(z^2 - 1))` written so the subtraction cannot lose
     * the small term. */
    double _Complex half = (double _Complex){ 0.5, 0.0 };
    double _Complex one = (double _Complex){ 1.0, 0.0 };
    return 2.0 * clog(csqrt((z + one) * half) + csqrt((z - one) * half));
}

double _Complex catanh(double _Complex z) {
    double _Complex one = (double _Complex){ 1.0, 0.0 };
    double _Complex w = clog((one + z) / (one - z));
    return (double _Complex){ __real__(w) / 2.0, __imag__(w) / 2.0 };
}

/* The `float` forms: compute in `double`, narrow once. */
float _Complex cexpf(float _Complex z) { return XCC_NARROW(cexp(XCC_PAIR(z))); }
float _Complex clogf(float _Complex z) { return XCC_NARROW(clog(XCC_PAIR(z))); }
float _Complex csqrtf(float _Complex z) { return XCC_NARROW(csqrt(XCC_PAIR(z))); }
float _Complex cpowf(float _Complex b, float _Complex e) { return XCC_NARROW(cpow(XCC_PAIR(b), XCC_PAIR(e))); }
float _Complex csinf(float _Complex z) { return XCC_NARROW(csin(XCC_PAIR(z))); }
float _Complex ccosf(float _Complex z) { return XCC_NARROW(ccos(XCC_PAIR(z))); }
float _Complex ctanf(float _Complex z) { return XCC_NARROW(ctan(XCC_PAIR(z))); }
float _Complex csinhf(float _Complex z) { return XCC_NARROW(csinh(XCC_PAIR(z))); }
float _Complex ccoshf(float _Complex z) { return XCC_NARROW(ccosh(XCC_PAIR(z))); }
float _Complex ctanhf(float _Complex z) { return XCC_NARROW(ctanh(XCC_PAIR(z))); }
float _Complex casinf(float _Complex z) { return XCC_NARROW(casin(XCC_PAIR(z))); }
float _Complex cacosf(float _Complex z) { return XCC_NARROW(cacos(XCC_PAIR(z))); }
float _Complex catanf(float _Complex z) { return XCC_NARROW(catan(XCC_PAIR(z))); }
float _Complex casinhf(float _Complex z) { return XCC_NARROW(casinh(XCC_PAIR(z))); }
float _Complex cacoshf(float _Complex z) { return XCC_NARROW(cacosh(XCC_PAIR(z))); }
float _Complex catanhf(float _Complex z) { return XCC_NARROW(catanh(XCC_PAIR(z))); }
