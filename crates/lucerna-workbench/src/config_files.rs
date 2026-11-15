//! The user's settings, keybindings and theme files: where they live,
//! loading them into [`AppState`], hot reload through a debounced file
//! watcher, the commands that open them, and the status-bar item that
//! reports a broken file.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError, unbounded};
use futures::StreamExt;
use gpui::{
    App, Context, IntoElement, ParentElement, Render, Styled, Subscription, Task, WeakEntity,
    Window, div, prelude::*,
};
use lucerna_settings::{Keymap, Layer, SettingsError, SettingsStore};
use lucerna_text::Point;
use lucerna_theme::{Appearance, DEFAULT_DARK_THEME, DEFAULT_LIGHT_THEME, ThemeRegistry};
use notify::{RecommendedWatcher, RecursiveMode, Watcher as _};

use crate::extensions::service;
use crate::layout::color;
use crate::status_message;
use crate::tasks::{spawn_guarded, try_spawn_guarded};
use crate::{AppState, CommandRegistry, OpenMode, Workbench};

/// A batch of changes ends once no event arrived for this long...
pub const CONFIG_DEBOUNCE: Duration = Duration::from_millis(50);
/// ...or when it has been collecting for this long.
const MAX_BATCH: Duration = Duration::from_millis(200);

/// What a missing user `settings.json` is created with.
pub(crate) const USER_SETTINGS_TEMPLATE: &str = "// Lucerna user settings\n{\n}\n";
const WORKSPACE_SETTINGS_TEMPLATE: &str = "// Lucerna workspace settings\n{\n}\n";
const KEYBINDINGS_TEMPLATE: &str = "// Lucerna user keybindings\n[\n]\n";

/// Where the configuration files are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigPaths {
    /// `~/.config/lucerna`: `settings.json`, `keybindings.json`, `themes/`.
    pub user_dir: PathBuf,
    /// `<root>/.lucerna/settings.json`; `None` while no folder is open.
    pub workspace_settings: Option<PathBuf>,
}

impl ConfigPaths {
    /// The user directory under `$HOME`, and the workspace settings of
    /// `root`.
    pub fn for_root(root: &Path) -> ConfigPaths {
        let home = std::env::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        ConfigPaths::with_user_dir(home.join(".config").join("lucerna"), root)
    }

    /// Like [`ConfigPaths::for_root`] with another user directory; for tests.
    pub fn with_user_dir(user_dir: impl Into<PathBuf>, root: &Path) -> ConfigPaths {
        ConfigPaths {
            user_dir: user_dir.into(),
            workspace_settings: Some(root.join(".lucerna").join("settings.json")),
        }
    }

    /// The user directory under `$HOME` and no workspace layer: for a
    /// window with no folder open (the Welcome screen).
    pub fn user_only() -> ConfigPaths {
        let home = std::env::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        ConfigPaths::user_only_with_user_dir(home.join(".config").join("lucerna"))
    }

    /// Like [`ConfigPaths::user_only`] with another user directory; for
    /// tests.
    pub fn user_only_with_user_dir(user_dir: impl Into<PathBuf>) -> ConfigPaths {
        ConfigPaths {
            user_dir: user_dir.into(),
            workspace_settings: None,
        }
    }

    pub fn user_settings(&self) -> PathBuf {
        self.user_dir.join("settings.json")
    }

    pub fn keybindings(&self) -> PathBuf {
        self.user_dir.join("keybindings.json")
    }

    /// User theme families, `*.json` in Zed's format.
    pub fn themes_dir(&self) -> PathBuf {
        self.user_dir.join("themes")
    }

    /// `<root>/.lucerna`; `None` while no folder is open.
    pub fn workspace_dir(&self) -> Option<&Path> {
        self.workspace_settings.as_deref().map(|path| {
            path.parent()
                .expect("workspace settings live in a directory")
        })
    }

    /// The directories the watcher follows.
    fn watched_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = vec![self.user_dir.clone()];
        dirs.extend(self.workspace_dir().map(Path::to_path_buf));
        dirs
    }
}

