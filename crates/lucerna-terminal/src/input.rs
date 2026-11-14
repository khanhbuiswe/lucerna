//! Key presses to the bytes a terminal application expects (xterm
//! conventions, ctlseqs "PC-Style Function Keys" and "Alt and Meta Keys").

use crate::Modes;

/// One key press as the workbench sees it.
#[derive(Debug, Clone, Copy)]
pub struct KeyInput<'a> {
    /// gpui key name: "a", "enter", "left", "f5", "pageup"...
    pub key: &'a str,
    /// What the keyboard layout produced for this press (gpui `key_char`).
    pub text: Option<&'a str>,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub cmd: bool,
}

/// Bytes to write to the PTY for `k`, or `None` when the key is not for the
/// terminal (anything with cmd, or a key with no encoding).
///
/// With `option_is_meta` an alt press sends ESC plus the unmodified key;
/// without it, the layout's own text (e.g. "∫" for option+b on macOS).
pub fn key_to_bytes(k: &KeyInput, mode: Modes, option_is_meta: bool) -> Option<Vec<u8>> {
    if k.cmd {
        return None;
    }
    let modifier = 1 + u8::from(k.shift) + 2 * u8::from(k.alt) + 4 * u8::from(k.ctrl);

    if let Some(bytes) = special_key(k.key, modifier, mode) {
        return Some(bytes);
    }

    let mut out = Vec::new();
    let mut named = true;
    match k.key {
        "enter" => out.push(b'\r'),
        "tab" if k.shift => return Some(b"\x1b[Z".to_vec()),
        "tab" => out.push(b'\t'),
        "backspace" => out.push(if k.ctrl { 0x08 } else { 0x7f }),
        "escape" => out.push(0x1b),
        "space" if k.ctrl => out.push(0),
        "space" => out.push(b' '),
        key => {
            named = false;
            let mut chars = key.chars();
            let (Some(c), None) = (chars.next(), chars.next()) else {
                return None;
            };
            if k.ctrl {
                out.push(ctrl_byte(c)?);
            } else if k.alt && !option_is_meta {
                out.extend_from_slice(layout_text(k)?.as_bytes());
                return Some(out);
            } else if k.alt {
                let c = if k.shift { c.to_ascii_uppercase() } else { c };
                out.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
            } else {
                out.extend_from_slice(layout_text(k)?.as_bytes());
                return Some(out);
            }
        }
    }
    // Named keys have no layout character to keep, so alt always means ESC;
    // for characters only meta mode (or ctrl) does.
    if k.alt && (named || option_is_meta || k.ctrl) {
        out.insert(0, 0x1b);
    }
    Some(out)
}

/// The text the key produced, falling back to the key name for a single
/// character (upper-cased with shift).
fn layout_text<'a>(k: &KeyInput<'a>) -> Option<std::borrow::Cow<'a, str>> {
    match k.text {
        Some("") => None,
        Some(t) => Some(t.into()),
        None if k.shift => Some(k.key.to_uppercase().into()),
        None => Some(k.key.into()),
    }
}

/// The C0 control character for ctrl+`c`, as xterm sends it.
fn ctrl_byte(c: char) -> Option<u8> {
    Some(match c.to_ascii_lowercase() {
        c @ 'a'..='z' => c as u8 - b'a' + 1,
        // X's control mapping (`c & 0x1f`) for the backtick key: ⌃` is NUL
        // and ⌃~ is RS.
        '@' | '2' | '`' => 0x00,
        '~' => 0x1e,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '^' | '6' => 0x1e,
        '_' | '-' | '/' | '7' => 0x1f,
        '8' => 0x7f,
        _ => return None,
    })
}

