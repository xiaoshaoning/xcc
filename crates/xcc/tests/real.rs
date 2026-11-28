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

/// The directories `#include "..."` may read, in order: the fixture's own, then - for the one
/// fixture that names a header no C library has - the host's Windows SDK. The compiler never
/// touches a filesystem itself; this is the whole of what a host has to give it that is not text.
#[derive(Debug)]
struct Fixtures(Vec<PathBuf>);

impl Includes for Fixtures {
    fn read(&self, path: &str) -> Option<String> {
        self.0
            .iter()
            .find_map(|root| std::fs::read_to_string(root.join(path)).ok())
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
    run_fixture_with(
        test,
        library,
        files,
        defines,
        xcc::CStandard::default(),
        &[],
        &[],
    )
}

/// The same, with directories to search after the fixture's own: SQLite's Windows layer includes
/// `windows.h`, which is the host's SDK and not something a fixture may carry.
fn run_fixture_with(
    test: &str,
    library: &str,
    files: &[&str],
    defines: &[&str],
    standard: xcc::CStandard,
    more: &[PathBuf],
    arguments: &[&str],
) -> std::process::Output {
    let units: Vec<(String, String)> = files.iter().map(|file| source(library, file)).collect();
    let borrowed: Vec<(&str, &str)> = units
        .iter()
        .map(|(name, text)| (name.as_str(), text.as_str()))
        .collect();
    let options = Options {
        opt_level: 2,
        standard,
        defines: defines.iter().map(|define| (*define).to_owned()).collect(),
        // A fixture is real code and this is a compiler: the library's own budgets are sized for
        // an embedder handing over a snippet, and SQLite's amalgamation is nine megabytes.
        budgets: xcc::Budgets::for_a_compiler(),
        ..Options::default()
    };
    let mut roots = vec![root(library)];
    roots.extend_from_slice(more);
    let includes = Box::new(Fixtures(roots));
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
    let output = Command::new(&exe)
        .args(arguments)
        .output()
        .expect("the fixture runs");
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

/// `Lua`: the interpreter, the compiler, the garbage collector, the string library and its pattern
/// matcher, coroutines, and `string.pack` - 33 files of the 5.5.1 release, unmodified.
///
/// Two units need the Windows headers (`loadlib.c` loads DLLs, `lua.c` asks whether stdin is a
/// terminal) and `luaconf.h` defines `LUA_USE_WINDOWS` for `_WIN32` whether a program wants it or
/// not - so the compile-only half leaves those two out and runs everywhere, and the run half needs
/// a host with an SDK and says so.
#[test]
fn lua_compiles_without_a_windows_sdk() {
    // Every unit but the two that read `windows.h`.
    let files: Vec<String> = std::fs::read_dir(root("lua"))
        .expect("the fixture directory")
        .filter_map(|entry| entry.ok()?.file_name().to_str().map(str::to_owned))
        .filter(|name| name.ends_with(".c") && name != "loadlib.c" && name != "lua.c")
        .collect();
    // 34 units in the release's `src/`, two of which read `windows.h`.
    assert_eq!(
        files.len(),
        32,
        "the fixture lost or gained files: {files:?}"
    );
    let units: Vec<(String, String)> = files.iter().map(|file| source("lua", file)).collect();
    let options = Options {
        opt_level: 2,
        budgets: xcc::Budgets::for_a_compiler(),
        ..Options::default()
    };
    // One unit at a time, because this set has no `main` and no program to link: what it checks is
    // that the front end and the back end take every one of these files, which is where a
    // miscompile shows as a refusal or a verifier failure rather than as a wrong answer.
    // The includes are the fixture's own directory: `lapi.c` includes `lua.h` from beside itself.
    let mut compiler = Compile::with_includes(options, Box::new(Fixtures(vec![root("lua")])));
    for (name, text) in &units {
        compiler.compile(name, text).unwrap_or_else(|diagnostics| {
            panic!(
                "{name} does not compile: {}",
                diagnostics.render(xcc::Format::Human)
            )
        });
    }
}

/// The whole interpreter, linked and run on the driver script, compared with what a `clang -O2`
/// build of the same files printed: integers and bitwise operators, `//`, patterns, `string.pack`,
/// metamethods, coroutines, `pcall`/`xpcall` with table error objects, `goto`, `repeat`, a `for`
/// with a negative step, utf8, an `io` round trip, and `load`.
#[test]
#[cfg(target_os = "windows")]
fn lua_runs_its_driver_script_the_way_clang_does() {
    let Some(sdk) = windows_sdk() else {
        eprintln!("no Windows SDK headers on this machine: the Lua run test is skipped");
        return;
    };
    let run = run_fixture_with(
        "lua",
        "lua",
        &[
            "lapi.c",
            "lcode.c",
            "lctype.c",
            "ldebug.c",
            "ldo.c",
            "ldump.c",
            "lfunc.c",
            "lgc.c",
            "llex.c",
            "lmem.c",
            "lobject.c",
            "lopcodes.c",
            "lparser.c",
            "lstate.c",
            "lstring.c",
            "ltable.c",
            "ltm.c",
            "lundump.c",
            "lvm.c",
            "lzio.c",
            "lauxlib.c",
            "lbaselib.c",
            "lcorolib.c",
            "ldblib.c",
            "liolib.c",
            "lmathlib.c",
            "loadlib.c",
            "loslib.c",
            "lstrlib.c",
            "ltablib.c",
            "lutf8lib.c",
            "linit.c",
            "lua.c",
        ],
        &[],
        xcc::CStandard::default(),
        &sdk,
        // The script, by absolute path: the interpreter takes it as its one argument, which is how
        // the command line runs a program.
        &[&root("lua").join("driver.lua").to_string_lossy()],
    );
    let out = String::from_utf8_lossy(&run.stdout).replace('\r', "");
    assert_eq!(run.status.code(), Some(0), "{out}");
    assert_eq!(out, LUA_EXPECTED);
}

/// What a `clang -O2` build of the fixture printed for `driver.lua` when the fixture was added.
/// Both builds printed exactly this, line for line, which is what a differential test of an
/// interpreter is: every number, every error message, and every metatable question answered the same.
const LUA_EXPECTED: &str = r#"10 7 4 1 
3 1 3.5 1024.0 1024 240 9223372036854775807
9223372036854775807 -9223372036854775808 true
inf -inf true integer float
42  3.14 1e+300 ff "a\"b"
true quick 19 5 the quick brown fox-the quick brown fox
t q b f 4
the quick
120 xyz
1 0 0 0 2 0 0 0
7 8 9
1,2,5,9 4 1 9
9,5,2,1
1,2,3 2
10 20 30
(3) false 1
3
15 5
6765
true 2
true 20
true 105
dead
false table 42
false attempt to divide by zero
handled: boom
goto 15
repeat 3
97 98 99
Hello 5 233
123 1.5 16 12 1000.0
-4 -3 1 -9223372036854775808
abc|7   |+1.2e+03
line one 42
true 42
3
true
hi true
"#;

/// `yyjson`: 11 224 lines in one file, a JSON reader *and* writer with its own allocation strategy,
/// a number-to-text formatter, UTF-8 validation, and a macro layer that pastes four tokens at once
/// (`0x##d##c##b##a##UL`) - which is the bug it found: this compiler pasted one pair and walked past
/// the rest of a chain, so `a##b##c` failed at the second `##`.
///
/// It needs no Windows headers, so unlike the other two unfriendly fixtures this one is compiled and
/// **run** on every machine, and its answer compared with clang's.
#[test]
#[cfg(target_os = "windows")]
fn yyjson_reads_writes_and_agrees_with_clang() {
    let run = run_fixture_with(
        "yyjson",
        "yyjson",
        &["driver.c", "yyjson.c"],
        &[],
        xcc::CStandard::default(),
        &[],
        &[],
    );
    // Every line, not the bytes: the platform's line ending is not the library's business.
    let out: Vec<String> = String::from_utf8_lossy(&run.stdout)
        .lines()
        .map(str::trim_end)
        .map(str::to_owned)
        .collect();
    assert_eq!(run.status.code(), Some(0), "{out:?}");
    let expected: Vec<String> = YYJSON_EXPECTED.lines().map(str::to_owned).collect();
    assert_eq!(out, expected);
}

/// What a `clang -O0` build of the fixture printed when it was added: the reader's answers about a
/// document, then the writer's serialisation of a document built by hand.
const YYJSON_EXPECTED: &str = "\
3 2 x array -0.50
{\"who\":\"xcc\",\"version\":5}
";

/// `tre`: 7 663 lines of POSIX regular expressions - the most macro-heavy code in the fixtures -
/// and the second that needs the SDK, because its Windows branch reads `basetsd.h`.
///
/// It is here to pin M42: a pointer converted to a **narrower integer** is a `Truncate`, which was
/// missing from the conversion table until `tre` refused to compile without it. The driver is one
/// extended expression matched once with three groups, so the parse, the compile and the parallel
/// matcher all run.
///
/// `-DUSE_LOCAL_TRE_H` is tre's own switch: without it `regex.h` includes `<tre/tre.h>` from an
/// installed package - which on this machine is msys2's, and a build that took that path would be
/// comparing this compiler against a *different library* than the one being compiled.
#[test]
fn tre_matches_three_groups() {
    let Some(sdk) = windows_sdk() else {
        eprintln!("no Windows SDK headers on this machine: the tre test is skipped");
        return;
    };
    let run = run_fixture_with(
        "tre",
        "tre",
        &[
            "driver.c",
            "tre-ast.c",
            "tre-compile.c",
            "tre-filter.c",
            "tre-match-approx.c",
            "tre-match-backtrack.c",
            "tre-match-parallel.c",
            "tre-parse.c",
            "tre-stack.c",
            "tre-mem.c",
            "regcomp.c",
            "regerror.c",
            "regexec.c",
        ],
        &["USE_LOCAL_TRE_H"],
        xcc::CStandard::default(),
        &sdk,
        &[],
    );
    let out = String::from_utf8_lossy(&run.stdout);
    assert_eq!(run.status.code(), Some(0), "{out}");
    // What a `clang -O0` build of the same files prints: match 2..8, group 1 at 2..5, group 2 at 5..8.
    assert_eq!(out.trim_end(), "2 8 2 5 5 8");
}

/// `fixed_point`: a real program doing real arithmetic, and the **first fixture whose oracle is not
/// clang**.
///
/// 139 lines of fixed-point square root - Newton's method on Q30/Q31 values, with 64-bit multiplies,
/// shifts and `%llx`/`%f`/`%08x` output. The expected text beside it was produced by `gcc -O2` on Linux,
/// which makes this the one fixture that checks against a *second* implementation of the language
/// rather than against a different invocation of the same one.
///
/// It arrived by survey rather than by plan: of 2 000 files in the corpus that compile, 27 build an
/// executable and 13 print anything, and this is one of the two that agree with `gcc` byte for byte
/// (line endings aside - ours writes through the Windows CRT, `gcc` on Linux does not).
#[test]
fn fixed_point_square_roots_agree_with_gcc() {
    let run = run_fixture_with(
        "fixed_point",
        "fixed_point",
        &["debug_isqrt.c"],
        &[],
        xcc::CStandard::default(),
        &[],
        &[],
    );
    let out = String::from_utf8_lossy(&run.stdout).replace('\r', "");
    assert_eq!(run.status.code(), Some(0), "{out}");
    // `gcc -O2`'s output, recorded beside the program.
    let expected = include_str!("fixtures/fixed_point/expected.txt");
    assert_eq!(out, expected, "the fixed-point arithmetic disagreed");
}

/// `xxhash`: 250 KB of hash functions, and the **only** fixture of the last four that needs no
/// Windows SDK - so unlike SQLite's and Lua's run halves this one is checked on every machine.
///
/// It is here because it found a *wrong number*: `_umul128` is an MSVC intrinsic that a program calls
/// with no header, this compiler took it as an implicit `int`, and XXH3's 64x64-to-128 multiply lost
/// its high half. The three hashes below are the oracle: XXH64 and XXH32 were always right, and the
/// one that was wrong is the one that multiplies, so a regression of the intrinsic path shows up as a
/// disagreement rather than as a memory the next reader has to carry.
///
/// `-DXXH_VECTOR=0` is the library's own switch for its portable scalar path: its vector paths are
/// `__m128i` and `__m256i`, and this compiler shadows `emmintrin.h` with an empty bridge header.
#[test]
fn xxhash_agrees_with_clang_on_three_hashes() {
    let run = run_fixture_with(
        "xxhash",
        "xxhash",
        &["driver.c", "xxhash.c"],
        &["XXH_VECTOR=0"],
        xcc::CStandard::default(),
        &[],
        &[],
    );
    let out = String::from_utf8_lossy(&run.stdout);
    assert_eq!(run.status.code(), Some(0), "{out}");
    // What a `clang -O0` build of the same two files prints.
    assert_eq!(out.trim_end(), "ed714233c5a9a792 e4541a9cacf545aa 66716377");
}

/// `mimalloc`: Microsoft's allocator, 21 units and an amalgamation, with its own thread-local heaps,
/// its own atomics and its own OS layer. It is the fourth real program this compiler runs and the
/// first that is *about* the machine - thread-local storage, `gs:` reads, `lock cmpxchg`, and the
/// `rep movs` block copies the Windows headers name.
///
/// **The only fixture with no SDK-free half.** Every one of its 21 units includes the OS layer,
/// which includes `windows.h`, so there is nothing to compile on a machine without the SDK - this
/// test skips there, and CI is such a machine. Its value is where development happens.
#[test]
#[cfg(target_os = "windows")]
fn mimalloc_allocates_through_its_own_theaps() {
    let Some(sdk) = windows_sdk() else {
        eprintln!("no Windows SDK headers on this machine: the mimalloc test is skipped");
        return;
    };
    let run = run_fixture_with(
        "mimalloc",
        "mimalloc",
        &["driver.c", "src/static.c"],
        &[],
        // `mimalloc` is C11: `_Atomic`, thread-local storage, and `stdatomic.h`.
        xcc::CStandard::C11,
        // Its own headers live in `include/`, which is not the fixture's root.
        &[vec![root("mimalloc").join("include")], sdk].concat(),
        &[],
    );
    let out: Vec<String> = String::from_utf8_lossy(&run.stdout)
        .lines()
        .map(str::trim_end)
        .map(str::to_owned)
        .collect();
    assert_eq!(run.status.code(), Some(0), "{out:?}");
    // `mi_malloc`, a `strcpy` into it, `mi_version`, and `mi_realloc` - what a `clang -O0` build of
    // the same two files printed when the fixture was added.
    assert_eq!(out, ["mimalloc under xcc 1", "1"]);
}

/// `SQLite`: the amalgamation every application vendors - 265 977 lines, 9.6 MB, one translation
/// unit - compiled by this compiler with its own back end. `SQLITE_OS_OTHER=1` takes out the OS
/// layer, which is the only part that needs `windows.h`, so this is checked on **every** machine:
/// it is the front end and the object writer against the largest single file of real C there is.
///
/// What it cost when it was added: three bugs in the type of an integer constant, two in the
/// initialiser that a static object holds, a `for` loop whose update ended in a conditional, and
/// the constant folder that compared two operands before converting them. None of them was visible
/// in a corpus of 464 small programs.
#[test]
fn sqlite_compiles_without_a_windows_sdk() {
    let unit = source("sqlite", "sqlite3.c");
    let options = Options {
        opt_level: 2,
        defines: vec!["SQLITE_OS_OTHER=1".to_owned()],
        budgets: xcc::Budgets::for_a_compiler(),
        ..Options::default()
    };
    let artifact = Compile::new(options)
        .compile("sqlite3.c", &unit.1)
        .unwrap_or_else(|diagnostics| {
            panic!(
                "sqlite3.c does not compile: {}",
                diagnostics.render(xcc::Format::Human)
            )
        });
    // A 2.6 MB object is what 265 977 lines of SQLite come to. The number is here to catch a
    // compilation that stopped early and called it success, not to be tight.
    assert!(
        artifact.bytes().len() > 2_000_000,
        "the object is {} bytes, which is not the whole amalgamation",
        artifact.bytes().len()
    );
}

/// The same file with its Windows layer on: a real database, opened in memory, with a table, a
/// multi-row `INSERT`, an aggregate, and a prepared statement - the whole library, not just its
/// parser. Compared byte for byte with what a `clang` build printed when the fixture was added.
///
/// Windows headers are the host's: this machine has them and CI does not, so the test says which
/// it is rather than passing quietly. The compile-only test above is the one that never skips.
#[test]
#[cfg(target_os = "windows")]
fn sqlite_runs_a_query_through_its_windows_layer() {
    let Some(sdk) = windows_sdk() else {
        eprintln!("no Windows SDK headers on this machine: the SQLite Windows test is skipped");
        return;
    };
    let run = run_fixture_with(
        "sqlite",
        "sqlite",
        &["driver.c", "sqlite3.c"],
        &[],
        xcc::CStandard::default(),
        &sdk,
        &[],
    );
    let out = String::from_utf8_lossy(&run.stdout).into_owned();
    assert_eq!(run.status.code(), Some(0), "{out}");
    assert_eq!(out.replace('\r', ""), "6 one,two,three 3.51.3\n0 -1\n");
}

/// Where this machine keeps the platform's headers, or `None` - **the compiler's own answer**, not a
/// second search that agrees with it until one of them changes.
///
/// This used to spell three paths itself and know only Visual Studio **Community**, which is the one
/// edition a CI runner does not have: on `windows-latest` it answered `None`, so every fixture that
/// needs the SDK skipped there, and the halves that *run* SQLite, Lua, mimalloc and tre were never
/// exercised anywhere but the machine they were written on.
///
/// `xcc::platform::platform_directories` is what the compiler searches - `INCLUDE`, MSYS2's two
/// prefixes, every edition of Visual Studio, and the Windows SDK - so the test asks the same question
/// the compiler asks, and a machine that can build a program can run these tests.
fn windows_sdk() -> Option<Vec<PathBuf>> {
    let directories = xcc::platform::platform_directories();
    let has_windows = directories
        .iter()
        .any(|path| path.join("windows.h").is_file());
    has_windows.then_some(directories)
}
