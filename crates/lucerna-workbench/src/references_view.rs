//! The References tab (`references-view.findReferences`, `⇧⌥F12`; spec
//! §7.5): the references of the symbol at the cursor as excerpts grouped
//! by file, like the Search tab's results, under a summary row with
//! collapse-all and refresh buttons.
//!
//! The excerpts are built off the UI thread: each file is read once, every
//! reference line gets one line of context above and below, and excerpts
//! that overlap or touch are merged. The tab is never saved with the
//! workspace.

use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{
    App, Context, Entity, FocusHandle, Focusable, IntoElement, KeyDownEvent, ParentElement, Render,
    SharedString, Styled, Subscription, Task, WeakEntity, Window, div, prelude::*, px,
};
use lucerna_lsp::locations::ResolvedLocation;
use lucerna_project::{Excerpt, LineKind, LineMatch, MAX_LINE_CHARS};
use lucerna_text::{Buffer, Point};

use crate::code_view::CodeView;
use crate::excerpt_list::model::{LineRef, Results};
use crate::excerpt_list::{ExcerptList, ExcerptListEvent};
use crate::goto::{GotoKind, message_word, no_result_text};
use crate::icons::{CHEVRONS_DOWN_UP, CHEVRONS_UP_DOWN, REFRESH_CW};
use crate::layout::color;
use crate::tasks::guarded;
use crate::ui::{GRID, UI_FONT_SIZE_SMALL, bar_icon_button, search_bar_row, search_toolbar};
use crate::{AppState, CommandRegistry, Documents, LanguageServers, OpenMode, Workbench};

/// The summary row's height.
const SUMMARY_HEIGHT: f32 = 8. * GRID;

/// Where a lookup ran.
#[derive(Clone)]
struct Lookup {
    /// Absolute.
    path: Arc<Path>,
    point: Point,
    /// The identifier at `point`, named by the title.
    word: String,
}

/// The excerpts of a finished lookup.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Built {
    /// Files (relative to the root when under it) with their excerpts.
    pub files: Vec<(Arc<Path>, Vec<Excerpt>)>,
    /// The column each reference line opens at, its first reference's,
    /// by file and line number.
    pub columns: HashMap<(Arc<Path>, u32), u32>,
    /// The references in the files shown.
    pub count: usize,
}

pub struct ReferencesView {
    workbench: WeakEntity<Workbench>,
    servers: Entity<LanguageServers>,
    documents: Entity<Documents>,
    /// A passive view of the lookup's file while the tab is open: it keeps
    /// the document open in the servers, so a refresh can still ask them
    /// after the file's own tab closed (say a preview replaced it).
    origin_view: Option<Entity<CodeView>>,
    root: PathBuf,
    focus: FocusHandle,
    list: Entity<ExcerptList>,
    /// Focus of the list (`list`'s handle): arrows and Enter work here.
    list_focus: FocusHandle,
    lookup: Option<Lookup>,
    /// `Some(references)` once the lookup finished.
    count: Option<usize>,
    /// The files of the finished lookup.
    files_shown: usize,
    columns: HashMap<(Arc<Path>, u32), u32>,
    /// Bumped by every lookup; answers of older ones are dropped.
    generation: u64,
    _lookup: Task<()>,
    _list_events: Subscription,
}

impl ReferencesView {
    pub fn new(
        root: PathBuf,
        workbench: WeakEntity<Workbench>,
        servers: Entity<LanguageServers>,
        documents: Entity<Documents>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let list = {
            let (root, settings) = (root.clone(), documents.read(cx).settings().clone());
            cx.new(|cx| ExcerptList::new(root, "references", settings, cx))
        };
        let list_focus = list.read(cx).focus_handle().clone();
        let list_events =
            cx.subscribe_in(&list, window, |view, _, event, window, cx| match event {
                ExcerptListEvent::Activate(r) => view.open_line(*r, window, cx),
            });
        ReferencesView {
            workbench,
            servers,
            documents,
            origin_view: None,
            root,
            focus: cx.focus_handle(),
            list,
            list_focus,
            lookup: None,
            count: None,
            files_shown: 0,
            columns: HashMap::new(),
            generation: 0,
            _lookup: Task::ready(()),
            _list_events: list_events,
        }
    }

