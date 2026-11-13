//! Zed theme loading. Pure data: no UI dependencies.

mod icon_theme;
mod registry;
mod schema;

pub use icon_theme::{IconSource, IconTheme, IconThemeRegistry, ResolvedIcons, lucide_icon_theme};
pub use registry::{ThemeFile, ThemeRegistry, ThemeSource};

use std::collections::HashMap;

/// A color with components in `0.0..=1.0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    /// Parses `#rrggbb` or `#rrggbbaa`.
    pub fn parse_hex(s: &str) -> Option<Rgba> {
        let hex = s.strip_prefix('#')?;
        if !hex.is_ascii() || !(hex.len() == 6 || hex.len() == 8) {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        let a = if hex.len() == 8 { byte(6)? } else { 255 };
        Some(Rgba {
            r: f32::from(byte(0)?) / 255.0,
            g: f32::from(byte(2)?) / 255.0,
            b: f32::from(byte(4)?) / 255.0,
            a: f32::from(a) / 255.0,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HighlightStyle {
    pub color: Option<Rgba>,
    pub italic: bool,
    pub bold: bool,
}

/// UI colors taken from a Zed theme's `style`. Keys the theme lacks use the
/// neutral per-appearance colors from [`UiColors::fallback`].
#[derive(Debug, Clone, PartialEq)]
pub struct UiColors {
    pub background: Rgba,
    pub border: Rgba,
    pub text: Rgba,
    pub text_muted: Rgba,
    /// `text.disabled`: text of an entry that cannot be used, such as a
    /// greyed-out menu item.
    pub text_disabled: Rgba,
    /// `border.focused`: the border of a focused control.
    pub border_focused: Rgba,
    /// `text.accent`: text that stands out, such as a tab's unsaved dot.
    pub text_accent: Rgba,
    pub element_selected: Rgba,
    /// `element.background`: the resting background of a control or a
    /// header row, such as a search result's file header.
    pub element_background: Rgba,
    /// `editor.subheader.background`: the card of a file header in a
    /// multi-file view such as the Search tab's results.
    pub editor_subheader_background: Rgba,
    pub panel_background: Rgba,
    pub status_bar_background: Rgba,
    pub tab_active_background: Rgba,
    pub tab_inactive_background: Rgba,
    pub editor_background: Rgba,
    pub editor_foreground: Rgba,
    pub editor_line_number: Rgba,
    pub editor_active_line_number: Rgba,
    pub editor_active_line_background: Rgba,
    pub search_match_background: Rgba,
    pub selection: Rgba,
    /// The caret: the first player's `cursor`, else `editor.foreground`.
    pub cursor: Rgba,
    pub title_bar_background: Rgba,
    /// Floating surfaces: popovers, Quick Open, the find widget.
    pub elevated_surface_background: Rgba,
    /// Background of a borderless control under the mouse.
    pub ghost_element_hover: Rgba,
    /// The subtle border between chrome regions and between tabs.
    pub border_variant: Rgba,
    pub icon: Rgba,
    pub icon_muted: Rgba,
    /// `tab_bar.background`, else the theme's `panel.background`.
    pub tab_bar_background: Rgba,
    /// Error text, such as a broken settings file in the status bar.
    pub error: Rgba,
    pub warning: Rgba,
    pub info: Rgba,
    pub hint: Rgba,
    /// Diagnostic severity tints and borders (`<severity>.background`,
    /// `<severity>.border`).
    pub error_background: Rgba,
    pub warning_background: Rgba,
    pub info_background: Rgba,
    pub hint_background: Rgba,
    pub error_border: Rgba,
    pub warning_border: Rgba,
    pub info_border: Rgba,
    pub hint_border: Rgba,
    /// `link_text.hover`: a link or ⌘-hovered symbol under the mouse.
    pub link_text_hover: Rgba,
    /// `editor.highlighted_line.background`: the line a jump landed on.
    pub editor_highlighted_line_background: Rgba,
    /// `element.hover`: a control under the mouse.
    pub element_hover: Rgba,
    /// `scrollbar.thumb.background`: the thin overlay scrollbar's thumb.
    pub scrollbar_thumb: Rgba,
}

const fn gray(v: u8, a: u8) -> Rgba {
    Rgba {
        r: v as f32 / 255.0,
        g: v as f32 / 255.0,
        b: v as f32 / 255.0,
        a: a as f32 / 255.0,
    }
}

/// Error color for themes without an `error` key.
const ERROR_RED: Rgba = Rgba {
    r: 0xe5 as f32 / 255.0,
    g: 0x48 as f32 / 255.0,
    b: 0x4d as f32 / 255.0,
    a: 1.0,
};

impl UiColors {
    /// The single place that defines fallback colors for missing theme keys.
    pub fn fallback(appearance: Appearance) -> UiColors {
        let (bg, panel, fg, muted, border, sel, line_no, active_no) = match appearance {
            Appearance::Dark => (
                gray(0x1e, 255),
                gray(0x25, 255),
                gray(0xd4, 255),
                gray(0x8a, 255),
                gray(0x3c, 255),
                gray(0x3a, 255),
                gray(0x6e, 255),
                gray(0xc0, 255),
            ),
            Appearance::Light => (
                gray(0xff, 255),
                gray(0xf3, 255),
                gray(0x33, 255),
                gray(0x77, 255),
                gray(0xd0, 255),
                gray(0xdd, 255),
                gray(0x99, 255),
                gray(0x33, 255),
            ),
        };
        let (title_bar, elevated, hover, border_variant, scrollbar_thumb) = match appearance {
            Appearance::Dark => (
                gray(0x2b, 255),
                gray(0x2c, 255),
                gray(0x33, 255),
                gray(0x30, 255),
                gray(0xc8, 0x4c),
            ),
            Appearance::Light => (
                gray(0xe8, 255),
                gray(0xfa, 255),
                gray(0xe4, 255),
                gray(0xe2, 255),
                gray(0x38, 0x4c),
            ),
        };
        let hex = |s: &str| Rgba::parse_hex(s).expect("valid fallback hex");
        let pick = |dark: &str, light: &str| match appearance {
            Appearance::Dark => hex(dark),
            Appearance::Light => hex(light),
        };
        UiColors {
            background: panel,
            border,
            text: fg,
            text_muted: muted,
            text_disabled: muted,
            border_focused: border,
            text_accent: pick("#74ade8ff", "#5c78e2ff"),
            element_selected: sel,
            element_background: title_bar,
            editor_subheader_background: title_bar,
            panel_background: panel,
            status_bar_background: panel,
            tab_active_background: bg,
            tab_inactive_background: panel,
            editor_background: bg,
            editor_foreground: fg,
            editor_line_number: line_no,
            editor_active_line_number: active_no,
            editor_active_line_background: gray(0x80, 0x1f),
            search_match_background: gray(0x80, 0x55),
            selection: gray(0x80, 0x44),
            cursor: fg,
            title_bar_background: title_bar,
            elevated_surface_background: elevated,
            ghost_element_hover: hover,
            border_variant,
            icon: fg,
            icon_muted: muted,
            tab_bar_background: panel,
            error: ERROR_RED,
            warning: pick("#dec184ff", "#a48819ff"),
            info: pick("#74ade8ff", "#5c78e2ff"),
            hint: pick("#788ca6ff", "#7274a7ff"),
            error_background: pick("#d072771a", "#fbdfd9ff"),
            warning_background: pick("#dec1841a", "#faf2e6ff"),
            info_background: pick("#74ade81a", "#e2e2faff"),
            hint_background: pick("#5a6f891a", "#e2e2faff"),
            error_border: pick("#4c2b2cff", "#f6c6bdff"),
            warning_border: pick("#5d4c2fff", "#f4e7d1ff"),
            info_border: pick("#293b5bff", "#cbcdf6ff"),
            hint_border: pick("#293b5bff", "#cbcdf6ff"),
            link_text_hover: pick("#74ade8ff", "#5c78e2ff"),
            editor_highlighted_line_background: pick("#2f343eff", "#ebebecff"),
            element_hover: hover,
            scrollbar_thumb,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Theme {
    pub name: String,
    pub appearance: Appearance,
    pub ui: UiColors,
    syntax: HashMap<String, HighlightStyle>,
}

impl Theme {
    /// Style for a tree-sitter capture name: exact match first, then the
    /// name with its last `.segment` dropped, repeatedly.
    pub fn style_for(&self, capture: &str) -> Option<&HighlightStyle> {
        let mut name = capture;
        loop {
            if let Some(style) = self.syntax.get(name) {
                return Some(style);
            }
            name = name.rsplit_once('.')?.0;
        }
    }
}
