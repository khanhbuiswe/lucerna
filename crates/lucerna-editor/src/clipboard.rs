//! Copy, cut and paste over selections.
//!
//! Adapted from microsoft/vscode src/vs/editor/common/cursor/cursorTypeEditOperations.ts (paste) (MIT)
//! Adapted from microsoft/vscode src/vs/editor/common/cursor/cursorDeleteOperations.ts (cut) (MIT)
//! Adapted from microsoft/vscode src/vs/editor/common/viewModel/viewModelImpl.ts (copy) (MIT)

use std::ops::Range;

use lucerna_text::BufferSnapshot;

use crate::commands::{Item, Place, build, row_start};
use crate::plan::{Ctx, EditPlan, MultiCursorPaste};
use crate::selection::{Selection, SelectionSet};
use crate::undo_stops::EditKind;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClipboardText {
    pub text: String,
    /// One piece per selection when copied from several.
    pub pieces: Option<Vec<String>>,
    /// The text is a whole line taken from an empty selection.
    pub whole_line: bool,
}

/// `(selection id, range to copy, range to delete)`.
type Source = (u64, Range<usize>, Range<usize>);

/// One source per selection. Only when every selection is empty does an
/// empty one take its whole line (one per distinct line); next to a
/// non-empty selection it contributes an empty piece.
fn sources(ctx: Ctx, set: &SelectionSet) -> (Vec<Source>, bool) {
    let snap = ctx.snapshot;
    let mut out = Vec::new();
    let mut last_row = None;
    let all_empty = set.all().iter().all(Selection::is_empty);
    let mut whole = all_empty;
    for s in set.all() {
        if !all_empty {
            whole = false;
            out.push((s.id, s.range(), s.range()));
        } else if ctx.config.empty_selection_clipboard {
            let row = snap.offset_to_point(s.head).row;
            if last_row == Some(row) {
                continue;
            }
            last_row = Some(row);
            let start = row_start(snap, row);
            let end = if row + 1 < snap.line_count() {
                row_start(snap, row + 1)
            } else {
                snap.len_bytes()
            };
            // The last line takes the break before it, so the line goes away.
            let del_start =
                if row > 0 && start < end && !snap.text_for_range(start..end).ends_with('\n') {
                    start - 1
                } else {
                    start
                };
            out.push((s.id, start..end, del_start..end));
        }
    }
    let whole = whole && !out.is_empty();
    (out, whole)
}

fn collect(snap: &BufferSnapshot, ranges: &[Source], whole: bool) -> ClipboardText {
    let mut pieces: Vec<String> = ranges
        .iter()
        .map(|(_, r, _)| snap.text_for_range(r.clone()).to_string())
        .collect();
    if whole {
        for p in &mut pieces {
            if !p.ends_with('\n') {
                p.push('\n');
            }
        }
    }
    ClipboardText {
        text: pieces.join("\n"),
        pieces: (pieces.len() > 1).then_some(pieces),
        whole_line: whole,
    }
}

pub fn copy(ctx: Ctx, set: &SelectionSet) -> ClipboardText {
    let (ranges, whole) = sources(ctx, set);
    collect(ctx.snapshot, &ranges, whole)
}

pub fn cut(ctx: Ctx, set: &SelectionSet) -> (EditPlan, ClipboardText) {
    let (ranges, whole) = sources(ctx, set);
    let clip = collect(ctx.snapshot, &ranges, whole);
    let mut items: Vec<Item> = ranges
        .into_iter()
        .map(|(id, _, range)| Item {
            id,
            range,
            text: String::new(),
            place: Place::End,
        })
        .collect();
    // Every other selection (a duplicate line, or an empty one with
    // `empty_selection_clipboard` off) still needs a cursor.
    for s in set.all() {
        if items.iter().any(|i| i.id == s.id) {
            continue;
        }
        // Whole-line mode only applies when every selection was empty.
        let at = if ctx.config.empty_selection_clipboard {
            row_start(ctx.snapshot, ctx.snapshot.offset_to_point(s.head).row)
        } else {
            s.head
        };
        items.push(Item {
            id: s.id,
            range: at..at,
            text: String::new(),
            place: Place::End,
        });
    }
    (build(items, set, EditKind::Other, true, true), clip)
}

/// One piece of text per selection, or `None` when each gets the whole text
/// (`editor.multiCursorPaste`). Lines split only when there are as many as
/// selections, ignoring one trailing line break.
fn spread<'a>(
    ctx: Ctx,
    set: &SelectionSet,
    clip: &'a ClipboardText,
) -> Option<Vec<std::borrow::Cow<'a, str>>> {
    use std::borrow::Cow;
    let n = set.len();
    if n < 2 {
        return None;
    }
    // Pieces from a multi-cursor copy spread whatever the setting says;
    // `full` only stops external text from being split by line.
    if let Some(pieces) = &clip.pieces {
        return (pieces.len() == n)
            .then(|| pieces.iter().map(|p| Cow::Borrowed(p.as_str())).collect());
    }
    if clip.whole_line || ctx.config.multi_cursor_paste == MultiCursorPaste::Full {
        return None;
    }
    let text = clip.text.replace("\r\n", "\n").replace('\r', "\n");
    let text = text.strip_suffix('\n').unwrap_or(&text);
    let lines: Vec<Cow<str>> = text
        .split('\n')
        .map(|l| Cow::Owned(l.to_string()))
        .collect();
    (lines.len() == n).then_some(lines)
}

