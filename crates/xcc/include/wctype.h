/* wctype.h - C99 7.25: wide characters as classes, and case mapping for them.  Part of xcc.
 *
 * Clean room: the wording is C99 7.25 and nothing else.  The *widths* are the target's, measured
 * against the C runtime this compiler links: `wint_t` and the two opaque types are two bytes each
 * here, and `WEOF` is the value no character has.
 *
 * This header is the half of the wide-character story that needs nothing but declarations: every
 * function below is one the runtime exports under its own name.  `wchar.h` also needs the wide
 * *stdio* front ends, which the UCRT keeps in the program's half - `crates/xcc-crt` carries the
 * narrow ones, and the wide ones are the next thing it will carry.
 */
#ifndef XCC_WCTYPE_H
#define XCC_WCTYPE_H

/* 7.25p3: an integer type that can hold any wide character and `WEOF`, which is not one. */
typedef unsigned short wint_t;
#define WEOF 0xffff

/* 7.25p2 and p4: two opaque types, a class of characters and a mapping between them. They are
 * what `wctype` and `wctrans` hand out, and the runtime's own tables decide what they mean. */
typedef unsigned short wctype_t;
typedef unsigned short wctrans_t;

int iswalnum(wint_t character);
int iswalpha(wint_t character);
int iswblank(wint_t character);
int iswcntrl(wint_t character);
int iswdigit(wint_t character);
int iswgraph(wint_t character);
int iswlower(wint_t character);
int iswprint(wint_t character);
int iswpunct(wint_t character);
int iswspace(wint_t character);
int iswupper(wint_t character);
int iswxdigit(wint_t character);

/* 7.25.3.1: the extents, which are how a program asks about a class by its name - an implementation
 * may know classes the twelve above do not name. */
wctype_t wctype(const char *name);
int iswctype(wint_t character, wctype_t class);

/* 7.25.3.2: case mapping, and the same two questions in the other direction. */
wint_t towlower(wint_t character);
wint_t towupper(wint_t character);
wctrans_t wctrans(const char *name);
wint_t towctrans(wint_t character, wctrans_t mapping);

#endif
