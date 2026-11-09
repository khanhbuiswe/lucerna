//! Validation of grammars, queries, themes and icon themes.

use std::path::{Component, Path, PathBuf};

use serde_json::Value;

/// Loads compiled grammars and compiles queries against them. Implemented
/// outside this crate (by the syntax layer); tests use a fake.
pub trait GrammarValidator: Send + Sync {
    fn load(&self, name: &str, wasm: &[u8]) -> Result<(), String>;
    fn check_query(&self, name: &str, wasm: &[u8], kind: &str, source: &str) -> Result<(), String>;
}

/// `(name, appearance)` of each theme in a family file.
pub type ThemeNames = Vec<(String, String)>;

fn read_lenient(path: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    jsonc_parser::parse_to_serde_value(&text, &Default::default())
        .map_err(|e| format!("{}: {e}", path.display()))?
        .ok_or_else(|| format!("{}: empty file", path.display()))
}

/// `(name, appearance)` of every theme in a family file, plus each theme value.
fn family(path: &Path) -> Result<(ThemeNames, Vec<Value>), String> {
    let value = read_lenient(path)?;
    let themes = value
        .get("themes")
        .and_then(Value::as_array)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| format!("{}: expected a non-empty `themes` array", path.display()))?;
    let mut out = Vec::new();
    for (i, t) in themes.iter().enumerate() {
        let name = t.get("name").and_then(Value::as_str);
        let appearance = t.get("appearance").and_then(Value::as_str);
        match (name, appearance) {
            (Some(n), Some(a @ ("light" | "dark"))) => out.push((n.to_string(), a.to_string())),
            _ => {
                return Err(format!(
                    "{}: theme {i} needs a `name` and an `appearance` of \"light\" or \"dark\"",
                    path.display()
                ));
            }
        }
    }
    Ok((out, themes.clone()))
}

/// Checks the outer shape of a theme family file and returns
/// `(name, appearance)` for each theme.
pub fn check_theme_json(path: &Path) -> Result<ThemeNames, String> {
    family(path).map(|(t, _)| t)
}

/// Checks an icon theme family file. Icon paths are relative to `icons_root`
/// (the extension directory); each one that escapes it, or is missing,
/// produces a warning.
pub fn check_icon_theme_json(
    path: &Path,
    icons_root: &Path,
) -> Result<(ThemeNames, Vec<String>), String> {
    icon_theme_files(path, icons_root).map(|(t, _, w)| (t, w))
}

/// Like [`check_icon_theme_json`], also returning the relative (normalised)
/// path of every icon that exists inside `icons_root`.
pub fn icon_theme_files(
    path: &Path,
    icons_root: &Path,
) -> Result<(ThemeNames, Vec<PathBuf>, Vec<String>), String> {
    let (themes, values) = family(path)?;
    let mut refs = Vec::new();
    for v in &values {
        collect_icon_paths(v, None, &mut refs);
    }
    let root = icons_root
        .canonicalize()
        .map_err(|e| format!("{}: {e}", icons_root.display()))?;
    let mut files: Vec<PathBuf> = Vec::new();
    let mut warnings = Vec::new();
    let file_name = path.file_name().unwrap_or_default().to_string_lossy();
    for r in refs {
        match resolve_inside(&root, &r) {
            Ok(rel) => {
                if !files.contains(&rel) {
                    files.push(rel);
                }
            }
            Err(e) => warnings.push(format!("icon theme {file_name}: icon `{r}` {e}")),
        }
    }
    Ok((themes, files, warnings))
}

/// Strings under `path`, or under `collapsed` / `expanded` of
/// `directory_icons`, `chevron_icons` and `named_directory_icons` entries.
fn collect_icon_paths(v: &Value, key: Option<&str>, out: &mut Vec<String>) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                collect_icon_paths(x, Some(k), out);
            }
        }
        Value::Array(a) => a.iter().for_each(|x| collect_icon_paths(x, key, out)),
        Value::String(s) if matches!(key, Some("path" | "collapsed" | "expanded")) => {
            out.push(s.clone());
        }
        _ => {}
    }
}

/// Resolves `rel` against the canonical `root`. Returns the normalised
/// relative path when it names an existing file inside `root`.
fn resolve_inside(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let mut norm = PathBuf::new();
    for c in Path::new(rel).components() {
        match c {
            Component::CurDir => {}
            Component::Normal(s) => norm.push(s),
            _ => return Err("is outside the extension".into()),
        }
    }
    let canon = root
        .join(&norm)
        .canonicalize()
        .map_err(|_| "does not exist".to_string())?;
    if !canon.starts_with(root) {
        return Err("is outside the extension".into());
    }
    if !canon.is_file() {
        return Err("is not a file".into());
    }
    Ok(norm)
}
