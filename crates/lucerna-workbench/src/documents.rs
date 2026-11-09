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
    App, AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled, Task, WeakEntity,
    Window, div, prelude::*, px,
};
use lucerna_project::{ChangeKind, WorktreeEvent};
use lucerna_syntax::{Language, ParseOutcome, SYNC_PARSE_BUDGET, SyntaxState};
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
    pub language: &'static str,
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
}

impl Document {
    fn views(&self) -> impl Iterator<Item = Entity<CodeView>> + '_ {
        self.views.iter().filter_map(WeakEntity::upgrade)
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
        self.collect_closed();
        let key = self.key(&path);
        let documents = cx.weak_entity();
        let view = cx.new(|cx| {
            let mut view = CodeView::new(path, cx);
            view.set_documents(documents);
            view
        });
        match self.docs.get_mut(&key) {
            Some(doc) => {
                doc.views.push(view.downgrade());
                let state = doc.state.clone();
                let (snapshot, syntax) = (doc.snapshot.clone(), doc.syntax.clone());
                let go_to = snapshot.as_ref().and(doc.pending_go_to.take());
                let scroll = snapshot
                    .as_ref()
                    .and_then(|_| self.pending_scroll.remove(&key));
                view.update(cx, |v, cx| {
                    if let Some(snapshot) = snapshot {
                        v.set_buffer(snapshot, cx);
                        v.set_syntax(syntax, cx);
                    }
                    if let Some(row) = scroll {
                        v.set_scroll_top_row(row, cx);
                    }
                    if let Some(p) = go_to {
                        v.go_to(p, cx);
                    }
                    v.set_doc_state(state, cx);
                });
            }
            None => {
                let language = cx.global::<AppState>().languages.language_for_path(&key);
                view.update(cx, |v, cx| v.set_doc_state(DocState::Loading, cx));
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
                    },
                );
                self.start_job(&key, cx);
            }
        }
        cx.notify();
        view
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
            Some(doc) if doc.snapshot.is_none() => doc.pending_go_to = Some(point),
            _ => view.update(cx, |v, cx| v.go_to(point, cx)),
        }
    }

    /// Remembers `point` for the next view of `path` to open: it jumps
    /// there once its text is loaded.
    pub(crate) fn go_to_when_opened(&mut self, path: &Path, point: Point) {
        let key = self.key(path);
        match self.docs.get_mut(&key) {
            Some(doc) => doc.pending_go_to = Some(point),
            None => {
                self.pending_go_to.insert(key, point);
            }
        }
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
        self.collect_closed();
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
            language: doc.language.as_ref().map_or("Plain Text", |l| l.name),
        })
    }

    /// Drops documents whose every view has closed.
    fn collect_closed(&mut self) {
        self.docs.retain(|_, doc| {
            doc.views.retain(|v| v.upgrade().is_some());
            !doc.views.is_empty()
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
                for view in doc.views() {
                    let snapshot = snapshot.clone();
                    view.update(cx, |v, cx| v.set_buffer(snapshot, cx));
                }
                if let Some(row) = self.pending_scroll.remove(key) {
                    for view in doc.views() {
                        view.update(cx, |v, cx| v.set_scroll_top_row(row, cx));
                    }
                }
                // A requested jump moves the newest view only.
                if let (Some(p), Some(view)) = (doc.pending_go_to.take(), doc.views().last()) {
                    view.update(cx, |v, cx| v.go_to(p, cx));
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
                        Some(Arc::new(syntax.edited(&old, &snapshot, &reload)))
                    }
                    _ => None,
                };
                for view in doc.views() {
                    let snapshot = snapshot.clone();
                    view.update(cx, |v, cx| v.apply_reload(snapshot, reload, cx));
                }
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
            self.parse(key, cx);
        }
        if again {
            self.start_job(key, cx);
        }
        cx.notify();
    }

    /// Parses the current text: within [`SYNC_PARSE_BUDGET`] here, else to
    /// completion on a background thread. Large files are not parsed.
    fn parse(&mut self, key: &Arc<Path>, cx: &mut Context<Self>) {
        let settings = &cx.global::<AppState>().settings;
        let max_bytes = settings
            .get::<u64>("lucerna.largeFile.maxBytes", None)
            .unwrap_or(20 * 1024 * 1024);
        let max_lines = settings
            .get::<u32>("lucerna.largeFile.maxLines", None)
            .unwrap_or(300_000);
        let Some(doc) = self.docs.get_mut(key) else {
            return;
        };
        // A parse of older text is cancelled; its syntax stays shown (see
        // `finish_job`) until this one lands.
        doc.parse = None;
        let (Some(language), Some(buffer)) = (doc.language.clone(), doc.buffer.as_ref()) else {
            return;
        };
        if buffer.is_large(max_bytes, max_lines) {
            if doc.syntax.take().is_some() {
                for view in doc.views() {
                    view.update(cx, |v, cx| v.set_syntax(None, cx));
                }
            }
            return;
        }
        let snapshot = buffer.snapshot();
        let mut syntax = SyntaxState::new(language);
        if syntax.parse(&snapshot, Some(SYNC_PARSE_BUDGET)) == ParseOutcome::Done {
            set_syntax(doc, Arc::new(syntax), cx);
            return;
        }
        let revision = doc.revision;
        let work = spawn_contained(cx, move || {
            syntax.parse(&snapshot, None);
            syntax
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
                    Ok(syntax) => set_syntax(doc, Arc::new(syntax), cx),
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

fn set_syntax(doc: &mut Document, syntax: Arc<SyntaxState>, cx: &mut App) {
    for view in doc.views() {
        let syntax = syntax.clone();
        view.update(cx, |v, cx| v.set_syntax(Some(syntax), cx));
    }
    doc.syntax = Some(syntax);
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
                .child(status.language)
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
