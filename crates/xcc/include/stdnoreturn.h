#ifndef __XCC_STDNORETURN_H
#define __XCC_STDNORETURN_H
/* stdnoreturn.h - C11 7.23: one name for a specifier this compiler reads and drops.
 *
 * The promise is the programmer's - that the function does not return - and control falling off the
 * end of one is their bug rather than the compiler's. `_Noreturn` is consumed like `inline`.
 */
#define noreturn _Noreturn

#endif
