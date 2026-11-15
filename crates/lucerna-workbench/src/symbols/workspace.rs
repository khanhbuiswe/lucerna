//! Quick Open `#` (spec §7.6): the symbols of the whole workspace, as the
//! language servers find them for a query, ranked here (in-project files
//! first, at most [`MAX_WORKSPACE_SYMBOLS`]) and shown in two-line rows.

use std::collections::HashSet;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gpui::{AnyElement, App, IntoElement, ParentElement, SharedString, div, prelude::*, px, svg};
use lucerna_lsp::ServerKey;
use lucerna_lsp::position::PositionEncoding;
use lucerna_syntax::SymbolKind;
use lucerna_text::Point;
use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

use super::kind_icon;
use super::quick_open::matched;
use crate::AppState;
use crate::fonts::editor_font_family;
use crate::layout::color;
use crate::ui::{FILE_ICON_SIZE, GRID, UI_FONT_SIZE_SMALL};

/// How long typing must pause before `workspace/symbol` is sent.
pub const WORKSPACE_SYMBOLS_DEBOUNCE: Duration = Duration::from_millis(100);
/// The most symbols `#` lists.
pub const MAX_WORKSPACE_SYMBOLS: usize = 100;

/// The empty states (spec §7.6).
pub(crate) const TYPE_TO_SEARCH: &str = "Type to search symbols";
pub(crate) const NO_SERVER: &str = "No language server is running";

/// Height of a two-line `#` row.
pub(crate) const WORKSPACE_ROW_HEIGHT: f32 = 11. * GRID;

/// A symbol `workspace/symbol` found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceSymbolItem {
    pub name: String,
    pub kind: SymbolKind,
    /// The server's `containerName`, e.g. the module or type.
    pub container: Option<String>,
    /// Absolute.
    pub path: Arc<Path>,
    /// `None` when the server sent only the file (a `WorkspaceSymbol`
    /// to resolve) or the file could not be read.
    pub range: Option<Range<Point>>,
    /// Under the root and not ignored.
    pub in_project: bool,
    /// The symbol to send to `workspaceSymbol/resolve` when the server
    /// sent no range.
    pub(crate) unresolved: Option<Unresolved>,
}

/// A `WorkspaceSymbol` without a range and the server that sent it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Unresolved {
    pub key: ServerKey,
    pub symbol: lsp_types::WorkspaceSymbol,
}

/// A symbol of a server's answer, before ranking and before its range
/// is read against the file's text.
pub(crate) struct Candidate {
    pub name: String,
    pub kind: SymbolKind,
    pub container: Option<String>,
    pub path: PathBuf,
    pub range: Option<lsp_types::Range>,
    pub encoding: PositionEncoding,
    pub unresolved: Option<Unresolved>,
}

/// The part of `query` the names are filtered by: after the last `::`,
/// so `foo::Bar` (which the server reads as a path) matches `Bar`.
fn filter_term(query: &str) -> &str {
    query.rsplit("::").next().unwrap_or(query).trim()
}

fn pattern(term: &str) -> Pattern {
    Pattern::new(
        term,
        CaseMatching::Smart,
        Normalization::Smart,
        AtomKind::Fuzzy,
    )
}

/// The candidates whose name fuzzy-matches `query` (see
/// [`filter_term`]), without duplicates, files `in_project` first, then
/// best match first; at most [`MAX_WORKSPACE_SYMBOLS`]. Each comes with
/// whether it is in the project.
pub(crate) fn rank(
    candidates: Vec<Candidate>,
    query: &str,
    in_project: impl Fn(&Path) -> bool,
) -> Vec<(Candidate, bool)> {
    let term = filter_term(query);
    let pattern = pattern(term);
    let mut matcher = Matcher::new(Config::DEFAULT);
    let mut buf = Vec::new();
    let mut seen = HashSet::new();
    let mut scored: Vec<(bool, u32, Candidate)> = candidates
        .into_iter()
        .filter(|c| {
            let range = c
                .range
                .map(|r| (r.start.line, r.start.character, r.end.line, r.end.character));
            seen.insert((c.path.clone(), c.name.clone(), c.kind, range))
        })
        .filter_map(|c| {
            let score = match term.is_empty() {
                true => 0,
                false => pattern.score(Utf32Str::new(&c.name, &mut buf), &mut matcher)?,
            };
            Some((in_project(&c.path), score, c))
        })
        .collect();
    scored.sort_by(|(a_project, a_score, a), (b_project, b_score, b)| {
        b_project
            .cmp(a_project)
            .then(b_score.cmp(a_score))
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.path.cmp(&b.path))
    });
    scored.truncate(MAX_WORKSPACE_SYMBOLS);
    scored
        .into_iter()
        .map(|(project, _, c)| (c, project))
        .collect()
}

