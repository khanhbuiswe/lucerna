//! What a terminal reads from settings and the theme: its font (spec §7.3),
//! its palette (§7.1) and how its shell starts (§6.2).

use std::collections::HashMap;
use std::path::Path;

use gpui::{App, FontWeight, Pixels, SharedString, px};
use lucerna_terminal::{Rgba8, TerminalOptions, TerminalPalette, build_env, resolve_shell};

use crate::AppState;
use crate::fonts::editor_font_family;

/// `terminal.integrated.lineHeight` when unset.
const DEFAULT_LINE_HEIGHT: f32 = 1.2;
/// `editor.fontSize` when even that is unset.
const FALLBACK_FONT_SIZE: f32 = 14.;
/// `terminal.integrated.scrollback` (spec §9.2); the setting comes later.
const SCROLLBACK: usize = 10_000;
/// The grid a terminal starts with, until its first layout measures it.
const INITIAL_ROWS: u16 = 24;
const INITIAL_COLS: u16 = 80;

/// The terminal's font, from `terminal.integrated.*` with the editor's
/// font as the fallback.
#[derive(Debug, Clone, PartialEq)]
pub struct TerminalFont {
    pub family: SharedString,
    pub size: Pixels,
    /// A factor of `size`.
    pub line_height: f32,
    /// Extra pixels between cells.
    pub letter_spacing: f32,
    pub weight: FontWeight,
    pub bold_weight: FontWeight,
}

impl TerminalFont {
    /// Height of one row, in pixels.
    pub fn cell_height(&self) -> Pixels {
        self.size * self.line_height
    }
}

/// The terminal font (spec §7.3): an empty `fontFamily` is
/// `editor.fontFamily`, an unset `fontSize` is `editor.fontSize`,
/// `lineHeight` defaults to 1.2, `letterSpacing` to 0, the weights to
/// normal and bold.
pub fn terminal_font(cx: &App) -> TerminalFont {
    let settings = &cx.global::<AppState>().settings;
    let family = settings
        .get::<String>("terminal.integrated.fontFamily", None)
        .filter(|f| !f.trim().is_empty())
        .map(SharedString::from)
        .unwrap_or_else(|| editor_font_family(None, cx));
    let positive = |key: &str| settings.get::<f32>(key, None).filter(|v| *v > 0.);
    let size = positive("terminal.integrated.fontSize")
        .or_else(|| positive("editor.fontSize"))
        .unwrap_or(FALLBACK_FONT_SIZE);
    let line_height = positive("terminal.integrated.lineHeight")
        .unwrap_or(DEFAULT_LINE_HEIGHT)
        .max(1.);
    let letter_spacing = settings
        .get::<f32>("terminal.integrated.letterSpacing", None)
        .filter(|v| v.is_finite())
        .unwrap_or(0.);
    let weight = |key: &str, default: FontWeight| {
        settings
            .get::<serde_json::Value>(key, None)
            .and_then(|v| parse_weight(&v))
            .unwrap_or(default)
    };
    TerminalFont {
        family,
        size: px(size),
        line_height,
        letter_spacing,
        weight: weight("terminal.integrated.fontWeight", FontWeight::NORMAL),
        bold_weight: weight("terminal.integrated.fontWeightBold", FontWeight::BOLD),
    }
}

/// VS Code's font weights: `"normal"`, `"bold"` or a number 1–1000, which
/// may be written as a string.
fn parse_weight(value: &serde_json::Value) -> Option<FontWeight> {
    let number = match value {
        serde_json::Value::String(s) => match s.trim() {
            "normal" => return Some(FontWeight::NORMAL),
            "bold" => return Some(FontWeight::BOLD),
            s => s.parse::<f32>().ok()?,
        },
        serde_json::Value::Number(n) => n.as_f64()? as f32,
        _ => return None,
    };
    (1. ..=1000.)
        .contains(&number)
        .then_some(FontWeight(number))
}

/// `terminal.integrated.minimumContrastRatio`; 1 turns it off.
pub(crate) fn minimum_contrast(cx: &App) -> f32 {
    cx.global::<AppState>()
        .settings
        .get::<f32>("terminal.integrated.minimumContrastRatio", None)
        .filter(|v| v.is_finite())
        .unwrap_or(4.5)
        .clamp(1., 21.)
}

/// `terminal.integrated.drawBoldTextInBrightColors`.
pub(crate) fn bold_is_bright(cx: &App) -> bool {
    cx.global::<AppState>()
        .settings
        .get::<bool>("terminal.integrated.drawBoldTextInBrightColors", None)
        .unwrap_or(true)
}

fn rgba8(c: lucerna_theme::Rgba) -> Rgba8 {
    let byte = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
    Rgba8 {
        r: byte(c.r),
        g: byte(c.g),
        b: byte(c.b),
        a: byte(c.a),
    }
}

/// The active theme's terminal colours.
pub(crate) fn palette(cx: &App) -> TerminalPalette {
    let t = &cx.global::<AppState>().theme.terminal;
    TerminalPalette {
        ansi: t.ansi.map(rgba8),
        dim: t.dim.map(rgba8),
        foreground: rgba8(t.foreground),
        bright_foreground: rgba8(t.bright_foreground),
        dim_foreground: rgba8(t.dim_foreground),
        background: rgba8(t.background),
        cursor: rgba8(t.cursor),
        selection: rgba8(t.selection),
    }
}

