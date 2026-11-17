/* stdarg.h - refused, with a reason, rather than half-written.
 *
 * `va_list`, `va_start` and `va_arg` are not declarations: on Win64 they are the ABI's variadic
 * calling convention, and compiling one needs the register save area (`AL` says how many vector
 * registers were used) and a per-call-site signature. This back end can express neither, so a
 * program that uses them is refused where it uses them - see docs/ADR-006-varargs.md. A header
 * that declared them would only move the failure to a crash at run time.
 */
#error "xcc does not implement variadic functions yet: see docs/ADR-006-varargs.md"
#ifndef XCC_STDARG_H
#define XCC_STDARG_H
#endif
