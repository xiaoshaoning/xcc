/* setjmp.h - the non-local jump C99 7.13 asks for.  Part of xcc.
 *
 * Clean room: the wording is C99 7.13 and nothing else.  The *layout* of `jmp_buf` is not the
 * standard's to give: it is the ABI's, because `setjmp` and `longjmp` are the C library's own
 * functions writing into and reading from these bytes.  That is the same kind of fact as `va_list` in
 * ADR-006 - written once, where the ABI says it.
 *
 * x86-64 Windows, 256 bytes, 16-byte aligned: eleven callee-saved integers (Frame, Rbx, Rsp, Rbp,
 * Rsi, Rdi, R12-R15, Rip), the two control words, and the ten saved XMM registers.  The alignment is
 * not decoration - the C library's `setjmp` saves those XMM registers with `movaps`, which faults on
 * an address that is not 16-byte aligned.  C99 has no alignment specifier at all, so the member is
 * declared with this compiler's `__builtin_alignas`, which is what every real C library's headers do
 * with `_Alignas`/`__declspec(align)`/`__attribute__((aligned))`.
 */
#ifndef XCC_SETJMP_H
#define XCC_SETJMP_H

typedef struct _xcc_jump_unit {
    unsigned long long part[2];
} _xcc_jump_unit;

typedef struct _xcc_jump_buffer {
    unsigned long long frame;
    unsigned long long rbx;
    unsigned long long rsp;
    unsigned long long rbp;
    unsigned long long rsi;
    unsigned long long rdi;
    unsigned long long r12;
    unsigned long long r13;
    unsigned long long r14;
    unsigned long long r15;
    unsigned long long rip;
    unsigned long mxcsr;
    unsigned short fpcsr;
    unsigned short spare;
    __builtin_alignas(16) _xcc_jump_unit xmm[10];
} jmp_buf[1];

/* 7.13p2: `longjmp` never returns, and `setjmp` returns twice - which this compiler cannot see, so a
 * program that reads a local written between them is on its own (7.13.2p3).
 *
 * 7.13p1 calls `setjmp` a *macro*, and it has to be one here: this ABI's unwinder is passed the frame
 * it should stop at, and the C library's entry point takes that as its *second* argument - a null
 * there means "this frame", which is what a `setjmp` call at the top of a function wants. Measured:
 * dropping the argument to make the call look like C's one-parameter `setjmp` compiles, links, and
 * then faults inside `RtlUnwindEx`, because the unwinder is handed whatever the second register
 * happened to hold. `longjmp` is a function - its name is the one the C library exports. */
int _setjmp(jmp_buf environment, void *frame);
void longjmp(jmp_buf environment, int value);
#define setjmp(environment) _setjmp((environment), 0)

#endif