/// One configuration file (or the themes directory).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConfigFile {
    UserSettings,
    WorkspaceSettings,
    Keybindings,
    Themes,
}

/// A configuration file that could not be read or parsed.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigError {
    pub file: ConfigFile,
    pub path: PathBuf,
    pub error: SettingsError,
}

impl ConfigError {
    /// `"settings.json: line 3: expected value"`; workspace settings are
    /// named `.lucerna/settings.json`.
    pub fn status_text(&self) -> String {
        let name = match self.file {
            ConfigFile::UserSettings => "settings.json",
            ConfigFile::WorkspaceSettings => ".lucerna/settings.json",
            ConfigFile::Keybindings => "keybindings.json",
            ConfigFile::Themes => "themes",
        };
        format!("{name}: {}", self.error)
    }
}

/// The configuration files in use, their current errors and the watcher.
/// A GPUI global set by [`watch_config`].
pub struct ConfigState {
    paths: ConfigPaths,
    errors: BTreeMap<ConfigFile, ConfigError>,
    watcher: Option<ConfigWatcher>,
    _pump: Task<()>,
    /// A theme picker is previewing: reloads leave the active theme alone.
    theme_preview: bool,
    /// An icon theme picker is previewing: reloads leave the icons alone.
    icon_preview: bool,
    /// Bumped by every keymap load.
    keymap_generation: u64,
    /// The paths of the last batch [`reload_config`] was given, and a
    /// count of batches: the workbench reloads open files among them.
    changed: (u64, Arc<[PathBuf]>),
    /// The system color mode, for `window.autoDetectColorScheme`.
    system_appearance: Appearance,
}

impl gpui::Global for ConfigState {}

impl ConfigState {
    pub fn paths(&self) -> &ConfigPaths {
        &self.paths
    }

    /// Bumped each time keybindings are loaded into [`AppState`], so the
    /// workbench can drop a chord typed against the old keymap.
    pub fn keymap_generation(&self) -> u64 {
        self.keymap_generation
    }

    /// The paths of the last batch of changes, and how many batches there
    /// have been. Open documents among them are reloaded.
    pub fn last_changed(&self) -> (u64, &[PathBuf]) {
        (self.changed.0, &self.changed.1)
    }

    /// Files that failed to load, user settings first.
    pub fn errors(&self) -> impl Iterator<Item = &ConfigError> {
        self.errors.values()
    }
}

/// Loads every configuration file into [`AppState`], then follows changes
/// to them: settings go through `set_layer`, keybindings through
/// `Keymap::load`, user themes through `ThemeRegistry::build`, and the theme named by
/// `workbench.colorTheme` is made active. Replaces an earlier call's
/// paths and watcher.
///
/// The watcher is not started on GPUI's deterministic test scheduler,
/// which rejects wake-ups from other threads; tests deliver changes with
/// [`reload_config`].
///
/// # Panics
/// If the [`AppState`] global is not set.
pub fn watch_config(paths: ConfigPaths, cx: &mut App) {
    assert!(
        cx.has_global::<AppState>(),
        "set the AppState global before watch_config"
    );
    let (watcher, pump) = if cx.background_executor().is_test() {
        (None, Task::ready(()))
    } else {
        start_watching(&paths, cx)
    };
    cx.set_global(ConfigState {
        paths,
        errors: BTreeMap::new(),
        watcher,
        _pump: pump,
        theme_preview: false,
        icon_preview: false,
        keymap_generation: 0,
        changed: (0, Arc::from([])),
        system_appearance: to_appearance(cx.window_appearance()),
    });
    // Settings first: the theme files read depend on `workbench.colorTheme`.
    load(
        [
            ConfigFile::UserSettings,
            ConfigFile::WorkspaceSettings,
            ConfigFile::Themes,
            ConfigFile::Keybindings,
        ],
        true,
        cx,
    );
}

