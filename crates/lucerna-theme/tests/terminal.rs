use lucerna_theme::{Appearance, Rgba, ThemeRegistry};

fn hex(s: &str) -> Rgba {
    Rgba::parse_hex(s).unwrap()
}

fn load(json: &str, name: &str) -> std::sync::Arc<lucerna_theme::Theme> {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("t.json"), json).unwrap();
    let (reg, errors) = ThemeRegistry::build(&[], Some(dir.path()));
    assert!(errors.is_empty(), "{errors:?}");
    reg.get(name).unwrap()
}

#[test]
fn reads_zed_terminal_keys() {
    let t = load(
        r##"{"name":"F","themes":[{"name":"T","appearance":"dark","style":{
            "terminal.background":"#010203ff",
            "terminal.foreground":"#040506ff",
            "terminal.bright_foreground":"#070809ff",
            "terminal.dim_foreground":"#0a0b0cff",
            "terminal.ansi.red":"#112233ff",
            "terminal.ansi.bright_blue":"#445566ff",
            "terminal.ansi.dim_green":"#778899ff"}}]}"##,
        "T",
    );
    let c = &t.terminal;
    assert_eq!(c.background, hex("#010203ff"));
    assert_eq!(c.foreground, hex("#040506ff"));
    assert_eq!(c.bright_foreground, hex("#070809ff"));
    assert_eq!(c.dim_foreground, hex("#0a0b0cff"));
    assert_eq!(c.ansi[1], hex("#112233ff"));
    assert_eq!(c.ansi[12], hex("#445566ff"));
    assert_eq!(c.dim[2], hex("#778899ff"));
}

#[test]
fn missing_terminal_keys_fall_back() {
    let json = r##"{"name":"F","themes":[
        {"name":"D","appearance":"dark","style":{}},
        {"name":"L","appearance":"light","style":{}}]}"##;
    let d = load(json, "D");
    assert_eq!(d.terminal.background, d.ui.editor_background);
    assert_eq!(d.terminal.foreground, d.ui.editor_foreground);
    assert_eq!(d.terminal.bright_foreground, d.terminal.foreground);
    assert_eq!(d.terminal.dim_foreground, d.terminal.foreground);
    assert_eq!(d.terminal.ansi[2], hex("#0DBC79"));
    assert_eq!(d.terminal.ansi[0], hex("#000000"));
    assert_eq!(d.terminal.ansi[15], hex("#E5E5E5"));
    let l = load(json, "L");
    assert_eq!(l.terminal.ansi[2], hex("#107C10"));
    assert_eq!(l.terminal.ansi[15], hex("#A5A5A5"));
    assert_ne!(d.terminal.ansi, l.terminal.ansi);
}

#[test]
fn fallback_tables_are_opaque_and_per_appearance() {
    let d = lucerna_theme::TerminalColors::fallback(Appearance::Dark);
    let l = lucerna_theme::TerminalColors::fallback(Appearance::Light);
    for c in d.ansi.iter().chain(l.ansi.iter()) {
        assert_eq!(c.a, 1.0);
    }
    assert_ne!(d.ansi, l.ansi);
}

#[test]
fn dim_defaults_to_alpha_07() {
    let t = load(
        r##"{"name":"F","themes":[{"name":"T","appearance":"dark","style":{
            "terminal.ansi.red":"#112233ff",
            "terminal.ansi.dim_blue":"#aabbccff"}}]}"##,
        "T",
    );
    let c = &t.terminal;
    let red = c.ansi[1];
    assert_eq!(
        c.dim[1],
        Rgba {
            a: red.a * 0.7,
            ..red
        }
    );
    // an explicit dim key wins
    assert_eq!(c.dim[4], hex("#aabbccff"));
    // fallback ansi also dims
    let green = c.ansi[2];
    assert_eq!(
        c.dim[2],
        Rgba {
            a: green.a * 0.7,
            ..green
        }
    );
}

#[test]
fn cursor_and_selection_from_players() {
    let t = load(
        r##"{"name":"F","themes":[
            {"name":"P","appearance":"dark","style":{
                "players":[{"cursor":"#abcdef","selection":"#11223344"}]}},
            {"name":"N","appearance":"dark","style":{}}]}"##,
        "P",
    );
    assert_eq!(t.terminal.cursor, hex("#abcdef"));
    assert_eq!(t.terminal.selection, hex("#11223344"));
    let reg_none = load(
        r##"{"name":"F","themes":[{"name":"N","appearance":"dark","style":{}}]}"##,
        "N",
    );
    assert_eq!(reg_none.terminal.cursor, reg_none.ui.cursor);
    assert_eq!(reg_none.terminal.selection, reg_none.ui.selection);
}

#[test]
fn builtin_themes_parse() {
    let reg = ThemeRegistry::builtin();
    for name in reg.names() {
        let t = reg.get(&name).unwrap();
        assert!(t.terminal.background.a > 0.0, "{name}");
        assert!(t.terminal.foreground.a > 0.0, "{name}");
    }
}

#[test]
fn fallback_ansi_matches_vscode_reference() {
    // microsoft/vscode terminalColorRegistry.ts defaults (`light` and `dark`).
    let dark = [
        "#000000", "#cd3131", "#0dbc79", "#e5e510", "#2472c8", "#bc3fbc", "#11a8cd", "#e5e5e5",
        "#666666", "#f14c4c", "#23d18b", "#f5f543", "#3b8eea", "#d670d6", "#29b8db", "#e5e5e5",
    ];
    let light = [
        "#000000", "#cd3131", "#107c10", "#949800", "#0451a5", "#bc05bc", "#0598bc", "#555555",
        "#666666", "#f14c4c", "#14ce14", "#b5ba00", "#3b8eea", "#d670d6", "#29b8db", "#a5a5a5",
    ];
    for (appearance, table) in [(Appearance::Dark, dark), (Appearance::Light, light)] {
        let got = lucerna_theme::TerminalColors::fallback(appearance).ansi;
        for (i, h) in table.iter().enumerate() {
            assert_eq!(got[i], hex(h), "{appearance:?} ansi[{i}]");
        }
    }
}
