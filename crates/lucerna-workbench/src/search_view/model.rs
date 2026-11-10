//! The Search tab's results: files with excerpts, flattened into rows for the
//! list, plus match navigation. Pure data, no gpui.

use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use lucerna_project::{Excerpt, LineMatch, MAX_RESULTS, SearchQuery};

use crate::find_widget::build_regex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineRef {
    pub file: usize,
    pub excerpt: usize,
    pub line: usize,
}

/// A result line named by path and line number: unlike a [`LineRef`] it
/// stays valid while streamed results insert files before it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineAnchor {
    pub path: Arc<Path>,
    pub line_number: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Header(usize),
    Line(LineRef),
    Separator { file: usize },
}

#[derive(Debug, Clone)]
pub struct FileResult {
    pub path: Arc<Path>,
    pub excerpts: Vec<Excerpt>,
    pub collapsed: bool,
}

impl FileResult {
    pub fn match_count(&self) -> usize {
        self.excerpts.iter().map(Excerpt::match_count).sum()
    }
}

#[derive(Debug, Default)]
pub struct Results {
    files: Vec<FileResult>,
    rows: Vec<Row>,
}

impl Results {
    pub fn clear(&mut self) {
        self.files.clear();
        self.rows.clear();
    }

    /// Adds a file's excerpts, keeping files sorted by path. An existing
    /// path gets the excerpts appended.
    pub fn add_file(&mut self, path: Arc<Path>, excerpts: Vec<Excerpt>) {
        match self.files.binary_search_by(|f| f.path.cmp(&path)) {
            Ok(i) => self.files[i].excerpts.extend(excerpts),
            Err(i) => self.files.insert(
                i,
                FileResult {
                    path,
                    excerpts,
                    collapsed: false,
                },
            ),
        }
        self.rebuild();
    }

    pub fn files(&self) -> &[FileResult] {
        &self.files
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn line(&self, r: LineRef) -> Option<&LineMatch> {
        self.files
            .get(r.file)?
            .excerpts
            .get(r.excerpt)?
            .lines
            .get(r.line)
    }

    /// The index of the file `path` among [`files`](Self::files).
    pub fn file_index(&self, path: &Path) -> Option<usize> {
        self.files
            .binary_search_by(|f| f.path.as_ref().cmp(path))
            .ok()
    }

    /// Names line `r` by its file and line number, which stay put when
    /// streamed results shift file indices.
    pub fn anchor_of(&self, r: LineRef) -> Option<LineAnchor> {
        Some(LineAnchor {
            path: self.files.get(r.file)?.path.clone(),
            line_number: self.line(r)?.line_number,
        })
    }

    /// The current position of the line `anchor` names.
    pub fn resolve(&self, anchor: &LineAnchor) -> Option<LineRef> {
        let file = self.file_index(&anchor.path)?;
        self.files[file]
            .excerpts
            .iter()
            .enumerate()
            .find_map(|(excerpt, e)| {
                let line = e
                    .lines
                    .iter()
                    .position(|l| l.line_number == anchor.line_number)?;
                Some(LineRef {
                    file,
                    excerpt,
                    line,
                })
            })
    }

    pub fn toggle_collapsed(&mut self, file: usize) {
        if let Some(f) = self.files.get_mut(file) {
            f.collapsed = !f.collapsed;
            self.rebuild();
        }
    }

    pub fn set_all_collapsed(&mut self, collapsed: bool) {
        for f in &mut self.files {
            f.collapsed = collapsed;
        }
        self.rebuild();
    }

    pub fn expand(&mut self, file: usize) {
        if let Some(f) = self.files.get_mut(file) {
            f.collapsed = false;
            self.rebuild();
        }
    }

    pub fn row_of(&self, r: LineRef) -> Option<usize> {
        self.rows.iter().position(|row| *row == Row::Line(r))
    }

    pub fn match_count(&self) -> usize {
        self.files.iter().map(FileResult::match_count).sum()
    }

    /// The next (or previous) match line, wrapping around and skipping
    /// context lines. Collapsed files are included. From `None` it returns
    /// the first (forward) or last (backward) match.
    pub fn next_match(&self, from: Option<LineRef>, forward: bool) -> Option<LineRef> {
        let all: Vec<LineRef> = self.match_refs().collect();
        if all.is_empty() {
            return None;
        }
        let key = |r: &LineRef| (r.file, r.excerpt, r.line);
        let pos = from.map(|f| {
            let k = key(&f);
            all.partition_point(|r| key(r) < k)
        });
        let found = match (pos, forward) {
            (None, true) => 0,
            (None, false) => all.len() - 1,
            (Some(p), true) => {
                // First match strictly after `from`.
                let exact = all.get(p).is_some_and(|r| key(r) == key(&from.unwrap()));
                let next = if exact { p + 1 } else { p };
                next % all.len()
            }
            (Some(p), false) => (p + all.len() - 1) % all.len(),
        };
        all.get(found).copied()
    }

    fn match_refs(&self) -> impl Iterator<Item = LineRef> + '_ {
        self.files.iter().enumerate().flat_map(|(file, f)| {
            f.excerpts.iter().enumerate().flat_map(move |(excerpt, e)| {
                e.lines
                    .iter()
                    .enumerate()
                    .filter(|(_, l)| !l.ranges.is_empty())
                    .map(move |(line, _)| LineRef {
                        file,
                        excerpt,
                        line,
                    })
            })
        })
    }

