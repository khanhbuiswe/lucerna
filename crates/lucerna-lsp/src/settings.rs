//! The `lsp` setting, server selection and launch resolution.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;
use serde_json::Value;

use crate::adapters::Adapter;
use crate::launcher::Launcher;
use crate::transport::LaunchSpec;

#[derive(Deserialize, Default, Clone, PartialEq, Debug)]
pub struct BinarySettings {
    pub path: Option<PathBuf>,
    pub arguments: Option<Vec<String>>,
    pub env: Option<HashMap<String, String>>,
}

#[derive(Deserialize, Default, Clone, PartialEq, Debug)]
pub struct ServerSettings {
    pub binary: Option<BinarySettings>,
    pub initialization_options: Option<Value>,
    pub settings: Option<Value>,
}

/// The value of the `lsp` setting, by server name.
pub type ServersSettings = HashMap<String, ServerSettings>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartError {
    NotFound {
        message: String,
    },
    Failed {
        message: String,
        stderr_tail: Vec<String>,
    },
}

/// The adapters to run for `language_id`, in `list` order. `!name` removes a
/// server wherever it appears; `...` stands for the remaining adapters of
/// the language that are enabled by default; unknown names are ignored.
pub fn select_servers(
    list: &[String],
    adapters: &[Arc<dyn Adapter>],
    language_id: &str,
) -> Vec<Arc<dyn Adapter>> {
    let removed: HashSet<&str> = list.iter().filter_map(|s| s.strip_prefix('!')).collect();
    let for_language = || {
        adapters
            .iter()
            .filter(|a| a.language_ids().contains(&language_id) && !removed.contains(a.name()))
    };
    let mut selected: Vec<Arc<dyn Adapter>> = Vec::new();
    let mut push = |adapter: &Arc<dyn Adapter>| {
        if !selected.iter().any(|a| a.name() == adapter.name()) {
            selected.push(Arc::clone(adapter));
        }
    };
    for entry in list {
        if entry.starts_with('!') {
            continue;
        }
        if entry == "..." {
            for_language()
                .filter(|a| a.enabled_by_default())
                .for_each(&mut push);
        } else if let Some(adapter) = for_language().find(|a| a.name() == entry) {
            push(adapter);
        }
    }
    selected
}

fn not_found(adapter: &dyn Adapter) -> StartError {
    StartError::NotFound {
        message: format!(
            "{} not found on PATH. Install it or set \"lsp\": {{ \"{}\": {{ \"binary\": {{ \"path\": … }} }} }}",
            adapter.binary(),
            adapter.name()
        ),
    }
}

/// How to start `adapter` in `root`. May block (`which`, probe, login-shell
/// environment): call it off the UI thread.
///
/// # Errors
/// [`StartError::NotFound`] when the binary is not on `PATH` or fails its
/// probe.
pub fn resolve_launch(
    adapter: &dyn Adapter,
    settings: Option<&ServerSettings>,
    root: &Path,
    launcher: &dyn Launcher,
) -> Result<LaunchSpec, StartError> {
    let binary = settings.and_then(|s| s.binary.as_ref());
    let mut env = launcher.env();
    let program = if let Some(path) = binary.and_then(|b| b.path.clone()) {
        path
    } else {
        let program = launcher
            .which(adapter.binary())
            .ok_or_else(|| not_found(adapter))?;
        let probe_args = adapter.probe_args();
        if !probe_args.is_empty() && !launcher.probe(&program, probe_args, root) {
            return Err(not_found(adapter));
        }
        program
    };
    let args = binary
        .and_then(|b| b.arguments.clone())
        .unwrap_or_else(|| adapter.args().iter().map(|a| (*a).to_owned()).collect());
    if let Some(overrides) = binary.and_then(|b| b.env.as_ref()) {
        env.extend(overrides.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
    Ok(LaunchSpec {
        server: adapter.name().to_owned(),
        program,
        args,
        env,
        cwd: root.to_path_buf(),
    })
}
