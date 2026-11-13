//! What the Explorer asks of the operating system: show a file in the
//! system file manager, open a terminal in a folder and move a file to the
//! Trash. A trait so tests can record the calls (GPUI's test platform
//! panics on `reveal_path`) and choose whether the Trash works.

use std::path::Path;
use std::sync::Arc;

use gpui::{App, Global};

/// `Send + Sync`: [`SystemShell::trash`] runs on the background executor.
pub trait SystemShell: Send + Sync + 'static {
    /// Shows `path` selected in the system file manager.
    fn reveal(&self, path: &Path, cx: &App);
    /// Opens the terminal program `exec` in `dir`.
    fn open_terminal(&self, dir: &Path, exec: &str) -> std::io::Result<()>;
    /// Moves `path` (a file or a whole folder) to the Trash. Blocks: call
    /// it off the UI thread.
    fn trash(&self, path: &Path) -> Result<(), String>;
}

/// Overrides the shell [`system_shell`] returns.
pub struct SystemShellGlobal(pub Arc<dyn SystemShell>);

impl Global for SystemShellGlobal {}

/// The real operating system.
pub struct NativeShell;

impl SystemShell for NativeShell {
    fn reveal(&self, path: &Path, cx: &App) {
        cx.reveal_path(path);
    }

    fn open_terminal(&self, dir: &Path, exec: &str) -> std::io::Result<()> {
        #[cfg(target_os = "macos")]
        let mut command = {
            let mut command = std::process::Command::new("open");
            command.arg("-a").arg(exec).arg(dir);
            command
        };
        #[cfg(target_os = "windows")]
        let mut command = {
            let mut command = std::process::Command::new("cmd");
            command.args(["/c", "start", "", exec]).current_dir(dir);
            command
        };
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let mut command = {
            let mut command = std::process::Command::new(exec);
            command.current_dir(dir);
            command
        };
        command.spawn().map(drop)
    }

    fn trash(&self, path: &Path) -> Result<(), String> {
        lucerna_project::fs_ops::trash(path).map_err(|e| e.to_string())
    }
}

/// The installed shell, or the real one when none is installed.
pub fn system_shell(cx: &App) -> Arc<dyn SystemShell> {
    cx.try_global::<SystemShellGlobal>().map_or_else(
        || Arc::new(NativeShell) as Arc<dyn SystemShell>,
        |g| g.0.clone(),
    )
}
