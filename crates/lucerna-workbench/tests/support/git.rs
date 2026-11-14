//! Temporary git repositories, with `git` isolated from the developer's
//! global and system config.

use std::path::{Path, PathBuf};
use std::process::Command;

use lucerna_git::GitBinary;
use tempfile::TempDir;

#[cfg(windows)]
const NULL: &str = "NUL";
#[cfg(not(windows))]
const NULL: &str = "/dev/null";

pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", NULL)
        .env("GIT_CONFIG_SYSTEM", NULL)
        .output()
        .expect("spawn git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A fresh repo on `main` with committed `a.txt` and `b.txt`; the path is
/// canonical.
pub fn repo() -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().canonicalize().unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    git(&dir, &["config", "user.name", "Test"]);
    git(&dir, &["config", "user.email", "test@example.com"]);
    git(&dir, &["config", "commit.gpgsign", "false"]);
    std::fs::write(dir.join("a.txt"), "one\ntwo\nthree\n").unwrap();
    std::fs::write(dir.join("b.txt"), "b\n").unwrap();
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "init"]);
    (tmp, dir)
}

/// `git` from `PATH`, ignoring the global and system config.
pub fn isolated_git() -> GitBinary {
    GitBinary::new(PathBuf::from("git"))
        .with_env("GIT_CONFIG_GLOBAL", NULL)
        .with_env("GIT_CONFIG_SYSTEM", NULL)
}
