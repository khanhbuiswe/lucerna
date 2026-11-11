//! Open files: loads them off the main thread, parses them, and keeps every
//! [`CodeView`] of a path in step with the file on disk.
//!
//! One [`Document`] per file holds the buffer, its parsed syntax and the
//! views showing it. Each tab gets its own view (with its own scroll
//! position); a reload updates them all. Paths are normalised to a key:
//! absolute paths as given, relative ones joined onto the workbench root,
//! so a tab's path and a worktree change's root-relative path meet.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use gpui::{
    App, AppContext, Context, Entity, EventEmitter, IntoElement, ParentElement, Render, Styled,
    Task, WeakEntity, Window, div, prelude::*, px,
};
use lucerna_project::{ChangeKind, WorktreeEvent};
use lucerna_syntax::{GrammarError, Language, ParseOutcome, SYNC_PARSE_BUDGET, SyntaxState};
use lucerna_text::{Buffer, BufferSnapshot, LineEnding, LoadError, Point, Reload, check_size};

use crate::AppState;
use crate::code_view::CodeView;
use crate::editor_groups::EditorGroupsView;
use crate::tasks::catch_panic;
use crate::ui::GRID;
use crate::workspace_state::absolute_path;

/// Where a document is in its life.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocState {
    /// The first load has not finished.
    Loading,
    Ready,
    /// Loading failed; the view shows the error instead of text.
    Error(LoadError),
    /// The file is gone from disk; the last text stays visible.
    Deleted,
}

/// What the status bar shows for the active document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentStatus {
    /// `Ln {row+1}, Col {col+1}`, the column counted in chars.
    pub cursor: String,
    pub encoding: &'static str,
    /// `LF` or `CRLF`.
    pub line_ending: &'static str,
    /// The language's display name, or `Plain Text`.
    pub language: Arc<str>,
}

/// What happened to a document, for listeners such as the language
/// servers. Paths are the normalised keys of [`Documents`].
#[derive(Debug, Clone)]
pub enum DocumentEvent {
    /// The first text of a document is loaded.
    Opened {
        path: Arc<Path>,
        language_id: Option<Arc<str>>,
        snapshot: BufferSnapshot,
    },
    /// The text changed on disk and was reloaded. Not sent when a reload
    /// left the text as it was, unless the file had been deleted.
    Reloaded {
        path: Arc<Path>,
        snapshot: BufferSnapshot,
    },
    /// The last view of an opened document was dropped. Sent no later than
    /// the frame after (the workbench collects closed documents on each
    /// draw).
    Closed { path: Arc<Path> },
    /// An opened document now has another language (or none).
    LanguageChanged {
        path: Arc<Path>,
        language_id: Option<Arc<str>>,
        snapshot: BufferSnapshot,
    },
    /// A parse of the current text landed: the document has a new syntax
    /// state (a reload first keeps the old tree, edited, until then).
    Parsed { path: Arc<Path> },
}

/// Size and modification time, to tell whether a file changed on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stamp {
    modified: Option<SystemTime>,
    len: u64,
}

impl Stamp {
    fn of(meta: &fs::Metadata) -> Stamp {
        Stamp {
            modified: meta.modified().ok(),
            len: meta.len(),
        }
    }
}

/// Background work on one document. A reload takes the buffer with it so
/// it can diff against the old text.
enum Job {
    Load { force_text: bool },
    Reload(Buffer),
}

enum JobOutput {
    Loaded(Result<(Buffer, Stamp), LoadError>),
    /// The buffer comes back unchanged when the reload failed.
    Reloaded(Buffer, Result<(Reload, Stamp), LoadError>),
}

struct Document {
    state: DocState,
    /// `None` until loaded, and while a reload has it on another thread.
    buffer: Option<Buffer>,
    /// The text the views show; outlives the buffer's trips off-thread.
    snapshot: Option<BufferSnapshot>,
    /// Bumped per new text, so a parse of older text is dropped.
    revision: u64,
    encoding: &'static str,
    line_ending: LineEnding,
    language: Option<Arc<Language>>,
    syntax: Option<Arc<SyntaxState>>,
    force_text: bool,
    stamp: Option<Stamp>,
    views: Vec<WeakEntity<CodeView>>,
    /// The load or reload in flight; at most one per document.
    job: Option<Task<()>>,
    /// Another change arrived while `job` ran: reload once more after it.
    dirty: bool,
    parse: Option<Task<()>>,
    /// Where to put the cursor once the text is loaded.
    pending_go_to: Option<Point>,
    /// [`DocumentEvent::Opened`] was sent.
    announced: bool,
}

