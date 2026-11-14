//! The theme picker (`⌘K ⌘T`): Quick Open's list over the theme names.
//! Moving the selection previews a theme, Escape puts the previous one
//! back, and Enter writes `workbench.colorTheme` into the user
//! `settings.json`, keeping its comments. The same widget picks the icon
//! theme (`workbench.action.selectIconTheme`, `workbench.iconTheme`).

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    KeyDownEvent, Render, ScrollHandle, Subscription, Window, prelude::*,
};
use jsonc_parser::common::Ranged;
use jsonc_parser::tokens::Token;
use jsonc_parser::{CollectOptions, CommentCollectionStrategy, ParseOptions, ast, parse_to_ast};
use lucerna_settings::{Layer, SettingsError, SettingsStore};
use lucerna_theme::{ResolvedIcons, Theme};

use crate::config_files::{
    ConfigFile, ConfigState, USER_SETTINGS_TEMPLATE, apply_configured_theme,
    color_theme_setting_key, record_error, reload_config, set_icon_preview, set_theme_preview,
    system_appearance,
};
use crate::extensions::service::resolve_icons;
use crate::quick_open::picker::{
    fuzzy_filter, highlighted, picker_input_row, picker_list, picker_row, wrap_index,
};
use crate::text_input::TextInput;
use crate::ui::popover;
use crate::{AppState, CommandRegistry, Workbench};

/// The settings Enter writes.
const COLOR_THEME_KEY: &str = "workbench.colorTheme";
const ICON_THEME_KEY: &str = "workbench.iconTheme";

/// What the workbench does as the picker is used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThemePickerEvent {
    /// The selection moved to this theme.
    Preview(String),
    /// Enter (or a click) chose this theme.
    Confirm(String),
    Dismiss,
}

pub struct ThemePicker {
    input: Entity<TextInput>,
    names: Vec<String>,
    /// Matching names: index into `names` and matched char positions.
    rows: Vec<(usize, Vec<u32>)>,
    selected: usize,
    scroll: ScrollHandle,
}

impl EventEmitter<ThemePickerEvent> for ThemePicker {}

impl ThemePicker {
    /// Lists `names` with `current` selected.
    fn new(
        names: Vec<String>,
        current: &str,
        placeholder: &'static str,
        cx: &mut Context<Self>,
    ) -> Self {
        let this = cx.weak_entity();
        let input = cx.new(|cx| {
            let mut input = TextInput::new(cx);
            input.set_placeholder(placeholder);
            input.on_change(move |text, _, cx| {
                this.update(cx, |picker, cx| picker.query_changed(text, cx))
                    .ok();
            });
            input
        });
        let rows = fuzzy_filter(names.iter().map(String::as_str), "");
        let selected = names.iter().position(|n| n == current).unwrap_or(0);
        let scroll = ScrollHandle::new();
        scroll.scroll_to_item(selected);
        ThemePicker {
            input,
            names,
            rows,
            selected,
            scroll,
        }
    }

    pub fn input(&self) -> &Entity<TextInput> {
        &self.input
    }

    /// The theme names listed, in order.
    pub fn row_texts(&self) -> Vec<String> {
        self.rows
            .iter()
            .map(|(i, _)| self.names[*i].clone())
            .collect()
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    fn selected_name(&self) -> Option<&str> {
        let (index, _) = self.rows.get(self.selected)?;
        Some(&self.names[*index])
    }

    fn query_changed(&mut self, text: &str, cx: &mut Context<Self>) {
        self.rows = fuzzy_filter(self.names.iter().map(String::as_str), text.trim());
        self.select(0, cx);
    }

    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        self.selected = index;
        self.scroll.scroll_to_item(index);
        if let Some(name) = self.selected_name() {
            cx.emit(ThemePickerEvent::Preview(name.to_string()));
        }
        cx.notify();
    }

    fn confirm(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(&(name, _)) = self.rows.get(index) {
            cx.emit(ThemePickerEvent::Confirm(self.names[name].clone()));
        }
    }

    fn key_down(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let m = &ev.keystroke.modifiers;
        if m.control || m.alt || m.shift || m.platform {
            return;
        }
        match ev.keystroke.key.as_str() {
            "up" => self.select(wrap_index(self.selected, self.rows.len(), -1), cx),
            "down" => self.select(wrap_index(self.selected, self.rows.len(), 1), cx),
            "enter" => self.confirm(self.selected, cx),
            // Unless a key binding closed the picker first.
            "escape" => cx.emit(ThemePickerEvent::Dismiss),
            _ => return,
        }
        cx.stop_propagation();
    }

    fn render_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let (name, positions) = &self.rows[index];
        picker_row("theme-picker-row", index, index == self.selected, cx)
            .on_click(cx.listener(move |picker, _, _, cx| picker.confirm(index, cx)))
            .child(highlighted(self.names[*name].clone().into(), positions, cx))
            .into_any_element()
    }
}

