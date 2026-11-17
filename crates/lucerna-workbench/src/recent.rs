//! The folders (and files) opened lately, newest first, kept as JSON in
//! the app data directory for the Welcome screen and Open Recent.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

use crate::workspace_state::{tilde_path, write_atomic};

/// The file format version; a file of any other version is set aside.
const RECENT_VERSION: u32 = 1;

/// How many entries the list keeps.
pub const RECENT_LIMIT: usize = 100;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RecentKind {
    Folder,
    File,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct RecentEntry {
    pub path: PathBuf,
    pub kind: RecentKind,
}

impl RecentEntry {
    /// Whether the path still exists as the kind of thing it was saved as.
    pub(crate) fn exists(&self) -> bool {
        match self.kind {
            RecentKind::Folder => self.path.is_dir(),
            RecentKind::File => self.path.is_file(),
        }
    }
}

/// The title of the box that says a chosen entry is gone. The wording is
/// VS Code's `pathNotExistTitle` (`windowsMainService.ts`, MIT).
pub(crate) const MISSING_TITLE: &str = "Path does not exist";

/// The detail of that box, VS Code's `pathNotExistDetail`, with `path`
/// shown under `home` as `~`.
pub(crate) fn missing_detail(path: &Path, home: Option<&Path>) -> String {
    format!(
        "The path '{}' does not exist on this computer.",
        tilde_path(path, home)
    )
}

/// The recent entries, newest first.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RecentList {
    pub entries: Vec<RecentEntry>,
}

#[derive(Serialize, Deserialize)]
struct RecentFile {
    version: u32,
    entries: Vec<RecentEntry>,
}

/// Where the app data directory keeps the list: `<data_dir>/recent.json`.
pub fn recent_path(data_dir: &Path) -> PathBuf {
    data_dir.join("recent.json")
}

/// Reads the list. A missing file is an empty list; a file that does not
/// parse or has another version is renamed to `recent.json.bak` (replacing
/// an older one) and is an empty list too. A file that cannot be read is
/// an empty list as well; see [`try_load`].
pub fn load(path: &Path) -> RecentList {
    try_load(path).unwrap_or_else(|err| {
        tracing::warn!(%err, path = %path.display(), "could not read recent list");
        RecentList::default()
    })
}

/// [`load`], failing when the file exists but cannot be read, so that a
/// write does not replace a history it never saw.
fn try_load(path: &Path) -> io::Result<RecentList> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(RecentList::default()),
        Err(err) => return Err(err),
    };
    Ok(match serde_json::from_str::<RecentFile>(&text) {
        Ok(file) if file.version == RECENT_VERSION => RecentList {
            entries: file.entries,
        },
        _ => {
            let mut bak = path.as_os_str().to_owned();
            bak.push(".bak");
            if let Err(err) = std::fs::rename(path, &bak) {
                tracing::warn!(%err, path = %path.display(), "could not set aside recent list");
            }
            RecentList::default()
        }
    })
}

/// Held while [`record`], [`remove`] or [`clear`] rewrites the file: each
/// reads it, changes it and writes it back, so two at once in this process
/// would lose one change.
static STORE_WRITE: Mutex<()> = Mutex::new(());

fn write_lock() -> MutexGuard<'static, ()> {
    STORE_WRITE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn save(path: &Path, entries: Vec<RecentEntry>) -> io::Result<()> {
    let file = RecentFile {
        version: RECENT_VERSION,
        entries,
    };
    let json = serde_json::to_vec_pretty(&file).map_err(io::Error::other)?;
    write_atomic(path, &json)
}

/// Puts `entry` first, dropping an older one with the same path. Entries
/// whose path is gone stay, as in VS Code. Fails without writing when the
/// file cannot be read.
pub fn record(data_dir: &Path, entry: RecentEntry) -> io::Result<()> {
    let _writing = write_lock();
    let path = recent_path(data_dir);
    let mut entries = try_load(&path)?.entries;
    entries.retain(|e| e.path != entry.path);
    entries.insert(0, entry);
    entries.truncate(RECENT_LIMIT);
    save(&path, entries)
}

/// Forgets `path`; other entries stay, even when their path is gone. Fails
/// without writing when the file cannot be read.
pub fn remove(data_dir: &Path, path: &Path) -> io::Result<()> {
    let _writing = write_lock();
    let file = recent_path(data_dir);
    let mut entries = try_load(&file)?.entries;
    entries.retain(|e| e.path != path);
    save(&file, entries)
}

/// Empties the list.
pub fn clear(data_dir: &Path) -> io::Result<()> {
    let _writing = write_lock();
    save(&recent_path(data_dir), Vec::new())
}

/// An entry as a list row shows it: its name, and its folder with `~` for
/// `home`.
pub fn display_parts(entry: &RecentEntry, home: Option<&Path>) -> (String, String) {
    let name = entry
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| entry.path.display().to_string());
    let parent = entry
        .path
        .parent()
        .map(|p| tilde_path(p, home))
        .unwrap_or_default();
    (name, parent)
}

/// The app data directory, where the recent list lives (and the
/// workbench keeps its workspace state); `None` when the app has none.
pub struct RecentStore {
    pub data_dir: Option<PathBuf>,
}

impl gpui::Global for RecentStore {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_detail_uses_tilde() {
        assert_eq!(MISSING_TITLE, "Path does not exist");
        assert_eq!(
            missing_detail(Path::new("/Users/me/code/x"), Some(Path::new("/Users/me"))),
            "The path '~/code/x' does not exist on this computer."
        );
    }

