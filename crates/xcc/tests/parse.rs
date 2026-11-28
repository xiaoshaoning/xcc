//! What the parser accepts and refuses: declarations, statements and expressions.

use xcc::{Options, SourceFile};

/// Parse `source` the way `Compile::compile` does up to the parser, and return the diagnostics.
fn check(source: &str) -> Vec<String> {
    check_with(Options::default(), source)
}

/// The same, with options the host chose - a budget, say.
fn check_with(options: Options, source: &str) -> Vec<String> {
    let file = SourceFile::new("t.c", source);
    let preprocessed = xcc::preprocess(&file, &options, &());
    assert!(
        !preprocessed.has_errors(),
        "the test source does not preprocess: {:?}",
        preprocessed.diagnostics
    );
    // The parser is internal; the public entry point runs it.
    let diagnostics = match xcc::Compile::new(options).compile("t.c", source) {
        Ok(_) => Vec::new(),
        Err(diagnostics) => diagnostics.items().to_vec(),
    };
    diagnostics
        .into_iter()
        .map(|diagnostic| diagnostic.message)
        .collect()
}

/// Whether the source parses as declarations, with no complaint about the declarations
/// themselves: "the code generator is not implemented yet" notice is expected.
fn parses(source: &str) -> bool {
    let complaints: Vec<String> = check(source)
        .into_iter()
        .filter(|message| {
            !message.contains("code generator") && !message.contains("not lowered yet")
        })
        .collect();
    assert!(
        complaints.is_empty(),
        "{source:?} should parse, but: {complaints:?}"
    );
    true
}

/// The warnings `source` produced: the diagnostics that do not stop the translation. `check` cannot
/// see them, because a program with warnings compiles - which is the point of a warning.
fn warnings(source: &str) -> Vec<String> {
    match xcc::Compile::new(Options::default()).compile("t.c", source) {
        Ok(artifact) => artifact
            .diagnostics()
            .iter()
            .map(|diagnostic| diagnostic.message.clone())
            .collect(),
        // An error may carry warnings beside it; the tests that ask are about the warning.
        Err(diagnostics) => diagnostics
            .items()
            .iter()
            .filter(|diagnostic| !diagnostic.is_error())
            .map(|diagnostic| diagnostic.message.clone())
            .collect(),
    }
}

fn refuses(source: &str, expected: &str) {
    // Not `check`: its first assertion is that the source reaches the parser at all, and a lexical
    // refusal - "stray `\` in program", a universal character name that is not one - is reported
    // before the parser runs. The question here is only what the compiler said.
    let complaints: Vec<String> = match xcc::Compile::new(Options::default()).compile("t.c", source)
    {
        Ok(_) => Vec::new(),
        Err(diagnostics) => diagnostics
            .items()
            .iter()
            .map(|diagnostic| diagnostic.message.clone())
            .collect(),
    };
    assert!(
        complaints.iter().any(|message| message.contains(expected)),
        "{source:?} should be refused with {expected:?}, got {complaints:?}"
    );
}

#[test]
fn an_array_initialiser_may_end_with_a_comma() {
    // C99 6.7.8p1: a trailing comma in a brace list is allowed, and every real table is written
    // with one. An array whose size comes from its list has no capacity to check, so nothing else
    // noticed: lz4frame.c's error-string table - `{ LZ4F_LIST_ERRORS(LZ4F_GENERATE_STRING) }`, whose
    // every element is followed by a comma - was rejected with "expected an expression".
    assert!(parses(
        "const char *names[] = { \"a\", \"b\", };
int main(void) { return 0; }"
    ));
    assert!(parses(
        "static int numbers[] = { 1, 2, 3, };
int main(void) { return numbers[2]; }"
    ));
    assert!(parses(
        "int sized[2] = { 1, 2, };
int main(void) { return sized[1]; }"
    ));
    assert!(parses(
        "int partial[2] = { 1, };
int main(void) { return partial[0]; }"
    ));
    // And more values than it has room for is still the surplus 6.7.8p2 forbids.
    let surplus = warnings("int a[2] = { 1, 2, 3 };");
    assert!(
        surplus
            .iter()
            .any(|message| message.contains("more were given")),
        "{surplus:?}"
    );
}

#[test]
fn the_address_of_a_parenthesised_function_designator_is_the_designator() {
    // C99 6.5.3.2p3 asks about the *operand*, and parentheses do not change what it is: `&(g)` is
    // `&g`. The shortcut used to read the token in front of the operand, where a `(` hid it - and
    // when the rule was rewritten as a token *range* the other half went missing for a commit, so
    // `&f(x)->m` became `(&f)(x)->m` again. Both halves are here now.
    let function = "static void g(void) { }\n";
    assert!(parses(&format!(
        "{function}int f(void) {{ void (*p)(void) = &(g); p(); return 0; }}"
    )));
    assert!(parses(&format!(
        "{function}int f(void) {{ void (*p)(void) = &((g)); p(); return 0; }}"
    )));
    assert!(parses(
        "struct j { int d; };\nstatic struct j one;\nstatic struct j *mk(int n) { (void)n; return &one; }\n\
         int f(void) { return &mk(1)->d != 0; }"
    ));
    // A call's result is not a designator, so its address is still an rvalue's address.
    refuses(
        "static int fn(int x) { return x; }\nint f(void) { int *p = &fn(1); (void)p; return 0; }",
        "needs something it can take the address of",
    );
}

#[test]
fn imaginary_types_are_refused_by_name() {
    // C99 6.2.5p11 has three imaginary types; C11 removed them and `clang` says "imaginary types are
    // not supported" in every dialect - so refusing is the oracle's answer, with a reason rather
    // than the "expected `;` after a declaration" it used to be.
    //
    // The block-scope case is the one that matters: what decides whether a statement is a
    // declaration is `SPECIFIERS`, and `_Imaginary float z;` in a function was read as a use of an
    // undeclared *name* until the word was added to that list. Asserting only that it fails would
    // still pass on that message, so the reason is asserted.
    let reason = "imaginary types are not supported";
    refuses("double _Imaginary z = 1.0;", reason);
    refuses("int f(void) { _Imaginary float z; return 0; }", reason);
    refuses(
        "int f(void) { double _Imaginary z = { 1.0, 0.0 }; return 0; }",
        reason,
    );
    // And the type such a program means is there: a `_Complex` whose real part is zero.
    assert!(parses(
        "int f(void) { double _Complex z = { 1.0, 0.0 }; return (int)z; }"
    ));
}

#[test]
fn a_non_void_function_that_falls_off_its_end_is_warned_about() {
    // C99 6.9.1p12: reaching the `}` of a non-void function makes the *call's* value undefined, and
    // only `main` defines it. `clang` warns and returns whatever is in the register; this compiler
    // warns and traps, so the undefined case fails where it happens instead of yielding a number
    // that looks like an answer. The warning is the part a program can see.
    let expected = "control reaches the end of `f`, which returns int";
    assert!(warnings("int f(void) { if (0) return 1; }")
        .iter()
        .any(|message| message.contains(expected)));
    // `void` returning nothing is not a warning, and neither is a `main` - whose fall-off the
    // standard defines.
    assert!(warnings("void f(void) { }").is_empty());
    assert!(warnings("int main(void) { }").is_empty());
    // Every path returning is neither.
    assert!(warnings("int f(int x) { if (x) return 1; return 0; }").is_empty());
}

#[test]
fn a_diagnostic_about_a_macro_body_says_which_use_it_came_from() {
    // Every expansion of one macro produces tokens with the *same* body spans, so a diagnostic that
    // points into the body cannot say which use is at fault - and `sqlite3.c`'s remaining errors are
    // exactly that: `SQLITE_INT_TO_PTR` defined once, used seventeen times, and a caret under the
    // definition. The token carries the invocation it came from, and the diagnostic adds it.
    let source = "#define W(X) (X + undeclared_thing)
int main(void) { return W(1); }
";
    let error = xcc::Compile::new(Options::default())
        .compile("t.c", source)
        .expect_err("it names nothing");
    let notes: Vec<&str> = error
        .items()
        .iter()
        .flat_map(|diagnostic| diagnostic.notes.iter().map(String::as_str))
        .collect();
    assert!(notes.contains(&"in `W`, used at t.c:2:25"), "{notes:?}");
}

