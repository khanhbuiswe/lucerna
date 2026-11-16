use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lucerna_project::{ScanOptions, Worktree, WorktreeEvent, WorktreeSnapshot};
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

/// `a/b/c/d/e` (5 levels down) is the unloaded folder.
fn tree_to_h(root: &Path) {
    touch(root, "a/b/c/d/e/f/g/h.txt");
}

/// Waits until `done` holds for the snapshot, checking after each event.
fn wait_until(wt: &Worktree, done: impl Fn(&WorktreeSnapshot) -> bool) {
    let rx = wt.events();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done(&wt.snapshot()) {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(rx.recv_timeout(left).is_ok(), "timed out");
    }
}

/// Waits for the `Loaded` that answers a load.
fn wait_loaded(wt: &Worktree) {
    wait_event(wt, |e| matches!(e, WorktreeEvent::Loaded(_)));
}

/// Waits for an event that `want` accepts.
fn wait_event(wt: &Worktree, want: impl Fn(&WorktreeEvent) -> bool) {
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

#[test]
fn load_reads_one_level() {
    let tmp = TempDir::new().unwrap();
    tree_to_h(tmp.path());
    let wt = scan(tmp.path(), tmp.path(), Some(5));
    wt.load(Path::new("a/b/c/d/e"));
    wait_loaded(&wt);
    let snap = wt.snapshot();
    assert!(snap.contains(Path::new("a/b/c/d/e/f")));
    assert!(snap.is_unloaded(Path::new("a/b/c/d/e/f")));
    assert!(!snap.is_unloaded(Path::new("a/b/c/d/e")));
    assert!(!snap.contains(Path::new("a/b/c/d/e/f/g")));
}

#[test]
fn load_path_of_file_loads_ancestors() {
    let tmp = TempDir::new().unwrap();
    tree_to_h(tmp.path());
    let wt = scan(tmp.path(), tmp.path(), Some(5));
    wt.load(Path::new("a/b/c/d/e/f/g/h.txt"));
    wait_loaded(&wt);
    let snap = wt.snapshot();
    let file = Path::new("a/b/c/d/e/f/g/h.txt");
    assert!(snap.contains(file));
    for ancestor in file.ancestors() {
        assert!(!snap.is_unloaded(ancestor), "{}", ancestor.display());
    }
    assert!(snap.unloaded().is_empty());
}

#[test]
fn load_of_nested_repo_scans_it_fully() {
    let tmp = TempDir::new().unwrap();
    tree_to_h(tmp.path());
    fs::create_dir_all(tmp.path().join("a/b/c/d/e/repo/.git")).unwrap();
    touch(tmp.path(), "a/b/c/d/e/repo/x/y/z.txt");
    let wt = scan(tmp.path(), tmp.path(), Some(5));
    wt.load(Path::new("a/b/c/d/e"));
    wait_loaded(&wt);
    let snap = wt.snapshot();
    assert!(snap.contains(Path::new("a/b/c/d/e/repo/x/y/z.txt")));
    assert!(!snap.contains(Path::new("a/b/c/d/e/repo/.git")));
    assert!(!snap.is_unloaded(Path::new("a/b/c/d/e/repo")));
}

#[test]
fn new_file_inside_unloaded_folder_stays_hidden() {
    let tmp = TempDir::new().unwrap();
    tree_to_h(tmp.path());
    let wt = scan(tmp.path(), tmp.path(), Some(5));
    touch(tmp.path(), "a/b/c/d/e/new.txt");
    touch(tmp.path(), "a/top.txt");
    wait_until(&wt, |s| s.contains(Path::new("a/top.txt")));
    assert!(!wt.snapshot().contains(Path::new("a/b/c/d/e/new.txt")));
}

#[test]
fn loaded_folder_survives_rescan() {
    let tmp = TempDir::new().unwrap();
    tree_to_h(tmp.path());
    let wt = scan(tmp.path(), tmp.path(), Some(5));
    wt.load(Path::new("a/b/c/d/e"));
    wait_loaded(&wt);
    wt.set_options(ScanOptions {
        exclude: vec!["**/zzz".into()],
        scan_depth: Some(5),
        home: Some(tmp.path().to_path_buf()),
    });
    wait_event(&wt, |e| matches!(e, WorktreeEvent::Rescanned));
    let snap = wt.snapshot();
    assert!(snap.contains(Path::new("a/b/c/d/e/f")));
    assert!(snap.is_unloaded(Path::new("a/b/c/d/e/f")));
    assert!(!snap.is_unloaded(Path::new("a/b/c/d/e")));
}

#[test]
fn deleting_unloaded_folder_removes_it() {
    let tmp = TempDir::new().unwrap();
    tree_to_h(tmp.path());
    let wt = scan(tmp.path(), tmp.path(), Some(5));
    fs::remove_dir_all(tmp.path().join("a/b/c/d/e")).unwrap();
    wait_until(&wt, |s| !s.contains(Path::new("a/b/c/d/e")));
    assert!(wt.snapshot().unloaded().is_empty());
}

#[test]
fn loading_empty_folder_still_reports_change() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("a/b/c/d/e")).unwrap();
    let wt = scan(tmp.path(), tmp.path(), Some(5));
    assert!(wt.snapshot().is_unloaded(Path::new("a/b/c/d/e")));
    wt.load(Path::new("a/b/c/d/e"));
    wait_loaded(&wt);
    assert!(wt.snapshot().unloaded().is_empty());
}

