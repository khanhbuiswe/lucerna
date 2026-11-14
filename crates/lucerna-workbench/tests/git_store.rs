//! `GitStore`: a serial job queue that turns `git status` (and blob diffs)
//! into immutable snapshots. These tests drive real `git` processes in
//! temporary repositories, isolated from the developer's git config.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use gpui::{AppContext as _, Entity, TestAppContext};
use lucerna_git::{FileStatus, GitBinary, diff_hunks};
use lucerna_settings::{Keymap, Layer, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_text::BufferSnapshot;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::{AppState, GitState, GitStore};
use tempfile::TempDir;

#[cfg(windows)]
const NULL: &str = "NUL";
#[cfg(not(windows))]
const NULL: &str = "/dev/null";

fn init(cx: &mut TestAppContext, user_settings: Option<&str>) {
    let (keymap, error) = Keymap::load(Keymap::default_bindings(), None);
    assert!(error.is_none(), "keymap error: {error:?}");
    let mut settings = SettingsStore::with_defaults();
    if let Some(json) = user_settings {
        settings.set_layer(Layer::User, json).unwrap();
    }
    let themes = ThemeRegistry::builtin();
    let theme = themes.get_or_default("One Dark");
    cx.set_global(AppState {
        settings,
        keymap,
        themes,
        theme,
        theme_generation: 0,
        languages: Arc::new(LanguageRegistry::bundled()),
        languages_generation: 0,
        icon_themes: lucerna_theme::IconThemeRegistry::build(&[]).0,
        icons: lucerna_theme::IconThemeRegistry::build(&[])
            .0
            .resolve("Lucide"),
    });
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", NULL)
        .env("GIT_CONFIG_SYSTEM", NULL)
        .output()
        .expect("spawn git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A fresh repo on `main` with committed `a.txt` and `b.txt`.
fn repo() -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().canonicalize().unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    git(&dir, &["config", "user.name", "Test"]);
    git(&dir, &["config", "user.email", "test@example.com"]);
    git(&dir, &["config", "commit.gpgsign", "false"]);
    std::fs::write(dir.join("a.txt"), "one\ntwo\nthree\n").unwrap();
    std::fs::write(dir.join("b.txt"), "b\n").unwrap();
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "init"]);
    (tmp, dir)
}

fn isolated_git() -> GitBinary {
    GitBinary::new(PathBuf::from("git"))
        .with_env("GIT_CONFIG_GLOBAL", NULL)
        .with_env("GIT_CONFIG_SYSTEM", NULL)
}

/// A trusted store on `root`, settled after its first rescan.
fn store(root: &Path, cx: &mut TestAppContext) -> Entity<GitStore> {
    let root = root.to_path_buf();
    let store = cx.new(|cx| {
        let mut store = GitStore::new(root, cx);
        store.use_git_for_test(isolated_git());
        store
    });
    store.update(cx, |s, cx| s.set_trusted(true, cx));
    cx.run_until_parked();
    store
}

fn p(s: &str) -> Arc<Path> {
    Arc::from(Path::new(s))
}

fn statuses(store: &Entity<GitStore>, cx: &mut TestAppContext) -> HashMap<Arc<Path>, FileStatus> {
    store.read_with(cx, |s, _| (*s.snapshot().statuses).clone())
}

#[gpui::test]
fn rescan_reports_statuses(cx: &mut TestAppContext) {
    init(cx, None);
    let (_t, dir) = repo();
    std::fs::write(dir.join("a.txt"), "changed\n").unwrap();
    std::fs::write(dir.join("new.txt"), "n\n").unwrap();
    let store = store(&dir, cx);
    assert_eq!(store.read_with(cx, |s, _| s.state()), GitState::Ready);
    let expected = HashMap::from([
        (p("a.txt"), FileStatus::Modified),
        (p("new.txt"), FileStatus::Untracked),
    ]);
    assert_eq!(statuses(&store, cx), expected);
    let snap = store.read_with(cx, |s, _| s.snapshot());
    let head = git(&dir, &["rev-parse", "HEAD"]).trim().to_string();
    assert_eq!(snap.head.as_deref(), Some(head.as_str()));
    assert_eq!(snap.base_rev.as_deref(), Some(head.as_str()));
    assert_eq!(snap.default_branch.as_deref(), Some("main"));
    assert!(snap.generation > 0);
}

