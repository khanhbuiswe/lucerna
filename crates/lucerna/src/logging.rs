//! Logs to daily files in `~/Library/Logs/Lucerna` (stderr when that
//! folder cannot be made), filtered by `LUCERNA_LOG` (default `info`).

use std::io::IsTerminal;
use std::path::PathBuf;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::EnvFilter;

/// How many daily log files are kept.
pub const MAX_LOG_FILES: usize = 7;

/// Installs the global subscriber and a panic hook that logs panics.
/// Keep the guard alive: dropping it flushes and stops the log writer.
pub fn init() -> WorkerGuard {
    let filter = EnvFilter::try_from_env("LUCERNA_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let ((writer, guard), ansi) = match file_appender() {
        Ok(appender) => (tracing_appender::non_blocking(appender), false),
        Err(err) => {
            eprintln!("luc: logging to stderr: {err}");
            (
                tracing_appender::non_blocking(std::io::stderr()),
                std::io::stderr().is_terminal(),
            )
        }
    };
    let installed = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(writer)
        .with_ansi(ansi)
        .try_init();
    if let Err(err) = installed {
        eprintln!("luc: logging not installed: {err}");
    }
    install_panic_hook();
    guard
}

/// `~/Library/Logs/Lucerna`.
pub fn log_dir() -> Option<PathBuf> {
    std::env::home_dir().map(|home| home.join("Library").join("Logs").join("Lucerna"))
}

fn file_appender() -> Result<RollingFileAppender, String> {
    let dir = log_dir().ok_or("no home directory")?;
    std::fs::create_dir_all(&dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("lucerna")
        .filename_suffix("log")
        .max_log_files(MAX_LOG_FILES)
        .build(&dir)
        .map_err(|err| format!("{}: {err}", dir.display()))
}

/// Logs each panic with `tracing::error!`, then runs the default hook.
fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let thread = thread.name().unwrap_or("<unnamed>");
        tracing::error!(thread, "panic: {info}");
        default(info);
    }));
}
