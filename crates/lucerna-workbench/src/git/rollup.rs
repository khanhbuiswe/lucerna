//! Maps git statuses to Explorer decoration colors and rolls them up to
//! parent folders. Built once per status snapshot, off the UI thread.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use lucerna_git::FileStatus;

/// Decoration color of a file or folder. Variants are declared in priority
/// order, highest first, so the derived `Ord` makes the highest priority the
/// *smallest* value: a folder takes the `min` of its descendants.
// `Ignored` is produced once ignored files are shown (a later task).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GitColor {
    Conflict,
    Deleted,
    Modified,
    Created,
    Ignored,
}

pub fn color_of(status: FileStatus) -> GitColor {
    match status {
        FileStatus::Conflict => GitColor::Conflict,
        FileStatus::Deleted => GitColor::Deleted,
        FileStatus::Modified | FileStatus::TypeChanged => GitColor::Modified,
        FileStatus::Added | FileStatus::Untracked => GitColor::Created,
    }
}

/// Per-path colors: each file's own, and for every ancestor folder the
/// highest-priority color among its descendants.
#[derive(Debug, Default)]
pub struct Rollup {
    files: HashMap<Arc<Path>, GitColor>,
    dirs: HashMap<Arc<Path>, GitColor>,
}

impl Rollup {
    /// Paths are relative to the worktree root.
    pub fn build(entries: &[(Arc<Path>, FileStatus)]) -> Rollup {
        let mut rollup = Rollup::default();
        for (path, status) in entries {
            let color = color_of(*status);
            rollup.files.insert(path.clone(), color);
            for ancestor in path.ancestors().skip(1) {
                // The empty path is the root; it gets no decoration.
                if ancestor.as_os_str().is_empty() {
                    break;
                }
                match rollup.dirs.get_mut(ancestor) {
                    Some(existing) => *existing = (*existing).min(color),
                    None => {
                        rollup.dirs.insert(Arc::from(ancestor), color);
                    }
                }
            }
        }
        rollup
    }

    pub fn file(&self, path: &Path) -> Option<GitColor> {
        self.files.get(path).copied()
    }

    pub fn dir(&self, path: &Path) -> Option<GitColor> {
        self.dirs.get(path).copied()
    }

    pub fn badge(status: FileStatus) -> &'static str {
        match status {
            FileStatus::Modified => "M",
            FileStatus::Added => "A",
            FileStatus::Deleted => "D",
            FileStatus::Untracked => "U",
            FileStatus::Conflict => "!",
            FileStatus::TypeChanged => "T",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(p: &str, s: FileStatus) -> (Arc<Path>, FileStatus) {
        (Arc::from(Path::new(p)), s)
    }

    #[test]
    fn dir_takes_highest_priority() {
        let r = Rollup::build(&[
            entry("a/b/x.rs", FileStatus::Modified),
            entry("a/y.rs", FileStatus::Conflict),
        ]);
        assert_eq!(r.dir(Path::new("a")), Some(GitColor::Conflict));
        assert_eq!(r.dir(Path::new("a/b")), Some(GitColor::Modified));
        assert_eq!(r.file(Path::new("a/b/x.rs")), Some(GitColor::Modified));
        assert_eq!(r.dir(Path::new("c")), None);
    }

    #[test]
    fn untracked_is_created() {
        let r = Rollup::build(&[entry("n/new.txt", FileStatus::Untracked)]);
        assert_eq!(r.file(Path::new("n/new.txt")), Some(GitColor::Created));
        assert_eq!(r.dir(Path::new("n")), Some(GitColor::Created));
        assert_eq!(color_of(FileStatus::Added), GitColor::Created);
        assert_eq!(color_of(FileStatus::TypeChanged), GitColor::Modified);
        assert_eq!(color_of(FileStatus::Deleted), GitColor::Deleted);
        assert_eq!(Rollup::badge(FileStatus::Conflict), "!");
        assert_eq!(Rollup::badge(FileStatus::Untracked), "U");
    }

    #[test]
    fn root_level_file() {
        let r = Rollup::build(&[entry("Cargo.toml", FileStatus::Deleted)]);
        assert_eq!(r.file(Path::new("Cargo.toml")), Some(GitColor::Deleted));
        assert_eq!(r.dir(Path::new("")), None);
    }
}
