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
fn builtin_has_one_dark_and_light() {
    let reg = ThemeRegistry::builtin();
    let names = reg.names();
    assert!(names.contains(&"One Dark".to_string()));
    assert!(names.contains(&"One Light".to_string()));
    assert_eq!(reg.get("One Dark").unwrap().appearance, Appearance::Dark);
    assert_eq!(reg.get("One Light").unwrap().appearance, Appearance::Light);
}

#[test]
fn ui_colors_present_for_one_dark() {
    let t = ThemeRegistry::builtin().get("One Dark").unwrap();
    assert_eq!(
        t.ui.editor_background,
        Rgba::parse_hex("#282c33ff").unwrap()
    );
    assert_eq!(t.ui.text, Rgba::parse_hex("#dce0e5ff").unwrap());
    assert_eq!(t.ui.editor_line_number, Rgba::parse_hex("#4e5a5f").unwrap());
    assert_eq!(t.ui.selection, Rgba::parse_hex("#74ade83d").unwrap());
    assert_eq!(
        t.ui.status_bar_background,
        Rgba::parse_hex("#3b414dff").unwrap()
    );
}

#[test]
fn style_for_falls_back() {
    let t = ThemeRegistry::builtin().get("One Dark").unwrap();
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
    let mut reg = ThemeRegistry::builtin();
    assert!(reg.load_dir(dir.path()).is_empty());
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
fn load_dir_reports_bad_file_and_keeps_good() {
    let dir = tempfile::tempdir().unwrap();
    write_theme(dir.path(), "bad.json", "{ not json");
    write_theme(
        dir.path(),
        "good.json",
        r#"{"name":"F","themes":[{"name":"Good","appearance":"dark","style":{}}]}"#,
    );
    write_theme(dir.path(), "notes.txt", "ignored");
    let mut reg = ThemeRegistry::builtin();
    let errors = reg.load_dir(dir.path());
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].0, dir.path().join("bad.json"));
    assert!(!errors[0].1.is_empty());
    assert!(reg.get("Good").is_some());
    assert!(reg.get("One Dark").is_some());
    assert_eq!(reg.load_dir(&dir.path().join("missing")).len(), 1);
}

#[test]
fn unknown_name_defaults_to_one_dark() {
    let reg = ThemeRegistry::builtin();
    assert!(reg.get("Nope").is_none());
    assert_eq!(reg.get_or_default("Nope").name, "One Dark");
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
fn new_ui_colors_read_from_one_dark() {
    let t = ThemeRegistry::builtin().get("One Dark").unwrap();
    let fallback = UiColors::fallback(Appearance::Dark);
    let expected = [
        "#3b414dff",
        "#2f343eff",
        "#363c46ff",
        "#363c46ff",
        "#dce0e5ff",
        "#a9afbcff",
        "#2f343eff",
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
    let mut reg = ThemeRegistry::builtin();
    assert!(reg.load_dir(dir.path()).is_empty());
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
    let t = ThemeRegistry::builtin().get("One Dark").unwrap();
    assert_eq!(t.ui.error, Rgba::parse_hex("#d07277ff").unwrap());
    let dir = tempfile::tempdir().unwrap();
    write_theme(
        dir.path(),
        "plain.json",
        r#"{"name":"P","themes":[{"name":"Plain","appearance":"dark","style":{}}]}"#,
    );
    let mut reg = ThemeRegistry::builtin();
    assert!(reg.load_dir(dir.path()).is_empty());
    let fallback = reg.get("Plain").unwrap().ui.error;
    assert_eq!(fallback, UiColors::fallback(Appearance::Dark).error);
    assert!(
        fallback.r > 0.8 && fallback.g < 0.4 && fallback.b < 0.4,
        "red: {fallback:?}"
    );
}
