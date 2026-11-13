//! Saving and restoring the workspace layout: the state file's format and
//! failure handling, and a workbench that restores and saves it.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gpui::{Entity, TestAppContext, VisualTestContext, px};
use lucerna_settings::{Keymap, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::ThemeRegistry;
use lucerna_workbench::code_view::CodeView;
use lucerna_workbench::{
    AppState, EXTENSIONS_TAB_STATE, GroupState, GroupsState, OpenMode, SEARCH_TAB_STATE,
    SearchState, SidebarView, Workbench, WorkspaceState, load_state, save_state, state_path,
};
use tempfile::TempDir;

fn sample(paths: &[&Path]) -> WorkspaceState {
    WorkspaceState {
        version: 1,
        sidebar_visible: false,
        sidebar_width: 333.,
        sidebar_view: "search".into(),
        groups: GroupsState {
            groups: vec![
                GroupState {
                    tabs: vec![(paths[0].to_path_buf(), true)],
                    active: Some(0),
                },
                GroupState {
                    tabs: paths[1..]
                        .iter()
                        .map(|p| (p.to_path_buf(), false))
                        .collect(),
                    active: Some(0),
                },
            ],
            active_group: 1,
        },
        scroll: paths.iter().map(|p| (p.to_path_buf(), 7)).collect(),
        group_widths: vec![0.25, 0.75],
        search: Default::default(),
    }
}

#[test]
fn roundtrip() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("nested/state.json");
    let state = sample(&[Path::new("/a"), Path::new("/b")]);
    save_state(&path, &state).unwrap();
    assert_eq!(load_state(&path), Some(state));
    let leftovers: Vec<_> = fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(leftovers, ["state.json"], "no temp file stays behind");
}

#[test]
fn same_root_same_path_different_root_different_path() {
    let data = Path::new("/data");
    let (a, b) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    let path = state_path(data, a.path());
    assert_eq!(path, state_path(data, a.path()));
    // The canonical root counts: a `..` detour names the same folder.
    let detour = a.path().join("..").join(a.path().file_name().unwrap());
    assert_eq!(path, state_path(data, &detour));
    assert_ne!(path, state_path(data, b.path()));
    assert_eq!(path.parent(), Some(data.join("workspaces").as_path()));
    let name = path.file_name().unwrap().to_str().unwrap();
    let hex = name.strip_suffix(".json").unwrap();
    assert_eq!(hex.len(), 64);
    assert!(hex.bytes().all(|b| b.is_ascii_hexdigit()));
}

#[test]
fn missing_file_loads_none() {
    let dir = TempDir::new().unwrap();
    assert_eq!(load_state(&dir.path().join("none.json")), None);
}

#[test]
fn corrupt_file_renamed_bak_returns_none() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("state.json");
    let bak = dir.path().join("state.json.bak");
    fs::write(&bak, "older backup").unwrap();
    fs::write(&path, "{ not json").unwrap();
    assert_eq!(load_state(&path), None);
    assert!(!path.exists());
    assert_eq!(fs::read_to_string(&bak).unwrap(), "{ not json");

    // A version mismatch is set aside the same way.
    let mut state = sample(&[Path::new("/a"), Path::new("/b")]);
    state.version = 2;
    save_state(&path, &state).unwrap();
    let future = fs::read_to_string(&path).unwrap();
    assert_eq!(load_state(&path), None);
    assert!(!path.exists());
    assert_eq!(fs::read_to_string(&bak).unwrap(), future);
}

#[test]
fn missing_files_dropped_on_restore() {
    let dir = TempDir::new().unwrap();
    let (a, b, c) = (
        dir.path().join("a"),
        dir.path().join("b"),
        dir.path().join("c"),
    );
    for f in [&a, &c] {
        fs::write(f, "x").unwrap();
    }
    let mut state = sample(&[&a, &b, &c]);
    // The right group's active tab is `b`, which is gone.
    state.groups.groups[1].active = Some(0);
    let state = state.without_missing_files(dir.path());
    assert_eq!(
        state.groups.groups[1].tabs,
        vec![(c.clone(), false)],
        "only existing files remain"
    );
    assert_eq!(state.groups.groups[1].active, None);
    assert_eq!(state.scroll, HashMap::from([(a, 7), (c, 7)]));
}

