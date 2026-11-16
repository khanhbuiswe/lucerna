//! Syntax colors for excerpt lines. [`highlight_text`] and
//! [`highlight_file`] are pure and blocking: they parse the text with
//! tree-sitter, so they run on a background thread and never touch gpui.
//! A diff's lines are colored from the texts the host gives
//! ([`TextSource`]): current lines from the current text, deleted lines
//! from the base. [`HighlightCache`] keeps the colors, and the queue on
//! [`ExcerptList`] runs the jobs for the files on screen, at most
//! [`MAX_HIGHLIGHT_JOBS`] at once.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use gpui::{AppContext, Context};
use lucerna_project::{Excerpt, LineKind, LineMatch};
use lucerna_syntax::{LanguageRegistry, SyntaxState};
use lucerna_text::{Buffer, BufferSnapshot, Point};

use super::{ExcerptList, TextSource};
use crate::AppState;
use crate::tasks::{catch_panic, guarded};

/// The most files highlighted at once, so a long list never starves the
/// background executor.
pub const MAX_HIGHLIGHT_JOBS: usize = 3;

/// Files above this size are shown plain.
pub const MAX_HIGHLIGHT_BYTES: u64 = 2 * 1024 * 1024;

/// Byte ranges within `LineMatch::line_text`, with capture names.
pub type LineSpans = Vec<(Range<usize>, Arc<str>)>;
/// Keyed by 1-based `line_number`.
pub type FileHighlights = HashMap<u32, LineSpans>;

/// Blocking; call on a background thread. The colors of the 1-based
/// `line_numbers` of `text` (LF line endings), each line's spans relative
/// to its start and clipped to it. Empty when the text is over
/// [`MAX_HIGHLIGHT_BYTES`], has no language (by `rel_path`) or its grammar
/// fails; line numbers past the end are left out.
pub fn highlight_text(
    text: &str,
    rel_path: &Path,
    line_numbers: &[u32],
    registry: &Arc<LanguageRegistry>,
    styled: &dyn Fn(&str) -> bool,
) -> FileHighlights {
    if text.len() as u64 > MAX_HIGHLIGHT_BYTES || line_numbers.is_empty() {
        return FileHighlights::new();
    }
    catch_panic("excerpt highlight", || {
        highlight_rows(
            &BufferSnapshot::from_text(text),
            rel_path,
            line_numbers,
            registry,
            styled,
        )
    })
    .unwrap_or_default()
}

/// Blocking; call on a background thread. The colors of the excerpts'
/// current lines in the file on disk: loads it and calls
/// [`highlight_text`]. Empty when the file is too big, cannot be loaded,
/// has no language or its grammar fails. A line whose text no longer
/// starts with the excerpt's `line_text` (the file changed since the
/// search) is left out and shows plain, as are deleted lines.
pub fn highlight_file(
    abs_path: &Path,
    rel_path: &Path,
    excerpts: &[Excerpt],
    registry: &Arc<LanguageRegistry>,
    styled: &dyn Fn(&str) -> bool,
) -> FileHighlights {
    let too_big = std::fs::metadata(abs_path).is_ok_and(|m| m.len() > MAX_HIGHLIGHT_BYTES);
    if too_big {
        return FileHighlights::new();
    }
    let Ok(buffer) = Buffer::load(abs_path) else {
        return FileHighlights::new();
    };
    let text = buffer.snapshot().text();
    fitted(&text, rel_path, &current_lines(excerpts), registry, styled)
}

/// The excerpts' current (not deleted) lines, as `(key, row, line_text)`
/// for [`fitted`]: keyed and found by line number.
fn current_lines(excerpts: &[Excerpt]) -> Vec<(u32, u32, &str)> {
    excerpts
        .iter()
        .flat_map(|e| &e.lines)
        .filter(|l| !matches!(l.kind, LineKind::Deleted { .. }))
        .map(|l| (l.line_number, l.line_number, l.line_text.as_str()))
        .collect()
}

