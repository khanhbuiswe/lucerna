//! Batching one screen row into text runs, background rectangles and block
//! glyph rectangles, ready for the renderer.

use crate::color::{Rgba8, TerminalPalette, ensure_contrast, resolve_color_origin};
use crate::snapshot::{Cell, CellFlags, NamedColor, TermColor};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Underline {
    None,
    Single,
    Double,
    Curl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunStyle {
    pub fg: Rgba8,
    pub bold: bool,
    pub italic: bool,
    pub underline: Underline,
    pub strike: bool,
}

/// Text drawn in one style over `cells` columns starting at `col`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextRun {
    pub col: u16,
    pub cells: u16,
    pub text: String,
    pub style: RunStyle,
}

/// A filled background over `cols` columns starting at `col`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BgRect {
    pub col: u16,
    pub cols: u16,
    pub color: Rgba8,
}

/// A block glyph drawn as a rectangle; `x, y, w, h` are fractions of the
/// cell at `col`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlockRect {
    pub col: u16,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub color: Rgba8,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct LineLayout {
    pub runs: Vec<TextRun>,
    pub bgs: Vec<BgRect>,
    pub blocks: Vec<BlockRect>,
}

const UL: u8 = 1;
const UR: u8 = 2;
const LL: u8 = 4;
const LR: u8 = 8;

/// Rectangles (x, y, w, h, alpha) for U+2580..=U+259F, in cell fractions.
type BlockPart = (f32, f32, f32, f32, u8);

fn block_rects(ch: char) -> Option<Vec<BlockPart>> {
    let cp = ch as u32;
    if !(0x2580..=0x259F).contains(&cp) {
        return None;
    }
    let full = |a: u8| vec![(0.0, 0.0, 1.0, 1.0, a)];
    let quad = |mask: u8| {
        let mut v = Vec::new();
        for (bit, x, y) in [
            (UL, 0.0, 0.0),
            (UR, 0.5, 0.0),
            (LL, 0.0, 0.5),
            (LR, 0.5, 0.5),
        ] {
            if mask & bit != 0 {
                v.push((x, y, 0.5, 0.5, 255));
            }
        }
        v
    };
    Some(match cp {
        0x2580 => vec![(0.0, 0.0, 1.0, 0.5, 255)],
        0x2581..=0x2587 => {
            let h = (cp - 0x2580) as f32 / 8.0;
            vec![(0.0, 1.0 - h, 1.0, h, 255)]
        }
        0x2588 => full(255),
        0x2589..=0x258F => {
            let w = (0x2590 - cp) as f32 / 8.0;
            vec![(0.0, 0.0, w, 1.0, 255)]
        }
        0x2590 => vec![(0.5, 0.0, 0.5, 1.0, 255)],
        0x2591 => full(64),
        0x2592 => full(128),
        0x2593 => full(192),
        0x2594 => vec![(0.0, 0.0, 1.0, 0.125, 255)],
        0x2595 => vec![(0.875, 0.0, 0.125, 1.0, 255)],
        0x2596 => quad(LL),
        0x2597 => quad(LR),
        0x2598 => quad(UL),
        0x2599 => quad(UL | LL | LR),
        0x259A => quad(UL | LR),
        0x259B => quad(UL | UR | LL),
        0x259C => quad(UL | UR | LR),
        0x259D => quad(UR),
        0x259E => quad(UR | LL),
        _ => quad(UR | LL | LR),
    })
}

