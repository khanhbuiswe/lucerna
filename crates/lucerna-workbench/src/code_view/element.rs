//! The custom element that lays out, shapes and paints the visible rows:
//! gutter, text, selection, bracket boxes and whitespace marks. It also
//! turns mouse and scroll events into view changes.

mod hunk_bars;
mod paint;

pub(crate) use hunk_bars::bar_width as hunk_bar_width;

use std::sync::Arc;

use gpui::{
    App, Bounds, CursorStyle, DispatchPhase, Element, ElementId, ElementInputHandler, Font,
    GlobalElementId, Hitbox, HitboxBehavior, Hsla, InspectorElementId, IntoElement, LayoutId,
    MouseButton, MouseDownEvent, MouseExitEvent, MouseMoveEvent, MouseUpEvent, Pixels, ScrollEvent,
    ShapedLine, SharedString, Style, TextRun, UnderlineStyle, Window, font, point, px, relative,
    size, underline_y_offset,
};
use lucerna_editor::multicursor::ColumnPos;
use lucerna_lsp::diagnostics::Severity;
use lucerna_text::Point;
use lucerna_theme::{Theme, UiColors};

use super::cache::ShapeKey;
use super::caret::CaretStyle;
use super::diagnostics::MarkStyle;
use super::layout::{FoldState, char_columns, display_lines_with_tab_size, row_range, row_start};
use super::selection;
use super::{CodeView, CursorOverlay, CursorShape, Drag, hsla, link};
use crate::AppState;
use crate::hover::PopoverAnchor;
use crate::ui::ScrollMetrics;

/// Rows shaped below the viewport so the next scroll step is ready.
const OVERSCAN_ROWS: u32 = 3;
/// Gutter width in chars, beyond the line number digits.
const GUTTER_EXTRA_CHARS: f32 = 4.;
/// Left padding of the text, in chars.
const TEXT_PADDING_CHARS: f32 = 0.5;
const BRACKET_BOX_ALPHA: f32 = 0.4;
const ACTIVE_MATCH_ALPHA: f32 = 0.6;
const CURSOR_ALPHA: f32 = 0.4;
/// Most search matches painted in one frame (a minified line can hold
/// thousands).
const MAX_PAINTED_MATCHES: usize = 2_000;
/// The diagnostics icon column at the gutter's left edge, and its icons.
const DIAGNOSTIC_COLUMN: f32 = 16.;
const DIAGNOSTIC_ICON: f32 = 12.;
/// The gap before an inline diagnostic, in ems.
const INLINE_GAP_EMS: f32 = 4.;
const INLINE_PADDING_X: f32 = 4.;
const INLINE_RADIUS: f32 = 2.;
const INLINE_BACKGROUND_ALPHA: f32 = 0.05;
/// What is left of faded (`Unnecessary`) text: the background drawn over
/// it at 70%.
const FADE_ALPHA: f32 = 0.7;
const SCROLLBAR_MARK_HEIGHT: f32 = 2.;

/// Maps between a wrapped piece's buffer columns and the bytes of its
/// displayed text, where tabs are expanded to spaces.
#[derive(Debug, Clone, Default)]
pub(crate) struct ColumnMap {
    /// Buffer column (byte in the line) where the piece starts.
    buf_start: u32,
    /// Buffer bytes the piece shows (the `…` of a cut line is not one).
    buf_len: u32,
    /// Expanded tabs: (buffer byte in piece, display byte, display width).
    tabs: Vec<(u32, u32, u32)>,
}

impl ColumnMap {
    fn contains(&self, column: u32) -> bool {
        (self.buf_start..=self.buf_start + self.buf_len).contains(&column)
    }

    /// The display byte of buffer `column`, clamped to the piece.
    fn to_display(&self, column: u32) -> usize {
        let rel = column.clamp(self.buf_start, self.buf_start + self.buf_len) - self.buf_start;
        let extra: u32 = self
            .tabs
            .iter()
            .take_while(|(buf, _, _)| *buf < rel)
            .map(|(_, _, width)| width - 1)
            .sum();
        (rel + extra) as usize
    }

    /// The buffer column at display byte `display`.
    fn to_buffer(&self, display: usize) -> u32 {
        let display = display as u32;
        let mut extra = 0;
        for &(buf, disp, width) in &self.tabs {
            if display < disp {
                break;
            }
            if display < disp + width {
                // Inside the tab: snap to its nearer side.
                let after = display - disp > width / 2;
                return self.buf_start + buf + u32::from(after);
            }
            extra += width - 1;
        }
        self.buf_start + (display - extra).min(self.buf_len)
    }

    fn is_tab_fill(&self, display: usize) -> bool {
        let display = display as u32;
        self.tabs
            .iter()
            .any(|&(_, disp, width)| (disp..disp + width).contains(&display))
    }
}

/// One wrapped piece of a row, shaped.
pub(crate) struct Piece {
    pub line: ShapedLine,
    pub map: ColumnMap,
}

/// A buffer row's shaped pieces (one unless wrapped).
pub(crate) struct ShapedRow {
    pub pieces: Vec<Piece>,
}

/// One display line on screen.
pub(crate) struct FrameRow {
    /// Absolute y of the line's top.
    top: Pixels,
    buffer_row: u32,
    shaped: Arc<ShapedRow>,
    piece: usize,
}

impl FrameRow {
    fn piece(&self) -> &Piece {
        &self.shaped.pieces[self.piece]
    }

    fn is_last_piece(&self) -> bool {
        self.piece + 1 == self.shaped.pieces.len()
    }

    fn x_for(&self, column: u32) -> Pixels {
        let piece = self.piece();
        piece.line.x_for_index(piece.map.to_display(column))
    }
}

