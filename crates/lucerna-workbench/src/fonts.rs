//! Fonts bundled with Lucerna: Lilex (editor) and IBM Plex Sans (UI), both
//! under OFL-1.1 (see `assets/fonts/LICENSE-*` and `THIRD_PARTY_NOTICES.md`).

use std::borrow::Cow;

use gpui::{App, SharedString, TextSystem};

/// Family name of the bundled editor font.
pub const EDITOR_FONT_FAMILY: &str = "Lilex";
/// Family name of the bundled UI font.
pub const UI_FONT_FAMILY: &str = "IBM Plex Sans";

const FONTS: [&[u8]; 6] = [
    include_bytes!("../assets/fonts/Lilex-Regular.ttf"),
    include_bytes!("../assets/fonts/Lilex-Bold.ttf"),
    include_bytes!("../assets/fonts/Lilex-Italic.ttf"),
    include_bytes!("../assets/fonts/Lilex-BoldItalic.ttf"),
    include_bytes!("../assets/fonts/IBMPlexSans-Regular.ttf"),
    include_bytes!("../assets/fonts/IBMPlexSans-Medium.ttf"),
];

/// `editor.fontFamily` for `language` (its `[lang]` override applies),
/// else the bundled editor font.
pub fn editor_font_family(language: Option<&str>, cx: &App) -> SharedString {
    cx.global::<crate::AppState>()
        .settings
        .get::<String>("editor.fontFamily", language)
        .unwrap_or_else(|| EDITOR_FONT_FAMILY.into())
        .into()
}

/// Registers the bundled fonts with GPUI. Call it once at startup, before
/// any window opens; a failure is logged and the system fallback is used.
pub fn register(cx: &App) {
    register_with(cx.text_system());
}

/// [`register`] for a text system that is not an app's.
pub fn register_with(text_system: &TextSystem) {
    let fonts = FONTS.iter().map(|f| Cow::Borrowed(*f)).collect();
    if let Err(error) = text_system.add_fonts(fonts) {
        tracing::warn!("could not register the bundled fonts: {error}");
    }
}