    /// "References to `word`"; `None` (the tab says "References") before
    /// a lookup or when the cursor was on no word.
    pub fn title(&self) -> Option<SharedString> {
        let word = &self.lookup.as_ref()?.word;
        (!word.is_empty()).then(|| format!("References to {word}").into())
    }

    /// `"12 references in 4 files"`, or "No references found for 'word'";
    /// `None` while the lookup runs.
    pub fn summary(&self) -> Option<String> {
        let count = self.count?;
        let word = self.lookup.as_ref().map_or("", |l| l.word.as_str());
        Some(summary_text(count, self.files_shown, word))
    }

    /// The files with references, sorted by path, and the rows shown.
    pub fn results<'a>(&self, cx: &'a App) -> &'a Results {
        self.list.read(cx).results()
    }

    /// The selected line, at its current position.
    pub fn selected(&self, cx: &App) -> Option<LineRef> {
        self.list.read(cx).selected()
    }

    /// The focus handle of the list.
    pub fn list_focus_handle(&self) -> &FocusHandle {
        &self.list_focus
    }

    /// Looks up the references at `point` of the file `path` (absolute),
    /// named `word`, replacing the shown ones, and focuses the list.
    fn find(
        &mut self,
        path: Arc<Path>,
        point: Point,
        word: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let origin = self
            .documents
            .update(cx, |docs, cx| docs.open_preview(path.clone(), cx));
        self.origin_view = Some(origin);
        self.lookup = Some(Lookup { path, point, word });
        self.list.update(cx, |list, cx| list.clear(cx));
        self.columns.clear();
        self.files_shown = 0;
        // A new lookup: nothing of the previous one is kept, so the
        // refresh guard (keep results when no server can be asked) does
        // not apply and an answer still on its way is dropped.
        self.count = None;
        self.generation += 1;
        self.refresh(cx);
        window.focus(&self.list_focus.clone(), cx);
    }

    /// Runs the lookup again at the same place (the refresh button). The
    /// shown references stay until the answer replaces them, and stay as
    /// they are when no server can be asked (none running for the file).
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(lookup) = self.lookup.clone() else {
            return;
        };
        let can_ask = self
            .servers
            .read(cx)
            .can_request_locations(&lookup.path, GotoKind::References);
        if !can_ask && self.count.is_some() {
            tracing::debug!(path = ?lookup.path, "references refresh skipped: no server to ask");
            return;
        }
        self.generation += 1;
        self.count = None;
        cx.notify();
        let generation = self.generation;
        let request = self.servers.update(cx, |servers, cx| {
            servers.locations(&lookup.path, lookup.point, GotoKind::References, cx)
        });
        let root = self.root.clone();
        self._lookup = cx.spawn(async move |this, cx| {
            // Failed servers leave the list empty, as no references do.
            let locations = request.await.unwrap_or_default();
            let built = cx
                .background_spawn(async move {
                    guarded("references excerpts", || build(&root, &locations))
                })
                .await
                .unwrap_or_default();
            this.update(cx, |view, cx| view.show(generation, built, cx))
                .ok();
        });
    }

    /// Shows the excerpts of lookup `generation`; older ones are dropped.
    fn show(&mut self, generation: u64, built: Built, cx: &mut Context<Self>) {
        if generation != self.generation {
            return;
        }
        self.count = Some(built.count);
        self.files_shown = built.files.len();
        self.columns = built.columns;
        self.list
            .update(cx, |list, cx| list.set_files(built.files, cx));
        cx.notify();
    }

    /// The tab closed: lets the lookup's document close with its views.
    /// The results stay for when the tab opens again.
    pub(crate) fn release_origin(&mut self) {
        self.origin_view = None;
    }

    /// Opens the file of line `r` as a preview: a reference line at its
    /// first reference, a context line at its start. Recorded in the
    /// navigation history.
    pub fn open_line(&mut self, r: LineRef, _window: &mut Window, cx: &mut Context<Self>) {
        let results = self.results(cx);
        let (Some(file), Some(line), Some(workbench)) = (
            results.files().get(r.file),
            results.line(r),
            self.workbench.upgrade(),
        ) else {
            return;
        };
        let path = file.path.clone();
        let column = self
            .columns
            .get(&(path.clone(), line.line_number))
            .copied()
            .unwrap_or(0);
        let point = Point {
            row: line.line_number.saturating_sub(1),
            column,
        };
        workbench.update(cx, |wb, cx| {
            wb.set_references_used_last(true);
            wb.jump_to(path, point, OpenMode::Preview, cx);
        });
    }

    /// `F4` / `⇧F4`: selects the next / previous reference line (wrapping),
    /// unfolds its file and opens it as a preview.
    pub fn focus_match(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(r) = self
            .list
            .update(cx, |list, cx| list.step_match(forward, cx))
        {
            self.open_line(r, window, cx);
        }
    }

    /// Folds every file, or unfolds them all when all are folded.
    fn toggle_all_collapsed(&mut self, cx: &mut Context<Self>) {
        let all = self.results(cx).files().iter().all(|f| f.collapsed);
        self.list
            .update(cx, |list, cx| list.set_all_collapsed(!all, cx));
    }

    /// Keys in the list: `↑` / `↓` and `Enter`.
    fn list_key_down(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.list.update(cx, |list, cx| list.key_down(ev, cx)) {
            cx.stop_propagation();
        }
    }
}

