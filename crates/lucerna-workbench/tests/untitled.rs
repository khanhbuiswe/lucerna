//! Untitled documents in `Documents`: made empty and clean, dirty once they
//! hold text, never read from or written to disk, and in the language of
//! `files.defaultLanguage`.

mod support;

use std::fs;
use std::path::Path;
use std::sync::Arc;

use gpui::{Entity, Focusable, TestAppContext, VisualTestContext};
use lucerna_project::WorktreeEvent;
use lucerna_settings::{Keymap, Layer, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{
    AppState, DocState, Documents, OpenMode, SaveOptions, SaveReason, Workbench, load_state,
    state_path,
};
use tempfile::TempDir;

fn init(cx: &mut TestAppContext, user_settings: Option<&str>) {
    let (keymap, error) = Keymap::load(Keymap::default_bindings(), None);
    assert!(error.is_none(), "keymap error: {error:?}");
    let mut settings = SettingsStore::with_defaults();
    if let Some(json) = user_settings {
        settings.set_layer(Layer::User, json).unwrap();
    }
    let themes = ThemeRegistry::builtin();
    let theme = themes.get_or_default("Lucerna Dark");
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

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

fn settle(cx: &mut VisualTestContext) {
    draw(cx);
    cx.run_until_parked();
    draw(cx);
}

/// A workbench on an empty temporary folder.
fn workbench(cx: &mut TestAppContext) -> (TempDir, Entity<Workbench>, &mut VisualTestContext) {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    let (wb, cx) = cx.add_window_view(|window, cx| Workbench::new(root, window, cx));
    settle(cx);
    (dir, wb, cx)
}

fn documents(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Entity<Documents> {
    cx.update(|_, cx| wb.read(cx).documents().clone())
}

fn new_untitled(docs: &Entity<Documents>, cx: &mut VisualTestContext) -> Arc<Path> {
    let key = docs.update(cx, |d, cx| d.new_untitled(cx));
    settle(cx);
    key
}

fn open(docs: &Entity<Documents>, key: &Arc<Path>, cx: &mut VisualTestContext) -> Entity<CodeView> {
    let view = docs.update(cx, |d, cx| d.open(key.clone(), cx));
    settle(cx);
    view
}

fn type_text(
    docs: &Entity<Documents>,
    view: &Entity<CodeView>,
    key: &Path,
    text: &str,
    cx: &mut VisualTestContext,
) {
    let origin = view.entity_id();
    let at = view.read_with(cx, |v, _| v.snapshot().len_bytes());
    docs.update(cx, |d, cx| {
        d.apply_edits(key, vec![(at..at, Arc::from(text))], None, None, origin, cx)
    })
    .expect("the untitled document has text");
    settle(cx);
}

fn no_untitled_on_disk(dir: &TempDir) {
    for entry in fs::read_dir(dir.path()).unwrap() {
        let name = entry.unwrap().file_name();
        assert!(
            !name.to_string_lossy().contains("untitled:"),
            "{name:?} was written to disk"
        );
    }
    assert!(!Path::new("untitled:Untitled-1").exists());
}

#[gpui::test]
fn new_untitled_is_empty_and_clean(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, wb, cx) = workbench(cx);
    let docs = documents(&wb, cx);

    let first = new_untitled(&docs, cx);
    assert_eq!(&*first, Path::new("untitled:Untitled-1"));
    let second = new_untitled(&docs, cx);
    assert_eq!(&*second, Path::new("untitled:Untitled-2"));

    docs.read_with(cx, |d, _| {
        let snapshot = d.snapshot_of(&first).expect("the text is there at once");
        assert_eq!(snapshot.len_bytes(), 0);
        assert_eq!(d.state(&first), Some(&DocState::Ready));
        assert!(!d.is_dirty(&first));
        assert!(d.dirty_paths().is_empty());
    });

    // A new view of an existing Untitled shares its document.
    let view = open(&docs, &first, cx);
    assert_eq!(view.read_with(cx, |v, _| v.snapshot().len_bytes()), 0);
    docs.read_with(cx, |d, _| assert_eq!(d.views_of(&first).len(), 1));
}

#[gpui::test]
fn untitled_with_text_is_dirty(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, wb, cx) = workbench(cx);
    let docs = documents(&wb, cx);
    let key = new_untitled(&docs, cx);
    let view = open(&docs, &key, cx);

    type_text(&docs, &view, &key, "x", cx);
    docs.read_with(cx, |d, _| {
        assert!(d.is_dirty(&key));
        assert_eq!(d.dirty_paths(), vec![key.clone()]);
    });

    // Back to empty: nothing to lose, so clean again.
    let origin = view.entity_id();
    docs.update(cx, |d, cx| {
        d.apply_edits(&key, vec![(0..1, Arc::from(""))], None, None, origin, cx)
    });
    settle(cx);
    docs.read_with(cx, |d, _| {
        assert!(!d.is_dirty(&key));
        assert!(d.dirty_paths().is_empty());
    });
}

#[gpui::test]
fn open_of_an_unknown_untitled_makes_it_empty(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, cx) = workbench(cx);
    let docs = documents(&wb, cx);
    let key: Arc<Path> = Arc::from(Path::new("untitled:Untitled-3"));
    let view = open(&docs, &key, cx);
    assert_eq!(view.read_with(cx, |v, _| v.snapshot().len_bytes()), 0);
    docs.read_with(cx, |d, _| {
        assert_eq!(d.state(&key), Some(&DocState::Ready));
        assert!(!d.is_dirty(&key));
    });
    // The next number skips the one in use only.
    assert_eq!(&*new_untitled(&docs, cx), Path::new("untitled:Untitled-1"));
    assert_eq!(&*new_untitled(&docs, cx), Path::new("untitled:Untitled-2"));
    assert_eq!(&*new_untitled(&docs, cx), Path::new("untitled:Untitled-4"));
    no_untitled_on_disk(&dir);
}

