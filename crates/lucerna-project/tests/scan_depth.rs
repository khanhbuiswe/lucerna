use std::fs;
use std::path::Path;
use std::time::Duration;

use lucerna_project::{ScanOptions, Worktree, WorktreeEvent};
use tempfile::TempDir;

fn touch(root: &Path, rel: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, "x").unwrap();
}

fn wait_complete(wt: &Worktree) {
    let rx = wt.events();
    loop {
        let ev = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("timed out waiting for scan");
        if matches!(ev, WorktreeEvent::ScanComplete) {
            return;
        }
    }
}

fn deep_tree(root: &Path) {
    touch(root, "a/b/c/d/e/f/g.txt");
    touch(root, "a/b/c/d/x.txt");
}

fn scan(root: &Path, home: &Path, depth: Option<usize>) -> Worktree {
    let wt = Worktree::start(
        root.to_path_buf(),
        ScanOptions {
            exclude: Vec::new(),
            scan_depth: depth,
            home: Some(home.to_path_buf()),
        },
    );
    wait_complete(&wt);
    wt
}

#[test]
fn deep_folders_outside_repo_are_unloaded() {
    let tmp = TempDir::new().unwrap();
    deep_tree(tmp.path());
    let snap = scan(tmp.path(), tmp.path(), Some(5)).snapshot();
    assert!(snap.contains(Path::new("a/b/c/d/x.txt")));
    assert!(snap.contains(Path::new("a/b/c/d/e")));
    assert!(snap.is_unloaded(Path::new("a/b/c/d/e")));
    assert!(!snap.contains(Path::new("a/b/c/d/e/f")));
    assert_eq!(snap.unloaded().len(), 1);
}

#[test]
fn repo_root_scans_fully() {
    let tmp = TempDir::new().unwrap();
    deep_tree(tmp.path());
    fs::create_dir(tmp.path().join(".git")).unwrap();
    let snap = scan(tmp.path(), tmp.path(), Some(5)).snapshot();
    assert!(snap.contains(Path::new("a/b/c/d/e/f/g.txt")));
    assert!(snap.unloaded().is_empty());
}

#[test]
fn root_below_repo_scans_fully() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir(tmp.path().join(".git")).unwrap();
    deep_tree(&tmp.path().join("sub"));
    let snap = scan(&tmp.path().join("sub"), tmp.path(), Some(5)).snapshot();
    assert!(snap.contains(Path::new("a/b/c/d/e/f/g.txt")));
    assert!(snap.unloaded().is_empty());
}

#[test]
fn nested_repo_below_limit_scans_fully() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("a/b/c/d/repo/.git")).unwrap();
    touch(tmp.path(), "a/b/c/d/repo/x/y/z.txt");
    let snap = scan(tmp.path(), tmp.path(), Some(5)).snapshot();
    assert!(snap.contains(Path::new("a/b/c/d/repo/x/y/z.txt")));
    assert!(!snap.is_unloaded(Path::new("a/b/c/d/repo")));
}

#[test]
fn no_limit_scans_fully() {
    let tmp = TempDir::new().unwrap();
    deep_tree(tmp.path());
    let snap = scan(tmp.path(), tmp.path(), None).snapshot();
    assert!(snap.contains(Path::new("a/b/c/d/e/f/g.txt")));
    assert!(snap.unloaded().is_empty());
}
