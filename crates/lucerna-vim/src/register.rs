//! Registers and the system clipboard rules (spec §6.2).

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use crate::Action;

/// `vim.useSystemClipboard`: `"onYank"`, `true` or `false`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClipboardMode {
    #[default]
    OnYank,
    Always,
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegKind {
    Charwise,
    Linewise,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegContent {
    pub text: String,
    pub kind: RegKind,
}

/// The system clipboard as the workbench read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clipboard {
    pub text: String,
    /// The editor copied a whole line (a copy with no selection).
    pub whole_line: bool,
}

/// Which command wrote: `Delete` covers `d c x X s S C D`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WriteKind {
    Yank,
    Delete,
}

fn hash(text: &str) -> u64 {
    let mut h = DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

/// The internal registers: `"`, `0`-`9`, `-` and `a`-`z`. The clipboard
/// (`+`, `*`), `%` and `/` are not stored here; `.` is set by `set_dot`.
#[derive(Debug, Default)]
pub(crate) struct Registers {
    regs: HashMap<char, RegContent>,
    /// Hash of the clipboard text at the last write to `"`, to tell whether
    /// the clipboard changed since.
    fingerprint: Option<u64>,
    /// The text of the last linewise write, to type a clipboard that
    /// holds it again.
    last_linewise: Option<String>,
    dot: Option<String>,
}

impl Registers {
    /// Whether `name` is a register vim accepts after `"`.
    pub(crate) fn is_valid_name(c: char) -> bool {
        c.is_ascii_alphanumeric()
            || matches!(c, '"' | '-' | '_' | '+' | '*' | '.' | '%' | ':' | '/')
    }

    /// Sets the `.` register: the last inserted text.
    pub(crate) fn set_dot(&mut self, text: String) {
        self.dot = Some(text);
    }

    /// Stores `content` per the write rules. Returns the clipboard write
    /// that is due, if any. Read-only names and `_` write nothing.
    pub(crate) fn write(
        &mut self,
        name: Option<char>,
        content: RegContent,
        kind: WriteKind,
        mode: ClipboardMode,
        clipboard: Option<&Clipboard>,
    ) -> Option<Action> {
        let mut content = content;
        let mut to_clipboard = false;
        match name {
            Some('_' | '.' | '%' | ':' | '/') => return None,
            None | Some('"') => {}
            Some('+' | '*') => to_clipboard = true,
            Some(c) if c.is_ascii_uppercase() => {
                let low = c.to_ascii_lowercase();
                if let Some(old) = self.regs.get(&low) {
                    // A linewise append starts on a new line.
                    let linewise = content.kind == RegKind::Linewise;
                    let sep = if linewise && !old.text.ends_with('\n') {
                        "\n"
                    } else {
                        ""
                    };
                    content.text = format!("{}{sep}{}", old.text, content.text);
                }
                self.regs.insert(low, content.clone());
            }
            Some(c) if c.is_ascii_lowercase() => {
                self.regs.insert(c, content.clone());
            }
            Some(c) if c.is_ascii_digit() || c == '-' => {
                self.regs.insert(c, content.clone());
            }
            Some(_) => return None,
        }
        if matches!(name, None | Some('"')) {
            match kind {
                WriteKind::Yank => {
                    self.regs.insert('0', content.clone());
                }
                WriteKind::Delete => {
                    if content.kind == RegKind::Linewise || content.text.contains('\n') {
                        for i in (1..9u8).rev() {
                            let from = (b'0' + i) as char;
                            if let Some(c) = self.regs.remove(&from) {
                                self.regs.insert((from as u8 + 1) as char, c);
                            }
                        }
                        self.regs.insert('1', content.clone());
                    } else {
                        self.regs.insert('-', content.clone());
                    }
                }
            }
        }
        // Every write reaches `"`, so the mode decides the clipboard too.
        to_clipboard |= match mode {
            ClipboardMode::Always => true,
            ClipboardMode::OnYank => kind == WriteKind::Yank,
            ClipboardMode::Never => false,
        };
        if content.kind == RegKind::Linewise {
            self.last_linewise = Some(content.text.clone());
        }
        self.regs.insert('"', content.clone());
        self.fingerprint = if to_clipboard {
            Some(hash(&content.text))
        } else {
            clipboard.map(|c| hash(&c.text))
        };
        to_clipboard.then(|| Action::WriteClipboard {
            linewise: content.kind == RegKind::Linewise,
            text: content.text,
        })
    }

    /// The content of register `name` (`None` is `"`). `%` and `/` live
    /// outside the registers: see `VimGlobals::read_register`.
    pub(crate) fn read(
        &self,
        name: Option<char>,
        mode: ClipboardMode,
        clipboard: Option<&Clipboard>,
    ) -> Option<RegContent> {
        match name {
            None | Some('"') => {
                let use_clipboard = match mode {
                    ClipboardMode::Never => false,
                    ClipboardMode::Always => true,
                    ClipboardMode::OnYank => {
                        clipboard.is_some_and(|c| Some(hash(&c.text)) != self.fingerprint)
                    }
                };
                if use_clipboard && let Some(c) = clipboard {
                    return Some(self.clipboard_content(c));
                }
                self.regs.get(&'"').cloned()
            }
            Some('+' | '*') => clipboard.map(|c| self.clipboard_content(c)),
            Some('.') => self.dot.clone().map(|text| RegContent {
                text,
                kind: RegKind::Charwise,
            }),
            Some('_' | ':' | '%' | '/') => None,
            Some(c) => self.regs.get(&c.to_ascii_lowercase()).cloned(),
        }
    }

    fn clipboard_content(&self, c: &Clipboard) -> RegContent {
        let linewise = c.whole_line || self.last_linewise.as_deref() == Some(c.text.as_str());
        RegContent {
            text: c.text.clone(),
            kind: if linewise {
                RegKind::Linewise
            } else {
                RegKind::Charwise
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn content(text: &str, kind: RegKind) -> RegContent {
        RegContent {
            text: text.into(),
            kind,
        }
    }

    fn cw(text: &str) -> RegContent {
        content(text, RegKind::Charwise)
    }

    fn lw(text: &str) -> RegContent {
        content(text, RegKind::Linewise)
    }

    fn clip(text: &str, whole_line: bool) -> Clipboard {
        Clipboard {
            text: text.into(),
            whole_line,
        }
    }

    fn get(r: &Registers, name: char) -> Option<RegContent> {
        r.read(Some(name), ClipboardMode::Never, None)
    }

    use ClipboardMode::{Always, Never, OnYank};
    use WriteKind::{Delete, Yank};

    #[test]
    fn yank_writes_unnamed_and_zero_and_clipboard() {
        let mut r = Registers::default();
        let a = r.write(None, cw("abc"), Yank, OnYank, None);
        assert_eq!(
            a,
            Some(Action::WriteClipboard {
                text: "abc".into(),
                linewise: false
            })
        );
        assert_eq!(get(&r, '"'), Some(cw("abc")));
        assert_eq!(get(&r, '0'), Some(cw("abc")));
    }

    #[test]
    fn delete_after_external_copy_reads_the_delete() {
        let mut r = Registers::default();
        r.write(None, lw("A\n"), Yank, OnYank, None);
        // The clipboard was changed to B before the delete.
        let b = clip("B", false);
        r.write(None, lw("C\n"), Delete, OnYank, Some(&b));
        assert_eq!(r.read(None, OnYank, Some(&b)), Some(lw("C\n")));
    }

    #[test]
    fn external_copy_after_yank_wins() {
        let mut r = Registers::default();
        r.write(None, cw("A"), Yank, OnYank, None);
        let b = clip("B", false);
        assert_eq!(r.read(None, OnYank, Some(&b)), Some(cw("B")));
        // Unchanged clipboard (what we wrote): the internal register.
        let a = clip("A", false);
        assert_eq!(r.read(None, OnYank, Some(&a)), Some(cw("A")));
    }

    #[test]
    fn never_mode_does_not_touch_the_clipboard() {
        let mut r = Registers::default();
        assert_eq!(r.write(None, cw("abc"), Yank, Never, None), None);
        let b = clip("B", false);
        assert_eq!(r.read(None, Never, Some(&b)), Some(cw("abc")));
        let a = r.write(Some('+'), cw("p"), Yank, Never, None);
        assert!(matches!(a, Some(Action::WriteClipboard { .. })));
    }

    #[test]
    fn always_mode_writes_clipboard_on_delete_and_reads_it() {
        let mut r = Registers::default();
        let a = r.write(None, cw("x"), Delete, Always, None);
        assert_eq!(
            a,
            Some(Action::WriteClipboard {
                text: "x".into(),
                linewise: false
            })
        );
        let b = clip("B", false);
        assert_eq!(r.read(None, Always, Some(&b)), Some(cw("B")));
        assert_eq!(r.read(None, Always, None), Some(cw("x")));
    }

    #[test]
    fn numbered_registers_shift() {
        let mut r = Registers::default();
        for t in ["1\n", "2\n", "3\n"] {
            r.write(None, lw(t), Delete, Never, None);
        }
        assert_eq!(get(&r, '1'), Some(lw("3\n")));
        assert_eq!(get(&r, '2'), Some(lw("2\n")));
        assert_eq!(get(&r, '3'), Some(lw("1\n")));
    }

    #[test]
    fn numbered_ring_drops_the_oldest() {
        let mut r = Registers::default();
        for i in 1..=10 {
            r.write(None, lw(&format!("{i}\n")), Delete, Never, None);
        }
        assert_eq!(get(&r, '1'), Some(lw("10\n")));
        assert_eq!(get(&r, '9'), Some(lw("2\n")));
    }

    #[test]
    fn small_delete_goes_to_minus() {
        let mut r = Registers::default();
        r.write(None, lw("old\n"), Delete, Never, None);
        r.write(None, cw("ab"), Delete, Never, None);
        assert_eq!(get(&r, '-'), Some(cw("ab")));
        assert_eq!(get(&r, '1'), Some(lw("old\n")));
        assert_eq!(get(&r, '"'), Some(cw("ab")));
    }

    #[test]
    fn charwise_delete_with_newline_goes_to_numbered() {
        let mut r = Registers::default();
        r.write(None, cw("a\nb"), Delete, Never, None);
        assert_eq!(get(&r, '1'), Some(cw("a\nb")));
        assert_eq!(get(&r, '-'), None);
    }

    // Adapted from VSCodeVim test/register/register.test.ts: "A appends linewise text to "a"
    #[test]
    fn uppercase_appends_linewise() {
        let mut r = Registers::default();
        r.write(Some('a'), cw("tes"), Yank, Never, None);
        r.write(Some('A'), lw("test2\n"), Yank, Never, None);
        assert_eq!(get(&r, 'a'), Some(lw("tes\ntest2\n")));
        assert_eq!(get(&r, '"'), Some(lw("tes\ntest2\n")));
        assert_eq!(get(&r, '0'), None);
    }

    #[test]
    fn uppercase_linewise_after_linewise_adds_no_blank_line() {
        let mut r = Registers::default();
        r.write(Some('a'), lw("a\n"), Yank, Never, None);
        r.write(Some('A'), lw("b\n"), Yank, Never, None);
        assert_eq!(get(&r, 'a'), Some(lw("a\nb\n")));
    }

    #[test]
    fn uppercase_charwise_append_joins_directly() {
        let mut r = Registers::default();
        r.write(Some('a'), cw("ab"), Yank, Never, None);
        r.write(Some('A'), cw("cd"), Yank, Never, None);
        assert_eq!(get(&r, 'a'), Some(cw("abcd")));
    }

    #[test]
    fn black_hole_writes_nothing() {
        let mut r = Registers::default();
        r.write(None, cw("keep"), Yank, Never, None);
        assert_eq!(r.write(Some('_'), cw("gone"), Delete, Always, None), None);
        assert_eq!(get(&r, '"'), Some(cw("keep")));
    }

    // Adapted from VSCodeVim test/register/register.test.ts: "Small delete register is not set when another register is specified"
    #[test]
    fn named_delete_skips_small_and_numbered() {
        let mut r = Registers::default();
        r.write(None, lw("one\n"), Delete, Never, None);
        r.write(Some('x'), cw("ab"), Delete, Never, None);
        assert_eq!(get(&r, 'x'), Some(cw("ab")));
        assert_eq!(get(&r, '"'), Some(cw("ab")));
        assert_eq!(get(&r, '-'), None);
        assert_eq!(get(&r, '1'), Some(lw("one\n")));
        r.write(Some('y'), lw("two\n"), Delete, Never, None);
        assert_eq!(get(&r, '1'), Some(lw("one\n")));
        assert_eq!(get(&r, '2'), None);
    }

    #[test]
    fn whole_line_clipboard_reads_linewise() {
        let mut r = Registers::default();
        r.write(None, cw("x"), Yank, OnYank, None);
        let c = clip("foo\n", true);
        assert_eq!(r.read(None, OnYank, Some(&c)), Some(lw("foo\n")));
    }

    #[test]
    fn clipboard_matching_last_linewise_write_reads_linewise() {
        let mut r = Registers::default();
        r.write(None, lw("foo\n"), Yank, OnYank, None);
        r.write(None, cw("x"), Yank, OnYank, None);
        let c = clip("foo\n", false);
        assert_eq!(r.read(Some('+'), OnYank, Some(&c)), Some(lw("foo\n")));
        let d = clip("bar", false);
        assert_eq!(r.read(Some('*'), OnYank, Some(&d)), Some(cw("bar")));
    }

    #[test]
    fn dot_and_readonly() {
        let mut r = Registers::default();
        assert_eq!(r.write(Some('.'), cw("no"), Yank, Always, None), None);
        assert_eq!(get(&r, '.'), None);
        r.set_dot("typed".into());
        assert_eq!(get(&r, '.'), Some(cw("typed")));
        assert_eq!(get(&r, ':'), None);
    }
}
