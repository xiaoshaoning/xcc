//! The C runtime, as a JIT sees it: the functions the headers declare, resolved to addresses.
//!
//! A JIT is not a linker. An object file leaves `printf` as an undefined symbol for the linker to
//! satisfy; a JIT has to have the address before it can finalize, and `cranelift-jit` panics on
//! one it cannot find. So the C runtime's own functions are looked up here, by name, from the same
//! DLL that a linked program would import them from.
//!
//! The list is the headers' and the runtime source's: every function `crates/xcc/include` declares
//! is here, because a header that declares what `--run` cannot resolve is a header that lies, and so
//! are the two names the C runtime's own `printf` front ends call. A name the library does not
//! export is left out rather than faked, which is why the list is a search rather than an assertion.

/// Every function the C headers declare. `stdarg`'s macros are not functions and are absent;
/// everything variadic is here, because a *call* to one is lowered as an indirect call and only a
/// floating-point argument is refused (ADR-006).
pub(crate) const C_RUNTIME: &[&str] = &[
    // The UCRT's formatting engine, which is what the CRT source's `printf` front ends call. The
    // library exports this and not `printf` itself, so these are the names a link imports.
    "__stdio_common_vfprintf",
    "__stdio_common_vsprintf",
    "__stdio_common_vfscanf",
    "__stdio_common_vsscanf",
    // The at-exit registration the CRT source's `atexit` wrapper calls.
    "_crt_atexit",
    // The argument setup the CRT source's entry point uses, and the two accessors it reads the
    // result from: `argc`/`argv` are the runtime's, not a startup prologue this file writes.
    // wctype.h: wide character classes and case mapping, all exported by the library.
    "iswalnum",
    "iswalpha",
    "iswblank",
    "iswcntrl",
    "iswdigit",
    "iswgraph",
    "iswlower",
    "iswprint",
    "iswpunct",
    "iswspace",
    "iswupper",
    "iswxdigit",
    "iswctype",
    "towctrans",
    "towlower",
    "towupper",
    "wctrans",
    "wctype",
    // fenv.h: the nine the library exports. `feraiseexcept` and `feupdateenv` are the runtime's
    // own - no Windows runtime has them - and are listed as unavailable to a JIT below.
    "feclearexcept",
    "fegetenv",
    "fegetexceptflag",
    "fegetround",
    "feholdexcept",
    "fesetenv",
    "fesetexceptflag",
    "fesetround",
    "fetestexcept",
    "__p___argc",
    "__p___argv",
    "_configure_narrow_argv",
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
    "setvbuf",
    "tmpnam",
    "snprintf",
    "sprintf",
    "sscanf",
    "tmpfile",
    "ungetc",
    "vfprintf",
    "vfscanf",
    "vprintf",
    "vscanf",
    "vsnprintf",
    "vsprintf",
    "vsscanf",
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
    // setjmp.h: `setjmp` is `_setjmp` with the frame argument this ABI passes, and `longjmp` is its
    // own name. See the header for why the one is a macro.
    "_setjmp",
    "longjmp",
    // locale.h, signal.h
    "localeconv",
    "setlocale",
    "raise",
    "signal",
    // math.h, the C99 additions: every one is exported by the universal runtime, and a name the
    // headers declare that `--run` cannot resolve is a header that lies.
    "acosh",
    "asinh",
    "atanh",
    "cbrt",
    "copysign",
    "erf",
    "erfc",
    "exp2",
    "expm1",
    "fdim",
    "fma",
    "fmax",
    "fmin",
    "hypot",
    "ilogb",
    "lgamma",
    "llrint",
    "llround",
    "log1p",
    "log2",
    "logb",
    "lrint",
    "lround",
    "nan",
    "nearbyint",
    "nextafter",
    "nexttoward",
    "remainder",
    "remquo",
    "rint",
    "round",
    "scalbln",
    "scalbn",
    "tgamma",
    "trunc",
    // inttypes.h
    "imaxabs",
    "imaxdiv",
    "strtoimax",
    "strtoumax",
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
        "feraiseexcept",
        "no Windows runtime exports it: the C runtime a *link* compiles in carries it, over the nine functions the library does export, and a JIT has no linker to bring that layer - link the program instead of running it",
    ),
    (
        "feupdateenv",
        "no Windows runtime exports it: the C runtime a *link* compiles in carries it, over the nine functions the library does export, and a JIT has no linker to bring that layer - link the program instead of running it",
    ),
    (
        "snprintf",
        "no Windows runtime exports it, and the layer that carries it is the C runtime a *link* compiles in - a JIT has no linker to bring that layer, so link the program instead of running it",
    ),
];

