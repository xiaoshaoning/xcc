//! The C headers that ship with the compiler.
//!
//! A host that gives `Compile` nothing still gets `#include <stddef.h>`: the standard headers are
//! part of the compiler, not of the host's filesystem, so they are compiled into it with
//! `include_str!` and answered without a file ever being opened. A host's own files are consulted
//! first, so `-I` and a resolver can override any of these, which is what a program that ships
//! its own `stdint.h` expects.
//!
//! What a header may declare is a promise that the target's C library has it: these are
//! declarations, and the linker imports `ucrtbase.dll` for the bodies.

use crate::pp::Includes;

/// What this compiler does with one of the headers C99 requires of a hosted implementation.
///
/// There are exactly two answers and both are deliberate: a header either ships, or it refuses and
/// says why.  A third - `cannot find` - is the message a *typo* gets, and it tells a reader nothing
/// about whether the facility exists at all, so nothing may fall through to it.
pub(crate) enum Disposition {
    // No header is refused today - `complex.h` was the last one, and it ships now - but the table's
    // shape is what says a header *can* be refused with a reason rather than with silence, and the
    // test that walks C99's list checks every entry is one or the other.
    Shipped(&'static str),
    #[allow(dead_code)]
    Refused(&'static str),
}

/// Every header in C99's 7.1.2 table, and what happens to it. **The one place** that decides; the
/// test below walks it against the standard's own list so that a header cannot be added to one and
/// forgotten in the other, which is how `assert.h` and `errno.h` came to be in neither.
/// Whether `name` is a header this compiler answers for itself - shipped inside the binary, or
/// refused by name. Neither is a file a build system could rebuild.
///
/// The distinction a build system needs: a file inside the binary cannot be rebuilt, so it is not a
/// dependency - and a depfile that names `stdio.h` sends `make` looking for one.
pub(crate) fn is_shipped(name: &str) -> bool {
    C99_HEADERS
        .iter()
        .chain(C11_HEADERS)
        .any(|(shipped, _)| *shipped == name)
}

/// The three headers C11 adds that this compiler ships. C11 adds five, and two are absent on
/// purpose: `threads.h` would be a second threading interface over the same `_beginthreadex`, and
/// `uchar.h` needs `u"..."` and `U"..."` literals, which are a compiler feature rather than a
/// header. The ledger test checks C99's table against the standard and this one against its own
/// list, so neither can grow a member that the other's test does not know about.
const C11_HEADERS: &[(&str, Disposition)] = &[
    (
        "stdalign.h",
        Disposition::Shipped(include_str!("../include/stdalign.h")),
    ),
    (
        "stdatomic.h",
        Disposition::Shipped(include_str!("../include/stdatomic.h")),
    ),
    (
        "stdnoreturn.h",
        Disposition::Shipped(include_str!("../include/stdnoreturn.h")),
    ),
    (
        "threads.h",
        Disposition::Shipped(include_str!("../include/threads.h")),
    ),
    (
        "uchar.h",
        Disposition::Shipped(include_str!("../include/uchar.h")),
    ),
];

const C99_HEADERS: &[(&str, Disposition)] = &[
    (
        "assert.h",
        Disposition::Shipped(include_str!("../include/assert.h")),
    ),
    (
        "complex.h",
        Disposition::Shipped(include_str!("../include/complex.h")),
    ),
    (
        "ctype.h",
        Disposition::Shipped(include_str!("../include/ctype.h")),
    ),
    (
        "errno.h",
        Disposition::Shipped(include_str!("../include/errno.h")),
    ),
    (
        "fenv.h",
        Disposition::Shipped(include_str!("../include/fenv.h")),
    ),
    (
        "float.h",
        Disposition::Shipped(include_str!("../include/float.h")),
    ),
    (
        "inttypes.h",
        Disposition::Shipped(include_str!("../include/inttypes.h")),
    ),
    (
        "iso646.h",
        Disposition::Shipped(include_str!("../include/iso646.h")),
    ),
    (
        "limits.h",
        Disposition::Shipped(include_str!("../include/limits.h")),
    ),
    (
        "locale.h",
        Disposition::Shipped(include_str!("../include/locale.h")),
    ),
    (
        "math.h",
        Disposition::Shipped(include_str!("../include/math.h")),
    ),
    (
        "setjmp.h",
        Disposition::Shipped(include_str!("../include/setjmp.h")),
    ),
    (
        "signal.h",
        Disposition::Shipped(include_str!("../include/signal.h")),
    ),
    (
        "stdarg.h",
        Disposition::Shipped(include_str!("../include/stdarg.h")),
    ),
    (
        "stdbool.h",
        Disposition::Shipped(include_str!("../include/stdbool.h")),
    ),
    (
        "stddef.h",
        Disposition::Shipped(include_str!("../include/stddef.h")),
    ),
    (
        "stdint.h",
        Disposition::Shipped(include_str!("../include/stdint.h")),
    ),
    (
        "stdio.h",
        Disposition::Shipped(include_str!("../include/stdio.h")),
    ),
    (
        "stdlib.h",
        Disposition::Shipped(include_str!("../include/stdlib.h")),
    ),
    (
        "string.h",
        Disposition::Shipped(include_str!("../include/string.h")),
    ),
    (
        "tgmath.h",
        Disposition::Shipped(include_str!("../include/tgmath.h")),
    ),
    (
        "time.h",
        Disposition::Shipped(include_str!("../include/time.h")),
    ),
    (
        "wchar.h",
        Disposition::Shipped(include_str!("../include/wchar.h")),
    ),
    (
        "wctype.h",
        Disposition::Shipped(include_str!("../include/wctype.h")),
    ),
];

/// Every header in C99's table with what happens to it, for the test in `back::crt` that compares
/// this compiler's promises with the C runtime's answers.
#[cfg(test)]
pub(crate) fn c99_headers() -> &'static [(&'static str, Disposition)] {
    C99_HEADERS
}

