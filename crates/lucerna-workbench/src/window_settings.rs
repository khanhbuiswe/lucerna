//! One window's settings: its folder's `.lucerna/settings.json` (the
//! workspace layer) over the app's default and user layers in
//! [`AppState::settings`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::StreamExt as _;
use gpui::{App, AppContext as _, Context, Subscription, Task};
use lucerna_settings::{SettingsError, SettingsView, WorkspaceLayer};
use lucerna_syntax::{Associations, LanguageRegistry};
use serde::de::DeserializeOwned;

use crate::AppState;
use crate::config_files::{ConfigError, ConfigFile, ConfigWatcher, read, real_path};
use crate::extensions::service::registry_with_associations;

/// The workspace layer of one window, its file, the error reading it, and
/// the watcher that reloads it. A GPUI entity each `Workbench` owns.
///
/// It notifies when the workspace layer changes and when [`AppState`]
/// does (the user layer), so a view needs to observe only it.
pub struct WindowSettings {
    /// `<root>/.lucerna/settings.json`; `None` for a single-file window.
    path: Option<PathBuf>,
    workspace: WorkspaceLayer,
    error: Option<ConfigError>,
    /// Bumped by every reload.
    generation: u64,
    watcher: Option<ConfigWatcher>,
    _pump: Task<()>,
    _user: Subscription,
    /// Notifies when the font zoom changes, so views read their size again.
    _zoom: Subscription,
    reloading: Task<()>,
    /// The window's own language registry, when its folder's
    /// `files.associations` adds to the user's; else the app's is used.
    languages: Option<Arc<LanguageRegistry>>,
    /// What the registry in use (or being built) was made from.
    languages_inputs: LanguageInputs,
    /// Bumped whenever [`WindowSettings::languages`] changes.
    languages_generation: u64,
    building: Task<()>,
}

/// The inputs of a window's language registry: the app's registry
/// (by its generation) and the merged associations, `None` when the
/// folder adds none (the app's registry is used as is).
#[derive(Debug, Clone, PartialEq, Eq)]
struct LanguageInputs {
    app_generation: u64,
    associations: Option<BTreeMap<String, String>>,
}

impl WindowSettings {
    /// Reads `<root>/.lucerna/settings.json` on this thread (the one read
    /// the UI thread waits on, spec §3) and follows changes to it. `root`
    /// is `None` for a single-file window, which has no workspace layer.
    ///
    /// The watcher is not started on GPUI's test scheduler; tests call
    /// [`WindowSettings::reload`].
    pub fn new(root: Option<&Path>, cx: &mut Context<Self>) -> Self {
        let path = root.map(|root| root.join(".lucerna").join("settings.json"));
        let (workspace, error) = match &path {
            Some(path) => match load(path) {
                Ok(layer) => (layer, None),
                Err(error) => (WorkspaceLayer::default(), Some(config_error(path, error))),
            },
            None => (WorkspaceLayer::default(), None),
        };
        if let Some(path) = &path {
            log_app_scope_keys(path, &workspace);
        }
        let (watcher, pump) = match &path {
            Some(path) if !cx.background_executor().is_test() => watch(path, cx),
            _ => (None, Task::ready(())),
        };
        let mut settings = WindowSettings {
            path,
            workspace,
            error,
            generation: 0,
            watcher,
            _pump: pump,
            _zoom: cx.observe_global::<crate::font_zoom::FontZoom>(|_, cx| cx.notify()),
            _user: cx.observe_global::<AppState>(|this, cx| {
                this.refresh_languages(cx);
                cx.notify();
            }),
            reloading: Task::ready(()),
            languages: None,
            languages_inputs: LanguageInputs {
                app_generation: 0,
                associations: None,
            },
            languages_generation: 0,
            building: Task::ready(()),
        };
        // The first registry is made here, so documents restored with the
        // window get their language at once. It reads no file and shares
        // the app's languages: only the association globs are compiled.
        if !cx.has_global::<AppState>() {
            return settings;
        }
        let inputs = settings.language_inputs(cx);
        if let Some(associations) = &inputs.associations {
            let registry = cx
                .global::<AppState>()
                .languages
                .with_associations(Associations::new(associations));
            settings.languages = Some(Arc::new(registry));
        }
        settings.languages_inputs = inputs;
        settings
    }

    /// The language registry of this window: the app's, unless the
    /// folder's `files.associations` adds to the user's; then the same
    /// languages with the merged associations (spec §6.1.3).
    pub fn languages(&self, cx: &App) -> Arc<LanguageRegistry> {
        match &self.languages {
            Some(languages) => languages.clone(),
            None => cx.global::<AppState>().languages.clone(),
        }
    }

    /// Bumped whenever [`WindowSettings::languages`] changes: open
    /// documents then look their language up again.
    pub fn languages_generation(&self) -> u64 {
        self.languages_generation
    }

    fn language_inputs(&self, cx: &App) -> LanguageInputs {
        let view = self.view(cx);
        let merged = view.get::<BTreeMap<String, String>>("files.associations", None);
        let user = view.get_trusted::<BTreeMap<String, String>>("files.associations", None);
        // A folder value that is not an object (`merged` is `None`) adds
        // nothing: the app's registry, with the user's associations.
        LanguageInputs {
            app_generation: cx.global::<AppState>().languages_generation,
            associations: merged.filter(|merged| Some(merged) != user.as_ref()),
        }
    }

