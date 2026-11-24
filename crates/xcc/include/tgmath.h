/* tgmath.h - C99 7.22: one name per function, chosen by the type of the argument.  Part of xcc.
 *
 * Clean room: the wording is C99 7.22 and nothing else.
 *
 * **This header needs C11's `_Generic`, and says so by using it.** C99 asks a compiler for
 * type-generic macros but gives no way to ask "what type is this expression", so a C99-only
 * compiler has to guess - the usual guess is `sizeof`, which gets the *value* right and the *type*
 * wrong (a conditional over a `float` and a `double` call has type `double`), and then
 * `sqrt(4.0f)` returns a double. Refusing with a precise reason beats answering with the wrong
 * type: at `--std=c99` this file fails with the compiler's own "`_Generic` is C11: compile with
 * `--std=c11`", and at `--std=c11` every macro below is exact.
 *
 * 7.22's rule is that the *widest* argument decides, so the selector is the expression itself for
 * one argument and the sum of both for two - `_Generic`'s controlling expression is not evaluated
 * (C11 6.5.1.1p3), so an argument with a side effect still runs exactly once, in the call.
 *
 * **`long double` is the target's 8-byte double here**, so `default` is both of them and the `l`
 * functions are the `double` ones.
 *
 * **The complex half of 7.22 is absent.** It needs a `_Complex` type to dispatch on, and this
 * compiler has none - `<complex.h>` refuses for the same reason, and names the half that is
 * missing. A program that writes `csqrt(z)` gets "implicit declaration", which is the truth.
 */
#ifndef XCC_TGMATH_H
#define XCC_TGMATH_H

#include <math.h>

/* The call is written into each arm rather than made on the selection: the chosen arm is the only
 * one the compiler emits, so an argument with a side effect is evaluated once - and the type of the
 * selection is the type of the function that was chosen, un-promoted. */
#define _XCC_TG1(name, x) _Generic((x), float: name##f(x), default: name(x))
#define _XCC_TG2(name, x, y) _Generic((x) + (y), float: name##f(x, y), default: name(x, y))

/* 7.22's real functions, one macro each. The one-argument ones dispatch on the argument and the
 * two-argument ones on the sum, because 7.22 takes the largest of the argument types. The ones with
 * an `int` or a pointer alongside a value dispatch on the value alone, which is what 7.22 says. */
#define acos(x) _XCC_TG1(acos, x)
#define asin(x) _XCC_TG1(asin, x)
#define atan(x) _XCC_TG1(atan, x)
#define atan2(y, x) _XCC_TG2(atan2, y, x)
#define cos(x) _XCC_TG1(cos, x)
#define sin(x) _XCC_TG1(sin, x)
#define tan(x) _XCC_TG1(tan, x)
#define acosh(x) _XCC_TG1(acosh, x)
#define asinh(x) _XCC_TG1(asinh, x)
#define atanh(x) _XCC_TG1(atanh, x)
#define cosh(x) _XCC_TG1(cosh, x)
#define sinh(x) _XCC_TG1(sinh, x)
#define tanh(x) _XCC_TG1(tanh, x)
#define exp(x) _XCC_TG1(exp, x)
#define exp2(x) _XCC_TG1(exp2, x)
#define expm1(x) _XCC_TG1(expm1, x)
#define log(x) _XCC_TG1(log, x)
#define log10(x) _XCC_TG1(log10, x)
#define log1p(x) _XCC_TG1(log1p, x)
#define log2(x) _XCC_TG1(log2, x)
#define logb(x) _XCC_TG1(logb, x)
#define pow(x, y) _XCC_TG2(pow, x, y)
#define sqrt(x) _XCC_TG1(sqrt, x)
#define cbrt(x) _XCC_TG1(cbrt, x)
#define fabs(x) _XCC_TG1(fabs, x)
#define hypot(x, y) _XCC_TG2(hypot, x, y)
#define fmod(x, y) _XCC_TG2(fmod, x, y)
#define remainder(x, y) _XCC_TG2(remainder, x, y)
#define copysign(x, y) _XCC_TG2(copysign, x, y)
#define nextafter(x, y) _XCC_TG2(nextafter, x, y)
#define fdim(x, y) _XCC_TG2(fdim, x, y)
#define fmax(x, y) _XCC_TG2(fmax, x, y)
#define fmin(x, y) _XCC_TG2(fmin, x, y)
#define fma(x, y, z) _Generic((x) + (y) + (z), float: fmaf(x, y, z), default: fma(x, y, z))
#define ceil(x) _XCC_TG1(ceil, x)
#define floor(x) _XCC_TG1(floor, x)
#define trunc(x) _XCC_TG1(trunc, x)
#define round(x) _XCC_TG1(round, x)
#define nearbyint(x) _XCC_TG1(nearbyint, x)
#define rint(x) _XCC_TG1(rint, x)
#define lrint(x) _XCC_TG1(lrint, x)
#define llrint(x) _XCC_TG1(llrint, x)
#define lround(x) _XCC_TG1(lround, x)
#define llround(x) _XCC_TG1(llround, x)
#define ilogb(x) _XCC_TG1(ilogb, x)
#define erf(x) _XCC_TG1(erf, x)
#define erfc(x) _XCC_TG1(erfc, x)
#define lgamma(x) _XCC_TG1(lgamma, x)
#define tgamma(x) _XCC_TG1(tgamma, x)

/* The ones with a second argument that is not a value to dispatch on. */
#define frexp(x, exponent) _Generic((x), float: frexpf(x, exponent), default: frexp(x, exponent))
#define ldexp(x, exponent) _Generic((x), float: ldexpf(x, exponent), default: ldexp(x, exponent))
#define modf(x, whole) _Generic((x), float: modff(x, whole), default: modf(x, whole))
#define scalbn(x, n) _Generic((x), float: scalbnf(x, n), default: scalbn(x, n))
#define scalbln(x, n) _Generic((x), float: scalblnf(x, n), default: scalbln(x, n))
#define nexttoward(x, y) _Generic((x), float: nexttowardf(x, y), default: nexttoward(x, y))

/* 7.22 also makes the classification macros type-generic, which `math.h`'s already are: they accept
 * any arithmetic type. Nothing to dispatch. */

#endif
