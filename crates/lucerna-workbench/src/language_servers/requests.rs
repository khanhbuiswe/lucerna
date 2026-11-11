//! Requests the editor sends to the language servers of a document.

use std::collections::HashMap;
use std::future::Future;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;
use futures::future::BoxFuture;
use gpui::{AppContext, Context, Task};
use lsp_types::request::{
    DocumentSymbolRequest, GotoDeclaration, GotoDefinition, GotoImplementation, GotoTypeDefinition,
    HoverRequest, References, WorkspaceSymbolRequest, WorkspaceSymbolResolve,
};
use lsp_types::{
    DeclarationCapability, DocumentSymbolParams, GotoDefinitionParams, GotoDefinitionResponse,
    HoverContents, HoverParams, HoverProviderCapability, ImplementationProviderCapability,
    MarkedString, MarkupKind, OneOf, ReferenceContext, ReferenceParams, ServerCapabilities,
    TextDocumentIdentifier, TextDocumentPositionParams, TypeDefinitionProviderCapability,
    WorkspaceSymbolParams, WorkspaceSymbolResponse,
};
use lucerna_lsp::locations::{ResolvedLocation, locations_from_response, resolve_locations};
use lucerna_lsp::position::{PositionEncoding, path_to_uri, range_from_lsp, to_lsp, uri_to_path};
use lucerna_lsp::symbols::{symbol_kind, symbols_from_response};
use lucerna_lsp::{LanguageServer, LspError, ServerKey, Slot};
use lucerna_project::WorktreeSnapshot;
use lucerna_syntax::SymbolTree;
use lucerna_text::{Buffer, BufferSnapshot, Point};

use super::LanguageServers;
use crate::goto::GotoKind;
use crate::symbols::workspace::{Candidate, Unresolved, WorkspaceSymbolItem, rank};
use crate::workspace_state::absolute_path;

/// `textDocument/hover` gives up after this.
const HOVER_TIMEOUT: Duration = Duration::from_secs(10);
/// The `textDocument/definition` of a ⌘-hover gives up after this.
const LINK_TIMEOUT: Duration = Duration::from_secs(10);

/// A hover answer, as markdown.
#[derive(Debug, Clone, PartialEq)]
pub struct HoverInfo {
    pub markdown: String,
    /// The range the answer is about, in the document's text.
    pub range: Option<Range<Point>>,
    /// The document's language id: the language of code blocks that name
    /// none.
    pub language: Option<String>,
}

impl LanguageServers {
    /// Asks the first running server of `path` that has `hoverProvider`
    /// about `point`, with a 10 s timeout. Resolves to `None` when no
    /// server can answer, on errors and for empty answers. Dropping the
    /// task cancels the request (`$/cancelRequest`).
    pub fn hover(
        &self,
        path: &Path,
        point: Point,
        cx: &mut Context<Self>,
    ) -> Task<Option<HoverInfo>> {
        let path = absolute_path(&self.root, path);
        let Some(doc) = self.docs.get(path.as_path()) else {
            return Task::ready(None);
        };
        let server = self.running_for(&path).into_iter().find_map(|(_, server)| {
            let caps = server.capabilities()?;
            match caps.hover_provider.as_ref()? {
                HoverProviderCapability::Simple(false) => None,
                _ => Some(server),
            }
        });
        let Some(server) = server else {
            return Task::ready(None);
        };
        let snapshot = doc.snapshot.clone();
        let language = doc.language_id.as_deref().map(str::to_owned);
        let encoding = server.encoding();
        let params = HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier {
                    uri: path_to_uri(&path),
                },
                position: to_lsp(&snapshot, point, encoding),
            },
            work_done_progress_params: Default::default(),
        };
        let request = server.request_with_timeout::<HoverRequest>(params, HOVER_TIMEOUT);
        cx.background_spawn(async move {
            let hover = match request.await {
                Ok(hover) => hover?,
                Err(err) => {
                    tracing::debug!(%err, "hover failed");
                    return None;
                }
            };
            crate::tasks::guarded("hover", || hover_info(hover, &snapshot, encoding, language))
                .flatten()
        })
    }
}

