//! The C runtime, as a JIT sees it: the functions the headers declare, resolved to addresses.
//!
//! A JIT is not a linker. An object file leaves `printf` as an undefined symbol for the linker to
//! satisfy; a JIT has to have the address before it can finalize, and `cranelift-jit` panics on
//! one it cannot find. So the C runtime's own functions are looked up here, by name, from the same
//! DLL that a linked program would import them from.
//!
//! The list is the headers': every function `crates/xcc/include` declares is here, because a
//! header that declares what `--run` cannot resolve is a header that lies. A name the library
//! does not export is left out rather than faked, which is why the list is a search rather than an
//! assertion.

/// Every function the C headers declare. `stdarg`'s macros are not functions and are absent;
/// everything variadic is here, because a *call* to one is lowered as an indirect call and only a
/// floating-point argument is refused (ADR-006).
const C_RUNTIME: &[&str] = &[
    // stdio.h
    "__acrt_iob_func",
    "clearerr",
    "fclose",
    "feof",
    "ferror",
    "fflush",
    "fgetc",
    "fgets",
    "fopen",
    "fprintf",
    "fputc",
    "fputs",
    "fread",
    "freopen",
    "fscanf",
    "fseek",
    "ftell",
    "fwrite",
    "getc",
    "getchar",
    "perror",
    "printf",
    "putc",
    "putchar",
    "puts",
    "remove",
    "rename",
    "rewind",
    "scanf",
    "setbuf",
    "snprintf",
    "sprintf",
    "sscanf",
    "tmpfile",
    "ungetc",
    // stdlib.h
    "abort",
    "abs",
    "atexit",
    "atof",
    "atoi",
    "atoll",
    "bsearch",
    "calloc",
    "div",
    "exit",
    "free",
    "getenv",
    "llabs",
    "lldiv",
    "malloc",
    "qsort",
    "rand",
    "realloc",
    "srand",
    "strtod",
    "strtol",
    "strtoll",
    "strtoul",
    "strtoull",
    "system",
    // string.h
    "memchr",
    "memcmp",
    "memcpy",
    "memmove",
    "memset",
    "strcat",
    "strchr",
    "strcmp",
    "strcoll",
    "strcpy",
    "strcspn",
    "strerror",
    "strlen",
    "strncat",
    "strncmp",
    "strncpy",
    "strpbrk",
    "strrchr",
    "strspn",
    "strstr",
    "strtok",
    "strxfrm",
    // ctype.h
    "isalnum",
    "isalpha",
    "isblank",
    "iscntrl",
    "isdigit",
    "isgraph",
    "islower",
    "isprint",
    "ispunct",
    "isspace",
    "isupper",
    "isxdigit",
    "tolower",
    "toupper",
    // math.h
    "acos",
    "asin",
    "atan",
    "atan2",
    "ceil",
    "cos",
    "cosh",
    "exp",
    "fabs",
    "floor",
    "fmod",
    "frexp",
    "ldexp",
    "log",
    "log10",
    "modf",
    "pow",
    "sin",
    "sinh",
    "sqrt",
    "tan",
    "tanh",
    // errno.h: the accessor, not an object, because the C library keeps one per thread.
    "_errno",
    // time.h
    "clock",
    "difftime",
    "gmtime",
    "localtime",
    "mktime",
    "strftime",
    "time",
];

/// The names a shipped header reaches for that `--run` cannot provide, each with the reason.
///
/// `symbols()` answers with what the loaded C runtime exports, so a header may declare something no
/// runtime offers under that name - and then `--run` fails at load with a message about a symbol the
/// program never wrote. These are the exceptions the headers are allowed to have, and the message
/// quotes the reason so the reader learns why instead of guessing.
pub const UNAVAILABLE: &[(&str, &str)] = &[
    (
        "__acrt_iob_func",
        "the universal C runtime's stream accessor; the classic runtime that `--run` loads exports `_iob` instead, so `--run` reaches no `FILE *` stream (printf needs none, and works)",
    ),
    (
        "snprintf",
        "no Windows runtime exports it; the universal runtime's header spells it as an inline wrapper this compiler cannot reproduce",
    ),
];