#[gpui::test]
fn paths_changed_refreshes_only_given_paths(cx: &mut TestAppContext) {
    init(cx, None);
    let (_t, dir) = repo();
    let store = store(&dir, cx);
    assert!(statuses(&store, cx).is_empty());
    std::fs::write(dir.join("a.txt"), "changed\n").unwrap();
    std::fs::write(dir.join("b.txt"), "changed\n").unwrap();
    store.update(cx, |s, cx| s.paths_changed(vec![p("a.txt")], cx));
    cx.run_until_parked();
    assert_eq!(
        statuses(&store, cx),
        HashMap::from([(p("a.txt"), FileStatus::Modified)])
    );
}

#[gpui::test]
fn burst_of_changes_coalesces_into_one_queued_refresh(cx: &mut TestAppContext) {
    init(cx, None);
    let (_t, dir) = repo();
    let store = store(&dir, cx);
    let before = store.read_with(cx, |s, _| s.jobs_run());
    for i in 0..500 {
        let name = format!("f{i}.txt");
        std::fs::write(dir.join(&name), "x\n").unwrap();
        store.update(cx, |s, cx| s.paths_changed(vec![p(&name)], cx));
    }
    cx.run_until_parked();
    let ran = store.read_with(cx, |s, _| s.jobs_run()) - before;
    assert!((1..=2).contains(&ran), "ran {ran} status jobs");
    assert_eq!(statuses(&store, cx).len(), 500);
}

#[gpui::test]
fn commit_clears_statuses_after_rescan(cx: &mut TestAppContext) {
    init(cx, None);
    let (_t, dir) = repo();
    std::fs::write(dir.join("a.txt"), "changed\n").unwrap();
    let store = store(&dir, cx);
    assert_eq!(statuses(&store, cx).len(), 1);
    git(&dir, &["commit", "-q", "-am", "change"]);
    store.update(cx, |s, cx| s.rescan(cx));
    cx.run_until_parked();
    assert!(statuses(&store, cx).is_empty());
    let head = git(&dir, &["rev-parse", "HEAD"]).trim().to_string();
    let snap = store.read_with(cx, |s, _| s.snapshot());
    assert_eq!(snap.base_rev.as_deref(), Some(head.as_str()));
}

