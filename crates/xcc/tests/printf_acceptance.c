/* M8's acceptance item: a program that prints, compiled by xcc and linked against the C
 * runtime. Kept as a file rather than a string literal in the test, because a C program written
 * inside a Rust string is a C program with two sets of escapes in it, and the second set is a
 * mistake waiting to happen - which it was, twice.
 *
 * The variadic calls here pass integers, characters, strings and pointers only: a floating-point
 * argument to a variadic function needs `AL` set to the vector-register count, and the back end
 * cannot write a physical register (docs/ADR-006-varargs.md). `tests/parse.rs` pins the refusal.
 */
#include <stdio.h>
#include <string.h>

int main(void) {
    char line[32];
    int written = snprintf(line, sizeof line, "%s=%d", "answer", 42);
    printf("%s %d %c %x %s\n", line, written, 'Z', 255, line);
    fprintf(stderr, "%s\n", strcmp(line, "answer=42") == 0 ? "same" : "different");
    return written;
}
