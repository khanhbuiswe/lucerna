//! Paths the platform asks a running Lucerna to open: Finder's "Open With",
//! the default app for a file type, a folder dropped on the Dock icon. They
//! arrive as URLs, not on the command line.
//!
//! A process shows one window, because the workspace configuration is a
//! global tied to one root. So a request opens the first window, opens the
//! path in the current window when it lies inside it (as VS Code reuses the
//! window whose folder holds the file), or starts another `luc` process.

use std::path::{Path, PathBuf};

use url::Url;

/// What the current window shows, for [`route`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenWindow {
    root: PathBuf,
    /// Set when the window shows this one file alone, without its folder.
    single_file: Option<PathBuf>,
}

impl OpenWindow {
    /// A window on the folder `root`.
    pub fn folder(root: PathBuf) -> OpenWindow {
        OpenWindow {
            root,
            single_file: None,
        }
    }

    /// A window showing `file` alone.
    pub fn single_file(file: PathBuf) -> OpenWindow {
        let root = file.parent().unwrap_or(Path::new("/")).to_path_buf();
        OpenWindow {
            root,
            single_file: Some(file),
        }
    }
}

/// Where a requested path opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// No window yet: open the path as `luc <path>` would.
    FirstWindow,
    /// The path lies inside the current window: open it there.
    ThisWindow,
    /// Another folder: start a new `luc <path>` process.
    NewProcess,
}

/// The local paths among `urls`; other schemes and malformed URLs are
/// dropped.
pub fn file_paths(urls: &[String]) -> Vec<PathBuf> {
    urls.iter()
        .filter_map(|url| Url::parse(url).ok())
        .filter(|url| url.scheme() == "file")
        .filter_map(|url| url.to_file_path().ok())
        .collect()
}

/// Where the canonical `path` opens, given the current window.
pub fn route(path: &Path, window: Option<&OpenWindow>) -> Route {
    let Some(window) = window else {
        return Route::FirstWindow;
    };
    let inside = match &window.single_file {
        Some(file) => path == file,
        None => path.starts_with(&window.root),
    };
    if inside {
        Route::ThisWindow
    } else {
        Route::NewProcess
    }
}

/// Starts `luc <path>` in its own process, as typing it again would.
pub(crate) fn spawn_process(path: &Path) {
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(err) => {
            tracing::error!(%err, "could not find the luc executable");
            return;
        }
    };
    match std::process::Command::new(exe).arg(path).spawn() {
        Ok(mut child) => {
            // Reap it when it exits, so it does not linger as a zombie.
            std::thread::spawn(move || child.wait());
        }
        Err(err) => tracing::error!(%err, path = %path.display(), "could not start luc"),
    }
}
