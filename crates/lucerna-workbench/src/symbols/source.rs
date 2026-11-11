//! Where the symbols of a document come from (spec §7.6 "Nguồn symbol"):
//! a running server with `documentSymbolProvider`, else the language's
//! `outline.scm`, else nothing.
//!
//! Results are cached per document and revision. A document that opens
//! or reloads, and every document when a server becomes ready, is fetched
//! again after a 300 ms debounce. An empty server answer (some servers
//! answer so while indexing) is kept 2 s, then fetched again when next
//! asked for. The outline query runs on the background executor.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::Duration;

use gpui::{AppContext, Context, Entity, Subscription, Task};
use lucerna_syntax::{Language, SymbolTree, SyntaxState};
use lucerna_text::BufferSnapshot;

use crate::Workbench;
use crate::documents::{DocState, DocumentEvent, Documents};
use crate::language_servers::{LanguageServers, LanguageServersEvent};
use crate::tasks::guarded;
use crate::workspace_state::absolute_path;

/// How long a fetch waits, so a burst of changes costs one request.
pub const SYMBOLS_DEBOUNCE: Duration = Duration::from_millis(300);
/// How long an empty server answer is kept before it is asked again.
pub const EMPTY_SYMBOLS_TTL: Duration = Duration::from_secs(2);
/// How many times a fetch waits [`SYMBOLS_DEBOUNCE`] for the first parse
/// of an outline language before giving up.
const PARSE_RETRIES: u32 = 20;

/// The symbols of a document, as far as they are known.
#[derive(Debug, Clone)]
pub enum SymbolState {
    Ready(Arc<SymbolTree>),
    /// Waiting for the text, the parse, a server that is starting, or an
    /// answer.
    Loading,
    /// No server answers `documentSymbol` and the language has no
    /// `outline.scm`.
    Unavailable,
}

/// The cached symbols of one document.
struct Entry {
    revision: u64,
    /// The syntax state an outline was read from: a new parse of the same
    /// revision makes the outline stale. Held weakly, so the allocation
    /// cannot be reused by another state while compared.
    syntax: Option<Weak<SyntaxState>>,
    state: SymbolState,
    /// Shown until the fetch in flight replaces it.
    stale: bool,
    fetching: bool,
    /// Tells a fetch whether it is still the latest for this document.
    generation: u64,
    _fetch: Task<()>,
    _expiry: Task<()>,
}

/// The symbol cache of a workbench; see the module documentation.
pub struct SymbolSource {
    root: PathBuf,
    documents: Entity<Documents>,
    entries: HashMap<Arc<Path>, Entry>,
    generation: u64,
    _subscriptions: Vec<Subscription>,
}

/// What a fetch found.
struct Fetched {
    state: SymbolState,
    syntax: Option<Weak<SyntaxState>>,
    /// An empty server answer: kept for [`EMPTY_SYMBOLS_TTL`].
    expires: bool,
}

/// What the outline query gave.
enum Outline {
    Ready(SymbolTree),
    /// The document is not parsed yet.
    Retry,
    Unavailable,
}

impl SymbolSource {
    pub(crate) fn new(
        root: PathBuf,
        documents: &Entity<Documents>,
        servers: &Entity<LanguageServers>,
        cx: &mut Context<Workbench>,
    ) -> SymbolSource {
        SymbolSource {
            root,
            documents: documents.clone(),
            entries: HashMap::new(),
            generation: 0,
            _subscriptions: vec![
                cx.subscribe(documents, |wb, _, ev, cx| {
                    wb.symbols.on_document_event(ev, cx)
                }),
                cx.subscribe(servers, |wb, _, ev, cx| wb.symbols.on_servers_event(ev, cx)),
            ],
        }
    }

    fn key(&self, path: &Path) -> Arc<Path> {
        Arc::from(absolute_path(&self.root, path))
    }

    /// The symbols of the open document `path`. A missing or stale result
    /// starts a fetch; until it lands this gives `Loading`, or the stale
    /// result of the same revision. [`Workbench::symbols_changed`] is
    /// called when a fetch lands.
    pub fn get(&mut self, path: &Path, cx: &mut Context<Workbench>) -> SymbolState {
        let key = self.key(path);
        let docs = self.documents.read(cx);
        let Some(revision) = docs.revision_of(&key) else {
            return match docs.state(&key) {
                Some(DocState::Loading) => SymbolState::Loading,
                _ => SymbolState::Unavailable,
            };
        };
        let syntax = docs.syntax_of(&key).and_then(|(_, syntax)| syntax);
        match self.entries.get_mut(&key) {
            Some(entry) if entry.revision == revision => {
                if entry
                    .syntax
                    .as_ref()
                    .is_some_and(|read| !is_state(read, syntax.as_ref()))
                {
                    entry.stale = true;
                }
                let state = entry.state.clone();
                if entry.stale && !entry.fetching {
                    self.fetch(key, cx);
                }
                state
            }
            _ => {
                self.fetch(key, cx);
                SymbolState::Loading
            }
        }
    }