/// Re-reads the configuration files that `changed` paths touch and
/// applies them. What the watcher calls for each debounced batch.
pub fn reload_config(changed: &[PathBuf], cx: &mut App) {
    if !cx.has_global::<ConfigState>() {
        return;
    }
    // An open `settings.json` reloads too (the workbench observes this).
    let state = cx.global_mut::<ConfigState>();
    state.changed = (state.changed.0 + 1, Arc::from(changed));
    let state = cx.global::<ConfigState>();
    let files = classify(&state.paths, changed);
    if !files.is_empty() {
        load(files, false, cx);
    }
}

fn start_watching(paths: &ConfigPaths, cx: &mut App) -> (Option<ConfigWatcher>, Task<()>) {
    let (tx, mut rx) = futures::channel::mpsc::unbounded();
    let watcher = ConfigWatcher::start(paths, move |batch| {
        let _ = tx.unbounded_send(batch);
    });
    match watcher {
        Ok(watcher) => {
            let pump = cx.spawn(async move |cx| {
                while let Some(batch) = rx.next().await {
                    cx.update(|cx| reload_config(&batch, cx));
                }
            });
            (Some(watcher), pump)
        }
        Err(err) => {
            tracing::error!(%err, "could not watch the configuration files");
            (None, Task::ready(()))
        }
    }
}

/// Loads `files` and makes the configured theme active. The `initial`
/// load reads only the extension theme files holding the configured theme;
/// the rest load in the background (see `extensions::service`).
fn load(files: impl IntoIterator<Item = ConfigFile>, initial: bool, cx: &mut App) {
    let paths = cx.global::<ConfigState>().paths.clone();
    let mut settings_loaded = false;
    for file in files {
        settings_loaded |= matches!(
            file,
            ConfigFile::UserSettings | ConfigFile::WorkspaceSettings
        );
        match file {
            ConfigFile::UserSettings => load_settings(file, Layer::User, paths.user_settings(), cx),
            ConfigFile::WorkspaceSettings => {
                if let Some(path) = paths.workspace_settings.clone() {
                    load_settings(file, Layer::Workspace, path, cx);
                }
            }
            ConfigFile::Keybindings => {
                let path = paths.keybindings();
                let result = read(&path).and_then(|text| {
                    match Keymap::load(Keymap::default_bindings(), text.as_deref()) {
                        (keymap, None) => {
                            cx.global_mut::<ConfigState>().keymap_generation += 1;
                            cx.global_mut::<AppState>().keymap = keymap;
                            Ok(())
                        }
                        // The bindings in use stay.
                        (_, Some(err)) => Err(err),
                    }
                });
                record(file, path, result, cx);
            }
            ConfigFile::Themes => {
                let dir = paths.themes_dir();
                let user_dir = dir.is_dir().then_some(dir.as_path());
                // The first load reads only the extension files holding the
                // themes the settings name: both preferred ones while the
                // theme follows the system, so a mode switch needs no load.
                let wanted: Option<Vec<String>> =
                    initial.then(|| wanted_theme_names(&cx.global::<AppState>().settings));
                let extension_files = service::extension_theme_files(cx, wanted.as_deref());
                let (themes, errors) = ThemeRegistry::build(&extension_files, user_dir);
                for (path, err) in errors {
                    tracing::warn!(path = %path.display(), %err, "could not load theme");
                }
                cx.global_mut::<AppState>().themes = themes;
            }
        }
    }
    if initial {
        service::config_loaded(cx);
    } else if settings_loaded {
        service::settings_changed(cx);
    }
    reapply_configured_theme(cx);
}

/// [`apply_configured_theme`], unless a theme picker is previewing.
pub(crate) fn reapply_configured_theme(cx: &mut App) {
    if !cx
        .try_global::<ConfigState>()
        .is_some_and(|state| state.theme_preview)
    {
        apply_configured_theme(cx);
    }
}

/// While `previewing`, reloads keep the theme a picker shows. Does
/// nothing without configuration files.
pub(crate) fn set_theme_preview(previewing: bool, cx: &mut App) {
    if cx.has_global::<ConfigState>() {
        cx.global_mut::<ConfigState>().theme_preview = previewing;
    }
}

/// While `previewing`, reloads and registry swaps keep the icon theme a
/// picker shows. Does nothing without configuration files.
pub(crate) fn set_icon_preview(previewing: bool, cx: &mut App) {
    if cx.has_global::<ConfigState>() {
        cx.global_mut::<ConfigState>().icon_preview = previewing;
    }
}

