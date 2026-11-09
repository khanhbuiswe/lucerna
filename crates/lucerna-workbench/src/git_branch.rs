//! The git branch shown in the title bar, read straight from the
//! repository's `HEAD` file (no git library): the nearest `.git` at or
//! above the workspace root, following a `.git` file's `gitdir:` line as
//! worktrees and submodules use.

use std::path::{Path, PathBuf};

/// The `HEAD` file of the repository holding `root`, if any.
pub fn head_path(root: &Path) -> Option<PathBuf> {
    let dot_git = root
        .ancestors()
        .map(|dir| dir.join(".git"))
        .find(|p| p.exists())?;
    let git_dir = if dot_git.is_dir() {
        dot_git
    } else {
        let contents = std::fs::read_to_string(&dot_git).ok()?;
        let target = contents
            .lines()
            .find_map(|line| line.strip_prefix("gitdir:"))?
            .trim();
        // Relative to the folder holding the `.git` file.
        dot_git.parent()?.join(target)
    };
    Some(git_dir.join("HEAD")).filter(|head| head.is_file())
}

/// What a `HEAD` file names: the branch of `ref: refs/heads/<name>`, any
/// other ref's path, or the first 7 chars of a detached commit SHA.
pub fn parse_head(contents: &str) -> Option<String> {
    let head = contents.trim();
    if let Some(reference) = head.strip_prefix("ref:") {
        let reference = reference.trim();
        let name = reference.strip_prefix("refs/heads/").unwrap_or(reference);
        return (!name.is_empty()).then(|| name.to_string());
    }
    let is_sha = head.len() >= 7 && head.chars().all(|c| c.is_ascii_hexdigit());
    is_sha.then(|| head[..7].to_string())
}

/// The branch (or short SHA) checked out in the repository holding
/// `root`; `None` outside a repository.
pub fn read_branch(root: &Path) -> Option<String> {
    parse_head(&std::fs::read_to_string(head_path(root)?).ok()?)
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
}