impl Document {
    fn views(&self) -> impl Iterator<Item = Entity<CodeView>> + '_ {
        self.views.iter().filter_map(WeakEntity::upgrade)
    }

    fn language_id(&self) -> Option<Arc<str>> {
        self.language.as_ref().map(|l| l.id.clone())
    }

    fn set_state(&mut self, state: DocState, cx: &mut App) {
        for view in self.views() {
            let state = state.clone();
            view.update(cx, |v, cx| v.set_doc_state(state, cx));
        }
        self.state = state;
    }
}

/// Every open document, keyed by normalised path. A GPUI entity owned by
/// the [`crate::Workbench`].
pub struct Documents {
    root: PathBuf,
    docs: HashMap<Arc<Path>, Document>,
    /// Jumps requested for paths whose document does not exist yet.
    pending_go_to: HashMap<Arc<Path>, Point>,
    /// Scroll rows restored for paths whose text is not loaded yet; the
    /// first view to have the text takes its row.
    pending_scroll: HashMap<Arc<Path>, u32>,
}

impl EventEmitter<DocumentEvent> for Documents {}

impl Documents {
    /// Documents of the folder at `root`; relative paths are resolved
    /// against it.
    pub fn new(root: PathBuf) -> Self {
        Documents {
            root,
            docs: HashMap::new(),
            pending_go_to: HashMap::new(),
            pending_scroll: HashMap::new(),
        }
    }

    /// The key for `path`: as given when absolute, else joined onto the
    /// root.
    fn key(&self, path: &Path) -> Arc<Path> {
        Arc::from(absolute_path(&self.root, path))
    }

    /// A new view of `path`. The document is loaded once per path: the
    /// first open starts a background load, later opens share its text,
    /// syntax and reloads but get their own scroll position.
    pub fn open(&mut self, path: Arc<Path>, cx: &mut Context<Self>) -> Entity<CodeView> {
        self.open_view(path, false, cx)
    }

    /// A passive view of `path` for a picker's preview, read-only like
    /// every view: a click does not focus it, and it never takes a jump or
    /// scroll row waiting for the document's other views.
    pub fn open_preview(&mut self, path: Arc<Path>, cx: &mut Context<Self>) -> Entity<CodeView> {
        self.open_view(path, true, cx)
    }

    fn open_view(
        &mut self,
        path: Arc<Path>,
        passive: bool,
        cx: &mut Context<Self>,
    ) -> Entity<CodeView> {
        self.collect_closed(cx);
        let key = self.key(&path);
        let documents = cx.weak_entity();
        let view = cx.new(|cx| {
            let mut view = CodeView::new(path, cx);
            view.set_documents(documents);
            view.set_passive(passive);
            view
        });
        match self.docs.get_mut(&key) {
            Some(doc) => {
                // The language servers feed the views they know of; a new
                // view starts with what its siblings show.
                let diagnostics = doc.views().find_map(|v| v.read(cx).diagnostics_source());
                doc.views.push(view.downgrade());
                let state = doc.state.clone();
                let (snapshot, syntax) = (doc.snapshot.clone(), doc.syntax.clone());
                let language_id = doc.language.as_ref().map(|l| l.id.clone());
                let takes_pending = snapshot.is_some() && !passive;
                let go_to = takes_pending.then(|| doc.pending_go_to.take()).flatten();
                let scroll = takes_pending
                    .then(|| self.pending_scroll.remove(&key))
                    .flatten();
                view.update(cx, |v, cx| {
                    v.set_language_id(language_id, cx);
                    if let Some(snapshot) = snapshot {
                        v.set_buffer(snapshot, cx);
                        v.set_syntax(syntax, cx);
                    }
                    if let Some(diagnostics) = diagnostics {
                        v.set_diagnostics(diagnostics, cx);
                    }
                    // A jump centers its row; a scroll row asked for with it
                    // (Go Back) wins over that.
                    if let Some(p) = go_to {
                        v.go_to(p, cx);
                    }
                    if let Some(row) = scroll {
                        v.set_scroll_top_row(row, cx);
                    }
                    v.set_doc_state(state, cx);
                });
            }
            None => {
                // The first line is not known yet; the load looks again.
                let language = language_for(&self.root, &key, None, cx);
                let language_id = language.as_ref().map(|l| l.id.clone());
                view.update(cx, |v, cx| {
                    v.set_language_id(language_id, cx);
                    v.set_doc_state(DocState::Loading, cx);
                });
                self.docs.insert(
                    key.clone(),
                    Document {
                        state: DocState::Loading,
                        buffer: None,
                        snapshot: None,
                        revision: 0,
                        encoding: "UTF-8",
                        line_ending: LineEnding::Lf,
                        language,
                        syntax: None,
                        force_text: false,
                        stamp: None,
                        views: vec![view.downgrade()],
                        job: None,
                        dirty: false,
                        parse: None,
                        pending_go_to: self.pending_go_to.remove(&key),
                        announced: false,
                    },
                );
                self.start_job(&key, cx);
            }
        }
        cx.notify();
        view
    }