impl LanguageServers {
    /// Asks every running server of `path` that supports `kind` for the
    /// locations of the symbol at `point` (references include the
    /// declaration). The answers are joined, sorted by path and place, and
    /// deduplicated; files that are not open are read off the UI thread.
    /// Errors and timeouts count as no answer; `None` when every server
    /// asked failed (spec §10: say nothing then). Dropping the task cancels
    /// the requests.
    pub fn locations(
        &self,
        path: &Path,
        point: Point,
        kind: GotoKind,
        cx: &mut Context<Self>,
    ) -> Task<Option<Vec<ResolvedLocation>>> {
        let path = absolute_path(&self.root, path);
        let Some(doc) = self.docs.get(path.as_path()) else {
            return Task::ready(Some(Vec::new()));
        };
        let snapshot = &doc.snapshot;
        let uri = path_to_uri(&path);
        let requests: Vec<_> = self
            .running_for(&path)
            .into_iter()
            .filter(|(_, server)| {
                server
                    .capabilities()
                    .is_some_and(|caps| supports(&caps, kind))
            })
            .map(|(key, server)| {
                let encoding = server.encoding();
                let position = TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier { uri: uri.clone() },
                    position: to_lsp(snapshot, point, encoding),
                };
                let request = request_locations(&server, kind, position);
                request.map(move |locs| (key, encoding, locs))
            })
            .collect();
        if requests.is_empty() {
            return Task::ready(Some(Vec::new()));
        }
        let open = self.open_snapshots();
        let work = cx.background_spawn(async move {
            let answers = futures::future::join_all(requests).await;
            if answers.iter().all(|(_, _, locs)| locs.is_none()) {
                return None;
            }
            crate::tasks::guarded("goto-locations", move || {
                let open = |path: &Path| open.get(path).cloned();
                let mut origins = Vec::new();
                let mut all: Vec<ResolvedLocation> = Vec::new();
                for (key, encoding, locs) in answers {
                    let Some(locs) = locs else { continue };
                    let resolved = resolve_locations(locs, encoding, &open);
                    origins.extend(resolved.iter().map(|l| (l.path.to_path_buf(), key.clone())));
                    all.extend(resolved);
                }
                // Each server's answer is sorted; together they need it again.
                let key = |l: &ResolvedLocation| {
                    let r = &l.range;
                    let key = (r.start.row, r.start.column, r.end.row, r.end.column);
                    (l.path.clone(), key)
                };
                all.sort_by_key(key);
                all.dedup_by_key(|l| key(l));
                (all, origins)
            })
        });
        cx.spawn(async move |this, cx| {
            let (all, origins) = work.await?;
            this.update(cx, |servers, _| servers.note_origins(origins))
                .ok();
            Some(all)
        })
    }
}

impl LanguageServers {
    /// Asks the first running server of `path` that has
    /// `documentSymbolProvider` for the document's symbols. `None` when no
    /// running server can answer; an error or a `null` answer is an empty
    /// tree. The answer is converted off the UI thread. Dropping the task
    /// cancels the request.
    pub fn document_symbols(
        &self,
        path: &Path,
        cx: &mut Context<Self>,
    ) -> Task<Option<SymbolTree>> {
        let path = absolute_path(&self.root, path);
        let Some(doc) = self.docs.get(path.as_path()) else {
            return Task::ready(None);
        };
        let server = self
            .running_for(&path)
            .into_iter()
            .map(|(_, server)| server)
            .find(|server| {
                server
                    .capabilities()
                    .is_some_and(|caps| has_document_symbols(&caps))
            });
        let Some(server) = server else {
            return Task::ready(None);
        };
        let snapshot = doc.snapshot.clone();
        let encoding = server.encoding();
        let params = DocumentSymbolParams {
            text_document: TextDocumentIdentifier {
                uri: path_to_uri(&path),
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        };
        let request = server.request::<DocumentSymbolRequest>(params);
        cx.background_spawn(async move {
            let response = match request.await {
                Ok(response) => response,
                Err(err) => {
                    tracing::debug!(%err, "documentSymbol failed");
                    None
                }
            };
            let tree = match response {
                Some(response) => crate::tasks::guarded("document-symbols", || {
                    symbols_from_response(response, &snapshot, encoding)
                })
                .unwrap_or_default(),
                None => SymbolTree::default(),
            };
            Some(tree)
        })
    }

    /// Whether a server of the open document `path` is still starting,
    /// so it may yet answer [`LanguageServers::document_symbols`].
    pub fn symbol_server_pending(&self, path: &Path) -> bool {
        let path = absolute_path(&self.root, path);
        self.docs.get(path.as_path()).is_some_and(|doc| {
            doc.keys
                .iter()
                .any(|key: &ServerKey| matches!(self.registry.slot(key), Some(Slot::Starting)))
        })
    }
}

impl LanguageServers {
    /// Whether a running server answers `workspace/symbol`.
    pub fn has_workspace_symbols(&self) -> bool {
        self.registry.running().any(|(_, server)| {
            server
                .capabilities()
                .is_some_and(|caps| has_workspace_symbols(&caps))
        })
    }

