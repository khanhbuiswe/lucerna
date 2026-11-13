//! Opening from the workbench: the Open Recent picker (`ctrl-r`), Enter
//! replacing the window, `⌘Enter` opening a new one, Close Folder going
//! back to Welcome, Clear Recently Opened, and Open… on a picked path. A
//! fake process spawner records what a new `luc` would open.

use std::cell::RefCell;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{Entity, Focusable as _, TestAppContext, VisualTestContext};
use lucerna_settings::{Keymap, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::{IconThemeRegistry, ThemeRegistry};
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::process::{SpawnRequest, Spawner};
use lucerna_workbench::recent::{RecentStore, load, recent_path};
use lucerna_workbench::{AppState, OpenMode, WelcomeView, Workbench, status_message};

const CLEAR_MESSAGE: &str = "Do you want to clear all recently opened folders?";

type Spawns = Rc<RefCell<Vec<SpawnRequest>>>;

/// A temporary folder holding the data dir (`data/recent.json`), the
/// workbench's `root`, and the recent folders named.
struct Fixture {
    _dir: tempfile::TempDir,
    base: PathBuf,
}

impl Fixture {
    /// `recent` lists folders under the base, newest first; each is
    /// created.
    fn new(recent: &[&str]) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::create_dir_all(base.join("data")).unwrap();
        std::fs::create_dir_all(base.join("root")).unwrap();
        let entries: Vec<serde_json::Value> = recent
            .iter()
            .map(|name| {
                let path = base.join(name);
                std::fs::create_dir_all(&path).unwrap();
                serde_json::json!({ "path": path, "kind": "folder" })
            })
            .collect();
        let file = serde_json::json!({ "version": 1, "entries": entries });
        std::fs::write(recent_path(&base.join("data")), file.to_string()).unwrap();
        Fixture { _dir: dir, base }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.base.join(name)
    }

    fn stored(&self) -> Vec<PathBuf> {
        load(&recent_path(&self.base.join("data")))
            .entries
            .into_iter()
            .map(|e| e.path)
            .collect()
    }
}

/// Sets the app state, the recent store on the fixture's data dir, and a
/// spawner that records requests (and fails them when `fail`).
fn init(fixture: &Fixture, fail: bool, cx: &mut TestAppContext) -> Spawns {
    let (keymap, error) = Keymap::load(Keymap::default_bindings(), None);
    assert!(error.is_none(), "keymap error: {error:?}");
    let themes = ThemeRegistry::builtin();
    let theme = themes.get_or_default("One Dark");
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
    cx.set_global(RecentStore {
        data_dir: Some(fixture.path("data")),
    });
    let spawns: Spawns = Rc::default();
    let sink = spawns.clone();
    cx.set_global(Spawner(Rc::new(move |request: &SpawnRequest| {
        sink.borrow_mut().push(request.clone());
        if fail {
            Err(io::Error::other("no such program"))
        } else {
            Ok(())
        }
    })));
    spawns
}

fn workbench<'a>(
    fixture: &Fixture,
    cx: &'a mut TestAppContext,
) -> (Entity<Workbench>, &'a mut VisualTestContext) {
    let root = fixture.path("root");
    let (wb, cx) = cx.add_window_view(|window, cx| Workbench::new(root, window, cx));
    settle(cx);
    (wb, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

fn settle(cx: &mut VisualTestContext) {
    draw(cx);
    cx.run_until_parked();
    draw(cx);
}

fn keys(keystrokes: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keystrokes);
    settle(cx);
}

/// Keys that may close the window: no drawing afterwards.
fn final_keys(keystrokes: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes(keystrokes);
    cx.run_until_parked();
}

fn window_count(cx: &mut VisualTestContext) -> usize {
    cx.windows().len()
}

fn picker_rows(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Vec<String> {
    cx.update(|_, cx| {
        wb.read(cx)
            .recent_picker()
            .expect("the recent picker is shown")
            .read(cx)
            .row_texts()
    })
}

fn run(wb: &Entity<Workbench>, command: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| wb.update(cx, |wb, cx| wb.run_command(command, None, window, cx)));
    settle(cx);
}

fn status(cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| status_message::current(cx))
        .map(|s| s.to_string())
        .unwrap_or_default()
}

/// Opens `name` in the root, focuses it and types a character.
fn dirty_file(wb: &Entity<Workbench>, fixture: &Fixture, cx: &mut VisualTestContext) {
    std::fs::write(fixture.path("root").join("a.txt"), "abc\n").unwrap();
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new("a.txt")), OpenMode::Pinned, cx)
    });
    settle(cx);
    let view = cx.update(|_, cx| {
        wb.read(cx)
            .editor_groups()
            .read(cx)
            .active_body()
            .expect("the active tab has a body")
            .downcast::<CodeView>()
            .expect("a code view")
    });
    cx.update(|window, cx| {
        let focus = view.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    });
    settle(cx);
    cx.simulate_input("x");
    settle(cx);
}

