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

/// The text of a header this compiler ships, by the name an `#include` uses.
pub(crate) fn builtin(name: &str) -> Option<&'static str> {
    Some(match name {
        "ctype.h" => include_str!("../include/ctype.h"),
        "float.h" => include_str!("../include/float.h"),
        "limits.h" => include_str!("../include/limits.h"),
        "math.h" => include_str!("../include/math.h"),
        "stdarg.h" => include_str!("../include/stdarg.h"),
        "stdbool.h" => include_str!("../include/stdbool.h"),
        "stddef.h" => include_str!("../include/stddef.h"),
        "stdint.h" => include_str!("../include/stdint.h"),
        "stdio.h" => include_str!("../include/stdio.h"),
        "stdlib.h" => include_str!("../include/stdlib.h"),
        "string.h" => include_str!("../include/string.h"),
        "time.h" => include_str!("../include/time.h"),
        _ => return None,
    })
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

    #[test]
    fn every_header_ships_and_compiles_as_itself() {
        // The whole point of the module: a header is a file the preprocessor reads, so a header
        // with a mistake in it is a mistake in every program that includes it. This parses each
        // one on its own, which is also the only way `stdarg.h` can be checked: it refuses.
        for name in [
            "ctype.h",
            "float.h",
            "limits.h",
            "math.h",
            "stdbool.h",
            "stddef.h",
            "stdint.h",
            "stdio.h",
            "stdlib.h",
            "string.h",
            "time.h",
        ] {
            let text = builtin(name).unwrap_or_else(|| panic!("{name} ships"));
            let mut compiler = crate::Compile::new(crate::Options::default());
            compiler
                .preprocess(name, text)
                .unwrap_or_else(|diagnostics| panic!("{name}: {diagnostics:?}"));
            compiler
                .compile(name, &format!("#include <{name}>\n"))
                .unwrap_or_else(|diagnostics| panic!("{name}: {diagnostics:?}"));
        }
    }

    #[test]
    fn a_header_that_cannot_be_honoured_refuses_with_the_reason() {
        // `stdarg.h` is the one header that is a promise this compiler cannot keep: `va_list` is
        // the ABI's variadic convention, not a declaration (ADR-006). It says so, and the message
        // names the record so a reader can find out why.
        let mut compiler = crate::Compile::new(crate::Options::default());
        let diagnostics = compiler
            .compile(
                "t.c",
                "#include <stdarg.h>
",
            )
            .expect_err("stdarg.h refuses");
        let message = diagnostics
            .items()
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .collect::<Vec<_>>()
            .join("; ");
        assert!(message.contains("variadic"), "{message}");
        assert!(message.contains("ADR-006"), "{message}");
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
