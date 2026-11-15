//! What Lucerna shows when it starts without a folder to open.

use std::path::PathBuf;

use lucerna_settings::SettingsStore;

use crate::recent::{RecentEntry, RecentList};

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

/// Reads `window.restoreWindows`: whether a start without a path reopens
/// the last workspace, as Zed's `restore_on_startup: "last_workspace"`
/// does. `"none"` does not; any other value (or none) does, one window
/// only, since each Lucerna window is a process of its own. Spec:
/// `docs/superpowers/specs/2026-11-15-restore-last-workspace-design.md`.
pub fn restores_last_workspace(settings: &SettingsStore) -> bool {
    settings
        .get::<String>("window.restoreWindows", None)
        .as_deref()
        != Some("none")
}

/// The workspace to reopen: the newest recent entry, folder or file
/// opened alone, that still exists.
pub fn last_workspace(list: &RecentList) -> Option<&RecentEntry> {
    list.entries.iter().find(|entry| entry.exists())
}

/// What the window opened without a path shows first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupScreen {
    /// Reopen this workspace.
    Restore(PathBuf),
    /// The first-run onboarding page.
    Onboarding,
    Welcome,
    FolderPrompt,
}

/// Picks the startup screen: the workspace to restore wins, then the
/// first-run onboarding (`onboarded` is `Some(false)`), then `choice`.
/// `onboarded` is `None` when there is no data directory to remember it in.
pub fn startup_screen(
    restore: Option<&RecentEntry>,
    onboarded: Option<bool>,
    choice: Startup,
) -> StartupScreen {
    if let Some(entry) = restore {
        return StartupScreen::Restore(entry.path.clone());
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
    fn restores_unless_none() {
        assert!(restores_last_workspace(&SettingsStore::with_defaults()));
        assert!(restores_last_workspace(&SettingsStore::default()));
        for value in [r#""one""#, r#""all""#, r#""folders""#, r#""preserve""#] {
            let store = with_user(&format!(r#"{{"window.restoreWindows":{value}}}"#));
            assert!(restores_last_workspace(&store), "{value}");
        }
        let store = with_user(r#"{"window.restoreWindows":"none"}"#);
        assert!(!restores_last_workspace(&store));
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

    fn entry(path: &str) -> RecentEntry {
        RecentEntry {
            path: PathBuf::from(path),
            kind: crate::recent::RecentKind::Folder,
        }
    }

    #[test]
    fn restore_wins() {
        let e = entry("/p");
        assert_eq!(
            startup_screen(Some(&e), Some(false), Startup::Welcome),
            StartupScreen::Restore(PathBuf::from("/p"))
        );
    }

    #[test]
    fn first_run_onboarding() {
        assert_eq!(
            startup_screen(None, Some(false), Startup::Welcome),
            StartupScreen::Onboarding
        );
    }

    #[test]
    fn first_run_beats_folder_prompt() {
        assert_eq!(
            startup_screen(None, Some(false), Startup::FolderPrompt),
            StartupScreen::Onboarding
        );
    }

    #[test]
    fn onboarded_follows_choice() {
        assert_eq!(
            startup_screen(None, Some(true), Startup::Welcome),
            StartupScreen::Welcome
        );
        assert_eq!(
            startup_screen(None, Some(true), Startup::FolderPrompt),
            StartupScreen::FolderPrompt
        );
    }

    #[test]
    fn no_data_dir_follows_choice() {
        assert_eq!(
            startup_screen(None, None, Startup::Welcome),
            StartupScreen::Welcome
        );
        assert_eq!(
            startup_screen(None, None, Startup::FolderPrompt),
            StartupScreen::FolderPrompt
        );
    }
}
