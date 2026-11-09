use std::fs;
use std::path::PathBuf;

use assert_cmd::Command;
use lucerna::resolve_target;

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
    assert_eq!(resolve_target(None), Ok((canonical(cwd), None)));
}

#[test]
fn resolve_dir() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("sub");
    fs::create_dir(&sub).unwrap();
    assert_eq!(
        resolve_target(Some(sub.clone())),
        Ok((canonical(sub), None))
    );
}

#[test]
fn resolve_file_in_git_repo_uses_repo_root() {
    let repo = tempfile::tempdir().unwrap();
    fs::create_dir(repo.path().join(".git")).unwrap();
    let nested = repo.path().join("src").join("deep");
    fs::create_dir_all(&nested).unwrap();
    let file = nested.join("main.rs");
    fs::write(&file, "fn main() {}\n").unwrap();
    assert_eq!(
        resolve_target(Some(file.clone())),
        Ok((canonical(repo.path()), Some(canonical(file))))
    );
}

#[test]
fn resolve_file_in_git_worktree_uses_dir_with_git_file() {
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
    assert_eq!(
        resolve_target(Some(file.clone())),
        Ok((canonical(&worktree), Some(canonical(file))))
    );
}

#[test]
fn resolve_file_outside_git_uses_parent() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("notes.txt");
    fs::write(&file, "hi\n").unwrap();
    assert_eq!(
        resolve_target(Some(file.clone())),
        Ok((canonical(dir.path()), Some(canonical(file))))
    );
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
