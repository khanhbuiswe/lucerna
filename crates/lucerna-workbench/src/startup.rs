//! What Lucerna shows when it starts without a folder to open.

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
}
