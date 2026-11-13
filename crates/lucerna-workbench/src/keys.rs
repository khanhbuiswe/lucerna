//! Converts GPUI key events into `lucerna_settings` keystrokes.

use gpui::KeyDownEvent;
use lucerna_settings::Keystroke;

/// The keystroke for `ev`, with the key named exactly as the keymap files
/// name it (`escape`, `enter`, `tab`, `space`, `\\`, `=`, ...).
///
/// Uses `key`, never `key_char`: with modifiers held `key_char` is the
/// produced character (`alt+z` types `Ω`), not the key that was pressed.
///
/// On macOS, shift held with a key that is not a letter reports the
/// shifted character with `shift` cleared (`ctrl+shift+-` arrives as
/// `ctrl` + `_`, `cmd+shift+.` as `cmd` + `>`; gpui_macos
/// `events.rs` `parse_keystroke`, the `shift = false; chars_with_shift`
/// branch). Those are turned back into `shift` + the unshifted key, which
/// is how the keymap files write them. US layout.
///
/// A user binding written with the shifted character (`ctrl+_`,
/// `cmd+>`) therefore never matches: write `ctrl+shift+-` / `cmd+shift+.`.
/// (Normalising in the keymap parser would touch another crate for the
/// same effect.)
pub fn to_keystroke(ev: &KeyDownEvent) -> Keystroke {
    let gpui::Keystroke { modifiers, key, .. } = &ev.keystroke;
    let mut keystroke = Keystroke {
        cmd: modifiers.platform,
        ctrl: modifiers.control,
        alt: modifiers.alt,
        shift: modifiers.shift,
        key: canonical_key(key),
    };
    let chorded = keystroke.cmd || keystroke.ctrl || keystroke.alt;
    if chorded
        && !keystroke.shift
        && let Some(base) = unshifted(&keystroke.key)
    {
        keystroke.key = base.into();
        keystroke.shift = true;
    }
    keystroke
}