    /// The language registry was replaced: every open document looks its
    /// language up again and is parsed afresh, since its syntax resolves
    /// injected languages through the old registry.
    pub fn languages_changed(&mut self, cx: &mut Context<Self>) {
        self.collect_closed(cx);
        let keys: Vec<Arc<Path>> = self.docs.keys().cloned().collect();
        for key in keys {
            let doc = self.docs.get_mut(&key).expect("key from the map");
            // Even an unchanged root language is parsed again: the new
            // registry may add or drop the languages it injects.
            let old_id = doc.language_id();
            doc.language = language_for(&self.root, &key, doc.snapshot.as_ref(), cx);
            sync_language_id(doc, cx);
            if doc.announced
                && doc.language_id() != old_id
                && let Some(snapshot) = doc.snapshot.clone()
            {
                cx.emit(DocumentEvent::LanguageChanged {
                    path: key.clone(),
                    language_id: doc.language_id(),
                    snapshot,
                });
            }
            if doc.language.is_none() {
                // Cancels a parse in flight with the old language.
                doc.parse = None;
                clear_syntax(doc, cx);
            } else {
                // In the background: a swap must not parse every open file
                // on the UI thread. The old syntax stays shown meanwhile.
                self.parse(&key, false, cx);
            }
        }
        cx.notify();
    }

    /// The live views of the document at `path`.
    pub fn views_of(&self, path: &Path) -> Vec<Entity<CodeView>> {
        self.docs
            .get(&self.key(path))
            .map(|doc| doc.views().collect())
            .unwrap_or_default()
    }

    /// The text of the document at `path`, once loaded.
    pub fn snapshot_of(&self, path: &Path) -> Option<BufferSnapshot> {
        self.docs.get(&self.key(path))?.snapshot.clone()
    }

    /// Bumped per new text of the document at `path`; `None` until its
    /// text is loaded.
    pub fn revision_of(&self, path: &Path) -> Option<u64> {
        let doc = self.docs.get(&self.key(path))?;
        doc.snapshot.as_ref().map(|_| doc.revision)
    }

    /// The language of the document at `path` and its syntax state, once
    /// parsed.
    pub(crate) fn syntax_of(
        &self,
        path: &Path,
    ) -> Option<(Arc<Language>, Option<Arc<SyntaxState>>)> {
        let doc = self.docs.get(&self.key(path))?;
        Some((doc.language.clone()?, doc.syntax.clone()))
    }

    /// The language id of the document at `path`.
    pub fn language_of(&self, path: &Path) -> Option<Arc<str>> {
        self.docs.get(&self.key(path))?.language_id()
    }

    /// The keys of the documents that have been opened (their text has
    /// loaded) and still have a view.
    pub fn open_paths(&self) -> Vec<Arc<Path>> {
        self.docs
            .iter()
            .filter(|(_, doc)| doc.announced && doc.views().next().is_some())
            .map(|(key, _)| key.clone())
            .collect()
    }

    /// The state of the document at `path`, if it is open.
    pub fn state(&self, path: &Path) -> Option<&DocState> {
        self.docs.get(&self.key(path)).map(|d| &d.state)
    }

    /// Puts the cursor of `view` at `point` and scrolls to it, now if its
    /// text is loaded, else as soon as it is.
    pub fn go_to(&mut self, view: &Entity<CodeView>, point: Point, cx: &mut Context<Self>) {
        let key = self.key(view.read(cx).path());
        match self.docs.get_mut(&key) {
            Some(doc) if doc.snapshot.is_none() => {
                doc.pending_go_to = Some(point);
                // The jump supersedes a restored scroll row.
                self.pending_scroll.remove(&key);
            }
            _ => view.update(cx, |v, cx| v.go_to(point, cx)),
        }
    }