    /// Sends `workspace/symbol { query }` to every running server that
    /// supports it and joins the answers (errors count as none), off the
    /// UI thread: duplicates dropped, names fuzzy-filtered by the last
    /// `::` segment of `query`, files of `project` (the root's
    /// not-ignored files) first, at most 100, and only then are their
    /// ranges read against the files' text. Dropping the task cancels
    /// the requests.
    pub fn workspace_symbols(
        &self,
        query: String,
        project: WorktreeSnapshot,
        cx: &mut Context<Self>,
    ) -> Task<Vec<WorkspaceSymbolItem>> {
        let requests: Vec<_> = self
            .registry
            .running()
            .filter(|(_, server)| {
                server
                    .capabilities()
                    .is_some_and(|caps| has_workspace_symbols(&caps))
            })
            .map(|(key, server)| {
                let key = key.clone();
                let encoding = server.encoding();
                let request = server.request::<WorkspaceSymbolRequest>(WorkspaceSymbolParams {
                    query: query.clone(),
                    ..Default::default()
                });
                async move {
                    let response = request.await.unwrap_or_else(|err| {
                        tracing::debug!(%err, "workspace/symbol failed");
                        None
                    });
                    (key, encoding, response)
                }
            })
            .collect();
        if requests.is_empty() {
            return Task::ready(Vec::new());
        }
        let open = self.open_snapshots();
        let root = self.root.clone();
        let work = cx.background_spawn(async move {
            let answers = futures::future::join_all(requests).await;
            crate::tasks::guarded("workspace-symbols", move || {
                let mut origins = Vec::new();
                let candidates = answers
                    .into_iter()
                    .flat_map(|(key, encoding, response)| {
                        let found = candidates(key.clone(), encoding, response);
                        origins.extend(
                            found
                                .iter()
                                .filter(|c| !c.path.starts_with(&root))
                                .map(|c| (c.path.clone(), key.clone())),
                        );
                        found
                    })
                    .collect();
                let in_project = |path: &Path| {
                    path.strip_prefix(&root)
                        .is_ok_and(|relative| project.contains(relative))
                };
                let mut snapshots = HashMap::new();
                let items = rank(candidates, &query, in_project)
                    .into_iter()
                    .map(|(c, in_project)| {
                        let snapshot = snapshots
                            .entry(c.path.clone())
                            .or_insert_with(|| text_of(&c.path, &open));
                        let range = c
                            .range
                            .zip(snapshot.as_ref())
                            .map(|(range, snapshot)| range_from_lsp(snapshot, range, c.encoding));
                        WorkspaceSymbolItem {
                            name: c.name,
                            kind: c.kind,
                            container: c.container,
                            path: Arc::from(c.path),
                            range,
                            in_project,
                            unresolved: c.unresolved,
                        }
                    })
                    .collect::<Vec<_>>();
                (items, origins)
            })
            .unwrap_or_default()
        });
        cx.spawn(async move |this, cx| {
            let (items, origins) = work.await;
            this.update(cx, |servers, _| servers.note_origins(origins))
                .ok();
            items
        })
    }

    /// Where to open `item`: the start of its range; for a symbol sent
    /// without one, the range `workspaceSymbol/resolve` gives, if its
    /// server is running and supports that. `None` when there is none
    /// (open it at the top).
    pub fn resolve_workspace_symbol(
        &self,
        item: &WorkspaceSymbolItem,
        cx: &mut Context<Self>,
    ) -> Task<Option<Point>> {
        if let Some(range) = &item.range {
            return Task::ready(Some(range.start));
        }
        let Some(Unresolved { key, symbol }) = &item.unresolved else {
            return Task::ready(None);
        };
        let Some(Slot::Running(server)) = self.registry.slot(key) else {
            return Task::ready(None);
        };
        let resolves = server.capabilities().is_some_and(|caps| {
            matches!(
                &caps.workspace_symbol_provider,
                Some(OneOf::Right(options)) if options.resolve_provider == Some(true)
            )
        });
        if !resolves {
            return Task::ready(None);
        }
        let encoding = server.encoding();
        let request = server.request::<WorkspaceSymbolResolve>(symbol.clone());
        let open = self.open_snapshots();
        let path = item.path.to_path_buf();
        cx.background_spawn(async move {
            let symbol = match request.await {
                Ok(symbol) => symbol,
                Err(err) => {
                    tracing::debug!(%err, "workspaceSymbol/resolve failed");
                    return None;
                }
            };
            let OneOf::Left(location) = symbol.location else {
                return None;
            };
            crate::tasks::guarded("resolve-workspace-symbol", move || {
                let snapshot = text_of(&path, &open)?;
                Some(range_from_lsp(&snapshot, location.range, encoding).start)
            })
            .flatten()
        })
    }

