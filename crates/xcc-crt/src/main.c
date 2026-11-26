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
#include <threads.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
#include <stdarg.h>
#include <wchar.h>
#include <stdio.h>

/* `TlsAlloc` and the two accessors are the universal runtime's, and `TlsAlloc` in particular is the
 * *kernel's*: it appears in `ucrtbase.dll`'s dump as one of that library's own imports, so the
 * linker asks `kernel32.dll` for it. This compiler ships no `windows.h`, which is why the three are
 * declared here. */
unsigned long TlsAlloc(void);
void *TlsGetValue(unsigned long index);
int TlsSetValue(unsigned long index, void *value);
static unsigned long __xcc_tls_key;
static int __xcc_tls_key_made;

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
    // The thread-local storage key, made here for the one reason there is only one *thread* here:
    // made lazily instead, two threads could each allocate a key and one of them would lose the
    // block it had already filled.
    __xcc_tls_key = TlsAlloc();
    __xcc_tls_key_made = __xcc_tls_key != (unsigned long)-1;
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
    if (magnitude == 0.0) {
        // The formulas below divide by `2t`, and `t` is zero exactly here: `csqrt(0 + 0i)` was
        // `0 - nan` where the root of zero is zero.
        return (double _Complex){ 0.0, 0.0 };
    }
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
        if (__imag__(exponent) == 0.0) {
            if (power > 0.0) {
                return (double _Complex){ 0.0, 0.0 };
            }
            if (power < 0.0) {
                /* Annex G.6.4: an infinite result, and this compiler writes one rather than the
                 * NaN that dividing zero by zero would give. */
                return (double _Complex){ INFINITY, 0.0 };
            }
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
    /* The quotient `csin/ccos` is a NaN where both are infinite, and `tan` of a large imaginary
     * part is one of the places that happens. This is the same value written over the double
     * angles, which no pair of infinities can spoil: (sin 2x + i sinh 2y) / (cos 2x + cosh 2y). */
    double x = __real__(z), y = __imag__(z);
    double denominator = cos(2.0 * x) + cosh(2.0 * y);
    return (double _Complex){ sin(2.0 * x) / denominator, sinh(2.0 * y) / denominator };
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
    /* `tanh x` reaches 1 exactly - in double, from about |x| = 20 on - and past that the quotient
     * above is `inf/inf`. So the saturation is the answer, and it is asked for rather than
     * approximated by a bound: `ctanh(1e308)` is 1, not a NaN. */
    double x = __real__(z), y = __imag__(z);
    double steepness = tanh(x);
    if (fabs(steepness) == 1.0) {
        return (double _Complex){ steepness, copysign(0.0, y) };
    }
    double denominator = cosh(2.0 * x) + cos(2.0 * y);
    return (double _Complex){ sinh(2.0 * x) / denominator, sin(2.0 * y) / denominator };
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
    /* (i/2) log((1 - iz)/(1 + iz)) is the textbook form and it divides by zero at `z = +i`, where
     * the answer is an infinity rather than a NaN - and its ratio's *direction* decides between
     * pi/4 and -pi/4 on the real line, which this got wrong once. This is the same value over the
     * real functions: half the angle of `2x` against `1 - x^2 - y^2`, and a quarter of the log of
     * the squared distances to `+i` and `-i`. Every singular point falls out of it - `z = i` takes
     * the logarithm of a division by zero and gets its infinity. */
    double x = __real__(z), y = __imag__(z);
    double real = atan2(2.0 * x, 1.0 - x * x - y * y) / 2.0;
    double imaginary = log((x * x + (y + 1.0) * (y + 1.0)) / (x * x + (y - 1.0) * (y - 1.0))) / 4.0;
    return (double _Complex){ real, imaginary };
}

double _Complex casinh(double _Complex z) {
    /* `-i asin(iz)`: the standard's own rotation, and it inherits `casin`'s real-arithmetic form
     * instead of logging a sum that can be zero. `casinh(i)` - where `z + sqrt(z^2 + 1)` *is* zero -
     * was a NaN and is now `0 + pi/2 i`. */
    double _Complex w = casin((double _Complex){ -__imag__(z), __real__(z) });
    return (double _Complex){ __imag__(w), -__real__(w) };
}

