//! Which window a path opens in (spec §6.3 of the multi-window design),
//! without GPUI: the app's router describes its windows as
//! [`WindowInfo`]s, asks [`choose`], then does what the [`Choice`] says.
//!
//! The rules follow VS Code (`findWindowOnWorkspaceOrFolder`,
//! `findWindowOnFile`, `openFilesInNewWindow: "off"`); an empty window is
//! reused as Zed reuses one.

use std::path::{Path, PathBuf};

use url::Url;

/// What a window shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowKind {
    /// No folder: the Welcome screen or the Get Started page.
    Welcome,
    /// A workbench on this canonical folder.
    Folder(PathBuf),
    /// A workbench on this canonical file alone (single-file mode).
    File(PathBuf),
}

/// One window, for [`choose`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    pub kind: WindowKind,
    /// Larger is used more recently.
    pub last_active: u64,
}

/// Where the caller wants the path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// Wherever the rules put it.
    Auto,
    /// A new window, unless the path is open in one already.
    NewWindow,
    /// In place of what the window at this index shows (spec W8, W10),
    /// unless the path is open in a window already.
    ReplaceIn(usize),
}

/// What to do with the path; indices are positions in the `windows`
/// slice given to [`choose`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    /// The path is what this window shows: bring it to the front.
    Focus(usize),
    /// Open the file as a pinned tab in this workbench window.
    OpenTab(usize),
    /// Show the path in this window in place of what it shows.
    ReplaceRoot(usize),
    /// Open a window on the path.
    NewWindow,
}

/// Where the canonical `path` opens among `windows` (rules 1–4 of spec
/// §6.3). `is_dir` says whether `path` is a folder.
///
/// 1. A folder that is a folder window's root, or a file that is a
///    single-file window's file: focus that window, whatever the target.
/// 2. A file inside the roots of folder windows: the deepest root, as a
///    tab. A single-file window has no root here: its folder is not
///    scanned.
/// 3. A file outside every root: the most recently used folder window, as
///    a tab; else the most recently used Welcome window turns into a
///    single-file window; else a new window.
/// 4. A folder not open: the most recently used Welcome window; else a new
///    window.
pub fn choose(windows: &[WindowInfo], path: &Path, is_dir: bool, target: Target) -> Choice {
    let showing = most_recent(windows, |kind| match kind {
        WindowKind::Folder(root) => is_dir && root == path,
        WindowKind::File(file) => !is_dir && file == path,
        WindowKind::Welcome => false,
    });
    if let Some(index) = showing {
        return Choice::Focus(index);
    }
    match target {
        Target::NewWindow => return Choice::NewWindow,
        Target::ReplaceIn(index) => return Choice::ReplaceRoot(index),
        Target::Auto => {}
    }
    if !is_dir {
        let deepest = windows
            .iter()
            .enumerate()
            .filter_map(|(index, window)| match &window.kind {
                // Component by component: `/a/bc` is not inside `/a/b`.
                WindowKind::Folder(root) if path.starts_with(root) => {
                    Some((index, root.components().count(), window.last_active))
                }
                _ => None,
            })
            .max_by_key(|&(_, depth, last_active)| (depth, last_active));
        if let Some((index, _, _)) = deepest {
            return Choice::OpenTab(index);
        }
        if let Some(index) = most_recent(windows, |kind| matches!(kind, WindowKind::Folder(_))) {
            return Choice::OpenTab(index);
        }
    }
    match most_recent(windows, |kind| *kind == WindowKind::Welcome) {
        Some(index) => Choice::ReplaceRoot(index),
        None => Choice::NewWindow,
    }
}

/// The index of the most recently used window whose kind `matches`.
fn most_recent(windows: &[WindowInfo], matches: impl Fn(&WindowKind) -> bool) -> Option<usize> {
    windows
        .iter()
        .enumerate()
        .filter(|(_, window)| matches(&window.kind))
        .max_by_key(|(_, window)| window.last_active)
        .map(|(index, _)| index)
}