/// Whether an icon theme picker is previewing.
pub(crate) fn icon_previewing(cx: &App) -> bool {
    cx.try_global::<ConfigState>()
        .is_some_and(|state| state.icon_preview)
}

/// Shows `error` for `file` in the status bar until the file next loads.
pub(crate) fn record_error(file: ConfigFile, path: PathBuf, error: SettingsError, cx: &mut App) {
    record(file, path, Err(error), cx);
}

fn load_settings(file: ConfigFile, layer: Layer, path: PathBuf, cx: &mut App) {
    let result = read(&path).and_then(|text| {
        let text = text.as_deref().unwrap_or("{}");
        cx.global_mut::<AppState>().settings.set_layer(layer, text)
    });
    record(file, path, result, cx);
}

/// The file's text; `None` when it does not exist.
fn read(path: &Path) -> Result<Option<String>, SettingsError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(SettingsError {
            message: err.to_string(),
            line: None,
        }),
    }
}

fn record(file: ConfigFile, path: PathBuf, result: Result<(), SettingsError>, cx: &mut App) {
    let errors = &cx.global::<ConfigState>().errors;
    let new = result.err().map(|error| ConfigError { file, path, error });
    if errors.get(&file) == new.as_ref() {
        return;
    }
    let errors = &mut cx.global_mut::<ConfigState>().errors;
    match new {
        Some(err) => errors.insert(file, err),
        None => errors.remove(&file),
    };
}

pub(crate) const AUTO_DETECT_KEY: &str = "window.autoDetectColorScheme";
pub(crate) const COLOR_THEME_KEY: &str = "workbench.colorTheme";
pub(crate) const PREFERRED_DARK_KEY: &str = "workbench.preferredDarkColorTheme";
pub(crate) const PREFERRED_LIGHT_KEY: &str = "workbench.preferredLightColorTheme";

/// GPUI's window appearance as a theme appearance.
pub(crate) fn to_appearance(appearance: gpui::WindowAppearance) -> Appearance {
    match appearance {
        gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark => Appearance::Dark,
        gpui::WindowAppearance::Light | gpui::WindowAppearance::VibrantLight => Appearance::Light,
    }
}

pub(crate) fn auto_detect(settings: &SettingsStore) -> bool {
    settings.get(AUTO_DETECT_KEY, None).unwrap_or(true)
}

/// The setting that names the color theme in use, as in VS Code: one of
/// the preferred themes while `window.autoDetectColorScheme` is on (by the
/// system mode), else `workbench.colorTheme`.
pub(crate) fn color_theme_setting_key(
    settings: &SettingsStore,
    system: Appearance,
) -> &'static str {
    match (auto_detect(settings), system) {
        (false, _) => COLOR_THEME_KEY,
        (true, Appearance::Dark) => PREFERRED_DARK_KEY,
        (true, Appearance::Light) => PREFERRED_LIGHT_KEY,
    }
}

/// The name of the color theme the settings ask for in `system` mode.
pub(crate) fn configured_theme_name(settings: &SettingsStore, system: Appearance) -> String {
    let key = color_theme_setting_key(settings, system);
    settings.get(key, None).unwrap_or_else(|| {
        match key {
            PREFERRED_LIGHT_KEY => DEFAULT_LIGHT_THEME,
            _ => DEFAULT_DARK_THEME,
        }
        .to_string()
    })
}

/// The theme names the settings can select right now.
fn wanted_theme_names(settings: &SettingsStore) -> Vec<String> {
    if auto_detect(settings) {
        vec![
            configured_theme_name(settings, Appearance::Dark),
            configured_theme_name(settings, Appearance::Light),
        ]
    } else {
        vec![configured_theme_name(settings, Appearance::Dark)]
    }
}

/// The system color mode as last reported (the platform's when
/// [`watch_config`] was not called).
pub(crate) fn system_appearance(cx: &App) -> Appearance {
    cx.try_global::<ConfigState>()
        .map(|state| state.system_appearance)
        .unwrap_or_else(|| to_appearance(cx.window_appearance()))
}

