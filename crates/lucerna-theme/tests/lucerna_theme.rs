//! The bundled Lucerna themes: every Zed key is present and text meets
//! WCAG AA. Spec: docs/superpowers/specs/2026-11-14-lucerna-theme-design.md.

use lucerna_theme::{Appearance, DEFAULT_DARK_THEME, DEFAULT_LIGHT_THEME, Rgba, ThemeRegistry};
use serde_json::{Map, Value};

const LUCERNA_JSON: &str = include_str!("../assets/lucerna.json");

/// The style keys of Zed's One theme (Zed rev b5cd310), copied once from
/// `one.json`, plus the diff hunk backgrounds the git views read.
const STYLE_KEYS: &[&str] = &[
    "background",
    "border",
    "border.disabled",
    "border.focused",
    "border.selected",
    "border.transparent",
    "border.variant",
    "conflict",
    "conflict.background",
    "conflict.border",
    "created",
    "created.background",
    "created.border",
    "deleted",
    "deleted.background",
    "deleted.border",
    "drop_target.background",
    "editor.active_line.background",
    "editor.active_line_number",
    "editor.active_wrap_guide",
    "editor.background",
    "editor.document_highlight.read_background",
    "editor.document_highlight.write_background",
    "editor.foreground",
    "editor.gutter.background",
    "editor.highlighted_line.background",
    "editor.hover_line_number",
    "editor.invisible",
    "editor.line_number",
    "editor.subheader.background",
    "editor.wrap_guide",
    "element.active",
    "element.background",
    "element.disabled",
    "element.hover",
    "element.selected",
    "elevated_surface.background",
    "error",
    "error.background",
    "error.border",
    "ghost_element.active",
    "ghost_element.background",
    "ghost_element.disabled",
    "ghost_element.hover",
    "ghost_element.selected",
    "hidden",
    "hidden.background",
    "hidden.border",
    "hint",
    "hint.background",
    "hint.border",
    "icon",
    "icon.accent",
    "icon.disabled",
    "icon.muted",
    "icon.placeholder",
    "ignored",
    "ignored.background",
    "ignored.border",
    "info",
    "info.background",
    "info.border",
    "link_text.hover",
    "modified",
    "modified.background",
    "modified.border",
    "pane.focused_border",
    "panel.background",
    "panel.focused_border",
    "predictive",
    "predictive.background",
    "predictive.border",
    "renamed",
    "renamed.background",
    "renamed.border",
    "scrollbar.thumb.background",
    "scrollbar.thumb.border",
    "scrollbar.thumb.hover_background",
    "scrollbar.track.background",
    "scrollbar.track.border",
    "search.active_match_background",
    "search.match_background",
    "status_bar.background",
    "success",
    "success.background",
    "success.border",
    "surface.background",
    "tab.active_background",
    "tab.inactive_background",
    "tab_bar.background",
    "terminal.ansi.black",
    "terminal.ansi.blue",
    "terminal.ansi.bright_black",
    "terminal.ansi.bright_blue",
    "terminal.ansi.bright_cyan",
    "terminal.ansi.bright_green",
    "terminal.ansi.bright_magenta",
    "terminal.ansi.bright_red",
    "terminal.ansi.bright_white",
    "terminal.ansi.bright_yellow",
    "terminal.ansi.cyan",
    "terminal.ansi.dim_black",
    "terminal.ansi.dim_blue",
    "terminal.ansi.dim_cyan",
    "terminal.ansi.dim_green",
    "terminal.ansi.dim_magenta",
    "terminal.ansi.dim_red",
    "terminal.ansi.dim_white",
    "terminal.ansi.dim_yellow",
    "terminal.ansi.green",
    "terminal.ansi.magenta",
    "terminal.ansi.red",
    "terminal.ansi.white",
    "terminal.ansi.yellow",
    "terminal.background",
    "terminal.bright_foreground",
    "terminal.dim_foreground",
    "terminal.foreground",
    "text",
    "text.accent",
    "text.disabled",
    "text.muted",
    "text.placeholder",
    "title_bar.background",
    "title_bar.inactive_background",
    "toolbar.background",
    "unreachable",
    "unreachable.background",
    "unreachable.border",
    "version_control.added",
    "version_control.conflict_marker.ours",
    "version_control.conflict_marker.theirs",
    "version_control.deleted",
    "version_control.modified",
    "version_control.word_added",
    "version_control.word_deleted",
    "warning",
    "warning.background",
    "warning.border",
    "editor.diff_hunk.added.background",
    "editor.diff_hunk.deleted.background",
];

