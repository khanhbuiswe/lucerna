//! Zed icon themes (schema v0.3.0) and the built-in "Seti" and "Lucide" themes.

/// The icon theme used when settings name none, or an unknown one.
pub const DEFAULT_ICON_THEME: &str = "Seti";

pub use crate::seti::seti_icon_theme;
use crate::seti::shared_seti_theme;

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;

use crate::Appearance;
use crate::seti::SetiIndex;

/// Where an icon's pixels come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IconSource {
    /// A bundled Lucide icon, by upstream name such as `"file-code"`.
    Builtin(&'static str),
    /// An SVG inside an extension, already confined to its icons root.
    File(PathBuf),
    /// A bundled Seti glyph (an SVG name such as `"typescript"`) with its
    /// colour for a dark and for a light theme, `0xRRGGBB`.
    Glyph {
        name: &'static str,
        dark: u32,
        light: u32,
    },
}

#[derive(Debug, Clone, Default)]
struct Pair {
    collapsed: Option<IconSource>,
    expanded: Option<IconSource>,
}

impl Pair {
    fn get(&self, expanded: bool) -> Option<IconSource> {
        if expanded {
            &self.expanded
        } else {
            &self.collapsed
        }
        .clone()
    }
}

#[derive(Debug, Clone)]
pub struct IconTheme {
    pub name: String,
    pub appearance: Appearance,
    directory: Pair,
    named_directories: HashMap<String, Pair>,
    chevron: Pair,
    stems: Keys,
    suffixes: Keys,
    /// Lower-cased file-name prefixes checked before stems and suffixes.
    /// Only the built-in theme uses them (`LICENSE-MIT`, `COPYING`).
    name_prefixes: Vec<(&'static str, String)>,
    /// Built-in Lucide only: its suffix keys are extensions, so they apply
    /// only after a real extension dot (never to a whole name, a leading
    /// dot or the part before the first dot), as in Phase 1.
    extensions_only: bool,
    icons: HashMap<String, IconSource>,
    kind: Kind,
}

/// How a theme answers lookups.
#[derive(Debug, Clone)]
enum Kind {
    /// Stems, suffixes and typed icons (Lucide and extension themes).
    Mapping,
    /// The built-in Seti theme: file icons only.
    Seti(Arc<SetiIndex>),
}

/// A key table that answers an exact key first, then a case-insensitive one.
#[derive(Debug, Clone, Default)]
struct Keys {
    exact: HashMap<String, String>,
    lower: HashMap<String, String>,
}

impl Keys {
    fn new(exact: HashMap<String, String>) -> Keys {
        let mut lower = HashMap::new();
        for (k, v) in &exact {
            lower
                .entry(k.to_ascii_lowercase())
                .and_modify(|old: &mut String| {
                    // Deterministic when two keys differ only by case.
                    if v < old {
                        old.clone_from(v);
                    }
                })
                .or_insert_with(|| v.clone());
        }
        Keys { exact, lower }
    }

    fn get(&self, key: &str) -> Option<&String> {
        self.exact
            .get(key)
            .or_else(|| self.lower.get(&key.to_ascii_lowercase()))
    }
}

impl IconTheme {
    fn type_icon(&self, ty: &str) -> Option<IconSource> {
        self.icons.get(ty).cloned()
    }

    /// Stem lookup, then suffix lookup, for one key.
    fn typed(&self, key: &str, suffix_step: bool) -> Option<IconSource> {
        let suffix = if suffix_step || !self.extensions_only {
            self.suffixes.get(key)
        } else {
            None
        };
        let ty = self.stems.get(key).or(suffix)?;
        self.type_icon(ty)
    }

    /// The built-in Seti theme around a prebuilt index.
    pub(crate) fn seti(index: SetiIndex) -> IconTheme {
        IconTheme {
            name: "Seti".to_string(),
            appearance: Appearance::Dark,
            directory: Pair::default(),
            named_directories: HashMap::new(),
            chevron: Pair::default(),
            stems: Keys::default(),
            suffixes: Keys::default(),
            name_prefixes: Vec::new(),
            extensions_only: false,
            icons: HashMap::new(),
            kind: Kind::Seti(Arc::new(index)),
        }
    }

