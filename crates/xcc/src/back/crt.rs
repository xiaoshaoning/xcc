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
    // time.h
    "clock",
    "difftime",
    "gmtime",
    "localtime",
    "mktime",
    "strftime",
    "time",
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
        // (`snprintf` is not among them: no Windows C runtime exports it, and the UCRT's header
        // spells it as an inline wrapper this compiler cannot reproduce. `--run` says so.)
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
