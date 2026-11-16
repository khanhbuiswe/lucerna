//! The windows open now, kept in `<data dir>/windows.json` so the next
//! start reopens them all (spec §6.6 of the multi-window design). The
//! router writes the file in the background after every change to its
//! windows, and [`save_now`] once more, on this thread, as the app quits.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use gpui::App;
use serde::{Deserialize, Serialize};

use crate::recent::RecentKind;
use crate::router::{self, WindowKind, app_data_dir};
use crate::startup::RestoreWindows;
use crate::workspace_state::write_atomic;

/// The file format version; a file of any other version is read as empty.
const SESSION_VERSION: u32 = 1;

/// One workbench window: a folder, or a file opened alone.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SessionWindow {
    pub kind: RecentKind,
    pub path: PathBuf,
}

/// The workbench windows, most recently used first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Session {
    pub windows: Vec<SessionWindow>,
}

#[derive(Serialize, Deserialize)]
struct SessionFile {
    version: u32,
    windows: Vec<SessionWindow>,
}

/// Where the app data directory keeps the session: `<data_dir>/windows.json`.
pub fn session_path(data_dir: &Path) -> PathBuf {
    data_dir.join("windows.json")
}

/// Reads the session. A missing file, one that does not parse or has
/// another version, or one that cannot be read is an empty session; the
/// next write replaces it.
pub fn load(path: &Path) -> Session {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Session::default(),
        Err(err) => {
            tracing::warn!(%err, path = %path.display(), "could not read the window list");
            return Session::default();
        }
    };
    match serde_json::from_str::<SessionFile>(&text) {
        Ok(file) if file.version == SESSION_VERSION => Session {
            windows: file.windows,
        },
        Ok(file) => {
            tracing::warn!(version = file.version, path = %path.display(), "unknown window list version");
            Session::default()
        }
        Err(err) => {
            tracing::warn!(%err, path = %path.display(), "could not parse the window list");
            Session::default()
        }
    }
}

/// Writes `session` atomically (a temporary file named after the process,
/// then a rename, as `recent.json` is written).
pub fn save(path: &Path, session: &Session) -> io::Result<()> {
    let file = SessionFile {
        version: SESSION_VERSION,
        windows: session.windows.clone(),
    };
    let json = serde_json::to_vec_pretty(&file).map_err(io::Error::other)?;
    write_atomic(path, &json)
}

/// The windows a start reopens under `mode`, most recently used first:
/// those of `session` whose path still `exists`.
pub fn to_restore(
    session: &Session,
    mode: RestoreWindows,
    exists: impl Fn(&Path) -> bool,
) -> Vec<SessionWindow> {
    let existing = session.windows.iter().filter(|w| exists(&w.path)).cloned();
    match mode {
        RestoreWindows::Preserve | RestoreWindows::All => existing.collect(),
        RestoreWindows::Folders => existing.filter(|w| w.kind == RecentKind::Folder).collect(),
        RestoreWindows::One => existing.take(1).collect(),
        RestoreWindows::None => Vec::new(),
    }
}

/// The router's workbench windows now, most recently used first; Welcome
/// and Get Started windows are left out.
pub(crate) fn current(cx: &App) -> Session {
    let windows = router::windows(cx)
        .into_iter()
        .filter_map(|(_, kind)| match kind {
            WindowKind::Welcome => None,
            WindowKind::Folder(path) => Some(SessionWindow {
                kind: RecentKind::Folder,
                path,
            }),
            WindowKind::File(path) => Some(SessionWindow {
                kind: RecentKind::File,
                path,
            }),
        })
        .collect();
    Session { windows }
}

/// Numbers the writes in the order they were asked for, across the app.
static NEXT_WRITE: AtomicU64 = AtomicU64::new(1);

/// The number of the last write to each session file: a background write
/// that finishes after a newer one does not undo it.
static WRITTEN: Mutex<Option<HashMap<PathBuf, u64>>> = Mutex::new(None);

/// A number for a write asked for now, larger than any before it.
pub(crate) fn next_write() -> u64 {
    NEXT_WRITE.fetch_add(1, Ordering::Relaxed)
}

