//! Macros (spec §6.4): `q` records keys and typing as text in Vim key
//! notation, `@` reads a register back as keys and runs them.

use crate::repeat::Typed;
use crate::{Action, Key, Outcome, SearchQuery, Settings, ViewInfo, Vim, VimGlobals};

/// The most keys one `@` returns: a large count of a large register is cut
/// to fit (one run at least).
const MAX_MACRO_KEYS: usize = 1 << 20;

/// What the command line vim keeps while a macro runs is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CmdKind {
    /// `/` or `?`.
    Search { backward: bool },
    /// `:`.
    Ex,
}

/// The command line a running macro types into: `/`, `?` and `:` open it
/// here instead of the find widget or the command palette.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CmdLine {
    pub kind: CmdKind,
    pub text: String,
}

/// Vim key notation: plain chars, `<Esc>` `<CR>` (also `<Enter>`,
/// `<Return>`) `<BS>` `<Tab>` `<lt>` `<C-x>`, names case-insensitive; a
/// `\n` is `<CR>`, a `\t` is `<Tab>`; an unknown `<…>` is literal text.
pub fn parse_notation(text: &str) -> Vec<Key> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        rest = &rest[c.len_utf8()..];
        match c {
            '\n' => out.push(Key::Enter),
            '<' => match rest
                .find('>')
                .and_then(|end| Some((named(&rest[..end])?, end)))
            {
                Some((key, end)) => {
                    out.push(key);
                    rest = &rest[end + 1..];
                }
                None => out.push(Key::Char('<')),
            },
            c => out.push(Key::Char(c)),
        }
    }
    out
}

/// The key `<name>` stands for; `None` when the name is unknown.
fn named(name: &str) -> Option<Key> {
    let lower = name.to_ascii_lowercase();
    Some(match lower.as_str() {
        "esc" => Key::Escape,
        "cr" | "enter" | "return" => Key::Enter,
        "bs" => Key::Backspace,
        "tab" => Key::Char('\t'),
        "lt" => Key::Char('<'),
        _ => {
            let rest = lower.strip_prefix("c-")?;
            let mut chars = rest.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            // A letter is taken in lower case: `<C-R>` is `<C-r>`.
            Key::Ctrl(c)
        }
    })
}

/// Appends `key` in Vim key notation.
pub(crate) fn push_key(out: &mut String, key: Key) {
    match key {
        Key::Char('<') => out.push_str("<lt>"),
        Key::Char('\t') => out.push_str("<Tab>"),
        Key::Char(c) => out.push(c),
        Key::Ctrl(c) => {
            out.push_str("<C-");
            out.push(c);
            out.push('>');
        }
        Key::Escape => out.push_str("<Esc>"),
        Key::Enter => out.push_str("<CR>"),
        Key::Backspace => out.push_str("<BS>"),
    }
}

/// Appends each char of `text` in Vim key notation.
fn push_text(out: &mut String, text: &str) {
    for c in text.chars() {
        push_key(out, Key::Char(c));
    }
}

/// Appends one typing event of an Insert session in Vim key notation.
pub(crate) fn push_typed(out: &mut String, t: &Typed) {
    match t {
        Typed::Text(text) => push_text(out, text),
        Typed::Newline => out.push_str("<CR>"),
        Typed::DeleteLeft => out.push_str("<BS>"),
        Typed::Tab => out.push_str("<Tab>"),
    }
}

impl VimGlobals {
    /// The register a `q` is recording into, as typed (`a` or `A`).
    pub fn recording(&self) -> Option<char> {
        self.record.as_ref().map(|(name, _)| *name)
    }

    /// The workbench sets this while it runs an [`Outcome::Macro`]: the
    /// last change and the `.` register stay as they were, and a delete
    /// of lines does not shift the numbered registers.
    pub fn set_macro_running(&mut self, running: bool) {
        self.macro_running = running;
        self.registers.hold_ring = running;
    }

    pub fn macro_running(&self) -> bool {
        self.macro_running
    }

