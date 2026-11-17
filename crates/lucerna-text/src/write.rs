//! Writing a buffer back to disk: encoding-preserving and atomic.

use std::fmt;
use std::fs;
use std::hash::{DefaultHasher, Hasher};
use std::io::{self, Write};
use std::path::Path;

use encoding_rs::{EncoderResult, Encoding, UTF_16BE, UTF_16LE};

use crate::buffer::{Buffer, BufferSnapshot, LineEnding, Point};

/// Why a save failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteError {
    /// The text has a character the file's encoding cannot represent. `at`
    /// is in the LF-normalised text (byte column, for navigation); `column`
    /// is the 0-based char column, which the message shows 1-based.
    Unencodable {
        encoding: &'static str,
        at: Point,
        column: u32,
    },
    Io(String),
    PermissionDenied,
}

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WriteError::Unencodable {
                encoding,
                at,
                column,
            } => write!(
                f,
                "{encoding} cannot encode the character at Ln {}, Col {}",
                at.row + 1,
                column + 1
            ),
            WriteError::Io(reason) => f.write_str(reason),
            WriteError::PermissionDenied => f.write_str("Permission denied"),
        }
    }
}

impl std::error::Error for WriteError {}

impl From<io::Error> for WriteError {
    fn from(e: io::Error) -> Self {
        if e.kind() == io::ErrorKind::PermissionDenied {
            WriteError::PermissionDenied
        } else {
            WriteError::Io(e.to_string())
        }
    }
}

/// A completed write. `hash` is computed exactly as `decode_file` does over
/// the raw bytes, so a watcher event for our own write can be recognised.
/// `modified` and `len` are read from the written file's handle after
/// `sync_all` and before the rename, so a later write by another program
/// is never in them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Written {
    pub hash: u64,
    pub modified: Option<std::time::SystemTime>,
    pub len: u64,
}

/// Output side of the encoder: bytes in the file's encoding.
struct Sink {
    out: Vec<u8>,
    kind: SinkKind,
}

enum SinkKind {
    Utf16 { little_endian: bool },
    Other(encoding_rs::Encoder),
}

impl Sink {
    fn new(encoding: &'static Encoding, len_hint: usize) -> Sink {
        let kind = if encoding == UTF_16LE {
            SinkKind::Utf16 {
                little_endian: true,
            }
        } else if encoding == UTF_16BE {
            SinkKind::Utf16 {
                little_endian: false,
            }
        } else {
            SinkKind::Other(encoding.new_encoder())
        };
        Sink {
            out: Vec::with_capacity(len_hint + 4),
            kind,
        }
    }

    /// Encodes `s`; on an unmappable char returns its byte index in `s`.
    fn push(&mut self, s: &str, last: bool) -> Result<(), usize> {
        match &mut self.kind {
            SinkKind::Utf16 { little_endian } => {
                for unit in s.encode_utf16() {
                    let bytes = if *little_endian {
                        unit.to_le_bytes()
                    } else {
                        unit.to_be_bytes()
                    };
                    self.out.extend_from_slice(&bytes);
                }
                Ok(())
            }
            SinkKind::Other(encoder) => {
                let mut src = s;
                let mut consumed = 0;
                loop {
                    let need = encoder
                        .max_buffer_length_from_utf8_without_replacement(src.len())
                        .unwrap_or(src.len() * 4 + 16);
                    let start = self.out.len();
                    self.out.resize(start + need, 0);
                    let (result, read, written) = encoder.encode_from_utf8_without_replacement(
                        src,
                        &mut self.out[start..],
                        last,
                    );
                    self.out.truncate(start + written);
                    match result {
                        EncoderResult::InputEmpty => return Ok(()),
                        EncoderResult::OutputFull => {
                            consumed += read;
                            src = &src[read..];
                        }
                        EncoderResult::Unmappable(c) => {
                            return Err(consumed + read - c.len_utf8());
                        }
                    }
                }
            }
        }
    }
}

