//! Serde mirror of the Zed theme family JSON, and conversion to [`Theme`].

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::Value;

use crate::{Appearance, HighlightStyle, Rgba, Theme, UiColors};

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct ThemeFamily {
    pub name: String,
    pub themes: Vec<ThemeDef>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct ThemeDef {
    pub name: String,
    pub appearance: String,
    pub style: HashMap<String, Value>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SyntaxDef {
    color: Option<String>,
    font_style: Option<String>,
    font_weight: Option<f64>,
}

impl ThemeDef {
    pub(crate) fn into_theme(self) -> Result<Theme, String> {
        let appearance = match self.appearance.as_str() {
            "dark" | "" => Appearance::Dark,
            "light" => Appearance::Light,
            other => {
                return Err(format!(
                    "theme {:?}: unknown appearance {other:?}",
                    self.name
                ));
            }
        };
        let color = |key: &str| -> Option<Rgba> { Rgba::parse_hex(self.style.get(key)?.as_str()?) };
        let fb = UiColors::fallback(appearance);
        let player = |key: &str| {
            self.style
                .get("players")
                .and_then(|p| p.get(0))
                .and_then(|p| p.get(key))
                .and_then(Value::as_str)
                .and_then(Rgba::parse_hex)
        };
        let selection = player("selection");
        let editor_foreground = color("editor.foreground").unwrap_or(fb.editor_foreground);
        let ui = UiColors {
            background: color("background").unwrap_or(fb.background),
            border: color("border").unwrap_or(fb.border),
            text: color("text").unwrap_or(fb.text),
            text_muted: color("text.muted").unwrap_or(fb.text_muted),
            text_accent: color("text.accent").unwrap_or(fb.text_accent),
            element_selected: color("element.selected").unwrap_or(fb.element_selected),
            element_background: color("element.background").unwrap_or(fb.element_background),
            editor_subheader_background: color("editor.subheader.background")
                .unwrap_or(fb.editor_subheader_background),
            panel_background: color("panel.background").unwrap_or(fb.panel_background),
            status_bar_background: color("status_bar.background")
                .unwrap_or(fb.status_bar_background),
            tab_active_background: color("tab.active_background")
                .unwrap_or(fb.tab_active_background),
            tab_inactive_background: color("tab.inactive_background")
                .unwrap_or(fb.tab_inactive_background),
            editor_background: color("editor.background").unwrap_or(fb.editor_background),
            editor_foreground,
            editor_line_number: color("editor.line_number").unwrap_or(fb.editor_line_number),
            editor_active_line_number: color("editor.active_line_number")
                .unwrap_or(fb.editor_active_line_number),
            editor_active_line_background: color("editor.active_line.background")
                .unwrap_or(fb.editor_active_line_background),
            search_match_background: color("search.match_background")
                .unwrap_or(fb.search_match_background),
            selection: selection.unwrap_or(fb.selection),
            cursor: player("cursor").unwrap_or(editor_foreground),
            title_bar_background: color("title_bar.background").unwrap_or(fb.title_bar_background),
            elevated_surface_background: color("elevated_surface.background")
                .unwrap_or(fb.elevated_surface_background),
            ghost_element_hover: color("ghost_element.hover").unwrap_or(fb.ghost_element_hover),
            border_variant: color("border.variant").unwrap_or(fb.border_variant),
            icon: color("icon").unwrap_or(fb.icon),
            icon_muted: color("icon.muted").unwrap_or(fb.icon_muted),
            tab_bar_background: color("tab_bar.background")
                .or_else(|| color("panel.background"))
                .unwrap_or(fb.tab_bar_background),
            error: color("error").unwrap_or(fb.error),
            warning: color("warning").unwrap_or(fb.warning),
            info: color("info").unwrap_or(fb.info),
            hint: color("hint").unwrap_or(fb.hint),
            error_background: color("error.background").unwrap_or(fb.error_background),
            warning_background: color("warning.background").unwrap_or(fb.warning_background),
            info_background: color("info.background").unwrap_or(fb.info_background),
            hint_background: color("hint.background").unwrap_or(fb.hint_background),
            error_border: color("error.border").unwrap_or(fb.error_border),
            warning_border: color("warning.border").unwrap_or(fb.warning_border),
            info_border: color("info.border").unwrap_or(fb.info_border),
            hint_border: color("hint.border").unwrap_or(fb.hint_border),
            link_text_hover: color("link_text.hover").unwrap_or(fb.link_text_hover),
            editor_highlighted_line_background: color("editor.highlighted_line.background")
                .unwrap_or(fb.editor_highlighted_line_background),
            element_hover: color("element.hover").unwrap_or(fb.element_hover),
            scrollbar_thumb: color("scrollbar.thumb.background").unwrap_or(fb.scrollbar_thumb),
            vc_added: color("version_control.added").unwrap_or(fb.vc_added),
            vc_modified: color("version_control.modified").unwrap_or(fb.vc_modified),
            vc_deleted: color("version_control.deleted").unwrap_or(fb.vc_deleted),
            status_created: color("created").unwrap_or(fb.status_created),
            status_modified: color("modified").unwrap_or(fb.status_modified),
            status_deleted: color("deleted").unwrap_or(fb.status_deleted),
            status_conflict: color("conflict").unwrap_or(fb.status_conflict),
            status_ignored: color("ignored").unwrap_or(fb.status_ignored),
            diff_added_background: color("editor.diff_hunk.added.background").unwrap_or(fb.diff_added_background),
            diff_deleted_background: color("editor.diff_hunk.deleted.background").unwrap_or(fb.diff_deleted_background),
        };

        let mut syntax = HashMap::new();
        if let Some(Value::Object(map)) = self.style.get("syntax") {
            for (capture, value) in map {
                let Ok(def) = serde_json::from_value::<SyntaxDef>(value.clone()) else {
                    continue;
                };
                syntax.insert(
                    capture.clone(),
                    HighlightStyle {
                        color: def.color.as_deref().and_then(Rgba::parse_hex),
                        italic: def.font_style.as_deref() == Some("italic"),
                        bold: def.font_weight.is_some_and(|w| w >= 600.0),
                    },
                );
            }
        }
        Ok(Theme {
            name: self.name,
            appearance,
            ui,
            syntax,
        })
    }
}
