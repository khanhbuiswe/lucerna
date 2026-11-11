//! Keeping language servers' view of open documents up to date, and
//! matching file changes against the servers' watch registrations.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use globset::{Glob, GlobBuilder, GlobMatcher};
use lsp_types::notification::{DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument};
use lsp_types::{
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    FileChangeType, FileEvent, FileSystemWatcher, GlobPattern, OneOf,
    TextDocumentContentChangeEvent, TextDocumentIdentifier, TextDocumentItem,
    TextDocumentSyncCapability, TextDocumentSyncKind, VersionedTextDocumentIdentifier, WatchKind,
};
use lucerna_text::BufferSnapshot;

use crate::client::LanguageServer;
use crate::position::{PositionEncoding, path_to_uri, to_lsp, uri_to_path};
use crate::registry::ServerKey;

/// Lines longer than this (in chars) keep a file away from language servers.
const MAX_LINE_CHARS: usize = 20_000;

type Servers<'a> = &'a [(ServerKey, Arc<LanguageServer>)];

struct Doc {
    version: i32,
    /// What the server was last sent; incremental changes are relative to it.
    snapshot: BufferSnapshot,
}

/// Per `(path, server)`: the document version and the last text sent.
#[derive(Default)]
pub struct DocumentSync {
    docs: HashMap<(PathBuf, ServerKey), Doc>,
}

fn sync_kind(server: &LanguageServer) -> TextDocumentSyncKind {
    let Some(caps) = server.capabilities() else {
        return TextDocumentSyncKind::FULL;
    };
    match &caps.text_document_sync {
        Some(TextDocumentSyncCapability::Kind(kind)) => *kind,
        Some(TextDocumentSyncCapability::Options(options)) => {
            options.change.unwrap_or(TextDocumentSyncKind::NONE)
        }
        None => TextDocumentSyncKind::NONE,
    }
}

fn whole_text(snapshot: &BufferSnapshot) -> String {
    snapshot.text_for_range(0..snapshot.len_bytes())
}

fn full_range(snapshot: &BufferSnapshot, enc: PositionEncoding) -> lsp_types::Range {
    let end = snapshot.offset_to_point(snapshot.len_bytes());
    lsp_types::Range::new(lsp_types::Position::new(0, 0), to_lsp(snapshot, end, enc))
}

impl DocumentSync {
    /// `didOpen` at version 0, once per `(path, server)`.
    pub fn open(
        &mut self,
        path: &Path,
        lsp_language_id: &str,
        snapshot: &BufferSnapshot,
        servers: Servers<'_>,
    ) {
        for (key, server) in servers {
            let doc_key = (path.to_path_buf(), key.clone());
            if self.docs.contains_key(&doc_key) {
                continue;
            }
            server.notify::<DidOpenTextDocument>(DidOpenTextDocumentParams {
                text_document: TextDocumentItem {
                    uri: path_to_uri(path),
                    language_id: lsp_language_id.to_owned(),
                    version: 0,
                    text: whole_text(snapshot),
                },
            });
            self.docs.insert(
                doc_key,
                Doc {
                    version: 0,
                    snapshot: snapshot.clone(),
                },
            );
        }
    }

    /// `didChange` for servers that have the file open and want changes.
    /// Incremental servers get one change replacing the whole old text.
    pub fn change(&mut self, path: &Path, snapshot: &BufferSnapshot, servers: Servers<'_>) {
        for (key, server) in servers {
            let Some(doc) = self.docs.get_mut(&(path.to_path_buf(), key.clone())) else {
                continue;
            };
            let change = match sync_kind(server) {
                TextDocumentSyncKind::FULL => TextDocumentContentChangeEvent {
                    range: None,
                    range_length: None,
                    text: whole_text(snapshot),
                },
                TextDocumentSyncKind::INCREMENTAL => TextDocumentContentChangeEvent {
                    range: Some(full_range(&doc.snapshot, server.encoding())),
                    range_length: None,
                    text: whole_text(snapshot),
                },
                _ => continue,
            };
            doc.version += 1;
            doc.snapshot = snapshot.clone();
            server.notify::<DidChangeTextDocument>(DidChangeTextDocumentParams {
                text_document: VersionedTextDocumentIdentifier {
                    uri: path_to_uri(path),
                    version: doc.version,
                },
                content_changes: vec![change],
            });
        }
    }

    pub fn close(&mut self, path: &Path, servers: Servers<'_>) {
        for (key, server) in servers {
            if self
                .docs
                .remove(&(path.to_path_buf(), key.clone()))
                .is_some()
            {
                server.notify::<DidCloseTextDocument>(DidCloseTextDocumentParams {
                    text_document: TextDocumentIdentifier {
                        uri: path_to_uri(path),
                    },
                });
            }
        }
    }

