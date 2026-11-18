//! The replace job for closed files: read, check, edit, encode, re-check,
//! write. Free of gpui.
//!
//! The job writes real files that an agent may be writing too, so every step
//! errs towards skipping: a file is only written when the bytes on disk are
//! still the bytes that were read, and only the lines the results showed
//! are touched.

use std::fs;
use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::SystemTime;

use lucerna_text::{
    Buffer, BufferSnapshot, TempFiles, WriteError, check_regular, check_size, decode_file,
    write_bytes_tracked,
};
use regex::Regex;

use crate::replace::{ReplacePattern, replace_ranges};
use crate::search::{SearchQuery, line_matches, search_regex};

/// A result line the user saw: 1-based number, text as shown (no `\r`,
/// cut to the display limit).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShownLine {
    pub line_number: u32,
    pub text: String,
}

/// What to replace and with what.
#[derive(Debug, Clone)]
pub struct ReplaceSpec {
    pub query: SearchQuery,
    pub pattern: ReplacePattern,
    pub preserve_case: bool,
}

/// Why a file was left alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    ChangedSinceSearch,
    ChangedOnDisk,
    Lossy,
    Unreadable,
    Unwritable(String),
    Unencodable(&'static str),
    ChangedWhileReplacing,
    Missing,
    Conflict,
}