    /// Remembers `point` for the next view of `path` to open: it jumps
    /// there once its text is loaded.
    pub(crate) fn go_to_when_opened(&mut self, path: &Path, point: Point) {
        let key = self.key(path);
        // The jump supersedes a restored scroll row.
        self.pending_scroll.remove(&key);
        match self.docs.get_mut(&key) {
            Some(doc) => doc.pending_go_to = Some(point),
            None => {
                self.pending_go_to.insert(key, point);
            }
        }
    }

    /// [`Documents::go_to`], then scrolls `row` to the top of `view`
    /// instead of centering the cursor.
    pub fn go_to_and_scroll(
        &mut self,
        view: &Entity<CodeView>,
        point: Point,
        row: u32,
        cx: &mut Context<Self>,
    ) {
        let key = self.key(view.read(cx).path());
        match self.docs.get_mut(&key) {
            Some(doc) if doc.snapshot.is_none() => {
                doc.pending_go_to = Some(point);
                self.pending_scroll.insert(key, row);
            }
            _ => view.update(cx, |v, cx| {
                v.go_to(point, cx);
                v.set_scroll_top_row(row, cx);
            }),
        }
    }

    /// [`Documents::go_to_when_opened`] with a scroll row.
    pub(crate) fn go_to_and_scroll_when_opened(&mut self, path: &Path, point: Point, row: u32) {
        self.go_to_when_opened(path, point);
        self.scroll_when_opened(path, row);
    }

    /// Remembers a scroll row for `path`: its next view shows that row as
    /// soon as the text is loaded.
    pub(crate) fn scroll_when_opened(&mut self, path: &Path, row: u32) {
        let key = self.key(path);
        self.pending_scroll.insert(key, row);
    }

    /// Whether a restored scroll row for `path` still waits for its text.
    pub(crate) fn has_pending_scroll(&self, path: &Path) -> bool {
        self.pending_scroll.contains_key(&self.key(path))
    }

    /// Loads `path` again after an error.
    pub fn retry(&mut self, path: &Path, cx: &mut Context<Self>) {
        let key = self.key(path);
        let Some(doc) = self.docs.get_mut(&key) else {
            return;
        };
        if doc.snapshot.is_none() {
            doc.set_state(DocState::Loading, cx);
        }
        self.start_job(&key, cx);
        cx.notify();
    }

    /// Loads `path` as text even though it looks binary. Remembered for
    /// later reloads.
    pub fn open_as_text(&mut self, path: &Path, cx: &mut Context<Self>) {
        let key = self.key(path);
        let Some(doc) = self.docs.get_mut(&key) else {
            return;
        };
        doc.force_text = true;
        // A buffer decoded without `force_text` would keep refusing.
        doc.buffer = None;
        if doc.snapshot.is_none() {
            doc.set_state(DocState::Loading, cx);
        }
        self.start_job(&key, cx);
        cx.notify();
    }

