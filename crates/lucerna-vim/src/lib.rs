//! Vim modal editing state machine (no UI): it keeps its own cursor,
//! registers and marks, and reports side effects, edits included, as
//! [`Action`]s.
//! It knows nothing about the UI; the view is reached through [`ViewInfo`].

mod classify;
mod edit;
mod ex;
mod ex_run;
mod macros;
mod marks;
mod motion;
mod object;
mod operator;
mod register;
mod repeat;
mod scroll;
mod search;
mod state;
mod substitute;
#[cfg(test)]
mod test_util;
mod visual;

use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use lucerna_editor::{Ctx, EditPlan};
use lucerna_text::BufferSnapshot;

pub use ex::describe_command;
pub use ex_run::ExCommand;
pub use macros::parse_notation;
pub use register::{Clipboard, ClipboardMode, RegContent, RegKind};
pub use repeat::{ReplayStep, Typed};
pub use state::{Vim, VimGlobals};
pub use substitute::{BACKGROUND_SUBSTITUTE_BYTES, SubstituteJob, SubstituteResult};
pub use visual::{VimSelection, block_ranges, exclusive_range, yank_text};

/// A key as the vim layer sees it, already translated by the workbench.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Ctrl(char),
    Escape,
    Enter,
    /// Not handled by vim yet: it falls through to the keymap.
    Backspace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Visual,
    VisualLine,
    VisualBlock,
    /// Keys type text: they go to the editor, not to vim, until Escape.
    Insert,
}

/// Where `i a I A o O` start inserting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertAt {
    /// `i`: at the cursor.
    Before,
    /// `a`: after the char under the cursor.
    After,
    /// `I`: at the first non-blank of the line.
    LineStart,
    /// `A`: at the end of the line.
    LineEnd,
    /// `o`: on a new line below.
    LineBelow,
    /// `O`: on a new line above.
    LineAbove,
}

impl Mode {
    /// The text of the mode indicator.
    pub fn label(self) -> &'static str {
        match self {
            Mode::Normal => "-- NORMAL --",
            Mode::Visual => "-- VISUAL --",
            Mode::VisualLine => "-- VISUAL LINE --",
            Mode::VisualBlock => "-- VISUAL BLOCK --",
            Mode::Insert => "-- INSERT --",
        }
    }

    /// The value of the `vim.mode` context key.
    pub fn context_value(self) -> &'static str {
        match self {
            Mode::Normal => "Normal",
            Mode::Visual => "Visual",
            Mode::VisualLine => "VisualLine",
            Mode::VisualBlock => "VisualBlock",
            Mode::Insert => "Insert",
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
    /// `vim.useSystemClipboard`
    pub use_system_clipboard: ClipboardMode,
    /// `vim.changeWordIncludesWhitespace`: `cw` on a word acts like `dw`.
    pub change_word_includes_whitespace: bool,
    /// `vim.joinspaces`: `J` puts two spaces after `.`, `!` and `?`.
    pub joinspaces: bool,
    /// `vim.tildeop`: `~` is an operator like `g~`.
    pub tildeop: bool,
    /// `vim.gdefault`: `:s` replaces every match in a line unless it has `g`.
    pub gdefault: bool,
    /// `vim.highlightedyank.enable`: a yank emits [`Action::Yanked`].
    pub highlighted_yank: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            ignorecase: true,
            wrapscan: true,
            startofline: true,
            scroll: 0,
            scrolloff: 0,
            use_system_clipboard: ClipboardMode::OnYank,
            change_word_includes_whitespace: false,
            joinspaces: true,
            tildeop: false,
            gdefault: false,
            highlighted_yank: false,
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
    /// What edit plans are built against; `None` for a read-only view,
    /// which gets no [`Action::Edit`].
    fn edit_ctx(&self) -> Option<Ctx<'_>>;
    /// The byte ranges of the syntax nodes that contain `range`, innermost
    /// first; empty without a syntax tree. Text objects use them to skip
    /// brackets in strings and comments and to find tags.
    fn enclosing_nodes(&self, range: Range<usize>) -> Vec<Range<usize>>;
}

