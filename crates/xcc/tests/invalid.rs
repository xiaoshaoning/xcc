//! Programs C forbids, and programs it does not: what a compiler must say no to.
//!
//! Each case was measured against `clang -std=c99 -fsyntax-only` when it was written, and the ones
//! marked `accepted` are accepted there too - a compiler that refused them would be refusing C. The
//! list exists because a sweep of twenty programs found three the compiler let through and the test
//! did not: `const` was consumed and dropped, and a struct could have two members with one name.
//!
//! A case is one line of C and one expectation. When this list grows, it grows with the measurement
//! that found the case, not with a guess about what C might forbid.

use xcc::{Compile, Options};

/// Compile `source` and say whether it produced an object.
fn compiles(source: &str) -> bool {
    Compile::new(Options::default())
        .compile("t.c", source)
        .is_ok()
}

fn refused(source: &str) -> bool {
    !compiles(source)
}

#[test]
fn a_const_object_cannot_be_written() {
    // C99 6.3.2.1p1: a *modifiable* lvalue is one that is not const-qualified. Every way of writing
    // one is here, because the qualifier reaches the type by a different road in each.
    assert!(refused("int f(void) { const int c = 1; c = 2; return c; }"));
    assert!(refused("int f(void) { const int c = 1; c++; return c; }"));
    assert!(refused(
        "int f(void) { const int c = 1; c += 2; return c; }"
    ));
    assert!(refused(
        "typedef const int ci; int f(void) { ci c = 1; c = 2; return c; }"
    ));
    assert!(refused(
        "struct S { const int a; }; int f(struct S s) { s.a = 2; return s.a; }"
    ));
    assert!(refused(
        "int f(void) { const int *p = 0; *p = 1; return 0; }"
    ));
    assert!(refused(
        "int f(void) { const int a[2] = {1, 2}; a[0] = 3; return a[0]; }"
    ));
    // A parameter's qualifier belongs to the object inside the body, though the *function type*
    // drops it (6.7.5.3p15) - two facts, and getting the second right must not lose the first.
    assert!(refused("int f(const int x) { x = 1; return x; }"));

    // C99 6.3.2.1p1 again, from the other side: a struct *any* of whose members is const is not a
    // modifiable lvalue either, however deeply the member is buried. Measured against clang, which
    // refuses both.
    assert!(refused(
        "struct S { const int a; }; int f(struct S x) { struct S y = x; y = x; return y.a; }"
    ));
    assert!(refused(
        "struct I { const int a; }; struct O { struct I in; int b; };          int f(struct O x) { struct O y = x; y = x; return 0; }"
    ));
    // A struct with no const member is assignable, and its own type being const is a different fact.
    assert!(compiles(
        "struct S { int a; }; int f(struct S x) { struct S y = x; y = x; return y.a; }"
    ));

    // And what is still allowed: reading one, and writing a non-const through a pointer to const is
    // the program's own promise, so writing *through* the pointer is refused (above) while copying
    // the pointer is not.
    assert!(compiles("int f(void) { const int c = 1; return c; }"));
    assert!(compiles(
        "int f(void) { const int c = 1; const int *p = &c; return *p; }"
    ));
}

#[test]
fn a_struct_has_no_two_members_with_one_name() {
    // C99 6.7.2.1p1.
    assert!(refused("struct H { int a; int a; };"));
    assert!(refused("union U { int a; int a; };"));
    // A member of one struct and a member of another are different names.
    assert!(compiles(
        "struct A { int a; }; struct B { int a; }; int main(void) { return 0; }"
    ));
    // C99 6.7.2.1p2: an unnamed member is a legal `struct-declaration`, and clang warns rather than
    // refusing, so this compiler builds it too.
    assert!(compiles("struct H { int; }; int main(void) { return 0; }"));
    // C11 6.7.2.1p13 puts an anonymous record's members in the same scope as the ones around it,
    // so this is one member named twice - and the check has to read the *completed* member list to
    // see it, because the second `a` is inside another record.
    assert!(refused("struct S { int a; union { int a; }; };"));
    assert!(refused("struct S { struct { int a; }; int a; };"));
    assert!(compiles(
        "struct S { int a; union { int b; }; }; int f(void) { struct S s; s.a = 1; s.b = 2; return s.a + s.b; }"
    ));
}

#[test]
fn the_mistakes_that_were_already_caught_stay_caught() {
    assert!(refused("int f(int x) { case 1: return 0; return x; }"));
    assert!(refused(
        "int f(int x) { switch (x) { case 1: return 1; case 1: return 2; } return 0; }"
    ));
    assert!(refused(
        "int f(int x) { switch (x) { default: return 1; default: return 2; } return 0; }"
    ));
    assert!(refused("int f(void) { break; return 0; }"));
    assert!(refused("int f(void) { continue; return 0; }"));
    assert!(refused("int f(void) { void v; return 0; }"));
    assert!(refused("int f(void) { int a[2]; a = 0; return 0; }"));
    assert!(refused("int f(void) { int *p = &5; return 0; }"));
    assert!(refused("int g(int); int f(void) { return g(1, 2); }"));
    assert!(refused("nosuchtype x;"));
    assert!(refused("int a[-1];"));
    assert!(refused("int f(void) { a: ; a: ; return 0; }"));
    assert!(refused("int f(void) { goto nowhere; return 0; }"));
    assert!(refused("void f(void) { return 1; }"));
    // clang accepts these two as well - a forward declaration of an incomplete struct, and a
    // non-void function with no `return` (which it warns about rather than refusing).
    assert!(compiles(
        "struct S; int f(struct S s); int main(void) { return 0; }"
    ));
    assert!(compiles("int f(void) { } int main(void) { return f(); }"));
}
