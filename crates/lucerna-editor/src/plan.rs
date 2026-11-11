//! The output of an editing command: edits plus the resulting selections.

use std::ops::Range;
use std::sync::Arc;

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
    pub keep_indent: bool,
    pub trim_auto_whitespace: bool,
    pub word_separators: Arc<str>,
    pub empty_selection_clipboard: bool,
}

#[derive(Clone, Copy)]
pub struct Ctx<'a> {
    pub snapshot: &'a BufferSnapshot,
    pub config: &'a EditConfig,
}