/// The excerpts' deleted lines, as `(key, row, line_text)` for
/// [`fitted`]: keyed and found by base line.
fn base_lines(excerpts: &[Excerpt]) -> Vec<(u32, u32, &str)> {
    excerpts
        .iter()
        .flat_map(|e| &e.lines)
        .filter_map(|l| match l.kind {
            LineKind::Deleted { base_line } => Some((base_line, base_line, l.line_text.as_str())),
            _ => None,
        })
        .collect()
}

/// A host's text with LF line endings.
fn lf(text: &str) -> Cow<'_, str> {
    if text.contains("\r\n") {
        Cow::Owned(text.replace("\r\n", "\n"))
    } else {
        Cow::Borrowed(text)
    }
}

/// The colors of `lines` (`(key, 1-based row, line_text)`) in `text`,
/// keyed by `key`: a row's spans cut to its `line_text`, or nothing for a
/// row that does not start with it.
fn fitted(
    text: &str,
    rel_path: &Path,
    lines: &[(u32, u32, &str)],
    registry: &Arc<LanguageRegistry>,
    styled: &dyn Fn(&str) -> bool,
) -> FileHighlights {
    let mut out = FileHighlights::new();
    let numbers: Vec<u32> = lines.iter().map(|(_, n, _)| *n).collect();
    let rows = highlight_text(text, rel_path, &numbers, registry, styled);
    if rows.is_empty() {
        return out;
    }
    // Rows split as the buffer splits them: at `\n`, without a `\r`.
    let row_text: HashMap<u32, &str> = text
        .split('\n')
        .zip(1u32..)
        .filter(|(_, n)| rows.contains_key(n))
        .map(|(row, n)| (n, row.strip_suffix('\r').unwrap_or(row)))
        .collect();
    for &(key, number, line_text) in lines {
        let (Some(spans), Some(row)) = (rows.get(&number), row_text.get(&number)) else {
            continue;
        };
        if !row.starts_with(line_text) {
            continue;
        }
        let len = line_text.len();
        // The row starts with `line_text`, so these are boundaries unless
        // the cut fell inside a char: spans may not end on one.
        let line_spans: LineSpans = spans
            .iter()
            .filter(|(r, _)| r.start < len)
            .map(|(r, c)| (r.start..r.end.min(len), c.clone()))
            .filter(|(r, _)| {
                line_text.is_char_boundary(r.start) && line_text.is_char_boundary(r.end)
            })
            .collect();
        out.insert(key, line_spans);
    }
    out
}

/// The spans of the 1-based `line_numbers` of `snapshot`, each relative to
/// its row and clipped to it. Parses the whole text once and asks for the
/// spans of each run of consecutive rows.
fn highlight_rows(
    snapshot: &BufferSnapshot,
    rel_path: &Path,
    line_numbers: &[u32],
    registry: &Arc<LanguageRegistry>,
    styled: &dyn Fn(&str) -> bool,
) -> FileHighlights {
    let mut out = FileHighlights::new();
    let line_count = snapshot.line_count();
    let mut rows: Vec<u32> = line_numbers
        .iter()
        .filter(|n| **n >= 1 && **n <= line_count)
        .map(|n| n - 1)
        .collect();
    if rows.is_empty() {
        return out;
    }
    rows.sort_unstable();
    rows.dedup();
    let Some(language) = registry.language_for_file(rel_path, &snapshot.line(0)) else {
        return out;
    };
    let Ok(mut syntax) = SyntaxState::new(language, registry.clone()) else {
        return out;
    };
    syntax.parse(snapshot, None);
    let mut runs: Vec<Range<u32>> = Vec::new();
    for &row in &rows {
        match runs.last_mut() {
            Some(run) if run.end == row => run.end = row + 1,
            _ => runs.push(row..row + 1),
        }
    }
    for run in runs {
        let spans = syntax.highlights(snapshot, run.clone(), styled);
        let bounds: Vec<(usize, usize)> = run
            .clone()
            .map(|row| {
                let start = snapshot.point_to_offset(Point { row, column: 0 });
                (start, snapshot.line(row).len())
            })
            .collect();
        let spans: Vec<(Range<usize>, Arc<str>)> =
            spans.into_iter().map(|s| (s.range, s.capture)).collect();
        for (row, line_spans) in run.zip(spans_by_row(&bounds, &spans, &mut 0)) {
            out.insert(row + 1, line_spans);
        }
    }
    out
}

