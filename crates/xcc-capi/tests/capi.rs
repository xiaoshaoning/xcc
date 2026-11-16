//! The C API: the five functions called directly, and the same API from a host that is not Rust.
//!
//! The first tests call the functions as Rust can, which is fast and checks the behaviour. The
//! last builds the `cdylib` and runs `tests/host.py` against it, which checks the *boundary*: the
//! symbols are exported, the calling convention is the platform's, and a Python host can compile
//! C at run time and call it.

// The API is unsafe by nature; that is the thing under test.
#![allow(unsafe_code)]

use std::ffi::{c_char, c_void, CStr, CString};

use xcc_capi::{xcc_add_source, xcc_free, xcc_function, xcc_last_error, xcc_new, xcc_symbol};

/// A handle that frees itself, so a failing assertion does not leak one.
struct Compiler(*mut xcc_capi::Handle);

impl Drop for Compiler {
    fn drop(&mut self) {
        unsafe { xcc_free(self.0) }
    }
}

fn compiler(symbols: &[(&str, *const c_void)]) -> (Compiler, Vec<CString>) {
    let names: Vec<CString> = symbols
        .iter()
        .map(|(name, _)| CString::new(*name).expect("a name without a NUL"))
        .collect();
    let table: Vec<xcc_symbol> = names
        .iter()
        .zip(symbols)
        .map(|(name, (_, address))| xcc_symbol {
            name: name.as_ptr(),
            address: *address,
        })
        .collect();
    let handle = unsafe { xcc_new(table.as_ptr(), table.len()) };
    assert!(!handle.is_null(), "xcc_new never answers NULL");
    (Compiler(handle), names)
}

fn error(compiler: &Compiler) -> String {
    let pointer = unsafe { xcc_last_error(compiler.0) };
    assert!(!pointer.is_null(), "xcc_last_error never answers NULL");
    unsafe { CStr::from_ptr(pointer) }
        .to_string_lossy()
        .into_owned()
}

fn add_source(compiler: &Compiler, source: &str) -> i32 {
    let source = CString::new(source).expect("a source without a NUL");
    unsafe { xcc_add_source(compiler.0, source.as_ptr()) }
}

fn function(compiler: &Compiler, name: &str) -> *mut c_void {
    let name = CString::new(name).expect("a name without a NUL");
    unsafe { xcc_function(compiler.0, name.as_ptr()) }
}

#[test]
fn a_host_compiles_and_calls_without_rust() {
    let (compiler, _) = compiler(&[]);
    assert_eq!(
        add_source(&compiler, "int add(int a, int b) { return a + b; }"),
        0
    );
    assert_eq!(error(&compiler), "", "a call that worked left a message");

    let address = function(&compiler, "add");
    assert!(!address.is_null(), "`add` has no address");
    let add: extern "C" fn(i32, i32) -> i32 = unsafe { std::mem::transmute(address) };
    assert_eq!(add(2, 3), 5);
    assert_eq!(add(-7, 7), 0);

    // A second unit in one handle: no separate compile step, and it calls the first one after
    // declaring it - a source is a translation unit, and C has no implicit declarations.
    assert_eq!(
        add_source(
            &compiler,
            "int add(int, int); int twice(int x) { return add(x, x); }"
        ),
        0
    );
    let twice: extern "C" fn(i32) -> i32 =
        unsafe { std::mem::transmute(function(&compiler, "twice")) };
    assert_eq!(twice(4), 8);
}

#[test]
fn a_host_symbol_is_a_function_the_compiled_code_calls() {
    /// The host's own function, called from compiled C.
    extern "C" fn triple(x: i32) -> i32 {
        x * 3
    }
    let (compiler, _names) = compiler(&[("triple", triple as *const () as *const c_void)]);
    assert_eq!(
        add_source(
            &compiler,
            "int triple(int x); int use(void) { return triple(14); }"
        ),
        0,
        "{}",
        error(&compiler)
    );
    let use_it: extern "C" fn() -> i32 = unsafe { std::mem::transmute(function(&compiler, "use")) };
    assert_eq!(use_it(), 42);
}

#[test]
fn a_source_that_does_not_compile_says_why_and_leaves_the_rest_alone() {
    let (compiler, _) = compiler(&[]);
    assert_eq!(add_source(&compiler, "int good(void) { return 7; }"), 0);
    let good: extern "C" fn() -> i32 = unsafe { std::mem::transmute(function(&compiler, "good")) };
    assert_eq!(good(), 7);

    assert_eq!(
        add_source(&compiler, "int broken(void) { return nope; }"),
        -1
    );
    assert!(
        error(&compiler).contains("nope"),
        "the message does not name the mistake: {:?}",
        error(&compiler)
    );
    // The unit before it is still there, and the message is cleared by the next call that works.
    assert_eq!(good(), 7);
    assert!(!function(&compiler, "good").is_null());
    assert_eq!(error(&compiler), "");
}

#[test]
fn a_name_that_is_not_there_is_null_with_the_reason() {
    let (compiler, _) = compiler(&[]);
    assert!(
        function(&compiler, "missing").is_null(),
        "a missing function had an address"
    );
    assert!(
        error(&compiler).contains("missing"),
        "{:?}",
        error(&compiler)
    );
}

#[test]
fn a_handle_that_never_was_one_is_refused_rather_than_dereferenced() {
    // The C API cannot check a pointer, but NULL is the one wrong value a host hits by accident
    // (a failed `xcc_new` in a host that ignores the result), so it is refused, not chased.
    let null: *mut xcc_capi::Handle = std::ptr::null_mut();
    let source = CString::new("int f(void) { return 1; }").expect("no NUL");
    let name = CString::new("f").expect("no NUL");
    assert_eq!(unsafe { xcc_add_source(null, source.as_ptr()) }, -1);
    assert!(unsafe { xcc_function(null, name.as_ptr()) }.is_null());
    let message = unsafe { xcc_last_error(null) };
    assert_eq!(unsafe { CStr::from_ptr(message) }.to_str(), Ok(""));
    unsafe { xcc_free(null) };
    let _ = std::ptr::null::<c_char>();
}

#[test]
fn the_build_is_a_dll_a_python_host_can_call() {
    // A host that is not Rust: the acceptance test M7 names.
    let target = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug");
    let status = std::process::Command::new(env!("CARGO"))
        .args(["build", "-q", "-p", "xcc-capi"])
        .status()
        .expect("cargo runs");
    assert!(status.success(), "the cdylib did not build");
    let library = target.join("xcc_capi.dll");
    assert!(library.is_file(), "no {} to call", library.display());
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/host.py");
    let output = std::process::Command::new("python")
        .arg(script)
        .arg(&library)
        .output()
        .expect("python runs");
    assert!(
        output.status.success(),
        "the python host failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
