//! The Explorer view inside the workbench: mouse handling, worktree event
//! fan-out and auto-reveal, driven through GPUI's test platform.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::px;
use gpui::{
    Entity, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Pixels, Point, TestAppContext,
    VisualTestContext,
};
use lucerna_project::{Change, ChangeKind, WorktreeEvent};
use lucerna_settings::{Keymap, Layer, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::{AppState, OpenMode, Workbench, ui};
use tempfile::TempDir;

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

/// A project with `src/main.rs`, `a.rs` and `b.rs`.
fn project() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    for file in ["src/main.rs", "a.rs", "b.rs"] {
        std::fs::write(dir.path().join(file), "fn main() {}\n").unwrap();
    }
    dir
}

/// Opens the workbench on `root` once its scan finished, then delivers
/// `ScanComplete` by hand so the test does not depend on event timing.
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

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

fn p(path: &str) -> Arc<Path> {
    Arc::from(Path::new(path))
}

fn center(selector: &'static str, cx: &mut VisualTestContext) -> Point<Pixels> {
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is not drawn"))
        .center()
}

fn click(selector: &'static str, cx: &mut VisualTestContext) {
    let position = center(selector, cx);
    cx.simulate_click(position, Modifiers::none());
    draw(cx);
}

/// The second click of a double click.
fn double_click(selector: &'static str, cx: &mut VisualTestContext) {
    let position = center(selector, cx);
    click(selector, cx);
    cx.simulate_event(MouseDownEvent {
        position,
        modifiers: Modifiers::none(),
        button: MouseButton::Left,
        click_count: 2,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position,
        modifiers: Modifiers::none(),
        button: MouseButton::Left,
        click_count: 2,
    });
    draw(cx);
}

