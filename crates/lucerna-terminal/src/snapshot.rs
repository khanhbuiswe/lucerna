//! A copy of the visible screen in Lucerna's own types, taken under one lock.

use std::sync::Arc;

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::Point;
use alacritty_terminal::term::cell::{Cell as AlacCell, Flags};
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::{
    Color as AlacColor, CursorShape as AlacCursorShape, NamedColor as AlacNamedColor,
};

/// The visible screen, its cursor and the terminal modes.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub rows: u16,
    pub cols: u16,
    /// Visible rows only, `rows × cols`.
    pub cells: Vec<Vec<Cell>>,
    /// `wrapped[i]`: row `i` continues into row `i + 1` (soft wrap).
    pub wrapped: Vec<bool>,
    /// `None` when the app hid the cursor (DECTCEM) or it is scrolled out of view.
    pub cursor: Option<CursorState>,
    pub selection: Option<SelectionRange>,
    pub mode: Modes,
    /// Lines scrolled up into history; 0 at the bottom.
    pub display_offset: usize,
    /// Lines in scrollback, not counting the screen.
    pub history_len: usize,
}

/// One screen cell.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub ch: char,
    /// Combining marks and other zero-width characters drawn over `ch`.
    pub zerowidth: Option<Box<[char]>>,
    pub fg: TermColor,
    pub bg: TermColor,
    pub flags: CellFlags,
    /// OSC 8 link target.
    pub hyperlink: Option<Arc<str>>,
}

/// A cell colour before the theme turns it into RGB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TermColor {
    Named(NamedColor),
    Indexed(u8),
    Rgb(u8, u8, u8),
}

/// The ANSI colours and the theme's special colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NamedColor {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    BrightBlack,
    BrightRed,
    BrightGreen,
    BrightYellow,
    BrightBlue,
    BrightMagenta,
    BrightCyan,
    BrightWhite,
    Foreground,
    Background,
    Cursor,
    DimBlack,
    DimRed,
    DimGreen,
    DimYellow,
    DimBlue,
    DimMagenta,
    DimCyan,
    DimWhite,
    BrightForeground,
    DimForeground,
}

/// Where the cursor sits on the visible screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorState {
    pub row: u16,
    pub col: u16,
    pub shape: CursorShape,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorShape {
    Block,
    Underline,
    Bar,
    Hidden,
}

/// A point in the whole buffer: line 0 is the top of the screen at the
/// bottom of the scrollback, and history lines are negative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct GridPoint {
    pub line: i32,
    pub col: u16,
}

/// The selected span, inclusive at both ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionRange {
    pub start: GridPoint,
    pub end: GridPoint,
    pub is_block: bool,
}

/// How far to move the viewport. Positive `Lines` scroll up into history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scroll {
    Lines(i32),
    PageUp,
    PageDown,
    Top,
    Bottom,
}