impl Focusable for ReferencesView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ReferencesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = cx.global::<AppState>().theme.ui.clone();
        let files = self.results(cx).files();
        let all_collapsed = !files.is_empty() && files.iter().all(|f| f.collapsed);
        let collapse_icon = if all_collapsed {
            CHEVRONS_UP_DOWN
        } else {
            CHEVRONS_DOWN_UP
        };
        let summary = self.summary().unwrap_or_default();
        let bar = search_bar_row()
            .h(px(SUMMARY_HEIGHT))
            .child(
                div()
                    .debug_selector(|| "references-summary".into())
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(px(UI_FONT_SIZE_SMALL))
                    .text_color(color(ui.text_muted))
                    .child(summary),
            )
            .child(
                bar_icon_button("references-collapse-all", collapse_icon, false, cx)
                    .on_click(cx.listener(|view, _, _, cx| view.toggle_all_collapsed(cx))),
            )
            .child(
                bar_icon_button("references-refresh", REFRESH_CW, false, cx)
                    .on_click(cx.listener(|view, _, _, cx| view.refresh(cx))),
            );
        let toolbar = search_toolbar(cx)
            .debug_selector(|| "references-toolbar".into())
            .child(bar);
        div()
            .id("references-view")
            .debug_selector(|| "references-view".into())
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .size_full()
            .bg(color(ui.editor_background))
            .child(toolbar)
            .child(
                div()
                    .id("references-list")
                    .debug_selector(|| "references-list".into())
                    .track_focus(&self.list_focus)
                    .on_key_down(cx.listener(Self::list_key_down))
                    .flex_1()
                    .min_h_0()
                    .child(self.list.clone()),
            )
    }
}

/// `"{n} references in {m} files"`, singular for one; with no references,
/// "No references found for 'word'".
fn summary_text(references: usize, files: usize, word: &str) -> String {
    if references == 0 {
        return no_result_text(GotoKind::References, word);
    }
    let plural = |n: usize, word: &str| match n {
        1 => format!("1 {word}"),
        n => format!("{n} {word}s"),
    };
    format!(
        "{} in {}",
        plural(references, "reference"),
        plural(files, "file")
    )
}

