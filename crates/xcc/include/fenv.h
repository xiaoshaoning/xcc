/* fenv.h - C99 7.6: the floating-point environment.  Part of xcc.
 *
 * Clean room: the wording is C99 7.6 and nothing else. The *values* are the target's, measured
 * against the C runtime this compiler links: a program that hands `FE_UPWARD` to `fesetround` must
 * hand it the number the library reads.
 *
 * Two functions of the eleven are not exported by any Windows runtime - `feraiseexcept` and
 * `feupdateenv` - so the runtime this compiler compiles into every link carries them
 * (`crates/xcc-crt/src/main.c`), written in C over the nine that are.
 *
 * C99 7.6.1 makes `#pragma STDC FENV_ACCESS` the way a program promises to look at the environment.
 * This compiler does not implement that pragma, which costs nothing here: rounding mode and status
 * flags live in the machine's own registers, and a computation between two calls to `fegetround`
 * is not moved across one by anything this compiler does.
 */
#ifndef XCC_FENV_H
#define XCC_FENV_H

/* 7.6p3 to p5: the status flags, the rounding directions, and the union of the flags. The values
 * are the x87/SSE status word's own bits. */
#define FE_INVALID 0x10
#define FE_UNDERFLOW 0x2
#define FE_OVERFLOW 0x4
#define FE_DIVBYZERO 0x8
#define FE_INEXACT 0x1
#define FE_ALL_EXCEPT 0x1f

#define FE_TONEAREST 0
#define FE_DOWNWARD 0x100
#define FE_UPWARD 0x200
#define FE_TOWARDZERO 0x300

/* 7.6p2: the environment is the control word and the status word, eight bytes, which is what the
 * C runtime writes through a `fenv_t *`. A program may not look inside. */
typedef struct _xcc_fenv {
    unsigned int _control;
    unsigned int _status;
} fenv_t;

/* An exception flag or a group of them, in a form `fegetexceptflag` can hand back. */
typedef unsigned int fexcept_t;

int feclearexcept(int excepts);
int fegetexceptflag(fexcept_t *flag, int excepts);
int fegetenv(fenv_t *environment);
int fegetround(void);
int feholdexcept(fenv_t *environment);
int feraiseexcept(int excepts);
int fesetenv(const fenv_t *environment);
int fesetexceptflag(const fexcept_t *flag, int excepts);
int fesetround(int round);
int fetestexcept(int excepts);
int feupdateenv(const fenv_t *environment);

/* 7.6p6: the environment a program started in. It is a *pointer* to an object rather than a
 * sentinel: the runtime's `fesetenv` dereferences what it is given, so a `-1` would fault. The
 * object is filled in before `main` runs, by the same `fegetenv` a program can call itself - the
 * process starts in the default environment, so what is captured is the default. */
extern fenv_t _Fenv_default;
#define FE_DFL_ENV (&_Fenv_default)

#endif
