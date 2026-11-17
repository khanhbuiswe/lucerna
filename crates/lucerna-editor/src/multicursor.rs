//! Multi-cursor commands that grow the selection set from find matches:
//! add / move the next or previous match (`⌘D`, `⌘K ⌘D`) and select all
//! matches (`⇧⌘L`); and cursor commands: cursors above / below, at line
//! ends, Escape helpers and mouse toggling. Pure functions over a snapshot
//! and a [`SelectionSet`].
//!
//! Adapted from microsoft/vscode src/vs/editor/contrib/multicursor/browser/multicursor.ts (MIT)

use std::ops::Range;

use lucerna_text::{BufferSnapshot, Point};

use crate::find::{FindQuery, find_all, find_next};
use crate::motion::Layout;
use crate::selection::{Goal, Selection, SelectionSet};
use crate::smart_select::word_around;

/// How matches are compared.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MatchOptions {
    pub case_sensitive: bool,
    pub whole_word: bool,
}

/// What a run of `⌘D` presses searches for.
#[derive(Clone, Debug, PartialEq)]
pub struct MatchSession {
    pub text: String,
    pub options: MatchOptions,
}

/// The result of one step.
pub struct Step {
    pub set: SelectionSet,
    pub session: Option<MatchSession>,
    /// The word a lone cursor was expanded to, for the Find bar.
    pub seeded_word: Option<String>,
    /// Results were dropped to stay within the cursor limit.
    pub limited: bool,
}

/// What the next search looks for.
enum Prepared {
    /// Empty selections were expanded to words; nothing is searched.
    Expanded(Step),
    Search(MatchSession),
    Nothing,
}

/// Options for a session started from cursors.
const WORD_OPTIONS: MatchOptions = MatchOptions {
    case_sensitive: true,
    whole_word: true,
};

fn query<'a>(s: &'a MatchSession, separators: &'a str) -> FindQuery<'a> {
    FindQuery {
        pattern: &s.text,
        is_regex: false,
        case_sensitive: s.options.case_sensitive,
        whole_word: s.options.whole_word,
        word_separators: separators,
    }
}

fn text_of(snap: &BufferSnapshot, r: Range<usize>) -> String {
    snap.text_for_range(r).to_string()
}

fn forward(range: Range<usize>, id: u64) -> Selection {
    Selection {
        id,
        anchor: range.start,
        head: range.end,
        goal: Goal::None,
    }
}

/// Decides what to search for, or expands empty selections to words.
fn prepare(
    snap: &BufferSnapshot,
    set: &SelectionSet,
    session: Option<&MatchSession>,
    find: MatchOptions,
    separators: &str,
) -> Prepared {
    if set.all().iter().any(|s| s.is_empty()) {
        let mut changed = false;
        let sels: Vec<Selection> = set
            .all()
            .iter()
            .map(|s| {
                if !s.is_empty() {
                    return *s;
                }
                match word_around(snap, &(s.head..s.head), separators) {
                    Some(w) => {
                        changed = true;
                        forward(w, s.id)
                    }
                    None => *s,
                }
            })
            .collect();
        if !changed {
            return Prepared::Nothing;
        }
        let single = set.len() == 1;
        let word = text_of(snap, sels[0].range());
        let session = single.then(|| MatchSession {
            text: word.clone(),
            options: WORD_OPTIONS,
        });
        return Prepared::Expanded(Step {
            set: set.rebuild(sels, set.primary().id),
            session,
            seeded_word: single.then_some(word),
            limited: false,
        });
    }
    if let Some(session) = session {
        return Prepared::Search(session.clone());
    }
    let text = text_of(snap, set.newest().range());
    if set
        .all()
        .iter()
        .any(|s| snap.text_for_range(s.range()) != text.as_str())
    {
        return Prepared::Nothing;
    }
    Prepared::Search(MatchSession {
        text,
        options: find,
    })
}

