//! Document events and file changes sent to the servers: `didOpen`,
//! `didChange`, `didClose` and `didChangeWatchedFiles`.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use gpui::{Context, Entity};
use lsp_types::notification::DidChangeWatchedFiles;
use lsp_types::{DidChangeWatchedFilesParams, FileChangeType};
use lucerna_lsp::sync::too_large_for_lsp;
use lucerna_lsp::{LanguageServer, ServerKey, Slot, find_root, select_servers};
use lucerna_project::{ChangeKind, WorktreeEvent};

use super::{LanguageServers, LanguageServersEvent, OpenDoc, selection};
use crate::AppState;
use crate::documents::{DocState, DocumentEvent, Documents};
use crate::tasks::guarded;
use crate::workspace_state::absolute_path;

const WATCHED_FILES_BATCH: Duration = Duration::from_millis(100);

impl LanguageServers {
    /// Follows the files on disk: documents removed from disk are closed in
    /// their servers, and changes matching a server's watch registrations
    /// are sent as `workspace/didChangeWatchedFiles`, in 100 ms batches.
    /// Call it after [`Documents::on_worktree_event`] has seen the event.
    pub fn on_worktree_event(&mut self, ev: &WorktreeEvent, cx: &mut Context<Self>) {
        let documents = self.documents.read(cx);
        let gone: Vec<Arc<Path>> = self
            .docs
            .iter()
            .filter(|(path, doc)| !doc.deleted && documents.state(path) == Some(&DocState::Deleted))
            .map(|(path, _)| path.clone())
            .collect();
        for path in gone {
            let servers = self.running_for(&path);
            self.sync.close(&path, &servers);
            if let Some(doc) = self.docs.get_mut(&path) {
                doc.deleted = true;
            }
        }
        let WorktreeEvent::Changed(changes) = ev else {
            return;
        };
        if !self.trusted {
            return;
        }
        self.watched_changes.extend(changes.iter().map(|change| {
            let typ = match change.kind {
                ChangeKind::Added => FileChangeType::CREATED,
                ChangeKind::Modified => FileChangeType::CHANGED,
                ChangeKind::Removed => FileChangeType::DELETED,
            };
            (absolute_path(&self.root, &change.path), typ)
        }));
        if !self.watched_flush_pending {
            self.watched_flush_pending = true;
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(WATCHED_FILES_BATCH).await;
                this.update(cx, |servers, _| servers.flush_watched_files())
                    .ok();
            })
            .detach();
        }
    }

    pub(super) fn on_document_event(
        &mut self,
        _: Entity<Documents>,
        ev: &DocumentEvent,
        cx: &mut Context<Self>,
    ) {
        match ev {
            DocumentEvent::Opened {
                path,
                language_id,
                snapshot,
            } => {
                // `push_diagnostics` converts the raw entries.
                self.docs.insert(
                    path.clone(),
                    OpenDoc {
                        language_id: language_id.clone(),
                        snapshot: snapshot.clone(),
                        keys: Vec::new(),
                        selection: None,
                        search: 0,
                        _search_task: None,
                        deleted: false,
                    },
                );
                self.push_diagnostics(path, cx);
                self.find_servers(path, cx);
            }
            DocumentEvent::Reloaded { path, snapshot } => {
                // `push_diagnostics` converts the raw entries.
                let Some(doc) = self.docs.get_mut(path) else {
                    return;
                };
                doc.snapshot = snapshot.clone();
                let was_deleted = std::mem::take(&mut doc.deleted);
                self.push_diagnostics(path, cx);
                if was_deleted {
                    self.open_in_servers(path, cx);
                } else {
                    let servers = self.running_for(path);
                    if !servers.is_empty() {
                        self.sync.change(path, snapshot, &servers);
                        cx.emit(LanguageServersEvent::DocumentChanged(path.clone()));
                    }
                }
            }
            DocumentEvent::Closed { path } => {
                let servers = self.running_for(path);
                self.sync.close(path, &servers);
                self.docs.remove(path);
            }
            DocumentEvent::LanguageChanged {
                path,
                language_id,
                snapshot,
            } => {
                // Closed with the old servers (`didClose`), then opened
                // with the new language's.
                self.set_keys(path, Vec::new(), cx);
                let Some(doc) = self.docs.get_mut(path) else {
                    return;
                };
                doc.language_id = language_id.clone();
                doc.snapshot = snapshot.clone();
                self.find_servers(path, cx);
            }
        }
    }

    /// Picks the servers of `path` by its language's settings and looks
    /// for their roots (and the file's size limits) off the UI thread.
    pub(super) fn find_servers(&mut self, path: &Arc<Path>, cx: &mut Context<Self>) {
        if !self.trusted {
            return;
        }
        let Some(doc) = self.docs.get_mut(path) else {
            return;
        };
        doc.search += 1;
        let search = doc.search;
        let language = doc.language_id.clone();
        doc.selection = language.as_deref().map(|lang| selection(lang, cx));
        let adapters = match (&language, &doc.selection) {
            (Some(lang), Some(selection)) if selection.enabled => {
                select_servers(&selection.servers, &self.adapters, lang)
            }
            _ => Vec::new(),
        };
        if adapters.is_empty() {
            doc._search_task = None;
            self.set_keys(path, Vec::new(), cx);
            return;
        }
        let settings = &cx.global::<AppState>().settings;
        let max_bytes = settings
            .get::<u64>("lucerna.largeFile.maxBytes", language.as_deref())
            .unwrap_or(20 * 1024 * 1024);
        let max_lines = settings
            .get::<u32>("lucerna.largeFile.maxLines", language.as_deref())
            .unwrap_or(300_000);
        let (snapshot, file, root) = (doc.snapshot.clone(), path.clone(), self.root.clone());
        let work = cx.background_executor().spawn(async move {
            guarded("lsp-find-servers", move || {
                if too_large_for_lsp(&snapshot, max_bytes, max_lines) {
                    return Vec::new();
                }
                adapters
                    .iter()
                    .map(|adapter| ServerKey {
                        root: find_root(&**adapter, &file, &root).into(),
                        server: adapter.name(),
                    })
                    .collect()
            })
            .unwrap_or_default()
        });
        let path = path.clone();
        doc._search_task = Some(cx.spawn(async move |this, cx| {
            let keys = work.await;
            this.update(cx, |servers, cx| {
                let current = servers
                    .docs
                    .get(&path)
                    .is_some_and(|doc| doc.search == search);
                if current && servers.trusted {
                    servers.set_keys(&path, keys, cx);
                }
            })
            .ok();
        }));
    }

    /// Gives `path` the servers `keys`: closes it in servers it no longer
    /// has, starts missing servers and opens it in running ones.
    fn set_keys(&mut self, path: &Arc<Path>, keys: Vec<ServerKey>, cx: &mut Context<Self>) {
        let Some(doc) = self.docs.get_mut(path) else {
            return;
        };
        let old = std::mem::replace(&mut doc.keys, keys.clone());
        let dropped: Vec<(ServerKey, Arc<LanguageServer>)> = old
            .into_iter()
            .filter(|key| !keys.contains(key))
            .filter_map(|key| match self.registry.slot(&key) {
                Some(Slot::Running(server)) => Some((key, server.clone())),
                _ => None,
            })
            .collect();
        self.sync.close(path, &dropped);
        for key in keys {
            if self.registry.slot(&key).is_none() {
                self.start(key, cx);
            }
        }
        self.push_diagnostics(path, cx);
        self.open_in_servers(path, cx);
    }

    /// `didOpen` for `path` in each of its running servers that does not
    /// have it yet.
    pub(super) fn open_in_servers(&mut self, path: &Arc<Path>, cx: &mut Context<Self>) {
        let servers = self.running_for(path);
        let Some(doc) = self.docs.get(path) else {
            return;
        };
        let Some(language) = doc.language_id.as_deref() else {
            return;
        };
        if !self.trusted || doc.deleted {
            return;
        }
        let mut opened = false;
        for (key, server) in servers {
            if self.sync.is_open(path, &key) {
                continue;
            }
            let Some(adapter) = self.registry.adapter(key.server) else {
                continue;
            };
            let language_id = adapter.lsp_language_id(language);
            self.sync
                .open(path, &language_id, &doc.snapshot, &[(key, server)]);
            opened = true;
        }
        if opened {
            cx.emit(LanguageServersEvent::DocumentChanged(path.clone()));
        }
    }

    fn flush_watched_files(&mut self) {
        self.watched_flush_pending = false;
        let changes = std::mem::take(&mut self.watched_changes);
        if !self.trusted {
            return;
        }
        for (key, events) in self.watched.events_for(&changes) {
            if let Some(Slot::Running(server)) = self.registry.slot(&key) {
                server.notify::<DidChangeWatchedFiles>(DidChangeWatchedFilesParams {
                    changes: events,
                });
            }
        }
    }
}
