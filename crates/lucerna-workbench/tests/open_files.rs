//! Open files the worktree does not follow (outside the root, or
//! ignored) still reload: the user's `settings.json` through the config
//! watcher, the rest through the open-file watcher. GPUI tests deliver
//! the watchers' batches themselves, as the real ones would; the watcher
//! thread is tested on its own.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{Entity, TestAppContext, VisualTestContext};
use lucerna_project::WorktreeEvent;
use lucerna_settings::{Keymap, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{
    AppState, ConfigPaths, OpenFileWatcher, OpenMode, Workbench, reload_config, watch_config,
};
use tempfile::TempDir;

/// A project with a tracked `a.txt` and a gitignored `ignored.txt`, and a
/// user directory standing in for `~/.config/lucerna`.
struct Dirs {
    root: TempDir,
    user: TempDir,
}

impl Dirs {
    fn new() -> Dirs {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(".gitignore"), "ignored.txt\n").unwrap();
        std::fs::write(root.path().join("a.txt"), "a\n").unwrap();
        std::fs::write(root.path().join("ignored.txt"), "old\n").unwrap();
        let user = tempfile::tempdir().unwrap();
        std::fs::write(user.path().join("settings.json"), "{}\n").unwrap();
        Dirs { root, user }
    }

    fn settings(&self) -> PathBuf {
        self.user.path().join("settings.json")
    }
}

fn init(dirs: &Dirs, cx: &mut TestAppContext) {
    let (keymap, _) = Keymap::load(Keymap::default_bindings(), None);
    let themes = ThemeRegistry::builtin();
    let theme = themes.get_or_default("One Dark");
    cx.set_global(AppState {
        settings: SettingsStore::with_defaults(),
        keymap,
        themes,
        theme,
        theme_generation: 0,
        languages: Arc::new(LanguageRegistry::bundled()),
    });
    let paths = ConfigPaths::with_user_dir(dirs.user.path(), dirs.root.path());
    cx.update(|cx| watch_config(paths, cx));
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

fn open<'a>(
    root: &Path,
    cx: &'a mut TestAppContext,
) -> (Entity<Workbench>, &'a mut VisualTestContext) {
    let root = root.to_path_buf();
    let (wb, cx) = cx.add_window_view(move |window, cx| Workbench::new(root, window, cx));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !wb.read_with(cx, |wb, _| wb.worktree().snapshot().scan_complete()) {
        assert!(Instant::now() < deadline, "worktree scan did not finish");
        std::thread::sleep(Duration::from_millis(10));
    }
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::ScanComplete, cx)
    });
    draw(cx);
    (wb, cx)
}

fn open_file(wb: &Entity<Workbench>, path: &Path, cx: &mut VisualTestContext) -> Entity<CodeView> {
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(path), OpenMode::Pinned, cx)
    });
    draw(cx);
    cx.update(|_, cx| {
        wb.read(cx)
            .editor_groups()
            .read(cx)
            .active_body()
            .expect("drawn")
            .downcast::<CodeView>()
            .expect("a code view")
    })
}

fn text(view: &Entity<CodeView>, cx: &mut VisualTestContext) -> String {
    view.read_with(cx, |v, _| {
        let s = v.snapshot();
        s.text_for_range(0..s.len_bytes())
    })
}

#[gpui::test]
fn open_user_settings_reloads_after_a_write(cx: &mut TestAppContext) {
    let dirs = Dirs::new();
    init(&dirs, cx);
    let (wb, cx) = open(dirs.root.path(), cx);
    let view = open_file(&wb, &dirs.settings(), cx);
    assert_eq!(text(&view, cx), "{}\n");

    std::fs::write(dirs.settings(), "{ \"editor.tabSize\": 2 }\n").unwrap();
    // What the config watcher delivers: the real path of the file.
    let changed = dirs.settings().canonicalize().unwrap();
    cx.update(|_, cx| reload_config(&[changed], cx));
    draw(cx);
    assert_eq!(text(&view, cx), "{ \"editor.tabSize\": 2 }\n");
}

