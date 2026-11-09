//! The extension registry: a shallow clone of zed-industries/extensions
//! joined into a flat, cached list.

use crate::git::{check_url, is_lock_error, normalize_url, remove_stale_locks, run_git};
use crate::process::{CancelToken, ProcessError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use crate::lock::{lock_registry, lock_registry_clone};

pub const REGISTRY_URL: &str = "https://github.com/zed-industries/extensions";
pub const REGISTRY_BRANCH: &str = "main";
pub const STALE_AFTER: Duration = Duration::from_secs(24 * 3600);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryEntry {
    pub id: String,
    pub version: String,
    pub url: String,
    pub commit: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Registry {
    pub fetched_at: SystemTime,
    /// Sorted by id.
    pub entries: Vec<RegistryEntry>,
}

#[derive(Debug)]
pub enum RegistryError {
    Process(ProcessError),
    Io(std::io::Error),
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Process(e) => write!(f, "registry refresh failed: {e}"),
            Self::Io(e) => write!(f, "registry refresh failed: {e}"),
        }
    }
}

impl std::error::Error for RegistryError {}

impl From<ProcessError> for RegistryError {
    fn from(e: ProcessError) -> Self {
        Self::Process(e)
    }
}

impl From<std::io::Error> for RegistryError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Parses `.gitmodules` into submodule path -> url (also keyed by name).
fn parse_gitmodules(src: &str) -> HashMap<String, String> {
    let mut urls = HashMap::new();
    let mut name: Option<String> = None;
    let mut path: Option<String> = None;
    let mut url: Option<String> = None;
    let mut flush =
        |name: &mut Option<String>, path: &mut Option<String>, url: &mut Option<String>| {
            if let Some(u) = url.take() {
                if let Some(p) = path.take() {
                    urls.insert(p, u.clone());
                }
                if let Some(n) = name.take() {
                    urls.entry(n).or_insert(u);
                }
            }
            *name = None;
            *path = None;
        };
    for line in src.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("[submodule") {
            flush(&mut name, &mut path, &mut url);
            name = rest
                .trim()
                .trim_end_matches(']')
                .trim()
                .trim_matches('"')
                .to_string()
                .into();
        } else if let Some((k, v)) = line.split_once('=') {
            match k.trim() {
                "path" => path = Some(v.trim().to_string()),
                "url" => url = Some(v.trim().to_string()),
                _ => {}
            }
        }
    }
    flush(&mut name, &mut path, &mut url);
    urls
}

/// Parses `git ls-tree` output into gitlink path -> commit.
fn parse_ls_tree(src: &str) -> HashMap<String, String> {
    src.lines()
        .filter_map(|l| {
            let (meta, path) = l.split_once('\t')?;
            let mut it = meta.split_whitespace();
            let (mode, kind, sha) = (it.next()?, it.next()?, it.next()?);
            (mode == "160000" && kind == "commit").then(|| (path.to_string(), sha.to_string()))
        })
        .collect()
}

/// Joins `extensions.toml`, `.gitmodules` and `git ls-tree HEAD extensions/`.
/// Entries missing a url or commit are skipped with a warning.
pub fn parse_parts(
    extensions_toml: &str,
    gitmodules: &str,
    ls_tree: &str,
) -> (Vec<RegistryEntry>, Vec<String>) {
    let mut warnings = Vec::new();
    let table: toml::Table = match extensions_toml.parse() {
        Ok(t) => t,
        Err(e) => {
            warnings.push(format!("extensions.toml is invalid: {e}"));
            return (Vec::new(), warnings);
        }
    };
    let urls = parse_gitmodules(gitmodules);
    let commits = parse_ls_tree(ls_tree);
    let mut entries = Vec::new();
    for (id, value) in &table {
        let Some(t) = value.as_table() else {
            warnings.push(format!("{id}: not a table, skipped"));
            continue;
        };
        let get = |k: &str| t.get(k).and_then(|v| v.as_str());
        let (Some(submodule), Some(version)) = (get("submodule"), get("version")) else {
            warnings.push(format!("{id}: missing submodule or version, skipped"));
            continue;
        };
        let Some(url) = urls.get(submodule) else {
            warnings.push(format!("{id}: no url in .gitmodules, skipped"));
            continue;
        };
        let Some(commit) = commits.get(submodule) else {
            warnings.push(format!("{id}: no commit in tree, skipped"));
            continue;
        };
        entries.push(RegistryEntry {
            id: id.clone(),
            version: version.to_string(),
            url: normalize_url(url),
            commit: commit.clone(),
            path: get("path").map(str::to_string),
            name: None,
            description: None,
        });
    }
    entries.sort_by(|a, b| a.id.cmp(&b.id));
    (entries, warnings)
}