/// A side effect for the view to carry out. The cursor itself is read back
/// from [`Vim::cursor`] after every key.
#[derive(Debug, Clone, PartialEq)]
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
    Fold {
        row: u32,
        action: FoldAction,
    },
    ModeChanged(Mode),
    /// Run the workbench command with this id (`gd`, `]d`, `ctrl-o`...).
    RunCommand(&'static str),
    /// Insert mode began: the view puts its caret where `InsertAt` says
    /// ([`Vim::cursor`] for all but `LineBelow` and `LineAbove`, which open
    /// a line) and takes the keys until Escape.
    EnterInsert(InsertAt),
    /// Apply this plan to the view's document. Every edit from one key
    /// forms one undo step; vim's cursor is clamped afterwards. A feed
    /// emits at most one `Edit`: every plan is built against the text
    /// before the feed, so a command combines its edits into one plan.
    Edit(EditPlan),
    /// Undo this many steps of the document.
    Undo(u32),
    /// Redo this many steps of the document.
    Redo(u32),
    /// Put `text` on the system clipboard; `linewise` marks it as whole
    /// lines for the editor's paste.
    WriteClipboard {
        text: String,
        linewise: bool,
    },
    /// The command failed: a motion that could not move, a search with
    /// no match, an operator whose motion failed, `.` with nothing to
    /// repeat, a `:` command with an error (its message came first as
    /// [`Action::Message`]). A macro stops here.
    Failed,
    /// Carry out a `:` command the editor owns (`:w`, `:q`...).
    RunEx(ExCommand),
    /// Open the command line with `prefill` typed in.
    OpenCommandLine {
        prefill: String,
    },
    /// Show `text` in the status area.
    Message(String),
    /// Text was yanked (`y{motion}`, `yy`, `Y`, Visual `y`, `:y`) and
    /// [`Settings::highlighted_yank`] is on: the byte ranges of the yanked
    /// text in the current text. Whole lines include their line break.
    Yanked(Vec<Range<usize>>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// The key was consumed; stop propagation.
    Handled(Vec<Action>),
    /// The key is not vim's; pass it on to the keymap.
    Unhandled,
    /// The matches for the query are not ready. The state is unchanged and
    /// the same key should be fed again once they are.
    Retry(SearchQuery),
    /// Run these steps in order, all in one undo group: feed each
    /// [`ReplayStep::Key`] (and apply its actions) before the next step,
    /// and type each [`ReplayStep::Type`] through the editor's typing path
    /// (with the view's live auto-closed regions, so type-over and pair
    /// deletion behave as when typed) without calling
    /// [`Vim::record_typed`]. A [`ReplayStep::Search`] (the `/` or `?` of
    /// an operator, `d/foo<CR>`) calls [`Vim::search_submitted`] with its
    /// query from vim's cursor, finding the matches at once: answer its
    /// [`Outcome::Retry`] by searching synchronously, with no find widget.
    /// When [`Vim::operator_pending`] is false there (the keys before it
    /// failed), stop: drop it and every step after it.
    ///
    /// - **Stop** when a `Type` step comes while vim is not in Insert mode
    ///   (the command that should have entered it failed, e.g. `ci(` with
    ///   no pair): drop it and every step after it.
    /// - A key fed here may itself return `Replay` (a counted insert at
    ///   Escape): run those steps in place, before the rest.
    /// - `.` returns it, and so does the user's own Escape that ends a
    ///   counted insert (`3ifoo<Esc>`), outside any `.`. That one runs
    ///   inside the Insert session's undo group, still open (its last
    ///   step, Escape, ends the session), and its typing is not recorded
    ///   either: the record already holds one copy.
    Replay(Vec<ReplayStep>),
    /// A `:s` on a large buffer: run [`SubstituteJob::run`] on the view's
    /// snapshot (it is `Send`, so it may run in the background), then pass
    /// the result to [`Vim::finish_substitute`] if the text is unchanged.
    /// Vim's state is unchanged until then.
    Substitute(SubstituteJob),
    /// `@a`: run these keys in order, all in one undo group, with
    /// [`VimGlobals::set_macro_running`] set until they end, then call
    /// [`Vim::macro_ended`]. A key that comes while vim is in Insert mode
    /// (other than Escape) types: `Char('\t')` is [`Typed::Tab`],
    /// `Char(c)` [`Typed::Text`], `Enter` [`Typed::Newline`], `Backspace`
    /// [`Typed::DeleteLeft`]; a `Ctrl` key is skipped. The whole run stops
    /// at a key that returns [`Action::Failed`]. A `Macro` returned by a
    /// key in the run (`@b` inside `@a`) runs in place.
    Macro(Vec<Key>),
}