    /// Like [`SymbolSource::get`], except that an expired empty answer is
    /// given as is rather than asked again: for readers that ask on every
    /// change, such as the breadcrumbs, which would otherwise ask it again
    /// every [`EMPTY_SYMBOLS_TTL`]. A new revision, or an outline read from
    /// an older parse, is fetched as `get` does.
    pub(crate) fn current(&mut self, path: &Path, cx: &mut Context<Workbench>) -> SymbolState {
        let key = self.key(path);
        let docs = self.documents.read(cx);
        let revision = docs.revision_of(&key);
        let syntax = docs.syntax_of(&key).and_then(|(_, syntax)| syntax);
        match self.entries.get(&key) {
            Some(entry)
                if Some(entry.revision) == revision
                    && entry
                        .syntax
                        .as_ref()
                        .is_none_or(|read| is_state(read, syntax.as_ref())) =>
            {
                entry.state.clone()
            }
            _ => self.get(path, cx),
        }
    }

    fn on_document_event(&mut self, ev: &DocumentEvent, cx: &mut Context<Workbench>) {
        match ev {
            DocumentEvent::Opened { path, .. }
            | DocumentEvent::Reloaded { path, .. }
            | DocumentEvent::LanguageChanged { path, .. } => self.fetch(path.clone(), cx),
            DocumentEvent::Closed { path } => {
                self.entries.remove(path);
            }
            // An outline read from the old tree (or the edited one a
            // reload keeps until the parse) is out of date.
            DocumentEvent::Parsed { path } => {
                if self
                    .entries
                    .get(path)
                    .is_some_and(|e| e.fetching || e.syntax.is_some())
                {
                    self.fetch(path.clone(), cx);
                }
            }
        }
    }

    fn on_servers_event(&mut self, ev: &LanguageServersEvent, cx: &mut Context<Workbench>) {
        let refetch: Vec<Arc<Path>> = match ev {
            LanguageServersEvent::ServerReady(_) => self.entries.keys().cloned().collect(),
            // A server that failed or stopped leaves `Loading` behind.
            LanguageServersEvent::StatusChanged => self
                .entries
                .iter()
                .filter(|(_, e)| !e.fetching && matches!(e.state, SymbolState::Loading))
                .map(|(path, _)| path.clone())
                .collect(),
            _ => return,
        };
        for path in refetch {
            self.fetch(path, cx);
        }
    }

    /// Fetches the symbols of `path` after [`SYMBOLS_DEBOUNCE`], replacing
    /// a fetch in flight. The current result stays (stale) meanwhile when
    /// it is of the current revision.
    fn fetch(&mut self, path: Arc<Path>, cx: &mut Context<Workbench>) {
        let Some(revision) = self.documents.read(cx).revision_of(&path) else {
            return;
        };
        self.generation += 1;
        let generation = self.generation;
        let task = cx.spawn({
            let path = path.clone();
            async move |wb, cx| {
                for _ in 0..PARSE_RETRIES {
                    cx.background_executor().timer(SYMBOLS_DEBOUNCE).await;
                    let Ok(request) = wb.update(cx, |wb, cx| {
                        wb.language_servers()
                            .clone()
                            .update(cx, |servers, cx| servers.document_symbols(&path, cx))
                    }) else {
                        return;
                    };
                    let fetched = match request.await {
                        Some(tree) => Some(Fetched {
                            expires: tree.0.is_empty(),
                            state: SymbolState::Ready(Arc::new(tree)),
                            syntax: None,
                        }),
                        None => {
                            let Ok(pending) = wb.update(cx, |wb, cx| {
                                wb.language_servers().read(cx).symbol_server_pending(&path)
                            }) else {
                                return;
                            };
                            if pending {
                                Some(Fetched {
                                    state: SymbolState::Loading,
                                    syntax: None,
                                    expires: false,
                                })
                            } else {
                                let Ok(inputs) = wb.update(cx, |wb, cx| {
                                    let docs = wb.documents().read(cx);
                                    (docs.syntax_of(&path), docs.snapshot_of(&path))
                                }) else {
                                    return;
                                };
                                outline(inputs, cx).await
                            }
                        }
                    };
                    let done = wb
                        .update(cx, |wb, cx| {
                            wb.symbols.finish(&path, generation, fetched, cx);
                            wb.symbols_changed(&path, cx);
                            !wb.symbols.is_fetching(&path, generation)
                        })
                        .unwrap_or(true);
                    if done {
                        return;
                    }
                }
                wb.update(cx, |wb, cx| {
                    let gave_up = Fetched {
                        state: SymbolState::Unavailable,
                        syntax: None,
                        expires: false,
                    };
                    wb.symbols.finish(&path, generation, Some(gave_up), cx);
                    wb.symbols_changed(&path, cx);
                })
                .ok();
            }
        });
        let fresh = self
            .entries
            .remove(&path)
            .filter(|e| e.revision == revision);
        let state = fresh
            .as_ref()
            .map_or(SymbolState::Loading, |e| e.state.clone());
        self.entries.insert(
            path,
            Entry {
                revision,
                syntax: fresh.as_ref().and_then(|e| e.syntax.clone()),
                state,
                stale: true,
                fetching: true,
                generation,
                _fetch: task,
                _expiry: Task::ready(()),
            },
        );
    }