/// The first match from `from` that overlaps none of `taken`, stepping
/// past overlapping ones; `None` once the search wraps all the way round.
fn next_free(
    snap: &BufferSnapshot,
    q: &FindQuery,
    taken: &[Range<usize>],
    from: usize,
    backwards: bool,
) -> Option<Range<usize>> {
    let overlaps = |r: &Range<usize>| taken.iter().any(|t| r.start < t.end && t.start < r.end);
    let first = find_next(snap, q, from, backwards)?;
    let mut found = first.clone();
    for _ in 0..taken.len() * 2 + 4 {
        if !overlaps(&found) {
            return Some(found);
        }
        let from = if backwards { found.start } else { found.end };
        found = find_next(snap, q, from, backwards)?;
        if found == first {
            return None;
        }
    }
    None
}

/// `⌘D`: add the next (or, `backwards`, previous) match to the selection set
/// (the primary stays). With empty selections it only expands them to words.
pub fn add_match(
    snap: &BufferSnapshot,
    set: &SelectionSet,
    session: Option<&MatchSession>,
    find: MatchOptions,
    separators: &str,
    backwards: bool,
    limit: usize,
) -> Option<Step> {
    let new_session = match prepare(snap, set, session, find, separators) {
        Prepared::Expanded(step) => return Some(step),
        Prepared::Nothing => return None,
        Prepared::Search(s) => s,
    };
    let newest = set.newest().range();
    let from = if backwards { newest.start } else { newest.end };
    let taken: Vec<Range<usize>> = set.all().iter().map(Selection::range).collect();
    let found = next_free(
        snap,
        &query(&new_session, separators),
        &taken,
        from,
        backwards,
    )?;
    if set.len() >= limit {
        return Some(Step {
            set: set.clone(),
            session: Some(new_session),
            seeded_word: None,
            limited: true,
        });
    }
    let mut work = set.clone();
    let id = work.next_id();
    let mut sels = set.all().to_vec();
    sels.push(forward(found, id));
    Some(Step {
        set: work.rebuild(sels, set.primary().id),
        session: Some(new_session),
        seeded_word: None,
        limited: false,
    })
}

/// `⌘K ⌘D`: like [`add_match`], but the newest selection is dropped first.
pub fn move_match(
    snap: &BufferSnapshot,
    set: &SelectionSet,
    session: Option<&MatchSession>,
    find: MatchOptions,
    separators: &str,
    backwards: bool,
) -> Option<Step> {
    let new_session = match prepare(snap, set, session, find, separators) {
        Prepared::Expanded(step) => return Some(step),
        Prepared::Nothing => return None,
        Prepared::Search(s) => s,
    };
    let newest = *set.newest();
    let range = newest.range();
    let rest: Vec<Selection> = set
        .all()
        .iter()
        .filter(|s| s.id != newest.id)
        .copied()
        .collect();
    let taken: Vec<Range<usize>> = rest.iter().map(Selection::range).collect();
    let from = if backwards { range.start } else { range.end };
    let found = next_free(
        snap,
        &query(&new_session, separators),
        &taken,
        from,
        backwards,
    )?;
    if found == range {
        return None;
    }
    let mut work = set.clone();
    let id = work.next_id();
    let mut sels = rest;
    sels.push(forward(found, id));
    // The primary stays unless the dropped newest selection was it.
    let primary = if newest.id == set.primary().id {
        id
    } else {
        set.primary().id
    };
    Some(Step {
        set: work.rebuild(sels, primary),
        session: Some(new_session),
        seeded_word: None,
        limited: false,
    })
}

