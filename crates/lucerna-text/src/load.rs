use std::fmt;
use std::hash::{DefaultHasher, Hasher};
use std::io;

use encoding_rs::{Encoding, UTF_8};

use crate::buffer::LineEnding;

/// Files larger than this are refused (512 MiB).
pub const MAX_OPEN_BYTES: u64 = 536_870_912;

const BINARY_SNIFF_BYTES: usize = 8192;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    NotFound,
    PermissionDenied,
    Binary,
    TooLarge { bytes: u64 },
    /// A FIFO, device or socket: reading it could block or never end.
    NotRegular,
    Io(String),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::NotFound => write!(f, "The file was not found."),
            LoadError::PermissionDenied => write!(f, "Permission denied."),
            LoadError::Binary => write!(f, "The file appears to be binary."),
            LoadError::TooLarge { bytes } => write!(
                f,
                "The file is too large to open ({} MiB, limit {} MiB).",
                bytes / (1024 * 1024),
                MAX_OPEN_BYTES / (1024 * 1024)
            ),
            LoadError::NotRegular => write!(f, "Not a regular file"),
            LoadError::Io(message) => write!(f, "Could not read the file: {message}"),
        }
    }
}

impl std::error::Error for LoadError {}

impl From<io::Error> for LoadError {
    fn from(err: io::Error) -> Self {
        match err.kind() {
            io::ErrorKind::NotFound => LoadError::NotFound,
            io::ErrorKind::PermissionDenied => LoadError::PermissionDenied,
            _ => LoadError::Io(err.to_string()),
        }
    }
}

/// Rejects lengths above [`MAX_OPEN_BYTES`].
/// Refuses what is not a regular file (FIFO, device, socket), which can
/// report length 0 yet block or never end when read. `metadata` follows
/// symlinks, so a link to a regular file passes.
///
/// # Errors
/// [`LoadError::NotRegular`].
pub fn check_regular(meta: &std::fs::Metadata) -> Result<(), LoadError> {
    if meta.is_file() {
        Ok(())
    } else {
        Err(LoadError::NotRegular)
    }
}

pub fn check_size(len: u64) -> Result<(), LoadError> {
    if len > MAX_OPEN_BYTES {
        Err(LoadError::TooLarge { bytes: len })
    } else {
        Ok(())
    }
}

/// The result of [`decode_file`]: LF-normalised text plus the facts needed to
/// write the file back byte-for-byte.
#[derive(Debug, Clone)]
pub struct Decoded {
    /// Text with every `\r\n` turned into `\n`; a lone `\r` is kept.
    pub text: String,
    pub encoding: &'static Encoding,
    pub has_bom: bool,
    /// The dominant line ending of the original bytes.
    pub line_ending: LineEnding,
    /// Hash of the raw bytes. Deterministic within one process, never persisted.
    pub hash: u64,
}

/// Decodes `bytes` to LF-normalised UTF-8 text.
///
/// Order: size, binary sniff (NUL in the first 8 KiB, skipped for `force_text`
/// and for UTF-16 BOMs, whose text legitimately contains NULs), BOM, valid
/// UTF-8, then `chardetng`. Public so callers can decode off the main thread.
pub fn decode_file(bytes: &[u8], force_text: bool) -> Result<Decoded, LoadError> {
    check_size(bytes.len() as u64)?;
    let bom = Encoding::for_bom(bytes);
    let utf16_bom = matches!(bom, Some((e, _)) if e != UTF_8);
    if !force_text && !utf16_bom && bytes.iter().take(BINARY_SNIFF_BYTES).any(|&b| b == 0) {
        return Err(LoadError::Binary);
    }
    let (raw, encoding, has_bom) = if let Some((encoding, bom_len)) = bom {
        let (text, _) = encoding.decode_without_bom_handling(&bytes[bom_len..]);
        (text.into_owned(), encoding, true)
    } else if let Ok(text) = std::str::from_utf8(bytes) {
        (text.to_owned(), UTF_8, false)
    } else {
        let mut detector = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
        detector.feed(bytes, true);
        let encoding = detector.guess(None, chardetng::Utf8Detection::Deny);
        let (text, _) = encoding.decode_without_bom_handling(bytes);
        (text.into_owned(), encoding, false)
    };
    let line_ending = dominant_line_ending(&raw);
    let text = if raw.contains("\r\n") {
        raw.replace("\r\n", "\n")
    } else {
        raw
    };
    let mut hasher = DefaultHasher::new();
    hasher.write(bytes);
    Ok(Decoded {
        text,
        encoding,
        has_bom,
        line_ending,
        hash: hasher.finish(),
    })
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
