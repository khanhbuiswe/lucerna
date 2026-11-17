use std::borrow::Cow;
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use encoding_rs::Encoding;
use ropey::Rope;

use crate::history::{History, HistoryState, Inverse, SelectionsToken, UndoResult, compose_all};
use crate::load::{Decoded, LoadError, check_regular, check_size, decode_file};
use crate::patch::{Patch, PatchEdit};

/// The dominant line ending of a file, for the status bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    CrLf,
}

/// A position in the text. `column` is a UTF-8 byte offset within the line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Point {
    pub row: u32,
    pub column: u32,
}

#[derive(Debug)]
pub struct Buffer {
    rope: Rope,
    encoding: &'static Encoding,
    line_ending: LineEnding,
    has_bom: bool,
    disk_hash: u64,
    lossy: bool,
    history: History,
    version: u64,
}

impl Buffer {
    pub fn load(path: &Path) -> Result<Buffer, LoadError> {
        let meta = std::fs::metadata(path)?;
        check_regular(&meta)?;
        check_size(meta.len())?;
        let bytes = std::fs::read(path)?;
        Buffer::from_bytes(&bytes, false)
    }

    pub fn from_bytes(bytes: &[u8], force_text: bool) -> Result<Buffer, LoadError> {
        Ok(Buffer::from_decoded(decode_file(bytes, force_text)?))
    }

    pub fn from_decoded(decoded: Decoded) -> Buffer {
        Buffer {
            rope: Rope::from_str(&decoded.text),
            encoding: decoded.encoding,
            line_ending: decoded.line_ending,
            has_bom: decoded.has_bom,
            disk_hash: decoded.hash,
            lossy: decoded.lossy,
            history: History::new(),
            version: 0,
        }
    }

    pub fn has_bom(&self) -> bool {
        self.has_bom
    }

    /// The hash of the bytes last loaded or saved.
    pub fn disk_hash(&self) -> u64 {
        self.disk_hash
    }

    pub fn set_line_ending(&mut self, line_ending: LineEnding) {
        self.line_ending = line_ending;
    }

