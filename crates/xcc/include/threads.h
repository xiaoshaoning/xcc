/* threads.h - C11 7.26.
 *
 * Threads, mutexes, condition variables, thread-specific storage and one-time initialisation, over
 * what this target already has: `_beginthreadex` for a thread (the C runtime's own entry point, so
 * the per-thread runtime state is set up), the Win32 critical section for a mutex, and the Win32
 * condition variable for `cnd_t`. The bodies are in `crates/xcc-crt/src/main.c`.
 *
 * Two representations are this implementation's, which 7.26 allows: a mutex is a critical section
 * (40 bytes on x64: two pointers, two longs, a handle, a pointer and a spin count) and a condition
 * variable is one pointer. They are written as byte arrays here so the header does not need
 * `windows.h` - the sizes are measured, and the runtime source asserts them.
 *
 * The three things this header does not do, each for a reason and not by omission:
 *   - `mtx_timed`: a critical section cannot be entered with a timeout. `mtx_init` returns
 *     `thrd_error` for it rather than handing back a mutex that would block forever.
 *   - a `tss_t` destructor is not run when a thread ends: Windows' own thread-local storage has no
 *     notion of one, and running them would need a registry and a hook in every thread's exit.
 *   - `cnd_timedwait`'s timeout is a whole number of milliseconds, because that is the resolution
 *     the call underneath has.
 */
#ifndef __XCC_THREADS_H
#define __XCC_THREADS_H

#include <time.h>

/* 7.26.1: the two macros and the result codes. `thread_local` is the storage class this compiler
 * implements for `_Atomic`-free thread storage (C11 6.7.3). */
#define thread_local _Thread_local

enum {
    thrd_success,
    thrd_nomem,
    thrd_timedout,
    thrd_busy,
    thrd_error
};

/* 7.26.1.2: a thread identifier. A handle and the id the system knows it by: the handle is what a
 * join waits on, and the id is what `thrd_equal` compares - a handle is closed by a join or a
 * detach, and comparing dead handles would be comparing nothing. */
typedef struct {
    void *handle;
    unsigned long id;
} thrd_t;

typedef int (*thrd_start_t)(void *);

int thrd_create(thrd_t *thread, thrd_start_t start, void *argument);
int thrd_equal(thrd_t left, thrd_t right);
thrd_t thrd_current(void);
int thrd_sleep(const struct timespec *duration, struct timespec *remaining);
void thrd_yield(void);
void thrd_exit(int result);
int thrd_detach(thrd_t thread);
int thrd_join(thrd_t thread, int *result);

/* 7.26.4: the four kinds of mutex. Two are supported, and `mtx_timed` is refused at `mtx_init`. */
typedef enum { mtx_plain = 0, mtx_recursive = 1, mtx_timed = 2 } mtx_type;

/* A critical section, which is 40 bytes on this target and is recursive by nature: `mtx_plain` and
 * `mtx_recursive` are therefore the same object, and the type says which the caller asked for. */
typedef struct {
    unsigned long long opaque[5];
} mtx_t;

int mtx_init(mtx_t *mutex, int type);
int mtx_lock(mtx_t *mutex);
int mtx_trylock(mtx_t *mutex);
int mtx_unlock(mtx_t *mutex);
void mtx_destroy(mtx_t *mutex);

/* 7.26.3: a condition variable, one pointer. */
typedef struct {
    unsigned long long opaque[1];
} cnd_t;

int cnd_init(cnd_t *condition);
int cnd_signal(cnd_t *condition);
int cnd_broadcast(cnd_t *condition);
int cnd_wait(cnd_t *condition, mtx_t *mutex);
int cnd_timedwait(cnd_t *condition, mtx_t *mutex, const struct timespec *absolute);
void cnd_destroy(cnd_t *condition);

/* 7.26.6: thread-specific storage, which is the runtime's own thread-local slot. */
typedef unsigned long tss_t;
typedef void (*tss_dtor_t)(void *);

#define TSS_DTOR_ITERATIONS 4

int tss_create(tss_t *key, tss_dtor_t destructor);
void *tss_get(tss_t key);
int tss_set(tss_t key, void *value);
void tss_delete(tss_t key);

/* 7.26.2: one-time initialisation. One int, and an atomic swap of it. */
typedef struct {
    int state;
} once_flag;
typedef void (*once_callback)(void);

#define ONCE_FLAG_INIT { 0 }

void call_once(once_flag *flag, once_callback callback);

#endif