/// The local paths among `urls`, as the platform sends them (Finder's
/// "Open With", a folder dropped on the Dock icon); other schemes and
/// malformed URLs are dropped.
pub fn file_paths(urls: &[String]) -> Vec<PathBuf> {
    urls.iter()
        .filter_map(|url| Url::parse(url).ok())
        .filter(|url| url.scheme() == "file")
        .filter_map(|url| url.to_file_path().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(path: &str, last_active: u64) -> WindowInfo {
        WindowInfo {
            kind: WindowKind::Folder(PathBuf::from(path)),
            last_active,
        }
    }

    fn file(path: &str, last_active: u64) -> WindowInfo {
        WindowInfo {
            kind: WindowKind::File(PathBuf::from(path)),
            last_active,
        }
    }

    fn welcome(last_active: u64) -> WindowInfo {
        WindowInfo {
            kind: WindowKind::Welcome,
            last_active,
        }
    }

    fn auto(windows: &[WindowInfo], path: &str, is_dir: bool) -> Choice {
        choose(windows, Path::new(path), is_dir, Target::Auto)
    }

    #[test]
    fn choose_focuses_open_folder() {
        let windows = [folder("/a", 2), folder("/b", 1)];
        assert_eq!(auto(&windows, "/b", true), Choice::Focus(1));
    }

    #[test]
    fn choose_focuses_open_single_file() {
        let windows = [folder("/z", 2), file("/a/x.rs", 1)];
        assert_eq!(auto(&windows, "/a/x.rs", false), Choice::Focus(1));
    }

    #[test]
    fn choose_deepest_root_wins() {
        let windows = [folder("/a", 5), folder("/a/b", 1)];
        assert_eq!(auto(&windows, "/a/b/c.rs", false), Choice::OpenTab(1));
        assert_eq!(auto(&windows, "/a/c.rs", false), Choice::OpenTab(0));
    }

    #[test]
    fn choose_single_file_window_is_not_a_root() {
        let windows = [file("/a/x.rs", 5), folder("/z", 3), welcome(9)];
        assert_eq!(auto(&windows, "/a/y.rs", false), Choice::OpenTab(1));
    }

    #[test]
    fn choose_loose_file_goes_to_last_active_folder() {
        let windows = [folder("/a", 1), folder("/b", 7), folder("/c", 3)];
        assert_eq!(auto(&windows, "/elsewhere/x.rs", false), Choice::OpenTab(1));
    }

    #[test]
    fn choose_loose_file_reuses_welcome() {
        let windows = [file("/a/x.rs", 4), welcome(2)];
        assert_eq!(auto(&windows, "/b/y.rs", false), Choice::ReplaceRoot(1));
    }

    #[test]
    fn choose_loose_file_new_window_when_none() {
        assert_eq!(auto(&[], "/b/y.rs", false), Choice::NewWindow);
        let windows = [file("/a/x.rs", 4)];
        assert_eq!(auto(&windows, "/b/y.rs", false), Choice::NewWindow);
    }

    #[test]
    fn choose_folder_reuses_welcome() {
        let windows = [welcome(3), folder("/a", 9), welcome(5)];
        assert_eq!(auto(&windows, "/b", true), Choice::ReplaceRoot(2));
        assert_eq!(auto(&[folder("/a", 1)], "/b", true), Choice::NewWindow);
    }

    #[test]
    fn choose_new_window_still_focuses_open() {
        let windows = [welcome(3), folder("/a", 1)];
        let path = Path::new("/a");
        assert_eq!(
            choose(&windows, path, true, Target::NewWindow),
            Choice::Focus(1)
        );
        assert_eq!(
            choose(&windows, Path::new("/b"), true, Target::NewWindow),
            Choice::NewWindow
        );
        assert_eq!(
            choose(&windows, Path::new("/a/x.rs"), false, Target::NewWindow),
            Choice::NewWindow
        );
    }

    #[test]
    fn choose_replace_in() {
        let windows = [folder("/a", 1), folder("/b", 2)];
        assert_eq!(
            choose(&windows, Path::new("/c"), true, Target::ReplaceIn(1)),
            Choice::ReplaceRoot(1)
        );
        // Open elsewhere already: focus it, keep this window.
        assert_eq!(
            choose(&windows, Path::new("/a"), true, Target::ReplaceIn(1)),
            Choice::Focus(0)
        );
    }

    #[test]
    fn choose_focuses_canonical_root() {
        let windows = [folder("/a/b", 1), welcome(2)];
        // A sibling whose name only starts with the root's is outside it.
        assert_eq!(auto(&windows, "/a/bc", true), Choice::ReplaceRoot(1));
        assert_eq!(auto(&windows, "/a/bc/x.rs", false), Choice::OpenTab(0));
        assert_eq!(
            choose(
                &[folder("/a/b", 1)],
                Path::new("/a/bc/x.rs"),
                false,
                Target::Auto
            ),
            Choice::OpenTab(0),
            "a loose file still goes to the last folder window"
        );
        assert_eq!(auto(&windows, "/a/b", true), Choice::Focus(0));
        let deep = [folder("/a/b", 1), folder("/a", 0)];
        assert_eq!(auto(&deep, "/a/bc/x.rs", false), Choice::OpenTab(1));
    }

    #[test]
    fn file_paths_keeps_file_urls_and_decodes_them() {
        let urls = vec![
            "file:///Users/me/My%20Project/main.rs".to_string(),
            "file:///tmp/".to_string(),
            "https://example.com/x".to_string(),
            "not a url".to_string(),
        ];
        assert_eq!(
            file_paths(&urls),
            vec![
                PathBuf::from("/Users/me/My Project/main.rs"),
                PathBuf::from("/tmp/"),
            ]
        );
    }
}
