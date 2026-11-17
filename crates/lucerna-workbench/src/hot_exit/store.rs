//! The on-disk backup of unsaved documents, one folder per window under
//! `<data_dir>/backups/<key>/`. Pure file handling, no gpui, so the jobs
//! that schedule writes and restore them can share it.
//!
//! A backup file is one JSON header line (see [`BackupHeader`]) followed
//! by the document text as UTF-8 with `\n` line endings.

use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};

use lucerna_text::LineEnding;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::workspace_state::write_atomic;

/// The backup file format version; a file of any other version is set
/// aside.
const BACKUP_VERSION: u32 = 1;

/// The file in a window's folder that says which window it belongs to.
const WORKSPACE_FILE: &str = "workspace.json";

/// Which window a set of backups belongs to. A folder window and a
/// single-file window on a file inside that folder keep apart backups.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceKey {
    Folder(PathBuf),
    File(PathBuf),
}

/// What is needed, besides the text, to restore and later save a backed-up
/// document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BackupHeader {
    /// Absolute path of the document's file.
    pub path: PathBuf,
    /// Modification time of the file the edits started from, as seconds
    /// and nanoseconds since the Unix epoch.
    pub mtime: Option<(u64, u32)>,
    /// Size in bytes of that file.
    pub size: Option<u64>,
    /// Hex SHA-256 of the decoded base text (see [`text_sha256`]).
    pub base_sha256: Option<String>,
    /// WHATWG name of the file's encoding.
    pub encoding: String,
    pub bom: bool,
    #[serde(with = "line_ending_name")]
    pub line_ending: LineEnding,
    /// The text holds U+FFFD for bytes that were not valid in `encoding`.
    /// A backup written before this field existed reads as `false`.
    #[serde(default)]
    pub lossy: bool,
}

/// One document's unsaved text with its header.
#[derive(Debug, Clone, PartialEq)]
pub struct Backup {
    pub header: BackupHeader,
    pub text: String,
}

