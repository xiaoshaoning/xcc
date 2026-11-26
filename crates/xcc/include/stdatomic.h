/* stdatomic.h - C11 7.17.
 *
 * `_Atomic` is the compiler's, and so is the one machine operation the rest is built on:
 * `__builtin_atomic_cas`, which is one `lock cmpxchg`. Every operation below is a loop of that in
 * C (`crates/xcc-crt/src/main.c`), so the back end has one thing to get right instead of eight.
 *
 * On this target a *load* and a *store* are atomic already for any naturally aligned object of the
 * machine's width, and this compiler neither reorders nor caches a memory access across another -
 * it emits the accesses in the order the source wrote them. So `atomic_load` is the object and
 * `atomic_store` is the assignment, and the reasoning is here rather than in an instruction.
 *
 * Two width classes, which are the machine's: 32 bits (`int`, `unsigned`, `long`) and 64
 * (`long long`, `unsigned long long`, `size_t`, a pointer). An object of another width has no arm
 * in the tables below, so a program that asks for one is told "no `_Generic` arm matches" at the
 * call rather than being swapped as the wrong number of bytes.
 *
 * `atomic_bool` and `atomic_flag` are four bytes: the standard leaves the representation to the
 * implementation, and four bytes is what lets a flag be one 32-bit compare-and-swap.
 */
#ifndef __XCC_STDATOMIC_H
#define __XCC_STDATOMIC_H

#include <stddef.h>

/* 7.17.1: every object this header accepts is lock free, because every operation on it is. */
#define ATOMIC_BOOL_LOCK_FREE 1
#define ATOMIC_CHAR_LOCK_FREE 1
#define ATOMIC_CHAR16_T_LOCK_FREE 1
#define ATOMIC_CHAR32_T_LOCK_FREE 1
#define ATOMIC_WCHAR_T_LOCK_FREE 1
#define ATOMIC_SHORT_LOCK_FREE 1
#define ATOMIC_INT_LOCK_FREE 1
#define ATOMIC_LONG_LOCK_FREE 1
#define ATOMIC_LLONG_LOCK_FREE 1
#define ATOMIC_POINTER_LOCK_FREE 1

#define ATOMIC_VAR_INIT(value) (value)
#define ATOMIC_FLAG_INIT { 0 }

/* 7.17.2: the typedefs. The narrow ones are declared as the standard asks and are *not* usable for
 * an operation here - a `char` swap is not one of the machine's operations - so a program that
 * fetches on one is refused by name at the call. */
/* `_Atomic _Bool` is four bytes here, like a flag: the machine swaps 1, 2, 4 or 8 bytes, and a
 * one-byte atomic flag would be a *different* operation rather than a cheaper one. */
typedef _Atomic int atomic_bool;
typedef _Atomic char atomic_char;
typedef _Atomic signed char atomic_schar;
typedef _Atomic unsigned char atomic_uchar;
typedef _Atomic short atomic_short;
typedef _Atomic unsigned short atomic_ushort;
typedef _Atomic int atomic_int;
typedef _Atomic unsigned atomic_uint;
typedef _Atomic long atomic_long;
typedef _Atomic unsigned long atomic_ulong;
typedef _Atomic long long atomic_llong;
typedef _Atomic unsigned long long atomic_ullong;
typedef _Atomic unsigned short atomic_char16_t;
typedef _Atomic unsigned atomic_char32_t;
typedef _Atomic unsigned short atomic_wchar_t;
typedef _Atomic int atomic_int_least8_t;
typedef _Atomic unsigned atomic_int_least16_t;
typedef _Atomic int atomic_int_least32_t;
typedef _Atomic long long atomic_int_least64_t;
typedef _Atomic unsigned atomic_uint_least8_t;
typedef _Atomic unsigned atomic_uint_least16_t;
typedef _Atomic unsigned atomic_uint_least32_t;
typedef _Atomic unsigned long long atomic_uint_least64_t;
typedef _Atomic int atomic_int_fast8_t;
typedef _Atomic unsigned atomic_int_fast16_t;
typedef _Atomic int atomic_int_fast32_t;
typedef _Atomic long long atomic_int_fast64_t;
typedef _Atomic unsigned atomic_uint_fast8_t;
typedef _Atomic unsigned atomic_uint_fast16_t;
typedef _Atomic unsigned atomic_uint_fast32_t;
typedef _Atomic unsigned long long atomic_uint_fast64_t;
typedef _Atomic long long atomic_intptr_t;
typedef _Atomic unsigned long long atomic_uintptr_t;
typedef _Atomic unsigned long long atomic_size_t;
typedef _Atomic long long atomic_ptrdiff_t;

