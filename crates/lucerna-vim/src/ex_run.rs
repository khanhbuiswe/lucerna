//! Running a `:` command line (spec §8): the `:` key, go to line, `:d`
//! `:y` `:>` `:<`, and the commands the workbench carries out.

use std::ops::RangeInclusive;

use lucerna_editor::SelectionSet;
use lucerna_text::Point;

use crate::classify::line;
use crate::edit;
use crate::ex::{self, Command, ExError, SubstituteArgs};
use crate::macros::{CmdKind, push_key};
use crate::operator::{MotionKind, OpRange, Operator, row_start};
use crate::substitute::{BACKGROUND_SUBSTITUTE_BYTES, SubstituteJob};
use crate::visual::line_end;
use crate::{Action, Key, Mode, Outcome, Settings, ViewInfo, Vim, VimGlobals};

/// What the workbench carries out for a `:` command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExCommand {
    /// `:w`, `:w!`
    Save { overwrite: bool },
    /// `:wa`
    SaveAll,
    /// `:q`, `:q!`
    Close { revert: bool },
    /// `:qa`
    CloseAll,
    /// `:wq`, `:x` (and with `!`)
    SaveAndClose {
        only_if_dirty: bool,
        overwrite: bool,
    },
    /// `:noh`
    NoHighlight,
}

/// What a `:` command did: actions now, or a substitute to run in the background.
enum Exec {
    Actions(Vec<Action>),
    Job(SubstituteJob),
}

/// `line` in Vim key notation as typed after the `:` key put `prefill`
/// there: `:` and the rest, or `:<C-u>` and the whole line when it no
/// longer starts with the prefill.
fn record_line(prefill: &str, line: &str) -> String {
    match line.strip_prefix(prefill) {
        Some(rest) => {
            let mut out = String::from(":");
            for c in rest.chars() {
                push_key(&mut out, Key::Char(c));
            }
            out
        }
        None => record_line_cleared(line),
    }
}

/// `:<C-u>` and `line` in Vim key notation.
fn record_line_cleared(line: &str) -> String {
    let mut out = String::from(":<C-u>");
    for c in line.chars() {
        push_key(&mut out, Key::Char(c));
    }
    out
}

fn is_visual(mode: Mode) -> bool {
    matches!(mode, Mode::Visual | Mode::VisualLine | Mode::VisualBlock)
}

impl Vim {
    /// The `:` key. A count `n` prefills `.` (1) or `.,.+{n-1}`; from
    /// Visual the selection ends (setting `'<` `'>`) and `'<,'>` is
    /// prefilled.
    pub(crate) fn open_command_line(
        &mut self,
        count: Option<u32>,
        view: &dyn ViewInfo,
        g: &mut VimGlobals,
    ) -> Outcome {
        let mut actions = Vec::new();
        let prefill = if is_visual(self.mode) {
            actions.extend(self.exit_visual(view, g));
            "'<,'>".to_string()
        } else {
            match count {
                None => String::new(),
                Some(0 | 1) => ".".to_string(),
                Some(n) => format!(".,.+{}", n - 1),
            }
        };
        if g.macro_running() {
            // A running macro types the command into vim's own line.
            self.open_macro_cmdline(CmdKind::Ex, prefill);
            return Outcome::Handled(actions);
        }
        self.last_prefill = Some(prefill.clone());
        actions.push(Action::OpenCommandLine { prefill });
        Outcome::Handled(actions)
    }

    /// The `:` line the `:` key opened closed without running, holding
    /// `line`. When the key put a prefill there (from Visual, after a
    /// count), what it undid is already in the macro being recorded: the
    /// macro gets the line as [`Vim::ex_command`] records it, ending in
    /// `<Esc>`, so its replay leaves Visual too.
    pub fn command_line_dismissed(&mut self, line: &str, g: &mut VimGlobals) {
        let Some(prefill) = self.last_prefill.take().filter(|p| !p.is_empty()) else {
            return;
        };
        let mut recorded = record_line(&prefill, line);
        recorded.push_str("<Esc>");
        g.record_text(&recorded);
    }

