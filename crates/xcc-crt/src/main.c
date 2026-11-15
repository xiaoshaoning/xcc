/* The C runtime for programs built by xcc (ADR-005).
 *
 * Windows starts a program at the entry point named in the PE header, not at `main`. This is
 * that entry point: it calls `main` and hands the result to the C library's `exit`, which
 * flushes stdio and runs at-exit handlers before calling `ExitProcess`. Calling `ExitProcess`
 * here instead would skip the flush - a program that printf's would print nothing and exit 0.
 *
 * It is written in C and compiled by xcc itself: the first program xcc links is also the first
 * it compiles from its own runtime, so every hello.exe is a regression test of the compiler.
 *
 * `argc`/`argv` are M6 (ADR-005): they need CommandLineToArgvW, WideCharToMultiByte, a heap and
 * Windows' own quoting rule, none of which belongs in the twelve lines that make a program run.
 */

void exit(int code);
int main(void);

void mainCRTStartup(void) {
    exit(main());
}