    /// The workbench sets this while it runs an [`Outcome::Replay`]: the
    /// keys it feeds are not added to the macro being recorded (the key
    /// that returned the replay was).
    pub fn set_replaying(&mut self, replaying: bool) {
        self.replaying = replaying;
    }

    /// Whether what the user does now goes into the macro being recorded.
    fn recording_now(&self) -> bool {
        self.record.is_some() && !self.macro_running && !self.replaying
    }

    /// Appends `text` (already in key notation) to the macro being recorded.
    pub(crate) fn record_text(&mut self, text: &str) {
        if self.recording_now()
            && let Some((_, macro_text)) = &mut self.record
        {
            macro_text.push_str(text);
        }
    }
}

impl Vim {
    /// The workbench calls this when a macro run ends: an unfinished
    /// command line is dropped.
    pub fn macro_ended(&mut self) {
        if let Some(CmdLine {
            kind: CmdKind::Search { .. },
            ..
        }) = self.cmdline.take()
        {
            self.search_dismissed();
        }
    }

    /// After `key` was fed: adds it to the macro being recorded when it
    /// was being recorded before and still is (not the `q` that started
    /// or stopped it). Keys that went on to the keymap are not added, nor
    /// a `/`, `?` or `:` that opened the find widget or the command line:
    /// the search or the command is added when it runs.
    pub(crate) fn record_macro_key(
        key: Key,
        recording_before: bool,
        outcome: &Outcome,
        g: &mut VimGlobals,
    ) {
        if !recording_before || !g.recording_now() {
            return;
        }
        let opened = matches!(outcome, Outcome::Handled(actions) if actions.iter().any(|a| {
            matches!(a, Action::OpenSearch { .. } | Action::OpenCommandLine { .. })
        }));
        if opened || matches!(outcome, Outcome::Unhandled | Outcome::Retry(_)) {
            return;
        }
        let mut text = String::new();
        push_key(&mut text, key);
        g.record_text(&text);
    }

    /// `q{name}`: starts recording into register `name` (`A`–`Z` append).
    pub(crate) fn start_recording(&mut self, name: char, g: &mut VimGlobals) -> Outcome {
        if name.is_ascii_alphabetic() && !g.macro_running {
            g.record = Some((name, String::new()));
        }
        Outcome::Handled(vec![])
    }

    /// `q` without a prefix: stops the recording, storing the macro.
    pub(crate) fn q_key(&mut self, g: &mut VimGlobals) -> Outcome {
        if g.macro_running {
            return Outcome::Handled(vec![]);
        }
        if let Some((name, text)) = g.record.take() {
            g.registers.set_macro(name, text);
        }
        Outcome::Handled(vec![])
    }

    /// `[n]@{c}`: the keys of register `c` (`@` the last one run), `n`
    /// times over.
    pub(crate) fn run_macro(
        &mut self,
        c: char,
        n: u32,
        view: &dyn ViewInfo,
        g: &mut VimGlobals,
        s: &Settings,
    ) -> Outcome {
        let name = if c == '@' {
            match g.last_macro {
                Some(name) => name,
                None => return Outcome::Handled(vec![Action::Failed]),
            }
        } else {
            c
        };
        if !(name.is_ascii_alphanumeric() || matches!(name, '"' | '-' | '+' | '*' | '.')) {
            return Outcome::Handled(vec![
                Action::Message(format!("E354: Invalid register name: '{name}'")),
                Action::Failed,
            ]);
        }
        g.last_macro = Some(name);
        let keys = g
            .read_register(Some(name), s.use_system_clipboard, view.path())
            .map(|content| parse_notation(&content.text))
            .unwrap_or_default();
        if keys.is_empty() {
            return Outcome::Handled(vec![Action::Failed]);
        }
        let times = (n as usize).min(MAX_MACRO_KEYS / keys.len()).max(1);
        Outcome::Macro(keys.repeat(times))
    }

    /// Opens the command line of a running macro.
    pub(crate) fn open_macro_cmdline(&mut self, kind: CmdKind, prefill: String) {
        self.cmdline = Some(CmdLine {
            kind,
            text: prefill,
        });
    }

