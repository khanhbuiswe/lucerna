//! The settings model of the onboarding page: theme mode and editor font,
//! mapped to and from settings keys. No gpui.

use lucerna_settings::SettingsStore;
use lucerna_theme::{Appearance, DEFAULT_DARK_THEME, DEFAULT_LIGHT_THEME, ThemeRegistry};
use serde_json::{Value, json};

use crate::config_files::{
    AUTO_DETECT_KEY, COLOR_THEME_KEY, PREFERRED_DARK_KEY, PREFERRED_LIGHT_KEY, auto_detect,
};

pub const FONT_SIZE_MIN: u32 = 6;
pub const FONT_SIZE_MAX: u32 = 100;
const FONT_SIZE_DEFAULT: u32 = 14;
const FONT_FAMILY_DEFAULT: &str = "Lilex";
pub(crate) const FONT_SIZE_KEY: &str = "editor.fontSize";
pub(crate) const FONT_FAMILY_KEY: &str = "editor.fontFamily";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ThemeMode {
    Light,
    Dark,
    System,
}

/// The mode the settings describe: System while autodetect is on, else the
/// appearance of `workbench.colorTheme`.
pub fn current_mode(settings: &SettingsStore, themes: &ThemeRegistry) -> ThemeMode {
    if auto_detect(settings) {
        return ThemeMode::System;
    }
    let name: String = settings
        .get(COLOR_THEME_KEY, None)
        .unwrap_or_else(|| DEFAULT_DARK_THEME.to_string());
    match themes.get_or_default(&name).appearance {
        Appearance::Light => ThemeMode::Light,
        Appearance::Dark => ThemeMode::Dark,
    }
}

/// The preferred theme for `appearance`, read directly from its key (not
/// `workbench.colorTheme`, which may name the other appearance).
pub fn preferred_theme(settings: &SettingsStore, appearance: Appearance) -> String {
    let (key, default) = match appearance {
        Appearance::Light => (PREFERRED_LIGHT_KEY, DEFAULT_LIGHT_THEME),
        Appearance::Dark => (PREFERRED_DARK_KEY, DEFAULT_DARK_THEME),
    };
    settings
        .get(key, None)
        .unwrap_or_else(|| default.to_string())
}

/// The settings edits that apply `mode`.
pub fn mode_edits(mode: ThemeMode, settings: &SettingsStore) -> Vec<(&'static str, Value)> {
    let fixed = |appearance| {
        vec![
            (AUTO_DETECT_KEY, json!(false)),
            (
                COLOR_THEME_KEY,
                json!(preferred_theme(settings, appearance)),
            ),
        ]
    };
    match mode {
        ThemeMode::System => vec![(AUTO_DETECT_KEY, json!(true))],
        ThemeMode::Light => fixed(Appearance::Light),
        ThemeMode::Dark => fixed(Appearance::Dark),
    }
}

/// The theme(s) the preview shows once `mode` is applied: one theme, or the
/// light and dark preferred themes side by side for System.
pub fn preview_themes(mode: ThemeMode, settings: &SettingsStore) -> (String, Option<String>) {
    match mode {
        ThemeMode::Light => (preferred_theme(settings, Appearance::Light), None),
        ThemeMode::Dark => (preferred_theme(settings, Appearance::Dark), None),
        ThemeMode::System => (
            preferred_theme(settings, Appearance::Light),
            Some(preferred_theme(settings, Appearance::Dark)),
        ),
    }
}

pub fn current_font_size(settings: &SettingsStore) -> u32 {
    let size: f64 = settings
        .get(FONT_SIZE_KEY, None)
        .unwrap_or(FONT_SIZE_DEFAULT as f64);
    if size.is_nan() {
        return FONT_SIZE_DEFAULT;
    }
    size.round()
        .clamp(FONT_SIZE_MIN as f64, FONT_SIZE_MAX as f64) as u32
}

