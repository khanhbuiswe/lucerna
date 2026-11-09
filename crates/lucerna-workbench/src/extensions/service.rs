//! The extension service (spec §4.7): runs store operations in the
//! background, follows the store's index, and swaps the language, theme
//! and icon-theme registries in [`AppState`] when it changes.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use futures::StreamExt;
use gpui::{App, Task};
use lucerna_extension::index::{ExtensionIndex, IndexedExtension, LanguageEntry};
use lucerna_extension::process::CancelToken;
use lucerna_extension::registry::is_stale;
use lucerna_extension::store::{InstallProgress, Store, StoreError, StoreEvent};
use lucerna_syntax::{Associations, ExtensionLanguage, LanguageRegistry};
use lucerna_theme::{IconThemeRegistry, ThemeRegistry};

use crate::AppState;
use crate::config_files::{ConfigState, icon_previewing, reapply_configured_theme};
use crate::tasks::{catch_panic, try_spawn_guarded};

/// How long after the first frame the registry is refreshed, when it is
/// stale and `extensions.autoCheckUpdates` is on.
pub const STARTUP_REFRESH_DELAY: Duration = Duration::from_secs(5);

/// The registries built from an index, ready to swap into [`AppState`].
pub struct Registries {
    pub languages: Arc<LanguageRegistry>,
    pub themes: ThemeRegistry,
    pub icon_themes: IconThemeRegistry,
    /// One line per file that could not be loaded.
    pub warnings: Vec<String>,
}

/// The extensions whose installed directory is usable.
fn usable(index: &ExtensionIndex) -> impl Iterator<Item = (&String, &IndexedExtension)> {
    index
        .extensions
        .iter()
        .filter(|(_, ext)| ext.broken.is_none())
}

/// The languages of every usable extension, as the syntax crate takes them.
pub fn extension_languages(index: &ExtensionIndex) -> Vec<ExtensionLanguage> {
    usable(index)
        .flat_map(|(id, ext)| {
            ext.languages.iter().map(move |l| ExtensionLanguage {
                extension_id: id.clone(),
                name: l.name.clone(),
                dir: l.dir.clone(),
                grammar: l
                    .grammar
                    .as_ref()
                    .map(|g| (g.name.clone(), g.wasm_path.clone())),
                path_suffixes: l.path_suffixes.clone(),
                first_line_pattern: l.first_line_pattern.clone(),
                hidden: l.hidden,
            })
        })
        .collect()
}

/// `(extension id, theme family file)`, each file once. With `only`, just
/// the files that hold a theme of that name.
fn theme_files(index: &ExtensionIndex, only: Option<&str>) -> Vec<(String, PathBuf)> {
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    for (id, ext) in usable(index) {
        for theme in &ext.themes {
            if only.is_some_and(|name| name != theme.name) {
                continue;
            }
            let file = (id.clone(), theme.path.clone());
            if !files.contains(&file) {
                files.push(file);
            }
        }
    }
    files
}

/// `(extension id, icon theme file, icons root)`, each file once, in id
/// order. With `only`, just the files that hold a theme of that name.
fn icon_theme_files(index: &ExtensionIndex, only: Option<&str>) -> Vec<(String, PathBuf, PathBuf)> {
    let mut files: Vec<(String, PathBuf, PathBuf)> = Vec::new();
    for (id, ext) in usable(index) {
        for theme in &ext.icon_themes {
            if only.is_some_and(|name| name != theme.name) {
                continue;
            }
            let file = (id.clone(), theme.path.clone(), theme.icons_root.clone());
            if !files.contains(&file) {
                files.push(file);
            }
        }
    }
    files
}

fn describe_errors(errors: Vec<(PathBuf, String)>) -> impl Iterator<Item = String> {
    errors
        .into_iter()
        .map(|(path, err)| format!("{}: {err}", path.display()))
}

/// Builds every registry from `index`. Reads theme and icon-theme files and
/// query sources, so it runs on a background thread. `user_themes` is the
/// user's themes directory; it need not exist.
pub fn rebuild_registries(
    index: &ExtensionIndex,
    associations: &BTreeMap<String, String>,
    user_themes: &Path,
) -> Registries {
    let languages = Arc::new(LanguageRegistry::build(
        &extension_languages(index),
        Associations::new(associations),
    ));
    let (themes, icon_themes, warnings) = build_themes(index, user_themes);
    Registries {
        languages,
        themes,
        icon_themes,
        warnings,
    }
}

