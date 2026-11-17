/* time.h - C99 7.23, the pieces that do not need the clock's own types beyond `time_t`. */
#ifndef XCC_TIME_H
#define XCC_TIME_H

#include <stddef.h>

typedef long long time_t;
typedef long long clock_t;

#define CLOCKS_PER_SEC 1000

struct tm {
    int tm_sec;
    int tm_min;
    int tm_hour;
    int tm_mday;
    int tm_mon;
    int tm_year;
    int tm_wday;
    int tm_yday;
    int tm_isdst;
};

clock_t clock(void);
time_t time(time_t *at);
double difftime(time_t later, time_t earlier);
time_t mktime(struct tm *broken);
struct tm *localtime(const time_t *at);
struct tm *gmtime(const time_t *at);
size_t strftime(char *restrict to, size_t count, const char *restrict format,
                const struct tm *restrict broken);

#endif
