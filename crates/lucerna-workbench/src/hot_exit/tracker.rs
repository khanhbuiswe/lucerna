//! Keeps a window's backups of unsaved documents up to date (spec §5.1):
//! a backup is written [`BACKUP_DELAY`] after the document's last change,
//! off the UI thread, and deleted once the document has no unsaved changes
//! or is closed with Don't Save.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use gpui::{App, AppContext as _, Context, Entity, Subscription, Task};

use super::store::{self, Backup, BackupHeader, WorkspaceKey};
use crate::AppState;
use crate::documents::{BackupSource, DocumentEvent, Documents};

/// How long after a document's last change its backup is written.
pub const BACKUP_DELAY: Duration = Duration::from_millis(1000);

/// Writes and deletes the backups of one window's documents into its
/// backup folder. A GPUI entity owned by the [`crate::Workbench`], made
/// only when the window has a data dir.
pub struct HotExit {
    documents: Entity<Documents>,
    key: WorkspaceKey,
    queue: Arc<Queue>,
    /// The pending write of each document, due [`BACKUP_DELAY`] after its
    /// last change; dropping it cancels the write.
    scheduled: HashMap<Arc<Path>, Task<()>>,
    /// The app is quitting: a document closed now keeps its backup.
    quitting: bool,
    _documents: Subscription,
}

/// The store calls of one backup folder. They run one at a time, since
/// deleting a folder's last backup deletes the folder, which a write of
/// another path at the same moment would lose. Each call has a ticket; a
/// call whose path got a newer ticket before it ran is dropped, so an
/// older text never overwrites a newer one and a write never brings back
/// a backup deleted after it was asked for.
struct Queue {
    dir: PathBuf,
    /// Held for the whole of a store call.
    io: Mutex<()>,
    tickets: Mutex<Tickets>,
}

#[derive(Default)]
struct Tickets {
    next: u64,
    /// The newest ticket of each path.
    latest: HashMap<Arc<Path>, u64>,
    /// The document revision each path's backup was last written from.
    written: HashMap<Arc<Path>, u64>,
}

/// A store call for one path.
enum Op {
    /// Write the backup of the text of `revision`.
    Write {
        source: BackupSource,
        revision: Option<u64>,
    },
    Remove,
}

impl Queue {
    fn tickets(&self) -> std::sync::MutexGuard<'_, Tickets> {
        self.tickets.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A ticket for a new call on `path`, making older ones stale.
    fn ticket(&self, path: &Arc<Path>) -> u64 {
        let mut tickets = self.tickets();
        tickets.next += 1;
        let ticket = tickets.next;
        tickets.latest.insert(path.clone(), ticket);
        ticket
    }

    /// Marks `path`'s backup as not current, though its text is: a write
    /// asked for before now that has not run is dropped, and one running
    /// now does not count as current.
    fn invalidate(&self, path: &Arc<Path>) {
        self.ticket(path);
        self.tickets().written.remove(path);
    }

    /// Runs the call of `ticket` on `path` unless a newer one was asked
    /// for. Blocks while another call runs: call it off the UI thread,
    /// except when quitting.
    fn run(&self, key: &WorkspaceKey, path: &Arc<Path>, ticket: u64, op: Op) -> io::Result<()> {
        let _io = self.io.lock().unwrap_or_else(PoisonError::into_inner);
        if self.tickets().latest.get(path) != Some(&ticket) {
            return Ok(());
        }
        match op {
            Op::Write { source, revision } => {
                let backup = backup_of(path, source);
                store::write_backup(&self.dir, key, &backup)?;
                // Not current if the backup went stale while it wrote.
                let mut tickets = self.tickets();
                if let Some(revision) = revision
                    && tickets.latest.get(path) == Some(&ticket)
                {
                    tickets.written.insert(path.clone(), revision);
                }
                Ok(())
            }
            Op::Remove => {
                {
                    let mut tickets = self.tickets();
                    tickets.written.remove(path);
                    // Nothing is left to order against: a stale call finds
                    // no ticket and is dropped too.
                    tickets.latest.remove(path);
                }
                store::remove_backup(&self.dir, path)
            }
        }
    }
}

/// The backup of `source`; hashing the base is the slow part, which is
/// why this runs off the UI thread.
fn backup_of(path: &Path, source: BackupSource) -> Backup {
    let (mtime, size) = match source.stamp.map(|s| s.to_parts()) {
        Some((mtime, size)) => (mtime, Some(size)),
        None => (None, None),
    };
    Backup {
        header: BackupHeader {
            path: path.to_path_buf(),
            mtime,
            size,
            base_sha256: source.base.map(|b| store::text_sha256(&b.text())),
            encoding: source.encoding.to_owned(),
            bom: source.bom,
            line_ending: source.line_ending,
        },
        text: source.text.text(),
    }
}

impl HotExit {
    /// Keeps the backups of `documents` in `<data_dir>/backups/<key>/`.
    /// With hot exit off, deletes that folder instead and writes nothing.
    pub fn new(
        documents: Entity<Documents>,
        data_dir: PathBuf,
        key: WorkspaceKey,
        cx: &mut App,
    ) -> Entity<HotExit> {
        let dir = store::backup_dir(&data_dir, &key);
        let queue = Arc::new(Queue {
            dir,
            io: Mutex::new(()),
            tickets: Mutex::new(Tickets::default()),
        });
        if !Self::enabled(cx) {
            let queue = queue.clone();
            cx.background_spawn(async move {
                let _io = queue.io.lock().unwrap_or_else(PoisonError::into_inner);
                if let Err(err) = store::remove_all(&queue.dir) {
                    tracing::warn!(%err, "could not delete hot exit backups");
                }
            })
            .detach();
        }
        cx.new(|cx| HotExit {
            _documents: cx.subscribe(&documents, |this: &mut HotExit, _, ev, cx| {
                this.document_event(ev, cx)
            }),
            documents,
            key,
            queue,
            scheduled: HashMap::new(),
            quitting: false,
        })
    }