    /// Whether the fetch `generation` of `path` is still running (it
    /// waits for a parse).
    fn is_fetching(&self, path: &Path, generation: u64) -> bool {
        self.entries
            .get(path)
            .is_some_and(|e| e.generation == generation && e.fetching)
    }

    /// Stores what the fetch `generation` of `path` found, if it is still
    /// the latest. `None` (the document is not parsed yet) keeps it going.
    fn finish(
        &mut self,
        path: &Arc<Path>,
        generation: u64,
        fetched: Option<Fetched>,
        cx: &mut Context<Workbench>,
    ) {
        let Some(entry) = self.entries.get_mut(path) else {
            return;
        };
        if entry.generation != generation {
            return;
        }
        let Some(fetched) = fetched else {
            entry.state = SymbolState::Loading;
            return;
        };
        entry.state = fetched.state;
        entry.syntax = fetched.syntax;
        entry.stale = false;
        entry.fetching = false;
        if fetched.expires {
            let path = path.clone();
            entry._expiry = cx.spawn(async move |wb, cx| {
                cx.background_executor().timer(EMPTY_SYMBOLS_TTL).await;
                wb.update(cx, |wb, cx| {
                    if let Some(entry) = wb.symbols.entries.get_mut(&path)
                        && entry.generation == generation
                    {
                        entry.stale = true;
                        wb.symbols_changed(&path, cx);
                    }
                })
                .ok();
            });
        }
    }
}

/// A document's language and syntax state (once parsed), and its text.
type OutlineInputs = (
    Option<(Arc<Language>, Option<Arc<SyntaxState>>)>,
    Option<BufferSnapshot>,
);

/// The outline of a document from its language and syntax state, read on
/// the background executor. `None` while it waits for the first parse.
async fn outline(inputs: OutlineInputs, cx: &mut gpui::AsyncApp) -> Option<Fetched> {
    let unavailable = Fetched {
        state: SymbolState::Unavailable,
        syntax: None,
        expires: false,
    };
    let (Some((language, syntax)), Some(snapshot)) = inputs else {
        return Some(unavailable);
    };
    let read = syntax.as_ref().map(Arc::downgrade);
    let outline = cx
        .background_spawn(async move {
            guarded("outline", || {
                read_outline(&language, syntax.as_deref(), &snapshot)
            })
            .unwrap_or(Outline::Unavailable)
        })
        .await;
    match outline {
        Outline::Ready(tree) => Some(Fetched {
            state: SymbolState::Ready(Arc::new(tree)),
            syntax: read,
            expires: false,
        }),
        Outline::Retry => None,
        Outline::Unavailable => Some(unavailable),
    }
}

/// Runs the outline query of `language` on `syntax`. Looks for the
/// language's `outline.scm` (a `stat`) and runs the query: off the UI
/// thread.
fn read_outline(
    language: &Language,
    syntax: Option<&SyntaxState>,
    snapshot: &BufferSnapshot,
) -> Outline {
    if !language.has_outline() {
        return Outline::Unavailable;
    }
    match syntax {
        Some(syntax) => match syntax.outline(snapshot) {
            Some(tree) => Outline::Ready(tree),
            None if syntax.tree().is_none() => Outline::Retry,
            // The query does not compile.
            None => Outline::Unavailable,
        },
        None if language.is_failed() => Outline::Unavailable,
        None => Outline::Retry,
    }
}

/// Whether `read` is the syntax state `current`.
fn is_state(read: &Weak<SyntaxState>, current: Option<&Arc<SyntaxState>>) -> bool {
    current.is_some_and(|current| std::ptr::eq(read.as_ptr(), Arc::as_ptr(current)))
}