/// Records the system color mode and applies the theme it selects (unless
/// a theme picker is previewing). The window appearance observer calls it.
pub fn set_system_appearance(appearance: Appearance, cx: &mut App) {
    if cx.has_global::<ConfigState>() {
        let state = cx.global_mut::<ConfigState>();
        if state.system_appearance == appearance {
            return;
        }
        state.system_appearance = appearance;
    }
    reapply_configured_theme(cx);
}

/// Follows the system color mode while `window` is open.
pub(crate) fn follow_system_appearance(window: &mut Window, _: &mut App) {
    window
        .observe_window_appearance(|window, cx| {
            set_system_appearance(to_appearance(window.appearance()), cx);
        })
        .detach();
}

/// Makes the theme the settings select active: by the system mode while
/// `window.autoDetectColorScheme` is on, else `workbench.colorTheme`
/// ("Lucerna Dark" when the name is unknown).
pub(crate) fn apply_configured_theme(cx: &mut App) {
    let system = system_appearance(cx);
    let state = cx.global::<AppState>();
    let name = configured_theme_name(&state.settings, system);
    let theme = state.themes.get_or_default(&name);
    if !Arc::ptr_eq(&theme, &state.theme) {
        cx.global_mut::<AppState>().set_theme(theme);
    }
}

/// Which configuration files the changed paths touch. A change to a
/// configuration directory itself (created, removed, or events lost)
/// touches every file in it.
fn classify(paths: &ConfigPaths, changed: &[PathBuf]) -> BTreeSet<ConfigFile> {
    let user_dir = real_path(&paths.user_dir);
    let themes = user_dir.join("themes");
    let workspace_dir = paths.workspace_dir().map(real_path);
    let mut files = BTreeSet::new();
    for path in changed {
        let path = real_path(path);
        if user_dir.starts_with(&path) {
            files.extend([
                ConfigFile::UserSettings,
                ConfigFile::Keybindings,
                ConfigFile::Themes,
            ]);
        }
        if let Some(workspace_dir) = &workspace_dir
            && (workspace_dir.starts_with(&path) || path == workspace_dir.join("settings.json"))
        {
            files.insert(ConfigFile::WorkspaceSettings);
        }
        if path == user_dir.join("settings.json") {
            files.insert(ConfigFile::UserSettings);
        }
        if path == user_dir.join("keybindings.json") {
            files.insert(ConfigFile::Keybindings);
        }
        if path.starts_with(&themes) {
            files.insert(ConfigFile::Themes);
        }
    }
    files
}

/// `path` with symlinks resolved, as FSEvents reports it (`/private/var`
/// for `/var`). Missing components are resolved through the deepest
/// ancestor that exists.
pub(crate) fn real_path(path: &Path) -> PathBuf {
    for ancestor in path.ancestors() {
        if let Ok(real) = std::fs::canonicalize(ancestor) {
            let rest = path
                .strip_prefix(ancestor)
                .expect("an ancestor is a prefix");
            return if rest.as_os_str().is_empty() {
                real
            } else {
                real.join(rest)
            };
        }
    }
    path.to_path_buf()
}

/// Watches the user and workspace configuration directories and reports
/// changed paths in batches debounced by [`CONFIG_DEBOUNCE`]. A directory
/// that does not exist yet is followed through its parent until it does.
/// Dropping it stops watching.
pub struct ConfigWatcher {
    state: Arc<Mutex<WatchState>>,
}

struct WatchState {
    watcher: RecommendedWatcher,
    dirs: Vec<PathBuf>,
    watched: Vec<(PathBuf, RecursiveMode)>,
}

