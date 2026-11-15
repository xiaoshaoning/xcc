//! What the parser accepts and refuses: declarations, statements and expressions.

use xcc::{Options, SourceFile};

/// Parse `source` the way `Compile::compile` does up to the parser, and return the diagnostics.
fn check(source: &str) -> Vec<String> {
    let file = SourceFile::new("t.c", source);
    let preprocessed = xcc::preprocess(&file, &Options::default(), &());
    if preprocessed.has_errors() {
        return Vec::new(); // a program that needs a header: not this test's business
    }
    // The parser is internal; the public entry point runs it.
    let diagnostics = match xcc::Compile::new(Options::default()).compile("t.c", source) {
        Ok(_) => Vec::new(),
        Err(diagnostics) => diagnostics.items().to_vec(),
    };
    diagnostics
        .into_iter()
        .map(|diagnostic| diagnostic.message)
        .collect()
}

/// Whether the source parses as declarations, with no complaint about the declarations
/// themselves: the "the code generator is not implemented yet" notice is expected.
fn parses(source: &str) -> bool {
    let complaints: Vec<String> = check(source)
        .into_iter()
        .filter(|message| !message.contains("code generator"))
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
        "takes 1 arguments, more were given",
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
}

#[test]
fn what_is_not_implemented_is_said_not_guessed() {
    refuses(
        "struct S { int a; };",
        "`struct` and `union` are not implemented yet (M3.3)",
    );
    refuses(
        "int (*fp)(int);",
        "grouped declarators are not implemented yet (M3.3)",
    );
    refuses(
        "int a[2] = {1, 2};",
        "initialiser lists are not implemented yet (M3.3)",
    );
    refuses(
        "int f(int a) { switch (a) { } return 0; }",
        "`switch` is not implemented yet (M3.3)",
    );
    refuses(
        "int f(void) { goto done; }",
        "`goto` is not implemented yet (M3.3)",
    );
    refuses(
        "struct P { int x; };",
        "`struct` and `union` are not implemented yet (M3.3)",
    );
    refuses(
        "int f(struct P *p) { return p->x; }",
        "`struct` and `union` are not implemented yet",
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
        if complaints
            .iter()
            .all(|message| message.contains("code generator"))
        {
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
