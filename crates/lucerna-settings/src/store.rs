use jsonc_parser::ast;
use jsonc_parser::common::{Range, Ranged};
use jsonc_parser::{CollectOptions, CommentCollectionStrategy, ParseOptions, parse_to_ast};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::SettingsError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    Default,
    User,
    Workspace,
}

#[derive(Debug, Default)]
pub struct SettingsStore {
    default: Map<String, Value>,
    user: Map<String, Value>,
    workspace: Map<String, Value>,
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
            Layer::Workspace => self.workspace = map,
        }
        Ok(())
    }

    /// Looks up `key`, preferring (highest first) VS Code's order:
    /// Workspace `[lang]`, User `[lang]`, Default `[lang]`, then Workspace,
    /// User, Default. The first value found replaces lower ones wholesale
    /// (no object merging). A value that does not deserialize into `T`
    /// yields `None`.
    pub fn get<T: DeserializeOwned>(&self, key: &str, language: Option<&str>) -> Option<T> {
        let value = self.candidates(key, language).next()?;
        T::deserialize(value).ok()
    }

    /// Every value of `key`, highest precedence first.
    fn candidates<'a>(
        &'a self,
        key: &'a str,
        language: Option<&str>,
    ) -> impl Iterator<Item = &'a Value> + 'a {
        let layers = [&self.workspace, &self.user, &self.default];
        let section = language.map(|lang| format!("[{lang}]"));
        let overrides = layers.into_iter().filter_map(move |layer| {
            let Some(Value::Object(overrides)) = layer.get(section.as_deref()?) else {
                return None;
            };
            lookup(overrides, key)
        });
        overrides.chain(
            layers
                .into_iter()
                .filter_map(move |layer| lookup(layer, key)),
        )
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