impl ConfigWatcher {
    /// Starts watching; `on_batch` runs on the watcher's thread.
    pub fn start(
        paths: &ConfigPaths,
        on_batch: impl Fn(Vec<PathBuf>) + Send + 'static,
    ) -> notify::Result<ConfigWatcher> {
        let (tx, rx) = unbounded();
        let watcher = notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        })?;
        let state = Arc::new(Mutex::new(WatchState {
            watcher,
            dirs: paths.watched_dirs(),
            watched: Vec::new(),
        }));
        lock(&state).rearm();
        let weak = Arc::downgrade(&state);
        try_spawn_guarded("lucerna-config-watch", move || {
            debounce_loop(&rx, &weak, &on_batch);
        })
        .map_err(notify::Error::io)?;
        Ok(ConfigWatcher { state })
    }

    /// Re-picks what to watch, after a configuration directory was made.
    pub fn rearm(&self) {
        lock(&self.state).rearm();
    }
}

fn lock(state: &Mutex<WatchState>) -> std::sync::MutexGuard<'_, WatchState> {
    state.lock().unwrap_or_else(|e| e.into_inner())
}

impl WatchState {
    /// Watches each directory that exists, recursively, and the parent of
    /// each that does not.
    fn rearm(&mut self) {
        let mut wanted: Vec<(PathBuf, RecursiveMode)> = Vec::new();
        for dir in &self.dirs {
            let target = if dir.is_dir() {
                Some((dir.clone(), RecursiveMode::Recursive))
            } else {
                dir.parent()
                    .filter(|p| p.is_dir())
                    .map(|p| (p.to_path_buf(), RecursiveMode::NonRecursive))
            };
            if let Some(target) = target.filter(|t| !wanted.contains(t)) {
                wanted.push(target);
            }
        }
        for (path, mode) in &self.watched {
            if !wanted.contains(&(path.clone(), *mode)) {
                let _ = self.watcher.unwatch(path);
            }
        }
        let mut watched = Vec::new();
        for target in wanted {
            if self.watched.contains(&target) {
                watched.push(target);
                continue;
            }
            match self.watcher.watch(&target.0, target.1) {
                Ok(()) => watched.push(target),
                Err(err) => {
                    tracing::warn!(path = %target.0.display(), %err, "could not watch")
                }
            }
        }
        self.watched = watched;
    }
}

fn debounce_loop(
    rx: &Receiver<notify::Result<notify::Event>>,
    state: &Weak<Mutex<WatchState>>,
    on_batch: &dyn Fn(Vec<PathBuf>),
) {
    // Ends once the watcher, which holds the sender, is dropped.
    while let Ok(first) = rx.recv() {
        let started = Instant::now();
        let mut events = vec![first];
        loop {
            let left = MAX_BATCH
                .saturating_sub(started.elapsed())
                .min(CONFIG_DEBOUNCE);
            if left.is_zero() {
                break;
            }
            match rx.recv_timeout(left) {
                Ok(event) => events.push(event),
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => break,
            }
        }
        let Some(state) = state.upgrade() else {
            return;
        };
        let dirs = {
            let mut state = lock(&state);
            state.rearm();
            state.dirs.clone()
        };
        drop(state);
        let mut changed = BTreeSet::new();
        for event in events {
            match event {
                Ok(event) => {
                    if event.need_rescan() {
                        changed.extend(dirs.iter().cloned());
                    }
                    changed.extend(event.paths);
                }
                // Events may have been lost: reload everything.
                Err(_) => changed.extend(dirs.iter().cloned()),
            }
        }
        if !changed.is_empty() {
            on_batch(changed.into_iter().collect());
        }
    }
}

pub(crate) fn register_commands(commands: &mut CommandRegistry) {
    commands.register(
        "workbench.action.openSettingsJson",
        "Preferences: Open User Settings (JSON)",
        |wb, _, window, cx| {
            open_config_file(
                wb,
                |paths| Some(paths.user_settings()),
                USER_SETTINGS_TEMPLATE,
                window,
                cx,
            )
        },
    );
    commands.register(
        "workbench.action.openGlobalKeybindingsFile",
        "Preferences: Open Keyboard Shortcuts (JSON)",
        |wb, _, window, cx| {
            open_config_file(
                wb,
                |paths| Some(paths.keybindings()),
                KEYBINDINGS_TEMPLATE,
                window,
                cx,
            )
        },
    );
    commands.register(
        "workbench.action.openWorkspaceSettingsFile",
        "Preferences: Open Workspace Settings (JSON)",
        |wb, _, window, cx| {
            open_config_file(
                wb,
                |paths| paths.workspace_settings.clone(),
                WORKSPACE_SETTINGS_TEMPLATE,
                window,
                cx,
            )
        },
    );
    commands.register(
        "lucerna.openLogsFolder",
        "Lucerna: Open Logs Folder",
        |_, _, _, _| open_logs_folder(),
    );
}

