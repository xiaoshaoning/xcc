/* errno.h - the error numbers C99 7.5 asks for.  Part of xcc.
 *
 * Clean room: the wording is C99 7.5 and nothing else.
 *
 * `errno` is not an object: the C library keeps one per thread, and reaches it through a function.
 * That is why it is a macro here - a program writes `errno = 0` and reads it back, and the
 * expansion is what makes that work.
 */
#ifndef XCC_ERRNO_H
#define XCC_ERRNO_H

/* The target's C library names the accessor `_errno`; both the universal and the older runtime
 * export it, so this is a declaration and not an implementation. */
int *_errno(void);
#define errno (*_errno())

/* The three 7.5p2 requires.  The values are the target C library's, because a program is entitled
 * to compare `errno` with them and to see the same number the library set. */
#define EDOM 33
#define EILSEQ 42
#define ERANGE 34

#endif
