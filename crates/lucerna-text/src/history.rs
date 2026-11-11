//! Transaction history for undo and redo. The grouping policy (when to stop
//! a group) belongs to the caller; this only knows explicit stops, nesting
//! depth and open/closed transactions.

use std::any::Any;
use std::ops::Range;
use std::sync::Arc;

use crate::patch::{Patch, PatchEdit};

/// Opaque caller-owned selections stored with a transaction.
pub type SelectionsToken = Arc<dyn Any + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransactionId(u64);

/// The id on top of the undo stack. Equal states mean equal text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryState(Option<TransactionId>);

pub struct UndoResult {
    pub patch: Patch,
    pub selections: Option<SelectionsToken>,
}

/// One applied edit with what is needed to undo and redo it. Ranges are in
/// the frame at the time the edit was applied (a batch is applied back to
/// front, so earlier offsets are unaffected by later ones).
#[derive(Debug)]
pub(crate) struct Inverse {
    pub range_after: Range<usize>,
    pub old_text: Arc<str>,
    pub range_before: Range<usize>,
    pub new_text: Arc<str>,
}

impl Inverse {
    pub fn undo_patch(&self) -> PatchEdit {
        PatchEdit {
            old: self.range_after.clone(),
            new: self.range_before.clone(),
        }
    }

    pub fn redo_patch(&self) -> PatchEdit {
        PatchEdit {
            old: self.range_before.clone(),
            new: self.range_after.clone(),
        }
    }
}

pub(crate) struct Transaction {
    id: TransactionId,
    pub edits: Vec<Inverse>,
    pub selections_before: Option<SelectionsToken>,
    pub selections_after: Option<SelectionsToken>,
    pub open: bool,
}

impl std::fmt::Debug for Transaction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Transaction")
            .field("id", &self.id)
            .field("edits", &self.edits.len())
            .field("open", &self.open)
            .finish()
    }
}

#[derive(Debug)]
pub(crate) struct History {
    undo: Vec<Transaction>,
    redo: Vec<Transaction>,
    depth: usize,
    /// The top transaction was pushed by the current outermost
    /// `start_transaction` (so it is dropped if it stays empty).
    top_is_new: bool,
    next_id: u64,
    saved: HistoryState,
}

impl History {
    pub fn new() -> History {
        History {
            undo: Vec::new(),
            redo: Vec::new(),
            depth: 0,
            top_is_new: false,
            next_id: 0,
            saved: HistoryState(None),
        }
    }

    pub fn in_transaction(&self) -> bool {
        self.depth > 0
    }

    pub fn start(&mut self, selections_before: Option<SelectionsToken>) {
        self.depth += 1;
        if self.depth > 1 {
            return;
        }
        if self.undo.last().is_some_and(|t| t.open) {
            self.top_is_new = false;
            return;
        }
        let id = TransactionId(self.next_id);
        self.next_id += 1;
        self.undo.push(Transaction {
            id,
            edits: Vec::new(),
            selections_before,
            selections_after: None,
            open: true,
        });
        self.top_is_new = true;
    }

    pub fn end(&mut self, selections_after: Option<SelectionsToken>) -> Option<TransactionId> {
        if self.depth == 0 {
            return None;
        }
        self.depth -= 1;
        if self.depth > 0 {
            return None;
        }
        if self.top_is_new && self.undo.last().is_some_and(|t| t.edits.is_empty()) {
            self.undo.pop();
            self.top_is_new = false;
            return None;
        }
        self.top_is_new = false;
        let top = self.undo.last_mut()?;
        if selections_after.is_some() {
            top.selections_after = selections_after;
        }
        Some(top.id)
    }

    pub fn push_stop(&mut self) {
        if let Some(top) = self.undo.last_mut() {
            top.open = false;
        }
    }

    /// Appends applied edits to the open top transaction and drops redo.
    pub fn record(&mut self, edits: Vec<Inverse>) {
        self.redo.clear();
        self.undo
            .last_mut()
            .expect("record requires an open transaction")
            .edits
            .extend(edits);
    }

    pub fn pop_undo(&mut self) -> Option<Transaction> {
        self.undo.pop()
    }

    pub fn pop_redo(&mut self) -> Option<Transaction> {
        self.redo.pop()
    }

    pub fn push_redo(&mut self, mut t: Transaction) {
        t.open = false;
        self.redo.push(t);
    }

    pub fn push_undo(&mut self, mut t: Transaction) {
        t.open = false;
        self.undo.push(t);
    }