    /// The text the servers were sent, for the files open here.
    fn open_snapshots(&self) -> HashMap<PathBuf, BufferSnapshot> {
        self.docs
            .iter()
            .map(|(path, doc)| (path.to_path_buf(), doc.snapshot.clone()))
            .collect()
    }
}

/// The text of `path`: from `open`, else read from disk. Blocks.
fn text_of(path: &Path, open: &HashMap<PathBuf, BufferSnapshot>) -> Option<BufferSnapshot> {
    open.get(path)
        .cloned()
        .or_else(|| Buffer::load(path).ok().map(|buffer| buffer.snapshot()))
}

/// The symbols of one server's `workspace/symbol` answer; symbols outside
/// files are dropped. A `WorkspaceSymbol` with only a URI keeps itself
/// for `workspaceSymbol/resolve`.
fn candidates(
    key: ServerKey,
    encoding: PositionEncoding,
    response: Option<WorkspaceSymbolResponse>,
) -> Vec<Candidate> {
    let container = |c: Option<String>| c.filter(|c| !c.is_empty());
    match response {
        None => Vec::new(),
        Some(WorkspaceSymbolResponse::Flat(infos)) => infos
            .into_iter()
            .filter_map(|info| {
                Some(Candidate {
                    path: uri_to_path(&info.location.uri)?,
                    range: Some(info.location.range),
                    name: info.name,
                    kind: symbol_kind(info.kind),
                    container: container(info.container_name),
                    encoding,
                    unresolved: None,
                })
            })
            .collect(),
        Some(WorkspaceSymbolResponse::Nested(symbols)) => symbols
            .into_iter()
            .filter_map(|symbol| {
                let (path, range, unresolved) = match &symbol.location {
                    OneOf::Left(location) => {
                        (uri_to_path(&location.uri)?, Some(location.range), None)
                    }
                    OneOf::Right(location) => {
                        let unresolved = Unresolved {
                            key: key.clone(),
                            symbol: symbol.clone(),
                        };
                        (uri_to_path(&location.uri)?, None, Some(unresolved))
                    }
                };
                Some(Candidate {
                    name: symbol.name,
                    kind: symbol_kind(symbol.kind),
                    container: container(symbol.container_name),
                    path,
                    range,
                    encoding,
                    unresolved,
                })
            })
            .collect(),
    }
}

/// Whether a server with `caps` answers `workspace/symbol`.
fn has_workspace_symbols(caps: &ServerCapabilities) -> bool {
    !matches!(
        caps.workspace_symbol_provider,
        None | Some(OneOf::Left(false))
    )
}

/// Whether a server with `caps` answers `textDocument/documentSymbol`.
fn has_document_symbols(caps: &ServerCapabilities) -> bool {
    !matches!(
        caps.document_symbol_provider,
        None | Some(OneOf::Left(false))
    )
}

impl LanguageServers {
    /// Whether [`LanguageServers::locations`] can ask anything about
    /// `path`: its document is open here and a running server of it
    /// supports `kind`.
    pub fn can_request_locations(&self, path: &Path, kind: GotoKind) -> bool {
        let path = absolute_path(&self.root, path);
        self.docs.contains_key(path.as_path())
            && self.running_for(&path).iter().any(|(_, server)| {
                server
                    .capabilities()
                    .is_some_and(|caps| supports(&caps, kind))
            })
    }

