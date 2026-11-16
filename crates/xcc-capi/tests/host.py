"""A host that is not Rust: compile C at run time and call it.

The acceptance test M7 names. It drives `xcc_capi.dll` through ctypes exactly as a Python program
would - no Rust, no generated bindings, no test framework - and checks the five functions the
header promises: a compiler, source added twice (so the second unit calls into the first), a host
symbol, a function's address, the error message, and the free.

    python host.py <path to xcc_capi.dll>
"""

import ctypes
import sys


class Symbol(ctypes.Structure):
    _fields_ = [("name", ctypes.c_char_p), ("address", ctypes.c_void_p)]


def load(path):
    """The five functions, with the C types they are called through."""
    library = ctypes.CDLL(path)
    library.xcc_new.restype = ctypes.c_void_p
    library.xcc_new.argtypes = [ctypes.POINTER(Symbol), ctypes.c_size_t]
    library.xcc_add_source.restype = ctypes.c_int
    library.xcc_add_source.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
    library.xcc_function.restype = ctypes.c_void_p
    library.xcc_function.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
    library.xcc_last_error.restype = ctypes.c_char_p
    library.xcc_last_error.argtypes = [ctypes.c_void_p]
    library.xcc_free.restype = None
    library.xcc_free.argtypes = [ctypes.c_void_p]
    return library


def add_source(library, compiler, source):
    status = library.xcc_add_source(compiler, source.encode())
    return status, library.xcc_last_error(compiler).decode()


def address_of(library, compiler, name):
    """The address of a function, or a failure that says which name and why."""
    address = library.xcc_function(compiler, name.encode())
    assert address, f"{name}: {library.xcc_last_error(compiler).decode()}"
    return address


def main(path):
    library = load(path)

    # A host symbol: the C runtime's own `abs`, which the compiled code will call.
    runtime = ctypes.CDLL("msvcrt")
    symbols = (Symbol * 1)()
    symbols[0].name = b"abs"
    symbols[0].address = ctypes.cast(runtime.abs, ctypes.c_void_p)

    compiler = library.xcc_new(symbols, 1)
    assert compiler, "xcc_new answered NULL"

    status, message = add_source(
        library,
        compiler,
        "int abs(int);\n"
        "int add(int a, int b) { return a + b; }\n"
        "int call_abs(int x) { return abs(x); }\n",
    )
    assert status == 0, f"the first source did not compile: {message}"

    # A second unit in the same handle calls the first one. It declares what it uses, because a
    # source is a translation unit of its own - there is no separate compile step, but there is no
    # shared header either.
    status, message = add_source(
        library,
        compiler,
        "int add(int a, int b);\n"
        "int call_abs(int x);\n"
        "int twice(int x) { return add(x, x) + call_abs(-1); }",
    )
    assert status == 0, f"the second source did not compile: {message}"

    add = ctypes.CFUNCTYPE(ctypes.c_int, ctypes.c_int, ctypes.c_int)(
        address_of(library, compiler, "add")
    )
    assert add(2, 3) == 5, "add(2, 3)"
    assert add(-7, 7) == 0, "add(-7, 7)"

    twice = ctypes.CFUNCTYPE(ctypes.c_int, ctypes.c_int)(
        address_of(library, compiler, "twice")
    )
    # twice(4) is add(4, 4) + call_abs(-1), which is abs(-1) = 1, so 9.
    assert twice(4) == 9, f"twice(4) is {twice(4)}"

    # A bad source: -1, and the mistake named in the message, in this host's own words.
    status, message = add_source(library, compiler, "int broken(void) { return nope; }")
    assert status == -1, "a source that does not compile was accepted"
    assert "nope" in message, f"the message does not name the mistake: {message!r}"

    # And the handle still works afterwards: a refused unit does not poison the ones before it.
    assert add(1, 1) == 2, "the handle stopped working after a refused source"

    # The message describes the last *call*, not the last failure: one that works clears it.
    assert library.xcc_function(compiler, b"add"), "add vanished"
    assert library.xcc_last_error(compiler).decode() == "", "the error was not cleared"

    # A name that is not there is NULL, with the reason.
    assert not library.xcc_function(compiler, b"missing"), "a missing function had an address"
    assert "missing" in library.xcc_last_error(compiler).decode()

    library.xcc_free(compiler)
    print("python host: compiled, called, refused, and freed")


if __name__ == "__main__":
    main(sys.argv[1])
