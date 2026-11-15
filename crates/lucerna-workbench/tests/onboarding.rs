//! The Get Started page: what it draws from the settings, what its
//! controls and keys write into the user `settings.json`, and the events
//! it emits. It runs standalone, as the app's router hosts it, with the
//! user directory in a temp dir, and as the Get Started tab of a
//! workbench.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use gpui::Focusable as _;
use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};
use lucerna_settings::{Keymap, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::{IconThemeRegistry, ThemeRegistry};
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{
    AppState, ConfigPaths, ConfigState, Host, OnboardingEvent, OnboardingPage, OpenMode, TabItem,
    Workbench, reload_config, watch_config,
};
use tempfile::TempDir;

const TEMPLATE: &str = "// Lucerna user settings\n{\n}\n";

type Events = Arc<Mutex<Vec<OnboardingEvent>>>;

struct Page<'a> {
    view: Entity<OnboardingPage>,
    events: Events,
    user: TempDir,
    cx: &'a mut VisualTestContext,
}

/// Sets the app state up with the default keymap and `settings` (none:
/// no file) as the user `settings.json` in a new temp dir, returned.
fn init(settings: Option<&str>, cx: &mut TestAppContext) -> TempDir {
    let user = tempfile::tempdir().unwrap();
    if let Some(text) = settings {
        std::fs::write(user.path().join("settings.json"), text).unwrap();
    }
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
    let paths = ConfigPaths::user_only_with_user_dir(user.path());
    cx.update(|cx| watch_config(paths, cx));
    user
}

/// Opens the page with `settings` (none: no file) as the user
/// `settings.json`.
fn open<'a>(host: Host, settings: Option<&str>, cx: &'a mut TestAppContext) -> Page<'a> {
    let user = init(settings, cx);
    let (view, cx) = cx.add_window_view(move |window, cx| OnboardingPage::new(host, window, cx));
    let events: Events = Arc::default();
    let sink = events.clone();
    cx.update(|_, cx| {
        cx.subscribe(&view, move |_, event: &OnboardingEvent, _| {
            sink.lock().unwrap().push(event.clone());
        })
        .detach();
    });
    draw(cx);
    Page {
        view,
        events,
        user,
        cx,
    }
}

impl Page<'_> {
    fn settings_path(&self) -> PathBuf {
        self.user.path().join("settings.json")
    }

    fn file(&self) -> String {
        std::fs::read_to_string(self.settings_path()).unwrap()
    }

    /// The user file, parsed (comments stripped).
    fn json(&self) -> serde_json::Value {
        let text: String = self
            .file()
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        serde_json::from_str(&text).unwrap()
    }

    fn click(&mut self, selector: &'static str) {
        let at = self
            .cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is drawn"))
            .center();
        self.cx.simulate_click(at, Modifiers::none());
        draw(self.cx);
    }

    fn keys(&mut self, keystrokes: &str) {
        self.cx.simulate_keystrokes(keystrokes);
        draw(self.cx);
    }

    fn drawn(&mut self, selector: &'static str) -> bool {
        self.cx.debug_bounds(selector).is_some()
    }

    fn drawn_font_size(&mut self) -> Option<u32> {
        let view = self.view.clone();
        self.cx.update(|_, cx| view.read(cx).drawn_font_size())
    }

    /// Whether the Vim switch is drawn on: its knob sits at the right.
    fn vim_switch_on(&mut self) -> bool {
        let switch = self.cx.debug_bounds("onboarding-vim").unwrap();
        let knob = self.cx.debug_bounds("onboarding-vim-knob").unwrap();
        knob.center().x > switch.center().x
    }

    fn theme_name(&mut self) -> String {
        self.cx
            .update(|_, cx| cx.global::<AppState>().theme.name.clone())
    }

    fn errors(&mut self) -> usize {
        self.cx
            .update(|_, cx| cx.global::<ConfigState>().errors().count())
    }

    fn events(&self) -> Vec<OnboardingEvent> {
        self.events.lock().unwrap().clone()
    }

    fn focus_ring_shown(&mut self) -> bool {
        let view = self.view.clone();
        self.cx.update(|_, cx| view.read(cx).focus_ring_shown())
    }
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