/// The syntax captures of the same theme.
const SYNTAX_KEYS: &[&str] = &[
    "attribute",
    "boolean",
    "comment",
    "comment.doc",
    "constant",
    "constructor",
    "diff.minus",
    "diff.plus",
    "embedded",
    "emphasis",
    "emphasis.strong",
    "enum",
    "function",
    "hint",
    "keyword",
    "label",
    "link_text",
    "link_uri",
    "namespace",
    "number",
    "operator",
    "predictive",
    "preproc",
    "primary",
    "property",
    "punctuation",
    "punctuation.bracket",
    "punctuation.delimiter",
    "punctuation.list_marker",
    "punctuation.markup",
    "punctuation.special",
    "selector",
    "selector.pseudo",
    "string",
    "string.escape",
    "string.regex",
    "string.special",
    "string.special.symbol",
    "tag",
    "text.literal",
    "title",
    "type",
    "variable",
    "variable.parameter",
    "variable.special",
    "variant",
];

fn themes() -> Vec<(String, Map<String, Value>)> {
    let value = jsonc_parser::parse_to_serde_value(LUCERNA_JSON, &Default::default())
        .unwrap()
        .unwrap();
    value["themes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            let name = t["name"].as_str().unwrap().to_string();
            (name, t["style"].as_object().unwrap().clone())
        })
        .collect()
}

fn rgba(hex: &str) -> [f64; 4] {
    let h = hex.strip_prefix('#').unwrap();
    let b = |i: usize| f64::from(u8::from_str_radix(&h[i..i + 2], 16).unwrap()) / 255.0;
    [b(0), b(2), b(4), if h.len() == 8 { b(6) } else { 1.0 }]
}

fn luminance(c: [f64; 3]) -> f64 {
    let lin = |v: f64| {
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
}

/// WCAG 2.x contrast of `fg` over `bg`; an `fg` with alpha is blended onto
/// `bg` first, and `bg` is taken as opaque.
fn contrast(fg: &str, bg: &str) -> f64 {
    let (f, b) = (rgba(fg), rgba(bg));
    let blend = |i: usize| f[i] * f[3] + b[i] * (1.0 - f[3]);
    let (lf, lb) = (
        luminance([blend(0), blend(1), blend(2)]),
        luminance([b[0], b[1], b[2]]),
    );
    (lf.max(lb) + 0.05) / (lf.min(lb) + 0.05)
}

fn get<'a>(style: &'a Map<String, Value>, key: &str) -> &'a str {
    style
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("missing {key}"))
}

fn syntax_colors(style: &Map<String, Value>) -> Vec<(String, String)> {
    style["syntax"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.clone(), v["color"].as_str().unwrap().to_string()))
        .collect()
}

/// Every `(label, fg)` that is below `min` over `bg`.
fn failures(pairs: &[(String, String)], bg: &str, min: f64) -> Vec<String> {
    pairs
        .iter()
        .filter(|(_, fg)| contrast(fg, bg) < min)
        .map(|(k, fg)| format!("{k} {fg} = {:.2} on {bg}", contrast(fg, bg)))
        .collect()
}

fn keyed(style: &Map<String, Value>, keys: &[&str]) -> Vec<(String, String)> {
    keys.iter()
        .map(|k| (k.to_string(), get(style, k).to_string()))
        .collect()
}

#[test]
fn builtin_is_only_lucerna() {
    let reg = ThemeRegistry::builtin();
    assert_eq!(reg.names(), vec!["Lucerna Dark", "Lucerna Light"]);
    assert_eq!(DEFAULT_DARK_THEME, "Lucerna Dark");
    assert_eq!(DEFAULT_LIGHT_THEME, "Lucerna Light");
    assert_eq!(
        reg.get("Lucerna Dark").unwrap().appearance,
        Appearance::Dark
    );
    assert_eq!(
        reg.get("Lucerna Light").unwrap().appearance,
        Appearance::Light
    );
    assert_eq!(reg.get_or_default("Lucerna Dark").name, "Lucerna Dark");
}

#[test]
fn every_zed_key_is_present() {
    for (name, style) in themes() {
        let missing: Vec<&str> = STYLE_KEYS
            .iter()
            .copied()
            .filter(|k| !style.contains_key(*k))
            .collect();
        assert!(missing.is_empty(), "{name} lacks {missing:?}");
        let syntax = style["syntax"].as_object().unwrap();
        let missing: Vec<&str> = SYNTAX_KEYS
            .iter()
            .copied()
            .filter(|k| !syntax.contains_key(*k))
            .collect();
        assert!(missing.is_empty(), "{name} lacks syntax {missing:?}");
        assert_eq!(style["players"].as_array().unwrap().len(), 8, "{name}");
    }
}

#[test]
fn text_colors_meet_aa_on_editor_background() {
    for (name, style) in themes() {
        let mut pairs = keyed(
            &style,
            &[
                "text",
                "text.muted",
                "text.placeholder",
                "text.accent",
                "editor.foreground",
                "editor.line_number",
                "editor.active_line_number",
                "error",
                "warning",
                "info",
                "hint",
                "success",
                "created",
                "modified",
                "deleted",
                "conflict",
                "renamed",
                "ignored",
                "terminal.foreground",
            ],
        );
        pairs.extend(syntax_colors(&style));
        let skip: &[&str] = if name == "Lucerna Dark" {
            &["black"]
        } else {
            &["white", "bright_white"]
        };
        for color in [
            "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
        ] {
            for key in [color.to_string(), format!("bright_{color}")] {
                if !skip.contains(&key.as_str()) {
                    let k = format!("terminal.ansi.{key}");
                    pairs.push((k.clone(), get(&style, &k).to_string()));
                }
            }
        }
        let bad = failures(&pairs, get(&style, "editor.background"), 4.5);
        assert!(bad.is_empty(), "{name}: {bad:#?}");
    }
}

