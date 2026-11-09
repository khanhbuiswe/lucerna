use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::schema::ThemeFamily;
use crate::{Appearance, Theme};

const ONE_JSON: &str = include_str!("../assets/one.json");
const DEFAULT_THEME: &str = "One Dark";

#[derive(Debug, Clone)]
pub struct ThemeRegistry {
    themes: BTreeMap<String, Arc<Theme>>,
    default: Arc<Theme>,
}

/// Where a theme file came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThemeSource {
    /// An installed extension, by id.
    Extension(String),
    /// The user's themes directory.
    User,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeFile {
    pub path: PathBuf,
    pub source: ThemeSource,
}

/// Parses a theme family. Lenient: comments and trailing commas are fine.
fn parse_family(text: &str) -> Result<Vec<Theme>, String> {
    let value = jsonc_parser::parse_to_serde_value(text, &jsonc_parser::ParseOptions::default())
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "empty theme file".to_string())?;
    let family: ThemeFamily = serde_json::from_value(value).map_err(|e| e.to_string())?;
    family.themes.into_iter().map(|t| t.into_theme()).collect()
}

impl ThemeRegistry {
    /// The bundled One family: "One Dark" and "One Light".
    pub fn builtin() -> ThemeRegistry {
        Self::build(&[], None).0
    }

    /// Layers the registry: built-in, then extension theme files sorted by
    /// extension id, then every `*.json` in `user_dir`. A later layer wins
    /// on name. Returns one error per file (or `user_dir` itself) that could
    /// not be loaded; good files still load.
    pub fn build(
        extension_files: &[(String, PathBuf)],
        user_dir: Option<&Path>,
    ) -> (ThemeRegistry, Vec<(PathBuf, String)>) {
        let themes: BTreeMap<String, Arc<Theme>> = parse_family(ONE_JSON)
            .expect("bundled one.json is valid")
            .into_iter()
            .map(|t| (t.name.clone(), Arc::new(t)))
            .collect();
        let default = themes
            .get(DEFAULT_THEME)
            .expect("bundled one.json has One Dark")
            .clone();
        let mut registry = ThemeRegistry { themes, default };

        let mut errors = Vec::new();
        // Which extension supplied each theme name, to warn about clashes.
        let mut owner: BTreeMap<String, String> = BTreeMap::new();
        let mut warned: BTreeSet<String> = BTreeSet::new();
        let mut files: Vec<&(String, PathBuf)> = extension_files.iter().collect();
        files.sort();
        for (id, path) in files {
            match load_file(path) {
                Ok(themes) => {
                    for t in themes {
                        let name = t.name.clone();
                        if let Some(prev) = owner.insert(name.clone(), id.clone())
                            && prev != *id
                            && warned.insert(name.clone())
                        {
                            tracing::warn!(
                                theme = %name, first = %prev, later = %id,
                                "theme name provided by two extensions; the later id wins"
                            );
                        }
                        registry.themes.insert(name, Arc::new(t));
                    }
                }
                Err(e) => errors.push((path.clone(), e)),
            }
        }
        if let Some(dir) = user_dir {
            registry.load_user_dir(dir, &mut errors);
        }
        (registry, errors)
    }

    fn load_user_dir(&mut self, dir: &Path, errors: &mut Vec<(PathBuf, String)>) {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) => {
                errors.push((dir.to_path_buf(), e.to_string()));
                return;
            }
        };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect();
        paths.sort();
        for path in paths {
            match load_file(&path) {
                Ok(themes) => {
                    for t in themes {
                        self.themes.insert(t.name.clone(), Arc::new(t));
                    }
                }
                Err(e) => errors.push((path, e)),
            }
        }
    }

    /// Every theme name with its appearance, sorted by name: for the picker.
    pub fn themes_with_appearance(&self) -> Vec<(String, Appearance)> {
        self.themes
            .iter()
            .map(|(name, t)| (name.clone(), t.appearance))
            .collect()
    }

    pub fn get(&self, name: &str) -> Option<Arc<Theme>> {
        self.themes.get(name).cloned()
    }

    /// The named theme, or "One Dark" when the name is unknown.
    pub fn get_or_default(&self, name: &str) -> Arc<Theme> {
        self.get(name).unwrap_or_else(|| self.default.clone())
    }

    pub fn names(&self) -> Vec<String> {
        self.themes.keys().cloned().collect()
    }
}

fn load_file(path: &Path) -> Result<Vec<Theme>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    parse_family(&text)
}
