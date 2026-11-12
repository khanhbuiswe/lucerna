//! The Welcome screen: the recent list it loads, the rows it draws, and
//! the events its buttons and keys emit. It runs standalone, as the app's
//! router hosts it.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use gpui::{Entity, Focusable as _, Modifiers, TestAppContext, VisualTestContext, WindowHandle};
use lucerna_settings::{Keymap, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::{IconThemeRegistry, ThemeRegistry};
use lucerna_workbench::app::{self, Launch};
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::process::{SpawnRequest, Spawner};
use lucerna_workbench::recent::{RecentEntry, RecentKind, RecentStore, load, recent_path};
use lucerna_workbench::{AppState, OpenMode, WelcomeEvent, WelcomeView, Workbench};

fn init(user_keymap: Option<&str>, data_dir: Option<PathBuf>, cx: &mut TestAppContext) {
    let (keymap, error) = Keymap::load(Keymap::default_bindings(), user_keymap);
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
    cx.set_global(RecentStore { data_dir });
}

/// A temporary data dir whose `recent.json` lists `folders` (newest
/// first), each created under the same temporary dir unless `missing`.
struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new(folders: &[&str], missing: &[&str]) -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let entries: Vec<serde_json::Value> = folders
            .iter()
            .map(|name| {
                let path = dir.path().join(name);
                if !missing.contains(name) {
                    std::fs::create_dir_all(&path).unwrap();
                }
                serde_json::json!({ "path": path, "kind": "folder" })
            })
            .collect();
        let file = serde_json::json!({ "version": 1, "entries": entries });
        std::fs::write(recent_path(dir.path()), file.to_string()).unwrap();
        Fixture { dir }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn data_dir(&self) -> PathBuf {
        self.dir.path().to_path_buf()
    }

    fn stored(&self) -> Vec<PathBuf> {
        load(&recent_path(self.dir.path()))
            .entries
            .into_iter()
            .map(|e| e.path)
            .collect()
    }
}

type Events = Arc<Mutex<Vec<WelcomeEvent>>>;

fn open(cx: &mut TestAppContext) -> (Entity<WelcomeView>, Events, &mut VisualTestContext) {
    let (view, cx) = cx.add_window_view(WelcomeView::new);
    let events: Events = Arc::default();
    let sink = events.clone();
    cx.update(|_, cx| {
        cx.subscribe(&view, move |_, event: &WelcomeEvent, _| {
            sink.lock().unwrap().push(event.clone());
        })
        .detach();
    });
    draw(cx);
    (view, events, cx)
}

