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
                        too_large: false,
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
                let (max_bytes, max_lines) = large_file_limits(doc.language_id.as_deref(), cx);
                // O(1): sizes only; `find_servers` checks long lines too.
                let over =
                    snapshot.len_bytes() as u64 > max_bytes || snapshot.line_count() > max_lines;
                doc.snapshot = snapshot.clone();
                let was_deleted = std::mem::take(&mut doc.deleted);
                let was_too_large = doc.too_large;
                let grew_too_large = over && !was_too_large;
                if grew_too_large {
                    doc.too_large = true;
                    doc.search += 1;
                    doc._search_task = None;
                }
                self.push_diagnostics(path, cx);
                if over {
                    // Grew past the limit: no more full-text `didChange`
                    // built here; it leaves its servers until it shrinks.
                    if grew_too_large {
                        self.set_keys(path, Vec::new(), cx);
                    }
                } else if was_too_large {
                    self.find_servers(path, cx);
                } else if was_deleted {
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
            // The servers parse for themselves.
            DocumentEvent::Parsed { .. } => {}
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
            doc.too_large = false;
            self.set_keys(path, Vec::new(), cx);
            return;
        }
        let (max_bytes, max_lines) = large_file_limits(language.as_deref(), cx);
        let (snapshot, file, root) = (doc.snapshot.clone(), path.clone(), self.root.clone());
        // A file outside the workspace (a library, the standard library)
        // never starts a server: it joins one already running here.
        let outside = (!path.starts_with(&self.root)).then(|| {
            let running: Vec<&ServerKey> = self.registry.running().map(|(key, _)| key).collect();
            external_keys(
                adapters.iter().map(|adapter| adapter.name()),
                self.origins.get(&**path),
                &running,
                &self.root,
            )
        });
        let work = cx.background_executor().spawn(async move {
            guarded("lsp-find-servers", move || {
                if too_large_for_lsp(&snapshot, max_bytes, max_lines) {
                    return None;
                }
                if let Some(keys) = outside {
                    return Some(keys);
                }
                adapters
                    .iter()
                    .filter_map(|adapter| {
                        Some(ServerKey {
                            root: find_root(&**adapter, &file, &root)?.into(),
                            server: adapter.name(),
                        })
                    })
                    .collect::<Vec<_>>()
                    .into()
            })
            // A panic finds no servers but is not a size limit.
            .unwrap_or_else(|| Some(Vec::new()))
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
                    if let Some(doc) = servers.docs.get_mut(&path) {
                        doc.too_large = keys.is_none();
                    }
                    servers.set_keys(&path, keys.unwrap_or_default(), cx);
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

/// `lucerna.largeFile.maxBytes` and `.maxLines` for `language`.
fn large_file_limits(language: Option<&str>, cx: &gpui::App) -> (u64, u32) {
    let settings = &cx.global::<AppState>().settings;
    let max_bytes = settings
        .get::<u64>("lucerna.largeFile.maxBytes", language)
        .unwrap_or(20 * 1024 * 1024);
    let max_lines = settings
        .get::<u32>("lucerna.largeFile.maxLines", language)
        .unwrap_or(300_000);
    (max_bytes, max_lines)
}

/// The servers a file outside the workspace joins, one per selected server
/// name: the server whose answer led to the file (`origin`) while it runs,
/// else a running server of that name, the one rooted at the workspace
/// first; none when no such server runs. Never a new key.
fn external_keys(
    servers: impl Iterator<Item = &'static str>,
    origin: Option<&ServerKey>,
    running: &[&ServerKey],
    workspace_root: &Path,
) -> Vec<ServerKey> {
    servers
        .filter_map(|server| {
            if let Some(origin) = origin.filter(|o| o.server == server && running.contains(o)) {
                return Some(origin.clone());
            }
            running
                .iter()
                .filter(|key| key.server == server)
                .min_by_key(|key| (&*key.root != workspace_root, key.root.clone()))
                .map(|key| (*key).clone())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(root: &str, server: &'static str) -> ServerKey {
        ServerKey {
            root: Arc::from(Path::new(root)),
            server,
        }
    }

    #[test]
    fn external_file_prefers_the_origin_server() {
        let (ws, a, b) = (key("/ws", "ra"), key("/ws/a", "ra"), key("/ws/b", "ra"));
        let running = [&ws, &a, &b];
        let picked = external_keys(["ra"].into_iter(), Some(&b), &running, Path::new("/ws"));
        assert_eq!(picked, [b.clone()]);
        // Without an origin (or a stopped one): the workspace-rooted server.
        let stopped = key("/ws/c", "ra");
        let picked = external_keys(
            ["ra"].into_iter(),
            Some(&stopped),
            &running,
            Path::new("/ws"),
        );
        assert_eq!(picked, [ws.clone()]);
        let picked = external_keys(["ra"].into_iter(), None, &[&b, &a], Path::new("/ws"));
        assert_eq!(picked, [a.clone()]);
    }

    #[test]
    fn external_file_without_a_running_server_gets_none() {
        let other = key("/ws", "gopls");
        let picked = external_keys(["ra"].into_iter(), None, &[&other], Path::new("/ws"));
        assert!(picked.is_empty());
    }
}
