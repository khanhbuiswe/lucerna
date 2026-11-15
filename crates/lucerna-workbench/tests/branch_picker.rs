//! The branch picker in Checkout mode: row order, the Create row, and the
//! events it emits. It runs standalone, as the workbench hosts it.

use std::sync::{Arc, Mutex};

use gpui::{Entity, Focusable, TestAppContext, VisualTestContext};
use lucerna_git::{BranchRef, SwitchTarget, is_valid_branch_name};
use lucerna_settings::{Keymap, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::{IconThemeRegistry, ThemeRegistry};
use lucerna_workbench::{AppState, BranchPicker, BranchPickerEvent};

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

fn local(name: &str, committed: i64, is_head: bool) -> BranchRef {
    BranchRef {
        name: name.into(),
        remote: None,
        committed,
        is_head,
    }
}

fn remote(name: &str, committed: i64) -> BranchRef {
    BranchRef {
        name: name.into(),
        remote: Some("origin".into()),
        committed,
        is_head: false,
    }
}

/// `main` (current), `feature`, `origin/docs`, as `branch_list` returns them.
fn standard() -> Vec<BranchRef> {
    vec![
        local("main", 300, true),
        local("feature", 200, false),
        remote("origin/docs", 100),
    ]
}

type Events = Arc<Mutex<Vec<BranchPickerEvent>>>;

fn open(
    branches: Vec<BranchRef>,
    cx: &mut TestAppContext,
) -> (Entity<BranchPicker>, Events, &mut VisualTestContext) {
    init(cx);
    let (picker, cx) = cx.add_window_view(move |_, cx| {
        BranchPicker::new_checkout(branches, Arc::new(is_valid_branch_name), cx)
    });
    let events: Events = Arc::default();
    let sink = events.clone();
    cx.update(|_, cx| {
        cx.subscribe(&picker, move |_, event: &BranchPickerEvent, _| {
            sink.lock().unwrap().push(event.clone());
        })
        .detach();
    });
    draw(cx);
    let focus = cx.update(|_, cx| picker.read(cx).focus_handle(cx));
    cx.update(|window, cx| window.focus(&focus, cx));
    (picker, events, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

fn texts(picker: &Entity<BranchPicker>, cx: &mut VisualTestContext) -> Vec<String> {
    cx.update(|_, cx| picker.read(cx).row_texts())
}

fn selected(picker: &Entity<BranchPicker>, cx: &mut VisualTestContext) -> usize {
    cx.update(|_, cx| picker.read(cx).selected_index())
}

fn create_row(picker: &Entity<BranchPicker>, cx: &mut VisualTestContext) -> Option<(String, bool)> {
    cx.update(|_, cx| picker.read(cx).create_row())
}

fn type_query(picker: &Entity<BranchPicker>, text: &str, cx: &mut VisualTestContext) {
    let input = cx.update(|_, cx| picker.read(cx).input().clone());
    cx.update(|window, cx| input.update(cx, |input, cx| input.set_text(text, window, cx)));
    draw(cx);
}

fn events(events: &Events) -> Vec<BranchPickerEvent> {
    events.lock().unwrap().clone()
}

#[gpui::test]
fn checkout_rows_order_and_mark(cx: &mut TestAppContext) {
    // Handed over out of order: the picker groups and sorts them.
    let (picker, _, cx) = open(
        vec![
            remote("origin/docs", 100),
            local("old", 50, false),
            local("feature", 200, false),
            local("main", 300, true),
            remote("origin/new", 400),
        ],
        cx,
    );
    let rows = texts(&picker, cx);
    let names: Vec<&str> = rows.iter().map(|r| r.split("  ").next().unwrap()).collect();
    assert_eq!(
        names,
        ["✓ main", "feature", "old", "origin/new", "origin/docs"]
    );
}

#[gpui::test]
fn rows_show_relative_time(cx: &mut TestAppContext) {
    let (picker, _, cx) = open(vec![local("main", 0, true)], cx);
    let rows = texts(&picker, cx);
    assert!(rows[0].ends_with(" years ago"), "{rows:?}");
}

#[gpui::test]
fn create_row_for_new_name(cx: &mut TestAppContext) {
    let (picker, _, cx) = open(standard(), cx);
    assert_eq!(create_row(&picker, cx), None);
    type_query(&picker, "new thing", cx);
    assert_eq!(create_row(&picker, cx), Some(("new-thing".into(), true)));
    let rows = texts(&picker, cx);
    assert_eq!(rows.last().unwrap(), "+ Create branch \"new-thing\"");
}

#[gpui::test]
fn create_row_compares_sanitized_names(cx: &mut TestAppContext) {
    let (picker, _, cx) = open(standard(), cx);
    type_query(&picker, " main ", cx);
    assert_eq!(create_row(&picker, cx), None);
    let rows = texts(&picker, cx);
    assert!(
        rows[selected(&picker, cx)].starts_with("✓ main"),
        "{rows:?}"
    );
}

#[gpui::test]
fn create_row_hidden_when_remote_has_it(cx: &mut TestAppContext) {
    let (picker, _, cx) = open(vec![remote("origin/foo", 1)], cx);
    type_query(&picker, "foo", cx);
    assert_eq!(create_row(&picker, cx), None);
    // `origin/foo` itself is not a branch part: `origin/foo` is creatable.
    type_query(&picker, "origin/foo", cx);
    assert_eq!(create_row(&picker, cx), Some(("origin/foo".into(), true)));
}

#[gpui::test]
fn invalid_name_row_is_inert(cx: &mut TestAppContext) {
    let (picker, events_, cx) = open(standard(), cx);
    type_query(&picker, "a..b", cx);
    assert_eq!(create_row(&picker, cx), Some(("a..b".into(), false)));
    let rows = texts(&picker, cx);
    assert!(
        rows.last().unwrap().ends_with("Invalid branch name"),
        "{rows:?}"
    );
    // Only the Create row is left; it is selected.
    assert_eq!(selected(&picker, cx), rows.len() - 1);
    cx.simulate_keystrokes("enter");
    assert_eq!(events(&events_), vec![]);
    let bounds = cx.debug_bounds("branch-picker-row-0").expect("row drawn");
    cx.simulate_click(bounds.center(), gpui::Modifiers::none());
    assert_eq!(events(&events_), vec![]);
}

#[gpui::test]
fn enter_emits_targets(cx: &mut TestAppContext) {
    let (picker, events_, cx) = open(standard(), cx);
    // The current branch.
    cx.simulate_keystrokes("enter");
    // A local branch.
    cx.simulate_keystrokes("down enter");
    // A remote branch.
    cx.simulate_keystrokes("down enter");
    // The Create row.
    type_query(&picker, "new thing", cx);
    assert_eq!(create_row(&picker, cx), Some(("new-thing".into(), true)));
    assert_eq!(selected(&picker, cx), texts(&picker, cx).len() - 1);
    cx.simulate_keystrokes("enter");
    assert_eq!(
        events(&events_),
        vec![
            BranchPickerEvent::Dismiss,
            BranchPickerEvent::Checkout(SwitchTarget::Local("feature".into())),
            BranchPickerEvent::Checkout(SwitchTarget::Remote {
                remote: "origin".into(),
                branch: "docs".into(),
            }),
            BranchPickerEvent::Checkout(SwitchTarget::Create("new-thing".into())),
        ]
    );
}

#[gpui::test]
fn click_picks_a_row(cx: &mut TestAppContext) {
    let (_, events_, cx) = open(standard(), cx);
    let bounds = cx.debug_bounds("branch-picker-row-1").expect("row drawn");
    cx.simulate_click(bounds.center(), gpui::Modifiers::none());
    assert_eq!(
        events(&events_),
        vec![BranchPickerEvent::Checkout(SwitchTarget::Local(
            "feature".into()
        ))]
    );
}

#[gpui::test]
fn escape_dismisses(cx: &mut TestAppContext) {
    let (_, events_, cx) = open(standard(), cx);
    cx.simulate_keystrokes("escape");
    assert_eq!(events(&events_), vec![BranchPickerEvent::Dismiss]);
}
