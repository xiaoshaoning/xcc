/* malloc.h - the Microsoft header that holds the MSVC heap alongside C's own.
 *
 * `stdlib.h` is where C puts `malloc` and `free`; MSVC also keeps this header, and a program includes
 * it for two things this compiler's `stdlib.h` does not have: `_alloca`, and the *aligned* family.
 * `_alloca` has no library body - it moves the caller's stack - so it is a macro over the compiler's
 * own operation, and `alloca` is the POSIX spelling of the same thing.
 *
 * This header ships because MSVC programs write `#include <malloc.h>` and mean these names. What it
 * declares is the whole of what the UCRT exports under this header - a measurement, not a guess: the
 * names below are in `ucrtbase.dll`'s own export table, which a test asks for the rest of.
 */
#ifndef XCC_MALLOC_H
#define XCC_MALLOC_H

#include <stddef.h>
#include <stdlib.h>

#define _alloca(size) __xcc_alloca(size)
#define alloca(size) __xcc_alloca(size)

void *_aligned_malloc(size_t size, size_t alignment);
void *_aligned_realloc(void *block, size_t size, size_t alignment);
void _aligned_free(void *block);
size_t _aligned_msize(void *block, size_t alignment, size_t offset);
int _heapmin(void);

#endif