#[gpui::test]
fn ctrl_r_shows_picker_without_current_folder(cx: &mut TestAppContext) {
    let fixture = Fixture::new(&["root", "other"]);
    init(&fixture, false, cx);
    let (wb, cx) = workbench(&fixture, cx);
    keys("ctrl-r", cx);
    let rows = picker_rows(&wb, cx);
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(rows[0].starts_with("other "), "{rows:?}");
}

#[gpui::test]
fn ctrl_r_again_moves_selection(cx: &mut TestAppContext) {
    let fixture = Fixture::new(&["other", "third"]);
    init(&fixture, false, cx);
    let (wb, cx) = workbench(&fixture, cx);
    keys("ctrl-r", cx);
    keys("ctrl-r", cx);
    let selected = cx.update(|_, cx| {
        wb.read(cx)
            .recent_picker()
            .expect("the picker is still shown")
            .read(cx)
            .selected_index()
    });
    assert_eq!(selected, 1);
    keys("ctrl-shift-r", cx);
    let selected = cx.update(|_, cx| {
        wb.read(cx)
            .recent_picker()
            .unwrap()
            .read(cx)
            .selected_index()
    });
    assert_eq!(selected, 0);
}

#[gpui::test]
fn escape_closes_the_picker(cx: &mut TestAppContext) {
    let fixture = Fixture::new(&["other"]);
    init(&fixture, false, cx);
    let (wb, cx) = workbench(&fixture, cx);
    keys("ctrl-r", cx);
    keys("escape", cx);
    assert!(cx.update(|_, cx| wb.read(cx).recent_picker().is_none()));
}

#[gpui::test]
fn enter_replaces_the_window(cx: &mut TestAppContext) {
    let fixture = Fixture::new(&["other"]);
    let spawns = init(&fixture, false, cx);
    let (_wb, cx) = workbench(&fixture, cx);
    keys("ctrl-r", cx);
    final_keys("enter", cx);
    assert_eq!(
        *spawns.borrow(),
        vec![SpawnRequest::Path(fixture.path("other"))]
    );
    assert_eq!(window_count(cx), 0, "the window closed");
}

#[gpui::test]
fn cmd_enter_spawns_and_keeps_the_window(cx: &mut TestAppContext) {
    let fixture = Fixture::new(&["other"]);
    let spawns = init(&fixture, false, cx);
    let (wb, cx) = workbench(&fixture, cx);
    keys("ctrl-r", cx);
    keys("cmd-enter", cx);
    assert_eq!(
        *spawns.borrow(),
        vec![SpawnRequest::Path(fixture.path("other"))]
    );
    assert_eq!(window_count(cx), 1);
    assert!(cx.update(|_, cx| wb.read(cx).recent_picker().is_none()));
}

#[gpui::test]
fn cancel_on_unsaved_prompt_keeps_everything(cx: &mut TestAppContext) {
    let fixture = Fixture::new(&["other"]);
    let spawns = init(&fixture, false, cx);
    let (wb, cx) = workbench(&fixture, cx);
    dirty_file(&wb, &fixture, cx);
    keys("ctrl-r", cx);
    keys("enter", cx);
    assert!(cx.has_pending_prompt(), "asks to save");
    cx.simulate_prompt_answer("Cancel");
    settle(cx);
    assert!(spawns.borrow().is_empty());
    assert_eq!(window_count(cx), 1);
}

#[gpui::test]
fn failed_spawn_keeps_window_and_reports(cx: &mut TestAppContext) {
    let fixture = Fixture::new(&["other"]);
    let spawns = init(&fixture, true, cx);
    let (_wb, cx) = workbench(&fixture, cx);
    keys("ctrl-r", cx);
    keys("enter", cx);
    assert_eq!(spawns.borrow().len(), 1);
    assert_eq!(window_count(cx), 1);
    let status = status(cx);
    assert!(status.starts_with("Could not open "), "{status:?}");
    assert!(status.contains("other"), "{status:?}");
}

#[gpui::test]
fn close_folder_spawns_welcome(cx: &mut TestAppContext) {
    let fixture = Fixture::new(&[]);
    let spawns = init(&fixture, false, cx);
    let (_wb, cx) = workbench(&fixture, cx);
    final_keys("cmd-k f", cx);
    assert_eq!(*spawns.borrow(), vec![SpawnRequest::Welcome]);
    assert_eq!(window_count(cx), 0, "the window closed");
}

