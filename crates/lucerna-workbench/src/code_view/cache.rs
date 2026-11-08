//! Per-snapshot caches of the code view: highlight spans, shaped rows,
//! wrap counts and minimap rows. Each is bounded and keyed so a new
//! snapshot, font size, wrap width, theme or syntax never reuses stale
//! entries.

use std::collections::{HashMap, VecDeque};
use std::ops::Range;
use std::sync::Arc;

use lucerna_syntax::SyntaxState;
use lucerna_text::BufferSnapshot;

use super::element::ShapedRow;
use super::layout::wrap_count;
use super::minimap::MinimapRow;

/// Rows above and below the viewport whose highlights are fetched with it.
const HIGHLIGHT_MARGIN: u32 = 100;
/// Shaped rows kept; several screens' worth.
const SHAPED_ROWS: usize = 1024;
const WRAP_COUNTS: usize = 100_000;
const MINIMAP_ROWS: usize = 4096;

/// One highlight span on a row: absolute byte range and capture name.
pub(crate) type RowSpan = (Range<usize>, Arc<str>);

/// What shaped rows depend on besides the row itself.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ShapeKey {
    pub revision: u64,
    pub wrap_chars: Option<u32>,
    pub font_size: f32,
    pub tab_size: u32,
    /// Identity of the active theme.
    pub theme: usize,
}

#[derive(Default)]
pub(crate) struct Caches {
    /// Revision the highlights were fetched for, and the rows fetched.
    highlight_rows: Option<(u64, Range<u32>)>,
    highlights: HashMap<u32, Vec<RowSpan>>,
    shape_key: Option<ShapeKey>,
    shaped: HashMap<u32, Arc<ShapedRow>>,
    shaped_order: VecDeque<u32>,
    wrap_key: Option<(u64, Option<u32>)>,
    wraps: HashMap<u32, u32>,
    minimap_key: Option<(u64, usize)>,
    minimap: HashMap<u32, Arc<MinimapRow>>,
}

impl Caches {
    /// Drops everything; for a new snapshot.
    pub fn clear(&mut self) {
        *self = Caches::default();
    }

    /// Drops what depends on highlights.
    pub fn syntax_changed(&mut self) {
        self.highlight_rows = None;
        self.highlights.clear();
        self.shaped.clear();
        self.shaped_order.clear();
        self.minimap.clear();
    }

    pub fn highlight_rows(&self) -> Option<Range<u32>> {
        self.highlight_rows.as_ref().map(|(_, rows)| rows.clone())
    }

    /// Makes sure highlights cover `visible`, fetching `visible` ± 100 rows
    /// when they do not.
    pub fn ensure_highlights(
        &mut self,
        snapshot: &BufferSnapshot,
        revision: u64,
        syntax: &SyntaxState,
        visible: Range<u32>,
    ) {
        if let Some((rev, rows)) = &self.highlight_rows
            && *rev == revision
            && rows.start <= visible.start
            && rows.end >= visible.end
        {
            return;
        }
        let line_count = snapshot.line_count();
        let rows = visible.start.saturating_sub(HIGHLIGHT_MARGIN)
            ..visible.end.saturating_add(HIGHLIGHT_MARGIN).min(line_count);
        self.highlights.clear();
        for span in syntax.highlights(snapshot, rows.clone()) {
            if span.range.is_empty() {
                continue;
            }
            let first = snapshot.offset_to_point(span.range.start).row;
            let last = snapshot.offset_to_point(span.range.end - 1).row;
            for row in first.max(rows.start)..=last.min(rows.end.saturating_sub(1)) {
                self.highlights
                    .entry(row)
                    .or_default()
                    .push((span.range.clone(), span.capture.clone()));
            }
        }
        self.highlight_rows = Some((revision, rows));
    }

    /// Spans touching `row`, outer before inner; `None` when the row's
    /// highlights were not fetched.
    pub fn spans(&self, row: u32) -> Option<&[RowSpan]> {
        let (_, rows) = self.highlight_rows.as_ref()?;
        if !rows.contains(&row) {
            return None;
        }
        Some(self.highlights.get(&row).map_or(&[], Vec::as_slice))
    }

    pub fn shaped(&mut self, key: ShapeKey, row: u32) -> Option<Arc<ShapedRow>> {
        if self.shape_key != Some(key) {
            self.shape_key = Some(key);
            self.shaped.clear();
            self.shaped_order.clear();
        }
        self.shaped.get(&row).cloned()
    }

    pub fn insert_shaped(&mut self, row: u32, shaped: Arc<ShapedRow>) {
        if self.shaped.len() >= SHAPED_ROWS
            && let Some(oldest) = self.shaped_order.pop_front()
        {
            self.shaped.remove(&oldest);
        }
        if self.shaped.insert(row, shaped).is_none() {
            self.shaped_order.push_back(row);
        }
    }

    /// Display lines of `row`, cached per wrap width.
    pub fn wrap_count(
        &mut self,
        snapshot: &BufferSnapshot,
        revision: u64,
        row: u32,
        wrap_chars: Option<u32>,
    ) -> u32 {
        if wrap_chars.is_none() {
            return 1;
        }
        let key = (revision, wrap_chars);
        if self.wrap_key != Some(key) || self.wraps.len() >= WRAP_COUNTS {
            self.wrap_key = Some(key);
            self.wraps.clear();
        }
        *self
            .wraps
            .entry(row)
            .or_insert_with(|| wrap_count(snapshot, row, wrap_chars))
    }

    pub fn minimap_row(&mut self, key: (u64, usize), row: u32) -> Option<Arc<MinimapRow>> {
        if self.minimap_key != Some(key) || self.minimap.len() >= MINIMAP_ROWS {
            self.minimap_key = Some(key);
            self.minimap.clear();
        }
        let cached = self.minimap.get(&row)?;
        // A row drawn before its highlights arrived is redone.
        if !cached.highlighted && self.spans(row).is_some() {
            return None;
        }
        Some(cached.clone())
    }

    pub fn insert_minimap_row(&mut self, row: u32, minimap_row: Arc<MinimapRow>) {
        self.minimap.insert(row, minimap_row);
    }
}
