use std::ops::Range;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crossbeam_channel::Sender;
use grep_matcher::Matcher;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::{BinaryDetection, Searcher, SearcherBuilder, Sink, SinkContext, SinkMatch};
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
    /// Lines of context kept before and after each matching line.
    pub context_lines: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineMatch {
    pub line_number: u32,
    pub line_text: String,
    pub ranges: Vec<Range<usize>>,
}

/// A run of consecutive lines around one or more matches. A line with empty
/// `ranges` is context.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Excerpt {
    pub lines: Vec<LineMatch>,
}

impl Excerpt {
    /// Number of matching (non-context) lines.
    pub fn match_count(&self) -> usize {
        self.lines.iter().filter(|l| !l.ranges.is_empty()).count()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchEvent {
    File {
        path: Arc<Path>,
        excerpts: Vec<Excerpt>,
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
        let context = query.context_lines as usize;
        let mut searcher = SearcherBuilder::new()
            .before_context(context)
            .after_context(context)
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
            let mut excerpts = match result {
                Ok(excerpts) => excerpts,
                Err(payload) => {
                    let message = panic_message(payload.as_ref());
                    tracing::error!(path = %path.display(), %message, "searching a file panicked");
                    return WalkState::Continue;
                }
            };
            if cancel.load(Ordering::Relaxed) {
                return WalkState::Quit;
            }
            let wanted: usize = excerpts.iter().map(Excerpt::match_count).sum();
            if wanted == 0 {
                return WalkState::Continue;
            }
            // Reserve room under the cap; the last file may be cut short.
            let before = total
                .try_update(Ordering::SeqCst, Ordering::SeqCst, |t| {
                    Some((t + wanted).min(MAX_RESULTS))
                })
                .unwrap_or(MAX_RESULTS);
            let allowed = wanted.min(MAX_RESULTS - before);
            if allowed < wanted {
                truncate_matches(&mut excerpts, allowed);
            }
            let at_cap = before + allowed >= MAX_RESULTS;
            if at_cap {
                truncated.store(true, Ordering::SeqCst);
            }
            if !excerpts.is_empty() {
                let event = SearchEvent::File {
                    path: Arc::from(path),
                    excerpts,
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

/// The excerpts of one file. Unreadable files yield nothing.
fn search_file(
    searcher: &mut Searcher,
    matcher: &RegexMatcher,
    path: &Path,
    cancel: &AtomicBool,
) -> Vec<Excerpt> {
    let mut sink = ExcerptSink {
        matcher,
        cancel,
        excerpts: Vec::new(),
        match_count: 0,
        new_excerpt: true,
    };
    if let Err(err) = searcher.search_path(matcher, path, &mut sink) {
        tracing::debug!(path = %path.display(), %err, "could not search file");
    }
    sink.excerpts
}

/// Collects matching and context lines into excerpts.
struct ExcerptSink<'a> {
    matcher: &'a RegexMatcher,
    cancel: &'a AtomicBool,
    excerpts: Vec<Excerpt>,
    match_count: usize,
    /// The next line starts a new excerpt (set by a context break).
    new_excerpt: bool,
}

impl ExcerptSink<'_> {
    fn push(&mut self, line_number: Option<u64>, bytes: &[u8], is_match: bool) {
        let line_number = line_number.unwrap_or(0);
        let line = line_match(
            self.matcher,
            line_number,
            &String::from_utf8_lossy(bytes),
            is_match,
        );
        let contiguous = self
            .excerpts
            .last()
            .and_then(|e| e.lines.last())
            .is_some_and(|prev| u64::from(prev.line_number) + 1 == line_number);
        if self.new_excerpt || !contiguous || self.excerpts.is_empty() {
            self.excerpts.push(Excerpt::default());
        }
        self.new_excerpt = false;
        if let Some(excerpt) = self.excerpts.last_mut() {
            excerpt.lines.push(line);
        }
    }
}

impl Sink for ExcerptSink<'_> {
    type Error = std::io::Error;

    fn matched(&mut self, _: &Searcher, mat: &SinkMatch<'_>) -> Result<bool, Self::Error> {
        self.push(mat.line_number(), mat.bytes(), true);
        self.match_count += 1;
        Ok(self.match_count < MAX_RESULTS && !self.cancel.load(Ordering::Relaxed))
    }

    fn context(&mut self, _: &Searcher, ctx: &SinkContext<'_>) -> Result<bool, Self::Error> {
        self.push(ctx.line_number(), ctx.bytes(), false);
        Ok(!self.cancel.load(Ordering::Relaxed))
    }

    fn context_break(&mut self, _: &Searcher) -> Result<bool, Self::Error> {
        self.new_excerpt = true;
        Ok(true)
    }
}

/// Keeps everything up to and including the `allowed`-th match line and
/// drops the rest (trailing context included) and any empty excerpts.
fn truncate_matches(excerpts: &mut Vec<Excerpt>, allowed: usize) {
    let mut seen = 0;
    let mut keep = 0;
    for excerpt in excerpts.iter_mut() {
        if seen >= allowed {
            break;
        }
        let mut cut = excerpt.lines.len();
        for (i, line) in excerpt.lines.iter().enumerate() {
            if !line.ranges.is_empty() {
                seen += 1;
                if seen == allowed {
                    cut = i + 1;
                    break;
                }
            }
        }
        excerpt.lines.truncate(cut);
        keep += 1;
    }
    excerpts.truncate(keep);
    excerpts.retain(|e| !e.lines.is_empty());
}

/// A reported line. A matching line always has a range, even when every match
/// lies past the cut: then it is the empty range at the cut, so the line is
/// not mistaken for context.
fn line_match(matcher: &RegexMatcher, line_number: u64, line: &str, is_match: bool) -> LineMatch {
    let line = line.trim_end_matches(['\n', '\r']);
    let mut ranges = Vec::new();
    if is_match {
        let _ = matcher.find_iter(line.as_bytes(), |m| {
            ranges.push(m.start()..m.end());
            true
        });
    }
    let keep = line
        .char_indices()
        .nth(MAX_LINE_CHARS)
        .map_or(line.len(), |(i, _)| i);
    ranges.retain(|r| r.start < keep);
    for r in &mut ranges {
        r.end = r.end.min(keep);
    }
    if is_match && ranges.is_empty() {
        ranges.push(keep..keep);
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