/// Every C runtime function this machine has, by name.
///
/// Empty where there is no Windows C runtime to ask (the target is Windows x86-64, so on any other
/// host a JIT'ed program simply has no library to call - which the JIT reports when it needs one).
pub fn symbols() -> Vec<(&'static str, *const u8)> {
    platform::load()
}

#[cfg(windows)]
mod platform {
    use super::C_RUNTIME;
    use std::ffi::CString;

    /// The C runtimes a machine may have, in the order to try: the classic one first, because it
    /// exports `printf` and friends under their own names. Exactly one of these is used.
    const LIBRARIES: [&str; 2] = ["msvcrt.dll", "ucrtbase.dll"];

    extern "system" {
        fn LoadLibraryA(name: *const u8) -> *mut u8;
        fn GetProcAddress(module: *mut u8, name: *const u8) -> *const u8;
    }

    /// Look every name up in **one** C runtime: the first that loads, and only it.
    ///
    /// One library, not whichever answers first: two CRTs in one process keep two heaps and two
    /// sets of stdio buffers, so a block from one `malloc` freed by the other's `free`, or a
    /// `FILE *` from one handed to the other's `fprintf`, is the corruption that gives a C runtime
    /// its reputation. The classic DLL is tried first because it exports the `printf` family under
    /// their own names (the UCRT offers them as inline wrappers around `__stdio_common_*`), and a
    /// name the chosen library does not export is simply absent - `--run` then says it cannot call
    /// it, which is true.
    pub(super) fn load() -> Vec<(&'static str, *const u8)> {
        // SAFETY: the process's own runtime, and names it exports. The libraries are never freed:
        // the JIT that uses these addresses lives as long as the process cares to, and a
        // `FreeLibrary` while a compiled function held one would be the bug worth avoiding.
        #[allow(unsafe_code)]
        unsafe {
            let mut module = std::ptr::null_mut();
            for library in LIBRARIES {
                let name = CString::new(library).expect("a library name without a NUL");
                module = LoadLibraryA(name.as_ptr().cast());
                if !module.is_null() {
                    break;
                }
            }
            if module.is_null() {
                return Vec::new();
            }
            let mut found = Vec::new();
            for symbol in C_RUNTIME {
                let name = CString::new(*symbol).expect("a symbol name without a NUL");
                let address = GetProcAddress(module, name.as_ptr().cast());
                if !address.is_null() {
                    found.push((*symbol, address));
                }
            }
            found
        }
    }
}

#[cfg(not(windows))]
mod platform {
    /// Nothing to look up: there is no Windows C runtime on a host that is not Windows.
    pub(super) fn load() -> Vec<(&'static str, *const u8)> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_c_runtime_answers_for_what_the_headers_promise() {
        let found = symbols();
        if !cfg!(windows) {
            assert!(found.is_empty(), "no Windows runtime elsewhere");
            return;
        }
        assert!(!found.is_empty(), "the C runtime has something");
        // The four a program cannot do without, and the one that made this module necessary.
        // (`snprintf` is not among them: it is in `UNAVAILABLE`, with the reason `--run` quotes.)
        for required in [
            "printf", "fprintf", "sprintf", "malloc", "strlen", "qsort", "fopen",
        ] {
            assert!(
                found
                    .iter()
                    .any(|(name, address)| *name == required && !address.is_null()),
                "the C runtime has no `{required}`"
            );
        }
    }