/// The key a US-layout shifted character is typed with.
fn unshifted(key: &str) -> Option<&'static str> {
    Some(match key {
        "_" => "-",
        "+" => "=",
        "<" => ",",
        ">" => ".",
        "?" => "/",
        ":" => ";",
        "\"" => "'",
        "{" => "[",
        "}" => "]",
        "|" => "\\",
        "~" => "`",
        "!" => "1",
        "@" => "2",
        "#" => "3",
        "$" => "4",
        "%" => "5",
        "^" => "6",
        "&" => "7",
        "*" => "8",
        "(" => "9",
        ")" => "0",
        _ => return None,
    })
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

    fn ctrl_shift_cmd() -> Modifiers {
        Modifiers {
            control: true,
            shift: true,
            platform: true,
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
            // As gpui_macos reports them: the shifted character, `shift`
            // cleared.
            (event(">", None, cmd()), "shift+cmd+."),
            (event("-", None, ctrl()), "ctrl+-"),
            (event("_", None, ctrl()), "ctrl+shift+-"),
            // Open Recent, Open…, and the second stroke of `cmd+k f`.
            (event("r", None, ctrl()), "ctrl+r"),
            (event("r", None, ctrl_shift()), "ctrl+shift+r"),
            (event("o", None, cmd()), "cmd+o"),
            (event("f", Some("f"), Modifiers::none()), "f"),
        ];
        // Cursor movement.
        for (key, plain, shifted) in [
            ("left", "left", "shift+left"),
            ("right", "right", "shift+right"),
            ("up", "up", "shift+up"),
            ("down", "down", "shift+down"),
            ("home", "home", "shift+home"),
            ("end", "end", "shift+end"),
            ("pageup", "pageup", "shift+pageup"),
            ("pagedown", "pagedown", "shift+pagedown"),
        ] {
            rows.push((event(key, None, Modifiers::none()), plain));
            rows.push((event(key, None, Modifiers::shift()), shifted));
        }
        for (key, with_cmd, with_cmd_shift) in [
            ("left", "cmd+left", "shift+cmd+left"),
            ("right", "cmd+right", "shift+cmd+right"),
            ("up", "cmd+up", "shift+cmd+up"),
            ("down", "cmd+down", "shift+cmd+down"),
        ] {
            rows.push((event(key, None, cmd()), with_cmd));
            rows.push((event(key, None, cmd_shift()), with_cmd_shift));
        }
        for (key, with_alt, with_alt_shift) in [
            ("left", "alt+left", "shift+alt+left"),
            ("right", "alt+right", "shift+alt+right"),
        ] {
            rows.push((event(key, None, alt()), with_alt));
            rows.push((event(key, None, alt_shift()), with_alt_shift));
        }
        for letter in ["a", "b", "e", "f", "n", "p"] {
            let binding: &'static str = match letter {
                "a" => "ctrl+a",
                "b" => "ctrl+b",
                "e" => "ctrl+e",
                "f" => "ctrl+f",
                "n" => "ctrl+n",
                _ => "ctrl+p",
            };
            rows.push((event(letter, None, ctrl()), binding));
        }
        rows.push((event("a", None, cmd()), "cmd+a"));
        // Editing.
        rows.extend([
            (event("z", None, cmd()), "cmd+z"),
            (event("z", None, cmd_shift()), "shift+cmd+z"),
            (event("s", None, cmd()), "cmd+s"),
            (event("s", None, alt_cmd()), "alt+cmd+s"),
            // The second stroke of `cmd+k s`.
            (event("s", Some("s"), Modifiers::none()), "s"),
            (event("u", None, cmd()), "cmd+u"),
            (event("x", None, cmd()), "cmd+x"),
            (event("v", None, cmd()), "cmd+v"),
            (event("backspace", None, Modifiers::none()), "backspace"),
            (
                event("backspace", None, Modifiers::shift()),
                "shift+backspace",
            ),
            (event("backspace", None, alt()), "alt+backspace"),
            (event("backspace", None, cmd()), "cmd+backspace"),
            (event("delete", None, Modifiers::none()), "delete"),
            (event("delete", None, alt()), "alt+delete"),
            (event("delete", None, cmd()), "cmd+delete"),
            (event("h", None, ctrl()), "ctrl+h"),
            (event("d", None, ctrl()), "ctrl+d"),
            (event("k", None, ctrl()), "ctrl+k"),
            (event("o", None, ctrl()), "ctrl+o"),
            (event("tab", Some("\t"), Modifiers::none()), "tab"),
            (event("tab", None, Modifiers::shift()), "shift+tab"),
            (event("enter", Some("\n"), Modifiers::none()), "enter"),
            (event("/", None, cmd()), "cmd+/"),
            (event("a", Some("Å"), alt_shift()), "shift+alt+a"),
            (event("up", None, alt()), "alt+up"),
            (event("down", None, alt()), "alt+down"),
            (event("up", None, alt_shift()), "shift+alt+up"),
            (event("down", None, alt_shift()), "shift+alt+down"),
            (event("k", None, cmd_shift()), "cmd+shift+k"),
            (event("enter", Some("\n"), cmd()), "cmd+enter"),
            (event("enter", Some("\n"), cmd_shift()), "cmd+shift+enter"),
            (event("]", None, cmd()), "cmd+]"),
            (event("[", None, cmd()), "cmd+["),
            (event("j", None, ctrl()), "ctrl+j"),
            (
                event("right", None, ctrl_shift_cmd()),
                "ctrl+shift+cmd+right",
            ),
            (event("left", None, ctrl_shift_cmd()), "ctrl+shift+cmd+left"),
        ]);
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
        // Terminal. macOS reports ⌃⇧` as ⌃ + `~`, ⌘⇧] as ⌘ + `}`.
        rows.extend([
            (event("`", None, ctrl()), "ctrl+`"),
            (event("~", None, ctrl()), "ctrl+shift+`"),
            (event("j", None, cmd()), "cmd+j"),
            (event("}", None, cmd()), "cmd+shift+]"),
            (event("{", None, cmd()), "cmd+shift+["),
            (event("pageup", None, alt_cmd()), "alt+cmd+pageup"),
            (event("pagedown", None, alt_cmd()), "alt+cmd+pagedown"),
            (event("home", None, cmd()), "cmd+home"),
            (event("end", None, cmd()), "cmd+end"),
            (event("c", None, ctrl_shift()), "ctrl+shift+c"),
            (event("v", None, ctrl_shift()), "ctrl+shift+v"),
        ]);
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
    fn shift_with_symbol_keys_matches_either_event_shape() {
        // macOS: shifted character, no shift flag.
        let mac = to_keystroke(&event("_", None, ctrl()));
        // Platforms that keep the key and the flag (and gpui's test platform).
        let kept = to_keystroke(&event("-", None, ctrl_shift()));
        let expected = parse_keys("ctrl+shift+-").unwrap().remove(0);
        assert_eq!(mac, expected);
        assert_eq!(kept, expected);
        let expected = parse_keys("shift+cmd+.").unwrap().remove(0);
        assert_eq!(to_keystroke(&event(">", None, cmd())), expected);
        assert_eq!(to_keystroke(&event(".", None, cmd_shift())), expected);
        // Without a chord modifier the character is left alone.
        assert_eq!(
            to_keystroke(&event("_", Some("_"), Modifiers::none())).key,
            "_"
        );
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
