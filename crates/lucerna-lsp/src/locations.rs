//! Turning LSP locations into editor positions with a preview line.

use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use lucerna_text::{Buffer, BufferSnapshot, Point};

use crate::position::{PositionEncoding, range_from_lsp, uri_to_path};

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedLocation {
    pub path: Arc<Path>,
    pub range: Range<Point>,
    /// The text of `range.start.row`.
    pub line_text: String,
}

/// Reads and decodes a file from disk. Blocks: call it off the UI thread.
pub(crate) fn read_lines_snapshot(path: &Path) -> Option<BufferSnapshot> {
    Buffer::load(path).ok().map(|buffer| buffer.snapshot())
}

/// Converts LSP ranges to points. Open files come from `open`; the others
/// are read from disk (once per file), so this blocks: call it off the UI
/// thread. Unreadable files are dropped, duplicates removed, and the result
/// is sorted by path, then start, then end.
pub fn resolve_locations(
    locs: Vec<(PathBuf, lsp_types::Range)>,
    encoding: PositionEncoding,
    open: &dyn Fn(&Path) -> Option<BufferSnapshot>,
) -> Vec<ResolvedLocation> {
    let mut snapshots: HashMap<PathBuf, Option<(Arc<Path>, BufferSnapshot)>> = HashMap::new();
    let mut out = Vec::new();
    for (path, range) in locs {
        let entry = snapshots.entry(path).or_insert_with_key(|path| {
            open(path)
                .or_else(|| read_lines_snapshot(path))
                .map(|snapshot| (Arc::from(path.as_path()), snapshot))
        });
        let Some((path, snapshot)) = entry else {
            continue;
        };
        let range = range_from_lsp(snapshot, range, encoding);
        out.push(ResolvedLocation {
            path: Arc::clone(path),
            line_text: snapshot.line(range.start.row).into_owned(),
            range,
        });
    }
    let key = |l: &ResolvedLocation| {
        (
            l.path.clone(),
            l.range.start.row,
            l.range.start.column,
            l.range.end.row,
            l.range.end.column,
        )
    };
    out.sort_by_key(key);
    out.dedup_by_key(|l| key(l));
    out
}

/// `LocationLink` uses `targetSelectionRange`; non-file URIs are dropped.
pub fn locations_from_response(
    resp: Option<lsp_types::GotoDefinitionResponse>,
) -> Vec<(PathBuf, lsp_types::Range)> {
    use lsp_types::GotoDefinitionResponse::{Array, Link, Scalar};
    let pairs: Vec<(lsp_types::Uri, lsp_types::Range)> = match resp {
        None => Vec::new(),
        Some(Scalar(loc)) => vec![(loc.uri, loc.range)],
        Some(Array(locs)) => locs.into_iter().map(|l| (l.uri, l.range)).collect(),
        Some(Link(links)) => links
            .into_iter()
            .map(|l| (l.target_uri, l.target_selection_range))
            .collect(),
    };
    pairs
        .into_iter()
        .filter_map(|(uri, range)| Some((uri_to_path(&uri)?, range)))
        .collect()
}
