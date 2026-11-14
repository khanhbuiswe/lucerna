use lucerna_theme::{Appearance, Rgba, ThemeRegistry, UiColors};

fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

#[test]
fn parse_hex_rgb_and_rgba() {
    let c = Rgba::parse_hex("#ff8000").unwrap();
    assert!(approx(c.r, 1.0) && approx(c.g, 128.0 / 255.0) && approx(c.b, 0.0) && approx(c.a, 1.0));
    let c = Rgba::parse_hex("#00ff0080").unwrap();
    assert!(approx(c.g, 1.0) && approx(c.a, 128.0 / 255.0));
    for bad in ["", "ff8000", "#fff", "#gg0000", "#ff80001", "#é23456"] {
        assert!(Rgba::parse_hex(bad).is_none(), "{bad}");
    }
}

#[test]
fn builtin_has_lucerna_dark_and_light() {
    let reg = ThemeRegistry::builtin();
    let names = reg.names();
    assert!(names.contains(&"Lucerna Dark".to_string()));
    assert!(names.contains(&"Lucerna Light".to_string()));
    assert_eq!(
        reg.get("Lucerna Dark").unwrap().appearance,
        Appearance::Dark
    );
    assert_eq!(
        reg.get("Lucerna Light").unwrap().appearance,
        Appearance::Light
    );
}

#[test]
fn ui_colors_present_for_lucerna_dark() {
    let t = ThemeRegistry::builtin().get("Lucerna Dark").unwrap();
    assert_eq!(
        t.ui.editor_background,
        Rgba::parse_hex("#191919ff").unwrap()
    );
    assert_eq!(t.ui.text, Rgba::parse_hex("#efefefff").unwrap());
    assert_eq!(
        t.ui.editor_line_number,
        Rgba::parse_hex("#9c9c9cff").unwrap()
    );
    assert_eq!(t.ui.selection, Rgba::parse_hex("#4f8cff40").unwrap());
    assert_eq!(t.ui.cursor, Rgba::parse_hex("#4f8cffff").unwrap());
    assert_eq!(
        t.ui.status_bar_background,
        Rgba::parse_hex("#191919ff").unwrap()
    );
}

#[test]
fn style_for_falls_back() {
    let t = ThemeRegistry::builtin().get("Lucerna Dark").unwrap();
    let f = t.style_for("function").expect("function style");
    assert_eq!(t.style_for("function.method.call"), Some(f));
    assert_eq!(t.style_for("function.method.call").unwrap().color, f.color);
    assert!(t.style_for("nonexistent.capture").is_none());
    // exact match wins over the parent
    assert_ne!(t.style_for("string.escape"), t.style_for("string.nope"));
}

fn write_theme(dir: &std::path::Path, file: &str, json: &str) {
    std::fs::write(dir.join(file), json).unwrap();
}

#[test]
fn italic_and_bold_mapping() {
    let dir = tempfile::tempdir().unwrap();
    write_theme(
        dir.path(),
        "t.json",
        r##"{"name":"F","author":"a","themes":[{"name":"Styled","appearance":"light","unknown":1,"style":{
            "syntax":{
              "a":{"color":"#112233ff","font_style":"italic","font_weight":null},
              "b":{"color":null,"font_style":null,"font_weight":700},
              "c":{"color":"#000000","font_style":"normal","font_weight":400},
              "d":{"font_weight":600}
            }}}]}"##,
    );
    let (reg, errors) = ThemeRegistry::build(&[], Some(dir.path()));
    assert!(errors.is_empty());
    let t = reg.get("Styled").unwrap();
    let a = t.style_for("a").unwrap();
    assert!(a.italic && !a.bold && a.color.is_some());
    let b = t.style_for("b").unwrap();
    assert!(b.bold && !b.italic && b.color.is_none());
    let c = t.style_for("c").unwrap();
    assert!(!c.bold && !c.italic);
    assert!(t.style_for("d").unwrap().bold);
    // missing UI keys use the fixed per-appearance fallback
    assert_eq!(t.ui, UiColors::fallback(Appearance::Light));
}

