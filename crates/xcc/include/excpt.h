// The exception-handler header, which the SDK does **not** ship.
//
// `<um/winnt.h>` writes `#include <excpt.h>` in the middle of itself, and Microsoft's copy of this
// header lives in the *toolset* (`VC/Tools/MSVC/<version>/include`), not in the Windows Kits. A
// machine with the Kits and no toolset therefore has no `excpt.h` for the SDK's own headers - and
// what it does have is mingw's, whose first act is `#include <crtdefs.h>` = `<corecrt.h>`, which in
// this include chain is the **SDK's** `corecrt.h` and defines neither `__MINGW_EXTENSION` nor
// `_CRTIMP`. Two C runtimes' headers, one translation unit: clang refuses that chain too.
//
// So this is a **bridge header**, read before the host's directories, and it is Microsoft's
// content: the disposition enumeration, the x86-64 handler prototype, and the accessor macros. The
// enumeration is a *type* rather than a `#define` (mingw spells it `int`), because that is what the
// SDK's `winnt.h` composes with - it uses `EXCEPTION_DISPOSITION` as the return type of a function
// it declares a few lines later.
#ifndef _INC_EXCPT
#define _INC_EXCPT

#include <vcruntime.h>

#pragma pack(push, _CRT_PACKING)

#ifdef __cplusplus
extern "C" {
#endif

#ifndef _EXCEPTION_DISPOSITION_DEFINED
#define _EXCEPTION_DISPOSITION_DEFINED
typedef enum _EXCEPTION_DISPOSITION {
    ExceptionContinueExecution = 0,
    ExceptionContinueSearch = 1,
    ExceptionNestedException = 2,
    ExceptionCollidedUnwind = 3
} EXCEPTION_DISPOSITION;
#endif

// The record and the context are declared by `winnt.h` either before this header is reached or
// after it, so they are incomplete here and only named.
struct _EXCEPTION_RECORD;
struct _CONTEXT;
struct _DISPATCHER_CONTEXT;

EXCEPTION_DISPOSITION __cdecl __C_specific_handler(struct _EXCEPTION_RECORD *_ExceptionRecord,
                                                   void *_EstablisherFrame,
                                                   struct _CONTEXT *_ContextRecord,
                                                   struct _DISPATCHER_CONTEXT *_DispatcherContext);

unsigned long __cdecl _exception_code(void);
void *__cdecl _exception_info(void);
int __cdecl _abnormal_termination(void);

#define GetExceptionCode _exception_code
#define exception_code _exception_code
#define GetExceptionInformation() ((struct _EXCEPTION_POINTERS *)_exception_info())
#define exception_info() ((struct _EXCEPTION_POINTERS *)_exception_info())
#define AbnormalTermination _abnormal_termination
#define abnormal_termination _abnormal_termination

#ifdef __cplusplus
}
#endif

#pragma pack(pop)

#endif // _INC_EXCPT
