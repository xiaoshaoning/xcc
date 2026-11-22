/* iso646.h - the spellings C99 7.9 asks for.  Part of xcc.
 *
 * Clean room: the wording is C99 7.9 and nothing else.
 *
 * Eleven macros and nothing else: a program written where `&`, `|` and `!` do not exist in the
 * character set spells them as words, and every implementation of the header is these macros.
 */
#ifndef XCC_ISO646_H
#define XCC_ISO646_H

#define and &&
#define and_eq &=
#define bitand &
#define bitor |
#define compl ~
#define not !
#define not_eq !=
#define or ||
#define or_eq |=
#define xor ^
#define xor_eq ^=

#endif
