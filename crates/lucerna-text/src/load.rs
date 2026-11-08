use std::fmt;
use std::io;

use encoding_rs::{Encoding, UTF_8};

/// Files larger than this are refused (512 MiB).
pub const MAX_OPEN_BYTES: u64 = 536_870_912;

const BINARY_SNIFF_BYTES: usize = 8192;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    NotFound,
    PermissionDenied,
    Binary,
    TooLarge { bytes: u64 },
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
pub fn check_size(len: u64) -> Result<(), LoadError> {
    if len > MAX_OPEN_BYTES {
        Err(LoadError::TooLarge { bytes: len })
    } else {
        Ok(())
    }
}

/// Decodes `bytes` to UTF-8 text, returning the text and the encoding used.
///
/// Order: size, binary sniff (NUL in the first 8 KiB, skipped for `force_text`
/// and for UTF-16 BOMs, whose text legitimately contains NULs), BOM, valid
/// UTF-8, then `chardetng`.
pub(crate) fn decode(
    bytes: &[u8],
    force_text: bool,
) -> Result<(String, &'static Encoding), LoadError> {
    check_size(bytes.len() as u64)?;
    let bom = Encoding::for_bom(bytes);
    let utf16_bom = matches!(bom, Some((e, _)) if e != UTF_8);
    if !force_text && !utf16_bom && bytes.iter().take(BINARY_SNIFF_BYTES).any(|&b| b == 0) {
        return Err(LoadError::Binary);
    }
    if let Some((encoding, bom_len)) = bom {
        let (text, _) = encoding.decode_without_bom_handling(&bytes[bom_len..]);
        return Ok((text.into_owned(), encoding));
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Ok((text.to_owned(), UTF_8));
    }
    let mut detector = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
    detector.feed(bytes, true);
    let encoding = detector.guess(None, chardetng::Utf8Detection::Deny);
    let (text, _) = encoding.decode_without_bom_handling(bytes);
    Ok((text.into_owned(), encoding))
}
