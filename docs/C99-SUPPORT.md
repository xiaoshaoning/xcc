# C99 support matrix

What xcc accepts, what it refuses, and what is not built yet - the answer to "do you support X?"
without having to read the parser.

**Status**: M10. The front end (preprocessor, parser, types, initialisers, the C library's
declarations) is done; there are two back ends - Cranelift behind `--opt-level=0`/`1`, and this
compiler's own behind `--opt-level=2`, which lowers everything the front end accepts. What is left is
variadic *definitions* - the rows below say why it is `--opt-level=2` only - the optimizer's backlog,
and the `planned` rows.

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
| `__FILE__`, `__LINE__`, `__DATE__`, `__TIME__`, `__STDC__`, `__STDC_VERSION__`, `__STDC_HOSTED__` | `sound` (M2.4) - the date and time come from `Options::stamp`, never from a clock, so the same source gives the same tokens. `__STDC_HOSTED__` is **1**: this is a hosted implementation, because it ships the standard headers and a linked program calls the host's C runtime (C99 6.10.8p1) |
| `#pragma pack`, `#pragma comment` | `refused` - a diagnostic, not a silent misparse |

## Types

| construct | status |
|---|---|
| `char`, `short`, `int`, `long`, `long long` and unsigned forms; `_Bool` | `sound` (M2, M6) - arithmetic, comparisons, division and shifts at the edges behave as `clang -O0` does on the same machine: a shift count at or beyond the width masks to the hardware's five or six bits, and `LLONG_MIN / -1` raises the divide error in both, which C leaves undefined |
| `float`, `double` | `sound` (M6): literals, SSE arithmetic, comparisons (C's unordered `!=`), conversions both ways, constants folded where a file-scope initialiser needs bytes, `sizeof` |
| `long double` | `sound` (M6), and 64-bit by design: the same type as `double`, as MSVC has it on x86-64. 80-bit arithmetic is in the refusals below |
| `_Complex`, `_Imaginary`, `_Decimal*` | `refused` |
| pointers, arrays, functions, declarations nested to any depth, grouped declarators (`int (*fp)(int)`, `int (*a)[3]`) | `sound` (M3.1, M3.3) |
| `enum`, with constant enumerators | `sound` (M3.1) |
| `struct`/`union`, tags, forward declarations, anonymous aggregates, bitfields | `sound` (M3.3) - laid out with the target's sizes, alignment and bitfield-unit rules, measured against `clang` on win64 (see the layout test); a C11 `_Alignas` on a member is still `planned` |
| a struct or union local, member access, struct assignment and initialisation, unions with type punning | `sound` (M6): an aggregate in a frame is a slot whose value is its address, so assignment copies bytes and a union's members share them. Copy-initialisation (`struct P q = p;`) and designators work |
| calls through a function pointer | `sound` (M6): the signature is the function type the pointer points at, aggregates and their ABI rules included |
| aggregates passed and returned by value | `sound` (M6) on the Win64 ABI: 1, 2, 4 and 8 bytes travel in an integer register, anything else by reference to a caller-owned copy, and a large result comes back through a buffer the caller passes first. Measured against Rust's `extern "C"`, which implements the same ABI, in both directions |
| a bitfield read, written, stepped and initialised | `sound` (M6): the unit is loaded once and the field shifted and masked out of it; a write merges the new bits back without disturbing its neighbours, and a signed field is sign-extended from its own width. `++`/`--` step in the field's own type and merge through the same write, so a value too wide is truncated by the field, as C stores it. In an initialiser the merge is the same arithmetic over the object's bytes, in a frame and in a global. The address of one is refused, as C99 6.5.3.2p1 says |
| a struct value inside a brace list (`struct Q q = {v};`) | `refused` - brace elision decides by the expression's type, which is not known before it is read; `struct Q q = {v, 3};` spelled with member's own braces works |
| `typedef` | `sound` (M3.1) |
| qualifiers (`const`, `volatile`, `restrict`), `inline` | `partial` - accepted and ignored, which is conforming for `restrict` and wrong for `const`; enforcing `const` arrives with the expression checker (M3.2) |
| variably modified types, VLAs as parameters | `planned` - VLA objects are accepted, with a stack-growth check |
| a file-scope object read or written in code | `sound` (M6): a global is data, so its value is the symbol's address - loaded, stored, subscripted and taken a member of like any other place |
| an `int` from an undeclared function assigned to a pointer | `sound` - **a warning**, and the assignment is made: `char *p = malloc(10)` with no declaration of `malloc` is how 1990s Windows C is written, and MSVC accepts it with C4047. The leniency is *only* for a result of an implicit call - an `int` from anywhere else still needs a cast |
| a call to a function nothing declared | `sound` - **diagnosed as a warning and compiled**, which is what MSVC's C4013 and GCC's `-std=gnu89` do and what real code on this platform still needs: the callee is taken to be a function returning `int`, and the *call site's* arguments are its signature, which is the same lowering a variadic call already uses (ADR-006). C99 6.5.2.2p1 makes it a constraint violation, so it is diagnosed; a translation that stopped there would refuse a great deal of working C |
| excess array initialisers, a too-long string for its array | `sound` - **diagnosed as a warning and compiled**: C99 6.7.8p2 and 6.7.8p14 make them constraint violations, and every compiler that meets them in the wild reports and carries on. What fits is written; `char s[2] = "ab"` is *valid* (6.7.8p14 drops the NUL when there is no room) |
| `asm("...")` | `refused` - not a call to an undeclared function: a language extension this compiler does not implement, so it says so instead of emitting a call to a symbol no linker has |
| `--std=c11`: `_Static_assert` and `_Generic` | `sound` (M10) - the flag exists, and in `c11` both constructs are parsed and mean what C11 6.7.10 and 6.5.1.1 say: a static assertion is checked where it is asked and reports the source's own message, and a generic selection reads every arm (for its type and its syntax), then reads the winner again for real, so the arms that lose emit nothing. **It needed a front-end fix underneath**: a declared `long` was the `int` type object, because the specifier path chose a type from `(signed, bits)`; `_Generic(l, int: 1, default: 9)` answered 1 for a `long`. C99 6.2.5p4 says those are two types even when the target makes them the same width, and the spelling now picks the type |
| `_Alignof(type)` | `sound` (M10) behind `--std=c11`: `sizeof` for alignment, a constant like `sizeof` |
| `_Alignas(type-or-constant)` on an object | `partial` (M10) behind `--std=c11`: raises the object's alignment - the *object's*, so it travels with the slot and with the global rather than being inferred from the type - and a value **below** the type's own is a diagnostic (C11 6.7.5p6). Zero has no effect (6.7.5p4) and any other value must be a power of two (6.7.5p3) - `_Alignas(3)` is refused in clang's own words, because three is not a small alignment, it is not an alignment. Sixteen bytes is the limit, `max_align_t` on this ABI and what the stack guarantees, so an *extended* alignment such as `_Alignas(32)` is refused with the number named, which 6.7.5p6 permits |
| `_Atomic` (without `<stdatomic.h>`) | `partial` (M10) behind `--std=c11`: `_Atomic T` and `_Atomic(T)` declare an object of type `T`. The atomicity of the *accesses* is not implemented, and this row says so rather than implying it |
| `--std=c99` (the default), `__STDC_VERSION__` | `sound` - `199901L` in `c99` and `201112L` in `c11`, which is the one place a program can ask what it is being compiled as |

## Expressions and statements

| construct | status |
|---|---|
| full C99 operator set and precedence, assignment operators, conditional, comma | `sound` (M3.2), with C's usual arithmetic conversions, array-to-pointer decay and constant folding of integer expressions |
| casts, `sizeof` a type, `sizeof` an expression | `sound` (M3.2) |
| `sizeof` a VLA, `offsetof` | `planned` (with VLAs) |
| an address in a file-scope initialiser: `char *p = "hello"`, `char *t[] = {"a", "b"}` | `sound` (M10) **at `--opt-level=2`**: the object's bytes hold eight zeroes and the data section carries a relocation naming the object the address is for, which the linker fills. An address *expression* - `int *p = &x;`, `arr + 2` - is still refused, because the parser has no symbolic constant to carry it ([ADR-004](ADR-004-data.md)) |
| compound literals, brace initialisers with C's brace elision | `sound` (M6) in a function - a compound literal gets a slot of its own and the walker fills it; at file scope a compound literal is refused (there is no frame to put it in) |
| designated initialisers (`.x = 1`, `[2] = ...`) one level deep | `sound` (M3.4); a designator that names a sub-object (`.a.b =`, `[1].x =`) is refused with a diagnostic |
| constant expressions (`#if` apart): arithmetic, `sizeof` a type, enumerators, character constants | `sound` (M3.1); an *integer* constant expression is required where C asks for one, so `int a[1.5]` and `case 1.5:` are refused rather than read as the float's bits |
| floating constants, including `f`/`l` suffixes and exponents | `partial` - decimal forms are read once into the type's bits; hexadecimal constants (`0x1.8p3`) are refused, which C99 6.4.4.2 requires and this reader does not do |
| implicit conversions, usual arithmetic conversions, assignment compatibility | `sound` (M3.2) |
| `switch` with dense and sparse cases (jump tables where they pay) | `planned` |
| a call to a variadic function (`printf`-style) with integer, character, string or pointer arguments | `sound` (M8): the call site's arguments are its signature, so it is lowered as an indirect call, and the default argument promotions are applied. A call with no floating-point argument leaves `AL` alone, which is harmless: the callee reads no vector registers |
| a floating-point argument to a variadic function | `sound` (M10) **at `--opt-level=2`**: the argument is promoted to `double` and written to the vector register *and* to the general register of its own position, which is Win64's duplicate rule - and is what lets one eight-byte-per-argument walk read a list that mixes the classes. The caller says zero vector registers in `AL`, so a callee reads the general ones, where the copies are. Verified against the real C runtime's `printf` with `%f`, `%.2f`, `%e` and `%g`, byte for byte against clang's output. Levels 0 and 1 refuse the construct with the reason rather than print zeros ([ADR-006](ADR-006-varargs.md)) |
| `va_list`, `va_start`, `va_arg` of an integer, pointer or record-free type | `sound` (M10) **at `--opt-level=2`**: `va_list` is a `char *` walking Win64's argument slots, which a variadic function's prologue fills from its four register arguments; `va_arg` is a load and an eight-byte step. Cranelift has no home area to point at and refuses with a span, so this is the one construct where the two lowerings deliberately differ ([ADR-006](ADR-006-varargs.md)) |
| `va_arg` of a floating-point type, **reading** | `sound` (M10) at `--opt-level=2`, against a caller that follows Win64: a variadic `double` is duplicated into a general-purpose register precisely so a walk like this finds it, so the read is the same eight bytes as any other argument. Verified with a clang-built caller; MinGW's gcc does not duplicate one, and would read as zeros ([ADR-006](ADR-006-varargs.md)) |
| `if`/`else`, `while`, `do`/`while`, `for`, `return`, `break`, `continue`, blocks | `sound` (M3.2) |
| `switch`, `goto`, labels | `sound` (M6): the controlling expression must be an integer, `case` values unique, one `default`, and a `goto` lowers to a jump - forward or backward, with a label on the entry block or a join given a block of its own |
| initialisation of aggregates, static and automatic | `sound` (M6): brace lists, elision and designators, read into a frame slot or a file-scope object's bytes |
| `__func__` | `planned` |

## Run time and headers

| construct | status |
|---|---|
| freestanding: `stddef.h`, `stdarg.h`, `stdbool.h`, `stdint.h`, `limits.h`, `float.h` | `sound` (M8) - they ship inside the compiler, consulted after the host's, so no `-I` is needed and `-I` still shadows them. `stdarg.h` was planned as the exception and is a real header now ([ADR-006](ADR-006-varargs.md)) |
| hosted: `stdio.h`, `stdlib.h`, `string.h`, `ctype.h`, `math.h`, `time.h` | `partial` (M8) - the declarations ship and a linked program uses the host's C runtime; `--run` resolves every function the headers declare out of one loaded C runtime, except `snprintf`, which no Windows runtime exports directly |
| `assert.h` | `sound` (M10) - `assert` is a macro over the C library's `fprintf` and `abort`, so the header needs no runtime of its own; `NDEBUG` gives 7.2p1's void expression. Verified both ways, and `errno`/`strtol` overflow read back `ERANGE` |
| `errno.h`, `iso646.h` | `sound` (M10) - `errno` is `#define errno (*_errno())`, because the C library keeps one per thread, and the `E*` values are the target's own; `iso646.h` is its eleven macros. Both work under `--run`: `_errno` is in the runtime's symbol list, and a test compares that list with what the headers actually name |
| a `FILE *` stream (`stdin`, `stdout`, `stderr`) under `--run` | `refused` - the headers spell the streams for the *universal* runtime (`__acrt_iob_func`), and `--run` loads the classic one, which exports `_iob` instead. One runtime per process is deliberate (two keep two heaps and two sets of buffers), so `--run` reports the symbol and the reason. `printf`/`fprintf` need no `FILE *` and work. **This is why `assert` runs only on the object path**: its diagnostic needs `stderr` |
| `setjmp.h` | `refused` - and the diagnostic says why: `jmp_buf`'s layout is the ABI's, not the C library's, and a JIT'ed frame cannot be long-jumped out of without unwind information |
| `locale.h`, `signal.h` | `sound` (M10) - `setlocale`, `localeconv` and `struct lconv` in the standard's own member order; `signal`, `raise`, `sig_atomic_t` and the target's six signals. `localeconv()->decimal_point` is what a program writing a decimal number by hand reads |
| `inttypes.h`, `complex.h`, `fenv.h`, `tgmath.h`, `wchar.h`, `wctype.h` | `refused` - each with its reason in the message, never `cannot find`. Which headers exist and what happens to each is **one table** in `src/headers.rs`, walked by a test against C99 7.1.2's own list |

## Refusals, gathered

These are decisions, not gaps: each one is a diagnostic with a note saying why.

| refused | because |
|---|---|
| 80-bit `long double`, `_Complex`, `_Decimal` | the back end is SSE; 80-bit arithmetic would mean x87 and a second float model (ADR-001, decision 3) |
| `#include_next`, `#pragma pack`, `#pragma comment` | host and toolchain concerns; guessing is worse than saying no |
| C++, GNU statement expressions, nested functions, `typeof`, `__attribute__` beyond a named few | the language is C99 (ADR-001, decision 4) |
| any construct the front end cannot translate faithfully | miscompiling is not an option (CONSTRAINTS 3) |
