//! Zed theme loading. Pure data: no UI dependencies.

mod registry;
mod schema;

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
    pub element_selected: Rgba,
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
        UiColors {
            background: panel,
            border,
            text: fg,
            text_muted: muted,
            element_selected: sel,
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
            title_bar_background: title_bar,
            elevated_surface_background: elevated,
            ghost_element_hover: hover,
            border_variant,
            icon: fg,
            icon_muted: muted,
            tab_bar_background: panel,
            error: ERROR_RED,
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
