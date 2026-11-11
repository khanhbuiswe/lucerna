//! The server table: language servers as data, merged from the built-in
//! `servers.json` and the `lsp` setting.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, OnceLock};

use serde::Deserialize;
use serde_json::Value;

use crate::adapters::RootSearch;
use crate::settings::BinarySettings;

/// The `languageId` sent in `didOpen`: one value, or a map from Lucerna
/// language id.
#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(untagged)]
pub enum LanguageIdMap {
    One(String),
    Map(BTreeMap<String, String>),
}

/// An install command, one for all systems or per OS (`macos`, `linux`,
/// `windows`).
#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(untagged)]
pub enum Install {
    One(String),
    PerOs(BTreeMap<String, String>),
}

#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Relay {
    VueTsserver,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ServerDef {
    pub name: Arc<str>,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub languages: Vec<String>,
    pub file_types: Vec<String>,
    pub language_id: Option<LanguageIdMap>,
    pub root_markers: Vec<String>,
    pub root_search: RootSearch,
    /// `None` when probing is off (`"probe": false`).
    pub probe: Option<Vec<String>>,
    pub enabled: bool,
    pub relay: Option<Relay>,
    pub install: Option<Install>,
    pub homepage: Option<String>,
    /// One-line note shown in Health.
    pub note: Option<String>,
    pub binary: Option<BinarySettings>,
    pub initialization_options: Option<Value>,
    pub settings: Option<Value>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawProbe {
    Flag(bool),
    Args(Vec<String>),
}

#[derive(Deserialize)]
struct RawDef {
    command: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    languages: Vec<String>,
    #[serde(default)]
    file_types: Vec<String>,
    language_id: Option<LanguageIdMap>,
    #[serde(default)]
    root_markers: Vec<String>,
    #[serde(default)]
    root_search: RootSearch,
    probe: Option<RawProbe>,
    #[serde(default = "enabled_default")]
    enabled: bool,
    relay: Option<Relay>,
    install: Option<Install>,
    homepage: Option<String>,
    note: Option<String>,
    binary: Option<BinarySettings>,
    initialization_options: Option<Value>,
    settings: Option<Value>,
}

fn enabled_default() -> bool {
    true
}

impl RawDef {
    fn into_def(self, name: &str) -> Result<ServerDef, String> {
        let has_command =
            self.command.is_some() || self.binary.as_ref().is_some_and(|b| b.path.is_some());
        if !has_command {
            return Err("missing `command`".to_owned());
        }
        if self.languages.is_empty() && self.file_types.is_empty() {
            return Err("missing `languages` or `file_types`".to_owned());
        }
        let probe = match self.probe {
            None | Some(RawProbe::Flag(true)) => Some(vec!["--version".to_owned()]),
            Some(RawProbe::Flag(false)) => None,
            Some(RawProbe::Args(args)) => Some(args),
        };
        Ok(ServerDef {
            name: Arc::from(name),
            command: self.command,
            args: self.args,
            env: self.env,
            languages: self.languages,
            file_types: self.file_types,
            language_id: self.language_id,
            root_markers: self.root_markers,
            root_search: self.root_search,
            probe,
            enabled: self.enabled,
            relay: self.relay,
            install: self.install,
            homepage: self.homepage,
            note: self.note,
            binary: self.binary,
            initialization_options: self.initialization_options,
            settings: self.settings,
        })
    }
}

impl ServerDef {
    /// The `languageId` for `didOpen`.
    pub fn lsp_language_id(&self, lucerna_language: Option<&str>) -> String {
        match (&self.language_id, lucerna_language) {
            (Some(LanguageIdMap::One(id)), _) => id.clone(),
            (Some(LanguageIdMap::Map(map)), Some(lang)) => {
                map.get(lang).map_or_else(|| lang.to_owned(), Clone::clone)
            }
            (None, Some(lang)) => lang.to_owned(),
            (_, None) => self.languages.first().cloned().unwrap_or_default(),
        }
    }

    /// Whether this server applies to a file: by Lucerna language when the
    /// file has one, else by file name or extension.
    pub fn matches(&self, language: Option<&str>, path: &Path) -> bool {
        if let Some(language) = language {
            return self.languages.iter().any(|l| l == language);
        }
        let name = path.file_name().and_then(|n| n.to_str());
        let ext = path.extension().and_then(|e| e.to_str());
        self.file_types.iter().any(|entry| {
            name == Some(entry.as_str()) || ext.is_some_and(|e| e.eq_ignore_ascii_case(entry))
        })
    }

    pub fn install_for(&self, os: &str) -> Option<&str> {
        match self.install.as_ref()? {
            Install::One(command) => Some(command),
            Install::PerOs(map) => map.get(os).map(String::as_str),
        }
    }

    /// Whether a running server started from `other` would be identical:
    /// the fields that decide how it is launched and initialized.
    pub fn launch_fields_eq(&self, other: &ServerDef) -> bool {
        self.command == other.command
            && self.args == other.args
            && self.env == other.env
            && self.binary == other.binary
            && self.initialization_options == other.initialization_options
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TableError {
    pub server: String,
    pub message: String,
}

#[derive(Clone, Debug, Default)]
pub struct ServerTable {
    pub servers: BTreeMap<Arc<str>, ServerDef>,
    pub errors: Vec<TableError>,
}

/// Merge `over` into `base`: objects merge recursively, anything else
/// (arrays included) replaces.
pub fn merge_json(base: &mut Value, over: &Value) {
    match (base, over) {
        (Value::Object(base), Value::Object(over)) => {
            for (key, value) in over {
                match base.get_mut(key) {
                    Some(slot) => merge_json(slot, value),
                    None => {
                        base.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (base, over) => *base = over.clone(),
    }
}

impl ServerTable {
    /// The built-in table, parsed once.
    ///
    /// # Panics
    /// If the embedded `servers.json` is not valid JSON (a build bug).
    pub fn defaults() -> &'static Value {
        static DEFAULTS: OnceLock<Value> = OnceLock::new();
        DEFAULTS.get_or_init(|| {
            serde_json::from_str(include_str!("../assets/servers.json"))
                .expect("assets/servers.json is valid JSON")
        })
    }

    /// The defaults merged with the `lsp` setting, validated. Invalid
    /// entries are left out and reported in `errors`.
    pub fn build(lsp_setting: Option<&Value>) -> ServerTable {
        let mut merged = Self::defaults().clone();
        if let Some(setting) = lsp_setting.filter(|s| s.is_object()) {
            merge_json(&mut merged, setting);
        }
        let mut table = ServerTable::default();
        let Value::Object(entries) = merged else {
            return table;
        };
        for (name, value) in entries {
            let result = serde_json::from_value::<RawDef>(value)
                .map_err(|e| e.to_string())
                .and_then(|raw| raw.into_def(&name));
            match result {
                Ok(def) => {
                    table.servers.insert(Arc::clone(&def.name), def);
                }
                Err(message) => table.errors.push(TableError {
                    server: name,
                    message,
                }),
            }
        }
        table
    }

    pub fn get(&self, name: &str) -> Option<&ServerDef> {
        self.servers.get(name)
    }
}
