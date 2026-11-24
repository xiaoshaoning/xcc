//! Real code, vendored: two MIT libraries from other people, compiled and linked on every run.
//!
//! A corpus of 464 small programs finds what small programs can find. These two found the rest -
//! cJSON three front-end bugs and a linker bug, miniz three front-end bugs, a pass bug and a hang -
//! and they are checked in so that the finding stays found. Each test reads the library's own
//! sources, compiles them with this compiler's front end and its own back end at `--opt-level=2`,
//! links them with this compiler's own linker, runs the program, and compares what it printed with
//! what a `clang` build of the same files printed when the fixture was added.
//!
//! What is *not* here: a copy of that `clang` build. The expected text is inline, and
//! `tests/fixtures/README.md` says how it was recorded.

use std::path::PathBuf;
use std::process::Command;

use xcc::{Compile, Includes, Options};

/// The fixture directory, as `#include "..."` sees it: the one thing a host has to give a
/// compilation that is not a string. The compiler never touches a filesystem itself.
#[derive(Debug)]
struct Fixtures(PathBuf);

impl Includes for Fixtures {
    fn read(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(self.0.join(path)).ok()
    }
}

/// Where one fixture lives.
fn root(library: &str) -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests/fixtures");
    path.push(library);
    path
}

/// One fixture file, as the linker wants it: its name and its text.
fn source(library: &str, file: &str) -> (String, String) {
    let path = root(library).join(file);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    (file.to_owned(), text)
}

