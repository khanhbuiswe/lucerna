//! Draws a terminal's visible grid. Prepaint measures a cell, resizes the
//! terminal to the bounds, takes one snapshot and lays out each row as
//! runs (`lucerna_terminal::layout_line`); paint draws them in spec §5.2's
//! order.
//!
//! A run of plain narrow characters is not shaped: each character's glyph
//! is looked up once (per font style) and drawn at its cell, which is what
//! shaping onto a fixed grid with ligatures off gives anyway. Only runs
//! with wide characters, combining marks or characters no font maps to a
//! single glyph are shaped, each at a forced cell width.

use std::collections::HashMap;
use std::rc::Rc;

use gpui::{
    App, Bounds, ContentMask, CursorStyle, DispatchPhase, Element, ElementId, ElementInputHandler,
    Entity, Font, FontFeatures, FontId, FontStyle, FontWeight, GlobalElementId, GlyphId, Hitbox,
    HitboxBehavior, Hsla, InspectorElementId, IntoElement, LayoutId, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Pixels, Point, Rgba, ScrollEvent, ShapedLine, SharedString,
    StrikethroughStyle, Style, TextAlign, TextRun, UnderlineStyle, Window, fill, outline, point,
    px, relative, size,
};
use lucerna_terminal::{
    Cell, CellFlags, CursorShape, LineLayout, Modes, Rgba8, Snapshot, TerminalPalette, Underline,
    ViewStamp, layout_line,
};
use unicode_width::UnicodeWidthChar;

use super::settings::{
    InactiveCursor, TerminalFont, bold_is_bright, inactive_cursor, minimum_contrast, palette,
    terminal_font,
};
use super::view::TerminalView;
use crate::AppState;

/// Width of the bar cursor and height of the underline cursor.
const CURSOR_THICKNESS: f32 = 2.;

pub(crate) struct TerminalElement {
    view: Entity<TerminalView>,
}

impl TerminalElement {
    pub fn new(view: Entity<TerminalView>) -> Self {
        TerminalElement { view }
    }
}

impl IntoElement for TerminalElement {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

/// One piece of a row, drawn at `origin`.
struct Piece {
    origin: Point<Pixels>,
    kind: PieceKind,
    /// First column and the text, for tests.
    col: u16,
    text: SharedString,
}

enum PieceKind {
    /// Text shaped at a forced cell width.
    Shaped(Box<ShapedLine>),
    /// A run of plain narrow characters: each glyph at its cell's offset
    /// from `origin`, in one colour, with the run's lines.
    Glyphs {
        glyphs: Vec<(Pixels, CachedGlyph)>,
        color: Hsla,
        width: Pixels,
        underline: Option<UnderlineStyle>,
        strike: Option<StrikethroughStyle>,
    },
}

/// A character's glyph in one style: the font that has it (the terminal
/// font or a fallback) and its id.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CachedGlyph {
    font_id: FontId,
    id: GlyphId,
}

/// Glyphs looked up so far for the terminal font; `None` for a character
/// that does not map to one plain glyph, which is shaped instead.
#[derive(Default)]
pub(crate) struct GlyphCache {
    /// The regular font and the bold weight the glyphs were looked up in.
    font: Option<(Font, FontWeight)>,
    map: HashMap<(char, bool, bool), Option<CachedGlyph>>,
}

/// Distinct characters kept before the cache starts over.
const GLYPH_CACHE_LIMIT: usize = 8192;

impl GlyphCache {
    /// Forgets every glyph when the terminal font or its bold weight
    /// changed.
    fn for_font(&mut self, font: &Font, bold: FontWeight) {
        let key = (font.clone(), bold);
        if self.font.as_ref() != Some(&key) || self.map.len() > GLYPH_CACHE_LIMIT {
            self.font = Some(key);
            self.map.clear();
        }
    }
}