fn build_themes(
    index: &ExtensionIndex,
    user_themes: &Path,
) -> (ThemeRegistry, IconThemeRegistry, Vec<String>) {
    let user_dir = user_themes.is_dir().then_some(user_themes);
    let (themes, theme_errors) = ThemeRegistry::build(&theme_files(index, None), user_dir);
    let (icon_themes, icon_errors) = IconThemeRegistry::build(&icon_theme_files(index, None));
    let warnings = describe_errors(theme_errors)
        .chain(describe_errors(icon_errors))
        .collect();
    (themes, icon_themes, warnings)
}

/// Swaps the registries into [`AppState`], bumps `languages_generation`
/// (each open workbench then has its documents look their language up
/// again, see `Documents::languages_changed`), re-applies
/// `workbench.colorTheme` and `workbench.iconTheme`, and redraws. When
/// `r.languages` is the registry in use, nothing language-related changes
/// and open documents are not parsed again.
pub(crate) fn apply_registries(r: Registries, cx: &mut App) {
    for warning in &r.warnings {
        tracing::warn!(%warning, "extension registries");
    }
    if !Arc::ptr_eq(&r.languages, &cx.global::<AppState>().languages) {
        // Associations changed meanwhile apply too; the languages are shared.
        let associations = Associations::new(&cx.default_global::<RegistryInputs>().associations);
        let languages = r.languages.with_associations(associations);
        // Grammars of uninstalled or replaced extensions are dropped; open
        // documents keep theirs until they are parsed again.
        lucerna_syntax::retain_grammars(&languages.grammar_keys());
        set_languages(languages, cx);
    }
    let state = cx.global_mut::<AppState>();
    state.themes = r.themes;
    state.icon_themes = r.icon_themes;
    resolve_icons(cx);
    reapply_configured_theme(cx);
    cx.refresh_windows();
}

fn set_languages(languages: LanguageRegistry, cx: &mut App) {
    let state = cx.global_mut::<AppState>();
    state.languages = Arc::new(languages);
    state.languages_generation += 1;
}

/// What an index says about languages: a change here needs a new language
/// registry. Install info is included since an update keeps the paths.
fn language_inputs(
    index: &ExtensionIndex,
) -> Vec<(
    &String,
    &lucerna_extension::index::InstallInfo,
    &[LanguageEntry],
)> {
    usable(index)
        .filter(|(_, ext)| !ext.languages.is_empty())
        .map(|(id, ext)| (id, &ext.info, ext.languages.as_slice()))
        .collect()
}

fn apply_themes(themes: ThemeRegistry, icon_themes: IconThemeRegistry, cx: &mut App) {
    let state = cx.global_mut::<AppState>();
    state.themes = themes;
    state.icon_themes = icon_themes;
    resolve_icons(cx);
    reapply_configured_theme(cx);
    cx.refresh_windows();
}

fn configured_icon_theme(cx: &App) -> String {
    cx.global::<AppState>()
        .settings
        .get("workbench.iconTheme", None)
        .unwrap_or_else(|| "Lucide".to_string())
}

fn configured_associations(cx: &App) -> BTreeMap<String, String> {
    cx.global::<AppState>()
        .settings
        .get("files.associations", None)
        .unwrap_or_default()
}

/// Makes the icon theme named by `workbench.iconTheme` active (Lucide when
/// it is not installed; the setting is kept). While an icon theme picker
/// previews, the previewed theme stays (it is looked up again by name in
/// the registry, so a swapped registry cannot leave it stale).
pub(crate) fn resolve_icons(cx: &mut App) {
    if icon_previewing(cx) {
        let state = cx.global_mut::<AppState>();
        let shown = state.icons.name().to_string();
        state.icons = state.icon_themes.resolve(&shown);
        return;
    }
    let name = configured_icon_theme(cx);
    let state = cx.global_mut::<AppState>();
    state.icons = state.icon_themes.resolve(&name);
}

