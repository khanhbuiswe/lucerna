//! Switching branches (spec §4.3–§4.5): the title bar branch button,
//! `git.checkout`, the gates on the git state, the unsaved-files check,
//! git's refusal and the single switch in flight.

mod support;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    Entity, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, TestAppContext,
    VisualTestContext, point, px,
};
use lucerna_git::SwitchTarget;
use lucerna_project::{Change, ChangeKind, WorktreeEvent};
use lucerna_settings::{Keymap, Layer, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{
    AppState, BranchPickerEvent, BranchPickerMode, CheckoutAnchor, GitState, OpenMode, Workbench,
    status_message,
};
use support::git::{git, isolated_git, repo};
use tempfile::TempDir;

const BUTTON: &str = "title-bar-branch-button";
const UNSAVED: &str = "Save or revert unsaved files before switching branches";

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
    for _ in 0..3 {
        draw(cx);
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(60));
    }
    draw(cx);
}

/// A workbench on `root`; `trusted` lets git run.
fn open_workbench<'a>(
    root: &Path,
    settings: Option<&str>,
    trusted: bool,
    cx: &'a mut TestAppContext,
) -> (Entity<Workbench>, &'a mut VisualTestContext) {
    init(cx, settings);
    let root = root.to_path_buf();
    let (wb, cx) = cx.add_window_view(|window, cx| Workbench::new(root, window, cx));
    wb.update(cx, |wb, cx| {
        wb.git().update(cx, |git, cx| {
            git.use_git_for_test(isolated_git());
            git.set_trusted(trusted, cx);
        });
    });
    settle(cx);
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(&WorktreeEvent::ScanComplete, cx)
    });
    settle(cx);
    (wb, cx)
}

/// A repo on `main` (committed `a.txt`, `b.txt`) with a branch `feature`
/// that adds `f.txt`, changed by `prepare` before the workbench opens.
fn workbench_with(
    trusted: bool,
    prepare: impl FnOnce(&Path),
    cx: &mut TestAppContext,
) -> (TempDir, PathBuf, Entity<Workbench>, &mut VisualTestContext) {
    let (tmp, dir) = repo();
    git(&dir, &["checkout", "-q", "-b", "feature"]);
    std::fs::write(dir.join("f.txt"), "feature\n").unwrap();
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "feature"]);
    git(&dir, &["checkout", "-q", "main"]);
    prepare(&dir);
    let (wb, cx) = open_workbench(&dir, None, trusted, cx);
    (tmp, dir, wb, cx)
}

fn workbench(
    cx: &mut TestAppContext,
) -> (TempDir, PathBuf, Entity<Workbench>, &mut VisualTestContext) {
    workbench_with(true, |_| {}, cx)
}

fn head(dir: &Path) -> String {
    git(dir, &["rev-parse", "--abbrev-ref", "HEAD"])
        .trim()
        .to_string()
}

fn picker_rows(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Option<Vec<String>> {
    cx.update(|_, cx| wb.read(cx).branch_picker().map(|p| p.read(cx).row_texts()))
}

fn picker_mode(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Option<BranchPickerMode> {
    cx.update(|_, cx| wb.read(cx).branch_picker().map(|p| p.read(cx).mode()))
}

fn click_button(cx: &mut VisualTestContext) {
    draw(cx);
    let bounds = cx
        .debug_bounds(BUTTON)
        .unwrap_or_else(|| panic!("{BUTTON} is drawn"));
    cx.simulate_click(bounds.center(), Modifiers::none());
    settle(cx);
}

fn run(wb: &Entity<Workbench>, command: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| {
        wb.update(cx, |wb, cx| {
            assert!(wb.run_command(command, None, window, cx), "{command}");
        })
    });
    settle(cx);
}

fn message(cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|_, cx| status_message::current(cx).map(|m| m.to_string()))
}

fn switches(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> usize {
    cx.update(|_, cx| wb.read(cx).switches_started())
}

/// Opens the picker from the title bar, types `query` and presses Enter.
fn pick(wb: &Entity<Workbench>, query: &str, cx: &mut VisualTestContext) {
    click_button(cx);
    assert!(picker_rows(wb, cx).is_some(), "the picker is shown");
    cx.simulate_input(query);
    settle(cx);
    cx.simulate_keystrokes("enter");
    settle(cx);
    assert!(picker_rows(wb, cx).is_none(), "picking closes the picker");
}

fn open(wb: &Entity<Workbench>, name: &str, cx: &mut VisualTestContext) -> Entity<CodeView> {
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new(name)), OpenMode::Pinned, cx)
    });
    settle(cx);
    cx.update(|_, cx| {
        wb.read(cx)
            .editor_groups()
            .read(cx)
            .active_body()
            .expect("the active tab has a body")
            .downcast::<CodeView>()
            .expect("tab bodies are code views")
    })
}