/// Cuts `spans` (byte ranges of the text) to the rows `bounds` (each row's
/// start offset and length, in order and not overlapping): each row gets
/// the parts of the spans over it, relative to its start, in span order.
/// Each span visits only the rows it covers, found by binary search, so a
/// long run costs O(spans · log rows + output), not O(rows · spans).
/// `steps` counts the rows visited, for tests.
fn spans_by_row(
    bounds: &[(usize, usize)],
    spans: &[(Range<usize>, Arc<str>)],
    steps: &mut usize,
) -> Vec<LineSpans> {
    let mut out = vec![LineSpans::new(); bounds.len()];
    for (range, capture) in spans {
        // The first row ending after the span starts.
        let first = bounds.partition_point(|(start, len)| start + len <= range.start);
        for (row, &(start, len)) in bounds.iter().enumerate().skip(first) {
            if start >= range.end {
                break;
            }
            *steps += 1;
            let from = range.start.max(start) - start;
            let to = (range.end - start).min(len);
            if from < to {
                out[row].push((from..to, capture.clone()));
            }
        }
    }
    out
}

/// The colors of a file's excerpts: current lines from `source.current`
/// (the file on disk when there is no current text), keyed by line
/// number, and deleted lines from `source.base` (plain when there is
/// none), keyed by base line.
fn highlight_job(
    abs_path: &Path,
    rel_path: &Path,
    excerpts: &[Excerpt],
    source: Option<&TextSource>,
    registry: &Arc<LanguageRegistry>,
    styled: &dyn Fn(&str) -> bool,
) -> (FileHighlights, FileHighlights) {
    let current = match source.and_then(|s| s.current.as_deref()) {
        Some(text) => fitted(
            &lf(text),
            rel_path,
            &current_lines(excerpts),
            registry,
            styled,
        ),
        None => highlight_file(abs_path, rel_path, excerpts, registry, styled),
    };
    let base = match source.and_then(|s| s.base.as_deref()) {
        Some(text) => fitted(&lf(text), rel_path, &base_lines(excerpts), registry, styled),
        None => FileHighlights::new(),
    };
    (current, base)
}

/// The most files [`HighlightCache`] keeps colors for.
pub const MAX_CACHED_FILES: usize = 200;

/// Which text a line's colors come from: deleted lines from the base,
/// every other line from the current text. The cache keys them apart, as
/// a base line and a current line may share a number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Current,
    Base,
}

impl Side {
    /// The side `line` is colored from, and its line number there.
    pub fn of(line: &LineMatch) -> (Side, u32) {
        match line.kind {
            LineKind::Deleted { base_line } => (Side::Base, base_line),
            _ => (Side::Current, line.line_number),
        }
    }
}

/// Syntax colors of result lines, kept across searches so a re-search
/// does not flash its lines plain. Keyed by path, then side and line
/// number; each line holds the text its spans were computed for, and is
/// used only while the shown line has that same text. A file's colors are
/// also tagged with the generation of its [`TextSource`]: a new source
/// asks for them again, showing the old ones meanwhile. The least
/// recently used files beyond [`MAX_CACHED_FILES`] are dropped.
#[derive(Debug, Default)]
pub struct HighlightCache {
    files: HashMap<Arc<Path>, CachedFile>,
    tick: u64,
    /// Line scans run by [`missing_at`](Self::missing_at), for tests.
    scans: usize,
}

#[derive(Debug, Default)]
struct CachedFile {
    lines: HashMap<(Side, u32), (String, LineSpans)>,
    generation: u64,
    used: u64,
    /// The results revision at which every line of the file was found
    /// colored for `generation`: no need to look again until either
    /// changes.
    complete_at: Option<u64>,
}

impl HighlightCache {
    pub fn clear(&mut self) {
        self.files.clear();
    }