    pub fn encoding(&self) -> &'static Encoding {
        self.encoding
    }

    pub fn line_ending(&self) -> LineEnding {
        self.line_ending
    }

    pub fn snapshot(&self) -> BufferSnapshot {
        BufferSnapshot {
            rope: self.rope.clone(),
        }
    }

    /// Takes the encoding, BOM, line ending and hash of a newer decode of
    /// the file, whose text the buffer was edited to match (a reload).
    pub fn set_disk_state(&mut self, decoded: &Decoded) {
        self.encoding = decoded.encoding;
        self.has_bom = decoded.has_bom;
        self.line_ending = decoded.line_ending;
        self.disk_hash = decoded.hash;
        self.lossy = decoded.lossy;
    }

    /// Some bytes of the file were not valid in its encoding and are U+FFFD
    /// in the text; saving would write the replacement character.
    pub fn is_lossy(&self) -> bool {
        self.lossy
    }

    pub fn clear_lossy(&mut self) {
        self.lossy = false;
    }

    /// Incremented by every edit, undo and redo.
    pub fn version(&self) -> u64 {
        self.version
    }

    pub fn history_state(&self) -> HistoryState {
        self.history.state()
    }

    pub fn set_saved(&mut self, state: HistoryState) {
        self.history.set_saved(state);
    }

    /// No history state counts as saved: dirty until the next `set_saved`.
    pub fn clear_saved(&mut self) {
        self.history.clear_saved();
    }

    pub fn is_dirty(&self) -> bool {
        self.history.is_dirty()
    }

    /// Whether an undone step is waiting to be redone. Any edit drops it.
    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    /// Closes the open undo group, then captures what to write and the
    /// history state that becomes clean once the write succeeds.
    pub fn begin_save(&mut self) -> (BufferSnapshot, HistoryState) {
        self.push_stop();
        (self.snapshot(), self.history_state())
    }

    pub fn start_transaction(&mut self, selections_before: Option<SelectionsToken>) {
        self.history.start(selections_before);
    }

    pub fn end_transaction(
        &mut self,
        selections_after: Option<SelectionsToken>,
    ) -> Option<crate::history::TransactionId> {
        self.history.end(selections_after)
    }

    /// Closes the top transaction: the next edit starts a new undo step.
    pub fn push_stop(&mut self) {
        self.history.push_stop();
    }

    /// Applies `edits`, whose sorted, non-overlapping ranges are against the
    /// current text. `\r\n` in inserted text becomes `\n`. Outside a
    /// transaction the edit is its own (appendable) transaction. Ranges must
    /// lie on char boundaries.
    pub fn edit<I: IntoIterator<Item = (Range<usize>, Arc<str>)>>(&mut self, edits: I) -> Patch {
        let edits: Vec<(Range<usize>, Arc<str>)> = edits
            .into_iter()
            .map(|(r, t)| {
                let t = if t.contains("\r\n") {
                    Arc::from(t.replace("\r\n", "\n"))
                } else {
                    t
                };
                (r, t)
            })
            .filter(|(r, t)| !(r.is_empty() && t.is_empty()))
            .collect();
        if edits.is_empty() {
            return Patch::default();
        }
        debug_assert!(
            edits.windows(2).all(|w| w[0].0.end <= w[1].0.start),
            "edits must be sorted and non-overlapping"
        );
        let standalone = !self.history.in_transaction();
        if standalone {
            self.history.start(None);
        }
        let mut patch = Vec::with_capacity(edits.len());
        let mut delta = 0isize;
        for (r, t) in &edits {
            let start = (r.start as isize + delta) as usize;
            patch.push(PatchEdit {
                old: r.clone(),
                new: start..start + t.len(),
            });
            delta += t.len() as isize - r.len() as isize;
        }
        let mut inverses = Vec::with_capacity(edits.len());
        for (r, t) in edits.into_iter().rev() {
            let old_text = splice(&mut self.rope, r.clone(), &t);
            inverses.push(Inverse {
                range_after: r.start..r.start + t.len(),
                old_text,
                range_before: r,
                new_text: t,
            });
        }
        self.history.record(inverses);
        self.version += 1;
        if standalone {
            self.history.end(None);
        }
        Patch::new(patch)
    }

    pub fn undo(&mut self) -> Option<UndoResult> {
        if self.history.in_transaction() {
            return None;
        }
        self.push_stop();
        let t = self.history.pop_undo()?;
        let mut patches = Vec::with_capacity(t.edits.len());
        for inv in t.edits.iter().rev() {
            splice(&mut self.rope, inv.range_after.clone(), &inv.old_text);
            patches.push(Patch::new(vec![inv.undo_patch()]));
        }
        let selections = t.selections_before.clone();
        self.history.push_redo(t);
        self.version += 1;
        Some(UndoResult {
            patch: compose_all(patches),
            selections,
        })
    }

    pub fn redo(&mut self) -> Option<UndoResult> {
        if self.history.in_transaction() {
            return None;
        }
        self.push_stop();
        let t = self.history.pop_redo()?;
        let mut patches = Vec::with_capacity(t.edits.len());
        for inv in &t.edits {
            splice(&mut self.rope, inv.range_before.clone(), &inv.new_text);
            patches.push(Patch::new(vec![inv.redo_patch()]));
        }
        let selections = t.selections_after.clone();
        self.history.push_undo(t);
        self.version += 1;
        Some(UndoResult {
            patch: compose_all(patches),
            selections,
        })
    }

    pub fn is_large(&self, max_bytes: u64, max_lines: u32) -> bool {
        self.rope.len_bytes() as u64 > max_bytes || self.snapshot().line_count() > max_lines
    }
}

/// Replaces the byte `range` with `text`, returning the removed text.
fn splice(rope: &mut Rope, range: Range<usize>, text: &str) -> Arc<str> {
    let old: Arc<str> = Arc::from(rope.byte_slice(range.clone()).to_string());
    let start = rope.byte_to_char(range.start);
    let end = rope.byte_to_char(range.end);
    if start < end {
        rope.remove(start..end);
    }
    if !text.is_empty() {
        rope.insert(start, text);
    }
    old
}

/// An immutable, cheap-to-clone view of the text (`Send + Sync`).
#[derive(Debug, Clone)]
pub struct BufferSnapshot {
    rope: Rope,
}

impl BufferSnapshot {
    /// `text` must already be LF-normalised.
    pub fn from_text(text: &str) -> BufferSnapshot {
        BufferSnapshot {
            rope: Rope::from_str(text),
        }
    }

    /// The whole text (LF line endings).
    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    /// Content equality without copying the text.
    pub fn text_eq(&self, other: &BufferSnapshot) -> bool {
        self.rope == other.rope
    }

    /// An empty file has one line; a trailing newline does not add one.
    pub fn line_count(&self) -> u32 {
        let lines = self.rope.len_lines();
        let ends_with_newline =
            self.rope.len_bytes() > 0 && self.rope.byte(self.rope.len_bytes() - 1) == b'\n';
        (if ends_with_newline { lines - 1 } else { lines }) as u32
    }

