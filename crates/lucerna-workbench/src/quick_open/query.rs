//! What the Quick Open input asks for, decided by its first character.

/// The mode and argument of a Quick Open input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuickOpenQuery {
    /// No prefix: fuzzy-find files.
    Files(String),
    /// `>`: filter commands.
    Commands(String),
    /// `:`: go to a 1-based line and optional 1-based column.
    GotoLine {
        line: Option<u32>,
        column: Option<u32>,
    },
}

/// Parses the Quick Open input. A leading `>` gives
/// [`QuickOpenQuery::Commands`] and a leading `:` gives
/// [`QuickOpenQuery::GotoLine`] (`":12:5"` is line 12, column 5; parts
/// that are not numbers are `None`). Arguments are trimmed.
pub fn parse_query(input: &str) -> QuickOpenQuery {
    if let Some(rest) = input.strip_prefix('>') {
        QuickOpenQuery::Commands(rest.trim().to_string())
    } else if let Some(rest) = input.strip_prefix(':') {
        let mut parts = rest.splitn(2, ':').map(|p| p.trim().parse::<u32>().ok());
        QuickOpenQuery::GotoLine {
            line: parts.next().flatten(),
            column: parts.next().flatten(),
        }
    } else {
        QuickOpenQuery::Files(input.trim().to_string())
    }
}
