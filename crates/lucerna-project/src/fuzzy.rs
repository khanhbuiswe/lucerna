//! Fuzzy file matching for Quick Open.

use crate::{EntryKind, WorktreeSnapshot};
use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::canonical_combining_class;

pub const QUICK_OPEN_LIMIT: usize = 50;

/// How many files to examine between cancellation checks.
const CANCEL_STRIDE: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMatch {
    pub path: Arc<Path>,
    pub score: u32,
    /// Char indices into `path.to_string_lossy()`, ascending, deduplicated.
    pub positions: Vec<u32>,
}

/// Matches `query` against the files of `snapshot`, best first.
///
/// An empty query lists `recent` files that still exist, then the rest by
/// path. Otherwise recent files come first (in the order given), then by
/// descending score, then ascending path length. Returns an empty Vec if
/// `cancel` is set while working.
pub fn match_files(
    snapshot: &WorktreeSnapshot,
    query: &str,
    recent: &[Arc<Path>],
    limit: usize,
    cancel: &AtomicBool,
) -> Vec<FileMatch> {
    if limit == 0 || cancel.load(AtomicOrdering::Relaxed) {
        return Vec::new();
    }
    if query.trim().is_empty() {
        empty_query(snapshot, recent, limit, cancel)
    } else {
        fuzzy_query(snapshot, query, recent, limit, cancel)
    }
}

fn empty_query(
    snapshot: &WorktreeSnapshot,
    recent: &[Arc<Path>],
    limit: usize,
    cancel: &AtomicBool,
) -> Vec<FileMatch> {
    let mut seen: HashSet<&Path> = HashSet::new();
    let mut out: Vec<FileMatch> = Vec::new();
    for path in recent {
        if out.len() >= limit {
            break;
        }
        if is_file(snapshot, path) && seen.insert(path) {
            out.push(plain(path));
        }
    }
    if out.len() >= limit {
        return out;
    }
    let mut rest: Vec<&Arc<Path>> = Vec::new();
    for (i, entry) in snapshot.entries().iter().enumerate() {
        if i % CANCEL_STRIDE == 0 && cancel.load(AtomicOrdering::Relaxed) {
            return Vec::new();
        }
        if entry.kind == EntryKind::File && !seen.contains(&*entry.path) {
            rest.push(&entry.path);
        }
    }
    rest.sort_unstable();
    out.extend(rest.into_iter().take(limit - out.len()).map(plain));
    out
}

fn plain(path: &Arc<Path>) -> FileMatch {
    FileMatch {
        path: path.clone(),
        score: 0,
        positions: Vec::new(),
    }
}

fn is_file(snapshot: &WorktreeSnapshot, path: &Path) -> bool {
    snapshot
        .children(path.parent().unwrap_or(Path::new("")))
        .iter()
        .any(|e| &*e.path == path && e.kind == EntryKind::File)
}

struct Candidate<'a> {
    path: &'a Arc<Path>,
    score: u32,
    len: usize,
    recent_rank: Option<usize>,
}

fn compare(a: &Candidate, b: &Candidate) -> Ordering {
    match (a.recent_rank, b.recent_rank) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => b
            .score
            .cmp(&a.score)
            .then(a.len.cmp(&b.len))
            .then_with(|| a.path.cmp(b.path)),
    }
}

/// `text` as NFC chars in `chars`, so NFD and NFC spellings match each
/// other. `spans[i]` is the char range of `text` that `chars[i]` came from
/// (a combining sequence maps to its whole range). Matching on chars rather
/// than letting nucleo split graphemes keeps positions valid char indices.
fn nfc_chars(text: &str, chars: &mut Vec<char>, spans: &mut Vec<(u32, u32)>) {
    chars.clear();
    spans.clear();
    let mut cluster = String::new();
    let mut start = 0u32;
    let mut index = 0u32;
    let mut flush = |cluster: &mut String, start: u32, end: u32, chars: &mut Vec<char>| {
        for c in cluster.nfc() {
            chars.push(c);
            spans.push((start, end));
        }
        cluster.clear();
    };
    for c in text.chars() {
        if canonical_combining_class(c) == 0 {
            flush(&mut cluster, start, index, chars);
            start = index;
        }
        cluster.push(c);
        index += 1;
    }
    flush(&mut cluster, start, index, chars);
}

fn fuzzy_query(
    snapshot: &WorktreeSnapshot,
    query: &str,
    recent: &[Arc<Path>],
    limit: usize,
    cancel: &AtomicBool,
) -> Vec<FileMatch> {
    let query: String = query.nfc().collect();
    let pattern = Pattern::new(
        &query,
        CaseMatching::Smart,
        Normalization::Smart,
        AtomKind::Fuzzy,
    );
    let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
    let mut buf: Vec<char> = Vec::new();
    let mut spans: Vec<(u32, u32)> = Vec::new();
    let mut recent_rank: HashMap<&Path, usize> = HashMap::with_capacity(recent.len());
    for (i, p) in recent.iter().enumerate() {
        recent_rank.entry(&**p).or_insert(i);
    }

    let mut candidates: Vec<Candidate> = Vec::new();
    for (i, entry) in snapshot.entries().iter().enumerate() {
        if i % CANCEL_STRIDE == 0 && cancel.load(AtomicOrdering::Relaxed) {
            return Vec::new();
        }
        if entry.kind != EntryKind::File {
            continue;
        }
        let text = entry.path.to_string_lossy();
        // ASCII paths (the common case) skip normalisation and keep nucleo's
        // allocation-free ASCII matcher.
        let haystack = if text.is_ascii() {
            Utf32Str::Ascii(text.as_bytes())
        } else {
            nfc_chars(&text, &mut buf, &mut spans);
            Utf32Str::Unicode(&buf)
        };
        let Some(score) = pattern.score(haystack, &mut matcher) else {
            continue;
        };
        candidates.push(Candidate {
            path: &entry.path,
            score,
            len: text.len(),
            recent_rank: recent_rank.get(&*entry.path).copied(),
        });
    }

    if candidates.len() > limit {
        candidates.select_nth_unstable_by(limit, compare);
        candidates.truncate(limit);
    }
    candidates.sort_unstable_by(compare);

    // Highlight positions are only needed for the survivors.
    let mut out = Vec::with_capacity(candidates.len());
    for c in candidates {
        let text = c.path.to_string_lossy();
        let mut positions = Vec::new();
        if text.is_ascii() {
            pattern.indices(
                Utf32Str::Ascii(text.as_bytes()),
                &mut matcher,
                &mut positions,
            );
        } else {
            nfc_chars(&text, &mut buf, &mut spans);
            let mut matched = Vec::new();
            pattern.indices(Utf32Str::Unicode(&buf), &mut matcher, &mut matched);
            for m in matched {
                if let Some(&(from, to)) = spans.get(m as usize) {
                    positions.extend(from..to);
                }
            }
        }
        positions.sort_unstable();
        positions.dedup();
        out.push(FileMatch {
            path: c.path.clone(),
            score: c.score,
            positions,
        });
    }
    out
}
