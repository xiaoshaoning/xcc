//! The C API: what a host that is not Rust calls.
//!
//! The same product [`xcc::Jit`] is, behind a pointer a C, Python or Lua host can hold: make a
//! compiler, add source, ask for a function's address, free it. Nothing here re-implements the
//! compiler - each function is one call into `Jit`, and the message a call failed with is kept on
//! the handle so the host can read it afterwards in its own words.
//!
//! This is the one crate where `unsafe` is expected: a raw pointer crossing a language boundary
//! is what a C API is, and the rustdoc on each function says what the boundary requires.

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};

use xcc::Jit;

/// A function the compiled code may call, by name: the host's `malloc`, or a callback of its own.
#[repr(C)]
pub struct xcc_symbol {
    /// The name the C source will use, NUL-terminated.
    pub name: *const c_char,
    /// The address C would give that name.
    pub address: *const c_void,
}

/// One compiler, and what its last call would have said.
///
/// The host holds this as an `xcc *` and never looks inside. One handle is used by one thread at
/// a time; two handles run on two threads, as `xcc` has no global state (CONSTRAINTS 2).
pub struct Handle {
    /// The compiler, or why there is none. Building the instruction set can fail, and that is a
    /// machine's problem rather than a program's - the handle still owns the reason, so the host
    /// reads it from `xcc_last_error` like any other.
    jit: Result<Jit, String>,
    /// The message the last call failed with, or empty. It lives on the handle because that is
    /// where `xcc_last_error` points: the string stays valid until the next call on this handle.
    error: CString,
}

/// What `xcc_last_error` answers for a handle the host never had.
const NO_HANDLE: &[u8] = b"\0";

/// Make a compiler that may call `count` host symbols by name.
///
/// Never NULL and never fails as such: a symbol list this cannot read, or an instruction set that
/// cannot be built, leaves a handle whose `xcc_add_source` refuses with the reason in
/// `xcc_last_error`. One error path, so a host that checks `xcc_add_source` checks everything.
///
/// The symbols are copied during this call and the names must outlive nothing.
///
/// # Safety
///
/// `symbols` must point to `count` readable `xcc_symbol`s, and each name in it must be a
/// NUL-terminated string. `NULL` with `count` 0 is the common case: no symbols.
#[no_mangle]
pub unsafe extern "C" fn xcc_new(symbols: *const xcc_symbol, count: usize) -> *mut Handle {
    let mut table: Vec<(&str, *const u8)> = Vec::with_capacity(count);
    let mut failure = None;
    if count > 0 {
        if symbols.is_null() {
            failure = Some("a symbol count without a symbol array".to_owned());
        } else {
            // SAFETY: the caller promises `count` readable elements.
            for symbol in unsafe { std::slice::from_raw_parts(symbols, count) } {
                if symbol.name.is_null() {
                    failure = Some("a symbol with no name".to_owned());
                    break;
                }
                // SAFETY: the caller promises a NUL-terminated name.
                match unsafe { CStr::from_ptr(symbol.name) }.to_str() {
                    Ok(name) => table.push((name, symbol.address.cast())),
                    Err(_) => {
                        failure = Some("a symbol name is not valid UTF-8".to_owned());
                        break;
                    }
                }
            }
        }
    }
    let jit = match failure {
        Some(reason) => Err(reason),
        // SAFETY: the addresses were promised valid for as long as the JIT lives, and it lives
        // until `xcc_free`.
        None => unsafe { Jit::with_symbols(&table) },
    };
    let handle = Handle {
        jit,
        error: CString::default(),
    };
    Box::into_raw(Box::new(handle))
}