#[test]
fn user_dir_reports_bad_file_and_keeps_good() {
    let dir = tempfile::tempdir().unwrap();
    write_theme(dir.path(), "bad.json", "{ not json");
    write_theme(
        dir.path(),
        "good.json",
        r#"{"name":"F","themes":[{"name":"Good","appearance":"dark","style":{}}]}"#,
    );
    write_theme(dir.path(), "notes.txt", "ignored");
    let (reg, errors) = ThemeRegistry::build(&[], Some(dir.path()));
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].0, dir.path().join("bad.json"));
    assert!(!errors[0].1.is_empty());
    assert!(reg.get("Good").is_some());
    assert!(reg.get("Lucerna Dark").is_some());
    assert_eq!(
        ThemeRegistry::build(&[], Some(&dir.path().join("missing")))
            .1
            .len(),
        1
    );
}

#[test]
fn unknown_name_defaults_to_lucerna_dark() {
    let reg = ThemeRegistry::builtin();
    assert!(reg.get("Nope").is_none());
    assert_eq!(reg.get_or_default("Nope").name, "Lucerna Dark");
}

#[test]
fn theme_and_registry_are_send_sync() {
    fn check<T: Send + Sync + std::fmt::Debug>() {}
    check::<lucerna_theme::Theme>();
    check::<ThemeRegistry>();
}

/// The chrome colors added for the Zed-style layout, by Zed key.
fn chrome_colors(ui: &UiColors) -> [(&'static str, Rgba); 7] {
    [
        ("title_bar.background", ui.title_bar_background),
        (
            "elevated_surface.background",
            ui.elevated_surface_background,
        ),
        ("ghost_element.hover", ui.ghost_element_hover),
        ("border.variant", ui.border_variant),
        ("icon", ui.icon),
        ("icon.muted", ui.icon_muted),
        ("tab_bar.background", ui.tab_bar_background),
    ]
}

#[test]
fn new_ui_colors_read_from_lucerna_dark() {
    let t = ThemeRegistry::builtin().get("Lucerna Dark").unwrap();
    let fallback = UiColors::fallback(Appearance::Dark);
    let expected = [
        "#191919ff",
        "#222222ff",
        "#252424ff",
        "#252525ff",
        "#efefefff",
        "#9c9c9cff",
        "#191919ff",
    ];
    let got = chrome_colors(&t.ui);
    let fb = chrome_colors(&fallback);
    for (((key, got), (_, fb)), hex) in got.into_iter().zip(fb).zip(expected) {
        assert_eq!(got, Rgba::parse_hex(hex).unwrap(), "{key}");
        assert_ne!(got, fb, "{key} must come from the theme");
    }
}

#[test]
fn new_ui_colors_fall_back_when_missing() {
    let dir = tempfile::tempdir().unwrap();
    write_theme(
        dir.path(),
        "t.json",
        r##"{"name":"F","themes":[
            {"name":"Bare Dark","appearance":"dark","style":{}},
            {"name":"Bare Light","appearance":"light","style":{}},
            {"name":"Panel Only","appearance":"dark","style":{"panel.background":"#123456ff"}}
        ]}"##,
    );
    let (reg, errors) = ThemeRegistry::build(&[], Some(dir.path()));
    assert!(errors.is_empty());
    for (name, appearance) in [
        ("Bare Dark", Appearance::Dark),
        ("Bare Light", Appearance::Light),
    ] {
        let fb = UiColors::fallback(appearance);
        let got = reg.get(name).unwrap();
        for ((key, a), (_, b)) in chrome_colors(&got.ui).into_iter().zip(chrome_colors(&fb)) {
            assert_eq!(a, b, "{name} {key}");
        }
    }
    // The two appearances' fallbacks differ.
    let dark = chrome_colors(&UiColors::fallback(Appearance::Dark));
    let light = chrome_colors(&UiColors::fallback(Appearance::Light));
    for ((key, d), (_, l)) in dark.into_iter().zip(light) {
        assert_ne!(d, l, "{key}");
    }
    // Without `tab_bar.background` the tab bar uses the theme's panel color.
    let panel = reg.get("Panel Only").unwrap();
    assert_eq!(
        panel.ui.tab_bar_background,
        Rgba::parse_hex("#123456ff").unwrap()
    );
}

