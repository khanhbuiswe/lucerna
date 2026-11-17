//! Finding a server's project root from its root markers.

use std::fs;
use std::path::{Path, PathBuf};

use globset::{GlobBuilder, GlobMatcher};
use serde::Deserialize;

use crate::table::ServerDef;

/// Which marker directory wins when several enclose a file.
///
/// Outermost is the default, as in Helix and Zed: libraries inside the
/// project (`vendor/`, `node_modules/`, `deps/`) carry their own markers,
/// and the nearest one would start a second server for them.
#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum RootSearch {
    /// The nearest directory containing any marker.
    Nearest,
    /// The outermost directory containing any marker.
    #[default]
    Outermost,
    /// Markers are ranked by their order: the nearest directory containing
    /// the first marker wins, then the second, and so on.
    #[serde(rename = "ordered")]
    NearestByMarker,
}

/// A root marker: a file or directory name, or a glob (`*.sln`) over the
/// names of a directory's entries.
enum Marker<'a> {
    Name(&'a str),
    Glob(GlobMatcher),
    /// A glob that does not parse: matches nothing.
    Invalid,
}

impl<'a> Marker<'a> {
    fn new(marker: &'a str) -> Marker<'a> {
        if !marker.contains(['*', '?']) {
            return Marker::Name(marker);
        }
        // Case-insensitive where file names are (Windows).
        match GlobBuilder::new(marker)
            .case_insensitive(cfg!(windows))
            .build()
        {
            Ok(glob) => Marker::Glob(glob.compile_matcher()),
            Err(_) => Marker::Invalid,
        }
    }

    fn in_dir(&self, dir: &Path) -> bool {
        match self {
            Marker::Name(name) => dir.join(name).exists(),
            Marker::Glob(glob) => fs::read_dir(dir).is_ok_and(|entries| {
                entries
                    .filter_map(Result::ok)
                    .any(|entry| glob.is_match(entry.file_name()))
            }),
            Marker::Invalid => false,
        }
    }
}

/// The project root `def` uses for `file`. Never above `workspace_root`;
/// the workspace root when no marker is found. `None` when the file lies
/// outside the workspace: no server is ever started for such a root (it is
/// not trusted, and a library file would index a whole registry); the file
/// joins an already running server instead. Markers containing `*` or `?`
/// are globs over the entry names of each directory.
pub fn find_root(def: &ServerDef, file: &Path, workspace_root: &Path) -> Option<PathBuf> {
    let parent = file.parent().unwrap_or(file);
    if !parent.starts_with(workspace_root) {
        return None;
    }
    // From the file's directory up to and including the workspace root.
    let dirs: Vec<&Path> = parent
        .ancestors()
        .take_while(|dir| dir.starts_with(workspace_root))
        .collect();
    let markers: Vec<Marker<'_>> = def.root_markers.iter().map(|m| Marker::new(m)).collect();
    let found = match def.root_search {
        RootSearch::Nearest => dirs
            .iter()
            .find(|dir| markers.iter().any(|m| m.in_dir(dir))),
        RootSearch::Outermost => dirs
            .iter()
            .rev()
            .find(|dir| markers.iter().any(|m| m.in_dir(dir))),
        RootSearch::NearestByMarker => markers
            .iter()
            .find_map(|m| dirs.iter().find(|dir| m.in_dir(dir))),
    };
    Some(found.map_or_else(|| workspace_root.to_path_buf(), |dir| dir.to_path_buf()))
}