fn open_with<'a>(
    fixture: &Fixture,
    user_keymap: Option<&str>,
    cx: &'a mut TestAppContext,
) -> (Entity<WelcomeView>, Events, &'a mut VisualTestContext) {
    init(user_keymap, Some(fixture.data_dir()), cx);
    open(cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

fn settle(cx: &mut VisualTestContext) {
    draw(cx);
    cx.run_until_parked();
    draw(cx);
}

fn drawn(selector: &'static str, cx: &mut VisualTestContext) -> bool {
    cx.debug_bounds(selector).is_some()
}

fn center(selector: &'static str, cx: &mut VisualTestContext) -> gpui::Point<gpui::Pixels> {
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"))
        .center()
}

fn click(selector: &'static str, modifiers: Modifiers, cx: &mut VisualTestContext) {
    let at = center(selector, cx);
    cx.simulate_click(at, modifiers);
    draw(cx);
}

fn title(view: &Entity<WelcomeView>, cx: &mut VisualTestContext) -> String {
    cx.update(|_, cx| view.read(cx).title().to_string())
}

fn recent(view: &Entity<WelcomeView>, cx: &mut VisualTestContext) -> Vec<PathBuf> {
    cx.update(|_, cx| {
        view.read(cx)
            .recent()
            .expect("the list is loaded")
            .iter()
            .map(|e| e.path.clone())
            .collect()
    })
}

fn open_event(path: &Path, new_window: bool) -> WelcomeEvent {
    WelcomeEvent::Open {
        path: path.to_path_buf(),
        new_window,
    }
}

const SEVEN: &[&str] = &["a", "b", "c", "d", "e", "f", "g"];

#[gpui::test]
fn shows_five_recent_and_more(cx: &mut TestAppContext) {
    let fixture = Fixture::new(SEVEN, &[]);
    let (view, _, cx) = open_with(&fixture, None, cx);
    for selector in [
        "welcome-recent-0",
        "welcome-recent-1",
        "welcome-recent-2",
        "welcome-recent-3",
        "welcome-recent-4",
        "welcome-more",
    ] {
        assert!(drawn(selector, cx), "{selector}");
    }
    assert!(!drawn("welcome-recent-5", cx));
    assert_eq!(title(&view, cx), "Welcome back to Lucerna");
}

#[gpui::test]
fn hides_recent_section_when_empty(cx: &mut TestAppContext) {
    let fixture = Fixture::new(&[], &[]);
    let (view, _, cx) = open_with(&fixture, None, cx);
    assert!(!drawn("welcome-recent-0", cx));
    assert!(!drawn("welcome-more", cx));
    assert_eq!(title(&view, cx), "Welcome to Lucerna");
    assert!(drawn("welcome-title", cx));
}

#[gpui::test]
fn skips_missing_folders(cx: &mut TestAppContext) {
    let fixture = Fixture::new(&["a", "gone", "b"], &["gone"]);
    let (view, _, cx) = open_with(&fixture, None, cx);
    assert_eq!(
        recent(&view, cx),
        vec![fixture.path("a"), fixture.path("b")]
    );
    assert!(drawn("welcome-recent-1", cx));
    assert!(!drawn("welcome-recent-2", cx));
}

#[gpui::test]
fn remove_button_drops_entry(cx: &mut TestAppContext) {
    let fixture = Fixture::new(&["a", "b"], &[]);
    let (view, events, cx) = open_with(&fixture, None, cx);
    let at = center("welcome-recent-0", cx);
    cx.simulate_mouse_move(at, None, Modifiers::none());
    draw(cx);
    click("welcome-recent-remove-0", Modifiers::none(), cx);
    assert_eq!(recent(&view, cx), vec![fixture.path("b")]);
    assert_eq!(fixture.stored(), vec![fixture.path("b")]);
    assert!(events.lock().unwrap().is_empty());
}

#[gpui::test]
fn click_opens_and_cmd_click_opens_new_window(cx: &mut TestAppContext) {
    let fixture = Fixture::new(&["a", "b"], &[]);
    let (_, events, cx) = open_with(&fixture, None, cx);
    click("welcome-recent-1", Modifiers::none(), cx);
    let secondary = cx.update(|_, cx| Modifiers::secondary_key(cx));
    click("welcome-recent-1", secondary, cx);
    assert_eq!(
        *events.lock().unwrap(),
        vec![
            open_event(&fixture.path("b"), false),
            open_event(&fixture.path("b"), true),
        ]
    );
}

#[gpui::test]
fn cmd_1_opens_first_recent(cx: &mut TestAppContext) {
    let fixture = Fixture::new(&["a", "b"], &[]);
    let (_, events, cx) = open_with(&fixture, None, cx);
    if cfg!(target_os = "macos") {
        cx.simulate_keystrokes("cmd-1");
    } else {
        cx.simulate_keystrokes("ctrl-1");
    }
    draw(cx);
    assert_eq!(
        *events.lock().unwrap(),
        vec![open_event(&fixture.path("a"), false)]
    );
}

#[gpui::test]
fn arrows_move_focus_and_enter_presses(cx: &mut TestAppContext) {
    let fixture = Fixture::new(&["a", "b"], &[]);
    let (view, events, cx) = open_with(&fixture, None, cx);
    // Open… (or Open File… and Open Folder…), Open Recent…, then the rows.
    let first_row = if cfg!(target_os = "macos") { 2 } else { 3 };
    assert_eq!(cx.update(|_, cx| view.read(cx).focused_index()), 0);
    for _ in 0..first_row {
        cx.simulate_keystrokes("down");
    }
    cx.simulate_keystrokes("tab");
    assert_eq!(
        cx.update(|_, cx| view.read(cx).focused_index()),
        first_row + 1
    );
    cx.simulate_keystrokes("shift-tab up");
    cx.simulate_keystrokes("down enter");
    draw(cx);
    assert_eq!(
        *events.lock().unwrap(),
        vec![open_event(&fixture.path("a"), false)]
    );
}

#[gpui::test]
fn vanished_folder_shows_warning(cx: &mut TestAppContext) {
    let fixture = Fixture::new(&["a", "b"], &[]);
    let (view, events, cx) = open_with(&fixture, None, cx);
    std::fs::remove_dir(fixture.path("a")).unwrap();
    click("welcome-recent-0", Modifiers::none(), cx);
    assert!(drawn("welcome-warning", cx));
    assert!(
        cx.update(|_, cx| view.read(cx).warning().map(str::to_string))
            .is_some_and(|w| w.ends_with("a no longer exists")),
    );
    assert!(events.lock().unwrap().is_empty());
    assert_eq!(recent(&view, cx), vec![fixture.path("b")]);
    assert_eq!(fixture.stored(), vec![fixture.path("b")]);
}

#[gpui::test]
fn ctrl_r_opens_picker_and_escape_closes(cx: &mut TestAppContext) {
    let fixture = Fixture::new(&["a", "b"], &[]);
    let (view, _, cx) = open_with(
        &fixture,
        Some(r#"[{"key":"ctrl+r","command":"workbench.action.openRecent"}]"#),
        cx,
    );
    cx.simulate_keystrokes("ctrl-r");
    draw(cx);
    assert!(cx.update(|_, cx| view.read(cx).recent_picker().is_some()));
    cx.simulate_keystrokes("escape");
    draw(cx);
    assert!(cx.update(|_, cx| view.read(cx).recent_picker().is_none()));
}

#[gpui::test]
fn more_opens_the_picker_and_its_open_event_opens(cx: &mut TestAppContext) {
    let fixture = Fixture::new(SEVEN, &[]);
    let (view, events, cx) = open_with(&fixture, None, cx);
    click("welcome-more", Modifiers::none(), cx);
    let picker = cx
        .update(|_, cx| view.read(cx).recent_picker().cloned())
        .expect("the picker is open");
    cx.update(|_, cx| picker.update(cx, |p, cx| p.select_next(6, cx)));
    cx.update(|_, cx| picker.update(cx, |p, cx| p.confirm(true, cx)));
    draw(cx);
    assert_eq!(
        *events.lock().unwrap(),
        vec![open_event(&fixture.path("g"), true)]
    );
    assert!(cx.update(|_, cx| view.read(cx).recent_picker().is_none()));
}

#[gpui::test]
fn picker_remove_drops_entry(cx: &mut TestAppContext) {
    let fixture = Fixture::new(&["a", "b"], &[]);
    let (view, _, cx) = open_with(&fixture, None, cx);
    cx.update(|window, cx| view.update(cx, |v, cx| v.show_recent_picker(window, cx)));
    let picker = cx
        .update(|_, cx| view.read(cx).recent_picker().cloned())
        .expect("the picker is open");
    cx.update(|_, cx| picker.update(cx, |p, cx| p.remove_at(0, cx)));
    draw(cx);
    assert_eq!(recent(&view, cx), vec![fixture.path("b")]);
    assert_eq!(fixture.stored(), vec![fixture.path("b")]);
}

#[gpui::test]
fn open_button_prompts_and_cancel_does_nothing(cx: &mut TestAppContext) {
    let fixture = Fixture::new(&[], &[]);
    let (_, events, cx) = open_with(&fixture, None, cx);
    let button = if cfg!(target_os = "macos") {
        "welcome-open"
    } else {
        "welcome-open-folder"
    };
    click(button, Modifiers::none(), cx);
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|_| None);
    draw(cx);
    assert!(events.lock().unwrap().is_empty());

    click(button, Modifiers::none(), cx);
    let picked = fixture.data_dir();
    let answer = picked.clone();
    cx.simulate_path_prompt_response(move |options| {
        assert!(options.directories);
        Some(vec![answer])
    });
    draw(cx);
    assert_eq!(*events.lock().unwrap(), vec![open_event(&picked, false)]);
}

#[gpui::test]
fn quit_command_emits_quit(cx: &mut TestAppContext) {
    init(None, None, cx);
    let (view, events, cx) = open(cx);
    let handled = cx.update(|window, cx| {
        view.update(cx, |v, cx| {
            v.run_command("workbench.action.quit", window, cx)
        })
    });
    assert!(handled);
    let ignored = cx.update(|window, cx| {
        view.update(cx, |v, cx| {
            v.run_command("workbench.action.togglesidebarVisibility", window, cx)
        })
    });
    assert!(!ignored);
    assert_eq!(*events.lock().unwrap(), vec![WelcomeEvent::Quit]);
}

#[gpui::test]
fn no_data_dir_shows_get_started_only(cx: &mut TestAppContext) {
    init(None, None, cx);
    let (view, _, cx) = open(cx);
    if cfg!(target_os = "macos") {
        assert!(drawn("welcome-open", cx));
    } else {
        assert!(drawn("welcome-open-file", cx));
        assert!(drawn("welcome-open-folder", cx));
    }
    assert!(drawn("welcome-open-recent", cx));
    assert!(!drawn("welcome-recent-0", cx));
    assert_eq!(
        cx.update(|_, cx| view.read(cx).recent().map(<[_]>::len)),
        Some(0)
    );
}

type Spawns = Rc<RefCell<Vec<SpawnRequest>>>;

/// A Welcome window opened through the app's router, with a fake process
/// spawner; returns the window and its view.
fn routed_welcome(
    fixture: &Fixture,
    cx: &mut TestAppContext,
) -> (WindowHandle<WelcomeView>, Entity<WelcomeView>, Spawns) {
    init(None, Some(fixture.data_dir()), cx);
    let spawns: Spawns = Rc::default();
    let sink = spawns.clone();
    let handle = cx.update(|cx| {
        cx.set_global(Spawner(Rc::new(move |request: &SpawnRequest| {
            sink.borrow_mut().push(request.clone());
            Ok(())
        })));
        app::open_welcome_for_test(cx)
    });
    cx.run_until_parked();
    let view = handle.root(cx).expect("the root is the Welcome view");
    (handle, view, spawns)
}

fn emit(view: &Entity<WelcomeView>, events: Vec<WelcomeEvent>, cx: &mut TestAppContext) {
    cx.update(|cx| {
        view.update(cx, |_, cx| {
            for event in events {
                cx.emit(event);
            }
        })
    });
    cx.run_until_parked();
}

/// The only window's root, when it is a workbench.
fn only_workbench(cx: &mut TestAppContext) -> Option<Entity<Workbench>> {
    cx.update(|cx| {
        let windows = cx.windows();
        assert_eq!(windows.len(), 1, "one window");
        windows[0]
            .update(cx, |root, _, _| root.downcast::<Workbench>().ok())
            .unwrap()
    })
}

fn shown_root(cx: &mut TestAppContext) -> Option<PathBuf> {
    cx.update(app::router_shown_root_for_test)
}

fn stored_entries(fixture: &Fixture) -> Vec<RecentEntry> {
    load(&recent_path(fixture.dir.path())).entries
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap()
}

#[gpui::test]
fn opening_a_recent_folder_replaces_the_root(cx: &mut TestAppContext) {
    let fixture = Fixture::new(&["proj"], &[]);
    let (_, view, spawns) = routed_welcome(&fixture, cx);
    emit(&view, vec![open_event(&fixture.path("proj"), false)], cx);
    assert!(only_workbench(cx).is_some());
    assert_eq!(shown_root(cx), Some(canonical(&fixture.path("proj"))));
    assert!(spawns.borrow().is_empty());
}

#[gpui::test]
fn opening_records_the_folder(cx: &mut TestAppContext) {
    let fixture = Fixture::new(&["other", "proj"], &[]);
    let (_, view, _) = routed_welcome(&fixture, cx);
    emit(&view, vec![open_event(&fixture.path("proj"), false)], cx);
    assert_eq!(
        stored_entries(&fixture).first(),
        Some(&RecentEntry {
            path: canonical(&fixture.path("proj")),
            kind: RecentKind::Folder,
        })
    );
}

#[gpui::test]
fn new_window_spawns_instead(cx: &mut TestAppContext) {
    let fixture = Fixture::new(&["proj"], &[]);
    let (handle, view, spawns) = routed_welcome(&fixture, cx);
    emit(&view, vec![open_event(&fixture.path("proj"), true)], cx);
    assert_eq!(
        *spawns.borrow(),
        vec![SpawnRequest::Path(fixture.path("proj"))]
    );
    assert!(handle.root(cx).is_ok(), "the window still shows Welcome");
    assert_eq!(shown_root(cx), None);
}

#[gpui::test]
fn a_second_open_after_the_root_changed_is_ignored(cx: &mut TestAppContext) {
    // A double-click on a recent row emits Open twice.
    let fixture = Fixture::new(&["a", "b"], &[]);
    let (_, view, spawns) = routed_welcome(&fixture, cx);
    emit(
        &view,
        vec![
            open_event(&fixture.path("a"), false),
            open_event(&fixture.path("b"), false),
            open_event(&fixture.path("b"), true),
        ],
        cx,
    );
    assert!(only_workbench(cx).is_some());
    assert_eq!(shown_root(cx), Some(canonical(&fixture.path("a"))));
    assert!(spawns.borrow().is_empty());
}

#[gpui::test]
fn cli_launch_records_single_file_as_file(cx: &mut TestAppContext) {
    let fixture = Fixture::new(&[], &[]);
    init(None, Some(fixture.data_dir()), cx);
    let file = fixture.path("notes.txt");
    std::fs::write(&file, "notes\n").unwrap();
    let file = canonical(&file);
    let launch = Launch {
        root: file.parent().unwrap().to_path_buf(),
        file: Some(file.clone()),
        single_file: true,
        on_first_frame: None,
    };
    cx.update(|cx| app::open_launch_for_test(launch, cx));
    cx.run_until_parked();
    assert_eq!(
        stored_entries(&fixture).first(),
        Some(&RecentEntry {
            path: file,
            kind: RecentKind::File,
        })
    );
}

#[gpui::test]
fn quit_asks_the_replaced_window_about_unsaved_changes(cx: &mut TestAppContext) {
    let fixture = Fixture::new(&["proj"], &[]);
    std::fs::write(fixture.path("proj").join("file.txt"), "abc\n").unwrap();
    let (_, view, _) = routed_welcome(&fixture, cx);
    emit(&view, vec![open_event(&fixture.path("proj"), false)], cx);
    let wb = only_workbench(cx).expect("the root is a workbench");
    let window = cx.update(|cx| cx.windows()[0]);
    let cx = &mut VisualTestContext::from_window(window, cx);
    wb.update(cx, |wb, cx| {
        wb.open_path(Arc::from(Path::new("file.txt")), OpenMode::Pinned, cx)
    });
    settle(cx);
    let focus = cx.update(|_, cx| {
        wb.read(cx)
            .editor_groups()
            .read(cx)
            .active_body()
            .expect("the file is open")
            .downcast::<CodeView>()
            .expect("tab bodies are code views")
            .focus_handle(cx)
    });
    cx.update(|window, cx| window.focus(&focus, cx));
    cx.simulate_input("x");
    settle(cx);
    let docs = cx.update(|_, cx| wb.read(cx).documents().clone());
    assert!(docs.read_with(cx, |d, _| d.is_dirty(Path::new("file.txt"))));
    cx.update(|window, cx| {
        wb.update(cx, |wb, cx| {
            wb.run_command("workbench.action.quit", None, window, cx)
        })
    });
    settle(cx);
    assert!(
        cx.has_pending_prompt(),
        "quitting asks about the unsaved file"
    );
    cx.simulate_prompt_answer("Cancel");
    settle(cx);
}
