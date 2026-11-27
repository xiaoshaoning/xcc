//! Where this machine keeps the headers a compiler does not ship.
//!
//! The compiler reads nothing by itself: `Headers` wraps whatever `Includes` the *host* builds, and
//! the host is the layer that knows an installation's layout. This module is that knowledge, in one
//! place and free of policy - [`platform_directories`] answers "where might they be", and the caller
//! decides whether to look.

use std::path::PathBuf;

/// The directories a machine keeps its **platform** headers in - the C runtime's and the Windows
/// SDK's - in the order a search should try them.
///
/// A compiler that ships its own `<stdio.h>` still has to find `<windows.h>` and `<unistd.h>`
/// somewhere, and on Windows "somewhere" is one of four places. This is the one function that knows
/// them; the *host* decides whether to use it, because the compiler's rule is that it reads nothing
/// by itself - `Headers` wraps whatever `Includes` the host builds, and the command line's host is
/// `platform_directories()` after its `-I`s.
///
/// `INCLUDE` comes first: a build system that sets it means it, and that is MSVC's own convention.
/// Nothing here is cached or probed twice; the list is computed once per compile and the cost is a
/// handful of `is_file` calls on directories that are usually not there.
pub fn platform_directories() -> Vec<PathBuf> {
    let mut directories: Vec<PathBuf> = Vec::new();
    let mut add = |path: PathBuf| {
        if path.join("stdio.h").is_file() && !directories.contains(&path) {
            directories.push(path);
        }
    };

    // MSVC's own convention, and the only one a build system can set: `INCLUDE=dir;dir;...`.
    if let Some(include) = std::env::var_os("INCLUDE") {
        for directory in std::env::split_paths(&include) {
            add(directory);
        }
    }

    // mingw-w64, which is what a machine with clang and no Visual Studio has. `ucrt64` first: it is
    // the one whose runtime this compiler links against.
    for candidate in ["C:/msys64/ucrt64/include", "C:/msys64/mingw64/include"] {
        add(PathBuf::from(candidate));
    }

    // Visual Studio: the C runtime's headers are three directories below the toolset's version,
    // which is a name nobody should have to spell - so the newest toolset that has them wins.
    let visual_studio = PathBuf::from("C:/Program Files/Microsoft Visual Studio/2022");
    for edition in ["Community", "Professional", "Enterprise", "BuildTools"] {
        let toolsets = visual_studio.join(edition).join("VC/Tools/MSVC");
        let Ok(entries) = std::fs::read_dir(&toolsets) else {
            continue;
        };
        let mut versions: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
        versions.sort();
        versions.reverse();
        for version in versions {
            add(version.join("include"));
        }
    }

    // The Windows SDK's own three directories, again newest first: `ucrt` is the C runtime's headers,
    // `um` and `shared` are the API's.
    let kits = PathBuf::from("C:/Program Files (x86)/Windows Kits/10/Include");
    if let Ok(entries) = std::fs::read_dir(&kits) {
        let mut versions: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
        versions.sort();
        versions.reverse();
        for version in versions {
            for part in ["ucrt", "shared", "um"] {
                add(version.join(part));
            }
        }
    }

    directories
}

#[cfg(test)]
mod tests {
    use super::platform_directories;

    /// Whatever this machine has, the answer is directories that exist and hold `stdio.h` - and it is
    /// the same answer twice. Which of them exist is a fact about the machine, so it is not asserted:
    /// a test that requires an SDK is a test that fails on the machine that has none.
    #[test]
    fn platform_directories_names_only_directories_that_have_stdio_h() {
        let first = platform_directories();
        let again = platform_directories();
        assert_eq!(first, again, "the same question, the same answer");
        for directory in &first {
            assert!(
                directory.join("stdio.h").is_file(),
                "{} was named and does not hold stdio.h",
                directory.display()
            );
            assert_eq!(
                first.iter().filter(|other| *other == directory).count(),
                1,
                "{} is named twice",
                directory.display()
            );
        }
    }
}