#[gpui::test]
fn draws_current_settings(cx: &mut TestAppContext) {
    let mut page = open(
        Host::Window,
        Some(r#"{"editor.fontSize": 18, "vim.enabled": true}"#),
        cx,
    );
    assert_eq!(page.drawn_font_size(), Some(18));
    assert!(page.vim_switch_on());
    for selector in [
        "onboarding-finish",
        "onboarding-theme-0",
        "onboarding-theme-1",
        "onboarding-theme-2",
        "onboarding-preview",
        "onboarding-font-size-minus",
        "onboarding-font-size-value",
        "onboarding-font-size-plus",
        "onboarding-open-settings",
    ] {
        assert!(page.drawn(selector), "{selector} is drawn");
    }
    // System by default: the preview splits.
    assert!(page.drawn("onboarding-preview-right"));
}

#[gpui::test]
fn title_bar_only_as_window_root(cx: &mut TestAppContext) {
    let mut page = open(Host::Window, Some(TEMPLATE), cx);
    assert!(page.drawn("title-bar-title"));
}

#[gpui::test]
fn no_title_bar_in_a_tab(cx: &mut TestAppContext) {
    let mut page = open(Host::Tab, Some(TEMPLATE), cx);
    assert!(!page.drawn("title-bar-title"));
    assert!(page.drawn("onboarding-finish"));
}

#[gpui::test]
fn dark_writes_autodetect_and_color_theme(cx: &mut TestAppContext) {
    let mut page = open(Host::Window, Some(TEMPLATE), cx);
    page.click("onboarding-theme-1");
    let file = page.file();
    assert!(file.starts_with("// Lucerna user settings\n"), "{file}");
    let json = page.json();
    assert_eq!(json["window.autoDetectColorScheme"], false);
    assert_eq!(json["workbench.colorTheme"], "Lucerna Dark");
    assert_eq!(page.theme_name(), "Lucerna Dark");
    // One theme: the preview does not split.
    assert!(!page.drawn("onboarding-preview-right"));
}

#[gpui::test]
fn light_writes_preferred_light(cx: &mut TestAppContext) {
    let mut page = open(Host::Window, Some(TEMPLATE), cx);
    page.click("onboarding-theme-0");
    assert_eq!(page.json()["workbench.colorTheme"], "Lucerna Light");
    assert_eq!(page.theme_name(), "Lucerna Light");
}

#[gpui::test]
fn system_writes_autodetect_true(cx: &mut TestAppContext) {
    let mut page = open(
        Host::Window,
        Some(r#"{"window.autoDetectColorScheme": false, "workbench.colorTheme": "Lucerna Dark"}"#),
        cx,
    );
    page.click("onboarding-theme-2");
    assert_eq!(page.json()["window.autoDetectColorScheme"], true);
}

#[gpui::test]
fn selected_theme_click_writes_nothing(cx: &mut TestAppContext) {
    let text = r#"{"window.autoDetectColorScheme": false, "workbench.colorTheme": "Lucerna Dark"}"#;
    let mut page = open(Host::Window, Some(text), cx);
    page.click("onboarding-theme-1");
    assert_eq!(page.file(), text);
}

#[gpui::test]
fn plus_three_times_writes_17(cx: &mut TestAppContext) {
    let mut page = open(Host::Window, Some(TEMPLATE), cx);
    assert_eq!(page.drawn_font_size(), Some(14));
    page.click("onboarding-font-size-plus");
    page.click("onboarding-font-size-plus");
    page.click("onboarding-font-size-plus");
    assert_eq!(page.json()["editor.fontSize"], 17);
    assert_eq!(page.drawn_font_size(), Some(17));
}

#[gpui::test]
fn minus_writes_13(cx: &mut TestAppContext) {
    let mut page = open(Host::Window, Some(TEMPLATE), cx);
    page.click("onboarding-font-size-minus");
    assert_eq!(page.json()["editor.fontSize"], 13);
}

#[gpui::test]
fn plus_at_100_writes_nothing(cx: &mut TestAppContext) {
    let text = r#"{"editor.fontSize": 100}"#;
    let mut page = open(Host::Window, Some(text), cx);
    page.click("onboarding-font-size-plus");
    assert_eq!(page.file(), text);
    // By key too: the click left focus on +.
    page.keys("enter");
    assert_eq!(page.file(), text);
}

#[gpui::test]
fn vim_toggle_writes_true(cx: &mut TestAppContext) {
    let mut page = open(Host::Window, Some(TEMPLATE), cx);
    assert!(!page.vim_switch_on());
    page.click("onboarding-vim");
    assert_eq!(page.json()["vim.enabled"], true);
    assert!(page.vim_switch_on());
}

#[gpui::test]
fn broken_settings_not_written(cx: &mut TestAppContext) {
    let mut page = open(Host::Window, Some(TEMPLATE), cx);
    assert_eq!(page.errors(), 0);
    // Broken on disk before the watcher reloads it.
    std::fs::write(page.settings_path(), "{ \"a\": ").unwrap();
    page.click("onboarding-theme-1");
    assert_eq!(page.file(), "{ \"a\": ");
    assert_eq!(
        page.errors(),
        1,
        "the failure shows as a configuration error"
    );
}

#[gpui::test]
fn broken_settings_warn_in_window(cx: &mut TestAppContext) {
    let mut page = open(Host::Window, Some(TEMPLATE), cx);
    std::fs::write(page.settings_path(), "{ \"a\": ").unwrap();
    page.click("onboarding-theme-1");
    assert!(page.drawn("onboarding-warning"));
    // Fixed: the next save clears the warning.
    std::fs::write(page.settings_path(), TEMPLATE).unwrap();
    page.click("onboarding-vim");
    assert_eq!(page.json()["vim.enabled"], true);
    assert!(!page.drawn("onboarding-warning"));
}

#[gpui::test]
fn broken_settings_no_warning_in_tab(cx: &mut TestAppContext) {
    // The workbench's status bar shows the error.
    let mut page = open(Host::Tab, Some(TEMPLATE), cx);
    std::fs::write(page.settings_path(), "{ \"a\": ").unwrap();
    page.click("onboarding-theme-1");
    assert!(!page.drawn("onboarding-warning"));
}

#[gpui::test]
fn reflects_hand_edit(cx: &mut TestAppContext) {
    let mut page = open(Host::Window, Some(TEMPLATE), cx);
    let path = page.settings_path();
    std::fs::write(&path, r#"{"editor.fontSize": 20, "vim.enabled": true}"#).unwrap();
    page.cx.update(|_, cx| reload_config(&[path], cx));
    draw(page.cx);
    assert_eq!(page.drawn_font_size(), Some(20));
    assert!(page.vim_switch_on());
}

#[gpui::test]
fn cmd_enter_emits_finish(cx: &mut TestAppContext) {
    let mut page = open(Host::Window, Some(TEMPLATE), cx);
    page.keys("cmd-enter");
    assert_eq!(page.events(), vec![OnboardingEvent::Finish]);
}

#[gpui::test]
fn finish_button_emits_finish(cx: &mut TestAppContext) {
    let mut page = open(Host::Tab, Some(TEMPLATE), cx);
    page.click("onboarding-finish");
    assert_eq!(page.events(), vec![OnboardingEvent::Finish]);
}

#[gpui::test]
fn focus_ring_only_for_keyboard(cx: &mut TestAppContext) {
    let mut page = open(Host::Window, Some(TEMPLATE), cx);
    assert!(!page.focus_ring_shown());
    page.keys("tab");
    assert!(page.focus_ring_shown());
    page.click("onboarding-vim");
    assert!(!page.focus_ring_shown());
    // A key brings it back.
    page.keys("up");
    assert!(page.focus_ring_shown());
}

#[gpui::test]
fn open_settings_in_window_creates_file(cx: &mut TestAppContext) {
    let mut page = open(Host::Window, None, cx);
    assert!(!page.settings_path().exists());
    page.click("onboarding-open-settings");
    assert_eq!(page.file(), TEMPLATE);
    assert_eq!(
        page.events(),
        vec![OnboardingEvent::OpenSettingsInNewWindow(
            page.settings_path()
        )]
    );
}

#[gpui::test]
fn open_settings_in_tab_asks_the_workbench(cx: &mut TestAppContext) {
    let mut page = open(Host::Tab, None, cx);
    page.click("onboarding-open-settings");
    assert_eq!(
        page.events(),
        vec![OnboardingEvent::OpenSettingsInWorkbench]
    );
    assert!(!page.settings_path().exists());
}

#[gpui::test]
fn arrows_in_theme_group_change_the_mode(cx: &mut TestAppContext) {
    // Focus starts on the selected theme button: System.
    let mut page = open(Host::Window, Some(TEMPLATE), cx);
    page.keys("left");
    assert_eq!(page.json()["workbench.colorTheme"], "Lucerna Dark");
    page.keys("left");
    assert_eq!(page.json()["workbench.colorTheme"], "Lucerna Light");
    page.keys("right right");
    assert_eq!(page.json()["window.autoDetectColorScheme"], true);
}

#[gpui::test]
fn space_presses_the_focused_control(cx: &mut TestAppContext) {
    // System, then −, +, Vim.
    let mut page = open(Host::Window, Some(TEMPLATE), cx);
    page.keys("tab tab tab space");
    assert_eq!(page.json()["vim.enabled"], true);
    // Up twice: − ; Enter.
    page.keys("up up enter");
    assert_eq!(page.json()["editor.fontSize"], 13);
}

#[gpui::test]
fn shift_tab_reaches_finish(cx: &mut TestAppContext) {
    // From System back three: Finish.
    let mut page = open(Host::Window, Some(TEMPLATE), cx);
    page.keys("shift-tab shift-tab shift-tab enter");
    assert_eq!(page.events(), vec![OnboardingEvent::Finish]);
}

// The Get Started tab of a workbench.

struct Bench<'a> {
    wb: Entity<Workbench>,
    user: TempDir,
    _root: TempDir,
    cx: &'a mut VisualTestContext,
}

/// A workbench on a folder holding `a.txt` ("one"), with `settings` as the
/// user `settings.json`.
fn bench<'a>(settings: Option<&str>, cx: &'a mut TestAppContext) -> Bench<'a> {
    let user = init(settings, cx);
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("a.txt"), "one").unwrap();
    let dir = root.path().to_path_buf();
    let (wb, cx) = cx.add_window_view(move |window, cx| Workbench::new(dir, window, cx));
    draw(cx);
    Bench {
        wb,
        user,
        _root: root,
        cx,
    }
}