#[test]
fn load_of_loaded_path_does_nothing() {
    let tmp = TempDir::new().unwrap();
    tree_to_h(tmp.path());
    let wt = scan(tmp.path(), tmp.path(), Some(5));
    let rx = wt.events();
    wt.load(Path::new("a/b"));
    wt.load(Path::new("../outside"));
    wt.load(Path::new("a/b/c/d/e"));
    // The first `Loaded` answers the last request: the others sent none.
    let folders = loop {
        match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
            WorktreeEvent::Loaded(folders) => break folders,
            _ => continue,
        }
    };
    assert_eq!(folders, [Arc::from(Path::new("a/b/c/d/e"))]);
}

#[test]
fn load_sends_loaded_not_changed() {
    let tmp = TempDir::new().unwrap();
    tree_to_h(tmp.path());
    let wt = scan(tmp.path(), tmp.path(), Some(5));
    let rx = wt.events();
    wt.load(Path::new("a/b/c/d/e/f/g/h.txt"));
    loop {
        match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
            WorktreeEvent::Loaded(folders) => {
                assert_eq!(folders.len(), 3, "{folders:?}");
                break;
            }
            WorktreeEvent::Changed(changes) => {
                assert!(
                    !changes.iter().any(|c| c.path.starts_with("a/b/c/d/e")),
                    "load reported as a change: {changes:?}"
                );
            }
            _ => {}
        }
    }
    // Nothing about the loaded folders follows either.
    while let Ok(ev) = rx.recv_timeout(Duration::from_millis(300)) {
        if let WorktreeEvent::Changed(changes) = ev {
            assert!(!changes.iter().any(|c| c.path.starts_with("a/b/c/d/e")));
        }
    }
}

/// `.git` above `home` is not this project's repository: depth applies.
#[test]
fn repo_above_home_does_not_count() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir(tmp.path().join(".git")).unwrap();
    let (home, root) = (tmp.path().join("home"), tmp.path().join("home/proj"));
    deep_tree(&root);
    let snap = scan(&root, &home, Some(2)).snapshot();
    assert!(snap.is_unloaded(Path::new("a/b")));
    assert!(!snap.contains(Path::new("a/b/c")));
}

#[test]
fn gitignore_above_home_is_ignored() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join(".gitignore"), "*.log\n").unwrap();
    let (home, root) = (tmp.path().join("home"), tmp.path().join("home/proj"));
    touch(&root, "a.log");
    let wt = scan(&root, &home, None);
    assert!(wt.snapshot().contains(Path::new("a.log")));
    // A watcher-reported file goes through `IgnoreChecker`.
    touch(&root, "b.log");
    wait_until(&wt, |s| s.contains(Path::new("b.log")));
}

#[test]
fn gitignore_in_home_still_applies() {
    let tmp = TempDir::new().unwrap();
    let (home, root) = (tmp.path().join("home"), tmp.path().join("home/proj"));
    fs::create_dir_all(&home).unwrap();
    fs::write(home.join(".gitignore"), "*.log\n").unwrap();
    touch(&root, "a.log");
    touch(&root, "keep.txt");
    let wt = scan(&root, &home, None);
    assert!(wt.snapshot().contains(Path::new("keep.txt")));
    assert!(!wt.snapshot().contains(Path::new("a.log")));
    touch(&root, "b.log");
    touch(&root, "c.txt");
    wait_until(&wt, |s| s.contains(Path::new("c.txt")));
    assert!(!wt.snapshot().contains(Path::new("b.log")));
}
