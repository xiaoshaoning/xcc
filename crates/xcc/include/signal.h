/* signal.h - the signal handling C99 7.14 asks for.  Part of xcc.
 *
 * Clean room: the wording is C99 7.14 and nothing else.
 *
 * A signal handler is a function the C library calls, so this header is declarations and the
 * target's own signal numbers - the values matter, because a program is entitled to compare what
 * `signal` returns with them.
 */
#ifndef XCC_SIGNAL_H
#define XCC_SIGNAL_H

/* 7.14p2: the type of an object a handler may assign to and the interrupted code may read. */
typedef int sig_atomic_t;

/* 7.14p1: the values `signal` returns to say what it did, and the actions it takes. */
#define SIG_DFL ((void (*)(int))0)
#define SIG_IGN ((void (*)(int))1)
#define SIG_ERR ((void (*)(int))-1)

/* 7.14p3: the signals every implementation has. The numbers are the target C library's. */
#define SIGINT 2
#define SIGILL 4
#define SIGFPE 8
#define SIGSEGV 11
#define SIGTERM 15
#define SIGABRT 22

/* 7.14p1: `signal` returns the handler it replaced, which is why its type names one. */
void (*signal(int signal_number, void (*handler)(int)))(int);
int raise(int signal_number);

#endif
