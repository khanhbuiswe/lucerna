//! `extension.toml` / `extension.json` loading with layout-based defaults.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize};

/// Highest `schema_version` this crate accepts.
const MAX_SCHEMA_VERSION: u32 = 1;

#[derive(Debug)]
pub enum ManifestError {
    Io(PathBuf, String),
    Parse(PathBuf, String),
    Invalid(String),
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(p, e) => write!(f, "cannot read {}: {e}", p.display()),
            Self::Parse(p, e) => write!(f, "cannot parse {}: {e}", p.display()),
            Self::Invalid(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ManifestError {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GrammarManifest {
    pub repository: String,
    #[serde(alias = "commit")]
    pub rev: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExtensionManifest {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authors: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lib: Option<toml::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub themes: Vec<PathBuf>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub icon_themes: Vec<PathBuf>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<PathBuf>,
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "one_or_many"
    )]
    pub snippets: Vec<PathBuf>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub grammars: BTreeMap<String, GrammarManifest>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub language_servers: BTreeMap<String, toml::Value>,
    /// Every remaining key (`context_servers`, `slash_commands`, ...), kept raw.
    #[serde(flatten)]
    pub other: BTreeMap<String, toml::Value>,
}

fn one_or_many<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<PathBuf>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(PathBuf),
        Many(Vec<PathBuf>),
    }
    Ok(match OneOrMany::deserialize(d)? {
        OneOrMany::One(p) => vec![p],
        OneOrMany::Many(v) => v,
    })
}

/// Manifest keys that exist in Zed but that Lucerna does not run.
const UNSUPPORTED_OTHER: [&str; 5] = [
    "context_servers",
    "slash_commands",
    "capabilities",
    "debug_adapters",
    "indexed_docs_providers",
];

impl ExtensionManifest {
    /// Features declared by the extension that Lucerna ignores.
    pub fn unsupported(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if !self.language_servers.is_empty() {
            out.push("language_servers");
        }
        if self.lib.is_some() {
            out.push("lib");
        }
        for key in UNSUPPORTED_OTHER {
            if self.other.contains_key(key) {
                out.push(key);
            }
        }
        out
    }

    fn populate_defaults(&mut self, dir: &Path) -> Result<(), ManifestError> {
        if self.schema_version == 0 {
            self.languages.clear();
            self.grammars.clear();
            self.themes.clear();
            read_grammars_dir(dir, &mut self.grammars);
        }
        for (key, list) in [
            ("themes", &mut self.themes),
            ("icon_themes", &mut self.icon_themes),
            ("languages", &mut self.languages),
            ("snippets", &mut self.snippets),
        ] {
            let mut out = Vec::new();
            for p in list.iter() {
                let n =
                    normalize_rel(p).map_err(|e| ManifestError::Invalid(format!("{key}: {e}")))?;
                push_unique(&mut out, n);
            }
            *list = out;
        }
        for (name, g) in &self.grammars {
            if let Some(p) = &g.path {
                normalize_rel(Path::new(p))
                    .map_err(|e| ManifestError::Invalid(format!("grammar `{name}` path: {e}")))?;
            }
        }
        for sub in sorted_dirs(dir.join("languages")) {
            if is_real_file(&sub.join("config.toml")) {
                push_unique(&mut self.languages, rel("languages", &sub));
            }
        }
        for f in sorted_json(dir.join("themes")) {
            push_unique(&mut self.themes, rel("themes", &f));
        }
        for f in sorted_json(dir.join("icon_themes")) {
            push_unique(&mut self.icon_themes, rel("icon_themes", &f));
        }
        if is_real_file(&dir.join("snippets.json")) {
            push_unique(&mut self.snippets, PathBuf::from("snippets.json"));
        }
        Ok(())
    }
}

/// Strip `.` components; reject empty, absolute and escaping paths.
fn normalize_rel(p: &Path) -> Result<PathBuf, String> {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::Normal(s) => out.push(s),
            _ => {
                return Err(format!(
                    "path `{}` must be relative and stay inside the extension",
                    p.display()
                ));
            }
        }
    }
    if out.as_os_str().is_empty() {
        return Err(format!("path `{}` is empty", p.display()));
    }
    Ok(out)
}

fn is_real_file(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok_and(|m| m.is_file())
}

fn rel(parent: &str, child: &Path) -> PathBuf {
    Path::new(parent).join(child.file_name().unwrap_or_default())
}

