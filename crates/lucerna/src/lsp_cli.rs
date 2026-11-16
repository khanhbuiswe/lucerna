//! `luc lsp`: language server tools on the command line. Never starts GPUI.

use std::io::Write;
use std::path::Path;

use clap::{Args, Subcommand};
use lucerna_lsp::health::{current_os, format_table, health};
use lucerna_lsp::{Launcher, ProcessLauncher, ServerTable};
use lucerna_settings::{Layer, SettingsStore};
use lucerna_workbench::ConfigPaths;
use serde_json::Value;

/// Language server tools.
#[derive(Debug, Args)]
pub struct LspArgs {
    #[command(subcommand)]
    action: LspCommand,
}

#[derive(Debug, Subcommand)]
pub enum LspCommand {
    /// List every language server in the table and whether it is installed.
    Health,
}

/// Runs the command and returns the process exit code.
pub fn run(args: LspArgs) -> i32 {
    match args.action {
        LspCommand::Health => {
            let table = user_table();
            let mut stdout = std::io::stdout().lock();
            run_health(&table, &ProcessLauncher::new(), &mut stdout)
        }
    }
}

/// The server table from the built-in servers and the User settings file
/// the app reads. The current folder's Workspace settings are never read:
/// they could name any program.
fn user_table() -> ServerTable {
    let path = ConfigPaths::user_only().user_settings();
    let mut settings = SettingsStore::with_defaults();
    if let Some(text) = read_settings(&path)
        && let Err(e) = settings.set_layer(Layer::User, &text)
    {
        eprintln!("warning: {}: {e}", path.display());
    }
    ServerTable::build(settings.get_trusted::<Value>("lsp", None).as_ref())
}

fn read_settings(path: &Path) -> Option<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            eprintln!("warning: {}: {e}", path.display());
            None
        }
    }
}

/// Prints the health table of `table` to `out`, then the entries left out
/// of it. Always 0: a missing server is a report, not a failure. A closed
/// pipe (`luc lsp health | head`) stops the output quietly.
pub fn run_health(table: &ServerTable, launcher: &dyn Launcher, out: &mut dyn Write) -> i32 {
    let mut text = format_table(&health(table, launcher, current_os()));
    if !table.errors.is_empty() {
        text.push('\n');
        for e in &table.errors {
            text.push_str(&format!("Invalid `lsp.{}`: {}\n", e.server, e.message));
        }
    }
    let _ = out.write_all(text.as_bytes()).and_then(|()| out.flush());
    0
}
