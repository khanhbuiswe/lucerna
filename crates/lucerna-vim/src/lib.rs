//! Vim navigation for a read-only code view: a key-driven state machine
//! that keeps its own cursor and reports side effects as [`Action`]s.
//! It knows nothing about the UI; the view is reached through [`ViewInfo`].

mod classify;
mod marks;
mod motion;
mod scroll;
mod search;
mod state;
#[cfg(test)]
mod test_util;
mod visual;

use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use lucerna_text::BufferSnapshot;

pub use state::{Vim, VimGlobals};
pub use visual::{VimSelection, block_ranges, exclusive_range, yank_text};

/// A key as the vim layer sees it, already translated by the workbench.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Ctrl(char),
    Escape,
    Enter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Visual,
    VisualLine,
    VisualBlock,
}

impl Mode {
    /// The text of the mode indicator.
    pub fn label(self) -> &'static str {
        match self {
            Mode::Normal => "-- NORMAL --",
            Mode::Visual => "-- VISUAL --",
            Mode::VisualLine => "-- VISUAL LINE --",
            Mode::VisualBlock => "-- VISUAL BLOCK --",
        }
    }

    /// The value of the `vim.mode` context key.
    pub fn context_value(self) -> &'static str {
        match self {
            Mode::Normal => "Normal",
            Mode::Visual => "Visual",
            Mode::VisualLine => "VisualLine",
            Mode::VisualBlock => "VisualBlock",
        }
    }
}

/// User settings that change how commands behave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// `vim.ignorecase`
    pub ignorecase: bool,
    /// `vim.wrapscan`
    pub wrapscan: bool,
    /// `vim.startofline`
    pub startofline: bool,
    /// `vim.scroll`: rows for `ctrl+d`/`ctrl+u`; 0 means half a page.
    pub scroll: u32,
    /// `editor.cursorSurroundingLines`
    pub scrolloff: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            ignorecase: true,
            wrapscan: true,
            startofline: true,
            scroll: 0,
            scrolloff: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SearchQuery {
    pub pattern: String,
    pub is_regex: bool,
    pub case_sensitive: bool,
    pub whole_word: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoldAction {
    Toggle,
    Close,
    Open,
}

/// What the vim layer may read from the view. Vim keeps the cursor and the
/// visual anchor itself; the view's selection is only an output.
pub trait ViewInfo {
    /// Marks are kept per file.
    fn path(&self) -> &Arc<Path>;
    fn snapshot(&self) -> &BufferSnapshot;
    /// The first visible buffer row.
    fn top_row(&self) -> u32;
    /// 0 before the first paint; callers then assume 30.
    fn visible_rows(&self) -> u32;
    /// True for a row inside a closed fold.
    fn is_row_hidden(&self, row: u32) -> bool;
    fn tab_size(&self) -> u32;
    /// The partner bracket of the one at `offset`, syntax-aware when possible.
    fn matching_bracket(&self, offset: usize) -> Option<usize>;
    /// Every match of `query` as sorted byte ranges; `None` while a
    /// background search is still running.
    fn matches(&self, query: &SearchQuery) -> Option<Arc<[Range<usize>]>>;
}

/// A side effect for the view to carry out. The cursor itself is read back
/// from [`Vim::cursor`] after every key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// The cursor or selection changed; `jump` centres it if off-screen.
    CursorMoved {
        jump: bool,
    },
    ScrollTopRow(u32),
    OpenSearch {
        backward: bool,
    },
    Highlight(SearchQuery),
    Yank(String),
    Fold {
        row: u32,
        action: FoldAction,
    },
    ModeChanged(Mode),
    /// Run the workbench command with this id (`gd`, `]d`, `ctrl-o`...).
    RunCommand(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The key was consumed; stop propagation.
    Handled(Vec<Action>),
    /// The key is not vim's; pass it on to the keymap.
    Unhandled,
    /// The matches for the query are not ready. The state is unchanged and
    /// the same key should be fed again once they are.
    Retry(SearchQuery),
}