    /// Runs one command line (the text after `:`). Ends a Visual mode
    /// first (setting `'<` `'>`), drops pending keys and count. An error
    /// returns `[Message(err), Failed]`. The line is kept in register `:`
    /// (also when it failed), unless it is empty.
    ///
    /// The macro being recorded gets `:`, the line after the prefill the
    /// `:` key put there, and `<CR>`; a line that does not start with
    /// that prefill (or a command line opened otherwise) is recorded as
    /// `:<C-u>`, the whole line, and `<CR>`.
    pub fn ex_command(
        &mut self,
        line: &str,
        view: &dyn ViewInfo,
        g: &mut VimGlobals,
        s: &Settings,
    ) -> Outcome {
        let prefill = self.last_prefill.take();
        let mut recorded = match prefill.as_deref() {
            Some(p) => record_line(p, line),
            None => record_line_cleared(line),
        };
        recorded.push_str("<CR>");
        g.record_text(&recorded);
        let mut actions = Vec::new();
        if is_visual(self.mode) {
            actions.extend(self.exit_visual(view, g));
        }
        self.clear_pending();
        self.cmd_keys.clear();
        if line.trim().is_empty() {
            return Outcome::Handled(actions);
        }
        let result = self.run_ex(line, view, g, s);
        g.registers.set_last_command(line.to_string());
        match result {
            Ok(Exec::Actions(done)) => actions.extend(done),
            // Leaving Visual produced actions the job would lose: run it
            // here (the workbench's `:` key has already left Visual).
            Ok(Exec::Job(job)) if actions.is_empty() => return Outcome::Substitute(job),
            Ok(Exec::Job(job)) => {
                let result = job.run(view.snapshot());
                if let Outcome::Handled(done) = self.finish_substitute(&job, result, view, g, s) {
                    actions.extend(done);
                }
            }
            Err(e) => {
                actions.push(Action::Message(e.message()));
                actions.push(Action::Failed);
            }
        }
        Outcome::Handled(actions)
    }

    fn run_ex(
        &mut self,
        text: &str,
        view: &dyn ViewInfo,
        g: &mut VimGlobals,
        s: &Settings,
    ) -> Result<Exec, ExError> {
        let parsed = ex::parse(text)?;
        let snap = view.snapshot();
        let last_row = snap.line_count().saturating_sub(1);
        let clamp = matches!(parsed.command, Command::Goto);
        let rows = ex::resolve(
            &parsed.range,
            self.cursor.row,
            last_row,
            g.marks.get(view.path()),
            clamp,
        )?;
        // With a count, the range is `count` rows from the last row of the
        // range.
        let counted = |count: Option<u32>| -> RangeInclusive<u32> {
            match count {
                Some(n) => {
                    let first = *rows.end();
                    first..=first.saturating_add(n - 1).min(last_row)
                }
                None => rows.clone(),
            }
        };
        let run = |c| Ok(Exec::Actions(vec![Action::RunEx(c)]));
        let actions = match parsed.command {
            Command::Goto => self.go_to_line(*rows.end(), view, g, s),
            Command::Write { bang } => return run(ExCommand::Save { overwrite: bang }),
            Command::WriteAll => return run(ExCommand::SaveAll),
            Command::Quit { bang } => return run(ExCommand::Close { revert: bang }),
            Command::QuitAll => return run(ExCommand::CloseAll),
            Command::WriteQuit { bang } => {
                return run(ExCommand::SaveAndClose {
                    only_if_dirty: false,
                    overwrite: bang,
                });
            }
            Command::Exit { bang } => {
                return run(ExCommand::SaveAndClose {
                    only_if_dirty: true,
                    overwrite: bang,
                });
            }
            Command::NoHighlight => return run(ExCommand::NoHighlight),
            Command::Delete { register, count } => {
                self.ex_operator(Operator::Delete, counted(count), register, view, g, s)
            }
            Command::Yank { register, count } => {
                self.ex_operator(Operator::Yank, counted(count), register, view, g, s)
            }
            Command::Shift {
                right,
                depth,
                count,
            } => self.ex_shift(counted(count), !right, depth, view),
            Command::Substitute(args) => {
                let count = match &args {
                    SubstituteArgs::Repeat { count, .. } | SubstituteArgs::New { count, .. } => {
                        *count
                    }
                };
                let job = self.substitute_job(args, counted(count), g, s)?;
                // A macro runs it at once, whatever the size.
                if view.snapshot().len_bytes() <= BACKGROUND_SUBSTITUTE_BYTES || g.macro_running() {
                    let result = job.run(view.snapshot());
                    match self.finish_substitute(&job, result, view, g, s) {
                        Outcome::Handled(done) => done,
                        _ => Vec::new(),
                    }
                } else {
                    return Ok(Exec::Job(job));
                }
            }
        };
        Ok(Exec::Actions(actions))
    }