/// The index the registries are built from: the service's, or an empty one
/// when no service runs.
fn current_index(cx: &App) -> Arc<ExtensionIndex> {
    cx.try_global::<ExtensionService>()
        .map_or_else(Default::default, |s| s.index.clone())
}

fn user_themes_dir(cx: &App) -> PathBuf {
    cx.try_global::<ConfigState>()
        .map(|s| s.paths().themes_dir())
        .unwrap_or_default()
}

/// What the registries in [`AppState`] were last built from, and the
/// rebuild in flight.
#[derive(Default)]
struct RegistryInputs {
    /// `files.associations` of the language registry in use.
    associations: BTreeMap<String, String>,
    /// The index the language registry in use was built from.
    languages_index: Arc<ExtensionIndex>,
    /// Bumped per scheduled build; an older build's result is dropped.
    generation: u64,
    task: Option<Task<()>>,
}

impl gpui::Global for RegistryInputs {}

/// Rebuilds every registry on the background executor, then swaps them in.
/// The language registry is swapped only when the index changed what it
/// says about languages. A later call supersedes one still running.
fn schedule_rebuild(cx: &mut App) {
    let index = current_index(cx);
    let user_themes = user_themes_dir(cx);
    let inputs = cx.default_global::<RegistryInputs>();
    let associations = inputs.associations.clone();
    inputs.generation += 1;
    let generation = inputs.generation;
    let built_from = index.clone();
    let work = cx.background_executor().spawn(async move {
        catch_panic("rebuild extension registries", || {
            rebuild_registries(&index, &associations, &user_themes)
        })
    });
    let task = cx.spawn(async move |cx| {
        let Ok(registries) = work.await else {
            return;
        };
        cx.update(|cx| {
            let inputs = cx.global_mut::<RegistryInputs>();
            if inputs.generation != generation {
                return;
            }
            let mut registries = registries;
            if language_inputs(&inputs.languages_index) == language_inputs(&built_from) {
                // No language changed: keep the registry in use.
                registries.languages = cx.global::<AppState>().languages.clone();
            } else {
                inputs.languages_index = built_from;
            }
            apply_registries(registries, cx);
        });
    });
    cx.global_mut::<RegistryInputs>().task = Some(task);
}

/// Applies `files.associations` when it changed. The languages are shared
/// with the registry in use, so nothing is rebuilt or recompiled.
fn apply_associations(cx: &mut App) {
    let associations = configured_associations(cx);
    let inputs = cx.default_global::<RegistryInputs>();
    if inputs.associations == associations {
        return;
    }
    inputs.associations = associations.clone();
    let languages = cx
        .global::<AppState>()
        .languages
        .with_associations(Associations::new(&associations));
    set_languages(languages, cx);
}

/// After settings changed: applies `files.associations` and resolves
/// `workbench.iconTheme` again.
pub(crate) fn settings_changed(cx: &mut App) {
    apply_associations(cx);
    resolve_icons(cx);
}

/// After the first load of the configuration files: applies
/// `files.associations` (so restored tabs open with the right language),
/// resolves `workbench.iconTheme` reading only the icon theme file it
/// names, then loads every extension theme and icon theme in the
/// background.
pub(crate) fn config_loaded(cx: &mut App) {
    apply_associations(cx);
    let index = current_index(cx);
    let icon_theme = configured_icon_theme(cx);
    let files = icon_theme_files(&index, Some(&icon_theme));
    if !files.is_empty() {
        let (icon_themes, errors) = IconThemeRegistry::build(&files);
        for warning in describe_errors(errors) {
            tracing::warn!(%warning, "icon theme");
        }
        cx.global_mut::<AppState>().icon_themes = icon_themes;
    }
    resolve_icons(cx);
    let has_themes = usable(&index).any(|(_, e)| !e.themes.is_empty() || !e.icon_themes.is_empty());
    if has_themes {
        load_all_themes(index, cx);
    }
}

