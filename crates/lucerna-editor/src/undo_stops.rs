//! When consecutive edits form one undo step (spec §4.2).

/// The kind of the last edit, used to decide where undo stops go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditKind {
    Other,
    DeletingLeft,
    DeletingRight,
    TypingOther,
    TypingFirstSpace,
    TypingConsecutiveSpace,
}

impl EditKind {
    fn is_typing(self) -> bool {
        matches!(
            self,
            EditKind::TypingOther | EditKind::TypingFirstSpace | EditKind::TypingConsecutiveSpace
        )
    }

    /// Both space kinds count as one kind when comparing.
    fn normalized(self) -> EditKind {
        match self {
            EditKind::TypingFirstSpace => EditKind::TypingConsecutiveSpace,
            other => other,
        }
    }
}

/// Whether an undo stop belongs between an edit of kind `prev` and one of
/// kind `cur`.
// Adapted from microsoft/vscode src/vs/editor/common/cursor/cursorTypeEditOperations.ts (MIT)
pub fn should_stop_between(prev: EditKind, cur: EditKind) -> bool {
    if prev.is_typing() && !cur.is_typing() {
        // Always stop before non-typing operations.
        return true;
    }
    if prev == EditKind::TypingFirstSpace {
        // `abc |d`: no stop; `abc  |d`: stop.
        return false;
    }
    prev.normalized() != cur.normalized()
}

/// The kind of a typed `text` after an edit of kind `prev`.
// Adapted from microsoft/vscode src/vs/editor/common/cursor/cursorTypeEditOperations.ts (MIT)
pub fn typing_kind(text: &str, prev: EditKind) -> EditKind {
    if text == " " {
        if matches!(
            prev,
            EditKind::TypingFirstSpace | EditKind::TypingConsecutiveSpace
        ) {
            EditKind::TypingConsecutiveSpace
        } else {
            EditKind::TypingFirstSpace
        }
    } else {
        EditKind::TypingOther
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use EditKind::*;

    fn groups(s: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut prev = Other;
        for ch in s.chars() {
            let t = ch.to_string();
            let kind = typing_kind(&t, prev);
            if out.is_empty() || should_stop_between(prev, kind) {
                out.push(String::new());
            }
            out.last_mut().unwrap().push(ch);
            prev = kind;
        }
        out
    }

    #[test]
    fn stop_rule_matches_vscode() {
        assert!(should_stop_between(TypingOther, TypingFirstSpace));
        assert!(!should_stop_between(TypingFirstSpace, TypingOther));
        assert!(!should_stop_between(
            TypingFirstSpace,
            TypingConsecutiveSpace
        ));
        assert!(should_stop_between(TypingConsecutiveSpace, TypingOther));
        assert!(should_stop_between(TypingOther, DeletingLeft));
        assert!(!should_stop_between(DeletingLeft, DeletingLeft));
        assert!(should_stop_between(DeletingLeft, DeletingRight));
        assert_eq!(groups("abc def"), ["abc", " def"]);
        assert_eq!(groups("abc  def"), ["abc", "  ", "def"]);
    }

    #[test]
    fn typing_kind_tracks_spaces() {
        assert_eq!(typing_kind(" ", TypingOther), TypingFirstSpace);
        assert_eq!(typing_kind(" ", TypingFirstSpace), TypingConsecutiveSpace);
        assert_eq!(
            typing_kind(" ", TypingConsecutiveSpace),
            TypingConsecutiveSpace
        );
        assert_eq!(typing_kind("a", TypingFirstSpace), TypingOther);
        assert_eq!(typing_kind("  ", Other), TypingOther);
    }
}
