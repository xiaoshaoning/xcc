/* xcc - a small, embeddable C compiler.  The C API.
 *
 * A host keeps an `xcc *` and never looks inside it.  A handle is used by one thread at a time;
 * two handles run on two threads, as the compiler has no global state.
 *
 *   xcc *c = xcc_new(NULL, 0);
 *   if (xcc_add_source(c, "int add(int a, int b) { return a + b; }") != 0)
 *       fprintf(stderr, "%s\n", xcc_last_error(c));
 *   else {
 *       int (*add)(int, int) = (int (*)(int, int))xcc_function(c, "add");
 *       if (add) printf("%d\n", add(2, 3));
 *   }
 *   xcc_free(c);
 *
 * There is no separate compile step: xcc_add_source compiles what it is given.  Each source is a
 * translation unit of its own, so one that calls another's function declares it first, exactly as
 * one .c file declares what another defines.
 */
#ifndef XCC_H
#define XCC_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* One compiler.  Opaque. */
typedef struct xcc xcc;

/* A function the compiled code may call, by name. */
typedef struct xcc_symbol {
    const char *name;   /* the name the C source uses */
    const void *address; /* the address C would give it */
} xcc_symbol;

/* Make a compiler that may call `count` host symbols.  Never NULL: a symbol list this cannot read
 * leaves a handle whose first xcc_add_source refuses, with the reason in xcc_last_error. */
xcc *xcc_new(const xcc_symbol *symbols, size_t count);

/* Compile `source`, adding its functions to the ones already in `compiler`.  0 when every stage
 * was clean, -1 otherwise with every error in xcc_last_error, one per line. */
int xcc_add_source(xcc *compiler, const char *source);

/* The address of `name`, or NULL with the reason in xcc_last_error.  Calling it through the wrong
 * signature is C's oldest problem, and this API cannot check it. */
void *xcc_function(xcc *compiler, const char *name);

/* What the last call failed with: empty when it succeeded, never NULL.  The pointer stays valid
 * until the next call on the same handle. */
const char *xcc_last_error(xcc *compiler);

/* Free the compiler and everything it compiled.  NULL is refused, not freed. */
void xcc_free(xcc *compiler);

#ifdef __cplusplus
}
#endif

#endif /* XCC_H */
