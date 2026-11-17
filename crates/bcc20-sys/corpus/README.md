/* The measuring stick's corpus.
 *
 * Six files both compilers build: xcc to 64-bit COFF, the ported Borland C++ 2.0 to 16-bit OMF.
 * The comparison between them is M9 of PLAN.md - time, code size and diagnostics, side by side,
 * on a small set of plain C, checked by `examples/compare.rs` and `tests/acceptance.rs`.
 *
 * The rules, because a corpus of one kind of code measures one kind of compiler:
 *
 * - **Freestanding.** No `#include`: Borland C++ 2.0's headers are not in this repository (they
 *   come with the product), so a file that needs one cannot be compiled by both.
 * - **C89.** The DOS compiler predates C99: no `//` comments, no declarations after statements,
 *   no `long long`, no compound literals. Writing the corpus in the older dialect is also the
 *   point - it is the dialect the measuring stick exists to compare against.
 * - **No I/O, no library calls.** Both compilers must emit a function body from what the source
 *   says; a call to `printf` would measure the linker, not the compiler.
 * - **Deterministic.** No clock, no random seed, no undefined behaviour: a file that computes a
 *   different answer each run is not a comparison.
 */
