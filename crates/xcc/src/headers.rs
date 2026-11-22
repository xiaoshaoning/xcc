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
enum Header {
    Shipped(&'static str),
    Refused(&'static str),
}

/// Every header in C99's 7.1.2 table, and what happens to it. **The one place** that decides; the
/// test below walks it against the standard's own list so that a header cannot be added to one and
/// forgotten in the other, which is how `assert.h` and `errno.h` came to be in neither.
const C99_HEADERS: &[(&str, Header)] = &[
    (
        "assert.h",
        Header::Shipped(include_str!("../include/assert.h")),
    ),
    (
        "complex.h",
        Header::Refused("`_Complex` and `_Imaginary` are not implemented"),
    ),
    (
        "ctype.h",
        Header::Shipped(include_str!("../include/ctype.h")),
    ),
    (
        "errno.h",
        Header::Shipped(include_str!("../include/errno.h")),
    ),
    (
        "fenv.h",
        Header::Refused("there is no access to the floating-point environment"),
    ),
    (
        "float.h",
        Header::Shipped(include_str!("../include/float.h")),
    ),
    (
        "inttypes.h",
        Header::Refused("not implemented; `<stdint.h>` and `printf` cover today's need"),
    ),
    (
        "iso646.h",
        Header::Shipped(include_str!("../include/iso646.h")),
    ),
    (
        "limits.h",
        Header::Shipped(include_str!("../include/limits.h")),
    ),
    (
        "locale.h",
        Header::Refused("not implemented; the C library's `localeconv` is not declared"),
    ),
    ("math.h", Header::Shipped(include_str!("../include/math.h"))),
    (
        "setjmp.h",
        Header::Refused("not implemented; `jmp_buf`'s layout is the ABI's, not the C library's"),
    ),
    (
        "signal.h",
        Header::Refused("not implemented; the C library's `signal` is not declared"),
    ),
    (
        "stdarg.h",
        Header::Shipped(include_str!("../include/stdarg.h")),
    ),
    (
        "stdbool.h",
        Header::Shipped(include_str!("../include/stdbool.h")),
    ),
    (
        "stddef.h",
        Header::Shipped(include_str!("../include/stddef.h")),
    ),
    (
        "stdint.h",
        Header::Shipped(include_str!("../include/stdint.h")),
    ),
    (
        "stdio.h",
        Header::Shipped(include_str!("../include/stdio.h")),
    ),
    (
        "stdlib.h",
        Header::Shipped(include_str!("../include/stdlib.h")),
    ),
    (
        "string.h",
        Header::Shipped(include_str!("../include/string.h")),
    ),
    (
        "tgmath.h",
        Header::Refused("not implemented; its macros need `_Generic`"),
    ),
    ("time.h", Header::Shipped(include_str!("../include/time.h"))),
    (
        "wchar.h",
        Header::Refused("not implemented; wide characters and `L\"...\"` literals are not"),
    ),
    (
        "wctype.h",
        Header::Refused("not implemented; wide characters are not"),
    ),
];

/// The text of a header this compiler ships, by the name an `#include` uses.
pub(crate) fn builtin(name: &str) -> Option<&'static str> {
    match disposition(name) {
        Some(Header::Shipped(text)) => Some(text),
        _ => None,
    }
}

/// Why a standard header is not available, when the reason is this compiler's to give.
///
/// The distinction matters to the reader of a diagnostic: `cannot find` is what a misspelt name
/// gets, and a name from C99's own table deserves better than being called a typo.
pub(crate) fn refusal(name: &str) -> Option<&'static str> {
    match disposition(name) {
        Some(Header::Refused(reason)) => Some(reason),
        _ => None,
    }
}

fn disposition(name: &str) -> Option<&Header> {
    C99_HEADERS
        .iter()
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
    fn read(&self, path: &str) -> Option<String> {
        self.host
            .read(path)
            .or_else(|| builtin(path).map(str::to_owned))
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
                Header::Shipped(_) => assert!(
                    row.contains(&format!("`{stem}`")),
                    "the README should name `{stem}`, which ships"
                ),
                Header::Refused(_) => assert!(
                    !row.contains(&format!("`{stem}`")),
                    "the README should not name `{stem}`, which refuses"
                ),
            }
        }
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
            let Header::Shipped(text) = disposition else {
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
            let Header::Refused(reason) = disposition else {
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
        assert!(!artifact.object().is_empty());
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
    fn a_header_the_host_has_wins() {
        #[derive(Debug)]
        struct Mine;
        impl Includes for Mine {
            fn read(&self, path: &str) -> Option<String> {
                (path == "stddef.h").then(|| String::from("/* mine */\n"))
            }
        }
        let headers = Headers::new(Box::new(Mine));
        assert_eq!(headers.read("stddef.h").as_deref(), Some("/* mine */\n"));
    }
}
