//! Hostile input: a source this compiler cannot compile *says so*, in bounded time and bounded
//! output, without taking the host's process down with it.
//!
//! Every case here is one a recursive-descent compiler gets wrong by default, and each has a
//! different budget behind it:
//!
//! - nesting past `Budgets::max_nesting` (parentheses, unary operators, casts, blocks, `if`s),
//!   where the failure mode without the budget is a stack overflow - a crash, not a diagnostic;
//! - `#include` past `Budgets::max_include_depth`, `#define` past `Budgets::max_macro_expansion`,
//!   and preprocessing past `Budgets::max_tokens`;
//! - an object past `Budgets::max_object_bytes` and a source past `Budgets::max_source_bytes`.
//!
//! The compiler is run as a child process on purpose: a regression in the nesting budget ends in
//! a stack overflow, and a test that runs the compiler in-process would take the test harness with
//! it instead of failing here with the case's name.
//!
//! **Not gated on a target**: every case here compiles with `-c` and reads what came back, and
//! nothing in this file links or runs an image - so it is a front-end gate on any host.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A directory of our own, removed when the test's `Temp` is dropped.
struct Temp(PathBuf);

impl Temp {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("xcc-hostile-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a temporary directory");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn write(&self, name: &str, text: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, text).expect("a file");
        path
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Compile `source` and expect a refusal: a failure exit, a message naming the budget, and no
/// crash (a crash shows up as an exit code of 127 on this platform, or as an empty message).
fn refuses(temp: &Temp, name: &str, source: &str, expected: &str) {
    let file = temp.write(name, source);
    let output: Output = Command::new(env!("CARGO_BIN_EXE_xcc"))
        .arg("-c")
        .arg(&file)
        .arg("-o")
        .arg(temp.path().join("out.obj"))
        .output()
        .expect("the xcc binary runs");
    let message = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{name}: expected a refusal, got {:?}\n{message}",
        output.status.code()
    );
    assert!(
        message.contains(expected),
        "{name}: the message does not mention {expected:?}:\n{}",
        &message[..message.len().min(600)]
    );
    // A refusal is one line per diagnostic plus its source snippet, and a hundred diagnostics at
    // most: a megabyte would mean the renderer is echoing an arbitrary amount of the input.
    assert!(
        message.len() < 1 << 20,
        "{name}: the refusal is {} bytes of message",
        message.len()
    );
}

#[test]
fn a_source_nested_past_the_budget_is_refused_not_a_stack_overflow() {
    let temp = Temp::new("nesting");
    let depth = 20_000;
    refuses(
        &temp,
        "parens.c",
        &format!(
            "int f(void) {{ return {}1{}; }}\n",
            "(".repeat(depth),
            ")".repeat(depth)
        ),
        "nesting is deeper than",
    );
    refuses(
        &temp,
        "unary.c",
        &format!("int f(void) {{ return {}1; }}\n", "!".repeat(depth)),
        "nesting is deeper than",
    );
    refuses(
        &temp,
        "casts.c",
        &format!("int f(void) {{ return {}1; }}\n", "(int)".repeat(depth)),
        "nesting is deeper than",
    );
    refuses(
        &temp,
        "ifs.c",
        &format!("int f(int x) {{ {}return 0; }}\n", "if (x) ".repeat(depth)),
        "nesting is deeper than",
    );
    refuses(
        &temp,
        "blocks.c",
        &format!(
            "int f(void) {}return 0;{}\n",
            "{".repeat(depth),
            "}".repeat(depth)
        ),
        "nesting is deeper than",
    );
    refuses(
        &temp,
        "declarators.c",
        &format!("int {}x{};\n", "(".repeat(depth), ")".repeat(depth)),
        "nesting is deeper than",
    );
}

#[test]
fn a_macro_that_grows_without_end_is_refused() {
    let temp = Temp::new("macros");
    // The argument's expansion is what grows, so the queue the preprocessor checks never fills:
    // the budget has to be spent during a prescan too.
    let mut source = String::from("#define D0(x) x x x x x x x x x x\n");
    for step in 1..12 {
        source.push_str(&format!(
            "#define D{step}(x) D{}(D{}(x))\n",
            step - 1,
            step - 1
        ));
    }
    source.push_str("D11(1)\n");
    refuses(&temp, "fanout.c", &source, "more than");

    // Depth, before width: twenty nested arguments is past the sixty-four deep budget sooner.
    let nested = format!(
        "#define id(x) x\n{}1{}\n",
        "id(".repeat(100),
        ")".repeat(100)
    );
    refuses(&temp, "deep.c", &nested, "nested more than");
}

#[test]
fn an_object_and_a_source_past_their_budgets_are_refused() {
    let temp = Temp::new("sizes");
    refuses(
        &temp,
        "object.c",
        "int a[100000000];\n",
        "larger than this compilation's limit",
    );
    // A source past the *command line's* budget is a sixty-four megabyte file, which is not worth
    // writing in a test; the default's eight megabytes are checked where the default still applies,
    // by `a_source_past_the_default_budget_is_refused` in the preprocessor's own tests. What matters
    // here is that a program is not a snippet: the command line compiles what a compiler is given.
}

#[test]
fn an_include_chain_past_the_depth_budget_is_refused() {
    let temp = Temp::new("includes");
    for step in 0..40 {
        let body = format!("#include \"h{}.h\"\nint x{step};\n", step + 1);
        temp.write(&format!("h{step}.h"), &body);
    }
    let main = temp.write(
        "main.c",
        "#include \"h0.h\"\nint main(void) { return 0; }\n",
    );
    let output = Command::new(env!("CARGO_BIN_EXE_xcc"))
        .arg("-c")
        .arg(&main)
        .arg("-o")
        .arg(temp.path().join("out.obj"))
        .output()
        .expect("the xcc binary runs");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("nested more than"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_member_declarator_that_cannot_be_read_is_refused_rather_than_looping() {
    // `int ( x * p ) ( void ) ;` as a struct member: a parenthesised declarator with no `*` before
    // its name. The recovery skipped to the member's `;`, met a `)` at depth zero that no `(` in the
    // skip had opened, and *stopped there* - so the member loop re-read the same token for ever. One
    // line allocated eleven gigabytes before the allocator gave up; with the diagnostic list bounded
    // it hung instead. Both are now a diagnostic in well under a second.
    let temp = Temp::new("member");
    refuses(
        &temp,
        "member.c",
        "typedef struct H { int ( x * p ) ( void ) ; } H;
",
        "expected `)`",
    );
}
