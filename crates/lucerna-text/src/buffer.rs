use std::borrow::Cow;
use std::ops::Range;
use std::path::Path;

use encoding_rs::Encoding;
use ropey::Rope;

use crate::load::{Decoded, LoadError, check_size, decode_file};
use crate::reload::Reload;

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
    force_text: bool,
}

impl Buffer {
    pub fn load(path: &Path) -> Result<Buffer, LoadError> {
        let len = std::fs::metadata(path)?.len();
        check_size(len)?;
        let bytes = std::fs::read(path)?;
        Buffer::from_bytes(&bytes, false)
    }

    pub fn from_bytes(bytes: &[u8], force_text: bool) -> Result<Buffer, LoadError> {
        let mut buffer = Buffer::from_decoded(decode_file(bytes, force_text)?);
        buffer.force_text = force_text;
        Ok(buffer)
    }

    pub fn from_decoded(decoded: Decoded) -> Buffer {
        Buffer {
            rope: Rope::from_str(&decoded.text),
            encoding: decoded.encoding,
            line_ending: decoded.line_ending,
            has_bom: decoded.has_bom,
            disk_hash: decoded.hash,
            force_text: false,
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

    /// Replaces the contents with `bytes` and describes what changed.
    /// On error the buffer is left untouched.
    pub fn reload(&mut self, bytes: &[u8]) -> Result<Reload, LoadError> {
        let decoded = decode_file(bytes, self.force_text)?;
        let new = BufferSnapshot {
            rope: Rope::from_str(&decoded.text),
        };
        let reload = Reload::diff(&self.snapshot(), &new);
        self.line_ending = decoded.line_ending;
        self.encoding = decoded.encoding;
        self.has_bom = decoded.has_bom;
        self.disk_hash = decoded.hash;
        self.rope = new.rope;
        Ok(reload)
    }

    pub fn is_large(&self, max_bytes: u64, max_lines: u32) -> bool {
        self.rope.len_bytes() as u64 > max_bytes || self.snapshot().line_count() > max_lines
    }
}

/// An immutable, cheap-to-clone view of the text (`Send + Sync`).
#[derive(Debug, Clone)]
pub struct BufferSnapshot {
    rope: Rope,
}

impl BufferSnapshot {
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
