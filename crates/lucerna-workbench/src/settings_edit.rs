//! Writes settings into the user `settings.json` without disturbing its
//! comments or formatting: one or several top-level keys at once, in one
//! atomic file write.

use std::io;
use std::path::{Path, PathBuf};

use gpui::App;
use jsonc_parser::common::Ranged;
use jsonc_parser::tokens::Token;
use jsonc_parser::{CollectOptions, CommentCollectionStrategy, ParseOptions, ast, parse_to_ast};
use lucerna_settings::{Layer, SettingsError, SettingsStore};
use serde_json::Value;

use crate::config_files::{
    ConfigFile, ConfigState, USER_SETTINGS_TEMPLATE, record_error, reload_config,
};

/// Writes `edits` into the user `settings.json` and reloads the
/// configuration. A file that cannot be read or does not parse is left
/// alone and the failure is shown as its configuration error. Without
/// configuration files (no [`ConfigState`]) there is nothing to write.
pub(crate) fn save_user_settings(edits: &[(&str, Value)], cx: &mut App) -> Result<(), ()> {
    let Some(state) = cx.try_global::<ConfigState>() else {
        tracing::warn!("watch_config was not called: the settings are not saved");
        return Ok(());
    };
    let path = state.paths().user_settings();
    write_settings(&path, edits).map_err(|error| {
        tracing::error!(path = %path.display(), %error, "could not save the settings");
        record_error(ConfigFile::UserSettings, path.clone(), error, cx);
    })?;
    reload_config(&[path], cx);
    Ok(())
}

/// Reads the settings file at `path` (the template when it does not
/// exist), applies `edits` and writes the result atomically, once.
pub(crate) fn write_settings(path: &Path, edits: &[(&str, Value)]) -> Result<(), SettingsError> {
    let io_error = |e: io::Error| SettingsError {
        message: e.to_string(),
        line: None,
    };
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => USER_SETTINGS_TEMPLATE.to_string(),
        Err(err) => return Err(io_error(err)),
    };
    let updated = set_settings(&text, edits)?;
    write_atomically(path, &updated).map_err(io_error)
}

/// Writes a sibling temp file and renames it over `path` (over the file a
/// symlink points to), so watchers see one complete change.
fn write_atomically(path: &Path, contents: &str) -> io::Result<()> {
    let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let (Some(dir), Some(name)) = (target.parent(), target.file_name()) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a file path",
        ));
    };
    std::fs::create_dir_all(dir)?;
    let temp: PathBuf = dir.join(format!(
        ".{}.{}.tmp",
        name.to_string_lossy(),
        std::process::id()
    ));
    std::fs::write(&temp, contents)?;
    std::fs::rename(&temp, &target).inspect_err(|_| {
        let _ = std::fs::remove_file(&temp);
    })
}

/// `text` (a JSONC object) with each top-level key of `edits` set, in
/// order. See [`set_value`] for how one key is edited. The text is
/// validated again after every edit, so a failure leaves the caller's
/// text untouched.
pub(crate) fn set_settings(text: &str, edits: &[(&str, Value)]) -> Result<String, SettingsError> {
    let mut text = text.to_string();
    for (key, value) in edits {
        text = set_value(&text, key, value)?;
    }
    Ok(text)
}

/// `text` (a JSONC object) with the top-level `key` set to the string
/// `value`. See [`set_settings`]. The app itself goes through
/// [`set_settings`]; this is the shape the tests exercise.
#[cfg(test)]
fn set_string_setting(text: &str, key: &str, value: &str) -> Result<String, SettingsError> {
    set_settings(text, &[(key, Value::String(value.to_string()))])
}

