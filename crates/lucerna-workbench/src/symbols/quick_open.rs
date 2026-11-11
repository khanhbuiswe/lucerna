//! Quick Open `@` (spec §7.6): the active editor's symbols, filtered,
//! optionally grouped by kind (`@:`), with the selected one previewed in
//! the editor. Quick Open's view delegates its `@` rows here.

use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use gpui::{
    AnyElement, App, Div, Entity, HighlightStyle, IntoElement, ParentElement, SharedString,
    StyledText, div, prelude::*, px, svg,
};
use lucerna_syntax::{SymbolKind, SymbolTree};
use lucerna_text::Point;

use super::{SymbolState, kind_icon, kind_plural};
use crate::AppState;
use crate::code_view::CodeView;
use crate::fonts::editor_font_family;
use crate::layout::color;
use crate::locations_picker::highlighted_rows;
use crate::quick_open::picker::{PICKER_ROW_HEIGHT, fuzzy_filter, match_color};
use crate::ui::{FILE_ICON_SIZE, GRID, UI_FONT_SIZE_SMALL};

/// The empty states (spec §7.6).
pub(crate) const NO_SYMBOLS: &str = "No symbols found";
pub(crate) const LOADING_SYMBOLS: &str = "Loading symbols…";
pub(crate) const NO_SYMBOL_SOURCE: &str = "No language server or outline for this file";

/// Indentation per nesting level of an unfiltered list.
const INDENT: f32 = 12.;

/// What Quick Open reads from the workbench for `@`: the active editor
/// and its symbols when Quick Open opened.
pub(crate) struct SymbolsContext {
    pub path: Arc<Path>,
    pub editor: Entity<CodeView>,
    pub state: SymbolState,
    pub cursor: Point,
}

/// A symbol as a row shows it.
struct Item {
    name: SharedString,
    detail: Option<SharedString>,
    kind: SymbolKind,
    range: Range<Point>,
    selection_range: Range<Point>,
    depth: usize,
    /// The names of the containing symbols, outermost first.
    ancestors: Vec<SharedString>,
    /// The ancestors and the name, space-separated: what a query of
    /// several words matches.
    haystack: String,
    /// Where the name starts in `haystack`, in chars.
    name_start: u32,
}

/// Which symbol a row shows, kept across new symbol trees: its name and
/// selection range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SymbolId {
    name: SharedString,
    selection_range: Range<Point>,
}

/// One `@` row.
pub(crate) enum SymbolRow {
    /// A `@:` group: not selectable.
    Header { kind: SymbolKind, count: usize },
    Symbol {
        index: usize,
        /// Matched char indices into the name.
        positions: Vec<u32>,
        /// Unfiltered and ungrouped: indented by depth. Otherwise flat,
        /// with the ancestors at the end.
        nested: bool,
    },
}

impl SymbolRow {
    pub(crate) fn is_selectable(&self) -> bool {
        matches!(self, SymbolRow::Symbol { .. })
    }
}

/// The `@` mode of a Quick Open: the symbols, the editor previewing them
/// and where that editor was before the first preview.
pub(crate) struct SymbolPicker {
    path: Arc<Path>,
    editor: Entity<CodeView>,
    /// `Err` holds the empty-state message.
    items: Result<Vec<Item>, &'static str>,
    /// The index of the deepest symbol at the cursor.
    at_cursor: Option<usize>,
    cursor: Point,
    /// Cursor and scroll top row before the first preview.
    restore: Option<(Point, u32)>,
}

impl SymbolPicker {
    pub(crate) fn new(context: SymbolsContext) -> SymbolPicker {
        let mut picker = SymbolPicker {
            path: context.path,
            editor: context.editor,
            items: Err(LOADING_SYMBOLS),
            at_cursor: None,
            cursor: context.cursor,
            restore: None,
        };
        picker.set_state(context.state);
        picker
    }

    pub(crate) fn path(&self) -> &Arc<Path> {
        &self.path
    }