/// Why a name the headers declare cannot be called by a JIT on this machine, when the answer is known.
///
/// Two different facts, one question. `UNAVAILABLE` is the names no Windows runtime exports; the
/// second is the names the *classic* runtime lacks while the universal one has them - which is a
/// property of which runtime a JIT loads, not of the name. Both are ours to explain: a reader who
/// wrote `log2` should not have to discover that this runtime is the older one.
pub(crate) fn missing_reason(name: &str) -> Option<&'static str> {
    if let Some((_, reason)) = UNAVAILABLE.iter().find(|(absent, _)| *absent == name) {
        return Some(reason);
    }
    C_RUNTIME.contains(&name).then_some(
        "the C runtime `--run` loads does not export this name, though the headers declare it and the universal runtime has it; link the program instead of running it",
    )
}

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

    /// Whether `library` exports `symbol`.
    ///
    /// The same question a JIT asks, about one name - and the same loader, so that a test asking it
    /// is testing what a program will find rather than a list somebody typed. Test-facing: nothing
    /// in a build asks about one name.
    #[cfg(test)]
    pub(super) fn exports(library: &str, symbol: &str) -> bool {
        #[allow(unsafe_code)]
        // SAFETY: the process's own libraries, and names they may export; nothing is freed.
        unsafe {
            let library = CString::new(library).expect("a library name without a NUL");
            let module = LoadLibraryA(library.as_ptr().cast());
            if module.is_null() {
                return false;
            }
            let symbol = CString::new(symbol).expect("a symbol name without a NUL");
            !GetProcAddress(module, symbol.as_ptr().cast()).is_null()
        }
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
        // The names the headers define as macros: a body that calls one of these calls nothing.
        let mut ours: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for (_, disposition) in crate::headers::c99_headers() {
            let crate::headers::Disposition::Shipped(text) = disposition else {
                continue;
            };
            // Prose is not a promise: a comment naming `printf` is not a declaration of it.
            let text = without_comments(text);
            // Nor is a name this header defines itself: `isfinite`'s body calls `isnan`, which is
            // this header's own macro and not something to look up in a library at all.
            for line in text.lines() {
                if let Some(defined) = line.trim_start().strip_prefix("#define") {
                    if let Some(name) = defined
                        .trim_start()
                        .split(|c: char| !c.is_alphanumeric() && c != '_')
                        .next()
                    {
                        if !name.is_empty() {
                            ours.insert(name.to_owned());
                        }
                    }
                }
            }
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
            .filter(|name| !ours.contains(*name))
            .filter(|name| !available.contains(name) && !unavailable.contains(name))
            .collect();
        assert!(
            missing.is_empty(),
            "these names are in no list: {missing:?} - add each to C_RUNTIME, or to UNAVAILABLE with the reason"
        );
        // And the other way: a runtime name nothing declares is a symbol `--run` can call and no
        // program can reach, which is a promise to nobody. The CRT source is the second declarer:
        // its `printf` front ends call the UCRT's formatting engine, and that name belongs to the
        // runtime without belonging to a header.
        let front_ends = crate::back::CRT_SOURCE;
        let unreachable: Vec<&str> = C_RUNTIME
            .iter()
            .copied()
            .filter(|name| !declared.contains(*name) && !defines(front_ends, name))
            .collect();
        assert!(
            unreachable.is_empty(),
            "the runtime list has names nothing declares: {unreachable:?}"
        );
    }

    /// Every name the headers declare can be linked: the library exports it under the name the
    /// linker will ask for, or the C runtime source defines it.
    ///
    /// This is the measurement that found ten names with no home, written as a gate. A name that
    /// drifts out of `ucrtbase.dll` fails here rather than in a user's program at load time, with
    /// `ERROR_PROC_NOT_FOUND` and no clue which name.
    #[test]
    #[cfg(target_os = "windows")]
    fn every_declared_name_resolves_in_the_library_or_the_runtime() {
        let runtime_source = crate::back::CRT_SOURCE;
        let mut missing = Vec::new();
        for name in C_RUNTIME {
            let (library, exported) =
                crate::back::import_for(name).expect("a runtime name has a library");
            // The `printf` front ends are the *program's* half of the UCRT: the library has the
            // engine and the runtime source has the spelling, so a name the source defines is not
            // the library's to export.
            if !platform::exports(library, &exported) && !defines(runtime_source, &exported) {
                missing.push((name.to_string(), exported));
            }
        }
        assert!(
            missing.is_empty(),
            "declared, and neither exported by the library nor defined by the runtime: {missing:?}"
        );
    }

    /// Whether `source` defines or declares `name`: the name as a whole word, with a `(` after it.
    ///
    /// A substring match is not a test - `contains("time")` is true of the word `runtime`, and a
    /// gate satisfied by a comment is a gate that never fails. The `(` is what makes it a call, a
    /// definition or a declaration rather than a word in prose.
    fn defines(source: &str, name: &str) -> bool {
        source.match_indices(name).any(|(at, _)| {
            let before = source[..at].chars().next_back();
            let after = source[at + name.len()..].trim_start().chars().next();
            let standalone = !before.is_some_and(|c| c.is_alphanumeric() || c == '_');
            standalone && after == Some('(')
        })
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