#[test]
fn search_tab_survives_without_missing_files() {
    let dir = TempDir::new().unwrap();
    let a = dir.path().join("a");
    fs::write(&a, "x").unwrap();
    let mut state = sample(&[&a, &a]);
    state.groups.groups[1].tabs = vec![
        (PathBuf::from(SEARCH_TAB_STATE), true),
        (PathBuf::from(EXTENSIONS_TAB_STATE), true),
        (dir.path().join("gone"), true),
    ];
    state.groups.groups[1].active = Some(0);
    let state = state.without_missing_files(dir.path());
    assert_eq!(
        state.groups.groups[1].tabs,
        vec![
            (PathBuf::from(SEARCH_TAB_STATE), true),
            (PathBuf::from(EXTENSIONS_TAB_STATE), true),
        ],
        "the Search and Extensions tabs are not files"
    );
    assert_eq!(state.groups.groups[1].active, Some(0));
}

fn init(cx: &mut TestAppContext) {
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

fn lines(n: usize) -> String {
    (0..n).map(|i| format!("line {i}\n")).collect()
}

fn workbench<'a>(
    root: &Path,
    data: &Path,
    cx: &'a mut TestAppContext,
) -> (Entity<Workbench>, &'a mut VisualTestContext) {
    let (root, data) = (root.to_path_buf(), data.to_path_buf());
    cx.add_window_view(|window, cx| Workbench::with_data_dir(root, data, window, cx))
}