/// What a right click does (`terminal.integrated.rightClickBehavior`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RightClick {
    /// Selects the word under the mouse. `default` (a context menu in VS
    /// Code) acts like this until Lucerna has one.
    SelectWord,
    /// Copies the selection and clears it, or pastes when there is none.
    CopyPaste,
    Paste,
    Nothing,
}

/// When a paste of several lines asks first
/// (`terminal.integrated.enableMultiLinePasteWarning`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PasteWarning {
    /// Unless the app turned on bracketed paste.
    Auto,
    Always,
    Never,
}

/// The mouse and clipboard settings of spec §6.5, with VS Code's defaults.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MouseSettings {
    pub word_separators: String,
    pub copy_on_selection: bool,
    pub right_click: RightClick,
    pub option_click_forces_selection: bool,
    pub wheel_sensitivity: f32,
    pub fast_scroll_sensitivity: f32,
    pub paste_warning: PasteWarning,
}

pub(crate) fn mouse_settings(cx: &App) -> MouseSettings {
    let settings = &cx.global::<AppState>().settings;
    let key = |name: &str| format!("terminal.integrated.{name}");
    let sensitivity = |name: &str, default: f32| {
        settings
            .get::<f32>(&key(name), None)
            .filter(|v| v.is_finite() && *v > 0.)
            .unwrap_or(default)
    };
    let right_click = match settings
        .get::<String>(&key("rightClickBehavior"), None)
        .as_deref()
    {
        Some("copyPaste") => RightClick::CopyPaste,
        Some("paste") => RightClick::Paste,
        Some("nothing") => RightClick::Nothing,
        Some("selectWord" | "default") => RightClick::SelectWord,
        _ => default_right_click(),
    };
    let paste_warning =
        match settings.get::<serde_json::Value>(&key("enableMultiLinePasteWarning"), None) {
            Some(serde_json::Value::String(s)) if s == "always" => PasteWarning::Always,
            Some(serde_json::Value::String(s)) if s == "never" => PasteWarning::Never,
            // The old boolean form, as VS Code migrates it.
            Some(serde_json::Value::Bool(false)) => PasteWarning::Never,
            _ => PasteWarning::Auto,
        };
    MouseSettings {
        word_separators: settings
            .get::<String>(&key("wordSeparators"), None)
            .unwrap_or_else(|| lucerna_terminal::DEFAULT_WORD_SEPARATORS.to_string()),
        copy_on_selection: settings
            .get::<bool>(&key("copyOnSelection"), None)
            .unwrap_or(false),
        right_click,
        option_click_forces_selection: settings
            .get::<bool>(&key("macOptionClickForcesSelection"), None)
            .unwrap_or(false),
        wheel_sensitivity: sensitivity("mouseWheelScrollSensitivity", 1.),
        fast_scroll_sensitivity: sensitivity("fastScrollSensitivity", 5.),
        paste_warning,
    }
}

/// VS Code's per-platform `rightClickBehavior` default: `selectWord` on
/// macOS, `copyPaste` on Windows, `default` (a word, here) elsewhere.
fn default_right_click() -> RightClick {
    if cfg!(target_os = "windows") {
        RightClick::CopyPaste
    } else {
        RightClick::SelectWord
    }
}

/// How a new terminal in `cwd` starts: the `$SHELL` (a login shell on
/// macOS), Lucerna's environment plus the terminal variables.
///
/// Profiles, `terminal.integrated.env.<os>` and `cwd` come with the
/// settings and Restricted Mode, so both env layers are empty for now.
pub(crate) fn spawn_options(cwd: &Path) -> TerminalOptions {
    let (program, args) = resolve_shell(None, None, std::env::var_os("SHELL").as_deref());
    let setting_env: HashMap<String, Option<String>> = HashMap::new();
    let profile_env: HashMap<String, Option<String>> = HashMap::new();
    let mut env: HashMap<String, Option<String>> = build_env(
        std::env::vars(),
        &setting_env,
        &profile_env,
        env!("CARGO_PKG_VERSION"),
        true,
    )
    .into_iter()
    .map(|(k, v)| (k, Some(v)))
    .collect();
    for (key, value) in setting_env.iter().chain(&profile_env) {
        if value.is_none() {
            env.insert(key.clone(), None);
        }
    }
    TerminalOptions {
        program,
        args,
        cwd: cwd.to_path_buf(),
        env,
        rows: INITIAL_ROWS,
        cols: INITIAL_COLS,
        cell_width: 8,
        cell_height: 16,
        scrollback: SCROLLBACK,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weights_parse_like_vs_code() {
        let w = |v: serde_json::Value| parse_weight(&v);
        assert_eq!(w("normal".into()), Some(FontWeight::NORMAL));
        assert_eq!(w("bold".into()), Some(FontWeight::BOLD));
        assert_eq!(w("300".into()), Some(FontWeight(300.)));
        assert_eq!(w(600.into()), Some(FontWeight(600.)));
        assert_eq!(w(0.into()), None);
        assert_eq!(w("heavy".into()), None);
    }

    #[test]
    fn theme_colours_become_bytes() {
        let c = lucerna_theme::Rgba {
            r: 1.,
            g: 0.5,
            b: 0.,
            a: 1.,
        };
        assert_eq!(rgba8(c), Rgba8::rgb(255, 128, 0));
    }
}