impl Bench<'_> {
    fn run(&mut self, command: &str) {
        let wb = self.wb.clone();
        self.cx.update(|window, cx| {
            wb.update(cx, |wb, cx| {
                assert!(wb.run_command(command, None, window, cx), "{command}");
            })
        });
        draw(self.cx);
    }

    fn keys(&mut self, keystrokes: &str) {
        self.cx.simulate_keystrokes(keystrokes);
        draw(self.cx);
    }

    fn click(&mut self, selector: &'static str) {
        let at = self
            .cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is drawn"))
            .center();
        self.cx.simulate_click(at, Modifiers::none());
        draw(self.cx);
    }

    /// The titles of the Get Started tabs, in every group.
    fn onboarding_tabs(&mut self) -> Vec<String> {
        let wb = self.wb.clone();
        self.cx.update(|_, cx| {
            let groups = wb.read(cx).editor_groups().read(cx);
            groups
                .model()
                .groups()
                .iter()
                .flat_map(|g| &g.tabs)
                .filter(|t| t.item == TabItem::Onboarding)
                .map(|t| t.title())
                .collect()
        })
    }

    fn active_item(&mut self) -> Option<TabItem> {
        let wb = self.wb.clone();
        self.cx.update(|_, cx| {
            let groups = wb.read(cx).editor_groups().read(cx);
            groups.model().active_tab().map(|t| t.item.clone())
        })
    }
}