#[test]
fn error_color_from_theme_or_red_fallback() {
    let t = ThemeRegistry::builtin().get("Lucerna Dark").unwrap();
    assert_eq!(t.ui.error, Rgba::parse_hex("#f85149ff").unwrap());
    let dir = tempfile::tempdir().unwrap();
    write_theme(
        dir.path(),
        "plain.json",
        r#"{"name":"P","themes":[{"name":"Plain","appearance":"dark","style":{}}]}"#,
    );
    let (reg, errors) = ThemeRegistry::build(&[], Some(dir.path()));
    assert!(errors.is_empty());
    let fallback = reg.get("Plain").unwrap().ui.error;
    assert_eq!(fallback, UiColors::fallback(Appearance::Dark).error);
    assert!(
        fallback.r > 0.8 && fallback.g < 0.4 && fallback.b < 0.4,
        "red: {fallback:?}"
    );
}

#[test]
fn scrollbar_thumb_from_theme_or_fallback() {
    let dark = ThemeRegistry::builtin().get("Lucerna Dark").unwrap();
    assert_eq!(
        dark.ui.scrollbar_thumb,
        Rgba::parse_hex("#c8c8c84c").unwrap()
    );
    let dir = tempfile::tempdir().unwrap();
    write_theme(
        dir.path(),
        "plain.json",
        r#"{"name":"P","themes":[
            {"name":"Plain","appearance":"dark","style":{}},
            {"name":"Plain Light","appearance":"light","style":{}}
        ]}"#,
    );
    let (reg, errors) = ThemeRegistry::build(&[], Some(dir.path()));
    assert!(errors.is_empty());
    let fb_dark = UiColors::fallback(Appearance::Dark).scrollbar_thumb;
    let fb_light = UiColors::fallback(Appearance::Light).scrollbar_thumb;
    assert_eq!(reg.get("Plain").unwrap().ui.scrollbar_thumb, fb_dark);
    assert_eq!(reg.get("Plain Light").unwrap().ui.scrollbar_thumb, fb_light);
    assert_ne!(fb_dark, fb_light);
    // Translucent, so it never hides the rows under it.
    assert!(fb_dark.a < 1.0 && fb_light.a < 1.0);
}

#[test]
fn editor_subheader_background_from_theme_or_fallback() {
    let dark = ThemeRegistry::builtin().get("Lucerna Dark").unwrap();
    assert_eq!(
        dark.ui.editor_subheader_background,
        Rgba::parse_hex("#222222ff").unwrap()
    );
    let dir = tempfile::tempdir().unwrap();
    write_theme(
        dir.path(),
        "plain.json",
        r#"{"name":"P","themes":[
            {"name":"Plain","appearance":"dark","style":{}},
            {"name":"Plain Light","appearance":"light","style":{}}
        ]}"#,
    );
    let (reg, errors) = ThemeRegistry::build(&[], Some(dir.path()));
    assert!(errors.is_empty());
    for (name, appearance) in [
        ("Plain", Appearance::Dark),
        ("Plain Light", Appearance::Light),
    ] {
        assert_eq!(
            reg.get(name).unwrap().ui.editor_subheader_background,
            UiColors::fallback(appearance).editor_subheader_background,
            "{name}"
        );
    }
}