/// The headers C11 adds that this compiler ships. A second table because it is a second claim: C99's
/// list is the standard's and a test compares it to the standard, and these are this compiler's own
/// choices among C11's five. Read by the ledger test in `back::crt`, which is the only caller - hence
/// `cfg(test)`: an accessor with no production caller is a seam nobody is on the other side of.
#[cfg(test)]
pub(crate) fn c11_headers() -> &'static [(&'static str, Disposition)] {
    C11_HEADERS
}

/// The text of a header this compiler ships, by the name an `#include` uses.
pub(crate) fn builtin(name: &str) -> Option<&'static str> {
    match disposition(name) {
        Some(Disposition::Shipped(text)) => Some(text),
        _ => None,
    }
}

/// Why a standard header is not available, when the reason is this compiler's to give.
///
/// The distinction matters to the reader of a diagnostic: `cannot find` is what a misspelt name
/// gets, and a name from C99's own table deserves better than being called a typo.
pub(crate) fn refusal(name: &str) -> Option<&'static str> {
    match disposition(name) {
        Some(Disposition::Refused(reason)) => Some(reason),
        _ => None,
    }
}

/// The headers no standard requires and this compiler ships anyway: the names a *system* SDK's
/// headers use to ask the compiler what it is.
///
/// They are read before the host's include directories, so one of these shadows the SDK's own file
/// of the same name - which is the point, and why each one's include guard is the SDK's. `vadefs.h`
/// is `mingw-w64`'s: it asks whether the compiler is GCC, `clang` or MSVC, and refuses everything
/// else by name before any Win32 declaration has been reached.
const BRIDGE_HEADERS: &[(&str, Disposition)] = &[
    // The header a program writes when it wants the MSVC intrinsics: mingw's declares them behind
    // an identification this compiler does not claim, so `xxhash`'s `_umul128` was an implicit
    // `int` - a *wrong number*, not a missing one.
    (
        "intrin.h",
        Disposition::Shipped(include_str!("../include/intrin.h")),
    ),
    (
        "vadefs.h",
        Disposition::Shipped(include_str!("../include/vadefs.h")),
    ),
    // The SSE intrinsic headers `winnt.h` includes for two macros it never expands at file scope.
    // Empty, so that a program which *uses* an intrinsic is told the name is not declared.
    (
        "emmintrin.h",
        Disposition::Shipped(include_str!("../include/emmintrin.h")),
    ),
    (
        "x86intrin.h",
        Disposition::Shipped(include_str!("../include/x86intrin.h")),
    ),
];

fn disposition(name: &str) -> Option<&Disposition> {
    C99_HEADERS
        .iter()
        .chain(C11_HEADERS)
        .chain(BRIDGE_HEADERS)
        .find(|(header, _)| *header == name)
        .map(|(_, disposition)| disposition)
}

/// What a compilation reads its includes from: the host's files, then the compiler's headers.
///
/// One type rather than two, because "where an include comes from" is one decision with one
/// order: a host may shadow a standard header, and a header this compiler ships is found when the
/// host has nothing to say. A host with no files at all is `Headers::new(Box::new(()))`, which is
/// what [`crate::Compile::new`] is.
#[derive(Debug)]
pub(crate) struct Headers {
    host: Box<dyn Includes>,
}

