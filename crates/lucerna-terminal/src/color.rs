//! Turning terminal colours into RGB, and keeping text legible.

use std::cell::RefCell;
use std::collections::HashMap;

use crate::snapshot::{NamedColor, TermColor};

/// An 8-bit RGBA colour. The workbench converts it to a gpui colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgba8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba8 {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }
}

/// The theme's terminal colours.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalPalette {
    /// Colours 0-15: the eight normal ones, then the eight bright ones.
    pub ansi: [Rgba8; 16],
    /// Dim variants of colours 0-7.
    pub dim: [Rgba8; 8],
    pub foreground: Rgba8,
    pub bright_foreground: Rgba8,
    pub dim_foreground: Rgba8,
    pub background: Rgba8,
    pub cursor: Rgba8,
    pub selection: Rgba8,
}

fn named_index(c: NamedColor) -> Option<usize> {
    use NamedColor::*;
    Some(match c {
        Black => 0,
        Red => 1,
        Green => 2,
        Yellow => 3,
        Blue => 4,
        Magenta => 5,
        Cyan => 6,
        White => 7,
        BrightBlack => 8,
        BrightRed => 9,
        BrightGreen => 10,
        BrightYellow => 11,
        BrightBlue => 12,
        BrightMagenta => 13,
        BrightCyan => 14,
        BrightWhite => 15,
        _ => return None,
    })
}

fn dim_index(c: NamedColor) -> Option<usize> {
    use NamedColor::*;
    Some(match c {
        DimBlack => 0,
        DimRed => 1,
        DimGreen => 2,
        DimYellow => 3,
        DimBlue => 4,
        DimMagenta => 5,
        DimCyan => 6,
        DimWhite => 7,
        _ => return None,
    })
}

/// Resolves a colour and says whether it came from the theme (named colours
/// and indices 0-15), which is where minimum contrast applies.
pub(crate) fn resolve_color_origin(
    c: TermColor,
    palette: &TerminalPalette,
    is_fg: bool,
    bold: bool,
    dim: bool,
    bold_bright: bool,
) -> (Rgba8, bool) {
    // Normal 0..=7 colours may become bright or dim for foreground text.
    let ansi = |i: usize| -> Rgba8 {
        if i < 8 && is_fg {
            if bold && bold_bright {
                return palette.ansi[i + 8];
            }
            if dim && !bold {
                return palette.dim[i];
            }
        }
        palette.ansi[i]
    };
    match c {
        TermColor::Named(n) => {
            if let Some(i) = named_index(n) {
                return (ansi(i), true);
            }
            if let Some(i) = dim_index(n) {
                return (palette.dim[i], true);
            }
            let rgb = match n {
                NamedColor::Foreground => {
                    if is_fg && bold {
                        palette.bright_foreground
                    } else if is_fg && dim {
                        palette.dim_foreground
                    } else {
                        palette.foreground
                    }
                }
                NamedColor::Background => palette.background,
                NamedColor::Cursor => palette.cursor,
                NamedColor::BrightForeground => palette.bright_foreground,
                NamedColor::DimForeground => palette.dim_foreground,
                _ => palette.foreground,
            };
            (rgb, true)
        }
        TermColor::Indexed(i) if i < 16 => (ansi(i as usize), true),
        TermColor::Indexed(i) if i < 232 => {
            let n = i - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + 40 * v };
            (
                Rgba8::rgb(level(n / 36), level((n / 6) % 6), level(n % 6)),
                false,
            )
        }
        TermColor::Indexed(i) => {
            let v = 8 + 10 * (i - 232);
            (Rgba8::rgb(v, v, v), false)
        }
        TermColor::Rgb(r, g, b) => (Rgba8::rgb(r, g, b), false),
    }
}

/// Resolves a terminal colour to RGB through the palette.
pub fn resolve_color(
    c: TermColor,
    palette: &TerminalPalette,
    is_fg: bool,
    bold: bool,
    dim: bool,
    bold_bright: bool,
) -> Rgba8 {
    resolve_color_origin(c, palette, is_fg, bold, dim, bold_bright).0
}

/// The colour an OSC 4/10/11/12 query asks for, by alacritty's index:
/// 0-255 the 256 colours, then foreground (256), background (257), cursor
/// (258), the eight dim colours (259-266), bright (267) and dim (268)
/// foreground. A translucent colour is given over the background, as drawn.
pub(crate) fn query_color(palette: &TerminalPalette, index: usize) -> Option<Rgba8> {
    let c = match index {
        0..=255 => resolve_color(
            TermColor::Indexed(index as u8),
            palette,
            false,
            false,
            false,
            false,
        ),
        256 => palette.foreground,
        257 => palette.background,
        258 => palette.cursor,
        259..=266 => palette.dim[index - 259],
        267 => palette.bright_foreground,
        268 => palette.dim_foreground,
        _ => return None,
    };
    Some(over(c, palette.background))
}

