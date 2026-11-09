//! The extension store: install, update and uninstall with atomic swaps,
//! the index and change events (spec §3.5–§3.7).

use std::ffi::OsString;
use std::fmt;
use std::fs::File;
use std::io::Write;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use notify::{RecursiveMode, Watcher};

use crate::builder::{BuildError, BuildOutput, Stage, build, resolve_subdir, unique_suffix};
use crate::disk::{MIN_FREE_BYTES, free_bytes};
use crate::git::{fetch_commit, git_available_with};
use crate::index::{self, ExtensionIndex, InstallInfo, Source, write_install_info};
use crate::lock::IdLock;
use crate::manifest::load_manifest;
use crate::process::CancelToken;
use crate::registry::{self, REGISTRY_URL, Registry, RegistryEntry};
use crate::validate::GrammarValidator;

/// Quiet period before the `installed/` watcher reloads the index.
const WATCH_DEBOUNCE: Duration = Duration::from_millis(200);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallProgress {
    pub id: String,
    pub stage: Stage,
}

#[derive(Debug, Clone)]
pub enum StoreEvent {
    Changed(Arc<ExtensionIndex>),
    RegistryRefreshed,
}

#[derive(Debug)]
pub enum StoreError {
    UnknownId(String),
    Busy(String),
    GitMissing,
    Build(BuildError),
    Registry(String),
    Io(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownId(id) => write!(f, "unknown extension `{id}`"),
            Self::Busy(id) => write!(f, "another operation on `{id}` is in progress"),
            Self::GitMissing => write!(f, "git is not installed or cannot run"),
            Self::Build(e) => write!(f, "{e}"),
            Self::Registry(e) => write!(f, "{e}"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<BuildError> for StoreError {
    fn from(e: BuildError) -> Self {
        Self::Build(e)
    }
}

/// State shared by every clone of a [`Store`].
struct Shared {
    index: Mutex<Arc<ExtensionIndex>>,
    subscribers: Mutex<Vec<Sender<StoreEvent>>>,
    watcher: Mutex<Option<notify::RecommendedWatcher>>,
}

impl Shared {
    fn emit(&self, event: StoreEvent) {
        lock(&self.subscribers).retain(|tx| tx.send(event.clone()).is_ok());
    }

    /// Replaces the index and emits `Changed` when it differs.
    fn set_index(&self, index: ExtensionIndex) {
        let index = Arc::new(index);
        {
            let mut cur = lock(&self.index);
            if **cur == *index {
                return;
            }
            *cur = index.clone();
        }
        self.emit(StoreEvent::Changed(index));
    }
}

/// Poison-tolerant lock: a panic elsewhere must not wedge the store.
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Installed extensions under one root (`.../Lucerna/extensions`).
#[derive(Clone)]
pub struct Store {
    root: PathBuf,
    remote: String,
    validator: Arc<dyn GrammarValidator>,
    wasi_sdk: Option<PathBuf>,
    git_program: OsString,
    shared: Arc<Shared>,
}

/// Ids name directories, so they must be one plain path component.
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with('.')
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// The id in a `<prefix><id>-<unique_suffix>` leftover name.
fn leftover_id<'a>(name: &'a str, prefix: &str) -> Option<&'a str> {
    let rest = name.strip_prefix(prefix)?;
    // `unique_suffix` is three dash-separated parts.
    let mut cut = rest.len();
    for _ in 0..3 {
        cut = rest[..cut].rfind('-')?;
    }
    Some(&rest[..cut])
}

fn io_err(what: &str, path: &Path, e: impl fmt::Display) -> StoreError {
    StoreError::Io(format!("{what} {}: {e}", path.display()))
}

fn remove_in_background(path: PathBuf) {
    std::thread::spawn(move || {
        let _ = std::fs::remove_dir_all(&path);
    });
}

/// Removes a staging directory that has not been moved into `installed/`.
struct StagingGuard(Option<PathBuf>);

impl Drop for StagingGuard {
    fn drop(&mut self) {
        if let Some(p) = self.0.take() {
            let _ = std::fs::remove_dir_all(p);
        }
    }
}

impl Store {
    /// Opens the store at `root`: sweeps leftovers of interrupted
    /// operations (skipping ids another process is working on) and loads
    /// the index.
    pub fn open(root: PathBuf, validator: Arc<dyn GrammarValidator>) -> Store {
        sweep_leftovers(&root);
        let index = index::load_or_rebuild(&root);
        Store {
            root,
            remote: REGISTRY_URL.to_string(),
            validator,
            wasi_sdk: None,
            git_program: OsString::from("git"),
            shared: Arc::new(Shared {
                index: Mutex::new(Arc::new(index)),
                subscribers: Mutex::new(Vec::new()),
                watcher: Mutex::new(None),
            }),
        }
    }