#[gpui::test]
fn untitled_never_touches_disk(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, cx) = workbench(cx);
    let docs = documents(&wb, cx);
    let key = new_untitled(&docs, cx);
    let view = open(&docs, &key, cx);
    type_text(&docs, &view, &key, "hello", cx);

    let opts = SaveOptions {
        reason: SaveReason::Explicit,
        participants: true,
        overwrite: false,
    };
    let save = cx.update(|window, cx| docs.update(cx, |d, cx| d.save(&key, opts, window, cx)));
    settle(cx);
    let saved = futures::FutureExt::now_or_never(save).expect("the save is done");
    assert_eq!(saved, Ok(false));

    let revert = docs.update(cx, |d, cx| d.revert(&key, cx));
    settle(cx);
    futures::FutureExt::now_or_never(revert).expect("the revert is done");

    docs.update(cx, |d, cx| {
        d.on_worktree_event(&WorktreeEvent::Rescanned, cx)
    });
    settle(cx);

    docs.read_with(cx, |d, _| {
        assert_eq!(d.state(&key), Some(&DocState::Ready));
        assert!(d.is_dirty(&key), "save and revert left the text alone");
        let snapshot = d.snapshot_of(&key).unwrap();
        assert_eq!(snapshot.text_for_range(0..snapshot.len_bytes()), "hello");
    });
    no_untitled_on_disk(&dir);
}

