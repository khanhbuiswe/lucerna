//! Recursive filesystem watcher with a debounce loop.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError, unbounded};
use notify::event::{EventKind, ModifyKind};
use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher as _};

use crate::coalesce::{Coalesced, RawEvent, coalesce};
use crate::worktree::io_reason;

/// A batch ends once no event arrived for this long...
pub const DEBOUNCE: Duration = Duration::from_millis(50);
/// ...or when it has been collecting for this long.
const MAX_BATCH: Duration = Duration::from_millis(100);

/// Why a watcher could not start, worded for the user: `notify`'s text
/// for the error without the ` about ["/path", ...]` it appends when the
/// error names paths, and an I/O error without its ` (os error N)`.
pub fn watch_error_reason(err: &notify::Error) -> String {
    if let notify::ErrorKind::Io(io) = &err.kind {
        return io_reason(io);
    }
    let text = err.to_string();
    if err.paths.is_empty() {
        return text;
    }
    let paths = format!(" about {:?}", err.paths);
    match text.strip_suffix(&paths) {
        Some(reason) => reason.to_string(),
        None => text,
    }
}

/// Keeps the OS watcher and the debounce thread alive; dropping it stops both.
pub(crate) struct Watcher {
    // Dropped first, which closes the event channel and ends the thread.
    _watcher: RecommendedWatcher,
    _thread: JoinHandle<()>,
}

impl Watcher {
    /// Watches `root` recursively and calls `on_batch` with each coalesced
    /// batch from the debounce thread.
    pub(crate) fn start(
        root: &Path,
        on_batch: impl Fn(Coalesced) + Send + 'static,
    ) -> notify::Result<Watcher> {
        // FSEvents reports real paths (/private/var/...), so match against
        // the canonical root.
        let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        let (tx, rx) = unbounded();
        let mut watcher = notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        })?;
        watcher.watch(&root, RecursiveMode::Recursive)?;
        let thread = thread::Builder::new()
            .name("lucerna-watch".into())
            .spawn(move || debounce_loop(&rx, &root, &on_batch))
            .map_err(notify::Error::io)?;
        Ok(Watcher {
            _watcher: watcher,
            _thread: thread,
        })
    }
}

fn debounce_loop(rx: &Receiver<notify::Result<Event>>, root: &Path, on_batch: &dyn Fn(Coalesced)) {
    // Block on the first event of a batch; a closed channel ends the loop.
    while let Ok(first) = rx.recv() {
        let started = Instant::now();
        let mut batch = vec![first];
        let mut closed = false;
        loop {
            let left = MAX_BATCH.saturating_sub(started.elapsed()).min(DEBOUNCE);
            if left.is_zero() {
                break;
            }
            match rx.recv_timeout(left) {
                Ok(event) => batch.push(event),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => {
                    closed = true;
                    break;
                }
            }
        }
        let result = catch_unwind(AssertUnwindSafe(|| {
            let raw: Vec<RawEvent> = batch.into_iter().flat_map(to_raw).collect();
            if !raw.is_empty() {
                on_batch(coalesce(raw, root));
            }
        }));
        if let Err(payload) = result {
            let message = panic_message(payload.as_ref());
            tracing::error!(%message, "file watcher batch panicked");
        }
        if closed {
            return;
        }
    }
}

/// Readable text of a caught panic payload.
pub(crate) fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "non-string panic payload".into())
}

/// Maps a notify result to raw events.
///
/// FSEvents merges flags and reports renames and atomic writes ambiguously
/// (for example `Modify(Name(Any))` for both sides of a rename, or a create
/// and a remove folded into one event). So the event kind is only a hint: the
/// path's state on disk right now decides. A missing path is `Removed`. An
/// existing path is `Created` for create/rename-ish kinds and `Modified`
/// otherwise; if that is wrong (say the path was already known) the worktree
/// corrects it against its snapshot. Notify errors and "rescan" flags become
/// `Overflow`, since events may have been lost.
fn to_raw(res: notify::Result<Event>) -> Vec<RawEvent> {
    let event = match res {
        Ok(event) => event,
        Err(err) => {
            tracing::warn!(%err, "file watcher error; forcing rescan");
            return vec![RawEvent::Overflow];
        }
    };
    if event.need_rescan() {
        return vec![RawEvent::Overflow];
    }
    let present_kind = match event.kind {
        EventKind::Access(_) => return Vec::new(),
        EventKind::Create(_)
        | EventKind::Remove(_)
        | EventKind::Modify(ModifyKind::Name(_))
        | EventKind::Any => PresentAs::Created,
        EventKind::Modify(_) | EventKind::Other => PresentAs::Modified,
    };
    event
        .paths
        .into_iter()
        .map(|path: PathBuf| {
            if path.symlink_metadata().is_err() {
                RawEvent::Removed(path)
            } else if present_kind == PresentAs::Created {
                RawEvent::Created(path)
            } else {
                RawEvent::Modified(path)
            }
        })
        .collect()
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum PresentAs {
    Created,
    Modified,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watch_error_reason_leaves_out_paths_and_os_error_codes() {
        let path = PathBuf::from("/home/u/proj/node_modules/x/y");
        let limit = notify::Error::new(notify::ErrorKind::MaxFilesWatch).add_path(path.clone());
        assert!(limit.to_string().contains("about"));
        assert_eq!(watch_error_reason(&limit), "OS file watch limit reached.");
        let missing = notify::Error::path_not_found().add_path(path.clone());
        assert_eq!(watch_error_reason(&missing), "No path was found.");
        let two = notify::Error::generic("could not start").set_paths(vec![path.clone(), path]);
        assert_eq!(watch_error_reason(&two), "could not start");
        assert_eq!(
            watch_error_reason(&notify::Error::path_not_found()),
            "No path was found."
        );
        let io =
            notify::Error::io(std::io::Error::from_raw_os_error(24)).add_path(PathBuf::from("/p"));
        let reason = watch_error_reason(&io);
        assert!(
            !reason.contains("os error") && !reason.contains("about"),
            "{reason}"
        );
        assert!(!reason.is_empty());
    }
}