impl Headers {
    pub(crate) fn new(host: Box<dyn Includes>) -> Self {
        Self { host }
    }
}

impl Includes for Headers {
    /// The compiler's own headers first, then what the host's `-I` directories have.
    ///
    /// That is the opposite of the order a C++ compiler uses for `-I`, and it is what makes a
    /// *system* header directory usable at all: mingw-w64's `windows.h` lives beside its own
    /// `string.h`, `stdarg.h` and `stdint.h`, and those are written for a compiler that has
    /// `vadefs.h` and its whole CRT. Reading the compiler's own first means `#include <windows.h>`
    /// pulls in the SDK while `#include <string.h>` stays the runtime this linker links against -
    /// which is also the standard's reading of `<...>`: the places the *implementation* defines,
    /// then any the programmer adds.
    fn read(&self, path: &str) -> Option<String> {
        builtin(path)
            .map(str::to_owned)
            .or_else(|| self.host.read(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// C99 7.1.2's table of the headers a hosted implementation must provide - the oracle for
    /// [`C99_HEADERS`]. Written from the standard and not from the table, so that a header added to
    /// one and forgotten in the other is a failure rather than a gap.
    const C99_TABLE: &[&str] = &[
        "assert.h",
        "complex.h",
        "ctype.h",
        "errno.h",
        "fenv.h",
        "float.h",
        "inttypes.h",
        "iso646.h",
        "limits.h",
        "locale.h",
        "math.h",
        "setjmp.h",
        "signal.h",
        "stdarg.h",
        "stdbool.h",
        "stddef.h",
        "stdint.h",
        "stdio.h",
        "stdlib.h",
        "string.h",
        "tgmath.h",
        "time.h",
        "wchar.h",
        "wctype.h",
    ];

    #[test]
    fn the_readme_names_the_headers_that_ship_and_no_others() {
        // The README's header row is a hand-written list, and it said "twelve" while fifteen shipped.
        // A count would only be a second number to drift, so the test compares the *names*: every
        // header that ships is named there, and nothing that refuses is.
        let readme = include_str!("../../../README.md");
        let row = readme
            .lines()
            .find(|line| line.starts_with("| headers |"))
            .expect("the README has a headers row");
        for (name, disposition) in C99_HEADERS {
            let stem = name.trim_end_matches(".h");
            match disposition {
                Disposition::Shipped(_) => assert!(
                    row.contains(&format!("`{stem}`")),
                    "the README should name `{stem}`, which ships"
                ),
                Disposition::Refused(_) => assert!(
                    !row.contains(&format!("`{stem}`")),
                    "the README should not name `{stem}`, which refuses"
                ),
            }
        }
    }

    /// The README's *count* is checked too, not only the names it lists.
    ///
    /// It said "twelve" while fifteen shipped, "twenty" while nineteen did, and "twenty-four" while
    /// twenty-three did: a number in prose is a fact written down twice, and the copy no test reads
    /// is the copy that drifts. The names are compared by the test above; this is the word before
    /// them.
    #[test]
    fn the_readme_counts_the_headers_that_ship() {
        const NUMERALS: [&str; 30] = [
            "",
            "one",
            "two",
            "three",
            "four",
            "five",
            "six",
            "seven",
            "eight",
            "nine",
            "ten",
            "eleven",
            "twelve",
            "thirteen",
            "fourteen",
            "fifteen",
            "sixteen",
            "seventeen",
            "eighteen",
            "nineteen",
            "twenty",
            "twenty-one",
            "twenty-two",
            "twenty-three",
            "twenty-four",
            "twenty-five",
            "twenty-six",
            "twenty-seven",
            "twenty-eight",
            "twenty-nine",
        ];
        let readme = include_str!("../../../README.md");
        let line = readme
            .lines()
            .find(|line| line.starts_with("| headers |"))
            .expect("the README has a headers row");
        let word = line["| headers |".len()..]
            .split_whitespace()
            .next()
            .expect("a number word");
        let shipped = C99_HEADERS
            .iter()
            .chain(C11_HEADERS)
            .filter(|(_, disposition)| matches!(disposition, Disposition::Shipped(_)))
            .count();
        let counted = NUMERALS
            .iter()
            .position(|numeral| *numeral == word)
            .unwrap_or_else(|| panic!("`{word}` is not a number this test reads"));
        assert_eq!(
            counted, shipped,
            "the README says `{word}` headers ship; {shipped} do"
        );
    }

    #[test]
    fn the_ledger_is_c99s_table() {
        // `assert.h` and `errno.h` were in neither of the two lists this replaces, so a program that
        // included either was told the name did not exist. Two hand-written lists and one table in
        // three places is how that happens; this compares them to the standard instead.
        let named: Vec<&str> = C99_HEADERS.iter().map(|(name, _)| *name).collect();
        assert_eq!(named, C99_TABLE);
    }

    #[test]
    fn every_header_ships_and_compiles_as_itself() {
        // The whole point of the module: a header is a file the preprocessor reads, so a header
        // with a mistake in it is a mistake in every program that includes it. This parses each
        // one on its own, which is also the only way `stdarg.h` can be checked: it refuses.
        for (name, disposition) in C99_HEADERS {
            let Disposition::Shipped(text) = disposition else {
                continue; // refused headers have no text to parse; `a_header_...` covers them
            };
            let mut compiler = crate::Compile::new(crate::Options::default());
            compiler
                .preprocess(name, text)
                .unwrap_or_else(|diagnostics| panic!("{name}: {diagnostics:?}"));
            compiler
                .compile(
                    name,
                    &format!(
                        "#include <{name}>
"
                    ),
                )
                .unwrap_or_else(|diagnostics| panic!("{name}: {diagnostics:?}"));
        }
    }

    #[test]
    fn a_header_that_cannot_be_honoured_refuses_with_the_reason() {
        // The headers that are promises this compiler cannot keep refuse *with the reason*, and the
        // test checks the reason reaches the reader: before this, `#include <setjmp.h>` said
        // `cannot find`, which is what a misspelt name gets, and this test only asked whether
        // *something* failed - so the comment above it was true of no case at all.
        //
        // `stdarg.h` used to refuse because a `va_list` is the ABI's variadic convention rather
        // than a declaration. It is a real header now, and a program that reads an unnamed argument
        // compiles - the premise of this test went with it.
        for (name, disposition) in C99_HEADERS {
            let Disposition::Refused(reason) = disposition else {
                continue;
            };
            let mut compiler = crate::Compile::new(crate::Options::default());
            let errors = compiler
                .compile(
                    "t.c",
                    &format!(
                        "#include <{name}>
"
                    ),
                )
                .expect_err(name);
            let message = errors
                .items()
                .iter()
                .map(|diagnostic| diagnostic.message.as_str())
                .collect::<Vec<_>>()
                .join("; ");
            assert!(
                message.contains(reason),
                "{name} should refuse with {reason:?}, said {message:?}"
            );
        }
    }

    #[test]
    fn stdarg_h_is_a_header_now() {
        // The three macros over the builtins, and a variadic function that reads its arguments: the
        // whole of what `stdarg.h` promises at level 2, in one compilation. It is the program the
        // old `#error` existed to prevent from crashing at run time.
        let source = "#include <stdarg.h>
                      int sum(int count, ...) {
                          va_list ap; int total = 0; int i;
                          va_start(ap, count);
                          for (i = 0; i < count; i++) total += va_arg(ap, int);
                          va_end(ap);
                          return total;
                      }
                      int main(void) { return sum(4, 10, 20, 30, 2); }
";
        let options = crate::Options {
            opt_level: 2,
            ..crate::Options::default()
        };
        let artifact = crate::Compile::new(options)
            .compile("t.c", source)
            .unwrap_or_else(|diagnostics| panic!("{diagnostics:?}"));
        assert!(!artifact.bytes().is_empty());
    }

    #[test]
    fn a_header_is_read_from_the_compiler_when_the_host_has_nothing() {
        let headers = Headers::new(Box::new(()));
        assert!(headers
            .read("stddef.h")
            .is_some_and(|text| text.contains("size_t")));
        assert_eq!(headers.read("nosuch.h"), None);
    }

    #[test]
    fn the_compilers_own_headers_win_and_the_hosts_fill_the_gaps() {
        // The order `Headers::read` documents, and it is the opposite of the one a C++ compiler uses
        // for `-I` because these *are* the implementation's headers: `#include <windows.h>` must
        // reach the SDK while `#include <string.h>` stays the runtime this linker links against.
        #[derive(Debug)]
        struct Sdk;
        impl Includes for Sdk {
            fn read(&self, path: &str) -> Option<String> {
                match path {
                    "stddef.h" | "windows.h" => Some(String::from(
                        "/* the SDK's */
",
                    )),
                    _ => None,
                }
            }
        }
        let headers = Headers::new(Box::new(Sdk));
        assert_ne!(
            headers.read("stddef.h").as_deref(),
            Some(
                "/* the SDK's */
"
            ),
            "a header this compiler ships is the one that is read"
        );
        assert_eq!(
            headers.read("windows.h").as_deref(),
            Some(
                "/* the SDK's */
"
            ),
            "and one it does not ship comes from the host"
        );
    }
}
