//! What a terminal reads from settings and the theme: its font (spec §7.3),
//! its palette (§7.1) and how its shell starts (§6.2).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use gpui::{App, FontWeight, Pixels, SharedString, px};
use lucerna_settings::SettingsView;
use lucerna_terminal::{
    CursorShape, MAX_SCROLLBACK, Rgba8, TerminalOptions, TerminalPalette, build_env, resolve_shell,
};

use crate::AppState;
use crate::fonts::editor_font_family;

/// `terminal.integrated.lineHeight` when unset.
const DEFAULT_LINE_HEIGHT: f32 = 1.2;
/// `editor.fontSize` when even that is unset.
const FALLBACK_FONT_SIZE: f32 = 14.;
/// `terminal.integrated.scrollback` when unset (spec §9.2).
const DEFAULT_SCROLLBACK: usize = 10_000;
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
/// `editor.fontFamily`, an unset `fontSize` is `editor.fontSize` with the
/// font zoom ([`crate::font_zoom`]), `lineHeight` defaults to 1.2,
/// `letterSpacing` to 0, the weights to normal and bold.
pub fn terminal_font(settings: SettingsView<'_>, cx: &App) -> TerminalFont {
    let family = settings
        .get::<String>("terminal.integrated.fontFamily", None)
        .filter(|f| !f.trim().is_empty())
        .map(SharedString::from)
        .unwrap_or_else(|| editor_font_family(settings, None));
    let positive = |key: &str| settings.get::<f32>(key, None).filter(|v| *v > 0.);
    let size = positive("terminal.integrated.fontSize").unwrap_or_else(|| {
        let base = positive("editor.fontSize").unwrap_or(FALLBACK_FONT_SIZE);
        crate::font_zoom::zoomed(base, cx)
    });
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
pub(crate) fn minimum_contrast(settings: SettingsView<'_>) -> f32 {
    settings
        .get::<f32>("terminal.integrated.minimumContrastRatio", None)
        .filter(|v| v.is_finite())
        .unwrap_or(4.5)
        .clamp(1., 21.)
}

/// `terminal.integrated.drawBoldTextInBrightColors`.
pub(crate) fn bold_is_bright(settings: SettingsView<'_>) -> bool {
    settings
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

pub(crate) fn mouse_settings(settings: SettingsView<'_>) -> MouseSettings {
    let key = |name: &str| format!("terminal.integrated.{name}");
    let sensitivity = |name: &str, default: f32| {
        settings
            .get::<f32>(&key(name), None)
            .filter(|v| v.is_finite() && *v > 0.)
            .unwrap_or(default)
    };
    let right_click = right_click_from(
        settings
            .get::<String>(&key("rightClickBehavior"), None)
            .as_deref(),
    );
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

/// A `rightClickBehavior` value; unset or unknown is the platform's
/// default.
fn right_click_from(value: Option<&str>) -> RightClick {
    match value {
        Some("copyPaste") => RightClick::CopyPaste,
        Some("paste") => RightClick::Paste,
        Some("nothing") => RightClick::Nothing,
        Some("selectWord" | "default") => RightClick::SelectWord,
        _ => default_right_click(),
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

/// How a new terminal starts, and the warning to write into it first.
pub(crate) struct SpawnPlan {
    pub options: TerminalOptions,
    /// The `cwd` setting named a folder that does not exist.
    pub warning: Option<String>,
}

/// This platform's suffix of the per-OS settings (`env.osx`, …).
fn os_key() -> &'static str {
    if cfg!(target_os = "macos") {
        "osx"
    } else if cfg!(windows) {
        "windows"
    } else {
        "linux"
    }
}

/// The settings that pick a program and its environment (spec §6.3): in
/// Restricted Mode (`restricted`) the Workspace layer is skipped, since a
/// strange repository could point the shell at its own binary.
fn shell_setting(
    key: &str,
    restricted: bool,
    settings: SettingsView<'_>,
) -> Option<serde_json::Value> {
    let key = format!("terminal.integrated.{key}");
    if restricted {
        settings.get_trusted(&key, None)
    } else {
        settings.get(&key, None)
    }
}

/// An `env` object: strings set a variable, `null` removes it; other
/// values are skipped.
fn env_layer(value: Option<&serde_json::Value>) -> HashMap<String, Option<String>> {
    let Some(serde_json::Value::Object(map)) = value else {
        return HashMap::new();
    };
    map.iter()
        .filter_map(|(key, value)| match value {
            serde_json::Value::String(s) => Some((key.clone(), Some(s.clone()))),
            serde_json::Value::Null => Some((key.clone(), None)),
            _ => None,
        })
        .collect()
}

/// The profile `defaultProfile.<os>` names in `profiles.<os>`: its program
/// (the first existing entry when `path` is a list), arguments and env.
struct Profile {
    path: Option<PathBuf>,
    args: Option<Vec<String>>,
    env: HashMap<String, Option<String>>,
}

fn default_profile(restricted: bool, settings: SettingsView<'_>) -> Option<Profile> {
    let os = os_key();
    let name = shell_setting(&format!("defaultProfile.{os}"), restricted, settings)?;
    let name = name.as_str()?;
    let profiles = shell_setting(&format!("profiles.{os}"), restricted, settings)?;
    let profile = profiles.get(name)?.as_object()?;
    let path = match profile.get("path") {
        Some(serde_json::Value::String(p)) => Some(PathBuf::from(p)),
        Some(serde_json::Value::Array(paths)) => {
            let paths: Vec<PathBuf> = paths
                .iter()
                .filter_map(|p| p.as_str().map(PathBuf::from))
                .collect();
            paths
                .iter()
                .find(|p| p.is_file())
                .or(paths.first())
                .cloned()
        }
        _ => None,
    };
    let args = profile.get("args").and_then(|args| {
        args.as_array().map(|args| {
            args.iter()
                .filter_map(|a| a.as_str().map(str::to_string))
                .collect()
        })
    });
    Some(Profile {
        path,
        args,
        env: env_layer(profile.get("env")),
    })
}

/// `terminal.integrated.cwd` against `root`: empty is `root`, relative is
/// under it, `~` is home. A folder that does not exist gives home and a
/// warning.
fn start_folder(
    root: &Path,
    restricted: bool,
    settings: SettingsView<'_>,
) -> (PathBuf, Option<String>) {
    let setting = shell_setting("cwd", restricted, settings)
        .and_then(|v| v.as_str().map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty());
    let Some(setting) = setting else {
        return (root.to_path_buf(), None);
    };
    let home = home_dir();
    let path = match (setting.strip_prefix('~'), &home) {
        (Some(rest), Some(home)) if rest.is_empty() || rest.starts_with('/') => {
            home.join(rest.trim_start_matches('/'))
        }
        _ => root.join(&setting),
    };
    // One `stat`, only for a configured folder: the root is known to exist.
    if path.is_dir() {
        return (path, None);
    }
    let fallback = home.unwrap_or_else(|| root.to_path_buf());
    let warning = format!(
        "The terminal.integrated.cwd folder {} does not exist; starting in {}.",
        path.display(),
        fallback.display()
    );
    (fallback, Some(warning))
}

/// The user's home folder.
pub(crate) fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

/// `terminal.integrated.scrollback`, at most [`MAX_SCROLLBACK`].
pub(crate) fn scrollback(settings: SettingsView<'_>) -> usize {
    settings
        .get::<f64>("terminal.integrated.scrollback", None)
        .filter(|v| v.is_finite() && *v >= 0.)
        .map_or(DEFAULT_SCROLLBACK, |v| {
            v.min(MAX_SCROLLBACK as f64) as usize
        })
}

/// `terminal.integrated.cursorStyle`: the shape until an app sets one.
pub(crate) fn cursor_style(settings: SettingsView<'_>) -> CursorShape {
    match settings
        .get::<String>("terminal.integrated.cursorStyle", None)
        .as_deref()
    {
        Some("line") => CursorShape::Bar,
        Some("underline") => CursorShape::Underline,
        _ => CursorShape::Block,
    }
}

/// The cursor drawn while the terminal is not focused
/// (`terminal.integrated.cursorStyleInactive`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InactiveCursor {
    Outline,
    Block,
    Bar,
    Underline,
    Hidden,
}

pub(crate) fn inactive_cursor(settings: SettingsView<'_>) -> InactiveCursor {
    match settings
        .get::<String>("terminal.integrated.cursorStyleInactive", None)
        .as_deref()
    {
        Some("block") => InactiveCursor::Block,
        Some("line") => InactiveCursor::Bar,
        Some("underline") => InactiveCursor::Underline,
        Some("none") => InactiveCursor::Hidden,
        _ => InactiveCursor::Outline,
    }
}

/// `terminal.integrated.enableBell`.
pub(crate) fn bell_enabled(settings: SettingsView<'_>) -> bool {
    settings
        .get::<bool>("terminal.integrated.enableBell", None)
        .unwrap_or(false)
}

/// When quitting or closing the window asks first
/// (`terminal.integrated.confirmOnExit`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConfirmOnExit {
    Never,
    Always,
    HasChildProcesses,
}

pub(crate) fn confirm_on_exit(settings: SettingsView<'_>) -> ConfirmOnExit {
    match settings
        .get::<String>("terminal.integrated.confirmOnExit", None)
        .as_deref()
    {
        Some("always") => ConfirmOnExit::Always,
        Some("hasChildProcesses") => ConfirmOnExit::HasChildProcesses,
        _ => ConfirmOnExit::Never,
    }
}

/// How a new terminal for the workspace at `root` starts (spec §6.2): the
/// default profile or `$SHELL` (a login shell on macOS), in the `cwd`
/// setting's folder, with Lucerna's environment, then `env.<os>`, then the
/// profile's `env`, then the terminal variables. `restricted`: Restricted
/// Mode, where the Workspace layer of these settings is ignored.
pub(crate) fn spawn_plan(root: &Path, restricted: bool, settings: SettingsView<'_>) -> SpawnPlan {
    let profile = default_profile(restricted, settings);
    let (program, args) = match &profile {
        Some(p) => resolve_shell(
            p.path.as_deref(),
            p.args.clone(),
            std::env::var_os("SHELL").as_deref(),
        ),
        None => resolve_shell(None, None, std::env::var_os("SHELL").as_deref()),
    };
    let setting_env =
        env_layer(shell_setting(&format!("env.{}", os_key()), restricted, settings).as_ref());
    let profile_env = profile.map(|p| p.env).unwrap_or_default();
    let detect_locale = settings
        .get::<String>("terminal.integrated.detectLocale", None)
        .is_none_or(|v| v != "off");
    let mut env: HashMap<String, Option<String>> = build_env(
        std::env::vars(),
        &setting_env,
        &profile_env,
        env!("CARGO_PKG_VERSION"),
        detect_locale,
    )
    .into_iter()
    .map(|(k, v)| (k, Some(v)))
    .collect();
    // `build_env` drops removed variables; the spawner also needs them
    // named, to take them out of the inherited environment.
    for (key, value) in setting_env.iter().chain(&profile_env) {
        if value.is_none() && !env.contains_key(key) {
            env.insert(key.clone(), None);
        }
    }
    let (cwd, warning) = start_folder(root, restricted, settings);
    SpawnPlan {
        options: TerminalOptions {
            program,
            args,
            cwd,
            env,
            rows: INITIAL_ROWS,
            cols: INITIAL_COLS,
            cell_width: 8,
            cell_height: 16,
            scrollback: scrollback(settings),
        },
        warning,
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
    fn right_click_default_is_per_platform() {
        // The bundled default leaves it unset, so each platform gets its
        // own (VS Code: selectWord on macOS, copyPaste on Windows).
        let store = lucerna_settings::SettingsStore::with_defaults();
        let value = store.get::<String>("terminal.integrated.rightClickBehavior", None);
        assert_eq!(value, None);
        assert_eq!(right_click_from(value.as_deref()), default_right_click());
        assert_eq!(right_click_from(Some("bogus")), default_right_click());
        assert_eq!(right_click_from(Some("paste")), RightClick::Paste);
        assert_eq!(right_click_from(Some("default")), RightClick::SelectWord);
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