impl Buffer {
    /// Encodes the snapshot the way the file is stored: its encoding, BOM
    /// and line ending. An associated function so it can run on a
    /// background thread with copied settings.
    pub fn encode(
        snapshot: &BufferSnapshot,
        encoding: &'static Encoding,
        has_bom: bool,
        line_ending: LineEnding,
    ) -> Result<Vec<u8>, WriteError> {
        let mut sink = Sink::new(encoding, snapshot.len_bytes());
        let unencodable = |offset: usize| {
            let at = snapshot.offset_to_point(offset);
            WriteError::Unencodable {
                encoding: encoding.name(),
                at,
                column: snapshot.char_column(at),
            }
        };
        if has_bom {
            // The BOM is U+FEFF in the file's own encoding.
            sink.push("\u{FEFF}", false).map_err(|_| unencodable(0))?;
        }
        let mut offset = 0;
        for chunk in snapshot.chunks_in_range(0..snapshot.len_bytes()) {
            let mut seg_start = 0;
            while seg_start < chunk.len() {
                let (piece, newline) = match chunk[seg_start..].find('\n') {
                    Some(i) => (&chunk[seg_start..seg_start + i], true),
                    None => (&chunk[seg_start..], false),
                };
                sink.push(piece, false)
                    .map_err(|i| unencodable(offset + seg_start + i))?;
                seg_start += piece.len();
                if newline {
                    let eol = match line_ending {
                        LineEnding::Lf => "\n",
                        LineEnding::CrLf => "\r\n",
                    };
                    sink.push(eol, false)
                        .map_err(|_| unencodable(offset + seg_start))?;
                    seg_start += 1;
                }
            }
            offset += chunk.len();
        }
        sink.push("", true).map_err(|_| unencodable(offset))?;
        Ok(sink.out)
    }

    /// Encodes `snapshot` with this buffer's encoding, BOM and line ending,
    /// then writes it to `path`.
    pub fn write(&self, snapshot: &BufferSnapshot, path: &Path) -> Result<Written, WriteError> {
        let bytes = Buffer::encode(
            snapshot,
            self.encoding(),
            self.has_bom(),
            self.line_ending(),
        )?;
        write_bytes(path, &bytes)
    }
}

/// Writes `bytes` to `path`. A symlink is resolved and its target written,
/// leaving the link untouched. The write is atomic (temp file, fsync,
/// rename) unless the target has several hardlinks, where replacing the
/// inode would break the links: that is written in place. A read-only
/// target is refused with `PermissionDenied`. A new file gets mode 0o644
/// (the umask is not consulted).
pub fn write_bytes(path: &Path, bytes: &[u8]) -> Result<Written, WriteError> {
    let mut hasher = DefaultHasher::new();
    hasher.write(bytes);
    let hash = hasher.finish();
    let is_symlink = match fs::symlink_metadata(path) {
        Ok(meta) => meta.file_type().is_symlink(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => false,
        Err(e) => return Err(e.into()),
    };
    let target = if is_symlink {
        fs::canonicalize(path)?
    } else {
        path.to_path_buf()
    };
    let written = match fs::metadata(&target) {
        Ok(meta) => {
            if meta.permissions().readonly() {
                return Err(WriteError::PermissionDenied);
            }
            if link_count(&meta) > 1 {
                write_in_place(&target, bytes)?
            } else {
                write_atomic(&target, bytes, Some(meta))?
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => write_atomic(&target, bytes, None)?,
        Err(e) => return Err(e.into()),
    };
    Ok(Written {
        hash,
        modified: written.modified().ok(),
        len: written.len(),
    })
}

fn write_in_place(path: &Path, bytes: &[u8]) -> Result<fs::Metadata, WriteError> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(file.metadata()?)
}

/// Longest file name, in bytes, that common filesystems accept.
const MAX_NAME_BYTES: usize = 255;
/// Random characters `tempfile` appends after the prefix.
const TEMP_RAND_BYTES: usize = 8;

/// Temp file prefix `.{name}.luc~`, with `name` cut so the full temp name
/// (prefix plus random suffix) stays within `MAX_NAME_BYTES`.
fn temp_prefix(name: &str) -> String {
    // 6 = the leading '.' plus the ".luc~" suffix.
    let limit = MAX_NAME_BYTES - TEMP_RAND_BYTES - 6;
    let cut = name.floor_char_boundary(limit.min(name.len()));
    format!(".{}.luc~", &name[..cut])
}

/// Returns the temp file's metadata, read before the rename.
fn write_atomic(
    path: &Path,
    bytes: &[u8],
    old: Option<fs::Metadata>,
) -> Result<fs::Metadata, WriteError> {
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let name = path
        .file_name()
        .ok_or_else(|| WriteError::Io("path has no file name".into()))?
        .to_string_lossy();
    let mut temp = tempfile::Builder::new()
        .prefix(&temp_prefix(&name))
        .rand_bytes(TEMP_RAND_BYTES)
        .tempfile_in(parent)?;
    match old {
        Some(meta) => fs::set_permissions(temp.path(), meta.permissions())?,
        None => set_new_file_mode(temp.path())?,
    }
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    let meta = temp.as_file().metadata()?;
    temp.persist(path).map_err(|e| WriteError::from(e.error))?;
    sync_dir(parent);
    Ok(meta)
}

#[cfg(unix)]
fn set_new_file_mode(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o644))
}

