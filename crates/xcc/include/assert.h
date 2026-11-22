/* assert.h - the diagnostic C99 7.2 asks for.  Part of xcc.
 *
 * Clean room: the wording is C99 7.2 and nothing else.
 *
 * `assert` is a macro because the standard makes it one: the message has to name the *failing
 * expression's* file and line, which only the preprocessor knows.  The diagnostic and the abort
 * come from the C library, so this header needs no runtime of its own - which is why it can be a
 * header at all here, where `setjmp.h` cannot.
 */
#ifndef XCC_ASSERT_H
#define XCC_ASSERT_H

#include <stdio.h>
#include <stdlib.h>

/* 7.2p1: with `NDEBUG` defined, `assert` expands to an expression of type void that does nothing.
 * Every occurrence is the same shape either way, so `assert(...)` may sit where a statement or an
 * expression may, and a program cannot tell which form it got except by the diagnostic. */
#ifdef NDEBUG
#define assert(condition) ((void)0)
#else
/* The comma operator rather than a `do { } while (0)`: it is one expression, so `assert` works in
 * an `if` without braces, and in a comma expression, and it never needs a trailing semicolon of its
 * own.  `(void)` says the value is not used. */
#define assert(condition)                                                     \
    ((condition) ? (void)0                                                    \
                 : (void)(fprintf(stderr, "%s:%d: assertion `%s' failed\n",   \
                                  __FILE__, __LINE__, #condition),             \
                          abort()))
#endif

#endif
