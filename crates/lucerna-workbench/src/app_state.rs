//! Process-wide state shared by every view, stored as a GPUI global.

use std::sync::Arc;

use lucerna_settings::{Keymap, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::{Theme, ThemeRegistry};

pub struct AppState {
    pub settings: SettingsStore,
    pub keymap: Keymap,
    pub themes: ThemeRegistry,
    /// The active theme; views read colors from `theme.ui` when rendering.
    /// Change it with [`AppState::set_theme`].
    pub theme: Arc<Theme>,
    /// Bumped by every theme change. Caches of themed output (shaped
    /// rows, minimap rows) key on it rather than on the theme's address.
    pub theme_generation: u64,
    /// Built once at startup; maps file paths to languages.
    pub languages: Arc<LanguageRegistry>,
}

impl AppState {
    /// Makes `theme` active and bumps [`AppState::theme_generation`],
    /// unless it already is the active theme.
    pub fn set_theme(&mut self, theme: Arc<Theme>) {
        if !Arc::ptr_eq(&self.theme, &theme) {
            self.theme = theme;
            self.theme_generation += 1;
        }
    }
}

impl gpui::Global for AppState {}