    /// Follows the files on disk: reloads modified files, marks removed
    /// ones deleted, and after a rescan checks every open file.
    pub fn on_worktree_event(&mut self, ev: &WorktreeEvent, cx: &mut Context<Self>) {
        self.collect_closed(cx);
        match ev {
            WorktreeEvent::Changed(changes) => {
                for change in changes {
                    let key = self.key(&change.path);
                    let Some(doc) = self.docs.get_mut(&key) else {
                        continue;
                    };
                    match change.kind {
                        ChangeKind::Modified => self.start_job(&key, cx),
                        ChangeKind::Removed => {
                            doc.set_state(DocState::Deleted, cx);
                            // A load in flight may have read the old file.
                            if doc.job.is_some() {
                                doc.dirty = true;
                            }
                        }
                        ChangeKind::Added => {
                            if matches!(
                                doc.state,
                                DocState::Deleted | DocState::Error(LoadError::NotFound)
                            ) {
                                self.start_job(&key, cx);
                            }
                        }
                    }
                }
            }
            WorktreeEvent::Rescanned => {
                let keys: Vec<Arc<Path>> = self.docs.keys().cloned().collect();
                for key in keys {
                    let doc = &self.docs[&key];
                    match fs::metadata(&key) {
                        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                            if doc.snapshot.is_some() && doc.state != DocState::Deleted {
                                self.docs
                                    .get_mut(&key)
                                    .expect("key from the map")
                                    .set_state(DocState::Deleted, cx);
                            }
                        }
                        Ok(meta)
                            if doc.stamp != Some(Stamp::of(&meta))
                                || doc.state == DocState::Deleted =>
                        {
                            self.start_job(&key, cx)
                        }
                        _ => {}
                    }
                }
            }
            WorktreeEvent::ScanProgress | WorktreeEvent::ScanComplete => {}
        }
        cx.notify();
    }

    /// The status-bar segments for `view`, once its document has text.
    pub fn status(&self, view: &CodeView) -> Option<DocumentStatus> {
        let doc = self.docs.get(&self.key(view.path()))?;
        doc.snapshot.as_ref()?;
        let cursor = view.cursor();
        // Runs every frame: never copy the line, which may be megabytes.
        let col = view.snapshot().char_column(cursor);
        Some(DocumentStatus {
            cursor: format!("Ln {}, Col {}", cursor.row + 1, col + 1),
            encoding: doc.encoding,
            line_ending: match doc.line_ending {
                LineEnding::Lf => "LF",
                LineEnding::CrLf => "CRLF",
            },
            language: doc
                .language
                .as_ref()
                .map_or_else(|| Arc::from("Plain Text"), |l| l.name.clone()),
        })
    }

    /// Drops documents whose every view has closed, sending
    /// [`DocumentEvent::Closed`] for those that were opened. The workbench
    /// calls it after each draw.
    pub fn collect_closed(&mut self, cx: &mut Context<Self>) {
        let pending_go_to = &mut self.pending_go_to;
        self.docs.retain(|key, doc| {
            doc.views.retain(|v| v.upgrade().is_some());
            let keep = !doc.views.is_empty();
            // A jump no view took yet (one only a picker's preview held the
            // document for) waits for the next view of the path.
            if !keep && let Some(point) = doc.pending_go_to.take() {
                pending_go_to.insert(key.clone(), point);
            }
            if !keep && doc.announced {
                cx.emit(DocumentEvent::Closed { path: key.clone() });
            }
            keep
        });
    }

    /// Loads or reloads the document off the main thread. While one job
    /// runs, further requests collapse into a single follow-up reload.
    fn start_job(&mut self, key: &Arc<Path>, cx: &mut Context<Self>) {
        let Some(doc) = self.docs.get_mut(key) else {
            return;
        };
        if doc.job.is_some() {
            doc.dirty = true;
            return;
        }
        let job = match doc.buffer.take() {
            Some(buffer) => Job::Reload(buffer),
            None => Job::Load {
                force_text: doc.force_text,
            },
        };
        let path = key.clone();
        let work = spawn_contained(cx, move || run_job(&path, job));
        let key = key.clone();
        doc.job = Some(cx.spawn(async move |this, cx| {
            let output = work.await;
            this.update(cx, |docs, cx| docs.finish_job(&key, output, cx))
                .ok();
        }));
    }

    fn finish_job(
        &mut self,
        key: &Arc<Path>,
        output: Result<JobOutput, LoadError>,
        cx: &mut Context<Self>,
    ) {
        let Some(doc) = self.docs.get_mut(key) else {
            return;
        };
        doc.job = None;
        let mut parse = false;
        match output {
            Ok(JobOutput::Loaded(Ok((buffer, stamp)))) => {
                let snapshot = buffer.snapshot();
                take_buffer(doc, buffer, stamp);
                let old_id = doc.language_id();
                // Now with the first line, for `first_line_pattern`.
                doc.language = language_for(&self.root, key, Some(&snapshot), cx);
                sync_language_id(doc, cx);
                let (path, language_id) = (key.clone(), doc.language_id());
                // A document loaded again (after an error, or as text) was
                // already announced: it changed rather than opened.
                cx.emit(if !doc.announced {
                    doc.announced = true;
                    DocumentEvent::Opened {
                        path,
                        language_id,
                        snapshot: snapshot.clone(),
                    }
                } else if language_id != old_id {
                    DocumentEvent::LanguageChanged {
                        path,
                        language_id,
                        snapshot: snapshot.clone(),
                    }
                } else {
                    DocumentEvent::Reloaded {
                        path,
                        snapshot: snapshot.clone(),
                    }
                });
                for view in doc.views() {
                    let snapshot = snapshot.clone();
                    view.update(cx, |v, cx| v.set_buffer(snapshot, cx));
                }
                // A requested jump moves the newest view only. A preview
                // never takes it: without another view it waits for the
                // next one to open.
                let views: Vec<_> = doc.views().filter(|v| !v.read(cx).is_passive()).collect();
                if let Some(view) = views.last()
                    && let Some(p) = doc.pending_go_to.take()
                {
                    view.update(cx, |v, cx| v.go_to(p, cx));
                }
                // After the jump, which centers its row: a scroll row asked
                // for with it (Go Back) wins.
                if !views.is_empty()
                    && let Some(row) = self.pending_scroll.remove(key)
                {
                    for view in &views {
                        view.update(cx, |v, cx| v.set_scroll_top_row(row, cx));
                    }
                }
                doc.set_state(DocState::Ready, cx);
                parse = true;
            }
            Ok(JobOutput::Reloaded(buffer, Ok((reload, stamp))))
                if unchanged(doc, &buffer, &reload) =>
            {
                // `touch` or a rewrite with the same text: nothing to show,
                // and no new revision for the views to redo.
                doc.buffer = Some(buffer);
                doc.stamp = Some(stamp);
                // A deleted file came back as it was: listeners that let
                // it go take it up again.
                if doc.state == DocState::Deleted
                    && let Some(snapshot) = doc.snapshot.clone()
                {
                    cx.emit(DocumentEvent::Reloaded {
                        path: key.clone(),
                        snapshot,
                    });
                }
                doc.set_state(DocState::Ready, cx);
            }
            Ok(JobOutput::Reloaded(buffer, Ok((reload, stamp)))) => {
                tracing::debug!(path = %key.display(), "document reloaded");
                let snapshot = buffer.snapshot();
                let old = doc.snapshot.clone();
                take_buffer(doc, buffer, stamp);
                // The old tree, edited to the new text, keeps the colours
                // until the new parse lands; views edit their own copy.
                doc.syntax = match (doc.syntax.take(), old) {
                    (Some(syntax), Some(old)) => {
                        catch_panic("edit syntax", || syntax.edited(&old, &snapshot, &reload))
                            .ok()
                            .map(Arc::new)
                    }
                    _ => None,
                };
                for view in doc.views() {
                    let snapshot = snapshot.clone();
                    view.update(cx, |v, cx| v.apply_reload(snapshot, reload, cx));
                }
                cx.emit(DocumentEvent::Reloaded {
                    path: key.clone(),
                    snapshot,
                });
                doc.set_state(DocState::Ready, cx);
                parse = true;
            }
            Ok(JobOutput::Loaded(Err(err))) => {
                let state = failure_state(doc, err);
                doc.set_state(state, cx);
            }
            Ok(JobOutput::Reloaded(buffer, Err(err))) => {
                doc.buffer = Some(buffer);
                let state = failure_state(doc, err);
                doc.set_state(state, cx);
            }
            Err(err) => {
                // The job panicked and its buffer went with it.
                doc.buffer = None;
                doc.set_state(DocState::Error(err), cx);
            }
        }
        let again = std::mem::take(&mut doc.dirty);
        if parse {
            self.parse(key, true, cx);
        }
        if again {
            self.start_job(key, cx);
        }
        cx.notify();
    }

    /// Parses the current text: within [`SYNC_PARSE_BUDGET`] here when
    /// `try_sync`, else (or when that times out) to completion on a
    /// background thread. Large files are not parsed.
    fn parse(&mut self, key: &Arc<Path>, try_sync: bool, cx: &mut Context<Self>) {
        let Some(doc) = self.docs.get_mut(key) else {
            return;
        };
        let settings = &cx.global::<AppState>().settings;
        let registry = cx.global::<AppState>().languages.clone();
        let language = doc.language.as_ref().map(|l| l.id.clone());
        let max_bytes = settings
            .get::<u64>("lucerna.largeFile.maxBytes", language.as_deref())
            .unwrap_or(20 * 1024 * 1024);
        let max_lines = settings
            .get::<u32>("lucerna.largeFile.maxLines", language.as_deref())
            .unwrap_or(300_000);
        // A parse of older text is cancelled; its syntax stays shown (see
        // `finish_job`) until this one lands.
        doc.parse = None;
        let (Some(language), Some(buffer)) = (doc.language.clone(), doc.buffer.as_ref()) else {
            return;
        };
        if buffer.is_large(max_bytes, max_lines) {
            clear_syntax(doc, cx);
            return;
        }
        let snapshot = buffer.snapshot();
        let revision = doc.revision;
        // A WASM grammar's first load compiles a module, far over the sync
        // budget: it happens in the background job. Otherwise, on the UI
        // thread, a panic in the grammar must not take the app down (spec
        // §7); it is logged and the file shows as plain text. An injected
        // language whose WASM grammar is not loaded yet makes the sync parse
        // time out, so it loads here too.
        let mut started = None;
        if try_sync && language.grammar_ready() {
            let sync = catch_panic("parse", || {
                let mut syntax = SyntaxState::new(language.clone(), registry.clone())?;
                let outcome = syntax.parse(&snapshot, Some(SYNC_PARSE_BUDGET));
                Ok::<_, GrammarError>((syntax, outcome))
            });
            match sync {
                Ok(Ok((syntax, ParseOutcome::Done))) => {
                    set_syntax(doc, Arc::new(syntax), cx);
                    cx.emit(DocumentEvent::Parsed { path: key.clone() });
                    return;
                }
                Ok(Ok((syntax, ParseOutcome::TimedOut))) => started = Some(syntax),
                Ok(Err(err)) => {
                    tracing::warn!(path = %key.display(), %err, "grammar failed to load");
                    clear_syntax(doc, cx);
                    return;
                }
                Err(_) => {
                    clear_syntax(doc, cx);
                    return;
                }
            }
        }
        let work = spawn_contained(cx, move || {
            let mut syntax = match started {
                Some(syntax) => syntax,
                None => SyntaxState::new(language, registry)?,
            };
            syntax.parse(&snapshot, None);
            Ok::<_, GrammarError>(syntax)
        });
        let key = key.clone();
        doc.parse = Some(cx.spawn(async move |this, cx| {
            let syntax = work.await;
            this.update(cx, |docs, cx| {
                let Some(doc) = docs.docs.get_mut(&key) else {
                    return;
                };
                if doc.revision != revision {
                    return;
                }
                doc.parse = None;
                match syntax {
                    Ok(Ok(syntax)) => {
                        set_syntax(doc, Arc::new(syntax), cx);
                        cx.emit(DocumentEvent::Parsed { path: key.clone() });
                    }
                    Ok(Err(err)) => {
                        tracing::warn!(path = %key.display(), %err, "grammar failed to load");
                        clear_syntax(doc, cx);
                    }
                    // The text stays readable without highlighting.
                    Err(err) => tracing::error!(path = %key.display(), %err, "parse failed"),
                }
            })
            .ok();
        }));
    }
}

