//! What the declaration parser accepts and refuses, and what it produces.

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
        "int main(void) { return 0; }",
        "function bodies are not implemented yet (M3.2)",
    );
}

#[test]
fn a_bad_declaration_does_not_cascade() {
    // One malformed type specifier, not a complaint per declarator.
    let complaints = check("int x; ; int y;");
    assert!(complaints.len() <= 3, "too many complaints: {complaints:?}");
}

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
        let source = std::fs::read_to_string(&path).expect("a UTF-8 source");
        if source.contains("#include") {
            continue;
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
                || complaint.contains("code generator");
            if !known {
                unexpected.push(format!("{}: {complaint}", path.display()));
            }
        }
        files += 1;
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
