use std::fmt;

/// A settings (or keymap) parse/validation error.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsError {
    pub message: String,
    /// 1-based line of the error, when known.
    pub line: Option<usize>,
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(f, "line {line}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for SettingsError {}
