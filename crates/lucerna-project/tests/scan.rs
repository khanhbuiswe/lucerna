use std::fs;
use std::path::Path;
use std::time::Duration;

use lucerna_project::{EntryKind, ScanOptions, Worktree, WorktreeEvent};
use tempfile::TempDir;

fn touch(root: &Path, rel: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, "x").unwrap();
}

fn wait_for(wt: &Worktree, want: fn(&WorktreeEvent) -> bool) {
    let rx = wt.events();
    loop {
        let ev = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("timed out waiting for event");
        if want(&ev) {
            return;
        }
    }
}

fn wait_complete(wt: &Worktree) {
    wait_for(wt, |e| matches!(e, WorktreeEvent::ScanComplete));
}

fn paths(wt: &Worktree) -> Vec<String> {
    wt.snapshot()
        .entries()
        .iter()
        .map(|e| e.path.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn lists_files_and_dirs_sorted() {
    let dir = TempDir::new().unwrap();
    let r = dir.path();
    touch(r, "b.txt");
    touch(r, "A.txt");
    touch(r, "zdir/inner.rs");
    touch(r, "adir/Z.rs");
    touch(r, "adir/a.rs");
    let wt = Worktree::start(r.to_path_buf(), ScanOptions::default());
    wait_complete(&wt);
    assert_eq!(
        paths(&wt),
        [
            "adir",
            "adir/a.rs",
            "adir/Z.rs",
            "zdir",
            "zdir/inner.rs",
            "A.txt",
            "b.txt"
        ]
    );
    let snap = wt.snapshot();
    assert!(snap.scan_complete());
    assert!(snap.contains(Path::new("adir/a.rs")));
    assert!(!snap.contains(Path::new("nope")));
    let root_children: Vec<_> = snap
        .children(Path::new(""))
        .iter()
        .map(|e| e.path.to_string_lossy().into_owned())
        .collect();
    assert_eq!(root_children, ["adir", "zdir", "A.txt", "b.txt"]);
    assert_eq!(snap.children(Path::new("zdir")).len(), 1);
    assert_eq!(snap.files().count(), 5);
}

#[test]
fn respects_gitignore_without_git_dir() {
    let dir = TempDir::new().unwrap();
    let r = dir.path();
    fs::write(r.join(".gitignore"), "out/\n").unwrap();
    touch(r, "out/x");
    touch(r, "src/main.rs");
    let wt = Worktree::start(r.to_path_buf(), ScanOptions::default());
    wait_complete(&wt);
    let p = paths(&wt);
    assert!(!p.iter().any(|x| x.starts_with("out")), "{p:?}");
    assert!(p.contains(&"src/main.rs".to_string()));
}

#[test]
fn hidden_files_shown_but_files_exclude_applies() {
    let dir = TempDir::new().unwrap();
    let r = dir.path();
    touch(r, ".env");
    touch(r, ".git/config");
    let wt = Worktree::start(
        r.to_path_buf(),
        ScanOptions {
            exclude: vec!["**/.git".into()],
        },
    );
    wait_complete(&wt);
    let p = paths(&wt);
    assert!(p.contains(&".env".to_string()), "{p:?}");
    assert!(!p.iter().any(|x| x.starts_with(".git")), "{p:?}");
}

#[test]
fn symlink_loop_terminates() {
    let dir = TempDir::new().unwrap();
    let r = dir.path();
    touch(r, "f.txt");
    std::os::unix::fs::symlink(".", r.join("a")).unwrap();
    let wt = Worktree::start(r.to_path_buf(), ScanOptions::default());
    wait_complete(&wt);
    let snap = wt.snapshot();
    let a = snap
        .entries()
        .iter()
        .find(|e| &*e.path == Path::new("a"))
        .unwrap();
    assert_eq!(a.kind, EntryKind::Symlink);
    assert!(snap.children(Path::new("a")).is_empty());
}

#[test]
fn unicode_and_space_paths() {
    let dir = TempDir::new().unwrap();
    touch(dir.path(), "thư mục/tệp có dấu.rs");
    let wt = Worktree::start(dir.path().to_path_buf(), ScanOptions::default());
    wait_complete(&wt);
    assert!(wt.snapshot().contains(Path::new("thư mục/tệp có dấu.rs")));
}

#[test]
fn set_options_rescans() {
    let dir = TempDir::new().unwrap();
    touch(dir.path(), "x.log");
    touch(dir.path(), "y.rs");
    let wt = Worktree::start(dir.path().to_path_buf(), ScanOptions::default());
    wait_complete(&wt);
    assert!(wt.snapshot().contains(Path::new("x.log")));
    wt.set_options(ScanOptions {
        exclude: vec!["**/*.log".into()],
    });
    wait_for(&wt, |e| matches!(e, WorktreeEvent::Rescanned));
    let snap = wt.snapshot();
    assert!(!snap.contains(Path::new("x.log")));
    assert!(snap.contains(Path::new("y.rs")));
}

#[test]
fn rapid_set_options_coalesce_to_latest() {
    let dir = TempDir::new().unwrap();
    for i in 0..200 {
        touch(dir.path(), &format!("d{}/f{i}.log", i % 10));
        touch(dir.path(), &format!("d{}/f{i}.rs", i % 10));
    }
    let wt = Worktree::start(dir.path().to_path_buf(), ScanOptions::default());
    wait_complete(&wt);
    wt.set_options(ScanOptions {
        exclude: vec!["**/*.rs".into()],
    });
    wt.set_options(ScanOptions {
        exclude: vec!["**/*.log".into()],
    });
    wait_for(&wt, |e| matches!(e, WorktreeEvent::Rescanned));
    let snap = wt.snapshot();
    assert!(snap.contains(Path::new("d0/f0.rs")));
    assert!(!snap.contains(Path::new("d0/f0.log")));
    // Exactly one Rescanned for the two requests.
    std::thread::sleep(Duration::from_millis(200));
    let extra = wt
        .events()
        .try_iter()
        .filter(|e| matches!(e, WorktreeEvent::Rescanned))
        .count();
    assert_eq!(extra, 0);
}
