//! Open files the worktree does not follow (outside the root, or
//! ignored) still reload: the user's `settings.json` through the config
//! watcher, the rest through the open-file watcher. GPUI tests deliver
//! the watchers' batches themselves, as the real ones would; the watcher
//! thread is tested on its own.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{Entity, Focusable as _, TestAppContext, VisualTestContext};
use lucerna_project::WorktreeEvent;
use lucerna_settings::{Keymap, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_text::Point;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{
    AppState, ConfigPaths, FileView, MarkdownPreview, OpenFileWatcher, OpenMode, REPARSE_DELAY,
    TabItem, Workbench, editor_for, reload_config, watch_config,
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
    let theme = themes.get_or_default("Lucerna Dark");
    cx.set_global(AppState {
        settings: SettingsStore::with_defaults(),
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
    let paths = ConfigPaths::user_only_with_user_dir(dirs.user.path());
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

/// Waits up to 10 s for a batch that names `file`, as given or by its
/// real path.
fn wait_for(rx: &crossbeam_channel::Receiver<Vec<PathBuf>>, file: &Path) {
    let real = file.canonicalize().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let batch: Vec<PathBuf> = rx.recv_timeout(left).expect("a batch arrives");
        if batch.contains(&real) || batch.iter().any(|p| p == file) {
            return;
        }
    }
}

/// Removes `dir` and lets the batches about the removal pass, so the
/// batches that follow come from what the test does next. inotify reports
/// the deletion of `file` (in `dir`), which may come late: it is waited
/// for up to 2 s. FSEvents may name only `dir`, which reaches no batch.
fn remove_and_settle(rx: &crossbeam_channel::Receiver<Vec<PathBuf>>, dir: &Path, file: &Path) {
    let real = file.canonicalize().unwrap();
    std::fs::remove_dir_all(dir).unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while let Ok(batch) = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        if batch.iter().any(|p| *p == real || p == file) {
            break;
        }
    }
    std::thread::sleep(Duration::from_millis(300));
    while rx.try_recv().is_ok() {}
}

/// An ignored file whose folder is removed (`dist/` by a clean build)
/// still reloads once the folder and the file are written again, even
/// after another tab opened and closed in between.
#[test]
fn open_file_watcher_follows_a_recreated_parent() {
    let root = TempDir::new().unwrap();
    let dist = root.path().join("dist");
    let bundle = dist.join("bundle.js");
    let env = root.path().join(".env");
    std::fs::create_dir_all(&dist).unwrap();
    std::fs::write(&bundle, "1").unwrap();
    std::fs::write(&env, "1").unwrap();
    let (tx, rx) = crossbeam_channel::unbounded();
    let watcher = OpenFileWatcher::start(move |batch| {
        let _ = tx.send(batch);
    })
    .unwrap();
    watcher.set_files(vec![bundle.clone()]);
    std::thread::sleep(Duration::from_millis(300));
    while rx.try_recv().is_ok() {}

    remove_and_settle(&rx, &dist, &bundle);
    watcher.set_files(vec![bundle.clone(), env.clone()]);
    watcher.set_files(vec![bundle.clone()]);

    std::fs::create_dir_all(&dist).unwrap();
    std::fs::write(&bundle, "2").unwrap();
    wait_for(&rx, &bundle);
}

/// The watch moves down to `dist/js` at once when both folders come
/// back, and a file written before it got there is still reported.
#[test]
fn open_file_watcher_follows_a_recreated_nested_parent() {
    let root = TempDir::new().unwrap();
    let dist = root.path().join("dist");
    let js = dist.join("js");
    let bundle = js.join("bundle.js");
    std::fs::create_dir_all(&js).unwrap();
    std::fs::write(&bundle, "1").unwrap();
    let (tx, rx) = crossbeam_channel::unbounded();
    let watcher = OpenFileWatcher::start(move |batch| {
        let _ = tx.send(batch);
    })
    .unwrap();
    watcher.set_files(vec![bundle.clone()]);
    std::thread::sleep(Duration::from_millis(300));
    while rx.try_recv().is_ok() {}

    remove_and_settle(&rx, &dist, &bundle);

    std::fs::create_dir_all(&js).unwrap();
    std::fs::write(&bundle, "2").unwrap();
    wait_for(&rx, &bundle);
}

/// A file opened alone shows in the tree by itself; its folder is
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

/// `README.md` (30 lines), `docs/b.md`, `x.rs` and `y.rs` added to the
/// project.
fn md_dirs() -> Dirs {
    let dirs = Dirs::new();
    let root = dirs.root.path();
    let lines: String = (0..30).map(|i| format!("line {i}\n\n")).collect();
    std::fs::write(root.join("README.md"), lines).unwrap();
    std::fs::create_dir(root.join("docs")).unwrap();
    std::fs::write(root.join("docs/b.md"), "# b\n").unwrap();
    std::fs::write(root.join("x.rs"), "fn x() {}\n").unwrap();
    std::fs::write(root.join("y.rs"), "fn y() {}\n").unwrap();
    dirs
}

/// The active tab's item.
fn active_item(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> TabItem {
    cx.update(|_, cx| {
        let groups = wb.read(cx).editor_groups().read(cx);
        groups.model().active_tab().expect("a tab").item.clone()
    })
}

fn open_rel(wb: &Entity<Workbench>, rel: &str, mode: OpenMode, cx: &mut VisualTestContext) {
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new(rel)), mode, cx)
    });
    draw(cx);
}