    pub fn state(&self) -> HistoryState {
        let mut top = self.undo.last();
        if self.top_is_new
            && self.depth > 0
            && let Some(t) = top
            && t.edits.is_empty()
        {
            top = self
                .undo
                .len()
                .checked_sub(2)
                .and_then(|i| self.undo.get(i));
        }
        HistoryState(top.map(|t| t.id))
    }

    pub fn set_saved(&mut self, state: HistoryState) {
        self.saved = state;
    }

    pub fn is_dirty(&self) -> bool {
        self.state() != self.saved
    }
}

/// Composes patches applied in order, pairwise, so n patches cost
/// O(n log n) rather than O(n^2).
pub(crate) fn compose_all(mut patches: Vec<Patch>) -> Patch {
    if patches.is_empty() {
        return Patch::default();
    }
    while patches.len() > 1 {
        patches = patches
            .chunks(2)
            .map(|pair| match pair {
                [a, b] => a.compose(b),
                [a] => a.clone(),
                _ => unreachable!(),
            })
            .collect();
    }
    patches.pop().unwrap()
}

#[cfg(test)]
mod tests {
    use crate::{Bias, Buffer};
    use std::ops::Range;
    use std::sync::Arc;

    fn buf(s: &str) -> Buffer {
        Buffer::from_bytes(s.as_bytes(), false).unwrap()
    }

    fn text(b: &Buffer) -> String {
        let s = b.snapshot();
        s.text_for_range(0..s.len_bytes())
    }

    fn ed(b: &mut Buffer, r: Range<usize>, t: &str) {
        b.edit([(r, Arc::<str>::from(t))]);
    }

    #[test]
    fn edit_applies_back_to_front() {
        let mut b = buf("abcdef");
        let p = b.edit([
            (1..2, Arc::<str>::from("X")),
            (4..4, Arc::<str>::from("YY")),
        ]);
        assert_eq!(text(&b), "aXcdYYef");
        let new: Vec<_> = p.edits().iter().map(|e| e.new.clone()).collect();
        assert_eq!(new, vec![1..2, 4..6]);
    }

    #[test]
    fn undo_redo_round_trip() {
        let mut b = buf("abc");
        ed(&mut b, 3..3, "d");
        b.push_stop();
        ed(&mut b, 0..0, "ZZ");
        assert_eq!(text(&b), "ZZabcd");
        let r = b.undo().unwrap();
        assert_eq!(text(&b), "abcd");
        assert_eq!(r.patch.map_offset(2, Bias::Left), 0);
        assert_eq!(r.patch.map_offset(4, Bias::Left), 2);
        let r = b.undo().unwrap();
        assert_eq!(text(&b), "abc");
        assert_eq!(r.patch.map_offset(3, Bias::Left), 3);
        assert!(b.undo().is_none());
        let r = b.redo().unwrap();
        assert_eq!(text(&b), "abcd");
        assert_eq!(r.patch.map_offset(3, Bias::Right), 4);
        let r = b.redo().unwrap();
        assert_eq!(text(&b), "ZZabcd");
        assert_eq!(r.patch.map_offset(0, Bias::Right), 2);
        assert!(b.redo().is_none());
    }

    #[test]
    fn multi_edit_batch_undoes_and_redoes() {
        let mut b = buf("abcdef");
        b.edit([
            (1..2, Arc::<str>::from("X")),
            (4..4, Arc::<str>::from("YY")),
        ]);
        let r = b.undo().unwrap();
        assert_eq!(text(&b), "abcdef");
        assert_eq!(r.patch.map_offset(6, Bias::Left), 4);
        let r = b.redo().unwrap();
        assert_eq!(text(&b), "aXcdYYef");
        assert_eq!(r.patch.map_offset(5, Bias::Left), 7);
    }

    #[test]
    fn open_transaction_appends() {
        let mut b = buf("");
        b.start_transaction(None);
        ed(&mut b, 0..0, "a");
        b.end_transaction(None);
        b.start_transaction(None);
        ed(&mut b, 1..1, "b");
        b.end_transaction(None);
        assert_eq!(text(&b), "ab");
        b.undo().unwrap();
        assert_eq!(text(&b), "");
        assert!(b.undo().is_none());
    }

    #[test]
    fn stop_separates_transactions() {
        let mut b = buf("");
        ed(&mut b, 0..0, "a");
        b.push_stop();
        ed(&mut b, 1..1, "b");
        b.undo().unwrap();
        assert_eq!(text(&b), "a");
    }

