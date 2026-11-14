//! Scroll sync between a markdown editor and its preview (spec §4.5,
//! §5.5): the line ↔ scroll mapping, and the links between a preview and
//! the editors of its file shown in other groups (the ⌘K V layout).

use std::cell::Cell;
use std::ops::Range;
use std::path::Path;
use std::rc::Rc;

use gpui::{App, Entity, EntityId, Subscription, Window};
use lucerna_text::{Bias, BufferSnapshot, Patch, Point};

use crate::AppState;
use crate::code_view::{CodeView, CodeViewEvent};
use crate::editor_groups::{EditorGroupsView, TabId, TabItem};
use crate::workspace_state::absolute_path;

use super::{MarkdownPreview, MarkdownPreviewEvent};

/// Scrolling the editor scrolls the preview (VS Code's setting).
const SCROLL_PREVIEW_WITH_EDITOR: &str = "markdown.preview.scrollPreviewWithEditor";
/// Scrolling the preview scrolls the editor (VS Code's setting).
const SCROLL_EDITOR_WITH_PREVIEW: &str = "markdown.preview.scrollEditorWithPreview";

/// The largest fraction into a block: a fraction stays below 1, so the
/// line it maps to stays in its block.
const MAX_FRACTION: f32 = 1. - f32::EPSILON;

/// The block (an index into `root_lines`) holding source line `line` and
/// how far into it the line is. A block spans its lines and the blank ones
/// after it, up to the next block's first line (or `line_count`), so a
/// blank line between two blocks belongs to the one above. Before the
/// first block, and with no blocks, `(0, 0.)`.
pub fn line_to_item(root_lines: &[Range<u32>], line_count: u32, line: f32) -> (usize, f32) {
    let item = root_lines.partition_point(|lines| lines.start as f32 <= line);
    let Some(item) = item.checked_sub(1) else {
        return (0, 0.);
    };
    let (start, span) = block_span(root_lines, line_count, item);
    (item, ((line - start) / span).clamp(0., MAX_FRACTION))
}

/// The source line `fraction` of the way into block `item` (clamped to
/// the last block): the inverse of [`line_to_item`]. 0 with no blocks.
pub fn item_to_line(root_lines: &[Range<u32>], line_count: u32, item: usize, fraction: f32) -> f32 {
    if root_lines.is_empty() {
        return 0.;
    }
    let (start, span) = block_span(root_lines, line_count, item.min(root_lines.len() - 1));
    start + fraction.clamp(0., 1.) * span
}

/// The first line of block `item` and the number of lines (at least 1)
/// up to the next block's first line, or to `line_count` for the last.
fn block_span(root_lines: &[Range<u32>], line_count: u32, item: usize) -> (f32, f32) {
    let start = root_lines[item].start;
    let next = root_lines.get(item + 1).map_or(line_count, |l| l.start);
    (start as f32, (next.saturating_sub(start)).max(1) as f32)
}

/// Where source line `line` (fractional) of `old` is in `new`, the text
/// after `patch`: moved by the text inserted or removed above it, so the
/// same text stays at the top of the preview. A line whose start was
/// replaced (a whole-file rewrite) keeps its number.
pub(crate) fn map_line(
    patch: &Patch,
    old: &BufferSnapshot,
    new: &BufferSnapshot,
    line: f32,
) -> f32 {
    let row = line.max(0.).floor() as u32;
    if row >= old.line_count() {
        return line;
    }
    let offset = old.point_to_offset(Point { row, column: 0 });
    let replaced = patch
        .edits()
        .iter()
        .any(|e| e.old.start <= offset && offset < e.old.end);
    if replaced {
        return line;
    }
    // Text inserted right at the line's start goes above it.
    let new_row = new
        .offset_to_point(patch.map_offset(offset, Bias::Right))
        .row;
    new_row as f32 + (line - row as f32)
}

/// The source line at the top of `preview`, as an editor row.
pub(crate) fn source_line_for_preview(preview: &MarkdownPreview) -> u32 {
    preview.top_line().floor().max(0.) as u32
}

/// Whether the boolean setting `key` is on (the default).
fn enabled(key: &str, cx: &App) -> bool {
    cx.global::<AppState>()
        .settings
        .get::<bool>(key, None)
        .unwrap_or(true)
}

/// The links between each shown Markdown preview and each shown editor of
/// the same file in another group. The workbench calls
/// [`ScrollSync::refresh`] whenever the editor groups change.
#[derive(Default)]
pub(crate) struct ScrollSync {
    /// The (preview, editor) pairs linked now.
    pairs: Vec<(EntityId, EntityId)>,
    _subscriptions: Vec<Subscription>,
}

