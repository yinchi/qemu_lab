//! Path arithmetic: turning a path a user typed, which may be relative to the working directory and
//! may contain `.` and `..`, into the absolute path the filesystem walk (`files.rs`) resolves.
//!
//! Purely lexical -- no filesystem access, so it never checks that anything exists, and `..` simply removes the
//! component before it (there are no symbolic links to make that wrong). The name is Python's
//! `os.path.abspath`, deliberately not `canonicalize`/`realpath`, which in POSIX and Rust also
//! require the path to exist and resolve symlinks.
//!
//! Pure `no_std` + `alloc`, with no dependency on the rest of the kernel, so it is tested on the host
//! (`hosttests/`).

use alloc::string::String;
use alloc::vec::Vec;

use abi::errno::{ENAMETOOLONG, ENOENT};
use abi::fs::{NAME_MAX, PATH_MAX};

/// `path` as an absolute, normalized path: `/`, or `/` followed by components separated by single `/`s, with
/// no `.`, `..` or empty components and no trailing `/`. A relative `path` is taken from `cwd`, which must
/// itself be absolute and normalized (as `abspath` returns). `..` at the root stays at the root. A
/// trailing `/` is ignored (POSIX would insist the last component be a directory; nothing here checks).
///
/// Errors: `ENOENT` for an empty path (as POSIX pathname resolution has it), `ENAMETOOLONG` for a
/// component longer than `NAME_MAX` or a path (given or resulting) longer than `PATH_MAX`.
pub fn abspath(cwd: &str, path: &str) -> Result<String, isize> {
    if path.is_empty() {
        return Err(ENOENT);
    }
    if path.len() > PATH_MAX {
        return Err(ENAMETOOLONG);
    }
    let mut parts: Vec<&str> = if path.starts_with('/') {
        Vec::new()
    } else {
        cwd.split('/').filter(|c| !c.is_empty()).collect()
    };
    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            name => {
                if name.len() > NAME_MAX {
                    return Err(ENAMETOOLONG);
                }
                parts.push(name);
            }
        }
    }
    if parts.is_empty() {
        return Ok(String::from("/"));
    }
    let mut result = String::new();
    for part in parts {
        result.push('/');
        result.push_str(part);
    }
    if result.len() > PATH_MAX {
        return Err(ENAMETOOLONG);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    fn abs(cwd: &str, path: &str) -> String {
        abspath(cwd, path).unwrap()
    }

    #[test]
    fn absolute_paths_ignore_the_working_directory() {
        assert_eq!(abs("/bin", "/tests/notes.txt"), "/tests/notes.txt");
        assert_eq!(abs("/", "/"), "/");
        assert_eq!(abs("/a/b", "/"), "/");
    }

    #[test]
    fn relative_paths_start_at_the_working_directory() {
        assert_eq!(abs("/", "bin"), "/bin");
        assert_eq!(abs("/bin", "cat.exe"), "/bin/cat.exe");
        assert_eq!(abs("/a/b", "c/d"), "/a/b/c/d");
    }

    #[test]
    fn dot_and_empty_components_vanish() {
        assert_eq!(abs("/", "./bin/."), "/bin");
        assert_eq!(abs("/a", "b//c///d"), "/a/b/c/d");
        assert_eq!(abs("/a", "."), "/a");
        assert_eq!(abs("/", "/./."), "/");
    }

    #[test]
    fn dot_dot_removes_the_component_before_it() {
        assert_eq!(abs("/a/b", ".."), "/a");
        assert_eq!(abs("/a/b", "../.."), "/");
        assert_eq!(abs("/a/b", "../c"), "/a/c");
        assert_eq!(abs("/", "/bin/../fonts"), "/fonts");
        assert_eq!(abs("/", "a/b/../../c"), "/c");
    }

    #[test]
    fn dot_dot_at_the_root_stays_at_the_root() {
        assert_eq!(abs("/", ".."), "/");
        assert_eq!(abs("/", "../../.."), "/");
        assert_eq!(abs("/a", "../../b"), "/b");
        assert_eq!(abs("/", "/.."), "/");
    }

    #[test]
    fn a_trailing_slash_is_ignored() {
        assert_eq!(abs("/", "bin/"), "/bin");
        assert_eq!(abs("/", "/bin//"), "/bin");
    }

    #[test]
    fn an_empty_path_is_not_found() {
        assert_eq!(abspath("/", ""), Err(ENOENT));
    }

    #[test]
    fn names_longer_than_the_limit_are_refused() {
        let ok = "x".repeat(NAME_MAX);
        assert_eq!(abs("/", &ok), alloc::format!("/{ok}"));
        let long = "x".repeat(NAME_MAX + 1);
        assert_eq!(abspath("/", &long), Err(ENAMETOOLONG));
        assert_eq!(
            abspath("/a", &alloc::format!("b/{long}/c")),
            Err(ENAMETOOLONG)
        );
        // Even one that `..` would cancel again is refused: it was never a valid name.
        assert_eq!(
            abspath("/", &alloc::format!("{long}/..")),
            Err(ENAMETOOLONG)
        );
    }

    #[test]
    fn paths_longer_than_the_limit_are_refused() {
        let too_long = "a/".repeat(PATH_MAX / 2 + 1);
        assert_eq!(abspath("/", &too_long), Err(ENAMETOOLONG));
        // A short relative path that lands past the limit once joined onto a long working directory.
        let cwd = "/".to_string() + &"d/".repeat(PATH_MAX / 2 - 1) + "d";
        assert!(cwd.len() <= PATH_MAX);
        assert_eq!(abspath(&cwd, "tail_that_overflows"), Err(ENAMETOOLONG));
        // A path that is exactly as long as allowed is fine (the limit counts the leading `/`).
        let exact = "/".to_string() + &"d/".repeat(PATH_MAX / 2 - 1) + "d";
        assert_eq!(abspath("/", &exact).map(|p| p.len()), Ok(exact.len()));
    }

    #[test]
    fn the_result_is_a_fixed_point() {
        for (cwd, path) in [
            ("/a/b", "../c/./d//e/.."),
            ("/", "x/../y"),
            ("/z", "/w/../v"),
        ] {
            let once = abs(cwd, path);
            assert_eq!(abs("/somewhere/else", &once), once);
        }
    }
}