/// Where a row's text sits in its cell row.
#[derive(Clone, Copy)]
struct TextMetrics {
    font_size: Pixels,
    baseline: Pixels,
    underline: Pixels,
    strike: Pixels,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CursorKind {
    Block,
    Bar,
    Underline,
    /// The default shape while the view is not focused
    /// (`cursorStyleInactive`).
    Outline,
}

struct CursorPaint {
    kind: CursorKind,
    bounds: Bounds<Pixels>,
    color: Hsla,
    /// The character under a block cursor, in the background colour.
    glyph: Option<(Point<Pixels>, ShapedLine)>,
}

/// Where the grid's cells sit in the window. Painting and mouse hit
/// testing both use it, so a click lands on the cell drawn under it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CellGeometry {
    /// Top-left of cell (0, 0): one cell in from the left edge, and below
    /// the leftover height while following the output.
    pub origin: Point<Pixels>,
    pub cell_w: Pixels,
    pub cell_h: Pixels,
    pub rows: u16,
    pub cols: u16,
}

impl CellGeometry {
    /// `cols` cells of screen row `row` from column `col`.
    pub fn cells(&self, row: u16, col: u16, cols: u16) -> Bounds<Pixels> {
        Bounds::new(
            point(
                self.origin.x + self.cell_w * col as f32,
                self.origin.y + self.cell_h * row as f32,
            ),
            size(self.cell_w * cols as f32, self.cell_h),
        )
    }

    /// The grid's cells together.
    pub fn grid_bounds(&self) -> Bounds<Pixels> {
        Bounds::new(
            self.origin,
            size(
                self.cell_w * self.cols as f32,
                self.cell_h * self.rows as f32,
            ),
        )
    }

    /// The screen cell under `pos`, clamped to the grid, and whether `pos`
    /// is on its right half (where a selection ends after the cell).
    pub fn cell_at(&self, pos: Point<Pixels>) -> (u16, u16, bool) {
        let x = f32::from(pos.x - self.origin.x) / f32::from(self.cell_w);
        let y = f32::from(pos.y - self.origin.y) / f32::from(self.cell_h);
        let last_col = self.cols.saturating_sub(1) as f32;
        let last_row = self.rows.saturating_sub(1) as f32;
        let col = x.floor().clamp(0., last_col);
        let row = y.floor().clamp(0., last_row);
        let right = if x < 0. {
            false
        } else if x >= last_col + 1. {
            true
        } else {
            x - col >= 0.5
        };
        (row as u16, col as u16, right)
    }
}

/// What prepaint laid out and paint draws.
pub(crate) struct Frame {
    pub(super) bounds: Bounds<Pixels>,
    pub(super) geometry: CellGeometry,
    /// The painted screen's place in the buffer, its modes and whether it
    /// showed a selection: the mouse reads these instead of locking the
    /// terminal, and names cells of this screen.
    pub(super) stamp: ViewStamp,
    pub(super) mode: Modes,
    pub(super) has_selection: bool,
    /// The painted cells and which rows wrap, so link hover scans what is
    /// on screen without locking the terminal.
    pub(super) cells: Vec<Vec<Cell>>,
    pub(super) wrapped: Vec<bool>,
    hitbox: Hitbox,
    line_height: Pixels,
    pub(super) background: Rgba,
    backgrounds: Vec<(Bounds<Pixels>, Hsla)>,
    selection: Vec<Bounds<Pixels>>,
    selection_color: Hsla,
    /// Text pieces by screen row.
    rows: Vec<Vec<Piece>>,
    blocks: Vec<(Bounds<Pixels>, Hsla)>,
    cursor: Option<CursorPaint>,
    text: TextMetrics,
    /// The cursor's cell, where the IME's candidate window opens.
    pub(super) cursor_cell: Option<Bounds<Pixels>>,
    /// An IME composition, drawn over the cursor cell and underlined.
    marked: Option<MarkedPaint>,
}

/// A composition in progress: its background and its shaped text.
struct MarkedPaint {
    background: Bounds<Pixels>,
    origin: Point<Pixels>,
    line: ShapedLine,
}

impl Frame {
    /// The text of screen row `row`, each piece at its column.
    pub(super) fn row_text(&self, row: usize) -> String {
        let mut out = String::new();
        let mut col = 0usize;
        for piece in self.rows.get(row).into_iter().flatten() {
            while col < piece.col as usize {
                out.push(' ');
                col += 1;
            }
            out.push_str(&piece.text);
            col += piece
                .text
                .chars()
                .map(|c| c.width().unwrap_or(0))
                .sum::<usize>();
        }
        out
    }