/// [`LineEnding`] as `"lf"` / `"crlf"`; lucerna-text has no serde.
mod line_ending_name {
    use lucerna_text::LineEnding;
    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    pub fn serialize<S: Serializer>(value: &LineEnding, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(match value {
            LineEnding::Lf => "lf",
            LineEnding::CrLf => "crlf",
        })
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<LineEnding, D::Error> {
        match String::deserialize(d)?.as_str() {
            "lf" => Ok(LineEnding::Lf),
            "crlf" => Ok(LineEnding::CrLf),
            other => Err(D::Error::custom(format!("unknown line ending {other:?}"))),
        }
    }
}

/// The first line of a backup file: the header with its version.
#[derive(Serialize, Deserialize)]
struct Wire {
    version: u32,
    #[serde(flatten)]
    header: BackupHeader,
}

/// The `workspace.json` of a window's folder.
#[derive(Serialize, Deserialize)]
struct WorkspaceInfo {
    version: u32,
    kind: String,
    path: PathBuf,
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Hex SHA-256 of `text`.
pub fn text_sha256(text: &str) -> String {
    hex_sha256(text.as_bytes())
}

/// `path` canonical; when nothing is at `path` (a file deleted since),
/// its canonical parent joined with its name, so the path stays the same
/// once the file is gone. `None` when the parent cannot be canonicalised
/// either.
pub fn canonical_even_if_missing(path: &Path) -> Option<PathBuf> {
    if let Ok(canonical) = path.canonicalize() {
        return Some(canonical);
    }
    let name = path.file_name()?;
    let parent = path.parent()?.canonicalize().ok()?;
    Some(parent.join(name))
}

/// `<data_dir>/backups/<hex sha256>`: the hash is of the canonical folder
/// path, or of `"file:"` and the canonical file path for a single-file
/// window. A deleted path is hashed by its canonical parent and name (the
/// key stays the same when the file goes away); a path whose parent cannot
/// be canonicalised either is hashed as given.
pub fn backup_dir(data_dir: &Path, key: &WorkspaceKey) -> PathBuf {
    let (path, prefix): (&Path, &[u8]) = match key {
        WorkspaceKey::Folder(path) => (path, b""),
        WorkspaceKey::File(path) => (path, b"file:"),
    };
    let path = canonical_even_if_missing(path).unwrap_or_else(|| path.to_path_buf());
    let mut bytes = prefix.to_vec();
    bytes.extend_from_slice(path.as_os_str().as_encoded_bytes());
    data_dir.join("backups").join(hex_sha256(&bytes))
}

/// The backup file of `path` inside `dir`: named by the hash of the path.
pub fn backup_file(dir: &Path, path: &Path) -> PathBuf {
    dir.join(hex_sha256(path.as_os_str().as_encoded_bytes()))
}

/// Writes `backup` into `dir` atomically, creating `dir` and its
/// `workspace.json` first when they are missing.
pub fn write_backup(dir: &Path, key: &WorkspaceKey, backup: &Backup) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let info_path = dir.join(WORKSPACE_FILE);
    if !info_path.exists() {
        let (kind, path) = match key {
            WorkspaceKey::Folder(path) => ("folder", path),
            WorkspaceKey::File(path) => ("file", path),
        };
        let info = WorkspaceInfo {
            version: 1,
            kind: kind.to_owned(),
            path: path.clone(),
        };
        let json = serde_json::to_vec(&info).map_err(io::Error::other)?;
        write_atomic(&info_path, &json)?;
    }
    let wire = Wire {
        version: BACKUP_VERSION,
        header: backup.header.clone(),
    };
    let mut bytes = serde_json::to_vec(&wire).map_err(io::Error::other)?;
    bytes.push(b'\n');
    bytes.extend_from_slice(backup.text.as_bytes());
    write_atomic(&backup_file(dir, &backup.header.path), &bytes)
}

/// Deletes the backup of `path`; none is fine. A folder left with only its
/// `workspace.json` is deleted too.
pub fn remove_backup(dir: &Path, path: &Path) -> io::Result<()> {
    match std::fs::remove_file(backup_file(dir, path)) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    let only_workspace = entries
        .filter_map(Result::ok)
        .all(|entry| entry.file_name() == WORKSPACE_FILE);
    if only_workspace {
        std::fs::remove_dir_all(dir)?;
    }
    Ok(())
}

/// Deletes the whole folder; none is fine.
pub fn remove_all(dir: &Path) -> io::Result<()> {
    match std::fs::remove_dir_all(dir) {
        Err(err) if err.kind() != io::ErrorKind::NotFound => Err(err),
        _ => Ok(()),
    }
}

/// A file in a backup folder that holds a backup, not metadata, a set-aside
/// file or a leftover temporary file.
fn is_backup_name(name: &str) -> bool {
    name != WORKSPACE_FILE && !name.ends_with(".bak") && !name.ends_with(".tmp")
}

fn parse(bytes: &[u8]) -> Option<Backup> {
    let text = std::str::from_utf8(bytes).ok()?;
    let (head, body) = text.split_once('\n')?;
    let wire: Wire = serde_json::from_str(head).ok()?;
    (wire.version == BACKUP_VERSION).then(|| Backup {
        header: wire.header,
        text: body.to_owned(),
    })
}

/// Every backup in `dir`. A file that cannot be read or parsed is renamed
/// to `<name>.bak` (replacing an older one) and skipped.
pub fn read_backups(dir: &Path) -> Vec<Backup> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut backups = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name();
        if !name.to_str().is_some_and(is_backup_name) {
            continue;
        }
        let path = entry.path();
        match std::fs::read(&path).ok().and_then(|bytes| parse(&bytes)) {
            Some(backup) => backups.push(backup),
            None => {
                tracing::warn!(path = %path.display(), "unreadable hot exit backup, set aside");
                let mut aside = path.as_os_str().to_owned();
                aside.push(".bak");
                let _ = std::fs::rename(&path, PathBuf::from(aside));
            }
        }
    }
    backups
}

