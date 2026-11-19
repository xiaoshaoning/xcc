/* M8's acceptance item: a program that prints, compiled by xcc and linked against the C
 * runtime. Kept as a file rather than a string literal in the test, because a C program written
 * inside a Rust string is a C program with two sets of escapes in it, and the second set is a
 * mistake waiting to happen - which it was, twice.
 *
 * The floating-point arguments are the interesting ones: Win64 makes the caller write each variadic
 * `double` to a vector register *and* to the general register of its argument's position, and this
 * program is where that is checked against a real `printf` - one that reads the general registers,
 * as every variadic callee does - rather than against our own idea of the ABI (ADR-006). Only
 * `--opt-level=2` passes them; level 0 and 1 refuse the construct rather than print zeros.
 */
#include <stdio.h>
#include <string.h>

int main(void) {
    char line[32];
    int written = snprintf(line, sizeof line, "%s=%d", "answer", 42);
    printf("%s %d %c %x %s\n", line, written, 'Z', 255, line);
    printf("%f|%.2f|%e|%g|%d|%f\n", 1.5, 2.25, 1234.5, 0.5, 7, 100.125);
    fprintf(stderr, "%s\n", strcmp(line, "answer=42") == 0 ? "same" : "different");
    return written;
}