/// Link `files` with this compiler alone, run the image, and hand back what it did.
fn run_fixture(
    test: &str,
    library: &str,
    files: &[&str],
    defines: &[&str],
) -> std::process::Output {
    let units: Vec<(String, String)> = files.iter().map(|file| source(library, file)).collect();
    let borrowed: Vec<(&str, &str)> = units
        .iter()
        .map(|(name, text)| (name.as_str(), text.as_str()))
        .collect();
    let options = Options {
        opt_level: 2,
        defines: defines.iter().map(|define| (*define).to_owned()).collect(),
        ..Options::default()
    };
    let includes = Box::new(Fixtures(root(library)));
    let image = Compile::with_includes(options, includes)
        .link(&borrowed)
        .unwrap_or_else(|diagnostics| {
            panic!(
                "{library} does not link: {}",
                diagnostics.render(xcc::Format::Human)
            )
        });

    let dir = std::env::temp_dir().join(format!("xcc-real-{test}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a temporary directory");
    let exe = dir.join("fixture.exe");
    std::fs::write(&exe, image.bytes()).expect("the image is written");
    let output = Command::new(&exe).output().expect("the fixture runs");
    let _ = std::fs::remove_dir_all(&dir);
    output
}

/// `cJSON`: parse a document, read values out of the tree by name, and print it back.
#[test]
#[cfg(target_os = "windows")]
fn cjson_parses_walks_and_prints_back() {
    let run = run_fixture("cjson", "cjson", &["driver.c", "cJSON.c"], &[]);
    let out = String::from_utf8_lossy(&run.stdout).into_owned();
    assert_eq!(run.status.code(), Some(0), "{out}");
    assert_eq!(
        out.replace('\r', ""),
        "\
members=5
version=0 a number
first feature=front end
deeper=true
round trip: {\"name\":\"xcc\",\"version\":0.1,\"features\":[\"front end\",\"own back end\",\"linker\"],\"nested\":{\"deep\":{\"deeper\":true}},\"nothing\":null}
"
    );
}

/// `miniz`: deflate a buffer and inflate it back, then write a zip archive in memory and read the
/// file out of it. `MINIZ_NO_STDIO` is the library's own switch for a build with no file APIs - its
/// `<sys/stat.h>` is POSIX, which this compiler does not have.
#[test]
#[cfg(target_os = "windows")]
fn miniz_deflates_inflates_and_zips() {
    let run = run_fixture(
        "miniz",
        "miniz",
        &[
            "driver.c",
            "miniz.c",
            "miniz_tdef.c",
            "miniz_tinfl.c",
            "miniz_zip.c",
        ],
        &["MINIZ_NO_STDIO"],
    );
    let out = String::from_utf8_lossy(&run.stdout).into_owned();
    assert_eq!(run.status.code(), Some(0), "{out}");
    assert_eq!(
        out.replace('\r', ""),
        "\
deflate/inflate: identical
zip: 1 file(s), extract identical
zip entry named
"
    );
}

/// `lz4`: the block, streaming, high-compression, frame and stdio APIs, with a digest of every
/// compressed byte at the end. Three bugs came out of this fixture - two in the parser, and one
/// in the prologue, which crashed any frame larger than a stack page.
#[test]
#[cfg(target_os = "windows")]
fn lz4_compresses_streams_and_round_trips_every_api() {
    let run = run_fixture(
        "lz4",
        "lz4",
        &[
            "driver.c",
            "lz4.c",
            "lz4hc.c",
            "lz4frame.c",
            "lz4file.c",
            "xxhash.c",
        ],
        &[],
    );
    let out = String::from_utf8_lossy(&run.stdout).into_owned();
    assert_eq!(run.status.code(), Some(0), "{out}");
    // A raw string: the expected text is lines of names and numbers, with nothing to escape.
    assert_eq!(
        out.replace('\r', ""),
        r#"compress_fast/decompress_safe              301178
compress_fast/decompress_safe              301178
compress_fast/decompress_safe              301178
compress_fast/decompress_safe              301178
compress_destSize                          4017
compress_fast_extState                     301178
decompress_safe_partial(1000)              1000
compress_HC(12)                            298265
compress_HC_extStateHC(9)                  298271
fast_continue + safe_continue + saveDict   321106
loadDict + fast_continue                   100394
compress_HC_continue                       298288
frame compress in odd chunks               298303
frame decompress in odd chunks             300000
compressFrame_usingCDict                   100023
decompress after usingCDict                100000
stdio wrapper round trip                   7000
digest 14ccb783a2757039
"#
    );
}

/// `tinyexpr` (758 lines, zlib licence): a recursive-descent expression parser with its own
/// evaluation tree. It is the fourth real library here and the smallest - it found three bugs that
/// nothing else had reached: `te_expr` holds an anonymous union (C11 6.7.2.1p13), which this
/// compiler refused entirely; `power()` keeps a `static` table of function pointers, whose
/// initialiser was emitted as code and so wrote zeroes; and `te_variable variables[] = { ... }` is
/// an array whose size comes from its list, whose elements' unnamed members held the frame's
/// leftovers. `nan` in two of the lines is `NAN`, whose sign bit is the one `0.0/0.0` does not
/// give.
#[test]
#[cfg(target_os = "windows")]
fn tinyexpr_parses_evaluates_and_compiles_expressions() {
    let run = run_fixture("tinyexpr", "tinyexpr", &["driver.c", "tinyexpr.c"], &[]);
    let out = String::from_utf8_lossy(&run.stdout).into_owned();
    assert_eq!(run.status.code(), Some(0), "{out}");
    assert_eq!(
        out.replace('\r', ""),
        r#"1+2*3            -> 7.000000 (err 0)
sin(pi/2)        -> 1.000000 (err 0)
sqrt(2)^2        -> 2.000000 (err 0)
2^10             -> 1024.000000 (err 0)
(1+2)*(3+4)      -> 21.000000 (err 0)
5%3              -> 2.000000 (err 0)
-3+10/4          -> -0.500000 (err 0)
min(3,7)+max(2,9) -> nan (err 3)
log(100)/log(10) -> 2.000000 (err 0)
if(1<2, 42, 0)   -> nan (err 2)
3==3             -> 1.000000 (err 0)
3!=3             -> 0.000000 (err 0)
compiled x*y+1 -> 43.0 (err 0)
with x=2 -> 15.0
"#
    );
}
