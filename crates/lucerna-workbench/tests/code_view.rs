//! The code view driven through GPUI's test platform: selection and copy,
//! go-to, large-file mode, zoom and its commands.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use lucerna_editor::{Goal, Selection, SelectionSet};
use lucerna_settings::{Keymap, Layer, SettingsStore};
use lucerna_syntax::{LanguageRegistry, SyntaxState};
use lucerna_text::{Buffer, BufferSnapshot, Point, Reload, decode_file, diff_edits};
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{AppState, OpenMode, WindowSettings, Workbench};

/// Reloads `buffer` with `bytes` the way `Documents` does (a line diff
/// applied as an edit) and shows the result in `view`; returns the changed
/// rows.
fn reload_into(
    view: &Entity<CodeView>,
    buffer: &mut Buffer,
    bytes: &[u8],
    cx: &mut VisualTestContext,
) -> Reload {
    let old = buffer.snapshot();
    let decoded = decode_file(bytes, false).unwrap();
    let patch = buffer.edit(diff_edits(&old, &decoded.text));
    let snapshot = buffer.snapshot();
    view.update(cx, |v, cx| v.apply_edit(&old, snapshot.clone(), &patch, cx));
    patch.reload(&old, &snapshot)
}

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

/// The settings of a window without a folder: the app's alone.
fn no_folder(cx: &mut gpui::App) -> Entity<WindowSettings> {
    cx.new(|cx| WindowSettings::new(None, cx))
}

/// A code view alone in a window, showing `text`.
fn code_view<'a>(
    text: &str,
    cx: &'a mut TestAppContext,
) -> (Entity<CodeView>, &'a mut VisualTestContext) {
    let snapshot = snapshot(text);
    let (view, cx) = cx.add_window_view(|_, cx| CodeView::new(p("src/main.rs"), no_folder(cx), cx));
    view.update(cx, |v, cx| v.set_buffer(snapshot, cx));
    draw(cx);
    (view, cx)
}