/// Whether a reload left the text exactly as shown: no changed lines, the
/// same length, encoding and line ending.
fn unchanged(doc: &Document, buffer: &Buffer, reload: &Reload) -> bool {
    reload.start_row == reload.old_end_row
        && reload.old_end_row == reload.new_end_row
        && doc
            .snapshot
            .as_ref()
            .is_some_and(|s| s.len_bytes() == buffer.snapshot().len_bytes())
        && buffer.encoding().name() == doc.encoding
        && buffer.line_ending() == doc.line_ending
}

fn take_buffer(doc: &mut Document, buffer: Buffer, stamp: Stamp) {
    doc.snapshot = Some(buffer.snapshot());
    doc.encoding = buffer.encoding().name();
    doc.line_ending = buffer.line_ending();
    doc.buffer = Some(buffer);
    doc.stamp = Some(stamp);
    doc.revision += 1;
}

/// A file that vanished after it was shown is deleted; any other failure
/// is an error.
fn failure_state(doc: &Document, err: LoadError) -> DocState {
    if err == LoadError::NotFound && doc.snapshot.is_some() {
        DocState::Deleted
    } else {
        DocState::Error(err)
    }
}

/// Shows the document as plain text.
fn clear_syntax(doc: &mut Document, cx: &mut App) {
    if doc.syntax.take().is_some() {
        for view in doc.views() {
            view.update(cx, |v, cx| v.set_syntax(None, cx));
        }
    }
}