    /// Uses `url` as the registry repository instead of [`REGISTRY_URL`].
    pub fn with_remote(mut self, url: String) -> Store {
        self.remote = url;
        self
    }

    /// Test hook: build grammars with the wasi-sdk at `path`.
    #[doc(hidden)]
    pub fn with_wasi_sdk(mut self, path: impl Into<PathBuf>) -> Store {
        self.wasi_sdk = Some(path.into());
        self
    }

    /// Test hook: the program checked for by the "git is installed" check.
    #[doc(hidden)]
    pub fn with_git_program(mut self, path: impl Into<OsString>) -> Store {
        self.git_program = path.into();
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn index(&self) -> Arc<ExtensionIndex> {
        lock(&self.shared.index).clone()
    }

    pub fn registry(&self) -> Option<Registry> {
        registry::load_cached(&self.root)
    }

    pub fn subscribe(&self) -> Receiver<StoreEvent> {
        let (tx, rx) = crossbeam_channel::unbounded();
        lock(&self.shared.subscribers).push(tx);
        rx
    }

    fn installed_dir(&self) -> PathBuf {
        self.root.join("installed")
    }

    fn check_git(&self) -> Result<(), StoreError> {
        if git_available_with(&self.git_program) {
            Ok(())
        } else {
            Err(StoreError::GitMissing)
        }
    }

    fn check_disk(&self) -> Result<(), StoreError> {
        std::fs::create_dir_all(&self.root).map_err(|e| io_err("cannot create", &self.root, e))?;
        let free = free_bytes(&self.root).map_err(|e| io_err("cannot stat", &self.root, e))?;
        if free < MIN_FREE_BYTES {
            return Err(BuildError::DiskFull(free).into());
        }
        Ok(())
    }

    fn entry(&self, id: &str) -> Result<RegistryEntry, StoreError> {
        if !valid_id(id) {
            return Err(StoreError::UnknownId(id.to_string()));
        }
        self.registry()
            .and_then(|r| r.entries.into_iter().find(|e| e.id == id))
            .ok_or_else(|| StoreError::UnknownId(id.to_string()))
    }

    /// Opens `logs/<name>.log`, truncated or appended to.
    fn log_file(&self, name: &str, truncate: bool) -> Result<File, StoreError> {
        let dir = self.root.join("logs");
        std::fs::create_dir_all(&dir).map_err(|e| io_err("cannot create", &dir, e))?;
        let path = dir.join(format!("{name}.log"));
        let mut opts = std::fs::OpenOptions::new();
        opts.create(true);
        if truncate {
            opts.write(true).truncate(true);
        } else {
            opts.append(true);
        }
        opts.open(&path)
            .map_err(|e| io_err("cannot open", &path, e))
    }

    /// Runs `f` with panics turned into `StoreError::Io("internal error")`.
    fn guarded<T>(
        &self,
        what: &str,
        f: impl FnOnce() -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        match catch_unwind(AssertUnwindSafe(f)) {
            Ok(r) => r,
            Err(p) => {
                let msg = p
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| p.downcast_ref::<String>().cloned())
                    .unwrap_or_default();
                tracing::error!("panic during {what}: {msg}");
                Err(StoreError::Io("internal error".into()))
            }
        }
    }