    /// A bare range: like `{N}G` on its last row.
    fn go_to_line(
        &mut self,
        row: u32,
        view: &dyn ViewInfo,
        g: &mut VimGlobals,
        s: &Settings,
    ) -> Vec<Action> {
        let from = self.cursor;
        let Outcome::Handled(actions) = self.go_to_row(view, row, s.startofline, true) else {
            return Vec::new();
        };
        if self.cursor != from {
            g.marks.entry(view.path().clone()).or_default().last_jump = Some(from);
        }
        actions
    }

    /// `:d` and `:y` over whole `rows`, as `dd` and `yy`.
    fn ex_operator(
        &mut self,
        op: Operator,
        rows: RangeInclusive<u32>,
        register: Option<char>,
        view: &dyn ViewInfo,
        g: &mut VimGlobals,
        s: &Settings,
    ) -> Vec<Action> {
        let snap = view.snapshot();
        let range = row_start(snap, *rows.start())..line_end(snap, *rows.end());
        let r = OpRange {
            range,
            kind: MotionKind::Linewise,
        };
        let Some((mut actions, cursor, _)) = edit::run_operator(op, r, view, g, s, register) else {
            return Vec::new();
        };
        if op == Operator::Yank {
            return actions;
        }
        actions.push(Action::CursorMoved { jump: false });
        self.set_cursor_after_edit(cursor);
        actions
    }

    /// `:>` and `:<` over whole `rows`; the cursor goes to the first
    /// non-blank of the last row.
    fn ex_shift(
        &mut self,
        rows: RangeInclusive<u32>,
        outdent: bool,
        depth: u32,
        view: &dyn ViewInfo,
    ) -> Vec<Action> {
        let Some(ctx) = view.edit_ctx() else {
            return Vec::new();
        };
        let Some((mut plan, _)) = edit::shift_rows(ctx, *rows.start(), *rows.end(), outdent, depth)
        else {
            return Vec::new();
        };
        let snap = view.snapshot();
        let last = *rows.end();
        let base = row_start(snap, last);
        let old = line(snap, last);
        let mut lead = old.len() - old.trim_start_matches([' ', '\t']).len();
        let mut shift = 0isize;
        for (range, new) in &plan.edits {
            if range.start == base {
                lead = new.len();
            } else if range.start < base {
                shift += new.len() as isize - range.len() as isize;
            }
        }
        let cursor = Point {
            row: last,
            column: lead as u32,
        };
        plan.selections = SelectionSet::cursor((base as isize + shift) as usize + lead);
        self.set_cursor_after_edit(cursor);
        let mut actions = Vec::new();
        if !plan.edits.is_empty() {
            actions.push(Action::Edit(plan));
        }
        actions.push(Action::CursorMoved { jump: false });
        actions
    }

