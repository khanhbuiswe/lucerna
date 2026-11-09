use std::borrow::Cow;
use std::ops::Range;
use std::path::Path;

use encoding_rs::Encoding;
use ropey::Rope;

use crate::load::{LoadError, check_size, decode};
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
        let (text, encoding) = decode(bytes, force_text)?;
        Ok(Buffer {
            line_ending: dominant_line_ending(&text),
            rope: Rope::from_str(&text),
            encoding,
            force_text,
        })
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
        let (text, encoding) = decode(bytes, self.force_text)?;
        let new = BufferSnapshot {
            rope: Rope::from_str(&text),
        };
        let reload = Reload::diff(&self.snapshot(), &new);
        self.line_ending = dominant_line_ending(&text);
        self.encoding = encoding;
        self.rope = new.rope;
        Ok(reload)
    }

    pub fn is_large(&self, max_bytes: u64, max_lines: u32) -> bool {
        self.rope.len_bytes() as u64 > max_bytes || self.snapshot().line_count() > max_lines
    }
}

fn dominant_line_ending(text: &str) -> LineEnding {
    let lf = text.matches('\n').count();
    let crlf = text.matches("\r\n").count();
    if crlf > lf - crlf {
        LineEnding::CrLf
    } else {
        LineEnding::Lf
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
