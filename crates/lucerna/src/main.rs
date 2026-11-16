use std::path::PathBuf;
use std::time::Instant;

use clap::{Parser, Subcommand};
use lucerna::{Target, logging, prompts_for_folder, resolve_target};
use lucerna::{ext_cli, lsp_cli};
use lucerna_workbench::app::{self, Launch};

/// Lucerna: a read-only code viewer.
#[derive(Debug, Parser)]
#[command(name = "luc", version, args_conflicts_with_subcommands = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    /// File or directory to open. A folder literally named `ext` or `lsp`
    /// must be opened as `luc ./ext` or `luc ./lsp`.
    path: Option<PathBuf>,
    /// Print `startup_ms=<n>` to stderr once the first frame is painted.
    #[arg(long)]
    trace_startup: bool,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Search, install, update and remove extensions.
    Ext(ext_cli::ExtArgs),
    /// Language server tools: `luc lsp health` lists installed servers.
    Lsp(lsp_cli::LspArgs),
}

fn main() {
    let start = Instant::now();
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Ext(args)) => std::process::exit(ext_cli::run(args)),
        Some(Command::Lsp(args)) => std::process::exit(lsp_cli::run(args)),
        None => {}
    }
    let guard = logging::init();
    let on_first_frame = cli.trace_startup.then(|| -> Box<dyn FnOnce()> {
        Box::new(move || {
            let startup_ms = start.elapsed().as_millis();
            tracing::info!(startup_ms, "first frame painted");
            eprintln!("startup_ms={startup_ms}");
        })
    });
    let cwd = std::env::current_dir().ok();
    if cwd.is_some_and(|cwd| prompts_for_folder(cli.path.as_deref(), &cwd)) {
        tracing::info!("starting without a path");
        app::run(None);
        drop(guard);
        return;
    }
    let Target {
        root,
        file,
        single_file,
    } = match resolve_target(cli.path) {
        Ok(target) => target,
        Err(err) => {
            tracing::error!(err, "could not open");
            eprintln!("luc: {err}");
            drop(guard);
            std::process::exit(1);
        }
    };
    tracing::info!(root = %root.display(), file = ?file, single_file, "starting");
    app::run(Some(Launch {
        root,
        file,
        single_file,
        on_first_frame,
    }));
    drop(guard);
}