    /// Files with cached colors.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.files.len()
    }

    /// The spans of `line` in `path`, if computed for its current text.
    pub fn get(&self, path: &Path, line: &LineMatch) -> Option<&LineSpans> {
        let (text, spans) = self.files.get(path)?.lines.get(&Side::of(line))?;
        (*text == line.line_text).then_some(spans)
    }

    /// Whether some line of `excerpts` has no colors for its text yet, or
    /// the colors are of an older `generation` of the file's source.
    pub fn missing(&self, path: &Path, excerpts: &[Excerpt], generation: u64) -> bool {
        self.files
            .get(path)
            .is_none_or(|f| f.generation != generation)
            || excerpts
                .iter()
                .flat_map(|e| &e.lines)
                .any(|l| self.get(path, l).is_none())
    }

    /// [`missing`](Self::missing), remembered: once the file's lines are
    /// all found colored at results `revision`, later calls with the same
    /// revision and `generation` skip the scan of every line (run per
    /// frame for each file on screen; a whole-file diff tab is one long
    /// file).
    pub fn missing_at(
        &mut self,
        path: &Path,
        excerpts: &[Excerpt],
        generation: u64,
        revision: u64,
    ) -> bool {
        if self
            .files
            .get(path)
            .is_some_and(|f| f.generation == generation && f.complete_at == Some(revision))
        {
            return false;
        }
        self.scans += 1;
        let missing = self.missing(path, excerpts, generation);
        if !missing && let Some(f) = self.files.get_mut(path) {
            f.complete_at = Some(revision);
        }
        missing
    }

    /// Line scans run by [`missing_at`](Self::missing_at).
    pub fn scans(&self) -> usize {
        self.scans
    }

    /// Marks `path` as just used, so it is the last to be dropped.
    pub fn touch(&mut self, path: &Path) {
        self.tick += 1;
        if let Some(f) = self.files.get_mut(path) {
            f.used = self.tick;
        }
    }

    /// Stores the colors computed for `excerpts` from source `generation`:
    /// `current` by line number, `base` (deleted lines) by base line. A
    /// line the job left out (no language, a changed file) is stored
    /// plain, so it is not asked for again while its text stays the same.
    /// Colors of another generation are dropped first.
    pub fn insert(
        &mut self,
        path: Arc<Path>,
        excerpts: &[Excerpt],
        mut current: FileHighlights,
        mut base: FileHighlights,
        generation: u64,
    ) {
        self.tick += 1;
        let tick = self.tick;
        let file = self.files.entry(path.clone()).or_default();
        file.used = tick;
        file.complete_at = None;
        if file.generation != generation {
            file.lines.clear();
            file.generation = generation;
        }
        for line in excerpts.iter().flat_map(|e| &e.lines) {
            let key = Side::of(line);
            let spans = match key.0 {
                Side::Current => current.remove(&key.1),
                Side::Base => base.remove(&key.1),
            };
            file.lines
                .insert(key, (line.line_text.clone(), spans.unwrap_or_default()));
        }
        while self.files.len() > MAX_CACHED_FILES {
            let Some(oldest) = self
                .files
                .iter()
                .filter(|(p, _)| **p != path)
                .min_by_key(|(_, f)| f.used)
                .map(|(p, _)| p.clone())
            else {
                break;
            };
            self.files.remove(&oldest);
        }
    }

    /// The colors of `path`'s current lines by line number, for tests.
    pub fn file(&self, path: &Path) -> Option<FileHighlights> {
        let f = self.files.get(path)?;
        Some(
            f.lines
                .iter()
                .filter(|((side, _), _)| *side == Side::Current)
                .map(|((_, n), (_, spans))| (*n, spans.clone()))
                .collect(),
        )
    }
}

impl ExcerptList {
    /// Forgets every color and drops the queued and running jobs.
    fn reset_highlights(&mut self) {
        self.highlight_epoch += 1;
        self.highlight_cache.clear();
        self.highlight_pending.clear();
        self.highlight_queue.clear();
        self.highlight_tasks.clear();
    }