#[test]
fn theme_reads_diagnostic_colors() {
    let dir = tempfile::tempdir().unwrap();
    write_theme(
        dir.path(),
        "diag.json",
        r##"{"name":"D","themes":[
            {"name":"Set","appearance":"dark","style":{
                "warning":"#112233ff","info":"#112234ff","hint":"#112235ff",
                "error.background":"#112236ff","warning.background":"#112237ff",
                "info.background":"#112238ff","hint.background":"#112239ff",
                "error.border":"#11223aff","warning.border":"#11223bff",
                "info.border":"#11223cff","hint.border":"#11223dff",
                "link_text.hover":"#11223eff",
                "editor.highlighted_line.background":"#11223fff",
                "element.hover":"#112240ff"}},
            {"name":"Plain","appearance":"dark","style":{}},
            {"name":"Plain Light","appearance":"light","style":{}}
        ]}"##,
    );
    let (reg, errors) = ThemeRegistry::build(&[], Some(dir.path()));
    assert!(errors.is_empty());
    let hex = |s: &str| Rgba::parse_hex(s).unwrap();
    let set = &reg.get("Set").unwrap().ui;
    assert_eq!(set.warning, hex("#112233ff"));
    assert_eq!(set.info, hex("#112234ff"));
    assert_eq!(set.hint, hex("#112235ff"));
    assert_eq!(set.error_background, hex("#112236ff"));
    assert_eq!(set.warning_background, hex("#112237ff"));
    assert_eq!(set.info_background, hex("#112238ff"));
    assert_eq!(set.hint_background, hex("#112239ff"));
    assert_eq!(set.error_border, hex("#11223aff"));
    assert_eq!(set.warning_border, hex("#11223bff"));
    assert_eq!(set.info_border, hex("#11223cff"));
    assert_eq!(set.hint_border, hex("#11223dff"));
    assert_eq!(set.link_text_hover, hex("#11223eff"));
    assert_eq!(set.editor_highlighted_line_background, hex("#11223fff"));
    assert_eq!(set.element_hover, hex("#112240ff"));

    let dark = &reg.get("Plain").unwrap().ui;
    let light = &reg.get("Plain Light").unwrap().ui;
    assert_eq!(*dark, UiColors::fallback(Appearance::Dark));
    assert_eq!(*light, UiColors::fallback(Appearance::Light));
    assert_eq!(dark.warning, hex("#dec184ff"));
    assert_eq!(light.warning, hex("#a48819ff"));
    assert_eq!(dark.info, hex("#74ade8ff"));
    assert_eq!(light.info, hex("#5c78e2ff"));
    assert_eq!(dark.hint, hex("#788ca6ff"));
    assert_eq!(light.hint, hex("#7274a7ff"));
    assert_eq!(dark.error_background, hex("#d072771a"));
    assert_eq!(light.error_background, hex("#fbdfd9ff"));
    assert_eq!(dark.warning_background, hex("#dec1841a"));
    assert_eq!(light.warning_background, hex("#faf2e6ff"));
    assert_eq!(dark.info_background, hex("#74ade81a"));
    assert_eq!(light.info_background, hex("#e2e2faff"));
    assert_eq!(dark.hint_background, hex("#5a6f891a"));
    assert_eq!(light.hint_background, hex("#e2e2faff"));
    assert_eq!(dark.error_border, hex("#4c2b2cff"));
    assert_eq!(light.error_border, hex("#f6c6bdff"));
    assert_eq!(dark.warning_border, hex("#5d4c2fff"));
    assert_eq!(light.warning_border, hex("#f4e7d1ff"));
    assert_eq!(dark.info_border, hex("#293b5bff"));
    assert_eq!(light.info_border, hex("#cbcdf6ff"));
    assert_eq!(dark.hint_border, hex("#293b5bff"));
    assert_eq!(light.hint_border, hex("#cbcdf6ff"));
    assert_eq!(dark.link_text_hover, hex("#74ade8ff"));
    assert_eq!(light.link_text_hover, hex("#5c78e2ff"));
    assert_eq!(dark.editor_highlighted_line_background, hex("#2f343eff"));
    assert_eq!(light.editor_highlighted_line_background, hex("#ebebecff"));
    assert_eq!(dark.element_hover, dark.ghost_element_hover);
    assert_eq!(light.element_hover, light.ghost_element_hover);
}

#[test]
fn cursor_falls_back_to_editor_foreground() {
    let dir = tempfile::tempdir().unwrap();
    write_theme(
        dir.path(),
        "t.json",
        r##"{"name":"F","themes":[
            {"name":"No Cursor","appearance":"dark","style":{
                "editor.foreground":"#abcdefff","players":[{"selection":"#11223344"}]}}
        ]}"##,
    );
    let (reg, errors) = ThemeRegistry::build(&[], Some(dir.path()));
    assert!(errors.is_empty());
    let ui = &reg.get("No Cursor").unwrap().ui;
    assert_eq!(ui.cursor, Rgba::parse_hex("#abcdefff").unwrap());
    assert_eq!(ui.selection, Rgba::parse_hex("#11223344").unwrap());
}