#[gpui::test]
fn command_opens_tab(cx: &mut TestAppContext) {
    let mut b = bench(Some(TEMPLATE), cx);
    b.run("lucerna.openOnboarding");
    b.run("lucerna.openOnboarding");
    assert_eq!(b.onboarding_tabs(), ["Get Started"]);
    assert_eq!(b.active_item(), Some(TabItem::Onboarding));
    assert!(b.cx.debug_bounds("onboarding-finish").is_some());
}

#[gpui::test]
fn cmd_enter_closes_tab(cx: &mut TestAppContext) {
    let mut b = bench(Some(TEMPLATE), cx);
    b.run("lucerna.openOnboarding");
    b.keys("cmd-enter");
    assert!(b.onboarding_tabs().is_empty());
}

#[gpui::test]
fn finish_button_closes_tab(cx: &mut TestAppContext) {
    let mut b = bench(Some(TEMPLATE), cx);
    b.run("lucerna.openOnboarding");
    b.click("onboarding-finish");
    assert!(b.onboarding_tabs().is_empty());
}

#[gpui::test]
fn cmd_enter_in_editor_still_inserts_line(cx: &mut TestAppContext) {
    let mut b = bench(Some(TEMPLATE), cx);
    b.run("lucerna.openOnboarding");
    let wb = b.wb.clone();
    wb.update(b.cx, |wb, cx| {
        wb.open_path(
            Arc::from(std::path::Path::new("a.txt")),
            OpenMode::Pinned,
            cx,
        )
    });
    draw(b.cx);
    let view = b.cx.update(|_, cx| {
        wb.read(cx)
            .editor_groups()
            .read(cx)
            .active_body()
            .expect("the file tab has a body")
            .downcast::<CodeView>()
            .expect("a code view")
    });
    b.cx.update(|window, cx| {
        let focus = view.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
    });
    draw(b.cx);
    b.keys("cmd-enter");
    let text = view.read_with(b.cx, |v, _| {
        let s = v.snapshot();
        s.text_for_range(0..s.len_bytes())
    });
    assert_eq!(text, "one\n");
    assert_eq!(b.onboarding_tabs(), ["Get Started"]);
}