    fn rebuild(&mut self) {
        self.rows.clear();
        for (file, f) in self.files.iter().enumerate() {
            self.rows.push(Row::Header(file));
            if f.collapsed {
                continue;
            }
            for (excerpt, e) in f.excerpts.iter().enumerate() {
                if excerpt > 0 {
                    self.rows.push(Row::Separator { file });
                }
                for line in 0..e.lines.len() {
                    self.rows.push(Row::Line(LineRef {
                        file,
                        excerpt,
                        line,
                    }));
                }
            }
        }
    }
}

/// The first non-empty match of `query` in `line`.
pub fn first_match(query: &SearchQuery, line: &str) -> Option<Range<usize>> {
    let regex = build_regex(
        &query.pattern,
        query.is_regex,
        query.case_sensitive,
        query.whole_word,
    )
    .ok()?;
    regex
        .find_iter(line)
        .find(|m| !m.is_empty())
        .map(|m| m.range())
}

/// Comma-separated globs, trimmed, empty ones dropped.
pub fn globs(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|g| !g.is_empty())
        .map(str::to_owned)
        .collect()
}

/// `20000` as `"20,000"`.
pub fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The status line: `done` is `None` while searching, else `Some(truncated)`.
pub fn summary(matches: usize, files: usize, searched: bool, done: Option<bool>) -> Option<String> {
    if !searched {
        return None;
    }
    if matches == 0 {
        return Some(match done {
            Some(_) => "No results found".into(),
            None => "Searching…".into(),
        });
    }
    let plural = |n: usize, word: &str| match n {
        1 => format!("1 {word}"),
        n => format!("{} {word}s", thousands(n)),
    };
    let mut out = format!("{} in {}", plural(matches, "result"), plural(files, "file"));
    if done == Some(true) {
        out.push_str(&format!(" (truncated at {})", thousands(MAX_RESULTS)));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
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
    fn rows_have_header_lines_and_separators() {
        let mut r = Results::default();
        r.add_file(path("a"), vec![excerpt(1), excerpt(10)]);
        let l = |excerpt, line| {
            Row::Line(LineRef {
                file: 0,
                excerpt,
                line,
            })
        };
        assert_eq!(
            r.rows(),
            &[
                Row::Header(0),
                l(0, 0),
                l(0, 1),
                l(0, 2),
                Row::Separator { file: 0 },
                l(1, 0),
                l(1, 1),
                l(1, 2),
            ]
        );
        assert_eq!(r.match_count(), 2);
    }

    #[test]
    fn collapsed_file_shows_only_header() {
        let mut r = Results::default();
        r.add_file(path("a"), vec![excerpt(1)]);
        r.toggle_collapsed(0);
        assert_eq!(r.rows(), &[Row::Header(0)]);
        r.set_all_collapsed(false);
        assert_eq!(r.rows().len(), 4);
    }

    #[test]
    fn files_stay_sorted_and_merge() {
        let mut r = Results::default();
        r.add_file(path("b"), vec![excerpt(1)]);
        r.add_file(path("a"), vec![excerpt(1)]);
        r.add_file(path("b"), vec![excerpt(20)]);
        let names: Vec<_> = r.files().iter().map(|f| f.path.to_path_buf()).collect();
        assert_eq!(names, [Path::new("a"), Path::new("b")]);
        assert_eq!(r.files()[1].excerpts.len(), 2);
    }

    #[test]
    fn anchor_survives_files_inserted_before_it() {
        let mut r = Results::default();
        r.add_file(path("b"), vec![excerpt(1)]);
        let at = LineRef {
            file: 0,
            excerpt: 0,
            line: 1,
        };
        let anchor = r.anchor_of(at).unwrap();
        assert_eq!(r.resolve(&anchor), Some(at));
        r.add_file(path("a"), vec![excerpt(5)]);
        let moved = LineRef { file: 1, ..at };
        assert_eq!(r.resolve(&anchor), Some(moved));
        assert_eq!(r.line(moved).unwrap().line_number, 2);
        assert_eq!(r.file_index(Path::new("b")), Some(1));
        r.clear();
        assert_eq!(r.resolve(&anchor), None);
    }

    #[test]
    fn next_match_skips_context_and_wraps() {
        let mut r = Results::default();
        r.add_file(path("a"), vec![excerpt(1)]);
        r.add_file(path("b"), vec![excerpt(1)]);
        let a = LineRef {
            file: 0,
            excerpt: 0,
            line: 1,
        };
        let b = LineRef {
            file: 1,
            excerpt: 0,
            line: 1,
        };
        assert_eq!(r.next_match(Some(a), true), Some(b));
        assert_eq!(r.next_match(Some(b), true), Some(a));
        assert_eq!(r.next_match(Some(a), false), Some(b));
        assert_eq!(r.next_match(Some(b), false), Some(a));
        // From a context line.
        let ctx = LineRef {
            file: 0,
            excerpt: 0,
            line: 2,
        };
        assert_eq!(r.next_match(Some(ctx), true), Some(b));
        assert_eq!(r.next_match(Some(ctx), false), Some(a));
    }

    #[test]
    fn next_match_crosses_collapsed_files() {
        let mut r = Results::default();
        r.add_file(path("a"), vec![excerpt(1)]);
        r.add_file(path("b"), vec![excerpt(1)]);
        r.set_all_collapsed(true);
        let a = LineRef {
            file: 0,
            excerpt: 0,
            line: 1,
        };
        let b = r.next_match(Some(a), true).unwrap();
        assert_eq!(b.file, 1);
        assert_eq!(r.row_of(b), None);
        r.expand(b.file);
        assert!(r.row_of(b).is_some());
    }

    #[test]
    fn next_match_from_none_is_first_or_last() {
        let mut r = Results::default();
        assert_eq!(r.next_match(None, true), None);
        r.add_file(path("a"), vec![excerpt(1), excerpt(10)]);
        assert_eq!(
            r.next_match(None, true),
            Some(LineRef {
                file: 0,
                excerpt: 0,
                line: 1
            })
        );
        assert_eq!(
            r.next_match(None, false),
            Some(LineRef {
                file: 0,
                excerpt: 1,
                line: 1
            })
        );
    }

    #[test]
    fn zero_width_match_line_counts() {
        let mut r = Results::default();
        let cut = LineMatch {
            line_number: 1,
            line_text: "x".into(),
            ranges: std::iter::once(5..5).collect(),
        };
        r.add_file(path("a"), vec![Excerpt { lines: vec![cut] }]);
        assert_eq!(r.match_count(), 1);
        assert!(r.next_match(None, true).is_some());
    }

    #[test]
    fn summary_strings() {
        assert_eq!(summary(0, 0, true, None).as_deref(), Some("Searching…"));
        assert_eq!(
            summary(0, 0, true, Some(false)).as_deref(),
            Some("No results found")
        );
        assert_eq!(
            summary(1, 1, true, Some(false)).as_deref(),
            Some("1 result in 1 file")
        );
        assert_eq!(
            summary(20_000, 3, true, Some(true)).as_deref(),
            Some("20,000 results in 3 files (truncated at 20,000)")
        );
        assert_eq!(summary(5, 1, false, None), None);
    }

    #[test]
    fn globs_split_on_commas() {
        assert_eq!(globs(" src/**, *.rs ,,"), vec!["src/**", "*.rs"]);
        assert!(globs("").is_empty());
    }

    #[test]
    fn thousands_separators() {
        assert_eq!(thousands(20_000), "20,000");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }
}
