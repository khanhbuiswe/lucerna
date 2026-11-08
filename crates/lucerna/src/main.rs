use std::path::PathBuf;

use clap::Parser;

/// Lucerna: a read-only code viewer.
#[derive(Debug, Parser)]
#[command(name = "luc", version)]
struct Cli {
    /// File or directory to open.
    path: Option<PathBuf>,
}

fn main() {
    let _cli = Cli::parse();
}
