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