    fn folder(path: PathBuf) -> RecentEntry {
        RecentEntry {
            path,
            kind: RecentKind::Folder,
        }
    }

    fn paths(data: &Path) -> Vec<PathBuf> {
        load(&recent_path(data))
            .entries
            .into_iter()
            .map(|e| e.path)
            .collect()
    }

    fn mkdir(base: &Path, name: &str) -> PathBuf {
        let dir = base.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn record_puts_newest_first_and_dedupes() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b) = (mkdir(tmp.path(), "a"), mkdir(tmp.path(), "b"));
        let data = tmp.path().join("data");
        record(&data, folder(a.clone())).unwrap();
        record(&data, folder(b.clone())).unwrap();
        record(&data, folder(a.clone())).unwrap();
        assert_eq!(paths(&data), [a, b]);
    }

    #[test]
    fn record_caps_at_limit() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        let dirs: Vec<_> = (0..=RECENT_LIMIT)
            .map(|i| mkdir(tmp.path(), &format!("d{i}")))
            .collect();
        for dir in &dirs {
            record(&data, folder(dir.clone())).unwrap();
        }
        let got = paths(&data);
        assert_eq!(got.len(), RECENT_LIMIT);
        assert!(!got.contains(&dirs[0]));
    }

    #[test]
    fn concurrent_records_keep_every_entry() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        let mut dirs: Vec<_> = (0..8)
            .map(|i| mkdir(tmp.path(), &format!("d{i}")))
            .collect();
        std::thread::scope(|scope| {
            for dir in &dirs {
                let data = &data;
                scope.spawn(move || {
                    for _ in 0..5 {
                        record(data, folder(dir.clone())).unwrap();
                    }
                });
            }
        });
        let mut got = paths(&data);
        got.sort();
        dirs.sort();
        assert_eq!(got, dirs);
    }

    #[test]
    fn remove_and_clear() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b) = (mkdir(tmp.path(), "a"), mkdir(tmp.path(), "b"));
        let data = tmp.path().join("data");
        record(&data, folder(a.clone())).unwrap();
        record(&data, folder(b.clone())).unwrap();
        remove(&data, &a).unwrap();
        assert_eq!(paths(&data), [b]);
        clear(&data).unwrap();
        assert!(paths(&data).is_empty());
    }

    #[test]
    fn writes_keep_missing_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b) = (mkdir(tmp.path(), "a"), mkdir(tmp.path(), "b"));
        let data = tmp.path().join("data");
        record(&data, folder(a.clone())).unwrap();
        std::fs::remove_dir(&a).unwrap();
        record(&data, folder(b.clone())).unwrap();
        assert_eq!(paths(&data), [b.clone(), a.clone()]);
        remove(&data, &b).unwrap();
        assert_eq!(paths(&data), [a]);
    }

    #[test]
    fn corrupt_or_other_version_is_set_aside() {
        for text in ["{", r#"{"version":2,"entries":[]}"#] {
            let tmp = tempfile::tempdir().unwrap();
            let file = recent_path(tmp.path());
            std::fs::write(&file, text).unwrap();
            assert!(load(&file).entries.is_empty());
            assert!(tmp.path().join("recent.json.bak").exists());
            assert!(!file.exists());
        }
    }

    #[test]
    fn no_tmp_left_after_write() {
        let tmp = tempfile::tempdir().unwrap();
        let a = mkdir(tmp.path(), "a");
        let data = tmp.path().join("data");
        record(&data, folder(a)).unwrap();
        assert!(recent_path(&data).exists());
        let names: Vec<_> = std::fs::read_dir(&data)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["recent.json"]);
    }

    /// Two processes writing at once each use a temporary file of their own.
    #[test]
    fn tmp_file_is_per_process() {
        let tmp = crate::workspace_state::tmp_path(Path::new("/data/recent.json"));
        let pid = std::process::id();
        assert_eq!(tmp, PathBuf::from(format!("/data/recent.json.{pid}.tmp")));
    }

    /// A file that exists but cannot be read is not overwritten: the next
    /// write would lose the whole history.
    #[cfg(unix)]
    #[test]
    fn unreadable_file_is_not_overwritten() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let a = mkdir(tmp.path(), "a");
        let data = tmp.path().join("data");
        record(&data, folder(a.clone())).unwrap();
        let file = recent_path(&data);
        let before = std::fs::read(&file).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read(&file).is_ok() {
            // Running as root: the file is readable anyway.
            return;
        }
        let b = mkdir(tmp.path(), "b");
        let recorded = record(&data, folder(b));
        let removed = remove(&data, &a);
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(recorded.is_err());
        assert!(removed.is_err());
        assert_eq!(std::fs::read(&file).unwrap(), before);
    }

    #[test]
    fn display_parts_uses_tilde() {
        let entry = folder(PathBuf::from("/Users/me/code/x"));
        assert_eq!(
            display_parts(&entry, Some(Path::new("/Users/me"))),
            ("x".to_string(), "~/code".to_string())
        );
        let entry = folder(PathBuf::from("/Users/me/x"));
        assert_eq!(
            display_parts(&entry, Some(Path::new("/Users/me"))),
            ("x".to_string(), "~".to_string())
        );
        assert_eq!(
            tilde_path(Path::new("/Users/me"), Some(Path::new("/Users/me"))),
            "~"
        );
    }
}
