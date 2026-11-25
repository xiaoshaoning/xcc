/* vadefs.h - what mingw-w64's headers ask about varargs, answered in this compiler's terms.
 *
 * Not a C99 header and not part of this compiler's runtime: a *bridge*. A system SDK has to know how
 * the compiler spells `va_list` before it can declare a variadic function, and mingw-w64's
 * `vadefs.h` decides by asking whether it is GCC, `clang`, or MSVC - and says `#error VARARGS not
 * implemented for this compiler` about anything else. The honest answer is neither of those three,
 * so this header answers the question the SDK is really asking: `va_list` is a `char *` on this
 * target (the Win64 ABI's own definition, which is also what MSVC spells), and the four
 * `_crt_va_*` names are this compiler's `stdarg.h` macros under the SDK's spelling.
 *
 * It shadows the SDK's own `vadefs.h`, which is why this file's include guard is that file's.
 */
#ifndef _INC_VADEFS
#define _INC_VADEFS

#include <stdarg.h>

/* `stdarg.h` above has defined `va_list` and the four macros. The SDK's names for them: */
#ifndef _VA_LIST_DEFINED
#define _VA_LIST_DEFINED
#endif

#define _crt_va_start(list, last) va_start(list, last)
#define _crt_va_arg(list, type) va_arg(list, type)
#define _crt_va_end(list) va_end(list)
#define _crt_va_copy(into, from) ((into) = (from))

/* `_ADDRESSOF(v)` is `&(v)`, and `__gnuc_va_list` is only ever named under `__GNUC__` - which this
 * compiler does not define, because it is not GCC and would then be offered `__builtin_va_list`,
 * statement expressions and the rest. */
#define _ADDRESSOF(v) (&(v))

#endif