#[gpui::test]
fn vanished_folder_in_picker_reports(cx: &mut TestAppContext) {
    let fixture = Fixture::new(&["other", "third"]);
    let spawns = init(&fixture, false, cx);
    let (_wb, cx) = workbench(&fixture, cx);
    keys("ctrl-r", cx);
    std::fs::remove_dir(fixture.path("other")).unwrap();
    keys("enter", cx);
    assert!(spawns.borrow().is_empty());
    assert_eq!(window_count(cx), 1);
    let status = status(cx);
    assert!(status.ends_with("other no longer exists"), "{status:?}");
    assert_eq!(fixture.stored(), vec![fixture.path("third")]);
}

#[gpui::test]
fn clear_recent_asks_then_clears(cx: &mut TestAppContext) {
    let fixture = Fixture::new(&["other", "third"]);
    init(&fixture, false, cx);
    let (wb, cx) = workbench(&fixture, cx);
    run(&wb, "workbench.action.clearRecentFiles", cx);
    assert_eq!(
        cx.pending_prompt().map(|(message, _)| message),
        Some(CLEAR_MESSAGE.to_string())
    );
    cx.simulate_prompt_answer("Cancel");
    settle(cx);
    assert_eq!(fixture.stored().len(), 2);
    run(&wb, "workbench.action.clearRecentFiles", cx);
    cx.simulate_prompt_answer("Clear");
    settle(cx);
    assert!(fixture.stored().is_empty());
}

#[gpui::test]
fn open_file_outside_root_opens_a_tab(cx: &mut TestAppContext) {
    let fixture = Fixture::new(&[]);
    let spawns = init(&fixture, false, cx);
    let (wb, cx) = workbench(&fixture, cx);
    let file = fixture.path("outside.txt");
    std::fs::write(&file, "hello\n").unwrap();
    cx.update(|window, cx| {
        wb.update(cx, |wb, cx| {
            wb.open_picked_paths(vec![file.clone()], window, cx)
        })
    });
    settle(cx);
    let active = cx.update(|_, cx| {
        wb.read(cx)
            .editor_groups()
            .read(cx)
            .model()
            .active_tab()
            .and_then(|tab| tab.path().map(|p| p.to_path_buf()))
    });
    assert_eq!(active, Some(file));
    assert!(spawns.borrow().is_empty());
    assert_eq!(window_count(cx), 1);
}

#[gpui::test]
fn open_folder_replaces_the_window(cx: &mut TestAppContext) {
    let fixture = Fixture::new(&["other"]);
    let spawns = init(&fixture, false, cx);
    let (wb, cx) = workbench(&fixture, cx);
    let folder = fixture.path("other");
    cx.update(|window, cx| {
        wb.update(cx, |wb, cx| {
            wb.open_picked_paths(vec![folder.clone()], window, cx)
        })
    });
    cx.run_until_parked();
    assert_eq!(*spawns.borrow(), vec![SpawnRequest::Path(folder)]);
    assert_eq!(window_count(cx), 0, "the window closed");
}

#[gpui::test]
fn welcome_clear_recent_hides_the_list(cx: &mut TestAppContext) {
    let fixture = Fixture::new(&["other"]);
    init(&fixture, false, cx);
    let (view, cx) = cx.add_window_view(WelcomeView::new);
    settle(cx);
    assert_eq!(
        cx.update(|_, cx| view.read(cx).recent().map(<[_]>::len)),
        Some(1)
    );
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.run_command("workbench.action.clearRecentFiles", window, cx)
        })
    });
    settle(cx);
    assert_eq!(
        cx.pending_prompt().map(|(message, _)| message),
        Some(CLEAR_MESSAGE.to_string())
    );
    cx.simulate_prompt_answer("Clear");
    settle(cx);
    assert!(fixture.stored().is_empty());
    assert_eq!(
        cx.update(|_, cx| view.read(cx).recent().map(<[_]>::len)),
        Some(0)
    );
}

#[gpui::test]
fn welcome_ctrl_r_in_picker_moves_selection(cx: &mut TestAppContext) {
    let fixture = Fixture::new(&["other", "third"]);
    init(&fixture, false, cx);
    let (view, cx) = cx.add_window_view(WelcomeView::new);
    settle(cx);
    keys("ctrl-r", cx);
    keys("ctrl-r", cx);
    keys("ctrl-shift-r", cx);
    keys("ctrl-shift-r", cx);
    let selected = cx.update(|_, cx| {
        view.read(cx)
            .recent_picker()
            .expect("the picker is shown")
            .read(cx)
            .selected_index()
    });
    // Wrapped from the first row to the last.
    assert_eq!(selected, 1);
}