#[gpui::test]
fn dark_from_tab_changes_theme(cx: &mut TestAppContext) {
    let light =
        r#"{"window.autoDetectColorScheme": false, "workbench.colorTheme": "Lucerna Light"}"#;
    let mut b = bench(Some(light), cx);
    let theme = |b: &mut Bench| {
        b.cx.update(|_, cx| cx.global::<AppState>().theme.name.clone())
    };
    assert_eq!(theme(&mut b), "Lucerna Light");
    b.run("lucerna.openOnboarding");
    b.click("onboarding-theme-1");
    let file = std::fs::read_to_string(b.user.path().join("settings.json")).unwrap();
    let json: serde_json::Value = serde_json::from_str(&file).unwrap();
    assert_eq!(json["workbench.colorTheme"], "Lucerna Dark");
    assert_eq!(theme(&mut b), "Lucerna Dark");
}

#[gpui::test]
fn open_settings_from_tab_opens_the_file_in_the_workbench(cx: &mut TestAppContext) {
    let mut b = bench(Some(TEMPLATE), cx);
    b.run("lucerna.openOnboarding");
    b.click("onboarding-open-settings");
    let settings = b.user.path().join("settings.json");
    match b.active_item() {
        Some(TabItem::File(path)) => assert!(
            path.ends_with("settings.json") && settings.exists(),
            "{path:?}"
        ),
        other => panic!("the settings file is active, not {other:?}"),
    }
}
