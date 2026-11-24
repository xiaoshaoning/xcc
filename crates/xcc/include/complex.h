/* complex.h - C99 7.3.
 *
 * The type is the compiler's: `_Complex` over a `float` or a `double`, which this target lays out
 * as two of its element and passes the way it passes a record of the same size (an eight-byte one
 * in a register, a sixteen-byte one behind a pointer). Everything C gives the type and a record
 * does not - `+ - * /`, `== !=`, `!`, and the conversions in and out - is the compiler's own
 * arithmetic, with no call anywhere.
 *
 * What is *here* is the part that lives in a library, and it is all of it, because the universal
 * C runtime exports none of C99 7.3's functions: measured, all sixty-six spellings, `cacos` to
 * `cprojl`, come back "undefined symbol". So the twenty bases are the C runtime source's own
 * (`crates/xcc-crt/src/main.c`): `conj`, `cabs`, `carg` and `cproj`, which are exact, and the
 * sixteen transcendentals of 7.3.5 to 7.3.8, which are the standard's own formulas over `math.h`.
 * `creal` and `cimag` need nothing at all - they are the two halves of the representation, so they
 * are macros here, which is what 7.3.9.5 allows them to be.
 */
#ifndef __XCC_COMPLEX_H
#define __XCC_COMPLEX_H

/* 7.3p2: the two spellings the header adds to the language. */
#define complex _Complex
#define _Complex_I ((const float _Complex){ 0.0F, 1.0F })
#define I _Complex_I

/* 7.3.9.5 and 7.3.9.6: the real and imaginary parts, which need no call: they *are* the two halves
 * of the representation, and a macro is what the standard allows them to be. */
#define creal(z) (__real__(z))
#define cimag(z) (__imag__(z))
#define crealf(z) (__real__(z))
#define cimagf(z) (__imag__(z))
#define creall(z) (__real__(z))
#define cimagl(z) (__imag__(z))

/* The four that are exact arithmetic on those parts, or one math.h call. */
double _Complex conj(double _Complex z);
float _Complex conjf(float _Complex z);

double cabs(double _Complex z);
float cabsf(float _Complex z);

double carg(double _Complex z);
float cargf(float _Complex z);

double _Complex cproj(double _Complex z);
float _Complex cprojf(float _Complex z);

/* 7.3.5 to 7.3.8: the transcendentals, which the runtime this compiler links carries because no
 * Windows runtime exports any of them. Each `float` form computes in `double` and narrows once,
 * like the four `math.h` wrappers. */
double _Complex cexp(double _Complex z);
float _Complex cexpf(float _Complex z);

double _Complex clog(double _Complex z);
float _Complex clogf(float _Complex z);

double _Complex csqrt(double _Complex z);
float _Complex csqrtf(float _Complex z);

double _Complex cpow(double _Complex base, double _Complex exponent);
float _Complex cpowf(float _Complex base, float _Complex exponent);

double _Complex csin(double _Complex z);
float _Complex csinf(float _Complex z);

double _Complex ccos(double _Complex z);
float _Complex ccosf(float _Complex z);

double _Complex ctan(double _Complex z);
float _Complex ctanf(float _Complex z);

double _Complex csinh(double _Complex z);
float _Complex csinhf(float _Complex z);

double _Complex ccosh(double _Complex z);
float _Complex ccoshf(float _Complex z);

double _Complex ctanh(double _Complex z);
float _Complex ctanhf(float _Complex z);

double _Complex casin(double _Complex z);
float _Complex casinf(float _Complex z);

double _Complex cacos(double _Complex z);
float _Complex cacosf(float _Complex z);

double _Complex catan(double _Complex z);
float _Complex catanf(float _Complex z);

double _Complex casinh(double _Complex z);
float _Complex casinhf(float _Complex z);

double _Complex cacosh(double _Complex z);
float _Complex cacoshf(float _Complex z);

double _Complex catanh(double _Complex z);
float _Complex catanhf(float _Complex z);

/* 7.3 asks for an `l` form of each, and this target's `long double _Complex` **is** its
 * `double _Complex` - so a definition would be a redefinition of the row above, and the
 * spelling is a macro, exactly as `math.h` writes its own `l` family. Declaring them as
 * functions instead was a promise the linker could not keep: `cabsl` was an undefined symbol. */
#define conjl conj
#define cabsl cabs
#define cargl carg
#define cprojl cproj

#endif
