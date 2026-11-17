//! Untitled documents: unsaved buffers keyed by a fake relative path
//! `untitled:Untitled-N`, so they live in the path-keyed documents, tabs and
//! hot exit like any file. `is_untitled` guards every disk or external touch.

use std::path::Path;
use std::sync::Arc;

pub const PREFIX: &str = "untitled:";

const MAX_LABEL_CHARS: usize = 40;

/// True for a relative path of exactly one component starting with `PREFIX`.
pub fn is_untitled(path: &Path) -> bool {
    if path.is_absolute() {
        return false;
    }
    let mut components = path.components();
    match (components.next(), components.next()) {
        (Some(std::path::Component::Normal(name)), None) => {
            name.to_str().is_some_and(|n| n.starts_with(PREFIX))
        }
        _ => false,
    }
}

pub fn path(n: u32) -> Arc<Path> {
    Arc::from(Path::new(&format!("{PREFIX}Untitled-{n}")))
}

pub fn number(path: &Path) -> Option<u32> {
    if !is_untitled(path) {
        return None;
    }
    path.to_str()?
        .strip_prefix(PREFIX)?
        .strip_prefix("Untitled-")?
        .parse()
        .ok()
}

/// `Untitled-N`: the part after `PREFIX`.
pub fn name(path: &Path) -> String {
    let text = path.to_string_lossy();
    text.strip_prefix(PREFIX).unwrap_or(&text).to_string()
}

/// The smallest number from 1 not in `used`.
pub fn next_number(used: impl IntoIterator<Item = u32>) -> u32 {
    let used: std::collections::HashSet<u32> = used.into_iter().collect();
    (1..).find(|n| !used.contains(n)).unwrap_or(1)
}

/// Tab label: the first non-blank line, trimmed, at most 40 chars (then `…`).
pub fn label(text_first_lines: &str) -> Option<String> {
    let line = text_first_lines
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())?;
    if line.chars().count() > MAX_LABEL_CHARS {
        let cut: String = line.chars().take(MAX_LABEL_CHARS).collect();
        Some(format!("{cut}…"))
    } else {
        Some(line.to_string())
    }
}

/// Save-dialog file name from a tab label: forbidden and control characters
/// dropped, trimmed; `None` when nothing is left.
pub fn suggested_file_name(label: &str) -> Option<String> {
    let cleaned: String = label
        .chars()
        .filter(|c| {
            !c.is_control() && !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
        .collect();
    let cleaned = cleaned.trim();
    (!cleaned.is_empty()).then(|| cleaned.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_only_relative_single_component() {
        assert!(is_untitled(Path::new("untitled:Untitled-1")));
        assert!(!is_untitled(Path::new("/tmp/untitled:Untitled-1")));
        assert!(!is_untitled(Path::new("dir/untitled:x")));
        assert!(!is_untitled(Path::new("src/main.rs")));
    }

    #[test]
    fn numbers() {
        assert_eq!(&*path(3), Path::new("untitled:Untitled-3"));
        assert_eq!(number(&path(3)), Some(3));
        assert_eq!(name(&path(3)), "Untitled-3");
        assert_eq!(next_number([]), 1);
        assert_eq!(next_number([1, 3]), 2);
        assert_eq!(next_number([2, 1]), 3);
    }

    #[test]
    fn labels() {
        assert_eq!(label(""), None);
        assert_eq!(label("\n   \n"), None);
        assert_eq!(
            label("\n  fn main() {}\nx").as_deref(),
            Some("fn main() {}")
        );
        let long = "a".repeat(50);
        assert_eq!(label(&long).unwrap(), format!("{}…", "a".repeat(40)));
        let vi = "ệ".repeat(41);
        assert_eq!(label(&vi).unwrap(), format!("{}…", "ệ".repeat(40)));
    }

    #[test]
    fn suggested_names() {
        assert_eq!(suggested_file_name("a/b:c*?.rs").as_deref(), Some("abc.rs"));
        assert_eq!(suggested_file_name(" <>|\"\\ "), None);
    }
}
