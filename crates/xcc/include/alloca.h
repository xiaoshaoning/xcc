/* alloca.h - the POSIX spelling of "bytes off the stack, now".
 *
 * There is no function here because there cannot be one: an allocation that ends when the *caller*
 * returns changes the caller's stack, and a function cannot move its caller's stack pointer. So this
 * is the compiler's own operation, and the name is a macro over it - the same shape MSVC's `ctype.h`
 * gives `isascii`, and for the same reason: the header says what the platform exports, and the
 * compiler provides what no library can.
 */
#ifndef XCC_ALLOCA_H
#define XCC_ALLOCA_H

#define alloca(size) __xcc_alloca(size)

#endif