/// `⇧⌘L`: select every match of the selected text (or the word under a
/// cursor), keeping the match at the primary selection primary.
pub fn select_all_matches(
    snap: &BufferSnapshot,
    set: &SelectionSet,
    session: Option<&MatchSession>,
    find: MatchOptions,
    separators: &str,
    limit: usize,
) -> Option<Step> {
    let primary = *set.primary();
    let target = if primary.is_empty() {
        let word = word_around(snap, &(primary.head..primary.head), separators)?;
        MatchSession {
            text: text_of(snap, word),
            options: WORD_OPTIONS,
        }
    } else if let Some(session) = session {
        session.clone()
    } else {
        MatchSession {
            text: text_of(snap, primary.range()),
            options: find,
        }
    };
    let mut matches = find_all(snap, &query(&target, separators)).ok()?;
    if matches.is_empty() {
        return None;
    }
    let holds_primary = |m: &Range<usize>| m.start <= primary.head && primary.head <= m.end;
    let limit = limit.max(1);
    let limited = matches.len() > limit;
    if limited {
        let keep = matches
            .iter()
            .position(holds_primary)
            .map(|p| matches[p].clone());
        matches.truncate(limit);
        if let Some(keep) = keep
            && !matches.contains(&keep)
        {
            *matches.last_mut().expect("limit is at least one") = keep;
        }
    }
    let mut work = set.clone();
    let mut primary_id = None;
    let sels: Vec<Selection> = matches
        .into_iter()
        .map(|m| {
            let id = work.next_id();
            if holds_primary(&m) {
                primary_id.get_or_insert(id);
            }
            forward(m, id)
        })
        .collect();
    let primary_id = primary_id.unwrap_or(sels[0].id);
    Some(Step {
        set: work.rebuild(sels, primary_id),
        session: Some(target),
        seeded_word: None,
        limited,
    })
}

fn cursor(id: u64, offset: usize, goal: Goal) -> Selection {
    Selection {
        id,
        anchor: offset,
        head: offset,
        goal,
    }
}

/// `⌥⌘↑` / `⌥⌘↓`: every selection gets a cursor one display row above (or
/// below) its head, keeping its goal column. A cursor that would stay on its
/// source's row (the first or last row) is not added. Returns the set, the
/// ids of the cursors actually added (one layer), and whether the limit cut
/// some off. The primary stays.
pub fn insert_cursors_vertical(
    snap: &BufferSnapshot,
    set: &SelectionSet,
    up: bool,
    layout: &dyn Layout,
    limit: usize,
) -> (SelectionSet, Vec<u64>, bool) {
    let rows = if up { -1 } else { 1 };
    let len = snap.len_bytes();
    let mut work = set.clone();
    let mut sels = set.all().to_vec();
    let mut added = Vec::new();
    let mut limited = false;
    for s in set.all() {
        let (offset, goal) = layout.vertical(snap, s.head, s.goal, rows);
        let boundary = offset == 0 || offset == len;
        let same_row = snap.offset_to_point(offset).row == snap.offset_to_point(s.head).row;
        if offset == s.head || (boundary && same_row) {
            continue;
        }
        if sels.len() >= limit {
            limited = true;
            break;
        }
        let id = work.next_id();
        sels.push(cursor(id, offset, goal));
        added.push(id);
    }
    if added.is_empty() {
        return (set.clone(), added, limited);
    }
    let out = work.rebuild(sels, set.primary().id);
    added.retain(|id| out.all().iter().any(|s| s.id == *id));
    (out, added, limited)
}

/// Drops the selections with these ids (never the last one). A dropped
/// primary gives way to the newest remaining selection.
pub fn remove_ids(set: &SelectionSet, ids: &[u64]) -> SelectionSet {
    let rest: Vec<Selection> = set
        .all()
        .iter()
        .filter(|s| !ids.contains(&s.id))
        .copied()
        .collect();
    if rest.is_empty() || rest.len() == set.len() {
        return set.clone();
    }
    let primary = set.primary().id;
    let primary = if rest.iter().any(|s| s.id == primary) {
        primary
    } else {
        rest.iter().max_by_key(|s| s.id).map_or(primary, |s| s.id)
    };
    set.rebuild(rest, primary)
}