fn set_syntax(doc: &mut Document, syntax: Arc<SyntaxState>, cx: &mut App) {
    for view in doc.views() {
        let syntax = syntax.clone();
        view.update(cx, |v, cx| v.set_syntax(Some(syntax), cx));
    }
    doc.syntax = Some(syntax);
}

/// Gives every view of `doc` its language's id.
fn sync_language_id(doc: &Document, cx: &mut App) {
    let id = doc.language.as_ref().map(|l| l.id.clone());
    for view in doc.views() {
        let id = id.clone();
        view.update(cx, |v, cx| v.set_language_id(id, cx));
    }
}

/// The language of the file at `key`: by `files.associations` against its
/// path relative to `root`, its name, and the first line of `snapshot`.
fn language_for(
    root: &Path,
    key: &Path,
    snapshot: Option<&BufferSnapshot>,
    cx: &App,
) -> Option<Arc<Language>> {
    let path = key.strip_prefix(root).unwrap_or(key);
    let first_line = snapshot.map(first_line).unwrap_or_default();
    cx.global::<AppState>()
        .languages
        .language_for_file(path, &first_line)
}

/// Most bytes of the first line handed to language detection.
const FIRST_LINE_BYTES: usize = 256;

/// The first line of `snapshot`, cut to [`FIRST_LINE_BYTES`] (on a char
/// boundary) without copying the rest of a long line.
fn first_line(snapshot: &BufferSnapshot) -> String {
    let mut end = snapshot.len_bytes().min(FIRST_LINE_BYTES);
    // Back off continuation bytes so the cut lands on a char boundary.
    while end > 0 && end < snapshot.len_bytes() && snapshot.byte_at(end) & 0xC0 == 0x80 {
        end -= 1;
    }
    let mut line = String::new();
    for chunk in snapshot.chunks_in_range(0..end) {
        match chunk.find('\n') {
            Some(i) => {
                line.push_str(&chunk[..i]);
                break;
            }
            None => line.push_str(chunk),
        }
    }
    line.truncate(line.trim_end_matches('\r').len());
    line
}

