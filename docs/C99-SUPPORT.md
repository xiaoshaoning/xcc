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
| translation phases 1-2: trigraphs, line splicing, CRLF | `sound` - in `lex.rs`, which is where a later trigraph warning would come from |
| comments, `//` and `/* */` | `sound` |
| identifiers, keywords, punctuators, universal character names | `planned` |
| integer constants (decimal, octal, hex, suffixes `u`/`l`/`ll` in any order) | `partial` - lexed as pp-numbers; classifying and evaluating them is M3 |
| character constants, escapes, wide `L'x'` | `planned` - wide characters are `unsigned short`, as the target has no `wchar_t` of its own |
| string literals and `L"x"` | `partial` - the lexer finds their ends; concatenation and escape decoding are M3 |
| floating constants, all three forms | `partial` - lexed as pp-numbers; the value is M3 |
| `#include` quoted and angled, `#pragma once`, include depth budget | `sound` (M2.4) - the host supplies the files through `Includes`; the compiler opens nothing itself |
| `#include_next` | `refused` - a host concern (see the refusals below) |
| `#define` object-like, `#undef`, redefinition checking | `sound` (M2.1) |
| `#define` function-like, `#`/`##`, variadic `...`/`__VA_ARGS__`, argument prescan and rescan | `sound` (M2.2), with the C rules that have observable consequences: an argument is expanded unless it is stringified or pasted, `##` produces one token or a diagnostic, and a macro never expands inside its own expansion |
| `#if`/`#ifdef`/`#ifndef`/`#elif`/`#else`/`#endif`, `defined`, integer constant expressions | `sound` (M2.3) - `intmax_t`/`uintmax_t` arithmetic, `&&`/`||` short-circuit so `#if 0 && 1/0` is not an error, and division by zero or signed overflow diagnosed rather than produced |
| `#error`, `#warning` | `sound` - the rest of the line is the message |
| `#line` | `partial` (M2.4) - it renumbers `__LINE__` and `__FILE__`; diagnostics still report physical lines, deliberately |
| `#pragma once`, unknown `#pragma` ignored with a warning | `sound` for the directive itself; `once` matters at M2.4 |
| `__FILE__`, `__LINE__`, `__DATE__`, `__TIME__`, `__STDC__`, `__STDC_VERSION__`, `__STDC_HOSTED__` | `sound` (M2.4) - the date and time come from `Options::stamp`, never from a clock, so the same source gives the same tokens. `__STDC_HOSTED__` is **0** until M8 ships headers, which is a real difference from a hosted compiler and the one disagreement the gate's probe file demonstrates |
| `#pragma pack`, `#pragma comment` | `refused` - a diagnostic, not a silent misparse |

## Types

| construct | status |
|---|---|
| `char`, `short`, `int`, `long`, `long long` and unsigned forms; `_Bool` | `planned` |
| `float`, `double` | `planned` |
| `long double` | `planned`, and 64-bit by design: the same type as `double`, as MSVC has it on x86-64. 80-bit arithmetic is in the refusals below |
| `_Complex`, `_Imaginary`, `_Decimal*` | `refused` |
| pointers, arrays, functions, declarations nested to any depth | `partial` (M3.1) - grouped declarators (`int (*fp)(int)`, `int (*a)[3]`) are refused until M3.3 |
| `enum`, with constant enumerators | `sound` (M3.1) |
| `struct`, `union`, bitfields, anonymous members (C11) | `planned` (M3.3) - refused with a diagnostic until then |
| `typedef` | `sound` (M3.1) |
| qualifiers (`const`, `volatile`, `restrict`), `inline` | `partial` - accepted and ignored, which is conforming for `restrict` and wrong for `const`; enforcing `const` arrives with the expression checker (M3.2) |
| variably modified types, VLAs as parameters | `planned` - VLA objects are accepted, with a stack-growth check |
| `_Alignas`, `_Alignof`, `_Static_assert`, `_Generic`, `_Atomic` (without `<stdatomic.h>`) | `planned` behind `--std=c11` |

## Expressions and statements

| construct | status |
|---|---|
| full C99 operator set and precedence, assignment operators, conditional, comma | `planned` |
| casts, `sizeof`, `sizeof` a VLA, `offsetof` | `planned` |
| compound literals, designated initialisers (nested, and the GNU `[a ... b]` form) | `planned` - the GNU range form accepted with a warning |
| constant expressions (`#if` apart): arithmetic, `sizeof` a type, enumerators, character constants | `sound` (M3.1) |
| implicit conversions, usual arithmetic conversions, folding of non-constant expressions | `planned` (M3.2) |
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