/// Everything one frame laid out: geometry for painting and hit testing.
pub(crate) struct Frame {
    bounds: Bounds<Pixels>,
    text_bounds: Bounds<Pixels>,
    /// Absolute x of column 0, after horizontal scrolling.
    text_x: Pixels,
    line_height: Pixels,
    char_width: Pixels,
    /// Chars per display line when wrapping.
    pub wrap_chars: Option<u32>,
    rows: Vec<FrameRow>,
    numbers: Vec<(Pixels, ShapedLine)>,
    /// Fold chevrons of every fold header on screen: (top, header row,
    /// chevron, painted). All of them take clicks; only those shown while
    /// the gutter is hovered, or folded, are painted.
    chevrons: Vec<(Pixels, u32, ShapedLine, bool)>,
    /// The non-empty selections touching the rows on screen.
    selections: Vec<std::ops::Range<Point>>,
    /// The IME composition's marked ranges on screen, underlined.
    marked: Vec<std::ops::Range<Point>>,
    /// The cells of the carets to paint (every cursor on screen), while
    /// the view has focus, no cursor overlay is set and the blink is on.
    carets: Vec<std::ops::Range<Point>>,
    caret_style: CaretStyle,
    /// The cell of the cursor overlay: the char under it on one row, empty
    /// at the line end or on an empty line.
    cursor_overlay: Option<std::ops::Range<Point>>,
    /// Search matches touching the rows on screen; `true` for the active one.
    matches: Vec<(std::ops::Range<Point>, bool)>,
    /// The range of the hover shown, painted with `element.hover`.
    hover_range: Option<std::ops::Range<Point>>,
    cursor_row: Option<u32>,
    /// Tops of the display lines in [`CodeView::set_line_highlights`].
    highlighted_lines: Vec<Pixels>,
    brackets: Vec<Point>,
    /// `·` and `→`, when whitespace is rendered.
    whitespace: Option<(ShapedLine, ShapedLine)>,
    hitbox: Option<Hitbox>,
    /// The scrollbar's geometry: one row's height per buffer row shown.
    scroll_metrics: ScrollMetrics,
    diagnostics: FrameDiagnostics,
    /// Git change bars and deletion pills in the gutter.
    hunk_bars: Vec<hunk_bars::HunkBar>,
    /// The clickable bars (`scm.diffDecorationsGutterAction` is `diff`):
    /// each one's visible part and the row a click on it opens.
    hunk_hitboxes: Vec<(Hitbox, u32)>,
    /// How far the diagnostic icons move right to clear the change bars.
    icon_shift: Pixels,
    colors: FrameColors,
}

/// The diagnostics on screen, laid out.
#[derive(Default)]
pub(crate) struct FrameDiagnostics {
    /// Marks touching the rows on screen, with their color.
    marks: Vec<(std::ops::Range<Point>, MarkStyle, Hsla)>,
    /// From a display line's top to its underline.
    underline_y: Pixels,
    /// Gutter icons: (top, icon, color).
    icons: Vec<(Pixels, DiagnosticIcon, Hsla)>,
    inline: Vec<InlineDiagnostic>,
    /// Scrollbar marks: (offset from the top, color).
    scrollbar: Arc<[(Pixels, Hsla)]>,
}

/// A gutter icon: its sprite atlas key and SVG.
pub(crate) type DiagnosticIcon = (&'static str, &'static [u8]);

/// The scrollbar marks as last laid out, with what they depend on.
pub(crate) struct ScrollbarMarks {
    /// [`CodeView`]'s diagnostic rows generation.
    rows_generation: u64,
    folds: FoldState,
    height: Pixels,
    theme_generation: u64,
    marks: Arc<[(Pixels, Hsla)]>,
}

#[derive(Clone, Copy)]
struct FrameColors {
    background: Hsla,
    gutter: Hsla,
    active_line: Hsla,
    highlighted_line: Hsla,
    selection: Hsla,
    bracket: Hsla,
    search_match: Hsla,
    active_match: Hsla,
    hover: Hsla,
    cursor: Hsla,
    caret: Hsla,
    fade: Hsla,
    strike: Hsla,
}

/// An inline diagnostic message after a row's text.
struct InlineDiagnostic {
    buffer_row: u32,
    pill: Bounds<Pixels>,
    origin: gpui::Point<Pixels>,
    text: ShapedLine,
    color: Hsla,
}

/// Where [`CodeView::layout_diagnostics`] puts things.
struct DiagnosticGeometry {
    text_bounds: Bounds<Pixels>,
    text_x: Pixels,
    line_height: Pixels,
    font_size: Pixels,
    icon_column: bool,
    underline_y: Pixels,
}

fn severity_color(ui: &UiColors, severity: Severity) -> Hsla {
    hsla(match severity {
        Severity::Error => ui.error,
        Severity::Warning => ui.warning,
        Severity::Information => ui.info,
        Severity::Hint => ui.hint,
    })
}

fn severity_icon(severity: Severity) -> DiagnosticIcon {
    match severity {
        Severity::Error => ("lucerna-diagnostic-circle-x", crate::icons::CIRCLE_X),
        Severity::Warning => (
            "lucerna-diagnostic-triangle-alert",
            crate::icons::TRIANGLE_ALERT,
        ),
        Severity::Information | Severity::Hint => ("lucerna-diagnostic-info", crate::icons::INFO),
    }
}

impl Frame {
    pub fn scroll_metrics(&self) -> ScrollMetrics {
        self.scroll_metrics
    }

    pub fn visible_rows(&self) -> u32 {
        (self.bounds.size.height / self.line_height).floor().max(0.) as u32
    }

    pub fn gutter_width(&self) -> f32 {
        f32::from(self.text_bounds.left() - self.bounds.left())
    }

    pub fn hunk_bar_count(&self) -> usize {
        self.hunk_bars.len()
    }

    /// The visible part of bar `i` (a deletion pill is half clipped).
    pub fn hunk_bar_bounds(&self, i: usize) -> Option<Bounds<Pixels>> {
        let bar = self.hunk_bars.get(i)?;
        Some(bar.bounds.intersect(&self.bounds))
    }

    pub fn inline_rows(&self) -> Vec<u32> {
        self.diagnostics
            .inline
            .iter()
            .map(|i| i.buffer_row)
            .collect()
    }

    /// The buffer position under `position`, clamped to the rows on screen.
    pub(crate) fn point_at(&self, position: gpui::Point<Pixels>) -> Option<Point> {
        self.point_past_end(position).map(|(p, _)| p)
    }

    /// [`Frame::point_at`], and how many whole columns `position` lies
    /// past the end of its buffer line (0 inside the text, or at the end
    /// of a wrapped piece that is not the line's last). For `⇧⌥`-drag,
    /// which may select past the line end.
    pub(crate) fn point_past_end(&self, position: gpui::Point<Pixels>) -> Option<(Point, u32)> {
        let row = self
            .rows
            .iter()
            .find(|r| position.y < r.top + self.line_height)
            .or(self.rows.last())?;
        let piece = row.piece();
        let x = position.x - self.text_x;
        let index = piece.line.closest_index_for_x(x);
        let past = if row.is_last_piece() && x > piece.line.width() && self.char_width > px(0.) {
            ((x - piece.line.width()) / self.char_width).round().max(0.) as u32
        } else {
            0
        };
        Some((
            Point {
                row: row.buffer_row,
                column: piece.map.to_buffer(index),
            },
            past,
        ))
    }