    pub fn file_icon(&self, path: &Path) -> Option<IconSource> {
        if let Kind::Seti(index) = &self.kind {
            let name = path.file_name().map(|n| n.to_string_lossy());
            return Some(index.lookup(name.as_deref().unwrap_or("")));
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let lower = name.to_ascii_lowercase();
        for (prefix, ty) in &self.name_prefixes {
            if lower.starts_with(prefix)
                && let Some(icon) = self.type_icon(ty)
            {
                return Some(icon);
            }
        }
        if let Some(icon) = self.typed(&name, false) {
            return Some(icon);
        }
        if let Some(first_dot) = name.find('.') {
            let mut rest = &name[first_dot + 1..];
            // A name like `.json` has no extension.
            let mut skip = self.extensions_only && first_dot == 0;
            loop {
                if !skip
                    && !rest.is_empty()
                    && let Some(icon) = self.typed(rest, true)
                {
                    return Some(icon);
                }
                skip = false;
                match rest.split_once('.') {
                    Some((_, tail)) => rest = tail,
                    None => break,
                }
            }
            if first_dot > 0
                && let Some(icon) = self.typed(&name[..first_dot], false)
            {
                return Some(icon);
            }
        }
        self.type_icon("default")
    }

    pub fn directory_icon(&self, name: &str, expanded: bool) -> Option<IconSource> {
        self.named_directories
            .get(name)
            .or_else(|| self.named_directories.get(&name.to_ascii_lowercase()))
            .and_then(|p| p.get(expanded))
            .or_else(|| self.directory.get(expanded))
    }

    pub fn chevron(&self, expanded: bool) -> Option<IconSource> {
        self.chevron.get(expanded)
    }
}

const SOURCE_EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "go", "c", "h", "cpp", "java", "kt",
    "swift", "rb", "php", "cs", "html", "css", "scss",
];

/// The built-in Lucide icon theme: the Phase 1 mapping as data.
pub fn lucide_icon_theme() -> IconTheme {
    let mut stems = HashMap::new();
    let mut suffixes = HashMap::new();
    for stem in [".gitignore", ".gitattributes", ".gitmodules"] {
        stems.insert(stem.to_string(), "git".to_string());
    }
    let groups: &[(&str, &[&str])] = &[
        ("lock", &["lock"]),
        ("json", &["json", "jsonc"]),
        ("markdown", &["md", "markdown"]),
        ("config", &["toml", "yaml", "yml"]),
        ("shell", &["sh", "bash", "zsh"]),
        (
            "image",
            &["png", "jpg", "jpeg", "gif", "svg", "webp", "ico"],
        ),
        ("code", SOURCE_EXTENSIONS),
    ];
    for (ty, exts) in groups {
        for ext in *exts {
            suffixes.insert((*ext).to_string(), (*ty).to_string());
        }
    }
    let builtin = |ty: &str, icon: &'static str| (ty.to_string(), IconSource::Builtin(icon));
    IconTheme {
        name: "Lucide".to_string(),
        appearance: Appearance::Dark,
        directory: Pair {
            collapsed: Some(IconSource::Builtin("folder")),
            expanded: Some(IconSource::Builtin("folder-open")),
        },
        named_directories: HashMap::new(),
        chevron: Pair {
            collapsed: Some(IconSource::Builtin("chevron-right")),
            expanded: Some(IconSource::Builtin("chevron-down")),
        },
        stems: Keys::new(stems),
        suffixes: Keys::new(suffixes),
        extensions_only: true,
        name_prefixes: vec![
            ("license", "license".to_string()),
            ("copying", "license".to_string()),
        ],
        icons: HashMap::from([
            builtin("default", "file"),
            builtin("git", "git-branch"),
            builtin("license", "scale"),
            builtin("lock", "lock"),
            builtin("json", "braces"),
            builtin("markdown", "book-open"),
            builtin("config", "settings"),
            builtin("shell", "terminal"),
            builtin("image", "image"),
            builtin("code", "file-code"),
        ]),
        kind: Kind::Mapping,
    }
}

// ---- Zed JSON schema ------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Family {
    themes: Vec<ThemeDef>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ThemeDef {
    name: String,
    appearance: String,
    directory_icons: PairDef,
    named_directory_icons: HashMap<String, PairDef>,
    chevron_icons: PairDef,
    file_stems: HashMap<String, String>,
    file_suffixes: HashMap<String, String>,
    file_icons: HashMap<String, IconDef>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct PairDef {
    collapsed: Option<String>,
    expanded: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct IconDef {
    path: String,
}

/// Confines `rel` to `root`: relative, no `..` escape, and (after symlink
/// resolution) still under the canonical root.
fn confine(root: &Path, canonical_root: &Path, rel: &str) -> Result<PathBuf, String> {
    let rel_path = Path::new(rel);
    if rel_path.is_absolute()
        || rel_path
            .components()
            .any(|c| matches!(c, Component::Prefix(_)))
    {
        return Err(format!("icon path {rel:?} is outside the extension"));
    }
    let full = root
        .join(rel_path)
        .canonicalize()
        .map_err(|e| format!("icon path {rel:?}: {e}"))?;
    if !full.starts_with(canonical_root) {
        return Err(format!("icon path {rel:?} is outside the extension"));
    }
    Ok(full)
}

struct Resolver<'a> {
    root: &'a Path,
    canonical_root: Option<PathBuf>,
    warnings: Vec<String>,
}

impl Resolver<'_> {
    fn icon(&mut self, rel: &str) -> Option<IconSource> {
        let Some(canonical) = &self.canonical_root else {
            self.warnings
                .push(format!("icon path {rel:?}: icons root is unreadable"));
            return None;
        };
        match confine(self.root, canonical, rel) {
            Ok(p) => Some(IconSource::File(p)),
            Err(w) => {
                self.warnings.push(w);
                None
            }
        }
    }

    fn pair(&mut self, def: &PairDef) -> Pair {
        Pair {
            collapsed: def.collapsed.as_deref().and_then(|p| self.icon(p)),
            expanded: def.expanded.as_deref().and_then(|p| self.icon(p)),
        }
    }
}

fn parse_file(json: &Path, root: &Path) -> Result<(Vec<IconTheme>, Vec<String>), String> {
    let text = std::fs::read_to_string(json).map_err(|e| e.to_string())?;
    let value = jsonc_parser::parse_to_serde_value(&text, &jsonc_parser::ParseOptions::default())
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "empty icon theme file".to_string())?;
    let family: Family = serde_json::from_value(value).map_err(|e| e.to_string())?;
    let mut r = Resolver {
        root,
        canonical_root: root.canonicalize().ok(),
        warnings: Vec::new(),
    };
    let mut themes = Vec::new();
    for def in family.themes {
        let appearance = match def.appearance.as_str() {
            "light" => Appearance::Light,
            _ => Appearance::Dark,
        };
        let directory = r.pair(&def.directory_icons);
        let chevron = r.pair(&def.chevron_icons);
        let named_directories = def
            .named_directory_icons
            .iter()
            .map(|(k, v)| (k.clone(), r.pair(v)))
            .collect();
        let icons = def
            .file_icons
            .iter()
            .filter_map(|(ty, d)| Some((ty.clone(), r.icon(&d.path)?)))
            .collect();
        themes.push(IconTheme {
            name: def.name,
            appearance,
            directory,
            named_directories,
            chevron,
            stems: Keys::new(def.file_stems),
            suffixes: Keys::new(def.file_suffixes),
            name_prefixes: Vec::new(),
            extensions_only: false,
            icons,
            kind: Kind::Mapping,
        });
    }
    Ok((themes, r.warnings))
}

