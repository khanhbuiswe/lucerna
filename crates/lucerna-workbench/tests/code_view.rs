//! The code view driven through GPUI's test platform: selection and copy,
//! go-to, large-file mode, zoom and its commands.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{Entity, TestAppContext, VisualTestContext};
use lucerna_settings::{Keymap, Layer, SettingsStore};
use lucerna_syntax::{LanguageRegistry, SyntaxState};
use lucerna_text::{Buffer, BufferSnapshot, Point};
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{AppState, OpenMode, Workbench};

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
        languages: Arc::new(LanguageRegistry::bundled()),
    });
}

fn snapshot(text: &str) -> BufferSnapshot {
    Buffer::from_bytes(text.as_bytes(), false)
        .unwrap()
        .snapshot()
}

fn p(name: &str) -> Arc<Path> {
    Arc::from(Path::new(name))
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

/// A code view alone in a window, showing `text`.
fn code_view<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeView>, &'a mut VisualTestContext) {
    let snapshot = snapshot(text);
    let (view, cx) = cx.add_window_view(|_, cx| CodeView::new(p("src/main.rs"), cx));
    view.update(cx, |v, cx| v.set_buffer(snapshot, cx));
    draw(cx);
    (view, cx)
}

/// A workbench whose tabs get code views; the caller loads them.
fn workbench(cx: &mut TestAppContext) -> (Entity<Workbench>, &mut VisualTestContext) {
    let (wb, cx) =
        cx.add_window_view(|window, cx| Workbench::new(PathBuf::from("/tmp/project"), window, cx));
    wb.update(cx, |wb, cx| {
        wb.editor_groups().update(cx, |groups, cx| {
            groups.set_tab_body_factory(Rc::new(CodeView::tab_body), cx)
        })
    });
    (wb, cx)
}

/// Opens `path` in the workbench and loads `text` into its code view.
fn open(
    wb: &Entity<Workbench>,
    path: &str,
    text: &str,
    cx: &mut VisualTestContext,
) -> Entity<CodeView> {
    wb.update(cx, |wb, cx| wb.open_path(p(path), OpenMode::Pinned, cx));
    // Tab bodies are made while drawing.
    draw(cx);
    let view = active_code_view(wb, cx).expect("the active tab shows a code view");
    let snapshot = snapshot(text);
    view.update(cx, |v, cx| v.set_buffer(snapshot, cx));
    draw(cx);
    view
}

fn active_code_view(
    wb: &Entity<Workbench>,
    cx: &mut VisualTestContext,
) -> Option<Entity<CodeView>> {
    cx.update(|_, cx| {
        wb.read(cx)
            .editor_groups()
            .read(cx)
            .active_body()?
            .downcast::<CodeView>()
            .ok()
    })
}

fn run(wb: &Entity<Workbench>, command: &str, cx: &mut VisualTestContext) {
    let ran =
        cx.update(|window, cx| wb.update(cx, |wb, cx| wb.run_command(command, None, window, cx)));
    assert!(ran, "{command} is registered");
}

#[gpui::test]
fn copy_selection_returns_text(cx: &mut TestAppContext) {
    init(cx, None);
    let (view, cx) = code_view("héllo world\nsecond line\n", cx);
    assert_eq!(view.read_with(cx, |v, _| v.copy_selection()), None);

    let range = Point { row: 0, column: 1 }..Point { row: 1, column: 6 };
    view.update(cx, |v, cx| v.select(range.clone(), cx));
    assert_eq!(view.read_with(cx, |v, _| v.selection()), Some(range));
    assert_eq!(
        view.read_with(cx, |v, _| v.copy_selection()),
        Some("éllo world\nsecond".to_string())
    );

    // A column inside `é` is clamped to the char boundary before it, and
    // a reversed selection copies the same text.
    let reversed = Point { row: 1, column: 3 }..Point { row: 0, column: 2 };
    view.update(cx, |v, cx| v.select(reversed, cx));
    assert_eq!(
        view.read_with(cx, |v, _| v.copy_selection()),
        Some("éllo world\nsec".to_string())
    );
}