    /// Refreshes the registry clone and `registry.json`; returns warnings.
    pub fn refresh_registry(&self, cancel: &CancelToken) -> Result<Vec<String>, StoreError> {
        self.guarded("registry refresh", || {
            self.check_git()?;
            let mut log = self.log_file(".registry", true)?;
            let (_, warnings) = registry::refresh(&self.root, &self.remote, cancel, &mut log)
                .map_err(|e| StoreError::Registry(e.to_string()))?;
            self.shared.emit(StoreEvent::RegistryRefreshed);
            Ok(warnings)
        })
    }

    /// Fetches the source of `id`, reads its manifest and stores its name
    /// and description in `registry.json`.
    pub fn describe(&self, id: &str, cancel: &CancelToken) -> Result<RegistryEntry, StoreError> {
        self.guarded("describe", || {
            let mut entry = self.entry(id)?;
            self.check_git()?;
            let _lock = IdLock::try_acquire(&self.root, id)?;
            self.check_disk()?;
            let mut log = self.log_file(id, false)?;
            let source = self.fetch_source(&entry, cancel, &mut log)?;
            let manifest = load_manifest(&source).map_err(BuildError::from)?;
            entry.name = Some(manifest.name);
            entry.description = manifest.description;
            if let Some(mut reg) = self.registry()
                && let Some(e) = reg.entries.iter_mut().find(|e| e.id == id)
            {
                e.name = entry.name.clone();
                e.description = entry.description.clone();
                registry::write_atomic(&self.root, &reg)
                    .map_err(|e| io_err("cannot write", &self.root.join("registry.json"), e))?;
            }
            Ok(entry)
        })
    }

    /// Fetches `work/<id>/src` at the entry's commit and applies its `path`.
    fn fetch_source(
        &self,
        entry: &RegistryEntry,
        cancel: &CancelToken,
        log: &mut dyn Write,
    ) -> Result<PathBuf, StoreError> {
        let checkout = self.root.join("work").join(&entry.id).join("src");
        fetch_commit(&entry.url, &entry.commit, &checkout, cancel, log)
            .map_err(BuildError::from)?;
        resolve_subdir(&checkout, entry.path.as_deref()).map_err(|e| BuildError::Io(e).into())
    }

    /// Installs or updates `id` from the registry; returns build warnings.
    /// On failure the previously installed version is kept.
    pub fn install(
        &self,
        id: &str,
        progress: &dyn Fn(InstallProgress),
        cancel: &CancelToken,
    ) -> Result<Vec<String>, StoreError> {
        self.guarded("install", || {
            let entry = self.entry(id)?;
            self.check_git()?;
            let _lock = IdLock::try_acquire(&self.root, id)?;
            self.check_disk()?;
            let mut log = self.log_file(id, true)?;
            let r = self.install_locked(&entry, progress, cancel, &mut log);
            if let Err(e) = &r {
                let _ = writeln!(log, "install failed: {e}");
            }
            r
        })
    }

    fn install_locked(
        &self,
        entry: &RegistryEntry,
        progress: &dyn Fn(InstallProgress),
        cancel: &CancelToken,
        log: &mut dyn Write,
    ) -> Result<Vec<String>, StoreError> {
        let id = entry.id.as_str();
        progress(InstallProgress {
            id: id.to_string(),
            stage: Stage::Fetching,
        });
        let source = self.fetch_source(entry, cancel, log)?;
        let out = build(
            &source,
            &self.root,
            id,
            Some(&entry.version),
            self.validator.as_ref(),
            self.wasi_sdk.as_deref(),
            cancel,
            log,
            &|stage| {
                progress(InstallProgress {
                    id: id.to_string(),
                    stage,
                })
            },
        )?;
        let info = InstallInfo::new(
            entry.commit.clone(),
            out.manifest.version.clone(),
            Source::Registry,
        );
        self.commit_install(id, out, &info)
    }

