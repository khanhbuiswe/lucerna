//! Multi-cursor commands that grow the selection set from find matches:
//! add / move the next or previous match (`⌘D`, `⌘K ⌘D`) and select all
//! matches (`⇧⌘L`). Pure functions over a snapshot and a [`SelectionSet`].
//!
//! Adapted from microsoft/vscode src/vs/editor/contrib/multicursor/browser/multicursor.ts (MIT)

use std::ops::Range;

use lucerna_text::BufferSnapshot;

use crate::find::{FindQuery, find_all, find_next};
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

/// `⌘D`: add the next (or, `backwards`, previous) match as the new primary
/// selection. With empty selections it only expands them to words.
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
        set: work.rebuild(sels, id),
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
    Some(Step {
        set: work.rebuild(sels, id),
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
        assert_eq!(render(&snap, &s1.set), "foo «foo» *«foo»");
        let s2 = add_match(&snap, &s1.set, s1.session.as_ref(), LOOSE, SEPS, false, 100).unwrap();
        assert_eq!(s2.set.len(), 3);
        assert_eq!(s2.set.primary().range(), 0..3);
        let s3 = add_match(&snap, &s2.set, s2.session.as_ref(), LOOSE, SEPS, false, 100);
        assert!(s3.is_none());
    }

    #[test]
    fn add_from_selection_uses_find_options() {
        let src = "«Foo» foo FOO";
        let step = add(src, None, LOOSE, false).unwrap();
        assert_eq!(shown(src, &step), "«Foo» *«foo» FOO");
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
        assert_eq!(shown(src, &step), "foo «foo» «foo»");
    }

    #[test]
    fn move_replaces_newest() {
        let (snap, mut set) = marked("«foo» «foo» foo");
        let id = set.next_id();
        let mut sels = set.all().to_vec();
        sels[1].id = id;
        let set = set.rebuild(sels, id);
        assert_eq!(set.newest().range(), 4..7);
        let step = move_match(&snap, &set, None, LOOSE, SEPS, false).unwrap();
        assert_eq!(render(&snap, &step.set), "«foo» foo *«foo»");
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