    pub(crate) fn set_state(&mut self, state: SymbolState) {
        (self.items, self.at_cursor) = match state {
            SymbolState::Ready(tree) => {
                let (items, at_cursor) = items(&tree, self.cursor);
                (Ok(items), at_cursor)
            }
            SymbolState::Loading => (Err(LOADING_SYMBOLS), None),
            SymbolState::Unavailable => (Err(NO_SYMBOL_SOURCE), None),
        };
    }

    /// The identity of item `index`.
    pub(crate) fn symbol_id(&self, index: usize) -> Option<SymbolId> {
        let item = self.items.as_ref().ok()?.get(index)?;
        Some(SymbolId {
            name: item.name.clone(),
            selection_range: item.selection_range.clone(),
        })
    }

    /// The index of the item `id` names.
    pub(crate) fn index_of(&self, id: &SymbolId) -> Option<usize> {
        self.items
            .as_ref()
            .ok()?
            .iter()
            .position(|item| item.name == id.name && item.selection_range == id.selection_range)
    }

    /// The rows for `text` and the row to select first: the deepest
    /// symbol at the cursor when `text` is empty, else the first symbol.
    /// `Err` is the message to show instead.
    pub(crate) fn rows(
        &self,
        text: &str,
        group_by_kind: bool,
    ) -> Result<(Vec<SymbolRow>, usize), &'static str> {
        let items = self.items.as_ref().map_err(|message| *message)?;
        let mut matches = filter(items, text);
        if matches.is_empty() {
            return Err(NO_SYMBOLS);
        }
        let nested = text.is_empty() && !group_by_kind;
        let mut rows = Vec::new();
        if group_by_kind {
            // Stable: document order (or best first) within a kind.
            matches.sort_by_key(|(index, _)| items[*index].kind);
            for group in matches.chunk_by(|a, b| items[a.0].kind == items[b.0].kind) {
                rows.push(SymbolRow::Header {
                    kind: items[group[0].0].kind,
                    count: group.len(),
                });
                rows.extend(group.iter().map(|(index, positions)| SymbolRow::Symbol {
                    index: *index,
                    positions: positions.clone(),
                    nested,
                }));
            }
        } else {
            rows.extend(
                matches
                    .into_iter()
                    .map(|(index, positions)| SymbolRow::Symbol {
                        index,
                        positions,
                        nested,
                    }),
            );
        }
        let at_cursor = self.at_cursor.filter(|_| text.is_empty());
        let selected = at_cursor
            .and_then(|at| {
                rows.iter()
                    .position(|r| matches!(r, SymbolRow::Symbol { index, .. } if *index == at))
            })
            .or_else(|| rows.iter().position(SymbolRow::is_selectable))
            .unwrap_or(0);
        Ok((rows, selected))
    }

    /// The row as plain text, for tests: the name, the detail and, for
    /// flat rows, `in` and the ancestors; nested rows start with two
    /// spaces per level. A header is `"{kind plural} ({count})"`.
    pub(crate) fn row_text(&self, row: &SymbolRow) -> String {
        match row {
            SymbolRow::Header { kind, count } => header_text(*kind, *count),
            SymbolRow::Symbol { index, nested, .. } => {
                let Ok(items) = &self.items else {
                    return String::new();
                };
                let item = &items[*index];
                let mut text = String::new();
                if *nested {
                    text.push_str(&"  ".repeat(item.depth));
                }
                text.push_str(&item.name);
                if let Some(detail) = &item.detail {
                    text.push(' ');
                    text.push_str(detail);
                }
                if let Some(ancestors) = ancestors_text(item, *nested) {
                    text.push(' ');
                    text.push_str(&ancestors);
                }
                text
            }
        }
    }

    /// Where Enter puts the cursor: the start of the selection range.
    pub(crate) fn target(&self, index: usize) -> Option<Point> {
        let items = self.items.as_ref().ok()?;
        Some(items.get(index)?.selection_range.start)
    }

    /// Scrolls the editor so the symbol's first row is in the middle and
    /// highlights its rows. The first preview remembers where the editor
    /// was, for [`SymbolPicker::end_preview`].
    pub(crate) fn preview(&mut self, index: usize, cx: &mut App) {
        let Some(item) = self.items.as_ref().ok().and_then(|items| items.get(index)) else {
            return;
        };
        let rows = highlighted_rows(&item.range);
        let top = item.range.start.row;
        if self.restore.is_none() {
            let v = self.editor.read(cx);
            self.restore = Some((v.cursor(), v.scroll_top_row()));
        }
        self.editor.update(cx, |v, cx| {
            v.set_line_highlights(vec![rows], cx);
            let half = v.visible_rows() / 2;
            v.set_scroll_top_row(top.saturating_sub(half), cx);
        });
    }

    /// Whether a preview moved the editor.
    pub(crate) fn is_previewing(&self) -> bool {
        self.restore.is_some()
    }

    /// Puts the editor back where it was before the first preview and
    /// clears the highlight.
    pub(crate) fn end_preview(&mut self, cx: &mut App) {
        let Some((cursor, scroll)) = self.restore.take() else {
            return;
        };
        self.editor.update(cx, |v, cx| {
            v.set_line_highlights(Vec::new(), cx);
            if v.cursor() != cursor {
                v.set_cursor(cursor, cx);
            }
            v.set_scroll_top_row(scroll, cx);
        });
    }

    /// The content of a row; the caller wraps a symbol in a picker row.
    pub(crate) fn render_row(&self, row: &SymbolRow, cx: &App) -> AnyElement {
        let ui = &cx.global::<AppState>().theme.ui;
        let muted = color(ui.text_muted);
        match row {
            SymbolRow::Header { kind, count } => div()
                .flex()
                .items_center()
                .h(px(PICKER_ROW_HEIGHT))
                .px(px(2. * GRID))
                .text_size(px(UI_FONT_SIZE_SMALL))
                .text_color(muted)
                .child(header_text(*kind, *count))
                .into_any_element(),
            SymbolRow::Symbol {
                index,
                positions,
                nested,
            } => {
                let Ok(items) = &self.items else {
                    return div().into_any_element();
                };
                let item = &items[*index];
                let indent = if *nested {
                    item.depth as f32 * INDENT
                } else {
                    0.
                };
                let name = matched(item.name.clone(), positions, cx);
                row_content(indent)
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
                            .font_family(editor_font_family(None, cx))
                            .child(name),
                    )
                    .children(
                        item.detail.clone().map(|detail| {
                            div().min_w_0().truncate().text_color(muted).child(detail)
                        }),
                    )
                    .children(ancestors_text(item, *nested).map(|ancestors| {
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .flex()
                            .justify_end()
                            .text_color(muted)
                            .child(ancestors)
                    }))
                    .into_any_element()
            }
        }
    }
}