    pub(super) fn text_baseline(&self) -> Pixels {
        self.text.baseline
    }

    pub(super) fn selection_rect_count(&self) -> usize {
        self.selection.len()
    }

    pub(super) fn cursor_selector(&self) -> Option<&'static str> {
        Some(match self.cursor.as_ref()?.kind {
            CursorKind::Block => "terminal-cursor-block",
            CursorKind::Bar => "terminal-cursor-bar",
            CursorKind::Underline => "terminal-cursor-underline",
            CursorKind::Outline => "terminal-cursor-outline",
        })
    }
}

fn rgba(c: Rgba8) -> Rgba {
    Rgba {
        r: c.r as f32 / 255.,
        g: c.g as f32 / 255.,
        b: c.b as f32 / 255.,
        a: c.a as f32 / 255.,
    }
}

fn hsla(c: Rgba8) -> Hsla {
    rgba(c).into()
}

fn base_font(font: &TerminalFont) -> Font {
    Font {
        family: font.family.clone(),
        features: FontFeatures::disable_ligatures(),
        fallbacks: None,
        weight: font.weight,
        style: FontStyle::Normal,
    }
}

/// Rounds `v` to the device pixel grid.
fn snap(v: Pixels, scale: f32) -> Pixels {
    px((f32::from(v) * scale).round() / scale)
}

/// Rows and columns that fit `bounds`, one cell kept as the left margin;
/// `None` when not even one cell fits (a hidden or collapsed panel), so the
/// terminal keeps its size.
fn grid_for(bounds: Bounds<Pixels>, cell_w: Pixels, cell_h: Pixels) -> Option<(u16, u16)> {
    let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
    let (cw, ch) = (f32::from(cell_w), f32::from(cell_h));
    if w < cw * 2. || h < ch || cw <= 0. || ch <= 0. {
        return None;
    }
    let rows = (h / ch).floor().clamp(1., u16::MAX as f32) as u16;
    let cols = ((w - cw) / cw).floor().clamp(1., u16::MAX as f32) as u16;
    Some((rows, cols))
}

/// Everything shaping a row needs.
struct Shaper<'a> {
    font: &'a TerminalFont,
    base: Font,
    cell_w: Pixels,
}

