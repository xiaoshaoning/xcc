/* string.h - C99 7.21.  Declarations only: the functions are the C runtime's, which the linker
 * brings in.  A name here is a promise that the target's library has it. */
#ifndef XCC_STRING_H
#define XCC_STRING_H

#include <stddef.h>

void *memcpy(void *restrict to, const void *restrict from, size_t count);
void *memmove(void *to, const void *from, size_t count);
void *memset(void *to, int value, size_t count);
int memcmp(const void *left, const void *right, size_t count);
void *memchr(const void *haystack, int needle, size_t count);

char *strcpy(char *restrict to, const char *restrict from);
char *strncpy(char *restrict to, const char *restrict from, size_t count);
char *strcat(char *restrict to, const char *restrict from);
char *strncat(char *restrict to, const char *restrict from, size_t count);
int strcmp(const char *left, const char *right);
int strncmp(const char *left, const char *right, size_t count);

/* Case-insensitive comparison: not C99, and what a Windows program calls. The underscore is the
   SDK's spelling - the UCRT exports these, and the SDK's own `stralign.h` is written in them. */
int _stricmp(const char *left, const char *right);
int _strnicmp(const char *left, const char *right, size_t count);
int strcoll(const char *left, const char *right);
size_t strxfrm(char *restrict to, const char *restrict from, size_t count);
char *strchr(const char *text, int needle);
char *strrchr(const char *text, int needle);
size_t strcspn(const char *text, const char *reject);
size_t strspn(const char *text, const char *accept);
char *strpbrk(const char *text, const char *accept);
char *strstr(const char *haystack, const char *needle);
char *strtok(char *restrict text, const char *restrict separators);
size_t strlen(const char *text);
char *strerror(int code);

#endif