/// The excerpts of `locations` (sorted by path, as
/// [`LanguageServers::locations`] returns them). Reads each file once, so
/// this blocks: call it off the UI thread. Unreadable files are left out.
pub(crate) fn build(root: &Path, locations: &[ResolvedLocation]) -> Built {
    let mut by_file: BTreeMap<&Arc<Path>, Vec<&ResolvedLocation>> = BTreeMap::new();
    for loc in locations {
        by_file.entry(&loc.path).or_default().push(loc);
    }
    let mut built = Built::default();
    for (path, locs) in by_file {
        let Ok(buffer) = Buffer::load(path) else {
            tracing::debug!(?path, "references: file unreadable");
            continue;
        };
        let snapshot = buffer.snapshot();
        let line_count = snapshot.line_count();
        let shown: Arc<Path> = match path.strip_prefix(root) {
            Ok(relative) => Arc::from(relative),
            Err(_) => path.clone(),
        };
        // The reference ranges of each row, in bytes of its line.
        let mut rows: BTreeMap<u32, Vec<Range<usize>>> = BTreeMap::new();
        let mut count = 0;
        for loc in locs {
            let (start, end) = (loc.range.start, loc.range.end);
            // An inverted range (a server bug) is left out, uncounted.
            if start.row >= line_count || (end.row, end.column) < (start.row, start.column) {
                continue;
            }
            count += 1;
            let first = built
                .columns
                .entry((shown.clone(), start.row + 1))
                .or_insert(start.column);
            *first = (*first).min(start.column);
            for row in start.row..=end.row.min(line_count - 1) {
                let len = snapshot.line(row).len();
                let from = if row == start.row {
                    start.column as usize
                } else {
                    0
                };
                let to = if row == end.row {
                    end.column as usize
                } else {
                    len
                };
                rows.entry(row).or_default().push(from..to.max(from));
            }
        }
        if rows.is_empty() {
            continue;
        }
        let excerpts = merged_windows(rows.keys().copied(), line_count)
            .into_iter()
            .map(|window| Excerpt {
                lines: window
                    .map(|row| {
                        let ranges = rows.get(&row).map(Vec::as_slice);
                        line_match(row, &snapshot.line(row), ranges)
                    })
                    .collect(),
            })
            .collect();
        built.count += count;
        built.files.push((shown, excerpts));
    }
    built
}

/// The rows shown around the reference `rows` (ascending): one line of
/// context above and below each, windows that overlap or touch merged.
fn merged_windows(rows: impl Iterator<Item = u32>, line_count: u32) -> Vec<Range<u32>> {
    let mut windows: Vec<Range<u32>> = Vec::new();
    for row in rows {
        let window = row.saturating_sub(1)..(row + 2).min(line_count);
        match windows.last_mut() {
            Some(last) if window.start <= last.end => last.end = last.end.max(window.end),
            _ => windows.push(window),
        }
    }
    windows
}

/// Line `row` cut at [`MAX_LINE_CHARS`] chars, as the search reports
/// lines, with `ranges` clamped to it on char boundaries. A reference line
/// always keeps a range (the empty one at the cut when every reference
/// lies past it), so it is never taken for context.
fn line_match(row: u32, line: &str, ranges: Option<&[Range<usize>]>) -> LineMatch {
    let keep = line
        .char_indices()
        .nth(MAX_LINE_CHARS)
        .map_or(line.len(), |(i, _)| i);
    let floor = |mut i: usize| {
        i = i.min(keep);
        while !line.is_char_boundary(i) {
            i -= 1;
        }
        i
    };
    let mut kept: Vec<Range<usize>> = ranges
        .unwrap_or_default()
        .iter()
        .filter(|r| r.start < keep)
        .map(|r| floor(r.start)..floor(r.end))
        .collect();
    kept.sort_by_key(|r| (r.start, r.end));
    kept.dedup();
    if ranges.is_some() && kept.is_empty() {
        kept.push(keep..keep);
    }
    LineMatch {
        line_number: row + 1,
        line_text: line[..keep].to_owned(),
        ranges: kept,
        kind: LineKind::Normal,
    }
}

/// `references-view.findReferences`: looks up the references at the
/// cursor of the active editor and shows them in the References tab,
/// opening it or bringing it to the front.
pub fn find_references_in_tab(
    wb: &mut Workbench,
    window: &mut Window,
    cx: &mut Context<Workbench>,
) {
    let Some(view) = wb.active_code_view(cx) else {
        return;
    };
    let (path, point, word) = {
        let v = view.read(cx);
        let point = v.cursor();
        (wb.absolute_path(v.path()), point, message_word(v, point))
    };
    wb.update_editor_groups(cx, |m| m.open_references());
    wb.references_view()
        .clone()
        .update(cx, |view, cx| view.find(path, point, word, window, cx));
    cx.notify();
}

/// Opens the next / previous reference. Deferred: opening a file updates
/// the workbench, which a command handler already holds.
fn focus_match(
    wb: &mut Workbench,
    forward: bool,
    window: &mut Window,
    cx: &mut Context<Workbench>,
) {
    let view = wb.references_view().clone();
    window.defer(cx, move |window, cx| {
        view.update(cx, |view, cx| view.focus_match(forward, window, cx))
    });
}

