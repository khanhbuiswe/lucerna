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
    pub theme: Arc<Theme>,
    /// Built once at startup; maps file paths to languages.
    pub languages: Arc<LanguageRegistry>,
}

impl gpui::Global for AppState {}