/// Cursor, editing and function keys. `modifier` is the xterm parameter
/// (1 = none, 2 = shift, 3 = alt, 5 = ctrl, sums for combinations).
fn special_key(key: &str, modifier: u8, mode: Modes) -> Option<Vec<u8>> {
    // Final letter keys: CSI/SS3 <letter>, or CSI 1;m <letter> when modified.
    let letter = match key {
        "up" => Some('A'),
        "down" => Some('B'),
        "right" => Some('C'),
        "left" => Some('D'),
        "home" => Some('H'),
        "end" => Some('F'),
        "f1" => Some('P'),
        "f2" => Some('Q'),
        "f3" => Some('R'),
        "f4" => Some('S'),
        _ => None,
    };
    if let Some(l) = letter {
        let is_fkey = key.starts_with('f');
        return Some(
            if modifier > 1 {
                format!("\x1b[1;{modifier}{l}")
            } else if is_fkey || mode.contains(Modes::APP_CURSOR) {
                format!("\x1bO{l}")
            } else {
                format!("\x1b[{l}")
            }
            .into_bytes(),
        );
    }
    let number = match key {
        "insert" => 2,
        "delete" => 3,
        "pageup" => 5,
        "pagedown" => 6,
        "f5" => 15,
        "f6" => 17,
        "f7" => 18,
        "f8" => 19,
        "f9" => 20,
        "f10" => 21,
        "f11" => 23,
        "f12" => 24,
        "f13" => 25,
        "f14" => 26,
        "f15" => 28,
        "f16" => 29,
        "f17" => 31,
        "f18" => 32,
        "f19" => 33,
        "f20" => 34,
        _ => return None,
    };
    Some(
        if modifier > 1 {
            format!("\x1b[{number};{modifier}~")
        } else {
            format!("\x1b[{number}~")
        }
        .into_bytes(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(name: &str) -> KeyInput<'_> {
        KeyInput {
            key: name,
            text: None,
            ctrl: false,
            alt: false,
            shift: false,
            cmd: false,
        }
    }
    fn enc(k: KeyInput) -> Option<Vec<u8>> {
        key_to_bytes(&k, Modes::empty(), false)
    }
    fn s(b: Option<Vec<u8>>) -> String {
        String::from_utf8(b.expect("bytes")).unwrap()
    }

    #[test]
    fn cmd_keys_never_reach_pty() {
        assert_eq!(
            enc(KeyInput {
                cmd: true,
                text: Some("a"),
                ..key("a")
            }),
            None
        );
        assert_eq!(
            enc(KeyInput {
                cmd: true,
                ..key("left")
            }),
            None
        );
    }

    #[test]
    fn plain_text_is_utf8() {
        let k = KeyInput {
            text: Some("ệ"),
            ..key("e")
        };
        assert_eq!(enc(k), Some("ệ".as_bytes().to_vec()));
        let k = KeyInput {
            text: Some("a"),
            ..key("a")
        };
        assert_eq!(enc(k), Some(b"a".to_vec()));
    }

    #[test]
    fn enter_tab_backspace_escape() {
        assert_eq!(s(enc(key("enter"))), "\r");
        assert_eq!(s(enc(key("tab"))), "\t");
        assert_eq!(s(enc(key("backspace"))), "\x7f");
        assert_eq!(s(enc(key("escape"))), "\x1b");
        assert_eq!(
            s(enc(KeyInput {
                shift: true,
                ..key("tab")
            })),
            "\x1b[Z"
        );
        assert_eq!(s(enc(key("space"))), " ");
    }

    #[test]
    fn ctrl_letters() {
        assert_eq!(
            s(enc(KeyInput {
                ctrl: true,
                ..key("c")
            })),
            "\x03"
        );
        assert_eq!(
            s(enc(KeyInput {
                ctrl: true,
                ..key("a")
            })),
            "\x01"
        );
        assert_eq!(
            s(enc(KeyInput {
                ctrl: true,
                ..key("z")
            })),
            "\x1a"
        );
        assert_eq!(
            s(enc(KeyInput {
                ctrl: true,
                ..key("space")
            })),
            "\x00"
        );
        assert_eq!(
            s(enc(KeyInput {
                ctrl: true,
                ..key("[")
            })),
            "\x1b"
        );
        assert_eq!(
            s(enc(KeyInput {
                ctrl: true,
                ..key("\\")
            })),
            "\x1c"
        );
        assert_eq!(
            s(enc(KeyInput {
                ctrl: true,
                ..key("]")
            })),
            "\x1d"
        );
        assert_eq!(
            s(enc(KeyInput {
                ctrl: true,
                ..key("/")
            })),
            "\x1f"
        );
        assert_eq!(
            s(enc(KeyInput {
                ctrl: true,
                ..key("`")
            })),
            "\x00"
        );
        assert_eq!(
            s(enc(KeyInput {
                ctrl: true,
                shift: true,
                ..key("~")
            })),
            "\x1e"
        );
    }

    #[test]
    fn arrows_respect_app_cursor() {
        assert_eq!(s(enc(key("left"))), "\x1b[D");
        let app = key_to_bytes(&key("left"), Modes::APP_CURSOR, false);
        assert_eq!(s(app), "\x1bOD");
        assert_eq!(s(enc(key("home"))), "\x1b[H");
        assert_eq!(
            s(key_to_bytes(&key("end"), Modes::APP_CURSOR, false)),
            "\x1bOF"
        );
    }

    #[test]
    fn modified_arrows_use_csi_1_m() {
        assert_eq!(
            s(enc(KeyInput {
                shift: true,
                ..key("up")
            })),
            "\x1b[1;2A"
        );
        assert_eq!(
            s(enc(KeyInput {
                ctrl: true,
                ..key("right")
            })),
            "\x1b[1;5C"
        );
        let k = KeyInput {
            alt: true,
            ..key("left")
        };
        assert_eq!(s(key_to_bytes(&k, Modes::empty(), true)), "\x1b[1;3D");
        // Modified keys ignore application cursor mode.
        let k = KeyInput {
            ctrl: true,
            shift: true,
            ..key("down")
        };
        assert_eq!(s(key_to_bytes(&k, Modes::APP_CURSOR, false)), "\x1b[1;6B");
    }

    #[test]
    fn function_keys() {
        assert_eq!(s(enc(key("f1"))), "\x1bOP");
        assert_eq!(s(enc(key("f4"))), "\x1bOS");
        assert_eq!(s(enc(key("f5"))), "\x1b[15~");
        assert_eq!(s(enc(key("f12"))), "\x1b[24~");
        assert_eq!(s(enc(key("f20"))), "\x1b[34~");
        assert_eq!(
            s(enc(KeyInput {
                shift: true,
                ..key("f1")
            })),
            "\x1b[1;2P"
        );
        assert_eq!(
            s(enc(KeyInput {
                ctrl: true,
                ..key("f5")
            })),
            "\x1b[15;5~"
        );
        assert_eq!(s(enc(key("pageup"))), "\x1b[5~");
        assert_eq!(s(enc(key("pagedown"))), "\x1b[6~");
        assert_eq!(s(enc(key("insert"))), "\x1b[2~");
        assert_eq!(s(enc(key("delete"))), "\x1b[3~");
        assert_eq!(
            s(enc(KeyInput {
                shift: true,
                ..key("delete")
            })),
            "\x1b[3;2~"
        );
    }

    #[test]
    fn option_as_meta() {
        let k = KeyInput {
            alt: true,
            text: Some("∫"),
            ..key("b")
        };
        assert_eq!(s(key_to_bytes(&k, Modes::empty(), false)), "∫");
        assert_eq!(s(key_to_bytes(&k, Modes::empty(), true)), "\x1bb");
        let k = KeyInput {
            alt: true,
            shift: true,
            text: Some("Í"),
            ..key("b")
        };
        assert_eq!(s(key_to_bytes(&k, Modes::empty(), true)), "\x1bB");
        let k = KeyInput {
            alt: true,
            ..key("enter")
        };
        assert_eq!(s(key_to_bytes(&k, Modes::empty(), true)), "\x1b\r");
        let k = KeyInput {
            alt: true,
            ctrl: true,
            ..key("c")
        };
        assert_eq!(s(key_to_bytes(&k, Modes::empty(), true)), "\x1b\x03");
    }

    #[test]
    fn alt_on_named_keys_sends_esc_without_option_as_meta() {
        let go = |k: KeyInput| s(key_to_bytes(&k, Modes::empty(), false));
        assert_eq!(
            go(KeyInput {
                alt: true,
                ..key("backspace")
            }),
            "\x1b\x7f"
        );
        assert_eq!(
            go(KeyInput {
                alt: true,
                ..key("enter")
            }),
            "\x1b\r"
        );
        assert_eq!(
            go(KeyInput {
                alt: true,
                ..key("tab")
            }),
            "\x1b\t"
        );
    }

    #[test]
    fn shift_enter_is_plain_cr() {
        assert_eq!(
            s(enc(KeyInput {
                shift: true,
                ..key("enter")
            })),
            "\r"
        );
    }

    #[test]
    fn unknown_keys_give_none() {
        assert_eq!(enc(key("capslock")), None);
        assert_eq!(enc(key("f21")), None);
    }
}
