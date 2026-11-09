//! `index.json`: what is installed, rebuilt from `installed/*` (spec §3.7).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::builder::unique_suffix;
use crate::language_config::{load_language_config, query_files};
use crate::lock::lock_index;
use crate::manifest::{ExtensionManifest, load_manifest};
use crate::snippets::check_snippets;
use crate::validate::{check_icon_theme_json, check_theme_json};

/// Install metadata file inside each `installed/<id>/`.
pub const INSTALL_INFO_FILE: &str = "lucerna.json";
const INDEX_FILE: &str = "index.json";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    #[default]
    Registry,
    Dev,
}

/// Contents of `lucerna.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallInfo {
    /// Commit of the extension repository (empty for dev installs).
    pub commit: String,
    pub version: String,
    /// Seconds since the Unix epoch.
    pub installed_at: u64,
    pub source: Source,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrammarRef {
    pub name: String,
    pub wasm_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanguageEntry {
    pub name: String,
    /// The installed language directory (holds `config.toml` and queries).
    pub dir: PathBuf,
    pub grammar: Option<GrammarRef>,
    pub path_suffixes: Vec<String>,
    pub first_line_pattern: Option<String>,
    pub hidden: bool,
    /// Query kinds present (`highlights`, `injections`, ...).
    pub queries: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeEntry {
    pub name: String,
    /// `"light"` or `"dark"`.
    pub appearance: String,
    /// The theme family file.
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IconThemeEntry {
    pub name: String,
    pub appearance: String,
    pub path: PathBuf,
    /// Icon paths in the theme are relative to this directory.
    pub icons_root: PathBuf,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct IndexedExtension {
    pub manifest: ExtensionManifest,
    pub info: InstallInfo,
    pub languages: Vec<LanguageEntry>,
    pub themes: Vec<ThemeEntry>,
    pub icon_themes: Vec<IconThemeEntry>,
    /// Valid snippet count per language (`"*"` for global snippets).
    pub snippets: BTreeMap<String, usize>,
    /// Why the installed directory is unusable, when it is.
    pub broken: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExtensionIndex {
    /// Keyed by the `installed/` directory name (the extension id).
    pub extensions: BTreeMap<String, IndexedExtension>,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl InstallInfo {
    pub fn new(commit: String, version: String, source: Source) -> Self {
        Self {
            commit,
            version,
            installed_at: now_secs(),
            source,
        }
    }
}

/// Reads `lucerna.json` in `dir`.
pub fn read_install_info(dir: &Path) -> Result<InstallInfo, String> {
    let path = dir.join(INSTALL_INFO_FILE);
    let bytes =
        std::fs::read(&path).map_err(|e| format!("cannot read {INSTALL_INFO_FILE}: {e}"))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("cannot parse {INSTALL_INFO_FILE}: {e}"))
}

/// Writes `lucerna.json` into `dir`.
pub fn write_install_info(dir: &Path, info: &InstallInfo) -> std::io::Result<()> {
    let json = serde_json::to_vec_pretty(info).map_err(std::io::Error::other)?;
    std::fs::write(dir.join(INSTALL_INFO_FILE), json)
}

/// Indexes every non-dot directory in `installed`. A directory whose
/// manifest or install info cannot be read is marked `broken`.
pub fn rebuild(installed: &Path) -> ExtensionIndex {
    let mut index = ExtensionIndex::default();
    let Ok(rd) = std::fs::read_dir(installed) else {
        return index;
    };
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || !entry.path().is_dir() {
            continue;
        }
        index
            .extensions
            .insert(name.clone(), index_one(&entry.path(), &name));
    }
    index
}

fn index_one(dir: &Path, id: &str) -> IndexedExtension {
    let manifest = match load_manifest(dir) {
        Ok(m) => m,
        Err(e) => {
            return IndexedExtension {
                manifest: ExtensionManifest {
                    id: id.to_string(),
                    name: id.to_string(),
                    ..Default::default()
                },
                info: read_install_info(dir).unwrap_or_default(),
                broken: Some(format!("extension.toml: {e}")),
                ..Default::default()
            };
        }
    };
    let (info, broken) = match read_install_info(dir) {
        Ok(i) => (i, None),
        Err(e) => (InstallInfo::default(), Some(e)),
    };
    let mut ext = IndexedExtension {
        info,
        broken,
        ..Default::default()
    };

    for rel in &manifest.languages {
        let ldir = dir.join(rel);
        let cfg = match load_language_config(&ldir) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("{id}: language {} skipped: {e}", rel.display());
                continue;
            }
        };
        let grammar = cfg.grammar.as_ref().and_then(|g| {
            let wasm_path = dir.join("grammars").join(format!("{g}.wasm"));
            wasm_path.is_file().then(|| GrammarRef {
                name: g.clone(),
                wasm_path,
            })
        });
        let queries = query_files(&ldir).0.into_iter().map(|(k, _)| k).collect();
        ext.languages.push(LanguageEntry {
            name: cfg.name,
            dir: ldir,
            grammar,
            path_suffixes: cfg.path_suffixes,
            first_line_pattern: cfg.first_line_pattern,
            hidden: cfg.hidden,
            queries,
        });
    }

    for rel in &manifest.themes {
        let path = dir.join(rel);
        match check_theme_json(&path) {
            Ok(names) => ext
                .themes
                .extend(names.into_iter().map(|(name, appearance)| ThemeEntry {
                    name,
                    appearance,
                    path: path.clone(),
                })),
            Err(e) => tracing::warn!("{id}: theme skipped: {e}"),
        }
    }

    for rel in &manifest.icon_themes {
        let path = dir.join(rel);
        match check_icon_theme_json(&path, dir) {
            Ok((names, _)) => {
                ext.icon_themes
                    .extend(names.into_iter().map(|(name, appearance)| IconThemeEntry {
                        name,
                        appearance,
                        path: path.clone(),
                        icons_root: dir.to_path_buf(),
                    }))
            }
            Err(e) => tracing::warn!("{id}: icon theme skipped: {e}"),
        }
    }

    for rel in &manifest.snippets {
        match check_snippets(&dir.join(rel)) {
            Ok(file) => {
                let key = file.language.unwrap_or_else(|| "*".to_string());
                *ext.snippets.entry(key).or_default() += file.valid;
            }
            Err(e) => tracing::warn!("{id}: snippets skipped: {e}"),
        }
    }

    ext.manifest = manifest;
    ext
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// Reads `<root>/index.json` as it is: no lock, no staleness check, no
/// rebuild (the fast startup path, spec §8). `None` when it is missing or
/// corrupt. Writers replace it with a rename, so a plain read is atomic.
pub fn read(root: &Path) -> Option<ExtensionIndex> {
    let bytes = std::fs::read(root.join(INDEX_FILE)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Reads `<root>/index.json`. It is rebuilt (and written back) when it is
/// missing or corrupt, or when `installed/` changed after it was written.
/// Holds the cross-process index lock while it reads, rebuilds or writes.
pub fn load_or_rebuild(root: &Path) -> ExtensionIndex {
    let _lock =
        lock_index(root).inspect_err(|e| tracing::warn!("cannot lock the extension index: {e}"));
    let path = root.join(INDEX_FILE);
    let installed = root.join("installed");
    let fresh = match (mtime(&path), mtime(&installed)) {
        (Some(i), Some(d)) => d <= i,
        _ => false,
    };
    if fresh && let Ok(bytes) = std::fs::read(&path) {
        match serde_json::from_slice(&bytes) {
            Ok(index) => return index,
            Err(e) => tracing::warn!("rebuilding corrupt {}: {e}", path.display()),
        }
    }
    rebuild_and_write(root)
}

/// Rebuilds the index from `installed/` and writes it. The caller holds
/// the index lock.
fn rebuild_and_write(root: &Path) -> ExtensionIndex {
    let index = rebuild(&root.join("installed"));
    if let Err(e) = write(root, &index) {
        tracing::warn!("cannot write {}: {e}", root.join(INDEX_FILE).display());
    }
    index
}

/// [`rebuild`] plus [`write`] under the cross-process index lock, so a
/// snapshot never overwrites a newer one written by another process.
pub fn reindex(root: &Path) -> ExtensionIndex {
    let _lock =
        lock_index(root).inspect_err(|e| tracing::warn!("cannot lock the extension index: {e}"));
    rebuild_and_write(root)
}

/// Removes `index.json.tmp-*` files left by an interrupted write.
pub fn sweep_temp_files(root: &Path) {
    // Under the lock, no other process is between its write and rename.
    let _lock = lock_index(root);
    let prefix = format!("{INDEX_FILE}.tmp-");
    let Ok(rd) = std::fs::read_dir(root) else {
        return;
    };
    for e in rd.flatten() {
        if e.file_name().to_string_lossy().starts_with(&prefix) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Atomically writes `<root>/index.json` (temp file plus rename).
pub fn write(root: &Path, index: &ExtensionIndex) -> std::io::Result<()> {
    std::fs::create_dir_all(root)?;
    let tmp = root.join(format!("{INDEX_FILE}.tmp-{}", unique_suffix()));
    let json = serde_json::to_vec_pretty(index).map_err(std::io::Error::other)?;
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, root.join(INDEX_FILE)).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}