#[test]
fn git_keys_read_and_fall_back() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("g.json"),
        r##"{"name":"G","themes":[
          {"name":"With","appearance":"dark","style":{"version_control.added":"#112233ff","created":"#445566ff"}},
          {"name":"Without","appearance":"dark","style":{}},
          {"name":"WithoutLight","appearance":"light","style":{}}
        ]}"##,
    )
    .unwrap();
    let (reg, errors) = ThemeRegistry::build(&[], Some(dir.path()));
    assert!(errors.is_empty(), "{errors:?}");
    let hex = |s: &str| Rgba::parse_hex(s).unwrap();

    let with = reg.get("With").unwrap();
    assert_eq!(with.ui.vc_added, hex("#112233ff"));
    assert_eq!(with.ui.status_created, hex("#445566ff"));
    // Keys the theme lacks still fall back.
    assert_eq!(with.ui.vc_modified, hex("#1B81A8"));

    let dark = reg.get("Without").unwrap();
    assert_eq!(dark.ui.vc_added, hex("#487E02"));
    assert_eq!(dark.ui.vc_modified, hex("#1B81A8"));
    assert_eq!(dark.ui.vc_deleted, hex("#F14C4C"));
    assert_eq!(dark.ui.status_created, hex("#73C991"));
    assert_eq!(dark.ui.status_modified, hex("#E2C08D"));
    assert_eq!(dark.ui.status_deleted, hex("#C74E39"));
    assert_eq!(dark.ui.status_conflict, hex("#E4676B"));
    assert_eq!(dark.ui.status_ignored, hex("#8C8C8C"));
    assert_eq!(dark.ui.diff_added_background, hex("#9BB95533"));
    assert_eq!(dark.ui.diff_deleted_background, hex("#FF000033"));

    let light = reg.get("WithoutLight").unwrap();
    assert_eq!(light.ui.vc_added, hex("#48985D"));
    assert_eq!(light.ui.status_modified, hex("#895503"));
    assert_eq!(light.ui.status_ignored, hex("#8E8E90"));
}

#[test]
fn zed_theme_reads_text_disabled_and_border_focused() {
    let dir = tempfile::tempdir().unwrap();
    write_theme(
        dir.path(),
        "t.json",
        r##"{"name":"F","themes":[{"name":"Keys","appearance":"dark","style":{
            "text.disabled":"#111111ff","border.focused":"#222222ff"}}]}"##,
    );
    let (reg, errors) = ThemeRegistry::build(&[], Some(dir.path()));
    assert!(errors.is_empty());
    let ui = &reg.get("Keys").unwrap().ui;
    assert_eq!(ui.text_disabled, Rgba::parse_hex("#111111ff").unwrap());
    assert_eq!(ui.border_focused, Rgba::parse_hex("#222222ff").unwrap());
}

#[test]
fn missing_disabled_and_focused_fall_back() {
    let dir = tempfile::tempdir().unwrap();
    write_theme(
        dir.path(),
        "t.json",
        r##"{"name":"F","themes":[{"name":"Bare","appearance":"dark","style":{
            "text.muted":"#333333ff","border":"#444444ff"}}]}"##,
    );
    let (reg, errors) = ThemeRegistry::build(&[], Some(dir.path()));
    assert!(errors.is_empty());
    let ui = &reg.get("Bare").unwrap().ui;
    assert_eq!(ui.text_disabled, ui.text_muted);
    assert_eq!(ui.border_focused, ui.border);
}

/// The hover and drag colors are the resting thumb made more opaque, even
/// where the theme sets `scrollbar.thumb.hover_background`.
#[test]
fn scrollbar_thumb_hover_and_active_derive_from_the_thumb() {
    let dark = ThemeRegistry::builtin().get("Lucerna Dark").unwrap();
    let thumb = dark.ui.scrollbar_thumb;
    for (c, k) in [
        (dark.ui.scrollbar_thumb_hover, 1.6),
        (dark.ui.scrollbar_thumb_active, 2.2),
    ] {
        assert_eq!((c.r, c.g, c.b), (thumb.r, thumb.g, thumb.b));
        assert!((c.a - thumb.a * k).abs() < 1e-6, "{c:?}");
    }
    let dir = tempfile::tempdir().unwrap();
    write_theme(
        dir.path(),
        "opaque.json",
        r##"{"name":"O","themes":[{"name":"Opaque","appearance":"dark",
            "style":{"scrollbar.thumb.background":"#808080cc"}}]}"##,
    );
    let (reg, errors) = ThemeRegistry::build(&[], Some(dir.path()));
    assert!(errors.is_empty());
    let ui = &reg.get("Opaque").unwrap().ui;
    assert_eq!(ui.scrollbar_thumb_active.a, 1.0);
    assert_eq!(ui.scrollbar_thumb_hover.a, 1.0);
    let fb = UiColors::fallback(Appearance::Light);
    assert!(fb.scrollbar_thumb_hover.a > fb.scrollbar_thumb.a);
}
