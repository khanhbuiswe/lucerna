use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use lucerna::{Target, prompts_for_folder, resolve_target, resolve_target_with_home};

#[test]
fn version_flag_prints_name_and_version() {
    let home = tempfile::tempdir().unwrap();
    Command::cargo_bin("luc")
        .unwrap()
        .env("HOME", home.path())
        .arg("--version")
        .assert()
        .success()
        .stdout("luc 0.1.0\n");
}

fn canonical(path: impl Into<PathBuf>) -> PathBuf {
    fs::canonicalize(path.into()).unwrap()
}

#[test]
fn resolve_none_is_cwd() {
    let cwd = std::env::current_dir().unwrap();
    assert_eq!(resolve_target(None), Ok(Target::folder(canonical(cwd))));
}

#[test]
fn resolve_dir() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("sub");
    fs::create_dir(&sub).unwrap();
    assert_eq!(
        resolve_target(Some(sub.clone())),
        Ok(Target::folder(canonical(sub)))
    );
}

/// A file opens alone, as in Zed and VS Code: its folder (or repository)
/// is never scanned, and the root is the file's folder.
fn lone(file: &Path) -> Target {
    let file = canonical(file);
    Target {
        root: file.parent().unwrap().to_path_buf(),
        file: Some(file),
        single_file: true,
    }
}

#[test]
fn resolve_file_in_git_repo_opens_alone() {
    let repo = tempfile::tempdir().unwrap();
    fs::create_dir(repo.path().join(".git")).unwrap();
    let nested = repo.path().join("src").join("deep");
    fs::create_dir_all(&nested).unwrap();
    let file = nested.join("main.rs");
    fs::write(&file, "fn main() {}\n").unwrap();
    assert_eq!(resolve_target(Some(file.clone())), Ok(lone(&file)));
}

#[test]
fn resolve_file_in_git_worktree_opens_alone() {
    let outer = tempfile::tempdir().unwrap();
    fs::create_dir(outer.path().join(".git")).unwrap();
    let worktree = outer.path().join("worktree");
    let nested = worktree.join("src");
    fs::create_dir_all(&nested).unwrap();
    fs::write(
        worktree.join(".git"),
        "gitdir: ../.git/worktrees/worktree\n",
    )
    .unwrap();
    let file = nested.join("lib.rs");
    fs::write(&file, "\n").unwrap();
    assert_eq!(resolve_target(Some(file.clone())), Ok(lone(&file)));
}

/// A file in a plain folder such as `~/Downloads` opens alone; the folder
/// is not listed in the sidebar.
#[test]
fn resolve_file_in_plain_folder_opens_alone() {
    let home = tempfile::tempdir().unwrap();
    let downloads = home.path().join("Downloads");
    fs::create_dir(&downloads).unwrap();
    let file = downloads.join("notes.md");
    fs::write(&file, "hi\n").unwrap();
    let target = resolve_target_with_home(Some(file.clone()), Some(home.path())).unwrap();
    assert_eq!(target, lone(&file));
    // A loose file in `$HOME` itself too.
    let in_home = home.path().join("todo.md");
    fs::write(&in_home, "").unwrap();
    let target = resolve_target_with_home(Some(in_home.clone()), Some(home.path())).unwrap();
    assert_eq!(target, lone(&in_home));
}

#[test]
fn resolve_folders_stay_folders() {
    let home = tempfile::tempdir().unwrap();
    let repo = home.path().join("repo");
    fs::create_dir_all(repo.join(".git")).unwrap();
    let sub = home.path().join("sub");
    fs::create_dir(&sub).unwrap();
    for dir in [home.path().to_path_buf(), repo, sub] {
        let target = resolve_target_with_home(Some(dir.clone()), Some(home.path())).unwrap();
        assert_eq!(target, Target::folder(canonical(&dir)));
    }
}

#[test]
fn resolve_missing_errors() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nope");
    assert_eq!(
        resolve_target(Some(missing.clone())),
        Err(format!("path not found: {}", missing.display()))
    );
}

/// `luc <path>` and the paths Finder sends the running app resolve the
/// same way.
#[test]
fn resolve_matches_the_router() {
    use lucerna_workbench::router::Launch;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.rs");
    fs::write(&file, "\n").unwrap();
    let missing = dir.path().join("nope");
    for path in [dir.path().to_path_buf(), file, missing] {
        let cli = resolve_target(Some(path.clone()));
        let router = Launch::for_path(path).map(|l| Target {
            root: l.root,
            file: l.file,
            single_file: l.single_file,
        });
        assert_eq!(cli, router);
    }
}

#[test]
fn missing_path_exits_1() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nope");
    let home = tempfile::tempdir().unwrap();
    let output = Command::cargo_bin("luc")
        .unwrap()
        .env("HOME", home.path())
        .arg(&missing)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(&format!("path not found: {}", missing.display())),
        "stderr: {stderr}"
    );
}

#[test]
fn prompts_for_folder_only_without_path_at_filesystem_root() {
    use std::path::Path;
    // Finder and the Dock start the app with no arguments in `/`.
    assert!(prompts_for_folder(None, Path::new("/")));
    // A terminal `luc` with no path opens the current directory.
    assert!(!prompts_for_folder(None, Path::new("/Users/me/project")));
    // An explicit path always wins, even `/`.
    assert!(!prompts_for_folder(Some(Path::new("/")), Path::new("/")));
}
