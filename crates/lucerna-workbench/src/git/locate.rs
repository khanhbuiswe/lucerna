//! Finds the `git` binary to run.

use std::path::{Path, PathBuf};

/// Picks the `git` to use: the `git.path` setting when it is an executable
/// file, otherwise the first `git` on `path_var` (default: the process `PATH`).
/// On macOS the `/usr/bin/git` shim is skipped unless the Command Line Tools
/// are installed, since running it would pop up the installer dialog.
pub(crate) fn locate_git(setting: Option<PathBuf>, path_var: Option<&str>) -> Option<PathBuf> {
    if let Some(path) = setting {
        return is_executable_file(&path).then_some(path);
    }
    let env_path;
    let path_var = match path_var {
        Some(p) => p,
        None => {
            env_path = std::env::var("PATH").unwrap_or_default();
            &env_path
        }
    };
    let found = lucerna_lsp::env::which("git", path_var)?;
    let clt = if cfg!(target_os = "macos") && found == Path::new("/usr/bin/git") {
        clt_installed()
    } else {
        true
    };
    accept_candidate(&found, clt).then_some(found)
}

/// The macOS rule: `/usr/bin/git` is only a stub that opens the installer
/// when the Command Line Tools are missing.
pub(crate) fn accept_candidate(path: &Path, clt_installed: bool) -> bool {
    !(cfg!(target_os = "macos") && path == Path::new("/usr/bin/git") && !clt_installed)
}

/// `xcode-select -p` exits 0 when a developer dir (CLT or Xcode) is set.
fn clt_installed() -> bool {
    std::process::Command::new("xcode-select")
        .arg("-p")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn make_git(dir: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join("git");
        std::fs::write(&p, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[cfg(unix)]
    #[test]
    fn setting_wins() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let from_setting = make_git(a.path());
        make_git(b.path());
        let got = locate_git(Some(from_setting.clone()), Some(b.path().to_str().unwrap()));
        assert_eq!(got, Some(from_setting));
        // A setting that is not executable yields None (no silent fallback).
        let missing = a.path().join("nope");
        assert_eq!(
            locate_git(Some(missing), Some(b.path().to_str().unwrap())),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn path_lookup() {
        let d = tempfile::tempdir().unwrap();
        let g = make_git(d.path());
        assert_eq!(locate_git(None, Some(d.path().to_str().unwrap())), Some(g));
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(locate_git(None, Some(empty.path().to_str().unwrap())), None);
    }

    #[test]
    fn shim_without_clt_is_not_git() {
        let shim = Path::new("/usr/bin/git");
        assert!(accept_candidate(shim, true));
        assert_eq!(accept_candidate(shim, false), !cfg!(target_os = "macos"));
        assert!(accept_candidate(Path::new("/opt/homebrew/bin/git"), false));
    }
}