    /// Builds the extension in `dir` and installs a copy of it with
    /// `source: "dev"`. No version check and no fetch.
    pub fn install_dev(
        &self,
        dir: &Path,
        progress: &dyn Fn(InstallProgress),
        cancel: &CancelToken,
    ) -> Result<Vec<String>, StoreError> {
        self.guarded("dev install", || {
            let manifest = load_manifest(dir).map_err(BuildError::from)?;
            let id = manifest.id.clone();
            if !valid_id(&id) {
                return Err(StoreError::Build(BuildError::Io(format!(
                    "invalid extension id `{id}`"
                ))));
            }
            if !manifest.grammars.is_empty() {
                self.check_git()?;
            }
            let _lock = IdLock::try_acquire(&self.root, &id)?;
            self.check_disk()?;
            let mut log = self.log_file(&id, true)?;
            let r = build(
                dir,
                &self.root,
                &id,
                None,
                self.validator.as_ref(),
                self.wasi_sdk.as_deref(),
                cancel,
                &mut log,
                &|stage| {
                    progress(InstallProgress {
                        id: id.clone(),
                        stage,
                    })
                },
            )
            .map_err(StoreError::from)
            .and_then(|out| {
                let info =
                    InstallInfo::new(String::new(), out.manifest.version.clone(), Source::Dev);
                self.commit_install(&id, out, &info)
            });
            if let Err(e) = &r {
                let _ = writeln!(log, "install failed: {e}");
            }
            r
        })
    }

    /// Writes `lucerna.json` into the staging dir and swaps it into
    /// `installed/<id>` (the caller holds the id lock).
    fn commit_install(
        &self,
        id: &str,
        out: BuildOutput,
        info: &InstallInfo,
    ) -> Result<Vec<String>, StoreError> {
        let mut staging = StagingGuard(Some(out.staging.clone()));
        write_install_info(&out.staging, info)
            .map_err(|e| io_err("cannot write lucerna.json in", &out.staging, e))?;
        let installed = self.installed_dir();
        std::fs::create_dir_all(&installed).map_err(|e| io_err("cannot create", &installed, e))?;
        let target = installed.join(id);
        let old = installed.join(format!(".old-{id}-{}", unique_suffix()));
        let had_old = target.exists();
        if had_old {
            std::fs::rename(&target, &old).map_err(|e| io_err("cannot move", &target, e))?;
        }
        if let Err(e) = std::fs::rename(&out.staging, &target) {
            if had_old {
                let _ = std::fs::rename(&old, &target);
            }
            return Err(io_err("cannot install into", &target, e));
        }
        staging.0 = None;
        if had_old {
            remove_in_background(old);
        }
        self.reindex();
        Ok(out.warnings)
    }

    /// Rebuilds and writes `index.json`, then emits `Changed`.
    fn reindex(&self) {
        let index = index::rebuild(&self.installed_dir());
        if let Err(e) = index::write(&self.root, &index) {
            tracing::warn!("cannot write index.json: {e}");
        }
        self.shared.set_index(index);
    }

    /// Removes `installed/<id>`; `work/<id>` stays as a cache.
    pub fn uninstall(&self, id: &str) -> Result<(), StoreError> {
        self.guarded("uninstall", || {
            if !valid_id(id) {
                return Err(StoreError::UnknownId(id.to_string()));
            }
            let _lock = IdLock::try_acquire(&self.root, id)?;
            let installed = self.installed_dir();
            let target = installed.join(id);
            if !target.is_dir() {
                return Err(StoreError::UnknownId(id.to_string()));
            }
            let trash = installed.join(format!(".trash-{id}-{}", unique_suffix()));
            std::fs::rename(&target, &trash).map_err(|e| io_err("cannot move", &target, e))?;
            self.reindex();
            remove_in_background(trash);
            Ok(())
        })
    }

