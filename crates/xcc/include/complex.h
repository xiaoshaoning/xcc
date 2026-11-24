/* complex.h - C99 7.3.
 *
 * The type is the compiler's: `_Complex` over a `float` or a `double`, which this target lays out
 * as two of its element and passes the way it passes a record of the same size (an eight-byte one
 * in a register, a sixteen-byte one behind a pointer). Everything C gives the type and a record
 * does not - `+ - * /`, `== !=`, `!`, and the conversions in and out - is the compiler's own
 * arithmetic, with no call anywhere.
 *
 * What is *here* is the part that lives in a library, and it is short because the universal C
 * runtime exports none of C99 7.3's functions: measured, all sixty-six spellings, `cacos` to
 * `cprojl`, come back "undefined symbol". The nineteen transcendental ones are therefore not
 * declared - a program that calls `csin` is told it is not declared here rather than given a link
 * error - and the six that are exact are below, four of them in the runtime this compiler links
 * on every link (`crates/xcc-crt/src/main.c`) and two as macros over the parts.
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

/* 7.3 asks for an `l` form of each, and this target's `long double _Complex` **is** its
 * `double _Complex` - so a definition would be a redefinition of the row above, and the
 * spelling is a macro, exactly as `math.h` writes its own `l` family. Declaring them as
 * functions instead was a promise the linker could not keep: `cabsl` was an undefined symbol. */
#define conjl conj
#define cabsl cabs
#define cargl carg
#define cprojl cproj

#endif