#[gpui::test]
fn copy_command_writes_clipboard(cx: &mut TestAppContext) {
    init(cx, None);
    let (wb, cx) = workbench(cx);
    let view = open(&wb, "a.txt", "alpha\nbeta\n", cx);
    view.update(cx, |v, cx| {
        v.select(Point { row: 0, column: 2 }..Point { row: 1, column: 2 }, cx)
    });
    run(&wb, "editor.action.clipboardCopyAction", cx);
    let copied = cx.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(copied.as_deref(), Some("pha\nbe"));
}

#[gpui::test]
fn go_to_centers_row(cx: &mut TestAppContext) {
    init(cx, None);
    let text: String = (0..1000).map(|i| format!("line {i}\n")).collect();
    let (view, cx) = code_view(&text, cx);
    let visible = view.read_with(cx, |v, _| v.visible_rows());
    assert!(visible > 4, "the window shows several rows, got {visible}");

    let target = Point {
        row: 500,
        column: 3,
    };
    view.update(cx, |v, cx| v.go_to(target, cx));
    draw(cx);
    assert_eq!(
        view.read_with(cx, |v, _| v.scroll_top_row()),
        500 - visible / 2
    );
    assert_eq!(view.read_with(cx, |v, _| v.cursor()), target);

    // Near the start it cannot center, and stays at the top.
    view.update(cx, |v, cx| v.go_to(Point { row: 1, column: 0 }, cx));
    assert_eq!(view.read_with(cx, |v, _| v.scroll_top_row()), 0);

    // Scrolling is clamped to the last row.
    view.update(cx, |v, cx| v.set_scroll_top_row(5000, cx));
    assert_eq!(view.read_with(cx, |v, _| v.scroll_top_row()), 999);
}