#[cfg(not(unix))]
fn set_new_file_mode(_: &Path) -> io::Result<()> {
    Ok(())
}

/// Best effort: makes the rename durable.
#[cfg(unix)]
fn sync_dir(dir: &Path) {
    if let Ok(d) = fs::File::open(dir) {
        let _ = d.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_dir(_: &Path) {}

#[cfg(unix)]
fn link_count(meta: &fs::Metadata) -> u64 {
    std::os::unix::fs::MetadataExt::nlink(meta)
}

#[cfg(not(unix))]
fn link_count(_: &fs::Metadata) -> u64 {
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load::decode_file;

    fn utf16(text: &str, le: bool, bom: bool) -> Vec<u8> {
        let mut units: Vec<u16> = Vec::new();
        if bom {
            units.push(0xFEFF);
        }
        units.extend(text.encode_utf16());
        units
            .into_iter()
            .flat_map(|u| if le { u.to_le_bytes() } else { u.to_be_bytes() })
            .collect()
    }

    fn fixtures() -> Vec<(&'static str, Vec<u8>)> {
        let sjis = encoding_rs::SHIFT_JIS.encode("日本語\r\n").0.into_owned();
        vec![
            ("utf8", "hello\nworld\n".as_bytes().to_vec()),
            ("utf8-bom", b"\xEF\xBB\xBFhello\nworld\n".to_vec()),
            ("utf16le-bom-crlf", utf16("a\r\nb\r\n", true, true)),
            ("utf16be-bom", utf16("a\nb\n", false, true)),
            ("shift-jis", sjis),
            (
                "windows-1252",
                encoding_rs::WINDOWS_1252.encode("café\n").0.into_owned(),
            ),
        ]
    }

    #[test]
    fn write_round_trips_every_encoding() {
        let dir = tempfile::TempDir::new().unwrap();
        for (name, bytes) in fixtures() {
            let buffer = Buffer::from_bytes(&bytes, false).unwrap();
            let path = dir.path().join(name);
            fs::write(&path, b"old").unwrap();
            let written = buffer.write(&buffer.snapshot(), &path).unwrap();
            assert_eq!(fs::read(&path).unwrap(), bytes, "{name}");
            assert_eq!(
                written.hash,
                decode_file(&bytes, false).unwrap().hash,
                "{name}"
            );
        }
    }

    #[test]
    fn edited_text_uses_file_line_ending() {
        let mut buffer = Buffer::from_bytes(b"a\r\nb\r\n", false).unwrap();
        buffer.edit([(0..0, std::sync::Arc::from("x\n"))]);
        let bytes = Buffer::encode(
            &buffer.snapshot(),
            buffer.encoding(),
            buffer.has_bom(),
            buffer.line_ending(),
        )
        .unwrap();
        assert_eq!(bytes, b"x\r\na\r\nb\r\n");
    }

    #[test]
    fn unencodable_reports_position() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("f.txt");
        let original = encoding_rs::WINDOWS_1252
            .encode("café\nabc\n")
            .0
            .into_owned();
        fs::write(&path, &original).unwrap();
        let mut buffer = Buffer::from_bytes(&original, false).unwrap();
        assert_eq!(buffer.encoding(), encoding_rs::WINDOWS_1252);
        let at = buffer
            .snapshot()
            .point_to_offset(Point { row: 1, column: 2 });
        buffer.edit([(at..at, std::sync::Arc::from("ệ"))]);
        let err = buffer.write(&buffer.snapshot(), &path).unwrap_err();
        assert_eq!(
            err,
            WriteError::Unencodable {
                encoding: "windows-1252",
                at: Point { row: 1, column: 2 },
                column: 2
            }
        );
        assert_eq!(
            err.to_string(),
            "windows-1252 cannot encode the character at Ln 2, Col 3"
        );
        assert_eq!(fs::read(&path).unwrap(), original);
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_keeps_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("f.txt");
        fs::write(&path, b"old").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        write_bytes(&path, b"new").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_is_written_through() {
        let dir = tempfile::TempDir::new().unwrap();
        let target = dir.path().join("target.txt");
        let link = dir.path().join("link.txt");
        fs::write(&target, b"old").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        write_bytes(&link, b"new").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new");
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn written_stamp_is_the_written_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("f.txt");
        fs::write(&path, b"old").unwrap();
        let written = write_bytes(&path, b"new").unwrap();
        assert_eq!(written.len, 3);
        assert_eq!(
            written.modified,
            fs::metadata(&path).unwrap().modified().ok()
        );
        // A later write by another program is not in it.
        fs::write(&path, b"agent, longer").unwrap();
        assert_eq!(written.len, 3);
    }

    #[cfg(unix)]
    #[test]
    fn hardlink_written_stamp_is_the_written_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let a = dir.path().join("a.txt");
        let b = dir.path().join("b.txt");
        fs::write(&a, b"old").unwrap();
        fs::hard_link(&a, &b).unwrap();
        let written = write_bytes(&a, b"new").unwrap();
        assert_eq!(written.len, 3);
        assert_eq!(written.modified, fs::metadata(&a).unwrap().modified().ok());
        fs::write(&a, b"agent, longer").unwrap();
        assert_eq!(written.len, 3);
    }

    #[cfg(unix)]
    #[test]
    fn hardlink_is_written_in_place() {
        let dir = tempfile::TempDir::new().unwrap();
        let a = dir.path().join("a.txt");
        let b = dir.path().join("b.txt");
        fs::write(&a, b"old").unwrap();
        fs::hard_link(&a, &b).unwrap();
        write_bytes(&a, b"new").unwrap();
        assert_eq!(fs::read(&a).unwrap(), b"new");
        assert_eq!(fs::read(&b).unwrap(), b"new");
    }

    #[cfg(unix)]
    #[test]
    fn failed_atomic_write_leaves_original_and_no_temp() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("f.txt");
        fs::write(&path, b"old").unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o555)).unwrap();
        // Root ignores directory permissions: probe instead of trusting `$USER`.
        let root = fs::File::create(dir.path().join("probe")).is_ok();
        let result = write_bytes(&path, b"new");
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
        if root {
            return;
        }
        assert_eq!(result, Err(WriteError::PermissionDenied));
        assert_eq!(fs::read(&path).unwrap(), b"old");
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with(".f.txt.luc~"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn temp_prefix_keeps_short_names() {
        assert_eq!(temp_prefix("f.txt"), ".f.txt.luc~");
    }

    #[test]
    fn temp_prefix_fits_the_name_limit() {
        for name in ["a".repeat(250), format!("{}.rs", "ệ".repeat(83))] {
            let prefix = temp_prefix(&name);
            assert!(prefix.len() + TEMP_RAND_BYTES <= MAX_NAME_BYTES, "{name}");
            let cut = &prefix[1..prefix.len() - ".luc~".len()];
            assert!(name.starts_with(cut), "cut on a char boundary");
            assert!(
                cut.len() >= MAX_NAME_BYTES - TEMP_RAND_BYTES - 6 - 3,
                "cut no shorter than needed"
            );
        }
    }

    #[test]
    fn long_name_is_saved() {
        let dir = tempfile::TempDir::new().unwrap();
        for name in ["a".repeat(250), format!("{}.rs", "ệ".repeat(83))] {
            let path = dir.path().join(&name);
            if fs::write(&path, b"old").is_err() {
                // The filesystem itself refuses this name.
                return;
            }
            write_bytes(&path, b"new").unwrap();
            assert_eq!(fs::read(&path).unwrap(), b"new");
        }
        let hidden: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with('.'))
            .collect();
        assert!(hidden.is_empty(), "{hidden:?}");
    }

    #[test]
    fn unencodable_column_counts_chars() {
        let original = encoding_rs::WINDOWS_1252.encode("café \n").0.into_owned();
        let mut buffer = Buffer::from_bytes(&original, false).unwrap();
        assert_eq!(buffer.encoding(), encoding_rs::WINDOWS_1252);
        buffer.edit([(6..6, std::sync::Arc::from("ệ"))]);
        let err = Buffer::encode(&buffer.snapshot(), buffer.encoding(), false, LineEnding::Lf)
            .unwrap_err();
        // "caf" + "é" is 4 chars but 5 bytes of UTF-8; the space follows.
        assert_eq!(
            err,
            WriteError::Unencodable {
                encoding: "windows-1252",
                at: Point { row: 0, column: 6 },
                column: 5
            }
        );
        assert_eq!(
            err.to_string(),
            "windows-1252 cannot encode the character at Ln 1, Col 6"
        );
    }

    #[cfg(unix)]
    #[test]
    fn read_only_file_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("f.txt");
        fs::write(&path, b"old").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        assert_eq!(
            write_bytes(&path, b"new"),
            Err(WriteError::PermissionDenied)
        );
        assert_eq!(fs::read(&path).unwrap(), b"old");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o444
        );
    }

    #[cfg(unix)]
    #[test]
    fn new_file_gets_default_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("new.txt");
        write_bytes(&path, b"x").unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o644
        );
    }
}
