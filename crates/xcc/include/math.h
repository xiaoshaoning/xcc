/* math.h - C99 7.12, the whole of it.  Part of xcc.
 *
 * Clean room: the wording is C99 7.12 and nothing else.
 *
 * Every function here is declared because the target's C library exports it - a header is a
 * promise, and a promise the linker cannot keep is worse than an absent declaration. The names
 * are also in `back/crt.rs`'s list, which a test compares against this file, so a name can be
 * added to one and forgotten in the other only by breaking the build.
 */
#ifndef XCC_MATH_H
#define XCC_MATH_H

/* For `DBL_MAX` and `DBL_MIN`: the classification macros below are about *the range of a type*, and a
 * header that hard-codes those digits disagrees with the ABI the day someone reads it. */
#include <float.h>

/* 7.12p3: one infinity, and its two wider spellings. `long double` is the target's 8-byte double,
 * so the three are the same number - said here rather than left to a reader to work out. */
#define HUGE_VAL 1.79769313486231570815e+308
#define HUGE_VALF 3.40282346638528859812e+38F
#define HUGE_VALL HUGE_VAL

/* 7.12p4: `float_t` and `double_t` are the types the library actually computes in, and this one
 * computes in `double` for both. */
typedef double float_t;
typedef double double_t;

/* 7.12.3, the classification macros over the functions below them. */
#define FP_NAN 0
#define FP_INFINITE 1
#define FP_ZERO 2
#define FP_SUBNORMAL 3
#define FP_NORMAL 4

/* Trigonometry (7.12.4). */
double acos(double x);
double asin(double x);
double atan(double x);
double atan2(double y, double x);
double cos(double x);
double sin(double x);
double tan(double x);
double acosh(double x);
double asinh(double x);
double atanh(double x);
double cosh(double x);
double sinh(double x);
double tanh(double x);

/* Exponents and logarithms (7.12.6). */
double exp(double x);
double exp2(double x);
double expm1(double x);
double frexp(double value, int *exponent);
int ilogb(double x);
double ldexp(double value, int exponent);
double log(double x);
double log10(double x);
double log1p(double x);
double log2(double x);
double logb(double x);
double modf(double value, double *integral);
double scalbn(double x, int n);
double scalbln(double x, long n);

/* Powers and the absolute value (7.12.7). */
double cbrt(double x);
double fabs(double x);
double hypot(double x, double y);
double pow(double base, double exponent);
double sqrt(double x);

/* Error and gamma functions (7.12.8). */
double erf(double x);
double erfc(double x);
double lgamma(double x);
double tgamma(double x);

/* Rounding and remainders (7.12.9). */
double ceil(double x);
double floor(double x);
double nearbyint(double x);
double rint(double x);
long lrint(double x);
long long llrint(double x);
double round(double x);
long lround(double x);
long long llround(double x);
double trunc(double x);
double fmod(double numerator, double denominator);
double remainder(double x, double y);
double remquo(double x, double y, int *quotient);

/* The last three (7.12.11, 7.12.12). */
double copysign(double magnitude, double sign);
double nan(const char *spelling);
double nextafter(double x, double y);
double nexttoward(double x, long double y);
double fdim(double x, double y);
double fmax(double x, double y);
double fmin(double x, double y);
double fma(double x, double y, double z);

/* 7.12.3p6 to p11: each asks the question without linking a call, and **each may evaluate its
 * argument more than once** - `isnan(f())` calls `f` twice. That is what a macro in this header does,
 * the standard does not forbid it, and a program with a side effect in the argument must hoist it.
 *
 * `isinf` compares against the *range of the type*, not against `HUGE_VAL`: an infinity is greater
 * than any finite value, and `HUGE_VAL` is finite here (MSVC's choice, above). Comparing against it
 * called a real infinity finite - a wrong answer rather than a missing feature. */
#define isnan(x) ((x) != (x))
#define isinf(x) ((x) > DBL_MAX || (x) < -DBL_MAX)
#define isfinite(x) (!isinf(x) && !isnan(x))
#define signbit(x) (copysign(1.0, (double)(x)) < 0)
#define isnormal(x) (isfinite(x) && fabs(x) >= DBL_MIN)
#define fpclassify(x)     (isnan(x) ? FP_NAN : isinf(x) ? FP_INFINITE :         ((x) == 0 ? FP_ZERO : (fabs(x) < DBL_MIN ? FP_SUBNORMAL : FP_NORMAL)))

#endif