fn abs(dirs: &Dirs, rel: &str) -> Arc<Path> {
    Arc::from(dirs.root.path().join(rel))
}

fn user_settings(dirs: &Dirs, json: &str, cx: &mut VisualTestContext) {
    std::fs::write(dirs.settings(), json).unwrap();
    let changed = dirs.settings().canonicalize().unwrap();
    cx.update(|_, cx| reload_config(&[changed], cx));
    draw(cx);
}

#[gpui::test]
fn md_opens_as_preview_by_default(cx: &mut TestAppContext) {
    let dirs = md_dirs();
    init(&dirs, cx);
    let (wb, cx) = open(dirs.root.path(), cx);
    open_rel(&wb, "README.md", OpenMode::Preview, cx);
    assert_eq!(
        active_item(&wb, cx),
        TabItem::MarkdownPreview(abs(&dirs, "README.md"))
    );
}

#[gpui::test]
fn md_jump_opens_source(cx: &mut TestAppContext) {
    let dirs = md_dirs();
    init(&dirs, cx);
    let (wb, cx) = open(dirs.root.path(), cx);
    let point = Point { row: 10, column: 0 };
    wb.update(cx, |wb, cx| {
        wb.open_path_at(
            Arc::from(Path::new("README.md")),
            OpenMode::Preview,
            point,
            cx,
        )
    });
    draw(cx);
    assert_eq!(active_item(&wb, cx), TabItem::File(abs(&dirs, "README.md")));
}

#[gpui::test]
fn point_less_open_reuses_the_open_source(cx: &mut TestAppContext) {
    let dirs = md_dirs();
    init(&dirs, cx);
    let (wb, cx) = open(dirs.root.path(), cx);
    let point = Point { row: 10, column: 0 };
    wb.update(cx, |wb, cx| {
        wb.open_path_at(
            Arc::from(Path::new("README.md")),
            OpenMode::Pinned,
            point,
            cx,
        )
    });
    draw(cx);
    open_rel(&wb, "x.rs", OpenMode::Pinned, cx);
    open_rel(&wb, "README.md", OpenMode::Pinned, cx);
    assert_eq!(active_item(&wb, cx), TabItem::File(abs(&dirs, "README.md")));
    let tabs = cx.update(|_, cx| {
        wb.read(cx).editor_groups().read(cx).model().groups()[0]
            .tabs
            .len()
    });
    assert_eq!(tabs, 2);
}

#[gpui::test]
fn go_back_returns_to_source(cx: &mut TestAppContext) {
    let dirs = md_dirs();
    init(&dirs, cx);
    let (wb, cx) = open(dirs.root.path(), cx);
    open_rel(&wb, "x.rs", OpenMode::Pinned, cx);
    let jump = |rel: &'static str, row: u32, cx: &mut VisualTestContext| {
        wb.update(cx, |wb, cx| {
            let point = Point { row, column: 0 };
            wb.jump_to(Arc::from(Path::new(rel)), point, OpenMode::Preview, cx)
        });
        draw(cx);
    };
    jump("README.md", 10, cx);
    // Replaces README.md's preview tab.
    jump("y.rs", 0, cx);
    cx.update(|window, cx| {
        wb.update(cx, |wb, cx| {
            wb.run_command("workbench.action.navigateBack", None, window, cx)
        })
    });
    draw(cx);
    assert_eq!(active_item(&wb, cx), TabItem::File(abs(&dirs, "README.md")));
}