macro_rules! flag_set {
    ($(#[$meta:meta])* $name:ident: $repr:ty { $($(#[$fmeta:meta])* $flag:ident = $value:expr;)* }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
        pub struct $name($repr);

        impl $name {
            $($(#[$fmeta])* pub const $flag: Self = Self($value);)*

            pub const fn empty() -> Self {
                Self(0)
            }

            pub const fn bits(self) -> $repr {
                self.0
            }

            pub const fn is_empty(self) -> bool {
                self.0 == 0
            }

            /// All of `other`'s flags are set.
            pub const fn contains(self, other: Self) -> bool {
                self.0 & other.0 == other.0
            }

            /// Any of `other`'s flags is set.
            pub const fn intersects(self, other: Self) -> bool {
                self.0 & other.0 != 0
            }

            pub fn insert(&mut self, other: Self) {
                self.0 |= other.0;
            }
        }

        impl std::ops::BitOr for $name {
            type Output = Self;
            fn bitor(self, rhs: Self) -> Self {
                Self(self.0 | rhs.0)
            }
        }

        impl std::ops::BitOrAssign for $name {
            fn bitor_assign(&mut self, rhs: Self) {
                self.0 |= rhs.0;
            }
        }

        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                let mut set = f.debug_set();
                $(if self.contains(Self::$flag) && Self::$flag.0.count_ones() == 1 {
                    set.entry(&format_args!(stringify!($flag)));
                })*
                set.finish()
            }
        }
    };
}

flag_set! {
    /// Terminal modes the UI needs for drawing and for encoding input.
    Modes: u32 {
        ALT_SCREEN = 1 << 0;
        MOUSE_REPORT_CLICK = 1 << 1;
        MOUSE_DRAG = 1 << 2;
        MOUSE_MOTION = 1 << 3;
        /// Any mouse reporting mode.
        MOUSE_MODE = (1 << 1) | (1 << 2) | (1 << 3);
        SGR_MOUSE = 1 << 4;
        UTF8_MOUSE = 1 << 5;
        BRACKETED_PASTE = 1 << 6;
        APP_CURSOR = 1 << 7;
        APP_KEYPAD = 1 << 8;
        ALTERNATE_SCROLL = 1 << 9;
        FOCUS_IN_OUT = 1 << 10;
    }
}

flag_set! {
    /// How a cell's glyph is drawn.
    CellFlags: u16 {
        BOLD = 1 << 0;
        DIM = 1 << 1;
        ITALIC = 1 << 2;
        UNDERLINE = 1 << 3;
        DOUBLE_UNDERLINE = 1 << 4;
        UNDERCURL = 1 << 5;
        STRIKEOUT = 1 << 6;
        INVERSE = 1 << 7;
        HIDDEN = 1 << 8;
        /// The first half of a double-width character.
        WIDE = 1 << 9;
        /// A placeholder cell that draws nothing: the second half of a wide
        /// character, or the end of a line a wide character wrapped past.
        WIDE_SPACER = 1 << 10;
    }
}

const MODE_MAP: [(TermMode, Modes); 11] = [
    (TermMode::ALT_SCREEN, Modes::ALT_SCREEN),
    (TermMode::MOUSE_REPORT_CLICK, Modes::MOUSE_REPORT_CLICK),
    (TermMode::MOUSE_DRAG, Modes::MOUSE_DRAG),
    (TermMode::MOUSE_MOTION, Modes::MOUSE_MOTION),
    (TermMode::SGR_MOUSE, Modes::SGR_MOUSE),
    (TermMode::UTF8_MOUSE, Modes::UTF8_MOUSE),
    (TermMode::BRACKETED_PASTE, Modes::BRACKETED_PASTE),
    (TermMode::APP_CURSOR, Modes::APP_CURSOR),
    (TermMode::APP_KEYPAD, Modes::APP_KEYPAD),
    (TermMode::ALTERNATE_SCROLL, Modes::ALTERNATE_SCROLL),
    (TermMode::FOCUS_IN_OUT, Modes::FOCUS_IN_OUT),
];

const FLAG_MAP: [(Flags, CellFlags); 14] = [
    (Flags::BOLD, CellFlags::BOLD),
    (Flags::DIM, CellFlags::DIM),
    (Flags::ITALIC, CellFlags::ITALIC),
    (Flags::UNDERLINE, CellFlags::UNDERLINE),
    // Lucerna draws dotted and dashed underlines as plain ones.
    (Flags::DOTTED_UNDERLINE, CellFlags::UNDERLINE),
    (Flags::DASHED_UNDERLINE, CellFlags::UNDERLINE),
    (Flags::DOUBLE_UNDERLINE, CellFlags::DOUBLE_UNDERLINE),
    (Flags::UNDERCURL, CellFlags::UNDERCURL),
    (Flags::STRIKEOUT, CellFlags::STRIKEOUT),
    (Flags::INVERSE, CellFlags::INVERSE),
    (Flags::HIDDEN, CellFlags::HIDDEN),
    (Flags::WIDE_CHAR, CellFlags::WIDE),
    (Flags::WIDE_CHAR_SPACER, CellFlags::WIDE_SPACER),
    (Flags::LEADING_WIDE_CHAR_SPACER, CellFlags::WIDE_SPACER),
];

pub(crate) fn modes(mode: TermMode) -> Modes {
    let mut out = Modes::empty();
    for (theirs, ours) in MODE_MAP {
        if mode.contains(theirs) {
            out.insert(ours);
        }
    }
    out
}

fn cell_flags(flags: Flags) -> CellFlags {
    let mut out = CellFlags::empty();
    for (theirs, ours) in FLAG_MAP {
        if flags.contains(theirs) {
            out.insert(ours);
        }
    }
    out
}

fn named(color: AlacNamedColor) -> NamedColor {
    use AlacNamedColor as A;
    match color {
        A::Black => NamedColor::Black,
        A::Red => NamedColor::Red,
        A::Green => NamedColor::Green,
        A::Yellow => NamedColor::Yellow,
        A::Blue => NamedColor::Blue,
        A::Magenta => NamedColor::Magenta,
        A::Cyan => NamedColor::Cyan,
        A::White => NamedColor::White,
        A::BrightBlack => NamedColor::BrightBlack,
        A::BrightRed => NamedColor::BrightRed,
        A::BrightGreen => NamedColor::BrightGreen,
        A::BrightYellow => NamedColor::BrightYellow,
        A::BrightBlue => NamedColor::BrightBlue,
        A::BrightMagenta => NamedColor::BrightMagenta,
        A::BrightCyan => NamedColor::BrightCyan,
        A::BrightWhite => NamedColor::BrightWhite,
        A::Foreground => NamedColor::Foreground,
        A::Background => NamedColor::Background,
        A::Cursor => NamedColor::Cursor,
        A::DimBlack => NamedColor::DimBlack,
        A::DimRed => NamedColor::DimRed,
        A::DimGreen => NamedColor::DimGreen,
        A::DimYellow => NamedColor::DimYellow,
        A::DimBlue => NamedColor::DimBlue,
        A::DimMagenta => NamedColor::DimMagenta,
        A::DimCyan => NamedColor::DimCyan,
        A::DimWhite => NamedColor::DimWhite,
        A::BrightForeground => NamedColor::BrightForeground,
        A::DimForeground => NamedColor::DimForeground,
    }
}

fn color(color: AlacColor) -> TermColor {
    match color {
        AlacColor::Named(n) => TermColor::Named(named(n)),
        AlacColor::Indexed(i) => TermColor::Indexed(i),
        AlacColor::Spec(rgb) => TermColor::Rgb(rgb.r, rgb.g, rgb.b),
    }
}

/// Converts one cell. `last_link` lets a run of cells under the same OSC 8
/// link share one allocation.
fn cell(cell: &AlacCell, last_link: &mut Option<Arc<str>>) -> Cell {
    let hyperlink = cell.hyperlink().map(|link| match last_link {
        Some(prev) if &**prev == link.uri() => prev.clone(),
        _ => {
            let uri: Arc<str> = Arc::from(link.uri());
            *last_link = Some(uri.clone());
            uri
        }
    });
    Cell {
        ch: cell.c,
        zerowidth: cell.zerowidth().map(Box::from),
        fg: color(cell.fg),
        bg: color(cell.bg),
        flags: cell_flags(cell.flags),
        hyperlink,
    }
}

fn grid_point(point: Point) -> GridPoint {
    GridPoint {
        line: point.line.0,
        col: point.column.0 as u16,
    }
}

/// Copies the visible screen. The caller holds the terminal lock.
pub(crate) fn take<T: EventListener>(term: &Term<T>) -> Snapshot {
    let rows = term.screen_lines();
    let cols = term.columns();
    let content = term.renderable_content();
    let offset = content.display_offset;

    let mut cells: Vec<Vec<Cell>> = (0..rows).map(|_| Vec::with_capacity(cols)).collect();
    let mut wrapped = vec![false; rows];
    let mut last_link = None;
    for indexed in content.display_iter {
        let row = indexed.point.line.0 + offset as i32;
        if let Some(r) = usize::try_from(row).ok().filter(|r| *r < rows) {
            cells[r].push(cell(indexed.cell, &mut last_link));
            if indexed.cell.flags.contains(Flags::WRAPLINE) {
                wrapped[r] = true;
            }
        }
    }

    let cursor_row = content.cursor.point.line.0 + offset as i32;
    let cursor = match content.cursor.shape {
        AlacCursorShape::Hidden => None,
        _ if !(0..rows as i32).contains(&cursor_row) => None,
        shape => Some(CursorState {
            row: cursor_row as u16,
            col: content.cursor.point.column.0 as u16,
            shape: match shape {
                AlacCursorShape::Underline => CursorShape::Underline,
                AlacCursorShape::Beam => CursorShape::Bar,
                _ => CursorShape::Block,
            },
        }),
    };

    Snapshot {
        rows: rows as u16,
        cols: cols as u16,
        cells,
        wrapped,
        cursor,
        selection: content.selection.map(|s| SelectionRange {
            start: grid_point(s.start),
            end: grid_point(s.end),
            is_block: s.is_block,
        }),
        mode: modes(content.mode),
        display_offset: offset,
        history_len: term.history_size(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_map_each_flag() {
        let m = modes(TermMode::ALT_SCREEN | TermMode::SGR_MOUSE | TermMode::SHOW_CURSOR);
        assert_eq!(m, Modes::ALT_SCREEN | Modes::SGR_MOUSE);
        assert_eq!(format!("{m:?}"), "{ALT_SCREEN, SGR_MOUSE}");
    }

    #[test]
    fn mouse_mode_covers_every_report_mode() {
        for m in [
            Modes::MOUSE_REPORT_CLICK,
            Modes::MOUSE_DRAG,
            Modes::MOUSE_MOTION,
        ] {
            assert!(m.intersects(Modes::MOUSE_MODE));
        }
        assert!(!Modes::SGR_MOUSE.intersects(Modes::MOUSE_MODE));
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn long_line_sets_wrapped() {
        let t = crate::Terminal::fake(3, 10, 0);
        t.feed(b"abcdefghijklmno\r\nxyz");
        let snap = t.snapshot();
        assert_eq!(snap.wrapped, vec![true, false, false]);
    }
}