impl Shaper<'_> {
    fn run_font(&self, bold: bool, italic: bool) -> Font {
        let mut font = self.base.clone();
        if bold {
            font.weight = self.font.bold_weight;
        }
        if italic {
            font.style = FontStyle::Italic;
        }
        font
    }

    /// GPUI's run for `len` bytes drawn in `style`.
    fn text_run(&self, style: &lucerna_terminal::RunStyle, len: usize) -> TextRun {
        let color = hsla(style.fg);
        let underline = match style.underline {
            Underline::None => None,
            Underline::Curl => Some(UnderlineStyle {
                thickness: px(1.),
                color: Some(color),
                wavy: true,
            }),
            Underline::Single | Underline::Double => Some(UnderlineStyle {
                thickness: px(1.),
                color: Some(color),
                wavy: false,
            }),
        };
        TextRun {
            len,
            font: self.run_font(style.bold, style.italic),
            color,
            background_color: None,
            underline,
            strikethrough: style.strike.then_some(StrikethroughStyle {
                thickness: px(1.),
                color: Some(color),
            }),
        }
    }

    fn shape(
        &self,
        text: &str,
        run: &lucerna_terminal::TextRun,
        forced: bool,
        window: &mut Window,
    ) -> ShapedLine {
        let runs = [self.text_run(&run.style, text.len())];
        let text = SharedString::from(text.to_string());
        window
            .text_system()
            .shape_line(text, self.font.size, &runs, forced.then_some(self.cell_w))
    }

    /// The glyph `c` draws in this style, looked up once.
    fn glyph(
        &self,
        cache: &mut GlyphCache,
        c: char,
        bold: bool,
        italic: bool,
        window: &mut Window,
    ) -> Option<CachedGlyph> {
        *cache.map.entry((c, bold, italic)).or_insert_with(|| {
            let mut buf = [0u8; 4];
            let text = c.encode_utf8(&mut buf);
            let runs = [TextRun {
                len: text.len(),
                font: self.run_font(bold, italic),
                color: Hsla::default(),
                background_color: None,
                underline: None,
                strikethrough: None,
            }];
            let layout = window
                .text_system()
                .layout_line(text, self.font.size, &runs, None);
            match layout.runs.as_slice() {
                [run] if run.glyphs.len() == 1 && !run.glyphs[0].is_emoji => Some(CachedGlyph {
                    font_id: run.font_id,
                    id: run.glyphs[0].id,
                }),
                _ => None,
            }
        })
    }

    /// Lays out one run at `y`: as looked-up glyphs when every character
    /// is a plain narrow one, else shaped.
    fn run_pieces(
        &self,
        run: &lucerna_terminal::TextRun,
        origin_x: Pixels,
        y: Pixels,
        cache: &mut GlyphCache,
        window: &mut Window,
        out: &mut Vec<Piece>,
    ) {
        let (bold, italic) = (run.style.bold, run.style.italic);
        let mut glyphs = Vec::with_capacity(run.text.len());
        let mut plain = true;
        for (i, c) in run.text.chars().enumerate() {
            if c.width() != Some(1) {
                plain = false;
                break;
            }
            if c == ' ' {
                continue;
            }
            match self.glyph(cache, c, bold, italic, window) {
                Some(g) => glyphs.push((self.cell_w * i as f32, g)),
                None => {
                    plain = false;
                    break;
                }
            }
        }
        if !plain {
            self.pieces(run, origin_x, y, window, out);
            return;
        }
        let tr = self.text_run(&run.style, run.text.len());
        out.push(Piece {
            origin: point(origin_x + self.cell_w * run.col as f32, y),
            kind: PieceKind::Glyphs {
                glyphs,
                color: tr.color,
                width: self.cell_w * run.cells as f32,
                underline: tr.underline,
                strike: tr.strikethrough,
            },
            col: run.col,
            text: run.text.clone().into(),
        });
    }

    /// Shapes one run into pieces at `y`. Narrow text is forced onto the
    /// cell grid; each wide character (with its marks) is shaped on its
    /// own at its two cells, since forcing would squeeze it into one.
    fn pieces(
        &self,
        run: &lucerna_terminal::TextRun,
        origin_x: Pixels,
        y: Pixels,
        window: &mut Window,
        out: &mut Vec<Piece>,
    ) {
        let mut col = run.col;
        let mut start = 0usize;
        let mut start_col = col;
        let mut push = |text: &str, at: u16, forced: bool, out: &mut Vec<Piece>| {
            if text.is_empty() {
                return;
            }
            out.push(Piece {
                origin: point(origin_x + self.cell_w * at as f32, y),
                kind: PieceKind::Shaped(Box::new(self.shape(text, run, forced, window))),
                col: at,
                text: text.to_string().into(),
            });
        };
        let text = run.text.as_str();
        let mut chars = text.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            if c.width() != Some(2) {
                if c.width().unwrap_or(0) > 0 {
                    col += 1;
                }
                continue;
            }
            push(&text[start..i], start_col, true, out);
            let mut end = i + c.len_utf8();
            while let Some(&(j, m)) = chars.peek() {
                if m.width() != Some(0) {
                    break;
                }
                end = j + m.len_utf8();
                chars.next();
            }
            push(&text[i..end], col, false, out);
            col += 2;
            start = end;
            start_col = col;
        }
        push(&text[start..], start_col, true, out);
    }
}