/// "argument 1 is pointer to char" is a complaint about **argument 1**, and the caret belongs on it.
///
/// It did not: the diagnostic used the whole call's span, so the caret landed on the call's `(` - and
/// when the argument is a macro, the use has no token at all (the invocation is replaced by its
/// expansion before the parser sees it), so the diagnostic could not say which macro either. The span
/// is the invocation's now, and the note names it, which is the second route to a site - the first
/// one reads a token's `expansion` and works for a token from a macro *body*.
/// A macro used inside a header is noted with its own name, in its own file.
///
/// The lookup is by span *and* file, and this is the case where both matter: `main.c` expands `OUTER`
/// before the header expands `BAD`, so a lookup that confused the two files would either note the
/// wrong macro or find none.
#[test]
fn a_site_in_a_header_is_found_after_a_later_file_reset_the_offsets() {
    #[derive(Debug)]
    struct Headers;
    impl xcc::Includes for Headers {
        fn read(&self, path: &str) -> Option<String> {
            (path == "h.h").then(|| {
                "#define BAD ((char*)1)
void f(int a);
void g(void) { f(BAD); }
"
                .to_owned()
            })
        }
    }
    let source = "#define OUTER 1
static int v = OUTER;
#include \"h.h\"
";
    let error = xcc::Compile::with_includes(Options::default(), Box::new(Headers))
        .compile("main.c", source)
        .expect_err("a pointer is not an int");
    let notes: Vec<&str> = error
        .items()
        .iter()
        .flat_map(|diagnostic| diagnostic.notes.iter().map(String::as_str))
        .collect();
    assert!(notes.contains(&"in `BAD`"), "{notes:?}");
}

#[test]
fn an_argument_with_the_wrong_type_points_at_the_argument() {
    let source = "#define ARG ((char*)1)
void f(int a);
void g(void) { f(ARG + 1 + 1 + 1); }
";
    let error = xcc::Compile::new(Options::default())
        .compile("t.c", source)
        .expect_err("a pointer is not an int");
    let first = &error.items()[0];
    assert_eq!(
        first.span.unwrap().start as usize,
        source.find("ARG +").expect("the argument is there"),
        "the caret is on the argument, not on the call's `(`"
    );
    assert_eq!(
        first.span.unwrap().len,
        3,
        "and it covers the whole invocation"
    );
    assert!(
        first.notes.iter().any(|note| note == "in `ARG`"),
        "{:?}",
        first.notes
    );
}

/// The two ways a variable-length array gets its bookkeeping wrong, both found by reviewing the
/// commit that added them.
///
/// **Where the allocation goes.** The count and the element type came back from the declarator in a
/// parser field, and the declaration that read it was the *next* one: `typedef int Row[n];` was
/// refused with a message about a typedef, but the `int` declared after it was allocated the bytes -
/// `int later = 7;` became "cannot assign pointer to void to int". A declarator's answer is its
/// return value now, and a variable-length array that is not an automatic object says so at the
/// declaration that asked.
///
/// **Which name is one.** `sizeof a` was refused by asking a list of names that nothing ever pruned,
/// so a variable-length array in one function made `sizeof` of a plain array called `a` in another
/// function refuse - a sentence about the wrong function. The fact lives in the symbol entry now,
/// which the symbol table already scopes.
#[test]
fn a_variable_length_array_that_is_not_an_automatic_object_says_so() {
    let error = xcc::Compile::new(Options::default())
        .compile(
            "t.c",
            "int main(void) { int n = 4; typedef int Row[n]; return n; }",
        )
        .expect_err("a typedef cannot be a variable-length array");
    let messages: Vec<&str> = error
        .items()
        .iter()
        .map(|diagnostic| diagnostic.message.as_str())
        .collect();
    assert!(
        messages
            .iter()
            .any(|message| message.contains("must be an automatic object")),
        "{messages:?}"
    );
}

#[test]
fn a_plain_array_named_like_one_in_another_function_is_not_one() {
    // The `a` in `g` is `int a[3]`, and `sizeof a` is twelve. It was refused while a variable-length
    // array called `a` existed in a function that had already been compiled.
    let source = "int f(int n) { int a[n]; a[0] = n; return a[0]; }\n\
                  int g(void) { int a[3]; return (int)sizeof a; }\n\
                  int h(void) { return g() + f(9); }\n";
    // Level 2: the own back end is the only one that can move `rsp`, and a variable-length array
    // needs that.
    let artifact = xcc::Compile::new(Options {
        opt_level: 2,
        ..Options::default()
    })
    .compile("t.c", source)
    .expect("both arrays are legal");
    assert!(!artifact.bytes().is_empty());
}
/// The note that took four tokens to find.
///
/// A diagnostic about a token from a macro *body* points at the body - right, because that is where
/// the token was written - and the note adds the one thing the body cannot say: which use it came
/// from. The parser kept a two-token window for it, and `int r = TERN;` with `#define TERN (c ? p : q)`
/// complains at the `?` while the cursor is four tokens further on, so the window found nothing and
/// the note was simply absent. It is a scan back to the cursor now, and this is the program that made
/// the difference: before, the diagnostic had no note at all.
#[test]
fn a_note_is_found_for_a_macro_token_four_tokens_back() {
    let source = "#define TERN (c ? p : q)\nint main(void) {\n  int c = 1;\n  int *p = 0;\n  int q = 2;\n  int r = TERN;\n  return r;\n}\n";
    let error = xcc::Compile::new(Options::default())
        .compile("t.c", source)
        .expect_err("the arms of `?:` have different types");
    let notes: Vec<&str> = error
        .items()
        .iter()
        .flat_map(|diagnostic| diagnostic.notes.iter().map(String::as_str))
        .collect();
    assert!(notes.contains(&"in `TERN`, used at t.c:6:11"), "{notes:?}");
}
/// The note at the cursor, not just behind it.
///
/// A diagnostic raised *before* the offending token is consumed points at that token, and `#define T
/// (1 + )` used as `int x = T;` is that shape: "expected an expression, found `)`" with the `)` still
/// under the parser's feet. The scan that replaced the two-token window started at the token *before*
/// the cursor, so this note disappeared the moment the window became a scan - a narrower search than
/// the one it replaced. Measured against the commit before, which had the note: this test is the
/// difference.
#[test]
fn a_note_is_found_for_a_macro_token_under_the_cursor() {
    let source = "#define T (1 + )\nint main(void) { int x = T; return x; }\n";
    let error = xcc::Compile::new(Options::default())
        .compile("t.c", source)
        .expect_err("an operand is missing");
    let notes: Vec<&str> = error
        .items()
        .iter()
        .flat_map(|diagnostic| diagnostic.notes.iter().map(String::as_str))
        .collect();
    assert!(
        notes.iter().any(|note| note.starts_with("in `T`")),
        "{notes:?}"
    );
}
/// A statement expression whose block ends without an expression has no value, and GNU C says so by
/// treating it as `void`. A program that uses it as a value is a mistake, reported once.
#[test]
fn a_statement_expression_without_a_value_says_so() {
    let source = "int main(void) { int x = ({ int y = 1; }); return x; }\n";
    let error = xcc::Compile::new(Options::default())
        .compile("t.c", source)
        .expect_err("a block with no expression statement has no value");
    let messages: Vec<&str> = error
        .items()
        .iter()
        .map(|diagnostic| diagnostic.message.as_str())
        .collect();
    assert!(
        messages
            .iter()
            .any(|message| message.contains("must end with an expression")),
        "{messages:?}"
    );
}
/// A statement expression's value is its last expression statement **at its own level**.
///
/// The first version let the expression-statement branch write a field and read it afterwards, so a
/// block ending in a loop answered with the value of a statement *inside* the loop body:
/// `({ for (...) { s = s + i; } })` returned a value defined in the loop and used after it, and the
/// verifier said so - "v18 is defined in block 2 and used in block 4 which it does not dominate".
/// `clang` refuses the program, because a block whose last statement is a `for` has no value.
#[test]
fn a_statement_expression_ending_in_a_loop_has_no_value() {
    // The rule is the block's **last statement**, not the last expression statement anywhere in
    // it. Both of these are refused by `clang`; the second was accepted here with the value of the
    // `t` two statements earlier, which is a value the block's own end never provided.
    for source in [
        "int main(void) { int i; int v = ({ int s = 0; for (i = 0; i < 3; i = i + 1) { s = s + i; } }); return v; }\n",
        "int main(void) { int v = ({ int t = 1; t; int u = 2; }); return v; }\n",
    ] {
        let error = xcc::Compile::new(Options::default())
            .compile("t.c", source)
            .expect_err("the block's last statement is not an expression statement");
        let messages: Vec<&str> = error
            .items()
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .collect();
        assert!(
            messages
                .iter()
                .any(|message| message.contains("must end with an expression")),
            "{messages:?}"
        );
    }
}
/// An initialiser completes an array of unknown size - **through the symbol**, not only the type.
///
/// C99 6.7.8p22: "the type is completed by the initializer". A header declares
/// `extern struct code codes[];` and the unit that defines it writes the braces, so the completion
/// happens on a *second* declaration of the same name - and the first one left an incomplete array in
/// the symbol table. `sizeof codes / sizeof(struct code)` is how codec2 counts its LDPC codes, and it
/// was refused in the very file that had the array's size in front of it.
///
/// Found by surveying real programs that need the features this session added, which is what that
/// survey is for: no hand-written test had a header and a definition in two declarations.
#[test]
fn an_initialiser_completes_an_array_through_its_symbol() {
    let source = "struct code { const char *name; int rows; };\n\
                  extern struct code codes[];\n\
                  struct code codes[] = { { \"a\", 1 }, { \"b\", 2 }, { \"c\", 3 } };\n\
                  int codes_num(void) { return (int)(sizeof codes / sizeof(struct code)); }\n";
    // Level 2: the table holds string addresses, and a file-scope address is a relocation the own
    // back end writes.
    let artifact = xcc::Compile::new(Options {
        opt_level: 2,
        ..Options::default()
    })
    .compile("t.c", source)
    .expect("the initialiser completes the array");
    assert!(!artifact.bytes().is_empty());
}

