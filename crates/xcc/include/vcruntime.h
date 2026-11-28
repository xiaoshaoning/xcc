/* vcruntime.h - the little of the MSVC toolset's header that the Windows SDK's headers need.
 *
 * This is a **fallback**, not a bridge: it is read only when neither this compiler's own headers nor
 * the host's include directories have `vcruntime.h`, so a machine with Visual Studio installed gets
 * Microsoft's file, with everything in it, and nothing here is ever shadowed.
 *
 * Why it exists: the SDK's `corecrt.h` opens with `#include <vcruntime.h>`, and that header belongs
 * to the *toolset* rather than to the SDK. A machine with the Windows SDK and no Visual Studio can
 * therefore find `windows.h` and not compile it - which is exactly what CI found, with the SDK
 * fixtures reporting "unknown `#pragma region`" and then failing on the missing header.
 *
 * The content is the minimum those headers use: the calling-convention and `__declspec` spellings
 * they expect to exist, and the two types `corecrt.h` needs before it can define anything. It
 * declares nothing the UCRT does not export, and it is deliberately small: the more it declared, the
 * more it could get wrong that Microsoft's own would have got right.
 */
#ifndef _VCRUNTIME_H
#define _VCRUNTIME_H

#include <vadefs.h>

/* `corecrt.h` guards its declarations with this pair, which is how the toolset's header tells a C
 * compiler from a C++ one. */
#ifdef __cplusplus
#define _CRT_BEGIN_C_HEADER extern "C" {
#define _CRT_END_C_HEADER }
#else
#define _CRT_BEGIN_C_HEADER
#define _CRT_END_C_HEADER
#endif

/* The types `corecrt.h` uses before it defines anything of its own. */
#ifndef _SIZE_T_DEFINED
#define _SIZE_T_DEFINED
typedef unsigned __int64 size_t;
#endif
#ifndef _WCHAR_T_DEFINED
#define _WCHAR_T_DEFINED
typedef unsigned short wchar_t;
#endif

/* The alignment the SDK's headers lay their structures out with. Microsoft's `vcruntime.h` defines
 * it - `safeint.h` and the exception records write `#pragma pack(push, _CRT_PACKING)` - and it is
 * not in the Windows SDK, which is why a machine with the SDK and no toolset could not compile
 * `excpt.h`. Eight is the value MSVC gives it: the packing of every structure the SDK declares. */
#define _CRT_PACKING 8

/* A null pointer constant and the two `__pragma` spellings the SDK's headers write. `__pragma` is
 * MSVC's, and this compiler reads `#pragma` at line scope only - `__pragma` inside a macro is
 * dropped, which is what an attribute-style pragma is for. */
#ifndef NULL
#define NULL ((void *)0)
#endif
#define __pragma(...) __xcc_pragma(__VA_ARGS__)

#endif
