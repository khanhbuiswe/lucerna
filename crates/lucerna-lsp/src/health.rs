//! Health: which servers of the table are installed, for the Health tab
//! and `luc lsp health` (spec §7).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::launcher::Launcher;
use crate::table::{ServerDef, ServerTable};

#[derive(Clone, Debug, PartialEq)]
pub enum HealthStatus {
    /// The binary that would be started.
    Found(PathBuf),
    NotFound,
    /// `"enabled": false`: not looked up.
    Disabled,
}

/// One (language, server) pair of the table.
#[derive(Clone, Debug, PartialEq)]
pub struct HealthRow {
    pub language: String,
    pub server: Arc<str>,
    pub status: HealthStatus,
    /// The install command for the OS asked for.
    pub install: Option<String>,
    pub note: Option<String>,
}

/// The OS name install commands are keyed by.
pub fn current_os() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(windows) {
        "windows"
    } else {
        "linux"
    }
}

/// Checks every server of `table`: one row per (language, server), where
/// the languages are the server's `languages`, else its first file type.
/// Rows are sorted by language, enabled servers first, then by name.
/// Probes run in `$HOME`, as there is no project; see [`health_in`].
/// Blocks (`which`, the probe, the login-shell environment): call it off
/// the UI thread.
pub fn health(table: &ServerTable, launcher: &dyn Launcher, os: &str) -> Vec<HealthRow> {
    let home = std::env::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    health_in(table, launcher, os, &home)
}

/// [`health`] with the probes run in `probe_dir`: an open folder's root,
/// so a project toolchain (a `rust-toolchain.toml` shim) answers as it
/// will when the server starts there. Blocks.
pub fn health_in(
    table: &ServerTable,
    launcher: &dyn Launcher,
    os: &str,
    probe_dir: &Path,
) -> Vec<HealthRow> {
    let mut rows = Vec::new();
    for def in table.servers.values() {
        let status = status(def, launcher, probe_dir);
        let languages: Vec<&String> = if def.languages.is_empty() {
            def.file_types.iter().take(1).collect()
        } else {
            def.languages.iter().collect()
        };
        for language in languages {
            rows.push(HealthRow {
                language: language.clone(),
                server: Arc::clone(&def.name),
                status: status.clone(),
                install: def.install_for(os).map(str::to_owned),
                note: def.note.clone(),
            });
        }
    }
    rows.sort_by(|a, b| {
        let disabled = |r: &HealthRow| r.status == HealthStatus::Disabled;
        a.language
            .cmp(&b.language)
            .then_with(|| disabled(a).cmp(&disabled(b)))
            .then_with(|| a.server.cmp(&b.server))
    });
    rows
}

/// As `resolve_launch` finds the binary: `binary.path` (when it exists),
/// else `command` on `PATH` passing its probe, run in `probe_dir`.
fn status(def: &ServerDef, launcher: &dyn Launcher, probe_dir: &Path) -> HealthStatus {
    if !def.enabled {
        return HealthStatus::Disabled;
    }
    if let Some(path) = def.binary.as_ref().and_then(|b| b.path.as_ref()) {
        return if path.exists() {
            HealthStatus::Found(path.clone())
        } else {
            HealthStatus::NotFound
        };
    }
    let Some(program) = def.command.as_deref().and_then(|c| launcher.which(c)) else {
        return HealthStatus::NotFound;
    };
    if let Some(probe) = &def.probe {
        let args: Vec<&str> = probe.iter().map(String::as_str).collect();
        if !launcher.probe(&program, &args, probe_dir) {
            return HealthStatus::NotFound;
        }
    }
    HealthStatus::Found(program)
}

/// `✓ <path>`, `✗ not found` or `disabled`.
pub fn status_text(status: &HealthStatus) -> String {
    match status {
        HealthStatus::Found(path) => format!("✓ {}", path.display()),
        HealthStatus::NotFound => "✗ not found".to_owned(),
        HealthStatus::Disabled => "disabled".to_owned(),
    }
}

/// The rows as aligned columns: Language, Server, Status, Install. Each
/// line ends with a newline and has no trailing spaces.
pub fn format_table(rows: &[HealthRow]) -> String {
    let header = ["Language", "Server", "Status", "Install"].map(str::to_owned);
    let lines: Vec<[String; 4]> = std::iter::once(header)
        .chain(rows.iter().map(|r| {
            [
                r.language.clone(),
                r.server.to_string(),
                status_text(&r.status),
                r.install.clone().unwrap_or_default(),
            ]
        }))
        .collect();
    let mut widths = [0; 4];
    for line in &lines {
        for (width, cell) in widths.iter_mut().zip(line) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let mut out = String::new();
    for line in &lines {
        let mut text = String::new();
        for (i, cell) in line.iter().enumerate() {
            text.push_str(cell);
            if i + 1 < line.len() {
                let pad = widths[i] - cell.chars().count() + 2;
                text.extend(std::iter::repeat_n(' ', pad));
            }
        }
        out.push_str(text.trim_end());
        out.push('\n');
    }
    out
}