/// Writes `session` to `path` unless a write asked for after `write` (see
/// [`next_write`]) already did.
pub(crate) fn save_in_order(path: &Path, session: &Session, write: u64) -> io::Result<()> {
    let mut written = WRITTEN
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let written = written.get_or_insert_with(HashMap::new);
    if written.get(path).is_some_and(|last| *last > write) {
        return Ok(());
    }
    save(path, session)?;
    written.insert(path.to_path_buf(), write);
    Ok(())
}

/// Writes the windows open now, on this thread: the last write before the
/// app quits (spec §6.6). Nothing without an app data directory.
pub fn save_now(cx: &mut App) {
    let Some(data_dir) = app_data_dir(cx) else {
        return;
    };
    let session = current(cx);
    if let Err(err) = save_in_order(&session_path(&data_dir), &session, next_write()) {
        tracing::warn!(%err, "could not write the window list");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(kind: RecentKind, path: &str) -> SessionWindow {
        SessionWindow {
            kind,
            path: PathBuf::from(path),
        }
    }

    fn session() -> Session {
        Session {
            windows: vec![
                window(RecentKind::File, "/x.rs"),
                window(RecentKind::Folder, "/gone"),
                window(RecentKind::Folder, "/a"),
                window(RecentKind::Folder, "/b"),
            ],
        }
    }

    fn paths(windows: &[SessionWindow]) -> Vec<&str> {
        windows.iter().map(|w| w.path.to_str().unwrap()).collect()
    }

    fn exists(path: &Path) -> bool {
        path != Path::new("/gone")
    }

    #[test]
    fn session_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = session_path(dir.path());
        assert_eq!(path, dir.path().join("windows.json"));
        save(&path, &session()).unwrap();
        assert_eq!(load(&path), session());
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(json["version"], 1);
        assert_eq!(json["windows"][0]["kind"], "file");
        assert_eq!(json["windows"][2]["kind"], "folder");
        assert_eq!(json["windows"][2]["path"], "/a");
    }

    #[test]
    fn session_load_corrupt_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = session_path(dir.path());
        assert_eq!(load(&path), Session::default(), "no file");
        for text in [
            "not json",
            r#"{"version":2}"#,
            r#"{"version":2,"windows":[{"kind":"folder","path":"/a"}]}"#,
        ] {
            std::fs::write(&path, text).unwrap();
            assert_eq!(load(&path), Session::default(), "{text}");
        }
        // The next write replaces it.
        save(&path, &session()).unwrap();
        assert_eq!(load(&path), session());
    }

    #[test]
    fn to_restore_all_skips_missing() {
        let all = to_restore(&session(), RestoreWindows::All, exists);
        assert_eq!(paths(&all), ["/x.rs", "/a", "/b"]);
        let preserve = to_restore(&session(), RestoreWindows::Preserve, exists);
        assert_eq!(preserve, all);
    }

    #[test]
    fn to_restore_folders_only() {
        let folders = to_restore(&session(), RestoreWindows::Folders, exists);
        assert_eq!(paths(&folders), ["/a", "/b"]);
    }

    #[test]
    fn to_restore_one_takes_first() {
        let one = to_restore(&session(), RestoreWindows::One, exists);
        assert_eq!(paths(&one), ["/x.rs"]);
        let one = to_restore(&session(), RestoreWindows::One, |p| p != Path::new("/x.rs"));
        assert_eq!(paths(&one), ["/gone"], "the first that exists");
    }

    #[test]
    fn to_restore_none_is_empty() {
        assert!(to_restore(&session(), RestoreWindows::None, exists).is_empty());
    }

    #[test]
    fn older_write_does_not_undo_newer() {
        let dir = tempfile::tempdir().unwrap();
        let path = session_path(dir.path());
        let (older, newer) = (next_write(), next_write());
        save_in_order(&path, &session(), newer).unwrap();
        save_in_order(&path, &Session::default(), older).unwrap();
        assert_eq!(load(&path), session());
    }
}