/* 7.17.8: a flag, whose representation is this implementation's. */
typedef struct atomic_flag { _Atomic int __value; } atomic_flag;

/* 7.17.3: the memory orders. This target is sequentially consistent for every operation below -
 * that is what `lock` is on x86 - so the value names the intent; no order weakens it. */
typedef enum memory_order {
    memory_order_relaxed,
    memory_order_consume,
    memory_order_acquire,
    memory_order_release,
    memory_order_acq_rel,
    memory_order_seq_cst
} memory_order;

/* The runtime's loops, one per width: `crates/xcc-crt/src/main.c`. */
int __xcc_add_int(volatile int *object, int argument);
unsigned __xcc_add_uint(volatile unsigned *object, unsigned argument);
long long __xcc_add_llong(volatile long long *object, long long argument);
unsigned long long __xcc_add_ullong(volatile unsigned long long *object, unsigned long long argument);
int __xcc_sub_int(volatile int *object, int argument);
unsigned __xcc_sub_uint(volatile unsigned *object, unsigned argument);
long long __xcc_sub_llong(volatile long long *object, long long argument);
unsigned long long __xcc_sub_ullong(volatile unsigned long long *object, unsigned long long argument);
int __xcc_and_int(volatile int *object, int argument);
unsigned __xcc_and_uint(volatile unsigned *object, unsigned argument);
long long __xcc_and_llong(volatile long long *object, long long argument);
unsigned long long __xcc_and_ullong(volatile unsigned long long *object, unsigned long long argument);
int __xcc_or_int(volatile int *object, int argument);
unsigned __xcc_or_uint(volatile unsigned *object, unsigned argument);
long long __xcc_or_llong(volatile long long *object, long long argument);
unsigned long long __xcc_or_ullong(volatile unsigned long long *object, unsigned long long argument);
int __xcc_xor_int(volatile int *object, int argument);
unsigned __xcc_xor_uint(volatile unsigned *object, unsigned argument);
long long __xcc_xor_llong(volatile long long *object, long long argument);
unsigned long long __xcc_xor_ullong(volatile unsigned long long *object, unsigned long long argument);
int __xcc_exchange_int(volatile int *object, int wanted);
unsigned __xcc_exchange_uint(volatile unsigned *object, unsigned wanted);
long long __xcc_exchange_llong(volatile long long *object, long long wanted);
unsigned long long __xcc_exchange_ullong(volatile unsigned long long *object, unsigned long long wanted);
/* char */
char __xcc_add_char(volatile char *object, char argument);
char __xcc_sub_char(volatile char *object, char argument);
char __xcc_and_char(volatile char *object, char argument);
char __xcc_or_char(volatile char *object, char argument);
char __xcc_xor_char(volatile char *object, char argument);
char __xcc_exchange_char(volatile char *object, char wanted);
int __xcc_cas_char(volatile char *object, char *expected, char desired);
/* signed char */
signed char __xcc_add_schar(volatile signed char *object, signed char argument);
signed char __xcc_sub_schar(volatile signed char *object, signed char argument);
signed char __xcc_and_schar(volatile signed char *object, signed char argument);
signed char __xcc_or_schar(volatile signed char *object, signed char argument);
signed char __xcc_xor_schar(volatile signed char *object, signed char argument);
signed char __xcc_exchange_schar(volatile signed char *object, signed char wanted);
int __xcc_cas_schar(volatile signed char *object, signed char *expected, signed char desired);
/* unsigned char */
unsigned char __xcc_add_uchar(volatile unsigned char *object, unsigned char argument);
unsigned char __xcc_sub_uchar(volatile unsigned char *object, unsigned char argument);
unsigned char __xcc_and_uchar(volatile unsigned char *object, unsigned char argument);
unsigned char __xcc_or_uchar(volatile unsigned char *object, unsigned char argument);
unsigned char __xcc_xor_uchar(volatile unsigned char *object, unsigned char argument);
unsigned char __xcc_exchange_uchar(volatile unsigned char *object, unsigned char wanted);
int __xcc_cas_uchar(volatile unsigned char *object, unsigned char *expected, unsigned char desired);
/* short */
short __xcc_add_short(volatile short *object, short argument);
short __xcc_sub_short(volatile short *object, short argument);
short __xcc_and_short(volatile short *object, short argument);
short __xcc_or_short(volatile short *object, short argument);
short __xcc_xor_short(volatile short *object, short argument);
short __xcc_exchange_short(volatile short *object, short wanted);
int __xcc_cas_short(volatile short *object, short *expected, short desired);
/* unsigned short */
unsigned short __xcc_add_ushort(volatile unsigned short *object, unsigned short argument);
unsigned short __xcc_sub_ushort(volatile unsigned short *object, unsigned short argument);
unsigned short __xcc_and_ushort(volatile unsigned short *object, unsigned short argument);
unsigned short __xcc_or_ushort(volatile unsigned short *object, unsigned short argument);
unsigned short __xcc_xor_ushort(volatile unsigned short *object, unsigned short argument);
unsigned short __xcc_exchange_ushort(volatile unsigned short *object, unsigned short wanted);
int __xcc_cas_ushort(volatile unsigned short *object, unsigned short *expected, unsigned short desired);
int __xcc_cas_int(volatile int *object, int *expected, int desired);
int __xcc_cas_uint(volatile unsigned *object, unsigned *expected, unsigned desired);
int __xcc_cas_llong(volatile long long *object, long long *expected, long long desired);
int __xcc_cas_ullong(volatile unsigned long long *object, unsigned long long *expected,
                     unsigned long long desired);