    #[test]
    fn the_runtime_list_is_what_the_headers_reach_for() {
        // Two lists that must agree - the functions the shipped headers name, and the functions the
        // C runtime is asked for - and nothing compared them, which is how `_errno` came to be
        // declared by `errno.h` and absent here: `--run` then failed at load with a message about a
        // symbol the program never wrote.
        //
        // The extraction is the header text itself: an identifier followed by `(`, on a declaration
        // or on a macro body (a macro is how `stderr` reaches `__acrt_iob_func`). A function-like
        // macro's own name is skipped, and so are the type words that look like calls.
        const KEYWORDS: &[&str] = &[
            "int", "void", "char", "unsigned", "long", "short", "float", "double", "struct",
            "union", "enum", "const", "static", "extern", "typedef", "sizeof", "defined",
        ];
        let mut declared = std::collections::BTreeSet::new();
        for (_, disposition) in crate::headers::c99_headers() {
            let crate::headers::Disposition::Shipped(text) = disposition else {
                continue;
            };
            // Prose is not a promise: a comment naming `printf` is not a declaration of it.
            let text = without_comments(text);
            for line in text.lines() {
                let body = match line.trim_start().strip_prefix('#') {
                    // A macro: scan its body, and not the name being defined. A function-like
                    // macro's parameter list is part of its name for this purpose, which is why the
                    // body is what follows the first whitespace.
                    Some(directive) => match directive.trim_start().strip_prefix("define") {
                        Some(define) => define
                            .trim_start()
                            .split_once(char::is_whitespace)
                            .map(|(_, body)| body),
                        None => None,
                    },
                    None => Some(line),
                };
                let Some(body) = body else { continue };
                for name in names_called(body) {
                    // A `__builtin_` is this compiler's own, not the C runtime's.
                    if !KEYWORDS.contains(&name.as_str()) && !name.starts_with("__builtin_") {
                        declared.insert(name);
                    }
                }
            }
        }
        assert!(
            declared.contains("printf") && declared.contains("__acrt_iob_func"),
            "the extraction reads the headers: {declared:?}"
        );
        let available: std::collections::BTreeSet<&str> = C_RUNTIME.iter().copied().collect();
        let unavailable: std::collections::BTreeSet<&str> =
            UNAVAILABLE.iter().map(|(name, _)| *name).collect();
        let missing: Vec<&str> = declared
            .iter()
            .map(String::as_str)
            .filter(|name| !available.contains(name) && !unavailable.contains(name))
            .collect();
        assert!(
            missing.is_empty(),
            "these names are in no list: {missing:?} - add each to C_RUNTIME, or to UNAVAILABLE with the reason"
        );
        // And the other way: a runtime name no header names is a symbol `--run` can call and no
        // program can declare, which is a promise to nobody.
        let unreachable: Vec<&str> = C_RUNTIME
            .iter()
            .copied()
            .filter(|name| !declared.contains(*name))
            .collect();
        assert!(
            unreachable.is_empty(),
            "the runtime list has names no header declares: {unreachable:?}"
        );
    }

    /// `text` with its `/* ... */` comments removed, newlines kept so lines still line up.
    fn without_comments(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(start) = rest.find("/*") {
            out.push_str(&rest[..start]);
            match rest[start..].find("*/") {
                Some(end) => {
                    out.extend(rest[start..start + end].chars().filter(|c| {
                        *c == "
"
                        .chars()
                        .next()
                        .unwrap()
                    }));
                    rest = &rest[start + end + 2..];
                }
                None => return out,
            }
        }
        out.push_str(rest);
        out
    }

    /// Every identifier in `line` that is followed by `(` - what a header *calls*, as opposed to what
    /// it merely mentions.
    fn names_called(line: &str) -> Vec<String> {
        let mut names = Vec::new();
        for (position, _) in line.match_indices('(') {
            let before = line[..position].trim_end();
            let reversed: String = before
                .chars()
                .rev()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            // The name's *first* character is the last one read, because the scan ran backwards: a
            // name never starts with a digit, so that is what tells a name from a number.
            let starts_with_digit = reversed.chars().last().is_some_and(|c| c.is_ascii_digit());
            if !reversed.is_empty() && !starts_with_digit {
                names.push(reversed.chars().rev().collect());
            }
        }
        names
    }

    #[test]
    fn every_name_in_the_list_is_unique() {
        // A duplicate would be resolved twice and shadowed once, which is the kind of thing that
        // is invisible until the day the list matters.
        let mut names: Vec<&str> = C_RUNTIME.to_vec();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(names.len(), before, "the list has a duplicate");
    }
}
