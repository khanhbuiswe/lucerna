//! The output of an editing command: edits plus the resulting selections.

use std::ops::Range;
use std::sync::Arc;

use lucerna_syntax::{LanguageEditing, SyntaxState};
use lucerna_text::BufferSnapshot;

use crate::selection::SelectionSet;
use crate::undo_stops::EditKind;

/// What a command wants to do to the buffer. Edits are in pre-edit
/// coordinates; `selections` are in post-edit coordinates.
#[derive(Debug, Clone)]
pub struct EditPlan {
    pub edits: Vec<(Range<usize>, Arc<str>)>,
    pub selections: SelectionSet,
    pub kind: EditKind,
    pub stop_before: bool,
    pub stop_after: bool,
}

#[derive(Debug, Clone)]
pub struct EditConfig {
    pub tab_size: u32,
    pub insert_spaces: bool,
    pub auto_indent: AutoIndent,
    pub auto_closing_brackets: AutoClosing,
    pub auto_closing_quotes: AutoClosing,
    pub auto_closing_overtype: AutoClosingEdit,
    pub auto_closing_delete: AutoClosingEdit,
    pub auto_surround: AutoSurround,
    pub trim_auto_whitespace: bool,
    pub word_separators: Arc<str>,
    pub empty_selection_clipboard: bool,
}

#[derive(Clone, Copy)]
pub struct Ctx<'a> {
    pub snapshot: &'a BufferSnapshot,
    pub config: &'a EditConfig,
    /// The document language's brackets, comments and indent rules.
    pub language: Option<&'a LanguageEditing>,
    /// The document's syntax tree, for the scope at an offset.
    pub syntax: Option<&'a SyntaxState>,
}

macro_rules! setting_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident = $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
        pub enum $name {
            $($variant),+
        }

        impl $name {
            /// The setting's value, as written in the settings file.
            pub fn parse(value: &str) -> Option<Self> {
                match value {
                    $($text => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }
    };
}

setting_enum! {
    /// `editor.autoIndent`, in order of how much it does.
    AutoIndent {
        None = "none",
        Keep = "keep",
        Brackets = "brackets",
        Advanced = "advanced",
        Full = "full",
    }
}

setting_enum! {
    /// `editor.autoClosingBrackets` and `editor.autoClosingQuotes`.
    AutoClosing {
        Always = "always",
        LanguageDefined = "languageDefined",
        BeforeWhitespace = "beforeWhitespace",
        Never = "never",
    }
}

setting_enum! {
    /// `editor.autoClosingOvertype` and `editor.autoClosingDelete`.
    AutoClosingEdit {
        Always = "always",
        Auto = "auto",
        Never = "never",
    }
}

setting_enum! {
    /// `editor.autoSurround`.
    AutoSurround {
        LanguageDefined = "languageDefined",
        Quotes = "quotes",
        Brackets = "brackets",
        Never = "never",
    }
}