    /// After the app's registry or this window's settings changed: uses
    /// the app's registry, or builds the window's own in the background.
    fn refresh_languages(&mut self, cx: &mut Context<Self>) {
        if !cx.has_global::<AppState>() {
            return;
        }
        let inputs = self.language_inputs(cx);
        if inputs == self.languages_inputs {
            return;
        }
        self.languages_inputs = inputs.clone();
        let Some(associations) = inputs.associations else {
            self.building = Task::ready(());
            self.languages = None;
            self.languages_generation += 1;
            return;
        };
        let build = registry_with_associations(Associations::new(&associations), cx);
        self.building = cx.spawn(async move |this, cx| {
            let registry = build.await;
            this.update(cx, |this, cx| {
                this.languages = Some(Arc::new(registry));
                this.languages_generation += 1;
                cx.notify();
            })
            .ok();
        });
    }

    /// The app's default and user layers with this window's workspace
    /// layer over them.
    pub fn view<'a>(&'a self, cx: &'a App) -> SettingsView<'a> {
        cx.global::<AppState>()
            .settings
            .view(self.path.as_ref().map(|_| &self.workspace))
    }

    /// See [`SettingsView::get`].
    pub fn get<T: DeserializeOwned>(
        &self,
        key: &str,
        language: Option<&str>,
        cx: &App,
    ) -> Option<T> {
        self.view(cx).get(key, language)
    }

    /// See [`SettingsView::get_trusted`].
    pub fn get_trusted<T: DeserializeOwned>(
        &self,
        key: &str,
        language: Option<&str>,
        cx: &App,
    ) -> Option<T> {
        self.view(cx).get_trusted(key, language)
    }

    /// Reads the file again in the background, then takes its layer (a
    /// missing file is an empty one) or, when it is broken, keeps the
    /// layer in use and records the error.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.path.clone() else {
            return;
        };
        let read = cx.background_spawn(async move {
            let result = load(&path);
            (path, result)
        });
        self.reloading = cx.spawn(async move |this, cx| {
            let (path, result) = read.await;
            this.update(cx, |this, cx| this.apply(&path, result, cx))
                .ok();
        });
    }

    fn apply(
        &mut self,
        path: &Path,
        result: Result<WorkspaceLayer, SettingsError>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(layer) => {
                log_app_scope_keys(path, &layer);
                self.workspace = layer;
                self.error = None;
            }
            Err(error) => self.error = Some(config_error(path, error)),
        }
        self.generation += 1;
        self.refresh_languages(cx);
        cx.notify();
    }

    /// Replaces the workspace layer with `jsonc`; for tests. On error the
    /// layer in use stays.
    pub fn set_workspace_json(
        &mut self,
        jsonc: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), SettingsError> {
        self.workspace = WorkspaceLayer::parse(jsonc)?;
        self.error = None;
        self.generation += 1;
        self.refresh_languages(cx);
        cx.notify();
        Ok(())
    }

    /// Why the file could not be read or parsed, until it next loads.
    pub fn error(&self) -> Option<&ConfigError> {
        self.error.as_ref()
    }

    /// Bumped by every reload of the workspace layer.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// `<root>/.lucerna/settings.json`; `None` for a single-file window.
    pub fn settings_path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Re-picks what the watcher follows, after `.lucerna` was made.
    pub(crate) fn rearm_watcher(&self) {
        if let Some(watcher) = &self.watcher {
            watcher.rearm();
        }
    }
}

/// The workspace layer in `path`; empty when the file does not exist.
fn load(path: &Path) -> Result<WorkspaceLayer, SettingsError> {
    let text = read(path)?;
    WorkspaceLayer::parse(text.as_deref().unwrap_or("{}"))
}

fn config_error(path: &Path, error: SettingsError) -> ConfigError {
    ConfigError {
        file: ConfigFile::WorkspaceSettings,
        path: path.to_path_buf(),
        error,
    }
}

/// App-scope keys in a folder's settings are ignored (spec §6.1.3).
fn log_app_scope_keys(path: &Path, layer: &WorkspaceLayer) {
    let keys = layer.app_scope_keys();
    if !keys.is_empty() {
        tracing::info!(
            path = %path.display(),
            ?keys,
            "ignoring application settings in workspace settings"
        );
    }
}

/// Watches `<root>/.lucerna` (through `root` until it exists) and reloads
/// on each debounced batch that touches the settings file.
fn watch(path: &Path, cx: &mut Context<WindowSettings>) -> (Option<ConfigWatcher>, Task<()>) {
    let dir = path
        .parent()
        .expect("workspace settings live in a directory")
        .to_path_buf();
    let (tx, mut rx) = futures::channel::mpsc::unbounded();
    let watcher = ConfigWatcher::start_dirs(vec![dir.clone()], move |batch| {
        let _ = tx.unbounded_send(batch);
    });
    match watcher {
        Ok(watcher) => {
            let pump = cx.spawn(async move |this, cx| {
                while let Some(batch) = rx.next().await {
                    if !touches(&dir, &batch) {
                        continue;
                    }
                    if this.update(cx, |this, cx| this.reload(cx)).is_err() {
                        return;
                    }
                }
            });
            (Some(watcher), pump)
        }
        Err(err) => {
            tracing::error!(%err, "could not watch the workspace settings");
            (None, Task::ready(()))
        }
    }
}

/// Whether `changed` names `dir/settings.json`, or `dir` or a folder
/// holding it (made, removed, or events lost).
fn touches(dir: &Path, changed: &[PathBuf]) -> bool {
    let dir = real_path(dir);
    let file = dir.join("settings.json");
    changed
        .iter()
        .map(|path| real_path(path))
        .any(|path| path == file || dir.starts_with(&path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touches_the_file_and_its_folders() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join(".lucerna");
        assert!(touches(&dir, &[dir.join("settings.json")]));
        assert!(touches(&dir, std::slice::from_ref(&dir)));
        assert!(touches(&dir, &[root.path().to_path_buf()]));
        assert!(!touches(&dir, &[root.path().join("a.txt")]));
        assert!(!touches(&dir, &[dir.join("other.json")]));
    }
}