#[gpui::test]
fn association_default_opens_source(cx: &mut TestAppContext) {
    let dirs = md_dirs();
    init(&dirs, cx);
    let (wb, cx) = open(dirs.root.path(), cx);
    user_settings(
        &dirs,
        r#"{ "workbench.editorAssociations": { "*.md": "default" } }"#,
        cx,
    );
    open_rel(&wb, "README.md", OpenMode::Pinned, cx);
    assert_eq!(active_item(&wb, cx), TabItem::File(abs(&dirs, "README.md")));
}

#[gpui::test]
fn association_full_path_glob(cx: &mut TestAppContext) {
    let dirs = md_dirs();
    init(&dirs, cx);
    let (wb, cx) = open(dirs.root.path(), cx);
    user_settings(
        &dirs,
        r#"{ "workbench.editorAssociations": {
               "*.md": "default",
               "docs/**/*.md": "vscode.markdown.preview.editor" } }"#,
        cx,
    );
    open_rel(&wb, "docs/b.md", OpenMode::Pinned, cx);
    assert_eq!(
        active_item(&wb, cx),
        TabItem::MarkdownPreview(abs(&dirs, "docs/b.md"))
    );
    open_rel(&wb, "README.md", OpenMode::Pinned, cx);
    assert_eq!(active_item(&wb, cx), TabItem::File(abs(&dirs, "README.md")));
}

#[gpui::test]
fn rs_unaffected(cx: &mut TestAppContext) {
    let dirs = md_dirs();
    init(&dirs, cx);
    let (wb, cx) = open(dirs.root.path(), cx);
    open_rel(&wb, "x.rs", OpenMode::Preview, cx);
    assert_eq!(active_item(&wb, cx), TabItem::File(abs(&dirs, "x.rs")));
}

fn assoc(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn editor_for_image_extensions_ignore_case() {
    let m = assoc(&[]);
    assert_eq!(
        editor_for(Path::new("/r/a.PNG"), Path::new("a.PNG"), &m),
        FileView::Image
    );
    assert_eq!(
        editor_for(Path::new("/r/i.svg"), Path::new("i.svg"), &m),
        FileView::Image
    );
    assert_eq!(
        editor_for(Path::new("/r/a.rs"), Path::new("a.rs"), &m),
        FileView::Source
    );
}

#[test]
fn editor_for_image_associations() {
    let m = assoc(&[
        ("*.svg", "default"),
        ("*.dat", "imagePreview.previewEditor"),
    ]);
    assert_eq!(
        editor_for(Path::new("/r/i.svg"), Path::new("i.svg"), &m),
        FileView::Source
    );
    assert_eq!(
        editor_for(Path::new("/r/x.dat"), Path::new("x.dat"), &m),
        FileView::Image
    );
}

#[test]
fn editor_for_matches_relative_and_absolute_globs() {
    let map = |pairs: &[(&str, &str)]| -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    };
    let (abs, rel) = (Path::new("/w/docs/a.md"), Path::new("docs/a.md"));
    let preview = "vscode.markdown.preview.editor";
    assert_eq!(editor_for(abs, rel, &map(&[])), FileView::Source);
    assert_eq!(
        editor_for(abs, rel, &map(&[("*.md", preview)])),
        FileView::Preview
    );
    assert_eq!(
        editor_for(
            abs,
            rel,
            &map(&[("*.md", preview), ("docs/*.md", "default")])
        ),
        FileView::Source,
        "the longer glob wins"
    );
    assert_eq!(
        editor_for(abs, rel, &map(&[("/w/**/*.md", preview)])),
        FileView::Preview
    );
    assert_eq!(
        editor_for(abs, rel, &map(&[("*.md", "other.editor")])),
        FileView::Source
    );
}

/// A gitignored Markdown file shown only in the dynamic preview is still
/// watched, so an outside rewrite reaches the preview.
#[gpui::test]
fn dynamic_preview_of_ignored_file_follows_writes(cx: &mut TestAppContext) {
    let dirs = Dirs::new();
    let notes = dirs.root.path().join("notes.md");
    std::fs::write(dirs.root.path().join(".gitignore"), "notes.md\n").unwrap();
    std::fs::write(&notes, "# Old\n").unwrap();
    init(&dirs, cx);
    let (wb, cx) = open(dirs.root.path(), cx);
    wb.update(cx, |wb, cx| {
        wb.update_editor_groups(cx, |m| {
            m.open_dynamic_preview(Arc::from(notes.as_path()));
        })
    });
    draw(cx);
    assert_eq!(
        wb.read_with(cx, |wb, _| wb.watched_open_files().to_vec()),
        vec![notes.clone()],
        "no tab has the file's path, but the dynamic preview shows it"
    );

    std::fs::write(&notes, "# New title\n").unwrap();
    let changed = notes.canonicalize().unwrap();
    wb.update(cx, |wb, cx| wb.on_open_file_changes(&[changed], cx));
    cx.executor().advance_clock(REPARSE_DELAY);
    draw(cx);
    let preview = cx.update(|_, cx| {
        let groups = wb.read(cx).editor_groups().read(cx);
        let id = groups.model().dynamic_preview().unwrap().1.id;
        groups
            .body_of(id)
            .unwrap()
            .downcast::<MarkdownPreview>()
            .unwrap()
    });
    let text = preview.read_with(cx, |p, _| p.blocks().map(|b| format!("{:?}", b.blocks())));
    assert!(text.unwrap().contains("New title"));
}

