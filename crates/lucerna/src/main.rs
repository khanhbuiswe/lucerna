use std::path::PathBuf;
use std::time::Instant;

use clap::{Parser, Subcommand};
use lucerna::ext_cli;
use lucerna::{Target, logging, resolve_target};
use lucerna_workbench::app::{self, Launch};

/// Lucerna: a read-only code viewer.
#[derive(Debug, Parser)]
#[command(name = "luc", version, args_conflicts_with_subcommands = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    /// File or directory to open. A folder literally named `ext` must be
    /// opened as `luc ./ext`.
    path: Option<PathBuf>,
    /// Print `startup_ms=<n>` to stderr once the first frame is painted.
    #[arg(long)]
    trace_startup: bool,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Search, install, update and remove extensions.
    Ext(ext_cli::ExtArgs),
}

fn main() {
    let start = Instant::now();
    let cli = Cli::parse();
    if let Some(Command::Ext(args)) = cli.command {
        std::process::exit(ext_cli::run(args));
    }
    let guard = logging::init();
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
    let on_first_frame = cli.trace_startup.then(|| -> Box<dyn FnOnce()> {
        Box::new(move || {
            let startup_ms = start.elapsed().as_millis();
            tracing::info!(startup_ms, "first frame painted");
            eprintln!("startup_ms={startup_ms}");
        })
    });
    app::run(Launch {
        root,
        file,
        single_file,
        on_first_frame,
    });
    drop(guard);
}
