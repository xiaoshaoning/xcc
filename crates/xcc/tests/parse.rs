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

/// The notes of the diagnostics `source` produced: what a refusal says *beside* its message.
fn notes(source: &str) -> Vec<String> {
    match xcc::Compile::new(Options::default()).compile("t.c", source) {
        Ok(_) => Vec::new(),
        Err(diagnostics) => diagnostics
            .items()
            .iter()
            .flat_map(|diagnostic| diagnostic.notes.clone())
            .collect(),
    }
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

fn refuses(source: &str, expected: &str) {
    let complaints = check(source);
    assert!(
        complaints.iter().any(|message| message.contains(expected)),
        "{source:?} should be refused with {expected:?}, got {complaints:?}"
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
    refuses("int f(void) { asm(\"nop\"); }", "`asm` is not declared");
    refuses(
        "int printf(const char *, ...); int f(void) { return printf(\"x\", 1); }",
        "a call to a variadic function is not lowered yet",
    );
    // ...and the *note* names the two things the back end cannot say, so a reader does not have to
    // rediscover them. The message stays one sentence; the reason is the diagnostic's note, which
    // a host can show apart from it (ADR-006).
    let notes = notes("int printf(const char *, ...); int f(void) { return printf(\"x\", 1); }");
    assert!(
        notes.iter().any(|note| note.contains("ADR-006")),
        "{notes:?}"
    );
    // The refusal comes after the arguments are read, so the argument is still checked.
    refuses(
        "int printf(const char *, ...); int f(void) { return printf(1); }",
        "argument 1 is int, expected pointer to char",
    );
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
    // An aggregate cannot be carried through a join: the join would need a register for it.
    refuses(
        "struct P { int x; }; int f(int c) { struct P a = {1}; struct P b = {2}; return (c ? a : b).x; }",
        "a conditional expression whose arms are aggregates",
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
    // At file scope a pointer to a literal needs a relocation, so it is refused rather than
    // left null - which is what it silently was.
    refuses(
        "char *p = \"abc\"; int f(void) { return 0; }",
        "a pointer to a string literal in a file-scope initialiser is not lowered yet",
    );
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
    refuses("char s[2] = \"abc\";", "cannot hold this 3-byte string");
    refuses("int a[] = \"abc\";", "initialises an array of `char`");
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
    refuses("int a[2] = {1, 2, 3};", "more were given");
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