    /// Queues the files of the visible rows with lines the cache has no
    /// colors for and starts jobs up to [`MAX_HIGHLIGHT_JOBS`].
    pub(super) fn request_highlights(&mut self, files: Vec<usize>, cx: &mut Context<Self>) {
        let theme = cx.global::<AppState>().theme_generation;
        if theme != self.highlight_theme {
            self.highlight_theme = theme;
            self.reset_highlights();
        }
        let visible: HashSet<Arc<Path>> = files
            .iter()
            .filter_map(|&f| self.results.files().get(f))
            .map(|f| f.path.clone())
            .collect();
        // Files scrolled past must not hold up the ones on screen.
        let pending = &mut self.highlight_pending;
        self.highlight_queue.retain(|path| {
            let keep = visible.contains(path);
            if !keep {
                pending.remove(path);
            }
            keep
        });
        for file in files {
            let Some(result) = self.results.files().get(file) else {
                continue;
            };
            // A file shown as a note has no lines on screen.
            if self
                .results
                .meta(&result.path)
                .is_some_and(|m| m.note.is_some())
            {
                continue;
            }
            let path = result.path.clone();
            self.highlight_cache.touch(&path);
            let generation = self.source_generation(&path);
            if !self.highlight_cache.missing_at(
                &path,
                &result.excerpts,
                generation,
                self.results.revision(),
            ) || !self.highlight_pending.insert(path.clone())
            {
                continue;
            }
            self.highlight_queue.push_back(path);
        }
        self.start_highlight_jobs(cx);
    }

    fn start_highlight_jobs(&mut self, cx: &mut Context<Self>) {
        while self.highlight_tasks.len() < MAX_HIGHLIGHT_JOBS {
            let Some(path) = self.highlight_queue.pop_front() else {
                break;
            };
            let Some(excerpts) = self
                .results
                .files()
                .iter()
                .find(|f| f.path == path)
                .map(|f| f.excerpts.clone())
            else {
                self.highlight_pending.remove(&path);
                continue;
            };
            // The window's registry: its folder may add associations.
            let languages = self.settings.read(cx).languages(cx);
            let theme = cx.global::<AppState>().theme.clone();
            let (abs, rel, epoch) = (self.root.join(&path), path.clone(), self.highlight_epoch);
            let source = self.text_sources.get(&path).cloned();
            let generation = source.as_ref().map_or(0, |(_, g)| *g);
            let job = cx.background_spawn(async move {
                let (current, base) = guarded("excerpt highlight", || {
                    highlight_job(
                        &abs,
                        &rel,
                        &excerpts,
                        source.as_ref().map(|(s, _)| s),
                        &languages,
                        &|c| theme.style_for(c).is_some(),
                    )
                })
                .unwrap_or_default();
                (excerpts, current, base)
            });
            let key = path.clone();
            let task = cx.spawn(async move |this, cx| {
                let (excerpts, current, base) = job.await;
                this.update(cx, |list, cx| {
                    list.highlights_done(epoch, key, &excerpts, (current, base, generation), cx)
                })
                .ok();
            });
            self.highlight_tasks.insert(path, task);
            self.highlight_jobs_started += 1;
            self.peak_highlight_jobs = self.peak_highlight_jobs.max(self.highlight_tasks.len());
        }
    }