fn push_unique(v: &mut Vec<PathBuf>, p: PathBuf) {
    if !v.contains(&p) {
        v.push(p);
    }
}

/// Directory entries sorted by path; symlinks are skipped with a warning.
fn sorted_entries(dir: impl AsRef<Path>) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    v.retain(|p| {
        let link = std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink());
        if link {
            tracing::warn!("skipping symlink {}", p.display());
        }
        !link
    });
    v.sort();
    v
}

fn sorted_dirs(dir: impl AsRef<Path>) -> Vec<PathBuf> {
    sorted_entries(dir)
        .into_iter()
        .filter(|p| p.is_dir())
        .collect()
}

fn sorted_json(dir: impl AsRef<Path>) -> Vec<PathBuf> {
    sorted_entries(dir)
        .into_iter()
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "json"))
        .collect()
}

fn read_grammars_dir(dir: &Path, out: &mut BTreeMap<String, GrammarManifest>) {
    for f in sorted_entries(dir.join("grammars")) {
        if !f.is_file() || f.extension().is_none_or(|e| e != "toml") {
            continue;
        }
        let Some(name) = f.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let parsed = std::fs::read_to_string(&f)
            .map_err(|e| e.to_string())
            .and_then(|t| toml::from_str::<GrammarManifest>(&t).map_err(|e| e.to_string()));
        match parsed {
            Ok(g) => {
                out.insert(name.to_string(), g);
            }
            Err(e) => tracing::warn!("skipping grammar file {}: {e}", f.display()),
        }
    }
}

fn valid_grammar_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Read `extension.toml` (or legacy `extension.json`) from `dir`.
pub fn load_manifest(dir: &Path) -> Result<ExtensionManifest, ManifestError> {
    let toml_path = dir.join("extension.toml");
    let json_path = dir.join("extension.json");
    let mut m = if toml_path.is_file() {
        parse_toml(&toml_path)?
    } else if json_path.is_file() {
        parse_json(&json_path, dir)?
    } else {
        return Err(ManifestError::Invalid(format!(
            "no extension.toml or extension.json in {}",
            dir.display()
        )));
    };
    if m.schema_version > MAX_SCHEMA_VERSION {
        return Err(ManifestError::Invalid(format!(
            "unsupported schema_version {} (maximum {MAX_SCHEMA_VERSION})",
            m.schema_version
        )));
    }
    m.populate_defaults(dir)?;
    if let Some(bad) = m.grammars.keys().find(|n| !valid_grammar_name(n)) {
        return Err(ManifestError::Invalid(format!(
            "invalid grammar name `{bad}` (expected [a-z0-9_]+)"
        )));
    }
    Ok(m)
}

fn parse_toml(path: &Path) -> Result<ExtensionManifest, ManifestError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| ManifestError::Io(path.to_path_buf(), e.to_string()))?;
    let table: toml::Table = toml::from_str(&text)
        .map_err(|e| ManifestError::Parse(path.to_path_buf(), e.to_string()))?;
    for key in ["id", "name", "version", "schema_version"] {
        if !table.contains_key(key) {
            return Err(ManifestError::Invalid(format!(
                "{}: missing required key `{key}`",
                path.display()
            )));
        }
    }
    table
        .try_into()
        .map_err(|e: toml::de::Error| ManifestError::Parse(path.to_path_buf(), e.to_string()))
}

fn parse_json(path: &Path, dir: &Path) -> Result<ExtensionManifest, ManifestError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| ManifestError::Io(path.to_path_buf(), e.to_string()))?;
    let mut value: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| ManifestError::Parse(path.to_path_buf(), e.to_string()))?;
    strip_nulls(&mut value);
    let mut m: ExtensionManifest = serde_json::from_value(value)
        .map_err(|e| ManifestError::Parse(path.to_path_buf(), e.to_string()))?;
    m.id = dir
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    m.schema_version = 0;
    if m.name.is_empty() {
        m.name = m.id.clone();
    }
    Ok(m)
}

/// TOML has no null; drop JSON nulls recursively (array elements too).
fn strip_nulls(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(m) => {
            m.retain(|_, x| !x.is_null());
            m.values_mut().for_each(strip_nulls);
        }
        serde_json::Value::Array(a) => {
            a.retain(|x| !x.is_null());
            a.iter_mut().for_each(strip_nulls);
        }
        _ => {}
    }
}