    /// Registry-installed ids whose registry version or commit differs from
    /// the installed one.
    pub fn updates(&self) -> Vec<String> {
        let Some(reg) = self.registry() else {
            return Vec::new();
        };
        let index = self.index();
        reg.entries
            .iter()
            .filter(|e| {
                index.extensions.get(&e.id).is_some_and(|x| {
                    x.broken.is_none()
                        && x.info.source == Source::Registry
                        && (x.info.version != e.version || x.info.commit != e.commit)
                })
            })
            .map(|e| e.id.clone())
            .collect()
    }

    /// Deletes `work/` (fetched sources and cached grammar builds).
    pub fn clear_build_cache(&self) -> Result<(), StoreError> {
        let work = self.root.join("work");
        match std::fs::remove_dir_all(&work) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(io_err("cannot remove", &work, e)),
        }
    }

    /// Watches `installed/` (200 ms debounce) so changes made by another
    /// process reload the index and emit `Changed`. Idempotent.
    pub fn watch(&self) -> notify::Result<()> {
        let mut slot = lock(&self.shared.watcher);
        if slot.is_some() {
            return Ok(());
        }
        let installed = self.installed_dir();
        std::fs::create_dir_all(&installed)?;
        let (tx, rx) = crossbeam_channel::unbounded::<()>();
        let mut watcher =
            notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                if res.is_ok() {
                    let _ = tx.send(());
                }
            })?;
        watcher.watch(&installed, RecursiveMode::NonRecursive)?;
        *slot = Some(watcher);
        let weak = Arc::downgrade(&self.shared);
        let root = self.root.clone();
        std::thread::Builder::new()
            .name("lucerna-extensions-watch".into())
            .spawn(move || watch_loop(rx, weak, root))
            .map_err(notify::Error::io)?;
        Ok(())
    }
}

/// Debounces watcher events and reloads the index from disk. Ends when the
/// store (and with it the watcher, which owns the sender) is dropped.
fn watch_loop(rx: Receiver<()>, shared: Weak<Shared>, root: PathBuf) {
    while rx.recv().is_ok() {
        loop {
            match rx.recv_timeout(WATCH_DEBOUNCE) {
                Ok(()) => {}
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
        let Some(shared) = shared.upgrade() else {
            return;
        };
        let reloaded = catch_unwind(AssertUnwindSafe(|| index::load_or_rebuild(&root)));
        match reloaded {
            Ok(index) => shared.set_index(index),
            Err(_) => tracing::error!("panic while reloading the extension index"),
        }
    }
}

/// Removes `installed/.old-*`, `installed/.trash-*` and `staging/*` left by
/// interrupted operations, except those of an id whose lock is held (an
/// operation running in another process).
fn sweep_leftovers(root: &Path) {
    let mut doomed = Vec::new();
    let installed = root.join("installed");
    if let Ok(rd) = std::fs::read_dir(&installed) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if let Some(id) = leftover_id(&name, ".old-").or_else(|| leftover_id(&name, ".trash-"))
            {
                doomed.push((id.to_string(), e.path()));
            } else if name.starts_with(".old-") || name.starts_with(".trash-") {
                doomed.push((String::new(), e.path()));
            }
        }
    }
    if let Ok(rd) = std::fs::read_dir(root.join("staging")) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let id = leftover_id(&name, "").unwrap_or_default().to_string();
            doomed.push((id, e.path()));
        }
    }
    for (id, path) in doomed {
        // Holding the lock proves no live operation owns this leftover.
        let _lock = if valid_id(&id) {
            match IdLock::try_acquire(root, &id) {
                Ok(l) => Some(l),
                Err(_) => continue,
            }
        } else {
            None
        };
        let r = if path.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        if let Err(e) = r {
            tracing::warn!("cannot remove leftover {}: {e}", path.display());
        }
    }
}