impl TerminalView {
    /// Lays out one frame for `bounds`: resizes the terminal when the grid
    /// changed, then snapshots it and shapes the visible rows.
    fn layout(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) -> Rc<Frame> {
        let font = terminal_font(cx);
        let generation = cx.global::<AppState>().theme_generation;
        let palette: Rc<TerminalPalette> = match &self.palette {
            Some((g, p)) if *g == generation => p.clone(),
            _ => {
                let p = Rc::new(palette(cx));
                self.palette = Some((generation, p.clone()));
                p
            }
        };
        let (min_contrast, bold_bright) = (minimum_contrast(cx), bold_is_bright(cx));
        let scale = window.scale_factor();

        let base = base_font(&font);
        let text_system = window.text_system();
        let font_id = text_system.resolve_font(&base);
        let advance = text_system
            .advance(font_id, font.size, 'm')
            .map(|s| s.width)
            .ok()
            .filter(|w| *w > px(0.))
            .unwrap_or(font.size * 0.6);
        let cell_w = (advance + px(font.letter_spacing)).max(px(1.));
        let cell_h = snap(font.cell_height(), scale).max(px(1.));
        // Glyphs drawn from the cache sit where a shaped line in the
        // terminal font would put them.
        let ascent = text_system.ascent(font_id, font.size);
        // Font metrics give the descent below the baseline as a negative
        // number; a shaped line's layout, which this matches, uses it positive.
        let descent = text_system.descent(font_id, font.size).abs();
        let padding_top = (cell_h - ascent - descent) / 2.;
        let text_metrics = TextMetrics {
            font_size: font.size,
            baseline: padding_top + ascent,
            underline: gpui::underline_y_offset(cell_h, ascent, descent),
            strike: (ascent * 0.5 + padding_top + ascent) * 0.5,
        };

        // `resize` only passes a change on, so a new cell size with the
        // same grid still reaches the app.
        if let Some(grid) = grid_for(bounds, cell_w, cell_h) {
            self.grid = grid;
            let px_u16 = |v: Pixels| f32::from(v).round().clamp(1., u16::MAX as f32) as u16;
            self.terminal()
                .resize(grid.0, grid.1, px_u16(cell_w), px_u16(cell_h));
        }
        let snapshot = self.terminal().snapshot();

        // Leftover height goes above the grid while it follows the output,
        // so the prompt sits on the bottom edge; below it when scrolled up.
        let grid_h = cell_h * snapshot.rows as f32;
        let leftover = (bounds.size.height - grid_h).max(px(0.));
        let top = if snapshot.display_offset == 0 {
            bounds.top() + leftover
        } else {
            bounds.top()
        };
        let origin = point(snap(bounds.left() + cell_w, scale), snap(top, scale));
        let geometry = CellGeometry {
            origin,
            cell_w,
            cell_h,
            rows: snapshot.rows,
            cols: snapshot.cols,
        };
        let cell = |row: u16, col: u16, cols: u16| geometry.cells(row, col, cols);

        let mut glyphs = std::mem::take(&mut self.glyphs);
        glyphs.for_font(&base, font.bold_weight);
        let shaper = Shaper {
            font: &font,
            base,
            cell_w,
        };
        let mut backgrounds = Vec::new();
        let mut blocks = Vec::new();
        let mut rows = Vec::with_capacity(snapshot.cells.len());
        for (r, cells) in snapshot.cells.iter().enumerate() {
            let r = r as u16;
            let LineLayout {
                runs,
                bgs,
                blocks: block_rects,
            } = layout_line(cells, &palette, min_contrast, bold_bright);
            for bg in bgs {
                backgrounds.push((cell(r, bg.col, bg.cols), hsla(bg.color)));
            }
            for b in block_rects {
                let c = cell(r, b.col, 1);
                let bounds = Bounds::new(
                    point(c.left() + cell_w * b.x, c.top() + cell_h * b.y),
                    size(cell_w * b.w, cell_h * b.h),
                );
                blocks.push((bounds, hsla(b.color)));
            }
            let y = origin.y + cell_h * r as f32;
            let mut pieces = Vec::with_capacity(runs.len());
            for run in &runs {
                shaper.run_pieces(run, origin.x, y, &mut glyphs, window, &mut pieces);
            }
            rows.push(pieces);
        }
        self.glyphs = glyphs;

        let selection = selection_rects(&snapshot, &cell);
        let focused = self.focus.is_focused(window);
        let shape = if focused {
            None
        } else {
            Some(inactive_cursor(cx))
        };
        let cursor = cursor_paint(&snapshot, &palette, shape, &shaper, &cell, window);
        let cursor_cell = snapshot.cursor.map(|c| cell(c.row, c.col, 1));
        let marked = self
            .marked
            .as_deref()
            .zip(cursor_cell)
            .map(|(text, at)| marked_paint(text, at, &shaper, &palette, window));
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let stamp = snapshot.stamp();
        let frame = Rc::new(Frame {
            bounds,
            geometry,
            stamp,
            mode: snapshot.mode,
            has_selection: snapshot.selection.is_some(),
            cells: snapshot.cells,
            wrapped: snapshot.wrapped,
            hitbox,
            line_height: cell_h,
            background: rgba(palette.background),
            backgrounds,
            selection,
            selection_color: hsla(palette.selection),
            rows,
            blocks,
            cursor,
            text: text_metrics,
            cursor_cell,
            marked,
        });
        self.last_frame = Some(frame.clone());
        self.links_frame_changed(stamp);
        frame
    }
}

