//! `:s` (spec §8.2): translating Vim's pattern and replacement syntax,
//! running a substitute on a snapshot, and applying its result.

use std::ops::{Range, RangeInclusive};

use lucerna_editor::EditKind;
use lucerna_editor::edit::replace_ranges;
use lucerna_text::{BufferSnapshot, Point};
use regex::{Captures, Regex, RegexBuilder};

use crate::ex::{ExError, SubstituteArgs};
use crate::{Action, Outcome, SearchQuery, Settings, ViewInfo, Vim, VimGlobals};

/// A buffer above this many bytes runs `:s` in the background.
pub const BACKGROUND_SUBSTITUTE_BYTES: usize = 1024 * 1024;

/// What `:s` without a pattern repeats.
#[derive(Debug, Clone)]
pub(crate) struct LastSubstitute {
    /// The pattern as the regex engine reads it.
    pattern: String,
    /// The pattern as the user typed it, for `E486`.
    typed: String,
    case_sensitive: bool,
    /// The replacement with `~` already resolved.
    template: String,
}

/// Vim's escapes in a Rust-regex pattern: `\(` `\)` `\{` `\}` `\+` `\?` `\|` lose the backslash;
/// `\<` `\>` stay (regex 1.13 reads them as word starts and ends).
pub(crate) fn translate_pattern(vim: &str) -> String {
    let mut out = String::with_capacity(vim.len());
    let mut chars = vim.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some(n @ ('(' | ')' | '{' | '}' | '+' | '?' | '|')) => out.push(n),
            Some(n) => {
                out.push('\\');
                out.push(n);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Case conversion in force while a replacement is built.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CaseMode {
    Keep,
    Upper,
    Lower,
}

struct ReplacementBuilder {
    out: String,
    /// `\u` or `\l`: applies to the next char only.
    once: Option<CaseMode>,
    mode: CaseMode,
}

impl ReplacementBuilder {
    fn push_char(&mut self, c: char) {
        let case = self.once.take().unwrap_or(self.mode);
        match case {
            CaseMode::Keep => self.out.push(c),
            CaseMode::Upper => self.out.extend(c.to_uppercase()),
            CaseMode::Lower => self.out.extend(c.to_lowercase()),
        }
    }

    fn push_str(&mut self, text: &str) {
        for c in text.chars() {
            self.push_char(c);
        }
    }

    /// A newline or tab: no case, and it does not use up `\u`.
    fn push_raw(&mut self, c: char) {
        self.out.push(c);
    }
}

/// `&` `\0` match, `\1`–`\9` groups, `\n` newline, `\t` tab, `\\` `\&` literal, `~` = `previous`,
/// `\u \U \l \L \e \E` case.
pub(crate) fn expand_replacement(template: &str, caps: &Captures, previous: &str) -> String {
    let mut b = ReplacementBuilder {
        out: String::new(),
        once: None,
        mode: CaseMode::Keep,
    };
    let group = |b: &mut ReplacementBuilder, i: usize| {
        if let Some(m) = caps.get(i) {
            b.push_str(m.as_str());
        }
    };
    let mut chars = template.chars();
    while let Some(c) = chars.next() {
        match c {
            '&' => group(&mut b, 0),
            '~' => {
                let text = expand_replacement(previous, caps, "");
                b.push_str(&text);
            }
            '\\' => match chars.next() {
                Some(d @ '0'..='9') => group(&mut b, d as usize - '0' as usize),
                Some('n' | 'r') => b.push_raw('\n'),
                Some('t') => b.push_raw('\t'),
                Some('u') => b.once = Some(CaseMode::Upper),
                Some('l') => b.once = Some(CaseMode::Lower),
                Some('U') => b.mode = CaseMode::Upper,
                Some('L') => b.mode = CaseMode::Lower,
                Some('e' | 'E') => {
                    b.mode = CaseMode::Keep;
                    b.once = None;
                }
                Some(other) => b.push_char(other),
                None => b.push_char('\\'),
            },
            other => b.push_char(other),
        }
    }
    b.out
}

/// Replaces each unescaped `~` in `template` with `previous`.
fn resolve_tilde(template: &str, previous: &str) -> String {
    let mut out = String::with_capacity(template.len());
    let mut chars = template.chars();
    while let Some(c) = chars.next() {
        match c {
            '~' => out.push_str(previous),
            '\\' => {
                out.push('\\');
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// A substitute ready to run on a snapshot; Send, so the workbench may run it in the background.
#[derive(Debug, Clone)]
pub struct SubstituteJob {
    regex: Regex,
    /// The replacement with `~` resolved.
    template: String,
    rows: RangeInclusive<u32>,
    global: bool,
    report_only: bool,
    /// The pattern as the regex engine reads it, and its case.
    pattern: String,
    case_sensitive: bool,
    /// The pattern as the user typed it.
    typed: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubstituteResult {
    /// Byte ranges of the snapshot and their new text.
    edits: Vec<(Range<usize>, String)>,
    substitutions: u32,
    lines: u32,
    /// Where the line that holds the end of the last replacement starts, in
    /// the text after the edit, and its row there.
    cursor_offset: usize,
    cursor_row: u32,
}

impl PartialEq for SubstituteJob {
    fn eq(&self, other: &Self) -> bool {
        self.regex.as_str() == other.regex.as_str()
            && self.template == other.template
            && self.rows == other.rows
            && self.global == other.global
            && self.report_only == other.report_only
            && self.pattern == other.pattern
            && self.case_sensitive == other.case_sensitive
            && self.typed == other.typed
    }
}

impl SubstituteJob {
    /// Finds every substitution on `snapshot` (rows past its end are
    /// skipped). The text is not changed.
    pub fn run(&self, snapshot: &BufferSnapshot) -> SubstituteResult {
        let mut result = SubstituteResult {
            edits: Vec::new(),
            substitutions: 0,
            lines: 0,
            cursor_offset: 0,
            cursor_row: 0,
        };
        let last = (*self.rows.end()).min(snapshot.line_count().saturating_sub(1));
        // The text after the edit moves by `shift` bytes and `rows_added`
        // rows relative to the snapshot.
        let mut shift = 0isize;
        let mut rows_added = 0u32;
        for row in *self.rows.start()..=last {
            let text = snapshot.line(row);
            let base = snapshot.point_to_offset(Point { row, column: 0 });
            let mut found = false;
            // The new offset of the start of the line that holds the end
            // of the last replacement on this row.
            let mut line_start = (base as isize + shift) as usize;
            let mut take = |caps: &Captures| {
                let m = caps.get(0).expect("group 0 always matches");
                let new = expand_replacement(&self.template, caps, "");
                let start = base + m.start();
                result.substitutions += 1;
                if let Some(nl) = new.rfind('\n') {
                    let new_start = (start as isize + shift) as usize;
                    line_start = new_start + nl + 1;
                }
                rows_added += new.matches('\n').count() as u32;
                shift += new.len() as isize - m.len() as isize;
                result.edits.push((start..base + m.end(), new));
                found = true;
            };
            if self.global {
                for caps in self.regex.captures_iter(&text) {
                    take(&caps);
                }
            } else if let Some(caps) = self.regex.captures(&text) {
                take(&caps);
            }
            if found {
                result.lines += 1;
                result.cursor_offset = line_start;
                result.cursor_row = row + rows_added;
            }
        }
        result
    }
}

impl Vim {
    /// Builds the job for `:s` over `rows`, remembering nothing yet.
    pub(crate) fn substitute_job(
        &self,
        args: SubstituteArgs,
        rows: RangeInclusive<u32>,
        g: &VimGlobals,
        s: &Settings,
    ) -> Result<SubstituteJob, ExError> {
        let (flags, pattern, typed, template, case) = match args {
            SubstituteArgs::Repeat { flags, .. } => {
                let last = g
                    .last_substitute
                    .as_ref()
                    .ok_or(ExError::NoPreviousPattern)?;
                (
                    flags,
                    last.pattern.clone(),
                    last.typed.clone(),
                    last.template.clone(),
                    last.case_sensitive,
                )
            }
            SubstituteArgs::New {
                pattern,
                replacement,
                flags,
                ..
            } => {
                let previous = g.last_substitute.as_ref().map_or("", |l| &l.template);
                let template = resolve_tilde(&replacement, previous);
                if pattern.is_empty() {
                    let (q, _) = g.last_search.as_ref().ok_or(ExError::NoPreviousPattern)?;
                    let mut p = if q.is_regex {
                        q.pattern.clone()
                    } else {
                        regex::escape(&q.pattern)
                    };
                    if q.whole_word {
                        p = format!(r"\b(?:{p})\b");
                    }
                    (flags, p, q.pattern.clone(), template, q.case_sensitive)
                } else {
                    (flags, translate_pattern(&pattern), pattern, template, true)
                }
            }
        };
        let case_sensitive = flags.ignore_case.map_or(case, |ignore| !ignore);
        let regex = RegexBuilder::new(&pattern)
            .case_insensitive(!case_sensitive)
            .build()
            .map_err(|_| ExError::InvalidPattern(typed.clone()))?;
        Ok(SubstituteJob {
            regex,
            template,
            rows,
            global: flags.global ^ s.gdefault,
            report_only: flags.report_only,
            pattern,
            case_sensitive,
            typed,
        })
    }

    /// Applies a job's result computed for the view's current text.
    pub fn finish_substitute(
        &mut self,
        job: &SubstituteJob,
        result: SubstituteResult,
        view: &dyn ViewInfo,
        g: &mut VimGlobals,
        _s: &Settings,
    ) -> Outcome {
        let query = SearchQuery {
            pattern: job.pattern.clone(),
            is_regex: true,
            case_sensitive: job.case_sensitive,
            whole_word: false,
        };
        g.last_search = Some((query.clone(), false));
        g.last_substitute = Some(LastSubstitute {
            pattern: job.pattern.clone(),
            typed: job.typed.clone(),
            case_sensitive: job.case_sensitive,
            template: job.template.clone(),
        });
        if result.substitutions == 0 {
            return Outcome::Handled(vec![
                Action::Message(ExError::PatternNotFound(job.typed.clone()).message()),
                Action::Failed,
            ]);
        }
        let plural = |n: u32| if n == 1 { "" } else { "s" };
        let (n, m) = (result.substitutions, result.lines);
        let mut actions = Vec::new();
        let ctx = if job.report_only {
            None
        } else {
            view.edit_ctx()
        };
        let message = match ctx {
            Some(ctx) => {
                let plan = replace_ranges(ctx, result.edits, result.cursor_offset, EditKind::Other);
                actions.push(Action::Edit(plan));
                self.set_cursor_after_edit(Point {
                    row: result.cursor_row,
                    column: 0,
                });
                actions.push(Action::CursorMoved { jump: false });
                format!("{n} substitution{} on {m} line{}", plural(n), plural(m))
            }
            None => format!(
                "{n} match{} on {m} line{}",
                if n == 1 { "" } else { "es" },
                plural(m)
            ),
        };
        actions.push(Action::Highlight(query));
        actions.push(Action::Message(message));
        Outcome::Handled(actions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{assert_edit, assert_edit_with, run_edit, run_edit_with};

    #[test]
    fn translates_vim_escapes() {
        assert_eq!(translate_pattern("\\(a\\|b\\)\\+"), "(a|b)+");
        assert_eq!(translate_pattern("\\<foo\\>"), "\\<foo\\>");
        assert_eq!(translate_pattern("a\\{2,3\\}\\?"), "a{2,3}?");
        assert_eq!(translate_pattern("\\\\(x"), "\\\\(x");
        assert_eq!(translate_pattern("x\\"), "x\\");
    }

    // Adapted from VSCodeVim test/cmd_line/substitute.test.ts: "Replace single word once"
    #[test]
    fn replace_once() {
        assert_edit("|aba", ":%s/a/d<CR>", "|dba");
    }

    // Adapted from VSCodeVim test/cmd_line/substitute.test.ts: "Replace with `g` flag"
    #[test]
    fn replace_with_g() {
        assert_edit("|aba", ":%s/a/d/g<CR>", "|dbd");
    }

    // Adapted from VSCodeVim test/cmd_line/substitute.test.ts: "Replace with flags AND count"
    #[test]
    fn flags_and_count() {
        assert_edit(
            "|blah blah\nblah\nblah blah\nblah blah",
            ":.s/blah/yay/g 2<CR>",
            "yay yay\n|yay\nblah blah\nblah blah",
        );
    }

    // Adapted from VSCodeVim test/cmd_line/substitute.test.ts: "Replace across all lines"
    #[test]
    fn replace_across_lines() {
        assert_edit("|aba\nab", ":%s/a/d/g<CR>", "dbd\n|db");
    }

    // Adapted from VSCodeVim test/cmd_line/substitute.test.ts: "Replace across relative line range using dot, plus, and minus"
    #[test]
    fn relative_range() {
        assert_edit(
            "blah blah\nbla|h\nblah blah\nblah blah",
            ":.-1,.+1s/blah/yay<CR>",
            "yay blah\nyay\n|yay blah\nblah blah",
        );
    }

    // Adapted from VSCodeVim test/cmd_line/substitute.test.ts: "Replace across relative line range using numLines+colon shorthand"
    #[test]
    fn count_colon_shorthand() {
        assert_edit(
            "blah blah\n|blah\nblah blah\nblah blah",
            "3:s/blah/yay<CR>",
            "blah blah\nyay\nyay blah\n|yay blah",
        );
    }

    // Adapted from VSCodeVim test/cmd_line/substitute.test.ts: "Repeat replacement across relative line range"
    #[test]
    fn repeat_over_range() {
        assert_edit(
            "|blah blah\nblah\nblah blah\nblah blah",
            ":s/blah/yay<CR>j3:s<CR>",
            "yay blah\nyay\nyay blah\n|yay blah",
        );
    }

    // Adapted from VSCodeVim test/cmd_line/substitute.test.ts: "Replace with \\n"
    #[test]
    fn replace_with_newline() {
        assert_edit("one |two three", ":s/t/\\n/g<CR>", "one \nwo \n|hree");
    }

    // Adapted from VSCodeVim test/cmd_line/substitute.test.ts: "Preserve \\b in regular expression"
    #[test]
    fn word_boundary_kept() {
        assert_edit(
            "one |two three thirteen",
            ":s/\\bt/x/g<CR>",
            "|one xwo xhree xhirteen",
        );
    }

    // Adapted from VSCodeVim test/cmd_line/substitute.test.ts: "Replace text in selection"
    #[test]
    fn replace_in_selection() {
        assert_edit("|aba\nab", "Vj:s/a/d/g<CR>", "dbd\n|db");
    }

    // Adapted from VSCodeVim test/cmd_line/substitute.test.ts: "Substitute support marks"
    #[test]
    fn marks_as_range() {
        assert_edit("|aba\naba\nabc", "majmb:'a,'bs/a/d/g<CR>", "dbd\n|dbd\nabc");
    }

    // Adapted from VSCodeVim test/cmd_line/substitute.test.ts: "`n` flag (report count)"
    #[test]
    fn n_flag_counts_without_editing() {
        let start = "apple\nban|ana\ncelery\ndragonfruit";
        let (text, actions) = run_edit(start, ":%s/a//gn<CR>");
        assert_eq!(text, start);
        assert!(actions.contains(&Action::Message("5 matches on 3 lines".into())));
        assert!(!actions.iter().any(|a| matches!(a, Action::Edit(_))));
    }

    // Adapted from VSCodeVim test/cmd_line/substitute.test.ts: "Substitute with previous search using *"
    #[test]
    fn empty_pattern_uses_the_last_search() {
        assert_edit(
            "|foo\nbar\nfoo\nbar",
            "*:%s//fighters/<CR>",
            "fighters\nbar\n|fighters\nbar",
        );
    }

    // Adapted from VSCodeVim test/cmd_line/substitute.test.ts: "`~` in replace string uses previous replace string"
    #[test]
    fn tilde_is_the_previous_replacement() {
        assert_edit(
            "|one two three\ntwo three four",
            ":s/two/xyz/<CR>j:s/three/~ ~/<CR>",
            "one xyz three\n|two xyz xyz four",
        );
    }

    // Adapted from VSCodeVim test/cmd_line/substitute.test.ts: "Substitute repeat previous should accept flags"
    #[test]
    fn repeat_accepts_flags() {
        assert_edit("|fooo", ":s/o/un/<CR>:s g<CR>", "|fununun");
    }

    // Adapted from VSCodeVim test/cmd_line/substitute.test.ts: "\\U"
    #[test]
    fn upper_case() {
        assert_edit(
            "|she sells seashells by the seashore",
            ":s/s\\S*/\\U&x/g<CR>",
            "|SHEX SELLSX SEASHELLSX by the SEASHOREX",
        );
    }

    // Adapted from VSCodeVim test/cmd_line/substitute.test.ts: "\\u"
    #[test]
    fn upper_first() {
        assert_edit(
            "|she sells seashells by the seashore",
            ":s/s\\S*/\\u&x/g<CR>",
            "|Shex Sellsx Seashellsx by the Seashorex",
        );
    }

    // Adapted from VSCodeVim test/cmd_line/substitute.test.ts: "\\L then \\E"
    #[test]
    fn lower_then_end() {
        assert_edit(
            "|SHE SELLS SEASHELLS BY THE SEASHORE",
            ":s/S\\S*/\\L&\\EX/g<CR>",
            "|sheX sellsX seashellsX BY THE seashoreX",
        );
    }

    // Adapted from VSCodeVim test/cmd_line/substitute.test.ts: "Replace all matches in the line"
    #[test]
    fn gdefault_replaces_all() {
        let s = Settings {
            gdefault: true,
            ..Settings::default()
        };
        assert_edit_with(s.clone(), |_| {}, "|aba", ":%s/a/d<CR>", "|dbd");
        // Adapted from VSCodeVim test/cmd_line/substitute.test.ts: "Replace with `g` flag inverts global flag"
        assert_edit_with(s, |_| {}, "|aba", ":%s/a/d/g<CR>", "|dba");
    }

    #[test]
    fn message_counts_substitutions_and_lines_in_one_edit() {
        let (text, actions) = run_edit("|foo foo\nfoo", ":%s/foo/bar/g<CR>");
        assert_eq!(text, "bar bar\n|bar");
        assert!(actions.contains(&Action::Message("3 substitutions on 2 lines".into())));
        assert_eq!(
            actions
                .iter()
                .filter(|a| matches!(a, Action::Edit(_)))
                .count(),
            1
        );
        let (_, actions) = run_edit("|foo", ":s/foo/bar/<CR>");
        assert!(actions.contains(&Action::Message("1 substitution on 1 line".into())));
    }

    #[test]
    fn no_match_fails() {
        let (text, actions) = run_edit("|abc", ":s/x/y/<CR>");
        assert_eq!(text, "|abc");
        assert!(actions.ends_with(&[
            Action::Message("E486: Pattern not found: x".into()),
            Action::Failed
        ]));
    }

    #[test]
    fn invalid_pattern_reports_e383() {
        let (text, actions) = run_edit("|abc", ":s/(/x/<CR>");
        assert_eq!(text, "|abc");
        assert!(actions.ends_with(&[
            Action::Message("E383: Invalid search string: (".into()),
            Action::Failed
        ]));
    }

    #[test]
    fn no_previous_pattern_is_e35() {
        let (_, actions) = run_edit("|abc", ":s//x/<CR>");
        assert!(actions.contains(&Action::Message(
            "E35: No previous regular expression".into()
        )));
        let (_, actions) = run_edit("|abc", ":s<CR>");
        assert!(actions.contains(&Action::Message(
            "E35: No previous regular expression".into()
        )));
    }

    #[test]
    fn empty_matches_prefix_every_line() {
        assert_edit("|a\n\nb", ":%s/^/# /<CR>", "# a\n# \n|# b");
        assert_edit("|ab", ":s/x*/-/g<CR>", "|-a-b-");
    }

    #[test]
    fn unicode_text_keeps_byte_offsets() {
        assert_edit("|xin chào", ":s/chào/\\U&/<CR>", "|xin CHÀO");
        assert_edit("|tiếng Việt", ":s/ế/e/<CR>", "|tieng Việt");
        assert_edit("|đ ế\nđ", ":%s/ế/ê\\nx/<CR>", "đ ê\n|x\nđ");
        assert_edit("|ếế", ":s/ế/a/g<CR>", "|aa");
        assert_edit("|Đà", ":s/đ/d/i<CR>", "|dà");
    }

    #[test]
    fn newline_replacement_puts_the_cursor_on_the_last_new_line() {
        assert_edit("|a,b\nc,d", ":%s/,/\\n/g<CR>", "a\nb\nc\n|d");
        assert_edit("|a,b\nc", ":s/,/\\n\\n/<CR>", "a\n\n|b\nc");
    }

    #[test]
    fn case_flags() {
        assert_edit("|Foo foo", ":s/foo/x/g<CR>", "|Foo x");
        assert_edit("|Foo foo", ":s/foo/x/gi<CR>", "|x x");
    }

    #[test]
    fn groups_and_escapes() {
        assert_edit("|ab", ":s/\\(a\\)\\(b\\)/\\2\\1/<CR>", "|ba");
        assert_edit("|a", ":s/a/\\&\\\\/<CR>", "|&\\");
        assert_edit("|a", ":s/a/[&]\\t/<CR>", "|[a]\t");
    }

    #[test]
    fn n_goes_to_the_next_match_of_the_substituted_pattern() {
        assert_edit("|foo foo\nx\nfoo", ":s/foo/bar/<CR>n", "bar |foo\nx\nfoo");
    }

    #[test]
    fn read_only_view_reports_without_editing() {
        let (text, actions) = run_edit_with(
            Settings::default(),
            |v| v.readonly = true,
            "|aa\naa",
            ":%s/a/b/g<CR>",
        );
        assert_eq!(text, "|aa\naa");
        assert!(!actions.iter().any(|a| matches!(a, Action::Edit(_))));
        assert!(actions.contains(&Action::Message("4 matches on 2 lines".into())));
    }

    #[test]
    fn large_buffer_returns_a_job_that_gives_the_same_result() {
        use crate::test_util::TestView;
        let mut body = "foo\n".repeat(BACKGROUND_SUBSTITUTE_BYTES / 4 + 10);
        body.push_str("end");
        let view = TestView::new(&format!("|{body}"));
        let (mut vim, mut g, s) = (
            Vim::new(view.cursor),
            VimGlobals::default(),
            Settings::default(),
        );
        let out = vim.ex_command("%s/foo/bar/", &view, &mut g, &s);
        let Outcome::Substitute(job) = out else {
            panic!("expected a job, got {out:?}");
        };
        let result = job.run(view.snapshot());
        let out = vim.finish_substitute(&job, result, &view, &mut g, &s);
        let Outcome::Handled(actions) = out else {
            panic!("{out:?}")
        };
        let n = BACKGROUND_SUBSTITUTE_BYTES / 4 + 10;
        assert!(actions.contains(&Action::Message(format!("{n} substitutions on {n} lines"))));
        assert!(actions.iter().any(|a| matches!(a, Action::Edit(_))));
    }
}