/// Reads `registry.json`; a corrupt file is deleted and yields `None`.
pub fn load_cached(root: &Path) -> Option<Registry> {
    let path = root.join("registry.json");
    let bytes = std::fs::read(&path).ok()?;
    match serde_json::from_slice(&bytes) {
        Ok(r) => Some(r),
        Err(e) => {
            tracing::warn!("discarding corrupt {}: {e}", path.display());
            let _ = std::fs::remove_file(&path);
            None
        }
    }
}

pub fn is_stale(reg: &Registry, now: SystemTime) -> bool {
    now.duration_since(reg.fetched_at)
        .map(|age| age > STALE_AFTER)
        .unwrap_or(false)
}

fn git_show(
    repo: &Path,
    spec: &str,
    cancel: &CancelToken,
    log: &mut dyn Write,
) -> Result<String, ProcessError> {
    let o = run_git(Some(repo), &["show", spec], false, cancel, log)?;
    Ok(String::from_utf8_lossy(&o.stdout).into_owned())
}

type Sources = (String, String, String);

fn read_sources(
    repo: &Path,
    cancel: &CancelToken,
    log: &mut dyn Write,
) -> Result<Sources, ProcessError> {
    let toml = git_show(repo, "HEAD:extensions.toml", cancel, log)?;
    let gitmodules = git_show(repo, "HEAD:.gitmodules", cancel, log)?;
    let ls = run_git(
        Some(repo),
        &["ls-tree", "HEAD", "--", "extensions/"],
        false,
        cancel,
        log,
    )?;
    Ok((
        toml,
        gitmodules,
        String::from_utf8_lossy(&ls.stdout).into_owned(),
    ))
}

fn update_clone(
    repo: &Path,
    allow_file_urls: bool,
    cancel: &CancelToken,
    log: &mut dyn Write,
) -> Result<Sources, ProcessError> {
    run_git(
        Some(repo),
        &["fetch", "--depth", "1", "--", "origin", REGISTRY_BRANCH],
        allow_file_urls,
        cancel,
        log,
    )?;
    run_git(
        Some(repo),
        &["update-ref", "HEAD", "FETCH_HEAD"],
        false,
        cancel,
        log,
    )?;
    read_sources(repo, cancel, log)
}

/// Whether `repo` is a usable clone: its git dir opens and `HEAD` names a
/// commit that is present. A failed fetch of a healthy clone is a network
/// or remote problem, not a reason to delete it.
fn clone_is_healthy(repo: &Path, cancel: &CancelToken, log: &mut dyn Write) -> bool {
    let git_dir = format!("--git-dir={}", repo.join(".git").display());
    run_git(
        None,
        &[
            &git_dir,
            "rev-parse",
            "--verify",
            "--quiet",
            "HEAD^{commit}",
        ],
        false,
        cancel,
        log,
    )
    .is_ok()
}

/// `remote` is already checked with [`check_url`].
fn fresh_clone(
    repo: &Path,
    remote: &str,
    allow_file_urls: bool,
    cancel: &CancelToken,
    log: &mut dyn Write,
) -> Result<Sources, ProcessError> {
    if repo.exists() {
        let _ = std::fs::remove_dir_all(repo);
    }
    let repo_s = repo.to_string_lossy();
    run_git(
        None,
        &[
            "clone",
            "--depth",
            "1",
            "--filter=blob:none",
            "--no-checkout",
            "--branch",
            REGISTRY_BRANCH,
            "--",
            remote,
            &repo_s,
        ],
        allow_file_urls,
        cancel,
        log,
    )?;
    read_sources(repo, cancel, log)
}