/// The selected cells on screen, one rectangle per row.
fn selection_rects(
    snapshot: &Snapshot,
    cell: &impl Fn(u16, u16, u16) -> Bounds<Pixels>,
) -> Vec<Bounds<Pixels>> {
    let Some(sel) = snapshot.selection else {
        return Vec::new();
    };
    let offset = snapshot.display_offset as i32;
    let last_col = snapshot.cols.saturating_sub(1);
    let mut out = Vec::new();
    for line in sel.start.line..=sel.end.line {
        let row = line + offset;
        if !(0..snapshot.rows as i32).contains(&row) {
            continue;
        }
        let (from, to) = if sel.is_block {
            (
                sel.start.col.min(sel.end.col),
                sel.start.col.max(sel.end.col),
            )
        } else {
            (
                if line == sel.start.line {
                    sel.start.col
                } else {
                    0
                },
                if line == sel.end.line {
                    sel.end.col
                } else {
                    last_col
                },
            )
        };
        let to = to.min(last_col);
        if from <= to {
            out.push(cell(row as u16, from, to - from + 1));
        }
    }
    out
}

/// The composition at the cursor cell: the terminal's colours, underlined,
/// on the background so it hides the cells it covers.
fn marked_paint(
    text: &str,
    at: Bounds<Pixels>,
    shaper: &Shaper<'_>,
    palette: &TerminalPalette,
    window: &mut Window,
) -> MarkedPaint {
    let run = lucerna_terminal::TextRun {
        col: 0,
        cells: 0,
        text: text.to_string(),
        style: lucerna_terminal::RunStyle {
            fg: palette.foreground,
            bold: false,
            italic: false,
            underline: Underline::Single,
            strike: false,
        },
    };
    let line = shaper.shape(text, &run, false, window);
    let width = line.width.max(at.size.width);
    MarkedPaint {
        background: Bounds::new(at.origin, size(width, at.size.height)),
        origin: at.origin,
        line,
    }
}

fn cursor_paint(
    snapshot: &Snapshot,
    palette: &TerminalPalette,
    inactive: Option<InactiveCursor>,
    shaper: &Shaper<'_>,
    cell: &impl Fn(u16, u16, u16) -> Bounds<Pixels>,
    window: &mut Window,
) -> Option<CursorPaint> {
    let cursor = snapshot.cursor?;
    let under = snapshot
        .cells
        .get(cursor.row as usize)
        .and_then(|row| row.get(cursor.col as usize));
    let wide = under.is_some_and(|c| c.flags.contains(CellFlags::WIDE));
    let full = cell(cursor.row, cursor.col, if wide { 2 } else { 1 });
    let thickness = px(CURSOR_THICKNESS);
    // `inactive` is the unfocused view's shape, whatever the app chose.
    let kind = match (inactive, cursor.shape) {
        (_, CursorShape::Hidden) | (Some(InactiveCursor::Hidden), _) => return None,
        (Some(InactiveCursor::Outline), _) => CursorKind::Outline,
        (Some(InactiveCursor::Block), _) | (None, CursorShape::Block) => CursorKind::Block,
        (Some(InactiveCursor::Bar), _) | (None, CursorShape::Bar) => CursorKind::Bar,
        (Some(InactiveCursor::Underline), _) | (None, CursorShape::Underline) => {
            CursorKind::Underline
        }
    };
    let bounds = match kind {
        CursorKind::Block | CursorKind::Outline => full,
        CursorKind::Bar => Bounds::new(full.origin, size(thickness, full.size.height)),
        CursorKind::Underline => Bounds::new(
            point(full.left(), full.bottom() - thickness),
            size(full.size.width, thickness),
        ),
    };
    let glyph = (kind == CursorKind::Block)
        .then(|| under.filter(|c| c.ch != ' ' && !c.flags.contains(CellFlags::HIDDEN)))
        .flatten()
        .map(|c| {
            let mut text = String::from(c.ch);
            if let Some(z) = &c.zerowidth {
                text.extend(z.iter());
            }
            let run = lucerna_terminal::TextRun {
                col: cursor.col,
                cells: if wide { 2 } else { 1 },
                text: text.clone(),
                style: lucerna_terminal::RunStyle {
                    fg: palette.background,
                    bold: c.flags.contains(CellFlags::BOLD),
                    italic: c.flags.contains(CellFlags::ITALIC),
                    underline: Underline::None,
                    strike: false,
                },
            };
            (full.origin, shaper.shape(&text, &run, !wide, window))
        });
    Some(CursorPaint {
        kind,
        bounds,
        color: hsla(palette.cursor),
        glyph,
    })
}

