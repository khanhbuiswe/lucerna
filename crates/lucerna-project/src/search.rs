use std::ops::Range;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crossbeam_channel::Sender;
use grep_matcher::Matcher;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::sinks::Lossy;
use grep_searcher::{BinaryDetection, Searcher, SearcherBuilder};
use ignore::{WalkParallel, WalkState};

use crate::watch::panic_message;
use crate::worktree::{ScanOptions, walk_builder_with_includes};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchQuery {
    pub pattern: String,
    pub is_regex: bool,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineMatch {
    pub line_number: u32,
    pub line_text: String,
    pub ranges: Vec<Range<usize>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchEvent {
    File {
        path: Arc<Path>,
        matches: Vec<LineMatch>,
    },
    Done {
        total: usize,
        truncated: bool,
    },
    Error(String),
}

pub const MAX_RESULTS: usize = 20_000;

/// Most characters of a matching line that are reported.
const MAX_LINE_CHARS: usize = 1000;

/// The regex source for a search pattern: escaped unless `is_regex`, and
/// wrapped as `\b(?:…)\b` for whole-word. Case folding is left to the
/// regex builder (`case_insensitive`).
pub fn build_pattern(pattern: &str, is_regex: bool, whole_word: bool) -> String {
    let source = if is_regex {
        pattern.to_owned()
    } else {
        regex::escape(pattern)
    };
    if whole_word {
        format!(r"\b(?:{source})\b")
    } else {
        source
    }
}

/// Searches the files below `root` and streams one `File` event per file
/// with matches, then exactly one `Done`. Blocking: run it on a thread.
///
/// The walk uses the worktree scan's ignore policy; `query.include` limits
/// the files, `query.exclude` and `settings_exclude` remove files. At most
/// `MAX_RESULTS` matches are sent; reaching that limit ends the search with
/// `truncated: true`. A cancelled search sends no `Done` and stops early.
///
/// `cancel` is re-checked inside the line loop and again right before a
/// file's events are sent, but a microscopic window remains between that
/// check and the send. Consumers must therefore also drop events from
/// superseded searches (use a fresh channel per search).
pub fn search(
    root: &Path,
    query: &SearchQuery,
    settings_exclude: &[String],
    sink: Sender<SearchEvent>,
    cancel: Arc<AtomicBool>,
) {
    let source = build_pattern(&query.pattern, query.is_regex, query.whole_word);
    let matcher = match RegexMatcherBuilder::new()
        .case_insensitive(!query.case_sensitive)
        .build(&source)
    {
        Ok(matcher) => matcher,
        Err(err) => {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            let _ = sink.send(SearchEvent::Error(err.to_string()));
            let _ = sink.send(SearchEvent::Done {
                total: 0,
                truncated: false,
            });
            return;
        }
    };
    let mut excludes = query.exclude.clone();
    excludes.extend_from_slice(settings_exclude);
    let opts = ScanOptions { exclude: excludes };
    let walker: WalkParallel =
        walk_builder_with_includes(root, &opts, &query.include).build_parallel();

    // Matches reserved so far; never above MAX_RESULTS.
    let total = AtomicUsize::new(0);
    let truncated = AtomicBool::new(false);
    walker.run(|| {
        let mut searcher = SearcherBuilder::new()
            .binary_detection(BinaryDetection::quit(0))
            .build();
        let (matcher, sink, cancel) = (&matcher, &sink, &cancel);
        let (total, truncated) = (&total, &truncated);
        Box::new(move |entry| {
            if cancel.load(Ordering::Relaxed) {
                return WalkState::Quit;
            }
            let Ok(entry) = entry else {
                return WalkState::Continue;
            };
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                return WalkState::Continue;
            }
            let path = entry.path();
            let result = catch_unwind(AssertUnwindSafe(|| {
                search_file(&mut searcher, matcher, path, cancel)
            }));
            let mut matches = match result {
                Ok(matches) => matches,
                Err(payload) => {
                    let message = panic_message(payload.as_ref());
                    tracing::error!(path = %path.display(), %message, "searching a file panicked");
                    return WalkState::Continue;
                }
            };
            if cancel.load(Ordering::Relaxed) {
                return WalkState::Quit;
            }
            if matches.is_empty() {
                return WalkState::Continue;
            }
            // Reserve room under the cap; the last file may be cut short.
            let wanted = matches.len();
            let before = total
                .try_update(Ordering::SeqCst, Ordering::SeqCst, |t| {
                    Some((t + wanted).min(MAX_RESULTS))
                })
                .unwrap_or(MAX_RESULTS);
            let allowed = wanted.min(MAX_RESULTS - before);
            matches.truncate(allowed);
            let at_cap = before + allowed >= MAX_RESULTS;
            if at_cap {
                truncated.store(true, Ordering::SeqCst);
            }
            if !matches.is_empty() {
                let event = SearchEvent::File {
                    path: Arc::from(path),
                    matches,
                };
                if sink.send(event).is_err() {
                    return WalkState::Quit;
                }
            }
            if at_cap {
                WalkState::Quit
            } else {
                WalkState::Continue
            }
        })
    });

    if cancel.load(Ordering::Relaxed) {
        return;
    }
    let _ = sink.send(SearchEvent::Done {
        total: total.load(Ordering::SeqCst),
        truncated: truncated.load(Ordering::SeqCst),
    });
}

/// The matching lines of one file. Unreadable files yield nothing.
fn search_file(
    searcher: &mut Searcher,
    matcher: &RegexMatcher,
    path: &Path,
    cancel: &AtomicBool,
) -> Vec<LineMatch> {
    let mut matches = Vec::new();
    let result = searcher.search_path(
        matcher,
        path,
        Lossy(|line_number, line| {
            matches.push(line_match(matcher, line_number, line));
            Ok(matches.len() < MAX_RESULTS && !cancel.load(Ordering::Relaxed))
        }),
    );
    if let Err(err) = result {
        tracing::debug!(path = %path.display(), %err, "could not search file");
    }
    matches
}

fn line_match(matcher: &RegexMatcher, line_number: u64, line: &str) -> LineMatch {
    let line = line.trim_end_matches(['\n', '\r']);
    let mut ranges = Vec::new();
    let _ = matcher.find_iter(line.as_bytes(), |m| {
        ranges.push(m.start()..m.end());
        true
    });
    let keep = line
        .char_indices()
        .nth(MAX_LINE_CHARS)
        .map_or(line.len(), |(i, _)| i);
    ranges.retain(|r| r.start < keep);
    for r in &mut ranges {
        r.end = r.end.min(keep);
    }
    LineMatch {
        line_number: u32::try_from(line_number).unwrap_or(u32::MAX),
        line_text: line[..keep].to_owned(),
        ranges,
    }
}

#[cfg(test)]
mod tests {
    use super::build_pattern;

    #[test]
    fn literal_is_escaped() {
        assert_eq!(build_pattern("a.b(c)", false, false), r"a\.b\(c\)");
    }

    #[test]
    fn regex_passes_through() {
        assert_eq!(build_pattern(r"a.+b", true, false), "a.+b");
    }

    #[test]
    fn whole_word_wraps() {
        assert_eq!(build_pattern("log", false, true), r"\b(?:log)\b");
        assert_eq!(build_pattern("a|b", true, true), r"\b(?:a|b)\b");
        assert_eq!(build_pattern("a.b", false, true), r"\b(?:a\.b)\b");
    }
}