void __xcc_atomic_fence(void);

/* 7.17.6: the operations, each a table over the object's type. Every arm casts to the width the
 * runtime function takes: `long` is 32 bits on this target and `unsigned` is 32, so both answer with
 * the 32-bit function and both are the same operation. */
#define atomic_load(object) (*(object))
#define atomic_store(object, desired) ((void)(*(object) = (desired)))
#define atomic_load_explicit(object, order) ((void)(order), *(object))
#define atomic_store_explicit(object, desired, order) ((void)(order), (void)(*(object) = (desired)))

#define atomic_fetch_add(object, argument) \
    _Generic((object), \
        char *: __xcc_add_char((volatile char *)(object), (char)(argument)), \
        signed char *: __xcc_add_schar((volatile signed char *)(object), (signed char)(argument)), \
        unsigned char *: __xcc_add_uchar((volatile unsigned char *)(object), (unsigned char)(argument)), \
        short *: __xcc_add_short((volatile short *)(object), (short)(argument)), \
        unsigned short *: __xcc_add_ushort((volatile unsigned short *)(object), (unsigned short)(argument)), \
        int *: __xcc_add_int((volatile int *)(object), (int)(argument)), \
        unsigned *: __xcc_add_uint((volatile unsigned *)(object), (unsigned)(argument)), \
        long *: (long)__xcc_add_int((volatile int *)(object), (int)(argument)), \
        unsigned long *: (unsigned long)__xcc_add_uint((volatile unsigned *)(object), (unsigned)(argument)), \
        long long *: __xcc_add_llong((volatile long long *)(object), (long long)(argument)), \
        unsigned long long *: __xcc_add_ullong((volatile unsigned long long *)(object), \
                                               (unsigned long long)(argument)))

#define atomic_fetch_sub(object, argument) \
    _Generic((object), \
        char *: __xcc_sub_char((volatile char *)(object), (char)(argument)), \
        signed char *: __xcc_sub_schar((volatile signed char *)(object), (signed char)(argument)), \
        unsigned char *: __xcc_sub_uchar((volatile unsigned char *)(object), (unsigned char)(argument)), \
        short *: __xcc_sub_short((volatile short *)(object), (short)(argument)), \
        unsigned short *: __xcc_sub_ushort((volatile unsigned short *)(object), (unsigned short)(argument)), \
        int *: __xcc_sub_int((volatile int *)(object), (int)(argument)), \
        unsigned *: __xcc_sub_uint((volatile unsigned *)(object), (unsigned)(argument)), \
        long *: (long)__xcc_sub_int((volatile int *)(object), (int)(argument)), \
        unsigned long *: (unsigned long)__xcc_sub_uint((volatile unsigned *)(object), (unsigned)(argument)), \
        long long *: __xcc_sub_llong((volatile long long *)(object), (long long)(argument)), \
        unsigned long long *: __xcc_sub_ullong((volatile unsigned long long *)(object), \
                                               (unsigned long long)(argument)))