fn row_content(indent: f32) -> Div {
    div()
        .flex()
        .flex_1()
        .min_w_0()
        .items_center()
        .gap(px(2. * GRID))
        .pl(px(indent))
}

fn header_text(kind: SymbolKind, count: usize) -> String {
    format!("{} ({count})", kind_plural(kind))
}

/// `in Foo › bar` for a flat row with ancestors.
fn ancestors_text(item: &Item, nested: bool) -> Option<String> {
    (!nested && !item.ancestors.is_empty()).then(|| {
        let names: Vec<&str> = item.ancestors.iter().map(|a| a.as_ref()).collect();
        format!("in {}", names.join(" › "))
    })
}

/// `text` with the chars at `positions` on the accent color at 30%
/// opacity, as Zed marks matches in its outline.
pub(crate) fn matched(text: SharedString, positions: &[u32], cx: &App) -> StyledText {
    let style = HighlightStyle {
        background_color: Some(match_color(cx).opacity(0.3)),
        ..Default::default()
    };
    let ranges: Vec<(Range<usize>, HighlightStyle)> = text
        .char_indices()
        .enumerate()
        .filter(|(i, _)| positions.binary_search(&(*i as u32)).is_ok())
        .map(|(_, (byte, ch))| (byte..byte + ch.len_utf8(), style))
        .collect();
    StyledText::new(text).with_highlights(ranges)
}

