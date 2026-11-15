use jsonc_parser::ast;
use jsonc_parser::common::{Range, Ranged};
use jsonc_parser::{CollectOptions, CommentCollectionStrategy, ParseOptions, parse_to_ast};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::SettingsError;
use crate::scope::is_app_scope;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    Default,
    User,
}

/// The settings of one folder (`<root>/.lucerna/settings.json`), parsed.
/// Each window holds its own; [`SettingsStore::view`] reads it over the
/// Default and User layers. The default value is empty.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorkspaceLayer {
    map: Map<String, Value>,
}

impl WorkspaceLayer {
    /// Parses JSONC text, as [`SettingsStore::set_layer`] does.
    pub fn parse(jsonc: &str) -> Result<WorkspaceLayer, SettingsError> {
        parse_object(jsonc).map(|map| WorkspaceLayer { map })
    }

    /// The app-scope keys ([`is_app_scope`]) the layer sets, which lookups
    /// ignore, sorted; nested and `[lang]` forms are spelled out with dots.
    /// For logging.
    pub fn app_scope_keys(&self) -> Vec<String> {
        fn walk(map: &Map<String, Value>, prefix: &str, found: &mut Vec<String>) {
            for (key, value) in map {
                let key = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                // `[rust].workbench.iconTheme` is the app-scope key after
                // its section.
                let bare = key.strip_prefix('[').and_then(|rest| {
                    let (_, rest) = rest.split_once(']')?;
                    Some(rest.strip_prefix('.').unwrap_or(rest))
                });
                if is_app_scope(bare.unwrap_or(&key)) {
                    found.push(key);
                } else if let Value::Object(inner) = value {
                    walk(inner, &key, found);
                }
            }
        }
        let mut found = Vec::new();
        walk(&self.map, "", &mut found);
        found.sort();
        found
    }
}

#[derive(Debug, Default)]
pub struct SettingsStore {
    default: Map<String, Value>,
    user: Map<String, Value>,
}

impl SettingsStore {
    /// A store whose default layer is the bundled `default-settings.json`.
    pub fn with_defaults() -> SettingsStore {
        let mut store = SettingsStore::default();
        store
            .set_layer(
                Layer::Default,
                include_str!("../assets/default-settings.json"),
            )
            .expect("bundled default-settings.json is valid");
        store
    }

    /// Replaces a layer with the parsed JSONC text. On error the previous
    /// value of the layer is kept.
    pub fn set_layer(&mut self, layer: Layer, jsonc: &str) -> Result<(), SettingsError> {
        let map = parse_object(jsonc)?;
        match layer {
            Layer::Default => self.default = map,
            Layer::User => self.user = map,
        }
        Ok(())
    }

    /// The Default and User layers with `workspace` over them.
    pub fn view<'a>(&'a self, workspace: Option<&'a WorkspaceLayer>) -> SettingsView<'a> {
        SettingsView {
            store: self,
            workspace,
        }
    }

    /// Looks `key` up in the Default and User layers alone (no workspace
    /// layer): what app-scope code reads. See [`SettingsView::get`].
    pub fn get<T: DeserializeOwned>(&self, key: &str, language: Option<&str>) -> Option<T> {
        self.view(None).get(key, language)
    }

    /// Same as [`get`](Self::get): without a workspace layer every value is
    /// trusted.
    pub fn get_trusted<T: DeserializeOwned>(&self, key: &str, language: Option<&str>) -> Option<T> {
        self.view(None).get_trusted(key, language)
    }
}

/// The settings one window sees: [`SettingsStore`]'s Default and User
/// layers with its folder's [`WorkspaceLayer`] (if any) over them.
#[derive(Debug, Clone, Copy)]
pub struct SettingsView<'a> {
    store: &'a SettingsStore,
    workspace: Option<&'a WorkspaceLayer>,
}