/// The char indices of `name` that `query` matches (see [`filter_term`]).
pub(crate) fn match_positions(name: &str, query: &str) -> Vec<u32> {
    let term = filter_term(query);
    if term.is_empty() {
        return Vec::new();
    }
    let mut positions = Vec::new();
    let mut buf = Vec::new();
    pattern(term).indices(
        Utf32Str::new(name, &mut buf),
        &mut Matcher::new(Config::DEFAULT),
        &mut positions,
    );
    positions.sort_unstable();
    positions.dedup();
    positions
}

/// Where a row says the symbol is: the path relative to `root` (absolute
/// outside it) and the 1-based line, e.g. `src/main.rs:12`.
pub(crate) fn location_label(item: &WorkspaceSymbolItem, root: &Path) -> String {
    let path = item.path.strip_prefix(root).unwrap_or(&item.path);
    match &item.range {
        Some(range) => format!("{}:{}", path.display(), range.start.row + 1),
        None => path.display().to_string(),
    }
}

/// A row as plain text, for tests: name, container and location.
pub(crate) fn row_text(item: &WorkspaceSymbolItem, location: &str) -> String {
    let mut parts = vec![item.name.as_str()];
    parts.extend(item.container.as_deref());
    parts.push(location);
    parts.join(" ")
}

/// The content of a row (as Zed's project symbols): the kind icon, the
/// name in the code font with the matched chars marked and the container
/// dimmed; below, the location, small and dimmed.
pub(crate) fn render_row(
    item: &WorkspaceSymbolItem,
    positions: &[u32],
    location: SharedString,
    cx: &App,
) -> AnyElement {
    let ui = &cx.global::<AppState>().theme.ui;
    let muted = color(ui.text_muted);
    let name = matched(item.name.clone().into(), positions, cx);
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .child(
            div()
                .flex()
                .items_center()
                .min_w_0()
                .gap(px(2. * GRID))
                .child(
                    svg()
                        .flex_none()
                        .data(kind_icon(item.kind))
                        .size(px(FILE_ICON_SIZE))
                        .text_color(color(ui.icon_muted)),
                )
                .child(
                    div()
                        .flex_none()
                        .font_family(editor_font_family(
                            cx.global::<crate::AppState>().settings.view(None),
                            None,
                        ))
                        .child(name),
                )
                .children(
                    item.container
                        .clone()
                        .map(|c| div().min_w_0().truncate().text_color(muted).child(c)),
                ),
        )
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(px(UI_FONT_SIZE_SMALL))
                .text_color(muted)
                .child(location),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(name: &str, path: &str, line: u32) -> Candidate {
        let position = lsp_types::Position { line, character: 0 };
        Candidate {
            name: name.into(),
            kind: SymbolKind::Function,
            container: None,
            path: path.into(),
            range: Some(lsp_types::Range {
                start: position,
                end: position,
            }),
            encoding: PositionEncoding::Utf16,
            unresolved: None,
        }
    }

    fn names(ranked: &[(Candidate, bool)]) -> Vec<&str> {
        ranked.iter().map(|(c, _)| c.name.as_str()).collect()
    }

    #[test]
    fn project_first_then_score_deduplicated_capped() {
        let in_project = |p: &Path| p.starts_with("/root");
        let ranked = rank(
            vec![
                candidate("foo", "/ext/a.rs", 0),
                candidate("f_o_o", "/root/b.rs", 0),
                candidate("foo", "/ext/a.rs", 0),
                candidate("bar", "/root/b.rs", 1),
            ],
            "foo",
            in_project,
        );
        assert_eq!(names(&ranked), vec!["f_o_o", "foo"]);
        assert!(ranked[0].1, "in the project");
        assert!(!ranked[1].1, "outside");

        let many = (0..150)
            .map(|i| candidate(&format!("foo{i}"), "/ext/a.rs", i))
            .collect();
        assert_eq!(rank(many, "foo", in_project).len(), MAX_WORKSPACE_SYMBOLS);
    }

    #[test]
    fn path_queries_filter_on_the_last_segment() {
        let ranked = rank(
            vec![candidate("Bar", "/a.rs", 0), candidate("foo", "/a.rs", 1)],
            "foo::Bar",
            |_| true,
        );
        assert_eq!(names(&ranked), vec!["Bar"]);
        assert_eq!(match_positions("Bar", "foo::Bar"), vec![0, 1, 2]);
        assert_eq!(
            rank(vec![candidate("x", "/a.rs", 0)], "foo::", |_| true).len(),
            1
        );
    }
}