/// `(path, pinned)` of each tab in the active group.
fn tabs(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Vec<(PathBuf, bool)> {
    cx.update(|_, cx| {
        // Tabs hold absolute paths; compare them relative to the root.
        let root = wb.read(cx).root().to_path_buf();
        let model = wb.read(cx).editor_groups().read(cx).model();
        model.groups()[model.active_group()]
            .tabs
            .iter()
            .map(|t| {
                (
                    t.path().unwrap().strip_prefix(&root).unwrap().to_path_buf(),
                    t.pinned,
                )
            })
            .collect()
    })
}

fn visible(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Vec<String> {
    cx.update(|_, cx| {
        wb.read(cx)
            .explorer()
            .read(cx)
            .model()
            .visible_rows()
            .iter()
            .map(|r| r.path.display().to_string())
            .collect()
    })
}

fn recently_changed(wb: &Entity<Workbench>, path: &str, cx: &mut VisualTestContext) -> bool {
    cx.update(|_, cx| {
        wb.read(cx)
            .explorer()
            .read(cx)
            .model()
            .is_recently_changed(Path::new(path), Instant::now())
    })
}

#[gpui::test]
fn click_opens_preview_double_click_pins(cx: &mut TestAppContext) {
    init(cx, None);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    assert_eq!(visible(&wb, cx), ["src", "a.rs", "b.rs"]);

    click("explorer-row-a.rs", cx);
    assert_eq!(tabs(&wb, cx), [(PathBuf::from("a.rs"), false)]);

    // A second preview replaces the first.
    click("explorer-row-b.rs", cx);
    assert_eq!(tabs(&wb, cx), [(PathBuf::from("b.rs"), false)]);

    double_click("explorer-row-b.rs", cx);
    assert_eq!(tabs(&wb, cx), [(PathBuf::from("b.rs"), true)]);

    // Clicking a directory toggles it and opens nothing.
    click("explorer-row-src", cx);
    assert_eq!(visible(&wb, cx), ["src", "src/main.rs", "a.rs", "b.rs"]);
    assert_eq!(tabs(&wb, cx).len(), 1);
}

#[gpui::test]
fn changed_event_decorates_until_opened(cx: &mut TestAppContext) {
    init(cx, None);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    let changes = WorktreeEvent::Changed(vec![
        Change {
            path: p("a.rs"),
            kind: ChangeKind::Modified,
        },
        Change {
            path: p("b.rs"),
            kind: ChangeKind::Removed,
        },
    ]);
    wb.update(cx, |wb, cx| wb.on_worktree_event(&changes, cx));
    assert!(recently_changed(&wb, "a.rs", cx));
    assert!(!recently_changed(&wb, "b.rs", cx));

    click("explorer-row-a.rs", cx);
    assert!(!recently_changed(&wb, "a.rs", cx));
}

#[gpui::test]
fn changed_decoration_respects_setting(cx: &mut TestAppContext) {
    init(
        cx,
        Some(r#"{ "explorer.decorations.externalChanges": false }"#),
    );
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    let changes = WorktreeEvent::Changed(vec![Change {
        path: p("a.rs"),
        kind: ChangeKind::Added,
    }]);
    wb.update(cx, |wb, cx| wb.on_worktree_event(&changes, cx));
    assert!(!recently_changed(&wb, "a.rs", cx));
}

#[gpui::test]
fn active_tab_is_revealed(cx: &mut TestAppContext) {
    init(cx, None);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    wb.update(cx, |wb, cx| {
        wb.open_path(p("src/main.rs"), OpenMode::Pinned, cx)
    });
    draw(cx);
    assert_eq!(visible(&wb, cx), ["src", "src/main.rs", "a.rs", "b.rs"]);
    assert!(cx.debug_bounds("explorer-row-src/main.rs").is_some());
}

#[gpui::test]
fn auto_reveal_off_leaves_tree_alone(cx: &mut TestAppContext) {
    init(cx, Some(r#"{ "explorer.autoReveal": false }"#));
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    wb.update(cx, |wb, cx| {
        wb.open_path(p("src/main.rs"), OpenMode::Pinned, cx)
    });
    draw(cx);
    assert_eq!(visible(&wb, cx), ["src", "a.rs", "b.rs"]);
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

#[gpui::test]
fn root_row_replaces_uppercase_header(cx: &mut TestAppContext) {
    init(cx, None);
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("lucerna");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("a.rs"), "").unwrap();
    let (wb, cx) = open(&root, cx);
    // The old uppercase header row is gone.
    assert!(cx.debug_bounds("sidebar-header").is_none());
    let row = cx.debug_bounds("explorer-root").expect("root row is drawn");
    assert_eq!(row.size.height, px(ui::TREE_ROW_HEIGHT));
    assert!(cx.debug_bounds("explorer-root-icon").is_some());
    let label = cx.update(|_, cx| wb.read(cx).explorer().read(cx).root_label().to_string());
    assert_eq!(label, "lucerna");
    // The root row sits above the first entry.
    let first = cx.debug_bounds("explorer-row-a.rs").unwrap();
    assert_eq!(first.top(), row.bottom());
    // Clicking it toggles nothing.
    click("explorer-root", cx);
    assert_eq!(visible(&wb, cx), ["a.rs"]);
}

#[gpui::test]
fn indent_guides_match_depth(cx: &mut TestAppContext) {
    init(cx, None);
    let dir = project();
    std::fs::create_dir(dir.path().join("src/nested")).unwrap();
    std::fs::write(dir.path().join("src/nested/deep.rs"), "").unwrap();
    let (wb, cx) = open(dir.path(), cx);
    click("explorer-row-src", cx);
    click("explorer-row-src/nested", cx);
    let rows: Vec<(String, u16)> = cx.update(|_, cx| {
        wb.read(cx)
            .explorer()
            .read(cx)
            .model()
            .visible_rows()
            .iter()
            .map(|r| (r.path.display().to_string(), r.depth))
            .collect()
    });
    assert!(rows.iter().any(|(_, d)| *d == 2), "{rows:?}");
    for (path, depth) in rows {
        let count = (0..8)
            .take_while(|k| {
                cx.debug_bounds(leak(format!("explorer-guide-{path}-{k}")))
                    .is_some()
            })
            .count();
        assert_eq!(count, usize::from(depth), "{path}");
        let row = cx
            .debug_bounds(leak(format!("explorer-row-{path}")))
            .unwrap();
        for k in 0..usize::from(depth) {
            let guide = cx
                .debug_bounds(leak(format!("explorer-guide-{path}-{k}")))
                .unwrap();
            assert_eq!(guide.size.width, px(1.));
            assert_eq!(guide.top(), row.top());
            assert_eq!(guide.size.height, row.size.height);
        }
    }
}

#[gpui::test]
fn scrollbar_thumb_only_when_list_overflows(cx: &mut TestAppContext) {
    init(cx, None);
    let small = project();
    {
        let (_, cx) = open(small.path(), cx);
        assert!(cx.debug_bounds("explorer-list").is_some());
        assert!(cx.debug_bounds("explorer-scrollbar-thumb").is_none());
    }

    let big = tempfile::tempdir().unwrap();
    for i in 0..300 {
        std::fs::write(big.path().join(format!("f{i:03}.rs")), "").unwrap();
    }
    let (_, cx) = open(big.path(), cx);
    let thumb = cx
        .debug_bounds("explorer-scrollbar-thumb")
        .expect("thumb is drawn when the list overflows");
    let list = cx.debug_bounds("explorer-list").unwrap();
    assert_eq!(thumb.size.width, px(ui::SCROLLBAR_WIDTH));
    assert_eq!(thumb.right(), list.right());
    assert_eq!(thumb.top(), list.top());
    assert!(thumb.size.height < list.size.height);
}

fn ext_icon_theme(dir: &Path) -> (lucerna_theme::IconThemeRegistry, PathBuf) {
    let root = dir.canonicalize().unwrap();
    std::fs::create_dir_all(root.join("icons")).unwrap();
    std::fs::write(root.join("icons/json.svg"), "<svg/>").unwrap();
    std::fs::write(root.join("icons/dir.svg"), "<svg/>").unwrap();
    std::fs::write(
        root.join("icons.json"),
        r#"{"name":"Ext","author":"t","themes":[{"name":"Ext Icons","appearance":"dark",
        "directory_icons":{"collapsed":"./icons/dir.svg","expanded":"./icons/dir.svg"},
        "file_suffixes":{"json":"json"},"file_icons":{"json":{"path":"./icons/json.svg"}}}]}"#,
    )
    .unwrap();
    let (registry, warnings) = lucerna_theme::IconThemeRegistry::build(&[(
        "ext".to_string(),
        root.join("icons.json"),
        root.clone(),
    )]);
    assert!(warnings.is_empty(), "{warnings:?}");
    (registry, root)
}

fn row_icon(
    wb: &Entity<Workbench>,
    path: &str,
    cx: &mut VisualTestContext,
) -> lucerna_theme::IconSource {
    cx.update(|_, cx| {
        wb.read(cx)
            .explorer()
            .read(cx)
            .row_icon(Path::new(path), cx)
            .unwrap_or_else(|| panic!("no row {path}"))
    })
}

#[gpui::test]
fn explorer_uses_extension_icon(cx: &mut TestAppContext) {
    init(cx, None);
    let themes = tempfile::tempdir().unwrap();
    let (registry, root) = ext_icon_theme(themes.path());
    cx.update(|cx| {
        let state = cx.global_mut::<AppState>();
        state.icons = registry.resolve("Ext Icons");
        state.icon_themes = registry;
    });
    let dir = project();
    std::fs::write(dir.path().join("a.json"), "{}").unwrap();
    let (wb, cx) = open(dir.path(), cx);
    assert_eq!(
        row_icon(&wb, "a.json", cx),
        lucerna_theme::IconSource::File(root.join("icons/json.svg"))
    );
    assert_eq!(
        row_icon(&wb, "src", cx),
        lucerna_theme::IconSource::File(root.join("icons/dir.svg"))
    );
    // No mapping for .rs and none in Lucide's way: the theme falls back.
    assert!(matches!(
        row_icon(&wb, "a.rs", cx),
        lucerna_theme::IconSource::Builtin(_)
    ));
    // The extension icon is drawn with an `img`; it stays in its slot.
    assert!(cx.debug_bounds("explorer-row-a.json").is_some());
}

#[gpui::test]
fn explorer_falls_back_to_lucide(cx: &mut TestAppContext) {
    use lucerna_theme::IconSource::Builtin;
    init(cx, None);
    let dir = project();
    let (wb, cx) = open(dir.path(), cx);
    assert_eq!(row_icon(&wb, "a.rs", cx), Builtin("file-code"));
    assert_eq!(row_icon(&wb, "src", cx), Builtin("folder"));
    click("explorer-row-src", cx);
    assert_eq!(row_icon(&wb, "src", cx), Builtin("folder-open"));
}