impl SettingsView<'_> {
    /// Looks up `key`, preferring (highest first) VS Code's order:
    /// Workspace `[lang]`, User `[lang]`, Default `[lang]`, then Workspace,
    /// User, Default. Object values merge across these (R26): the higher
    /// one wins per key, recursively, so a user `files.exclude` adds to the
    /// default globs and `"glob": false` turns a lower one off. Any other
    /// value replaces lower ones. A value that does not deserialize into
    /// `T` yields `None`. An app-scope key ([`is_app_scope`]) skips the
    /// Workspace layer.
    pub fn get<T: DeserializeOwned>(&self, key: &str, language: Option<&str>) -> Option<T> {
        let workspace = self.workspace.filter(|_| !is_app_scope(key));
        get_from(self.layers(workspace), key, language)
    }

    /// Like [`get`](Self::get) but without the Workspace layer, for settings
    /// that can run code (a language server's binary) and so must not come
    /// from an untrusted folder.
    pub fn get_trusted<T: DeserializeOwned>(&self, key: &str, language: Option<&str>) -> Option<T> {
        get_from(self.layers(None), key, language)
    }

    /// The layers to search, highest first.
    fn layers<'a>(&'a self, workspace: Option<&'a WorkspaceLayer>) -> Vec<&'a Map<String, Value>> {
        workspace
            .map(|w| &w.map)
            .into_iter()
            .chain([&self.store.user, &self.store.default])
            .collect()
    }
}

fn get_from<T: DeserializeOwned>(
    layers: Vec<&Map<String, Value>>,
    key: &str,
    language: Option<&str>,
) -> Option<T> {
    let mut candidates = candidates(layers, key, language);
    let first = candidates.next()?;
    if !first.is_object() {
        return T::deserialize(first).ok();
    }
    // Objects below the first one merge in, up to the first non-object.
    let objects: Vec<&Value> = std::iter::once(first)
        .chain(candidates.take_while(|v| v.is_object()))
        .collect();
    let mut merged = Value::Object(Map::new());
    for value in objects.into_iter().rev() {
        merge(&mut merged, value);
    }
    T::deserialize(merged).ok()
}

/// Every value of `key` in `layers` (highest first): the `[lang]`
/// overrides of every layer, then the plain values.
fn candidates<'a>(
    layers: Vec<&'a Map<String, Value>>,
    key: &'a str,
    language: Option<&str>,
) -> impl Iterator<Item = &'a Value> + 'a {
    let section = language.map(|lang| format!("[{lang}]"));
    let flat = layers.clone();
    let overrides = layers.into_iter().filter_map(move |layer| {
        let Some(Value::Object(overrides)) = layer.get(section.as_deref()?) else {
            return None;
        };
        lookup(overrides, key)
    });
    overrides.chain(flat.into_iter().filter_map(move |layer| lookup(layer, key)))
}