/// A project with `CLAUDE.md` and `AGENTS.md -> CLAUDE.md`, as agents'
/// repos have, plus `docs/OUT.md` linking to `outside.md` beyond the root.
#[cfg(unix)]
fn symlink_dirs() -> (Dirs, TempDir) {
    let dirs = Dirs::new();
    let root = dirs.root.path();
    std::fs::write(root.join("CLAUDE.md"), "one\ntwo\nthree\n").unwrap();
    std::os::unix::fs::symlink("CLAUDE.md", root.join("AGENTS.md")).unwrap();
    let outside = TempDir::new().unwrap();
    std::fs::write(outside.path().join("outside.md"), "far\n").unwrap();
    std::fs::create_dir(root.join("docs")).unwrap();
    std::os::unix::fs::symlink(outside.path().join("outside.md"), root.join("docs/OUT.md"))
        .unwrap();
    (dirs, outside)
}

/// `.md` files open as text, so the tests see their code views.
#[cfg(unix)]
fn md_as_source(dirs: &Dirs, cx: &mut VisualTestContext) {
    user_settings(
        dirs,
        r#"{ "workbench.editorAssociations": { "*.md": "default" } }"#,
        cx,
    );
}

/// What the worktree and the open-file watcher deliver after another
/// program changed `rel` (relative to the root) and the watched `links`.
#[cfg(unix)]
fn report(wb: &Entity<Workbench>, rel: &str, links: &[&Path], cx: &mut VisualTestContext) {
    let change = lucerna_project::Change {
        path: Arc::from(Path::new(rel)),
        kind: lucerna_project::ChangeKind::Modified,
    };
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::Changed(vec![change]), cx)
    });
    let links: Vec<PathBuf> = links.iter().map(|p| p.to_path_buf()).collect();
    wb.update(cx, |wb, cx| wb.on_open_file_changes(&links, cx));
    draw(cx);
}

/// T4-2: a tab opened through a symlink (`AGENTS.md -> CLAUDE.md`) is
/// watched, and reloads when the agent writes the target, as does the
/// target's own tab: two documents for one file, both current.
#[cfg(unix)]
#[gpui::test]
fn symlinked_tab_reloads_when_its_target_changes(cx: &mut TestAppContext) {
    let (dirs, outside) = symlink_dirs();
    init(&dirs, cx);
    let (wb, cx) = open(dirs.root.path(), cx);
    md_as_source(&dirs, cx);
    let root = dirs.root.path();
    let (agents, out) = (root.join("AGENTS.md"), root.join("docs/OUT.md"));
    let link = open_file(&wb, Path::new("AGENTS.md"), cx);
    let target = open_file(&wb, Path::new("CLAUDE.md"), cx);
    let far = open_file(&wb, Path::new("docs/OUT.md"), cx);
    assert_eq!(text(&link, cx), "one\ntwo\nthree\n");
    assert_eq!(text(&far, cx), "far\n");
    let mut want = vec![agents.clone(), out.clone()];
    want.sort();
    assert_eq!(
        wb.read_with(cx, |wb, _| wb.watched_open_files().to_vec()),
        want,
        "tabs reached through a symlink are watched; CLAUDE.md is the tree's"
    );

    std::fs::write(root.join("CLAUDE.md"), "one\ntwo\nTHREE\n").unwrap();
    report(&wb, "CLAUDE.md", &[&agents], cx);
    assert_eq!(text(&link, cx), "one\ntwo\nTHREE\n");
    assert_eq!(text(&target, cx), "one\ntwo\nTHREE\n");

    // A target outside the root.
    std::fs::write(outside.path().join("outside.md"), "near\n").unwrap();
    wb.update(cx, |wb, cx| {
        wb.on_open_file_changes(std::slice::from_ref(&out), cx)
    });
    draw(cx);
    assert_eq!(text(&far, cx), "near\n");
}