/// `current` moved by `delta`, or `None` when it is already at the limit.
pub fn stepped_font_size(current: u32, delta: i32) -> Option<u32> {
    let next = (current as i64 + delta as i64).clamp(FONT_SIZE_MIN as i64, FONT_SIZE_MAX as i64);
    let next = next as u32;
    (next != current).then_some(next)
}

pub fn current_font_family(settings: &SettingsStore) -> String {
    settings
        .get::<String>(FONT_FAMILY_KEY, None)
        .filter(|family| !family.trim().is_empty())
        .unwrap_or_else(|| FONT_FAMILY_DEFAULT.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lucerna_settings::Layer;

    fn store(user: &str) -> SettingsStore {
        let mut store = SettingsStore::with_defaults();
        store.set_layer(Layer::User, user).unwrap();
        store
    }

    fn mode(user: &str) -> ThemeMode {
        current_mode(&store(user), &ThemeRegistry::builtin())
    }

    #[test]
    fn defaults_are_system() {
        assert_eq!(mode("{}"), ThemeMode::System);
    }

    #[test]
    fn autodetect_off_dark_theme_is_dark() {
        let user =
            r#"{"window.autoDetectColorScheme": false, "workbench.colorTheme": "Lucerna Dark"}"#;
        assert_eq!(mode(user), ThemeMode::Dark);
    }

    #[test]
    fn autodetect_off_light_theme_is_light() {
        let user =
            r#"{"window.autoDetectColorScheme": false, "workbench.colorTheme": "Lucerna Light"}"#;
        assert_eq!(mode(user), ThemeMode::Light);
    }

    #[test]
    fn dark_edits_use_preferred_dark() {
        let settings = store(
            r#"{"window.autoDetectColorScheme": false, "workbench.preferredDarkColorTheme": "My Dark"}"#,
        );
        let edits = mode_edits(ThemeMode::Dark, &settings);
        assert!(edits.contains(&("workbench.colorTheme", json!("My Dark"))));
        assert!(edits.contains(&("window.autoDetectColorScheme", json!(false))));
    }

    #[test]
    fn light_edits_default_to_lucerna_light() {
        let edits = mode_edits(ThemeMode::Light, &store("{}"));
        assert!(edits.contains(&("workbench.colorTheme", json!("Lucerna Light"))));
    }

    #[test]
    fn system_edits_only_autodetect() {
        let edits = mode_edits(ThemeMode::System, &store("{}"));
        assert_eq!(edits, vec![("window.autoDetectColorScheme", json!(true))]);
    }

    #[test]
    fn system_preview_is_split() {
        let (light, dark) = preview_themes(ThemeMode::System, &store("{}"));
        assert_eq!(light, "Lucerna Light");
        assert_eq!(dark.as_deref(), Some("Lucerna Dark"));
        let (one, none) = preview_themes(ThemeMode::Dark, &store("{}"));
        assert_eq!(one, "Lucerna Dark");
        assert_eq!(none, None);
    }

    #[test]
    fn font_size_steps_and_clamps() {
        assert_eq!(stepped_font_size(14, 1), Some(15));
        assert_eq!(stepped_font_size(100, 1), None);
        assert_eq!(stepped_font_size(6, -1), None);
    }

    #[test]
    fn font_size_rounds_and_clamps() {
        assert_eq!(
            current_font_size(&store(r#"{"editor.fontSize": 13.6}"#)),
            14
        );
        assert_eq!(
            current_font_size(&store(r#"{"editor.fontSize": 200}"#)),
            100
        );
        assert_eq!(current_font_size(&store("{}")), 14);
    }

    #[test]
    fn family_fallback_is_lilex() {
        assert_eq!(current_font_family(&store("{}")), "Lilex");
        assert_eq!(
            current_font_family(&store(r#"{"editor.fontFamily": ""}"#)),
            "Lilex"
        );
        assert_eq!(
            current_font_family(&store(r#"{"editor.fontFamily": "Menlo"}"#)),
            "Menlo"
        );
    }
}