fn active_view(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Entity<CodeView> {
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

struct Project {
    root: TempDir,
    data: TempDir,
}

impl Project {
    fn new() -> Self {
        let project = Project {
            root: TempDir::new().unwrap(),
            data: TempDir::new().unwrap(),
        };
        for name in ["a.txt", "b.txt", "c.txt"] {
            fs::write(project.root.path().join(name), lines(300)).unwrap();
        }
        project
    }

    fn file(&self, name: &str) -> PathBuf {
        self.root.path().join(name)
    }

    fn state_file(&self) -> PathBuf {
        state_path(self.data.path(), self.root.path())
    }
}

#[gpui::test]
fn restore_applies_scroll_and_sidebar(cx: &mut TestAppContext) {
    init(cx);
    let project = Project::new();
    let (a, b, c) = (
        project.file("a.txt"),
        project.file("b.txt"),
        project.file("c.txt"),
    );
    let gone = project.file("gone.txt");
    let mut saved = sample(&[&a, &gone, &b, &c]);
    saved.groups.groups[1].active = Some(1); // `b`
    saved.scroll = HashMap::from([(a.clone(), 40), (b.clone(), 120), (gone.clone(), 9)]);
    save_state(&project.state_file(), &saved).unwrap();

    let (wb, cx) = workbench(project.root.path(), project.data.path(), cx);
    wb.read_with(cx, |wb, _| {
        assert!(!wb.sidebar_visible());
        // `"search"` is from before the Search panel became a tab.
        assert_eq!(wb.active_sidebar(), SidebarView::Explorer);
        assert_eq!(wb.sidebar_width(), px(333.));
    });
    let restored = cx.update(|_, cx| wb.read(cx).editor_groups().read(cx).model().state());
    assert_eq!(
        restored.groups[1].tabs,
        vec![(b.clone(), true), (c.clone(), false)],
        "the missing file's tab is gone"
    );
    assert_eq!(restored.groups[1].active, Some(0), "active tab follows `b`");
    assert_eq!(restored.active_group, 1);

    // The focused group's tab is drawn and loads; its saved row applies
    // once the text is there.
    settle(cx);
    assert_eq!(
        active_view(&wb, cx).read_with(cx, |v, _| v.scroll_top_row()),
        120
    );

    // Activating the other group's tab draws it later; same.
    wb.update(cx, |wb, cx| {
        wb.update_editor_groups(cx, |m| m.focus_group(0))
    });
    settle(cx);
    assert_eq!(
        active_view(&wb, cx).read_with(cx, |v, _| v.scroll_top_row()),
        40
    );
}

#[gpui::test]
fn jump_before_load_beats_the_restored_scroll_row(cx: &mut TestAppContext) {
    init(cx);
    let project = Project::new();
    let (a, b, c) = (
        project.file("a.txt"),
        project.file("b.txt"),
        project.file("c.txt"),
    );
    let long: String = (1..=500).map(|i| format!("line {i}\n")).collect();
    fs::write(&b, long).unwrap();
    let mut saved = sample(&[&c, &a, &b]); // `b` is a background tab
    saved.scroll = HashMap::from([(b.clone(), 400)]);
    save_state(&project.state_file(), &saved).unwrap();

    let (wb, cx) = workbench(project.root.path(), project.data.path(), cx);
    // `b` is a restored background tab: not drawn, so not loaded.
    wb.update(cx, |wb, cx| {
        wb.jump_to(
            Arc::from(b.as_path()),
            lucerna_text::Point { row: 20, column: 0 },
            OpenMode::Preview,
            cx,
        )
    });
    settle(cx);
    let view = active_view(&wb, cx);
    let (cursor, top) = view.read_with(cx, |v, _| (v.cursor(), v.scroll_top_row()));
    assert_eq!(cursor.row, 20);
    assert!(top <= 20, "the jump target is in view, not row {top}");
}

#[gpui::test]
fn without_a_data_dir_nothing_is_restored_or_saved(cx: &mut TestAppContext) {
    init(cx);
    let project = Project::new();
    save_state(
        &project.state_file(),
        &sample(&[&project.file("a.txt"), &project.file("b.txt")]),
    )
    .unwrap();
    let root = project.root.path().to_path_buf();
    let (wb, cx) = cx.add_window_view(|window, cx| Workbench::new(root, window, cx));
    assert!(wb.read_with(cx, |wb, _| wb.sidebar_visible()));
    wb.update(cx, |wb, cx| wb.save_workspace_state(cx));
}

#[gpui::test]
fn saves_throttled_after_layout_changes(cx: &mut TestAppContext) {
    init(cx);
    let project = Project::new();
    let (wb, cx) = workbench(project.root.path(), project.data.path(), cx);
    assert!(!project.state_file().exists(), "opening saves nothing");

    let a: Arc<Path> = Arc::from(project.file("a.txt").as_path());
    wb.update(cx, |wb, cx| wb.open_path(a, OpenMode::Pinned, cx));
    settle(cx);
    assert!(
        !project.state_file().exists(),
        "the save waits for the timer"
    );
    cx.executor().advance_clock(Duration::from_millis(1900));
    cx.run_until_parked();
    assert!(!project.state_file().exists(), "not before 2 s");
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    let saved = load_state(&project.state_file()).expect("saved after 2 s");
    assert_eq!(
        saved.groups.groups[0].tabs,
        vec![(project.file("a.txt"), true)]
    );

    // Scrolling settles into a save too, with the row of the open tab.
    active_view(&wb, cx).update(cx, |v, cx| v.set_scroll_top_row(77, cx));
    settle(cx);
    cx.executor().advance_clock(Duration::from_secs(3));
    cx.run_until_parked();
    let saved = load_state(&project.state_file()).unwrap();
    assert_eq!(saved.scroll, HashMap::from([(project.file("a.txt"), 77)]));

    // Sidebar changes are layout changes.
    wb.update(cx, |wb, cx| wb.set_sidebar_width(px(400.), cx));
    cx.executor().advance_clock(Duration::from_secs(3));
    cx.run_until_parked();
    assert_eq!(
        load_state(&project.state_file()).unwrap().sidebar_width,
        400.
    );
}

#[gpui::test]
fn restoring_does_not_save(cx: &mut TestAppContext) {
    init(cx);
    let project = Project::new();
    let (a, gone) = (project.file("a.txt"), project.file("gone.txt"));
    let mut saved = sample(&[&a, &gone]);
    saved.scroll = HashMap::from([(a, 30)]);
    save_state(&project.state_file(), &saved).unwrap();
    let before = fs::read_to_string(project.state_file()).unwrap();

    let (_wb, cx) = workbench(project.root.path(), project.data.path(), cx);
    settle(cx);
    cx.executor().advance_clock(Duration::from_secs(5));
    cx.run_until_parked();
    assert_eq!(fs::read_to_string(project.state_file()).unwrap(), before);
}

#[gpui::test]
fn saves_when_the_window_closes(cx: &mut TestAppContext) {
    init(cx);
    let project = Project::new();
    let (wb, cx) = workbench(project.root.path(), project.data.path(), cx);
    let a: Arc<Path> = Arc::from(project.file("b.txt").as_path());
    wb.update(cx, |wb, cx| wb.open_path(a, OpenMode::Pinned, cx));
    settle(cx);
    assert!(!project.state_file().exists());
    // Closing runs the window's should-close hook.
    assert!(cx.simulate_close());
    assert!(load_state(&project.state_file()).is_some());
}

fn tab_paths(wb: &Entity<Workbench>, cx: &mut VisualTestContext) -> Vec<PathBuf> {
    cx.update(|_, cx| {
        wb.read(cx)
            .editor_groups()
            .read(cx)
            .model()
            .groups()
            .iter()
            .flat_map(|g| g.tabs.iter().map(|t| t.path().unwrap().to_path_buf()))
            .collect()
    })
}

/// The test's working directory is the crate, not the project, so a
/// relative path resolved against it would not exist.
#[gpui::test]
fn relative_tab_paths_are_saved_absolute_and_restored(cx: &mut TestAppContext) {
    init(cx);
    let project = Project::new();
    assert!(!Path::new("a.txt").exists(), "cwd must not be the project");
    {
        let (wb, cx) = workbench(project.root.path(), project.data.path(), cx);
        let rel: Arc<Path> = Arc::from(Path::new("a.txt"));
        wb.update(cx, |wb, cx| wb.open_path(rel, OpenMode::Pinned, cx));
        settle(cx);
        assert_eq!(tab_paths(&wb, cx), vec![project.file("a.txt")]);
        wb.update(cx, |wb, cx| wb.save_workspace_state(cx));
    }
    let saved = load_state(&project.state_file()).expect("saved");
    assert_eq!(
        saved.groups.groups[0].tabs,
        vec![(project.file("a.txt"), true)]
    );
    let (wb, cx) = workbench(project.root.path(), project.data.path(), cx);
    assert_eq!(tab_paths(&wb, cx), vec![project.file("a.txt")]);
}

/// A state file written before paths were normalised may hold relative
/// paths; they resolve against the root, not the working directory.
#[gpui::test]
fn relative_paths_in_an_old_state_file_restore(cx: &mut TestAppContext) {
    init(cx);
    let project = Project::new();
    let mut saved = sample(&[Path::new("a.txt"), Path::new("b.txt")]);
    saved.groups.groups[1].active = Some(0);
    save_state(&project.state_file(), &saved).unwrap();
    let (wb, cx) = workbench(project.root.path(), project.data.path(), cx);
    assert_eq!(
        tab_paths(&wb, cx),
        vec![project.file("a.txt"), project.file("b.txt")]
    );
    // Any change saves; the restored rows go back out keyed by absolute
    // path.
    let c: Arc<Path> = Arc::from(project.file("c.txt").as_path());
    wb.update(cx, |wb, cx| wb.open_path(c, OpenMode::Pinned, cx));
    wb.update(cx, |wb, cx| wb.save_workspace_state(cx));
    let resaved = load_state(&project.state_file()).unwrap();
    for name in ["a.txt", "b.txt"] {
        assert_eq!(
            resaved.scroll.get(&project.file(name)),
            Some(&7),
            "restored scroll rows are keyed by absolute path: {:?}",
            resaved.scroll
        );
    }
}

#[gpui::test]
fn relative_and_absolute_open_the_same_tab(cx: &mut TestAppContext) {
    init(cx);
    let project = Project::new();
    let (wb, cx) = workbench(project.root.path(), project.data.path(), cx);
    let rel: Arc<Path> = Arc::from(Path::new("a.txt"));
    let abs: Arc<Path> = Arc::from(project.file("a.txt").as_path());
    wb.update(cx, |wb, cx| wb.open_path(rel, OpenMode::Pinned, cx));
    wb.update(cx, |wb, cx| wb.open_path(abs, OpenMode::Pinned, cx));
    settle(cx);
    assert_eq!(tab_paths(&wb, cx), vec![project.file("a.txt")]);
    assert_eq!(
        cx.update(|_, cx| wb.read(cx).recent_files().to_vec()),
        vec![Arc::<Path>::from(project.file("a.txt").as_path())]
    );
}

/// Spec §2.1: the split sizes come back. A file from before widths were
/// saved, or whose widths do not fit the restored groups, gets equal ones.
#[gpui::test]
fn group_widths_round_trip(cx: &mut TestAppContext) {
    init(cx);
    let project = Project::new();
    let (a, b) = (project.file("a.txt"), project.file("b.txt"));
    save_state(&project.state_file(), &sample(&[&a, &b])).unwrap();
    {
        let (wb, cx) = workbench(project.root.path(), project.data.path(), cx);
        let widths =
            cx.update(|_, cx| wb.read(cx).editor_groups().read(cx).group_widths().to_vec());
        assert_eq!(widths, [0.25, 0.75]);
        // Open another file so the state differs from what was loaded.
        let c: Arc<Path> = Arc::from(project.file("c.txt").as_path());
        wb.update(cx, |wb, cx| wb.open_path(c, OpenMode::Pinned, cx));
        wb.update(cx, |wb, cx| wb.save_workspace_state(cx));
    }
    assert_eq!(
        load_state(&project.state_file()).unwrap().group_widths,
        [0.25, 0.75]
    );

    // Old file without the field: equal widths.
    let json = fs::read_to_string(project.state_file()).unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
    value.as_object_mut().unwrap().remove("group_widths");
    fs::write(project.state_file(), value.to_string()).unwrap();
    let old = load_state(&project.state_file()).expect("still loads");
    assert!(old.group_widths.is_empty());
    let (wb, cx) = workbench(project.root.path(), project.data.path(), cx);
    let widths = cx.update(|_, cx| wb.read(cx).editor_groups().read(cx).group_widths().to_vec());
    assert_eq!(widths, [0.5, 0.5]);
}

/// The group widths a workbench restores from `state`.
fn restored_widths(project: &Project, state: &WorkspaceState, cx: &mut TestAppContext) -> Vec<f32> {
    save_state(&project.state_file(), state).unwrap();
    let (wb, cx) = workbench(project.root.path(), project.data.path(), cx);
    cx.update(|_, cx| wb.read(cx).editor_groups().read(cx).group_widths().to_vec())
}

#[gpui::test]
fn mismatched_group_widths_are_normalised(cx: &mut TestAppContext) {
    init(cx);
    let project = Project::new();
    let (a, b, gone) = (
        project.file("a.txt"),
        project.file("b.txt"),
        project.file("gone.txt"),
    );
    // Three widths for two groups.
    let mut state = sample(&[&a, &b]);
    state.group_widths = vec![0.2, 0.3, 0.5];
    assert_eq!(restored_widths(&project, &state, cx), [0.5, 0.5]);

    // The right group's only file is gone: one group, the full width.
    let mut state = sample(&[&a, &gone]);
    state.group_widths = vec![0.25, 0.75];
    assert_eq!(restored_widths(&project, &state, cx), [1.0]);

    // Widths that do not sum to 1 are scaled; nonsense ones are dropped.
    let mut state = sample(&[&a, &b]);
    state.group_widths = vec![1.0, 3.0];
    assert_eq!(restored_widths(&project, &state, cx), [0.25, 0.75]);
    state.group_widths = vec![-1.0, 1.0];
    assert_eq!(restored_widths(&project, &state, cx), [0.5, 0.5]);
}

#[test]
fn old_state_without_search_field_loads() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("state.json");
    save_state(&path, &sample(&[Path::new("/a"), Path::new("/b")])).unwrap();
    let mut value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert!(value.as_object_mut().unwrap().remove("search").is_some());
    fs::write(&path, serde_json::to_string(&value).unwrap()).unwrap();
    let state = load_state(&path).expect("a file without `search` still loads");
    assert_eq!(state.search, SearchState::default());
}

/// A lone file keeps its own state, keyed by the file: not shared with its
/// folder opened as a project, nor with another lone file beside it.
#[gpui::test]
fn single_file_state_is_keyed_by_the_file(cx: &mut TestAppContext) {
    init(cx);
    let project = Project::new();
    let a = project.file("a.txt");
    let (wb, cx) = {
        let (root, a, data) = (
            project.root.path().to_path_buf(),
            a.clone(),
            project.data.path().to_path_buf(),
        );
        cx.add_window_view(move |window, cx| {
            Workbench::single_file(root, &a, Some(data), window, cx)
        })
    };
    let tab: Arc<Path> = Arc::from(a.as_path());
    wb.update(cx, |wb, cx| wb.open_path(tab, OpenMode::Pinned, cx));
    settle(cx);
    assert!(cx.simulate_close());
    assert!(load_state(&state_path(project.data.path(), &a)).is_some());
    assert!(
        !project.state_file().exists(),
        "the folder's state is untouched"
    );
}