/// `c` drawn over the opaque `bg`.
fn over(c: Rgba8, bg: Rgba8) -> Rgba8 {
    if c.a == 255 {
        return Rgba8 { a: 255, ..c };
    }
    let t = c.a as f32 / 255.0;
    let m = |f: u8, b: u8| (f as f32 * t + b as f32 * (1.0 - t)).round() as u8;
    Rgba8::rgb(m(c.r, bg.r), m(c.g, bg.g), m(c.b, bg.b))
}

fn channel(v: u8) -> f32 {
    let s = v as f32 / 255.0;
    if s <= 0.03928 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}

fn luminance(c: Rgba8) -> f32 {
    0.2126 * channel(c.r) + 0.7152 * channel(c.g) + 0.0722 * channel(c.b)
}

/// WCAG 2.x contrast ratio, from 1 to 21. Alpha is ignored.
pub fn contrast_ratio(a: Rgba8, b: Rgba8) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

fn mix(from: Rgba8, to: u8, t: f32) -> Rgba8 {
    let m = |a: u8| {
        (a as f32 + (to as f32 - a as f32) * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Rgba8 {
        r: m(from.r),
        g: m(from.g),
        b: m(from.b),
        a: from.a,
    }
}

/// Moves `fg` toward white or black until it reaches `min` contrast against
/// `bg`, changing it as little as possible. `min <= 1` leaves it alone.
pub fn ensure_contrast(fg: Rgba8, bg: Rgba8, min: f32) -> Rgba8 {
    if min <= 1.0 || contrast_ratio(fg, bg) >= min {
        return fg;
    }
    let key = (fg, bg, min.to_bits());
    if let Some(hit) = CACHE.with(|c| c.borrow().get(&key).copied()) {
        return hit;
    }
    let toward = |target: u8| -> (Rgba8, bool) {
        let end = mix(fg, target, 1.0);
        if contrast_ratio(end, bg) < min {
            return (end, false);
        }
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        for _ in 0..12 {
            let mid = (lo + hi) / 2.0;
            if contrast_ratio(mix(fg, target, mid), bg) >= min {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        (mix(fg, target, hi), true)
    };
    let (white, white_ok) = toward(255);
    let (black, black_ok) = toward(0);
    let pick = match (white_ok, black_ok) {
        (true, false) => white,
        (false, true) => black,
        (true, true) => {
            // The smaller change wins.
            let dist = |c: Rgba8| {
                (c.r as i32 - fg.r as i32).abs()
                    + (c.g as i32 - fg.g as i32).abs()
                    + (c.b as i32 - fg.b as i32).abs()
            };
            if dist(white) <= dist(black) {
                white
            } else {
                black
            }
        }
        (false, false) => {
            if contrast_ratio(white, bg) >= contrast_ratio(black, bg) {
                white
            } else {
                black
            }
        }
    };
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() > 4096 {
            c.clear();
        }
        c.insert(key, pick);
    });
    pick
}

type ContrastKey = (Rgba8, Rgba8, u32);

thread_local! {
    static CACHE: RefCell<HashMap<ContrastKey, Rgba8>> = RefCell::new(HashMap::new());
}

#[cfg(test)]
pub(crate) fn test_palette() -> TerminalPalette {
    let ansi = [
        Rgba8::rgb(0, 0, 0),
        Rgba8::rgb(205, 49, 49),
        Rgba8::rgb(13, 188, 121),
        Rgba8::rgb(229, 229, 16),
        Rgba8::rgb(36, 114, 200),
        Rgba8::rgb(188, 63, 188),
        Rgba8::rgb(17, 168, 205),
        Rgba8::rgb(229, 229, 229),
        Rgba8::rgb(102, 102, 102),
        Rgba8::rgb(241, 76, 76),
        Rgba8::rgb(35, 209, 139),
        Rgba8::rgb(245, 245, 67),
        Rgba8::rgb(59, 142, 234),
        Rgba8::rgb(214, 112, 214),
        Rgba8::rgb(41, 184, 219),
        Rgba8::rgb(255, 255, 255),
    ];
    let mut dim = [Rgba8::rgb(0, 0, 0); 8];
    for (i, d) in dim.iter_mut().enumerate() {
        *d = Rgba8 { a: 178, ..ansi[i] };
    }
    TerminalPalette {
        ansi,
        dim,
        foreground: Rgba8::rgb(204, 204, 204),
        bright_foreground: Rgba8::rgb(255, 255, 255),
        dim_foreground: Rgba8::rgb(150, 150, 150),
        background: Rgba8::rgb(0, 0, 0),
        cursor: Rgba8::rgb(255, 255, 255),
        selection: Rgba8 {
            r: 60,
            g: 90,
            b: 160,
            a: 128,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_16_to_231_is_xterm_cube() {
        let p = test_palette();
        let c = resolve_color(TermColor::Indexed(196), &p, true, false, false, false);
        assert_eq!((c.r, c.g, c.b), (255, 0, 0));
        let c = resolve_color(TermColor::Indexed(16), &p, true, false, false, false);
        assert_eq!((c.r, c.g, c.b), (0, 0, 0));
        let c = resolve_color(TermColor::Indexed(17), &p, true, false, false, false);
        assert_eq!((c.r, c.g, c.b), (0, 0, 95));
    }

    #[test]
    fn index_232_to_255_is_gray_ramp() {
        let p = test_palette();
        let c = resolve_color(TermColor::Indexed(232), &p, true, false, false, false);
        assert_eq!((c.r, c.g, c.b), (8, 8, 8));
        let c = resolve_color(TermColor::Indexed(255), &p, true, false, false, false);
        assert_eq!((c.r, c.g, c.b), (238, 238, 238));
    }

    #[test]
    fn named_from_palette() {
        let p = test_palette();
        let c = resolve_color(
            TermColor::Named(NamedColor::Red),
            &p,
            true,
            false,
            false,
            false,
        );
        assert_eq!(c, p.ansi[1]);
        let c = resolve_color(
            TermColor::Named(NamedColor::DimGreen),
            &p,
            true,
            false,
            false,
            false,
        );
        assert_eq!(c, p.dim[2]);
        let c = resolve_color(
            TermColor::Named(NamedColor::Foreground),
            &p,
            true,
            false,
            false,
            false,
        );
        assert_eq!(c, p.foreground);
        let c = resolve_color(TermColor::Indexed(9), &p, true, false, false, false);
        assert_eq!(c, p.ansi[9]);
    }

    #[test]
    fn bold_bright() {
        let p = test_palette();
        let c = resolve_color(
            TermColor::Named(NamedColor::Red),
            &p,
            true,
            true,
            false,
            true,
        );
        assert_eq!(c, p.ansi[9]);
        let c = resolve_color(
            TermColor::Named(NamedColor::Red),
            &p,
            true,
            true,
            false,
            false,
        );
        assert_eq!(c, p.ansi[1]);
        let c = resolve_color(
            TermColor::Named(NamedColor::Red),
            &p,
            false,
            true,
            false,
            true,
        );
        assert_eq!(c, p.ansi[1]);
    }

    #[test]
    fn query_color_covers_xterm_indices_and_named_slots() {
        let p = test_palette();
        let rgb = |i| query_color(&p, i).map(|c| (c.r, c.g, c.b));
        assert_eq!(rgb(1), Some((205, 49, 49)));
        assert_eq!(rgb(16), Some((0, 0, 0)));
        assert_eq!(rgb(231), Some((255, 255, 255)));
        assert_eq!(rgb(232), Some((8, 8, 8)));
        assert_eq!(rgb(255), Some((238, 238, 238)));
        assert_eq!(rgb(256), Some((204, 204, 204)));
        assert_eq!(rgb(257), Some((0, 0, 0)));
        assert_eq!(rgb(258), Some((255, 255, 255)));
        // Dim red at alpha 178 over the black background.
        assert_eq!(rgb(260), Some((143, 34, 34)));
        assert_eq!(rgb(269), None);
    }

    #[test]
    fn contrast_ratio_extremes() {
        let r = contrast_ratio(Rgba8::rgb(0, 0, 0), Rgba8::rgb(255, 255, 255));
        assert!((r - 21.0).abs() < 0.01);
    }

    #[test]
    fn ensure_contrast_lifts_dark_on_black() {
        let bg = Rgba8::rgb(0, 0, 0);
        let out = ensure_contrast(Rgba8::rgb(40, 40, 40), bg, 4.5);
        assert!(contrast_ratio(out, bg) >= 4.5);
        assert_eq!(
            ensure_contrast(Rgba8::rgb(40, 40, 40), bg, 1.0),
            Rgba8::rgb(40, 40, 40)
        );
    }
}
