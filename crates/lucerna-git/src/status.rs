use crate::GitError;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileStatus {
    Modified,
    Added,
    Deleted,
    Untracked,
    Conflict,
    TypeChanged,
}

#[cfg(unix)]
fn path_from_bytes(b: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    PathBuf::from(std::ffi::OsStr::from_bytes(b))
}

#[cfg(not(unix))]
fn path_from_bytes(b: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(b).into_owned())
}

/// Parses `git status --porcelain=v2 -z --no-renames` output into repo-relative
/// paths. Ignored (`!`) entries are skipped.
pub fn parse_status_v2(out: &[u8]) -> Result<Vec<(PathBuf, FileStatus)>, GitError> {
    let mut result = Vec::new();
    let mut records = out.split(|&b| b == 0);
    while let Some(rec) = records.next() {
        if rec.is_empty() {
            continue;
        }
        let bad = || {
            GitError::Parse(format!(
                "bad status record: {}",
                String::from_utf8_lossy(rec)
            ))
        };
        match rec[0] {
            b'1' => {
                // 1 XY sub mH mI mW hH hI path
                let mut f = rec.splitn(9, |&b| b == b' ');
                let xy = f.nth(1).ok_or_else(bad)?;
                let path = f.nth(6).ok_or_else(bad)?;
                if xy.len() != 2 {
                    return Err(bad());
                }
                let (x, y) = (xy[0], xy[1]);
                let status = if x == b'A' || y == b'A' {
                    FileStatus::Added
                } else if x == b'D' || y == b'D' {
                    FileStatus::Deleted
                } else if x == b'T' || y == b'T' {
                    FileStatus::TypeChanged
                } else {
                    FileStatus::Modified
                };
                result.push((path_from_bytes(path), status));
            }
            b'2' => {
                // Cannot occur with --no-renames; tolerate it: path, then origPath record.
                let mut f = rec.splitn(10, |&b| b == b' ');
                let path = f.nth(9).ok_or_else(bad)?;
                records.next();
                result.push((path_from_bytes(path), FileStatus::Modified));
            }
            b'u' => {
                // u XY sub m1 m2 m3 mW h1 h2 h3 path
                let path = rec.splitn(11, |&b| b == b' ').nth(10).ok_or_else(bad)?;
                result.push((path_from_bytes(path), FileStatus::Conflict));
            }
            b'?' => {
                let path = rec.get(2..).ok_or_else(bad)?;
                result.push((path_from_bytes(path), FileStatus::Untracked));
            }
            b'!' | b'#' => {}
            _ => return Err(bad()),
        }
    }
    Ok(result)
}