/// The agent points the link elsewhere: the tab shows the new target,
/// and follows it from then on. Removing the target marks it deleted.
#[cfg(unix)]
#[gpui::test]
fn retargeted_symlink_tab_follows_the_new_target(cx: &mut TestAppContext) {
    let (dirs, _outside) = symlink_dirs();
    init(&dirs, cx);
    let (wb, cx) = open(dirs.root.path(), cx);
    md_as_source(&dirs, cx);
    let root = dirs.root.path();
    let agents = root.join("AGENTS.md");
    let link = open_file(&wb, Path::new("AGENTS.md"), cx);

    std::fs::write(root.join("docs/RULES.md"), "rules\n").unwrap();
    std::fs::remove_file(&agents).unwrap();
    std::os::unix::fs::symlink("docs/RULES.md", &agents).unwrap();
    report(&wb, "AGENTS.md", &[&agents], cx);
    assert_eq!(text(&link, cx), "rules\n");

    std::fs::write(root.join("docs/RULES.md"), "more rules\n").unwrap();
    report(&wb, "docs/RULES.md", &[&agents], cx);
    assert_eq!(text(&link, cx), "more rules\n");

    std::fs::remove_file(root.join("docs/RULES.md")).unwrap();
    report(&wb, "docs/RULES.md", &[&agents], cx);
    assert_eq!(
        link.read_with(cx, |v, _| v.doc_state().clone()),
        lucerna_workbench::DocState::Deleted
    );
}

/// Unsaved edits in a symlinked tab merge with the agent's write to the
/// target, as in any tab.
#[cfg(unix)]
#[gpui::test]
fn dirty_symlinked_tab_merges_the_targets_change(cx: &mut TestAppContext) {
    let (dirs, _outside) = symlink_dirs();
    init(&dirs, cx);
    let (wb, cx) = open(dirs.root.path(), cx);
    md_as_source(&dirs, cx);
    let root = dirs.root.path();
    let agents = root.join("AGENTS.md");
    let link = open_file(&wb, Path::new("AGENTS.md"), cx);
    cx.update(|window, cx| {
        let focus = link.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    });
    draw(cx);
    cx.simulate_input("x");
    draw(cx);
    assert_eq!(text(&link, cx), "xone\ntwo\nthree\n");

    std::fs::write(root.join("CLAUDE.md"), "one\ntwo\nTHREE\n").unwrap();
    report(&wb, "CLAUDE.md", &[&agents], cx);
    assert_eq!(text(&link, cx), "xone\ntwo\nTHREE\n");
    let docs = cx.update(|_, cx| wb.read(cx).documents().clone());
    docs.read_with(cx, |d, _| {
        assert!(d.is_dirty(&agents));
        assert!(!d.has_conflict(&agents));
    });
}

/// The real watcher follows a link's target in another directory, and
/// reports the link (as given) when the target changes or goes away, or
/// once retargeted, when the new target changes.
#[cfg(unix)]
#[test]
fn open_file_watcher_follows_a_links_target() {
    let (dir, far) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    let target = far.path().join("target.md");
    let link = dir.path().join("link.md");
    std::fs::write(&target, "1").unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let (tx, rx) = crossbeam_channel::unbounded();
    let watcher = OpenFileWatcher::start(move |batch| {
        let _ = tx.send(batch);
    })
    .unwrap();
    watcher.set_files(vec![link.clone()]);
    std::thread::sleep(Duration::from_millis(300));
    while rx.try_recv().is_ok() {}

    let wait_for_link = |what: &str| {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let batch: Vec<PathBuf> = rx
                .recv_timeout(left)
                .unwrap_or_else(|_| panic!("a batch arrives after {what}"));
            if batch.contains(&link) {
                break;
            }
        }
    };
    std::fs::write(&target, "2").unwrap();
    wait_for_link("a write");
    std::fs::remove_file(&target).unwrap();
    wait_for_link("a removal");

    // Pointed elsewhere, and told again: the new target is followed.
    let other = TempDir::new().unwrap();
    let new_target = other.path().join("new.md");
    std::fs::write(&new_target, "1").unwrap();
    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink(&new_target, &link).unwrap();
    watcher.set_files(vec![link.clone()]);
    std::thread::sleep(Duration::from_millis(300));
    while rx.try_recv().is_ok() {}
    std::fs::write(&new_target, "2").unwrap();
    wait_for_link("a write to the new target");
}