    pub(crate) fn set_cursor_after_edit(&mut self, cursor: Point) {
        self.cursor = cursor;
        self.anchor = cursor;
        self.goal = None;
    }
}

#[cfg(test)]
mod tests {
    use crate::test_util::{assert_edit, run_edit, run_edit_with, run_keys};
    use crate::{Action, ExCommand, Mode, Settings};

    fn without_open(actions: Vec<Action>) -> Vec<Action> {
        actions
            .into_iter()
            .filter(|a| !matches!(a, Action::OpenCommandLine { .. }))
            .collect()
    }

    // Adapted from VSCodeVim test/cmd_line/delete.test.ts: ":d"
    #[test]
    fn delete_current_line() {
        assert_edit("line 1\nli|ne 2\nline 3", ":d<CR>", "line 1\n|line 3");
    }

    // Adapted from VSCodeVim test/cmd_line/delete.test.ts: ":2,3d"
    #[test]
    fn delete_range() {
        assert_edit("line 1\nli|ne 2\nline 3", ":2,3d<CR>", "|line 1");
    }

    // Adapted from VSCodeVim test/cmd_line/delete.test.ts: ":d 2"
    #[test]
    fn delete_count() {
        assert_edit("line 1\nli|ne 2\nline 3", ":d 2<CR>", "|line 1");
    }

    // Adapted from VSCodeVim test/cmd_line/yank.test.ts: ":yank will yank a single line"
    #[test]
    fn yank_single_line() {
        assert_edit("|one\ntwo\nthree", ":yank<CR>p", "one\n|one\ntwo\nthree");
    }

    // Adapted from VSCodeVim test/cmd_line/yank.test.ts: ":yank [x] [cnt] will yank [cnt] lines"
    #[test]
    fn yank_count_lines() {
        assert_edit(
            "|one\ntwo\nthree\nfour\nfive",
            ":ya3<CR>p",
            "one\n|one\ntwo\nthree\ntwo\nthree\nfour\nfive",
        );
    }

    // Adapted from VSCodeVim test/cmd_line/yank.test.ts: ":[range]yank will yank from the end of the range, if range a line number"
    #[test]
    fn yank_from_the_end_of_the_range() {
        assert_edit(
            "|one\ntwo\nthree\nfour\nfive",
            ":.+3yan<CR>p",
            "one\n|four\ntwo\nthree\nfour\nfive",
        );
    }

    #[test]
    fn goto_line_goes_to_first_non_blank_and_marks_the_jump() {
        let text = "|a\nb\nc\nd\n  e\nf\ng\nh\ni\nj";
        assert_edit(text, ":5<CR>", "a\nb\nc\nd\n  |e\nf\ng\nh\ni\nj");
        assert_edit(text, ":5<CR>''", "|a\nb\nc\nd\n  e\nf\ng\nh\ni\nj");
    }

    #[test]
    fn goto_line_is_clamped() {
        assert_edit("|a\nb\nc", ":999<CR>", "a\nb\n|c");
        assert_edit("|a\nb\nc", ":$<CR>", "a\nb\n|c");
    }

    #[test]
    fn colon_prefills_the_range() {
        let (_, vim, actions) = run_keys(Settings::default(), |_| {}, "|a\nb\nc", "Vj:");
        assert_eq!(vim.mode(), Mode::Normal);
        assert!(actions.contains(&Action::OpenCommandLine {
            prefill: "'<,'>".into()
        }));
        let (_, _, actions) = run_keys(Settings::default(), |_| {}, "|a\nb\nc", "3:");
        assert_eq!(
            actions,
            [Action::OpenCommandLine {
                prefill: ".,.+2".into()
            }]
        );
        let (_, _, actions) = run_keys(Settings::default(), |_| {}, "|a\nb\nc", "1:");
        assert_eq!(
            actions,
            [Action::OpenCommandLine {
                prefill: ".".into()
            }]
        );
        let (_, vim, actions) = run_keys(Settings::default(), |_| {}, "|a\nb\nc", ":");
        assert_eq!(actions, [Action::OpenCommandLine { prefill: "".into() }]);
        assert_eq!(vim.last_prefill.as_deref(), Some(""));
    }

