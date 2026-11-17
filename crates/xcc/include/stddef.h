/* stddef.h - the types C's own library is built on.  Part of xcc.
 *
 * Clean room: the wording is C99 7.17 and nothing else.  Every header here is a file the
 * preprocessor reads like any other, so what xcc accepts is what it can compile.
 */
#ifndef XCC_STDDEF_H
#define XCC_STDDEF_H

typedef unsigned long long size_t;
typedef long long ptrdiff_t;
/* The target has no `wchar_t` of its own: 16-bit, as Windows uses it. */
typedef unsigned short wchar_t;

#define NULL ((void *)0)

/* C99 7.17p3 defines `offsetof` as `(size_t)&(((type *)0)->member)`, which needs a
 * pointer-to-integer conversion and an address for a member of nothing.  The compiler knows the
 * layout, so it answers instead. */
#define offsetof(type, member) __builtin_offsetof(type, member)

#endif
