#ifndef __XCC_STDALIGN_H
#define __XCC_STDALIGN_H
/* stdalign.h - C11 7.15: three names for what the compiler already has.
 *
 * The macros rather than keywords is what C11 chose, so a program written for this header also
 * compiles where the names are the language's own.
 */
#define alignas _Alignas
#define alignof _Alignof
#define __alignas_is_defined 1
#define __alignof_is_defined 1

#endif