impl Focusable for ThemePicker {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl Render for ThemePicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows: Vec<AnyElement> = (0..self.rows.len())
            .map(|i| self.render_row(i, cx))
            .collect();
        popover(cx)
            .id("theme-picker")
            .occlude()
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(ThemePickerEvent::Dismiss)))
            .child(picker_input_row(&self.input, cx))
            .when(!rows.is_empty(), |p| {
                p.child(picker_list("theme-picker-list", &self.scroll).children(rows))
            })
    }
}

/// What a shown picker chooses, with what was active before it opened.
/// Escape restores that when no configuration files are loaded (no
/// [`ConfigState`]).
#[derive(Clone)]
enum Picking {
    ColorTheme(Arc<Theme>),
    IconTheme(ResolvedIcons),
}

/// The shown picker and what closing it restores.
pub(crate) struct ShownThemePicker {
    picker: Entity<ThemePicker>,
    _events: Subscription,
    picking: Picking,
    focus_before: Option<FocusHandle>,
}

pub(crate) fn register_commands(commands: &mut CommandRegistry) {
    commands.register(
        "workbench.action.selectTheme",
        "Preferences: Color Theme",
        |wb, _, window, cx| wb.show_theme_picker(window, cx),
    );
    commands.register(
        "workbench.action.selectIconTheme",
        "Preferences: File Icon Theme",
        |wb, _, window, cx| wb.show_icon_theme_picker(window, cx),
    );
}

impl Workbench {
    /// The shown theme picker, if any.
    pub fn theme_picker(&self) -> Option<&Entity<ThemePicker>> {
        self.theme_picker.as_ref().map(|shown| &shown.picker)
    }

    /// Shows the theme picker with the active theme selected, closing any
    /// other modal overlay.
    pub fn show_theme_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.theme_picker.is_some() {
            return;
        }
        let state = cx.global::<AppState>();
        let (names, original) = (state.themes.names(), state.theme.clone());
        let current = original.name.clone();
        self.show_picker(
            names,
            &current,
            "Select Color Theme",
            Picking::ColorTheme(original),
            window,
            cx,
        );
        set_theme_preview(true, cx);
    }

    /// Shows the picker over the icon themes with the active one selected.
    pub fn show_icon_theme_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.theme_picker.is_some() {
            return;
        }
        let state = cx.global::<AppState>();
        let (names, original) = (state.icon_themes.names(), state.icons.clone());
        let current = original.name().to_string();
        self.show_picker(
            names,
            &current,
            "Select File Icon Theme",
            Picking::IconTheme(original),
            window,
            cx,
        );
        set_icon_preview(true, cx);
    }

    fn show_picker(
        &mut self,
        names: Vec<String>,
        current: &str,
        placeholder: &'static str,
        picking: Picking,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_modal_overlays(window, cx);
        let picker = cx.new(|cx| ThemePicker::new(names, current, placeholder, cx));
        let events = cx.subscribe_in(&picker, window, Self::on_theme_picker_event);
        let focus_before = window.focused(cx);
        window.focus(&picker.focus_handle(cx), cx);
        self.theme_picker = Some(ShownThemePicker {
            picker,
            _events: events,
            picking,
            focus_before,
        });
        cx.notify();
    }

    /// Closes the theme picker. `revert` makes the configured theme
    /// (`workbench.colorTheme` or `workbench.iconTheme` as last loaded, so a
    /// reload during the preview counts) active again; without
    /// configuration files, the one from before the picker opened.
    pub(crate) fn close_theme_picker(
        &mut self,
        revert: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(shown) = self.theme_picker.take() else {
            return;
        };
        set_theme_preview(false, cx);
        set_icon_preview(false, cx);
        if revert {
            revert_preview(shown.picking, cx);
        }
        let focused = window.focused(cx);
        if focused.is_none() || shown.picker.focus_handle(cx).contains_focused(window, cx) {
            match shown.focus_before {
                Some(focus) => window.focus(&focus, cx),
                None => self.focus_editor_area(window, cx),
            }
        }
        cx.notify();
    }

    fn on_theme_picker_event(
        &mut self,
        _: &Entity<ThemePicker>,
        event: &ThemePickerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let is_icons = matches!(
            self.theme_picker.as_ref().map(|shown| &shown.picking),
            Some(Picking::IconTheme(_))
        );
        match event {
            ThemePickerEvent::Preview(name) => preview(name, is_icons, cx),
            ThemePickerEvent::Confirm(name) => {
                let key = if is_icons {
                    ICON_THEME_KEY
                } else {
                    COLOR_THEME_KEY
                };
                match save_setting(key, name, cx) {
                    Ok(()) => {
                        preview(name, is_icons, cx);
                        self.close_theme_picker(false, window, cx);
                        if let Some(path) = cx
                            .try_global::<ConfigState>()
                            .map(|s| s.paths().user_settings())
                        {
                            reload_config(&[path], cx);
                        }
                    }
                    // The picker stays open; the status bar shows why.
                    Err(()) => {
                        if let Some(shown) = &self.theme_picker {
                            revert_preview(shown.picking.clone(), cx);
                        }
                    }
                }
            }
            ThemePickerEvent::Dismiss => self.close_theme_picker(true, window, cx),
        }
    }
}