/// Lays out one row of cells. `min_contrast` of 1 disables contrast
/// adjustment.
pub fn layout_line(
    row: &[Cell],
    palette: &TerminalPalette,
    min_contrast: f32,
    bold_bright: bool,
) -> LineLayout {
    let mut out = LineLayout::default();
    // The run being built, and whether it holds anything but spaces.
    let mut open = false;
    let mut col: u16 = 0;
    let mut after_wide = false;

    for cell in row {
        let flags = cell.flags;
        if flags.contains(CellFlags::WIDE_SPACER) {
            // The spacer after a wide cell is already counted in its width.
            if !after_wide {
                col = col.saturating_add(1);
            }
            after_wide = false;
            continue;
        }
        after_wide = flags.contains(CellFlags::WIDE);
        let width: u16 = if flags.contains(CellFlags::WIDE) {
            2
        } else {
            1
        };
        let this_col = col;
        col = col.saturating_add(width);

        let bold = flags.contains(CellFlags::BOLD);
        let dim = flags.contains(CellFlags::DIM);
        let inverse = flags.contains(CellFlags::INVERSE);
        let (fg_term, bg_term) = if inverse {
            (cell.bg, cell.fg)
        } else {
            (cell.fg, cell.bg)
        };

        let (bg, _) = resolve_color_origin(bg_term, palette, false, false, false, bold_bright);
        let (mut fg, fg_palette) =
            resolve_color_origin(fg_term, palette, true, bold, dim, bold_bright);

        // Background rectangle, merged with the previous one when adjacent.
        if bg_term != TermColor::Named(NamedColor::Background) {
            match out.bgs.last_mut() {
                Some(last) if last.color == bg && last.col + last.cols == this_col => {
                    last.cols += width;
                }
                _ => out.bgs.push(BgRect {
                    col: this_col,
                    cols: width,
                    color: bg,
                }),
            }
        }

        let hidden = flags.contains(CellFlags::HIDDEN);
        let ch = if hidden { ' ' } else { cell.ch };

        if let Some(rects) = block_rects(ch).filter(|_| !hidden) {
            for (x, y, w, h, a) in rects {
                let color = Rgba8 {
                    a: ((fg.a as u32 * a as u32) / 255) as u8,
                    ..fg
                };
                out.blocks.push(BlockRect {
                    col: this_col,
                    x,
                    y,
                    w,
                    h,
                    color,
                });
            }
            open = false;
            continue;
        }

        if fg_palette && min_contrast > 1.0 {
            fg = ensure_contrast(fg, bg, min_contrast);
        }

        let underline = if flags.contains(CellFlags::UNDERCURL) {
            Underline::Curl
        } else if flags.contains(CellFlags::DOUBLE_UNDERLINE) {
            Underline::Double
        } else if flags.contains(CellFlags::UNDERLINE) {
            Underline::Single
        } else {
            Underline::None
        };
        let strike = flags.contains(CellFlags::STRIKEOUT);
        let style = RunStyle {
            fg,
            bold,
            italic: flags.contains(CellFlags::ITALIC),
            underline,
            strike,
        };

        let has_zw = cell.zerowidth.as_ref().is_some_and(|z| !z.is_empty());
        let plain_space = ch == ' ' && !has_zw && underline == Underline::None && !strike;

        let continues = open
            && out.runs.last().is_some_and(|r| {
                r.col + r.cells == this_col
                    && (r.style == style
                        // A plain space only joins runs that draw no line.
                        || (plain_space
                            && r.style.underline == Underline::None
                            && !r.style.strike))
            });
        if continues {
            let run = out.runs.last_mut().expect("open run");
            run.text.push(ch);
            if let Some(z) = &cell.zerowidth {
                run.text.extend(z.iter());
            }
            run.cells += width;
        } else if plain_space {
            // Spaces draw nothing, so they never start a run.
            open = false;
        } else {
            let mut text = String::new();
            text.push(ch);
            if let Some(z) = &cell.zerowidth {
                text.extend(z.iter());
            }
            out.runs.push(TextRun {
                col: this_col,
                cells: width,
                text,
                style,
            });
            open = true;
        }
    }

    // Trailing spaces at the end of the line draw nothing.
    if let Some(last) = out.runs.last_mut()
        && last.style.underline == Underline::None
        && !last.style.strike
    {
        while last.text.ends_with(' ') {
            last.text.pop();
            last.cells -= 1;
        }
    }
    if out.runs.last().is_some_and(|r| r.text.is_empty()) {
        out.runs.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::test_palette;
    use crate::snapshot::NamedColor;

    fn cell(ch: char) -> Cell {
        Cell {
            ch,
            zerowidth: None,
            fg: TermColor::Named(NamedColor::Foreground),
            bg: TermColor::Named(NamedColor::Background),
            flags: CellFlags::empty(),
            hyperlink: None,
        }
    }

    fn row(s: &str) -> Vec<Cell> {
        s.chars().map(cell).collect()
    }

    fn layout(r: &[Cell]) -> LineLayout {
        layout_line(r, &test_palette(), 1.0, false)
    }

    #[test]
    fn same_style_cells_form_one_run() {
        let l = layout(&row("hello"));
        assert_eq!(l.runs.len(), 1);
        assert_eq!(l.runs[0].cells, 5);
        assert_eq!(l.runs[0].text, "hello");
    }

    #[test]
    fn style_change_splits_run() {
        let mut r = row("abcd");
        for c in &mut r[..2] {
            c.fg = TermColor::Named(NamedColor::Red);
        }
        for c in &mut r[2..] {
            c.fg = TermColor::Named(NamedColor::Green);
        }
        let l = layout(&r);
        assert_eq!(l.runs.len(), 2);
        assert_eq!((l.runs[1].col, l.runs[1].text.as_str()), (2, "cd"));
    }

    #[test]
    fn spaces_inside_styled_text_stay_in_run() {
        let l = layout(&row("a b"));
        assert_eq!(l.runs.len(), 1);
        assert_eq!(l.runs[0].text, "a b");
        assert_eq!(l.runs[0].cells, 3);
    }

    #[test]
    fn trailing_spaces_dropped() {
        let l = layout(&row("ab   "));
        assert_eq!(l.runs.len(), 1);
        assert_eq!(l.runs[0].cells, 2);
    }

    #[test]
    fn default_bg_spaces_make_no_rect() {
        let l = layout(&row("   "));
        assert!(l.bgs.is_empty());
        assert!(l.runs.is_empty());
    }

    #[test]
    fn adjacent_same_bg_merges() {
        let mut r = row("abc");
        for c in &mut r {
            c.bg = TermColor::Named(NamedColor::Blue);
        }
        let l = layout(&r);
        assert_eq!(l.bgs.len(), 1);
        assert_eq!((l.bgs[0].col, l.bgs[0].cols), (0, 3));
        assert_eq!(l.bgs[0].color, test_palette().ansi[4]);
    }

    #[test]
    fn inverse_swaps_fg_bg() {
        let mut r = row("a");
        r[0].fg = TermColor::Named(NamedColor::Red);
        r[0].flags = CellFlags::INVERSE;
        let l = layout(&r);
        let p = test_palette();
        assert_eq!(l.bgs[0].color, p.ansi[1]);
        assert_eq!(l.runs[0].style.fg, p.background);
    }

    #[test]
    fn wide_char_counts_two_cells_and_skips_spacer() {
        let mut r = row("中 a");
        r[0].flags = CellFlags::WIDE;
        r[1].flags = CellFlags::WIDE_SPACER;
        let l = layout(&r);
        assert_eq!(l.runs.len(), 1);
        assert_eq!(l.runs[0].text, "中a");
        assert_eq!(l.runs[0].cells, 3);
    }

    #[test]
    fn wide_char_in_last_column() {
        // A wide char at cols-2 plus its spacer, then a row that starts at 0.
        let mut r = row("ab中 ");
        r[2].flags = CellFlags::WIDE;
        r[3].flags = CellFlags::WIDE_SPACER;
        let l = layout(&r);
        assert_eq!(l.runs[0].cells, 4);
        // A leading spacer left at the end of a row by a wrapped wide char.
        let mut r = row("ab ");
        r[2].flags = CellFlags::WIDE_SPACER;
        let l = layout(&r);
        assert_eq!(l.runs[0].text, "ab");
        let l = layout(&row("xy"));
        assert_eq!(l.runs[0].col, 0);
    }

    #[test]
    fn combining_marks_join_previous_cell() {
        let mut r = row("e");
        r[0].zerowidth = Some(vec!['\u{302}', '\u{301}'].into_boxed_slice());
        let l = layout(&r);
        assert_eq!(l.runs[0].text, "e\u{302}\u{301}");
        assert_eq!(l.runs[0].cells, 1);
    }

    #[test]
    fn block_glyphs_become_rects() {
        let l = layout(&row("█"));
        assert!(l.runs.is_empty());
        assert_eq!(l.blocks.len(), 1);
        let b = l.blocks[0];
        assert_eq!((b.x, b.y, b.w, b.h), (0.0, 0.0, 1.0, 1.0));
        let l = layout(&row("▀"));
        let b = l.blocks[0];
        assert_eq!((b.x, b.y, b.w, b.h), (0.0, 0.0, 1.0, 0.5));
    }

    #[test]
    fn contrast_applies_only_to_palette_colors() {
        let p = test_palette();
        let mut r = row("a");
        r[0].fg = TermColor::Named(NamedColor::BrightBlack);
        let mut dark = p.clone();
        dark.ansi[8] = Rgba8::rgb(30, 30, 30);
        let l = layout_line(&r, &dark, 4.5, false);
        assert!(crate::color::contrast_ratio(l.runs[0].style.fg, dark.background) >= 4.5);

        r[0].fg = TermColor::Rgb(20, 20, 20);
        let l = layout_line(&r, &dark, 4.5, false);
        assert_eq!(l.runs[0].style.fg, Rgba8::rgb(20, 20, 20));
        r[0].fg = TermColor::Indexed(100);
        let l = layout_line(&r, &dark, 4.5, false);
        assert_eq!(l.runs[0].style.fg, Rgba8::rgb(135, 135, 0));
    }

    #[test]
    fn contrast_one_disables() {
        let mut r = row("a");
        let mut p = test_palette();
        p.ansi[8] = Rgba8::rgb(30, 30, 30);
        r[0].fg = TermColor::Named(NamedColor::BrightBlack);
        let l = layout_line(&r, &p, 1.0, false);
        assert_eq!(l.runs[0].style.fg, Rgba8::rgb(30, 30, 30));
    }

    #[test]
    fn hidden_cells_render_as_spaces() {
        let mut r = row("ab");
        r[1].flags = CellFlags::HIDDEN;
        let l = layout(&r);
        assert_eq!(l.runs[0].text, "a");
    }

    fn underlined(mut c: Cell) -> Cell {
        c.flags = CellFlags::UNDERLINE;
        c
    }

    #[test]
    fn plain_space_does_not_inherit_underline() {
        let r = vec![underlined(cell('a')), cell(' '), underlined(cell('b'))];
        let l = layout(&r);
        assert_eq!(l.runs.len(), 2);
        assert_eq!((l.runs[0].col, l.runs[0].text.as_str()), (0, "a"));
        assert_eq!((l.runs[1].col, l.runs[1].text.as_str()), (2, "b"));
    }

    #[test]
    fn trailing_plain_space_after_underline_is_not_underlined() {
        let r = vec![underlined(cell('a')), cell(' '), cell(' ')];
        let l = layout(&r);
        assert_eq!(l.runs.len(), 1);
        assert_eq!((l.runs[0].text.as_str(), l.runs[0].cells), ("a", 1));
    }

    #[test]
    fn underlined_space_is_drawn() {
        let r = vec![
            underlined(cell('a')),
            underlined(cell(' ')),
            underlined(cell('b')),
        ];
        let l = layout(&r);
        assert_eq!(l.runs.len(), 1);
        assert_eq!(l.runs[0].text, "a b");
    }
}
