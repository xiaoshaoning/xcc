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
| character constants, every escape C99 has (simple, octal, `\x`), wide `L'x'` | `sound` - a multi-character constant is refused rather than given an implementation-defined value; wide characters are `unsigned short`, as the target has no `wchar_t` of its own |
| string literals and `L"x"` | `partial` - a literal has the `char[n]` type C gives it, adjacent literals concatenate, and its length counts escapes as the bytes they are; the bytes themselves are lowered at M4. A string too long for its array, or for an `int` array, is refused |
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
| `float`, `double` | `sound` (M6): literals, SSE arithmetic, comparisons (C's unordered `!=`), conversions both ways, constants folded where a file-scope initialiser needs bytes, `sizeof` |
| `long double` | `sound` (M6), and 64-bit by design: the same type as `double`, as MSVC has it on x86-64. 80-bit arithmetic is in the refusals below |
| `_Complex`, `_Imaginary`, `_Decimal*` | `refused` |
| pointers, arrays, functions, declarations nested to any depth, grouped declarators (`int (*fp)(int)`, `int (*a)[3]`) | `sound` (M3.1, M3.3) |
| `enum`, with constant enumerators | `sound` (M3.1) |
| `struct`/`union`, tags, forward declarations, anonymous aggregates, bitfields | `sound` (M3.3) - laid out with the target's sizes, alignment and bitfield-unit rules, measured against `clang` on win64 (see the layout test); `_Alignas` and C11 anonymous members are still `planned` |
| a bitfield read in code or named in an initialiser | `refused` - it shares its storage unit with its neighbours, so it has no address and needs a mask and a shift the emitter does not do yet |
| a struct or union local, member access, struct assignment and initialisation, unions with type punning | `sound` (M6): an aggregate in a frame is a slot whose value is its address, so assignment copies bytes and a union's members share them. Copy-initialisation (`struct P q = p;`) and designators work |
| aggregates passed or returned by value, a bitfield read in code | `refused` - the Win64 aggregate ABI and bitfield extraction are not lowered; each has a diagnostic |
| a struct value inside a brace list (`struct Q q = {v};`) | `refused` - brace elision decides by the expression's type, which is not known before it is read; `struct Q q = {v, 3};` spelled with member's own braces works |
| `typedef` | `sound` (M3.1) |
| qualifiers (`const`, `volatile`, `restrict`), `inline` | `partial` - accepted and ignored, which is conforming for `restrict` and wrong for `const`; enforcing `const` arrives with the expression checker (M3.2) |
| variably modified types, VLAs as parameters | `planned` - VLA objects are accepted, with a stack-growth check |
| `_Alignas`, `_Alignof`, `_Static_assert`, `_Generic`, `_Atomic` (without `<stdatomic.h>`) | `planned` behind `--std=c11` |

## Expressions and statements

| construct | status |
|---|---|
| full C99 operator set and precedence, assignment operators, conditional, comma | `sound` (M3.2), with C's usual arithmetic conversions, array-to-pointer decay and constant folding of integer expressions |
| casts, `sizeof` a type, `sizeof` an expression | `sound` (M3.2) |
| `sizeof` a VLA, `offsetof` | `planned` (with VLAs) |
| compound literals, brace initialisers with C's brace elision | `sound` (M3.3) |
| designated initialisers (`.x = 1`, `[2] = ...`) one level deep | `sound` (M3.4); a designator that names a sub-object (`.a.b =`, `[1].x =`) is refused with a diagnostic |
| constant expressions (`#if` apart): arithmetic, `sizeof` a type, enumerators, character constants | `sound` (M3.1); an *integer* constant expression is required where C asks for one, so `int a[1.5]` and `case 1.5:` are refused rather than read as the float's bits |
| floating constants, including `f`/`l` suffixes and exponents | `partial` - decimal forms are read once into the type's bits; hexadecimal constants (`0x1.8p3`) are refused, which C99 6.4.4.2 requires and this reader does not do |
| implicit conversions, usual arithmetic conversions, assignment compatibility | `sound` (M3.2) |
| `switch` with dense and sparse cases (jump tables where they pay) | `planned` |
| `if`/`else`, `while`, `do`/`while`, `for`, `return`, `break`, `continue`, blocks | `sound` (M3.2) |
| `switch`, `goto`, labels | `sound` (M3.4): the controlling expression must be an integer, `case` values unique, one `default`, `goto` may jump forward |
| initialisation of aggregates, static and automatic | `partial` (M3.3, M3.4): brace lists, elision and designators are checked for type and shape; the values themselves are not lowered until M4 |
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