#[gpui::test]
fn large_file_disables_highlight(cx: &mut TestAppContext) {
    init(cx, Some(r#"{ "lucerna.largeFile.maxLines": 10 }"#));
    let (wb, cx) = workbench(cx);
    let source: String = (0..20)
        .map(|i| format!("fn f{i}() {{\n    {i}\n}}\n"))
        .collect();
    let rust = cx.update(|_, cx| {
        cx.global::<AppState>()
            .languages
            .language_for_path(Path::new("big.rs"))
            .unwrap()
    });

    let big = open(&wb, "big.rs", &source, cx);
    let mut syntax = SyntaxState::new(rust.clone());
    syntax.parse(&snapshot(&source), None);
    big.update(cx, |v, cx| v.set_syntax(Some(Arc::new(syntax)), cx));
    cx.run_until_parked();
    draw(cx);
    big.read_with(cx, |v, _| {
        assert!(v.is_large());
        assert_eq!(v.cached_highlight_rows(), None);
        assert!(v.fold_ranges().is_empty());
    });
    assert!(cx.debug_bounds("code-view-minimap").is_none());
    let status = cx
        .debug_bounds("large-file-status")
        .expect("the status bar shows the large-file notice");
    assert!(status.size.width > gpui::px(0.));

    // A small file gets highlights, folds and a minimap, and no notice.
    let small_source = "fn main() {\n    let x = 1;\n}\n";
    let small = open(&wb, "small.rs", small_source, cx);
    let mut syntax = SyntaxState::new(rust);
    syntax.parse(&snapshot(small_source), None);
    small.update(cx, |v, cx| v.set_syntax(Some(Arc::new(syntax)), cx));
    cx.run_until_parked();
    draw(cx);
    small.read_with(cx, |v, _| {
        assert!(!v.is_large());
        assert!(v.cached_highlight_rows().is_some());
        assert_eq!(v.fold_ranges().len(), 1);
        assert_eq!(v.fold_ranges()[0], 0..2);
    });
    assert!(cx.debug_bounds("code-view-minimap").is_some());
    assert!(cx.debug_bounds("large-file-status").is_none());
}

#[gpui::test]
fn zoom_clamps(cx: &mut TestAppContext) {
    init(cx, None);
    let (view, cx) = code_view("text\n", cx);
    assert_eq!(view.read_with(cx, |v, _| v.font_size()), 13.);
    view.update(cx, |v, cx| v.zoom(100., cx));
    assert_eq!(view.read_with(cx, |v, _| v.font_size()), 40.);
    view.update(cx, |v, cx| v.zoom(-100., cx));
    assert_eq!(view.read_with(cx, |v, _| v.font_size()), 6.);
    view.update(cx, |v, cx| v.zoom(1., cx));
    assert_eq!(view.read_with(cx, |v, _| v.font_size()), 7.);
}

#[gpui::test]
fn zoom_and_wrap_commands(cx: &mut TestAppContext) {
    init(cx, Some(r#"{ "editor.fontSize": 39 }"#));
    let (wb, cx) = workbench(cx);
    let view = open(&wb, "a.txt", "text\n", cx);
    run(&wb, "editor.action.fontZoomIn", cx);
    run(&wb, "editor.action.fontZoomIn", cx);
    assert_eq!(view.read_with(cx, |v, _| v.font_size()), 40.);
    run(&wb, "editor.action.fontZoomOut", cx);
    assert_eq!(view.read_with(cx, |v, _| v.font_size()), 39.);

    assert!(!view.read_with(cx, |v, _| v.word_wrap()));
    cx.simulate_keystrokes("alt-z");
    assert!(view.read_with(cx, |v, _| v.word_wrap()));
    run(&wb, "editor.action.toggleWordWrap", cx);
    assert!(!view.read_with(cx, |v, _| v.word_wrap()));
}

// Geometry on the test platform's text system: 13 px font, 1.5 line
// height, every char 7.8 px wide. With under 100 lines the gutter is
// six chars wide and the text starts half a char after it.
const LINE: f32 = 19.5;
const CHAR: f32 = 7.8;
const TEXT_X: f32 = 6.5 * CHAR;

/// The middle of `row`, slightly right of the boundary before `column`.
fn at(row: u32, column: u32) -> gpui::Point<gpui::Pixels> {
    gpui::point(
        gpui::px(TEXT_X + column as f32 * CHAR + 1.),
        gpui::px((row as f32 + 0.5) * LINE),
    )
}

fn pt(row: u32, column: u32) -> Point {
    Point { row, column }
}

fn mouse_down(
    position: gpui::Point<gpui::Pixels>,
    clicks: usize,
    shift: bool,
    cx: &mut VisualTestContext,
) {
    let modifiers = if shift {
        gpui::Modifiers::shift()
    } else {
        gpui::Modifiers::none()
    };
    cx.simulate_event(gpui::MouseDownEvent {
        button: gpui::MouseButton::Left,
        position,
        modifiers,
        click_count: clicks,
        first_mouse: false,
    });
    cx.simulate_mouse_up(position, gpui::MouseButton::Left, modifiers);
}

#[gpui::test]
fn mouse_selects_by_drag_word_line_and_shift_click(cx: &mut TestAppContext) {
    init(cx, None);
    let (view, cx) = code_view("alpha beta\nsecond line\nthird\n", cx);
    let selection = |cx: &mut VisualTestContext| view.read_with(cx, |v, _| v.selection());

    let left = gpui::MouseButton::Left;
    let none = gpui::Modifiers::none();
    cx.simulate_mouse_down(at(0, 2), left, none);
    cx.simulate_mouse_move(at(1, 4), left, none);
    cx.simulate_mouse_up(at(1, 4), left, none);
    assert_eq!(selection(cx), Some(pt(0, 2)..pt(1, 4)));
    assert_eq!(
        view.read_with(cx, |v, _| v.copy_selection()).as_deref(),
        Some("pha beta\nseco")
    );

    mouse_down(at(0, 7), 2, false, cx);
    assert_eq!(selection(cx), Some(pt(0, 6)..pt(0, 10)));

    mouse_down(at(1, 1), 3, false, cx);
    assert_eq!(selection(cx), Some(pt(1, 0)..pt(2, 0)));

    mouse_down(at(0, 2), 1, false, cx);
    assert_eq!(selection(cx), None);
    mouse_down(at(2, 3), 1, true, cx);
    assert_eq!(selection(cx), Some(pt(0, 2)..pt(2, 3)));

    // Past the end of a line the cursor goes to the line end.
    mouse_down(at(2, 40), 1, false, cx);
    assert_eq!(view.read_with(cx, |v, _| v.cursor()), pt(2, 5));
}

#[gpui::test]
fn gutter_chevron_folds(cx: &mut TestAppContext) {
    init(cx, None);
    let (view, cx) = code_view("fn a() {\n    x\n}\nlast\n", cx);
    cx.run_until_parked();
    draw(cx);
    let folds = view.read_with(cx, |v, _| v.fold_ranges().to_vec());
    assert_eq!(folds.len(), 1);
    assert_eq!(folds[0], 0..2);

    let chevron = gpui::point(gpui::px(TEXT_X - CHAR), gpui::px(LINE * 0.5));
    mouse_down(chevron, 1, false, cx);
    draw(cx);
    // Row 1 is hidden, so the second line on screen is buffer row 2.
    mouse_down(at(1, 0), 1, false, cx);
    assert_eq!(view.read_with(cx, |v, _| v.cursor()), pt(2, 0));

    mouse_down(chevron, 1, false, cx);
    draw(cx);
    mouse_down(at(1, 0), 1, false, cx);
    assert_eq!(view.read_with(cx, |v, _| v.cursor()), pt(1, 0));
}

#[gpui::test]
fn wheel_scrolls_by_pixels(cx: &mut TestAppContext) {
    init(cx, None);
    let text: String = (0..90).map(|i| format!("line {i}\n")).collect();
    let (view, cx) = code_view(&text, cx);
    let wheel = |delta: gpui::ScrollDelta, cx: &mut VisualTestContext| {
        cx.simulate_event(gpui::ScrollEvent {
            position: at(2, 2),
            delta,
            ..Default::default()
        });
        draw(cx);
    };
    wheel(
        gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(-LINE * 10.5))),
        cx,
    );
    assert_eq!(view.read_with(cx, |v, _| v.scroll_top_row()), 10);
    wheel(gpui::ScrollDelta::Lines(gpui::point(0., 3.)), cx);
    assert_eq!(view.read_with(cx, |v, _| v.scroll_top_row()), 7);
    wheel(gpui::ScrollDelta::Lines(gpui::point(0., 100.)), cx);
    assert_eq!(view.read_with(cx, |v, _| v.scroll_top_row()), 0);
    wheel(gpui::ScrollDelta::Lines(gpui::point(0., -1000.)), cx);
    assert_eq!(view.read_with(cx, |v, _| v.scroll_top_row()), 89);
}

#[gpui::test]
fn reload_keeps_scroll_and_selection(cx: &mut TestAppContext) {
    init(cx, None);
    let old: String = (0..100).map(|i| format!("line {i}\n")).collect();
    let mut buffer = Buffer::from_bytes(old.as_bytes(), false).unwrap();
    let (view, cx) = code_view(&old, cx);
    view.update(cx, |v, cx| {
        v.set_scroll_top_row(50, cx);
        v.select(pt(60, 1)..pt(61, 2), cx);
    });

    let new = format!("{}{old}", "added\n".repeat(5));
    let reload = buffer.reload(new.as_bytes()).unwrap();
    let snapshot = buffer.snapshot();
    view.update(cx, |v, cx| v.apply_reload(snapshot, reload, cx));
    draw(cx);
    assert_eq!(view.read_with(cx, |v, _| v.scroll_top_row()), 55);
    assert_eq!(
        view.read_with(cx, |v, _| v.selection()),
        Some(pt(65, 1)..pt(66, 2))
    );
    assert_eq!(
        view.read_with(cx, |v, _| v.copy_selection()).as_deref(),
        Some("ine 60\nli")
    );
}
