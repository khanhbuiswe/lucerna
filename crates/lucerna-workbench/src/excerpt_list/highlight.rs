//! Syntax colors for excerpt lines. [`highlight_file`] is pure and
//! blocking: it parses the file with tree-sitter, so it runs on a
//! background thread and never touches gpui. [`HighlightCache`] keeps the
//! colors, and the queue on [`ExcerptList`] runs the jobs for the files on
//! screen, at most [`MAX_HIGHLIGHT_JOBS`] at once.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use gpui::{AppContext, Context};
use lucerna_project::{Excerpt, LineMatch};
use lucerna_syntax::{LanguageRegistry, SyntaxState};
use lucerna_text::{Buffer, Point};

use super::ExcerptList;
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

/// Blocking; call on a background thread. Empty when the file is too big,
/// cannot be loaded, has no language or its grammar fails. A line whose text
/// no longer starts with the excerpt's `line_text` (the file changed since
/// the search) is left out and shows plain.
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
    catch_panic("search highlight", || {
        highlight_loaded(abs_path, rel_path, excerpts, registry, styled)
    })
    .unwrap_or_default()
}

fn highlight_loaded(
    abs_path: &Path,
    rel_path: &Path,
    excerpts: &[Excerpt],
    registry: &Arc<LanguageRegistry>,
    styled: &dyn Fn(&str) -> bool,
) -> FileHighlights {
    let mut out = FileHighlights::new();
    let Ok(buffer) = Buffer::load(abs_path) else {
        return out;
    };
    let snapshot = buffer.snapshot();
    let Some(language) = registry.language_for_file(rel_path, &snapshot.line(0)) else {
        return out;
    };
    let Ok(mut syntax) = SyntaxState::new(language, registry.clone()) else {
        return out;
    };
    syntax.parse(&snapshot, None);
    let line_count = snapshot.line_count();

    for excerpt in excerpts {
        let rows: Vec<(u32, &LineMatch)> = excerpt
            .lines
            .iter()
            .filter(|l| l.line_number >= 1 && l.line_number <= line_count)
            .map(|l| (l.line_number - 1, l))
            .collect();
        let (Some(first), Some(last)) = (
            rows.iter().map(|(r, _)| *r).min(),
            rows.iter().map(|(r, _)| *r).max(),
        ) else {
            continue;
        };
        let spans = syntax.highlights(&snapshot, first..last + 1, styled);
        for (row, line) in rows {
            if !snapshot.line(row).starts_with(line.line_text.as_str()) {
                continue;
            }
            let start = snapshot.point_to_offset(Point { row, column: 0 });
            let len = line.line_text.len();
            let mut line_spans = LineSpans::new();
            for span in &spans {
                let from = span.range.start.max(start) - start;
                let to = span.range.end.saturating_sub(start).min(len);
                if span.range.end <= start || from >= to {
                    continue;
                }
                // Row text starts with line_text, so these are boundaries
                // unless the cut fell inside a char; line_text is a prefix
                // ending on a boundary, but spans may not be on one.
                if line.line_text.is_char_boundary(from) && line.line_text.is_char_boundary(to) {
                    line_spans.push((from..to, span.capture.clone()));
                }
            }
            out.insert(line.line_number, line_spans);
        }
    }
    out
}

/// The most files [`HighlightCache`] keeps colors for.
pub const MAX_CACHED_FILES: usize = 200;

/// Syntax colors of result lines, kept across searches so a re-search
/// does not flash its lines plain. Keyed by path, then line number; each
/// line holds the text its spans were computed for, and is used only
/// while the shown line has that same text. The least recently used files
/// beyond [`MAX_CACHED_FILES`] are dropped.
#[derive(Debug, Default)]
pub struct HighlightCache {
    files: HashMap<Arc<Path>, CachedFile>,
    tick: u64,
}