/// Loads every extension theme and icon theme (and the user's themes) in
/// the background. Dropped when a full rebuild is scheduled meanwhile.
fn load_all_themes(index: Arc<ExtensionIndex>, cx: &mut App) {
    let user_themes = user_themes_dir(cx);
    let generation = cx.default_global::<RegistryInputs>().generation;
    let work = cx.background_executor().spawn(async move {
        catch_panic("load extension themes", || {
            build_themes(&index, &user_themes)
        })
    });
    cx.spawn(async move |cx| {
        let Ok((themes, icon_themes, warnings)) = work.await else {
            return;
        };
        cx.update(|cx| {
            if cx.global::<RegistryInputs>().generation != generation {
                return;
            }
            for warning in &warnings {
                tracing::warn!(%warning, "extension themes");
            }
            apply_themes(themes, icon_themes, cx);
        });
    })
    .detach();
}

/// The extension theme files to load with the user's themes; with `only`,
/// just those holding a theme of that name.
pub(crate) fn extension_theme_files(cx: &App, only: Option<&str>) -> Vec<(String, PathBuf)> {
    theme_files(&current_index(cx), only)
}

/// Installed extensions and the operations running on them. A GPUI
/// global set by [`ExtensionService::start`].
pub struct ExtensionService {
    store: Store,
    /// The index the registries follow.
    index: Arc<ExtensionIndex>,
    /// The store's events, when no pump forwards them (tests).
    events: Option<crossbeam_channel::Receiver<StoreEvent>>,
    /// Running operations, by extension id.
    running: HashMap<String, CancelToken>,
    progress: HashMap<String, InstallProgress>,
    errors: HashMap<String, String>,
    warnings: HashMap<String, Vec<String>>,
    /// Installs waiting for the one running (`update_all`).
    queue: VecDeque<String>,
    /// The queued install that runs.
    updating: Option<String>,
    updates: Vec<String>,
    refreshing: Option<CancelToken>,
    registry_error: Option<String>,
    _pump: Task<()>,
}

impl gpui::Global for ExtensionService {}

/// What an operation's background half returns.
type OpResult = Result<Vec<String>, StoreError>;

impl ExtensionService {
    /// Makes `store` the app's extension store. Outside tests it also
    /// watches `installed/` and forwards the store's events.
    pub fn start(store: Store, cx: &mut App) {
        let events = store.subscribe();
        // Each event has exactly one consumer, so they are handled in the
        // order the store emitted them (it emits under its reload lock): an
        // older index never replaces a newer one.
        let (events, pump) = if cx.background_executor().is_test() {
            // The test scheduler rejects wake-ups from other threads; tests
            // deliver events with `handle_event`, and operations drain them.
            (Some(events), Task::ready(()))
        } else {
            (None, start_pump(&store, events, cx))
        };
        let index = store.index();
        cx.default_global::<RegistryInputs>().languages_index = index.clone();
        cx.set_global(ExtensionService {
            index,
            store,
            events,
            running: HashMap::new(),
            progress: HashMap::new(),
            errors: HashMap::new(),
            warnings: HashMap::new(),
            queue: VecDeque::new(),
            updating: None,
            updates: Vec::new(),
            refreshing: None,
            registry_error: None,
            _pump: pump,
        });
        refresh_updates(cx);
    }