/// Opens a configuration file in a pinned tab, creating it (and its
/// directory) from `template` first when it does not exist.
fn open_config_file(
    wb: &mut Workbench,
    path: impl FnOnce(&ConfigPaths) -> Option<PathBuf>,
    template: &str,
    window: &mut Window,
    cx: &mut Context<Workbench>,
) {
    let Some(state) = cx.try_global::<ConfigState>() else {
        tracing::warn!("watch_config was not called: no configuration files to open");
        return;
    };
    let Some(path) = path(&state.paths) else {
        status_message::show("No folder is open", cx);
        return;
    };
    match create_if_missing(&path, template) {
        Ok(true) => {
            if let Some(watcher) = &state.watcher {
                watcher.rearm();
            }
        }
        Ok(false) => {}
        Err(err) => {
            tracing::error!(path = %path.display(), %err, "could not create the file");
            return;
        }
    }
    wb.open_path(Arc::from(path), OpenMode::Pinned, cx);
    wb.focus_editor_area(window, cx);
}

/// Writes `template` to `path` unless it exists. Returns whether it did.
pub(crate) fn create_if_missing(path: &Path, template: &str) -> io::Result<bool> {
    if path.exists() {
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    use std::io::Write as _;
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(mut file) => file.write_all(template.as_bytes()).map(|()| true),
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => Ok(false),
        Err(err) => Err(err),
    }
}

/// `open ~/Library/Logs/Lucerna`, creating the folder first.
fn open_logs_folder() {
    let Some(home) = std::env::home_dir() else {
        return;
    };
    let dir = home.join("Library").join("Logs").join("Lucerna");
    if let Err(err) = std::fs::create_dir_all(&dir) {
        tracing::error!(%err, "could not create the logs folder");
        return;
    }
    match std::process::Command::new("open").arg(&dir).spawn() {
        // Reap it off the main thread.
        Ok(mut child) => drop(spawn_guarded("lucerna-open-logs", move || child.wait())),
        Err(err) => tracing::error!(%err, "could not open the logs folder"),
    }
}

/// The status-bar item for a broken configuration file, in the theme's
/// error color. Clicking it opens the file at the reported line.
pub(crate) struct ConfigErrorItem {
    workbench: WeakEntity<Workbench>,
    _errors: Subscription,
}

impl ConfigErrorItem {
    pub(crate) fn new(workbench: WeakEntity<Workbench>, cx: &mut Context<Self>) -> Self {
        ConfigErrorItem {
            workbench,
            _errors: cx.observe_global::<ConfigState>(|_, cx| cx.notify()),
        }
    }
}

