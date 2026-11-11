//! Converts GPUI key events into `lucerna_settings` keystrokes.

use gpui::KeyDownEvent;
use lucerna_settings::Keystroke;

/// The keystroke for `ev`, with the key named exactly as the keymap files
/// name it (`escape`, `enter`, `tab`, `space`, `\\`, `=`, ...).
///
/// Uses `key`, never `key_char`: with modifiers held `key_char` is the
/// produced character (`alt+z` types `Ω`), not the key that was pressed.
pub fn to_keystroke(ev: &KeyDownEvent) -> Keystroke {
    let gpui::Keystroke { modifiers, key, .. } = &ev.keystroke;
    Keystroke {
        cmd: modifiers.platform,
        ctrl: modifiers.control,
        alt: modifiers.alt,
        shift: modifiers.shift,
        key: canonical_key(key),
    }
}

fn canonical_key(key: &str) -> String {
    match key {
        "esc" => "escape".into(),
        "return" => "enter".into(),
        " " => "space".into(),
        "\t" => "tab".into(),
        other => other.to_lowercase(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::Modifiers;
    use lucerna_settings::{Keymap, parse_keys};

    /// A key-down as macOS reports it: `key` is the unmodified key, and
    /// `key_char` is what would be typed (none while cmd/ctrl is held).
    fn event(key: &str, key_char: Option<&str>, modifiers: Modifiers) -> KeyDownEvent {
        KeyDownEvent {
            keystroke: gpui::Keystroke {
                modifiers,
                key: key.into(),
                key_char: key_char.map(Into::into),
            },
            is_held: false,
            prefer_character_input: false,
        }
    }

    fn cmd() -> Modifiers {
        Modifiers::command()
    }

    fn cmd_shift() -> Modifiers {
        Modifiers::command_shift()
    }

    fn alt_cmd() -> Modifiers {
        Modifiers {
            alt: true,
            platform: true,
            ..Modifiers::none()
        }
    }

    fn ctrl() -> Modifiers {
        Modifiers::control()
    }

    fn alt_shift() -> Modifiers {
        Modifiers {
            alt: true,
            shift: true,
            ..Modifiers::none()
        }
    }

    fn ctrl_shift() -> Modifiers {
        Modifiers {
            control: true,
            shift: true,
            ..Modifiers::none()
        }
    }

    fn alt() -> Modifiers {
        Modifiers::alt()
    }

    /// One row per keystroke used in `default-keybindings.json`.
    fn table() -> Vec<(KeyDownEvent, &'static str)> {
        let mut rows = vec![
            (event("p", None, cmd()), "cmd+p"),
            (event("p", None, cmd_shift()), "cmd+shift+p"),
            (event("g", None, ctrl()), "ctrl+g"),
            (event("f", None, cmd_shift()), "cmd+shift+f"),
            (event("f", None, cmd()), "cmd+f"),
            (event("escape", None, Modifiers::none()), "escape"),
            (event("b", None, cmd()), "cmd+b"),
            (event("e", None, cmd_shift()), "cmd+shift+e"),
            (event("x", None, cmd_shift()), "cmd+shift+x"),
            (event("\\", None, cmd()), "cmd+\\"),
            (event("w", None, cmd()), "cmd+w"),
            (event("q", None, cmd()), "cmd+q"),
            (event("tab", Some("\t"), ctrl()), "ctrl+tab"),
            (event("k", None, cmd()), "cmd+k"),
            (event("t", None, cmd()), "cmd+t"),
            (event(",", None, cmd()), "cmd+,"),
            (event("z", Some("Ω"), alt()), "alt+z"),
            (event("=", None, cmd()), "cmd+="),
            (event("-", None, cmd()), "cmd+-"),
            (event("c", None, cmd()), "cmd+c"),
            (event("c", None, alt_cmd()), "alt+cmd+c"),
            (event("w", None, alt_cmd()), "alt+cmd+w"),
            (event("r", None, alt_cmd()), "alt+cmd+r"),
            (event("j", None, cmd_shift()), "shift+cmd+j"),
            (event("f4", None, Modifiers::none()), "f4"),
            (event("f4", None, Modifiers::shift()), "shift+f4"),
            (event("f12", None, Modifiers::none()), "f12"),
            (event("f12", None, alt()), "alt+f12"),
            (event("f12", None, cmd()), "cmd+f12"),
            (event("f12", None, cmd_shift()), "shift+cmd+f12"),
            (event("f12", None, Modifiers::shift()), "shift+f12"),
            (event("f12", None, alt_shift()), "shift+alt+f12"),
            (event("i", None, cmd()), "cmd+i"),
            (event("f8", None, Modifiers::none()), "f8"),
            (event("f8", None, Modifiers::shift()), "shift+f8"),
            (event("f8", None, alt()), "alt+f8"),
            (event("f8", None, alt_shift()), "shift+alt+f8"),
            (event("o", None, cmd_shift()), "shift+cmd+o"),
            (event(".", None, cmd_shift()), "shift+cmd+."),
            (event("-", None, ctrl()), "ctrl+-"),
            (event("-", None, ctrl_shift()), "ctrl+shift+-"),
        ];
        for (digit, binding) in [
            ("1", "cmd+1"),
            ("2", "cmd+2"),
            ("3", "cmd+3"),
            ("4", "cmd+4"),
            ("5", "cmd+5"),
            ("6", "cmd+6"),
            ("7", "cmd+7"),
            ("8", "cmd+8"),
        ] {
            rows.push((event(digit, None, cmd()), binding));
        }
        rows
    }

    #[test]
    fn default_keymap_keystrokes_convert() {
        for (ev, binding) in table() {
            let expected = parse_keys(binding).unwrap().remove(0);
            assert_eq!(to_keystroke(&ev), expected, "{binding}");
        }
    }

    #[test]
    fn table_covers_every_default_binding() {
        // The bundled file is JSONC with only whole-line comments.
        let json: String = Keymap::default_bindings()
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let bindings: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap();
        let covered: Vec<_> = table()
            .iter()
            .map(|(_, b)| parse_keys(b).unwrap().remove(0))
            .collect();
        for binding in &bindings {
            let keys = binding["key"].as_str().unwrap();
            for stroke in parse_keys(keys).unwrap() {
                assert!(covered.contains(&stroke), "no table row for '{keys}'");
            }
        }
    }

    #[test]
    fn named_keys_use_keymap_names() {
        for (gpui_key, name) in [
            ("escape", "escape"),
            ("esc", "escape"),
            ("enter", "enter"),
            ("return", "enter"),
            ("tab", "tab"),
            ("backspace", "backspace"),
            ("space", "space"),
            (" ", "space"),
            ("up", "up"),
            ("down", "down"),
            ("left", "left"),
            ("right", "right"),
        ] {
            let ev = event(gpui_key, None, Modifiers::none());
            assert_eq!(to_keystroke(&ev).key, name, "{gpui_key:?}");
        }
    }
}
