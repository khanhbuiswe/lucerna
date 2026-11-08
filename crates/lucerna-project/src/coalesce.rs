//! Folds a burst of raw filesystem events into per-path changes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::worktree::{Change, ChangeKind};

/// More raw events than this in one batch is cheaper to handle as a rescan.
pub const RESCAN_THRESHOLD: usize = 1000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawEvent {
    Created(PathBuf),
    Removed(PathBuf),
    Modified(PathBuf),
    Renamed {
        from: PathBuf,
        to: PathBuf,
    },
    /// The OS dropped events; the watcher can no longer be trusted.
    Overflow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Coalesced {
    Changes(Vec<Change>),
    Rescan,
}

/// Coalesces `events` (absolute paths) into root-relative changes, in the
/// order each path was first touched.
///
/// A rename is expanded to Removed(from), Removed(to), Added(to): the target
/// of a rename may have replaced an existing file, which is what an atomic
/// write is. Removed-then-Added folds to Modified, so an atomic write reports
/// only `Modified`. A rename into a fresh name therefore also comes out as
/// `Modified`; the worktree reconciles kinds against its snapshot.
pub fn coalesce(events: Vec<RawEvent>, root: &Path) -> Coalesced {
    if events.len() > RESCAN_THRESHOLD || events.contains(&RawEvent::Overflow) {
        return Coalesced::Rescan;
    }
    let mut order: Vec<Arc<Path>> = Vec::new();
    let mut state: HashMap<Arc<Path>, Option<ChangeKind>> = HashMap::new();
    let mut apply = |abs: &Path, kind: ChangeKind| {
        let Ok(rel) = abs.strip_prefix(root) else {
            return;
        };
        if rel.as_os_str().is_empty() {
            return;
        }
        let slot = match state.get_mut(rel) {
            Some(slot) => slot,
            None => {
                let rel: Arc<Path> = Arc::from(rel);
                order.push(rel.clone());
                state.entry(rel).or_insert(None)
            }
        };
        *slot = fold(*slot, kind);
    };
    for event in &events {
        match event {
            RawEvent::Created(p) => apply(p, ChangeKind::Added),
            RawEvent::Removed(p) => apply(p, ChangeKind::Removed),
            RawEvent::Modified(p) => apply(p, ChangeKind::Modified),
            RawEvent::Renamed { from, to } => {
                apply(from, ChangeKind::Removed);
                apply(to, ChangeKind::Removed);
                apply(to, ChangeKind::Added);
            }
            RawEvent::Overflow => unreachable!("handled above"),
        }
    }
    let changes = order
        .into_iter()
        .filter_map(|path| {
            let kind = state[&path]?;
            Some(Change { path, kind })
        })
        .collect();
    Coalesced::Changes(changes)
}

/// Net effect of `next` following `current` (`None`: nothing net yet).
fn fold(current: Option<ChangeKind>, next: ChangeKind) -> Option<ChangeKind> {
    use ChangeKind::{Added, Modified, Removed};
    match (current, next) {
        (Some(Added), Removed) => None,
        (Some(Removed), Added) => Some(Modified),
        (Some(Added), Modified) => Some(Added),
        (Some(Modified), Removed) => Some(Removed),
        (_, next) => Some(next),
    }
}