#[test]
fn list_text_meets_aa_on_selected_row() {
    for (name, style) in themes() {
        let pairs = keyed(
            &style,
            &[
                "text",
                "text.muted",
                "created",
                "modified",
                "deleted",
                "conflict",
                "renamed",
                "ignored",
            ],
        );
        let bad = failures(&pairs, get(&style, "element.selected"), 4.5);
        assert!(bad.is_empty(), "{name}: {bad:#?}");
    }
}

#[test]
fn syntax_meets_aa_on_active_line() {
    for (name, style) in themes() {
        let bg = rgba(get(&style, "editor.active_line.background"));
        assert_eq!(bg[3], 1.0, "{name}: the active line is opaque");
        let bad = failures(
            &syntax_colors(&style),
            get(&style, "editor.active_line.background"),
            4.5,
        );
        assert!(bad.is_empty(), "{name}: {bad:#?}");
    }
}

#[test]
fn non_text_meets_3_to_1() {
    for (name, style) in themes() {
        let mut pairs = keyed(
            &style,
            &[
                "border.focused",
                "version_control.added",
                "version_control.modified",
                "version_control.deleted",
            ],
        );
        let cursor = style["players"][0]["cursor"].as_str().unwrap().to_string();
        pairs.push(("players[0].cursor".into(), cursor));
        let bad = failures(&pairs, get(&style, "editor.background"), 3.0);
        assert!(bad.is_empty(), "{name}: {bad:#?}");
    }
}

#[test]
fn spec_palette_values() {
    let all = themes();
    let (dark, light) = (&all[0].1, &all[1].1);
    let syn =
        |s: &Map<String, Value>, k: &str| s["syntax"][k]["color"].as_str().unwrap().to_string();
    assert_eq!(get(dark, "editor.background"), "#191919ff");
    assert_eq!(syn(dark, "keyword"), "#cc7bf4ff");
    assert_eq!(syn(dark, "function"), "#70b8ffff");
    assert_eq!(syn(dark, "comment"), "#818898ff");
    assert_eq!(dark["players"][0]["cursor"], "#4f8cffff");
    assert_eq!(get(light, "editor.background"), "#ffffffff");
    assert_eq!(syn(light, "keyword"), "#8101c2ff");
    assert_eq!(syn(light, "comment"), "#677080ff");
    assert_eq!(syn(light, "number"), "#007a7aff");
    assert_eq!(get(light, "text.accent"), "#1a5dccff");
}

/// Both Lucerna themes set every git color the workbench reads, with the
/// spec's values (§ Palette, Git), so none falls back to VS Code's colors.
#[test]
fn git_colors_come_from_the_theme() {
    let reg = ThemeRegistry::builtin();
    let expected: [(&str, [&str; 10]); 2] = [
        (
            "Lucerna Dark",
            [
                "#37ac42ff",
                "#4f8cffff",
                "#ff707bff",
                "#7ee787ff",
                "#e1c08dff",
                "#ff7b72ff",
                "#f0883eff",
                "#9c9c9cff",
                "#253a24ff",
                "#2e1e1fff",
            ],
        ),
        (
            "Lucerna Light",
            [
                "#218635ff",
                "#4f8cffff",
                "#b7224cff",
                "#116329ff",
                "#895503ff",
                "#c21d29ff",
                "#a84400ff",
                "#666666ff",
                "#e7f3e7ff",
                "#feeeefff",
            ],
        ),
    ];
    let keys = [
        "version_control.added",
        "version_control.modified",
        "version_control.deleted",
        "created",
        "modified",
        "deleted",
        "conflict",
        "ignored",
        "editor.diff_hunk.added.background",
        "editor.diff_hunk.deleted.background",
    ];
    let all = themes();
    for (name, hexes) in expected {
        let style = &all.iter().find(|(n, _)| n == name).unwrap().1;
        let ui = &reg.get(name).unwrap().ui;
        let read = [
            ui.vc_added,
            ui.vc_modified,
            ui.vc_deleted,
            ui.status_created,
            ui.status_modified,
            ui.status_deleted,
            ui.status_conflict,
            ui.status_ignored,
            ui.diff_added_background,
            ui.diff_deleted_background,
        ];
        for ((key, hex), got) in keys.iter().zip(hexes).zip(read) {
            assert_eq!(get(style, key), hex, "{name} {key}");
            assert_eq!(got, Rgba::parse_hex(hex).unwrap(), "{name} {key}");
        }
    }
}