/// The same completion for a plain array of numbers, and for a local one.
#[test]
fn a_local_array_of_unknown_size_is_completed_too() {
    let source = "int main(void) {\n\
                    int sizes[] = { 1, 2, 3, 4 };\n\
                    return (int)(sizeof sizes / sizeof sizes[0]);\n\
                  }\n";
    let artifact = xcc::Compile::new(Options::default())
        .compile("t.c", source)
        .expect("the local's count comes from its initialiser");
    assert!(!artifact.bytes().is_empty());
}
/// A variable-length array whose **element** is also one is refused by name.
///
/// `int m[rows][cols]` with both dimensions runtime has an element type - `int[cols]` - whose count is
/// a value, and a type here cannot hold one. The declarator kept a single "this is a variable-length
/// array" slot, so the outer dimension overwrote the inner: `m` was allocated `rows * sizeof(int *)`
/// bytes and every index used a stride of eight. A crash in polar_codes' DSP code, which declares
/// `int L[n+1][l*N]`, and a wrong answer waiting for anyone whose program did not fault.
///
/// Refused rather than approximated, and the message says why: a stride is a value and a stride lives in
/// a type.
#[test]
fn a_variable_length_array_of_variable_length_arrays_says_so() {
    // Both shapes: two variable dimensions, and a variable one inside a constant array. The second
    // was refused with "cannot assign to an array" - a sentence about the wrong thing - until the
    // review that asked what a *partial* nesting looked like.
    for source in [
        "int f(int rows, int cols) { int m[rows][cols]; m[0][0] = 1; return m[0][0]; }\n",
        "int f(int cols) { int m[3][cols]; m[0][0] = 1; return m[0][0]; }\n",
    ] {
        let error = xcc::Compile::new(Options {
            opt_level: 2,
            ..Options::default()
        })
        .compile("t.c", source)
        .expect_err("an array of variable-length arrays has no stride");
        let messages: Vec<&str> = error
            .items()
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .collect();
        let wanted = "a variable-length array whose element is also one is not supported: its stride is a value, and a stride lives in a type";
        assert!(messages.contains(&wanted), "{messages:?}");
    }
}
#[test]
fn a_hex_constant_may_be_unsigned_and_a_declarator_may_hold_two_lists() {
    // C99 6.4.4.1p5's table has two axes and this compiler implemented one. The *radix*: a
    // hexadecimal constant takes an unsigned type where a decimal one would be too large -
    // `sqlite3.c` writes `0xfffffffffc000000LL`, a `unsigned long long` that was "too large". And the
    // *suffix*, which picks a **type** and not a width: `1L` is a `long`, and this target's `int` and
    // `long` are both thirty-two bits, so a table keyed on `(signed, width)` answered `int` for both
    // `1L` and `1UL` - which `_Generic` asks about directly.
    let c11 = Options {
        standard: xcc::CStandard::C11,
        ..Options::default()
    };
    let complaints = check_with(
        c11.clone(),
        r#"
#define T(x) _Generic((x), int: 1, unsigned int: 2, long: 3, unsigned long: 4, long long: 5, unsigned long long: 6)
enum {
  hex_ll = T(0xfffffffffc000000LL),
  hex_int = T(0xffffffff),
  decimal = T(4294967295),
  suffixed = T(0xffffffffU)
};
_Static_assert(hex_ll == 6, "a hexadecimal `LL` takes `unsigned long long`");
_Static_assert(hex_int == 2, "and so does one with no suffix");
_Static_assert(decimal == 5, "a decimal one never takes an unsigned type");
_Static_assert(suffixed == 2, "a `U` suffix means what it says");
_Static_assert(T(1L) == 3, "a `L` suffix is a `long` and not an `int`");
_Static_assert(T(1UL) == 4, "and `UL` is an `unsigned long`");
_Static_assert(T(0x10L) == 3, "a hexadecimal `L` is a `long` too");
_Static_assert(T(1LL) == 5, "`LL` is the 64-bit one");
"#,
    );
    assert!(complaints.is_empty(), "{complaints:?}");

    // A declarator can hold *two* parameter lists, and the function's own is the first: the second
    // is its return type's. Taking the second for the first left `p` undeclared in this body.
    assert!(parses(
        "typedef struct vfs { int (*close)(struct vfs *); } vfs;\n\
         int (*getter(vfs *p)) (struct vfs *) { return p->close; }\n\
         void (*nothing(vfs *p, void *handle))(void) { (void)p; (void)handle; return 0; }"
    ));
    // ...and a parameter list inside a *group* is the function's own, which is the case sqlite
    // writes: `void (*f(vfs *pVfs, ...))(void)`.
    assert!(parses(
        "typedef struct vfs { int x; } vfs;\nvoid (*f(vfs *p))(void) { return (void (*)(void))0; }"
    ));
}

#[test]
fn the_spellings_a_windows_sdk_writes_are_read_as_what_they_mean() {
    // None of this is C99. All of it is in `windows.h`, and each one is read as the thing it means
    // rather than refused: a header written for another compiler asks for a type, a qualifier or an
    // empty declaration, and there is worth knowing about the program in none of those.
    //
    // `__int64` and its three siblings, with `unsigned` in front as the SDK writes it.
    assert!(parses("int f(void) { unsigned __int64 a = 1; __int32 b = 2; __int16 c = 3; __int8 d = 4; return (int)(a + b + c + d); }"));
    // `__inline`, `__forceinline`, `__restrict` and the alignment promises: no-ops here.
    assert!(parses("static __inline int g(void) { return 1; }"));
    assert!(parses("__forceinline int h(void) { return 2; }"));
    assert!(parses("int *__restrict p; int f(void) { return p == 0; }"));
    assert!(parses(
        // `UNALIGNED` in `winnt.h` is `#define UNALIGNED __unaligned`; the word itself is the no-op.
        "#define UNALIGNED __unaligned\ntypedef unsigned short WCHAR; typedef WCHAR UNALIGNED *LPUWSTR;"
    ));
    // `__extension__`, GCC's marker: it says the next declaration uses an extension.
    assert!(parses("__extension__ typedef unsigned long long ULL;"));
    // An empty declaration, which every macro that expands to nothing leaves behind.
    assert!(parses(";"));
    assert!(parses(
        "struct S { int a; ; int b; }; int f(void) { return (int)sizeof(struct S); }"
    ));
    // `#pragma pack(_CRT_PACKING)` - the argument is a macro, and it is expanded like any other.
    assert!(parses(
        "#define _CRT_PACKING 8
#pragma pack(push,_CRT_PACKING)
struct S { char c; int i; };
#pragma pack(pop)"
    ));
}

