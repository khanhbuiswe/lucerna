//! What the Quick Open input asks for, decided by its first character.

/// The mode and argument of a Quick Open input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuickOpenQuery {
    /// No prefix: fuzzy-find files.
    Files(String),
    /// `>`: filter commands.
    Commands(String),
    /// `:`: go to a line and optional 1-based column. A negative line
    /// counts from the end of the file; see [`resolve_line`].
    GotoLine {
        line: Option<i64>,
        column: Option<u32>,
    },
    /// `@`: the symbols of the active document; `@:` groups them by kind.
    Symbols { text: String, group_by_kind: bool },
    /// `#`: symbols of the whole workspace.
    WorkspaceSymbols(String),
    /// `:` with Vim on: a Vim command line (leading spaces trimmed).
    VimCommand(String),
}

/// Parses the Quick Open input. A leading `>` gives
/// [`QuickOpenQuery::Commands`], a leading `:` gives
/// [`QuickOpenQuery::GotoLine`] (`":12:5"`, `":12,5"` and `":12#5"` are
/// line 12, column 5; `":-1"` is the last line; parts that are not numbers,
/// and a negative column, are `None`; a third part is ignored), `@` gives
/// [`QuickOpenQuery::Symbols`] (grouped by kind after `@:`) and `#` gives
/// [`QuickOpenQuery::WorkspaceSymbols`]. Arguments are trimmed.
///
/// This is the meaning with Vim off, following VS Code's
/// `gotoLineQuickAccess` (MIT; only its behaviour is referenced). Vim's `:`
/// command line uses its own parser.
pub fn parse_query(input: &str) -> QuickOpenQuery {
    if let Some(rest) = input.strip_prefix('@') {
        let (rest, group_by_kind) = match rest.strip_prefix(':') {
            Some(rest) => (rest, true),
            None => (rest, false),
        };
        QuickOpenQuery::Symbols {
            text: rest.trim().to_string(),
            group_by_kind,
        }
    } else if let Some(rest) = input.strip_prefix('#') {
        QuickOpenQuery::WorkspaceSymbols(rest.trim().to_string())
    } else if let Some(rest) = input.strip_prefix('>') {
        QuickOpenQuery::Commands(rest.trim().to_string())
    } else if let Some(rest) = input.strip_prefix(':') {
        let mut parts = rest.split([':', ',', '#']).map(str::trim);
        QuickOpenQuery::GotoLine {
            line: parts.next().and_then(|p| p.parse::<i64>().ok()),
            column: parts.next().and_then(|p| p.parse::<u32>().ok()),
        }
    } else {
        QuickOpenQuery::Files(input.trim().to_string())
    }
}

/// As [`parse_query`], but `:` gives [`QuickOpenQuery::VimCommand`]: the
/// meaning while Vim is on and the active editor takes its keys.
pub fn parse_query_vim(input: &str) -> QuickOpenQuery {
    match input.strip_prefix(':') {
        Some(rest) => QuickOpenQuery::VimCommand(rest.trim_start().to_string()),
        None => parse_query(input),
    }
}

/// The real 1-based line for a go-to-line `line` in a file of `line_count`
/// lines. A negative number counts from the end (`-1` is the last line);
/// the result is clamped into `[1, max(line_count, 1)]`.
pub fn resolve_line(line: i64, line_count: u32) -> u32 {
    let last = i64::from(line_count.max(1));
    let line = if line < 0 {
        last.saturating_add(1).saturating_add(line)
    } else {
        line
    };
    line.clamp(1, last) as u32
}