    #[test]
    fn visual_colon_shifts_the_selected_lines() {
        let (text, _) = run_edit_with(
            Settings::default(),
            |v| v.config.insert_spaces = true,
            "|a\nb\nc",
            "Vj:><CR>",
        );
        assert_eq!(text, "    a\n    |b\nc");
    }

    #[test]
    fn unset_mark_fails_without_editing() {
        let (text, actions) = run_edit("|a\nb\nc", ":'a,'b><CR>");
        assert_eq!(text, "|a\nb\nc");
        assert_eq!(
            without_open(actions),
            [Action::Message("E20: Mark not set".into()), Action::Failed]
        );
    }

    #[test]
    fn unknown_command_fails() {
        let (_, actions) = run_edit("|a", ":foo<CR>");
        assert_eq!(
            without_open(actions),
            [
                Action::Message("E492: Not an editor command: foo".into()),
                Action::Failed
            ]
        );
    }

    #[test]
    fn workbench_commands() {
        for (keys, want) in [
            (":w<CR>", ExCommand::Save { overwrite: false }),
            (":w!<CR>", ExCommand::Save { overwrite: true }),
            (":wa<CR>", ExCommand::SaveAll),
            (":q<CR>", ExCommand::Close { revert: false }),
            (":q!<CR>", ExCommand::Close { revert: true }),
            (":qa<CR>", ExCommand::CloseAll),
            (
                ":wq<CR>",
                ExCommand::SaveAndClose {
                    only_if_dirty: false,
                    overwrite: false,
                },
            ),
            (
                ":x!<CR>",
                ExCommand::SaveAndClose {
                    only_if_dirty: true,
                    overwrite: true,
                },
            ),
            (":noh<CR>", ExCommand::NoHighlight),
        ] {
            let (_, actions) = run_edit("|a\nb", keys);
            assert_eq!(without_open(actions), [Action::RunEx(want)], "{keys}");
        }
    }

    #[test]
    fn delete_in_a_read_only_view_does_not_edit() {
        let (text, actions) = run_edit_with(
            Settings::default(),
            |v| v.readonly = true,
            "|a\nb",
            ":d<CR>",
        );
        assert_eq!(text, "|a\nb");
        assert!(!actions.iter().any(|a| matches!(a, Action::Edit(_))));
    }

    #[test]
    fn yank_in_a_read_only_view_still_yanks() {
        let (_, actions) = run_edit_with(
            Settings::default(),
            |v| v.readonly = true,
            "|a\nb",
            ":2y<CR>",
        );
        assert!(actions.contains(&Action::WriteClipboard {
            text: "b\n".into(),
            linewise: true
        }));
        assert!(!actions.iter().any(|a| matches!(a, Action::Failed)));
    }

    #[test]
    fn shift_puts_the_cursor_on_the_last_row_and_yank_keeps_it() {
        let (text, _) = run_edit_with(
            Settings::default(),
            |v| v.config.insert_spaces = true,
            "|a\nb\nc",
            ":1,2>><CR>",
        );
        assert_eq!(text, "        a\n        |b\nc");
        assert_edit("a\n|b\nc", ":1,3y<CR>", "a\n|b\nc");
    }

    #[test]
    fn the_colon_register_holds_the_last_command() {
        assert_edit("|a\nb\nc\nd", ":d 2<CR>\":p", "cd |2\nd");
        // A failed command is kept too.
        assert_edit("|a", ":foo<CR>\":p", "afo|o");
    }
}
