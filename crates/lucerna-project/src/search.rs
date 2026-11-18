use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crossbeam_channel::Sender;
use grep_regex::{RegexMatcher, RegexMatcherBuilder};
use grep_searcher::{BinaryDetection, Searcher, SearcherBuilder, Sink, SinkContext, SinkMatch};
use ignore::{WalkParallel, WalkState};
use regex::Regex;

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
    /// `editor.wordSeparators`: what `whole_word` accepts as a boundary.
    /// Empty means only whitespace and line edges bound a word.
    pub word_separators: String,
}

/// What a reported line is. Search results are always [`Normal`]; a diff
/// view reports added and deleted lines too.
///
/// [`Normal`]: LineKind::Normal
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineKind {
    /// A match or a context line (in a diff, an unchanged line).
    #[default]
    Normal,
    /// A line of the current text that the base does not have.
    Added,
    /// A base line, 1-based `base_line`, that the current text lacks. Its
    /// [`LineMatch::line_number`] is the current row that follows the
    /// deletion, and its `ranges` are empty.
    Deleted { base_line: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineMatch {
    pub line_number: u32,
    pub line_text: String,
    pub ranges: Vec<Range<usize>>,
    pub kind: LineKind,
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
pub const MAX_LINE_CHARS: usize = 1000;

/// The regex source for a search pattern: escaped unless `is_regex`. Whole
/// word is not part of it; [`line_matches`] filters matches by
/// `editor.wordSeparators`. Case folding is left to the regex builder.
pub fn build_pattern(pattern: &str, is_regex: bool) -> String {
    if is_regex {
        pattern.to_owned()
    } else {
        regex::escape(pattern)
    }
}

/// The regex that decides matches inside one line: case folding unless
/// `case_sensitive`, `^`/`$` at line ends (CRLF too). Keep its flags aligned
/// with the grep matcher built in [`search`].
pub fn search_regex(query: &SearchQuery) -> Result<Regex, String> {
    regex::RegexBuilder::new(&build_pattern(&query.pattern, query.is_regex))
        .case_insensitive(!query.case_sensitive)
        .multi_line(true)
        .crlf(true)
        .build()
        .map_err(|err| err.to_string())
}

/// The non-empty matches in one line (no `\n`), filtered by whole word via
/// [`lucerna_text::words::is_word_bounded`] when `query.whole_word`. Search,
/// the replace preview and the replace job all decide "a match" here.
pub fn line_matches(regex: &Regex, query: &SearchQuery, line: &str) -> Vec<Range<usize>> {
    regex
        .find_iter(line)
        .filter(|m| !m.is_empty())
        .filter(|m| {
            if !query.whole_word {
                return true;
            }
            let text = &line[m.range()];
            lucerna_text::words::is_word_bounded(
                line[..m.start()].chars().next_back(),
                text.chars().next(),
                text.chars().next_back(),
                line[m.end()..].chars().next(),
                &query.word_separators,
            )
        })
        .map(|m| m.range())
        .collect()
}

/// The excerpts of one text, as [`search`] would report a file with that
/// text: same context lines and `MAX_LINE_CHARS` cut, but no `MAX_RESULTS`
/// across files.
pub fn search_text(query: &SearchQuery, text: &str) -> Result<Vec<Excerpt>, String> {
    let matcher = grep_matcher_for(query)?;
    let regex = search_regex(query)?;
    let mut searcher = searcher_for(query);
    Ok(search_file(
        &mut searcher,
        &matcher,
        &regex,
        query,
        Path::new(""),
        Some(text),
        &AtomicBool::new(false),
    ))
}

fn grep_matcher_for(query: &SearchQuery) -> Result<RegexMatcher, String> {
    RegexMatcherBuilder::new()
        .case_insensitive(!query.case_sensitive)
        // `$` and `^` must match at CRLF line ends too (VS Code does). In
        // multi-line mode with `crlf`, they match before `\r\n` as well as
        // `\n`, and `.` stops before `\r`. The terminator is forced back to
        // `\n` because `crlf(true)` sets `\r\n`, which the default `\n`
        // searcher rejects for every file. `line_match` trims the `\r`.
        .multi_line(true)
        .crlf(true)
        .line_terminator(Some(b'\n'))
        .build(&build_pattern(&query.pattern, query.is_regex))
        .map_err(|err| err.to_string())
}

fn searcher_for(query: &SearchQuery) -> Searcher {
    let context = query.context_lines as usize;
    SearcherBuilder::new()
        .before_context(context)
        .after_context(context)
        .binary_detection(BinaryDetection::quit(0))
        .build()
}

/// Searches the files below `root` and streams one `File` event per file
/// with matches, then exactly one `Done`. Blocking: run it on a thread.
///
/// The walk uses the worktree scan's ignore policy; `query.include` limits
/// the files, `query.exclude` and `settings_exclude` remove files. At most
/// `MAX_RESULTS` matches are sent; reaching that limit ends the search with
/// `truncated: true`. A cancelled search sends no `Done` and stops early.
///
/// `home` is the worktree's [`ScanOptions::home`]: parent folders' ignore
/// files are read up to it, as in the scan.
///
/// `skip_dirs` are root-relative folders the walk neither enters nor
/// reports: the worktree's unloaded folders (`files.scanDepth`).
///
/// `buffers` are the `(path, text)` of open files with unsaved changes,
/// paths as the walk yields them (`root` joined with the relative path).
/// As in VS Code, a walked file found there is searched on that text
/// instead of the disk, so its line numbers are the buffer's; the filters
/// still decide which files are searched.
///
/// `cancel` is re-checked inside the line loop and again right before a
/// file's events are sent, but a microscopic window remains between that
/// check and the send. Consumers must therefore also drop events from
/// superseded searches (use a fresh channel per search).
#[allow(clippy::too_many_arguments)]
pub fn search(
    root: &Path,
    query: &SearchQuery,
    settings_exclude: &[String],
    home: Option<&Path>,
    skip_dirs: &HashSet<Arc<Path>>,
    buffers: &[(Arc<Path>, String)],
    sink: Sender<SearchEvent>,
    cancel: Arc<AtomicBool>,
) {
    let built = grep_matcher_for(query).and_then(|m| Ok((m, search_regex(query)?)));
    let (matcher, regex) = match built {
        Ok(pair) => pair,
        Err(err) => {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            let _ = sink.send(SearchEvent::Error(err));
            let _ = sink.send(SearchEvent::Done {
                total: 0,
                truncated: false,
            });
            return;
        }
    };
    let mut excludes = query.exclude.clone();
    excludes.extend_from_slice(settings_exclude);
    let opts = ScanOptions {
        exclude: excludes,
        home: home.map(Path::to_path_buf),
        ..Default::default()
    };
    let buffers: HashMap<&Path, &str> = buffers
        .iter()
        .map(|(path, text)| (&**path, text.as_str()))
        .collect();
    let (skip, base): (HashSet<PathBuf>, PathBuf) = (
        skip_dirs.iter().map(|p| p.to_path_buf()).collect(),
        root.to_path_buf(),
    );
    let builder = walk_builder_with_includes(root, &opts, &query.include, move |entry| {
        skip.is_empty()
            || entry
                .path()
                .strip_prefix(&base)
                .map_or(true, |rel| !skip.contains(rel))
    });
    let walker: WalkParallel = builder.build_parallel();

    // Matches reserved so far; never above MAX_RESULTS.
    let total = AtomicUsize::new(0);
    let truncated = AtomicBool::new(false);
    walker.run(|| {
        let mut searcher = searcher_for(query);
        let (matcher, regex, sink, cancel, buffers) = (&matcher, &regex, &sink, &cancel, &buffers);
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
                search_file(
                    &mut searcher,
                    matcher,
                    regex,
                    query,
                    path,
                    buffers.get(path).copied(),
                    cancel,
                )
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

/// The excerpts of one file: of `buffer` when it has unsaved text, else of
/// the disk. Unreadable files yield nothing.
fn search_file(
    searcher: &mut Searcher,
    matcher: &RegexMatcher,
    regex: &Regex,
    query: &SearchQuery,
    path: &Path,
    buffer: Option<&str>,
    cancel: &AtomicBool,
) -> Vec<Excerpt> {
    let mut sink = ExcerptSink {
        regex,
        query,
        cancel,
        excerpts: Vec::new(),
        match_count: 0,
        new_excerpt: true,
    };
    let result = match buffer {
        Some(text) => searcher.search_slice(matcher, text.as_bytes(), &mut sink),
        None => search_disk_file(searcher, matcher, path, &mut sink),
    };
    if let Err(err) = result {
        tracing::debug!(path = %path.display(), %err, "could not search file");
    }
    // A line whose matches were all filtered out (whole word) is context;
    // an excerpt left without a match line is dropped.
    let mut excerpts = sink.excerpts;
    excerpts.retain(|e| e.match_count() > 0);
    excerpts
}

/// Searches a file on disk as the editor and the replace job see it: a file
/// that is not valid UTF-8 and has no BOM (which grep transcodes itself) is
/// decoded with `decode_file` first, so `line_text` of a Windows-1252 file
/// reads "café", not "caf\u{FFFD}". Valid UTF-8 and BOM files go straight to
/// the searcher. Files past `MAX_OPEN_BYTES` keep the streaming path.
fn search_disk_file(
    searcher: &mut Searcher,
    matcher: &RegexMatcher,
    path: &Path,
    sink: &mut ExcerptSink<'_>,
) -> Result<(), std::io::Error> {
    let meta = std::fs::metadata(path)?;
    if meta.len() > lucerna_text::MAX_OPEN_BYTES {
        return searcher.search_path(matcher, path, sink);
    }
    let bytes = std::fs::read(path)?;
    if has_bom(&bytes) || std::str::from_utf8(&bytes).is_ok() {
        return searcher.search_slice(matcher, &bytes, sink);
    }
    // A decode error (binary) means the file yields nothing, as before.
    if let Ok(decoded) = lucerna_text::decode_file(&bytes, false) {
        searcher.search_slice(matcher, decoded.text.as_bytes(), sink)?;
    }
    Ok(())
}

fn has_bom(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xEF, 0xBB, 0xBF])
        || bytes.starts_with(&[0xFF, 0xFE])
        || bytes.starts_with(&[0xFE, 0xFF])
}

/// Collects matching and context lines into excerpts.
struct ExcerptSink<'a> {
    regex: &'a Regex,
    query: &'a SearchQuery,
    cancel: &'a AtomicBool,
    excerpts: Vec<Excerpt>,
    match_count: usize,
    /// The next line starts a new excerpt (set by a context break).
    new_excerpt: bool,
}

impl ExcerptSink<'_> {
    /// Removes the line pushed last, and its excerpt if that is left empty.
    fn drop_last(&mut self) {
        if let Some(excerpt) = self.excerpts.last_mut() {
            excerpt.lines.pop();
            if excerpt.lines.is_empty() {
                self.excerpts.pop();
                self.new_excerpt = true;
            }
        }
    }

    /// Whether the line pushed last has a match (not filtered, not context).
    fn last_is_match(&self) -> bool {
        self.excerpts
            .last()
            .and_then(|e| e.lines.last())
            .is_some_and(|l| !l.ranges.is_empty())
    }

    fn push(&mut self, line_number: Option<u64>, bytes: &[u8], is_match: bool) {
        let line_number = line_number.unwrap_or(0);
        let line = line_match(
            self.regex,
            self.query,
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
        if self.last_is_match() {
            self.match_count += 1;
        } else if self.query.context_lines == 0 {
            // Every match was filtered out: with no context lines the line
            // is not reported at all.
            self.drop_last();
        }
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
/// not mistaken for context. A line whose matches were all filtered out by
/// whole word is reported as context.
fn line_match(
    regex: &Regex,
    query: &SearchQuery,
    line_number: u64,
    line: &str,
    is_match: bool,
) -> LineMatch {
    let line = line.trim_end_matches(['\n', '\r']);
    let mut ranges = if is_match {
        line_matches(regex, query, line)
    } else {
        Vec::new()
    };
    let is_match = is_match && !ranges.is_empty();
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
        kind: LineKind::Normal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEPS: &str = "-";

    fn q(pattern: &str, whole_word: bool, seps: &str) -> SearchQuery {
        SearchQuery {
            pattern: pattern.into(),
            case_sensitive: true,
            whole_word,
            word_separators: seps.into(),
            ..Default::default()
        }
    }

    fn matches(query: &SearchQuery, line: &str) -> Vec<Range<usize>> {
        line_matches(&search_regex(query).unwrap(), query, line)
    }

    #[test]
    fn literal_is_escaped() {
        assert_eq!(build_pattern("a.b(c)", false), r"a\.b\(c\)");
    }

    #[test]
    fn regex_passes_through() {
        assert_eq!(build_pattern(r"a.+b", true), "a.+b");
    }

    #[test]
    fn pattern_never_adds_word_boundary() {
        assert_eq!(build_pattern("log", false), "log");
        assert_eq!(build_pattern("a.b", false), r"a\.b");
    }

    #[test]
    fn whole_word_by_separators() {
        let query = q("foo", true, SEPS);
        assert_eq!(matches(&query, "foo-bar foo"), vec![0..3, 8..11]);
        assert!(matches(&query, "foobar").is_empty());
        assert!(matches(&q("foo", true, ""), "foo-bar").is_empty());
    }

    #[test]
    fn partial_matches_without_whole_word() {
        assert_eq!(matches(&q("foo", false, SEPS), "foobar"), vec![0..3]);
    }

    #[test]
    fn empty_matches_are_ignored() {
        assert!(matches(&q("x*", true, SEPS), "ab").is_empty());
        assert!(matches(&q("x*", false, SEPS), "ab").is_empty());
    }

    #[test]
    fn search_text_reports_excerpts_with_context() {
        let mut query = q("foo", true, SEPS);
        query.context_lines = 1;
        let excerpts = search_text(&query, "a\nfoo\nb").unwrap();
        assert_eq!(excerpts.len(), 1);
        let lines = &excerpts[0].lines;
        let numbers: Vec<u32> = lines.iter().map(|l| l.line_number).collect();
        assert_eq!(numbers, vec![1, 2, 3]);
        assert!(lines[0].ranges.is_empty());
        assert_eq!(lines[1].ranges, vec![0..3]);
        assert!(lines[2].ranges.is_empty());
    }

    #[test]
    fn search_text_drops_fully_filtered_lines() {
        let query = q("foo", true, SEPS);
        let excerpts = search_text(&query, "foobar\nfoo\n").unwrap();
        assert_eq!(excerpts.len(), 1);
        assert_eq!(excerpts[0].lines.len(), 1);
        assert_eq!(excerpts[0].lines[0].line_number, 2);
        assert!(search_text(&query, "foobar\n").unwrap().is_empty());
    }

    #[test]
    fn search_text_reports_bad_pattern() {
        let mut query = q("(", false, SEPS);
        query.is_regex = true;
        assert!(search_text(&query, "x").is_err());
    }
}