/// Merges `over` into `base`: objects key by key, recursively; anything
/// else replaces.
fn merge(base: &mut Value, over: &Value) {
    match (base, over) {
        (Value::Object(base), Value::Object(over)) => {
            for (key, value) in over {
                match base.get_mut(key) {
                    Some(slot) => merge(slot, value),
                    None => {
                        base.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (base, over) => *base = over.clone(),
    }
}

/// Exact flat key first, then the nested path.
fn lookup<'a>(map: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    if let Some(v) = map.get(key) {
        return Some(v);
    }
    let mut parts = key.split('.');
    let mut current = map.get(parts.next()?)?;
    for part in parts {
        current = current.as_object()?.get(part)?;
    }
    Some(current)
}

fn parse_object(text: &str) -> Result<Map<String, Value>, SettingsError> {
    match parse_jsonc(text)? {
        None => Ok(Map::new()),
        Some(Value::Object(map)) => Ok(map),
        Some(_) => Err(SettingsError {
            message: "settings must be a JSON object".into(),
            line: None,
        }),
    }
}

/// Parses JSONC (comments and trailing commas allowed, missing commas
/// rejected). `None` means the text holds no value (empty or comments only).
pub(crate) fn parse_jsonc(text: &str) -> Result<Option<Value>, SettingsError> {
    let options = ParseOptions {
        allow_comments: true,
        allow_trailing_commas: true,
        allow_loose_object_property_names: false,
    };
    let collect = CollectOptions {
        comments: CommentCollectionStrategy::Off,
        tokens: false,
    };
    let ast = parse_to_ast(text, &collect, &options)
        .map_err(|e| error_at(text, e.range().start, e.to_string()))?;
    let Some(root) = ast.value else {
        return Ok(None);
    };
    // The parser accepts a missing comma between members; JSONC does not.
    check_separators(text, &root)?;
    Ok(Some(Value::from(root)))
}

fn error_at(text: &str, offset: usize, message: String) -> SettingsError {
    let start = offset.min(text.len());
    let line = text.as_bytes()[..start]
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
        + 1;
    SettingsError {
        message,
        line: Some(line),
    }
}

fn check_separators(text: &str, value: &ast::Value) -> Result<(), SettingsError> {
    match value {
        ast::Value::Object(obj) => {
            let members: Vec<_> = obj
                .properties
                .iter()
                .map(|p| (p.range(), &p.value))
                .collect();
            check_members(text, &members)
        }
        ast::Value::Array(arr) => {
            let members: Vec<_> = arr.elements.iter().map(|v| (v.range(), v)).collect();
            check_members(text, &members)
        }
        _ => Ok(()),
    }
}

fn check_members(text: &str, members: &[(Range, &ast::Value)]) -> Result<(), SettingsError> {
    for pair in members.windows(2) {
        let (prev, next) = (pair[0].0, pair[1].0);
        if !has_comma(&text[prev.end..next.start]) {
            return Err(error_at(
                text,
                next.start,
                "Expected comma between members".into(),
            ));
        }
    }
    members
        .iter()
        .try_for_each(|(_, v)| check_separators(text, v))
}

/// Whether `gap` (whitespace and comments only) contains a comma.
fn has_comma(gap: &str) -> bool {
    let mut rest = gap;
    while let Some(c) = rest.chars().next() {
        if let Some(r) = rest.strip_prefix("//") {
            rest = r.split_once('\n').map_or("", |(_, after)| after);
        } else if let Some(r) = rest.strip_prefix("/*") {
            rest = r.split_once("*/").map_or("", |(_, after)| after);
        } else if c == ',' {
            return true;
        } else {
            rest = &rest[c.len_utf8()..];
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn store(user: &str) -> SettingsStore {
        let mut s = SettingsStore::with_defaults();
        s.set_layer(Layer::User, user).unwrap();
        s
    }

    fn ws(jsonc: &str) -> WorkspaceLayer {
        WorkspaceLayer::parse(jsonc).unwrap()
    }

    #[test]
    fn view_prefers_workspace() {
        let s = store(r#"{"editor.tabSize":4}"#);
        let w = ws(r#"{"editor.tabSize":2}"#);
        assert_eq!(s.view(Some(&w)).get::<u32>("editor.tabSize", None), Some(2));
        assert_eq!(s.view(None).get::<u32>("editor.tabSize", None), Some(4));
        assert_eq!(s.get::<u32>("editor.tabSize", None), Some(4));
    }

    /// Workspace `[lang]`, User `[lang]`, Workspace, User: each value shows
    /// once every layer above it is taken away.
    #[test]
    fn view_language_override_order() {
        // (workspace?, language?, value)
        let order = [
            (true, true, 1),
            (false, true, 2),
            (true, false, 3),
            (false, false, 4),
        ];
        for winner in 0..order.len() {
            let (mut user, mut workspace) = (Vec::new(), Vec::new());
            for &(in_ws, lang, value) in &order[winner..] {
                let entry = if lang {
                    format!(r#""[rust]": {{"editor.tabSize": {value}}}"#)
                } else {
                    format!(r#""editor.tabSize": {value}"#)
                };
                if in_ws { &mut workspace } else { &mut user }.push(entry);
            }
            let s = store(&format!("{{{}}}", user.join(",")));
            let w = ws(&format!("{{{}}}", workspace.join(",")));
            assert_eq!(
                s.view(Some(&w)).get::<u32>("editor.tabSize", Some("rust")),
                Some(order[winner].2),
                "with {:?}",
                &order[winner..]
            );
        }
    }

    /// R26 across Default, User and Workspace.
    #[test]
    fn view_merges_objects() {
        let s = store(r#"{"files.exclude": {"**/dist": true, "**/.DS_Store": false}}"#);
        let w = ws(r#"{"files.exclude": {"**/out": true, "**/dist": false}}"#);
        let m: BTreeMap<String, bool> = s.view(Some(&w)).get("files.exclude", None).unwrap();
        assert_eq!(
            m,
            BTreeMap::from([
                ("**/.DS_Store".into(), false),
                ("**/.git".into(), true),
                ("**/dist".into(), false),
                ("**/out".into(), true),
            ])
        );
        // A non-object in the workspace replaces the merged objects.
        let w = ws(r#"{"files.exclude": 7}"#);
        assert_eq!(s.view(Some(&w)).get::<u32>("files.exclude", None), Some(7));
    }

    #[test]
    fn trusted_skips_workspace() {
        let s = store(r#"{"lsp.x": "user"}"#);
        let w = ws(r#"{"lsp.x": "ws", "lsp.y": "ws", "[python]": {"lsp.x": "ws"}}"#);
        let view = s.view(Some(&w));
        assert_eq!(view.get::<String>("lsp.x", None).as_deref(), Some("ws"));
        assert_eq!(
            view.get_trusted::<String>("lsp.x", None).as_deref(),
            Some("user")
        );
        assert_eq!(
            view.get_trusted::<String>("lsp.x", Some("python"))
                .as_deref(),
            Some("user")
        );
        assert_eq!(view.get_trusted::<String>("lsp.y", None), None);
    }

    #[test]
    fn app_scope_ignored_in_workspace() {
        let s = store(r#"{"workbench.colorTheme": "Mine"}"#);
        let w = ws(
            r#"{"workbench.colorTheme":"X","window.restoreWindows":"none",
                "[rust]": {"workbench.iconTheme": "Y"},
                "workbench": {"startupEditor": "none"}, "editor.tabSize": 2}"#,
        );
        let view = s.view(Some(&w));
        assert_eq!(
            view.get::<String>("workbench.colorTheme", None).as_deref(),
            Some("Mine")
        );
        assert_eq!(
            view.get::<String>("window.restoreWindows", None),
            s.get::<String>("window.restoreWindows", None)
        );
        assert_eq!(
            view.get::<String>("workbench.iconTheme", Some("rust"))
                .as_deref(),
            Some("Seti")
        );
        assert_eq!(
            view.get::<String>("workbench.startupEditor", None),
            s.get::<String>("workbench.startupEditor", None)
        );
        assert_eq!(view.get::<u32>("editor.tabSize", None), Some(2));
        assert_eq!(
            w.app_scope_keys(),
            [
                "[rust].workbench.iconTheme",
                "window.restoreWindows",
                "workbench.colorTheme",
                "workbench.startupEditor",
            ]
        );
    }

    #[test]
    fn workspace_layer_parse() {
        assert!(WorkspaceLayer::parse("[1]").is_err());
        assert_eq!(
            WorkspaceLayer::parse("{\n\"a\": 1\n\"b\": 2\n}")
                .unwrap_err()
                .line,
            Some(3)
        );
        let empty = WorkspaceLayer::parse("// nothing").unwrap();
        assert_eq!(empty, WorkspaceLayer::default());
        assert!(empty.app_scope_keys().is_empty());
    }
}