#[gpui::test]
fn default_language_applies(cx: &mut TestAppContext) {
    init(cx, Some(r#"{ "files.defaultLanguage": "rust" }"#));
    let (_dir, wb, cx) = workbench(cx);
    let docs = documents(&wb, cx);
    let key = new_untitled(&docs, cx);
    docs.read_with(cx, |d, _| {
        assert_eq!(d.language_of(&key).as_deref(), Some("rust"));
    });
}

#[gpui::test]
fn unknown_default_language_is_plain_text(cx: &mut TestAppContext) {
    init(cx, Some(r#"{ "files.defaultLanguage": "nope" }"#));
    let (dir, wb, cx) = workbench(cx);
    fs::write(dir.path().join("plain.txt"), "").unwrap();
    let docs = documents(&wb, cx);
    let txt: Arc<Path> = Arc::from(Path::new("plain.txt"));
    let _txt_view = open(&docs, &txt, cx);
    let key = new_untitled(&docs, cx);
    docs.read_with(cx, |d, _| {
        assert_eq!(d.language_of(&key), d.language_of(&txt));
    });
}

/// Opens `key` as a tab, as ⌘N will, and types `text` into it.
fn open_tab_and_type(
    wb: &Entity<Workbench>,
    key: &Arc<Path>,
    text: &str,
    cx: &mut VisualTestContext,
) {
    wb.update(cx, |wb, cx| {
        wb.open_source(key.clone(), OpenMode::Pinned, cx)
    });
    settle(cx);
    let docs = documents(wb, cx);
    let view = open(&docs, key, cx);
    type_text(&docs, &view, key, text, cx);
}

#[gpui::test]
fn untitled_never_reaches_lsp(cx: &mut TestAppContext) {
    use lucerna_lsp::test_support::{FakeLauncher, FakeServer};
    use support::lsp::{
        TRUSTED, count, lsp_workbench_with, open as open_file, settle as lsp_settle,
    };

    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
    let fake = FakeServer::new();
    let launcher = Arc::new(FakeLauncher::new().with_server("rust-analyzer", fake.clone()));
    let settings = r#"{ "session.trustAllWorktrees": true, "files.defaultLanguage": "rust" }"#;
    assert!(TRUSTED.contains("trustAllWorktrees"));
    let (wb, cx) = lsp_workbench_with(dir.path(), launcher, settings, cx);

    // The real file starts the server and is the only document it hears of.
    open_file(&wb, "src/main.rs", cx);
    assert_eq!(count(&fake, "textDocument/didOpen"), 1);

    let docs = documents(&wb, cx);
    let key = new_untitled(&docs, cx);
    let view = open(&docs, &key, cx);
    type_text(&docs, &view, &key, "fn x() {}", cx);
    lsp_settle(cx);

    assert_eq!(count(&fake, "textDocument/didOpen"), 1);
    assert_eq!(count(&fake, "textDocument/didChange"), 0);
    assert_eq!(count(&fake, "textDocument/didSave"), 0);
}

#[gpui::test]
fn untitled_never_reaches_git_or_watcher(cx: &mut TestAppContext) {
    init(cx, None);
    let (_tmp, root) = support::git::repo();
    let root_path = root.clone();
    let (wb, cx) = cx.add_window_view(|window, cx| Workbench::new(root_path, window, cx));
    settle(cx);
    let docs = documents(&wb, cx);
    let key = docs.update(cx, |d, cx| d.new_untitled(cx));
    settle(cx);
    open_tab_and_type(&wb, &key, "hello", cx);

    cx.update(|_, cx| {
        let wb = wb.read(cx);
        assert!(!wb.git().read(cx).is_tracking(&key));
        assert!(
            wb.open_tab_paths(cx)
                .iter()
                .all(|p| !p.starts_with("untitled:")),
            "{:?}",
            wb.open_tab_paths(cx)
        );
        assert!(
            wb.watched_open_files()
                .iter()
                .all(|p| !p.starts_with("untitled:")),
            "{:?}",
            wb.watched_open_files()
        );
    });
    docs.read_with(cx, |d, _| assert!(!d.follows_disk(&key)));
    assert!(!root.join("untitled:Untitled-1").exists());
}

#[gpui::test]
fn untitled_not_in_workspace_state_or_recent(cx: &mut TestAppContext) {
    init(cx, None);
    let root = TempDir::new().unwrap();
    let data = TempDir::new().unwrap();
    fs::write(root.path().join("a.txt"), "a\n").unwrap();
    let (r, d) = (root.path().to_path_buf(), data.path().to_path_buf());
    let (wb, cx) = cx.add_window_view(|window, cx| Workbench::with_data_dir(r, d, window, cx));
    settle(cx);
    let docs = documents(&wb, cx);
    let key = docs.update(cx, |d, cx| d.new_untitled(cx));
    settle(cx);

    let a: Arc<Path> = Arc::from(root.path().join("a.txt").as_path());
    wb.update(cx, |wb, cx| wb.open_path(a.clone(), OpenMode::Pinned, cx));
    open_tab_and_type(&wb, &key, "hello", cx);
    wb.update(cx, |wb, cx| wb.save_workspace_state(cx));

    let saved = load_state(&state_path(data.path(), root.path())).expect("state saved");
    let tabs: Vec<_> = saved
        .groups
        .groups
        .iter()
        .flat_map(|g| g.tabs.iter().map(|(p, _)| p.clone()))
        .collect();
    assert_eq!(tabs, vec![a.to_path_buf()]);
    assert_eq!(
        cx.update(|_, cx| wb.read(cx).recent_files().to_vec()),
        vec![a]
    );
}

/// `(active tab's path, pinned, displayed title, path bar text)`.
fn active_tab(
    wb: &Entity<Workbench>,
    cx: &mut VisualTestContext,
) -> (String, bool, String, Option<String>) {
    cx.update(|_, cx| {
        let groups = wb.read(cx).editor_groups().read(cx);
        let model = groups.model();
        let group = model.active_group();
        let tab = model.active_tab().expect("an active tab");
        let ti = model.groups()[group]
            .tabs
            .iter()
            .position(|t| t.id == tab.id)
            .unwrap();
        (
            tab.path().unwrap().display().to_string(),
            tab.pinned,
            groups.tab_title(group, ti, cx).unwrap(),
            groups.path_bar_text(group),
        )
    })
}

#[gpui::test]
fn cmd_n_opens_untitled_tab(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, wb, cx) = workbench(cx);
    cx.simulate_keystrokes("cmd-n");
    settle(cx);

    let (path, pinned, title, bar) = active_tab(&wb, cx);
    assert_eq!(path, "untitled:Untitled-1");
    assert!(pinned);
    assert_eq!(title, "Untitled-1");
    assert_eq!(bar.as_deref(), Some("Untitled-1"));
    let focused = cx.update(|window, cx| {
        let body = wb.read(cx).editor_groups().read(cx).active_body().unwrap();
        let view = body.downcast::<CodeView>().expect("a code view");
        view.read(cx).focus_handle(cx).is_focused(window)
    });
    assert!(focused);
}

#[gpui::test]
fn untitled_title_follows_typing(cx: &mut TestAppContext) {
    init(cx, None);
    let (dir, wb, cx) = workbench(cx);
    let docs = documents(&wb, cx);
    cx.simulate_keystrokes("cmd-n");
    settle(cx);
    let key: Arc<Path> = Path::new("untitled:Untitled-1").into();
    assert!(!docs.read_with(cx, |d, _| d.is_dirty(&key)));

    // Real keystrokes into the focused editor.
    cx.simulate_keystrokes("enter");
    cx.simulate_input("  hello world");
    cx.simulate_keystrokes("enter");
    cx.simulate_input("second");
    settle(cx);
    assert!(docs.read_with(cx, |d, _| d.is_dirty(&key)));
    assert_eq!(active_tab(&wb, cx).2, "hello world");

    cx.simulate_keystrokes("cmd-a backspace");
    settle(cx);
    assert!(!docs.read_with(cx, |d, _| d.is_dirty(&key)));
    assert_eq!(active_tab(&wb, cx).2, "Untitled-1");
    no_untitled_on_disk(&dir);
}

#[gpui::test]
fn untitled_numbers_are_reused(cx: &mut TestAppContext) {
    init(cx, None);
    let (_dir, wb, cx) = workbench(cx);
    cx.simulate_keystrokes("cmd-n");
    settle(cx);
    cx.simulate_keystrokes("cmd-n");
    settle(cx);
    assert_eq!(active_tab(&wb, cx).0, "untitled:Untitled-2");

    // Activate tab 1 and close it: it is empty, so nothing is asked.
    cx.update(|_, cx| {
        wb.update(cx, |wb, cx| {
            wb.editor_groups().update(cx, |g, cx| {
                g.update_model(cx, |m| {
                    let id = m.groups()[0].tabs[0].id;
                    m.activate(id);
                })
            })
        })
    });
    settle(cx);
    assert_eq!(active_tab(&wb, cx).0, "untitled:Untitled-1");
    cx.simulate_keystrokes("cmd-w");
    settle(cx);
    assert!(!cx.has_pending_prompt());

    cx.simulate_keystrokes("cmd-n");
    settle(cx);
    assert_eq!(active_tab(&wb, cx).0, "untitled:Untitled-1");
}
