//! What the command line does, end to end: exit codes, streams and formats.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A directory of our own, removed when the test's `Temp` is dropped.
struct Temp(PathBuf);

impl Temp {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("xcc-cli-{name}-{}", std::process::id()));
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

fn xcc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_xcc"))
}

fn run(command: &mut Command) -> Output {
    command.output().expect("the xcc binary runs")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn version_says_who_it_is() {
    let output = run(xcc().arg("--version"));
    assert!(output.status.success());
    assert!(stdout(&output).starts_with("xcc "), "{}", stdout(&output));
}

#[test]
fn help_is_offered() {
    let output = run(xcc().arg("--help"));
    assert!(output.status.success());
    let text = stdout(&output);
    assert!(text.contains("Usage"), "{text}");
    assert!(text.contains("--format"), "{text}");
}

#[test]
fn diagnostics_go_to_stderr_in_the_asked_for_format() {
    let temp = Temp::new("format");
    let source = temp.write("t.c", "int x = ;\n");
    for (format, expected) in [
        ("msvc", "t.c(1,1): error: the parser is not implemented yet"),
        ("gcc", "t.c:1:1: error: the parser is not implemented yet"),
    ] {
        let output = run(xcc()
            .current_dir(temp.path())
            .args(["-c", "--format", format])
            .arg(&source));
        assert_eq!(output.status.code(), Some(1), "{format}");
        assert!(
            stderr(&output).contains(expected),
            "{format}: {}",
            stderr(&output)
        );
    }
}

#[test]
fn the_human_format_shows_the_line() {
    let temp = Temp::new("human");
    let source = temp.write("t.c", "int x = ;\n");
    let output = run(xcc().args(["-c"]).arg(&source));
    assert_eq!(output.status.code(), Some(1));
    let text = stderr(&output);
    assert!(text.contains("1 | int x = ;"), "{text}");
    assert!(text.contains("  | ^"), "{text}");
}

#[test]
fn without_c_it_says_linking_is_missing() {
    let temp = Temp::new("noconly");
    let source = temp.write("t.c", "int x;\n");
    let output = run(xcc().arg(&source));
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("linking is not implemented yet"));
}

#[test]
fn a_source_that_is_not_there_is_an_operating_error() {
    let temp = Temp::new("missing");
    let output = run(xcc().current_dir(temp.path()).args(["-c", "not-there.c"]));
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("cannot read not-there.c"));
}

#[test]
fn a_response_file_is_expanded() {
    let temp = Temp::new("response");
    let source = temp.write("t.c", "int x;\n");
    let response = temp.write("args.rsp", &format!("-c --format gcc {}", source.display()));
    let output = run(xcc()
        .current_dir(temp.path())
        .arg(format!("@{}", response.display())));
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("t.c:1:1: error:"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn an_include_is_read_through_the_include_paths() {
    let temp = Temp::new("include");
    std::fs::create_dir_all(temp.path().join("include")).expect("a directory");
    std::fs::write(
        temp.path().join("include/a.h"),
        "int from_header;
",
    )
    .expect("a header");
    let source = temp.write(
        "t.c",
        "#include <a.h>
int from_source;
",
    );
    let output = run(xcc()
        .args(["-c", "-I"])
        .arg(temp.path().join("include"))
        .arg(&source));
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("the parser is not implemented yet"),
        "preprocessing should have succeeded: {}",
        stderr(&output)
    );
}

#[test]
fn a_missing_include_says_which_one() {
    let temp = Temp::new("noinclude");
    let source = temp.write(
        "t.c",
        "#include \"nowhere.h\"
",
    );
    let output = run(xcc().args(["-c"]).arg(&source));
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("cannot find `nowhere.h`"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn defines_come_from_the_command_line() {
    let temp = Temp::new("define");
    let source = temp.write(
        "t.c",
        "#ifndef FLAG
#error no flag
#endif
",
    );
    let without = run(xcc().args(["-c"]).arg(&source));
    assert!(stderr(&without).contains("no flag"), "{}", stderr(&without));

    let with = run(xcc().args(["-c", "-DFLAG"]).arg(&source));
    assert!(
        stderr(&with).contains("the parser is not implemented yet"),
        "{}",
        stderr(&with)
    );
}

#[test]
fn a_missing_response_file_says_so() {
    let temp = Temp::new("noresponse");
    let output = run(xcc().current_dir(temp.path()).arg("@nope.rsp"));
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("cannot read response file"));
}