    /// The range to show as a link to the definition of the symbol at
    /// `point` (⌘-hover): the `originSelectionRange` of the first
    /// server's answer that has one, else the word at `point` when some
    /// server found a definition. `None` when none did, on errors and
    /// after a 10 s timeout. Dropping the task cancels the requests.
    pub fn definition_link(
        &self,
        path: &Path,
        point: Point,
        cx: &mut Context<Self>,
    ) -> Task<Option<Range<Point>>> {
        let path = absolute_path(&self.root, path);
        let Some(doc) = self.docs.get(path.as_path()) else {
            return Task::ready(None);
        };
        let snapshot = doc.snapshot.clone();
        let uri = path_to_uri(&path);
        let requests: Vec<_> = self
            .running_for(&path)
            .into_iter()
            .filter(|(_, server)| {
                server
                    .capabilities()
                    .is_some_and(|caps| supports(&caps, GotoKind::Definition))
            })
            .map(|(_, server)| {
                let encoding = server.encoding();
                let params = GotoDefinitionParams {
                    text_document_position_params: TextDocumentPositionParams {
                        text_document: TextDocumentIdentifier { uri: uri.clone() },
                        position: to_lsp(&snapshot, point, encoding),
                    },
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                };
                let request = server.request_with_timeout::<GotoDefinition>(params, LINK_TIMEOUT);
                async move {
                    match request.await {
                        Ok(response) => (encoding, response),
                        Err(err) => {
                            tracing::debug!(%err, "definition link request failed");
                            (encoding, None)
                        }
                    }
                }
            })
            .collect();
        if requests.is_empty() {
            return Task::ready(None);
        }
        cx.background_spawn(async move {
            let answers = futures::future::join_all(requests).await;
            crate::tasks::guarded("definition-link", move || {
                link_range(answers, &snapshot, point)
            })
            .flatten()
        })
    }
}

/// The link range of the definition answers: the first origin range of a
/// `LocationLink`, else the word at `point` when any answer has a file
/// location; `None` when none has.
fn link_range(
    answers: Vec<(PositionEncoding, Option<GotoDefinitionResponse>)>,
    snapshot: &BufferSnapshot,
    point: Point,
) -> Option<Range<Point>> {
    let mut found = false;
    for (encoding, response) in answers {
        let origin = match &response {
            Some(GotoDefinitionResponse::Link(links)) => links
                .iter()
                .find_map(|l| l.origin_selection_range)
                .map(|r| range_from_lsp(snapshot, r, encoding)),
            _ => None,
        };
        if locations_from_response(response).is_empty() {
            continue;
        }
        if let Some(origin) = origin.filter(|r| r.start != r.end) {
            return Some(origin);
        }
        found = true;
    }
    found.then(|| crate::code_view::word_at(snapshot, point))
}

/// Whether a server with `caps` answers `kind` requests.
fn supports(caps: &ServerCapabilities, kind: GotoKind) -> bool {
    match kind {
        GotoKind::Definition => {
            !matches!(caps.definition_provider, None | Some(OneOf::Left(false)))
        }
        GotoKind::TypeDefinition => !matches!(
            caps.type_definition_provider,
            None | Some(TypeDefinitionProviderCapability::Simple(false))
        ),
        GotoKind::Implementation => !matches!(
            caps.implementation_provider,
            None | Some(ImplementationProviderCapability::Simple(false))
        ),
        GotoKind::Declaration => !matches!(
            caps.declaration_provider,
            None | Some(DeclarationCapability::Simple(false))
        ),
        GotoKind::References => {
            !matches!(caps.references_provider, None | Some(OneOf::Left(false)))
        }
    }
}

/// Sends the `kind` request for `position` to `server`; resolves to its
/// locations, none on errors.
fn request_locations(
    server: &LanguageServer,
    kind: GotoKind,
    position: TextDocumentPositionParams,
) -> BoxFuture<'static, Option<Vec<(PathBuf, lsp_types::Range)>>> {
    let goto = GotoDefinitionParams {
        text_document_position_params: position.clone(),
        work_done_progress_params: Default::default(),
        partial_result_params: Default::default(),
    };
    match kind {
        GotoKind::Definition => answer(kind, server.request::<GotoDefinition>(goto)),
        GotoKind::TypeDefinition => answer(kind, server.request::<GotoTypeDefinition>(goto)),
        GotoKind::Implementation => answer(kind, server.request::<GotoImplementation>(goto)),
        GotoKind::Declaration => answer(kind, server.request::<GotoDeclaration>(goto)),
        GotoKind::References => {
            let params = ReferenceParams {
                text_document_position: position,
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
                context: ReferenceContext {
                    include_declaration: true,
                },
            };
            let request = server.request::<References>(params);
            let request = async move {
                request
                    .await
                    .map(|locs| locs.map(GotoDefinitionResponse::Array))
            };
            answer(kind, request)
        }
    }
}