pub(crate) fn register_commands(commands: &mut CommandRegistry) {
    commands.register(
        "references-view.findReferences",
        "References: Find All References",
        |wb, _, window, cx| find_references_in_tab(wb, window, cx),
    );
    commands.register(
        "references-view.next",
        "References: Go to Next Reference",
        |wb, _, window, cx| focus_match(wb, true, window, cx),
    );
    commands.register(
        "references-view.prev",
        "References: Go to Previous Reference",
        |wb, _, window, cx| focus_match(wb, false, window, cx),
    );
}

#[cfg(test)]
#[allow(
    clippy::single_range_in_vec_init,
    reason = "a line with one reference has one range"
)]
mod tests {
    use super::*;

    #[test]
    fn summary_copy() {
        assert_eq!(summary_text(12, 4, "x"), "12 references in 4 files");
        assert_eq!(summary_text(1, 1, "x"), "1 reference in 1 file");
        assert_eq!(summary_text(3, 1, "x"), "3 references in 1 file");
        assert_eq!(summary_text(0, 0, "x"), "No references found for 'x'");
    }

    #[test]
    fn windows_merge_when_overlapping_or_touching() {
        // Rows 1, 3, 5 and 7 overlap one by one; 12 stands apart.
        assert_eq!(
            merged_windows([1, 3, 5, 7, 12].into_iter(), 14),
            vec![0..9, 11..14]
        );
        // Clamped at both ends.
        assert_eq!(merged_windows([0, 4].into_iter(), 5), vec![0..2, 3..5]);
        // Rows 1 and 4: windows 0..3 and 3..6 touch.
        assert_eq!(merged_windows([1, 4].into_iter(), 9), vec![0..6]);
        // Rows 1 and 5: row 3 lies between.
        assert_eq!(merged_windows([1, 5].into_iter(), 9), vec![0..3, 4..7]);
    }

    #[test]
    fn long_lines_are_cut_and_ranges_kept_on_char_boundaries() {
        let long = "é".repeat(MAX_LINE_CHARS + 10);
        let cut = MAX_LINE_CHARS * 2;
        // A range past the cut leaves the empty range at the cut.
        let m = line_match(4, &long, Some(&[cut + 4..cut + 6]));
        assert_eq!(m.line_number, 5);
        assert_eq!(m.line_text.len(), cut);
        assert_eq!(m.ranges, vec![cut..cut]);
        // One mid-char and across the cut: floored, clamped.
        let m = line_match(0, &long, Some(&[3..cut + 9]));
        assert_eq!(m.ranges, vec![2..cut]);
        // Context.
        assert!(line_match(0, "x", None).ranges.is_empty());
    }

    #[test]
    fn build_reads_files_and_merges_excerpts() {
        let dir = tempfile::TempDir::new().unwrap();
        let text: String = (1..=10).map(|n| format!("line {n}\n")).collect();
        std::fs::write(dir.path().join("a.txt"), &text).unwrap();
        let path: Arc<Path> = Arc::from(dir.path().join("a.txt"));
        let loc = |row: u32, start: u32, end: u32| ResolvedLocation {
            path: path.clone(),
            range: Point { row, column: start }..Point { row, column: end },
            line_text: String::new(),
        };
        let inverted = loc(5, 4, 1);
        let missing = ResolvedLocation {
            path: Arc::from(dir.path().join("gone.txt")),
            ..loc(0, 0, 1)
        };
        let built = build(
            dir.path(),
            &[
                loc(1, 0, 4),
                loc(1, 5, 6),
                loc(2, 0, 4),
                inverted,
                loc(8, 5, 6),
                missing,
            ],
        );
        assert_eq!(built.count, 4);
        assert_eq!(built.files.len(), 1);
        let (file, excerpts) = &built.files[0];
        assert_eq!(file.as_ref(), Path::new("a.txt"));
        let lines: Vec<(u32, Vec<Range<usize>>)> = excerpts
            .iter()
            .flat_map(|e| e.lines.iter().map(|l| (l.line_number, l.ranges.clone())))
            .collect();
        assert_eq!(excerpts.len(), 2);
        assert_eq!(
            lines,
            vec![
                (1, vec![]),
                (2, vec![0..4, 5..6]),
                (3, vec![0..4]),
                (4, vec![]),
                (8, vec![]),
                (9, vec![5..6]),
                (10, vec![]),
            ]
        );
        assert_eq!(built.columns[&(file.clone(), 2)], 0);
        assert_eq!(built.columns[&(file.clone(), 9)], 5);
    }
}