#[gpui::test]
fn default_branch_base_keeps_committed_changes(cx: &mut TestAppContext) {
    init(cx, Some(r#"{ "git.diffBase": "defaultBranch" }"#));
    let (_t, dir) = repo();
    let main = git(&dir, &["rev-parse", "HEAD"]).trim().to_string();
    git(&dir, &["checkout", "-q", "-b", "feature"]);
    std::fs::write(dir.join("a.txt"), "one\nTWO\nthree\n").unwrap();
    git(&dir, &["commit", "-q", "-am", "feature work"]);
    let store = store(&dir, cx);
    assert_eq!(
        statuses(&store, cx),
        HashMap::from([(p("a.txt"), FileStatus::Modified)])
    );
    let snap = store.read_with(cx, |s, _| s.snapshot());
    assert_eq!(snap.base_rev.as_deref(), Some(main.as_str()));
    let text = BufferSnapshot::from_text("one\nTWO\nthree\n");
    store.update(cx, |s, cx| s.track_buffer(p("a.txt"), text, cx));
    cx.executor().advance_clock(Duration::from_millis(50));
    cx.run_until_parked();
    let hunks = store.read_with(cx, |s, _| s.hunks(Path::new("a.txt")));
    assert!(hunks.is_some_and(|h| !h.is_empty()));
}

#[gpui::test]
fn track_buffer_hunks_follow_unsaved_text(cx: &mut TestAppContext) {
    init(cx, None);
    let (_t, dir) = repo();
    let store = store(&dir, cx);
    let unsaved = "zero\none\nthree\nfour\n";
    store.update(cx, |s, cx| {
        s.track_buffer(p("a.txt"), BufferSnapshot::from_text(unsaved), cx)
    });
    // Nothing before the 50 ms debounce has passed.
    cx.executor().advance_clock(Duration::from_millis(40));
    cx.run_until_parked();
    assert!(store.read_with(cx, |s, _| s.hunks(Path::new("a.txt")).is_none()));
    cx.executor().advance_clock(Duration::from_millis(10));
    cx.run_until_parked();
    let hunks = store
        .read_with(cx, |s, _| s.hunks(Path::new("a.txt")))
        .expect("hunks");
    assert_eq!(&*hunks, diff_hunks("one\ntwo\nthree\n", unsaved).as_slice());
    // Untracking frees them.
    store.update(cx, |s, _| s.untrack_buffer(Path::new("a.txt")));
    assert!(store.read_with(cx, |s, _| s.hunks(Path::new("a.txt")).is_none()));
}

#[gpui::test]
fn status_limit_turns_off_auto_refresh(cx: &mut TestAppContext) {
    init(cx, Some(r#"{ "git.statusLimit": 3 }"#));
    let (_t, dir) = repo();
    for i in 0..5 {
        std::fs::write(dir.join(format!("u{i}.txt")), "u\n").unwrap();
    }
    let store = store(&dir, cx);
    assert_eq!(
        store.read_with(cx, |s, _| s.state()),
        GitState::TooManyChanges
    );
    assert_eq!(statuses(&store, cx).len(), 5);
    let before = store.read_with(cx, |s, _| s.jobs_run());
    std::fs::write(dir.join("a.txt"), "changed\n").unwrap();
    store.update(cx, |s, cx| s.paths_changed(vec![p("a.txt")], cx));
    cx.run_until_parked();
    assert_eq!(store.read_with(cx, |s, _| s.jobs_run()), before);
    store.update(cx, |s, cx| s.rescan(cx));
    cx.run_until_parked();
    assert_eq!(store.read_with(cx, |s, _| s.jobs_run()), before + 1);
    assert_eq!(statuses(&store, cx).len(), 6);
}

#[gpui::test]
fn workspace_in_subfolder_maps_paths(cx: &mut TestAppContext) {
    init(cx, None);
    let (_t, dir) = repo();
    std::fs::create_dir(dir.join("sub")).unwrap();
    std::fs::write(dir.join("sub/x.txt"), "x\n").unwrap();
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "sub"]);
    std::fs::write(dir.join("sub/x.txt"), "changed\n").unwrap();
    std::fs::write(dir.join("sub/new.txt"), "n\n").unwrap();
    std::fs::write(dir.join("a.txt"), "outside\n").unwrap();
    let store = store(&dir.join("sub"), cx);
    assert_eq!(
        statuses(&store, cx),
        HashMap::from([
            (p("x.txt"), FileStatus::Modified),
            (p("new.txt"), FileStatus::Untracked),
        ])
    );
    std::fs::write(dir.join("sub/x.txt"), "x\n").unwrap();
    store.update(cx, |s, cx| s.paths_changed(vec![p("x.txt")], cx));
    cx.run_until_parked();
    assert_eq!(
        statuses(&store, cx),
        HashMap::from([(p("new.txt"), FileStatus::Untracked)])
    );
}

#[gpui::test]
fn untrusted_runs_nothing(cx: &mut TestAppContext) {
    init(cx, None);
    let (_t, dir) = repo();
    std::fs::write(dir.join("a.txt"), "changed\n").unwrap();
    assert!(GitStore::has_repo_hint(&dir));
    let store = cx.new(|cx| {
        let mut store = GitStore::new(dir.clone(), cx);
        store.use_git_for_test(isolated_git());
        store
    });
    assert_eq!(store.read_with(cx, |s, _| s.state()), GitState::Off);
    store.update(cx, |s, cx| {
        s.paths_changed(vec![p("a.txt")], cx);
        s.rescan(cx);
        s.track_buffer(p("a.txt"), BufferSnapshot::from_text("x\n"), cx);
    });
    cx.executor().advance_clock(Duration::from_millis(100));
    cx.run_until_parked();
    store.update(cx, |s, cx| s.set_trusted(false, cx));
    cx.run_until_parked();
    assert_eq!(store.read_with(cx, |s, _| s.state()), GitState::Restricted);
    assert_eq!(store.read_with(cx, |s, _| s.jobs_run()), 0);
    assert!(statuses(&store, cx).is_empty());
    assert!(store.read_with(cx, |s, _| s.hunks(Path::new("a.txt")).is_none()));
}

#[gpui::test]
fn diff_base_setting_change_rescans(cx: &mut TestAppContext) {
    init(cx, None);
    let (_t, dir) = repo();
    let main = git(&dir, &["rev-parse", "HEAD"]).trim().to_string();
    git(&dir, &["checkout", "-q", "-b", "feature"]);
    std::fs::write(dir.join("a.txt"), "committed\n").unwrap();
    git(&dir, &["commit", "-q", "-am", "feature work"]);
    let store = store(&dir, cx);
    assert!(statuses(&store, cx).is_empty());
    cx.update_global::<AppState, _>(|state, _| {
        state
            .settings
            .set_layer(Layer::User, r#"{ "git.diffBase": "defaultBranch" }"#)
            .unwrap();
    });
    store.update(cx, |s, cx| s.settings_changed(cx));
    cx.run_until_parked();
    assert_eq!(
        statuses(&store, cx),
        HashMap::from([(p("a.txt"), FileStatus::Modified)])
    );
    let snap = store.read_with(cx, |s, _| s.snapshot());
    assert_eq!(snap.base_rev.as_deref(), Some(main.as_str()));
}