    /// The char with text under `position`: over the text area, inside a
    /// line's text, and not whitespace.
    fn char_at(&self, position: gpui::Point<Pixels>) -> Option<Point> {
        if position.x < self.text_bounds.left() || !self.text_bounds.contains(&position) {
            return None;
        }
        let row = self.row_at(position.y)?;
        let piece = row.piece();
        let index = piece.line.index_for_x(position.x - self.text_x)?;
        if piece.map.is_tab_fill(index) {
            return None;
        }
        let column = piece.map.to_buffer(index);
        if column >= piece.map.buf_start + piece.map.buf_len {
            return None;
        }
        let text = piece.line.text.get(index..)?;
        text.chars()
            .next()
            .filter(|c| !c.is_whitespace())
            .map(|_| Point {
                row: row.buffer_row,
                column,
            })
    }

    /// The left of the char at `p` and the top of its display line, when
    /// on screen.
    fn position_of(&self, p: Point) -> Option<(Pixels, Pixels)> {
        let row = self.rows.iter().find(|r| {
            let map = &r.piece().map;
            r.buffer_row == p.row
                && p.column >= map.buf_start
                && (p.column < map.buf_start + map.buf_len || r.is_last_piece())
        })?;
        Some((self.text_x + row.x_for(p.column), row.top))
    }

    /// The middle of the char at `p`.
    pub fn char_position(&self, p: Point) -> Option<gpui::Point<Pixels>> {
        let (x, top) = self.position_of(p)?;
        Some(point(x + self.char_width / 2., top + self.line_height / 2.))
    }

    /// The cell of the char at `p` (one char wide, one line high).
    pub fn char_bounds(&self, p: Point) -> Option<Bounds<Pixels>> {
        let (x, top) = self.position_of(p)?;
        Some(Bounds::new(
            point(x, top),
            size(self.char_width, self.line_height),
        ))
    }

    fn row_at(&self, y: Pixels) -> Option<&FrameRow> {
        self.rows
            .iter()
            .find(|r| y >= r.top && y < r.top + self.line_height)
    }
}

pub(crate) struct CodeElement {
    view: gpui::Entity<CodeView>,
}

impl CodeElement {
    pub fn new(view: gpui::Entity<CodeView>) -> Self {
        CodeElement { view }
    }
}

impl IntoElement for CodeElement {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for CodeElement {
    type RequestLayoutState = ();
    type PrepaintState = Arc<Frame>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Arc<Frame> {
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let frame = self
            .view
            .update(cx, |view, cx| view.layout_frame(bounds, hitbox, window, cx));
        let anchor_at = |p: Point| {
            let (x, line_top) = frame.position_of(p)?;
            Some(PopoverAnchor {
                editor: frame.bounds,
                x,
                line_top,
                line_height: frame.line_height,
                char_width: frame.char_width,
            })
        };
        let anchor = self.view.read(cx).hover().anchor().and_then(anchor_at);
        crate::hover::prepaint_popover(&self.view, anchor, window, cx);
        let anchor = self.view.read(cx).goto_message_anchor().and_then(anchor_at);
        crate::goto::prepaint_message(&self.view, anchor, window, cx);
        frame
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        frame: &mut Arc<Frame>,
        window: &mut Window,
        cx: &mut App,
    ) {
        // Typed text and IME input reach the view while it has focus.
        let focus = self.view.read(cx).focus.clone();
        window.handle_input(
            &focus,
            ElementInputHandler::new(frame.bounds, self.view.clone()),
            cx,
        );
        paint::paint_frame(frame, window, cx);
        if let Some(hitbox) = &frame.hitbox {
            // The link shows only while the mouse is over it.
            let style = if self.view.read(cx).link_range().is_some() {
                CursorStyle::PointingHand
            } else {
                CursorStyle::IBeam
            };
            window.set_cursor_style(style, hitbox);
        }
        for (hitbox, _) in &frame.hunk_hitboxes {
            window.set_cursor_style(CursorStyle::PointingHand, hitbox);
        }
        register_mouse(&self.view, frame, window);
    }
}

impl CodeView {
    fn base_font(&self) -> Font {
        font(self.config.font_family.clone())
    }

