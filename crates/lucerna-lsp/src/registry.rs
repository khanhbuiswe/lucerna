//! The server slots of a workspace. A plain data structure: the workbench
//! drives it and owns all threads.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use serde_json::Value;

use crate::adapters::{Adapter, find_root};
use crate::client::LanguageServer;
use crate::settings::{ServerSettings, ServersSettings, select_servers};

#[derive(Clone, Hash, Eq, PartialEq, Debug)]
pub struct ServerKey {
    pub root: Arc<Path>,
    pub server: &'static str,
}

pub enum Slot {
    Starting,
    Running(Arc<LanguageServer>),
    Failed {
        message: String,
        stderr_tail: Vec<String>,
    },
    NotFound {
        message: String,
    },
    Stopped,
}

/// What a settings change requires of the existing slots.
#[derive(Debug, Default, PartialEq)]
pub struct SettingsDiff {
    /// `binary` or `initialization_options` changed.
    pub restart: Vec<ServerKey>,
    /// Only `settings` changed; the new value (`Null` when removed).
    pub reconfigure: Vec<(ServerKey, Value)>,
}

pub struct Registry {
    adapters: Vec<Arc<dyn Adapter>>,
    slots: HashMap<ServerKey, Slot>,
    settings: ServersSettings,
}

impl Registry {
    pub fn new(adapters: Vec<Arc<dyn Adapter>>) -> Registry {
        Registry {
            adapters,
            slots: HashMap::new(),
            settings: ServersSettings::new(),
        }
    }

    /// The servers for a file and the project root each uses. Checks the
    /// filesystem for root markers: call it off the UI thread.
    pub fn keys_for(
        &self,
        language_id: &str,
        path: &Path,
        workspace_root: &Path,
        list: &[String],
    ) -> Vec<ServerKey> {
        select_servers(list, &self.adapters, language_id)
            .iter()
            .map(|adapter| ServerKey {
                root: find_root(&**adapter, path, workspace_root).into(),
                server: adapter.name(),
            })
            .collect()
    }

    pub fn adapter(&self, server: &str) -> Option<Arc<dyn Adapter>> {
        self.adapters.iter().find(|a| a.name() == server).cloned()
    }

    pub fn slot(&self, key: &ServerKey) -> Option<&Slot> {
        self.slots.get(key)
    }

    pub fn set_slot(&mut self, key: ServerKey, slot: Slot) {
        self.slots.insert(key, slot);
    }

    pub fn running(&self) -> impl Iterator<Item = (&ServerKey, &Arc<LanguageServer>)> {
        self.slots.iter().filter_map(|(key, slot)| match slot {
            Slot::Running(server) => Some((key, server)),
            _ => None,
        })
    }

    pub fn keys(&self) -> impl Iterator<Item = &ServerKey> {
        self.slots.keys()
    }

    pub fn settings(&self, server: &str) -> Option<&ServerSettings> {
        self.settings.get(server)
    }

    /// Replaces the settings and says which existing slots are affected.
    /// Servers dropped from the map count as changed back to defaults.
    pub fn set_settings(&mut self, new: ServersSettings) -> SettingsDiff {
        let default = ServerSettings::default();
        let mut diff = SettingsDiff::default();
        for key in self.slots.keys() {
            let old = self.settings.get(key.server).unwrap_or(&default);
            let now = new.get(key.server).unwrap_or(&default);
            if old.binary != now.binary || old.initialization_options != now.initialization_options
            {
                diff.restart.push(key.clone());
            } else if old.settings != now.settings {
                diff.reconfigure
                    .push((key.clone(), now.settings.clone().unwrap_or(Value::Null)));
            }
        }
        self.settings = new;
        diff
    }

    pub fn remove(&mut self, key: &ServerKey) -> Option<Slot> {
        self.slots.remove(key)
    }
}
