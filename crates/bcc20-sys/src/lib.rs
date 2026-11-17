//! Borland C++ 2.0 as a library: the FFI over `libbcc20`, hand-written from `bcc20.h`.
//!
//! This is M9's measuring stick. `libbcc20` is borland-reborn's port of `BCC.EXE` - the compiler
//! that built early-1990s DOS games, translated function by function and checked byte for byte
//! against Borland's own binary. Compiling the same source with it and with xcc is how the front
//! end gets cross-checked against a 1991 implementation, and how the numbers in PLAN.md's successor
//! are produced.
//!
//! **Loaded, not linked.** The library is found at run time (`BCC20_LIB`, else `libbcc20.dll` on
//! `PATH`), so this crate builds on a machine that has never seen it and a missing library is
//! `None` rather than a link error. It is also the only way to call a mingw-built DLL from a
//! Rust/MSVC toolchain: the import library an MSVC linker wants is not one `cc` wrote.
//!
//! The ABI is `bcc20.h`'s, and this file mirrors it: struct layouts, `int`s, and the ownership
//! rule that the result owns what it returns and `bcc20_free` releases it. Nothing here is
//! re-exported from the C++ side; a change in `bcc20.h` has to be made here too, which is what
//! "hand-written" costs and why the test at the bottom compiles something real.

use std::ffi::{c_char, c_int, c_void, CStr, CString};

pub mod object;

/// A file the compiler may read, or one it wrote.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct RawFile {
    /// A DOS name, as BCC asks for it: `T.C`, `PROJ\X.H`.
    name: *const c_char,
    data: *const c_void,
    size: usize,
}

/// One compile, as `bcc20.h` describes it.
#[repr(C)]
struct RawJob {
    /// BCC's arguments, without the program's own name.
    argc: c_int,
    argv: *const *const c_char,
    files: *const RawFile,
    n_files: c_int,
    /// Host directory for any other file, or null: it stands for drive C:'s root.
    root: *const c_char,
    /// The clock and the files' times, in seconds since 1970; 0 for the host's.
    time: i64,
    /// 1: stop where `BCC.EXE`'s 8086 stack would ("Out of memory").
    stack_limit: c_int,
    /// The compiler explaining its choices, or `None`.
    explain: Option<unsafe extern "C" fn(*mut c_void, *const c_char)>,
    explain_context: *mut c_void,
}

/// What one compile produced. Owned by the caller until [`Compiler::free`].
#[repr(C)]
struct RawResult {
    status: c_int,
    /// What BCC printed; not NUL-terminated (`console_size` bytes).
    console: *mut c_char,
    console_size: usize,
    outputs: *mut RawFile,
    n_outputs: c_int,
}

type CompileFn = unsafe extern "C" fn(*const RawJob, *mut RawResult) -> c_int;
type FreeFn = unsafe extern "C" fn(*mut RawResult);

/// One BCC command line, from memory to memory.
pub struct Job<'a> {
    /// BCC's arguments: `-c`, `-ml`, `-O`, `-IC:\\INCLUDE`, and the DOS name of the source.
    pub args: Vec<String>,
    /// The files the job holds: the source, and any headers it reads.
    pub files: Vec<(&'a str, &'a [u8])>,
    /// A host directory for anything else (`-IC:\\INCLUDE`'s real home), or `None`.
    pub root: Option<&'a str>,
    /// The clock the compile sees, in seconds since 1970; 0 for the host's.
    pub time: i64,
    /// Whether `BCC.EXE`'s 8086 stack limit applies.
    pub stack_limit: bool,
}

impl<'a> Job<'a> {
    /// A job compiling `source`, which BCC will call `dos_name`, with `args` before it.
    ///
    /// The common case: one source, no headers, no clock. A job that needs more sets the fields.
    pub fn new(dos_name: &'a str, source: &'a str, args: &[&str]) -> Self {
        let mut all: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
        all.push(dos_name.to_owned());
        Self {
            args: all,
            files: vec![(dos_name, source.as_bytes())],
            root: None,
            time: 0,
            stack_limit: true,
        }
    }
}

/// What one compile produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// `BCC.EXE`'s exit status.
    pub status: i32,
    /// What it printed.
    pub console: String,
    /// Every file it wrote, by its DOS name.
    pub outputs: Vec<(String, Vec<u8>)>,
}

impl Outcome {
    /// Whether BCC considered the compile a success.
    pub fn succeeded(&self) -> bool {
        self.status == 0
    }
}

/// The loaded library: two functions, and the module handle that keeps them alive.
pub struct Compiler {
    compile: CompileFn,
    free: FreeFn,
    /// The module, kept so the function pointers stay valid for as long as this `Compiler`.
    _module: *mut c_void,
}

impl std::fmt::Debug for Compiler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Compiler").finish_non_exhaustive()
    }
}