/// `⇧⌥I`: a cursor at the end of every line of every non-empty selection;
/// the last line only when the selection ends past its start. Empty
/// selections are dropped. `None` when no cursor results. The first cursor
/// of the old primary stays primary; when it was empty, the first cursor is.
pub fn cursors_at_line_ends(
    snap: &BufferSnapshot,
    set: &SelectionSet,
    limit: usize,
) -> Option<(SelectionSet, bool)> {
    let mut work = set.clone();
    let old_primary = set.primary().id;
    let mut sels: Vec<Selection> = Vec::new();
    let mut primary = None;
    for s in set.all().iter().filter(|s| !s.is_empty()) {
        let r = s.range();
        let (start, end) = (snap.offset_to_point(r.start), snap.offset_to_point(r.end));
        let mut ends: Vec<usize> = (start.row..end.row)
            .map(|row| snap.point_to_offset(Point { row, column: 0 }) + snap.line(row).len())
            .collect();
        if end.column > 0 {
            ends.push(r.end);
        }
        for offset in ends {
            let id = work.next_id();
            if s.id == old_primary {
                primary.get_or_insert(id);
            }
            sels.push(cursor(id, offset, Goal::None));
        }
    }
    if sels.is_empty() {
        return None;
    }
    let cap = limit.max(1);
    let limited = sels.len() > cap;
    sels.truncate(cap);
    let primary = primary
        .filter(|p| sels.iter().any(|s| s.id == *p))
        .unwrap_or(sels[0].id);
    Some((work.rebuild(sels, primary), limited))
}

/// First Escape step: only the primary selection remains. `None` when it
/// already is the only one.
pub fn remove_secondary(set: &SelectionSet) -> Option<SelectionSet> {
    if set.len() < 2 {
        return None;
    }
    let primary = *set.primary();
    Some(set.rebuild(vec![primary], primary.id))
}

/// Second Escape step: the lone selection collapses to a cursor at its
/// head. `None` for an empty one (or several selections).
pub fn cancel_selection(set: &SelectionSet) -> Option<SelectionSet> {
    let only = set.primary();
    if set.len() != 1 || only.is_empty() {
        return None;
    }
    Some(set.rebuild(vec![cursor(only.id, only.head, Goal::None)], only.id))
}

/// Alt-click: removes the selection touching `offset` (never the last one),
/// otherwise adds a cursor there as primary. At the limit nothing is added.
pub fn toggle_cursor_at(set: &SelectionSet, offset: usize, limit: usize) -> SelectionSet {
    let hit = set.all().iter().find(|s| {
        let r = s.range();
        r.start <= offset && offset <= r.end
    });
    if let Some(hit) = hit {
        return remove_ids(set, &[hit.id]);
    }
    if set.len() >= limit {
        return set.clone();
    }
    let mut work = set.clone();
    let id = work.next_id();
    let mut sels = set.all().to_vec();
    sels.push(cursor(id, offset, Goal::None));
    work.rebuild(sels, id)
}

/// The id [`add_selection`] returns when the limit stopped it.
pub const NO_SELECTION: u64 = u64::MAX;

