//! An excerpt list's data: files with excerpts, flattened into rows for the
//! list, plus match navigation. Pure data, no gpui.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use lucerna_project::{Excerpt, LineMatch};

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

impl Row {
    /// The index of the file the row belongs to.
    pub fn file(&self) -> usize {
        match *self {
            Row::Header(file) | Row::Separator { file } => file,
            Row::Line(r) => r.file,
        }
    }

    /// Whether two rows are of the same kind, so are drawn the same height.
    pub fn same_kind(&self, other: &Row) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
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
    /// Match lines in all files, counted once per change.
    match_count: usize,
    /// The paths the running search has reported, as in Zed's reused
    /// excerpts: the other files are its predecessor's, kept on screen
    /// until [`prune`](Self::prune).
    confirmed: HashSet<Arc<Path>>,
}

impl Results {
    pub fn clear(&mut self) {
        self.files.clear();
        self.rows.clear();
        self.match_count = 0;
        self.confirmed.clear();
    }

    /// A new search starts over the shown files: they stay until it
    /// reports them again ([`update_files`](Self::update_files)) or
    /// finishes without them ([`prune`](Self::prune)).
    pub fn begin_update(&mut self) {
        self.confirmed.clear();
    }

    /// Applies a batch of the running search: the first report of a path
    /// replaces its excerpts in place (keeping its fold), a later one
    /// appends to them, and a new path is inserted in sorted order. Sorts
    /// and rebuilds the rows once.
    pub fn update_files(&mut self, batch: impl IntoIterator<Item = (Arc<Path>, Vec<Excerpt>)>) {
        let mut fresh: Vec<FileResult> = Vec::new();
        let mut fresh_ix: HashMap<Arc<Path>, usize> = HashMap::new();
        let mut changed = false;
        for (path, excerpts) in batch {
            changed = true;
            match self.files.binary_search_by(|f| f.path.cmp(&path)) {
                Ok(i) => {
                    if self.confirmed.insert(path) {
                        self.files[i].excerpts = excerpts;
                    } else {
                        self.files[i].excerpts.extend(excerpts);
                    }
                }
                Err(_) => match fresh_ix.get(&path) {
                    Some(&i) => fresh[i].excerpts.extend(excerpts),
                    None => {
                        self.confirmed.insert(path.clone());
                        fresh_ix.insert(path.clone(), fresh.len());
                        fresh.push(FileResult {
                            path,
                            excerpts,
                            collapsed: false,
                        });
                    }
                },
            }
        }
        if !changed {
            return;
        }
        if !fresh.is_empty() {
            self.files.extend(fresh);
            self.files.sort_by(|a, b| a.path.cmp(&b.path));
        }
        self.rebuild();
    }

    /// The running search finished: drops the files it did not report.
    /// Our walk is parallel, not path-ordered, so this waits for the end
    /// (Zed also prunes as its sorted stream passes a path).
    pub fn prune(&mut self) {
        let before = self.files.len();
        let confirmed = &self.confirmed;
        self.files.retain(|f| confirmed.contains(&f.path));
        if self.files.len() != before {
            self.rebuild();
        }
    }

    /// The largest line number among the files: sizes the gutter.
    pub fn max_line_number(&self) -> u32 {
        self.files
            .iter()
            .flat_map(|f| &f.excerpts)
            .filter_map(|e| e.lines.last())
            .map(|l| l.line_number)
            .max()
            .unwrap_or(0)
    }

    /// Adds a file's excerpts, keeping files sorted by path. An existing
    /// path gets the excerpts appended.
    pub fn add_file(&mut self, path: Arc<Path>, excerpts: Vec<Excerpt>) {
        self.add_files(std::iter::once((path, excerpts)));
    }

