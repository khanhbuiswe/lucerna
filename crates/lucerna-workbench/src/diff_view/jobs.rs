//! Loading the files of a diff tab: debounced reloads, a queue of at most
//! [`MAX_LOAD_JOBS`] background loads, and applying their results.

use std::path::Path;
use std::sync::Arc;

use gpui::{AppContext, Context};
use lucerna_git::GitError;

use super::load::{LoadInput, Loaded, load_file};
use super::{DiffKind, DiffView, MAX_LOAD_JOBS, RELOAD_DEBOUNCE, file_meta};
use crate::excerpt_list::model::FileMeta;
use crate::tasks::guarded;

impl DiffView {
    /// Reloads file `path` once [`RELOAD_DEBOUNCE`] passes with no other
    /// edit or new hunks of any file: a keystroke and the hunks it brings
    /// make one load.
    pub(super) fn schedule_reload(&mut self, path: Arc<Path>, cx: &mut Context<Self>) {
        if !self.files.contains_key(&path) {
            return;
        }
        self.pending_reloads.insert(path);
        // A newer timer replaces (drops) the running one.
        self.reload_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(RELOAD_DEBOUNCE).await;
            this.update(cx, |view, cx| view.flush_reloads(cx)).ok();
        }));
    }

    /// Reloads the files whose text changed since their last load; new
    /// hunks of a buffer already loaded add nothing.
    fn flush_reloads(&mut self, cx: &mut Context<Self>) {
        self.reload_timer = None;
        let documents = self.documents.read(cx);
        let due: Vec<Arc<Path>> = self
            .pending_reloads
            .drain()
            .filter(|path| {
                self.files.get(path).is_some_and(|entry| {
                    !self.queued.contains(path)
                        && documents.revision_of(path) != entry.buffer_revision
                })
            })
            .collect();
        for path in due {
            self.queue_load(path);
        }
        self.start_jobs(cx);
    }

    /// Loads file `path` again, if it is listed.
    pub(super) fn reload(&mut self, path: &Path, cx: &mut Context<Self>) {
        let Some((path, _)) = self.files.get_key_value(path) else {
            return;
        };
        let path = path.clone();
        self.queue_load(path);
        self.start_jobs(cx);
    }

    /// Queues a load of `path`; results of earlier loads are dropped.
    pub(super) fn queue_load(&mut self, path: Arc<Path>) {
        if let Some(entry) = self.files.get_mut(&path) {
            entry.version += 1;
            if self.queued.insert(path.clone()) {
                self.queue.push_back(path);
            }
        }
    }

    /// Starts queued loads up to [`MAX_LOAD_JOBS`] at once.
    pub(super) fn start_jobs(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.git.read(cx).repo() else {
            return;
        };
        while self.jobs.len() < MAX_LOAD_JOBS {
            let Some(path) = self.queue.pop_front() else {
                break;
            };
            self.queued.remove(&path);
            let full_file = matches!(self.kind, DiffKind::File(_));
            let documents = self.documents.read(cx);
            let Some(entry) = self.files.get_mut(&path) else {
                continue;
            };
            let rev = self.rev.clone();
            entry.buffer_revision = documents.revision_of(&path);
            let input = LoadInput {
                repo: repo.clone(),
                abs_path: self.root.join(&path),
                path: path.clone(),
                status: entry.status,
                cached_base: entry.base.clone().filter(|_| entry.base_rev == rev),
                rev: rev.clone(),
                buffer: documents.snapshot_of(&path),
                full_file,
            };
            let version = entry.version;
            self.loads_started += 1;
            let key = path.clone();
            let task = cx.spawn(async move |this, cx| {
                let loaded = cx
                    .background_spawn(async move {
                        guarded("git diff load", || load_file(input))
                            .unwrap_or_else(|| Err(GitError::Parse("panicked".into())))
                    })
                    .await;
                this.update(cx, |view, cx| {
                    view.finish_load(key, version, rev, loaded, cx)
                })
                .ok();
            });
            // A running load of the same file is replaced (and dropped).
            self.jobs.insert(path, task);
            self.peak_jobs = self.peak_jobs.max(self.jobs.len());
        }
    }

    fn finish_load(
        &mut self,
        path: Arc<Path>,
        version: u64,
        rev: Option<String>,
        loaded: Result<Loaded, GitError>,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self.files.get_mut(&path) else {
            return;
        };
        // This load holds the slot: a newer one replaces (drops) it.
        self.jobs.remove(&path);
        if entry.version != version {
            // Edited meanwhile: the newer load is queued.
            self.start_jobs(cx);
            return;
        }
        let loaded = loaded.unwrap_or_else(|err| {
            tracing::warn!(%err, path = %path.display(), "could not diff the file");
            Loaded::default()
        });
        entry.added = loaded.added;
        entry.removed = loaded.removed;
        entry.loaded = true;
        if loaded.base.is_some() {
            entry.base = loaded.base;
            entry.base_rev = rev;
        }
        let meta = FileMeta {
            note: loaded.note,
            ..file_meta(entry.status, Some((loaded.added, loaded.removed)))
        };
        let (source, excerpts) = (loaded.source, loaded.excerpts);
        self.list.update(cx, |list, cx| {
            list.set_text_source(path.clone(), source, cx);
            list.update_results(
                |r| {
                    r.set_meta(path.clone(), meta);
                    r.set_file(path, excerpts);
                },
                cx,
            );
        });
        self.apply_pending_row(cx);
        self.start_jobs(cx);
        cx.notify();
    }
}
