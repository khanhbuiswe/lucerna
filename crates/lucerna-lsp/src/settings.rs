//! The `lsp` setting, server selection and launch resolution.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::launcher::Launcher;
use crate::table::{ServerDef, ServerTable};
use crate::transport::LaunchSpec;
use crate::vars::{VarError, Vars, expand};

#[derive(Deserialize, Default, Clone, PartialEq, Debug)]
pub struct BinarySettings {
    pub path: Option<PathBuf>,
    pub arguments: Option<Vec<String>>,
    pub env: Option<HashMap<String, String>>,
}

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

/// The servers to run for a file, in `list` order. `!name` removes a
/// server wherever it appears; `...` stands for the remaining servers that
/// match the file (by `language` when a server of the table lists it,
/// else by `path`'s file type) and are enabled, by name; a named server is
/// used when it matches the file, even if disabled; unknown names are
/// ignored.
pub fn select_servers<'a>(
    list: &[String],
    table: &'a ServerTable,
    language: Option<&str>,
    path: &Path,
) -> Vec<&'a ServerDef> {
    // A language no server lists (an extension's "Terraform Vars") is
    // matched by file type, as a file without a language is.
    let language = language.filter(|l| table.lists_language(l));
    let removed: HashSet<&str> = list.iter().filter_map(|s| s.strip_prefix('!')).collect();
    let matching = || {
        table
            .servers
            .values()
            .filter(|def| !removed.contains(&*def.name) && def.matches(language, path))
    };
    let mut selected: Vec<&ServerDef> = Vec::new();
    let mut push = |def: &'a ServerDef| {
        if !selected.iter().any(|d| d.name == def.name) {
            selected.push(def);
        }
    };
    for entry in list {
        if entry.starts_with('!') {
            continue;
        }
        if entry == "..." {
            matching().filter(|def| def.enabled).for_each(&mut push);
        } else if let Some(def) = matching().find(|def| &*def.name == entry) {
            push(def);
        }
    }
    selected
}

fn not_found(def: &ServerDef) -> StartError {
    StartError::NotFound {
        message: format!(
            "{} not found on PATH. Install it or set \"lsp\": {{ \"{}\": {{ \"binary\": {{ \"path\": … }} }} }}",
            def.command.as_deref().unwrap_or(&def.name),
            def.name
        ),
    }
}

fn var_failed(err: &VarError) -> StartError {
    StartError::Failed {
        message: err.to_string(),
        stderr_tail: Vec::new(),
    }
}

/// How to start `def` in `root`: `binary.path` as is, else `command` found
/// on `PATH` (and probed unless probing is off); `binary.arguments` or
/// `args`; the launcher's environment with `env` then `binary.env` on top.
/// Arguments and environment values are expanded with `vars`. May block
/// (`which`, probe, login-shell environment): call it off the UI thread.
///
/// # Errors
/// [`StartError::NotFound`] when the binary is not on `PATH` or fails its
/// probe; [`StartError::Failed`] for a variable that cannot be expanded.
pub fn resolve_launch(
    def: &ServerDef,
    root: &Path,
    launcher: &dyn Launcher,
    vars: &Vars,
) -> Result<LaunchSpec, StartError> {
    let binary = def.binary.as_ref();
    let program = if let Some(path) = binary.and_then(|b| b.path.clone()) {
        path
    } else {
        let command = def.command.as_deref().ok_or_else(|| not_found(def))?;
        let program = launcher.which(command).ok_or_else(|| not_found(def))?;
        if let Some(probe) = &def.probe {
            let probe: Vec<&str> = probe.iter().map(String::as_str).collect();
            if !launcher.probe(&program, &probe, root) {
                return Err(not_found(def));
            }
        }
        program
    };
    let args = binary
        .and_then(|b| b.arguments.as_ref())
        .unwrap_or(&def.args)
        .iter()
        .map(|arg| expand(arg, vars))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| var_failed(&err))?;
    let mut env = launcher.env();
    let overrides = def
        .env
        .iter()
        .chain(binary.and_then(|b| b.env.as_ref()).into_iter().flatten());
    for (key, value) in overrides {
        let value = expand(value, vars).map_err(|err| var_failed(&err))?;
        env.insert(key.clone(), value);
    }
    Ok(LaunchSpec {
        server: def.name.to_string(),
        program,
        args,
        env,
        cwd: root.to_path_buf(),
    })
}