#[gpui::test]
fn open_gitignored_file_reloads_after_a_write(cx: &mut TestAppContext) {
    let dirs = Dirs::new();
    init(&dirs, cx);
    let (wb, cx) = open(dirs.root.path(), cx);
    let ignored = dirs.root.path().join("ignored.txt");
    let view = open_file(&wb, Path::new("ignored.txt"), cx);
    open_file(&wb, Path::new("a.txt"), cx);
    open_file(&wb, &dirs.settings(), cx);
    // The worktree follows `a.txt`; the open-file watcher the others.
    let mut want = vec![ignored.clone(), dirs.settings()];
    want.sort();
    assert_eq!(
        wb.read_with(cx, |wb, _| wb.watched_open_files().to_vec()),
        want
    );

    std::fs::write(&ignored, "new\n").unwrap();
    // What the open-file watcher delivers: the real path of the file.
    let changed = ignored.canonicalize().unwrap();
    wb.update(cx, |wb, cx| wb.on_open_file_changes(&[changed], cx));
    draw(cx);
    assert_eq!(text(&view, cx), "new\n");

    std::fs::remove_file(&ignored).unwrap();
    let changed = dirs.root.path().canonicalize().unwrap().join("ignored.txt");
    wb.update(cx, |wb, cx| wb.on_open_file_changes(&[changed], cx));
    draw(cx);
    assert_eq!(
        view.read_with(cx, |v, _| v.doc_state().clone()),
        lucerna_workbench::DocState::Deleted
    );
}

/// The real watcher: a write to a watched file arrives as one debounced
/// batch naming it; a write to its unwatched neighbour does not.
#[test]
fn open_file_watcher_reports_writes_to_watched_files_only() {
    let dir = TempDir::new().unwrap();
    let watched = dir.path().join("watched.txt");
    let other = dir.path().join("other.txt");
    std::fs::write(&watched, "1").unwrap();
    std::fs::write(&other, "1").unwrap();
    let (tx, rx) = crossbeam_channel::unbounded();
    let watcher = OpenFileWatcher::start(move |batch| {
        let _ = tx.send(batch);
    })
    .unwrap();
    watcher.set_files(vec![watched.clone()]);
    // FSEvents may report events from before the watch started; let them pass.
    std::thread::sleep(Duration::from_millis(300));
    while rx.try_recv().is_ok() {}

    std::fs::write(&other, "2").unwrap();
    std::fs::write(&watched, "2").unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let real = watched.canonicalize().unwrap();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let batch: Vec<PathBuf> = rx.recv_timeout(left).expect("a batch arrives");
        assert!(
            batch.iter().all(|p| p.ends_with("watched.txt")),
            "only watched files are reported: {batch:?}"
        );
        if batch.contains(&real) || batch.contains(&watched) {
            break;
        }
    }
}

/// R27: a file opened alone shows in the tree by itself; its folder is
/// never scanned, and the file is followed by the open-file watcher.
#[gpui::test]
fn single_file_mode_never_scans(cx: &mut TestAppContext) {
    let dirs = Dirs::new();
    init(&dirs, cx);
    let root = dirs.root.path().to_path_buf();
    let file = root.join("a.txt");
    let (wb, cx) = {
        let (root, file) = (root.clone(), file.clone());
        cx.add_window_view(move |window, cx| Workbench::single_file(root, &file, None, window, cx))
    };
    std::thread::sleep(Duration::from_millis(200));
    let entries = |cx: &mut VisualTestContext| {
        wb.read_with(cx, |wb, _| {
            let snapshot = wb.worktree().snapshot();
            assert!(snapshot.scan_complete());
            snapshot
                .entries()
                .iter()
                .map(|e| e.path.to_path_buf())
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(
        entries(cx),
        [PathBuf::from("a.txt")],
        "only the file, no scan"
    );
    open_file(&wb, &file, cx);
    assert_eq!(
        wb.read_with(cx, |wb, _| wb.watched_open_files().to_vec()),
        [file]
    );
}