fn insert(wb: &Entity<Workbench>, view: &Entity<CodeView>, name: &str, cx: &mut VisualTestContext) {
    let docs = cx.update(|_, cx| wb.read(cx).documents().clone());
    let origin = view.entity_id();
    docs.update(cx, |d, cx| {
        d.apply_edits(
            Path::new(name),
            vec![(0..0, Arc::from("x"))],
            None,
            None,
            origin,
            cx,
        )
    })
    .expect("the document is loaded");
    settle(cx);
}

#[gpui::test]
fn click_opens_checkout_picker_with_current_first(cx: &mut TestAppContext) {
    let (_t, _dir, wb, cx) = workbench(cx);
    assert_eq!(
        cx.update(|_, cx| wb.read(cx).git_branch().map(str::to_owned)),
        Some("main".into())
    );
    click_button(cx);
    assert_eq!(picker_mode(&wb, cx), Some(BranchPickerMode::Checkout));
    let rows = picker_rows(&wb, cx).unwrap();
    assert!(rows[0].starts_with("✓ main"), "{rows:?}");
    assert!(rows[1].starts_with("feature"), "{rows:?}");
    // Anchored under the button, left-aligned to it.
    draw(cx);
    let button = cx.debug_bounds(BUTTON).unwrap();
    let picker = cx.debug_bounds("branch-picker").expect("picker drawn");
    assert!(
        (picker.left() - button.left()).abs() < px(1.),
        "{picker:?} vs {button:?}"
    );
    assert!(picker.top() >= button.bottom(), "{picker:?} vs {button:?}");

    // Escape closes it; the command shows it at the Quick Open position.
    cx.simulate_keystrokes("escape");
    settle(cx);
    assert!(picker_rows(&wb, cx).is_none());
    run(&wb, "git.checkout", cx);
    assert_eq!(picker_mode(&wb, cx), Some(BranchPickerMode::Checkout));
    let picker = cx.debug_bounds("branch-picker").expect("picker drawn");
    let window = cx.update(|window, _| window.viewport_size());
    let center = picker.center().x;
    assert!(
        (center - window.width / 2.).abs() < px(2.),
        "centered: {picker:?} in {window:?}"
    );
}

#[gpui::test]
fn picking_branch_switches_and_updates_title(cx: &mut TestAppContext) {
    let (_t, dir, wb, cx) = workbench(cx);
    let before = cx.update(|_, cx| wb.read(cx).git().read(cx).snapshot().head);
    pick(&wb, "feature", cx);
    assert_eq!(head(&dir), "feature");
    assert!(dir.join("f.txt").exists());
    assert_eq!(
        cx.update(|_, cx| wb.read(cx).git_branch().map(str::to_owned)),
        Some("feature".into())
    );
    assert!(!cx.update(|_, cx| wb.read(cx).is_switching_branch()));
    let after = cx.update(|_, cx| wb.read(cx).git().read(cx).snapshot().head);
    let feature = git(&dir, &["rev-parse", "HEAD"]).trim().to_string();
    assert_ne!(before, after, "the statuses were rescanned");
    assert_eq!(after.as_deref(), Some(feature.as_str()));
}

#[gpui::test]
fn create_branch_from_picker(cx: &mut TestAppContext) {
    let (_t, dir, wb, cx) = workbench(cx);
    pick(&wb, "new topic", cx);
    assert_eq!(head(&dir), "new-topic");
    assert_eq!(
        cx.update(|_, cx| wb.read(cx).git_branch().map(str::to_owned)),
        Some("new-topic".into())
    );
}

#[gpui::test]
fn unsaved_file_blocks_switch(cx: &mut TestAppContext) {
    let (_t, dir, wb, cx) = workbench(cx);
    let view = open(&wb, "a.txt", cx);
    insert(&wb, &view, "a.txt", cx);
    pick(&wb, "feature", cx);
    assert_eq!(message(cx).as_deref(), Some(UNSAVED));
    assert_eq!(head(&dir), "main");
    assert_eq!(switches(&wb, cx), 0);
}