impl Compiler {
    /// Load the ported compiler, or `None` when this machine does not have it.
    ///
    /// `BCC20_LIB` names the library file when it is somewhere else; otherwise the system looks
    /// for `libbcc20.dll` along `PATH`.
    #[cfg(windows)]
    pub fn load() -> Option<Self> {
        let name = std::env::var_os("BCC20_LIB")
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|| "libbcc20.dll".to_owned());
        let name = CString::new(name).ok()?;
        // SAFETY: a library name, and two symbols this crate declares exactly as `bcc20.h` does.
        #[allow(unsafe_code)]
        unsafe {
            let module = LoadLibraryA(name.as_ptr());
            if module.is_null() {
                return None;
            }
            let compile = GetProcAddress(module, c"bcc20_compile".as_ptr())?;
            let free = GetProcAddress(module, c"bcc20_free".as_ptr())?;
            Some(Self {
                compile: std::mem::transmute::<Symbol, CompileFn>(compile),
                free: std::mem::transmute::<Symbol, FreeFn>(free),
                _module: module,
            })
        }
    }

    /// The port is a Windows library: elsewhere there is nothing to load.
    #[cfg(not(windows))]
    pub fn load() -> Option<Self> {
        None
    }

    /// Run one job.
    pub fn compile(&self, job: &Job<'_>) -> Outcome {
        let arguments: Vec<CString> = job
            .args
            .iter()
            .map(|arg| CString::new(arg.as_str()).expect("an argument without a NUL"))
            .collect();
        let mut argv: Vec<*const c_char> = arguments.iter().map(|arg| arg.as_ptr()).collect();
        let names: Vec<CString> = job
            .files
            .iter()
            .map(|(name, _)| CString::new(*name).expect("a name without a NUL"))
            .collect();
        let files: Vec<RawFile> = names
            .iter()
            .zip(&job.files)
            .map(|(name, (_, data))| RawFile {
                name: name.as_ptr(),
                data: data.as_ptr().cast(),
                size: data.len(),
            })
            .collect();
        let root = job
            .root
            .map(|root| CString::new(root).expect("a path without a NUL"));
        let raw = RawJob {
            argc: argv.len() as c_int,
            argv: argv.as_mut_ptr(),
            files: files.as_ptr(),
            n_files: files.len() as c_int,
            root: root.as_ref().map_or(std::ptr::null(), |root| root.as_ptr()),
            time: job.time,
            stack_limit: c_int::from(job.stack_limit),
            explain: None,
            explain_context: std::ptr::null_mut(),
        };
        let mut result = std::mem::MaybeUninit::<RawResult>::zeroed();
        // SAFETY: the job's pointers outlive the call (everything it names is alive in this frame),
        // and the result is freed below through the library's own function.
        #[allow(unsafe_code)]
        unsafe {
            let status = (self.compile)(&raw, result.as_mut_ptr());
            let mut result = result.assume_init();
            let console = if result.console.is_null() || result.console_size == 0 {
                String::new()
            } else {
                let bytes =
                    std::slice::from_raw_parts(result.console.cast::<u8>(), result.console_size);
                String::from_utf8_lossy(bytes).into_owned()
            };
            let mut outputs = Vec::new();
            if !result.outputs.is_null() {
                for file in
                    std::slice::from_raw_parts(result.outputs, result.n_outputs.max(0) as usize)
                {
                    if file.name.is_null() {
                        continue;
                    }
                    let name = CStr::from_ptr(file.name).to_string_lossy().into_owned();
                    let data = if file.data.is_null() || file.size == 0 {
                        Vec::new()
                    } else {
                        std::slice::from_raw_parts(file.data.cast::<u8>(), file.size).to_vec()
                    };
                    outputs.push((name, data));
                }
            }
            (self.free)(&mut result);
            // The return value, not `result.status`: `bcc20_compile` answers -1 *without running*
            // when a job has more files than it takes, and a zeroed result would read as success.
            Outcome {
                status,
                console,
                outputs,
            }
        }
    }
}

/// What `GetProcAddress` answers before it is turned into the right signature.
#[cfg(windows)]
type Symbol = unsafe extern "system" fn() -> isize;

#[cfg(windows)]
extern "system" {
    fn LoadLibraryA(name: *const c_char) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const c_char) -> Option<Symbol>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `bcc20.h` says what the library does not: "not reentrant: one compile at a time per
    /// process". The test harness runs tests in threads, so they take turns here - which is the
    /// rule a host must follow too.
    static TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// The library, or a message saying why the test cannot run: a machine without the port has
    /// not failed at anything.
    fn compiler() -> Option<Compiler> {
        let compiler = Compiler::load();
        if compiler.is_none() {
            eprintln!("no libbcc20 on this machine: set BCC20_LIB or put it on PATH");
        }
        compiler
    }

    #[test]
    fn a_function_compiles_to_a_dos_object() {
        let Some(compiler) = compiler() else {
            return;
        };
        let _turn = TURN.lock().expect("the compiler turn");
        let source = "int add(int a, int b) { return a + b; }\n";
        let job = Job::new("T.C", source, &["-c", "-ml", "-O"]);
        let outcome = compiler.compile(&job);
        assert!(outcome.succeeded(), "console: {}", outcome.console);
        // Borland's own banner, then the source's name: what BCC.EXE printed.
        assert!(
            outcome.console.contains("Borland C++"),
            "{}",
            outcome.console
        );
        // `T.OBJ` is what a `-c` job writes, in Borland's OMF: the first byte of a record is 0x80
        // and the next two are its length.
        let (written, object) = outcome
            .outputs
            .iter()
            .find(|(file, _)| file.ends_with(".OBJ"))
            .unwrap_or_else(|| panic!("no object among {:?}", names(&outcome)));
        assert_eq!(written, "T.OBJ");
        assert!(object.len() > 32, "{object:?}");
        assert_eq!(object[0], 0x80, "an OMF record starts with its type");
    }

    #[test]
    fn a_source_with_a_mistake_says_so_in_bccs_words() {
        let Some(compiler) = compiler() else {
            return;
        };
        let _turn = TURN.lock().expect("the compiler turn");
        let job = Job::new("T.C", "int f(void) { return nope; }\n", &["-c", "-ml"]);
        let outcome = compiler.compile(&job);
        assert!(!outcome.succeeded(), "a mistake was accepted");
        assert!(
            outcome
                .console
                .to_ascii_lowercase()
                .contains("undefined symbol"),
            "{}",
            outcome.console
        );
    }

    fn names(outcome: &Outcome) -> Vec<&str> {
        outcome
            .outputs
            .iter()
            .map(|(name, _)| name.as_str())
            .collect()
    }
}