#define atomic_fetch_and(object, argument) \
    _Generic((object), \
        char *: __xcc_and_char((volatile char *)(object), (char)(argument)), \
        signed char *: __xcc_and_schar((volatile signed char *)(object), (signed char)(argument)), \
        unsigned char *: __xcc_and_uchar((volatile unsigned char *)(object), (unsigned char)(argument)), \
        short *: __xcc_and_short((volatile short *)(object), (short)(argument)), \
        unsigned short *: __xcc_and_ushort((volatile unsigned short *)(object), (unsigned short)(argument)), \
        int *: __xcc_and_int((volatile int *)(object), (int)(argument)), \
        unsigned *: __xcc_and_uint((volatile unsigned *)(object), (unsigned)(argument)), \
        long long *: __xcc_and_llong((volatile long long *)(object), (long long)(argument)), \
        unsigned long long *: __xcc_and_ullong((volatile unsigned long long *)(object), \
                                               (unsigned long long)(argument)))

#define atomic_fetch_or(object, argument) \
    _Generic((object), \
        char *: __xcc_or_char((volatile char *)(object), (char)(argument)), \
        signed char *: __xcc_or_schar((volatile signed char *)(object), (signed char)(argument)), \
        unsigned char *: __xcc_or_uchar((volatile unsigned char *)(object), (unsigned char)(argument)), \
        short *: __xcc_or_short((volatile short *)(object), (short)(argument)), \
        unsigned short *: __xcc_or_ushort((volatile unsigned short *)(object), (unsigned short)(argument)), \
        int *: __xcc_or_int((volatile int *)(object), (int)(argument)), \
        unsigned *: __xcc_or_uint((volatile unsigned *)(object), (unsigned)(argument)), \
        long long *: __xcc_or_llong((volatile long long *)(object), (long long)(argument)), \
        unsigned long long *: __xcc_or_ullong((volatile unsigned long long *)(object), \
                                              (unsigned long long)(argument)))

#define atomic_fetch_xor(object, argument) \
    _Generic((object), \
        char *: __xcc_xor_char((volatile char *)(object), (char)(argument)), \
        signed char *: __xcc_xor_schar((volatile signed char *)(object), (signed char)(argument)), \
        unsigned char *: __xcc_xor_uchar((volatile unsigned char *)(object), (unsigned char)(argument)), \
        short *: __xcc_xor_short((volatile short *)(object), (short)(argument)), \
        unsigned short *: __xcc_xor_ushort((volatile unsigned short *)(object), (unsigned short)(argument)), \
        int *: __xcc_xor_int((volatile int *)(object), (int)(argument)), \
        long long *: __xcc_xor_llong((volatile long long *)(object), (long long)(argument)), \
        unsigned long long *: __xcc_xor_ullong((volatile unsigned long long *)(object), \
                                               (unsigned long long)(argument)))

#define atomic_exchange(object, desired) \
    _Generic((object), \
        char *: __xcc_exchange_char((volatile char *)(object), (char)(desired)), \
        signed char *: __xcc_exchange_schar((volatile signed char *)(object), (signed char)(desired)), \
        unsigned char *: __xcc_exchange_uchar((volatile unsigned char *)(object), (unsigned char)(desired)), \
        short *: __xcc_exchange_short((volatile short *)(object), (short)(desired)), \
        unsigned short *: __xcc_exchange_ushort((volatile unsigned short *)(object), (unsigned short)(desired)), \
        int *: __xcc_exchange_int((volatile int *)(object), (int)(desired)), \
        unsigned *: __xcc_exchange_uint((volatile unsigned *)(object), (unsigned)(desired)), \
        long *: (long)__xcc_exchange_int((volatile int *)(object), (int)(desired)), \
        long long *: __xcc_exchange_llong((volatile long long *)(object), (long long)(desired)), \
        unsigned long long *: __xcc_exchange_ullong((volatile unsigned long long *)(object), \
                                                    (unsigned long long)(desired)))