/// Every symbol of `tree` in document order, and the index of the
/// deepest one containing `cursor`, from one flatten pass.
fn items(tree: &SymbolTree, cursor: Point) -> (Vec<Item>, Option<usize>) {
    let deepest = tree.path_at(cursor).last().copied();
    let mut at_cursor = None;
    let items = tree
        .flatten()
        .into_iter()
        .enumerate()
        .map(|(index, flat)| {
            if deepest.is_some_and(|d| std::ptr::eq(d, flat.symbol)) {
                at_cursor = Some(index);
            }
            // "ancestors name", for queries of several words.
            let mut haystack = String::new();
            for ancestor in &flat.ancestors {
                haystack.push_str(ancestor);
                haystack.push(' ');
            }
            let name_start = haystack.chars().count() as u32;
            haystack.push_str(&flat.symbol.name);
            Item {
                name: flat.symbol.name.clone().into(),
                detail: flat.symbol.detail.clone().map(Into::into),
                kind: flat.symbol.kind,
                range: flat.symbol.range.clone(),
                selection_range: flat.symbol.selection_range.clone(),
                depth: flat.depth,
                ancestors: flat
                    .ancestors
                    .iter()
                    .map(|a| SharedString::from(a.to_string()))
                    .collect(),
                haystack,
                name_start,
            }
        })
        .collect();
    (items, at_cursor)
}

/// The items matching `text`, with matched char indices into their names:
/// all of them in order when `text` is empty. One word must match within
/// the name; several match against the ancestors and the name together
/// (as Zed's outline does), best first.
fn filter(items: &[Item], text: &str) -> Vec<(usize, Vec<u32>)> {
    if text.split_whitespace().nth(1).is_none() {
        return fuzzy_filter(items.iter().map(|i| i.name.as_ref()), text);
    }
    fuzzy_filter(items.iter().map(|i| i.haystack.as_str()), text)
        .into_iter()
        .map(|(index, positions)| {
            let start = items[index].name_start;
            let positions = positions
                .into_iter()
                .filter_map(|p| p.checked_sub(start))
                .collect();
            (index, positions)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lucerna_syntax::Symbol;

    fn pt(row: u32, column: u32) -> Point {
        Point { row, column }
    }

    fn symbol(name: &str, kind: SymbolKind, rows: Range<u32>, children: Vec<Symbol>) -> Symbol {
        let range = pt(rows.start, 0)..pt(rows.end, 0);
        Symbol {
            name: name.into(),
            detail: None,
            kind,
            selection_range: range.clone(),
            range,
            children,
        }
    }

    fn tree() -> SymbolTree {
        SymbolTree(vec![
            symbol(
                "Foo",
                SymbolKind::Class,
                0..10,
                vec![symbol("run", SymbolKind::Method, 1..3, vec![])],
            ),
            symbol("run_all", SymbolKind::Function, 11..12, vec![]),
        ])
    }

    #[test]
    fn one_word_matches_the_name_several_the_path() {
        let (items, _) = items(&tree(), pt(0, 0));
        let names = |text: &str| -> Vec<&str> {
            filter(&items, text)
                .into_iter()
                .map(|(i, _)| items[i].name.as_ref())
                .collect()
        };
        assert_eq!(names("foo"), vec!["Foo"]);
        assert_eq!(names("foo run"), vec!["run"]);
        let (_, positions) = &filter(&items, "foo run")[0];
        assert_eq!(positions, &vec![0, 1, 2], "positions fall in the name");
        assert_eq!(names("").len(), 3);
    }

    #[test]
    fn deepest_symbol_at_the_cursor() {
        let tree = tree();
        assert_eq!(items(&tree, pt(2, 0)).1, Some(1));
        assert_eq!(items(&tree, pt(5, 0)).1, Some(0));
        assert_eq!(items(&tree, pt(10, 5)).1, None);
    }
}
