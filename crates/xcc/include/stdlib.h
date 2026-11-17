/* stdlib.h - C99 7.20, minus the parts this compiler cannot express (see stdarg.h for why the
 * variadic ones are absent). */
#ifndef XCC_STDLIB_H
#define XCC_STDLIB_H

#include <stddef.h>

#define EXIT_SUCCESS 0
#define EXIT_FAILURE 1
#define RAND_MAX 32767
#define MB_CUR_MAX 1

typedef struct {
    int quot;
    int rem;
} div_t;

typedef struct {
    long long quot;
    long long rem;
} lldiv_t;

double atof(const char *text);
int atoi(const char *text);
long long atoll(const char *text);
double strtod(const char *restrict text, char **restrict end);
long long strtoll(const char *restrict text, char **restrict end, int base);
unsigned long long strtoull(const char *restrict text, char **restrict end, int base);

int rand(void);
void srand(unsigned int seed);

void *malloc(size_t size);
void *calloc(size_t count, size_t size);
void *realloc(void *block, size_t size);
void free(void *block);

void exit(int status);
void abort(void);
int atexit(void (*function)(void));

char *getenv(const char *name);
int system(const char *command);

void *bsearch(const void *key, const void *base, size_t count, size_t size,
              int (*compare)(const void *, const void *));
void qsort(void *base, size_t count, size_t size,
           int (*compare)(const void *, const void *));

int abs(int value);
long long llabs(long long value);
div_t div(int numerator, int denominator);
lldiv_t lldiv(long long numerator, long long denominator);

#endif