    /// A key while the command line of a running macro is open.
    pub(crate) fn cmdline_key(
        &mut self,
        key: Key,
        view: &dyn ViewInfo,
        g: &mut VimGlobals,
        s: &Settings,
    ) -> Outcome {
        let Some(cmd) = &mut self.cmdline else {
            return Outcome::Handled(vec![]);
        };
        match key {
            Key::Char(c) => cmd.text.push(c),
            Key::Ctrl('u') => cmd.text.clear(),
            Key::Ctrl(_) => {}
            Key::Backspace => {
                if cmd.text.pop().is_none() {
                    self.cancel_cmdline();
                }
            }
            Key::Escape => self.cancel_cmdline(),
            Key::Enter => {
                let Some(cmd) = self.cmdline.take() else {
                    return Outcome::Handled(vec![]);
                };
                return match cmd.kind {
                    CmdKind::Search { backward } => {
                        let (is_regex, case_sensitive, whole_word) = g
                            .last_search
                            .as_ref()
                            .map_or((false, false, false), |(q, _)| {
                                (q.is_regex, q.case_sensitive, q.whole_word)
                            });
                        let query = SearchQuery {
                            pattern: cmd.text,
                            is_regex,
                            case_sensitive,
                            whole_word,
                        };
                        let origin = self.cursor;
                        self.search_submitted(query, backward, origin, view, g, s)
                    }
                    CmdKind::Ex => self.ex_command(&cmd.text, view, g, s),
                };
            }
        }
        Outcome::Handled(vec![])
    }

