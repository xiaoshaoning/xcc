/* locale.h - the locale C99 7.11 asks for.  Part of xcc.
 *
 * Clean room: the wording is C99 7.11 and nothing else.
 *
 * `localeconv` is why this header matters to a program that never calls `setlocale`: the decimal
 * point a *number* is printed with is the locale's, and code that writes a decimal number by hand
 * reads it from here.
 */
#ifndef XCC_LOCALE_H
#define XCC_LOCALE_H

#define LC_ALL 0
#define LC_COLLATE 1
#define LC_CTYPE 2
#define LC_MONETARY 3
#define LC_NUMERIC 4
#define LC_TIME 5

/* C99 7.11.2p2, in the standard's order: a program may rely on the order of a `struct` the
 * standard describes, and every member but the last is a string. */
struct lconv {
    char *decimal_point;
    char *thousands_sep;
    char *grouping;
    char *int_curr_symbol;
    char *currency_symbol;
    char *mon_decimal_point;
    char *mon_thousands_sep;
    char *mon_grouping;
    char *positive_sign;
    char *negative_sign;
    char int_frac_digits;
    char frac_digits;
    char p_cs_precedes;
    char p_sep_by_space;
    char n_cs_precedes;
    char n_sep_by_space;
    char p_sign_posn;
    char n_sign_posn;
    char int_p_cs_precedes;
    char int_p_sep_by_space;
    char int_n_cs_precedes;
    char int_n_sep_by_space;
    char int_p_sign_posn;
    char int_n_sign_posn;
};

char *setlocale(int category, const char *locale);
struct lconv *localeconv(void);

#endif