impl Render for ConfigErrorItem {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let error = cx
            .try_global::<ConfigState>()
            .and_then(|state| state.errors().next().cloned());
        let error_color = color(cx.global::<AppState>().theme.ui.error);
        let workbench = self.workbench.clone();
        div()
            .id("settings-error")
            .when(error.is_none(), |d| d.hidden())
            .when_some(error, |d, error| {
                let row = error.error.line.unwrap_or(1).saturating_sub(1) as u32;
                let path: Arc<Path> = Arc::from(error.path.as_path());
                d.debug_selector(|| "settings-error".into())
                    .text_color(error_color)
                    .cursor_pointer()
                    .child(error.status_text())
                    .on_click(move |_, window, cx| {
                        let point = Point { row, column: 0 };
                        let path = path.clone();
                        workbench
                            .update(cx, |wb, cx| {
                                wb.open_path_at(path, OpenMode::Pinned, point, cx);
                                wb.focus_editor_area(window, cx);
                            })
                            .ok();
                    })
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lucerna_settings::SettingsStore;
    use lucerna_theme::Appearance;

    fn store(user: &str) -> SettingsStore {
        let mut store = SettingsStore::with_defaults();
        store.set_layer(Layer::User, user).unwrap();
        store
    }

    #[test]
    fn theme_name_follows_system_when_auto_detect_is_on() {
        let s = store("{}");
        assert_eq!(configured_theme_name(&s, Appearance::Dark), "Lucerna Dark");
        assert_eq!(
            configured_theme_name(&s, Appearance::Light),
            "Lucerna Light"
        );
    }

    #[test]
    fn theme_name_uses_color_theme_when_auto_detect_is_off() {
        let s = store(r#"{"window.autoDetectColorScheme": false, "workbench.colorTheme": "X"}"#);
        assert_eq!(configured_theme_name(&s, Appearance::Dark), "X");
        assert_eq!(configured_theme_name(&s, Appearance::Light), "X");
    }

    #[test]
    fn preferred_settings_override_defaults() {
        let s = store(r#"{"workbench.preferredLightColorTheme": "Y"}"#);
        assert_eq!(configured_theme_name(&s, Appearance::Light), "Y");
        assert_eq!(configured_theme_name(&s, Appearance::Dark), "Lucerna Dark");
    }

    #[test]
    fn setting_key_matches_mode() {
        let on = store("{}");
        assert_eq!(
            color_theme_setting_key(&on, Appearance::Dark),
            "workbench.preferredDarkColorTheme"
        );
        assert_eq!(
            color_theme_setting_key(&on, Appearance::Light),
            "workbench.preferredLightColorTheme"
        );
        let off = store(r#"{"window.autoDetectColorScheme": false}"#);
        assert_eq!(
            color_theme_setting_key(&off, Appearance::Light),
            "workbench.colorTheme"
        );
    }

    #[test]
    fn window_appearance_maps_to_theme_appearance() {
        use gpui::WindowAppearance as W;
        assert_eq!(to_appearance(W::Dark), Appearance::Dark);
        assert_eq!(to_appearance(W::VibrantDark), Appearance::Dark);
        assert_eq!(to_appearance(W::Light), Appearance::Light);
        assert_eq!(to_appearance(W::VibrantLight), Appearance::Light);
    }

    #[test]
    fn classify_maps_paths_to_files() {
        let user = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let paths = ConfigPaths::with_user_dir(user.path(), root.path());
        let files = |changed: &[PathBuf]| -> Vec<ConfigFile> {
            classify(&paths, changed).into_iter().collect()
        };
        assert_eq!(files(&[paths.user_settings()]), [ConfigFile::UserSettings]);
        assert_eq!(files(&[paths.keybindings()]), [ConfigFile::Keybindings]);
        assert_eq!(
            files(&[paths.themes_dir().join("x.json")]),
            [ConfigFile::Themes]
        );
        assert_eq!(
            files(&[paths.workspace_settings.clone().unwrap()]),
            [ConfigFile::WorkspaceSettings]
        );
        assert_eq!(
            files(std::slice::from_ref(&paths.user_dir)),
            [
                ConfigFile::UserSettings,
                ConfigFile::Keybindings,
                ConfigFile::Themes
            ]
        );
        assert!(files(&[root.path().join("a.txt")]).is_empty());
        assert!(files(&[user.path().join("other.json")]).is_empty());
    }

    #[test]
    fn files_ignore_workspace_without_folder() {
        let user = tempfile::tempdir().unwrap();
        let paths = ConfigPaths::user_only_with_user_dir(user.path());
        let changed = [user.path().join("x/.lucerna/settings.json")];
        assert!(!classify(&paths, &changed).contains(&ConfigFile::WorkspaceSettings));
        assert!(paths.workspace_dir().is_none());
    }

    #[test]
    fn create_if_missing_keeps_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new").join("settings.json");
        assert!(create_if_missing(&path, USER_SETTINGS_TEMPLATE).unwrap());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            USER_SETTINGS_TEMPLATE
        );
        std::fs::write(&path, "{}").unwrap();
        assert!(!create_if_missing(&path, USER_SETTINGS_TEMPLATE).unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{}");
    }
}