/// The locations `request` resolves to; `None` (logged) on an error or a
/// timeout, which is not an answer.
fn answer(
    kind: GotoKind,
    request: impl Future<Output = Result<Option<GotoDefinitionResponse>, LspError>> + Send + 'static,
) -> BoxFuture<'static, Option<Vec<(PathBuf, lsp_types::Range)>>> {
    async move {
        match request.await {
            Ok(response) => Some(locations_from_response(response)),
            Err(err) => {
                tracing::debug!(?kind, %err, "location request failed");
                None
            }
        }
    }
    .boxed()
}

/// The markdown of `hover`; `None` when it says nothing.
fn hover_info(
    hover: lsp_types::Hover,
    snapshot: &BufferSnapshot,
    encoding: PositionEncoding,
    language: Option<String>,
) -> Option<HoverInfo> {
    let markdown = match hover.contents {
        HoverContents::Markup(markup) => match markup.kind {
            MarkupKind::Markdown => markup.value,
            MarkupKind::PlainText => escape_markdown(&markup.value),
        },
        HoverContents::Scalar(marked) => marked_string(marked),
        HoverContents::Array(items) => items
            .into_iter()
            .map(marked_string)
            .filter(|s| !s.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n\n"),
    };
    if markdown.trim().is_empty() {
        return None;
    }
    Some(HoverInfo {
        markdown,
        range: hover.range.map(|r| range_from_lsp(snapshot, r, encoding)),
        language,
    })
}

/// A legacy `MarkedString`: a plain string is markdown, a language string
/// a code block in that language.
fn marked_string(marked: MarkedString) -> String {
    match marked {
        MarkedString::String(s) => s,
        MarkedString::LanguageString(ls) => {
            // A fence longer than any backtick run in the code.
            let longest = ls
                .value
                .split(|c| c != '`')
                .map(str::len)
                .max()
                .unwrap_or(0);
            let fence = "`".repeat(longest.max(2) + 1);
            format!("{fence}{}\n{}\n{fence}", ls.language, ls.value)
        }
    }
}

/// Plain text as markdown showing it verbatim: punctuation escaped, line
/// breaks kept, leading indentation as no-break spaces (markdown would
/// drop it, or read four spaces as code), blank lines as paragraph breaks.
fn escape_markdown(text: &str) -> String {
    const NBSP: char = '\u{a0}';
    let mut out = String::with_capacity(text.len() + text.len() / 8);
    // Whether the last line written had text, so the next one continues
    // its paragraph.
    let mut in_paragraph = false;
    let mut blank = false;
    for line in text.lines() {
        if line.trim().is_empty() {
            blank = true;
            continue;
        }
        if in_paragraph {
            // A backslash at the end of a line is a hard break.
            out.push_str(if blank { "\n\n" } else { "\\\n" });
        }
        blank = false;
        in_paragraph = true;
        let body = line.trim_start_matches([' ', '\t']);
        for c in line[..line.len() - body.len()].chars() {
            let width = if c == '\t' { 4 } else { 1 };
            out.extend(std::iter::repeat_n(NBSP, width));
        }
        for c in body.chars() {
            if c.is_ascii_punctuation() {
                out.push('\\');
            }
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_strings_become_fenced_code() {
        let ls = MarkedString::LanguageString(lsp_types::LanguageString {
            language: "rust".into(),
            value: "let s = \"```\";".into(),
        });
        assert_eq!(marked_string(ls), "````rust\nlet s = \"```\";\n````");
        assert_eq!(marked_string(MarkedString::String("*a*".into())), "*a*");
    }

    #[test]
    fn plain_text_is_escaped() {
        assert_eq!(escape_markdown("a *b*\n# c"), "a \\*b\\*\\\n\\# c");
    }

    #[test]
    fn plain_text_keeps_indentation_and_paragraphs() {
        let md = escape_markdown("fn f()\n    indented\n\n  two");
        let nbsp = '\u{a0}';
        assert_eq!(
            md,
            format!(
                "fn f\\(\\)\\\n{}indented\n\n{}two",
                nbsp.to_string().repeat(4),
                nbsp.to_string().repeat(2)
            )
        );
        let blocks = crate::markdown::parse_markdown(&md, None);
        assert!(
            blocks
                .blocks()
                .iter()
                .all(|b| matches!(b, crate::markdown::Block::Paragraph(_))),
            "{blocks:?}"
        );
        assert_eq!(blocks.blocks().len(), 2);
    }
}