    fn cancel_cmdline(&mut self) {
        if let Some(cmd) = self.cmdline.take() {
            match cmd.kind {
                CmdKind::Search { .. } => self.search_dismissed(),
                CmdKind::Ex => {
                    self.clear_pending();
                    self.cmd_keys.clear();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{TestView, assert_edit, assert_edit_with, run_edit};
    use crate::{ClipboardMode, Settings};

    #[test]
    fn parses_notation() {
        assert_eq!(
            parse_notation("a<lt>b<Esc><C-r>\n<tab>"),
            [
                Key::Char('a'),
                Key::Char('<'),
                Key::Char('b'),
                Key::Escape,
                Key::Ctrl('r'),
                Key::Enter,
                Key::Char('\t'),
            ]
        );
        assert_eq!(
            parse_notation("<ENTER><Return><cr><bs><ESC><C-R>"),
            [
                Key::Enter,
                Key::Enter,
                Key::Enter,
                Key::Backspace,
                Key::Escape,
                Key::Ctrl('r')
            ]
        );
        // Unknown names and a lone `<` are literal text.
        assert_eq!(
            parse_notation("<foo>a<"),
            "<foo>a<".chars().map(Key::Char).collect::<Vec<_>>()
        );
        assert_eq!(parse_notation("\t"), [Key::Char('\t')]);
    }

    #[test]
    fn writes_typing_in_notation() {
        let mut out = String::new();
        push_typed(&mut out, &Typed::Text("chào <b>".into()));
        assert_eq!(out, "chào <lt>b>");
        for (t, want) in [
            (Typed::Newline, "<CR>"),
            (Typed::DeleteLeft, "<BS>"),
            (Typed::Tab, "<Tab>"),
        ] {
            let mut out = String::new();
            push_typed(&mut out, &t);
            assert_eq!(out, want);
        }
    }

    #[test]
    fn recorded_keys_parse_back() {
        let keys = [
            Key::Char('a'),
            Key::Char('<'),
            Key::Char('>'),
            Key::Char('\t'),
            Key::Char('ệ'),
            Key::Ctrl('r'),
            Key::Ctrl('u'),
            Key::Escape,
            Key::Enter,
            Key::Backspace,
            Key::Char('<'),
            Key::Char('C'),
            Key::Char('-'),
            Key::Char('x'),
            Key::Char('>'),
        ];
        let mut text = String::new();
        for k in keys {
            push_key(&mut text, k);
        }
        assert_eq!(parse_notation(&text), keys);
    }

    const VARS: &str = "|foo = 1\nbar = 'a'\nfoobar = foo + bar";

    // Adapted from VSCodeVim test/macro.test.ts: "Can record and execute"
    #[test]
    fn record_and_execute() {
        assert_edit(
            VARS,
            "qaA;<Esc>Ivar <Esc>qj@a",
            "var foo = 1;\nvar| bar = 'a';\nfoobar = foo + bar",
        );
    }

    // Adapted from VSCodeVim test/macro.test.ts: "Can repeat last invoked macro"
    #[test]
    fn repeat_last_invoked_macro() {
        assert_edit(
            VARS,
            "qaA;<Esc>Ivar <Esc>qj@aj@@",
            "var foo = 1;\nvar bar = 'a';\nvar| foobar = foo + bar;",
        );
    }

    const PLUS: &str = "|\"(\"+a+\",\"+b+\",\"+c+\",\"+d+\",\"+e+\")\"";
    const SPACED: &str = "\"(\" + a + \",\" + b + \",\" + c + \",\" + d + \",\" + e +| \")\"";

    // Adapted from VSCodeVim test/macro.test.ts: "Can play back with count"
    #[test]
    fn play_back_with_count() {
        assert_edit(PLUS, "f+s + <Esc>qq;.q8@q", SPACED);
    }

    // Adapted from VSCodeVim test/macro.test.ts: "Can play back with count, abort when a motion fails"
    #[test]
    fn play_back_with_count_aborts_when_a_motion_fails() {
        assert_edit(PLUS, "f+s + <Esc>qq;.q22@q", SPACED);
    }

    const LIST: &str = "1. |one\n2. two\n3. three\n4. four";
    const LIST_DONE: &str = "1) One\n2) Two\n3) Three\n4) F|our";

    // Adapted from VSCodeVim test/macro.test.ts: "Repeat change on contiguous lines"
    #[test]
    fn repeat_change_on_contiguous_lines() {
        assert_edit(LIST, "qa0f.r)w~jq3@a", LIST_DONE);
    }

    // Adapted from VSCodeVim test/macro.test.ts: "Append command to a macro"
    #[test]
    fn append_command_to_a_macro() {
        assert_edit(LIST, "qa0f.r)qqAw~jq3@a", LIST_DONE);
    }

    // Adapted from VSCodeVim test/macro.test.ts: "Append command to a not yet created register creates a new register"
    #[test]
    fn append_to_a_new_register_creates_it() {
        assert_edit(LIST, "qB0f.r)w~jq3@b", LIST_DONE);
    }

    // Adapted from VSCodeVim test/macro.test.ts: "Can handle calling an uppercase register"
    #[test]
    fn calling_an_uppercase_register() {
        assert_edit(LIST, "qa0f.r)w~jq3@A", LIST_DONE);
    }

    // Adapted from VSCodeVim test/macro.test.ts: "Can handle calling a non existing macro"
    #[test]
    fn calling_a_non_existing_macro() {
        let (text, actions) = run_edit("1. |one\n2. two", "@x");
        assert_eq!(text, "1. |one\n2. two");
        assert_eq!(actions, [Action::Failed]);
        let (_, actions) = run_edit("|one", "@@");
        assert_eq!(actions, [Action::Failed]);
    }

    // Adapted from VSCodeVim test/macro.test.ts: "Can execute macros with dot commands properly"
    #[test]
    fn macros_with_dot_commands() {
        let start = format!("|{}", ["test"; 7].join("\n"));
        assert_edit(&start, "qadd.q@a@a", "|test");
    }

    // Adapted from VSCodeVim test/macro.test.ts: "Failed `n` stops macro from repeating"
    #[test]
    fn failed_n_stops_the_macro() {
        let s = Settings {
            wrapscan: false,
            ..Settings::default()
        };
        assert_edit_with(
            s,
            |_| {},
            "|one two three\none two three\none two three",
            "/two<CR>0qqnea XXX<Esc>q5@q",
            "one two XXX three\none two XXX three\none two XX|X three",
        );
    }

    // Adapted from VSCodeVim test/macro.test.ts: "q[A-Z] (action) Creates new register, accessible by [a-z]"
    #[test]
    fn uppercase_creates_a_register_read_by_lowercase() {
        assert_edit("|", "qBione two<Esc>qo<Esc>@b", "one two\none tw|o");
    }

    // Adapted from VSCodeVim test/macro.test.ts: "Invalid register throws E354"
    #[test]
    fn invalid_register_throws_e354() {
        let (text, actions) = run_edit("one t|wo three", "@~");
        assert_eq!(text, "one t|wo three");
        assert_eq!(
            actions,
            [
                Action::Message("E354: Invalid register name: '~'".into()),
                Action::Failed
            ]
        );
    }

    #[test]
    fn failed_commands_emit_failed() {
        let fails = |start: &str, keys: &str| {
            let (_, actions) = run_edit(start, keys);
            assert_eq!(
                actions.last(),
                Some(&Action::Failed),
                "{keys:?} from {start:?}"
            );
        };
        for keys in [
            "h", "k", "b", "B", "ge", "gE", "{", "%", "Fz", "tz", ";", "N", ".", "dh", "ch", "db",
        ] {
            fails("|ab", keys);
        }
        for keys in [
            "l", "j", "w", "W", "e", "E", "}", "fz", "Tz", ",", "n", "dfz", "ci\"",
        ] {
            fails("a|b", keys);
        }
        // `*` and `#` with no word under or after the cursor.
        fails("|  ", "*");
        fails("|  ", "#");
        // `n` with no match left and `wrapscan` off.
        let s = Settings {
            wrapscan: false,
            ..Settings::default()
        };
        let (_, actions) = crate::test_util::run_edit_with(s, |_| {}, "x a |a", "*n");
        assert_eq!(actions.last(), Some(&Action::Failed));
        // A motion that moves does not fail.
        let (_, actions) = run_edit("|ab", "l");
        assert_eq!(actions, [Action::CursorMoved { jump: false }]);
    }

    #[test]
    fn a_failed_motion_under_an_operator_may_still_apply_it() {
        // `e` at the end of the buffer: the operator takes the last char.
        assert_edit("ab|c", "de", "a|b");
        assert_edit("ab|c", "ceX<Esc>", "ab|X");
        assert_edit("ab|c", "dE", "a|b");
        // `h` at column 0 fails, but `ch` still enters Insert and `dh`
        // changes nothing.
        let (text, actions) = run_edit("|ab", "chX<Esc>");
        assert_eq!(text, "|Xab");
        assert!(actions.contains(&Action::Failed));
        let (text, actions) = run_edit("|ab", "dh");
        assert_eq!(text, "|ab");
        assert_eq!(actions, [Action::Failed]);
        // `l` on an empty line too.
        let (text, actions) = run_edit("a\n|\nb", "x");
        assert_eq!(text, "a\n|\nb");
        assert_eq!(actions, [Action::Failed]);
    }

    #[test]
    fn macro_stops_when_motion_fails() {
        // `j` fails on the last line: the `x` after it does not run.
        assert_edit("|abc\nabc\nabc", "qajxq10@a", "abc\nbc\n|bc");
    }

    /// Feeds `keys` from `start` like `run_edit` and returns the globals.
    fn globals_after(start: &str, keys: &str) -> (String, VimGlobals) {
        let mut g = VimGlobals::default();
        let text = crate::test_util::run_edit_globals(Settings::default(), start, keys, &mut g);
        (text, g)
    }

    fn reg(g: &VimGlobals, name: char) -> Option<String> {
        g.read_register(Some(name), ClipboardMode::Never, std::path::Path::new("x"))
            .map(|c| c.text)
    }

    #[test]
    fn macro_with_vietnamese_text() {
        let (text, g) = globals_after("|một hai", "qaciwchào<Esc>qw@a");
        assert_eq!(text, "chào chà|o");
        assert_eq!(reg(&g, 'a').as_deref(), Some("ciwchào<Esc>"));
    }

    #[test]
    fn the_macro_register_pastes_as_text() {
        assert_edit("|ab", "qalq\"ap", "ab|l");
        let (_, g) = globals_after("|ab", "qalq");
        assert_eq!(
            g.read_register(Some('a'), ClipboardMode::Never, std::path::Path::new("x"))
                .map(|c| c.kind),
            Some(crate::RegKind::Charwise)
        );
    }

    #[test]
    fn any_register_text_runs() {
        assert_edit("|dd\none\ntwo", "\"ay$j@a", "dd\n|two");
    }

    #[test]
    fn a_search_is_recorded_and_runs_without_the_widget() {
        let (text, g) = globals_after("|foo1 foo2 foo3", "qa/foo<CR>xq");
        assert_eq!(text, "foo1 |oo2 foo3");
        assert_eq!(reg(&g, 'a').as_deref(), Some("/foo<CR>x"));
        let (text, actions) = run_edit("|foo1 foo2 foo3", "qa/foo<CR>xq@a");
        assert_eq!(text, "foo1 oo2 |oo3");
        let opened = actions
            .iter()
            .filter(|a| matches!(a, Action::OpenSearch { .. }))
            .count();
        assert_eq!(opened, 1, "only the recording opened the find widget");
    }

    #[test]
    fn a_dismissed_search_is_not_recorded() {
        let (_, g) = globals_after("|foo1 foo2", "qa/foo<Esc>xq");
        assert_eq!(reg(&g, 'a').as_deref(), Some("x"));
    }

    #[test]
    fn a_command_line_is_recorded_and_runs() {
        let (text, g) = globals_after("|a1\na2\na3", "qa:s/a/b/<CR>jq@a");
        assert_eq!(text, "b1\nb2\n|a3");
        assert_eq!(reg(&g, 'a').as_deref(), Some(":s/a/b/<CR>j"));
        // The prefill is not recorded: the `:` key rebuilds it.
        let (text, g) = globals_after("|a\nb\nc\nd\ne", "qaVj:d<CR>q@a");
        assert_eq!(reg(&g, 'a').as_deref(), Some("Vj:d<CR>"));
        assert_eq!(text, "|e");
    }

    #[test]
    fn a_macro_holds_the_ring_and_the_last_change() {
        let mut g = VimGlobals::default();
        g.registers.set_macro('m', "dddddd".into());
        let text = crate::test_util::run_edit_globals(
            Settings::default(),
            "|a\nb\nc\nd\ne\nf\ng\nh",
            "dddddd@m",
            &mut g,
        );
        assert_eq!(text, "|g\nh");
        // The deletes in the macro write `"` and `1` without shifting.
        assert_eq!(reg(&g, '1').as_deref(), Some("f\n"));
        assert_eq!(reg(&g, '"').as_deref(), Some("f\n"));
        assert_eq!(reg(&g, '2').as_deref(), Some("b\n"));
        assert_eq!(reg(&g, '3').as_deref(), Some("a\n"));
        // `.` still repeats the `x` before the macro, not its `dd`.
        let mut g = VimGlobals::default();
        g.registers.set_macro('m', "dd".into());
        let text =
            crate::test_util::run_edit_globals(Settings::default(), "|ab\ncd\nef", "x@m.", &mut g);
        assert_eq!(text, "|d\nef");
    }

    #[test]
    fn recording_reports_the_register() {
        let mut g = VimGlobals::default();
        let view = TestView::new("|a");
        let mut vim = Vim::new(view.cursor);
        let s = Settings::default();
        for k in parse_notation("qa") {
            vim.feed(k, &view, &mut g, &s);
        }
        assert_eq!(g.recording(), Some('a'));
        vim.feed(Key::Char('q'), &view, &mut g, &s);
        assert_eq!(g.recording(), None);
    }
}