    fn highlights_done(
        &mut self,
        epoch: u64,
        path: Arc<Path>,
        excerpts: &[Excerpt],
        (current, base, generation): (FileHighlights, FileHighlights, u64),
        cx: &mut Context<Self>,
    ) {
        if epoch != self.highlight_epoch {
            return;
        }
        self.highlight_tasks.remove(&path);
        // A file whose excerpts a re-search replaced meanwhile may be
        // queued again for its new lines.
        self.highlight_pending.remove(&path);
        self.highlight_cache
            .insert(path, excerpts, current, base, generation);
        self.start_highlight_jobs(cx);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(n: u32, text: &str) -> LineMatch {
        LineMatch {
            line_number: n,
            line_text: text.into(),
            ranges: Vec::new(),
            kind: LineKind::Normal,
        }
    }

    fn run(name: &str, content: &[u8], lines: Vec<LineMatch>) -> FileHighlights {
        let dir = tempfile::tempdir().unwrap();
        let abs = dir.path().join(name);
        std::fs::write(&abs, content).unwrap();
        let registry = Arc::new(LanguageRegistry::bundled());
        highlight_file(
            &abs,
            Path::new(name),
            &[Excerpt { lines }],
            &registry,
            &|_| true,
        )
    }

    #[test]
    fn rust_keyword_gets_a_capture() {
        let h = run("a.rs", b"fn main() {}\n", vec![line(1, "fn main() {}")]);
        let spans = &h[&1];
        assert!(
            spans
                .iter()
                .any(|(r, c)| *r == (0..2) && c.starts_with("keyword")),
            "{spans:?}"
        );
    }

    #[test]
    fn multi_line_comment_colors_inner_line() {
        let h = run("a.rs", b"/*\nneedle\n*/\n", vec![line(2, "needle")]);
        let spans = &h[&2];
        assert!(
            spans.iter().any(|(_, c)| c.starts_with("comment")),
            "{spans:?}"
        );
    }

    #[test]
    fn changed_line_is_left_plain() {
        let h = run("a.rs", b"fn new() {}\n", vec![line(1, "old")]);
        assert!(!h.contains_key(&1));
    }

    #[test]
    fn unknown_language_is_empty() {
        let h = run(
            "x.unknownext",
            b"fn main() {}\n",
            vec![line(1, "fn main() {}")],
        );
        assert!(h.is_empty());
    }

    #[test]
    fn oversized_file_is_empty() {
        let mut content = b"fn main() {}\n".to_vec();
        content.resize(MAX_HIGHLIGHT_BYTES as usize + 1, b' ');
        let h = run("a.rs", &content, vec![line(1, "fn main() {}")]);
        assert!(h.is_empty());
    }

    #[test]
    fn highlight_text_colors_whole_rows() {
        let registry = Arc::new(LanguageRegistry::bundled());
        let h = highlight_text(
            "// one\nfn two() {}\n",
            Path::new("a.rs"),
            &[2, 9, 0],
            &registry,
            &|_| true,
        );
        assert_eq!(h.keys().copied().collect::<Vec<_>>(), [2], "past the end");
        assert!(
            h[&2]
                .iter()
                .any(|(r, c)| *r == (0..2) && c.starts_with("keyword")),
            "{h:?}"
        );
        assert!(h[&2].iter().all(|(r, _)| r.end <= "fn two() {}".len()));
        let big = "x".repeat(MAX_HIGHLIGHT_BYTES as usize + 1);
        assert!(highlight_text(&big, Path::new("a.rs"), &[1], &registry, &|_| true).is_empty());
    }

    #[test]
    fn job_colors_deleted_lines_from_the_base_text() {
        let registry = Arc::new(LanguageRegistry::bundled());
        let deleted = LineMatch {
            kind: LineKind::Deleted { base_line: 2 },
            ..line(1, "fn old() {}")
        };
        let excerpts = [Excerpt {
            lines: vec![deleted, line(1, "let x = 1;")],
        }];
        let source = TextSource {
            current: Some(Arc::from("let x = 1;\r\n")),
            base: Some(Arc::from("// gone\nfn old() {}\n")),
        };
        let (current, base) = highlight_job(
            Path::new("/nonexistent/a.rs"),
            Path::new("a.rs"),
            &excerpts,
            Some(&source),
            &registry,
            &|_| true,
        );
        assert!(current[&1].iter().any(|(_, c)| c.starts_with("keyword")));
        assert_eq!(
            base.keys().copied().collect::<Vec<_>>(),
            [2],
            "by base line"
        );
        assert!(
            base[&2]
                .iter()
                .any(|(r, c)| *r == (0..2) && c.starts_with("keyword"))
        );
        // Without a base, deleted lines stay plain; without a current text,
        // the disk is read (here: nothing there).
        let (current, base) = highlight_job(
            Path::new("/nonexistent/a.rs"),
            Path::new("a.rs"),
            &excerpts,
            None,
            &registry,
            &|_| true,
        );
        assert!(current.is_empty() && base.is_empty());
    }

    /// The per-row cut as it was: every row walks every span.
    fn spans_by_row_naive(
        bounds: &[(usize, usize)],
        spans: &[(Range<usize>, Arc<str>)],
    ) -> Vec<LineSpans> {
        bounds
            .iter()
            .map(|&(start, len)| {
                let mut line_spans = LineSpans::new();
                for (range, capture) in spans {
                    if range.end <= start {
                        continue;
                    }
                    let from = range.start.max(start) - start;
                    let to = (range.end - start).min(len);
                    if from < to {
                        line_spans.push((from..to, capture.clone()));
                    }
                }
                line_spans
            })
            .collect()
    }

    /// Row bounds of `text` split at `\n`.
    fn bounds_of(text: &str) -> Vec<(usize, usize)> {
        let mut start = 0;
        text.split('\n')
            .map(|row| {
                let b = (start, row.len());
                start += row.len() + 1;
                b
            })
            .collect()
    }

    #[test]
    fn spans_by_row_matches_the_per_row_walk() {
        // Rows: "fn a() {", "", "/* x", "y */ b", "}" with spans that
        // nest, cross rows, sit on a newline, start past the end and come
        // out of order (as injections may).
        let text = "fn a() {\n\n/* x\ny */ b\n}";
        let bounds = bounds_of(text);
        let c = |s: &str| Arc::<str>::from(s);
        let spans = vec![
            (0..2, c("keyword")),
            (0..24, c("block")),
            (8..9, c("newline")),
            (10..19, c("comment")),
            (3..4, c("function")),
            (20..21, c("var")),
            (30..40, c("past")),
            (9..9, c("empty")),
        ];
        let mut steps = 0;
        assert_eq!(
            spans_by_row(&bounds, &spans, &mut steps),
            spans_by_row_naive(&bounds, &spans)
        );
        assert_eq!(
            spans_by_row(&[], &spans, &mut steps),
            Vec::<LineSpans>::new()
        );
    }

    #[test]
    fn spans_by_row_is_linear_on_a_long_run() {
        // 5k rows, three one-row spans each: a whole-file tab's run.
        let rows = 5_000;
        let text = "let x = 1;\n".repeat(rows);
        let bounds = bounds_of(&text);
        let spans: Vec<(Range<usize>, Arc<str>)> = bounds
            .iter()
            .flat_map(|&(start, _)| {
                [
                    (start..start + 3, Arc::from("keyword")),
                    (start + 4..start + 5, Arc::from("variable")),
                    (start + 8..start + 9, Arc::from("number")),
                ]
            })
            .collect();
        let mut steps = 0;
        let cut = spans_by_row(&bounds, &spans, &mut steps);
        assert!(cut[..rows].iter().all(|s| s.len() == 3));
        assert_eq!(cut[rows - 1][2], (8..9, Arc::from("number")));
        // One row visit per span on a row (the three past the last
        // newline cover none), not rows × spans (75 million).
        assert_eq!(steps, rows * 3);
    }

    #[test]
    fn spans_are_clipped_to_cut_line() {
        let h = run(
            "a.rs",
            b"fn main() { let value = 1; }\n",
            vec![line(1, "fn main() ")],
        );
        let spans = &h[&1];
        assert!(!spans.is_empty());
        assert!(spans.iter().all(|(r, _)| r.end <= 10), "{spans:?}");
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    fn line(n: u32, matched: bool) -> LineMatch {
        LineMatch {
            line_number: n,
            line_text: format!("line {n}"),
            ranges: if matched {
                std::iter::once(0..1).collect()
            } else {
                vec![]
            },
            kind: LineKind::Normal,
        }
    }

    /// context, match, context
    fn excerpt(start: u32) -> Excerpt {
        Excerpt {
            lines: vec![
                line(start, false),
                line(start + 1, true),
                line(start + 2, false),
            ],
        }
    }

    fn path(s: &str) -> Arc<Path> {
        Arc::from(Path::new(s))
    }

    #[test]
    fn highlight_cache_needs_the_same_text() {
        let mut cache = HighlightCache::default();
        let ex = vec![excerpt(1)];
        let spans: LineSpans = vec![(0..4, "keyword".into())];
        cache.insert(
            path("a"),
            &ex,
            FileHighlights::from([(2, spans.clone())]),
            FileHighlights::new(),
            0,
        );
        assert_eq!(cache.get(Path::new("a"), &line(2, true)), Some(&spans));
        assert_eq!(
            cache.get(Path::new("a"), &line(1, false)),
            Some(&LineSpans::new()),
            "a line left out is cached plain"
        );
        assert!(!cache.missing(Path::new("a"), &ex, 0));
        assert!(cache.missing(Path::new("a"), &ex, 1), "a newer source");
        let changed = LineMatch {
            line_text: "changed".into(),
            ..line(2, true)
        };
        assert_eq!(cache.get(Path::new("a"), &changed), None);
        let changed_ex = vec![Excerpt {
            lines: vec![changed],
        }];
        assert!(cache.missing(Path::new("a"), &changed_ex, 0));
        assert!(cache.missing(Path::new("b"), &ex, 0));
    }

    #[test]
    fn missing_at_skips_the_scan_while_complete() {
        let mut cache = HighlightCache::default();
        let ex = vec![excerpt(1)];
        let a = Path::new("a");
        assert!(cache.missing_at(a, &ex, 0, 1));
        assert_eq!(cache.scans(), 1);
        cache.insert(
            path("a"),
            &ex,
            FileHighlights::new(),
            FileHighlights::new(),
            0,
        );
        assert!(!cache.missing_at(a, &ex, 0, 1));
        assert!(!cache.missing_at(a, &ex, 0, 1));
        assert_eq!(cache.scans(), 2, "complete: the second call does not scan");
        // A new results revision or source generation looks again.
        let changed = vec![Excerpt {
            lines: vec![LineMatch {
                line_text: "changed".into(),
                ..line(2, true)
            }],
        }];
        assert!(cache.missing_at(a, &changed, 0, 2));
        assert!(cache.missing_at(a, &ex, 1, 2));
        assert_eq!(cache.scans(), 4);
        // New colors arriving clear the memo.
        assert!(!cache.missing_at(a, &ex, 0, 3));
        cache.insert(
            path("a"),
            &[],
            FileHighlights::new(),
            FileHighlights::new(),
            0,
        );
        assert!(!cache.missing_at(a, &ex, 0, 3));
        assert_eq!(cache.scans(), 6, "scanned again after the insert");
    }

    #[test]
    fn base_and_current_lines_are_cached_apart() {
        let mut cache = HighlightCache::default();
        let deleted = LineMatch {
            line_text: "old".into(),
            kind: LineKind::Deleted { base_line: 7 },
            ..line(2, false)
        };
        let ex = vec![Excerpt {
            lines: vec![deleted.clone(), line(2, true)],
        }];
        let current: LineSpans = vec![(0..1, "keyword".into())];
        let old: LineSpans = vec![(0..3, "comment".into())];
        cache.insert(
            path("a"),
            &ex,
            FileHighlights::from([(2, current.clone())]),
            FileHighlights::from([(7, old.clone())]),
            3,
        );
        assert_eq!(cache.get(Path::new("a"), &deleted), Some(&old));
        assert_eq!(cache.get(Path::new("a"), &line(2, true)), Some(&current));
        assert!(!cache.missing(Path::new("a"), &ex, 3));
        assert_eq!(
            cache.file(Path::new("a")),
            Some(FileHighlights::from([(2, current)])),
            "current lines only"
        );
        // A newer source drops the old colors once its own arrive.
        cache.insert(
            path("a"),
            &[],
            FileHighlights::new(),
            FileHighlights::new(),
            4,
        );
        assert_eq!(cache.get(Path::new("a"), &deleted), None);
    }

    #[test]
    fn highlight_cache_drops_the_least_recently_used() {
        let mut cache = HighlightCache::default();
        let ex = vec![excerpt(1)];
        for i in 0..MAX_CACHED_FILES {
            cache.insert(
                path(&format!("f{i}")),
                &ex,
                FileHighlights::new(),
                FileHighlights::new(),
                0,
            );
        }
        cache.touch(Path::new("f0"));
        cache.insert(
            path("new"),
            &ex,
            FileHighlights::new(),
            FileHighlights::new(),
            0,
        );
        assert_eq!(cache.len(), MAX_CACHED_FILES);
        assert!(cache.file(Path::new("f0")).is_some(), "touched, kept");
        assert!(cache.file(Path::new("f1")).is_none(), "oldest, dropped");
        assert!(cache.file(Path::new("new")).is_some());
    }
}
