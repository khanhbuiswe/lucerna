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
            "dark" => Appearance::Dark,
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
        let selection = self
            .style
            .get("players")
            .and_then(|p| p.get(0))
            .and_then(|p| p.get("selection"))
            .and_then(Value::as_str)
            .and_then(Rgba::parse_hex);
        let ui = UiColors {
            background: color("background").unwrap_or(fb.background),
            border: color("border").unwrap_or(fb.border),
            text: color("text").unwrap_or(fb.text),
            text_muted: color("text.muted").unwrap_or(fb.text_muted),
            element_selected: color("element.selected").unwrap_or(fb.element_selected),
            panel_background: color("panel.background").unwrap_or(fb.panel_background),
            status_bar_background: color("status_bar.background")
                .unwrap_or(fb.status_bar_background),
            tab_active_background: color("tab.active_background")
                .unwrap_or(fb.tab_active_background),
            tab_inactive_background: color("tab.inactive_background")
                .unwrap_or(fb.tab_inactive_background),
            editor_background: color("editor.background").unwrap_or(fb.editor_background),
            editor_foreground: color("editor.foreground").unwrap_or(fb.editor_foreground),
            editor_line_number: color("editor.line_number").unwrap_or(fb.editor_line_number),
            editor_active_line_number: color("editor.active_line_number")
                .unwrap_or(fb.editor_active_line_number),
            editor_active_line_background: color("editor.active_line.background")
                .unwrap_or(fb.editor_active_line_background),
            search_match_background: color("search.match_background")
                .unwrap_or(fb.search_match_background),
            selection: selection.unwrap_or(fb.selection),
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