// ---- Registry -------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct IconThemeRegistry {
    seti: Arc<IconTheme>,
    lucide: Arc<IconTheme>,
    themes: Vec<Arc<IconTheme>>,
}

impl IconThemeRegistry {
    /// Lucide plus every theme in the given extension files, in order. A
    /// later theme replaces an earlier one of the same name; nothing
    /// replaces Lucide. Returns one warning per unusable file or icon.
    pub fn build(
        files: &[(String, PathBuf, PathBuf)],
    ) -> (IconThemeRegistry, Vec<(PathBuf, String)>) {
        let seti = shared_seti_theme();
        let lucide = Arc::new(lucide_icon_theme());
        let mut themes: Vec<Arc<IconTheme>> = Vec::new();
        let mut warnings = Vec::new();
        for (_ext, json, root) in files {
            match parse_file(json, root) {
                Ok((parsed, warns)) => {
                    warnings.extend(warns.into_iter().map(|w| (json.clone(), w)));
                    for theme in parsed {
                        if theme.name == lucide.name
                            || theme.name == seti.name
                            || theme.name == "vs-seti"
                        {
                            continue;
                        }
                        themes.retain(|t| t.name != theme.name);
                        themes.push(Arc::new(theme));
                    }
                }
                Err(e) => warnings.push((json.clone(), e)),
            }
        }
        (
            IconThemeRegistry {
                seti,
                lucide,
                themes,
            },
            warnings,
        )
    }

    /// "Seti" and "Lucide" first, then the extension themes in load order.
    pub fn names(&self) -> Vec<String> {
        [&self.seti, &self.lucide]
            .into_iter()
            .chain(&self.themes)
            .map(|t| t.name.clone())
            .collect()
    }

    /// The named theme (`"vs-seti"` is Seti), or Seti when the name is
    /// unknown.
    pub fn resolve(&self, name: &str) -> ResolvedIcons {
        let active = if name == self.seti.name || name == "vs-seti" {
            &self.seti
        } else if name == self.lucide.name {
            &self.lucide
        } else {
            self.themes
                .iter()
                .find(|t| t.name == name)
                .unwrap_or(&self.seti)
        }
        .clone();
        ResolvedIcons {
            active,
            lucide: self.lucide.clone(),
        }
    }
}

/// An active icon theme with Lucide behind it, so every lookup answers.
#[derive(Debug, Clone)]
pub struct ResolvedIcons {
    active: Arc<IconTheme>,
    lucide: Arc<IconTheme>,
}

impl ResolvedIcons {
    pub fn name(&self) -> &str {
        &self.active.name
    }

    pub fn file(&self, path: &Path) -> IconSource {
        self.active
            .file_icon(path)
            .or_else(|| self.lucide.file_icon(path))
            .unwrap_or(IconSource::Builtin("file"))
    }

    pub fn directory(&self, name: &str, expanded: bool) -> IconSource {
        self.active
            .directory_icon(name, expanded)
            .or_else(|| self.lucide.directory_icon(name, expanded))
            .unwrap_or(IconSource::Builtin("folder"))
    }

    pub fn chevron(&self, expanded: bool) -> IconSource {
        self.active
            .chevron(expanded)
            .or_else(|| self.lucide.chevron(expanded))
            .unwrap_or(IconSource::Builtin("chevron-right"))
    }
}
