//! Process-wide state shared by every view, stored as a GPUI global.

use std::sync::Arc;

use lucerna_settings::{Keymap, SettingsStore};
use lucerna_syntax::LanguageRegistry;
use lucerna_theme::{IconThemeRegistry, ResolvedIcons, Theme, ThemeRegistry};

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
    /// Maps file paths to languages. Replaced when extensions or
    /// `files.associations` change; [`AppState::languages_generation`]
    /// counts the replacements.
    pub languages: Arc<LanguageRegistry>,
    /// Bumped each time [`AppState::languages`] is replaced, so open
    /// documents look their language up again.
    pub languages_generation: u64,
    /// Seti and Lucide plus the icon themes of installed extensions.
    pub icon_themes: IconThemeRegistry,
    /// The icon theme named by `workbench.iconTheme` (Seti when it names
    /// none that is installed).
    pub icons: ResolvedIcons,
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
