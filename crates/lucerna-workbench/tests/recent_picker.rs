//! The Open Recent picker: grouping, filtering, selection, and the events
//! it emits. It runs standalone, as the Welcome screen and the workbench
//! host it.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use lucerna_settings::{Keymap, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::{IconThemeRegistry, ThemeRegistry};
use lucerna_workbench::recent::{RecentEntry, RecentKind};
use lucerna_workbench::{AppState, RecentPicker, RecentPickerEvent, WindowSettings};

fn init(cx: &mut TestAppContext) {
    let (keymap, error) = Keymap::load(Keymap::default_bindings(), None);
    assert!(error.is_none(), "keymap error: {error:?}");
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
        icon_themes: IconThemeRegistry::build(&[]).0,
        icons: IconThemeRegistry::build(&[]).0.resolve("Lucide"),
    });
}

fn entry(path: &str, kind: RecentKind) -> RecentEntry {
    RecentEntry {
        path: PathBuf::from(path),
        kind,
    }
}

fn folder(path: &str) -> RecentEntry {
    entry(path, RecentKind::Folder)
}

type Events = Arc<Mutex<Vec<RecentPickerEvent>>>;

fn open(
    entries: Vec<RecentEntry>,
    cx: &mut TestAppContext,
) -> (Entity<RecentPicker>, Events, &mut VisualTestContext) {
    init(cx);
    let (picker, cx) = cx.add_window_view(move |_, cx| {
        let settings = cx.new(|cx| WindowSettings::new(None, cx));
        RecentPicker::new(entries, Some(PathBuf::from("/Users/me")), settings, cx)
    });
    let events: Events = Arc::default();
    let sink = events.clone();
    cx.update(|_, cx| {
        cx.subscribe(&picker, move |_, event: &RecentPickerEvent, _| {
            sink.lock().unwrap().push(event.clone());
        })
        .detach();
    });
    draw(cx);
    (picker, events, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

fn texts(picker: &Entity<RecentPicker>, cx: &mut VisualTestContext) -> Vec<String> {
    cx.update(|_, cx| picker.read(cx).row_texts())
}

fn type_query(picker: &Entity<RecentPicker>, text: &str, cx: &mut VisualTestContext) {
    let input = cx.update(|_, cx| picker.read(cx).input().clone());
    cx.update(|window, cx| input.update(cx, |input, cx| input.set_text(text, window, cx)));
    draw(cx);
}

#[gpui::test]
fn lists_folders_then_files_with_headers(cx: &mut TestAppContext) {
    let (picker, _, cx) = open(
        vec![
            folder("/Users/me/a/lucerna"),
            entry("/Users/me/notes.md", RecentKind::File),
            folder("/Users/me/src/zed"),
        ],
        cx,
    );
    assert_eq!(
        texts(&picker, cx),
        vec!["lucerna ~/a", "zed ~/src", "notes.md ~"]
    );
    assert!(cx.debug_bounds("recent-picker-header-folders").is_some());
    assert!(cx.debug_bounds("recent-picker-header-files").is_some());
    assert!(cx.debug_bounds("recent-picker-row-2").is_some());
}

#[gpui::test]
fn no_headers_without_files(cx: &mut TestAppContext) {
    let (_, _, cx) = open(vec![folder("/Users/me/a"), folder("/Users/me/b")], cx);
    assert!(cx.debug_bounds("recent-picker-header-folders").is_none());
    assert!(cx.debug_bounds("recent-picker-header-files").is_none());
}

#[gpui::test]
fn filters_on_name_and_parent(cx: &mut TestAppContext) {
    let (picker, _, cx) = open(
        vec![
            folder("/Users/me/a/lucerna"),
            entry("/Users/me/notes.md", RecentKind::File),
            folder("/Users/me/src/zed"),
        ],
        cx,
    );
    type_query(&picker, "src", cx);
    assert_eq!(texts(&picker, cx), vec!["zed ~/src"]);
    assert!(cx.debug_bounds("recent-picker-header-folders").is_none());
    assert!(cx.debug_bounds("recent-picker-header-files").is_none());
}

#[gpui::test]
fn ctrl_r_moves_to_the_next_row(cx: &mut TestAppContext) {
    let (picker, _, cx) = open(vec![folder("/Users/me/a"), folder("/Users/me/b")], cx);
    assert_eq!(cx.update(|_, cx| picker.read(cx).selected_index()), 0);
    cx.update(|_, cx| picker.update(cx, |p, cx| p.select_next(1, cx)));
    assert_eq!(cx.update(|_, cx| picker.read(cx).selected_index()), 1);
    cx.update(|_, cx| picker.update(cx, |p, cx| p.select_next(1, cx)));
    assert_eq!(cx.update(|_, cx| picker.read(cx).selected_index()), 0);
}

#[gpui::test]
fn confirm_emits_open(cx: &mut TestAppContext) {
    let (picker, events, cx) = open(vec![folder("/Users/me/a"), folder("/Users/me/b")], cx);
    cx.update(|_, cx| picker.update(cx, |p, cx| p.select_next(1, cx)));
    cx.update(|_, cx| picker.update(cx, |p, cx| p.confirm(true, cx)));
    assert_eq!(
        *events.lock().unwrap(),
        vec![RecentPickerEvent::Open {
            entry: folder("/Users/me/b"),
            new_window: true,
        }]
    );
}

#[gpui::test]
fn remove_emits_and_drops_the_row(cx: &mut TestAppContext) {
    let (picker, events, cx) = open(vec![folder("/Users/me/a"), folder("/Users/me/b")], cx);
    cx.update(|_, cx| picker.update(cx, |p, cx| p.remove_at(0, cx)));
    draw(cx);
    assert_eq!(
        *events.lock().unwrap(),
        vec![RecentPickerEvent::Remove(PathBuf::from("/Users/me/a"))]
    );
    assert_eq!(texts(&picker, cx), vec!["b ~"]);
}

#[gpui::test]
fn empty_list_shows_placeholder(cx: &mut TestAppContext) {
    let (_, _, cx) = open(Vec::new(), cx);
    assert!(cx.debug_bounds("recent-picker-empty").is_some());
}

#[gpui::test]
fn keys_drive_the_picker(cx: &mut TestAppContext) {
    let (picker, events, cx) = open(vec![folder("/Users/me/a"), folder("/Users/me/b")], cx);
    let focus = cx.update(|_, cx| gpui::Focusable::focus_handle(picker.read(cx), cx));
    cx.update(|window, cx| window.focus(&focus, cx));
    cx.simulate_keystrokes("down");
    assert_eq!(cx.update(|_, cx| picker.read(cx).selected_index()), 1);
    cx.simulate_keystrokes("ctrl-r");
    assert_eq!(cx.update(|_, cx| picker.read(cx).selected_index()), 0);
    cx.simulate_keystrokes("ctrl-shift-r");
    assert_eq!(cx.update(|_, cx| picker.read(cx).selected_index()), 1);
    cx.simulate_keystrokes("escape");
    assert_eq!(*events.lock().unwrap(), vec![RecentPickerEvent::Dismiss]);
}

fn two(cx: &mut TestAppContext) -> (Entity<RecentPicker>, Events, &mut VisualTestContext) {
    open(vec![folder("/Users/me/a"), folder("/Users/me/b")], cx)
}

fn center(selector: &'static str, cx: &mut VisualTestContext) -> gpui::Point<gpui::Pixels> {
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"))
        .center()
}

fn open_event(path: &str, new_window: bool) -> RecentPickerEvent {
    RecentPickerEvent::Open {
        entry: folder(path),
        new_window,
    }
}

#[gpui::test]
fn clicking_a_row_opens_it(cx: &mut TestAppContext) {
    let (_, events, cx) = two(cx);
    let at = center("recent-picker-row-1", cx);
    cx.simulate_click(at, gpui::Modifiers::none());
    assert_eq!(
        *events.lock().unwrap(),
        vec![open_event("/Users/me/b", false)]
    );
}

#[gpui::test]
fn cmd_clicking_a_row_opens_a_new_window(cx: &mut TestAppContext) {
    let (_, events, cx) = two(cx);
    let at = center("recent-picker-row-0", cx);
    cx.simulate_click(at, gpui::Modifiers::command());
    assert_eq!(
        *events.lock().unwrap(),
        vec![open_event("/Users/me/a", true)]
    );
}

#[gpui::test]
fn the_remove_button_only_removes(cx: &mut TestAppContext) {
    let (picker, events, cx) = two(cx);
    let at = center("recent-picker-row-0", cx);
    cx.simulate_mouse_move(at, None, gpui::Modifiers::none());
    draw(cx);
    let at = center("recent-picker-remove-0", cx);
    cx.simulate_click(at, gpui::Modifiers::none());
    assert_eq!(
        *events.lock().unwrap(),
        vec![RecentPickerEvent::Remove(PathBuf::from("/Users/me/a"))]
    );
    assert_eq!(texts(&picker, cx), vec!["b ~"]);
}

#[gpui::test]
fn the_new_window_button_only_opens_a_new_window(cx: &mut TestAppContext) {
    let (_, events, cx) = two(cx);
    let at = center("recent-picker-row-0", cx);
    cx.simulate_mouse_move(at, None, gpui::Modifiers::none());
    draw(cx);
    let at = center("recent-picker-new-window-0", cx);
    cx.simulate_click(at, gpui::Modifiers::none());
    assert_eq!(
        *events.lock().unwrap(),
        vec![open_event("/Users/me/a", true)]
    );
}

#[gpui::test]
fn enter_and_cmd_enter_open_the_selected_row(cx: &mut TestAppContext) {
    let (picker, events, cx) = two(cx);
    let focus = cx.update(|_, cx| gpui::Focusable::focus_handle(picker.read(cx), cx));
    cx.update(|window, cx| window.focus(&focus, cx));
    // Only the secondary modifier alone opens a new window.
    cx.simulate_keystrokes("alt-enter shift-cmd-enter");
    cx.simulate_keystrokes("down enter");
    cx.simulate_keystrokes("cmd-enter");
    assert_eq!(
        *events.lock().unwrap(),
        vec![
            open_event("/Users/me/b", false),
            open_event("/Users/me/b", true)
        ]
    );
}
