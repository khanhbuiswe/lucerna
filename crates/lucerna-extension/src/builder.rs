//! Builds an extension source tree into `staging/` (spec §3.5 steps 2–6):
//! reads the manifest, compiles grammars, validates every resource, drops
//! what fails and copies the rest into the installed layout (§3.1).

use std::collections::BTreeMap;
use std::fmt;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::disk::{MIN_FREE_BYTES, free_bytes};
use crate::git::fetch_commit;
use crate::language_config::{load_language_config, query_files};
use crate::manifest::{ExtensionManifest, GrammarManifest, ManifestError, load_manifest};
use crate::process::{CancelToken, ProcessError, run};
use crate::snippets::check_snippets;
use crate::toolchain::ensure_wasi_sdk;
use crate::validate::{GrammarValidator, check_theme_json, icon_theme_files};

/// Timeout for one grammar compile.
const COMPILE_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stage {
    Fetching,
    ReadingManifest,
    PreparingToolchain,
    /// `index` is 0-based.
    BuildingGrammar {
        name: String,
        index: usize,
        total: usize,
    },
    Validating,
    Installing,
}

#[derive(Debug)]
pub enum BuildError {
    Process(ProcessError),
    Manifest(ManifestError),
    VersionMismatch { registry: String, manifest: String },
    Checksum,
    CppScanner(String),
    NoParser(String),
    NothingUsable(Vec<String>),
    DiskFull(u64),
    Io(String),
    Cancelled,
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Process(e) => write!(f, "{e}"),
            Self::Manifest(e) => write!(f, "invalid manifest: {e}"),
            Self::VersionMismatch { registry, manifest } => write!(
                f,
                "version mismatch: the registry lists {registry} but extension.toml says {manifest}"
            ),
            Self::Checksum => write!(f, "the downloaded wasi-sdk failed its checksum"),
            Self::CppScanner(g) => write!(f, "grammar `{g}`: C++ scanner is not supported"),
            Self::NoParser(g) => write!(f, "grammar `{g}`: src/parser.c is missing"),
            Self::NothingUsable(w) => write!(
                f,
                "nothing in this extension is usable ({} problem{})",
                w.len(),
                if w.len() == 1 { "" } else { "s" }
            ),
            Self::DiskFull(free) => write!(
                f,
                "not enough free disk space: {:.1} GB available, 1.2 GB needed",
                *free as f64 / 1e9
            ),
            Self::Io(e) => write!(f, "{e}"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl std::error::Error for BuildError {}

impl From<ProcessError> for BuildError {
    fn from(e: ProcessError) -> Self {
        match e {
            ProcessError::Cancelled => Self::Cancelled,
            e => Self::Process(e),
        }
    }
}

impl From<ManifestError> for BuildError {
    fn from(e: ManifestError) -> Self {
        Self::Manifest(e)
    }
}

pub struct BuildOutput {
    /// `staging/<id>-<random>/`, laid out like `installed/<id>/`.
    pub staging: PathBuf,
    /// The populated manifest, pruned to what was kept.
    pub manifest: ExtensionManifest,
    pub warnings: Vec<String>,
}

/// A process-unique suffix for temporary directory names.
pub(crate) fn unique_suffix() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(
        "{nanos:x}-{:x}-{:x}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

/// Resolves the optional subdirectory `sub` of a checkout. The result is
/// canonical and guaranteed to be inside the canonical `checkout`, even
/// through symlinked components.
pub fn resolve_subdir(checkout: &Path, sub: Option<&str>) -> Result<PathBuf, String> {
    let base = checkout
        .canonicalize()
        .map_err(|e| format!("{}: {e}", checkout.display()))?;
    let Some(sub) = sub else {
        return Ok(base);
    };
    let lexical_ok = Path::new(sub)
        .components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
    let outside = || format!("path `{sub}` is outside the checkout");
    if !lexical_ok {
        return Err(outside());
    }
    match base.join(sub).canonicalize() {
        Ok(c) if !c.starts_with(&base) => Err(outside()),
        Ok(c) if c.is_dir() => Ok(c),
        _ => Err(format!("path `{sub}` is not a directory in the checkout")),
    }
}

/// Canonical `path` when it exists inside the canonical `base`.
fn locate(base: &Path, path: &Path) -> Result<PathBuf, &'static str> {
    let c = path.canonicalize().map_err(|_| "does not exist")?;
    if c.starts_with(base) {
        Ok(c)
    } else {
        Err("is outside the extension")
    }
}

/// Removes the staging directory unless the build succeeded.
struct StagingGuard {
    path: PathBuf,
    keep: bool,
}

impl Drop for StagingGuard {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

fn check_cancel(cancel: &CancelToken) -> Result<(), BuildError> {
    if cancel.is_cancelled() {
        Err(BuildError::Cancelled)
    } else {
        Ok(())
    }
}

fn io(what: &str, path: &Path, e: impl fmt::Display) -> BuildError {
    BuildError::Io(format!("{what} {}: {e}", path.display()))
}

fn copy_into(from: &Path, staging: &Path, rel: &Path) -> Result<(), BuildError> {
    let to = staging.join(rel);
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(|e| io("cannot create", parent, e))?;
    }
    std::fs::copy(from, &to).map_err(|e| io("cannot copy to", &to, e))?;
    Ok(())
}

struct KeptLanguage {
    rel: PathBuf,
    config: PathBuf,
    queries: Vec<(String, PathBuf)>,
}

/// Builds the extension in `source_dir` into `<root>/staging/<id>-<random>/`.
/// `expected_version` is the registry version (`None` for dev installs).
/// On error (including cancellation) the staging directory is removed.
#[allow(clippy::too_many_arguments)]
pub fn build(
    source_dir: &Path,
    root: &Path,
    id: &str,
    expected_version: Option<&str>,
    validator: &dyn GrammarValidator,
    cancel: &CancelToken,
    log: &mut dyn Write,
    progress: &dyn Fn(Stage),
) -> Result<BuildOutput, BuildError> {
    progress(Stage::ReadingManifest);
    let mut manifest = load_manifest(source_dir)?;
    if let Some(v) = expected_version
        && v != manifest.version
    {
        return Err(BuildError::VersionMismatch {
            registry: v.to_string(),
            manifest: manifest.version.clone(),
        });
    }
    check_cancel(cancel)?;
    std::fs::create_dir_all(root).map_err(|e| io("cannot create", root, e))?;
    let free = free_bytes(root).map_err(|e| io("cannot stat", root, e))?;
    if free < MIN_FREE_BYTES {
        return Err(BuildError::DiskFull(free));
    }
    let src_root = source_dir
        .canonicalize()
        .map_err(|e| io("cannot read", source_dir, e))?;
    let staging_parent = root.join("staging");
    std::fs::create_dir_all(&staging_parent)
        .map_err(|e| io("cannot create", &staging_parent, e))?;
    let mut guard = StagingGuard {
        path: staging_parent.join(format!("{id}-{}", unique_suffix())),
        keep: false,
    };
    let staging = guard.path.clone();
    std::fs::create_dir(&staging).map_err(|e| io("cannot create", &staging, e))?;
    let mut warnings = Vec::new();

    // Grammars.
    let mut built = Vec::new();
    if !manifest.grammars.is_empty() {
        progress(Stage::PreparingToolchain);
        let sdk = ensure_wasi_sdk(root, cancel, log, &|| {})?;
        let total = manifest.grammars.len();
        for (index, (name, g)) in manifest.grammars.iter().enumerate() {
            progress(Stage::BuildingGrammar {
                name: name.clone(),
                index,
                total,
            });
            check_cancel(cancel)?;
            match build_grammar(&sdk, root, id, name, g, &staging, cancel, log) {
                Ok(()) => built.push(name.clone()),
                Err(BuildError::Cancelled) => return Err(BuildError::Cancelled),
                Err(e @ (BuildError::CppScanner(_) | BuildError::NoParser(_))) => {
                    warnings.push(e.to_string())
                }
                Err(e) => warnings.push(format!("grammar `{name}`: {e}")),
            }
        }
    }

    progress(Stage::Validating);
    check_cancel(cancel)?;
    let mut good: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for name in built {
        let path = staging.join("grammars").join(format!("{name}.wasm"));
        let loaded = std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|wasm| validator.load(&name, &wasm).map(|()| wasm));
        match loaded {
            Ok(wasm) => {
                good.insert(name, wasm);
            }
            Err(e) => {
                warnings.push(format!("grammar `{name}` failed to load: {e}"));
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    let mut languages = Vec::new();
    for rel in &manifest.languages {
        if let Some(l) = check_language(&src_root, source_dir, rel, &good, validator, &mut warnings)
        {
            languages.push(l);
        }
    }

    let mut themes = Vec::new();
    for rel in &manifest.themes {
        match locate(&src_root, &source_dir.join(rel)) {
            Err(why) => warnings.push(format!("theme {} {why}; skipped", rel.display())),
            Ok(path) => match check_theme_json(&path) {
                Ok(_) => themes.push((rel.clone(), path)),
                Err(e) => warnings.push(format!("theme {} dropped: {e}", rel.display())),
            },
        }
    }

    let mut icon_themes = Vec::new();
    for rel in &manifest.icon_themes {
        match locate(&src_root, &source_dir.join(rel)) {
            Err(why) => warnings.push(format!("icon theme {} {why}; skipped", rel.display())),
            Ok(path) => match icon_theme_files(&path, &src_root) {
                Ok((_, icons, w)) => {
                    warnings.extend(w);
                    icon_themes.push((rel.clone(), path, icons));
                }
                Err(e) => warnings.push(format!("icon theme {} dropped: {e}", rel.display())),
            },
        }
    }

    let mut snippets = Vec::new();
    let mut usable_snippets = 0;
    for rel in &manifest.snippets {
        match locate(&src_root, &source_dir.join(rel)) {
            Err(why) => warnings.push(format!("snippets {} {why}; skipped", rel.display())),
            Ok(path) => match check_snippets(&path) {
                Ok(file) => {
                    if !file.errors.is_empty() {
                        let n = file.errors.len();
                        warnings.push(format!(
                            "snippets {}: {n} invalid snippet{}",
                            rel.display(),
                            if n == 1 { "" } else { "s" }
                        ));
                    }
                    if file.valid > 0 {
                        usable_snippets += 1;
                    }
                    snippets.push((rel.clone(), path));
                }
                Err(e) => warnings.push(format!("snippets {} dropped: {e}", rel.display())),
            },
        }
    }

    if languages.is_empty() && themes.is_empty() && icon_themes.is_empty() && usable_snippets == 0 {
        return Err(BuildError::NothingUsable(warnings));
    }

    progress(Stage::Installing);
    check_cancel(cancel)?;
    for l in &languages {
        copy_into(&l.config, &staging, &l.rel.join("config.toml"))?;
        for (kind, path) in &l.queries {
            copy_into(path, &staging, &l.rel.join(format!("{kind}.scm")))?;
        }
    }
    for (rel, path) in &themes {
        copy_into(path, &staging, rel)?;
    }
    for (rel, path, icons) in &icon_themes {
        copy_into(path, &staging, rel)?;
        for icon in icons {
            // `icon_theme_files` checked that it resolves inside `src_root`.
            let from = src_root
                .join(icon)
                .canonicalize()
                .map_err(|e| io("cannot read", icon, e))?;
            copy_into(&from, &staging, icon)?;
        }
    }
    for (rel, path) in &snippets {
        copy_into(path, &staging, rel)?;
    }

    manifest.languages = languages.into_iter().map(|l| l.rel).collect();
    manifest.themes = themes.into_iter().map(|t| t.0).collect();
    manifest.icon_themes = icon_themes.into_iter().map(|t| t.0).collect();
    manifest.snippets = snippets.into_iter().map(|s| s.0).collect();
    manifest.grammars.retain(|name, _| good.contains_key(name));
    // Everything is now explicit; schema 0 would re-derive it on load.
    manifest.schema_version = 1;
    let text = toml::to_string(&manifest)
        .map_err(|e| BuildError::Io(format!("cannot write extension.toml: {e}")))?;
    let toml_path = staging.join("extension.toml");
    std::fs::write(&toml_path, text).map_err(|e| io("cannot write", &toml_path, e))?;

    guard.keep = true;
    Ok(BuildOutput {
        staging,
        manifest,
        warnings,
    })
}

/// Validates one language; `None` (with a warning) when it is dropped.
fn check_language(
    src_root: &Path,
    source_dir: &Path,
    rel: &Path,
    good: &BTreeMap<String, Vec<u8>>,
    validator: &dyn GrammarValidator,
    warnings: &mut Vec<String>,
) -> Option<KeptLanguage> {
    let shown = rel.display();
    let dir = match locate(src_root, &source_dir.join(rel)) {
        Ok(d) if d.is_dir() => d,
        Ok(_) => {
            warnings.push(format!("language {shown} is not a directory; skipped"));
            return None;
        }
        Err(why) => {
            warnings.push(format!("language {shown} {why}; skipped"));
            return None;
        }
    };
    let config = match locate(src_root, &dir.join("config.toml")) {
        Ok(c) => c,
        Err(why) => {
            warnings.push(format!("language {shown}: config.toml {why}; skipped"));
            return None;
        }
    };
    let cfg = match load_language_config(&dir) {
        Ok(c) => c,
        Err(e) => {
            warnings.push(format!("language {shown} dropped: {e}"));
            return None;
        }
    };
    let name = &cfg.name;
    let wasm = match &cfg.grammar {
        Some(g) => match good.get(g) {
            Some(w) => Some((g.as_str(), w)),
            None => {
                warnings.push(format!(
                    "language `{name}` dropped: grammar `{g}` is not available"
                ));
                return None;
            }
        },
        None => None,
    };
    let (files, unsupported) = query_files(&dir);
    warnings.extend(
        unsupported
            .into_iter()
            .map(|w| format!("language `{name}`: {w}")),
    );
    let mut queries = Vec::new();
    for (kind, path) in files {
        let path = match locate(src_root, &path) {
            Ok(p) => p,
            Err(why) => {
                warnings.push(format!("language `{name}`: {kind}.scm {why}; skipped"));
                continue;
            }
        };
        let Some((grammar, wasm)) = wasm else {
            warnings.push(format!(
                "language `{name}`: {kind}.scm dropped: the language has no grammar"
            ));
            continue;
        };
        let checked = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|src| validator.check_query(grammar, wasm, &kind, &src));
        match checked {
            Ok(()) => queries.push((kind, path)),
            Err(e) => warnings.push(format!("language `{name}`: {kind}.scm dropped: {e}")),
        }
    }
    Some(KeptLanguage {
        rel: rel.to_path_buf(),
        config,
        queries,
    })
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// Fetches, compiles (or reuses the cached build of) grammar `name` and
/// copies it to `<staging>/grammars/<name>.wasm`.
#[allow(clippy::too_many_arguments)]
fn build_grammar(
    sdk: &Path,
    root: &Path,
    id: &str,
    name: &str,
    g: &GrammarManifest,
    staging: &Path,
    cancel: &CancelToken,
    log: &mut dyn Write,
) -> Result<(), BuildError> {
    let gdir = root.join("work").join(id).join("grammars");
    let checkout = gdir.join(name);
    fetch_commit(&g.repository, &g.rev, &checkout, cancel, log)?;
    let checkout = checkout
        .canonicalize()
        .map_err(|e| io("cannot read", &checkout, e))?;
    let gdir = gdir
        .canonicalize()
        .map_err(|e| io("cannot read", &gdir, e))?;
    let base = resolve_subdir(&checkout, g.path.as_deref()).map_err(BuildError::Io)?;
    let src = base.join("src");
    let parser = src.join("parser.c");
    let scanner = src.join("scanner.c");
    if !parser.is_file() {
        return Err(BuildError::NoParser(name.to_string()));
    }
    let has_scanner = scanner.is_file();
    if !has_scanner && src.join("scanner.cc").exists() {
        return Err(BuildError::CppScanner(name.to_string()));
    }
    let mut inputs = vec![parser];
    if has_scanner {
        inputs.push(scanner);
    }
    for p in std::iter::once(&src).chain(&inputs) {
        if !p.canonicalize().is_ok_and(|c| c.starts_with(&checkout)) {
            return Err(BuildError::Io(format!(
                "{} is outside the grammar checkout",
                p.display()
            )));
        }
    }

    let cache = gdir.join(format!("{name}.wasm"));
    let rev_file = gdir.join(format!("{name}.rev"));
    let fresh = std::fs::read_to_string(&rev_file).is_ok_and(|r| r.trim() == g.rev)
        && mtime(&cache).is_some_and(|w| inputs.iter().all(|i| mtime(i).is_some_and(|t| w > t)));
    if fresh {
        let _ = writeln!(log, "reusing cached {}", cache.display());
    } else {
        let _ = std::fs::remove_file(&rev_file);
        let mut cmd = Command::new(sdk.join("bin/clang"));
        cmd.args([
            "-fPIC",
            "-shared",
            "-Os",
            &format!("-Wl,--export=tree_sitter_{name}"),
            "-Wl,--max-memory=4294967296",
            "-o",
        ])
        .arg(&cache)
        .arg("-I")
        .arg(&src)
        .args(&inputs);
        if let Err(e) = run(cmd, COMPILE_TIMEOUT, cancel, log) {
            let _ = std::fs::remove_file(&cache);
            return Err(e.into());
        }
        std::fs::write(&rev_file, &g.rev).map_err(|e| io("cannot write", &rev_file, e))?;
    }
    copy_into(
        &cache,
        staging,
        &Path::new("grammars").join(format!("{name}.wasm")),
    )
}