    #[test]
    fn nested_transactions_join() {
        let mut b = buf("");
        b.start_transaction(None);
        b.start_transaction(None);
        ed(&mut b, 0..0, "a");
        assert!(b.end_transaction(None).is_none());
        ed(&mut b, 1..1, "b");
        assert!(b.end_transaction(None).is_some());
        b.push_stop();
        b.undo().unwrap();
        assert_eq!(text(&b), "");
    }

    #[test]
    fn undo_redo_are_refused_inside_a_transaction() {
        let mut b = buf("");
        ed(&mut b, 0..0, "a");
        b.push_stop();
        b.start_transaction(None);
        let v = b.version();
        assert!(b.undo().is_none());
        assert!(b.redo().is_none());
        assert_eq!(b.version(), v);
        ed(&mut b, 1..1, "b");
        b.end_transaction(None);
        assert_eq!(text(&b), "ab");
        assert!(b.is_dirty());
        b.undo().unwrap();
        assert_eq!(text(&b), "a");
        b.undo().unwrap();
        assert_eq!(text(&b), "");
        assert!(!b.is_dirty());
    }

    #[test]
    fn standalone_edit_keeps_selections_after() {
        let mut b = buf("");
        b.start_transaction(None);
        ed(&mut b, 0..0, "a");
        b.end_transaction(Some(Arc::new(7u32)));
        ed(&mut b, 1..1, "b");
        b.undo().unwrap();
        let r = b.redo().unwrap();
        assert_eq!(*r.selections.unwrap().downcast::<u32>().unwrap(), 7);
    }

    #[test]
    fn empty_transaction_is_dropped() {
        let mut b = buf("x");
        b.start_transaction(None);
        assert!(!b.is_dirty());
        assert!(b.end_transaction(None).is_none());
        assert!(b.undo().is_none());
    }

    #[test]
    fn selections_tokens_returned() {
        let mut b = buf("");
        b.start_transaction(Some(Arc::new(1u32)));
        ed(&mut b, 0..0, "a");
        b.end_transaction(Some(Arc::new(2u32)));
        b.start_transaction(Some(Arc::new(99u32)));
        ed(&mut b, 1..1, "b");
        b.end_transaction(Some(Arc::new(3u32)));
        let r = b.undo().unwrap();
        assert_eq!(*r.selections.unwrap().downcast::<u32>().unwrap(), 1);
        let r = b.redo().unwrap();
        assert_eq!(*r.selections.unwrap().downcast::<u32>().unwrap(), 3);
    }

    #[test]
    fn dirty_tracks_history_state() {
        let mut b = buf("a");
        assert!(!b.is_dirty());
        ed(&mut b, 1..1, "b");
        assert!(b.is_dirty());
        let (_, s) = b.begin_save();
        b.set_saved(s);
        assert!(!b.is_dirty());
        ed(&mut b, 2..2, "c");
        assert!(b.is_dirty());
        b.undo().unwrap();
        assert!(!b.is_dirty());
        b.redo().unwrap();
        assert!(b.is_dirty());
    }

    #[test]
    fn edit_during_save_stays_dirty() {
        let mut b = buf("a");
        ed(&mut b, 1..1, "b");
        let (snap, s) = b.begin_save();
        ed(&mut b, 2..2, "c");
        b.set_saved(s);
        assert_eq!(snap.len_bytes(), 2);
        assert!(b.is_dirty());
    }

    #[test]
    fn new_edit_after_undo_drops_redo() {
        let mut b = buf("a");
        ed(&mut b, 1..1, "b");
        b.undo().unwrap();
        ed(&mut b, 1..1, "c");
        assert!(b.redo().is_none());
        assert_eq!(text(&b), "ac");
    }

    #[test]
    fn version_increases_on_every_change() {
        let mut b = buf("a");
        let v0 = b.version();
        ed(&mut b, 1..1, "b");
        let v1 = b.version();
        b.undo().unwrap();
        let v2 = b.version();
        b.redo().unwrap();
        assert!(v0 < v1 && v1 < v2 && v2 < b.version());
    }

    #[test]
    fn crlf_in_inserted_text_is_normalised() {
        let mut b = buf("");
        ed(&mut b, 0..0, "a\r\nb\r\n");
        assert_eq!(text(&b), "a\nb\n");
        b.undo().unwrap();
        assert_eq!(text(&b), "");
        b.redo().unwrap();
        assert_eq!(text(&b), "a\nb\n");
    }
}