    /// Lays out the rows that fit in `bounds`, shaping only those (and a
    /// few more below) that are not cached.
    fn layout_frame(
        &mut self,
        bounds: Bounds<Pixels>,
        hitbox: Hitbox,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Arc<Frame> {
        let theme = cx.global::<AppState>().theme.clone();
        let ui = &theme.ui;
        let font_size = px(self.font_size);
        let line_height = px(self.line_height());
        let text_system = window.text_system();
        let font_id = text_system.resolve_font(&self.base_font());
        let char_width = text_system
            .em_advance(font_id, font_size)
            .ok()
            .filter(|w| *w > px(0.))
            .unwrap_or(font_size * 0.6);
        let underline_y = underline_y_offset(
            line_height,
            text_system.ascent(font_id, font_size),
            text_system.descent(font_id, font_size),
        );

        let line_count = self.snapshot.line_count();
        let digits = line_count.max(1).ilog10() + 1;
        let icon_column = self.config.gutter_icons && self.diagnostics_source;
        let gutter_width = char_width * (digits.max(2) as f32 + GUTTER_EXTRA_CHARS)
            + if icon_column {
                px(DIAGNOSTIC_COLUMN)
            } else {
                px(0.)
            };
        let text_bounds = Bounds {
            origin: point(bounds.left() + gutter_width, bounds.top()),
            size: size(
                (bounds.size.width - gutter_width).max(px(0.)),
                bounds.size.height,
            ),
        };
        let padding = char_width * TEXT_PADDING_CHARS;
        let wrap_chars = self.word_wrap.then(|| {
            ((text_bounds.size.width - padding * 2.) / char_width)
                .floor()
                .max(1.) as u32
        });
        if self.frame.as_ref().map(|f| f.wrap_chars) != Some(wrap_chars) {
            self.scroll.offset = 0.;
        }
        let visible_rows = (bounds.size.height / line_height).floor().max(0.) as u32;
        if let Some(row) = self.pending_center
            && visible_rows > 0
        {
            self.center(row, visible_rows);
        }
        let last = line_count.saturating_sub(1);
        self.scroll.top_row = self.folds.prev_visible(self.scroll.top_row.min(last));

        // Which rows show: walk down from the top using wrap counts only.
        let mut rows = Vec::new();
        let mut y = bounds.top() - px(self.scroll.offset);
        let mut row = self.scroll.top_row;
        let mut extra = 0;
        while row < line_count && extra <= OVERSCAN_ROWS {
            if y >= bounds.bottom() {
                extra += 1;
            }
            let count = self.caches.wrap_count(
                &self.snapshot,
                self.revision,
                row,
                wrap_chars,
                self.config.tab_size,
            );
            rows.push((row, y));
            y += line_height * count as f32;
            row = self.folds.next_visible(row + 1);
        }

        if let (false, Some(syntax), Some(&(first, _)), Some(&(end, _))) =
            (self.large, &self.syntax, rows.first(), rows.last())
            && syntax.tree().is_some()
        {
            let syntax = syntax.clone();
            let app = cx.global::<AppState>();
            self.caches.ensure_highlights(
                &self.snapshot,
                self.revision,
                &syntax,
                &app.theme,
                app.theme_generation,
                first..end + 1,
            );
        }

        let key = ShapeKey {
            revision: self.revision,
            wrap_chars,
            font_size: self.font_size,
            tab_size: self.config.tab_size,
            theme: cx.global::<AppState>().theme_generation,
        };
        let link = self.link_range();
        let mut frame_rows = Vec::new();
        let mut max_width = px(0.);
        for &(row, top) in &rows {
            let link = link.as_ref().and_then(|r| link::row_columns(r, row));
            let shaped = self.shaped_row(key, row, link, &theme, window);
            if top >= bounds.bottom() {
                continue;
            }
            for (piece, p) in shaped.pieces.iter().enumerate() {
                max_width = max_width.max(p.line.width());
                frame_rows.push(FrameRow {
                    top: top + line_height * piece as f32,
                    buffer_row: row,
                    shaped: shaped.clone(),
                    piece,
                });
            }
        }
        // The widest line seen so far, so scrolling vertically into short
        // lines does not snap the view back to the left.
        self.max_line_width = self.max_line_width.max(f32::from(max_width));
        let max_scroll_x =
            (px(self.max_line_width) + padding * 2. - text_bounds.size.width).max(px(0.));
        self.scroll_x = if wrap_chars.is_some() {
            0.
        } else {
            self.scroll_x.clamp(0., f32::from(max_scroll_x))
        };
        let text_x = text_bounds.left() + padding - px(self.scroll_x);

        let cursor_overlay = self.block_cursor().and_then(|at| self.cell_at(at));
        let cursor_row = match &cursor_overlay {
            Some(cell) => Some(cell.start.row),
            None => Some(self.selection_head().row),
        };
        let show_carets =
            cursor_overlay.is_none() && self.caret.visible() && self.focus.is_focused(window);
        let (selections, carets) = match (frame_rows.first(), frame_rows.last()) {
            (Some(first), Some(last)) => {
                self.visible_selections(first.buffer_row..last.buffer_row + 1, show_carets)
            }
            _ => (Vec::new(), Vec::new()),
        };
        let highlighted_lines = frame_rows
            .iter()
            .filter(|r| self.is_line_highlighted(r.buffer_row))
            .map(|r| r.top)
            .collect();
        let number_color = hsla(ui.editor_line_number);
        let active_color = hsla(ui.editor_active_line_number);
        let mut numbers = Vec::new();
        let mut chevrons = Vec::new();
        for r in frame_rows.iter().filter(|r| r.piece == 0) {
            let color = if Some(r.buffer_row) == cursor_row {
                active_color
            } else {
                number_color
            };
            let label = (r.buffer_row + 1).to_string();
            let shaped = shape(window, label, font_size, self.base_font(), color);
            numbers.push((r.top, shaped));
            if self
                .fold_ranges
                .binary_search_by_key(&r.buffer_row, |f| f.start)
                .is_ok()
            {
                let glyph = if self.folds.is_folded(r.buffer_row) {
                    "▸"
                } else {
                    "▾"
                };
                let shaped = shape(window, glyph, font_size, self.base_font(), number_color);
                let shown = self.fold_chevron_visible(r.buffer_row);
                chevrons.push((r.top, r.buffer_row, shaped, shown));
            }
        }
        let whitespace = self.config.render_whitespace.then(|| {
            let muted = hsla(ui.text_muted);
            (
                shape(window, "·", font_size, self.base_font(), muted),
                shape(window, "→", font_size, self.base_font(), muted),
            )
        });
        let mut matches = match (frame_rows.first(), frame_rows.last()) {
            (Some(first), Some(last)) => {
                self.visible_matches(first.buffer_row..last.buffer_row + 1)
            }
            _ => Vec::new(),
        };
        if let (Some(first), Some(last)) = (frame_rows.first(), frame_rows.last()) {
            let from = row_start(&self.snapshot, first.buffer_row);
            let to = row_start(&self.snapshot, last.buffer_row + 1);
            let len = self.snapshot.len_bytes();
            matches.extend(
                self.flash
                    .iter()
                    .filter(|r| r.end > from && r.start < to)
                    .take(MAX_PAINTED_MATCHES)
                    .map(|r| {
                        let range = self.snapshot.offset_to_point(r.start.min(len))
                            ..self.snapshot.offset_to_point(r.end.min(len));
                        (range, false)
                    }),
            );
        }
        let brackets = self
            .brackets
            .iter()
            .flatten()
            .map(|&offset| self.snapshot.offset_to_point(offset))
            .collect();

        // The thumb follows buffer rows (folded ones left out), not
        // wrapped display lines: one line's height each. It shows while the
        // rows overflow, and its range is the real scroll range, which
        // ends with the last row at the top.
        let lh = f32::from(line_height);
        let viewport = f32::from(bounds.size.height);
        let shown_rows = line_count - self.folds.hidden_before(line_count);
        let rows_above = self.scroll.top_row - self.folds.hidden_before(self.scroll.top_row);
        let scroll_metrics = ScrollMetrics {
            viewport,
            content: if shown_rows as f32 * lh > viewport {
                shown_rows.saturating_sub(1) as f32 * lh + viewport
            } else {
                viewport
            },
            // The top row's display lines share its one line of the scale.
            scroll_top: rows_above as f32 * lh
                + self.scroll.offset / self.wrap_count(self.scroll.top_row).max(1) as f32,
        };

        let scrollbar_marks = self.scrollbar_marks(bounds.size.height, ui, cx);
        let diagnostics = self.layout_diagnostics(
            &frame_rows,
            DiagnosticGeometry {
                text_bounds,
                text_x,
                line_height,
                font_size,
                icon_column,
                underline_y,
            },
            scrollbar_marks,
            ui,
            window,
        );
        let (hunk_bars, icon_shift) = if self.config.diff_gutter {
            let bars = hunk_bars::layout(
                &frame_rows,
                &self.diff_hunks,
                &self.folds,
                bounds.left(),
                line_height,
                hunk_bars::HunkColors::new(ui),
            );
            // The bars sit at the gutter's left edge; the icons move right
            // only when the bars would cover them.
            let inset = px((DIAGNOSTIC_COLUMN - DIAGNOSTIC_ICON) / 2.);
            let width = hunk_bars::bar_width(line_height);
            (bars, if width > inset { width } else { px(0.) })
        } else {
            (Vec::new(), px(0.))
        };
        // A passive view (a picker's preview) has no one to open the diff:
        // its bars take no clicks and show no hand.
        let hunk_hitboxes = if self.config.diff_gutter_click && !self.passive {
            hunk_bars
                .iter()
                .filter_map(|bar| {
                    let visible = bar.bounds.intersect(&bounds);
                    (!visible.is_empty()).then(|| {
                        (
                            window.insert_hitbox(visible, HitboxBehavior::Normal),
                            bar.row,
                        )
                    })
                })
                .collect()
        } else {
            Vec::new()
        };
        let frame = Arc::new(Frame {
            bounds,
            text_bounds,
            text_x,
            line_height,
            char_width,
            wrap_chars,
            rows: frame_rows,
            numbers,
            chevrons,
            selections,
            marked: self.visible_marked(),
            carets,
            caret_style: match self.cursor_overlay {
                Some(CursorOverlay {
                    shape: CursorShape::Bar,
                    ..
                }) => CaretStyle::Line(2),
                _ => self.config.cursor_style,
            },
            cursor_overlay,
            matches,
            hover_range: self.hover.highlight_range(),
            cursor_row,
            highlighted_lines,
            brackets,
            whitespace,
            hitbox: Some(hitbox),
            scroll_metrics,
            diagnostics,
            hunk_bars,
            hunk_hitboxes,
            icon_shift,
            colors: FrameColors {
                background: hsla(ui.editor_background),
                gutter: hsla(ui.editor_background),
                active_line: hsla(ui.editor_active_line_background),
                highlighted_line: hsla(ui.editor_highlighted_line_background),
                selection: hsla(ui.selection),
                bracket: hsla(ui.editor_foreground).opacity(BRACKET_BOX_ALPHA),
                search_match: hsla(ui.search_match_background),
                active_match: hsla(ui.editor_foreground).opacity(ACTIVE_MATCH_ALPHA),
                hover: hsla(ui.element_hover),
                cursor: hsla(ui.editor_foreground).opacity(CURSOR_ALPHA),
                caret: hsla(ui.cursor),
                fade: hsla(ui.editor_background).opacity(FADE_ALPHA),
                strike: hsla(ui.editor_foreground),
            },
        });
        self.frame = Some(frame.clone());
        self.report_scroll(cx);
        frame
    }

    /// The diagnostics on `rows`: marks, gutter icons (when the column
    /// shows), inline messages (when enabled; shaped only on screen) and
    /// the scrollbar marks of the whole file.
    fn layout_diagnostics(
        &self,
        rows: &[FrameRow],
        g: DiagnosticGeometry,
        scrollbar: Arc<[(Pixels, Hsla)]>,
        ui: &UiColors,
        window: &mut Window,
    ) -> FrameDiagnostics {
        let diags = &self.diagnostic_rows;
        let (Some(first), Some(last)) = (rows.first(), rows.last()) else {
            return FrameDiagnostics {
                scrollbar,
                ..FrameDiagnostics::default()
            };
        };
        let marks = diags
            .marks_in_paint_order(first.buffer_row..last.buffer_row + 1)
            .into_iter()
            .map(|m| (m.range.clone(), m.style, severity_color(ui, m.severity)))
            .collect();
        let icons = if g.icon_column {
            rows.iter()
                .filter(|r| r.piece == 0)
                .filter_map(|r| {
                    let severity = diags.gutter(r.buffer_row)?;
                    Some((r.top, severity_icon(severity), severity_color(ui, severity)))
                })
                .collect()
        } else {
            Vec::new()
        };
        let mut inline = Vec::new();
        if self.config.inline_diagnostics {
            let font_size = (g.font_size - px(1.)).max(px(1.));
            for r in rows.iter().filter(|r| r.is_last_piece()) {
                let Some((severity, message)) = diags.inline(r.buffer_row) else {
                    continue;
                };
                let left = g.text_x + r.piece().line.width() + g.font_size * INLINE_GAP_EMS;
                if left >= g.text_bounds.right() || message.is_empty() {
                    continue;
                }
                let color = severity_color(ui, severity);
                let text = shape(
                    window,
                    message.to_owned(),
                    font_size,
                    self.base_font(),
                    color,
                );
                let pill = Bounds::new(
                    point(left, r.top + px(2.)),
                    size(
                        text.width() + px(2. * INLINE_PADDING_X),
                        (g.line_height - px(4.)).max(px(0.)),
                    ),
                );
                let origin = point(left + px(INLINE_PADDING_X), r.top);
                inline.push(InlineDiagnostic {
                    buffer_row: r.buffer_row,
                    pill,
                    origin,
                    text,
                    color,
                });
            }
        }
        FrameDiagnostics {
            marks,
            underline_y: g.underline_y,
            icons,
            inline,
            scrollbar,
        }
    }

    /// The scrollbar marks of the whole file for a track `height` tall:
    /// laid out again only when the diagnostics, folds, height or theme
    /// change.
    fn scrollbar_marks(
        &mut self,
        height: Pixels,
        ui: &UiColors,
        cx: &gpui::Context<Self>,
    ) -> Arc<[(Pixels, Hsla)]> {
        let theme_generation = cx.global::<AppState>().theme_generation;
        if let Some(cached) = &self.scrollbar_marks
            && cached.rows_generation == self.diagnostic_rows_generation
            && cached.folds == self.folds
            && cached.height == height
            && cached.theme_generation == theme_generation
        {
            return cached.marks.clone();
        }
        // At each row's share of the rows shown (folded ones left out).
        let line_count = self.snapshot.line_count();
        let shown = (line_count - self.folds.hidden_before(line_count)).max(1);
        let track = f32::from(height) - SCROLLBAR_MARK_HEIGHT;
        let mut marks: Vec<(Pixels, Hsla, Severity)> = Vec::new();
        for &(row, severity) in self.diagnostic_rows.scrollbar() {
            let index = row - self.folds.hidden_before(row.min(line_count));
            let top = px((index as f32 / shown as f32 * track).round());
            match marks.last_mut() {
                // One mark per pixel row, the most severe.
                Some(prev) if prev.0 == top => {
                    if severity < prev.2 {
                        *prev = (top, severity_color(ui, severity), severity);
                    }
                }
                _ => marks.push((top, severity_color(ui, severity), severity)),
            }
        }
        let marks: Arc<[(Pixels, Hsla)]> = marks.into_iter().map(|(y, c, _)| (y, c)).collect();
        self.scrollbar_marks = Some(ScrollbarMarks {
            rows_generation: self.diagnostic_rows_generation,
            folds: self.folds.clone(),
            height,
            theme_generation,
            marks: marks.clone(),
        });
        marks
    }

    /// The marked ranges of the composition, as points (painting skips
    /// those off screen).
    fn visible_marked(&self) -> Vec<std::ops::Range<Point>> {
        self.marked
            .iter()
            .flatten()
            .map(|r| self.snapshot.offset_to_point(r.start)..self.snapshot.offset_to_point(r.end))
            .collect()
    }

    /// The non-empty selections and the caret cells (with `carets`) on
    /// buffer `rows`.
    fn visible_selections(
        &self,
        rows: std::ops::Range<u32>,
        carets: bool,
    ) -> (Vec<std::ops::Range<Point>>, Vec<std::ops::Range<Point>>) {
        let start = row_start(&self.snapshot, rows.start);
        let end = row_start(&self.snapshot, rows.end);
        let all = self.selections.all();
        // Sorted and non-overlapping: their ends are sorted too.
        let first = all.partition_point(|s| s.range().end < start);
        let mut ranges = Vec::new();
        let mut cells = Vec::new();
        for s in all[first..].iter().take_while(|s| s.range().start <= end) {
            let range = s.range();
            if !s.is_empty() {
                ranges.push(
                    self.snapshot.offset_to_point(range.start)
                        ..self.snapshot.offset_to_point(range.end),
                );
            }
            if carets && (start..=end).contains(&s.head) {
                cells.extend(self.cell_at(self.snapshot.offset_to_point(s.head)));
            }
        }
        (ranges, cells)
    }

    /// The cell of the char at `at`: from its column, moved back to a char
    /// boundary within the line, to the end of the char there.
    fn cell_at(&self, at: Point) -> Option<std::ops::Range<Point>> {
        if at.row >= self.snapshot.line_count() {
            return None;
        }
        let start = selection::boundary_offset(&self.snapshot, at);
        let line_end = row_range(&self.snapshot, at.row).end;
        let len = self
            .snapshot
            .chunks_in_range(start..line_end)
            .find_map(|chunk| chunk.chars().next())
            .filter(|&c| c != '\n' && c != '\r')
            .map_or(0, char::len_utf8);
        let column = (start - row_start(&self.snapshot, at.row)) as u32;
        Some(
            Point {
                row: at.row,
                column,
            }..Point {
                row: at.row,
                column: column + len as u32,
            },
        )
    }

    /// The search matches overlapping buffer `rows`, as points, with
    /// whether each is the active one.
    fn visible_matches(&self, rows: std::ops::Range<u32>) -> Vec<(std::ops::Range<Point>, bool)> {
        let start = row_start(&self.snapshot, rows.start);
        let end = row_start(&self.snapshot, rows.end);
        let len = self.snapshot.len_bytes();
        let first = self.match_highlights.partition_point(|r| r.end <= start);
        self.match_highlights[first..]
            .iter()
            .take_while(|r| r.start < end)
            .take(MAX_PAINTED_MATCHES)
            .enumerate()
            .map(|(i, r)| {
                let range = self.snapshot.offset_to_point(r.start.min(len))
                    ..self.snapshot.offset_to_point(r.end.min(len));
                (range, self.active_match == Some(first + i))
            })
            .collect()
    }

    /// The shaped pieces of `row`, from the cache or shaped now; the
    /// buffer columns `link` are drawn as a link.
    fn shaped_row(
        &mut self,
        key: ShapeKey,
        row: u32,
        link: Option<std::ops::Range<u32>>,
        theme: &Theme,
        window: &mut Window,
    ) -> Arc<ShapedRow> {
        if let Some(shaped) = self.caches.shaped(key, row, &link) {
            return shaped;
        }
        let lines = display_lines_with_tab_size(
            &self.snapshot,
            &FoldState::default(),
            key.wrap_chars,
            key.tab_size,
            row..row + 1,
        );
        let line_start = row_start(&self.snapshot, row);
        let spans = self.caches.spans(row).unwrap_or(&[]);
        let base = self.base_font();
        let default_color = hsla(theme.ui.editor_foreground);
        let link_color = hsla(theme.ui.link_text_hover);
        let piece_count = lines.len();
        let mut buf_start = 0u32;
        let mut pieces = Vec::with_capacity(piece_count);
        for (i, line) in lines.into_iter().enumerate() {
            let cut = line.truncated && i + 1 == piece_count;
            let buf_text = if cut {
                line.text.strip_suffix('…').unwrap_or(&line.text)
            } else {
                line.text.as_str()
            };
            let abs = line_start + buf_start as usize;
            let styles = span_styles(spans, abs..abs + buf_text.len());
            let mut text = String::with_capacity(line.text.len());
            // Runs: (length, (highlight span, whether in the link)).
            let mut keys: Vec<(usize, (Option<usize>, bool))> = Vec::new();
            let mut push = |text: &mut String, s: &str, style: (Option<usize>, bool)| {
                match keys.last_mut() {
                    Some((len, last)) if *last == style => *len += s.len(),
                    _ => keys.push((s.len(), style)),
                }
                text.push_str(s);
            };
            let mut map = ColumnMap {
                buf_start,
                buf_len: buf_text.len() as u32,
                tabs: Vec::new(),
            };
            // Display columns from the piece start, as wrapping counts them.
            let mut column = 0u32;
            for (b, c) in buf_text.char_indices() {
                let linked = link
                    .as_ref()
                    .is_some_and(|l| l.contains(&(buf_start + b as u32)));
                let style = (styles[b], linked);
                if c == '\t' {
                    let width = char_columns(c, column, key.tab_size);
                    map.tabs.push((b as u32, text.len() as u32, width));
                    push(&mut text, &" ".repeat(width as usize), style);
                    column += width;
                } else {
                    let mut utf8 = [0; 4];
                    push(&mut text, c.encode_utf8(&mut utf8), style);
                    column += char_columns(c, column, key.tab_size);
                }
            }
            if cut {
                push(&mut text, "…", (None, false));
            }
            let runs: Vec<TextRun> = keys
                .into_iter()
                .map(|(len, (style, linked))| {
                    let style = style.and_then(|s| theme.style_for(&spans[s].1));
                    let mut font = base.clone();
                    if style.is_some_and(|s| s.bold) {
                        font = font.bold();
                    }
                    if style.is_some_and(|s| s.italic) {
                        font = font.italic();
                    }
                    let color = style.and_then(|s| s.color).map_or(default_color, hsla);
                    TextRun {
                        len,
                        font,
                        color: if linked { link_color } else { color },
                        background_color: None,
                        // A straight 1 px line, as Zed draws links.
                        underline: linked.then_some(UnderlineStyle {
                            thickness: px(1.),
                            color: Some(link_color),
                            wavy: false,
                        }),
                        strikethrough: None,
                    }
                })
                .collect();
            let shaped =
                window
                    .text_system()
                    .shape_line(text.into(), px(key.font_size), &runs, None);
            buf_start += buf_text.len() as u32;
            pieces.push(Piece { line: shaped, map });
        }
        let shaped = Arc::new(ShapedRow { pieces });
        self.caches.insert_shaped(row, link, shaped.clone());
        shaped
    }

    /// A click: moves the cursor or selects. A ⌘-click on a char also
    /// remembers the go to definition to run at mouse-up.
    fn mouse_down(
        &mut self,
        ev: &MouseDownEvent,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if !self.passive {
            window.focus(&self.focus, cx);
        }
        self.hover.hide(cx);
        self.set_goto_message(None, cx);
        self.link_mouse_down(None);
        let Some(frame) = self.frame.clone() else {
            return;
        };
        if ev.position.x < frame.text_bounds.left() {
            let Some(row) = frame.row_at(ev.position.y) else {
                return;
            };
            let on_chevron = frame.chevrons.iter().any(|(_, r, ..)| *r == row.buffer_row)
                && ev.position.x >= frame.text_bounds.left() - frame.char_width * 2.;
            if on_chevron {
                self.toggle_fold(row.buffer_row, cx);
                return;
            }
            let line = selection::line_at(&self.snapshot, row.buffer_row);
            self.select(line, cx);
            self.drag = Some(Drag::Text);
            return;
        }
        let m = ev.modifiers;
        let secondary = m.secondary(cx);
        // `⇧⌥`-drag selects a column whatever `editor.multiCursorModifier`
        // says, as in VS Code.
        if m.shift && m.alt && !secondary {
            if let Some(from) = self.column_pos_at(&frame, ev.position) {
                self.set_column_selection(from, from, cx);
                self.drag = Some(Drag::Column { from });
            }
            return;
        }
        // `⌘⌥`-click is always go to definition to the side.
        let (add_cursor, definition) = if self.add_cursor_with_secondary(cx) {
            (secondary && !m.alt, m.alt)
        } else {
            (m.alt && !secondary, secondary)
        };
        let (add_cursor, definition) = (add_cursor && !m.shift, definition && !m.shift);
        if definition
            && ev.click_count == 1
            && let Some(p) = frame.char_at(ev.position)
        {
            // The cursor goes to the click first, so the answer is for
            // where the cursor is; a drag from here still selects.
            self.select(p..p, cx);
            self.link_mouse_down(Some((p, m.alt && secondary)));
            self.drag = Some(Drag::Text);
            return;
        }
        let Some(p) = frame.point_at(ev.position) else {
            return;
        };
        if add_cursor {
            self.add_cursor_mouse_down(p, ev.click_count, cx);
            return;
        }
        let range = match ev.click_count {
            0 | 1 if m.shift => self.selection_anchor()..p,
            0 | 1 => p..p,
            2 => selection::word_at(&self.snapshot, p),
            _ => selection::line_at(&self.snapshot, p.row),
        };
        self.select(range, cx);
        self.drag = Some(Drag::Text);
    }

    /// A click with the add-cursor modifier at `p`: a single click toggles
    /// a cursor there, a double / triple-click adds the word / line. A
    /// drag from here moves the head of what was added.
    fn add_cursor_mouse_down(
        &mut self,
        p: Point,
        click_count: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        let offset = self.snapshot.point_to_offset(p);
        let added = match click_count {
            0 | 1 => self.mouse_toggle_cursor(offset, cx).map(|id| (id, offset)),
            _ => {
                let range = if click_count == 2 {
                    selection::word_at(&self.snapshot, p)
                } else {
                    selection::line_at(&self.snapshot, p.row)
                };
                let start = self.snapshot.point_to_offset(range.start);
                let end = self.snapshot.point_to_offset(range.end);
                self.mouse_add_selection(start, end, cx)
                    .map(|id| (id, start))
            }
        };
        self.drag = added.map(|(id, anchor)| Drag::AddSelection { id, anchor });
    }

    /// The row and visual column under `position` for a column selection;
    /// past the line end it counts the columns the mouse is beyond it.
    fn column_pos_at(&self, frame: &Frame, position: gpui::Point<Pixels>) -> Option<ColumnPos> {
        let (p, past) = frame.point_past_end(position)?;
        let pos = self.column_pos(self.snapshot.point_to_offset(p));
        Some(ColumnPos {
            row: pos.row,
            col: pos.col + past,
        })
    }

    fn mouse_drag(&mut self, position: gpui::Point<Pixels>, cx: &mut gpui::Context<Self>) {
        let Some(frame) = self.frame.clone() else {
            return;
        };
        match self.drag {
            Some(Drag::Text) => {
                if let Some(p) = frame.point_at(position)
                    && p != self.selection_head()
                {
                    self.select(self.selection_anchor()..p, cx);
                }
            }
            Some(Drag::AddSelection { id, anchor }) => {
                if let Some(p) = frame.point_at(position) {
                    let head = self.snapshot.point_to_offset(p);
                    self.mouse_update_selection(id, anchor, head, cx);
                }
            }
            Some(Drag::Column { from }) => {
                if let Some(to) = self.column_pos_at(&frame, position)
                    && self.column_select != Some((from, to))
                {
                    self.set_column_selection(from, to, cx);
                }
            }
            Some(Drag::Minimap) | None => {}
        }
    }

    /// The fixed end of the primary selection.
    fn selection_anchor(&self) -> Point {
        self.snapshot
            .offset_to_point(self.selections.primary().anchor)
    }

    fn scroll_wheel(&mut self, ev: &ScrollEvent, cx: &mut gpui::Context<Self>) {
        let delta = ev.delta.pixel_delta(px(self.line_height()));
        self.hover.hide(cx);
        self.set_goto_message(None, cx);
        // Other text comes under the mouse: the next move says which.
        let held = self.link.held();
        self.link_mouse_moved(None, held, cx);
        self.scroll_by(-f32::from(delta.y));
        if !self.word_wrap {
            self.scroll_x = (self.scroll_x - f32::from(delta.x)).max(0.);
        }
        self.report_scroll(cx);
        cx.notify();
    }
}

/// For each byte of `range`, the index in `spans` of the innermost span
/// covering it.
fn span_styles(
    spans: &[(std::ops::Range<usize>, Arc<str>)],
    range: std::ops::Range<usize>,
) -> Vec<Option<usize>> {
    let mut styles = vec![None; range.len()];
    for (i, (span, _)) in spans.iter().enumerate() {
        let start = span.start.max(range.start);
        let end = span.end.min(range.end);
        for style in styles
            .iter_mut()
            .take(end.saturating_sub(range.start))
            .skip(start.saturating_sub(range.start))
        {
            *style = Some(i);
        }
    }
    styles
}

fn shape(
    window: &mut Window,
    text: impl Into<SharedString>,
    font_size: Pixels,
    font: Font,
    color: Hsla,
) -> ShapedLine {
    let text = text.into();
    let run = TextRun {
        len: text.len(),
        font,
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_line(text, font_size, &[run], None)
}

/// Mouse and scroll listeners for this frame: clicks and wheel only over
/// the element, drags wherever the mouse goes until the button is up.
fn register_mouse(view: &gpui::Entity<CodeView>, frame: &Arc<Frame>, window: &mut Window) {
    let Some(hitbox) = frame.hitbox.clone() else {
        return;
    };
    window.on_mouse_event({
        let (view, hitbox) = (view.clone(), hitbox.clone());
        let bars: Vec<(Bounds<Pixels>, u32)> = frame
            .hunk_hitboxes
            .iter()
            .map(|(hitbox, row)| (hitbox.bounds, *row))
            .collect();
        move |ev: &MouseDownEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble
                && ev.button == MouseButton::Left
                && hitbox.is_hovered(window)
            {
                // A click on a change bar opens the file's diff there.
                if let Some(&(_, row)) = bars.iter().find(|(b, _)| b.contains(&ev.position)) {
                    view.update(cx, |_, cx| {
                        cx.emit(super::CodeViewEvent::DiffHunkClicked { row })
                    });
                    cx.stop_propagation();
                    return;
                }
                view.update(cx, |v, cx| v.mouse_down(ev, window, cx));
                cx.stop_propagation();
            }
        }
    });
    window.on_mouse_event({
        let view = view.clone();
        move |ev: &MouseMoveEvent, phase, _, cx| {
            if phase != DispatchPhase::Bubble || ev.pressed_button != Some(MouseButton::Left) {
                return;
            }
            if matches!(
                view.read(cx).drag,
                Some(Drag::Text | Drag::AddSelection { .. } | Drag::Column { .. })
            ) {
                view.update(cx, |v, cx| v.mouse_drag(ev.position, cx));
            }
        }
    });
    window.on_mouse_event({
        let (view, hitbox) = (view.clone(), hitbox.clone());
        let gutter_right = frame.text_bounds.left();
        move |ev: &MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            let over_gutter = hitbox.is_hovered(window) && ev.position.x < gutter_right;
            if view.read(cx).gutter_hovered() != over_gutter {
                view.update(cx, |v, cx| v.set_gutter_hovered(over_gutter, cx));
            }
        }
    });
    // The LSP hover and the ⌘-hover link follow the char under the
    // mouse; the button held (a selection drag) counts as no char. While
    // ⌘ is held the link shows instead of the hover.
    window.on_mouse_event({
        let (view, hitbox, frame) = (view.clone(), hitbox.clone(), frame.clone());
        move |ev: &MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            let point = (ev.pressed_button.is_none() && hitbox.is_hovered(window))
                .then(|| frame.char_at(ev.position))
                .flatten();
            let held = view.read(cx).definition_modifier_held(&ev.modifiers, cx);
            view.update(cx, |v, cx| {
                v.link_mouse_moved(point, held, cx);
                let hover_point = point.filter(|_| !held);
                v.hover.mouse_moved(hover_point, ev.position, cx);
            });
        }
    });
    // The mouse can leave the window straight from the gutter, with no
    // move outside it to clear the hover.
    window.on_mouse_event({
        let view = view.clone();
        move |ev: &MouseExitEvent, phase, _, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            if view.read(cx).gutter_hovered() {
                view.update(cx, |v, cx| v.set_gutter_hovered(false, cx));
            }
            let held = view.read(cx).definition_modifier_held(&ev.modifiers, cx);
            view.update(cx, |v, cx| {
                v.link_mouse_moved(None, held, cx);
                v.hover.mouse_moved(None, ev.position, cx);
            });
        }
    });
    window.on_mouse_event({
        let view = view.clone();
        move |ev: &MouseUpEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble && ev.button == MouseButton::Left {
                let click = view.update(cx, |v, cx| {
                    v.drag = None;
                    v.link_mouse_up(cx)
                });
                if let Some(click) = click {
                    click.run(&view, window, cx);
                }
            }
        }
    });
    window.on_mouse_event({
        let view = view.clone();
        move |ev: &ScrollEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble && hitbox.should_handle_scroll(window) {
                view.update(cx, |v, cx| v.scroll_wheel(ev, cx));
                cx.stop_propagation();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::ColumnMap;

    /// `a\tbc` shown with tab size 4 from column 0: `a` then 3 spaces.
    fn map() -> ColumnMap {
        ColumnMap {
            buf_start: 0,
            buf_len: 4,
            tabs: vec![(1, 1, 3)],
        }
    }

    #[test]
    fn tabs_widen_display_columns() {
        let m = map();
        let display: Vec<_> = (0..=4).map(|c| m.to_display(c)).collect();
        assert_eq!(display, vec![0, 1, 4, 5, 6]);
        // Past the piece clamps to its end.
        assert_eq!(m.to_display(9), 6);
    }

    #[test]
    fn display_columns_map_back_snapping_inside_tabs() {
        let m = map();
        let buffer: Vec<_> = (0..=7).map(|d| m.to_buffer(d)).collect();
        assert_eq!(buffer, vec![0, 1, 1, 2, 2, 3, 4, 4]);
        assert!(m.is_tab_fill(2) && !m.is_tab_fill(4));
    }

    #[test]
    fn wrapped_pieces_offset_buffer_columns() {
        let m = ColumnMap {
            buf_start: 10,
            buf_len: 5,
            tabs: Vec::new(),
        };
        assert!(m.contains(10) && m.contains(15) && !m.contains(9));
        assert_eq!(m.to_display(12), 2);
        assert_eq!(m.to_buffer(3), 13);
    }
}