/// A workbench whose tabs get code views; the caller loads them.
fn workbench(cx: &mut TestAppContext) -> (Entity<Workbench>, &mut VisualTestContext) {
    let (wb, cx) =
        cx.add_window_view(|window, cx| Workbench::new(PathBuf::from("/tmp/project"), window, cx));
    wb.update(cx, |wb, cx| {
        let body = CodeView::tab_body(wb.settings().clone());
        wb.editor_groups()
            .update(cx, |groups, cx| groups.set_tab_body_factory(body, cx))
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
    init(
        cx,
        Some(r#"{ "lucerna.largeFile.maxLines": 10, "editor.minimap.enabled": true }"#),
    );
    let (wb, cx) = workbench(cx);
    let source: String = (0..20)
        .map(|i| format!("fn f{i}() {{\n    {i}\n}}\n"))
        .collect();
    let (rust, registry) = cx.update(|_, cx| {
        let languages = cx.global::<AppState>().languages.clone();
        (
            languages.language_for_path(Path::new("big.rs")).unwrap(),
            languages,
        )
    });

    let big = open(&wb, "big.rs", &source, cx);
    let mut syntax = SyntaxState::new(rust.clone(), registry.clone()).unwrap();
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
    let mut syntax = SyntaxState::new(rust, registry).unwrap();
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
fn minimap_is_off_by_default(cx: &mut TestAppContext) {
    init(cx, None);
    let (_view, cx) = code_view("fn main() {}\n", cx);
    assert!(cx.debug_bounds("code-view-minimap").is_none());
}

#[gpui::test]
fn zoom_resets_when_user_size_changes(cx: &mut TestAppContext) {
    use lucerna_workbench::font_zoom;
    init(cx, Some(r#"{ "editor.fontSize": 14 }"#));
    cx.update(|cx| {
        assert_eq!(font_zoom::step(14., 1., cx), Some(15.));
        assert_eq!(font_zoom::zoomed(14., cx), 15.);
        cx.global_mut::<AppState>()
            .settings
            .set_layer(Layer::User, r#"{ "editor.fontSize": 20 }"#)
            .unwrap();
        assert_eq!(font_zoom::zoomed(20., cx), 20.);
        assert_eq!(font_zoom::reset(20., cx), None);
    });
}

#[gpui::test]
fn zoom_applies_to_every_view(cx: &mut TestAppContext) {
    init(cx, None);
    let (wb, cx) = workbench(cx);
    let a = open(&wb, "a.txt", "text\n", cx);
    run(&wb, "editor.action.fontZoomIn", cx);
    run(&wb, "editor.action.fontZoomIn", cx);
    draw(cx);
    assert_eq!(a.read_with(cx, |v, _| v.font_size()), 16.);
    let b = open(&wb, "b.txt", "text\n", cx);
    assert_eq!(b.read_with(cx, |v, _| v.font_size()), 16.);
    run(&wb, "editor.action.fontZoomReset", cx);
    draw(cx);
    assert_eq!(a.read_with(cx, |v, _| v.font_size()), 14.);
    assert_eq!(b.read_with(cx, |v, _| v.font_size()), 14.);
}

#[gpui::test]
fn zoom_keys_and_limits(cx: &mut TestAppContext) {
    init(cx, Some(r#"{ "editor.fontSize": 99 }"#));
    let (wb, cx) = workbench(cx);
    let view = open(&wb, "a.txt", "text\n", cx);
    cx.simulate_keystrokes("cmd-= cmd-shift-=");
    draw(cx);
    assert_eq!(view.read_with(cx, |v, _| v.font_size()), 100.);
    cx.simulate_keystrokes("cmd--");
    draw(cx);
    assert_eq!(view.read_with(cx, |v, _| v.font_size()), 99.);
    cx.simulate_keystrokes("cmd-= cmd-0");
    draw(cx);
    assert_eq!(view.read_with(cx, |v, _| v.font_size()), 99.);
}

#[gpui::test]
fn zoom_shows_font_size(cx: &mut TestAppContext) {
    init(cx, None);
    let (wb, cx) = workbench(cx);
    open(&wb, "a.txt", "text\n", cx);
    run(&wb, "editor.action.fontZoomIn", cx);
    let shown = cx.update(|window, cx| {
        lucerna_workbench::status_message::current(window.window_handle().window_id(), cx)
            .map(|m| m.to_string())
    });
    assert_eq!(shown.as_deref(), Some("Font Size: 15"));
}

#[gpui::test]
fn zoom_keeps_top_row(cx: &mut TestAppContext) {
    init(cx, None);
    let (wb, cx) = workbench(cx);
    let text = "line\n".repeat(500);
    let view = open(&wb, "a.txt", &text, cx);
    view.update(cx, |v, cx| v.set_scroll_top_row(200, cx));
    draw(cx);
    run(&wb, "editor.action.fontZoomIn", cx);
    draw(cx);
    assert_eq!(view.read_with(cx, |v, _| v.font_size()), 15.);
    assert_eq!(view.read_with(cx, |v, _| v.scroll_top_row()), 200);
}

#[gpui::test]
fn user_font_size_change_resets_zoom(cx: &mut TestAppContext) {
    init(cx, None);
    let (wb, cx) = workbench(cx);
    let view = open(&wb, "a.txt", "text\n", cx);
    run(&wb, "editor.action.fontZoomIn", cx);
    cx.update(|_, cx| {
        cx.global_mut::<AppState>()
            .settings
            .set_layer(Layer::User, r#"{ "editor.fontSize": 20 }"#)
            .unwrap();
        cx.refresh_windows();
    });
    let settings = cx.update(|_, cx| wb.read(cx).settings().clone());
    settings.update(cx, |_, cx| cx.notify());
    draw(cx);
    assert_eq!(view.read_with(cx, |v, _| v.font_size()), 20.);

    // Back to the old value: the old zoom stays gone.
    cx.update(|_, cx| {
        cx.global_mut::<AppState>()
            .settings
            .set_layer(Layer::User, "{}")
            .unwrap();
    });
    settings.update(cx, |_, cx| cx.notify());
    draw(cx);
    assert_eq!(view.read_with(cx, |v, _| v.font_size()), 14.);
}

#[gpui::test]
fn zoom_steps_from_the_language_size(cx: &mut TestAppContext) {
    init(
        cx,
        Some(r#"{ "editor.fontSize": 14, "[markdown]": { "editor.fontSize": 99 } }"#),
    );
    let (wb, cx) = workbench(cx);
    let view = open(&wb, "a.md", "# a\n", cx);
    assert_eq!(view.read_with(cx, |v, _| v.font_size()), 99.);
    run(&wb, "editor.action.fontZoomIn", cx);
    let shown = cx.update(|window, cx| {
        lucerna_workbench::status_message::current(window.window_handle().window_id(), cx)
            .map(|m| m.to_string())
    });
    assert_eq!(shown.as_deref(), Some("Font Size: 100"));
    run(&wb, "editor.action.fontZoomIn", cx);
    run(&wb, "editor.action.fontZoomOut", cx);
    draw(cx);
    assert_eq!(view.read_with(cx, |v, _| v.font_size()), 99.);
}

#[gpui::test]
fn wrap_command(cx: &mut TestAppContext) {
    init(cx, None);
    let (wb, cx) = workbench(cx);
    let view = open(&wb, "a.txt", "text\n", cx);
    assert!(!view.read_with(cx, |v, _| v.word_wrap()));
    cx.simulate_keystrokes("alt-z");
    assert!(view.read_with(cx, |v, _| v.word_wrap()));
    run(&wb, "editor.action.toggleWordWrap", cx);
    assert!(!view.read_with(cx, |v, _| v.word_wrap()));
}

// Geometry on the test platform's text system: 14 px font, 1.6 line
// height, every char 8.4 px wide. With under 100 lines the gutter is
// six chars wide and the text starts half a char after it.
const LINE: f32 = 22.4;
const CHAR: f32 = 8.4;
const TEXT_X: f32 = 6.5 * CHAR;

/// The middle of `row`, slightly right of the boundary before `column`.
fn at(row: u32, column: u32) -> gpui::Point<gpui::Pixels> {
    gpui::point(
        gpui::px(TEXT_X + column as f32 * CHAR + 1.),
        gpui::px((row as f32 + 0.5) * LINE),
    )
}

/// One selection per range of `v`'s text, the first one primary.
fn selections_of(v: &CodeView, ranges: &[std::ops::Range<Point>]) -> SelectionSet {
    let s = v.snapshot();
    let selections = ranges
        .iter()
        .zip(0u64..)
        .map(|(r, id)| Selection {
            id,
            anchor: s.point_to_offset(r.start),
            head: s.point_to_offset(r.end),
            goal: Goal::None,
        })
        .collect();
    SelectionSet::from_selections(selections, 0, true)
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

/// Fold chevrons show while the mouse is over the gutter, and always on a
/// folded row.
#[gpui::test]
fn fold_chevrons_follow_gutter_hover(cx: &mut TestAppContext) {
    init(cx, None);
    let (view, cx) = code_view("fn a() {\n    x\n}\nlast\n", cx);
    cx.run_until_parked();
    draw(cx);
    let state = |cx: &mut VisualTestContext| {
        view.read_with(cx, |v, _| {
            (
                v.gutter_hovered(),
                v.fold_chevron_visible(0),
                v.fold_chevron_visible(3),
            )
        })
    };
    let none = gpui::Modifiers::none();
    // Hover off: no chevron.
    assert_eq!(state(cx), (false, false, false));

    // Over the gutter: the fold header shows one; a plain row never does.
    cx.simulate_mouse_move(
        gpui::point(gpui::px(CHAR), gpui::px(LINE * 0.5)),
        None,
        none,
    );
    assert_eq!(state(cx), (true, true, false));

    // Over the text: hidden again.
    cx.simulate_mouse_move(at(0, 2), None, none);
    assert_eq!(state(cx), (false, false, false));

    // Leaving the window straight from the gutter hides them too.
    let gutter = gpui::point(gpui::px(CHAR), gpui::px(LINE * 0.5));
    cx.simulate_mouse_move(gutter, None, none);
    assert_eq!(state(cx), (true, true, false));
    cx.simulate_event(gpui::MouseExitEvent {
        position: gutter,
        pressed_button: None,
        modifiers: none,
    });
    assert_eq!(state(cx), (false, false, false));

    // A folded row keeps its chevron with the mouse away.
    view.update(cx, |v, cx| v.toggle_fold(0, cx));
    draw(cx);
    assert_eq!(state(cx), (false, true, false));
}

/// The overlay scrollbar thumb: only when the text overflows, at the top
/// of the track when scrolled to the top and at its bottom at the end.
#[gpui::test]
fn scrollbar_thumb_tracks_scroll(cx: &mut TestAppContext) {
    init(cx, None);
    let (short, cx) = code_view("one\ntwo\nthree\n", cx);
    assert_eq!(short.read_with(cx, |v, _| v.scrollbar_thumb()), None);

    let text: String = (0..500).map(|i| format!("line {i}\n")).collect();
    short.update(cx, |v, cx| v.set_buffer(snapshot(&text), cx));
    draw(cx);
    let viewport = cx.update(|window, _| window.viewport_size());
    let thumb = short
        .read_with(cx, |v, _| v.scrollbar_thumb())
        .expect("500 lines overflow");
    assert_eq!(thumb.top(), gpui::px(0.));
    assert!(thumb.size.height > gpui::px(0.) && thumb.size.height < viewport.height);
    assert_eq!(
        thumb.size.width,
        gpui::px(lucerna_workbench::ui::SCROLLBAR_WIDTH)
    );
    assert_eq!(thumb.right(), viewport.width);

    let height = thumb.size.height;
    let thumb_at = |row: u32, cx: &mut VisualTestContext| {
        short.update(cx, |v, cx| v.set_scroll_top_row(row, cx));
        draw(cx);
        short.read_with(cx, |v, _| v.scrollbar_thumb()).unwrap()
    };
    // The scroll range ends with the last row at the top: with the last
    // page on screen the thumb has not reached the bottom yet...
    let visible = short.read_with(cx, |v, _| v.visible_rows());
    let last_page = thumb_at(500 - visible, cx);
    assert!(last_page.bottom() < viewport.height - gpui::px(1.));
    // ...halfway it is halfway down the track...
    let half = thumb_at(499 / 2, cx);
    let expected = (viewport.height - height) * (249. / 499.);
    assert!((half.top() - expected).abs() < gpui::px(0.5));
    // ...and with the last row at the top it is at the bottom.
    let end = thumb_at(499, cx);
    assert!((end.bottom() - viewport.height).abs() < gpui::px(0.01));
}

/// With the minimap on, the thumb sits left of it.
#[gpui::test]
fn scrollbar_thumb_is_left_of_the_minimap(cx: &mut TestAppContext) {
    init(cx, Some(r#"{ "editor.minimap.enabled": true }"#));
    let text: String = (0..500).map(|i| format!("line {i}\n")).collect();
    let (view, cx) = code_view(&text, cx);
    draw(cx);
    let minimap = cx.debug_bounds("code-view-minimap").expect("minimap on");
    let thumb = view.read_with(cx, |v, _| v.scrollbar_thumb()).unwrap();
    assert!(thumb.right() <= minimap.left());
    assert!(thumb.right() > minimap.left() - gpui::px(1.));
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
    reload_into(&view, &mut buffer, new.as_bytes(), cx);
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

/// A reload keeps closed folds on the lines they were on; only a fold
/// whose header line changed is dropped.
#[gpui::test]
fn reload_remaps_folds(cx: &mut TestAppContext) {
    init(cx, None);
    let old = "a {\n  1\n}\nb {\n  2\n}\nc\n";
    let mut buffer = Buffer::from_bytes(old.as_bytes(), false).unwrap();
    let (view, cx) = code_view(old, cx);
    cx.run_until_parked();
    let headers: Vec<u32> =
        view.read_with(cx, |v, _| v.fold_ranges().iter().map(|r| r.start).collect());
    assert_eq!(headers, [0, 3]);
    view.update(cx, |v, cx| {
        v.toggle_fold(0, cx);
        v.toggle_fold(3, cx);
    });

    // The second header changes; the first fold is above the change.
    reload_into(&view, &mut buffer, b"a {\n  1\n}\nB {\n  2\n}\nc\n", cx);
    cx.run_until_parked();
    draw(cx);
    view.read_with(cx, |v, _| {
        assert!(v.is_folded(0), "the fold above the change survives");
        assert!(!v.is_folded(3), "the fold whose header changed is dropped");
    });

    // Lines inserted above shift a closed fold down with its header.
    reload_into(
        &view,
        &mut buffer,
        b"new\nnew\na {\n  1\n}\nB {\n  2\n}\nc\n",
        cx,
    );
    cx.run_until_parked();
    view.read_with(cx, |v, _| {
        assert!(v.is_folded(2), "the fold moved down two lines");
        assert!(!v.is_folded(0));
    });
}

/// `"[rust]"` overrides apply to Rust views only.
#[gpui::test]
fn language_overrides_apply_to_their_language(cx: &mut TestAppContext) {
    init(
        cx,
        Some(r#"{"[rust]": {"editor.tabSize": 2}, "[python]": {"editor.wordWrap": "on"}}"#),
    );
    let (rust, cx) = cx.add_window_view(|_, cx| CodeView::new(p("src/main.rs"), no_folder(cx), cx));
    let python = cx.update(|_, cx| cx.new(|cx| CodeView::new(p("tool.py"), no_folder(cx), cx)));
    rust.read_with(cx, |v, _| {
        assert_eq!(v.tab_size(), 2);
        assert!(!v.word_wrap());
    });
    python.read_with(cx, |v, _| {
        assert_eq!(v.tab_size(), 4);
        assert!(v.word_wrap());
    });
}

fn numbered(lines: u32) -> String {
    (0..lines).map(|i| format!("line {i}\n")).collect()
}

/// A cursor below the view scrolls just far enough to sit on the last
/// visible row.
#[gpui::test]
fn set_cursor_scrolls_minimally(cx: &mut TestAppContext) {
    init(cx, None);
    let (view, cx) = code_view(&numbered(200), cx);
    let visible = view.read_with(cx, |v, _| v.visible_rows());
    assert!(visible > 4, "the window shows several rows, got {visible}");
    view.update(cx, |v, cx| v.set_cursor(pt(150, 0), cx));
    draw(cx);
    assert_eq!(
        view.read_with(cx, |v, _| v.scroll_top_row()),
        150 - visible + 1
    );
    assert_eq!(view.read_with(cx, |v, _| v.cursor()), pt(150, 0));
    assert_eq!(view.read_with(cx, |v, _| v.selection()), None);

    // Back above the view: the cursor row becomes the top row.
    view.update(cx, |v, cx| v.set_cursor(pt(20, 0), cx));
    assert_eq!(view.read_with(cx, |v, _| v.scroll_top_row()), 20);
}

/// `editor.cursorSurroundingLines` keeps that many rows below the cursor.
#[gpui::test]
fn set_cursor_respects_surrounding_lines(cx: &mut TestAppContext) {
    init(cx, Some(r#"{ "editor.cursorSurroundingLines": 3 }"#));
    let (view, cx) = code_view(&numbered(200), cx);
    let visible = view.read_with(cx, |v, _| v.visible_rows());
    view.update(cx, |v, cx| v.set_cursor(pt(150, 0), cx));
    // Three rows further than with no margin (150 - visible + 1).
    assert_eq!(
        view.read_with(cx, |v, _| v.scroll_top_row()),
        150 - visible + 1 + 3
    );
    // Going up, three rows stay above the cursor.
    view.update(cx, |v, cx| v.set_cursor(pt(100, 0), cx));
    assert_eq!(view.read_with(cx, |v, _| v.scroll_top_row()), 97);
}

#[gpui::test]
fn set_cursor_inside_view_does_not_scroll(cx: &mut TestAppContext) {
    init(cx, None);
    let (view, cx) = code_view(&numbered(200), cx);
    view.update(cx, |v, cx| v.set_scroll_top_row(40, cx));
    draw(cx);
    let visible = view.read_with(cx, |v, _| v.visible_rows());
    for row in [40, 41, 40 + visible - 1] {
        view.update(cx, |v, cx| v.set_cursor(pt(row, 2), cx));
        assert_eq!(view.read_with(cx, |v, _| v.scroll_top_row()), 40);
    }
}

#[gpui::test]
fn multiple_selections_copy_rows(cx: &mut TestAppContext) {
    init(cx, None);
    let (view, cx) = code_view("abcd\nefgh", cx);
    view.update(cx, |v, cx| {
        let set = selections_of(v, &[pt(0, 1)..pt(0, 3), pt(1, 1)..pt(1, 3)]);
        v.set_selections(set, false, cx);
    });
    draw(cx);
    assert_eq!(
        view.read_with(cx, |v, _| v.selection()),
        Some(pt(0, 1)..pt(0, 3))
    );
    assert_eq!(
        view.read_with(cx, |v, _| v.copy_selection()).as_deref(),
        Some("bc\nfg")
    );
    view.update(cx, |v, cx| v.clear_selection(cx));
    assert_eq!(view.read_with(cx, |v, _| v.copy_selection()), None);
}

#[gpui::test]
fn fold_at_toggles_innermost(cx: &mut TestAppContext) {
    use lucerna_workbench::code_view::FoldOp;
    init(cx, None);
    let text = "fn a() {\n    if x {\n        y();\n        z();\n    }\n}\nlast\n";
    let (view, cx) = code_view(text, cx);
    cx.run_until_parked();
    let folds = view.read_with(cx, |v, _| v.fold_ranges().to_vec());
    assert!(
        folds.contains(&(0..5)) && folds.contains(&(1..4)),
        "{folds:?}"
    );

    // Row 2 is in both folds; the inner one closes.
    let header = view.update(cx, |v, cx| v.fold_at(2, FoldOp::Close, cx));
    assert_eq!(header, Some(1));
    view.read_with(cx, |v, _| {
        assert!(v.is_folded(1) && !v.is_folded(0));
        assert!(v.is_row_hidden(2) && !v.is_row_hidden(1));
    });
    // Closing again is a no-op; opening and toggling act on the same fold.
    let header = view.update(cx, |v, cx| v.fold_at(1, FoldOp::Close, cx));
    assert_eq!(header, Some(1));
    assert!(view.read_with(cx, |v, _| v.is_folded(1)));
    view.update(cx, |v, cx| v.fold_at(1, FoldOp::Open, cx));
    assert!(view.read_with(cx, |v, _| !v.is_folded(1) && !v.is_row_hidden(2)));
    view.update(cx, |v, cx| v.fold_at(1, FoldOp::Toggle, cx));
    assert!(view.read_with(cx, |v, _| v.is_folded(1)));
    // A row in no fold.
    assert_eq!(
        view.update(cx, |v, cx| v.fold_at(6, FoldOp::Toggle, cx)),
        None
    );

    // set_cursor opens the folds hiding its row.
    view.update(cx, |v, cx| v.set_cursor(pt(2, 0), cx));
    assert!(view.read_with(cx, |v, _| !v.is_row_hidden(2)));
}

#[gpui::test]
fn selection_generation_bumps_on_mouse_select(cx: &mut TestAppContext) {
    init(cx, None);
    let (view, cx) = code_view("alpha beta\nsecond line\nthird\n", cx);
    let generation =
        |cx: &mut VisualTestContext| view.read_with(cx, |v, _| v.selection_generation());
    let before = generation(cx);
    let left = gpui::MouseButton::Left;
    let none = gpui::Modifiers::none();
    cx.simulate_mouse_down(at(0, 2), left, none);
    cx.simulate_mouse_move(at(1, 4), left, none);
    cx.simulate_mouse_up(at(1, 4), left, none);
    let after_mouse = generation(cx);
    assert!(after_mouse > before);

    // Every other source bumps it too.
    let mut last = after_mouse;
    let mut bumped = |cx: &mut VisualTestContext, what: &str| {
        let now = generation(cx);
        assert!(now > last, "{what} bumps the generation");
        last = now;
    };
    view.update(cx, |v, cx| v.clear_selection(cx));
    bumped(cx, "clear_selection");
    view.update(cx, |v, cx| v.set_cursor(pt(1, 0), cx));
    bumped(cx, "set_cursor");
    view.update(cx, |v, cx| {
        let set = selections_of(v, &[pt(0, 0)..pt(0, 1), pt(1, 0)..pt(1, 1)]);
        v.set_selections(set, false, cx)
    });
    bumped(cx, "set_selections");
    view.update(cx, |v, cx| v.set_buffer(snapshot("x\n"), cx));
    bumped(cx, "set_buffer");
}

#[gpui::test]
fn reload_emits_event(cx: &mut TestAppContext) {
    use lucerna_workbench::code_view::CodeViewEvent;
    use std::cell::RefCell;
    init(cx, None);
    let old = "a\nb\n";
    let mut buffer = Buffer::from_bytes(old.as_bytes(), false).unwrap();
    let (view, cx) = code_view(old, cx);
    let events = Rc::new(RefCell::new(Vec::new()));
    cx.update(|_, cx| {
        let events = events.clone();
        cx.subscribe(&view, move |_, event: &CodeViewEvent, _| {
            if let CodeViewEvent::Reloaded { reload, .. } = event {
                events.borrow_mut().push(*reload);
            }
        })
        .detach();
    });
    let before = view.read_with(cx, |v, _| v.selection_generation());
    let reload = reload_into(&view, &mut buffer, b"a\nB\nc\n", cx);
    cx.run_until_parked();
    assert_eq!(*events.borrow(), vec![reload]);
    assert!(view.read_with(cx, |v, _| v.selection_generation()) > before);
}

#[gpui::test]
fn cursor_overlay_paints_without_panic(cx: &mut TestAppContext) {
    use lucerna_workbench::code_view::{CursorOverlay, CursorShape};
    init(cx, Some(r#"{ "editor.renderWhitespace": "all" }"#));
    let (view, cx) = code_view("abc\n\n😀x\tz\n日本\n", cx);
    let positions = [
        pt(0, 3),  // end of line
        pt(0, 99), // past the end
        pt(1, 0),  // empty line
        pt(2, 0),  // emoji
        pt(2, 2),  // inside the emoji
        pt(2, 5),  // tab
        pt(3, 3),  // CJK
        pt(40, 0), // past the last row
    ];
    for at in positions {
        view.update(cx, |v, cx| {
            v.set_cursor_overlay(
                Some(CursorOverlay {
                    at,
                    shape: CursorShape::Block,
                }),
                cx,
            )
        });
        draw(cx);
    }
    // With a selection per row and wrapping too.
    view.update(cx, |v, cx| {
        let set = selections_of(v, &[pt(2, 0)..pt(2, 9), pt(3, 0)..pt(3, 6)]);
        v.set_selections(set, false, cx);
        v.toggle_word_wrap(cx);
    });
    draw(cx);
    view.update(cx, |v, cx| v.set_cursor_overlay(None, cx));
    draw(cx);
}

/// `matching_bracket` finds the partner of the bracket at an offset.
#[gpui::test]
fn matching_bracket_finds_partner(cx: &mut TestAppContext) {
    init(cx, None);
    let (view, cx) = code_view("f(a[1])\n", cx);
    view.read_with(cx, |v, _| {
        assert_eq!(v.matching_bracket(1), Some(6));
        assert_eq!(v.matching_bracket(6), Some(1));
        assert_eq!(v.matching_bracket(3), Some(5));
        assert_eq!(v.matching_bracket(0), None);
    });
}

/// With word wrap on, rows take several display lines; `set_cursor`
/// still brings the cursor row on screen.
#[gpui::test]
fn set_cursor_reveals_wrapped_rows(cx: &mut TestAppContext) {
    init(cx, Some(r#"{ "editor.wordWrap": "on" }"#));
    let text: String = (0..60)
        .map(|i| format!("{i:02}{}\n", "x".repeat(400)))
        .collect();
    let (view, cx) = code_view(&text, cx);
    let visible = view.read_with(cx, |v, _| v.visible_rows());
    // `visible` buffer rows take several times as many display lines.
    let target = visible;
    view.update(cx, |v, cx| v.set_cursor(pt(target, 0), cx));
    draw(cx);
    let top = view.read_with(cx, |v, _| v.scroll_top_row());
    assert!(top <= target && top > 0, "top {top}, target {target}");
    // The row under the bottom whole line is at or below the cursor row.
    mouse_down(
        gpui::point(gpui::px(TEXT_X + 1.), gpui::px(visible as f32 * LINE - 1.)),
        1,
        false,
        cx,
    );
    assert!(view.read_with(cx, |v, _| v.cursor()).row >= target);
}

/// Before the first paint `set_cursor` centers its row, like `go_to`.
#[gpui::test]
fn set_cursor_before_first_paint_centers(cx: &mut TestAppContext) {
    init(cx, None);
    // Called while the view is built, before the window first draws.
    let (view, cx) = cx.add_window_view(|_, cx| {
        let mut v = CodeView::new(p("src/main.rs"), no_folder(cx), cx);
        v.set_buffer(snapshot(&numbered(1000)), cx);
        assert_eq!(v.visible_rows(), 0);
        v.set_cursor(pt(500, 0), cx);
        v
    });
    draw(cx);
    let visible = view.read_with(cx, |v, _| v.visible_rows());
    assert_eq!(
        view.read_with(cx, |v, _| v.scroll_top_row()),
        500 - visible / 2
    );
}

/// The bottom margin never scrolls past the last row.
#[gpui::test]
fn set_cursor_margin_stops_at_end_of_file(cx: &mut TestAppContext) {
    init(cx, Some(r#"{ "editor.cursorSurroundingLines": 3 }"#));
    let (view, cx) = code_view(&numbered(200), cx);
    let visible = view.read_with(cx, |v, _| v.visible_rows());
    view.update(cx, |v, cx| v.set_cursor(pt(199, 0), cx));
    assert_eq!(view.read_with(cx, |v, _| v.scroll_top_row()), 200 - visible);
}

/// Rows hidden by a fold between the top and the cursor take no space.
#[gpui::test]
fn set_cursor_counts_folded_rows_as_hidden(cx: &mut TestAppContext) {
    init(cx, None);
    let body: String = (0..20).map(|i| format!("    {i}\n")).collect();
    let text = format!("a {{\n{body}}}\n{}", numbered(200));
    let (view, cx) = code_view(&text, cx);
    cx.run_until_parked();
    view.update(cx, |v, cx| v.toggle_fold(0, cx));
    draw(cx);
    assert!(view.read_with(cx, |v, _| v.is_row_hidden(20)));
    let visible = view.read_with(cx, |v, _| v.visible_rows());
    // On screen: row 0, then rows 21.. — the last one is 21 + visible - 2.
    view.update(cx, |v, cx| v.set_cursor(pt(21 + visible - 2, 0), cx));
    assert_eq!(view.read_with(cx, |v, _| v.scroll_top_row()), 0);
    view.update(cx, |v, cx| v.set_cursor(pt(21 + visible - 1, 0), cx));
    assert_eq!(view.read_with(cx, |v, _| v.scroll_top_row()), 21);
    assert!(view.read_with(cx, |v, _| v.is_folded(0)));
}

#[gpui::test]
fn row_selections_follow_a_reload(cx: &mut TestAppContext) {
    init(cx, None);
    let old = "abcd\nefgh\nijkl\n";
    let mut buffer = Buffer::from_bytes(old.as_bytes(), false).unwrap();
    let (view, cx) = code_view(old, cx);
    // A change on another row keeps them; a new first line moves them.
    view.update(cx, |v, cx| {
        let set = selections_of(v, &[pt(0, 1)..pt(0, 3), pt(1, 1)..pt(1, 3)]);
        v.set_selections(set, false, cx)
    });
    reload_into(&view, &mut buffer, b"abcd\nefgh\nIJKL\n", cx);
    assert_eq!(
        view.read_with(cx, |v, _| v.copy_selection()).as_deref(),
        Some("bc\nfg")
    );
    reload_into(&view, &mut buffer, b"new\nabcd\nefgh\nIJKL\n", cx);
    assert_eq!(
        view.read_with(cx, |v, _| v.copy_selection()).as_deref(),
        Some("bc\nfg")
    );
    assert_eq!(view.read_with(cx, |v, _| v.selections().len()), 2);
}

#[gpui::test]
fn edit_config_reads_auto_settings(cx: &mut TestAppContext) {
    use lucerna_editor::{AutoClosing, AutoClosingEdit, AutoIndent, AutoSurround};
    init(
        cx,
        Some(
            r#"{ "editor.autoIndent": "keep", "editor.autoClosingBrackets": "never",
                 "editor.autoClosingQuotes": "bogus" }"#,
        ),
    );
    let (view, cx) = code_view("fn main() {}\n", cx);
    let config = view.read_with(cx, |v, cx| v.edit_config(cx));
    assert_eq!(config.auto_indent, AutoIndent::Keep);
    assert_eq!(config.auto_closing_brackets, AutoClosing::Never);
    // Unknown values and unset keys fall back to the defaults.
    assert_eq!(config.auto_closing_quotes, AutoClosing::LanguageDefined);
    assert_eq!(config.auto_closing_overtype, AutoClosingEdit::Auto);
    assert_eq!(config.auto_closing_delete, AutoClosingEdit::Auto);
    assert_eq!(config.auto_surround, AutoSurround::LanguageDefined);
    assert_eq!(
        config.multi_cursor_paste,
        lucerna_editor::MultiCursorPaste::Spread
    );
    let language = view.read_with(cx, |v, cx| v.edit_language(cx));
    assert!(language.is_some_and(|l| !l.editing().brackets.is_empty()));
}

fn long_text() -> String {
    (0..500).map(|i| format!("line {i}\n")).collect()
}

/// The mouse strip of the editor's scrollbar: `x` px left of the text
/// area's right edge, here the window's.
fn strip(x: f32, y: gpui::Pixels, cx: &mut VisualTestContext) -> gpui::Point<gpui::Pixels> {
    let viewport = cx.update(|window, _| window.viewport_size());
    gpui::point(viewport.width - gpui::px(x), y)
}

fn mouse_drag(
    from: gpui::Point<gpui::Pixels>,
    to: gpui::Point<gpui::Pixels>,
    cx: &mut VisualTestContext,
) {
    let (left, none) = (gpui::MouseButton::Left, gpui::Modifiers::none());
    cx.simulate_mouse_down(from, left, none);
    draw(cx);
    cx.simulate_mouse_move(to, Some(left), none);
    draw(cx);
    cx.simulate_mouse_up(to, left, none);
    draw(cx);
}

#[gpui::test]
fn editor_scrollbar_drag_scrolls(cx: &mut TestAppContext) {
    init(cx, None);
    let (view, cx) = code_view(&long_text(), cx);
    let viewport = cx.update(|window, _| window.viewport_size());
    let thumb = view.read_with(cx, |v, _| v.scrollbar_thumb()).unwrap();
    let from = thumb.center();
    let to = gpui::point(from.x, from.y + viewport.height - thumb.size.height);
    mouse_drag(from, to, cx);
    assert_eq!(view.read_with(cx, |v, _| v.scroll_top_row()), 499);
    let end = view.read_with(cx, |v, _| v.scrollbar_thumb()).unwrap();
    assert!((end.bottom() - viewport.height).abs() < gpui::px(0.01));
}

#[gpui::test]
fn editor_track_click_jumps(cx: &mut TestAppContext) {
    init(cx, None);
    let (view, cx) = code_view(&long_text(), cx);
    let viewport = cx.update(|window, _| window.viewport_size());
    let at = strip(3., viewport.height / 2., cx);
    cx.simulate_click(at, gpui::Modifiers::none());
    draw(cx);
    let top = view.read_with(cx, |v, _| v.scroll_top_row());
    assert!((248..=250).contains(&top), "top row {top}");
}

#[gpui::test]
fn scroll_by_page_setting_pages(cx: &mut TestAppContext) {
    init(cx, Some(r#"{ "editor.scrollbar.scrollByPage": true }"#));
    let (view, cx) = code_view(&long_text(), cx);
    let viewport = cx.update(|window, _| window.viewport_size());
    let at = strip(3., viewport.height - gpui::px(2.), cx);
    cx.simulate_click(at, gpui::Modifiers::none());
    draw(cx);
    let (top, page) = view.read_with(cx, |v, _| (v.scroll_top_row(), v.visible_rows()));
    assert!(top.abs_diff(page) <= 1, "top {top}, page {page}");
}

#[gpui::test]
fn track_click_with_folds_lands_on_visible_row(cx: &mut TestAppContext) {
    use lucerna_workbench::code_view::FoldOp;
    init(cx, None);
    let mut text = String::from("fn big() {\n");
    text.extend((0..200).map(|i| format!("    x{i}();\n")));
    text.push_str("}\n");
    text.extend((0..297).map(|i| format!("line {i}\n")));
    let (view, cx) = code_view(&text, cx);
    cx.run_until_parked();
    assert_eq!(
        view.update(cx, |v, cx| v.fold_at(0, FoldOp::Close, cx)),
        Some(0)
    );
    draw(cx);
    // As in the other tests, n newline-ended lines are rows 0..n.
    let lines = text.matches('\n').count() as u32;
    let hidden = view.read_with(cx, |v, _| {
        (0..lines).filter(|&r| v.is_row_hidden(r)).count() as u32
    });
    assert!(hidden >= 200, "hidden {hidden}");
    let shown = lines - hidden;

    let viewport = cx.update(|window, _| window.viewport_size());
    let at = strip(3., viewport.height / 2., cx);
    cx.simulate_click(at, gpui::Modifiers::none());
    draw(cx);
    let top = view.read_with(cx, |v, _| v.scroll_top_row());
    // Past the fold, so every hidden row is above it.
    let index = top - hidden;
    assert!(
        index.abs_diff((shown - 1) / 2) <= 1,
        "top {top}, shown {shown}"
    );

    let thumb = view.read_with(cx, |v, _| v.scrollbar_thumb()).unwrap();
    let from = thumb.center();
    mouse_drag(from, gpui::point(from.x, from.y + viewport.height), cx);
    assert_eq!(view.read_with(cx, |v, _| v.scroll_top_row()), lines - 1);
}

#[gpui::test]
fn scrollbar_click_keeps_selection(cx: &mut TestAppContext) {
    init(cx, None);
    let (view, cx) = code_view(&long_text(), cx);
    let before = view.read_with(cx, |v, _| v.selection_head());
    let viewport = cx.update(|window, _| window.viewport_size());
    let at = strip(3., viewport.height / 2., cx);
    cx.simulate_click(at, gpui::Modifiers::none());
    draw(cx);
    let thumb = view.read_with(cx, |v, _| v.scrollbar_thumb()).unwrap();
    mouse_drag(
        thumb.center(),
        gpui::point(thumb.center().x, gpui::px(5.)),
        cx,
    );
    assert_eq!(view.read_with(cx, |v, _| v.selection_head()), before);
    assert_eq!(view.read_with(cx, |v, _| v.selection()), None);
}

#[gpui::test]
fn minimap_still_scrolls_next_to_scrollbar(cx: &mut TestAppContext) {
    init(cx, Some(r#"{ "editor.minimap.enabled": true }"#));
    let (view, cx) = code_view(&long_text(), cx);
    let minimap = cx.debug_bounds("code-view-minimap").expect("minimap on");
    let at = gpui::point(
        minimap.left() + gpui::px(2.),
        minimap.bottom() - gpui::px(5.),
    );
    cx.simulate_click(at, gpui::Modifiers::none());
    draw(cx);
    assert!(view.read_with(cx, |v, _| v.scroll_top_row()) > 0);
}

/// With word wrap, a row's display lines share its one-line slice of the
/// track: a slow drag keeps the thumb under the pointer.
#[gpui::test]
fn wrapped_drag_keeps_the_grab_point(cx: &mut TestAppContext) {
    init(cx, Some(r#"{ "editor.wordWrap": "on" }"#));
    let text: String = (0..300)
        .map(|i| format!("{i:03}{}\n", "x".repeat(400)))
        .collect();
    let (view, cx) = code_view(&text, cx);
    let start = view.read_with(cx, |v, _| v.scrollbar_thumb()).unwrap();
    let (left, none) = (gpui::MouseButton::Left, gpui::Modifiers::none());
    let from = start.center();
    cx.simulate_mouse_down(from, left, none);
    draw(cx);
    for step in 1..=20 {
        let dy = gpui::px(step as f32 * 0.7);
        cx.simulate_mouse_move(gpui::point(from.x, from.y + dy), Some(left), none);
        draw(cx);
        let thumb = view.read_with(cx, |v, _| v.scrollbar_thumb()).unwrap();
        let moved = thumb.top() - start.top();
        assert!(
            (moved - dy).abs() < gpui::px(0.5),
            "step {step}: moved {moved:?}, dy {dy:?}"
        );
    }
    cx.simulate_mouse_up(from, left, none);
}

/// A page is a screenful of display lines, not of buffer rows.
#[gpui::test]
fn wrapped_page_click_moves_one_screen(cx: &mut TestAppContext) {
    init(
        cx,
        Some(r#"{ "editor.wordWrap": "on", "editor.scrollbar.scrollByPage": true }"#),
    );
    let text: String = (0..300)
        .map(|i| format!("{i:03}{}\n", "x".repeat(400)))
        .collect();
    let (view, cx) = code_view(&text, cx);
    let viewport = cx.update(|window, _| window.viewport_size());
    let lines_on_screen = (viewport.height / gpui::px(LINE)) as u32;
    let at = strip(3., viewport.height - gpui::px(2.), cx);
    cx.simulate_click(at, gpui::Modifiers::none());
    draw(cx);
    let top = view.read_with(cx, |v, _| v.scroll_top_row());
    // Each row is several lines, so a screen is fewer rows than lines.
    assert!(
        top > 0 && top < lines_on_screen,
        "top {top}, lines {lines_on_screen}"
    );
}