/// Compile `source` into `compiler`, adding its functions to the ones already there.
///
/// Each source is a translation unit of its own: it must *declare* what it uses from a source
/// added earlier, exactly as one `.c` file declares what another defines. The functions are all
/// in one module, so the call resolves - the declaration is what C requires, not a link step.
///
/// Answers 0 when every stage was clean, and -1 otherwise, with the first things that went wrong
/// in `xcc_last_error` (all of them, one per line: a host fixing one mistake at a time is a host
/// that recompiles once per mistake).
///
/// # Safety
///
/// `compiler` must come from `xcc_new` and not have been freed; `source` must be NUL-terminated.
#[no_mangle]
pub unsafe extern "C" fn xcc_add_source(compiler: *mut Handle, source: *const c_char) -> c_int {
    // SAFETY: the caller promises a live handle.
    let Some(handle) = (unsafe { compiler.as_mut() }) else {
        return -1;
    };
    handle.error = CString::default();
    let source = if source.is_null() {
        None
    } else {
        // SAFETY: the caller promises a NUL-terminated string.
        unsafe { CStr::from_ptr(source) }.to_str().ok()
    };
    let Some(source) = source else {
        return refuse(handle, "the source is not valid UTF-8");
    };
    let outcome = match &mut handle.jit {
        Err(reason) => Err(reason.clone()),
        // A panic here is a bug in this compiler, and a host that is not Rust cannot unwind it:
        // it becomes a message like any other failure rather than an abort in the host's process.
        Ok(jit) => catch_unwind(AssertUnwindSafe(|| jit.add_source(source)))
            .unwrap_or_else(|payload| Err(internal_error(&payload))),
    };
    match outcome {
        Ok(()) => 0,
        Err(message) => refuse(handle, &message),
    }
}

/// The address of the function `name`, or NULL - with the reason in `xcc_last_error` when the
/// name is not one this compiler knows.
///
/// # Safety
///
/// `compiler` must come from `xcc_new` and not have been freed; `name` must be NUL-terminated.
/// Calling what comes back through the wrong signature is C's oldest problem, and this API cannot
/// check it.
#[no_mangle]
pub unsafe extern "C" fn xcc_function(compiler: *mut Handle, name: *const c_char) -> *mut c_void {
    // SAFETY: the caller promises a live handle.
    let Some(handle) = (unsafe { compiler.as_mut() }) else {
        return std::ptr::null_mut();
    };
    handle.error = CString::default();
    let Ok(jit) = &handle.jit else {
        return std::ptr::null_mut();
    };
    let name = if name.is_null() {
        None
    } else {
        // SAFETY: the caller promises a NUL-terminated string.
        unsafe { CStr::from_ptr(name) }.to_str().ok()
    };
    let Some(name) = name else {
        refuse(handle, "the function name is not valid UTF-8");
        return std::ptr::null_mut();
    };
    match jit.function(name) {
        Some(address) => address as *mut c_void,
        None => {
            refuse(handle, &format!("no function named `{name}`"));
            std::ptr::null_mut()
        }
    }
}

/// What the last call on `compiler` failed with: empty when it succeeded, so a host may print it
/// unconditionally. The pointer stays valid until the next call on the same handle.
///
/// # Safety
///
/// `compiler` must come from `xcc_new` and not have been freed - or be NULL, which answers an
/// empty string.
#[no_mangle]
pub unsafe extern "C" fn xcc_last_error(compiler: *mut Handle) -> *const c_char {
    match unsafe { compiler.as_ref() } {
        Some(handle) => handle.error.as_ptr(),
        None => NO_HANDLE.as_ptr().cast(),
    }
}

/// Free the compiler and everything it compiled.
///
/// # Safety
///
/// `compiler` must come from `xcc_new` and not have been freed already; NULL is refused, not
/// freed.
#[no_mangle]
pub unsafe extern "C" fn xcc_free(compiler: *mut Handle) {
    if let Some(handle) = unsafe { compiler.as_mut() } {
        // SAFETY: the pointer came from `Box::into_raw` in `xcc_new`, and the caller has not
        // freed it.
        drop(unsafe { Box::from_raw(handle) });
    }
}

/// Put `message` where `xcc_last_error` looks, and answer the failure code.
fn refuse(handle: &mut Handle, message: &str) -> c_int {
    // A message with a NUL inside cannot be handed out as a C string; the compiler writes none,
    // so dropping to what fits would lose the end of a diagnostic rather than misreport it.
    handle.error = CString::new(message).unwrap_or_default();
    -1
}

/// What a panic payload says, in the words a host would put in a log.
fn internal_error(payload: &Box<dyn std::any::Any + Send>) -> String {
    let message = payload
        .downcast_ref::<&str>()
        .map(|text| (*text).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "no message".to_owned());
    format!("internal error compiling: {message}")
}
