//! App-wide state that is not a setting, kept as JSON in the app data
//! directory: today only whether the first-run onboarding was shown.

use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::workspace_state::write_atomic;

/// The file format version; a file of any other version is set aside.
const STATE_VERSION: u64 = 1;

/// Where the app data directory keeps the state: `<data_dir>/state.json`.
pub fn state_path(data_dir: &Path) -> PathBuf {
    data_dir.join("state.json")
}

/// Reads the file as a JSON object of version 1. A missing file is
/// `Ok(None)`; one that does not parse, is not an object or has another
/// version is renamed to `state.json.bak` (replacing an older one) and is
/// `Ok(None)` too. A file that cannot be read is an error.
fn read_object(path: &Path) -> io::Result<Option<Map<String, Value>>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    };
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(map))
            if map.get("version").and_then(Value::as_u64) == Some(STATE_VERSION) =>
        {
            Ok(Some(map))
        }
        _ => {
            let mut bak = path.as_os_str().to_owned();
            bak.push(".bak");
            if let Err(err) = std::fs::rename(path, &bak) {
                tracing::warn!(%err, path = %path.display(), "could not set aside app state");
            }
            Ok(None)
        }
    }
}

/// Whether the onboarding was already shown. A missing file or key, or a
/// file that cannot be read, is `false`; a corrupt file is set aside.
pub fn onboarded(data_dir: &Path) -> bool {
    let path = state_path(data_dir);
    match read_object(&path) {
        Ok(map) => map
            .and_then(|map| map.get("onboarded").and_then(Value::as_bool))
            .unwrap_or(false),
        Err(err) => {
            tracing::warn!(%err, path = %path.display(), "could not read app state");
            false
        }
    }
}

/// Records that the onboarding was shown, keeping any other keys the file
/// holds. Creates the data directory when it is missing.
pub fn mark_onboarded(data_dir: &Path) -> io::Result<()> {
    let path = state_path(data_dir);
    let mut map = read_object(&path)?.unwrap_or_default();
    map.insert("version".to_string(), Value::from(STATE_VERSION));
    map.insert("onboarded".to_string(), Value::Bool(true));
    let json = serde_json::to_vec_pretty(&Value::Object(map)).map_err(io::Error::other)?;
    write_atomic(&path, &json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_is_not_onboarded() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!onboarded(dir.path()));
        // A file without the key is not onboarded either.
        std::fs::write(state_path(dir.path()), r#"{"version":1}"#).unwrap();
        assert!(!onboarded(dir.path()));
    }

    #[test]
    fn mark_then_read() {
        let dir = tempfile::tempdir().unwrap();
        mark_onboarded(dir.path()).unwrap();
        assert!(onboarded(dir.path()));
        // Marking again, and keeping keys this version does not know.
        std::fs::write(
            state_path(dir.path()),
            r#"{"version":1,"onboarded":false,"other":[1]}"#,
        )
        .unwrap();
        mark_onboarded(dir.path()).unwrap();
        let value: Value =
            serde_json::from_str(&std::fs::read_to_string(state_path(dir.path())).unwrap())
                .unwrap();
        assert_eq!(value["onboarded"], true);
        assert_eq!(value["version"], 1);
        assert_eq!(value["other"], serde_json::json!([1]));
    }

    #[test]
    fn corrupt_file_becomes_bak() {
        let dir = tempfile::tempdir().unwrap();
        let path = state_path(dir.path());
        std::fs::write(&path, "{").unwrap();
        assert!(!onboarded(dir.path()));
        assert!(!path.exists());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("state.json.bak")).unwrap(),
            "{"
        );
    }

    #[test]
    fn other_version_becomes_bak() {
        let dir = tempfile::tempdir().unwrap();
        let path = state_path(dir.path());
        std::fs::write(&path, r#"{"version":2,"onboarded":true}"#).unwrap();
        assert!(!onboarded(dir.path()));
        assert!(!path.exists());
        assert!(dir.path().join("state.json.bak").exists());
    }

    #[test]
    fn no_tmp_left() {
        let dir = tempfile::tempdir().unwrap();
        mark_onboarded(dir.path()).unwrap();
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("state.json")]);
    }

    #[test]
    fn mark_creates_data_dir() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("a").join("b");
        mark_onboarded(&data).unwrap();
        assert!(onboarded(&data));
    }
}
