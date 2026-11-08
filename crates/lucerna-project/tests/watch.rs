use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use lucerna_project::{ChangeKind, ScanOptions, Worktree, WorktreeEvent};
use tempfile::TempDir;

const TIMEOUT: Duration = Duration::from_secs(2);
const SILENCE: Duration = Duration::from_millis(300);

fn started(root: &Path) -> Worktree {
    let wt = Worktree::start(root.to_path_buf(), ScanOptions::default());
    let rx = wt.events();
    loop {
        let ev = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("scan timeout");
        if ev == WorktreeEvent::ScanComplete {
            return wt;
        }
    }
}

/// Collects `(path, kind)` of every `Changed` event until `done` holds or the
/// timeout passes. Returns what was seen.
fn collect_until(
    wt: &Worktree,
    done: impl Fn(&[(String, ChangeKind)]) -> bool,
) -> Vec<(String, ChangeKind)> {
    let rx = wt.events();
    let deadline = Instant::now() + TIMEOUT;
    let mut seen = Vec::new();
    while !done(&seen) {
        let left = deadline.saturating_duration_since(Instant::now());
        let Ok(ev) = rx.recv_timeout(left) else { break };
        if let WorktreeEvent::Changed(changes) = ev {
            seen.extend(
                changes
                    .into_iter()
                    .map(|c| (c.path.to_string_lossy().into_owned(), c.kind)),
            );
        }
    }
    seen
}

fn has(seen: &[(String, ChangeKind)], path: &str, kind: ChangeKind) -> bool {
    seen.iter().any(|(p, k)| p == path && *k == kind)
}

#[test]
fn create_modify_delete_reported() {
    let dir = TempDir::new().unwrap();
    let r = dir.path();
    let wt = started(r);

    fs::write(r.join("new.txt"), "1").unwrap();
    let seen = collect_until(&wt, |s| has(s, "new.txt", ChangeKind::Added));
    assert!(has(&seen, "new.txt", ChangeKind::Added), "{seen:?}");
    assert!(wt.snapshot().contains(Path::new("new.txt")));

    // Let the debounce window close so the next write is its own batch.
    std::thread::sleep(Duration::from_millis(200));
    fs::write(r.join("new.txt"), "22").unwrap();
    let seen = collect_until(&wt, |s| has(s, "new.txt", ChangeKind::Modified));
    assert!(has(&seen, "new.txt", ChangeKind::Modified), "{seen:?}");

    fs::remove_file(r.join("new.txt")).unwrap();
    let seen = collect_until(&wt, |s| has(s, "new.txt", ChangeKind::Removed));
    assert!(has(&seen, "new.txt", ChangeKind::Removed), "{seen:?}");
    assert!(!wt.snapshot().contains(Path::new("new.txt")));
}

#[test]
fn atomic_rename_reports_modified_only() {
    let dir = TempDir::new().unwrap();
    let r = dir.path();
    fs::write(r.join("f"), "old").unwrap();
    let wt = started(r);

    fs::write(r.join("f.tmp"), "new").unwrap();
    fs::rename(r.join("f.tmp"), r.join("f")).unwrap();

    let seen = collect_until(&wt, |s| has(s, "f", ChangeKind::Modified));
    assert!(has(&seen, "f", ChangeKind::Modified), "{seen:?}");
    // Let any straggling batch arrive.
    let more = collect_until(&wt, |_| false);
    let all: Vec<_> = seen.into_iter().chain(more).collect();
    for (path, kind) in &all {
        assert!(
            !(path == "f.tmp" || (path == "f" && *kind != ChangeKind::Modified)),
            "unexpected {path} {kind:?} in {all:?}"
        );
    }
    let snap = wt.snapshot();
    assert!(snap.contains(Path::new("f")));
    assert!(!snap.contains(Path::new("f.tmp")));
}

