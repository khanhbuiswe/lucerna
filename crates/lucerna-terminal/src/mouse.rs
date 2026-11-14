//! Mouse reports (xterm ctlseqs "Mouse Tracking") and the alternate-scroll
//! arrow keys.

use crate::Modes;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButtonKind {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
    /// No button held: pure motion.
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseAction {
    Press,
    Release,
    Motion,
}

/// The report for a mouse event at 0-based `row`/`col`, or `None` when the
/// application did not ask for it (or the position does not fit the encoding).
#[allow(clippy::too_many_arguments)]
pub fn mouse_report(
    button: MouseButtonKind,
    action: MouseAction,
    row: u16,
    col: u16,
    shift: bool,
    alt: bool,
    ctrl: bool,
    mode: Modes,
) -> Option<Vec<u8>> {
    if !mode.intersects(Modes::MOUSE_MODE) {
        return None;
    }
    let wheel = matches!(
        button,
        MouseButtonKind::WheelUp | MouseButtonKind::WheelDown
    );
    let base = match button {
        MouseButtonKind::Left => 0,
        MouseButtonKind::Middle => 1,
        MouseButtonKind::Right => 2,
        MouseButtonKind::WheelUp => 64,
        MouseButtonKind::WheelDown => 65,
        MouseButtonKind::None => 3,
    };
    let sgr = mode.contains(Modes::SGR_MOUSE);
    let mut code = match action {
        MouseAction::Press => base,
        MouseAction::Release if wheel => return None,
        // Legacy encodings cannot say which button was released.
        MouseAction::Release if sgr => base,
        MouseAction::Release => 3,
        MouseAction::Motion => {
            let wanted = if button == MouseButtonKind::None {
                Modes::MOUSE_MOTION
            } else {
                Modes::MOUSE_DRAG | Modes::MOUSE_MOTION
            };
            if wheel || !mode.intersects(wanted) {
                return None;
            }
            base + 32
        }
    };
    code += 4 * u8::from(shift) + 8 * u8::from(alt) + 16 * u8::from(ctrl);

    let (x, y) = (u32::from(col) + 1, u32::from(row) + 1);
    if sgr {
        let end = if action == MouseAction::Release {
            'm'
        } else {
            'M'
        };
        return Some(format!("\x1b[<{code};{x};{y}{end}").into_bytes());
    }

    let mut out = vec![0x1b, b'[', b'M', 32 + code];
    let utf8 = mode.contains(Modes::UTF8_MOUSE);
    for v in [x, y] {
        let limit = if utf8 { 2015 } else { 223 };
        if v > limit {
            return None;
        }
        let v = v + 32;
        if utf8 && v >= 128 {
            let c = char::from_u32(v)?;
            out.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
        } else {
            out.push(v as u8);
        }
    }
    Some(out)
}

/// Arrow keys standing in for the wheel on the alternate screen: negative
/// `lines` scroll up, positive down. Only when the application has not taken
/// the mouse itself.
pub fn alternate_scroll(lines: i32, mode: Modes) -> Option<Vec<u8>> {
    if lines == 0
        || !mode.contains(Modes::ALT_SCREEN | Modes::ALTERNATE_SCROLL)
        || mode.intersects(Modes::MOUSE_MODE)
    {
        return None;
    }
    let arrow: &[u8] = match (lines < 0, mode.contains(Modes::APP_CURSOR)) {
        (true, false) => b"\x1b[A",
        (true, true) => b"\x1bOA",
        (false, false) => b"\x1b[B",
        (false, true) => b"\x1bOB",
    };
    Some(arrow.repeat(lines.unsigned_abs() as usize))
}

#[cfg(test)]
mod tests {
    use super::*;
    use MouseAction::*;
    use MouseButtonKind as B;

    fn report(b: B, a: MouseAction, row: u16, col: u16, mode: Modes) -> Option<Vec<u8>> {
        mouse_report(b, a, row, col, false, false, false, mode)
    }
    fn s(b: Option<Vec<u8>>) -> String {
        String::from_utf8_lossy(&b.expect("bytes")).into_owned()
    }

    #[test]
    fn sgr_mouse_press_release() {
        let m = Modes::MOUSE_REPORT_CLICK | Modes::SGR_MOUSE;
        assert_eq!(s(report(B::Left, Press, 4, 9, m)), "\x1b[<0;10;5M");
        assert_eq!(s(report(B::Left, Release, 4, 9, m)), "\x1b[<0;10;5m");
        assert_eq!(s(report(B::Right, Press, 0, 0, m)), "\x1b[<2;1;1M");
        assert_eq!(s(report(B::WheelUp, Press, 0, 0, m)), "\x1b[<64;1;1M");
        assert_eq!(s(report(B::WheelDown, Press, 0, 0, m)), "\x1b[<65;1;1M");
        assert_eq!(report(B::WheelDown, Release, 0, 0, m), None);
    }

    #[test]
    fn sgr_modifiers() {
        let m = Modes::MOUSE_REPORT_CLICK | Modes::SGR_MOUSE;
        let r = mouse_report(B::Left, Press, 0, 0, true, true, true, m);
        assert_eq!(s(r), "\x1b[<28;1;1M");
    }

    #[test]
    fn normal_mouse_encoding() {
        let m = Modes::MOUSE_REPORT_CLICK;
        assert_eq!(
            report(B::Left, Press, 4, 9, m),
            Some(vec![0x1b, b'[', b'M', 32, 42, 37])
        );
        assert_eq!(
            report(B::Left, Release, 4, 9, m),
            Some(vec![0x1b, b'[', b'M', 35, 42, 37])
        );
        // col 222 -> 223 on the wire: the largest that fits a byte.
        assert!(report(B::Left, Press, 0, 222, m).is_some());
        assert_eq!(report(B::Left, Press, 0, 223, m), None);
        assert_eq!(report(B::Left, Press, 223, 0, m), None);
    }

    #[test]
    fn utf8_mouse_extends_range() {
        let m = Modes::MOUSE_REPORT_CLICK | Modes::UTF8_MOUSE;
        // col 299 -> 300 + 32 = 332 = U+014C -> 0xC5 0x8C.
        assert_eq!(
            report(B::Left, Press, 0, 299, m),
            Some(vec![0x1b, b'[', b'M', 32, 0xC5, 0x8C, 33])
        );
        assert_eq!(report(B::Left, Press, 0, 2100, m), None);
    }

    #[test]
    fn no_report_without_mouse_mode() {
        let m = Modes::SGR_MOUSE | Modes::ALT_SCREEN;
        assert_eq!(report(B::Left, Press, 0, 0, m), None);
        assert_eq!(report(B::None, Motion, 0, 0, m), None);
    }

    #[test]
    fn motion_only_with_drag_or_motion_mode() {
        let click = Modes::MOUSE_REPORT_CLICK | Modes::SGR_MOUSE;
        let drag = Modes::MOUSE_DRAG | Modes::SGR_MOUSE;
        let any = Modes::MOUSE_MOTION | Modes::SGR_MOUSE;
        assert_eq!(report(B::Left, Motion, 1, 1, click), None);
        assert_eq!(s(report(B::Left, Motion, 1, 1, drag)), "\x1b[<32;2;2M");
        assert_eq!(report(B::None, Motion, 1, 1, drag), None);
        assert_eq!(s(report(B::None, Motion, 1, 1, any)), "\x1b[<35;2;2M");
        assert_eq!(s(report(B::Left, Motion, 1, 1, any)), "\x1b[<32;2;2M");
    }

    #[test]
    fn wheel_in_alt_screen_sends_arrows() {
        let m = Modes::ALT_SCREEN | Modes::ALTERNATE_SCROLL;
        assert_eq!(s(alternate_scroll(-3, m)), "\x1b[A\x1b[A\x1b[A");
        assert_eq!(s(alternate_scroll(2, m)), "\x1b[B\x1b[B");
        assert_eq!(s(alternate_scroll(-1, m | Modes::APP_CURSOR)), "\x1bOA");
        assert_eq!(alternate_scroll(0, m), None);
    }

    #[test]
    fn alternate_scroll_needs_alt_screen_and_no_mouse_mode() {
        assert_eq!(alternate_scroll(-1, Modes::ALTERNATE_SCROLL), None);
        assert_eq!(alternate_scroll(-1, Modes::ALT_SCREEN), None);
        let m = Modes::ALT_SCREEN | Modes::ALTERNATE_SCROLL | Modes::MOUSE_REPORT_CLICK;
        assert_eq!(alternate_scroll(-1, m), None);
    }
}
