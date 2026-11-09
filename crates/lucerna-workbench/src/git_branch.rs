//! The git branch shown in the title bar, read straight from the
//! repository's `HEAD` file (no git library): the nearest `.git` at or
//! above the workspace root, following a `.git` file's `gitdir:` line as
//! worktrees and submodules use.

use std::io::Read as _;
use std::path::{Path, PathBuf};

/// Branch names longer than this are cut.
pub const MAX_BRANCH_CHARS: usize = 64;

/// Bytes read from `.git` and `HEAD`; both are a line in practice.
const MAX_READ: u64 = 4096;

/// The `HEAD` file of the repository holding `root`, if any. Only a
/// regular `.git` file is read (as a `gitdir:` pointer), so a FIFO or a
/// device named `.git` cannot block or flood the caller.
pub fn head_path(root: &Path) -> Option<PathBuf> {
    let dot_git = root
        .ancestors()
        .map(|dir| dir.join(".git"))
        .find(|p| p.exists())?;
    let meta = std::fs::metadata(&dot_git).ok()?;
    let git_dir = if meta.is_dir() {
        dot_git
    } else if meta.is_file() {
        let contents = read_bounded(&dot_git)?;
        let target = contents
            .lines()
            .find_map(|line| line.strip_prefix("gitdir:"))?
            .trim();
        // Relative to the folder holding the `.git` file.
        dot_git.parent()?.join(target)
    } else {
        return None;
    };
    Some(git_dir.join("HEAD")).filter(|head| is_regular_file(head))
}

fn is_regular_file(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file())
}

/// At most [`MAX_READ`] bytes of the regular file `path`, lossily decoded.
fn read_bounded(path: &Path) -> Option<String> {
    if !is_regular_file(path) {
        return None;
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_READ)
        .read_to_end(&mut bytes)
        .ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// What a `HEAD` file's first line names: the branch of
/// `ref: refs/heads/<name>`, any other ref's path, or the first 7 chars
/// of a detached commit SHA. Names are cut to [`MAX_BRANCH_CHARS`].
pub fn parse_head(contents: &str) -> Option<String> {
    let head = contents.lines().next()?.trim();
    if let Some(reference) = head.strip_prefix("ref:") {
        let reference = reference.trim();
        let name = reference.strip_prefix("refs/heads/").unwrap_or(reference);
        return (!name.is_empty()).then(|| name.chars().take(MAX_BRANCH_CHARS).collect());
    }
    let is_sha = head.len() >= 7 && head.chars().all(|c| c.is_ascii_hexdigit());
    is_sha.then(|| head[..7].to_string())
}

/// The branch (or short SHA) checked out in the repository holding
/// `root`; `None` outside a repository.
pub fn read_branch(root: &Path) -> Option<String> {
    parse_head(&read_bounded(&head_path(root)?)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn head_names_a_branch_or_a_short_sha() {
        assert_eq!(
            parse_head("ref: refs/heads/main\n").as_deref(),
            Some("main")
        );
        assert_eq!(
            parse_head("ref: refs/heads/feat/ui-polish").as_deref(),
            Some("feat/ui-polish")
        );
        assert_eq!(
            parse_head("1016b91c0ffee1234567890abcdef1234567890a\n").as_deref(),
            Some("1016b91")
        );
        assert_eq!(parse_head(""), None);
        assert_eq!(parse_head("garbage"), None);
    }

    #[test]
    fn reads_the_branch_from_a_git_dir_above_the_root() {
        let dir = tempfile::TempDir::new().unwrap();
        let git = dir.path().join(".git");
        fs::create_dir(&git).unwrap();
        fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        let nested = dir.path().join("crates/app");
        fs::create_dir_all(&nested).unwrap();
        assert_eq!(read_branch(dir.path()).as_deref(), Some("main"));
        assert_eq!(read_branch(&nested).as_deref(), Some("main"));
        assert_eq!(head_path(&nested), Some(git.join("HEAD")));
    }

    #[test]
    fn reads_a_detached_head() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::create_dir(dir.path().join(".git")).unwrap();
        fs::write(
            dir.path().join(".git/HEAD"),
            "8f037b9aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n",
        )
        .unwrap();
        assert_eq!(read_branch(dir.path()).as_deref(), Some("8f037b9"));
    }

    #[test]
    fn follows_a_gitdir_file() {
        let dir = tempfile::TempDir::new().unwrap();
        // The main checkout's per-worktree folder.
        let real = dir.path().join("main/.git/worktrees/ui");
        fs::create_dir_all(&real).unwrap();
        fs::write(real.join("HEAD"), "ref: refs/heads/feat/ui-polish\n").unwrap();
        // An absolute gitdir...
        let worktree = dir.path().join("ui");
        fs::create_dir(&worktree).unwrap();
        fs::write(
            worktree.join(".git"),
            format!("gitdir: {}\n", real.display()),
        )
        .unwrap();
        assert_eq!(read_branch(&worktree).as_deref(), Some("feat/ui-polish"));
        // ...and one relative to the `.git` file's folder.
        let sub = dir.path().join("sub");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join(".git"), "gitdir: ../main/.git/worktrees/ui\n").unwrap();
        assert_eq!(read_branch(&sub).as_deref(), Some("feat/ui-polish"));
    }

    #[test]
    fn no_repository_no_branch() {
        let dir = tempfile::TempDir::new().unwrap();
        // Temp folders are not inside a repository.
        assert_eq!(read_branch(dir.path()), None);
        assert_eq!(head_path(dir.path()), None);
        // A `.git` file that names no gitdir.
        fs::write(dir.path().join(".git"), "nonsense\n").unwrap();
        assert_eq!(read_branch(dir.path()), None);
    }

    #[test]
    fn head_uses_its_first_line_and_caps_the_name() {
        assert_eq!(
            parse_head("ref: refs/heads/main\nref: refs/heads/other\n").as_deref(),
            Some("main")
        );
        let long = format!("ref: refs/heads/{}", "b".repeat(200));
        assert_eq!(
            parse_head(&long).map(|n| n.chars().count()),
            Some(MAX_BRANCH_CHARS)
        );
    }

    #[test]
    fn an_oversized_head_is_read_bounded() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::create_dir(dir.path().join(".git")).unwrap();
        let huge = format!("ref: refs/heads/{}\n", "x".repeat(1 << 20));
        fs::write(dir.path().join(".git/HEAD"), huge).unwrap();
        assert_eq!(
            read_branch(dir.path()).map(|n| n.len()),
            Some(MAX_BRANCH_CHARS)
        );
    }

    /// Runs `f` on a thread; panics if it does not return within 2 s.
    fn within_timeout<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        rx.recv_timeout(std::time::Duration::from_secs(2))
            .expect("reading the branch blocked")
    }

    #[test]
    fn a_git_fifo_is_skipped_without_blocking() {
        let dir = tempfile::TempDir::new().unwrap();
        let fifo = dir.path().join(".git");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(made.success());
        let root = dir.path().to_path_buf();
        assert_eq!(within_timeout(move || read_branch(&root)), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_git_symlink_to_a_device_is_skipped() {
        let dir = tempfile::TempDir::new().unwrap();
        std::os::unix::fs::symlink("/dev/zero", dir.path().join(".git")).unwrap();
        let root = dir.path().to_path_buf();
        assert_eq!(within_timeout(move || read_branch(&root)), None);
    }
}