impl SkipReason {
    /// The text the app shows after the file name.
    pub fn message(&self) -> String {
        match self {
            SkipReason::ChangedSinceSearch => "changed since the search".into(),
            SkipReason::ChangedOnDisk => "changed on disk".into(),
            SkipReason::Lossy => "was not decoded cleanly".into(),
            SkipReason::Unreadable => "could not be read".into(),
            SkipReason::Unwritable(err) => format!("could not be written: {err}"),
            SkipReason::Unencodable(enc) => {
                format!("cannot encode the replacement in {enc}")
            }
            SkipReason::ChangedWhileReplacing => "changed while replacing".into(),
            SkipReason::Missing => "no longer exists".into(),
            SkipReason::Conflict => "has a conflict with the file on disk".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileOutcome {
    /// The number of replacements made.
    Replaced(usize),
    Skipped(SkipReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplaceProgress {
    pub done: usize,
    pub total: usize,
}

/// Edits for `text` (LF-normalised): the core shared by open documents and
/// files. Only matches that start inside a shown line's shown prefix are
/// kept; the edits are computed on the whole text so captures and
/// lookarounds see the real context.
///
/// # Errors
/// `ChangedSinceSearch` when a shown line is not (a prefix of) the text's
/// line any more, or when the query is not a valid pattern.
pub fn edits_for_text(
    text: &str,
    lines: &[ShownLine],
    spec: &ReplaceSpec,
) -> Result<Vec<(Range<usize>, String)>, SkipReason> {
    let regex = search_regex(&spec.query).map_err(|_| SkipReason::ChangedSinceSearch)?;
    edits_with(text, lines, spec, &regex)
}

fn edits_with(
    text: &str,
    lines: &[ShownLine],
    spec: &ReplaceSpec,
    regex: &Regex,
) -> Result<Vec<(Range<usize>, String)>, SkipReason> {
    // Start offset of every line, built once.
    let mut starts = vec![0usize];
    starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
    let mut ranges: Vec<Range<usize>> = Vec::new();
    for line in lines {
        let index = (line.line_number as usize)
            .checked_sub(1)
            .ok_or(SkipReason::ChangedSinceSearch)?;
        let start = *starts.get(index).ok_or(SkipReason::ChangedSinceSearch)?;
        let end = starts.get(index + 1).map_or(text.len(), |next| next - 1);
        let full = &text[start..end];
        let shown = line.text.len();
        if full.as_bytes().get(..shown) != Some(line.text.as_bytes()) {
            return Err(SkipReason::ChangedSinceSearch);
        }
        for m in line_matches(regex, &spec.query, full) {
            if m.start < shown {
                ranges.push(start + m.start..start + m.end);
            }
        }
    }
    ranges.sort_by_key(|r| (r.start, r.end));
    ranges.dedup();
    Ok(replace_ranges(
        text,
        regex,
        &ranges,
        &spec.pattern,
        spec.preserve_case,
    ))
}

/// Size and modification time: the cheap "did it change" check of the save
/// path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    len: u64,
    modified: Option<SystemTime>,
}

impl Stamp {
    fn of(meta: &fs::Metadata) -> Stamp {
        Stamp {
            len: meta.len(),
            modified: meta.modified().ok(),
        }
    }
}

/// One closed file: read, check, edit, encode, re-check, write.
pub fn replace_file(
    path: &Path,
    lines: &[ShownLine],
    spec: &ReplaceSpec,
    temps: &TempFiles,
) -> FileOutcome {
    replace_file_with_hook(path, lines, spec, temps, &|| {})
}

/// [`replace_file`] with `before_write` called after the edits are encoded
/// and before the file is re-checked, so a test can change the file in
/// between.
#[doc(hidden)]
pub fn replace_file_with_hook(
    path: &Path,
    lines: &[ShownLine],
    spec: &ReplaceSpec,
    temps: &TempFiles,
    before_write: &dyn Fn(),
) -> FileOutcome {
    match search_regex(&spec.query) {
        Ok(regex) => replace_one(path, lines, spec, &regex, temps, before_write),
        Err(_) => FileOutcome::Skipped(SkipReason::ChangedSinceSearch),
    }
}

fn skipped(reason: SkipReason) -> FileOutcome {
    FileOutcome::Skipped(reason)
}

fn read_error(err: &io::Error) -> SkipReason {
    if err.kind() == io::ErrorKind::NotFound {
        SkipReason::Missing
    } else {
        SkipReason::Unreadable
    }
}

fn replace_one(
    path: &Path,
    lines: &[ShownLine],
    spec: &ReplaceSpec,
    regex: &Regex,
    temps: &TempFiles,
    before_write: &dyn Fn(),
) -> FileOutcome {
    // The stamp comes before the bytes: a change in between makes the
    // re-check fail instead of going unnoticed.
    let meta = match fs::metadata(path) {
        Ok(meta) => meta,
        Err(err) => return skipped(read_error(&err)),
    };
    if check_regular(&meta).is_err() || check_size(meta.len()).is_err() {
        return skipped(SkipReason::Unreadable);
    }
    let stamp = Stamp::of(&meta);
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) => return skipped(read_error(&err)),
    };
    let decoded = match decode_file(&bytes, false) {
        Ok(decoded) => decoded,
        Err(_) => return skipped(SkipReason::Unreadable),
    };
    if decoded.lossy {
        return skipped(SkipReason::Lossy);
    }
    let edits = match edits_with(&decoded.text, lines, spec, regex) {
        Ok(edits) => edits,
        Err(reason) => return skipped(reason),
    };
    if edits.is_empty() {
        return FileOutcome::Replaced(0);
    }
    let mut out = String::with_capacity(decoded.text.len());
    let mut cursor = 0;
    for (range, replacement) in &edits {
        if range.start < cursor {
            // Overlapping edits cannot come from distinct matches; refuse
            // rather than guess.
            return skipped(SkipReason::ChangedSinceSearch);
        }
        out.push_str(&decoded.text[cursor..range.start]);
        out.push_str(replacement);
        cursor = range.end;
    }
    out.push_str(&decoded.text[cursor..]);
    let encoded = match Buffer::encode(
        &BufferSnapshot::from_text(&out),
        decoded.encoding,
        decoded.has_bom,
        decoded.line_ending,
    ) {
        Ok(encoded) => encoded,
        Err(WriteError::Unencodable { encoding, .. }) => {
            return skipped(SkipReason::Unencodable(encoding));
        }
        Err(err) => return skipped(SkipReason::Unwritable(err.to_string())),
    };
    before_write();
    match unchanged_since(path, stamp, decoded.hash) {
        Ok(true) => {}
        Ok(false) => return skipped(SkipReason::ChangedOnDisk),
        Err(reason) => return skipped(reason),
    }
    match write_bytes_tracked(path, &encoded, temps) {
        Ok(_) => FileOutcome::Replaced(edits.len()),
        Err(err) => {
            if !path.exists() {
                skipped(SkipReason::Missing)
            } else {
                skipped(SkipReason::Unwritable(err.to_string()))
            }
        }
    }
}

/// True when the file still has the bytes that were read: same stamp, or a
/// different stamp but the same hash (e.g. touched). Mirrors the save
/// path's `changed_on_disk`; `decode_file` hashes with the same hasher.
fn unchanged_since(path: &Path, stamp: Stamp, hash: u64) -> Result<bool, SkipReason> {
    let meta = fs::metadata(path).map_err(|err| read_error(&err))?;
    if Stamp::of(&meta) == stamp {
        return Ok(true);
    }
    let bytes = fs::read(path).map_err(|err| read_error(&err))?;
    Ok(hash_bytes(&bytes) == hash)
}

fn hash_bytes(bytes: &[u8]) -> u64 {
    use std::hash::{DefaultHasher, Hasher};
    let mut hasher = DefaultHasher::new();
    hasher.write(bytes);
    hasher.finish()
}

/// Every file, on up to `available_parallelism().min(8)` threads; blocking.
/// Stops taking files once `cancel` is set; files already written stay
/// written. Returns an outcome for each file it processed, in input order.
pub fn replace_files(
    files: Vec<(PathBuf, Vec<ShownLine>)>,
    spec: &ReplaceSpec,
    temps: &TempFiles,
    cancel: &AtomicBool,
    progress: &(dyn Fn(ReplaceProgress) + Sync),
) -> Vec<(PathBuf, FileOutcome)> {
    if cancel.load(Ordering::SeqCst) || files.is_empty() {
        return Vec::new();
    }
    let total = files.len();
    let regex = match search_regex(&spec.query) {
        Ok(regex) => regex,
        Err(_) => {
            return files
                .into_iter()
                .map(|(path, _)| (path, skipped(SkipReason::ChangedSinceSearch)))
                .collect();
        }
    };
    let threads = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .min(8)
        .min(total);
    let next = AtomicUsize::new(0);
    // Held while counting and reporting, so progress is monotonic.
    let done = Mutex::new(0usize);
    let results: Mutex<Vec<(usize, FileOutcome)>> = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                loop {
                    if cancel.load(Ordering::SeqCst) {
                        break;
                    }
                    let index = next.fetch_add(1, Ordering::SeqCst);
                    let Some((path, lines)) = files.get(index) else {
                        break;
                    };
                    let outcome = replace_one(path, lines, spec, &regex, temps, &|| {});
                    results
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push((index, outcome));
                    let mut done = done.lock().unwrap_or_else(|e| e.into_inner());
                    *done += 1;
                    progress(ReplaceProgress { done: *done, total });
                }
            });
        }
    });
    let mut results = results.into_inner().unwrap_or_else(|e| e.into_inner());
    results.sort_by_key(|(index, _)| *index);
    results
        .into_iter()
        .map(|(index, outcome)| (files[index].0.clone(), outcome))
        .collect()
}
