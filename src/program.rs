//! Finding the `claude` program the way a desktop user expects.
//!
//! A desktop session's `PATH` often lacks `~/.local/bin`, which is where
//! Claude Code's own installer puts `claude`, and one started before the
//! installer ran always does. So after `PATH` comes `~/.local/bin`, and on
//! macOS the application bundles (`/Applications/<name>.app/Contents/MacOS`,
//! then the same under `~/Applications`).

use std::path::{Path, PathBuf};

/// Where `program` would start from: a path as given (if it exists), or a
/// name searched for as described above. `None` when it is nowhere.
pub fn resolve(program: &str) -> Option<PathBuf> {
    if program.is_empty() {
        return None;
    }
    if program.contains(['/', '\\']) {
        let p = PathBuf::from(program);
        return p.exists().then_some(p);
    }
    let home = home_dir();
    find(
        program,
        &std::env::var_os("PATH").unwrap_or_default(),
        home.as_deref(),
    )
}

/// The user's home folder.
pub fn home_dir() -> Option<PathBuf> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var)
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

/// [`resolve`] for a bare name, with `PATH` and the home folder handed in.
fn find(program: &str, path: &std::ffi::OsStr, home: Option<&Path>) -> Option<PathBuf> {
    let own_bin = home.map(|h| h.join(".local").join("bin"));
    let bundles: Vec<PathBuf> = if cfg!(target_os = "macos") {
        std::iter::once(PathBuf::from("/Applications"))
            .chain(home.map(|h| h.join("Applications")))
            .map(|root| {
                root.join(format!("{program}.app"))
                    .join("Contents")
                    .join("MacOS")
            })
            .collect()
    } else {
        Vec::new()
    };
    std::env::split_paths(path)
        .chain(own_bin)
        .chain(bundles)
        .flat_map(|dir| candidates(&dir, program))
        .find(|c| is_executable(c))
}

/// The files `program` can be in `dir`.
#[cfg(not(windows))]
fn candidates(dir: &Path, program: &str) -> Vec<PathBuf> {
    vec![dir.join(program)]
}

/// On Windows, with each `PATHEXT` extension too: npm installs a CLI as a
/// `.cmd` shim, which a bare name never matches.
#[cfg(windows)]
fn candidates(dir: &Path, program: &str) -> Vec<PathBuf> {
    let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_owned());
    std::iter::once(dir.join(program))
        .chain(
            exts.split(';')
                .filter(|e| !e.is_empty())
                .map(|e| dir.join(format!("{program}{}", e.to_ascii_lowercase()))),
        )
        .collect()
}

#[cfg(unix)]
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(p: &Path) -> bool {
    p.is_file()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn executable(dir: &Path, name: &str) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[test]
    fn path_comes_first_then_local_bin() {
        let home = tempfile::tempdir().unwrap();
        let on_path = tempfile::tempdir().unwrap();
        let local = executable(&home.path().join(".local").join("bin"), "claude");
        let empty = std::env::join_paths([on_path.path()]).unwrap();
        assert_eq!(find("claude", &empty, Some(home.path())), Some(local));

        let first = executable(on_path.path(), "claude");
        assert_eq!(find("claude", &empty, Some(home.path())), Some(first));
    }

    #[test]
    fn a_file_that_is_not_executable_is_not_the_program() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("claude"), "not a program").unwrap();
        let path = std::env::join_paths([dir.path()]).unwrap();
        assert_eq!(find("claude", &path, None), None);
    }

    #[test]
    fn nowhere_and_empty_are_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = std::env::join_paths([dir.path()]).unwrap();
        assert_eq!(find("claude", &path, Some(dir.path())), None);
        assert_eq!(resolve(""), None);
        assert_eq!(resolve("/definitely/not/here/claude"), None);
    }
}