#[test]
fn a_pack_pragma_inside_a_body_is_refused_and_outside_it_is_laid_out() {
    // A record gets one layout, so it gets one limit - the one in force where its body ends. A
    // pragma *inside* the body would give different members different limits, which this compiler's
    // one pass cannot express, so it says so instead of quietly applying it to half the members.
    refuses(
        "struct S { int a;
#pragma pack(1)
int b; };",
        "a `#pragma pack` inside a struct or union body is not supported",
    );
    // Outside, it is a fact about a position in the token stream, and the layout follows it. The
    // numbers are `clang`'s, measured in `tests/exe.rs`.
    assert!(parses(
        "#pragma pack(1)
struct S { char c; int i; };
#pragma pack()
int f(void) { return (int)sizeof(struct S); }"
    ));
    refuses(
        "#pragma pack(3)
struct S { int a; };",
        "the limit here is 1, 2, 4, 8 or 16",
    );
}

#[test]
fn a_universal_character_name_in_an_identifier_is_that_identifier() {
    // C99 6.4.2.1 lets a universal character name stand for a letter of an identifier, and 6.4.3p2
    // makes it the *same* identifier as the character it names - so the two spellings collide.
    // Asserting a redefinition is the strong form of "they are one name": a program that merely
    // compiled could have two symbols with one meaning. The sources are Rust *raw* strings, because
    // a `\u` in an ordinary one is a Rust escape and never reaches the compiler.
    refuses(
        r#"int caf\u00e9 = 1;
int café = 2;"#,
        "redefinition of `café`",
    );
    refuses(
        r#"int f(void) { int a\u00e9 = 1; return aéb; }"#,
        "`aéb` is not declared",
    );
    assert!(parses(
        r#"int f(void) { int a\u00e9b = 1; return a\u00e9b + aéb; }"#
    ));
    // A character below `0xA0` cannot be spelled this way - the rule that also stops `\u0069nt`
    // from being the keyword `int` - and the three exceptions are not identifier characters.
    refuses(
        r#"int \u0041 = 1;"#,
        "a universal character name is four hex digits",
    );
    refuses(r#"int \u0024x = 1;"#, "`$` is not part of an identifier");
    refuses(
        r#"int a\u00 = 1;"#,
        "a universal character name is four hex digits",
    );
    refuses(
        r#"int a\uD800 = 1;"#,
        "a universal character name is four hex digits",
    );
    // A code point outside ASCII is an identifier character here whatever it is: completeness would
    // be Unicode's XID tables, and `clang` - which has them - refuses an emoji. Accepting a name
    // `clang` would refuse is a portability trap worth a line of documentation, not a table.
    assert!(parses(r#"int \U0001F600x = 1;"#));
}

#[test]
fn universal_character_names_are_characters() {
    // C99 6.4.3p2: `\u` is four hex digits and `\U` eight, and the value is a *character* - so a
    // narrow literal holds its UTF-8 bytes, a `u` literal its UTF-16 units, and a surrogate is not
    // a character at all.
    assert!(parses(
        "int f(void) { const char *s = \"caf\\u00e9\"; return s[3]; }"
    ));
    assert!(parses("int f(void) { return u'\\u00e9'; }"));
    assert!(parses("int f(void) { return (int)U'\\U0001F600'; }"));
    refuses(
        "int f(void) { const char *s = \"caf\\uD800\"; return s[0]; }",
        "escape this compiler does not read",
    );
    // C99 6.4.3p2's third rule: a character below `0xA0` is spelled as itself, so a universal
    // character name may not name one - except `$`, `@` and the backtick. `clang` says "character
    // 'A' cannot be specified by a universal character name" for the first and accepts the rest.
    refuses(
        "int f(void) { const char *s = \"\\u0041\"; return s[0]; }",
        "escape this compiler does not read",
    );
    refuses(
        "int f(void) { unsigned short *s = L\"\\u0042\"; return s[0]; }",
        "escape this compiler does not read",
    );
    assert!(parses(
        "int f(void) { const char *s = \"\\u0024\\u0040\\u0060\\u00a0\"; return s[0]; }"
    ));
}

#[test]
fn the_address_of_a_function_designator_is_the_designator() {
    // C99 6.5.3.2p3 - but only when the designator is the *whole* operand. `&f(x)->m` is the
    // address of a member of the call's result; taking the shortcut there made it `(&f)(x)->m`.
    // lz4frame.c writes exactly that, and found this.
    assert!(parses(
        "struct j { int done; };
static struct j *make(int n) { (void)n; return 0; }
         static void wait(const int *flag) { (void)flag; }
         int f(void) { wait(&make(1)->done); return 0; }"
    ));
    assert!(parses(
        "static void panic(void) { }
static void handover(void (*f)(void)) { f(); }
         int f(void) { handover(&panic); return 0; }"
    ));
    // `&f` where `f` is the whole operand is the designator, and `&f()` is the address of an
    // rvalue, which C does not have.
    assert!(parses(
        "static void g(void) { }
int f(void) { void (*p)(void) = &g; p(); return 0; }"
    ));
    refuses(
        "struct j { int d; };
static struct j *m(int n) { (void)n; return 0; }
         int f(void) { int *p = &m(1); (void)p; return 0; }",
        "needs something it can take the address of",
    );
}

#[test]
fn a_comment_on_a_directive_line_does_not_eat_the_next_line() {
    // A `//` comment swallows its own newline, and a newline is what says the next token begins a
    // logical line - which is what a directive reads until. Without the fix, `#define A 1 // c`
    // took the rest of the file as its body, and the line after any `//` comment was invisible to
    // `#ifdef`. Every shape here was measured: stb_image.h is written the first way.
    assert!(parses(
        "#define A 1 // trailing
#ifdef A
int x;
#endif
int y;
"
    ));
    assert!(parses(
        "#define A 1 // c
int x;
"
    ));
    assert!(parses(
        "#define A 1
#undef A  // c
int x;
"
    ));
    assert!(parses(
        "#if 1
int x;
#endif  // closes
int y;
"
    ));
    assert!(parses(
        "#if 0
#include <math.h>  // c
#endif
int x;
"
    ));
}

#[test]
fn a_struct_initialiser_may_end_with_a_comma() {
    // C99 6.7.8p1: a trailing comma in a brace list is allowed. A struct filled exactly had its
    // comma read as a fourth value - "too many initialisers" - because the capacity check ran
    // before the loop ate the separator. Found by stb_image.h, whose callbacks table is written
    // with the comma every C programmer writes.
    assert!(parses(
        "typedef struct c { int (*read)(void *); int (*eof)(void *); } c;         static int r(void *u) { return 0; } static int e(void *u) { return 1; }         static c mine = { r, e, };
int main(void) { return mine.read(0); }"
    ));
    // And a struct given *more* values than it has members is still refused.
    refuses(
        "struct s { int a; int b; }; static struct s x = { 1, 2, 3 };
int main(void) { return x.a; }",
        "too many initialisers",
    );
}

#[test]
fn a_string_literal_subscript_is_an_expression_and_not_an_initialiser() {
    // C99 6.7.8p14 makes `char s[] = "abc"` an initialiser of a character array. A literal that is
    // *subscripted* is not one: `unsigned a = "\x02\x03"[i];` has the value of a character, and the
    // initialiser path took the literal, dropped the `[i]` and then said a `char *` did not fit an
    // `unsigned`. miniz's deflate is written that way.
    assert!(parses(
        r#"unsigned f(int i) { unsigned a = "\x02\x03\x07"[i]; return a; }"#
    ));
    assert!(
        parses(r#"char *g(void) { char *p = "abc"; return p; }"#),
        "a pointer still decays"
    );
    assert!(
        parses(r#"char h(void) { char s[] = "abc"; return s[1]; }"#),
        "an array still fills"
    );
    assert!(parses(
        r#"unsigned k(void) { unsigned a = "abc"[1]; return a; }"#
    ));
    // The subscript after a literal is a *postfix* expression even with a cast in front of it.
    assert!(parses(r#"int m(void) { return (int)"abc"[0]; }"#));

    // And what the type checker still refuses: a literal is not an initialiser for a non-character
    // array, subscript or not.
    let refused = check(r#"int n(void) { int a[2] = "xy"; return a[0]; }"#);
    assert!(
        !refused.is_empty(),
        "an int array is still not a string's home"
    );
}

#[test]
fn simple_declarations_parse() {
    parses("int x;");
    parses("int x, y, z;");
    parses("unsigned long long big;");
    parses("char *p;");
    parses("char **argv;");
    parses("int a[10];");
    parses("int a[3][4];");
    parses("int open[];");
    parses("double d;");
    parses("long double ld;");
    parses("_Bool flag;");
    parses("const volatile int qualified;");
    parses("int x = 3, y = -2, z = 'a';");
    parses("int *p = 0;");
    parses("char *s = \"text\";");
    // C allows a compatible redeclaration, and two tentative definitions of one object.
    parses("int x; int x;");
    parses("extern int x; extern int x;");
}

#[test]
fn function_declarations_parse() {
    parses("int f(void);");
    parses("int f();");
    parses("int f(int a, char *b);");
    parses("void g(void);");
    parses("int printf(const char *format, ...);");
    parses("int *returns_pointer(int n);");
    parses("int takes_array(int a[10]);");
    parses("int takes_pointer_array(int *a[]);");
}

#[test]
fn typedefs_and_enums_parse() {
    parses("typedef int integer;");
    parses("typedef int integer;\ninteger x;");
    parses("typedef char *string;\nstring s;");
    parses("enum colour { RED, GREEN, BLUE };");
    parses("enum { A = 1, B, C = A + 10 };");
    parses("typedef enum { OFF, ON } flag;\nflag f;");
    parses("enum e { X = 1 };\nint a[X];");
}

// ---- statements ---------------------------------------------------------------

#[test]
fn functions_and_their_bodies_are_read() {
    parses("int f(void) { return 1; }");
    parses("int f(int a) { return a + 1; }");
    parses("void f(void) { }");
    parses("int f(int a, int b) { int s; s = a + b; return s; }");
    parses("int f(void) { return sizeof(int); }");
    parses("int f(void) { return sizeof 1; }");
    parses("int f(void) { return (char)300; }");
}

#[test]
fn control_flow_is_read() {
    parses("int f(int a) { if (a) return 1; else return 0; }");
    parses("int f(int a) { while (a) a = a - 1; return a; }");
    parses("int f(int a) { do { a = a - 1; } while (a); return a; }");
    parses("int f(void) { int s = 0; for (int i = 0; i < 4; i = i + 1) s = s + i; return s; }");
    parses("int f(int a) { for (;;) { break; } return a; }");
    parses("int f(int a) { while (a) { a = a - 1; continue; } return a; }");
    parses("int f(int a) { { int a; } return a; }");
    parses("int f(int a) { int x = 1; x = x + a; int y = x; return y; }");
}

#[test]
fn the_expression_parser_knows_types() {
    parses("int f(int a, int b) { return a * b + 1; }");
    parses("int f(int *p) { return p[1] + *p; }");
    parses("int f(int *p) { return *(p + 2); }");
    parses("int g(int a); int f(void) { return g(3); }");
    parses("int f(int a) { return a ? 1 : 2; }");
    parses("double f(double a) { return a * 2.5; }");
    parses("int f(int a) { a += 2; a -= 1; a *= 3; return a; }");
    parses("int f(char *s) { return s[0]; }");
    parses("int f(int *p) { if (p) return 1; return 0; }");
    parses("int f(int *p) { p = 0; return 1; }");
}

#[test]
fn type_mistakes_are_caught() {
    refuses("int f(void) { return \"text\"; }", "not pointer to char");
    refuses("void f(void) { return 1; }", "returns nothing");
    refuses("int f(void) { return; }", "needs a value");
    refuses("int f(int *p) { return p + p; }", "`+` cannot be applied");
    refuses(
        "int f(void) { int *p; p = 1; return 0; }",
        "cannot assign int to pointer to int",
    );
    refuses("int f(void) { break; }", "`break` outside a loop");
    refuses("int f(void) { continue; }", "`continue` outside a loop");
    refuses("int f(void) { int x; return x(); }", "cannot call int");
    refuses(
        "int g(int a); int f(void) { return g(1, 2); }",
        "this function takes 1 argument, 2 given",
    );
    refuses(
        "int g(int a); int f(void) { return g(); }",
        "this function takes 1 argument, 0 given",
    );
    refuses("int f(void) { return y; }", "`y` is not declared");
    refuses("int f(void) { return 1.0 % 2; }", "`%` cannot be applied");
    refuses(
        "int f(void) { 3 = 4; return 0; }",
        "cannot assign to this expression",
    );
    refuses(
        "int f(void) { int a[2]; return a[1.5]; }",
        "a subscript must be an integer",
    );
}

#[test]
fn constant_expressions_are_evaluated() {
    parses("int a[2 + 3];");
    parses("int a[sizeof(int) * 4];");
    parses("enum { N = 4 };\nint a[N];");
    parses("int a['a' - 'A'];");
    // Every escape C99 has, and a string literal is an array before it is a pointer.
    parses(r"int a['\n' + '\x41' + '\101' + '\a'];");
    parses("char s[] = \"ab\";");
    parses("int a[sizeof \"abc\"];");
}

#[test]
fn mistakes_are_reported_once_with_a_place() {
    refuses("int;", "declaration does not declare anything");
    refuses("x;", "expected a type specifier");
    refuses("int x", "expected `;` after a declaration");
    refuses("int 3x;", "declaration does not declare anything");
    refuses("int x; long long x;", "is already declared in this scope");
    refuses("typedef int T; int T;", "is already declared in this scope");
    refuses("enum e { A, A };", "is already declared in this scope");
    refuses("int f(int a, int a);", "duplicate parameter `a`");
    refuses("int a[sizeof(void)];", "`sizeof` needs a complete type");
    // C99 6.7.5.2p1: a size is greater than zero. Zero is a GNU extension this does not take.
    refuses(
        "int f(void) { int a[-1]; return 0; }",
        "an array's size must be greater than zero",
    );
    refuses(
        "int f(void) { int a[0]; return 0; }",
        "an array's size must be greater than zero",
    );
    // An object past the budget is refused, for a local and a global alike: the back end's frame
    // slot takes a `u32`, and a host compiling unfamiliar code wants a smaller ceiling than that.
    refuses(
        "int f(void) { int a[20000000]; return 0; }",
        "is larger than this compilation's limit",
    );
    refuses(
        "int a[20000000];",
        "is larger than this compilation's limit",
    );
    // The budget is the host's to set, which is what makes it a budget and not a magic number.
    let mut small = Options::default();
    small.budgets.max_object_bytes = 8;
    let complaints = check_with(small, "int a[4];");
    assert!(
        complaints
            .iter()
            .any(|message| message.contains("is larger than this compilation's limit of 8")),
        "{complaints:?}"
    );
    let mut exact = Options::default();
    exact.budgets.max_object_bytes = 8;
    assert!(
        check_with(exact, "int a[2];").is_empty(),
        "eight bytes fits an eight-byte budget"
    );
    // A float is a constant and is not an *integer* constant expression: its bits are not its
    // value, so a size, a `case` label or an enumerator cannot take one.
    refuses(
        "int f(void) { int a[1.5]; return 0; }",
        "expected an integer constant expression, not double",
    );
    refuses(
        "int f(int x) { switch (x) { case 1.5: return 1; } return 0; }",
        "expected an integer constant expression, not double",
    );
    refuses(
        "int f(void) { enum { A = 1.5 }; return A; }",
        "expected an integer constant expression, not double",
    );
    parses("int f(void) { char s[sizeof(double)]; return sizeof s; }");
    // A constant expression is wanted for its value, not its code, so a `?:` in a declarator or
    // a `case` label is read and folded rather than lowered into the middle of the declaration.
    parses("int f(void) { int a[1 ? 2 : 3]; return sizeof a; }");
    parses("int f(int x) { switch (x) { case 1 ? 2 : 3: return 1; } return 0; }");
    parses("int f(void) { enum { A = 1 && 1 ? 2 : 3 }; return A; }");
    // ...and `sizeof`'s operand is not evaluated, so what it contains need not be lowerable.
    parses("int f(void) { int x = 0; return sizeof(x = 3); }");
    parses("struct P { int a; }; int f(struct P *p) { return sizeof(p->a); }");
    // A record parameter is an object in the frame like any other, and `sizeof` reads its type
    // without evaluating anything.
    parses("struct P { int a; }; int f(struct P p) { return sizeof(p.a); }");
    // A value of the wrong type is one diagnostic, not a failed conversion and a missing store
    // as well: the type check runs before anything is written.
    let complaints = check("int f(void) { int a[2] = {1, \"x\"}; return 0; }");
    assert_eq!(complaints.len(), 1, "{complaints:?}");
    assert!(complaints[0].contains("cannot initialise int with pointer to char"));
}

#[test]
fn every_word_that_starts_a_declaration_is_read_as_one() {
    // `starts_specifier` asks a list, and the specifier reader has its own match: this pins
    // them together, so a word added to one and not the other fails here rather than in a
    // puzzling place later.
    for word in [
        "typedef", "extern", "static", "auto", "register", "const", "volatile", "restrict",
        "inline", "void", "_Bool", "char", "short", "int", "long", "float", "double", "signed",
        "unsigned", "struct", "union", "enum",
    ] {
        let complaints = check(&format!("{word} x;"));
        assert!(
            !complaints
                .iter()
                .any(|message| message.contains("expected a declaration")),
            "`{word}` should begin a declaration, but: {complaints:?}"
        );
    }
}

#[test]
fn what_is_not_implemented_is_said_not_guessed() {
    refuses(
        "int a[2][2] = {[1][0] = 1};",
        "a designator that goes deeper than one level is not implemented yet",
    );
    refuses("int a[2] = {[0 ... 1] = 3};", "not implemented yet");
    refuses(
        "int f(void) { asm(\"nop\"); }",
        "`asm` is not supported: this compiler generates no inline assembly",
    );
    // The refusal comes after the arguments are read, so the argument is still checked.
    refuses(
        "int printf(const char *, ...); int f(void) { return printf(1); }",
        "argument 1 is int, expected pointer to char",
    );
}

#[test]
fn an_unevaluated_address_still_obeys_what_address_of_allows() {
    // C99 6.5.3.4p2 says `sizeof` does not evaluate its operand; 6.5.3.2p1 says what `&` may be
    // applied to. The second is a rule about types, so it holds inside `sizeof` too: the first
    // two are refusals clang makes in the same words, and the third is the hole they came from.
    refuses(
        "int f(void) { return (int)sizeof(&5); }",
        "`&` needs something it can take the address of",
    );
    refuses(
        "struct S { int a : 3; }; int f(void) { struct S s; return (int)sizeof(&s.a); }",
        "the address of a bitfield cannot be taken",
    );
    parses("int f(void) { int x = 1; return (int)sizeof(&x); }");
    parses("int f(void) { int b[2][3]; return (int)sizeof(&b[0]); }");
}

#[test]
fn a_variadic_call_passes_both_classes_of_argument() {
    // ADR-006, in its final shape: the call site's own signature makes the call expressible, and a
    // floating-point argument is no longer refused - the *front end* promotes a `float` to a
    // `double` and says nothing more. Which register files that `double` goes into is the back
    // end's business, and level 0/1 refuse it there, at the call, with the reason.
    parses("int p(const char *, ...); int f(void) { return p(\"%d\", 1); }");
    parses("int p(const char *, ...); int f(void) { char c = 1; return p(\"%d\", c); }");
    parses("int p(const char *, ...); int f(void) { return p(\"%f\", 1.5); }");
    // A `float` is promoted to `double` on the way, which is C99 6.5.2.2p6 and not the back end's
    // choice to make.
    parses("int p(const char *, ...); int f(void) { float x = 1.5f; return p(\"%f\", x); }");
}

#[test]
fn an_undeclared_function_is_a_warning_and_its_result_is_an_int() {
    // C89's habit, which MSVC still has: the callee is a function returning `int`, diagnosed once,
    // and the call goes on. The *call site's* arguments are its signature, so `printf("%d", 1)`
    // works without a declaration - which is how most 1990s C on Windows is written.
    let said = warnings("int main(void) { return puts(\"hi\"); }");
    assert!(
        said.iter()
            .any(|message| message.contains("is not declared")),
        "{said:?}"
    );
    parses("int main(void) { return puts(\"hi\"); }");
    // An `int` from anywhere else still needs a cast: the warning is about the *implicit* return.
    refuses(
        "int f(void) { return 1; } int main(void) { char *p; p = f(); return *p; }",
        "cannot assign int to pointer to char",
    );
}

#[test]
fn alignas_is_read_the_way_c11_reads_it() {
    let c11 = Options {
        standard: xcc::CStandard::C11,
        ..Options::default()
    };
    // C11 6.7.5p4: zero has no effect, so it is an operand the reader must accept - in C11, where
    // the word exists at all.
    let complaints = check_with(
        c11.clone(),
        "int f(void) { _Alignas(0) int z = 1; return z; }",
    );
    assert!(complaints.is_empty(), "{complaints:?}");
    // p3: an alignment is a power of two. Three is not a small alignment, it is not an alignment.
    assert!(
        check_with(
            c11.clone(),
            "int f(void) { _Alignas(3) int x = 1; return x; }"
        )
        .iter()
        .any(|message| message.contains("power of two")),
        "a non-power-of-two alignment is refused"
    );
    // p6: an implementation may decline an extended alignment - and this one says which it declined
    // rather than quietly aligning to sixteen.
    assert!(
        check_with(c11, "int f(void) { _Alignas(32) int x = 1; return x; }")
            .iter()
            .any(|message| message.contains("extended alignment")),
        "an extended alignment is refused, with the number"
    );
}

#[test]
fn a_declaration_can_begin_with_every_specifier() {
    // The list of words that start a declaration (`SPECIFIERS`) and the reader that accepts them are
    // two places knowing one thing, and nothing used to say so: `_Alignas(16) int x;` inside a
    // function was read as a *call* to a function called `_Alignas`, because the list decides whether
    // a statement is a declaration. Every word is tried at block scope here, which is the direction
    // that fails when the two drift apart.
    let c11 = Options {
        standard: xcc::CStandard::C11,
        ..Options::default()
    };
    let declarations = [
        "typedef int T;",
        "extern int e;",
        "static int s;",
        "auto int a;",
        "register int r;",
        "const int c = 1;",
        "volatile int v;",
        "restrict int re;",
        "inline int in;",
        "void *p;",
        "_Bool b;",
        "char ch;",
        "short sh;",
        "int i;",
        "long l;",
        "float f;",
        "double d;",
        "signed int si;",
        "unsigned int ui;",
        "struct S { int member; };",
        "union U { int member; };",
        "enum E { A };",
        "_Atomic int atom;",
        "_Alignas(16) int aligned;",
    ];
    for declaration in declarations {
        let source = format!("int f(void) {{ {declaration} return 0; }}");
        let complaints = check_with(c11.clone(), &source);
        assert!(
            complaints.is_empty(),
            "{declaration:?} should begin a declaration, but: {complaints:?}"
        );
    }
}

#[test]
fn c11_is_a_dialect_you_ask_for() {
    // `_Static_assert` and `_Generic` are C11's, and the compiler says so instead of pretending the
    // words were never declared: the note names the flag.
    refuses(
        "_Static_assert(sizeof(int) == 4, \"32-bit int\");",
        "`_Static_assert` is C11: compile with `--std=c11`",
    );
    refuses(
        "int f(void) { return _Generic(1, int: 1, default: 0); }",
        "`_Generic` is C11: compile with `--std=c11`",
    );
    // And with the dialect, both parse - and a false assertion is an error with the source's own
    // message, which is the whole point of the feature.
    let c11 = Options {
        standard: xcc::CStandard::C11,
        ..Options::default()
    };
    let complaints = check_with(c11.clone(), "_Static_assert(1 == 2, \"one is not two\");");
    assert!(
        complaints
            .iter()
            .any(|message| message.contains("one is not two")),
        "{complaints:?}"
    );
    let complaints = check_with(
        c11,
        "int f(void) { return _Generic(1, int: 42, default: 0); }",
    );
    assert!(
        complaints.iter().all(|message| !message.contains("error")),
        "{complaints:?}"
    );
}

#[test]
fn an_identifier_has_one_definition() {
    // C99 6.9p5. The back end treats a second symbol as an internal error, so the front end has to
    // say it in C's words - and a tentative definition is not one yet.
    refuses("int x = 1; int x = 2;", "redefinition of `x`");
    refuses(
        "int f(void) { return 1; } int f(void) { return 2; }",
        "redefinition of `f`",
    );
    // A prototype between two bodies does not make the second one new.
    refuses(
        "int f(void) { return 1; } int f(void); int f(void) { return 2; }",
        "redefinition of `f`",
    );
    // And these are one object each, which C allows.
    parses("int f(void); int f(void) { return 1; }");
    parses("int x; int x;");
    parses("int x; int x = 1;");
    parses("int x = 1; int x;");
}

#[test]
fn what_aggregates_still_cannot_do() {
    // A bitfield is read, written, stepped and initialised now: what is left in the aggregate
    // path is a struct value inside a brace list, and a replacement of the storage unit itself.
    refuses(
        "struct P { int x, y; }; struct Q { struct P p; int n; };
         int f(struct P v) { struct Q q = {v}; return q.n; }",
        "cannot initialise int with struct P",
    );
    // C99 6.5.3.2p1: the only address a bitfield has is its whole unit's, so `&` refuses.
    refuses(
        "struct S { unsigned a : 3; }; int f(void) { struct S s; return (int)&s.a; }",
        "the address of a bitfield cannot be taken",
    );
    // An aggregate *can* now be carried through a join - the arms are already addresses and the
    // join carries one - so this asserts the answer rather than the refusal it used to. The
    // refusal was right about `cir` having no register for an aggregate and wrong about there being
    // nothing else to carry.
    parses(
        "struct P { int x; }; int f(int c) { struct P a = {1}; struct P b = {2}; return (c ? a : b).x; }",
    );
    // ...but it does travel by value across a call, on the ABI's terms (see the layout test and
    // the ABI test in `back`).
    parses("struct P { int x; }; int sink(struct P p) { return p.x; } int f(void) { struct P a = {3}; return sink(a); }");
    parses("struct P { int x; }; struct P make(int v) { struct P r; r.x = v; return r; } int f(void) { struct P p; p = make(1); return p.x; }");
    refuses(
        "struct Incomplete; int f(void) { struct Incomplete s; return 0; }",
        "has no size",
    );
    // A bitfield is merged into its unit now, so nine in a three-bit field is one, as C stores it.
    parses("struct S { unsigned a : 3; }; int f(void) { struct S s = {9}; return s.a; }");
    // A union takes one member's initialiser: a second is one error, not a second store over the
    // same bytes.
    refuses(
        "union U { int i; char c; }; int f(void) { union U u = {1, 2}; return 0; }",
        "a union takes one initialiser",
    );
}

#[test]
fn only_an_object_has_storage() {
    // C99 6.7.1p2: `void` is not an object type. A local, a global and a parameter each say so;
    // none of them used to reach the back end and come back as an internal error.
    refuses(
        "int f(void) { void v; return 0; }",
        "a variable cannot have type void",
    );
    refuses(
        "void v; int f(void) { return 0; }",
        "a variable cannot have type void",
    );
    refuses(
        "int f(void v) { return 0; }",
        "a parameter cannot have type void",
    );
    // A `_Bool` *value* is an integer, not a null pointer constant: only a constant zero is.
    refuses(
        "int f(void) { int *p; _Bool b = 1; p = b; return 0; }",
        "cannot assign _Bool to pointer to int",
    );
}

#[test]
fn a_variable_needs_a_complete_type() {
    refuses(
        "int f(void) { int a[]; return 0; }",
        "an array with no size cannot be a variable",
    );
    // An initialiser is what gives the array a size, so there is nothing wrong with it.
    parses("int f(void) { char s[] = \"abc\"; return s[0]; }");
    parses("int f(void) { int a[] = {1, 2, 3}; return a[2]; }");
    // A float has a value now, so its initialiser is a store like any other.
    parses("int f(void) { double d = 1.5; return 0; }");
    refuses(
        "int f(void) { int x = \"abc\"; return x; }",
        "cannot initialise int with pointer to char",
    );
    // A pointer to a string literal at file scope is an address, which level 2 writes as a
    // relocation. Level 0 is what `check` compiles with, and it refuses rather than leaving the
    // pointer null - which is what it silently was before either existed.
    refuses(
        "char *p = \"abc\"; int f(void) { return 0; }",
        "an initialiser that holds an address",
    );
    // The front end says nothing about it at either level: it wrote the bytes and the relocation,
    // and which lowerings can honour them is their own business.
    parses("char *p = \"abc\"; char *t[] = {\"a\", \"b\"};");
    parses("int a[] = {1, 2, 3}; struct { int x; } s = {4};");
}

#[test]
fn switch_and_goto_are_read() {
    parses("int f(int a) { switch (a) { case 1: return 0; default: return 1; } }");
    parses("int f(int a) { switch (a) { case 1: case 2: return 3; } return 0; }");
    parses("int f(int a) { switch (a) { } return a; }");
    parses("int f(int a) { switch (a) { case 'x': break; } return a; }");
    parses("int f(int a) { switch (a) { default: break; } return a; }");
    parses("int f(int a) { while (a) { switch (a) { case 1: break; } break; } }");
    parses("int f(int a) { switch (a) { case 1: { int b = 2; return b; } } }");
    parses("int f(void) { goto done; done: return 0; }");
    parses("int f(void) { done: return 0; }");
    parses("int f(int a) { if (a) goto out; return 1; out: return 0; }");
    parses("int f(void) { for (;;) { if (1) goto done; } done: return 0; }");
    // A designator moves the cursor and the list carries on from there (C99 6.7.8p17).
    parses("int a[5] = {[2] = 7};");
    parses("int a[5] = {1, [3] = 4};");
    parses("struct P { int x; int y; }; struct P p = {.y = 2, .x = 1};");
    parses("struct P { int x; int y; }; struct P p = {.x = 1, 2};");
    parses("union U { int i; char c; }; union U u = {.c = 1};");
    parses("struct P { int x; int y; }; struct P p = {.y = {2}};");
}

#[test]
fn switch_and_goto_mistakes_are_caught() {
    refuses(
        "int f(int a) { case 1: return a; }",
        "`case` outside a `switch`",
    );
    refuses(
        "int f(int a) { default: return a; }",
        "`default` outside a `switch`",
    );
    refuses(
        "int f(int a) { switch (a) { case 1: return 1; case 1: return 2; } }",
        "already has a case for 1",
    );
    refuses(
        "int f(int a) { switch (a) { default: return 1; default: return 2; } }",
        "already has a `default`",
    );
    refuses(
        "int f(int *p) { switch (p) { } }",
        "`switch` needs an integer",
    );
    refuses(
        "int f(void) { break; }",
        "`break` outside a loop or `switch`",
    );
    refuses(
        "int f(int a) { switch (a) { case 1: continue; } }",
        "`continue` outside a loop",
    );
    refuses("int f(void) { goto nowhere; }", "no label `nowhere`");
    refuses("int f(void) { d: d: return 0; }", "already has a label `d`");
    refuses(
        "struct P { int x; }; struct P p = {.z = 1};",
        "no member `z`",
    );
    refuses("int a[2] = {[5] = 1};", "past the end");
    refuses(
        "int a = 'ab';",
        "not a character constant this compiler reads",
    );
    refuses(
        r"int a = '\q';",
        "not a character constant this compiler reads",
    );
    // C99 6.7.8p14 and 6.7.8p2: too long is a constraint violation, so it is diagnosed - and the
    // objects are still written, which is what every compiler does with it in the wild. A *warning*
    // is the difference between "look at this" and "no object".
    assert!(
        warnings("char s[2] = \"abc\";")
            .iter()
            .any(|message| message.contains("cannot hold this 3-character string")),
        "a too-long string is warned about"
    );
    parses("char s[2] = \"ab\";");
    // The literal's own character type is what an array must have: `int` is not one, and a wide
    // literal does not fill an array of `char`.
    refuses(
        "int a[] = \"abc\";",
        "initialises an array of its own character type",
    );
    refuses(
        "char a[] = L\"abc\";",
        "initialises an array of its own character type",
    );
    refuses(
        "int a = \"x\";",
        "cannot initialise int with pointer to char",
    );
}

#[test]
fn aggregates_and_function_pointers_are_read() {
    parses("struct S { int a; int b; };");
    parses("struct S { int a; }; struct S s;");
    parses("struct S { int a; }; struct S *p; int f(void) { return p->a; }");
    parses("struct S { int a; }; int f(struct S s) { return s.a; }");
    parses("struct S; struct S *p; struct S { int a; };");
    parses("struct S { struct S *next; int v; };");
    parses("union U { int i; char c[4]; }; union U u;");
    parses("struct S { int a; }; struct S s = {1};");
    parses("struct S { int a; int b; }; struct S s = {1, 2};");
    parses("int a[3] = {1, 2, 3};");
    parses("int a[2][2] = {{1, 2}, {3, 4}};");
    parses("int a[2][2] = {1, 2, 3, 4};");
    parses("char s[] = \"abcd\";");
    parses("int a[] = {1, 2, 3};");
    parses("int (*fp)(int);");
    parses("int f(int a) { return a; } int (*fp)(int) = f;");
    parses("int (*fp)(int); int f(int a) { return (*fp)(a); }");
    parses("int (*table[2])(int);");
    parses("struct P { int x; int y; }; int f(void) { struct P p = {1, 2}; return p.x + p.y; }");
    parses("struct P { int x; }; int f(void) { return ((struct P){5}).x; }");
    parses("struct S { unsigned a : 3; unsigned b : 5; };");
    parses("struct S { int a; }; int f(void) { return sizeof(struct S); }");
}

#[test]
fn aggregate_mistakes_are_caught() {
    refuses(
        "struct S { int a; }; int f(void) { return s.a; }",
        "`s` is not declared",
    );
    refuses(
        "struct S { int a; }; int f(struct S s) { return s.b; }",
        "`b` is not a member",
    );
    refuses("int f(void) { int x; return x.a; }", "has no members");
    refuses(
        "struct S { int a; }; int f(struct S *p) { return p.a; }",
        "has no members",
    );
    // Excess initialisers are the same shape of mistake, and take the same answer.
    assert!(
        warnings("int a[2] = {1, 2, 3};")
            .iter()
            .any(|message| message.contains("more were given")),
        "an excess initialiser is warned about"
    );
    refuses(
        "struct S { int a; }; struct S s = {1, 2};",
        "too many initialisers",
    );
    refuses(
        "int a[2] = {1, 2}; int f(void) { return a[0.5]; }",
        "a subscript must be an integer",
    );
}

#[test]
fn a_bad_declaration_does_not_cascade() {
    // One malformed type specifier, not a complaint per declarator.
    let complaints = check("int x; ; int y;");
    assert!(complaints.len() <= 3, "too many complaints: {complaints:?}");
}

/// Programs in the corpus that a conforming C99 compiler must reject: an implicit function
/// declaration, Borland's `word`, the address of something that is not an lvalue, a call with
/// the wrong number of arguments, a call to a function that is not declared yet. Each is a
/// reason, not a count.
const EXPECTED_INVALID: &[(&str, &str)] = &[
    (
        "p6_printf.c",
        "calls printf with no declaration: C89 implicit declaration",
    ),
    ("p6_printf2.c", "same, with two calls"),
    ("p6_memset.c", "calls memset with no declaration"),
    ("p6_memcmp.c", "calls memcmp with no declaration"),
    ("p6_malloc.c", "calls malloc with no declaration"),
    ("p6_file.c", "calls read with no declaration"),
    ("cc18_printf05.c", "calls printf with no declaration"),
    ("cc18_printfX.c", "calls printf with no declaration"),
    ("cc21_wordloc.c", "uses Borland's `word`, which is not C"),
    ("cc26_word.c", "uses Borland's `word`, which is not C"),
    (
        "pp_badaddrof.c",
        "takes the address of something that is not an lvalue",
    ),
    ("pp_badaddrof2.c", "same"),
    (
        "cc9_badargs.c",
        "calls a one-argument function with two arguments",
    ),
    (
        "cc9_badargs2.c",
        "calls a function that is declared later, with the wrong count",
    ),
    (
        "cc9_mutual.c",
        "calls `odd` before it is declared: C99 has no implicit declarations",
    ),
    ("p6_exit.c", "calls `exit` with no declaration"),
    (
        "cc_bad_addrassign.c",
        "assigns through `&`, which is not an lvalue",
    ),
    ("pp_badassign.c", "same"),
    ("cc_bad_dblshift.c", "shifts a `double`"),
    (
        "cc16_strarray.c",
        "assigns to an array, which C99 6.5.16 does not allow",
    ),
    ("cc16_strarray2.c", "same, at file scope"),
    (
        "cc16_strarray3.c",
        "same, to a row of a two-dimensional array",
    ),
    (
        "pp_badarrinit.c",
        "gives a three-element array four initialisers",
    ),
    ("pp_badnestedinit.c", "gives a `[2][3]` array three rows"),
    (
        "pp_badarrinit2.c",
        "puts a three-byte string in a two-byte array",
    ),
    ("pp_dynwidth.c", "calls printf with no declaration"),
    ("pp_npercent.c", "calls printf with no declaration"),
    ("pp_pptr.c", "calls printf with no declaration"),
    ("pp_prtflen.c", "calls printf with no declaration"),
    ("pp_s.c", "calls printf with no declaration"),
    ("stress.c", "calls printf with no declaration"),
    ("stress2.c", "calls printf with no declaration"),
    ("pp_fdreuse.c", "calls open and close with no declaration"),
    (
        "pp_mread.c",
        "calls open, read, close and malloc with no declaration",
    ),
    (
        "cc48_bitfield_layout.c",
        "struct bitfield layout: refused as M3.3, then nothing else",
    ),
];

#[test]
fn every_corpus_program_is_read_without_panicking_or_surprising_itself() {
    let corpus = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus");
    let mut files = 0;
    let mut parsed = 0;
    let mut unexpected: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(corpus).expect("the corpus is in the repository") {
        let path = entry.expect("an entry").path();
        if path.extension().is_none_or(|extension| extension != "c") {
            continue;
        }
        let name = path
            .file_name()
            .expect("a file name")
            .to_string_lossy()
            .into_owned();
        let source = std::fs::read_to_string(&path).expect("a UTF-8 source");
        files += 1;
        if source.contains("#include") {
            continue;
        }
        if EXPECTED_INVALID.iter().any(|(known, _)| *known == name) {
            continue; // correct C99 behaviour on a program that is not C99
        }
        let complaints = check(&source);
        if complaints.iter().all(|message| {
            message.contains("code generator") || message.contains("not lowered yet")
        }) {
            parsed += 1;
        }
        // Every refusal must be a *known* one: something M3 has not built yet. Anything else
        // would be a parse that went wrong on real C.
        //
        // This leans on the wording of the message: renaming "not implemented yet (M3." would
        // break this test rather than the compiler. A `Diagnostic` code would be sturdier, and
        // is worth adding the first time it is needed for anything else.
        for complaint in &complaints {
            let known = complaint.contains("not implemented yet (M3.")
                || complaint.contains("not lowered yet")
                || complaint.contains("code generator")
                || complaint.contains("further diagnostics not shown");
            if !known {
                unexpected.push(format!("{name}: {complaint}"));
            }
        }
    }
    assert!(files > 400, "expected the corpus, found {files}");
    assert!(
        unexpected.is_empty(),
        "the corpus was read but refused for reasons that are not \"not built yet\":
{}",
        unexpected.join(
            "
"
        )
    );
    // M3.1 reads declarations; every corpus program also has a function body, so the number
    // that parse completely is a progress bar for M3.2, not a gate.
    eprintln!("parse gate: {parsed} of {files} corpus programs are read completely");
}

/// **One mistake, one diagnostic.** An expression that has been diagnosed is the error type, and
/// the error type is compatible with everything - that is what it is for - so every later question
/// about it ("has this a value?", "is it lowered?") has to see poison and stay quiet. It did not: a
/// poisoned initialiser produced a second complaint about a second thing, which sends a reader
/// looking for a second mistake.
#[test]
fn a_poisoned_expression_is_diagnosed_once() {
    // `1 + "text"` is pointer arithmetic and legal C; dereferencing an integer is not.
    let messages = check("int main(void) { int x = *1; return 0; }");
    assert_eq!(messages.len(), 1, "{messages:?}");

    // ... and the same for an intrinsic whose argument is the wrong type.
    let messages = check(
        "int main(void) { int x = 1; char e = 1;         return __builtin_atomic_compare_exchange(&x, &e, 2); }",
    );
    assert_eq!(messages.len(), 1, "{messages:?}");
}

/// `_Alignas` above sixteen is a promise about the **frame**, not about the data section:
/// `xxhash` aligns its accumulators to 64 at file scope and was refused.
#[test]
fn an_alignment_above_sixteen_belongs_to_static_storage() {
    let c11 = Options {
        standard: xcc::CStandard::C11,
        ..Options::default()
    };
    // A file-scope object is placed by the object writer, where 64 is an offset like any other.
    assert!(check_with(
        c11.clone(),
        "_Alignas(64) int counter;
int main(void) { return counter; }"
    )
    .is_empty());
    // An automatic one is placed by the prologue, which aligns the frame to sixteen - the ABI's
    // number - so the refusal names that and says what to do.
    let refused = check_with(
        c11,
        "int main(void) { _Alignas(64) int local = 0; return local; }",
    );
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert!(
        refused[0].contains("static storage duration"),
        "{refused:?}"
    );
}