double _Complex cacosh(double _Complex z) {
    /* `i acos(z)` above the real axis and its conjugate below it, which is Annex G's own branch:
     * the real part is never negative and the sign of the imaginary one follows the argument's.
     * `i(a + bi)` is `-b + ai`, and writing that the other way round negated the imaginary part of
     * `cacosh(-1)` - a conjugate where the branch says otherwise. */
    double _Complex w = cacos(z);
    double real = -__imag__(w), imaginary = __real__(w);
    if (__imag__(z) < 0.0) {
        real = -real;
        imaginary = -imaginary;
    }
    return (double _Complex){ real, imaginary };
}

double _Complex catanh(double _Complex z) {
    /* The real form, as `catan` has: a quarter of the log of the ratio of squared distances to
     * `+1` and `-1`, and half the angle of `2y` against `1 - x^2 - y^2`. The rotation
     * `-i catan(iz)` is the same value inside the domain but not on the cut: there `atan2` takes
     * the side of the signed zero, and `catanh(2)` came back with `-pi/2` where Annex G says
     * `+pi/2`. */
    double x = __real__(z), y = __imag__(z);
    double real = log(((1.0 + x) * (1.0 + x) + y * y) / ((1.0 - x) * (1.0 - x) + y * y)) / 4.0;
    double imaginary = atan2(2.0 * y, 1.0 - x * x - y * y) / 2.0;
    return (double _Complex){ real, imaginary };
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

/* ---------------------------------------------------------------- _Thread_local (C11 6.7.3)
 *
 * One block per thread, holding every `_Thread_local` object of the program, allocated on the
 * thread's first use and initialised from the template the linker laid out. The compiler reaches an
 * object through `__xcc_tls` on every access - a call, not the PE TLS directory's four instructions
 * - because the loader-filled directory is a bigger piece of the image writer than this milestone
 * took on, and a call is the price of not having it. The trade is recorded in docs/AUDIT.md.
 *
 * `TlsAlloc` hands out one process-wide slot, which holds this thread's block; the slot number is
 * the only thing the runtime keeps, and it is allocated on the first access from anywhere.
 *
 * Two limits, stated rather than hidden: a block is never freed (a program that creates threads in
 * a loop leaks one block per thread), and its destructors - which C does not have for this storage
 * duration - are not run on thread exit.
 */
void *__xcc_tls(unsigned offset, unsigned total, const void *initial) {
    char *block;
    if (!__xcc_tls_key_made) {
        return 0;
    }
    block = (char *)TlsGetValue(__xcc_tls_key);
    if (block == 0) {
        block = (char *)calloc(total ? total : 1, 1);
        if (block == 0) {
            return 0;
        }
        if (initial != 0 && total != 0) {
            /* The whole block at once: every `_Thread_local` object of the program is in it, and a
             * per-object copy here would leave every object but the first one at zero. */
            memcpy(block, initial, total);
        }
        TlsSetValue(__xcc_tls_key, block);
    }
    return block + offset;
}

/* ---------------------------------------------------------------- C11 7.17, the atomic operations
 *
 * The machine has exactly one atomic operation here - `lock cmpxchg`, which the compiler emits for
 * `__builtin_atomic_cas` - and every other one is a loop of it, written in C where the reasoning can
 * be read. A compare-and-swap loop is the textbook way to build them, it is a dozen lines, and it
 * means the *back end* has one thing to get right instead of eight.
 *
 * Two widths and no others: 32 bits for `int`, `unsigned` and `long` (32 on this target), 64 for
 * `long long`, `unsigned long long`, `size_t` and a pointer. `_Atomic float` is not offered, because
 * a float swap is not one instruction; `atomic_flag` and `atomic_bool` are four bytes here, which
 * the standard allows - the representation is the implementation's - and is what makes them atomic
 * with the same operation as an `int`.
 *
 * Each of these is one loop of the one primitive. The operator is written out rather than passed as
 * a macro parameter: five short functions per width read better than one function per operation
 * generated by pasting, and this compiler's preprocessor does not re-scan a name built by `##` into
 * a call anyway.
 */

#define XCC_ATOMIC(T, tag)                                                                     T __xcc_add_##tag(volatile T *object, T argument) {                                            T old = *object;                                                                           for (;;) {                                                                                     T seen = __builtin_atomic_cas(object, old, old + argument);                                if (seen == old) return old;                                                               old = seen;                                                                            }                                                                                      }                                                                                          T __xcc_sub_##tag(volatile T *object, T argument) {                                            T old = *object;                                                                           for (;;) {                                                                                     T seen = __builtin_atomic_cas(object, old, old - argument);                                if (seen == old) return old;                                                               old = seen;                                                                            }                                                                                      }                                                                                          T __xcc_and_##tag(volatile T *object, T argument) {                                            T old = *object;                                                                           for (;;) {                                                                                     T seen = __builtin_atomic_cas(object, old, old & argument);                                if (seen == old) return old;                                                               old = seen;                                                                            }                                                                                      }                                                                                          T __xcc_or_##tag(volatile T *object, T argument) {                                             T old = *object;                                                                           for (;;) {                                                                                     T seen = __builtin_atomic_cas(object, old, old | argument);                                if (seen == old) return old;                                                               old = seen;                                                                            }                                                                                      }                                                                                          T __xcc_xor_##tag(volatile T *object, T argument) {                                            T old = *object;                                                                           for (;;) {                                                                                     T seen = __builtin_atomic_cas(object, old, old ^ argument);                                if (seen == old) return old;                                                               old = seen;                                                                            }                                                                                      }                                                                                          T __xcc_exchange_##tag(volatile T *object, T wanted) {                                         T old = *object;                                                                           for (;;) {                                                                                     T seen = __builtin_atomic_cas(object, old, wanted);                                        if (seen == old) return old;                                                               old = seen;                                                                            }                                                                                      }

XCC_ATOMIC(int, int)
XCC_ATOMIC(unsigned, uint)
XCC_ATOMIC(long long, llong)
XCC_ATOMIC(unsigned long long, ullong)
/* The narrow widths too: `lock cmpxchg` has a form for one byte and for two, so `_Atomic
 * char` and `_Atomic short` are as cheap as the wide ones and need a loop each. */
XCC_ATOMIC(char, char)
XCC_ATOMIC(signed char, schar)
XCC_ATOMIC(unsigned char, uchar)
XCC_ATOMIC(short, short)
XCC_ATOMIC(unsigned short, ushort)

/* 7.17.7.4: the compare-and-exchange, whose *contract* is that a failure writes the value it found
 * into `expected`. One primitive, used once - the read of `*expected` is the comparand the caller
 * believes, and the value that comes back is what the object really held. */
#define XCC_CAS(T, tag)                                                                        int __xcc_cas_##tag(volatile T *object, T *expected, T desired) {                              T old = __builtin_atomic_cas(object, *expected, desired);                                  if (old == *expected) {                                                                        return 1;                                                                              }                                                                                          *expected = old;                                                                           return 0;                                                                              }

XCC_CAS(int, int)
XCC_CAS(unsigned, uint)
XCC_CAS(long long, llong)
XCC_CAS(unsigned long long, ullong)
XCC_CAS(char, char)
XCC_CAS(signed char, schar)
XCC_CAS(unsigned char, uchar)
XCC_CAS(short, short)
XCC_CAS(unsigned short, ushort)

/* The MSVC-named atomics, which mingw-w64's headers declare as intrinsics and the Windows SDK
 * leans on: `_InterlockedCompareExchange` is `InterlockedCompareExchange` by another name, and the
 * whole family is the `lock cmpxchg` loop this runtime already has under `__xcc_*`. Implemented
 * here so that a program written for MSVC reaches the same machine operation.
 */
long _InterlockedCompareExchange(long volatile *destination, long exchange, long comparand) {
    long expected = comparand;
    __builtin_atomic_cas(destination, expected, exchange);
    return expected;
}

long _InterlockedExchange(long volatile *target, long value) {
    long old = *target;
    for (;;) {
        long seen = __builtin_atomic_cas(target, old, value);
        if (seen == old) {
            return old;
        }
        old = seen;
    }
}

long _InterlockedExchangeAdd(long volatile *addend, long value) {
    long old = *addend;
    for (;;) {
        long seen = __builtin_atomic_cas(addend, old, old + value);
        if (seen == old) {
            return old;
        }
        old = seen;
    }
}

long _InterlockedIncrement(long volatile *addend) { return _InterlockedExchangeAdd(addend, 1) + 1; }
long _InterlockedDecrement(long volatile *addend) { return _InterlockedExchangeAdd(addend, -1) - 1; }
long _InterlockedAnd(long volatile *value, long mask) {
    long old = *value;
    for (;;) {
        long seen = __builtin_atomic_cas(value, old, old & mask);
        if (seen == old) {
            return old;
        }
        old = seen;
    }
}
long _InterlockedOr(long volatile *value, long mask) {
    long old = *value;
    for (;;) {
        long seen = __builtin_atomic_cas(value, old, old | mask);
        if (seen == old) {
            return old;
        }
        old = seen;
    }
}
long _InterlockedXor(long volatile *value, long mask) {
    long old = *value;
    for (;;) {
        long seen = __builtin_atomic_cas(value, old, old ^ mask);
        if (seen == old) {
            return old;
        }
        old = seen;
    }
}

/* The `__stos*` four are `memset` with an element size, and the products are the two halves a
 * 64x64 multiply gives, the high one through a pointer. */
/* The `movs` family: `rep movsb` and its siblings, which the Windows headers name and a program
 * calls. `memcpy` is the same instruction sequence with the same overlap rule (none), which is what
 * the intrinsic promises. */
void __movsb(unsigned char *dest, const unsigned char *src, size_t count) {
    memcpy(dest, src, count);
}

void __movsw(unsigned short *dest, const unsigned short *src, size_t count) {
    memcpy(dest, src, count * sizeof(unsigned short));
}

void __movsd(unsigned long *dest, const unsigned long *src, size_t count) {
    memcpy(dest, src, count * sizeof(unsigned long));
}

void __movsq(unsigned long long *dest, const unsigned long long *src, size_t count) {
    memcpy(dest, src, count * sizeof(unsigned long long));
}

void __stosb(unsigned char *dest, unsigned char value, size_t count) {
    memset(dest, value, count);
}

void __stosw(unsigned short *dest, unsigned short value, size_t count) {
    while (count-- > 0) {
        *dest++ = value;
    }
}

void __stosd(unsigned long *dest, unsigned long value, size_t count) {
    while (count-- > 0) {
        *dest++ = value;
    }
}

void __stosq(unsigned long long *dest, unsigned long long value, size_t count) {
    while (count-- > 0) {
        *dest++ = value;
    }
}

/* The low 64 bits of `a * b`, with the high 64 through `high`. Four 32-bit products, which is how
 * a compiler without a 128-bit type does it: `a = ah:al`, `b = bh:bl`. */
unsigned long long _umul128(unsigned long long a, unsigned long long b,
                            unsigned long long *high) {
    unsigned long long al = a & 0xFFFFFFFFu;
    unsigned long long ah = a >> 32;
    unsigned long long bl = b & 0xFFFFFFFFu;
    unsigned long long bh = b >> 32;
    unsigned long long low = al * bl;
    // `t2` is `ah*bl` plus the carry out of `al*bl`, and cannot overflow; `t3` is `al*bh` plus the
    // low half of `t2`, which cannot either. The product's low word is the low half of `t3` beside
    // the low half of `al*bl` - the first version forgot `al*bh` there, so its *low* word was wrong
    // while the high one, which uses the same `t3`, came out right.
    unsigned long long t2 = ah * bl + (low >> 32);
    unsigned long long t3 = al * bh + (t2 & 0xFFFFFFFFu);
    *high = ah * bh + (t2 >> 32) + (t3 >> 32);
    return ((t3 & 0xFFFFFFFFu) << 32) | (low & 0xFFFFFFFFu);
}

/* The signed one, over the magnitudes: the high half is the unsigned one's, corrected by the signs
 * of the two operands - which is what two's complement multiplication does to the top word. */
long long _mul128(long long a, long long b, long long *high) {
    unsigned long long ua = a < 0 ? 0 - (unsigned long long)a : (unsigned long long)a;
    unsigned long long ub = b < 0 ? 0 - (unsigned long long)b : (unsigned long long)b;
    unsigned long long top = 0;
    unsigned long long low = _umul128(ua, ub, &top);
    if ((a < 0) != (b < 0)) {
        // The product is negative: its magnitude's high word is complemented with the low one's
        // borrow, which is the same as negating the 128-bit value.
        low = 0 - low;
        top = ~top + (low == 0 ? 1 : 0);
    }
    *high = (long long)top;
    return (long long)low;
}

/* The Microsoft intrinsics a Windows SDK's headers *declare*.
 *
 * These are the SDK's own names, with the SDK's own signatures - `__shiftright128(unsigned __int64,
 * unsigned __int64, unsigned char)`, `_umul128(a, b, *high)`, `__stosd(unsigned __LONG32 *, ...)`
 * are transcribed from `psdk_inc/intrin-impl.h`, and nothing checks the transcription: a wrong width
 * here would be a call that disagrees with its caller's registers and no diagnostic anywhere. They
 * are *arithmetic*, not instruction encodings - the compilers that own them spell them `shrd`, `mul`
 * and `stos` for speed, and a program cannot tell a call from an instruction except by how long it
 * takes.
 *
 * A program that writes one now gets it, because the SDK declares it; a program that merely includes
 * `<windows.h>` no longer needs any of them, which is `__NO_INLINE__`'s doing - the inline wrappers
 * that used to name them are not compiled at all (see `pp/mod.rs`). Every one is checked against
 * `clang`, whose versions *are* instructions, in `tests/exe.rs`.
 *
 * The shift pair is the 128-bit shift `(high:low)` by `count`, in the SDK's argument order - low
 * part first, then the high one - and the count is a *runtime* value, so each guards both edges: a
 * shift by 0 is the operand unchanged, and by 64 or more is the other part.
 */
unsigned long long __shiftright128(unsigned long long low, unsigned long long high,
                                   unsigned char count) {
    if (count == 0) {
        return low;
    }
    if (count >= 64) {
        return count == 64 ? high : 0;
    }
    return (low >> count) | (high << (64 - count));
}

/* The *high* half of `(high:low) << count`, which is the half the SDK's `SHL`-based helpers want -
 * the first version of this returned the low half's expression, and `0x1111... : 0x2222...` by 4
 * came out `0x1111...12` where `clang` says `0x2222...21`. Counts of 64 and above are undefined
 * for both intrinsics (MSVC requires less than 64), so they are answered rather than trusted. */
unsigned long long __shiftleft128(unsigned long long low, unsigned long long high,
                                  unsigned char count) {
    if (count == 0) {
        return high;
    }
    if (count >= 64) {
        return count == 64 ? low : 0;
    }
    return (high << count) | (low >> (64 - count));
}

/* And the fence: a full barrier is a locked operation that changes nothing, and a compare-and-swap
 * against the value the object already holds is exactly that. */
void __xcc_atomic_fence(void) {
    static int scratch;
    (void)__builtin_atomic_cas(&scratch, 0, 0);
}

/* `_mm_mfence`, which `winnt.h` gives the names `MemoryBarrier` and `MemoryFence`. It is written
 * *after* the fence it calls, because the runtime is one translation unit and C99 wants a
 * declaration before a call - the first version sat above it, and every link printed
 * "`__xcc_atomic_fence` is not declared". */
void _mm_mfence(void) { __xcc_atomic_fence(); }

/* ---------------------------------------------------------------- C11 7.26, threads
 *
 * Every one of these is the target's own primitive, declared here because this compiler ships no
 * `windows.h`: `_beginthreadex` for a thread (the C runtime's entry point, so the per-thread runtime
 * state is set up before the function runs), the Win32 critical section for a mutex, the Win32
 * condition variable for `cnd_t`, and the runtime's own TLS slot for `tss_t`.
 *
 * The two representations in `threads.h` are this implementation's, and their sizes are measured
 * rather than guessed: a critical section is 40 bytes on x64 and a condition variable is one
 * pointer. The `_Static_assert`s below are what keeps the header and these casts agreeing.
 */
/* The header's own `mtx_t` and `cnd_t`, not copies of them: a second structure of the same shape
 * would be the same fact written twice, and the assertion would guard the copy rather than the
 * pair. These two are what the Win32 calls below take. */
_Static_assert(sizeof(mtx_t) == 40, "the critical section the header promises");
_Static_assert(sizeof(cnd_t) == 8, "the condition variable the header promises");

unsigned long long _beginthreadex(void *security, unsigned stack, unsigned (*start)(void *),
                                  void *argument, unsigned flags, unsigned *id);
/* The three the thread code needs and the first version forgot: they were called as implicitly
 * declared `int` functions, which the compiler warned about on *every* link - and a warning that
 * appears on every link is one a reader stops seeing. */
int CloseHandle(void *handle);
unsigned long WaitForSingleObject(void *handle, unsigned long milliseconds);
void GetSystemTimeAsFileTime(void *moment);
void _endthreadex(unsigned code);
void Sleep(unsigned long milliseconds);
unsigned long GetCurrentThreadId(void);
int GetExitCodeThread(void *handle, unsigned long *code);
void InitializeCriticalSection(mtx_t *section);
void DeleteCriticalSection(mtx_t *section);
void EnterCriticalSection(mtx_t *section);
void LeaveCriticalSection(mtx_t *section);
int TryEnterCriticalSection(mtx_t *section);
void InitializeConditionVariable(cnd_t *condition);
int SleepConditionVariableCS(cnd_t *condition, mtx_t *section, unsigned long milliseconds);
void WakeConditionVariable(cnd_t *condition);
void WakeAllConditionVariable(cnd_t *condition);
unsigned long TlsFree(unsigned long index);

int thrd_create(thrd_t *thread, thrd_start_t start, void *argument) {
    unsigned id = 0;
    if (thread == 0 || start == 0) {
        return thrd_error;
    }
    /* The C runtime's entry point takes a function returning `unsigned`; the standard's returns
     * `int`. The result is the thread's exit code either way, which is all a join reads. */
    thread->handle = (void *)_beginthreadex(0, 0, (unsigned (*)(void *))start, argument, 0, &id);
    thread->id = (unsigned long)id;
    return thread->handle == 0 ? thrd_nomem : thrd_success;
}

int thrd_equal(thrd_t left, thrd_t right) { return left.id == right.id; }

thrd_t thrd_current(void) {
    thrd_t thread;
    thread.handle = 0;
    thread.id = GetCurrentThreadId();
    return thread;
}

int thrd_sleep(const struct timespec *duration, struct timespec *remaining) {
    if (duration == 0) {
        return -1;
    }
    /* Milliseconds are the resolution the call underneath has; a sub-millisecond request still
     * sleeps for one rather than for none. */
    unsigned long milliseconds = (unsigned long)(duration->tv_sec * 1000L) +
                                 (unsigned long)(duration->tv_nsec / 1000000L);
    if (milliseconds == 0 && (duration->tv_sec != 0 || duration->tv_nsec != 0)) {
        milliseconds = 1;
    }
    Sleep(milliseconds);
    if (remaining != 0) {
        remaining->tv_sec = 0;
        remaining->tv_nsec = 0;
    }
    return 0;
}

void thrd_yield(void) { Sleep(0); }

void thrd_exit(int result) { _endthreadex((unsigned)result); }

int thrd_detach(thrd_t thread) {
    if (thread.handle == 0 || !CloseHandle(thread.handle)) {
        return thrd_error;
    }
    return thrd_success;
}

int thrd_join(thrd_t thread, int *result) {
    unsigned long code = 0;
    if (thread.handle == 0) {
        return thrd_error;
    }
    if (WaitForSingleObject(thread.handle, 0xFFFFFFFFul) != 0) {
        return thrd_error;
    }
    if (result != 0 && !GetExitCodeThread(thread.handle, &code)) {
        return thrd_error;
    }
    CloseHandle(thread.handle);
    if (result != 0) {
        *result = (int)code;
    }
    return thrd_success;
}

/* 7.26.4: a mutex is a critical section. `mtx_timed` is refused: a critical section cannot be
 * entered with a timeout, and handing back a mutex that would block forever would be a promise the
 * platform cannot keep. */
int mtx_init(mtx_t *mutex, int type) {
    if (mutex == 0 || type == mtx_timed) {
        return thrd_error;
    }
    InitializeCriticalSection(mutex);
    return thrd_success;
}

int mtx_lock(mtx_t *mutex) {
    EnterCriticalSection(mutex);
    return thrd_success;
}

int mtx_trylock(mtx_t *mutex) {
    return TryEnterCriticalSection(mutex) ? thrd_success : thrd_busy;
}

int mtx_unlock(mtx_t *mutex) {
    LeaveCriticalSection(mutex);
    return thrd_success;
}

void mtx_destroy(mtx_t *mutex) { DeleteCriticalSection(mutex); }

/* C11 7.27.2.5, which `cnd_timedwait` is the first caller of: the system clock in a `timespec`.
 * The Win32 file time counts 100-nanosecond ticks from 1601, and the Unix epoch is 11 644 473 600
 * seconds later - one subtraction, and the two fields of the answer. */
int timespec_get(struct timespec *moment, int base) {
    unsigned long long ticks;
    unsigned long long since_epoch;
    struct {
        unsigned low;
        unsigned high;
    } file_time;

    if (moment == 0 || base != TIME_UTC) {
        return 0;
    }
    GetSystemTimeAsFileTime(&file_time);
    ticks = ((unsigned long long)file_time.high << 32) | file_time.low;
    since_epoch = ticks - 116444736000000000ULL;
    moment->tv_sec = (long long)(since_epoch / 10000000ULL);
    moment->tv_nsec = (long)(since_epoch % 10000000ULL) * 100L;
    return base;
}

/* 7.26.3: a condition variable is the Win32 one. */
int cnd_init(cnd_t *condition) {
    if (condition == 0) {
        return thrd_error;
    }
    InitializeConditionVariable(condition);
    return thrd_success;
}

int cnd_signal(cnd_t *condition) {
    WakeConditionVariable(condition);
    return thrd_success;
}

int cnd_broadcast(cnd_t *condition) {
    WakeAllConditionVariable(condition);
    return thrd_success;
}

int cnd_wait(cnd_t *condition, mtx_t *mutex) {
    int woken = SleepConditionVariableCS(condition,
                                        mutex, 0xFFFFFFFFul);
    return woken ? thrd_success : thrd_error;
}

int cnd_timedwait(cnd_t *condition, mtx_t *mutex, const struct timespec *absolute) {
    struct timespec now;
    long long milliseconds;
    int woken;
    if (absolute == 0) {
        return thrd_error;
    }
    timespec_get(&now, TIME_UTC);
    milliseconds = (long long)(absolute->tv_sec - now.tv_sec) * 1000LL +
                   (long long)(absolute->tv_nsec - now.tv_nsec) / 1000000LL;
    if (milliseconds <= 0) {
        /* Already due: ask with the shortest wait there is, and report the timeout if it held. */
        woken = SleepConditionVariableCS(condition,
                                        mutex, 0);
        return woken ? thrd_success : thrd_timedout;
    }
    woken = SleepConditionVariableCS(condition,
                                     mutex, (unsigned long)milliseconds);
    return woken ? thrd_success : thrd_timedout;
}

void cnd_destroy(cnd_t *condition) { (void)condition; }

/* 7.26.6: thread-specific storage is the runtime's own slot. The destructor is *not* run, and the
 * header says so: Windows' thread-local storage has no destructor to run. */
int tss_create(tss_t *key, tss_dtor_t destructor) {
    unsigned long index;
    (void)destructor;
    if (key == 0) {
        return thrd_error;
    }
    index = TlsAlloc();
    if (index == (unsigned long)-1) {
        return thrd_error;
    }
    *key = index;
    return thrd_success;
}

void *tss_get(tss_t key) { return TlsGetValue(key); }

int tss_set(tss_t key, void *value) { return TlsSetValue(key, value) ? thrd_success : thrd_error; }

void tss_delete(tss_t key) { (void)TlsFree(key); }

/* 7.26.2: one-time initialisation. A compare-and-swap, which is what the machine has: 0 means "not
 * run", 1 "running", 2 "done". A thread that finds 1 spins, because the alternative - a blocking
 * wait - needs a condition variable that does not exist yet. */
void call_once(once_flag *flag, once_callback callback) {
    volatile int *state = (volatile int *)flag;
    for (;;) {
        int seen = __builtin_atomic_cas(state, 0, 1);
        if (seen == 0) {
            callback();
            (void)__builtin_atomic_cas(state, 1, 2);
            return;
        }
        if (seen == 2) {
            return;
        }
        if (seen == 1) {
            while (*state == 1) {
                Sleep(0);
            }
            if (*state == 2) {
                return;
            }
        }
    }
}
