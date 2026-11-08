//! The theme picker (`⌘K ⌘T`): Quick Open's list over the theme names.
//! Moving the selection previews a theme, Escape puts the previous one
//! back, and Enter writes `workbench.colorTheme` into the user
//! `settings.json`, keeping its comments.

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
use lucerna_theme::Theme;

use crate::config_files::{ConfigState, USER_SETTINGS_TEMPLATE, reload_config};
use crate::quick_open::picker::{
    fuzzy_filter, highlighted, picker_input_row, picker_list, picker_row, wrap_index,
};
use crate::text_input::TextInput;
use crate::ui::popover;
use crate::{AppState, CommandRegistry, Workbench};

/// The setting Enter writes.
const COLOR_THEME_KEY: &str = "workbench.colorTheme";

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
    fn new(names: Vec<String>, current: &str, cx: &mut Context<Self>) -> Self {
        let this = cx.weak_entity();
        let input = cx.new(|cx| {
            let mut input = TextInput::new(cx);
            input.set_placeholder("Select Color Theme");
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

/// The shown picker and what closing it restores.
pub(crate) struct ShownThemePicker {
    picker: Entity<ThemePicker>,
    _events: Subscription,
    /// The theme before the picker opened; Escape restores it.
    original: Arc<Theme>,
    focus_before: Option<FocusHandle>,
}

pub(crate) fn register_commands(commands: &mut CommandRegistry) {
    commands.register(
        "workbench.action.selectTheme",
        "Preferences: Color Theme",
        |wb, _, window, cx| wb.show_theme_picker(window, cx),
    );
}

impl Workbench {
    /// The shown theme picker, if any.
    pub fn theme_picker(&self) -> Option<&Entity<ThemePicker>> {
        self.theme_picker.as_ref().map(|shown| &shown.picker)
    }

    /// Shows the theme picker with the active theme selected, closing
    /// Quick Open.
    pub fn show_theme_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.theme_picker.is_some() {
            return;
        }
        self.close_quick_open(window, cx);
        let state = cx.global::<AppState>();
        let (names, original) = (state.themes.names(), state.theme.clone());
        let picker = cx.new(|cx| ThemePicker::new(names, &original.name, cx));
        let events = cx.subscribe_in(&picker, window, Self::on_theme_picker_event);
        let focus_before = window.focused(cx);
        window.focus(&picker.focus_handle(cx), cx);
        self.theme_picker = Some(ShownThemePicker {
            picker,
            _events: events,
            original,
            focus_before,
        });
        cx.notify();
    }

    /// Closes the theme picker; `revert` puts the theme from before it
    /// opened back.
    pub(crate) fn close_theme_picker(
        &mut self,
        revert: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(shown) = self.theme_picker.take() else {
            return;
        };
        if revert {
            cx.global_mut::<AppState>().set_theme(shown.original);
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
        match event {
            ThemePickerEvent::Preview(name) => {
                if let Some(theme) = cx.global::<AppState>().themes.get(name) {
                    cx.global_mut::<AppState>().set_theme(theme);
                }
            }
            ThemePickerEvent::Confirm(name) => {
                if let Some(theme) = cx.global::<AppState>().themes.get(name) {
                    cx.global_mut::<AppState>().set_theme(theme);
                }
                self.close_theme_picker(false, window, cx);
                save_color_theme(name, cx);
            }
            ThemePickerEvent::Dismiss => self.close_theme_picker(true, window, cx),
        }
    }
}

/// Writes `name` as `workbench.colorTheme` into the user `settings.json`
/// and applies the file. A file that does not parse is left alone.
fn save_color_theme(name: &str, cx: &mut App) {
    let Some(state) = cx.try_global::<ConfigState>() else {
        tracing::warn!("watch_config was not called: the theme is not saved");
        return;
    };
    let path = state.paths().user_settings();
    match write_color_theme(&path, name) {
        Ok(()) => reload_config(&[path], cx),
        Err(err) => tracing::error!(path = %path.display(), %err, "could not save the theme"),
    }
}

fn write_color_theme(path: &Path, name: &str) -> io::Result<()> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => USER_SETTINGS_TEMPLATE.to_string(),
        Err(err) => return Err(err),
    };
    let updated = set_string_setting(&text, COLOR_THEME_KEY, name)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    write_atomically(path, &updated)
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
/// `value`: an existing value is replaced in place, else the member is
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
    if let Some(prop) = object.properties.iter().find(|p| p.name.as_str() == key) {
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
        set_string_setting(text, COLOR_THEME_KEY, "One Light").unwrap()
    }

    #[test]
    fn appends_before_the_closing_brace() {
        assert_eq!(
            set(USER_SETTINGS_TEMPLATE),
            "// Lucerna user settings\n{\n  \"workbench.colorTheme\": \"One Light\"\n}\n"
        );
        assert_eq!(
            set("{\n  \"a\": 1 // one\n}\n"),
            "{\n  \"a\": 1, // one\n  \"workbench.colorTheme\": \"One Light\"\n}\n"
        );
        assert_eq!(
            set("{ \"a\": [1, 2], }"),
            "{ \"a\": [1, 2],\n  \"workbench.colorTheme\": \"One Light\"\n}"
        );
        assert_eq!(
            set("// only a comment\n"),
            "// only a comment\n{\n  \"workbench.colorTheme\": \"One Light\"\n}\n"
        );
    }

    #[test]
    fn replaces_an_existing_value_in_place() {
        assert_eq!(
            set("{\n  // theme\n  \"workbench.colorTheme\": \"One Dark\", /* x */\n  \"b\": 2\n}"),
            "{\n  // theme\n  \"workbench.colorTheme\": \"One Light\", /* x */\n  \"b\": 2\n}"
        );
        // Only top-level keys count.
        assert_eq!(
            set("{ \"x\": { \"workbench.colorTheme\": 1 } }"),
            "{ \"x\": { \"workbench.colorTheme\": 1 },\n  \"workbench.colorTheme\": \"One Light\"\n}"
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
        write_color_theme(&path, "One Light").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("\"workbench.colorTheme\": \"One Light\""),
            "{text}"
        );
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("settings.json")]);
    }
}
