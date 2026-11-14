//! Interleavings the test scheduler does not produce on its own, driven
//! through the store's internals.

use super::*;
use gpui::{AppContext as _, TestAppContext};
use lucerna_git::diff_hunks;
use std::process::Command;

#[cfg(windows)]
const NULL: &str = "NUL";
#[cfg(not(windows))]
const NULL: &str = "/dev/null";

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", NULL)
        .env("GIT_CONFIG_SYSTEM", NULL)
        .output()
        .expect("spawn git");
    assert!(out.status.success(), "git {args:?} failed");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A buffer tracked after a rescan started, whose hunks were diffed against
/// the old base before the rescan finished, is diffed again once the rescan
/// moves `base_rev`.
#[gpui::test]
fn job_moving_the_base_rediffs_buffers_it_did_not_reload(cx: &mut TestAppContext) {
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = tmp.path().canonicalize().unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    git(&dir, &["config", "user.name", "Test"]);
    git(&dir, &["config", "user.email", "test@example.com"]);
    git(&dir, &["config", "commit.gpgsign", "false"]);
    let old_text = "one\ntwo\nthree\n";
    let text = "one\nTWO\nthree\n";
    std::fs::write(dir.join("a.txt"), old_text).unwrap();
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "one"]);
    let old_head = git(&dir, &["rev-parse", "HEAD"]);
    std::fs::write(dir.join("a.txt"), text).unwrap();
    git(&dir, &["commit", "-q", "-am", "two"]);

    let root = dir.clone();
    let store = cx.new(|cx| {
        let mut store = GitStore::new(root, cx);
        store.use_git_for_test(
            GitBinary::new(PathBuf::from("git"))
                .with_env("GIT_CONFIG_GLOBAL", NULL)
                .with_env("GIT_CONFIG_SYSTEM", NULL),
        );
        store
    });
    store.update(cx, |s, cx| s.set_trusted(true, cx));
    cx.run_until_parked();

    let path: Arc<Path> = Arc::from(Path::new("a.txt"));
    store.update(cx, |s, cx| {
        // The state just before the rescan finishes: base still the old
        // HEAD, and a buffer the job did not reload already diffed against it.
        s.snapshot.base_rev = Some(old_head.clone());
        let snapshot = BufferSnapshot::from_text(text);
        s.buffers.insert(
            path.clone(),
            Tracked {
                snapshot,
                version: 1,
                task: None,
            },
        );
        s.bases.insert(
            path.clone(),
            Base {
                rev: old_head.clone(),
                text: Some(Arc::from(old_text)),
            },
        );
        s.hunks
            .insert(path.clone(), diff_hunks(old_text, text).into());
        let input = JobInput {
            repo: s.repo.clone().unwrap(),
            job: Job::Rescan,
            diff_base: DiffBase::Head,
            status_limit: 10_000,
            prev: s.snapshot.clone(),
            buffers: Vec::new(),
        };
        let out = run_job(input);
        s.running = Some(Task::ready(()));
        s.finish_job(s.epoch, out, cx);
    });
    cx.run_until_parked();
    let hunks = store.read_with(cx, |s, _| s.hunks(&path));
    assert_eq!(hunks.as_deref(), Some(&[][..]));
}