#[gpui::test]
fn conflicted_document_blocks_switch(cx: &mut TestAppContext) {
    let (_t, dir, wb, cx) = workbench(cx);
    let view = open(&wb, "a.txt", cx);
    insert(&wb, &view, "a.txt", cx);
    std::fs::write(dir.join("a.txt"), "ONE\ntwo\nthree\n").unwrap();
    wb.update(cx, |wb, cx| {
        wb.on_worktree_event(
            &WorktreeEvent::Changed(vec![Change {
                path: Arc::from(dir.join("a.txt")),
                kind: ChangeKind::Modified,
            }]),
            cx,
        )
    });
    settle(cx);
    let docs = cx.update(|_, cx| wb.read(cx).documents().clone());
    docs.update(cx, |d, cx| d.undo(Path::new("a.txt"), cx))
        .expect("undone");
    settle(cx);
    let (dirty, conflict) = cx.update(|_, cx| {
        let d = docs.read(cx);
        (d.dirty_paths(), d.has_conflict(Path::new("a.txt")))
    });
    assert!(dirty.is_empty(), "clean after the undo: {dirty:?}");
    assert!(conflict, "the conflict stays while a redo is possible");

    pick(&wb, "feature", cx);
    assert_eq!(message(cx).as_deref(), Some(UNSAVED));
    assert_eq!(head(&dir), "main");
}

#[gpui::test]
fn git_refusal_shows_prompt(cx: &mut TestAppContext) {
    let (_t, dir, wb, cx) = workbench_with(
        true,
        |dir| {
            // `a.txt` differs on `feature`, and has uncommitted changes
            // here: git refuses to overwrite them.
            git(dir, &["checkout", "-q", "feature"]);
            std::fs::write(dir.join("a.txt"), "feature a\n").unwrap();
            git(dir, &["commit", "-q", "-am", "a on feature"]);
            git(dir, &["checkout", "-q", "main"]);
            std::fs::write(dir.join("a.txt"), "local change\n").unwrap();
        },
        cx,
    );
    pick(&wb, "feature", cx);
    assert!(cx.has_pending_prompt(), "git's refusal is shown");
    let (title, detail) = cx.pending_prompt().expect("a prompt");
    assert_eq!(title, "Could not switch branches");
    assert!(detail.contains("would be overwritten"), "{detail}");
    assert_eq!(detail, detail.trim(), "the detail is trimmed");
    cx.simulate_prompt_answer("OK");
    settle(cx);
    assert!(!cx.has_pending_prompt());
    assert_eq!(head(&dir), "main");
    assert!(!cx.update(|_, cx| wb.read(cx).is_switching_branch()));
    assert_eq!(
        std::fs::read_to_string(dir.join("a.txt")).unwrap(),
        "local change\n"
    );
}

fn close_trust_modal(wb: &Entity<Workbench>, cx: &mut VisualTestContext) {
    if cx.update(|_, cx| wb.read(cx).trust_modal().is_some()) {
        cx.simulate_keystrokes("escape");
        settle(cx);
    }
    assert!(cx.update(|_, cx| wb.read(cx).trust_modal().is_none()));
}

#[gpui::test]
fn restricted_click_opens_trust_modal_and_runs_no_git(cx: &mut TestAppContext) {
    let (_t, _dir, wb, cx) = workbench_with(false, |_| {}, cx);
    assert_eq!(
        cx.update(|_, cx| wb.read(cx).git().read(cx).state()),
        GitState::Restricted
    );
    close_trust_modal(&wb, cx);
    click_button(cx);
    assert!(cx.update(|_, cx| wb.read(cx).trust_modal().is_some()));
    assert!(picker_rows(&wb, cx).is_none());
    close_trust_modal(&wb, cx);
    run(&wb, "git.checkout", cx);
    assert!(cx.update(|_, cx| wb.read(cx).trust_modal().is_some()));
    assert!(picker_rows(&wb, cx).is_none());
    assert_eq!(switches(&wb, cx), 0);
}