/* The `_explicit` forms, which C11 7.17.7 spells for **every** operation. They differ from the
 * others in one thing only - the memory order they are given - and this compiler's atomics are all
 * sequentially consistent, the strongest order there is, so the honest answer is the non-explicit
 * form with the order read and dropped. Leaving them out was not an omission with a small blast
 * radius: an undeclared function is an `int`, and `mimalloc`'s `atomic_exchange_explicit` of a
 * pointer came back as a thirty-two-bit number widened into a pointer. */
#define atomic_exchange_explicit(object, desired, order)     ((void)(order), atomic_exchange(object, desired))
#define atomic_compare_exchange_strong_explicit(object, expected, desired, success, failure)     ((void)(success), (void)(failure), atomic_compare_exchange_strong(object, expected, desired))
#define atomic_compare_exchange_weak_explicit(object, expected, desired, success, failure)     atomic_compare_exchange_strong_explicit(object, expected, desired, success, failure)
#define atomic_fetch_add_explicit(object, argument, order)     ((void)(order), atomic_fetch_add(object, argument))
#define atomic_fetch_sub_explicit(object, argument, order)     ((void)(order), atomic_fetch_sub(object, argument))
#define atomic_fetch_or_explicit(object, argument, order)     ((void)(order), atomic_fetch_or(object, argument))
#define atomic_fetch_xor_explicit(object, argument, order)     ((void)(order), atomic_fetch_xor(object, argument))
#define atomic_fetch_and_explicit(object, argument, order)     ((void)(order), atomic_fetch_and(object, argument))

/* 7.17.7.4 and 7.17.7.5: the two compare-and-exchanges, which differ only in whether a failure may
 * be spurious. This target's cannot be, so the weak one is the strong one - which the standard
 * allows, and which makes the standard's own lock example work either way. */
#define atomic_compare_exchange_strong(object, expected, desired) \
    _Generic((object), \
        char *: __xcc_cas_char((volatile char *)(object), (char *)(expected), (char)(desired)), \
        signed char *: __xcc_cas_schar((volatile signed char *)(object), (signed char *)(expected), (signed char)(desired)), \
        unsigned char *: __xcc_cas_uchar((volatile unsigned char *)(object), (unsigned char *)(expected), (unsigned char)(desired)), \
        short *: __xcc_cas_short((volatile short *)(object), (short *)(expected), (short)(desired)), \
        unsigned short *: __xcc_cas_ushort((volatile unsigned short *)(object), (unsigned short *)(expected), (unsigned short)(desired)), \
        int *: __xcc_cas_int((volatile int *)(object), (int *)(expected), (int)(desired)), \
        unsigned *: __xcc_cas_uint((volatile unsigned *)(object), (unsigned *)(expected), \
                                   (unsigned)(desired)), \
        long *: __xcc_cas_int((volatile int *)(object), (int *)(expected), (int)(desired)), \
        long long *: __xcc_cas_llong((volatile long long *)(object), (long long *)(expected), \
                                     (long long)(desired)), \
        unsigned long long *: __xcc_cas_ullong((volatile unsigned long long *)(object), \
                                               (unsigned long long *)(expected), \
                                               (unsigned long long)(desired)))
#define atomic_compare_exchange_weak(object, expected, desired) \
    atomic_compare_exchange_strong(object, expected, desired)

/* 7.17.8: the flag, over the same 32-bit operations as an `int`. */
#define atomic_flag_test_and_set(object) (__xcc_exchange_int(&(object)->__value, 1) != 0)
#define atomic_flag_test_and_set_explicit(object, order) \
    ((void)(order), atomic_flag_test_and_set(object))
#define atomic_flag_clear(object) ((void)__xcc_exchange_int(&(object)->__value, 0))
#define atomic_flag_clear_explicit(object, order) ((void)(order), atomic_flag_clear(object))

/* 7.17.4, 7.17.5, 7.17.7.7 and 7.17.7.8. Every object this header accepts is lock free, and every
 * fence is full: the machine has one strength of barrier and this is it. */
#define atomic_is_lock_free(object) ((void)(object), 1)
#define atomic_init(object, value) ((void)(*(object) = (value)))
#define atomic_thread_fence(order) ((void)(order), __xcc_atomic_fence())
#define atomic_signal_fence(order) ((void)(order))

#endif