    /// Adds a batch of files like repeated [`add_file`](Self::add_file)
    /// calls (sorted by path, same paths merged in batch order) but sorts
    /// and rebuilds the rows once.
    pub fn add_files(&mut self, batch: impl IntoIterator<Item = (Arc<Path>, Vec<Excerpt>)>) {
        let mut fresh: Vec<FileResult> = Vec::new();
        for (path, excerpts) in batch {
            match self.files.binary_search_by(|f| f.path.cmp(&path)) {
                Ok(i) => self.files[i].excerpts.extend(excerpts),
                Err(_) => fresh.push(FileResult {
                    path,
                    excerpts,
                    collapsed: false,
                }),
            }
        }
        if fresh.is_empty() && self.files.is_empty() {
            return;
        }
        // Stable sort keeps batch order among equal paths; merge them.
        fresh.sort_by(|a, b| a.path.cmp(&b.path));
        let mut merged: Vec<FileResult> = Vec::with_capacity(fresh.len());
        for f in fresh {
            match merged.last_mut() {
                Some(last) if last.path == f.path => last.excerpts.extend(f.excerpts),
                _ => merged.push(f),
            }
        }
        if !merged.is_empty() {
            self.files.extend(merged);
            self.files.sort_by(|a, b| a.path.cmp(&b.path));
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

    /// Match lines in all files; kept up to date by every change.
    pub fn match_count(&self) -> usize {
        self.match_count
    }

    /// The 0-based position of the match line `r` among all match lines,
    /// or `None` when `r` is a context line or gone.
    pub fn match_index(&self, r: LineRef) -> Option<usize> {
        if self.line(r)?.ranges.is_empty() {
            return None;
        }
        let before: usize = self.files[..r.file]
            .iter()
            .map(FileResult::match_count)
            .sum();
        let file = &self.files[r.file];
        let in_excerpts: usize = file.excerpts[..r.excerpt]
            .iter()
            .map(Excerpt::match_count)
            .sum();
        let in_lines = file.excerpts[r.excerpt].lines[..r.line]
            .iter()
            .filter(|l| !l.ranges.is_empty())
            .count();
        Some(before + in_excerpts + in_lines)
    }

    /// The first match line of file `file`.
    pub fn first_match_of(&self, file: usize) -> Option<LineRef> {
        let f = self.files.get(file)?;
        f.excerpts.iter().enumerate().find_map(|(excerpt, e)| {
            let line = e.lines.iter().position(|l| !l.ranges.is_empty())?;
            Some(LineRef {
                file,
                excerpt,
                line,
            })
        })
    }

    /// The row of file `file`'s header.
    pub fn header_row(&self, file: usize) -> Option<usize> {
        self.rows.iter().position(|row| *row == Row::Header(file))
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
        self.match_count = self.files.iter().map(FileResult::match_count).sum();
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

/// A line with every tab replaced by spaces up to the next multiple of the
/// tab size, plus the mapping of byte offsets into the original line.
#[derive(Debug, PartialEq, Eq)]
pub struct ExpandedLine {
    pub text: String,
    /// `offsets[i]` is the expanded offset of original byte `i`; one entry
    /// per original byte plus the end.
    offsets: Vec<usize>,
}

impl ExpandedLine {
    /// The expanded offset of original byte `offset` (clamped to the end).
    pub fn map(&self, offset: usize) -> usize {
        self.offsets[offset.min(self.offsets.len() - 1)]
    }
}

/// Expands tabs of `line` to `tab_size` columns (at least 1), counting
/// columns in chars.
pub fn expand_tabs(line: &str, tab_size: u32) -> ExpandedLine {
    let tab_size = tab_size.max(1) as usize;
    let mut text = String::with_capacity(line.len());
    let mut offsets = Vec::with_capacity(line.len() + 1);
    let mut column = 0;
    for c in line.chars() {
        for _ in 0..c.len_utf8() {
            offsets.push(text.len());
        }
        if c == '\t' {
            let n = tab_size - column % tab_size;
            text.extend(std::iter::repeat_n(' ', n));
            column += n;
        } else {
            text.push(c);
            column += 1;
        }
    }
    offsets.push(text.len());
    ExpandedLine { text, offsets }
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
    fn match_index_counts_match_lines_across_files() {
        let mut r = Results::default();
        r.add_file(path("a"), vec![excerpt(1), excerpt(10)]);
        r.add_file(path("b"), vec![excerpt(1)]);
        let at = |file, excerpt, line| LineRef {
            file,
            excerpt,
            line,
        };
        assert_eq!(r.match_index(at(0, 0, 1)), Some(0));
        assert_eq!(r.match_index(at(0, 1, 1)), Some(1));
        assert_eq!(r.match_index(at(1, 0, 1)), Some(2));
        assert_eq!(r.match_index(at(0, 0, 0)), None, "a context line");
        assert_eq!(r.match_index(at(5, 0, 0)), None, "gone");
        assert_eq!(r.first_match_of(1), Some(at(1, 0, 1)));
        assert_eq!(r.header_row(1), Some(8));
        assert!(Row::Header(0).same_kind(&Row::Header(3)));
        assert!(!Row::Header(0).same_kind(&Row::Separator { file: 0 }));
        assert_eq!(Row::Line(at(1, 0, 1)).file(), 1);
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
    fn add_files_matches_repeated_add_file() {
        let batch = || {
            vec![
                (path("c"), vec![excerpt(1)]),
                (path("a"), vec![excerpt(5)]),
                (path("b"), vec![excerpt(1)]),
                (path("a"), vec![excerpt(50)]),
                (path("c"), vec![excerpt(9)]),
            ]
        };
        let mut one = Results::default();
        one.add_file(path("b"), vec![excerpt(30)]);
        let mut many = Results::default();
        many.add_file(path("b"), vec![excerpt(30)]);
        for (p, e) in batch() {
            one.add_file(p, e);
        }
        many.add_files(batch());
        let shape = |r: &Results| {
            r.files()
                .iter()
                .map(|f| {
                    let starts: Vec<u32> =
                        f.excerpts.iter().map(|e| e.lines[0].line_number).collect();
                    (f.path.to_path_buf(), starts)
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(shape(&one), shape(&many));
        assert_eq!(one.rows(), many.rows());
        assert_eq!(many.files().len(), 3);
        assert_eq!(many.files()[0].excerpts.len(), 2);
    }

    #[test]
    fn expand_tabs_maps_offsets_on_mixed_line() {
        // "\té\tab\tx": tab(4) é(1) tab(to 8: 3) a b tab(to 12: 2)...
        let line = "\té\tab\tx";
        let e = expand_tabs(line, 4);
        assert_eq!(e.text, "    é   ab  x");
        // Original offsets: \t=0, é=1..3, \t=3, a=4, b=5, \t=6, x=7, end=8.
        assert_eq!(e.map(0), 0);
        assert_eq!(e.map(1), 4);
        assert_eq!(e.map(3), 4 + "é".len());
        assert_eq!(e.map(4), 4 + "é".len() + 3);
        assert_eq!(e.map(7), e.text.len() - 1);
        assert_eq!(e.map(8), e.text.len());
        assert_eq!(&e.text[e.map(4)..e.map(6)], "ab");
        assert!(e.text.is_char_boundary(e.map(2)));
        assert_eq!(expand_tabs("plain", 4).text, "plain");
        assert_eq!(expand_tabs("\t", 0).text, " ");
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
    fn thousands_separators() {
        assert_eq!(thousands(20_000), "20,000");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }

    fn paths(r: &Results) -> Vec<String> {
        r.files()
            .iter()
            .map(|f| f.path.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn re_search_replaces_in_place_and_prunes_at_done() {
        let mut r = Results::default();
        r.update_files([(path("a"), vec![excerpt(1)]), (path("b"), vec![excerpt(1)])]);
        r.toggle_collapsed(1);
        r.begin_update();
        // Nothing of the new search yet: the old files stay.
        assert_eq!(paths(&r), ["a", "b"]);
        r.update_files([
            (path("b"), vec![excerpt(20), excerpt(30)]),
            (path("c"), vec![excerpt(1)]),
        ]);
        assert_eq!(paths(&r), ["a", "b", "c"], "a waits for the end");
        let b = &r.files()[1];
        assert_eq!(b.excerpts.len(), 2, "b's excerpts replaced, not appended");
        assert_eq!(b.excerpts[0].lines[0].line_number, 20);
        assert!(b.collapsed, "the fold is kept");
        // A second report of a path in the same search appends.
        r.update_files([(path("b"), vec![excerpt(40)])]);
        assert_eq!(r.files()[1].excerpts.len(), 3);
        r.prune();
        assert_eq!(paths(&r), ["b", "c"]);
        assert_eq!(r.match_count(), 4);
        assert_eq!(r.max_line_number(), 42);
    }
}