    /// # Panics
    /// If [`ExtensionService::start`] was not called.
    pub fn global(cx: &App) -> &ExtensionService {
        cx.global::<ExtensionService>()
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    /// The stage of the install running for `id`.
    pub fn progress(&self, id: &str) -> Option<&InstallProgress> {
        self.progress.get(id)
    }

    /// Whether an operation on `id` runs or waits.
    pub fn is_busy(&self, id: &str) -> bool {
        self.running.contains_key(id) || self.queue.iter().any(|q| q == id)
    }

    /// Why the last operation on `id` failed.
    pub fn last_error(&self, id: &str) -> Option<&str> {
        self.errors.get(id).map(String::as_str)
    }

    /// The warnings of the last successful install of `id`.
    pub fn warnings(&self, id: &str) -> &[String] {
        self.warnings.get(id).map_or(&[], Vec::as_slice)
    }

    /// Installed extensions with a newer registry version.
    pub fn updates(&self) -> &[String] {
        &self.updates
    }

    pub fn is_refreshing(&self) -> bool {
        self.refreshing.is_some()
    }

    /// Why the last registry refresh failed.
    pub fn registry_error(&self) -> Option<&str> {
        self.registry_error.as_deref()
    }

    /// Applies one event of the store: a new index rebuilds the registries.
    /// What the event pump calls; tests call it directly.
    #[doc(hidden)]
    pub fn handle_event(event: StoreEvent, cx: &mut App) {
        match event {
            StoreEvent::Changed(index) => {
                cx.global_mut::<ExtensionService>().index = index;
                schedule_rebuild(cx);
            }
            StoreEvent::RegistryRefreshed => {}
        }
        refresh_updates(cx);
    }

    /// Installs or updates `id` from the registry.
    pub fn install(id: String, cx: &mut App) {
        let op_id = id.clone();
        Self::run_op(id, cx, move |store, progress, cancel| {
            store.install(&op_id, progress, cancel)
        });
    }

    pub fn uninstall(id: String, cx: &mut App) {
        let op_id = id.clone();
        Self::run_op(id, cx, move |store, _, _| {
            store.uninstall(&op_id).map(|()| Vec::new())
        });
    }

    /// Fetches `id` to read its name and description into `registry.json`.
    pub fn describe(id: String, cx: &mut App) {
        let op_id = id.clone();
        Self::run_op(id, cx, move |store, _, cancel| {
            store.describe(&op_id, cancel).map(|_| Vec::new())
        });
    }

    /// Installs every available update, one after another.
    pub fn update_all(cx: &mut App) {
        let service = cx.global_mut::<ExtensionService>();
        for id in service.updates.clone() {
            if !service.is_busy(&id) {
                service.queue.push_back(id);
            }
        }
        Self::start_queued(cx);
    }

    /// Cancels the operation on `id`, or drops it from the queue.
    pub fn cancel(id: String, cx: &mut App) {
        let service = cx.global_mut::<ExtensionService>();
        service.queue.retain(|q| *q != id);
        if let Some(token) = service.running.get(&id) {
            token.cancel();
        }
        cx.refresh_windows();
    }

    /// Fetches the registry again and recomputes the available updates.
    pub fn refresh_registry(cx: &mut App) {
        let service = cx.global_mut::<ExtensionService>();
        if service.refreshing.is_some() {
            return;
        }
        let cancel = CancelToken::default();
        service.refreshing = Some(cancel.clone());
        let store = service.store.clone();
        let work = cx.background_executor().spawn(async move {
            catch_panic("registry refresh", || store.refresh_registry(&cancel))
        });
        cx.spawn(async move |cx| {
            let result = work.await;
            cx.update(|cx| {
                let service = cx.global_mut::<ExtensionService>();
                service.refreshing = None;
                service.registry_error = match result {
                    Ok(Ok(warnings)) => {
                        for warning in warnings {
                            tracing::warn!(%warning, "registry refresh");
                        }
                        None
                    }
                    Ok(Err(err)) => Some(err.to_string()),
                    Err(panic) => Some(format!("internal error: {panic}")),
                };
                Self::drain_events(cx);
                refresh_updates(cx);
                cx.refresh_windows();
            });
        })
        .detach();
    }

    /// Refreshes the registry [`STARTUP_REFRESH_DELAY`] from now when it is
    /// stale or missing and `extensions.autoCheckUpdates` is on.
    pub(crate) fn schedule_startup_refresh(cx: &mut App) {
        if !cx.has_global::<ExtensionService>() {
            return;
        }
        let store = cx.global::<ExtensionService>().store.clone();
        cx.spawn(async move |cx| {
            cx.background_executor().timer(STARTUP_REFRESH_DELAY).await;
            let enabled = cx.update(|cx| {
                cx.global::<AppState>()
                    .settings
                    .get::<bool>("extensions.autoCheckUpdates", None)
                    .unwrap_or(true)
            });
            if !enabled {
                return;
            }
            let stale = cx
                .background_executor()
                .spawn(async move {
                    store
                        .registry()
                        .is_none_or(|r| is_stale(&r, SystemTime::now()))
                })
                .await;
            if stale {
                cx.update(Self::refresh_registry);
            }
        })
        .detach();
    }

    /// Runs `op` for `id` on the background executor unless an operation on
    /// `id` already runs; progress and the outcome come back to this global.
    fn run_op(
        id: String,
        cx: &mut App,
        op: impl FnOnce(&Store, &dyn Fn(InstallProgress), &CancelToken) -> OpResult + Send + 'static,
    ) {
        let service = cx.global_mut::<ExtensionService>();
        if service.running.contains_key(&id) {
            return;
        }
        let cancel = CancelToken::default();
        service.running.insert(id.clone(), cancel.clone());
        service.errors.remove(&id);
        let store = service.store.clone();
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<InstallProgress>();
        let work = cx.background_executor().spawn(async move {
            catch_panic("extension operation", || {
                op(
                    &store,
                    &|progress| {
                        let _ = tx.unbounded_send(progress);
                    },
                    &cancel,
                )
            })
        });
        let progress_id = id.clone();
        cx.spawn(async move |cx| {
            while let Some(progress) = rx.next().await {
                cx.update(|cx| {
                    let service = cx.global_mut::<ExtensionService>();
                    // A late report must not outlive the operation.
                    if service.running.contains_key(&progress_id) {
                        service.progress.insert(progress_id.clone(), progress);
                        cx.refresh_windows();
                    }
                });
            }
        })
        .detach();
        cx.spawn(async move |cx| {
            let result = work.await;
            cx.update(|cx| Self::finish_op(id, result, cx));
        })
        .detach();
        cx.refresh_windows();
    }

    fn finish_op(id: String, result: Result<OpResult, String>, cx: &mut App) {
        let service = cx.global_mut::<ExtensionService>();
        service.running.remove(&id);
        service.progress.remove(&id);
        if service.updating.as_ref() == Some(&id) {
            service.updating = None;
        }
        match result {
            Ok(Ok(warnings)) => {
                service.warnings.insert(id, warnings);
            }
            Ok(Err(err)) => {
                tracing::warn!(id, %err, "extension operation failed");
                service.errors.insert(id, err.to_string());
            }
            Err(panic) => {
                service
                    .errors
                    .insert(id, format!("internal error: {panic}"));
            }
        }
        Self::drain_events(cx);
        Self::start_queued(cx);
        cx.refresh_windows();
    }

    /// Starts the next queued install when no queued one runs.
    fn start_queued(cx: &mut App) {
        let service = cx.global_mut::<ExtensionService>();
        if service.updating.is_some() {
            return;
        }
        while let Some(id) = service.queue.pop_front() {
            if !service.running.contains_key(&id) {
                service.updating = Some(id.clone());
                Self::install(id, cx);
                return;
            }
        }
    }

    /// Handles the events the store emitted, when no pump does.
    fn drain_events(cx: &mut App) {
        let events: Vec<StoreEvent> = cx
            .global::<ExtensionService>()
            .events
            .as_ref()
            .map_or_else(Vec::new, |rx| rx.try_iter().collect());
        for event in events {
            Self::handle_event(event, cx);
        }
    }
}

/// Recomputes the available updates off the main thread (it reads
/// `registry.json`).
fn refresh_updates(cx: &mut App) {
    let store = cx.global::<ExtensionService>().store.clone();
    let work = cx
        .background_executor()
        .spawn(async move { catch_panic("extension updates", || store.updates()) });
    cx.spawn(async move |cx| {
        if let Ok(updates) = work.await {
            cx.update(|cx| {
                let service = cx.global_mut::<ExtensionService>();
                if service.updates != updates {
                    service.updates = updates;
                    cx.refresh_windows();
                }
            });
        }
    })
    .detach();
}

/// Watches `installed/` and forwards the store's events to the main thread.
fn start_pump(
    store: &Store,
    events: crossbeam_channel::Receiver<StoreEvent>,
    cx: &mut App,
) -> Task<()> {
    if let Err(err) = store.watch() {
        tracing::warn!(%err, "cannot watch the installed extensions");
    }
    let (tx, mut rx) = futures::channel::mpsc::unbounded();
    let spawned = try_spawn_guarded("lucerna-extension-events", move || {
        while let Ok(event) = events.recv() {
            if tx.unbounded_send(event).is_err() {
                return;
            }
        }
    });
    if let Err(err) = spawned {
        tracing::error!(%err, "cannot follow extension changes");
        return Task::ready(());
    }
    cx.spawn(async move |cx| {
        while let Some(event) = rx.next().await {
            cx.update(|cx| ExtensionService::handle_event(event, cx));
        }
    })
}
