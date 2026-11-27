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
    // A directory is a search path if it **is** a directory. It used to require `stdio.h`, as a cheap
    // "is this really a header directory" probe - and that silently dropped the two Windows SDK
    // directories that hold `windows.h` and nothing from the C runtime, so on a machine with Visual
    // Studio and no mingw the search found `ucrt` and stopped one directory short. The order below is
    // what decides preference; existence is all this has to ask.
    let mut add = |path: PathBuf| {
        if path.is_dir() && !directories.contains(&path) {
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
    use std::path::PathBuf;

    /// Whatever this machine has, the answer is directories that exist - and it is the same answer
    /// twice. *Which* of them exist is a fact about the machine, so it is not asserted: a test that
    /// requires an SDK is a test that fails on the machine that has none.
    ///
    /// It used to assert `stdio.h` in every one, which passed on this machine and would have hidden
    /// the bug it was written next to: the Windows SDK keeps `windows.h` in `um` and `shared`, which
    /// have no `stdio.h`, and those two were being dropped.
    /// Every directory the Windows SDK keeps `windows.h` in is in the search, not only the one that
    /// keeps the C runtime's headers.
    ///
    /// This is the bug the first version shipped: `add` accepted a path only if it held `stdio.h`, and
    /// `um` and `shared` hold `windows.h` and no `stdio.h` - so a machine with Visual Studio and no
    /// mingw found `ucrt` and stopped one directory short, and *nothing on this machine showed it*
    /// because mingw's own `windows.h` was found first. The assertion is a relationship, so it holds
    /// whether or not the Kits are installed.
    #[test]
    fn every_sdk_directory_that_holds_windows_h_is_searched() {
        let kits = PathBuf::from("C:/Program Files (x86)/Windows Kits/10/Include");
        let Ok(versions) = std::fs::read_dir(&kits) else {
            eprintln!("no Windows Kits on this machine: the test is skipped");
            return;
        };
        let newest = versions
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.join("um/windows.h").is_file())
            .max();
        let Some(newest) = newest else {
            eprintln!("no Windows Kits with `um/windows.h`: the test is skipped");
            return;
        };
        let searched = platform_directories();
        for part in ["ucrt", "shared", "um"] {
            let directory = newest.join(part);
            if directory.is_dir() {
                assert!(
                    searched.contains(&directory),
                    "{} holds headers and is not searched: {searched:?}",
                    directory.display()
                );
            }
        }
    }

    #[test]
    fn platform_directories_names_only_directories_that_exist_once_each() {
        let first = platform_directories();
        let again = platform_directories();
        assert_eq!(first, again, "the same question, the same answer");
        for directory in &first {
            assert!(
                directory.is_dir(),
                "{} was named and is not a directory",
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
