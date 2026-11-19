/* stdarg.h - the three macros C99 7.15 requires, over this compiler's own back end.
 *
 * A `va_list` is a `char *` that walks the argument slots: Win64 puts the first four arguments in
 * registers and the rest on the stack, and a variadic function's prologue writes the registers into
 * the caller's home area, which is directly below the stack arguments - so one pointer crosses both.
 *
 * `va_arg` needs the *type* its argument was passed with, which is why it is a builtin rather than a
 * function: the size of the slot is the same for every type, but the width and the sign of the read
 * are not. The three macros name the builtins and nothing else, so the diagnostics come from the
 * compiler and point at the source line.
 *
 * Cranelift, the back end behind --opt-level=0 and 1, has no home area to point at and refuses these
 * with a span; see docs/ADR-006-varargs.md.
 */
#ifndef XCC_STDARG_H
#define XCC_STDARG_H

typedef char *va_list;

#define va_start(list, last) __builtin_va_start(list, last)
#define va_arg(list, type)   __builtin_va_arg(list, type)
#define va_end(list)         ((void)0)

#endif