/// Writes `name` as the color theme (or, with `icons`, the file icon
/// theme) into the user settings and makes it active. On failure the
/// status bar shows why.
pub(crate) fn set_theme_setting(name: &str, icons: bool, cx: &mut App) -> Result<(), ()> {
    let key = if icons {
        ICON_THEME_KEY
    } else {
        color_theme_key(cx)
    };
    save_setting(key, name, cx)?;
    preview(name, icons, cx);
    if let Some(path) = cx
        .try_global::<ConfigState>()
        .map(|s| s.paths().user_settings())
    {
        reload_config(&[path], cx);
    }
    Ok(())
}

/// The setting a picked color theme is written to: the preferred theme
/// for the system mode while the theme follows it, as in VS Code.
fn color_theme_key(cx: &App) -> &'static str {
    color_theme_setting_key(&cx.global::<AppState>().settings, system_appearance(cx))
}

/// Makes the named theme (colors or icons) active without saving it.
fn preview(name: &str, icons: bool, cx: &mut App) {
    if icons {
        let state = cx.global_mut::<AppState>();
        state.icons = state.icon_themes.resolve(name);
        cx.refresh_windows();
    } else if let Some(theme) = cx.global::<AppState>().themes.get(name) {
        cx.global_mut::<AppState>().set_theme(theme);
    }
}

/// Makes the configured theme active again, or the one from before the
/// picker opened when no configuration files are loaded.
fn revert_preview(picking: Picking, cx: &mut App) {
    let has_config = cx.has_global::<ConfigState>();
    match picking {
        Picking::ColorTheme(original) => {
            if has_config {
                apply_configured_theme(cx);
            } else {
                cx.global_mut::<AppState>().set_theme(original);
            }
        }
        Picking::IconTheme(original) => {
            if has_config {
                set_icon_preview(false, cx);
                resolve_icons(cx);
                set_icon_preview(true, cx);
            } else {
                cx.global_mut::<AppState>().icons = original;
            }
            cx.refresh_windows();
        }
    }
}

/// Writes `name` as the setting `key` into the user `settings.json`.
/// A file that cannot be read or does not parse is left alone and the
/// failure is shown as its configuration error. Without configuration
/// files (no [`ConfigState`]) there is nothing to write.
fn save_setting(key: &str, name: &str, cx: &mut App) -> Result<(), ()> {
    let Some(state) = cx.try_global::<ConfigState>() else {
        tracing::warn!(key, "watch_config was not called: the theme is not saved");
        return Ok(());
    };
    let path = state.paths().user_settings();
    write_string_setting(&path, key, name).map_err(|error| {
        tracing::error!(path = %path.display(), %error, "could not save the theme");
        record_error(ConfigFile::UserSettings, path, error, cx);
    })
}

fn write_string_setting(path: &Path, key: &str, name: &str) -> Result<(), SettingsError> {
    let io_error = |e: io::Error| SettingsError {
        message: e.to_string(),
        line: None,
    };
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => USER_SETTINGS_TEMPLATE.to_string(),
        Err(err) => return Err(io_error(err)),
    };
    let updated = set_string_setting(&text, key, name)?;
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

/// `text` (a JSONC object) with the top-level `key` set to the string
/// `value`: an existing value (the last one, when the key is duplicated)
/// is replaced in place, else the member is
/// appended before the closing `}`. Comments and formatting elsewhere are
/// kept.
pub(crate) fn set_string_setting(
    text: &str,
    key: &str,
    value: &str,
) -> Result<String, SettingsError> {
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
    let quoted = serde_json::to_string(value).expect("a string serializes");
    let Some(ast::Value::Object(object)) = &parsed.value else {
        // Empty or comments only.
        let sep = if text.is_empty() || text.ends_with('\n') {
            ""
        } else {
            "\n"
        };
        return Ok(format!("{text}{sep}{{\n  \"{key}\": {quoted}\n}}\n"));
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
    let member = serde_json::to_string(key).expect("a string serializes");
    Ok(format!(
        "{}{comma}{between}\n  {member}: {quoted}\n{}",
        &text[..after],
        &text[close..]
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(text: &str) -> String {
        set_string_setting(text, COLOR_THEME_KEY, "Lucerna Light").unwrap()
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
        write_string_setting(&path, COLOR_THEME_KEY, "Lucerna Light").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("\"workbench.colorTheme\": \"Lucerna Light\""),
            "{text}"
        );
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("settings.json")]);
    }
}