#[test]
fn gitignored_changes_silent() {
    let dir = TempDir::new().unwrap();
    let r = dir.path();
    fs::write(r.join(".gitignore"), "out/\n").unwrap();
    fs::create_dir(r.join("out")).unwrap();
    let wt = started(r);

    fs::write(r.join("out/x"), "1").unwrap();
    let rx = wt.events();
    let deadline = Instant::now() + SILENCE;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(ev @ (WorktreeEvent::Changed(_) | WorktreeEvent::Rescanned)) => {
                panic!("ignored change produced {ev:?}")
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    assert!(!wt.snapshot().contains(Path::new("out/x")));
}

#[test]
fn bulk_write_5000_files_rescans() {
    let dir = TempDir::new().unwrap();
    let r = dir.path();
    let wt = started(r);

    fs::create_dir(r.join("bulk")).unwrap();
    for i in 0..5000 {
        fs::write(r.join(format!("bulk/f{i}.txt")), "x").unwrap();
    }

    let rx = wt.events();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let ev = rx.recv_timeout(left).expect("no Rescanned event");
        if ev == WorktreeEvent::Rescanned {
            break;
        }
    }
    // Later batches may still be applying; wait for the snapshot to settle.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let n = wt
            .snapshot()
            .files()
            .filter(|e| e.path.starts_with("bulk"))
            .count();
        if n == 5000 {
            break;
        }
        assert!(Instant::now() < deadline, "snapshot has {n} of 5000 files");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn ignore_file_inside_ignored_dir_is_silent() {
    let dir = TempDir::new().unwrap();
    let r = dir.path();
    fs::write(r.join(".gitignore"), "out/\n").unwrap();
    fs::create_dir(r.join("out")).unwrap();
    let wt = started(r);

    fs::write(r.join("out/.gitignore"), "*\n").unwrap();
    let rx = wt.events();
    let deadline = Instant::now() + SILENCE;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(ev @ (WorktreeEvent::Changed(_) | WorktreeEvent::Rescanned)) => {
                panic!("ignored change produced {ev:?}")
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
}

#[test]
fn removing_a_directory_reports_each_path_once() {
    let dir = TempDir::new().unwrap();
    let r = dir.path();
    for f in ["d/a.txt", "d/sub/b.txt", "d/sub/deep/c.txt", "keep.txt"] {
        fs::create_dir_all(r.join(f).parent().unwrap()).unwrap();
        fs::write(r.join(f), "x").unwrap();
    }
    let wt = started(r);

    fs::remove_dir_all(r.join("d")).unwrap();
    let want = [
        "d",
        "d/a.txt",
        "d/sub",
        "d/sub/b.txt",
        "d/sub/deep",
        "d/sub/deep/c.txt",
    ];
    let seen = collect_until(&wt, |s| want.iter().all(|w| has(s, w, ChangeKind::Removed)));
    // Straggling batches would show up as duplicates.
    let more = collect_until(&wt, |_| false);
    let all: Vec<_> = seen.into_iter().chain(more).collect();
    for w in want {
        let n = all.iter().filter(|(p, _)| p == w).count();
        assert_eq!(n, 1, "{w} reported {n} times in {all:?}");
        assert!(has(&all, w, ChangeKind::Removed), "{all:?}");
    }
    assert_eq!(all.len(), want.len(), "{all:?}");
    assert!(wt.snapshot().contains(Path::new("keep.txt")));
    assert!(!wt.snapshot().contains(Path::new("d")));
}

#[test]
fn edit_after_the_replay_window_is_reported_despite_old_mtime() {
    let dir = TempDir::new().unwrap();
    let r = dir.path();
    fs::write(r.join("f"), "old").unwrap();
    let wt = started(r);
    // Past the 2 s replay grace period.
    std::thread::sleep(Duration::from_millis(2300));

    let file = fs::OpenOptions::new()
        .write(true)
        .open(r.join("f"))
        .unwrap();
    use std::io::Write as _;
    (&file).write_all(b"new contents").unwrap();
    let past = std::time::SystemTime::now() - Duration::from_secs(86_400);
    file.set_times(fs::FileTimes::new().set_modified(past))
        .unwrap();
    drop(file);

    let seen = collect_until(&wt, |s| has(s, "f", ChangeKind::Modified));
    assert!(has(&seen, "f", ChangeKind::Modified), "{seen:?}");
}