    /// Drops all state of a server that stopped; nothing is sent.
    pub fn forget_server(&mut self, key: &ServerKey) {
        self.docs.retain(|(_, k), _| k != key);
    }

    pub fn version(&self, path: &Path, key: &ServerKey) -> Option<i32> {
        self.docs
            .get(&(path.to_path_buf(), key.clone()))
            .map(|doc| doc.version)
    }

    pub fn is_open(&self, path: &Path, key: &ServerKey) -> bool {
        self.version(path, key).is_some()
    }
}

/// Whether a file stays away from language servers: over the byte or line
/// limit, or with a line longer than 20 000 chars.
pub fn too_large_for_lsp(snapshot: &BufferSnapshot, max_bytes: u64, max_lines: u32) -> bool {
    if snapshot.len_bytes() as u64 > max_bytes || snapshot.line_count() > max_lines {
        return true;
    }
    // A line of at most MAX_LINE_CHARS bytes cannot have more chars.
    snapshot.len_bytes() > MAX_LINE_CHARS
        && (0..snapshot.line_count()).any(|row| {
            let line = snapshot.line(row);
            line.len() > MAX_LINE_CHARS && line.chars().count() > MAX_LINE_CHARS
        })
}

struct Watcher {
    /// The directory the glob is relative to; `None` for absolute globs.
    base: Option<PathBuf>,
    matcher: GlobMatcher,
    kind: WatchKind,
}

impl Watcher {
    fn new(root: &Path, watcher: FileSystemWatcher) -> Option<Watcher> {
        let (base, pattern) = match watcher.glob_pattern {
            GlobPattern::String(pattern) => {
                if Path::new(&pattern).is_absolute() {
                    (None, pattern)
                } else {
                    (Some(root.to_path_buf()), pattern)
                }
            }
            GlobPattern::Relative(relative) => {
                let uri = match relative.base_uri {
                    OneOf::Left(folder) => folder.uri,
                    OneOf::Right(uri) => uri,
                };
                (Some(uri_to_path(&uri)?), relative.pattern)
            }
        };
        let glob: Glob = GlobBuilder::new(&pattern)
            .literal_separator(true)
            .build()
            .ok()?;
        Some(Watcher {
            base,
            matcher: glob.compile_matcher(),
            kind: watcher
                .kind
                .unwrap_or(WatchKind::Create | WatchKind::Change | WatchKind::Delete),
        })
    }

    fn matches(&self, path: &Path, typ: FileChangeType) -> bool {
        let wanted = if typ == FileChangeType::CREATED {
            WatchKind::Create
        } else if typ == FileChangeType::DELETED {
            WatchKind::Delete
        } else {
            WatchKind::Change
        };
        if !self.kind.contains(wanted) {
            return false;
        }
        match &self.base {
            Some(base) => path
                .strip_prefix(base)
                .is_ok_and(|relative| self.matcher.is_match(relative)),
            None => self.matcher.is_match(path),
        }
    }
}

/// The `workspace/didChangeWatchedFiles` registrations of the servers.
#[derive(Default)]
pub struct WatchedFiles {
    registrations: HashMap<ServerKey, HashMap<String, Vec<Watcher>>>,
}

impl WatchedFiles {
    /// Plain globs are relative to the server root, relative patterns to
    /// their base URI. Watchers with an invalid glob are skipped.
    pub fn register(&mut self, key: ServerKey, id: String, watchers: Vec<FileSystemWatcher>) {
        let compiled = watchers
            .into_iter()
            .filter_map(|watcher| Watcher::new(&key.root, watcher))
            .collect();
        self.registrations
            .entry(key)
            .or_default()
            .insert(id, compiled);
    }

    pub fn unregister(&mut self, key: &ServerKey, id: &str) {
        if let Some(ids) = self.registrations.get_mut(key) {
            ids.remove(id);
            if ids.is_empty() {
                self.registrations.remove(key);
            }
        }
    }

    /// The events each server asked for, in the order of `changes`; one
    /// event per change even when several watchers match.
    pub fn events_for(
        &self,
        changes: &[(PathBuf, FileChangeType)],
    ) -> HashMap<ServerKey, Vec<FileEvent>> {
        let mut out: HashMap<ServerKey, Vec<FileEvent>> = HashMap::new();
        for (key, ids) in &self.registrations {
            for (path, typ) in changes {
                let wanted = ids
                    .values()
                    .flatten()
                    .any(|watcher| watcher.matches(path, *typ));
                if wanted {
                    out.entry(key.clone()).or_default().push(FileEvent {
                        uri: path_to_uri(path),
                        typ: *typ,
                    });
                }
            }
        }
        out
    }

    pub fn forget_server(&mut self, key: &ServerKey) {
        self.registrations.remove(key);
    }
}