#[derive(Debug, Default)]
struct CachedFile {
    lines: HashMap<u32, (String, LineSpans)>,
    used: u64,
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
        let (text, spans) = self.files.get(path)?.lines.get(&line.line_number)?;
        (*text == line.line_text).then_some(spans)
    }

    /// Whether some line of `excerpts` has no colors for its text yet.
    pub fn missing(&self, path: &Path, excerpts: &[Excerpt]) -> bool {
        excerpts
            .iter()
            .flat_map(|e| &e.lines)
            .any(|l| self.get(path, l).is_none())
    }

    /// Marks `path` as just used, so it is the last to be dropped.
    pub fn touch(&mut self, path: &Path) {
        self.tick += 1;
        if let Some(f) = self.files.get_mut(path) {
            f.used = self.tick;
        }
    }

    /// Stores the colors computed for `excerpts`. A line the job left out
    /// (no language, a changed file) is stored plain, so it is not asked
    /// for again while its text stays the same.
    pub fn insert(
        &mut self,
        path: Arc<Path>,
        excerpts: &[Excerpt],
        mut highlights: FileHighlights,
    ) {
        self.tick += 1;
        let tick = self.tick;
        let file = self.files.entry(path.clone()).or_default();
        file.used = tick;
        for line in excerpts.iter().flat_map(|e| &e.lines) {
            let spans = highlights.remove(&line.line_number).unwrap_or_default();
            file.lines
                .insert(line.line_number, (line.line_text.clone(), spans));
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

    /// The colors of `path` by line number, for tests.
    pub fn file(&self, path: &Path) -> Option<FileHighlights> {
        let f = self.files.get(path)?;
        Some(
            f.lines
                .iter()
                .map(|(n, (_, spans))| (*n, spans.clone()))
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
            let path = result.path.clone();
            self.highlight_cache.touch(&path);
            if !self.highlight_cache.missing(&path, &result.excerpts)
                || !self.highlight_pending.insert(path.clone())
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
            let state = cx.global::<AppState>();
            let (languages, theme) = (state.languages.clone(), state.theme.clone());
            let (abs, rel, epoch) = (self.root.join(&path), path.clone(), self.highlight_epoch);
            let job = cx.background_spawn(async move {
                let highlights = guarded("search highlight", || {
                    highlight_file(&abs, &rel, &excerpts, &languages, &|c| {
                        theme.style_for(c).is_some()
                    })
                })
                .unwrap_or_default();
                (excerpts, highlights)
            });
            let key = path.clone();
            let task = cx.spawn(async move |this, cx| {
                let (excerpts, highlights) = job.await;
                this.update(cx, |list, cx| {
                    list.highlights_done(epoch, key, &excerpts, highlights, cx)
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
        highlights: FileHighlights,
        cx: &mut Context<Self>,
    ) {
        if epoch != self.highlight_epoch {
            return;
        }
        self.highlight_tasks.remove(&path);
        // A file whose excerpts a re-search replaced meanwhile may be
        // queued again for its new lines.
        self.highlight_pending.remove(&path);
        self.highlight_cache.insert(path, excerpts, highlights);
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
        cache.insert(path("a"), &ex, FileHighlights::from([(2, spans.clone())]));
        assert_eq!(cache.get(Path::new("a"), &line(2, true)), Some(&spans));
        assert_eq!(
            cache.get(Path::new("a"), &line(1, false)),
            Some(&LineSpans::new()),
            "a line left out is cached plain"
        );
        assert!(!cache.missing(Path::new("a"), &ex));
        let changed = LineMatch {
            line_text: "changed".into(),
            ..line(2, true)
        };
        assert_eq!(cache.get(Path::new("a"), &changed), None);
        let changed_ex = vec![Excerpt {
            lines: vec![changed],
        }];
        assert!(cache.missing(Path::new("a"), &changed_ex));
        assert!(cache.missing(Path::new("b"), &ex));
    }

    #[test]
    fn highlight_cache_drops_the_least_recently_used() {
        let mut cache = HighlightCache::default();
        let ex = vec![excerpt(1)];
        for i in 0..MAX_CACHED_FILES {
            cache.insert(path(&format!("f{i}")), &ex, FileHighlights::new());
        }
        cache.touch(Path::new("f0"));
        cache.insert(path("new"), &ex, FileHighlights::new());
        assert_eq!(cache.len(), MAX_CACHED_FILES);
        assert!(cache.file(Path::new("f0")).is_some(), "touched, kept");
        assert!(cache.file(Path::new("f1")).is_none(), "oldest, dropped");
        assert!(cache.file(Path::new("new")).is_some());
    }
}
