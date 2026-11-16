//! What Lucerna shows when it starts without a folder to open.

use lucerna_settings::SettingsStore;

use crate::recent::{RecentEntry, RecentList};
use crate::session::SessionWindow;

/// The window opened at startup when no folder was requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Startup {
    /// The Welcome screen.
    Welcome,
    /// The native folder prompt; Cancel quits.
    FolderPrompt,
}

/// Reads `workbench.startupEditor`: `"none"` asks for a folder, any other
/// value (or none) shows Welcome.
pub fn startup_choice(settings: &SettingsStore) -> Startup {
    match settings
        .get::<String>("workbench.startupEditor", None)
        .as_deref()
    {
        Some("none") => Startup::FolderPrompt,
        _ => Startup::Welcome,
    }
}

/// Which windows a start reopens: `window.restoreWindows` (spec M12 of
/// the multi-window design, VS Code's values).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreWindows {
    /// Every window, also when the start opens a path.
    Preserve,
    /// Every window.
    All,
    /// The folder windows.
    Folders,
    /// The most recently used window.
    One,
    /// None: the startup editor shows instead.
    None,
}

/// Reads `window.restoreWindows`; a value it does not know (or none) is
/// `"all"`, VS Code's default.
pub fn restore_windows(settings: &SettingsStore) -> RestoreWindows {
    match settings
        .get::<String>("window.restoreWindows", None)
        .as_deref()
    {
        Some("preserve") => RestoreWindows::Preserve,
        Some("folders") => RestoreWindows::Folders,
        Some("one") => RestoreWindows::One,
        Some("none") => RestoreWindows::None,
        _ => RestoreWindows::All,
    }
}

/// The workspace to reopen: the newest recent entry, folder or file
/// opened alone, that still exists.
pub fn last_workspace(list: &RecentList) -> Option<&RecentEntry> {
    list.entries.iter().find(|entry| entry.exists())
}

/// What the window opened without a path shows first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupScreen {
    /// Reopen these windows, most recently used first.
    Restore(Vec<SessionWindow>),
    /// The first-run onboarding page.
    Onboarding,
    Welcome,
    FolderPrompt,
}

/// Picks the startup screen: windows to restore win, then the first-run
/// onboarding (`onboarded` is `Some(false)`), then `choice`. `onboarded`
/// is `None` when there is no data directory to remember it in.
pub fn startup_screen(
    restore: Vec<SessionWindow>,
    onboarded: Option<bool>,
    choice: Startup,
) -> StartupScreen {
    if !restore.is_empty() {
        return StartupScreen::Restore(restore);
    }
    if onboarded == Some(false) {
        return StartupScreen::Onboarding;
    }
    match choice {
        Startup::Welcome => StartupScreen::Welcome,
        Startup::FolderPrompt => StartupScreen::FolderPrompt,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lucerna_settings::Layer;

    fn with_user(json: &str) -> SettingsStore {
        let mut store = SettingsStore::with_defaults();
        store.set_layer(Layer::User, json).unwrap();
        store
    }

    #[test]
    fn restore_windows_default_is_all() {
        assert_eq!(
            restore_windows(&SettingsStore::with_defaults()),
            RestoreWindows::All
        );
        assert_eq!(
            restore_windows(&SettingsStore::default()),
            RestoreWindows::All
        );
        for (value, mode) in [
            (r#""preserve""#, RestoreWindows::Preserve),
            (r#""all""#, RestoreWindows::All),
            (r#""folders""#, RestoreWindows::Folders),
            (r#""one""#, RestoreWindows::One),
            (r#""none""#, RestoreWindows::None),
            (r#""most""#, RestoreWindows::All),
            ("42", RestoreWindows::All),
        ] {
            let store = with_user(&format!(r#"{{"window.restoreWindows":{value}}}"#));
            assert_eq!(restore_windows(&store), mode, "{value}");
        }
    }

    #[test]
    fn last_workspace_is_the_newest_that_exists() {
        use crate::recent::{RecentEntry, RecentKind, RecentList};
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("gone");
        let folder = dir.path().join("project");
        std::fs::create_dir(&folder).unwrap();
        let file = dir.path().join("notes.md");
        std::fs::write(&file, "").unwrap();
        let entry = |path: &std::path::Path, kind| RecentEntry {
            path: path.to_path_buf(),
            kind,
        };
        let list = RecentList {
            entries: vec![
                entry(&gone, RecentKind::Folder),
                entry(&folder, RecentKind::Folder),
                entry(&file, RecentKind::File),
            ],
        };
        assert_eq!(last_workspace(&list).map(|e| e.path.clone()), Some(folder));
        // A file opened alone is a workspace too.
        let list = RecentList {
            entries: vec![entry(&file, RecentKind::File)],
        };
        assert_eq!(last_workspace(&list).map(|e| e.path.clone()), Some(file));
        assert!(last_workspace(&RecentList::default()).is_none());
    }

    #[test]
    fn none_prompts_for_a_folder() {
        let store = with_user(r#"{"workbench.startupEditor":"none"}"#);
        assert_eq!(startup_choice(&store), Startup::FolderPrompt);
    }

    #[test]
    fn other_values_show_welcome() {
        assert_eq!(
            startup_choice(&SettingsStore::with_defaults()),
            Startup::Welcome
        );
        assert_eq!(startup_choice(&SettingsStore::default()), Startup::Welcome);
        for value in [
            r#""welcomePage""#,
            r#""welcomePageInEmptyWorkbench""#,
            r#""readme""#,
            r#""terminal""#,
            "42",
        ] {
            let store = with_user(&format!(r#"{{"workbench.startupEditor":{value}}}"#));
            assert_eq!(startup_choice(&store), Startup::Welcome, "{value}");
        }
    }

    #[test]
    fn restore_wins() {
        let windows = vec![SessionWindow {
            kind: crate::recent::RecentKind::Folder,
            path: std::path::PathBuf::from("/p"),
        }];
        assert_eq!(
            startup_screen(windows.clone(), Some(false), Startup::Welcome),
            StartupScreen::Restore(windows)
        );
    }

    #[test]
    fn first_run_onboarding() {
        assert_eq!(
            startup_screen(Vec::new(), Some(false), Startup::Welcome),
            StartupScreen::Onboarding
        );
    }

    #[test]
    fn first_run_beats_folder_prompt() {
        assert_eq!(
            startup_screen(Vec::new(), Some(false), Startup::FolderPrompt),
            StartupScreen::Onboarding
        );
    }

    #[test]
    fn onboarded_follows_choice() {
        assert_eq!(
            startup_screen(Vec::new(), Some(true), Startup::Welcome),
            StartupScreen::Welcome
        );
        assert_eq!(
            startup_screen(Vec::new(), Some(true), Startup::FolderPrompt),
            StartupScreen::FolderPrompt
        );
    }

    #[test]
    fn no_data_dir_follows_choice() {
        assert_eq!(
            startup_screen(Vec::new(), None, Startup::Welcome),
            StartupScreen::Welcome
        );
        assert_eq!(
            startup_screen(Vec::new(), None, Startup::FolderPrompt),
            StartupScreen::FolderPrompt
        );
    }
}
