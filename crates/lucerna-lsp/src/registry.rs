//! The server slots of a workspace. A plain data structure: the workbench
//! drives it and owns all threads.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use serde_json::Value;

use crate::adapters::find_root;
use crate::client::LanguageServer;
use crate::settings::select_servers;
use crate::table::ServerTable;

#[derive(Clone, Hash, Eq, PartialEq, Debug)]
pub struct ServerKey {
    pub root: Arc<Path>,
    pub server: Arc<str>,
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

/// What a new server table requires of the existing slots.
#[derive(Debug, Default, PartialEq)]
pub struct SettingsDiff {
    /// `command`, `args`, `env`, `binary` or `initialization_options`
    /// changed.
    pub restart: Vec<ServerKey>,
    /// Only `settings` changed; the new value (`Null` when removed), not
    /// expanded.
    pub reconfigure: Vec<(ServerKey, Value)>,
    /// The server left the table, or was disabled.
    pub stop: Vec<ServerKey>,
}

pub struct Registry {
    table: ServerTable,
    slots: HashMap<ServerKey, Slot>,
}

impl Registry {
    pub fn new(table: ServerTable) -> Registry {
        Registry {
            table,
            slots: HashMap::new(),
        }
    }

    pub fn table(&self) -> &ServerTable {
        &self.table
    }

    /// The servers for a file and the project root each uses; none for a
    /// file outside the workspace. Checks the filesystem for root markers:
    /// call it off the UI thread.
    pub fn keys_for(
        &self,
        language: Option<&str>,
        path: &Path,
        workspace_root: &Path,
        list: &[String],
    ) -> Vec<ServerKey> {
        select_servers(list, &self.table, language, path)
            .into_iter()
            .filter_map(|def| {
                Some(ServerKey {
                    root: find_root(def, path, workspace_root)?.into(),
                    server: Arc::clone(&def.name),
                })
            })
            .collect()
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

    /// Replaces the table and says which existing slots are affected.
    /// A server that left the table or was disabled is stopped; one whose
    /// launch fields changed is restarted; one whose `settings` alone
    /// changed is reconfigured.
    pub fn set_table(&mut self, table: ServerTable) -> SettingsDiff {
        let mut diff = SettingsDiff::default();
        for key in self.slots.keys() {
            let Some(now) = table.get(&key.server) else {
                diff.stop.push(key.clone());
                continue;
            };
            let Some(old) = self.table.get(&key.server) else {
                diff.restart.push(key.clone());
                continue;
            };
            if old.enabled && !now.enabled {
                diff.stop.push(key.clone());
            } else if !old.launch_fields_eq(now) {
                diff.restart.push(key.clone());
            } else if old.settings != now.settings {
                diff.reconfigure
                    .push((key.clone(), now.settings.clone().unwrap_or(Value::Null)));
            }
        }
        self.table = table;
        diff
    }

    pub fn remove(&mut self, key: &ServerKey) -> Option<Slot> {
        self.slots.remove(key)
    }
}
