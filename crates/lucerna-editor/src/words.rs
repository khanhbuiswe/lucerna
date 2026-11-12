//! VS Code word boundaries: character classes and the `cursorWordLeft` /
//! `cursorWordEndRight` motions.
//!
//! Adapted from microsoft/vscode src/vs/editor/common/core/wordCharacterClassifier.ts (MIT)
//! Adapted from microsoft/vscode src/vs/editor/common/cursor/cursorWordOperations.ts (MIT)
//!
//! Not ported: the `Intl.Segmenter` word splitting VS Code applies for CJK
//! locales. Columns are counted in chars, where VS Code counts UTF-16 units.

use lucerna_text::{BufferSnapshot, Point};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharClass {
    Regular,
    Whitespace,
    Separator,
}

/// Space and tab are whitespace, chars of `separators` are separators,
/// everything else is regular.
pub fn classify(c: char, separators: &str) -> CharClass {
    if c == ' ' || c == '\t' {
        CharClass::Whitespace
    } else if separators.contains(c) {
        CharClass::Separator
    } else {
        CharClass::Regular
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WordType {
    None,
    Regular,
    Separator,
}

/// A word on one line, as char indexes.
struct Word {
    start: usize,
    end: usize,
    word_type: WordType,
    next_class: CharClass,
}

struct Line {
    chars: Vec<char>,
    /// Byte offset of each char index, plus the line length.
    bytes: Vec<usize>,
    separators: String,
}

impl Line {
    fn new(snapshot: &BufferSnapshot, row: u32, separators: &str) -> Self {
        let text = snapshot.line(row);
        let mut chars = Vec::new();
        let mut bytes = Vec::new();
        for (i, c) in text.char_indices() {
            chars.push(c);
            bytes.push(i);
        }
        bytes.push(text.len());
        Line {
            chars,
            bytes,
            separators: separators.to_owned(),
        }
    }

    fn class(&self, i: usize) -> CharClass {
        classify(self.chars[i], &self.separators)
    }

    fn len(&self) -> usize {
        self.chars.len()
    }

    fn word(&self, word_type: WordType, next_class: CharClass, start: usize, end: usize) -> Word {
        Word {
            start,
            end,
            word_type,
            next_class,
        }
    }

    /// The last word that starts before `col`.
    fn find_previous_word(&self, col: usize) -> Option<Word> {
        let mut word_type = WordType::None;
        for i in (0..col).rev() {
            let class = self.class(i);
            let found = match class {
                CharClass::Regular => word_type == WordType::Separator,
                CharClass::Separator => word_type == WordType::Regular,
                CharClass::Whitespace => word_type != WordType::None,
            };
            if !found {
                match class {
                    CharClass::Regular => word_type = WordType::Regular,
                    CharClass::Separator => word_type = WordType::Separator,
                    CharClass::Whitespace => {}
                }
            }
            if found {
                return Some(self.word(
                    word_type,
                    class,
                    i + 1,
                    self.end_of_word(word_type, i + 1),
                ));
            }
        }
        (word_type != WordType::None).then(|| {
            self.word(
                word_type,
                CharClass::Whitespace,
                0,
                self.end_of_word(word_type, 0),
            )
        })
    }

    fn end_of_word(&self, word_type: WordType, start: usize) -> usize {
        for i in start..self.len() {
            let stop = match self.class(i) {
                CharClass::Whitespace => true,
                CharClass::Separator => word_type == WordType::Regular,
                CharClass::Regular => word_type == WordType::Separator,
            };
            if stop {
                return i;
            }
        }
        self.len()
    }

    /// The first word that ends after `col`.
    fn find_next_word(&self, col: usize) -> Option<Word> {
        let mut word_type = WordType::None;
        for i in col..self.len() {
            let class = self.class(i);
            let found = match class {
                CharClass::Regular => word_type == WordType::Separator,
                CharClass::Separator => word_type == WordType::Regular,
                CharClass::Whitespace => word_type != WordType::None,
            };
            if !found {
                match class {
                    CharClass::Regular => word_type = WordType::Regular,
                    CharClass::Separator => word_type = WordType::Separator,
                    CharClass::Whitespace => {}
                }
            }
            if found {
                return Some(self.word(word_type, class, self.start_of_word(word_type, i - 1), i));
            }
        }
        (word_type != WordType::None).then(|| {
            let len = self.len();
            self.word(
                word_type,
                CharClass::Whitespace,
                self.start_of_word(word_type, len - 1),
                len,
            )
        })
    }

    fn start_of_word(&self, word_type: WordType, start: usize) -> usize {
        for i in (0..=start).rev() {
            let stop = match self.class(i) {
                CharClass::Whitespace => true,
                CharClass::Separator => word_type == WordType::Regular,
                CharClass::Regular => word_type == WordType::Separator,
            };
            if stop {
                return i + 1;
            }
        }
        0
    }
}

fn last_row(snapshot: &BufferSnapshot) -> u32 {
    snapshot.offset_to_point(snapshot.len_bytes()).row
}

/// `cursorWordLeft` (WordStartFast; `fast` false gives `cursorWordStartLeft`).
/// `fast` skips a lone separator that is followed by a regular char, and VS
/// Code turns it off with several cursors so they do not stop apart.
pub fn word_start_left(
    snapshot: &BufferSnapshot,
    offset: usize,
    separators: &str,
    fast: bool,
) -> usize {
    let p = snapshot.offset_to_point(offset);
    let mut row = p.row;
    let mut col = snapshot.char_column(p) as usize;
    if col == 0 && row > 0 {
        row -= 1;
        col = Line::new(snapshot, row, separators).len();
    }
    let line = Line::new(snapshot, row, separators);
    let mut prev = line.find_previous_word(col);
    if fast
        && let Some(w) = &prev
        && w.word_type == WordType::Separator
        && w.end - w.start == 1
        && w.next_class == CharClass::Regular
    {
        prev = line.find_previous_word(w.start);
    }
    let base = snapshot.point_to_offset(Point { row, column: 0 });
    base + line.bytes[prev.map_or(0, |w| w.start)]
}

/// `cursorWordEndRight` (WordEnd).
pub fn word_end_right(snapshot: &BufferSnapshot, offset: usize, separators: &str) -> usize {
    let p = snapshot.offset_to_point(offset);
    let mut row = p.row;
    let mut col = snapshot.char_column(p) as usize;
    let mut line = Line::new(snapshot, row, separators);
    if col == line.len() && row < last_row(snapshot) {
        row += 1;
        col = 0;
        line = Line::new(snapshot, row, separators);
    }
    let mut next = line.find_next_word(col);
    if let Some(w) = &next
        && w.word_type == WordType::Separator
        && w.end - w.start == 1
        && w.next_class == CharClass::Regular
    {
        next = line.find_next_word(w.end);
    }
    let base = snapshot.point_to_offset(Point { row, column: 0 });
    base + line.bytes[next.map_or(line.len(), |w| w.end)]
}