/// Clones or updates `root/registry` from `remote`, joins its three sources
/// and atomically writes `root/registry.json`. `remote` must pass
/// [`check_url`] (`allow_file_urls` is a test opt-in).
pub fn refresh(
    root: &Path,
    remote: &str,
    allow_file_urls: bool,
    cancel: &CancelToken,
    log: &mut dyn Write,
) -> Result<(Registry, Vec<String>), RegistryError> {
    let remote = check_url(remote, allow_file_urls)
        .map_err(|e| RegistryError::Process(ProcessError::Rejected(e)))?;
    std::fs::create_dir_all(root)?;
    let repo = root.join("registry");
    // Held until the sources are read: one refresh at a time touches the clone.
    let clone_lock = lock_registry_clone(root)?;
    let mut incremental = repo.join(".git").exists();
    if incremental {
        // Under the clone lock no Lucerna git runs here: any lock file is
        // left by a killed one (Ctrl-C, timeout) and would fail every fetch.
        remove_stale_locks(&repo);
    }
    let (toml, gitmodules, ls) = loop {
        let attempt = if incremental {
            update_clone(&repo, allow_file_urls, cancel, log)
        } else {
            fresh_clone(&repo, &remote, allow_file_urls, cancel, log)
        };
        match attempt {
            Ok(v) => break v,
            // A broken clone (e.g. a half-killed one) heals by re-cloning
            // once; a healthy one is kept (the remote is unreachable).
            // A lock-file error that survives the cleanup also re-clones.
            Err(ref e @ ProcessError::Failed { .. })
                if incremental && (is_lock_error(e) || !clone_is_healthy(&repo, cancel, log)) =>
            {
                let _ = writeln!(log, "registry clone looks broken; re-cloning");
                incremental = false;
            }
            Err(e) => return Err(e.into()),
        }
    };
    drop(clone_lock);
    let (mut entries, warnings) = parse_parts(&toml, &gitmodules, &ls);

    // From reading the descriptions to keep until the write: a describe
    // must not write between them.
    let _lock = lock_registry(root)?;
    if let Some(prev) = load_cached(root) {
        let known: HashMap<_, _> = prev
            .entries
            .into_iter()
            .map(|e| (e.id.clone(), e))
            .collect();
        for e in &mut entries {
            if let Some(p) = known.get(&e.id).filter(|p| p.commit == e.commit) {
                e.name = p.name.clone();
                e.description = p.description.clone();
            }
        }
    }

    let registry = Registry {
        fetched_at: SystemTime::now(),
        entries,
    };
    write_atomic(root, &registry)?;
    Ok((registry, warnings))
}

/// Reads `registry.json`, lets `edit` change it, and writes it back, all
/// under the registry lock (see [`crate::lock`]), so concurrent describes
/// and refreshes never lose each other's changes. `None` (nothing written)
/// when there is no registry.
pub fn update_cached<T>(
    root: &Path,
    edit: impl FnOnce(&mut Registry) -> T,
) -> std::io::Result<Option<T>> {
    let _lock = lock_registry(root)?;
    let Some(mut registry) = load_cached(root) else {
        return Ok(None);
    };
    let out = edit(&mut registry);
    write_atomic(root, &registry)?;
    Ok(Some(out))
}

/// Writes `registry.json` via a temp file in the same directory plus rename.
/// The temp name is unique per call, so concurrent writers never rename
/// each other's file.
pub fn write_atomic(root: &Path, registry: &Registry) -> std::io::Result<()> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let tmp = root.join(format!("registry.json.tmp{}-{n}", std::process::id()));
    let json = serde_json::to_vec_pretty(registry)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, root.join("registry.json")).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}