pub fn paste(ctx: Ctx, set: &SelectionSet, clip: &ClipboardText) -> EditPlan {
    let snap = ctx.snapshot;
    let normalize = |t: &str| t.replace("\r\n", "\n").replace('\r', "\n");
    let per_selection = spread(ctx, set, clip);
    let items = set
        .all()
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let text = normalize(
                per_selection
                    .as_ref()
                    .map_or(clip.text.as_str(), |p| &*p[i]),
            );
            if clip.whole_line && s.is_empty() {
                let at = row_start(snap, snap.offset_to_point(s.head).row);
                Item {
                    id: s.id,
                    range: at..at,
                    text,
                    place: Place::Keep(s.head),
                }
            } else {
                Item {
                    id: s.id,
                    range: s.range(),
                    text,
                    place: Place::End,
                }
            }
        })
        .collect();
    build(items, set, EditKind::Other, true, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{marked, render_plan, test_config};

    fn ctx_run<T>(src: &str, f: impl FnOnce(Ctx, &SelectionSet, &BufferSnapshot) -> T) -> T {
        let (snap, set) = marked(src);
        let cfg = test_config();
        f(
            Ctx {
                snapshot: &snap,
                config: &cfg,
                language: None,
                syntax: None,
            },
            &set,
            &snap,
        )
    }

    fn clip(text: &str) -> ClipboardText {
        ClipboardText {
            text: text.into(),
            pieces: None,
            whole_line: false,
        }
    }

    #[test]
    fn copy_cases() {
        let c = ctx_run("a«bc»d", |c, s, _| copy(c, s));
        assert_eq!(c, clip("bc"));
        let c = ctx_run("a\nb|c\nd", |c, s, _| copy(c, s));
        assert_eq!((c.text.as_str(), c.whole_line), ("bc\n", true));
        let c = ctx_run("«a»b«c»d", |c, s, _| copy(c, s));
        assert_eq!(c.text, "a\nc");
        assert_eq!(c.pieces, Some(vec!["a".into(), "c".into()]));
        let c = ctx_run("a|b", |c, s, _| {
            let mut cfg = c.config.clone();
            cfg.empty_selection_clipboard = false;
            copy(Ctx { config: &cfg, ..c }, s)
        });
        assert_eq!(c.text, "");
    }

    #[test]
    fn cut_cases() {
        let (out, c) = ctx_run("a\nb|c\nd", |c, s, snap| {
            let (p, c) = cut(c, s);
            (render_plan(snap, &p), c)
        });
        assert_eq!(out, "a\n|d");
        assert_eq!((c.text.as_str(), c.whole_line), ("bc\n", true));
        let (out, c) = ctx_run("a«bc»d", |c, s, snap| {
            let (p, c) = cut(c, s);
            (render_plan(snap, &p), c)
        });
        assert_eq!(out, "a|d");
        assert_eq!(c, clip("bc"));
        let (out, c) = ctx_run("«a»b«c»d", |c, s, snap| {
            let (p, c) = cut(c, s);
            (render_plan(snap, &p), c)
        });
        assert_eq!(out, "|b|d");
        assert_eq!(c.pieces.as_ref().map(Vec::len), Some(2));
        let (out, _) = ctx_run("a|x\nb|y", |c, s, snap| {
            let (p, c) = cut(c, s);
            (render_plan(snap, &p), c)
        });
        assert_eq!(out, "|");
    }

    #[test]
    fn cut_last_line_takes_the_break_before_it() {
        let (out, c) = ctx_run("a\nb|", |c, s, snap| {
            let (p, c) = cut(c, s);
            (render_plan(snap, &p), c)
        });
        assert_eq!(out, "a|");
        assert_eq!((c.text.as_str(), c.whole_line), ("b\n", true));
    }

    #[test]
    fn cut_keeps_empty_selections_when_clipboard_line_is_off() {
        let out = ctx_run("«a»b|c", |c, s, snap| {
            let mut cfg = c.config.clone();
            cfg.empty_selection_clipboard = false;
            let (p, _) = cut(Ctx { config: &cfg, ..c }, s);
            render_plan(snap, &p)
        });
        assert_eq!(out, "|b|c");
    }

    #[test]
    fn paste_cases() {
        let go = |src: &str, c: &ClipboardText| {
            ctx_run(src, |cx, s, snap| render_plan(snap, &paste(cx, s, c)))
        };
        assert_eq!(go("a|b", &clip("xy")), "axy|b");
        assert_eq!(go("a«bc»d", &clip("x")), "ax|d");
        assert_eq!(go("a|\nb|", &clip("x")), "ax|\nbx|");
        assert_eq!(go("a|b", &clip("x\r\ny")), "ax\ny|b");
        let whole = ClipboardText {
            text: "bc\n".into(),
            pieces: None,
            whole_line: true,
        };
        assert_eq!(go("x|y", &whole), "bc\nx|y");
        let two = ClipboardText {
            text: "1\n2".into(),
            pieces: Some(vec!["1".into(), "2".into()]),
            whole_line: false,
        };
        assert_eq!(go("a|\nb|", &two), "a1|\nb2|");
        let mismatch = ClipboardText {
            text: "1\n2".into(),
            pieces: Some(vec!["1".into(), "2".into()]),
            whole_line: false,
        };
        assert_eq!(go("|", &mismatch), "1\n2|");
    }

    fn paste_with(src: &str, c: &ClipboardText, mode: MultiCursorPaste) -> String {
        ctx_run(src, |cx, s, snap| {
            let mut cfg = cx.config.clone();
            cfg.multi_cursor_paste = mode;
            render_plan(snap, &paste(Ctx { config: &cfg, ..cx }, s, c))
        })
    }

    #[test]
    fn copy_mixed_empty_gives_empty_piece() {
        let c = ctx_run("«ab» c|d", |c, s, _| copy(c, s));
        assert_eq!(c.pieces, Some(vec!["ab".into(), String::new()]));
        assert!(!c.whole_line);
    }

    #[test]
    fn copy_all_empty_whole_lines() {
        let c = ctx_run(
            "a|
b|",
            |c, s, _| copy(c, s),
        );
        assert_eq!(
            c.pieces,
            Some(vec![
                "a
"
                .into(),
                "b
"
                .into()
            ])
        );
        assert!(c.whole_line);
    }

    #[test]
    fn cut_mixed_empty_deletes_only_ranges() {
        let out = ctx_run("«ab» c|d", |c, s, snap| {
            let (p, _) = cut(c, s);
            render_plan(snap, &p)
        });
        assert_eq!(out, "| c|d");
    }

    #[test]
    fn paste_external_lines_spread() {
        let out = paste_with(
            "a|
b|",
            &clip(
                "x
y",
            ),
            MultiCursorPaste::Spread,
        );
        assert_eq!(
            out,
            "ax|
by|"
        );
        let out = paste_with(
            "a|
b|",
            &clip(
                "x
y
",
            ),
            MultiCursorPaste::Spread,
        );
        assert_eq!(
            out,
            "ax|
by|"
        );
    }

    #[test]
    fn paste_full() {
        let two = ClipboardText {
            text: "1
2"
            .into(),
            pieces: Some(vec!["1".into(), "2".into()]),
            whole_line: false,
        };
        let out = paste_with(
            "a|
b|",
            &two,
            MultiCursorPaste::Full,
        );
        assert_eq!(
            out,
            "a1|
b2|"
        );
        // A piece count that does not match: the whole text everywhere.
        let three = ClipboardText {
            text: "1
2
3"
            .into(),
            pieces: Some(vec!["1".into(), "2".into(), "3".into()]),
            whole_line: false,
        };
        let out = paste_with(
            "a|
b|",
            &three,
            MultiCursorPaste::Full,
        );
        assert_eq!(
            out,
            "a1
2
3|
b1
2
3|"
        );
        // External text is not split by line under `full`.
        let out = paste_with(
            "a|
b|",
            &clip(
                "x
y",
            ),
            MultiCursorPaste::Full,
        );
        assert_eq!(
            out,
            "ax
y|
bx
y|"
        );
        let out = paste_with(
            "a|
b|",
            &clip(
                "x
y",
            ),
            MultiCursorPaste::Full,
        );
        assert_eq!(
            out,
            "ax
y|
bx
y|"
        );
    }

    #[test]
    fn paste_lines_count_mismatch_full_text() {
        let out = paste_with(
            "a|
b|",
            &clip(
                "x
y
z",
            ),
            MultiCursorPaste::Spread,
        );
        assert_eq!(
            out,
            "ax
y
z|
bx
y
z|"
        );
    }

    #[test]
    fn paste_whole_line_clip_without_pieces_is_not_split() {
        let whole = ClipboardText {
            text: "a\nb\n".into(),
            pieces: None,
            whole_line: true,
        };
        let out = paste_with("x|\ny|", &whole, MultiCursorPaste::Spread);
        assert_eq!(out, "a\nb\nx|\na\nb\ny|");
    }

    #[test]
    fn plan_flags() {
        let p = ctx_run("a|b", |c, s, _| paste(c, s, &clip("x")));
        assert_eq!(p.kind, EditKind::Other);
        assert!(p.stop_before && p.stop_after);
    }
}
