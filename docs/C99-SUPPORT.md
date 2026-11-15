# C99 support matrix

What xcc accepts, what it refuses, and what is not built yet. This table is the work list for
M3 and M6, and the answer to "do you support X?" without having to read the parser.

**Status**: nothing below is implemented yet. M2 is the preprocessor, M3 the parser and types,
M6 the rest of C. Everything is `planned` until a test says otherwise.

| status | meaning |
|---|---|
| `planned` | specified here, not built |
| `partial` | built, with the caveat noted |
| `sound` | built, and covered by the differential harness |
| `refused` | never accepted: a diagnostic every time, no object written (see [CONSTRAINTS.md](CONSTRAINTS.md) 3) |

## Language baseline

`--std=c99` (default) is the 1999 standard. `--std=c11` adds the C11 pieces a host meets in
practice. There is no `gnu89` mode, no extensions beyond what C99 defines, and no C++.

## Lexing and preprocessing

| construct | status |
|---|---|
| translation phases 1-3, line splicing, trigraphs | `planned` - trigraphs accepted with a warning (`-Wtrigraphs`), not silently ignored |
| comments, `//` and `/* */` | `planned` |
| identifiers, keywords, punctuators, universal character names | `planned` |
| integer constants (decimal, octal, hex, suffixes `u`/`l`/`ll` in any order) | `planned` |
| character constants, escapes, wide `L'x'` | `planned` - wide characters are `unsigned short`, as the target has no `wchar_t` of its own |
| string literals, adjacent concatenation, `L"x"` | `planned` |
| floating constants, all three forms | `planned` |
| `#include` quoted and angled, `#include_next` | `planned` - `#include_next` refused (a host concern) |
| `#define` object and function-like, `#`/`##`, variadic `...`/`__VA_ARGS__` | `planned` |
| `#if`/`#ifdef`/`#ifndef`/`#elif`/`#else`/`#endif`, `defined`, full constant expressions | `planned` |
| `#line`, `#error`, `#warning` (C23, but every host writes it) | `planned` |
| `#pragma once`, unknown `#pragma` ignored with a warning | `planned` |
| `__FILE__`, `__LINE__`, `__DATE__`, `__TIME__` | `planned` - the clock comes from `Options`, so output stays reproducible |
| `#pragma pack`, `#pragma comment` | `refused` - a diagnostic, not a silent misparse |

## Types

| construct | status |
|---|---|
| `char`, `short`, `int`, `long`, `long long` and unsigned forms; `_Bool` | `planned` |
| `float`, `double` | `planned` |
| `long double` | `planned`, and 64-bit by design: the same type as `double`, as MSVC has it on x86-64. 80-bit arithmetic is in the refusals below |
| `_Complex`, `_Imaginary`, `_Decimal*` | `refused` |
| pointers, arrays, functions, mixed declarators to any depth | `planned` |
| `struct`, `union`, `enum`, bitfields, anonymous members (C11) | `planned` |
| `typedef`, qualifiers (`const`, `volatile`, `restrict`), `inline` | `planned` - `restrict` is accepted and has no code-generation effect, which is conforming |
| variably modified types, VLAs as parameters | `planned` - VLA objects are accepted, with a stack-growth check |
| `_Alignas`, `_Alignof`, `_Static_assert`, `_Generic`, `_Atomic` (without `<stdatomic.h>`) | `planned` behind `--std=c11` |

## Expressions and statements

| construct | status |
|---|---|
| full C99 operator set and precedence, assignment operators, conditional, comma | `planned` |
| casts, `sizeof`, `sizeof` a VLA, `offsetof` | `planned` |
| compound literals, designated initialisers (nested, and the GNU `[a ... b]` form) | `planned` - the GNU range form accepted with a warning |
| implicit conversions, usual arithmetic conversions, constant folding | `planned` |
| `switch` with dense and sparse cases (jump tables where they pay) | `planned` |
| all statement forms, `goto`, labels, `break`/`continue` | `planned` |
| initialisation of aggregates, static and automatic | `planned` |
| `__func__` | `planned` |

## Run time and headers

| construct | status |
|---|---|
| freestanding: `stddef.h`, `stdarg.h`, `stdbool.h`, `stdint.h`, `limits.h`, `float.h` | `planned` (M8) |
| hosted: `stdio.h`, `stdlib.h`, `string.h`, `ctype.h`, `math.h`, `time.h` | `planned` (M8), over UCRT imports |
| `setjmp.h` | `planned`, with the caveat that a JIT'ed frame cannot be long-jumped out of without unwind information |
| `signal.h`, `locale.h`, `wchar.h`, `wctype.h` | `refused` for now - a diagnostic when included, rather than a header that pretends |

## Refusals, gathered

These are decisions, not gaps: each one is a diagnostic with a note saying why.

| refused | because |
|---|---|
| 80-bit `long double`, `_Complex`, `_Decimal` | the back end is SSE; 80-bit arithmetic would mean x87 and a second float model (ADR-001, decision 3) |
| `#include_next`, `#pragma pack`, `#pragma comment` | host and toolchain concerns; guessing is worse than saying no |
| C++, GNU statement expressions, nested functions, `typeof`, `__attribute__` beyond a named few | the language is C99 (ADR-001, decision 4) |
| any construct the front end cannot translate faithfully | miscompiling is not an option (CONSTRAINTS 3) |