/// `text` with the top-level `key` set to `value`: an existing value
/// (the last one, when the key is duplicated) is replaced in place, else
/// the member is appended before the closing `}`. Comments and formatting
/// elsewhere are kept.
fn set_value(text: &str, key: &str, value: &Value) -> Result<String, SettingsError> {
    // The same validation settings get when loaded.
    SettingsStore::default().set_layer(Layer::User, text)?;
    let collect = CollectOptions {
        comments: CommentCollectionStrategy::Off,
        tokens: true,
    };
    let parsed =
        parse_to_ast(text, &collect, &ParseOptions::default()).map_err(|e| SettingsError {
            message: e.to_string(),
            line: None,
        })?;
    let quoted = serde_json::to_string(value).expect("a JSON value serializes");
    let member = serde_json::to_string(key).expect("a string serializes");
    let Some(ast::Value::Object(object)) = &parsed.value else {
        // Empty or comments only.
        let sep = if text.is_empty() || text.ends_with('\n') {
            ""
        } else {
            "\n"
        };
        return Ok(format!("{text}{sep}{{\n  {member}: {quoted}\n}}\n"));
    };
    // The store reads the last of duplicate keys, so edit that one.
    if let Some(prop) = object
        .properties
        .iter()
        .rev()
        .find(|p| p.name.as_str() == key)
    {
        let range = prop.value.range();
        return Ok(format!(
            "{}{quoted}{}",
            &text[..range.start],
            &text[range.end..]
        ));
    }
    let close = object.range.end - 1;
    let (after, comma) = match object.properties.last() {
        Some(last) => {
            let end = last.range.end;
            let has_comma = parsed.tokens.iter().flatten().any(|t| {
                matches!(t.token, Token::Comma) && t.range.start >= end && t.range.end <= close
            });
            (end, if has_comma { "" } else { "," })
        }
        None => (object.range.start + 1, ""),
    };
    let between = text[after..close].trim_end();
    Ok(format!(
        "{}{comma}{between}\n  {member}: {quoted}\n{}",
        &text[..after],
        &text[close..]
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const COLOR_THEME_KEY: &str = "workbench.colorTheme";

    fn set(text: &str) -> String {
        set_string_setting(text, COLOR_THEME_KEY, "Lucerna Light").unwrap()
    }

    #[test]
    fn writes_bool_and_number() {
        let updated = set_settings(
            "{\n  // c\n}\n",
            &[("vim.enabled", json!(true)), ("editor.fontSize", json!(15))],
        )
        .unwrap();
        assert!(updated.contains("// c"), "{updated}");
        let mut store = SettingsStore::default();
        store.set_layer(Layer::User, &updated).unwrap();
        assert_eq!(store.get::<bool>("vim.enabled", None), Some(true));
        assert_eq!(store.get::<u32>("editor.fontSize", None), Some(15));
    }

    #[test]
    fn replaces_in_place_keeping_comments() {
        let updated = set_settings(
            "{\n  \"editor.fontSize\": 14, // size\n  \"b\": 2\n}",
            &[("editor.fontSize", json!(15))],
        )
        .unwrap();
        assert!(
            updated.contains("\"editor.fontSize\": 15, // size"),
            "{updated}"
        );
    }

    #[test]
    fn two_keys_one_existing() {
        let updated = set_settings(
            "{\"window.autoDetectColorScheme\": true}",
            &[
                ("window.autoDetectColorScheme", json!(false)),
                ("workbench.colorTheme", json!("Lucerna Dark")),
            ],
        )
        .unwrap();
        assert_eq!(
            updated.matches("window.autoDetectColorScheme").count(),
            1,
            "{updated}"
        );
        let mut store = SettingsStore::default();
        store.set_layer(Layer::User, &updated).unwrap();
        assert_eq!(
            store.get::<bool>("window.autoDetectColorScheme", None),
            Some(false)
        );
        assert_eq!(
            store.get::<String>(COLOR_THEME_KEY, None).as_deref(),
            Some("Lucerna Dark")
        );
    }

    #[test]
    fn trailing_comma_kept_valid() {
        let updated = set_settings(
            "{ \"a\": [1, 2], }",
            &[("vim.enabled", json!(true)), ("editor.fontSize", json!(15))],
        )
        .unwrap();
        let mut store = SettingsStore::default();
        store.set_layer(Layer::User, &updated).unwrap();
        assert_eq!(store.get::<bool>("vim.enabled", None), Some(true));
        assert_eq!(store.get::<u32>("editor.fontSize", None), Some(15));
    }

    #[test]
    fn empty_text() {
        let updated = set_settings("", &[("editor.fontSize", json!(15))]).unwrap();
        assert_eq!(updated, "{\n  \"editor.fontSize\": 15\n}\n");
    }

    #[test]
    fn invalid_text_is_error() {
        let edits = [("editor.fontSize", json!(15))];
        assert!(set_settings("{ \"a\": ", &edits).is_err());
        assert!(set_settings("[]", &edits).is_err());
    }

    #[test]
    fn appends_before_the_closing_brace() {
        assert_eq!(
            set(USER_SETTINGS_TEMPLATE),
            "// Lucerna user settings\n{\n  \"workbench.colorTheme\": \"Lucerna Light\"\n}\n"
        );
        assert_eq!(
            set("{\n  \"a\": 1 // one\n}\n"),
            "{\n  \"a\": 1, // one\n  \"workbench.colorTheme\": \"Lucerna Light\"\n}\n"
        );
        assert_eq!(
            set("{ \"a\": [1, 2], }"),
            "{ \"a\": [1, 2],\n  \"workbench.colorTheme\": \"Lucerna Light\"\n}"
        );
        assert_eq!(
            set("// only a comment\n"),
            "// only a comment\n{\n  \"workbench.colorTheme\": \"Lucerna Light\"\n}\n"
        );
    }

    #[test]
    fn replaces_an_existing_value_in_place() {
        assert_eq!(
            set(
                "{\n  // theme\n  \"workbench.colorTheme\": \"Lucerna Dark\", /* x */\n  \"b\": 2\n}"
            ),
            "{\n  // theme\n  \"workbench.colorTheme\": \"Lucerna Light\", /* x */\n  \"b\": 2\n}"
        );
        // Only top-level keys count.
        assert_eq!(
            set("{ \"x\": { \"workbench.colorTheme\": 1 } }"),
            "{ \"x\": { \"workbench.colorTheme\": 1 },\n  \"workbench.colorTheme\": \"Lucerna Light\"\n}"
        );
    }

    #[test]
    fn edits_the_last_duplicate_key() {
        let text = "{\n  \"workbench.colorTheme\": \"A\",\n  \"workbench.colorTheme\": \"B\"\n}";
        let updated = set(text);
        assert_eq!(
            updated,
            "{\n  \"workbench.colorTheme\": \"A\",\n  \"workbench.colorTheme\": \"Lucerna Light\"\n}"
        );
        let mut store = SettingsStore::default();
        store.set_layer(Layer::User, &updated).unwrap();
        assert_eq!(
            store.get::<String>(COLOR_THEME_KEY, None).as_deref(),
            Some("Lucerna Light")
        );
    }

    #[test]
    fn language_override_is_left_alone() {
        let text = "{\n  \"[rust]\": { \"workbench.colorTheme\": \"A\" }\n}";
        let updated = set(text);
        assert_eq!(
            updated,
            "{\n  \"[rust]\": { \"workbench.colorTheme\": \"A\" },\n  \"workbench.colorTheme\": \"Lucerna Light\"\n}"
        );
        let mut store = SettingsStore::default();
        store.set_layer(Layer::User, &updated).unwrap();
        assert_eq!(
            store.get::<String>(COLOR_THEME_KEY, None).as_deref(),
            Some("Lucerna Light")
        );
        assert_eq!(
            store
                .get::<String>(COLOR_THEME_KEY, Some("rust"))
                .as_deref(),
            Some("A")
        );
    }

    #[test]
    fn refuses_broken_or_non_object_files() {
        assert!(set_string_setting("{ \"a\": ", COLOR_THEME_KEY, "x").is_err());
        assert!(set_string_setting("[]", COLOR_THEME_KEY, "x").is_err());
    }

    #[test]
    fn atomic_write_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        write_settings(
            &path,
            &[
                (COLOR_THEME_KEY, json!("Lucerna Light")),
                ("vim.enabled", json!(true)),
            ],
        )
        .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("\"workbench.colorTheme\": \"Lucerna Light\""),
            "{text}"
        );
        assert!(text.contains("\"vim.enabled\": true"), "{text}");
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("settings.json")]);
    }
}