impl Element for TerminalElement {
    type RequestLayoutState = ();
    type PrepaintState = Rc<Frame>;

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
    ) -> Rc<Frame> {
        self.view
            .update(cx, |view, cx| view.layout(bounds, window, cx))
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        frame: &mut Rc<Frame>,
        window: &mut Window,
        cx: &mut App,
    ) {
        // Typed text and IME compositions reach the view through this.
        let focus = self.view.read(cx).focus.clone();
        let focused = focus.is_focused(window);
        if focused {
            window.handle_input(
                &focus,
                ElementInputHandler::new(frame.bounds, self.view.clone()),
                cx,
            );
        }
        let link_shown = self.view.update(cx, |view, _| {
            view.frames_drawn += 1;
            view.input_handler_installed = focused;
            view.link_shown()
        });
        if link_shown {
            window.set_cursor_style(CursorStyle::PointingHand, &frame.hitbox);
        }
        register_mouse(&self.view, frame, window);
        let frame = &**frame;
        let lh = frame.line_height;
        window.paint_quad(fill(frame.bounds, Hsla::from(frame.background)));
        window.with_content_mask(
            Some(ContentMask {
                bounds: frame.bounds,
            }),
            |window| {
                for (bounds, color) in &frame.backgrounds {
                    window.paint_quad(fill(*bounds, *color));
                }
                for bounds in &frame.selection {
                    window.paint_quad(fill(*bounds, frame.selection_color));
                }
                // One layer for all the text: thousands of glyphs each
                // ordered on their own would cost more than drawing them.
                window.paint_layer(frame.bounds, |window| {
                    for piece in frame.rows.iter().flatten() {
                        paint_piece(piece, &frame.text, lh, window, cx);
                    }
                });
                for (bounds, color) in &frame.blocks {
                    window.paint_quad(fill(*bounds, *color));
                }
                if let Some(cursor) = &frame.cursor {
                    match cursor.kind {
                        CursorKind::Outline => window.paint_quad(outline(
                            cursor.bounds,
                            cursor.color,
                            gpui::BorderStyle::Solid,
                        )),
                        _ => window.paint_quad(fill(cursor.bounds, cursor.color)),
                    }
                    if let Some((origin, line)) = &cursor.glyph {
                        let _ = line.paint(*origin, lh, TextAlign::Left, None, window, cx);
                    }
                }
                if let Some(marked) = &frame.marked {
                    window.paint_quad(fill(marked.background, Hsla::from(frame.background)));
                    let _ = marked
                        .line
                        .paint(marked.origin, lh, TextAlign::Left, None, window, cx);
                }
            },
        );
    }
}