    /// `files.hotExit` is not `"off"`. An app setting: a folder's own
    /// value is not read.
    pub fn enabled(cx: &App) -> bool {
        cx.global::<AppState>()
            .settings
            .get::<String>("files.hotExit", None)
            .as_deref()
            != Some("off")
    }

    /// The backups the window left when it last quit, to restore as it
    /// opens (spec §5.4). Reads them on this thread.
    pub fn read_backups(&self) -> Vec<Backup> {
        let _io = self.queue.io.lock().unwrap_or_else(PoisonError::into_inner);
        store::read_backups(&self.queue.dir)
    }

    /// The app is quitting: documents closed from now on keep their
    /// backups.
    pub fn set_quitting(&mut self) {
        self.quitting = true;
    }

    fn document_event(&mut self, ev: &DocumentEvent, cx: &mut Context<Self>) {
        match ev {
            DocumentEvent::Opened { path, .. }
            | DocumentEvent::Edited { path, .. }
            | DocumentEvent::DirtyChanged {
                path, dirty: true, ..
            } => {
                if self.documents.read(cx).is_dirty(path) {
                    self.schedule(path.clone(), cx);
                }
            }
            DocumentEvent::DirtyChanged {
                path, dirty: false, ..
            } => self.remove(path, cx),
            DocumentEvent::Saved { path, .. } => {
                if self.documents.read(cx).is_dirty(path) {
                    // Edits made while it wrote are still unsaved: the text
                    // is unchanged, but the backup's base and stamp are now
                    // stale, so it is no longer current.
                    self.queue.invalidate(path);
                    self.schedule(path.clone(), cx);
                } else {
                    self.remove(path, cx);
                }
            }
            // Outside a quit, a document closed with unsaved changes was
            // closed with Don't Save.
            DocumentEvent::Closed { path } => {
                self.scheduled.remove(path);
                if !self.quitting {
                    self.remove(path, cx);
                }
            }
            _ => {}
        }
    }

    /// (Re)starts the delay of `path`'s write.
    fn schedule(&mut self, path: Arc<Path>, cx: &mut Context<Self>) {
        if !Self::enabled(cx) {
            return;
        }
        let key = path.clone();
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(BACKUP_DELAY).await;
            this.update(cx, |this, cx| {
                this.scheduled.remove(&path);
                this.write(&path, cx).detach();
            })
            .ok();
        });
        self.scheduled.insert(key, task);
    }

    /// Writes `path`'s backup in the background, now; resolves to whether
    /// it failed. Nothing to write (no unsaved changes) is no failure.
    fn write(&mut self, path: &Arc<Path>, cx: &mut Context<Self>) -> Task<bool> {
        let documents = self.documents.read(cx);
        let Some(source) = documents.backup_source(path) else {
            return Task::ready(false);
        };
        let revision = documents.revision_of(path);
        let (queue, key, path) = (self.queue.clone(), self.key.clone(), path.clone());
        let ticket = queue.ticket(&path);
        cx.background_spawn(async move {
            let op = Op::Write { source, revision };
            match queue.run(&key, &path, ticket, op) {
                Ok(()) => false,
                Err(err) => {
                    tracing::warn!(path = %path.display(), %err, "could not write hot exit backup");
                    true
                }
            }
        })
    }

    /// Cancels `path`'s pending write and deletes its backup, after any
    /// write already running.
    fn remove(&mut self, path: &Arc<Path>, cx: &mut Context<Self>) {
        self.scheduled.remove(path);
        let (queue, key, path) = (self.queue.clone(), self.key.clone(), path.clone());
        let ticket = queue.ticket(&path);
        cx.background_spawn(async move {
            if let Err(err) = queue.run(&key, &path, ticket, Op::Remove) {
                tracing::warn!(path = %path.display(), %err, "could not delete hot exit backup");
            }
        })
        .detach();
    }

    /// Writes the backup of every document with unsaved changes now, in
    /// the background, and resolves to the paths whose write failed.
    pub fn flush(&mut self, cx: &mut Context<Self>) -> Task<Vec<Arc<Path>>> {
        self.scheduled.clear();
        if !Self::enabled(cx) {
            return Task::ready(Vec::new());
        }
        let paths = self.documents.read(cx).dirty_paths();
        let writes: Vec<(Arc<Path>, Task<bool>)> = paths
            .into_iter()
            .map(|path| {
                let write = self.write(&path, cx);
                (path, write)
            })
            .collect();
        cx.background_spawn(async move {
            let mut failed = Vec::new();
            for (path, write) in writes {
                if write.await {
                    failed.push(path);
                }
            }
            failed
        })
    }

    /// Writes, on this thread, the backups not yet written of the text the
    /// documents now have: pending, still running, or failed. For the
    /// app's quit hook, which cannot wait for tasks.
    pub fn flush_blocking(&mut self, cx: &mut Context<Self>) {
        self.scheduled.clear();
        if !Self::enabled(cx) {
            return;
        }
        let documents = self.documents.read(cx);
        for path in documents.dirty_paths() {
            let revision = documents.revision_of(&path);
            if revision.is_some() && self.queue.tickets().written.get(&path) == revision.as_ref() {
                continue;
            }
            let Some(source) = documents.backup_source(&path) else {
                continue;
            };
            let ticket = self.queue.ticket(&path);
            let op = Op::Write { source, revision };
            if let Err(err) = self.queue.run(&self.key, &path, ticket, op) {
                tracing::warn!(path = %path.display(), %err, "could not write hot exit backup");
            }
        }
    }
}
