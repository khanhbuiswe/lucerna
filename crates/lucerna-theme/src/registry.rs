use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::Theme;
use crate::schema::ThemeFamily;

const ONE_JSON: &str = include_str!("../assets/one.json");
const DEFAULT_THEME: &str = "One Dark";

#[derive(Debug, Clone)]
pub struct ThemeRegistry {
    themes: BTreeMap<String, Arc<Theme>>,
    default: Arc<Theme>,
}

fn parse_family(json: &str) -> Result<Vec<Theme>, String> {
    let family: ThemeFamily = serde_json::from_str(json).map_err(|e| e.to_string())?;
    family.themes.into_iter().map(|t| t.into_theme()).collect()
}

impl ThemeRegistry {
    /// The bundled One family: "One Dark" and "One Light".
    pub fn builtin() -> ThemeRegistry {
        let themes = parse_family(ONE_JSON).expect("bundled one.json is valid");
        let themes: BTreeMap<_, _> = themes
            .into_iter()
            .map(|t| (t.name.clone(), Arc::new(t)))
            .collect();
        let default = themes
            .get(DEFAULT_THEME)
            .expect("bundled one.json has One Dark")
            .clone();
        ThemeRegistry { themes, default }
    }

    /// Loads every `*.json` theme family in `dir`. Returns one error per
    /// file (or the directory itself) that could not be loaded; good files
    /// still load.
    pub fn load_dir(&mut self, dir: &Path) -> Vec<(PathBuf, String)> {
        let mut errors = Vec::new();
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) => return vec![(dir.to_path_buf(), e.to_string())],
        };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect();
        paths.sort();
        for path in paths {
            let loaded = std::fs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|text| parse_family(&text));
            match loaded {
                Ok(themes) => {
                    for t in themes {
                        self.themes.insert(t.name.clone(), Arc::new(t));
                    }
                }
                Err(e) => errors.push((path, e)),
            }
        }
        errors
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
