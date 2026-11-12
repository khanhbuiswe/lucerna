//! Marked-text notation for tests: `|` is a cursor, `«…»` a selection whose
//! head is at `»`, `»…«` a reversed one, and `*` before a marker makes that
//! selection primary (default: the first).

use lucerna_text::{Buffer, BufferSnapshot};

use crate::plan::{AutoClosing, AutoClosingEdit, AutoIndent, AutoSurround};
use crate::selection::{Goal, Selection, SelectionSet};

pub fn marked(src: &str) -> (BufferSnapshot, SelectionSet) {
    let mut text = String::new();
    // (start offset, is_primary) of open forward / reversed selections.
    let mut fwd: Vec<(usize, bool)> = Vec::new();
    let mut rev: Vec<(usize, bool)> = Vec::new();
    // (start, end, reversed, primary)
    let mut done: Vec<(usize, usize, bool, bool)> = Vec::new();
    let mut star = false;
    for ch in src.chars() {
        match ch {
            '*' => star = true,
            '|' => {
                done.push((text.len(), text.len(), false, star));
                star = false;
            }
            '«' => {
                if fwd.is_empty() && !rev.is_empty() {
                    let (start, p) = rev.pop().unwrap();
                    done.push((start, text.len(), true, p));
                } else {
                    fwd.push((text.len(), star));
                }
                star = false;
            }
            '»' => {
                if let Some((start, p)) = fwd.pop() {
                    done.push((start, text.len(), false, p));
                } else {
                    rev.push((text.len(), star));
                }
                star = false;
            }
            c => text.push(c),
        }
    }
    assert!(
        fwd.is_empty() && rev.is_empty(),
        "unbalanced markers in {src:?}"
    );
    assert!(!done.is_empty(), "no selection in {src:?}");
    done.sort_by_key(|&(s, e, ..)| (s, e));
    let primary = done.iter().position(|d| d.3).unwrap_or(0) as u64;
    let selections = done
        .iter()
        .enumerate()
        .map(|(i, &(start, end, reversed, _))| {
            let (anchor, head) = if reversed { (end, start) } else { (start, end) };
            Selection {
                id: i as u64,
                anchor,
                head,
                goal: Goal::None,
            }
        })
        .collect();
    let snapshot = Buffer::from_bytes(text.as_bytes(), false)
        .expect("marked text must load")
        .snapshot();
    (
        snapshot,
        SelectionSet::from_selections(selections, primary, false),
    )
}

pub fn render(snapshot: &BufferSnapshot, set: &SelectionSet) -> String {
    let text = snapshot.text_for_range(0..snapshot.len_bytes());
    // (offset, rank, marker): closers before cursors before openers.
    let mut marks: Vec<(usize, u8, String)> = Vec::new();
    let primary = set.primary().id;
    let star = |s: &Selection| {
        if s.id == primary && set.len() > 1 && s.id != set.all()[0].id {
            "*"
        } else {
            ""
        }
    };
    for s in set.all() {
        let r = s.range();
        if s.is_empty() {
            marks.push((r.start, 1, format!("{}|", star(s))));
        } else if s.reversed() {
            marks.push((r.start, 2, format!("{}»", star(s))));
            marks.push((r.end, 0, "«".into()));
        } else {
            marks.push((r.start, 2, format!("{}«", star(s))));
            marks.push((r.end, 0, "»".into()));
        }
    }
    marks.sort_by_key(|m| (m.0, m.1));
    let mut out = String::new();
    let mut at = 0;
    for (offset, _, m) in marks {
        out.push_str(&text[at..offset]);
        out.push_str(&m);
        at = offset;
    }
    out.push_str(&text[at..]);
    out
}

/// The default config the command tests use: 4-wide indent with spaces.
pub fn test_config() -> crate::plan::EditConfig {
    crate::plan::EditConfig {
        tab_size: 4,
        insert_spaces: true,
        auto_indent: AutoIndent::Full,
        auto_closing_brackets: AutoClosing::LanguageDefined,
        auto_closing_quotes: AutoClosing::LanguageDefined,
        auto_closing_overtype: AutoClosingEdit::Auto,
        auto_closing_delete: AutoClosingEdit::Auto,
        auto_surround: AutoSurround::LanguageDefined,
        trim_auto_whitespace: true,
        word_separators: "`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?".into(),
        empty_selection_clipboard: true,
    }
}

/// Applies `plan` to `snapshot` and renders the result with its selections.
pub fn render_plan(snapshot: &BufferSnapshot, plan: &crate::plan::EditPlan) -> String {
    let mut text = snapshot.text_for_range(0..snapshot.len_bytes()).to_string();
    for (range, new) in plan.edits.iter().rev() {
        text.replace_range(range.clone(), new);
    }
    let after = Buffer::from_bytes(text.as_bytes(), false)
        .expect("result must load")
        .snapshot();
    render(&after, &plan.selections)
}
