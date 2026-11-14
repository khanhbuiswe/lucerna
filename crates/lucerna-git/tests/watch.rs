#![cfg(target_os = "macos")]

use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use lucerna_git::GitDirWatcher;

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn commit_signals_once() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    git(dir, &["init", "-q"]);
    std::fs::write(dir.join("a.txt"), "a\n").unwrap();
    git(dir, &["add", "a.txt"]);
    std::thread::sleep(Duration::from_millis(500));

    let count = Arc::new(AtomicUsize::new(0));
    let c = count.clone();
    let _w = GitDirWatcher::start(&[dir.join(".git")], move || {
        c.fetch_add(1, Ordering::SeqCst);
    })
    .unwrap();
    std::thread::sleep(Duration::from_millis(300));

    git(dir, &["commit", "-q", "-m", "x"]);
    std::thread::sleep(Duration::from_millis(1000));
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[test]
fn objects_only_changes_are_ignored() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    git(dir, &["init", "-q"]);
    std::thread::sleep(Duration::from_millis(500));
    let count = Arc::new(AtomicUsize::new(0));
    let c = count.clone();
    let _w = GitDirWatcher::start(&[dir.join(".git")], move || {
        c.fetch_add(1, Ordering::SeqCst);
    })
    .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    std::fs::write(dir.join(".git/objects/junk"), "x").unwrap();
    std::fs::write(dir.join(".git/FETCH_HEAD"), "x").unwrap();
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(count.load(Ordering::SeqCst), 0);
}