#[gpui::test]
fn gate_messages(cx: &mut TestAppContext) {
    let (_tmp, dir) = repo();
    let (wb, cx) = open_workbench(&dir, Some(r#"{ "git.enabled": false }"#), true, cx);
    assert_eq!(
        cx.update(|_, cx| wb.read(cx).git().read(cx).state()),
        GitState::Off
    );
    click_button(cx);
    assert_eq!(message(cx).as_deref(), Some("Git is off"));
    assert!(picker_rows(&wb, cx).is_none());
    run(&wb, "git.checkout", cx);
    assert_eq!(message(cx).as_deref(), Some("Git is off"));
    assert!(picker_rows(&wb, cx).is_none());
}

#[gpui::test]
fn gate_message_without_repo(cx: &mut TestAppContext) {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().canonicalize().unwrap();
    std::fs::write(dir.join("a.txt"), "a\n").unwrap();
    let (wb, cx) = open_workbench(&dir, None, true, cx);
    assert_eq!(
        cx.update(|_, cx| wb.read(cx).git().read(cx).state()),
        GitState::NoRepo
    );
    draw(cx);
    assert!(cx.debug_bounds(BUTTON).is_none(), "no branch is shown");
    run(&wb, "git.checkout", cx);
    assert_eq!(message(cx).as_deref(), Some("Git is not available"));
    assert!(picker_rows(&wb, cx).is_none());
}

#[gpui::test]
fn branch_click_does_not_reach_title_bar(cx: &mut TestAppContext) {
    let (_t, _dir, wb, cx) = workbench(cx);
    draw(cx);
    let at = cx.debug_bounds(BUTTON).unwrap().center();
    // A press, a 2 px move and a release: a click, not a window drag (the
    // test platform panics if a window move starts).
    cx.simulate_mouse_down(at, MouseButton::Left, Modifiers::none());
    assert!(!cx.update(|_, cx| wb.read(cx).title_bar_pressed()));
    let moved = at + point(px(2.), px(0.));
    cx.simulate_mouse_move(moved, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(moved, MouseButton::Left, Modifiers::none());
    settle(cx);
    assert!(!cx.update(|_, cx| wb.read(cx).title_bar_pressed()));
    assert!(
        picker_rows(&wb, cx).is_some(),
        "the click opened the picker"
    );

    // A double-click does not zoom the window (the test platform panics
    // on zoom). Its first press closes the picker and that click does not
    // open it again; the second click opens it.
    for (click_count, shown) in [(1, false), (2, true)] {
        cx.simulate_event(MouseDownEvent {
            position: at,
            button: MouseButton::Left,
            modifiers: Modifiers::none(),
            click_count,
            first_mouse: false,
        });
        cx.simulate_event(MouseUpEvent {
            position: at,
            button: MouseButton::Left,
            modifiers: Modifiers::none(),
            click_count,
        });
        settle(cx);
        assert_eq!(picker_rows(&wb, cx).is_some(), shown, "click {click_count}");
    }
    assert!(!cx.update(|_, cx| wb.read(cx).title_bar_pressed()));
}

#[gpui::test]
fn second_switch_ignored_while_running(cx: &mut TestAppContext) {
    let (_t, dir, wb, cx) = workbench(cx);
    click_button(cx);
    let picker = cx
        .update(|_, cx| wb.read(cx).branch_picker().cloned())
        .expect("the picker is shown");
    // Neither the first switch nor anything else runs before parking.
    picker.update(cx, |_, cx| {
        cx.emit(BranchPickerEvent::Checkout(SwitchTarget::Local(
            "feature".into(),
        )))
    });
    assert!(cx.update(|_, cx| wb.read(cx).is_switching_branch()));
    draw(cx);
    assert!(
        cx.debug_bounds("title-bar-branch-switching").is_some(),
        "the title says Switching…"
    );
    cx.update(|window, cx| {
        wb.update(cx, |wb, cx| {
            wb.switch_branch(SwitchTarget::Create("other".into()), window, cx);
            wb.show_checkout_picker(CheckoutAnchor::Center, window, cx);
        })
    });
    assert!(picker_rows(&wb, cx).is_none(), "no picker while switching");
    assert_eq!(switches(&wb, cx), 1);
    settle(cx);
    assert_eq!(switches(&wb, cx), 1);
    assert_eq!(head(&dir), "feature");
    assert!(git(&dir, &["branch", "--list", "other"]).trim().is_empty());
    assert!(!cx.update(|_, cx| wb.read(cx).is_switching_branch()));
    draw(cx);
    assert!(cx.debug_bounds("title-bar-branch-switching").is_none());
}

#[gpui::test]
fn checkout_command_registered(cx: &mut TestAppContext) {
    let (_t, _dir, wb, cx) = workbench(cx);
    let title = cx.update(|_, cx| {
        wb.read(cx)
            .commands()
            .all()
            .find(|(id, _)| *id == "git.checkout")
            .map(|(_, title)| title.to_string())
    });
    assert_eq!(title.as_deref(), Some("Git: Checkout to…"));
}