    /// The line without its `\n` or `\r\n`. Out-of-range rows give "".
    pub fn line(&self, row: u32) -> Cow<'_, str> {
        let row = row as usize;
        if row >= self.line_count() as usize {
            return Cow::Borrowed("");
        }
        let text: Cow<'_, str> = self.rope.line(row).into();
        match text {
            Cow::Borrowed(s) => Cow::Borrowed(trim_eol(s)),
            Cow::Owned(mut s) => {
                s.truncate(trim_eol(&s).len());
                Cow::Owned(s)
            }
        }
    }

    /// At most the first `max_chars` chars of line `row`, without its line
    /// break. Unlike [`Self::line`] it never copies the rest of a long
    /// line. Out-of-range rows give "".
    pub fn line_prefix(&self, row: u32, max_chars: usize) -> String {
        if row >= self.line_count() {
            return String::new();
        }
        self.rope
            .line(row as usize)
            .chars()
            .take_while(|c| !matches!(c, '\n' | '\r'))
            .take(max_chars)
            .collect()
    }

    /// The number of chars before `p` on its line: the column the status
    /// bar shows, 0-based. A byte column inside a char counts that char's
    /// start; past the end of the line it clamps to the line's length
    /// (without the line break). Out-of-range rows give 0. O(log n): the
    /// line is not copied.
    pub fn char_column(&self, p: Point) -> u32 {
        let row = p.row as usize;
        if row >= self.line_count() as usize {
            return 0;
        }
        let line = self.rope.line(row);
        let mut len = line.len_bytes();
        for eol in *b"\n\r" {
            if len > 0 && line.byte(len - 1) == eol {
                len -= 1;
            }
        }
        // `byte_to_char` names the char containing the byte: rounds down.
        line.byte_to_char((p.column as usize).min(len)) as u32
    }

    pub fn len_bytes(&self) -> usize {
        self.rope.len_bytes()
    }

    /// The byte at `offset`, which must be below [`Self::len_bytes`].
    pub fn byte_at(&self, offset: usize) -> u8 {
        self.rope.byte(offset)
    }

    /// The largest char boundary at or below `offset` (clamped to the end).
    pub fn floor_char_boundary(&self, offset: usize) -> usize {
        let mut offset = offset.min(self.rope.len_bytes());
        // Continuation bytes are 0b10xx_xxxx; the end is always a boundary.
        while offset > 0 && offset < self.rope.len_bytes() && self.rope.byte(offset) & 0xC0 == 0x80
        {
            offset -= 1;
        }
        offset
    }

    pub fn point_to_offset(&self, p: Point) -> usize {
        let row = (p.row as usize).min(self.rope.len_lines() - 1);
        (self.rope.line_to_byte(row) + p.column as usize).min(self.rope.len_bytes())
    }

    pub fn offset_to_point(&self, offset: usize) -> Point {
        let offset = offset.min(self.rope.len_bytes());
        let row = self.rope.byte_to_line(offset);
        Point {
            row: row as u32,
            column: (offset - self.rope.line_to_byte(row)) as u32,
        }
    }

    /// The UTF-16 offset of byte `offset` (moved back to a char boundary,
    /// clamped to the end), as platform text input APIs count. O(log n).
    pub fn offset_to_utf16(&self, offset: usize) -> usize {
        let offset = self.floor_char_boundary(offset);
        self.rope.char_to_utf16_cu(self.rope.byte_to_char(offset))
    }

    /// The byte offset of UTF-16 offset `utf16` (clamped to the end); an
    /// offset inside a surrogate pair gives the start of its char.
    pub fn utf16_to_offset(&self, utf16: usize) -> usize {
        let utf16 = utf16.min(self.rope.len_utf16_cu());
        self.rope.char_to_byte(self.rope.utf16_cu_to_char(utf16))
    }

    pub fn text_for_range(&self, r: Range<usize>) -> String {
        self.rope.byte_slice(r).to_string()
    }

    /// Chunks of the range, in order; feeds tree-sitter.
    pub fn chunks_in_range(&self, r: Range<usize>) -> impl Iterator<Item = &str> {
        self.rope.byte_slice(r).chunks()
    }
}

fn trim_eol(s: &str) -> &str {
    let s = s.strip_suffix('\n').unwrap_or(s);
    s.strip_suffix('\r').unwrap_or(s)
}

const _: fn() = || {
    fn assert_send_sync<T: Send + Sync + Clone>() {}
    assert_send_sync::<BufferSnapshot>();
};

#[cfg(test)]
mod tests {
    use super::*;
    use encoding_rs::UTF_16LE;

