use std::path::PathBuf;
use std::time::Instant;

use clap::Parser;
use lucerna::{logging, resolve_target};
use lucerna_workbench::app::{self, Launch};

/// Lucerna: a read-only code viewer.
#[derive(Debug, Parser)]
#[command(name = "luc", version)]
struct Cli {
    /// File or directory to open.
    path: Option<PathBuf>,
    /// Print `startup_ms=<n>` to stderr once the first frame is painted.
    #[arg(long)]
    trace_startup: bool,
}

fn main() {
    let start = Instant::now();
    let cli = Cli::parse();
    let guard = logging::init();
    let (root, file) = match resolve_target(cli.path) {
        Ok(target) => target,
        Err(err) => {
            tracing::error!(err, "could not open");
            eprintln!("luc: {err}");
            drop(guard);
            std::process::exit(1);
        }
    };
    tracing::info!(root = %root.display(), file = ?file, "starting");
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
        on_first_frame,
    });
    drop(guard);
}