/// Reads `path` whole, refusing files over the size limit.
fn read(path: &Path) -> Result<(Vec<u8>, Stamp), LoadError> {
    let meta = fs::metadata(path)?;
    check_size(meta.len())?;
    let bytes = fs::read(path)?;
    Ok((bytes, Stamp::of(&meta)))
}

fn run_job(path: &Path, job: Job) -> JobOutput {
    match job {
        Job::Load { force_text } => JobOutput::Loaded(
            read(path)
                .and_then(|(bytes, stamp)| Ok((Buffer::from_bytes(&bytes, force_text)?, stamp))),
        ),
        Job::Reload(mut buffer) => {
            let result = read(path).and_then(|(bytes, stamp)| Ok((buffer.reload(&bytes)?, stamp)));
            JobOutput::Reloaded(buffer, result)
        }
    }
}

/// Runs `f` on the background executor; a panic becomes
/// [`LoadError::Io`] instead of taking the app down.
fn spawn_contained<T: Send + 'static>(
    cx: &App,
    f: impl FnOnce() -> T + Send + 'static,
) -> Task<Result<T, LoadError>> {
    cx.background_executor().spawn(async move { contain(f) })
}

fn contain<T>(f: impl FnOnce() -> T) -> Result<T, LoadError> {
    catch_panic("documents", f)
        .map_err(|message| LoadError::Io(format!("internal error: {message}")))
}

/// The status-bar segments of the active document: cursor, encoding, line
/// ending and language.
pub(crate) struct DocumentStatusItem {
    groups: Entity<EditorGroupsView>,
    documents: Entity<Documents>,
}

impl DocumentStatusItem {
    pub(crate) fn new(groups: Entity<EditorGroupsView>, documents: Entity<Documents>) -> Self {
        DocumentStatusItem { groups, documents }
    }
}

impl Render for DocumentStatusItem {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status = self
            .groups
            .read(cx)
            .active_body()
            .and_then(|v| v.downcast::<CodeView>().ok())
            .and_then(|view| self.documents.read(cx).status(view.read(cx)));
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(4. * GRID))
            .when_some(status, |d, status| {
                d.child(
                    div()
                        .debug_selector(|| "status-cursor".into())
                        .child(status.cursor),
                )
                .child(status.encoding)
                .child(status.line_ending)
                .child(status.language.to_string())
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contain_turns_a_panic_into_an_io_error() {
        let result = contain(|| -> u32 { panic!("boom") });
        assert_eq!(result, Err(LoadError::Io("internal error: boom".into())));
        assert_eq!(contain(|| 7), Ok(7));
    }
}