/// The file of every single-file window that has at least one backup.
pub fn file_windows_with_backups(data_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(data_dir.join("backups")) else {
        return Vec::new();
    };
    let mut files = Vec::new();
    for dir in entries.filter_map(Result::ok).map(|entry| entry.path()) {
        let Some(info) = std::fs::read(dir.join(WORKSPACE_FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<WorkspaceInfo>(&bytes).ok())
        else {
            continue;
        };
        let has_backup = std::fs::read_dir(&dir).is_ok_and(|entries| {
            entries
                .filter_map(Result::ok)
                .any(|e| e.file_name().to_str().is_some_and(is_backup_name))
        });
        if info.kind == "file" && has_backup {
            files.push(info.path);
        }
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backup(path: &str, text: &str) -> Backup {
        Backup {
            header: BackupHeader {
                path: PathBuf::from(path),
                mtime: Some((1_700_000_000, 5)),
                size: Some(42),
                base_sha256: Some(text_sha256("base")),
                encoding: "UTF-8".into(),
                bom: false,
                line_ending: LineEnding::Lf,
                lossy: false,
            },
            text: text.into(),
        }
    }

    #[test]
    fn round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let key = WorkspaceKey::Folder(PathBuf::from("/p"));
        let dir = backup_dir(tmp.path(), &key);
        let mut b = backup("/tmp/dự án/a b.rs", "x\ny\n");
        b.header.line_ending = LineEnding::CrLf;
        b.header.bom = true;
        b.header.encoding = "UTF-16LE".into();
        b.header.lossy = true;
        write_backup(&dir, &key, &b).unwrap();
        assert_eq!(read_backups(&dir), vec![b]);
    }

    #[test]
    fn header_without_lossy_reads_as_false() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("abc"),
            "{\"version\":1,\"path\":\"/a\",\"mtime\":null,\"size\":null,\"base_sha256\":null,\"encoding\":\"UTF-8\",\"bom\":false,\"line_ending\":\"lf\"}\nabc",
        )
        .unwrap();
        let backups = read_backups(tmp.path());
        assert_eq!(backups.len(), 1);
        assert!(!backups[0].header.lossy);
        assert!(!tmp.path().join("abc.bak").exists());
    }

    #[test]
    fn corrupt_header_is_set_aside() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("abc");
        std::fs::write(&file, "not json\nabc").unwrap();
        assert!(read_backups(tmp.path()).is_empty());
        assert!(tmp.path().join("abc.bak").exists());
        assert!(!file.exists());
    }

    #[test]
    fn wrong_version_is_set_aside() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("abc");
        std::fs::write(
            &file,
            "{\"version\":2,\"path\":\"/a\",\"mtime\":null,\"size\":null,\"base_sha256\":null,\"encoding\":\"UTF-8\",\"bom\":false,\"line_ending\":\"lf\"}\nabc",
        )
        .unwrap();
        assert!(read_backups(tmp.path()).is_empty());
        assert!(tmp.path().join("abc.bak").exists());
    }

    #[test]
    fn remove_backup_drops_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let key = WorkspaceKey::Folder(PathBuf::from("/p"));
        let dir = backup_dir(tmp.path(), &key);
        let b = backup("/p/a.rs", "x");
        write_backup(&dir, &key, &b).unwrap();
        remove_backup(&dir, &b.header.path).unwrap();
        assert!(!dir.exists());
        remove_backup(&dir, &b.header.path).unwrap();
    }

    #[test]
    fn folder_and_file_window_backups_apart() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = WorkspaceKey::Folder(PathBuf::from("/p"));
        let file = WorkspaceKey::File(PathBuf::from("/p/a.rs"));
        let (fd, wd) = (
            backup_dir(tmp.path(), &folder),
            backup_dir(tmp.path(), &file),
        );
        assert_ne!(fd, wd);
        let (a, b) = (backup("/p/a.rs", "folder"), backup("/p/a.rs", "file"));
        write_backup(&fd, &folder, &a).unwrap();
        write_backup(&wd, &file, &b).unwrap();
        assert_eq!(read_backups(&fd), vec![a]);
        assert_eq!(read_backups(&wd), vec![b]);
    }

    #[test]
    fn file_windows_listed() {
        let tmp = tempfile::tempdir().unwrap();
        let file = WorkspaceKey::File(PathBuf::from("/p/a.rs"));
        let folder = WorkspaceKey::Folder(PathBuf::from("/q"));
        write_backup(
            &backup_dir(tmp.path(), &file),
            &file,
            &backup("/p/a.rs", "x"),
        )
        .unwrap();
        write_backup(
            &backup_dir(tmp.path(), &folder),
            &folder,
            &backup("/q/b.rs", "y"),
        )
        .unwrap();
        assert_eq!(
            file_windows_with_backups(tmp.path()),
            vec![PathBuf::from("/p/a.rs")]
        );
    }

    #[test]
    fn backup_dir_of_a_deleted_file_is_unchanged() {
        // On macOS the tempdir is under /var, a symlink to /private/var:
        // the key must not fall back to the uncanonical path.
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("a.rs");
        std::fs::write(&file, "x").unwrap();
        let key = WorkspaceKey::File(file.clone());
        let before = backup_dir(tmp.path(), &key);
        std::fs::remove_file(&file).unwrap();
        assert_eq!(backup_dir(tmp.path(), &key), before);
    }
}