/// Alt-drag and alt double / triple-click: adds the selection
/// `anchor..head` as primary and returns its id so a drag can keep updating
/// it (it may be gone after a merge). At the limit nothing is added and the
/// id is [`NO_SELECTION`].
pub fn add_selection(
    set: &SelectionSet,
    anchor: usize,
    head: usize,
    limit: usize,
) -> (SelectionSet, u64) {
    if set.len() >= limit {
        return (set.clone(), NO_SELECTION);
    }
    let mut work = set.clone();
    let id = work.next_id();
    let mut sels = set.all().to_vec();
    sels.push(Selection {
        id,
        anchor,
        head,
        goal: Goal::None,
    });
    (work.rebuild(sels, id), id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{marked, render};

    const SEPS: &str = "`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?";
    const LOOSE: MatchOptions = MatchOptions {
        case_sensitive: false,
        whole_word: false,
    };

    fn add(
        src: &str,
        session: Option<&MatchSession>,
        find: MatchOptions,
        back: bool,
    ) -> Option<Step> {
        let (snap, set) = marked(src);
        add_match(&snap, &set, session, find, SEPS, back, 10_000)
    }

    fn shown(src: &str, step: &Step) -> String {
        render(&marked(src).0, &step.set)
    }

    #[test]
    fn add_first_expands_words() {
        let src = "«ab»c x|y |z";
        let step = add(src, None, LOOSE, false).unwrap();
        assert_eq!(shown(src, &step), "«ab»c «xy» «z»");
        assert_eq!(step.seeded_word, None);
        assert_eq!(step.session, None);
    }

    #[test]
    fn add_first_single_cursor_seeds() {
        let src = "fo|o bar foo";
        let step = add(src, None, LOOSE, false).unwrap();
        assert_eq!(shown(src, &step), "«foo» bar foo");
        assert_eq!(step.seeded_word.as_deref(), Some("foo"));
        assert_eq!(
            step.session,
            Some(MatchSession {
                text: "foo".into(),
                options: WORD_OPTIONS
            })
        );
    }

    #[test]
    fn add_next_wraps_skips_overlap() {
        let src = "foo «foo» foo";
        let (snap, set) = marked(src);
        let s1 = add_match(&snap, &set, None, LOOSE, SEPS, false, 100).unwrap();
        assert_eq!(render(&snap, &s1.set), "foo «foo» «foo»");
        assert_eq!(s1.set.primary().range(), 4..7);
        let s2 = add_match(&snap, &s1.set, s1.session.as_ref(), LOOSE, SEPS, false, 100).unwrap();
        assert_eq!(s2.set.len(), 3);
        assert_eq!(s2.set.primary().range(), 4..7);
        let s3 = add_match(&snap, &s2.set, s2.session.as_ref(), LOOSE, SEPS, false, 100);
        assert!(s3.is_none());
    }

    #[test]
    fn add_from_selection_uses_find_options() {
        let src = "«Foo» foo FOO";
        let step = add(src, None, LOOSE, false).unwrap();
        assert_eq!(shown(src, &step), "«Foo» «foo» FOO");
        let strict = MatchOptions {
            case_sensitive: true,
            whole_word: false,
        };
        assert!(add(src, None, strict, false).is_none());
    }

    #[test]
    fn add_mismatched_text_noop() {
        assert!(add("«ab» «cd»", None, LOOSE, false).is_none());
    }

    #[test]
    fn add_previous() {
        let src = "foo foo «foo»";
        let step = add(src, None, LOOSE, true).unwrap();
        assert_eq!(shown(src, &step), "foo «foo» *«foo»");
    }

    #[test]
    fn move_replaces_newest() {
        let (snap, mut set) = marked("«foo» «foo» foo");
        let id = set.next_id();
        let mut sels = set.all().to_vec();
        sels[1].id = id;
        let set = set.rebuild(sels, set.primary().id);
        assert_eq!(set.newest().range(), 4..7);
        let step = move_match(&snap, &set, None, LOOSE, SEPS, false).unwrap();
        assert_eq!(render(&snap, &step.set), "«foo» foo «foo»");
        assert_eq!(step.set.primary().range(), 0..3);
    }

    #[test]
    fn move_single_selection_new_match_is_primary() {
        let (snap, set) = marked("«foo» foo");
        let step = move_match(&snap, &set, None, LOOSE, SEPS, false).unwrap();
        assert_eq!(step.set.len(), 1);
        assert_eq!(step.set.primary().range(), 4..7);
    }

    #[test]
    fn add_keeps_primary_with_several_selections() {
        let (snap, set) = marked("*«foo» «foo» foo");
        let primary = set.primary().range();
        let step = add_match(&snap, &set, None, LOOSE, SEPS, false, 100).unwrap();
        assert_eq!(step.set.len(), 3);
        assert_eq!(step.set.primary().range(), primary);
    }

    #[test]
    fn select_all_keeps_primary() {
        let (snap, set) = marked("x foo *«foo» foo");
        let step = select_all_matches(&snap, &set, None, LOOSE, SEPS, 100).unwrap();
        assert_eq!(step.set.len(), 3);
        assert_eq!(step.set.primary().range(), 6..9);
        assert_eq!(render(&snap, &step.set), "x «foo» *«foo» «foo»");
    }

    #[test]
    fn add_next_vietnamese_whole_word() {
        let (snap, set) = marked("chào| chàobạn chào");
        let s1 = add_match(&snap, &set, None, LOOSE, SEPS, false, 100).unwrap();
        assert_eq!(s1.set.primary().range(), 0..5);
        let s2 = add_match(&snap, &s1.set, s1.session.as_ref(), LOOSE, SEPS, false, 100).unwrap();
        let got: Vec<_> = s2.set.all().iter().map(Selection::range).collect();
        assert_eq!(got, vec![0..5, 17..22]);
        let s3 = add_match(&snap, &s2.set, s2.session.as_ref(), LOOSE, SEPS, false, 100);
        assert!(s3.is_none());
    }

    use crate::motion::PlainLayout;

    const PLAIN: PlainLayout = PlainLayout {
        tab_size: 4,
        page_rows: 10,
    };

    #[test]
    fn insert_below_every_selection_keeps_goal() {
        let (snap, set) = marked("ab|cdef\nx\nabcdef\n");
        let (s1, ids1, limited) = insert_cursors_vertical(&snap, &set, false, &PLAIN, 100);
        assert!(!limited);
        assert_eq!(ids1.len(), 1);
        assert_eq!(render(&snap, &s1), "ab|cdef\nx|\nabcdef\n");
        let added = s1.all().iter().find(|s| s.id == ids1[0]).unwrap();
        assert_eq!(added.goal, Goal::Column(2.0));
        assert_eq!(s1.primary().id, set.primary().id);
        // Again from every selection: the `x` cursor reaches (2, 2) with
        // the kept goal; the first one adds a duplicate and merges.
        let (s2, ids2, _) = insert_cursors_vertical(&snap, &s1, false, &PLAIN, 100);
        assert_eq!(render(&snap, &s2), "ab|cdef\nx|\nab|cdef\n");
        assert_eq!(ids2.len(), 1);
        // Two starting cursors on rows 0 and 3 give four.
        let (snap, set) = marked("a|b\nx\ny\na|b\nx\n");
        let (s, ids, _) = insert_cursors_vertical(&snap, &set, false, &PLAIN, 100);
        assert_eq!(ids.len(), 2);
        assert_eq!(s.len(), 4);
    }

    #[test]
    fn insert_then_remove_layer() {
        let (snap, set) = marked("a|b\ncd\nef");
        let (s, ids, _) = insert_cursors_vertical(&snap, &set, false, &PLAIN, 100);
        assert_eq!(s.len(), 2);
        let back = remove_ids(&s, &ids);
        assert_eq!(back.all(), set.all());
        assert_eq!(back.primary().id, set.primary().id);
    }

    #[test]
    fn insert_at_edge_noop() {
        let (snap, set) = marked("ab\nc|d");
        let (s, ids, limited) = insert_cursors_vertical(&snap, &set, false, &PLAIN, 100);
        assert_eq!(s, set);
        assert!(ids.is_empty() && !limited);
        let (snap, set) = marked("a|b\ncd");
        let (s, ids, _) = insert_cursors_vertical(&snap, &set, true, &PLAIN, 100);
        assert_eq!(s, set);
        assert!(ids.is_empty());
    }

    #[test]
    fn insert_respects_limit() {
        let (snap, set) = marked("a|b\ncd\nef\ng|h\nij");
        let (s, ids, limited) = insert_cursors_vertical(&snap, &set, false, &PLAIN, 3);
        assert!(limited);
        assert_eq!(ids.len(), 1);
        assert_eq!(s.len(), 3);
        assert_eq!(s.primary().id, set.primary().id);
    }

    #[test]
    fn line_ends_vscode() {
        let (snap, set) = marked("«ab\ncd\nef»gh");
        let (s, limited) = cursors_at_line_ends(&snap, &set, 100).unwrap();
        assert!(!limited);
        assert_eq!(render(&snap, &s), "ab|\ncd|\nef|gh");
        let (snap, set) = marked("«ab\n»cd");
        let (s, _) = cursors_at_line_ends(&snap, &set, 100).unwrap();
        assert_eq!(render(&snap, &s), "ab|\ncd");
        let (_, set) = marked("a|b c|d");
        assert!(cursors_at_line_ends(&snap, &set, 100).is_none());
        let (snap, set) = marked("«ab\ncd»");
        let (s, limited) = cursors_at_line_ends(&snap, &set, 1).unwrap();
        assert!(limited);
        assert_eq!(render(&snap, &s), "ab|\ncd");
    }

    #[test]
    fn line_ends_drops_empty_and_picks_primary() {
        let (snap, set) = marked("|x *«a\nb» «c»");
        let (s, _) = cursors_at_line_ends(&snap, &set, 100).unwrap();
        assert_eq!(render(&snap, &s), "x a|\nb| c|");
        // The old primary's first cursor stays primary.
        assert_eq!(s.primary().head, 3);
        let (snap, set) = marked("|x «a\nb»");
        let (s, _) = cursors_at_line_ends(&snap, &set, 100).unwrap();
        assert_eq!(render(&snap, &s), "x a|\nb|");
        assert_eq!(s.primary().id, s.all()[0].id);
    }

    #[test]
    fn escape_two_steps() {
        let (snap, set) = marked("«a» *«b»");
        let one = remove_secondary(&set).unwrap();
        assert_eq!(render(&snap, &one), "a «b»");
        assert!(remove_secondary(&one).is_none());
        let cur = cancel_selection(&one).unwrap();
        assert_eq!(render(&snap, &cur), "a b|");
        assert!(cancel_selection(&cur).is_none());
        assert!(remove_secondary(&cur).is_none());
    }

    #[test]
    fn toggle_adds_and_removes() {
        let (snap, set) = marked("a|bc de");
        let added = toggle_cursor_at(&set, 5, 100);
        assert_eq!(added.len(), 2);
        assert_eq!(added.primary().head, 5);
        let removed = toggle_cursor_at(&added, 5, 100);
        assert_eq!(render(&snap, &removed), "a|bc de");
        assert_eq!(removed.primary().head, 1);
        // The only selection stays.
        assert_eq!(toggle_cursor_at(&set, 1, 100).all(), set.all());
        // At the limit nothing is added.
        assert_eq!(toggle_cursor_at(&set, 5, 1).len(), 1);
    }

    #[test]
    fn add_selection_is_primary() {
        let (snap, set) = marked("a|bc de");
        let (s, id) = add_selection(&set, 4, 6, 100);
        assert_eq!(render(&snap, &s), "a|bc *«de»");
        assert_eq!(s.primary().id, id);
        let (s, id) = add_selection(&set, 4, 6, 1);
        assert_eq!(s, set);
        assert_eq!(id, NO_SELECTION);
    }

    #[test]
    fn limit_caps() {
        let (snap, set) = marked("«a» a a a a");
        let step = select_all_matches(&snap, &set, None, LOOSE, SEPS, 2).unwrap();
        assert_eq!(step.set.len(), 2);
        assert!(step.limited);
        let step = select_all_matches(&snap, &set, None, LOOSE, SEPS, 5).unwrap();
        assert_eq!(step.set.len(), 5);
        assert!(!step.limited);
    }
}