fn paint_piece(
    piece: &Piece,
    text: &TextMetrics,
    line_height: Pixels,
    window: &mut Window,
    cx: &mut App,
) {
    match &piece.kind {
        PieceKind::Shaped(line) => {
            let _ = line.paint(piece.origin, line_height, TextAlign::Left, None, window, cx);
        }
        PieceKind::Glyphs {
            glyphs,
            color,
            width,
            underline,
            strike,
        } => {
            let baseline = piece.origin.y + text.baseline;
            for (x, glyph) in glyphs {
                let _ = window.paint_glyph(
                    point(piece.origin.x + *x, baseline),
                    glyph.font_id,
                    glyph.id,
                    text.font_size,
                    *color,
                );
            }
            if let Some(style) = underline {
                let at = point(piece.origin.x, piece.origin.y + text.underline);
                window.paint_underline(at, *width, style);
            }
            if let Some(style) = strike {
                let at = point(piece.origin.x, piece.origin.y + text.strike);
                window.paint_strikethrough(at, *width, style);
            }
        }
    }
}

/// Mouse listeners for this frame: presses and the wheel only over the
/// grid, drags and releases wherever the mouse goes once a press began
/// here.
fn register_mouse(view: &Entity<TerminalView>, frame: &Rc<Frame>, window: &mut Window) {
    let hitbox = frame.hitbox.clone();
    window.on_mouse_event({
        let (view, hitbox) = (view.clone(), hitbox.clone());
        move |ev: &MouseDownEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble && hitbox.is_hovered(window) {
                view.update(cx, |v, cx| v.mouse_down(ev, window, cx));
                cx.stop_propagation();
            }
        }
    });
    window.on_mouse_event({
        let (view, hitbox) = (view.clone(), hitbox.clone());
        move |ev: &MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            let pressed = view.read(cx).mouse_pressed();
            if pressed || hitbox.is_hovered(window) {
                view.update(cx, |v, cx| v.mouse_move(ev, cx));
            } else if view.read(cx).link_active() {
                view.update(cx, |v, cx| v.release_link(cx));
            }
        }
    });

    window.on_mouse_event({
        let view = view.clone();
        move |ev: &MouseUpEvent, phase, _, cx| {
            if phase == DispatchPhase::Bubble && view.read(cx).mouse_pressed() {
                view.update(cx, |v, cx| v.mouse_up(ev, cx));
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
    use super::*;

    fn geometry() -> CellGeometry {
        CellGeometry {
            origin: point(px(10.), px(4.)),
            cell_w: px(10.),
            cell_h: px(20.),
            rows: 3,
            cols: 5,
        }
    }

    #[test]
    fn cell_at_maps_pixels_to_cells_and_sides() {
        let g = geometry();
        assert_eq!(g.cell_at(point(px(12.), px(5.))), (0, 0, false));
        assert_eq!(g.cell_at(point(px(18.), px(5.))), (0, 0, true));
        assert_eq!(g.cell_at(point(px(31.), px(45.))), (2, 2, false));
        // Outside the grid: clamped, past the right edge ends after it.
        assert_eq!(g.cell_at(point(px(0.), px(-30.))), (0, 0, false));
        assert_eq!(g.cell_at(point(px(500.), px(500.))), (2, 4, true));
    }

    #[test]
    fn cells_and_cell_at_agree() {
        let g = geometry();
        let b = g.cells(1, 3, 1);
        assert_eq!(g.cell_at(b.origin + point(px(1.), px(1.))), (1, 3, false));
        assert_eq!(
            g.cell_at(b.center()),
            (1, 3, true),
            "half way is the right side"
        );
        assert_eq!(b.size, size(px(10.), px(20.)));
    }

    fn b(w: f32, h: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(0.), px(0.)), size(px(w), px(h)))
    }

    #[test]
    fn grid_keeps_a_left_margin_and_rounds_down() {
        assert_eq!(grid_for(b(100., 50.), px(10.), px(16.)), Some((3, 9)));
    }

    #[test]
    fn grid_needs_room_for_one_cell() {
        assert_eq!(grid_for(b(0., 0.), px(10.), px(16.)), None);
        assert_eq!(grid_for(b(15., 100.), px(10.), px(16.)), None);
        assert_eq!(grid_for(b(100., 10.), px(10.), px(16.)), None);
        assert_eq!(grid_for(b(20., 16.), px(10.), px(16.)), Some((1, 1)));
    }
}
