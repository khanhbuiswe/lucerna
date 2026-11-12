//! What Lucerna shows when it starts without a folder to open.

use lucerna_settings::SettingsStore;

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