impl ScrollSync {
    /// Links the previews and editors the groups show now, keeping the
    /// links when the pairs did not change. A newly shown pair starts with
    /// the preview at the editor's top row.
    pub(crate) fn refresh(
        &mut self,
        groups: &Entity<EditorGroupsView>,
        root: &Path,
        window: &mut Window,
        cx: &mut App,
    ) {
        let candidates = shown_pairs(groups, root, cx);
        if candidates.is_empty() {
            if !self.pairs.is_empty() {
                *self = ScrollSync::default();
            }
            return;
        }
        // Drawing makes the bodies anyway; the links need them now.
        groups.update(cx, |view, cx| view.ensure_active_bodies(window, cx));
        let groups = groups.read(cx);
        let pairs: Vec<(Entity<MarkdownPreview>, Entity<CodeView>)> = candidates
            .into_iter()
            .filter_map(|(preview, editor)| {
                let preview = groups.body_of(preview)?.downcast().ok()?;
                let editor = groups.body_of(editor)?.downcast().ok()?;
                Some((preview, editor))
            })
            .collect();
        let ids: Vec<(EntityId, EntityId)> = pairs
            .iter()
            .map(|(p, e)| (p.entity_id(), e.entity_id()))
            .collect();
        if ids == self.pairs {
            return;
        }
        let mut subscriptions = Vec::with_capacity(pairs.len() * 2);
        for (preview, editor) in pairs {
            let id = (preview.entity_id(), editor.entity_id());
            if !self.pairs.contains(&id) && enabled(SCROLL_PREVIEW_WITH_EDITOR, cx) {
                let row = editor.read(cx).scroll_top_row();
                preview.update(cx, |p, cx| p.scroll_to_line(row as f32, cx));
            }
            subscriptions.extend(link(preview, editor, cx));
        }
        self.pairs = ids;
        self._subscriptions = subscriptions;
    }
}

/// The (preview, editor) tabs shown in different groups for the same file.
fn shown_pairs(groups: &Entity<EditorGroupsView>, root: &Path, cx: &App) -> Vec<(TabId, TabId)> {
    let mut previews = Vec::new();
    let mut editors = Vec::new();
    for (gi, group) in groups.read(cx).model().groups().iter().enumerate() {
        let Some(tab) = group.tabs.iter().find(|t| Some(t.id) == group.active) else {
            continue;
        };
        match &tab.item {
            TabItem::File(path) => editors.push((gi, tab.id, absolute_path(root, path))),
            TabItem::MarkdownPreview(path) | TabItem::MarkdownPreviewDynamic(path) => {
                previews.push((gi, tab.id, absolute_path(root, path)))
            }
            _ => {}
        }
    }
    let mut pairs = Vec::new();
    for (pg, preview, ppath) in &previews {
        for (eg, editor, epath) in &editors {
            if pg != eg && ppath == epath {
                pairs.push((*preview, *editor));
            }
        }
    }
    pairs
}

/// Scrolls `preview` with `editor` and `editor` with `preview`, each when
/// its setting is on. The editor's reply to a scroll the preview made is
/// dropped, so it does not move the preview back to a block's start.
fn link(
    preview: Entity<MarkdownPreview>,
    editor: Entity<CodeView>,
    cx: &mut App,
) -> [Subscription; 2] {
    // The row the editor was scrolled to for the preview: its next
    // `ScrolledTo` with that row is that scroll's echo.
    let echo = Rc::new(Cell::new(None::<u32>));
    let editor_scrolled = {
        let (preview, echo) = (preview.downgrade(), echo.clone());
        cx.subscribe(&editor, move |_, event: &CodeViewEvent, cx| {
            let CodeViewEvent::ScrolledTo(row) = *event else {
                return;
            };
            if echo.take() == Some(row) || !enabled(SCROLL_PREVIEW_WITH_EDITOR, cx) {
                return;
            }
            preview
                .update(cx, |p, cx| p.scroll_to_line(row as f32, cx))
                .ok();
        })
    };
    let preview_scrolled = {
        let editor = editor.downgrade();
        cx.subscribe(&preview, move |preview, _: &MarkdownPreviewEvent, cx| {
            if !enabled(SCROLL_EDITOR_WITH_PREVIEW, cx) {
                return;
            }
            let row = source_line_for_preview(preview.read(cx));
            editor
                .update(cx, |e, cx| {
                    e.set_scroll_top_row(row, cx);
                    echo.set(Some(e.scroll_top_row()));
                })
                .ok();
        })
    };
    [editor_scrolled, preview_scrolled]
}