    fn all_text(b: &Buffer) -> String {
        let s = b.snapshot();
        s.text_for_range(0..s.len_bytes())
    }

    #[test]
    fn line_prefix_is_bounded_and_without_eol() {
        let long = "x".repeat(2_000_000);
        let s = BufferSnapshot::from_text(&format!("ab\r\n{long}\nlast"));
        assert_eq!(s.line_prefix(0, 10), "ab");
        assert_eq!(s.line_prefix(1, 5), "xxxxx");
        assert_eq!(s.line_prefix(1, 10_000_000).len(), 2_000_000);
        assert_eq!(s.line_prefix(2, 10), "last");
        assert_eq!(s.line_prefix(9, 10), "");
        assert_eq!(BufferSnapshot::from_text("héllo").line_prefix(0, 2), "hé");
    }

    #[test]
    fn from_text_round_trips() {
        let s = BufferSnapshot::from_text("a\nb");
        assert_eq!(s.text_for_range(0..3), "a\nb");
        assert_eq!(s.line_count(), 2);
    }

    #[test]
    fn text_eq_compares_content() {
        let a = BufferSnapshot::from_text("a\nb");
        assert!(a.text_eq(&BufferSnapshot::from_text("a\nb")));
        assert!(!a.text_eq(&BufferSnapshot::from_text("a\nc")));
        assert!(!a.text_eq(&BufferSnapshot::from_text("a\n")));
    }

    #[test]
    fn utf16_offsets_round_trip() {
        // `é` is 2 bytes / 1 unit, `😀` 4 bytes / 2 units.
        let s = Buffer::from_bytes("aé😀\nb".as_bytes(), false)
            .unwrap()
            .snapshot();
        assert_eq!(s.offset_to_utf16(3), 2);
        assert_eq!(s.offset_to_utf16(7), 4);
        assert_eq!(s.offset_to_utf16(5), 2, "inside a char snaps back");
        assert_eq!(s.utf16_to_offset(4), 7);
        assert_eq!(s.utf16_to_offset(3), 3, "inside a pair snaps back");
        assert_eq!(s.utf16_to_offset(99), s.len_bytes());
    }

    #[test]
    fn floor_char_boundary_snaps_down() {
        let b = Buffer::from_bytes("aé€".as_bytes(), false).unwrap();
        let s = b.snapshot();
        let got: Vec<usize> = (0..=7).map(|i| s.floor_char_boundary(i)).collect();
        assert_eq!(got, [0, 1, 1, 3, 3, 3, 6, 6]);
    }

    #[test]
    fn crlf_is_normalised_and_remembered() {
        let b = Buffer::from_bytes(b"a\r\nb\r\n", false).unwrap();
        assert_eq!(all_text(&b), "a\nb\n");
        assert_eq!(b.line_ending(), LineEnding::CrLf);
        assert_eq!(b.snapshot().line(0), "a");
    }

    #[test]
    fn lone_cr_is_kept() {
        let b = Buffer::from_bytes(b"a\rb\n", false).unwrap();
        assert_eq!(all_text(&b), "a\rb\n");
    }

    #[test]
    fn mixed_endings_pick_majority() {
        let b = Buffer::from_bytes(b"a\r\nb\r\nc\n", false).unwrap();
        assert_eq!(b.line_ending(), LineEnding::CrLf);
        assert_eq!(all_text(&b), "a\nb\nc\n");
    }

    #[test]
    fn bom_is_recorded() {
        let b = Buffer::from_bytes(b"\xEF\xBB\xBFx", false).unwrap();
        assert!(b.has_bom());
        assert_eq!(all_text(&b), "x");
        let b = Buffer::from_bytes(b"\xFF\xFEx\x00", false).unwrap();
        assert!(b.has_bom());
        assert_eq!(b.encoding(), UTF_16LE);
        assert!(!Buffer::from_bytes(b"x", false).unwrap().has_bom());
    }

    #[test]
    fn hash_matches_bytes() {
        let a = Buffer::from_bytes(b"same", false).unwrap();
        let b = Buffer::from_bytes(b"same", false).unwrap();
        let c = Buffer::from_bytes(b"other", false).unwrap();
        assert_eq!(a.disk_hash(), b.disk_hash());
        assert_ne!(a.disk_hash(), c.disk_hash());
    }

    #[test]
    fn set_line_ending_overrides() {
        let mut b = Buffer::from_bytes(b"a\n", false).unwrap();
        b.set_line_ending(LineEnding::CrLf);
        assert_eq!(b.line_ending(), LineEnding::CrLf);
    }
}
