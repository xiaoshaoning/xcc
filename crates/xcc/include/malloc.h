/* malloc.h - the Microsoft header that holds the heap and, with it, `_alloca`.
 *
 * `stdlib.h` is where C puts the allocation functions; MSVC also keeps this header, and a program
 * that includes it is asking for `_alloca` as often as for `malloc`. `_alloca` has no library body -
 * it moves the caller's stack - so both spellings are macros over the compiler's own operation.
 */
#ifndef XCC_MALLOC_H
#define XCC_MALLOC_H

#include <stdlib.h>

#define _alloca(size) __xcc_alloca(size)
#define alloca(size) __xcc_alloca(size)

#endif
