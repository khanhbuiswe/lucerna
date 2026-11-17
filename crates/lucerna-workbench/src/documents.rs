//! Open files: loads them off the main thread, parses them, and keeps every
//! [`CodeView`] of a path in step with the file on disk.
//!
//! One [`Document`] per file holds the buffer, its undo history, its parsed
//! syntax and the views showing it. Each tab gets its own view (with its own
//! scroll position); an edit, an undo or a reload updates them all. The
//! buffer never leaves the main thread: a reload reads and diffs the file in
//! the background, then applies the diff here as one undoable edit. Paths are normalised to a key:
//! absolute paths as given, relative ones joined onto the workbench root,
//! so a tab's path and a worktree change's root-relative path meet.

use std::cell::Cell;
use std::collections::HashMap;
use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use futures::FutureExt as _;
use futures::channel::oneshot;
use futures::future::Shared;
use gpui::{
    App, AppContext, Context, Entity, EntityId, EventEmitter, IntoElement, ParentElement,
    PromptLevel, Render, Styled, Task, WeakEntity, Window, WindowId, div, prelude::*, px,
};
use lucerna_editor::indentation::{self, Indentation};
use lucerna_editor::merge::merge;
use lucerna_project::{ChangeKind, WorktreeEvent};
use lucerna_settings::SettingsView;
use lucerna_syntax::{GrammarError, Language, ParseOutcome, SYNC_PARSE_BUDGET, SyntaxState};
use lucerna_text::{
    Buffer, BufferSnapshot, Decoded, HistoryState, LineEnding, LoadError, Patch, PatchEdit, Point,
    SelectionsToken, WriteError, Written, check_regular, check_size, decode_file, diff_edits,
};

use crate::WindowSettings;
use crate::code_view::CodeView;
use crate::editor_groups::EditorGroupsView;
use crate::hot_exit::store::{Backup, text_sha256};
use crate::save::{
    self, CONFLICT_ANSWERS, CONFLICT_MESSAGE, SaveOptions, SaveReason, WriteJob, WriteOutcome,
};
use crate::status_message;
use crate::tasks::catch_panic;
use crate::ui::GRID;
use crate::untitled;
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

/// A change for the view that made it, which updates itself (see
/// [`Documents::apply_edits_from_view`]): `patch` turned `old` into
/// `snapshot`, and `syntax` is the document's syntax after it.
pub(crate) struct ViewUpdate {
    pub(crate) old: BufferSnapshot,
    pub(crate) snapshot: BufferSnapshot,
    pub(crate) patch: Patch,
    pub(crate) syntax: Option<Arc<SyntaxState>>,
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
    /// `Spaces: {n}` or `Tab Size: {n}`: the indentation in effect.
    pub indentation: String,
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
    /// The text changed from `old` to `snapshot` by `patch`: an edit from
    /// the view `origin`, or (`origin: None`) an undo, a redo or a reload
    /// from disk. Also sent with an empty patch when a deleted file came
    /// back as it was, so listeners that let it go take it up again.
    Edited {
        path: Arc<Path>,
        old: BufferSnapshot,
        snapshot: BufferSnapshot,
        patch: Arc<Patch>,
        origin: Option<EntityId>,
    },
    /// The text now differs from (or again matches) the file on disk.
    DirtyChanged { path: Arc<Path>, dirty: bool },
    /// The text was written to disk: `snapshot` is the text written, which
    /// edits made while writing may have changed since.
    Saved {
        path: Arc<Path>,
        snapshot: BufferSnapshot,
    },
    /// The file changed on disk while the text had unsaved changes (or that
    /// state ended); the text was left alone.
    ConflictChanged { path: Arc<Path>, conflict: bool },
    /// A reload found the file's content different from the text's base:
    /// another program changed it (never sent for our own saves, or for a
    /// rewrite with the same bytes).
    ChangedOnDisk { path: Arc<Path> },
    /// A view of the document lost focus to something outside it (not to
    /// its own find widget or hover); `files.autoSave: onFocusChange`.
    FocusLost { path: Arc<Path> },
    /// The last view of an opened document was dropped. Sent no later than
    /// the frame after (the workbench collects closed documents on each
    /// draw).
    Closed { path: Arc<Path> },
    /// The unsaved text at `path` was let go on purpose (Don't Save, Close
    /// on a deleted file's bar): sent by [`Documents::discard`] before any
    /// `Closed`, so hot exit deletes the backup even while the app quits.
    Discarded { path: Arc<Path> },
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

impl DocumentEvent {
    /// The document the event is about.
    pub fn path(&self) -> &Arc<Path> {
        match self {
            DocumentEvent::Opened { path, .. }
            | DocumentEvent::Edited { path, .. }
            | DocumentEvent::DirtyChanged { path, .. }
            | DocumentEvent::Saved { path, .. }
            | DocumentEvent::ConflictChanged { path, .. }
            | DocumentEvent::ChangedOnDisk { path }
            | DocumentEvent::FocusLost { path }
            | DocumentEvent::Closed { path }
            | DocumentEvent::Discarded { path }
            | DocumentEvent::LanguageChanged { path, .. }
            | DocumentEvent::Parsed { path } => path,
        }
    }
}

/// Size and modification time, to tell whether a file changed on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Stamp {
    modified: Option<SystemTime>,
    len: u64,
}

impl Stamp {
    pub(crate) fn of(meta: &fs::Metadata) -> Stamp {
        Stamp {
            modified: meta.modified().ok(),
            len: meta.len(),
        }
    }

    /// The stamp of the file a save wrote, taken from its handle.
    pub(crate) fn of_written(written: &Written) -> Stamp {
        Stamp {
            modified: written.modified,
            len: written.len,
        }
    }

    /// The modification time as seconds and nanoseconds since the Unix
    /// epoch (`None` when unknown or before it), and the size: how a
    /// hot exit backup keeps the stamp.
    pub(crate) fn to_parts(self) -> (Option<(u64, u32)>, u64) {
        let modified = self
            .modified
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|d| (d.as_secs(), d.subsec_nanos()));
        (modified, self.len)
    }

    /// The stamp [`Stamp::to_parts`] gave these parts.
    pub(crate) fn from_parts(modified: Option<(u64, u32)>, len: u64) -> Stamp {
        Stamp {
            modified: modified.map(|(secs, nanos)| {
                SystemTime::UNIX_EPOCH + std::time::Duration::new(secs, nanos)
            }),
            len,
        }
    }
}

/// What a hot exit backup of an unsaved document is made of
/// ([`Documents::backup_source`]).
pub(crate) struct BackupSource {
    /// The unsaved text.
    pub text: BufferSnapshot,
    /// The file's text the unsaved changes were made on.
    pub base: Option<BufferSnapshot>,
    /// The file's stamp as last loaded or saved.
    pub stamp: Option<Stamp>,
    pub encoding: &'static str,
    pub bom: bool,
    pub line_ending: LineEnding,
    /// The text holds U+FFFD for bytes that were not valid in `encoding`.
    pub lossy: bool,
}

/// The `base_hash` of a document restored from a backup whose base is not
/// the file on disk: no file's bytes hash to it, so a save first asks
/// before writing over the file (spec §5.4).
pub(crate) const RESTORED_UNKNOWN_BASE: u64 = 0;

pub(crate) type Edits = Vec<(Range<usize>, Arc<str>)>;

type WriteResult = Result<WriteOutcome, WriteError>;

/// A write started by [`Documents::start_write`].
struct StartedWrite {
    task: Task<WriteResult>,
    /// The history state the write captured; it becomes the saved one.
    state: HistoryState,
    /// The text the write captured.
    snapshot: BufferSnapshot,
    guard: SavingGuard,
}

/// Counts a write in flight in a document's `saving` for as long as it
/// lives, so a save dropped half-way cannot hold reloads back for good.
struct SavingGuard(Rc<Cell<u32>>);

impl SavingGuard {
    fn new(saving: &Rc<Cell<u32>>) -> Self {
        saving.set(saving.get() + 1);
        SavingGuard(saving.clone())
    }
}

impl Drop for SavingGuard {
    fn drop(&mut self) {
        self.0.set(self.0.get().saturating_sub(1));
    }
}

/// Where a save stands after a write ended.
enum SaveStep {
    Done(Result<bool, WriteError>),
    /// The file changed on disk under unsaved changes and was not written:
    /// the document is in conflict.
    Conflict,
}

/// A revert's read of the file: its decode, the edits from the text to it,
/// and its stamp.
type RevertOutput = (Decoded, Edits, Stamp);

/// A revert's read, or why it failed.
type RevertRead = Result<RevertOutput, LoadError>;

/// What the bar over the editor says about the file on disk (spec §7.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiskNotice {
    /// `reason` is set when the file on disk could not be read as text.
    Conflict {
        reason: Option<String>,
    },
    Deleted,
    /// The file had bytes that are not valid in its encoding, which a save
    /// replaces with U+FFFD (spec P3 §6.3). `encoding` is the
    /// `encoding_rs` name, like "UTF-8" or "Shift_JIS".
    Lossy {
        encoding: &'static str,
    },
}

/// Background work on one document. A reload diffs (or merges) the file
/// against a snapshot of the text; the buffer stays on the main thread.
enum Job {
    Load {
        force_text: bool,
    },
    Reload {
        force_text: bool,
        /// The text as it was when the job started.
        ours: BufferSnapshot,
        /// The file's text the unsaved changes were made on, to merge the
        /// disk's changes into them (spec §7.2). Set when the text is
        /// dirty.
        merge_base: Option<BufferSnapshot>,
        /// `lucerna.externalChanges.autoMerge`.
        auto_merge: bool,
        /// Whether `ours` had unsaved changes.
        dirty: bool,
        base_version: u64,
        base_hash: u64,
        base_epoch: u64,
    },
    /// Make the text a hot exit backup's, checking the file on disk is
    /// still the one it was made on.
    Restore {
        backup: Backup,
    },
}

/// What a reload does to the text.
enum ReloadOutcome {
    /// Make the text the file's: the edits from `ours` to the disk's text
    /// (none when they are already equal).
    Apply(Edits),
    /// Put the disk's changes into the unsaved text: edits on `ours`.
    Merge(Edits),
    /// The disk's changes meet the unsaved ones: leave the text alone.
    Conflict,
}

/// A file read for a reload of version `base_version` of the buffer.
struct Reloaded {
    /// The decode, its `text` moved into `theirs`.
    decoded: Decoded,
    /// The file's text.
    theirs: BufferSnapshot,
    base_version: u64,
    /// Whether the text was dirty when the job started.
    dirty: bool,
    outcome: ReloadOutcome,
    stamp: Stamp,
}

/// Why a document is in conflict with the disk.
pub(crate) struct Conflict {
    /// The file's text that could not be merged; `None` when it could not
    /// be read.
    disk: Option<DiskText>,
    /// Why the file could not be read.
    reason: Option<String>,
    /// Without `disk`: the stamp of the file that could not be read, and
    /// the hash of its bytes when they were read. Keep Mine expects them.
    unreadable: Option<(Stamp, Option<u64>)>,
}

/// A reload's read that failed: why, and what was learnt of the file
/// before it failed.
struct ReadFailure {
    err: LoadError,
    stamp: Option<Stamp>,
    /// The hash of the file's bytes, when they were read.
    hash: Option<u64>,
}

/// A file's text as read from disk, with what makes the buffer's text the
/// file's ([`Buffer::set_disk_state`]).
pub(crate) struct DiskText {
    text: BufferSnapshot,
    /// The decode without its text (which is `text`): encoding, BOM, line
    /// ending and the bytes' hash.
    decoded: Decoded,
    stamp: Stamp,
}

/// A backup made into a buffer with unsaved changes, and what is known of
/// the file it was made on.
struct Restored {
    buffer: Buffer,
    /// The file's text, when it is still the backup's base.
    base: Option<BufferSnapshot>,
    base_hash: u64,
    stamp: Option<Stamp>,
}

enum JobOutput {
    Loaded(Result<(Buffer, Stamp), LoadError>),
    /// A new Untitled document's empty buffer: no file behind it.
    Untitled(Buffer),
    Restored(Restored),
    /// A reload's read, and the disk epoch it started in.
    Reloaded(u64, Result<Reloaded, ReadFailure>),
}

struct Document {
    state: DocState,
    /// `None` until loaded.
    buffer: Option<Buffer>,
    /// The buffer's text, as the views show it.
    snapshot: Option<BufferSnapshot>,
    /// The buffer's version, as of the last change seen here.
    version: u64,
    /// The hash of the file's bytes as last loaded, reloaded or saved: a
    /// change on disk with the same hash is ignored.
    base_hash: u64,
    /// Bumped whenever the text is made the file's (a save, a reload, a
    /// revert). A reload read in an older epoch may predate a save, even
    /// one that wrote the same bytes, so its result is dropped.
    disk_epoch: u64,
    /// The file's text as last loaded, saved or reloaded: the base of a
    /// merge of the disk's changes into unsaved ones.
    base: Option<BufferSnapshot>,
    /// The file changed on disk in a way the unsaved changes could not
    /// take (or could not be read).
    conflict: Option<Conflict>,
    /// The user chose Keep on the notice of a deleted file: it is hidden
    /// until the file comes back. Cleared whenever the state leaves
    /// [`DocState::Deleted`].
    deleted_kept: bool,
    /// The user dismissed the bar about invalid bytes. Cleared whenever the
    /// file's text is taken again (load, reload, merge, revert).
    lossy_dismissed: bool,
    /// The user agreed to save over the invalid bytes. Cleared with
    /// `lossy_dismissed`.
    lossy_consent: bool,
    /// Keep Mine on a file that could not be read, when not even its stamp
    /// is known: the next write does not check the file on disk first.
    /// Consumed by that write (`start_write`); cleared whenever the file's
    /// text or stamp is taken again (`finish_reload`, `apply_merge`,
    /// `apply_disk`, Keep Mine with a known stamp).
    skip_disk_check: bool,
    /// Bumped per new text, so a parse of older text is dropped.
    revision: u64,
    encoding: &'static str,
    line_ending: LineEnding,
    language: Option<Arc<Language>>,
    /// Detected from the text on load and reload (`editor.detectIndentation`),
    /// and again when the settings it reads change.
    indentation: Option<Indentation>,
    /// The `editor.detectIndentation` and default indentation the last
    /// detection used; a settings change re-runs detection when they differ.
    indentation_inputs: Option<(bool, Indentation)>,
    syntax: Option<Arc<SyntaxState>>,
    force_text: bool,
    stamp: Option<Stamp>,
    views: Vec<WeakEntity<CodeView>>,
    /// The load or reload in flight; at most one per document.
    job: Option<Task<()>>,
    /// Another change arrived while `job` ran: reload once more after it.
    reload_again: bool,
    /// Writes in flight. A reload waits for them (`reload_after_save`):
    /// it could read the file from before the write.
    saving: Rc<Cell<u32>>,
    /// A reload was asked for, or finished, while a write was in flight:
    /// reload once the writes are done.
    reload_after_save: bool,
    /// Resolves when the last save (or revert) asked for is done; the next
    /// one waits for it, so they act on the file in the order asked for.
    last_save: Option<Shared<oneshot::Receiver<()>>>,
    parse: Option<Task<()>>,
    /// Set to stop the background parse of `parse` once newer text makes
    /// it useless (dropping the task cannot stop a running thread).
    parse_cancel: Option<Arc<AtomicBool>>,
    /// The view an edit or undo is coming from, while it runs: that view
    /// is in the middle of an update, so it takes the change itself (see
    /// [`Documents::apply_edits_from_view`]).
    skip_view: Option<EntityId>,
    /// Where to put the cursor once the text is loaded.
    pending_go_to: Option<GoTo>,
    /// [`DocumentEvent::Opened`] was sent.
    announced: bool,
    /// Made by [`Documents::open_from_backup`] for a tab whose view is not
    /// made yet: kept without views until the first one comes.
    awaiting_view: bool,
    /// Set by any view's selection change: the next edit starts a new
    /// undo step. A flag rather than a call, since views change their
    /// selections from inside `Documents` too.
    stop_requested: Rc<Cell<bool>>,
}

impl Document {
    /// A document not loaded yet, shown by `views`.
    fn new(
        language: Option<Arc<Language>>,
        views: Vec<WeakEntity<CodeView>>,
        pending_go_to: Option<GoTo>,
        stop_requested: Rc<Cell<bool>>,
    ) -> Document {
        Document {
            state: DocState::Loading,
            buffer: None,
            snapshot: None,
            version: 0,
            base_hash: 0,
            disk_epoch: 0,
            base: None,
            conflict: None,
            deleted_kept: false,
            lossy_dismissed: false,
            lossy_consent: false,
            skip_disk_check: false,
            revision: 0,
            encoding: "UTF-8",
            line_ending: LineEnding::Lf,
            language,
            indentation: None,
            indentation_inputs: None,
            syntax: None,
            force_text: false,
            stamp: None,
            views,
            job: None,
            reload_again: false,
            saving: Rc::default(),
            reload_after_save: false,
            last_save: None,
            parse: None,
            parse_cancel: None,
            skip_view: None,
            pending_go_to,
            announced: false,
            awaiting_view: false,
            stop_requested,
        }
    }

    fn views(&self) -> impl Iterator<Item = Entity<CodeView>> + '_ {
        self.views
            .iter()
            .filter(|v| Some(v.entity_id()) != self.skip_view)
            .filter_map(WeakEntity::upgrade)
    }

    /// Drops the parse in flight and stops its background work.
    fn cancel_parse(&mut self) {
        self.parse = None;
        if let Some(cancel) = self.parse_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    fn language_id(&self) -> Option<Arc<str>> {
        self.language.as_ref().map(|l| l.id.clone())
    }

    fn set_state(&mut self, state: DocState, cx: &mut App) {
        for view in self.views() {
            let state = state.clone();
            view.update(cx, |v, cx| v.set_doc_state(state, cx));
        }
        if state != DocState::Deleted {
            self.deleted_kept = false;
        }
        self.state = state;
    }
}

/// Where a jump puts the cursor: a [`Point`] (byte column), or a row and
/// a column counted in characters, as compilers and Quick Open's `:L:C`
/// give it. A character column can only become bytes once the text is
/// loaded, so it waits in that form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GoTo {
    Byte(Point),
    /// 0-based row and character column.
    Chars {
        row: u32,
        col: u32,
    },
}

impl From<Point> for GoTo {
    fn from(p: Point) -> Self {
        GoTo::Byte(p)
    }
}

impl GoTo {
    /// The point in `snapshot`.
    pub(crate) fn point_in(self, snapshot: &BufferSnapshot) -> Point {
        match self {
            GoTo::Byte(p) => p,
            GoTo::Chars { row, col } => char_point(snapshot, row, col),
        }
    }
}

/// The point at 0-based `row` and character column `char_col` of
/// `snapshot`: the row clamps to the last line and the column to the end
/// of its line.
pub(crate) fn char_point(snapshot: &BufferSnapshot, row: u32, char_col: u32) -> Point {
    let row = row.min(snapshot.line_count().saturating_sub(1));
    let text = snapshot.line(row);
    let column = text
        .char_indices()
        .nth(char_col as usize)
        .map_or(text.len(), |(byte, _)| byte) as u32;
    Point { row, column }
}

/// Every open document, keyed by normalised path. A GPUI entity owned by
/// the [`crate::Workbench`].
pub struct Documents {
    root: PathBuf,
    /// The window's settings: indentation, save and large-file settings
    /// are read from them, and each view is made with them.
    settings: Entity<WindowSettings>,
    docs: HashMap<Arc<Path>, Document>,
    /// Jumps requested for paths whose document does not exist yet.
    pending_go_to: HashMap<Arc<Path>, GoTo>,
    /// Scroll rows restored for paths whose text is not loaded yet; the
    /// first view to have the text takes its row.
    pending_scroll: HashMap<Arc<Path>, u32>,
    /// The window whose status bar shows what goes wrong here.
    window: Option<WindowId>,
}

impl EventEmitter<DocumentEvent> for Documents {}

impl Documents {
    /// Documents of the folder at `root`; relative paths are resolved
    /// against it. They read the window's `settings`.
    /// Tells the documents which window they belong to.
    pub fn set_window(&mut self, window: WindowId) {
        self.window = Some(window);
    }

    pub fn new(root: PathBuf, settings: Entity<WindowSettings>) -> Self {
        Documents {
            root,
            settings,
            docs: HashMap::new(),
            pending_go_to: HashMap::new(),
            pending_scroll: HashMap::new(),
            window: None,
        }
    }

    /// The window's settings the documents and their views read.
    pub fn settings(&self) -> &Entity<WindowSettings> {
        &self.settings
    }

    /// The key for `path`: as given when absolute, else joined onto the
    /// root.
    pub(crate) fn key(&self, path: &Path) -> Arc<Path> {
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

    /// Makes a new, empty Untitled document (`untitled:Untitled-N`, `N` the
    /// smallest number not in use) for a tab whose view is made next, and
    /// returns its key. Its text is there at once: it has no file to load.
    /// Its language is `files.defaultLanguage`'s, else plain text.
    pub fn new_untitled(&mut self, cx: &mut Context<Self>) -> Arc<Path> {
        self.collect_closed(cx);
        let n = untitled::next_number(self.docs.keys().filter_map(|k| untitled::number(k)));
        let key = untitled::path(n);
        let language = language_for(&self.root, &self.settings, &key, None, cx);
        let mut doc = Document::new(language, Vec::new(), None, Rc::default());
        doc.awaiting_view = true;
        self.docs.insert(key.clone(), doc);
        self.finish_job(&key, Ok(JobOutput::Untitled(empty_buffer())), cx);
        key
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
        let stop_requested = self
            .docs
            .get(&key)
            .map(|d| d.stop_requested.clone())
            .unwrap_or_default();
        let settings = self.settings.clone();
        let view = cx.new(|cx| {
            let mut view = CodeView::new(path, settings, cx);
            view.set_documents(documents, stop_requested.clone());
            view.set_passive(passive);
            view
        });
        match self.docs.get_mut(&key) {
            Some(doc) => {
                // The language servers feed the views they know of; a new
                // view starts with what its siblings show.
                let diagnostics = doc.views().find_map(|v| v.read(cx).diagnostics_source());
                // Likewise the git change bars.
                let hunks = doc
                    .views()
                    .map(|v| v.read(cx).shared_diff_hunks())
                    .find(|h| !h.is_empty());
                doc.views.push(view.downgrade());
                doc.awaiting_view = false;
                let state = doc.state.clone();
                let (snapshot, syntax) = (doc.snapshot.clone(), doc.syntax.clone());
                let language_id = doc.language.as_ref().map(|l| l.id.clone());
                let indentation = doc.indentation;
                let takes_pending = snapshot.is_some() && !passive;
                let go_to = takes_pending.then(|| doc.pending_go_to.take()).flatten();
                let scroll = takes_pending
                    .then(|| self.pending_scroll.remove(&key))
                    .flatten();
                view.update(cx, |v, cx| {
                    v.set_language_id(language_id, cx);
                    v.set_indentation(indentation, cx);
                    if let Some(snapshot) = snapshot {
                        v.set_buffer(snapshot, cx);
                        v.set_syntax(syntax, cx);
                    }
                    if let Some(diagnostics) = diagnostics {
                        v.set_diagnostics(diagnostics, cx);
                    }
                    if let Some(hunks) = hunks {
                        v.set_diff_hunks(hunks, cx);
                    }
                    // A jump centers its row; a scroll row asked for with it
                    // (Go Back) wins over that.
                    if let Some(p) = go_to {
                        let p = p.point_in(v.snapshot());
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
                let language = language_for(&self.root, &self.settings, &key, None, cx);
                let language_id = language.as_ref().map(|l| l.id.clone());
                view.update(cx, |v, cx| {
                    v.set_language_id(language_id, cx);
                    v.set_doc_state(DocState::Loading, cx);
                });
                let pending_go_to = self.pending_go_to.remove(&key);
                self.docs.insert(
                    key.clone(),
                    Document::new(
                        language,
                        vec![view.downgrade()],
                        pending_go_to,
                        stop_requested,
                    ),
                );
                if untitled::is_untitled(&key) {
                    // Never read from disk: an Untitled not known yet (a
                    // restored tab) starts empty.
                    self.finish_job(&key, Ok(JobOutput::Untitled(empty_buffer())), cx);
                } else {
                    self.start_job(&key, cx);
                }
            }
        }
        cx.notify();
        view
    }

    /// After a settings change: detects the indentation of every loaded
    /// document again with the new defaults, as VS Code's
    /// `ModelService._updateModelOptions` does, when
    /// `editor.detectIndentation`, `editor.tabSize` or `editor.insertSpaces`
    /// changed for its language.
    pub fn settings_changed(&mut self, cx: &mut Context<Self>) {
        for doc in self.docs.values_mut() {
            if doc.indentation_inputs.is_some()
                && doc.indentation_inputs
                    != Some(indentation_inputs(doc, self.settings.read(cx).view(cx)))
            {
                detect_indentation(doc, &self.settings, cx);
            }
        }
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
            doc.language =
                language_for(&self.root, &self.settings, &key, doc.snapshot.as_ref(), cx);
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
                doc.cancel_parse();
                clear_syntax(doc, cx);
            } else {
                // In the background: a swap must not parse every open file
                // on the UI thread. The old syntax stays shown meanwhile.
                self.parse(&key, false, false, cx);
            }
        }
        cx.notify();
    }

    /// After the file or folder `from` was renamed to `to` on disk: every
    /// document at `from` or under it takes its new path. Its text, unsaved
    /// changes, undo history and views (with their cursors) stay; the views
    /// show the new path, and the language is looked up again by the new
    /// name. An opened document is sent as [`DocumentEvent::Closed`] at the
    /// old path, then [`DocumentEvent::Opened`] at the new one, so language
    /// servers move it.
    ///
    /// A load or reload in flight read (or reads) the old path: it is
    /// dropped and started again at the new one, which also brings back a
    /// document the watcher already marked deleted. A parse in flight is
    /// restarted, since it lands by key. A save in flight still writes the
    /// old path (it captured it when it started).
    pub fn rename(&mut self, from: &Path, to: &Path, cx: &mut Context<Self>) {
        self.collect_closed(cx);
        let (from, to) = (self.key(from), self.key(to));
        let moved: Vec<(Arc<Path>, Arc<Path>)> = self
            .docs
            .keys()
            .filter_map(|key| {
                let new = crate::workspace_state::moved_path(key, &from, &to)?;
                Some((key.clone(), Arc::from(new)))
            })
            .collect();
        rekey(&mut self.pending_scroll, &from, &to);
        rekey(&mut self.pending_go_to, &from, &to);
        for (old, new) in moved {
            let Some(mut doc) = self.docs.remove(&old) else {
                continue;
            };
            for view in doc.views().collect::<Vec<_>>() {
                let new = new.clone();
                view.update(cx, |v, cx| v.set_path(new, cx));
            }
            let old_id = doc.language_id();
            doc.language =
                language_for(&self.root, &self.settings, &new, doc.snapshot.as_ref(), cx);
            sync_language_id(&doc, cx);
            if doc.indentation_inputs.is_some()
                && doc.indentation_inputs
                    != Some(indentation_inputs(&doc, self.settings.read(cx).view(cx)))
            {
                detect_indentation(&mut doc, &self.settings, cx);
            }
            let language_changed = doc.language_id() != old_id;
            // Lands by key: started again below at the new one.
            let restart_job = doc.job.take().is_some()
                || matches!(
                    doc.state,
                    DocState::Deleted | DocState::Error(LoadError::NotFound)
                );
            let reparse = language_changed || doc.parse.is_some();
            // Only an open tab of a file that is gone can hold the name
            // (the rename never replaces a file, and the Explorer refuses
            // it): it goes, before the moved document opens there.
            if let Some(stale) = self.docs.remove(&new)
                && stale.announced
            {
                cx.emit(DocumentEvent::Closed { path: new.clone() });
            }
            if doc.announced {
                cx.emit(DocumentEvent::Closed { path: old.clone() });
                if let Some(snapshot) = doc.snapshot.clone() {
                    cx.emit(DocumentEvent::Opened {
                        path: new.clone(),
                        language_id: doc.language_id(),
                        snapshot,
                    });
                } else {
                    doc.announced = false;
                }
            }
            self.docs.insert(new.clone(), doc);
            if restart_job {
                self.start_job(&new, cx);
            }
            if reparse {
                let doc = self.docs.get_mut(&new).expect("just inserted");
                if doc.language.is_none() {
                    doc.cancel_parse();
                    clear_syntax(doc, cx);
                } else {
                    self.parse(&new, false, false, cx);
                }
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

    /// Whether the document at `path` is open with its text loaded, so its
    /// reloads tell real disk changes from our own saves
    /// ([`DocumentEvent::ChangedOnDisk`]). Never for an untitled document:
    /// it has no file.
    pub fn follows_disk(&self, path: &Path) -> bool {
        !crate::untitled::is_untitled(path)
            && self
                .docs
                .get(&self.key(path))
                .is_some_and(|doc| doc.buffer.is_some())
    }

    /// The state of the document at `path`, if it is open.
    pub fn state(&self, path: &Path) -> Option<&DocState> {
        self.docs.get(&self.key(path)).map(|d| &d.state)
    }

    /// Applies `edits` (sorted, non-overlapping, against the current text)
    /// to the document at `path` as part of the current undo step, and
    /// updates every view. `selections_before` is kept if this starts a new
    /// step, `selections_after` replaces the step's. `None` when the text
    /// is not loaded.
    pub fn apply_edits(
        &mut self,
        path: &Path,
        edits: Edits,
        selections_before: Option<SelectionsToken>,
        selections_after: Option<SelectionsToken>,
        origin: EntityId,
        cx: &mut Context<Self>,
    ) -> Option<Patch> {
        let key = self.key(path);
        let doc = self.docs.get_mut(&key)?;
        let stop = doc.stop_requested.take();
        let buffer = doc.buffer.as_mut()?;
        if stop {
            buffer.push_stop();
        }
        let old = buffer.snapshot();
        let was_dirty = text_dirty(&key, buffer);
        // An edit from a stale range must not split a char or run past
        // the end: the rope would panic.
        let edits = edits.into_iter().map(|(r, text)| {
            let end = old.floor_char_boundary(r.end);
            (old.floor_char_boundary(r.start).min(end)..end, text)
        });
        buffer.start_transaction(selections_before);
        let patch = buffer.edit(edits);
        buffer.end_transaction(selections_after);
        self.commit_edit(&key, old, &patch, was_dirty, Some(origin), cx);
        cx.notify();
        Some(patch)
    }

    /// [`Documents::apply_edits`] for an edit made by `view` while it is
    /// being updated (typing, an edit command): every other view is
    /// updated here, and `view` gets what it needs to update itself.
    pub(crate) fn apply_edits_from_view(
        &mut self,
        path: &Path,
        edits: Edits,
        selections_before: Option<SelectionsToken>,
        selections_after: Option<SelectionsToken>,
        view: EntityId,
        cx: &mut Context<Self>,
    ) -> Option<ViewUpdate> {
        let key = self.key(path);
        let old = self.docs.get(&key)?.buffer.as_ref()?.snapshot();
        self.docs.get_mut(&key)?.skip_view = Some(view);
        let patch = self.apply_edits(path, edits, selections_before, selections_after, view, cx);
        let doc = self.docs.get_mut(&key)?;
        doc.skip_view = None;
        Some(ViewUpdate {
            old,
            snapshot: doc.snapshot.clone()?,
            patch: patch?,
            syntax: doc.syntax.clone(),
        })
    }

    /// [`Documents::undo`] (or, without `undo`, [`Documents::redo`]) asked
    /// for by `view` while it is being updated, as in
    /// [`Documents::apply_edits_from_view`]. `None` when there is nothing
    /// to undo; else the change for `view` and the step's selections.
    pub(crate) fn undo_from_view(
        &mut self,
        path: &Path,
        undo: bool,
        view: EntityId,
        cx: &mut Context<Self>,
    ) -> Option<(ViewUpdate, Option<SelectionsToken>)> {
        let key = self.key(path);
        let old = self.docs.get(&key)?.buffer.as_ref()?.snapshot();
        self.docs.get_mut(&key)?.skip_view = Some(view);
        let result = self.undo_redo_patch(path, undo, cx);
        let doc = self.docs.get_mut(&key)?;
        doc.skip_view = None;
        let (patch, selections) = result?;
        let update = ViewUpdate {
            old,
            snapshot: doc.snapshot.clone()?,
            patch,
            syntax: doc.syntax.clone(),
        };
        Some((update, selections))
    }

    /// Ends the current undo step of the document at `path`: the next
    /// edit starts a new one.
    pub fn push_stop(&mut self, path: &Path) {
        let key = self.key(path);
        if let Some(buffer) = self.docs.get_mut(&key).and_then(|d| d.buffer.as_mut()) {
            buffer.push_stop();
        }
    }

    /// Undoes the last step of the document at `path`. `None` when there
    /// is nothing to undo; else the selections from before that step.
    pub fn undo(&mut self, path: &Path, cx: &mut Context<Self>) -> Option<Option<SelectionsToken>> {
        self.undo_redo(path, true, cx)
    }

    /// Redoes the last undone step of the document at `path`. `None` when
    /// there is nothing to redo; else the selections from after that step.
    pub fn redo(&mut self, path: &Path, cx: &mut Context<Self>) -> Option<Option<SelectionsToken>> {
        self.undo_redo(path, false, cx)
    }

    fn undo_redo(
        &mut self,
        path: &Path,
        undo: bool,
        cx: &mut Context<Self>,
    ) -> Option<Option<SelectionsToken>> {
        self.undo_redo_patch(path, undo, cx)
            .map(|(_, selections)| selections)
    }

    fn undo_redo_patch(
        &mut self,
        path: &Path,
        undo: bool,
        cx: &mut Context<Self>,
    ) -> Option<(Patch, Option<SelectionsToken>)> {
        let key = self.key(path);
        let buffer = self.docs.get_mut(&key)?.buffer.as_mut()?;
        let old = buffer.snapshot();
        let was_dirty = text_dirty(&key, buffer);
        let result = if undo { buffer.undo() } else { buffer.redo() }?;
        self.commit_edit(&key, old, &result.patch, was_dirty, None, cx);
        cx.notify();
        Some((result.patch, result.selections))
    }

    /// Whether the text of the document at `path` differs from the file
    /// as last loaded or saved.
    /// An Untitled document is dirty while it holds any text.
    pub fn is_dirty(&self, path: &Path) -> bool {
        let key = self.key(path);
        self.docs
            .get(&key)
            .and_then(|d| d.buffer.as_ref())
            .is_some_and(|b| text_dirty(&key, b))
    }

    /// What a backup of the document at `path` holds: `None` unless its
    /// text is loaded and has unsaved changes.
    pub(crate) fn backup_source(&self, path: &Path) -> Option<BackupSource> {
        let key = self.key(path);
        let doc = self.docs.get(&key)?;
        let buffer = doc.buffer.as_ref().filter(|b| text_dirty(&key, b))?;
        Some(BackupSource {
            text: buffer.snapshot(),
            base: doc.base.clone(),
            stamp: doc.stamp,
            encoding: buffer.encoding().name(),
            bom: buffer.has_bom(),
            line_ending: buffer.line_ending(),
            lossy: buffer.is_lossy(),
        })
    }

    /// Whether a document is open at `path` or under it (a tab of a file
    /// deleted on disk counts).
    pub(crate) fn has_document_under(&self, path: &Path) -> bool {
        let path = self.key(path);
        self.docs.keys().any(|k| k.starts_with(&path))
    }

    /// Resolves once every save (or revert) asked for so far of the
    /// documents at `path` or under it has ended, so a rename does not race
    /// a write that would bring the old file back.
    pub(crate) fn saves_done_under(&self, path: &Path) -> impl Future<Output = ()> + use<> {
        let path = self.key(path);
        self.saves_done_where(&|k| k.starts_with(&path))
    }

    /// Resolves once every save (or revert) asked for so far of any
    /// document has ended, so a quit does not cut a write short.
    pub(crate) fn saves_done(&self) -> impl Future<Output = ()> + use<> {
        self.saves_done_where(&|_| true)
    }

    fn saves_done_where(
        &self,
        include: &dyn Fn(&Path) -> bool,
    ) -> impl Future<Output = ()> + use<> {
        let pending: Vec<Shared<oneshot::Receiver<()>>> = self
            .docs
            .iter()
            .filter(|(k, _)| include(k))
            .filter_map(|(_, d)| d.last_save.clone())
            .filter(|p| p.clone().now_or_never().is_none())
            .collect();
        async move {
            for save in pending {
                save.await.ok();
            }
        }
    }

    /// The id of the language of the document at `path`.
    pub(crate) fn language_id_of(&self, path: &Path) -> Option<Arc<str>> {
        self.docs
            .get(&self.key(path))
            .and_then(Document::language_id)
    }

    /// Tells listeners that a view of the document at `path` lost focus.
    pub(crate) fn view_focus_lost(&mut self, path: &Path, cx: &mut Context<Self>) {
        let key = self.key(path);
        if self.docs.contains_key(&key) {
            cx.emit(DocumentEvent::FocusLost { path: key });
        }
    }

    /// Whether the file at `path` changed on disk while its text had
    /// unsaved changes.
    pub fn has_conflict(&self, path: &Path) -> bool {
        self.docs
            .get(&self.key(path))
            .is_some_and(|d| d.conflict.is_some())
    }

    /// `Deleted` when the file is gone, the text is dirty and Keep was not
    /// chosen; `None` once Keep was chosen (a conflict is moot until the
    /// file returns); else `Conflict` while in conflict; else `Lossy` while
    /// the text has invalid bytes and the bar was not dismissed; else
    /// `None`.
    pub fn disk_notice(&self, path: &Path) -> Option<DiskNotice> {
        let doc = self.docs.get(&self.key(path))?;
        let dirty = doc.buffer.as_ref().is_some_and(Buffer::is_dirty);
        if doc.state == DocState::Deleted {
            if doc.deleted_kept {
                return None;
            }
            if dirty {
                return Some(DiskNotice::Deleted);
            }
        }
        if let Some(conflict) = doc.conflict.as_ref() {
            return Some(DiskNotice::Conflict {
                reason: conflict.reason.clone(),
            });
        }
        let buffer = doc.buffer.as_ref()?;
        (buffer.is_lossy() && !doc.lossy_dismissed).then(|| DiskNotice::Lossy {
            encoding: buffer.encoding().name(),
        })
    }

    /// Dismiss on the bar about invalid bytes: hides it until the file's
    /// text is taken again and is still lossy.
    pub fn dismiss_lossy(&mut self, path: &Path, cx: &mut Context<Self>) {
        let key = self.key(path);
        if let Some(doc) = self.docs.get_mut(&key) {
            doc.lossy_dismissed = true;
            cx.notify();
        }
    }

    /// Keep Mine (spec §7.3): ends the conflict; the disk text becomes the
    /// merge base and its stamp and hash the expected ones, so the next save
    /// writes without asking. Without disk text (unreadable file), the stamp
    /// (and hash, if the bytes were read) of the failed read are the
    /// expected ones; when even the stamp is unknown, the next write skips
    /// the disk check instead.
    ///
    /// The text keeps its own encoding and line ending, and no undo state
    /// counts as the file's any more (as after a merge): the text stays
    /// dirty until saved.
    pub fn keep_mine(&mut self, path: &Path, cx: &mut Context<Self>) {
        let key = self.key(path);
        let Some(doc) = self.docs.get_mut(&key) else {
            return;
        };
        let Some(conflict) = doc.conflict.as_ref() else {
            return;
        };
        let disk = conflict
            .disk
            .as_ref()
            .map(|d| (d.text.clone(), d.decoded.hash, d.stamp));
        let unreadable = conflict.unreadable;
        match (disk, unreadable) {
            (Some((text, hash, stamp)), _) => {
                doc.base = Some(text);
                doc.base_hash = hash;
                doc.stamp = Some(stamp);
                doc.skip_disk_check = false;
            }
            (None, Some((stamp, hash))) => {
                // The text stays the merge base: there is no other.
                if let Some(hash) = hash {
                    doc.base_hash = hash;
                }
                doc.stamp = Some(stamp);
                doc.skip_disk_check = false;
            }
            (None, None) => doc.skip_disk_check = true,
        }
        // A reload read against the old base runs again against this one.
        doc.disk_epoch += 1;
        if let Some(buffer) = doc.buffer.as_mut() {
            let was_dirty = buffer.is_dirty();
            buffer.clear_saved();
            if !was_dirty {
                cx.emit(DocumentEvent::DirtyChanged {
                    path: key.clone(),
                    dirty: true,
                });
            }
        }
        set_conflict(doc, &key, None, cx);
        cx.notify();
    }

    /// Keep on a deleted file: hides the notice; the tab stays struck
    /// through and a save makes the file again.
    pub fn keep_deleted(&mut self, path: &Path, cx: &mut Context<Self>) {
        let key = self.key(path);
        if let Some(doc) = self.docs.get_mut(&key)
            && doc.state == DocState::Deleted
        {
            doc.deleted_kept = true;
            cx.notify();
        }
    }

    /// The keys of the documents whose text has unsaved changes.
    pub fn dirty_paths(&self) -> Vec<Arc<Path>> {
        let mut paths: Vec<Arc<Path>> = self
            .docs
            .iter()
            .filter(|(key, d)| d.buffer.as_ref().is_some_and(|b| text_dirty(key, b)))
            .map(|(key, _)| key.clone())
            .collect();
        paths.sort();
        paths
    }

    /// Saves the document at `path` (spec §6.1). `Ok(false)` when there was
    /// nothing to save (no text loaded), the user answered the conflict
    /// prompt with Reload or Cancel, or the file changed on disk under a
    /// clean text (which is then reloaded); a failed write is also reported
    /// in the status bar, and the text stays dirty.
    ///
    /// 0. When the file had bytes invalid in its encoding and the user has
    ///    not agreed yet ([`Documents::needs_lossy_consent`]), an autosave
    ///    does nothing (`Ok(false)`) and any other save asks first; Cancel
    ///    is `Ok(false)`, Save is remembered for the document.
    /// 1. When the document is in conflict with the disk and
    ///    `files.saveConflictResolution` is `askUser`, asks first, unless
    ///    `opts.overwrite` says to write over the file (`:w!`).
    /// 2. With `opts.participants`, runs the save participants as one undo
    ///    step.
    /// 3. Captures the text and its history state and writes it on a
    ///    background thread; edits made meanwhile stay unsaved. Right
    ///    before writing, the file is checked against the one last loaded
    ///    or saved (like VS Code's etag): a file changed by another program
    ///    is not written over, the document goes into conflict and step 1
    ///    asks.
    /// 4. Marks the captured state saved, takes the written bytes' hash
    ///    (so the watcher's report of the write is ignored) and sends
    ///    [`DocumentEvent::Saved`].
    ///
    /// Saves and reverts of one document run one at a time, in the order
    /// asked for; a save that waited looks at the conflict when its turn
    /// comes.
    pub fn save(
        &mut self,
        path: &Path,
        opts: SaveOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<Result<bool, WriteError>> {
        let key = self.key(path);
        // No file to write to: Save As gives it one.
        if untitled::is_untitled(&key) {
            return Task::ready(Ok(false));
        }
        let lossy = self.needs_lossy_consent(&key);
        // An autosave never asks: it leaves invalid bytes alone.
        if lossy && opts.reason == SaveReason::Auto {
            return Task::ready(Ok(false));
        }
        let Some(doc) = self.docs.get_mut(&key) else {
            return Task::ready(Ok(false));
        };
        let (done, previous) = queue_disk_work(doc);
        // With nothing to wait for or ask, the text is captured now: an
        // edit made after this call is not part of the save.
        // `opts.overwrite` (`:w!`) neither asks nor checks the disk.
        let started = (previous.is_none() && !lossy && (opts.overwrite || !self.asks(&key, cx)))
            .then(|| self.start_write(&key, opts, opts.overwrite, cx));
        cx.spawn_in(window, async move |this, cx| {
            // Dropped at the end: the next save of the document goes on.
            let _done = done;
            let mut started = started;
            if started.is_none()
                && let Some(previous) = previous
            {
                previous.await.ok();
            }
            // The user chose Overwrite (or `:w!`): write without asking or
            // checking.
            let mut overwrite = opts.overwrite;
            loop {
                let write = match started.take() {
                    Some(write) => write,
                    None => {
                        // Asked once per document, before the conflict.
                        let Ok(lossy) = this.update(cx, |d, _| d.needs_lossy_consent(&key)) else {
                            return Ok(false);
                        };
                        if lossy {
                            // An autosave never asks: it leaves invalid
                            // bytes alone.
                            if opts.reason == SaveReason::Auto {
                                return Ok(false);
                            }
                            let Ok(prompt) = this.update_in(cx, |d, window, cx| {
                                let encoding = d
                                    .docs
                                    .get(&key)
                                    .and_then(|doc| doc.buffer.as_ref())
                                    .map_or("", |b| b.encoding().name());
                                save::prompt_lossy(&save::display_name(&key), encoding, window, cx)
                            }) else {
                                return Ok(false);
                            };
                            if prompt.await != Ok(save::LOSSY_SAVE) {
                                return Ok(false);
                            }
                            let agreed = this.update(cx, |d, _| {
                                if let Some(doc) = d.docs.get_mut(&key) {
                                    doc.lossy_consent = true;
                                }
                            });
                            if agreed.is_err() {
                                return Ok(false);
                            }
                        }
                        let Ok(ask) = this.update(cx, |d, cx| d.asks(&key, cx)) else {
                            return Ok(false);
                        };
                        // An autosave never asks: it leaves a conflict alone.
                        if opts.reason == SaveReason::Auto
                            && this.update(cx, |d, _| d.has_conflict(&key)).unwrap_or(true)
                        {
                            return Ok(false);
                        }
                        if ask && !overwrite {
                            let Ok(prompt) = this.update_in(cx, |_, window, cx| {
                                window.prompt(
                                    PromptLevel::Warning,
                                    CONFLICT_MESSAGE,
                                    None,
                                    &CONFLICT_ANSWERS[..],
                                    cx,
                                )
                            }) else {
                                return Ok(false);
                            };
                            match prompt.await {
                                Ok(0) => overwrite = true,
                                Ok(1) => {
                                    if let Ok(revert) =
                                        this.update(cx, |d, cx| d.revert_now(&key, cx))
                                    {
                                        revert.await;
                                    }
                                    return Ok(false);
                                }
                                _ => return Ok(false),
                            }
                        }
                        let Ok(write) =
                            this.update(cx, |d, cx| d.start_write(&key, opts, overwrite, cx))
                        else {
                            return Ok(false);
                        };
                        write
                    }
                };
                let Some(StartedWrite {
                    task,
                    state,
                    snapshot,
                    guard,
                }) = write
                else {
                    return Ok(false);
                };
                let result = task.await;
                let finish = |d: &mut Self, cx: &mut Context<Self>| {
                    d.finish_save(&key, state, snapshot, guard, result, cx)
                };
                match this.update(cx, finish) {
                    Ok(SaveStep::Done(result)) => return result,
                    // An autosave leaves the conflict alone.
                    Ok(SaveStep::Conflict) if opts.reason == SaveReason::Auto => {
                        return Ok(false);
                    }
                    // The document is in conflict now: ask.
                    Ok(SaveStep::Conflict) => {}
                    Err(_) => return Ok(false),
                }
            }
        })
    }

    /// Whether `files.saveConflictResolution` for the document at `key`
    /// says to write over a changed file without asking.
    fn overwrites(&self, key: &Arc<Path>, cx: &App) -> bool {
        let language = self.docs.get(key).and_then(Document::language_id);
        self.settings
            .read(cx)
            .get::<String>("files.saveConflictResolution", language.as_deref(), cx)
            .as_deref()
            == Some("overwriteFileOnDisk")
    }

    /// Whether the text of the document at `path` has invalid bytes from
    /// its file that the user has not yet agreed to replace with U+FFFD
    /// (spec §6.4): a save asks first, and an autosave skips it.
    pub(crate) fn needs_lossy_consent(&self, path: &Path) -> bool {
        self.docs
            .get(&self.key(path))
            .is_some_and(|d| !d.lossy_consent && d.buffer.as_ref().is_some_and(Buffer::is_lossy))
    }

    /// Whether a save of the document at `key` asks about a conflict first.
    fn asks(&self, key: &Arc<Path>, cx: &App) -> bool {
        self.docs.get(key).is_some_and(|d| d.conflict.is_some()) && !self.overwrites(key, cx)
    }

    /// Runs the save participants, captures the text and starts writing it;
    /// with `overwrite`, without checking the file on disk first. `None`
    /// when the document has no text to save.
    fn start_write(
        &mut self,
        key: &Arc<Path>,
        opts: SaveOptions,
        overwrite: bool,
        cx: &mut Context<Self>,
    ) -> Option<StartedWrite> {
        let check = !overwrite && !self.overwrites(key, cx);
        let window = self.window;
        let doc = self.docs.get_mut(key)?;
        let check = !std::mem::take(&mut doc.skip_disk_check) && check;
        // Only a file that never loaded has no text to save (a failed
        // reload keeps the text, see `reload_failed`).
        let Some(buffer) = doc
            .buffer
            .as_mut()
            .filter(|_| matches!(doc.state, DocState::Ready | DocState::Deleted))
        else {
            let reason = WriteError::Io("the file's text is not loaded".into());
            status_message::show_or_log(window, save::failure_message(key, &reason), cx);
            return None;
        };
        buffer.push_stop();
        if opts.participants {
            let language = doc.language_id();
            let participants = save::participants(
                self.settings.read(cx).view(cx),
                language.as_deref(),
                opts.reason,
            );
            // An autosave keeps the whitespace the user just typed: the
            // lines with a cursor are not trimmed (as VS Code does).
            let cursor_rows = if opts.reason == SaveReason::Auto {
                self.cursor_rows(key, cx)
            } else {
                Vec::new()
            };
            // One undo step: no stop between them.
            for participant in participants {
                let buffer = self.docs.get_mut(key)?.buffer.as_mut()?;
                let old = buffer.snapshot();
                let edits = participant(&old, &cursor_rows);
                if edits.is_empty() {
                    continue;
                }
                let was_dirty = buffer.is_dirty();
                let patch = buffer.edit(edits);
                self.commit_edit(key, old, &patch, was_dirty, None, cx);
            }
        }
        let doc = self.docs.get_mut(key)?;
        let buffer = doc.buffer.as_mut()?;
        let (snapshot, state) = buffer.begin_save();
        let job = WriteJob {
            snapshot: snapshot.clone(),
            encoding: buffer.encoding(),
            has_bom: buffer.has_bom(),
            line_ending: buffer.line_ending(),
            // While a reload is in flight the file may be newer than the
            // text's base: hash it rather than trust an equal stamp.
            expected: check.then_some((doc.stamp.filter(|_| doc.job.is_none()), doc.base_hash)),
            skip_if_missing: opts.reason == SaveReason::Auto,
            temps: save::save_temps(cx),
        };
        let guard = SavingGuard::new(&doc.saving);
        let path = key.clone();
        let hold = save::save_hold(cx);
        let task = cx.background_spawn(async move {
            if let Some(hold) = hold {
                hold.await.ok();
            }
            catch_panic("save", || job.run(&path))
                .unwrap_or_else(|message| Err(WriteError::Io(format!("internal error: {message}"))))
        });
        cx.notify();
        Some(StartedWrite {
            task,
            state,
            snapshot,
            guard,
        })
    }

    /// The sorted rows holding a cursor of any view of the document at
    /// `key`.
    fn cursor_rows(&self, key: &Arc<Path>, cx: &App) -> Vec<u32> {
        let Some(doc) = self.docs.get(key) else {
            return Vec::new();
        };
        let Some(buffer) = doc.buffer.as_ref() else {
            return Vec::new();
        };
        let text = buffer.snapshot();
        let mut rows: Vec<u32> = doc
            .views
            .iter()
            .filter_map(WeakEntity::upgrade)
            .flat_map(|v| {
                let heads: Vec<usize> = v
                    .read(cx)
                    .selections()
                    .all()
                    .iter()
                    .map(|s| s.head)
                    .collect();
                heads
            })
            .map(|head| text.offset_to_point(head.min(text.len_bytes())).row)
            .collect();
        rows.sort_unstable();
        rows.dedup();
        rows
    }

    /// Takes the end of a write started by [`Documents::start_write`] for
    /// the history `state` it captured.
    fn finish_save(
        &mut self,
        key: &Arc<Path>,
        state: HistoryState,
        snapshot: BufferSnapshot,
        guard: SavingGuard,
        result: WriteResult,
        cx: &mut Context<Self>,
    ) -> SaveStep {
        drop(guard);
        let Some(doc) = self.docs.get_mut(key) else {
            return SaveStep::Done(Ok(false));
        };
        let step = match result {
            Ok(WriteOutcome::Written(written)) => {
                if let Some(buffer) = doc.buffer.as_mut() {
                    let was_dirty = buffer.is_dirty();
                    // Edits made while writing stay unsaved.
                    buffer.set_saved(state);
                    let dirty = buffer.is_dirty();
                    doc.base_hash = written.hash;
                    doc.base = Some(snapshot.clone());
                    doc.disk_epoch += 1;
                    doc.stamp = Some(Stamp::of_written(&written));
                    // The invalid bytes are now U+FFFD on disk too.
                    buffer.clear_lossy();
                    set_conflict(doc, key, None, cx);
                    if doc.state == DocState::Deleted {
                        // The save made the file again.
                        doc.set_state(DocState::Ready, cx);
                        announce_return(doc, key, cx);
                    }
                    cx.emit(DocumentEvent::Saved {
                        path: key.clone(),
                        snapshot,
                    });
                    if dirty != was_dirty {
                        cx.emit(DocumentEvent::DirtyChanged {
                            path: key.clone(),
                            dirty,
                        });
                    }
                }
                SaveStep::Done(Ok(true))
            }
            Ok(WriteOutcome::Missing) => {
                // Deleted or moved before the watcher said so: the tab
                // waits for that event.
                tracing::debug!(path = %key.display(), "file is gone; autosave skipped");
                SaveStep::Done(Ok(false))
            }
            Ok(WriteOutcome::ChangedOnDisk) => {
                tracing::debug!(path = %key.display(), "file changed on disk; not saved");
                // Read the file once the writes are done: the watcher may
                // never report the change, and the conflict needs the
                // disk's text (or a merge ends it).
                doc.reload_after_save = true;
                if doc.buffer.as_ref().is_some_and(Buffer::is_dirty) {
                    if doc.conflict.is_none() {
                        let conflict = Conflict {
                            disk: None,
                            reason: None,
                            unreadable: None,
                        };
                        set_conflict(doc, key, Some(conflict), cx);
                    }
                    SaveStep::Conflict
                } else {
                    // Nothing unsaved to protect: the reload takes the
                    // file's text.
                    SaveStep::Done(Ok(false))
                }
            }
            Err(err) => {
                tracing::warn!(path = %key.display(), %err, "save failed");
                status_message::show_or_log(self.window, save::failure_message(key, &err), cx);
                SaveStep::Done(Err(err))
            }
        };
        if doc.saving.get() == 0 && std::mem::take(&mut doc.reload_after_save) {
            self.start_job(key, cx);
        }
        cx.notify();
        step
    }

    /// Replaces the text of the document at `path` with the file's, as one
    /// undo step, and ends any conflict (`workbench.action.files.revert`,
    /// and Reload in the conflict prompt). The text counts as saved.
    /// It waits for the document's saves in flight, and its saves wait for
    /// it.
    pub fn revert(&mut self, path: &Path, cx: &mut Context<Self>) -> Task<()> {
        let key = self.key(path);
        // No file to go back to.
        if untitled::is_untitled(&key) {
            return Task::ready(());
        }
        let Some(doc) = self.docs.get_mut(&key) else {
            return Task::ready(());
        };
        let (done, previous) = queue_disk_work(doc);
        cx.spawn(async move |this, cx| {
            let _done = done;
            if let Some(previous) = previous {
                previous.await.ok();
            }
            if let Ok(revert) = this.update(cx, |d, cx| d.revert_now(&key, cx)) {
                revert.await;
            }
        })
    }

    /// [`Documents::revert`] without waiting for saves: the caller holds
    /// the document's place in line.
    fn revert_now(&mut self, key: &Arc<Path>, cx: &mut Context<Self>) -> Task<()> {
        if untitled::is_untitled(key) {
            return Task::ready(());
        }
        let key = key.clone();
        cx.spawn(async move |this, cx| {
            loop {
                let Ok(Some((work, base))) = this.update(cx, |d, cx| d.start_revert(&key, cx))
                else {
                    return;
                };
                let output = work.await;
                let done = this
                    .update(cx, |d, cx| d.finish_revert(&key, base, output, cx))
                    .unwrap_or(true);
                if done {
                    return;
                }
            }
        })
    }

    /// Reads and diffs the file of `key` against its text in the
    /// background; also returns the buffer version and disk epoch it
    /// started from.
    fn start_revert(
        &mut self,
        key: &Arc<Path>,
        cx: &mut Context<Self>,
    ) -> Option<(Task<RevertRead>, (u64, u64))> {
        let doc = self.docs.get(key)?;
        let buffer = doc.buffer.as_ref()?;
        let (base, force_text) = (buffer.snapshot(), doc.force_text);
        let version = (buffer.version(), doc.disk_epoch);
        let path = key.clone();
        let work = spawn_contained(cx, move || {
            let (bytes, stamp) = read(&path)?;
            let decoded = decode_file(&bytes, force_text)?;
            let edits = diff_edits(&base, &decoded.text);
            Ok((decoded, edits, stamp))
        });
        let work = cx.background_spawn(async move { work.await.and_then(|r| r) });
        Some((work, version))
    }

    /// Applies a revert's read of the file. `false` when the text, or the
    /// file it was last made (a reload), changed since it was diffed: the
    /// revert runs again.
    fn finish_revert(
        &mut self,
        key: &Arc<Path>,
        (version, epoch): (u64, u64),
        output: RevertRead,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(doc) = self.docs.get_mut(key) else {
            return true;
        };
        let Some(buffer) = doc.buffer.as_ref() else {
            return true;
        };
        if buffer.version() != version || doc.disk_epoch != epoch {
            return false;
        }
        match output {
            Ok((decoded, edits, stamp)) => {
                tracing::debug!(path = %key.display(), "document reverted");
                self.apply_disk(key, &decoded, edits, stamp, cx);
            }
            Err(err) => {
                tracing::warn!(path = %key.display(), %err, "revert failed");
                let name = key.file_name().map_or_else(
                    || key.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                );
                status_message::show_or_log(
                    self.window,
                    format!("Failed to revert '{name}': {err}"),
                    cx,
                );
            }
        }
        cx.notify();
        true
    }

    /// After the buffer of `key` changed from `old` by `patch`: shows the
    /// new text in every view, edits the syntax to it and parses it, and
    /// tells listeners. `was_dirty` is the dirty state before the change.
    fn commit_edit(
        &mut self,
        key: &Arc<Path>,
        old: BufferSnapshot,
        patch: &Patch,
        was_dirty: bool,
        origin: Option<EntityId>,
        cx: &mut Context<Self>,
    ) {
        let Some(doc) = self.docs.get_mut(key) else {
            return;
        };
        let Some(buffer) = doc.buffer.as_ref() else {
            return;
        };
        doc.version = buffer.version();
        let dirty = text_dirty(key, buffer);
        let can_redo = buffer.can_redo();
        if !patch.is_empty() {
            let snapshot = buffer.snapshot();
            doc.snapshot = Some(snapshot.clone());
            doc.revision += 1;
            // The old tree, edited to the new text, keeps the colours
            // until the new parse lands; views edit their own copy.
            doc.syntax = doc.syntax.take().and_then(|syntax| {
                catch_panic("edit syntax", || syntax.edited(&old, &snapshot, patch))
                    .ok()
                    .map(Arc::new)
            });
            for view in doc.views() {
                let snapshot = snapshot.clone();
                view.update(cx, |v, cx| v.apply_edit(&old, snapshot, patch, cx));
            }
            cx.emit(DocumentEvent::Edited {
                path: key.clone(),
                old,
                snapshot,
                patch: Arc::new(patch.clone()),
                origin,
            });
        }
        if dirty != was_dirty {
            cx.emit(DocumentEvent::DirtyChanged {
                path: key.clone(),
                dirty,
            });
        }
        // Back to the saved text while the disk has newer text, with no
        // undone step left to redo: nothing of the user's is left to
        // protect, so the disk's text is taken now. With a step to redo
        // (an undo back to clean), taking it would drop that step, so the
        // conflict stays until the next disk event, a Revert, or a redo
        // back to clean with nothing left to redo.
        let reload = doc.conflict.is_some() && was_dirty && !dirty && !can_redo;
        if !patch.is_empty() {
            // From the edited tree: tree-sitter reuses what the edit did
            // not touch.
            self.parse(key, true, true, cx);
        }
        if reload {
            self.start_job(key, cx);
        }
    }

    /// Puts the cursor of `view` at `point` and scrolls to it, now if its
    /// text is loaded, else as soon as it is.
    pub fn go_to(&mut self, view: &Entity<CodeView>, point: Point, cx: &mut Context<Self>) {
        self.go_to_target(view, GoTo::Byte(point), cx);
    }

    /// [`Documents::go_to`] for a [`GoTo`], whose character column is
    /// turned into bytes against the loaded text.
    pub(crate) fn go_to_target(
        &mut self,
        view: &Entity<CodeView>,
        to: GoTo,
        cx: &mut Context<Self>,
    ) {
        let key = self.key(view.read(cx).path());
        match self.docs.get_mut(&key) {
            Some(doc) if doc.snapshot.is_none() => {
                doc.pending_go_to = Some(to);
                // The jump supersedes a restored scroll row.
                self.pending_scroll.remove(&key);
            }
            _ => view.update(cx, |v, cx| {
                let p = to.point_in(v.snapshot());
                v.go_to(p, cx)
            }),
        }
    }

    /// Remembers `to` for the next view of `path` to open: it jumps there
    /// once its text is loaded.
    pub(crate) fn go_to_when_opened(&mut self, path: &Path, to: impl Into<GoTo>) {
        let key = self.key(path);
        let to = to.into();
        // The jump supersedes a restored scroll row.
        self.pending_scroll.remove(&key);
        match self.docs.get_mut(&key) {
            Some(doc) => doc.pending_go_to = Some(to),
            None => {
                self.pending_go_to.insert(key, to);
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
                doc.pending_go_to = Some(GoTo::Byte(point));
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
        // A loaded buffer stays (with its history): the reload decodes the
        // file as text and applies it as an edit.
        doc.force_text = true;
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
                    // A file of that name is not the Untitled document.
                    if untitled::is_untitled(&key) {
                        continue;
                    }
                    let Some(doc) = self.docs.get_mut(&key) else {
                        continue;
                    };
                    match change.kind {
                        ChangeKind::Modified => self.start_job(&key, cx),
                        ChangeKind::Removed => {
                            doc.set_state(DocState::Deleted, cx);
                            // A load in flight may have read the old file.
                            if doc.job.is_some() {
                                doc.reload_again = true;
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
                let keys: Vec<Arc<Path>> = self
                    .docs
                    .keys()
                    .filter(|k| !untitled::is_untitled(k))
                    .cloned()
                    .collect();
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
            // Reading a folder changes no file.
            WorktreeEvent::ScanProgress
            | WorktreeEvent::ScanComplete
            | WorktreeEvent::Loaded(_)
            | WorktreeEvent::WatchFailed(_) => {}
        }
        cx.notify();
    }

    /// The indentation detected for the document at `path`, when
    /// `editor.detectIndentation` holds for its language.
    pub fn indentation(&self, path: &Path) -> Option<Indentation> {
        self.docs.get(&self.key(path))?.indentation
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
            indentation: if view.insert_spaces() {
                format!("Spaces: {}", view.tab_size())
            } else {
                format!("Tab Size: {}", view.tab_size())
            },
        })
    }

    /// Drops the document at `path` with its text and history now, even
    /// while a view of it lives on (the close prompt's Don't Save), sending
    /// [`DocumentEvent::Discarded`], then [`DocumentEvent::Closed`] if it
    /// was opened. The next open reads the disk.
    pub fn discard(&mut self, path: &Path, cx: &mut Context<Self>) {
        let key = self.key(path);
        let removed = self.docs.remove(&key);
        cx.emit(DocumentEvent::Discarded { path: key.clone() });
        if removed.is_some_and(|doc| doc.announced) {
            cx.emit(DocumentEvent::Closed { path: key });
        }
    }

    /// Drops documents whose every view has closed, sending
    /// [`DocumentEvent::Closed`] for those that were opened. The workbench
    /// calls it after each draw.
    pub fn collect_closed(&mut self, cx: &mut Context<Self>) {
        let pending_go_to = &mut self.pending_go_to;
        self.docs.retain(|key, doc| {
            doc.views.retain(|v| v.upgrade().is_some());
            let keep = !doc.views.is_empty() || doc.awaiting_view;
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
        // A save dropped before its write's end left a reload waiting.
        let waiting: Vec<Arc<Path>> = self
            .docs
            .iter()
            .filter(|(_, d)| d.reload_after_save && d.saving.get() == 0)
            .map(|(key, _)| key.clone())
            .collect();
        for key in waiting {
            if let Some(doc) = self.docs.get_mut(&key) {
                doc.reload_after_save = false;
            }
            self.start_job(&key, cx);
        }
    }

    /// Loads or reloads the document off the main thread. While one job
    /// runs, further requests collapse into a single follow-up reload.
    fn start_job(&mut self, key: &Arc<Path>, cx: &mut Context<Self>) {
        // No file to load or reload.
        if untitled::is_untitled(key) {
            return;
        }
        let auto_merge = self.auto_merge(key, cx);
        let Some(doc) = self.docs.get_mut(key) else {
            return;
        };
        if doc.job.is_some() {
            doc.reload_again = true;
            return;
        }
        if doc.saving.get() > 0 {
            // It could read the file half-way through our own write's
            // rename; the write's end starts it.
            doc.reload_after_save = true;
            return;
        }
        let job = match &doc.buffer {
            Some(buffer) => Job::Reload {
                force_text: doc.force_text,
                ours: buffer.snapshot(),
                merge_base: doc.base.clone().filter(|_| buffer.is_dirty()),
                auto_merge,
                dirty: buffer.is_dirty(),
                base_version: buffer.version(),
                base_hash: doc.base_hash,
                base_epoch: doc.disk_epoch,
            },
            None => Job::Load {
                force_text: doc.force_text,
            },
        };
        doc.job = Some(spawn_job(key, job, cx));
    }

    /// Opens the document a hot exit backup was made of, with the backup's
    /// text as unsaved changes, for a tab whose view is made next (spec
    /// §5.4). Returns its key. A document already loaded keeps its text.
    /// An Untitled backup is restored from its text alone, without reading
    /// the disk (see [`restore`]).
    pub(crate) fn open_from_backup(&mut self, backup: Backup, cx: &mut Context<Self>) -> Arc<Path> {
        let key = self.key(&backup.header.path);
        if !self.docs.contains_key(&key) {
            let language = language_for(&self.root, &self.settings, &key, None, cx);
            let pending_go_to = self.pending_go_to.remove(&key);
            let mut doc = Document::new(language, Vec::new(), pending_go_to, Rc::default());
            doc.awaiting_view = true;
            self.docs.insert(key.clone(), doc);
        }
        let doc = self.docs.get_mut(&key).expect("inserted above");
        if doc.buffer.is_none() {
            // Replaces a load in flight.
            doc.job = Some(spawn_job(&key, Job::Restore { backup }, cx));
        }
        key
    }

    /// Whether `lucerna.externalChanges.autoMerge` is on for the document
    /// at `key`.
    fn auto_merge(&self, key: &Arc<Path>, cx: &App) -> bool {
        let language = self.docs.get(key).and_then(Document::language_id);
        self.settings
            .read(cx)
            .get::<bool>("lucerna.externalChanges.autoMerge", language.as_deref(), cx)
            .unwrap_or(true)
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
            Ok(
                output @ (JobOutput::Loaded(Ok(_))
                | JobOutput::Untitled(_)
                | JobOutput::Restored(_)),
            ) => {
                let (buffer, stamp, restored) = match output {
                    JobOutput::Loaded(Ok((buffer, stamp))) => (buffer, Some(stamp), None),
                    JobOutput::Untitled(buffer) => (buffer, None, None),
                    JobOutput::Restored(r) => (r.buffer, r.stamp, Some((r.base, r.base_hash))),
                    _ => unreachable!("matched above"),
                };
                let snapshot = buffer.snapshot();
                let old_snapshot = doc.snapshot.clone();
                take_buffer(doc, buffer, stamp);
                let dirty = restored.is_some();
                if let Some((base, base_hash)) = restored {
                    doc.base = base;
                    doc.base_hash = base_hash;
                }
                let old_id = doc.language_id();
                // Now with the first line, for `first_line_pattern`.
                doc.language = language_for(&self.root, &self.settings, key, Some(&snapshot), cx);
                sync_language_id(doc, cx);
                detect_indentation(doc, &self.settings, cx);
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
                    // A new buffer replaces the old text whole.
                    let old = old_snapshot.unwrap_or_else(|| snapshot.clone());
                    let patch = Patch::new(vec![PatchEdit {
                        old: 0..old.len_bytes(),
                        new: 0..snapshot.len_bytes(),
                    }]);
                    DocumentEvent::Edited {
                        path,
                        old,
                        snapshot: snapshot.clone(),
                        patch: Arc::new(patch),
                        origin: None,
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
                    view.update(cx, |v, cx| {
                        let p = p.point_in(v.snapshot());
                        v.go_to(p, cx)
                    });
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
                if dirty {
                    cx.emit(DocumentEvent::DirtyChanged {
                        path: key.clone(),
                        dirty: true,
                    });
                }
                self.default_line_ending(key, cx);
                parse = true;
            }
            // Read while a save writes: it may be the file from before the
            // write. Read again once the writes are done.
            Ok(JobOutput::Reloaded(..)) if doc.saving.get() > 0 => doc.reload_after_save = true,
            // A save (or revert) landed while the job ran: what it read,
            // or failed to, may be older than the file now.
            Ok(JobOutput::Reloaded(epoch, _)) if epoch != doc.disk_epoch => {
                doc.reload_again = true;
            }
            // The stamp is taken only with the file's text (see
            // `finish_reload`): a stamp without its text would let a save
            // trust a file it never merged.
            Ok(JobOutput::Reloaded(_, Ok(reloaded))) => self.finish_reload(key, reloaded, cx),
            Ok(JobOutput::Loaded(Err(err))) => {
                let state = failure_state(doc, err);
                doc.set_state(state, cx);
            }
            Ok(JobOutput::Reloaded(_, Err(failure))) => {
                let seen = failure.stamp.map(|stamp| (stamp, failure.hash));
                reload_failed(doc, key, failure.err, seen, self.window, cx);
            }
            // The job panicked; a loaded buffer is still here.
            Err(err) if doc.buffer.is_some() => reload_failed(doc, key, err, None, self.window, cx),
            Err(err) => doc.set_state(DocState::Error(err), cx),
        }
        let Some(doc) = self.docs.get_mut(key) else {
            return;
        };
        let again = std::mem::take(&mut doc.reload_again);
        if parse {
            self.parse(key, true, false, cx);
        }
        if again {
            self.start_job(key, cx);
        }
        cx.notify();
    }

    /// Applies a reload's result to the document at `key` (spec §7.1):
    /// - the same bytes as last loaded or saved: nothing changes;
    /// - the buffer changed (or its dirty state did) since the job took its
    ///   snapshot: the job runs again against the new text;
    /// - a clean text, or one equal to the file's: the diff is applied as
    ///   one undo step of its own (which parses the new text), and the text
    ///   counts as saved;
    /// - unsaved changes the disk's do not meet: the disk's are merged in
    ///   as one undo step, and the text stays dirty;
    /// - else the text is left alone and the document is in conflict with
    ///   the disk.
    fn finish_reload(&mut self, key: &Arc<Path>, reloaded: Reloaded, cx: &mut Context<Self>) {
        let Some(doc) = self.docs.get_mut(key) else {
            return;
        };
        let Some(buffer) = doc.buffer.as_ref() else {
            return;
        };
        let was_deleted = doc.state == DocState::Deleted;
        // The file was read again: what Keep Mine skipped is known now.
        doc.skip_disk_check = false;
        let Reloaded {
            decoded,
            theirs,
            base_version,
            dirty,
            outcome,
            stamp,
        } = reloaded;
        if decoded.hash == doc.base_hash {
            // `touch`, a rewrite with the same bytes, or our own save: the
            // file is the one the text is based on, under a new stamp.
            doc.stamp = Some(stamp);
            set_conflict(doc, key, None, cx);
            doc.set_state(DocState::Ready, cx);
            if was_deleted {
                announce_return(doc, key, cx);
            }
            return;
        }
        if buffer.version() != base_version || buffer.is_dirty() != dirty {
            // Edited (or saved) while the job ran: diff or merge again.
            doc.reload_again = true;
            return;
        }
        cx.emit(DocumentEvent::ChangedOnDisk { path: key.clone() });
        match outcome {
            ReloadOutcome::Apply(edits) => {
                tracing::debug!(path = %key.display(), "document reloaded");
                self.apply_disk(key, &decoded, edits, stamp, cx);
            }
            ReloadOutcome::Merge(edits) => {
                tracing::debug!(path = %key.display(), "disk changes merged");
                self.apply_merge(key, decoded, theirs, edits, stamp, cx);
            }
            ReloadOutcome::Conflict => {
                let disk = DiskText {
                    text: theirs,
                    decoded,
                    stamp,
                };
                let conflict = Conflict {
                    disk: Some(disk),
                    reason: None,
                    unreadable: None,
                };
                set_conflict(doc, key, Some(conflict), cx);
                doc.set_state(DocState::Ready, cx);
                if was_deleted {
                    announce_return(doc, key, cx);
                }
            }
        }
    }

    /// Puts the disk's changes into the unsaved text of `key` (spec §7.2):
    /// applies `edits` as one undo step of its own and makes `theirs` the
    /// file's text, so the text stays dirty (every undo state too: none is
    /// the file's) and the next save writes without asking. Any conflict
    /// ends.
    fn apply_merge(
        &mut self,
        key: &Arc<Path>,
        decoded: Decoded,
        theirs: BufferSnapshot,
        edits: Edits,
        stamp: Stamp,
        cx: &mut Context<Self>,
    ) {
        let Some(doc) = self.docs.get_mut(key) else {
            return;
        };
        let Some(buffer) = doc.buffer.as_mut() else {
            return;
        };
        let was_deleted = doc.state == DocState::Deleted;
        let was_dirty = buffer.is_dirty();
        let old = buffer.snapshot();
        // The disk may differ from the base only in its bytes (say, its
        // line endings): then there is no step to undo and nothing to say.
        let patch = if edits.is_empty() {
            Patch::default()
        } else {
            buffer.push_stop();
            let patch = buffer.edit(edits);
            buffer.push_stop();
            patch
        };
        buffer.set_disk_state(&decoded);
        buffer.clear_saved();
        doc.base = Some(theirs);
        doc.base_hash = decoded.hash;
        doc.stamp = Some(stamp);
        doc.disk_epoch += 1;
        doc.skip_disk_check = false;
        doc.lossy_dismissed = false;
        doc.lossy_consent = false;
        doc.encoding = buffer.encoding().name();
        doc.line_ending = buffer.line_ending();
        set_conflict(doc, key, None, cx);
        doc.set_state(DocState::Ready, cx);
        if patch.is_empty() && was_deleted {
            announce_return(doc, key, cx);
        }
        let merged = !patch.is_empty();
        self.commit_edit(key, old, &patch, was_dirty, None, cx);
        self.default_line_ending(key, cx);
        if merged {
            status_message::show_or_log(self.window, "Merged changes from disk", cx);
        }
    }

    /// Makes the text of `key` the file's: applies `edits` (from the text
    /// as it is now to `decoded`'s) as one undo step of its own, takes the
    /// file's encoding, BOM, line ending and hash, and marks the text
    /// saved. Any conflict ends.
    fn apply_disk(
        &mut self,
        key: &Arc<Path>,
        decoded: &Decoded,
        edits: Edits,
        stamp: Stamp,
        cx: &mut Context<Self>,
    ) {
        let Some(doc) = self.docs.get_mut(key) else {
            return;
        };
        let Some(buffer) = doc.buffer.as_mut() else {
            return;
        };
        let was_deleted = doc.state == DocState::Deleted;
        let was_dirty = buffer.is_dirty();
        let old = buffer.snapshot();
        buffer.push_stop();
        let patch = buffer.edit(edits);
        buffer.push_stop();
        buffer.set_disk_state(decoded);
        let saved = buffer.history_state();
        buffer.set_saved(saved);
        doc.base_hash = decoded.hash;
        doc.base = Some(buffer.snapshot());
        doc.disk_epoch += 1;
        doc.stamp = Some(stamp);
        doc.skip_disk_check = false;
        doc.lossy_dismissed = false;
        doc.lossy_consent = false;
        doc.encoding = buffer.encoding().name();
        doc.line_ending = buffer.line_ending();
        set_conflict(doc, key, None, cx);
        doc.set_state(DocState::Ready, cx);
        if patch.is_empty() && was_deleted {
            announce_return(doc, key, cx);
        }
        self.commit_edit(key, old, &patch, was_dirty, None, cx);
        self.default_line_ending(key, cx);
        if let Some(doc) = self.docs.get_mut(key) {
            detect_indentation(doc, &self.settings, cx);
        }
    }

    /// A file with no line break at all says nothing of its line ending:
    /// `files.eol` picks the one new lines get.
    fn default_line_ending(&mut self, key: &Arc<Path>, cx: &App) {
        let Some(doc) = self.docs.get_mut(key) else {
            return;
        };
        let (Some(buffer), Some(snapshot)) = (doc.buffer.as_mut(), doc.snapshot.as_ref()) else {
            return;
        };
        let len = snapshot.len_bytes();
        if snapshot.line_count() > 1 || (len > 0 && snapshot.byte_at(len - 1) == b'\n') {
            return;
        }
        let language = doc.language.as_ref().map(|l| l.id.clone());
        let eol = self
            .settings
            .read(cx)
            .get::<String>("files.eol", language.as_deref(), cx);
        let line_ending = match eol.as_deref() {
            Some("\n") => LineEnding::Lf,
            Some("\r\n") => LineEnding::CrLf,
            // `auto`: the platform's.
            _ if cfg!(windows) => LineEnding::CrLf,
            _ => LineEnding::Lf,
        };
        buffer.set_line_ending(line_ending);
        doc.line_ending = line_ending;
    }

    /// Parses the current text: within [`SYNC_PARSE_BUDGET`] here when
    /// `try_sync`, else (or when that times out) to completion on a
    /// background thread. Large files are not parsed. With `incremental`
    /// (after an edit), the parse starts from the document's syntax, whose
    /// tree was edited to the text, so tree-sitter reuses what the edit
    /// did not touch. A parse of older text in flight is cancelled.
    fn parse(
        &mut self,
        key: &Arc<Path>,
        try_sync: bool,
        incremental: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(doc) = self.docs.get_mut(key) else {
            return;
        };
        let registry = self.settings.read(cx).languages(cx);
        let settings = self.settings.read(cx).view(cx);
        let language = doc.language.as_ref().map(|l| l.id.clone());
        let max_bytes = settings
            .get::<u64>("lucerna.largeFile.maxBytes", language.as_deref())
            .unwrap_or(20 * 1024 * 1024);
        let max_lines = settings
            .get::<u32>("lucerna.largeFile.maxLines", language.as_deref())
            .unwrap_or(300_000);
        // A parse of older text is cancelled; its syntax stays shown (see
        // `finish_job`) until this one lands.
        doc.cancel_parse();
        let (Some(language), Some(buffer)) = (doc.language.clone(), doc.buffer.as_ref()) else {
            return;
        };
        if buffer.is_large(max_bytes, max_lines) {
            clear_syntax(doc, cx);
            return;
        }
        let snapshot = buffer.snapshot();
        let revision = doc.revision;
        // Only a tree of the same language, edited to this text, is a seed.
        let mut seed = doc
            .syntax
            .as_ref()
            .filter(|s| incremental && Arc::ptr_eq(s.language(), &language))
            .and_then(|s| catch_panic("fork syntax", || s.fork()).ok());
        // A WASM grammar's first load compiles a module, far over the sync
        // budget: it happens in the background job. Otherwise, on the UI
        // thread, a panic in the grammar must not take the app down (spec
        // §7); it is logged and the file shows as plain text. An injected
        // language whose WASM grammar is not loaded yet makes the sync parse
        // time out, so it loads here too.
        let mut started = None;
        if try_sync && language.grammar_ready() {
            let seed = seed.take();
            let sync = catch_panic("parse", || {
                let mut syntax = match seed {
                    Some(seed) => seed,
                    None => SyntaxState::new(language.clone(), registry.clone())?,
                };
                let outcome = syntax.reparse(&snapshot, Some(SYNC_PARSE_BUDGET), None);
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
        let started = started.or(seed);
        let cancel = Arc::new(AtomicBool::new(false));
        doc.parse_cancel = Some(cancel.clone());
        let work = spawn_contained(cx, move || {
            let mut syntax = match started {
                Some(syntax) => syntax,
                None => SyntaxState::new(language, registry)?,
            };
            // Cancelled: newer text is being parsed; this result is dropped.
            syntax.reparse(&snapshot, None, Some(&cancel));
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
                doc.parse_cancel = None;
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

/// Sets the conflict of `doc` with the disk, telling listeners when it
/// starts or ends.
fn set_conflict(
    doc: &mut Document,
    key: &Arc<Path>,
    conflict: Option<Conflict>,
    cx: &mut Context<Documents>,
) {
    let changed = doc.conflict.is_some() != conflict.is_some();
    doc.conflict = conflict;
    if changed {
        cx.emit(DocumentEvent::ConflictChanged {
            path: key.clone(),
            conflict: doc.conflict.is_some(),
        });
    }
}

/// A reload of `doc` failed with `err`. Without text, the document shows
/// the error. A file gone from disk marks the text deleted. Else the text
/// (which may hold unsaved changes) stays shown, editable and savable: the
/// document is in conflict with the disk, and the status bar says why.
/// `seen` is the stamp of the file that could not be read, and the hash of
/// its bytes when they were read: Keep Mine expects them.
fn reload_failed(
    doc: &mut Document,
    key: &Arc<Path>,
    err: LoadError,
    seen: Option<(Stamp, Option<u64>)>,
    window: Option<WindowId>,
    cx: &mut Context<Documents>,
) {
    if doc.buffer.is_none() || err == LoadError::NotFound {
        let state = failure_state(doc, err);
        if doc.state != state {
            doc.set_state(state, cx);
        }
        return;
    }
    tracing::warn!(path = %key.display(), %err, "reload failed; text kept");
    let was_deleted = doc.state == DocState::Deleted;
    let conflict = Conflict {
        disk: None,
        reason: Some(err.to_string()),
        unreadable: seen,
    };
    set_conflict(doc, key, Some(conflict), cx);
    if doc.state != DocState::Ready {
        doc.set_state(DocState::Ready, cx);
    }
    if was_deleted {
        announce_return(doc, key, cx);
    }
    let name = save::display_name(key);
    status_message::show_or_log(window, format!("Failed to reload '{name}': {err}"), cx);
}

/// Puts a save or revert of `doc` in line: returns the sender to drop when
/// it is done, and the work before it to wait for, if any is still going.
fn queue_disk_work(
    doc: &mut Document,
) -> (oneshot::Sender<()>, Option<Shared<oneshot::Receiver<()>>>) {
    let (done, done_rx) = oneshot::channel::<()>();
    let previous = doc
        .last_save
        .replace(done_rx.shared())
        // Polled here: finished work nobody waited on is done too.
        .filter(|p| p.clone().now_or_never().is_none());
    (done, previous)
}

/// Moves the entries of `map` at `from` or under it to their paths under
/// `to` (see [`Documents::rename`]).
fn rekey<V>(map: &mut HashMap<Arc<Path>, V>, from: &Path, to: &Path) {
    let keys: Vec<Arc<Path>> = map
        .keys()
        .filter(|k| k.starts_with(from))
        .cloned()
        .collect();
    for key in keys {
        if let (Some(value), Some(new)) = (
            map.remove(&key),
            crate::workspace_state::moved_path(&key, from, to),
        ) {
            map.insert(Arc::from(new), value);
        }
    }
}

/// Tells listeners that a deleted file is back, with an empty
/// [`DocumentEvent::Edited`]: those that let it go take it up again.
fn announce_return(doc: &Document, key: &Arc<Path>, cx: &mut Context<Documents>) {
    if let Some(snapshot) = doc.snapshot.clone() {
        cx.emit(DocumentEvent::Edited {
            path: key.clone(),
            old: snapshot.clone(),
            snapshot,
            patch: Arc::new(Patch::default()),
            origin: None,
        });
    }
}

fn take_buffer(doc: &mut Document, buffer: Buffer, stamp: Option<Stamp>) {
    doc.snapshot = Some(buffer.snapshot());
    doc.version = buffer.version();
    doc.base_hash = buffer.disk_hash();
    doc.base = doc.snapshot.clone();
    doc.conflict = None;
    doc.lossy_dismissed = false;
    doc.lossy_consent = false;
    doc.encoding = buffer.encoding().name();
    doc.line_ending = buffer.line_ending();
    doc.buffer = Some(buffer);
    doc.stamp = stamp;
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

/// Guesses the document's indentation from its text when
/// `editor.detectIndentation` holds for its language (else none), and
/// hands it to the views. Runs on the main thread: it scans at most 10 000
/// lines.
fn detect_indentation(doc: &mut Document, settings: &Entity<WindowSettings>, cx: &mut App) {
    let inputs = indentation_inputs(doc, settings.read(cx).view(cx));
    doc.indentation_inputs = Some(inputs);
    let (detect, default) = inputs;
    doc.indentation = match (&doc.snapshot, detect) {
        (Some(snapshot), true) => Some(indentation::guess(snapshot, default)),
        _ => None,
    };
    for view in doc.views() {
        let detected = doc.indentation;
        view.update(cx, |v, cx| v.set_indentation(detected, cx));
    }
}

/// `editor.detectIndentation` and the default indentation (`editor.insertSpaces`,
/// `editor.tabSize`) for the document's language.
fn indentation_inputs(doc: &Document, settings: SettingsView<'_>) -> (bool, Indentation) {
    let language = doc.language_id();
    let detect = settings
        .get("editor.detectIndentation", language.as_deref())
        .unwrap_or(true);
    let default = Indentation {
        insert_spaces: settings
            .get("editor.insertSpaces", language.as_deref())
            .unwrap_or(true),
        tab_size: settings
            .get::<u32>("editor.tabSize", language.as_deref())
            .unwrap_or(4)
            .max(1),
    };
    (detect, default)
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
/// The registry is the window's (`settings`): its folder may add
/// associations. An Untitled document's is `files.defaultLanguage`'s.
fn language_for(
    root: &Path,
    settings: &Entity<WindowSettings>,
    key: &Path,
    snapshot: Option<&BufferSnapshot>,
    cx: &App,
) -> Option<Arc<Language>> {
    let settings = settings.read(cx);
    // No name or first line to go by: `files.defaultLanguage`, else plain
    // text.
    if untitled::is_untitled(key) {
        let name = settings.get::<String>("files.defaultLanguage", None, cx)?;
        return settings.languages(cx).language_by_name(name.trim());
    }
    let path = key.strip_prefix(root).unwrap_or(key);
    let first_line = snapshot.map(first_line).unwrap_or_default();
    settings.languages(cx).language_for_file(path, &first_line)
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
    check_regular(&meta)?;
    check_size(meta.len())?;
    let bytes = fs::read(path)?;
    Ok((bytes, Stamp::of(&meta)))
}

/// Reads and decodes `path` for a reload. On failure, says what was learnt
/// of the file first: its stamp, and its bytes' hash once they were read.
fn read_text(path: &Path, force_text: bool) -> Result<(Decoded, Stamp), ReadFailure> {
    let meta = fs::metadata(path).map_err(|err| ReadFailure {
        err: err.into(),
        stamp: None,
        hash: None,
    })?;
    let stamp = Stamp::of(&meta);
    let fail = |err, hash| ReadFailure {
        err,
        stamp: Some(stamp),
        hash,
    };
    check_regular(&meta).map_err(|err| fail(err, None))?;
    check_size(meta.len()).map_err(|err| fail(err, None))?;
    let bytes = fs::read(path).map_err(|err| fail(err.into(), None))?;
    let decoded =
        decode_file(&bytes, force_text).map_err(|err| fail(err, Some(save::hash_bytes(&bytes))))?;
    Ok((decoded, stamp))
}

fn run_job(path: &Path, job: Job) -> JobOutput {
    match job {
        Job::Load { force_text } => JobOutput::Loaded(
            read(path)
                .and_then(|(bytes, stamp)| Ok((Buffer::from_bytes(&bytes, force_text)?, stamp))),
        ),
        Job::Reload {
            force_text,
            ours,
            merge_base,
            auto_merge,
            dirty,
            base_version,
            base_hash,
            base_epoch,
        } => JobOutput::Reloaded(
            base_epoch,
            read_text(path, force_text).map(|(mut decoded, stamp)| {
                let text = std::mem::take(&mut decoded.text);
                let theirs = BufferSnapshot::from_text(&text);
                let outcome = if decoded.hash == base_hash || ours.text_eq(&theirs) {
                    // The same bytes, or the text the user already has.
                    ReloadOutcome::Apply(Vec::new())
                } else if !dirty || merge_base.as_ref().is_some_and(|b| ours.text_eq(b)) {
                    // Clean, or dirty only per the history (typed, then
                    // deleted): nothing of the user's to keep.
                    ReloadOutcome::Apply(diff_edits(&ours, &text))
                } else if let Some(base) = merge_base.filter(|_| auto_merge) {
                    let whole = |s: &BufferSnapshot| s.text_for_range(0..s.len_bytes());
                    match merge(&whole(&base), &whole(&ours), &text) {
                        Ok(edits) => ReloadOutcome::Merge(edits),
                        Err(_) => ReloadOutcome::Conflict,
                    }
                } else {
                    ReloadOutcome::Conflict
                };
                Reloaded {
                    decoded,
                    theirs,
                    base_version,
                    dirty,
                    outcome,
                    stamp,
                }
            }),
        ),
        Job::Restore { backup } => JobOutput::Restored(restore(path, backup)),
    }
}

/// The buffer of `backup`, its text unsaved and without undo history. Its
/// base is the file on disk when that still decodes to the backup's base
/// text; otherwise unknown, so a save asks before writing over the file.
fn restore(path: &Path, backup: Backup) -> Restored {
    let header = backup.header;
    // An Untitled document has no file: never read one of that name.
    let untitled = untitled::is_untitled(path);
    let disk = (!untitled)
        .then(|| read(path).ok())
        .flatten()
        .and_then(|(bytes, stamp)| Some((decode_file(&bytes, false).ok()?, stamp)));
    let mut buffer = Buffer::from_decoded(Decoded {
        text: backup.text,
        encoding: encoding_rs::Encoding::for_label(header.encoding.as_bytes())
            .unwrap_or(encoding_rs::UTF_8),
        has_bom: header.bom,
        line_ending: header.line_ending,
        hash: RESTORED_UNKNOWN_BASE,
        // Consent is not kept: the first save after a restore asks again.
        lossy: header.lossy,
    });
    buffer.clear_saved();
    match disk {
        Some((disk, stamp)) if header.base_sha256 == Some(text_sha256(&disk.text)) => Restored {
            buffer,
            base: Some(BufferSnapshot::from_text(&disk.text)),
            base_hash: disk.hash,
            stamp: Some(stamp),
        },
        _ => Restored {
            buffer,
            base: None,
            base_hash: RESTORED_UNKNOWN_BASE,
            stamp: header
                .size
                .filter(|_| !untitled)
                .map(|len| Stamp::from_parts(header.mtime, len)),
        },
    }
}

/// The empty buffer of a new Untitled document.
fn empty_buffer() -> Buffer {
    Buffer::from_decoded(Decoded {
        text: String::new(),
        encoding: encoding_rs::UTF_8,
        has_bom: false,
        line_ending: LineEnding::Lf,
        hash: RESTORED_UNKNOWN_BASE,
        lossy: false,
    })
}

/// Whether `buffer`, the text of the document at `key`, has changes to
/// lose: an Untitled document while it holds any text, a file's while it
/// differs from the file as last loaded or saved.
fn text_dirty(key: &Path, buffer: &Buffer) -> bool {
    if untitled::is_untitled(key) {
        buffer.snapshot().len_bytes() > 0
    } else {
        buffer.is_dirty()
    }
}

/// Runs `job` on the document at `key` in the background, then
/// [`Documents::finish_job`].
fn spawn_job(key: &Arc<Path>, job: Job, cx: &mut Context<Documents>) -> Task<()> {
    let path = key.clone();
    let work = spawn_contained(cx, move || run_job(&path, job));
    let key = key.clone();
    cx.spawn(async move |this, cx| {
        let output = work.await;
        this.update(cx, |docs, cx| docs.finish_job(&key, output, cx))
            .ok();
    })
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

/// The status-bar segments of the active document: cursor, indentation,
/// encoding, line ending and language.
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
                .child(
                    div()
                        .debug_selector(|| "status-indentation".into())
                        .child(status.indentation),
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

    #[test]
    fn stamp_parts_round_trip() {
        let stamp = Stamp {
            modified: Some(SystemTime::UNIX_EPOCH + std::time::Duration::new(1_700_000_000, 5)),
            len: 42,
        };
        assert_eq!(stamp.to_parts(), (Some((1_700_000_000, 5)), 42));
        let (modified, len) = stamp.to_parts();
        assert_eq!(Stamp::from_parts(modified, len), stamp);
        let unknown = Stamp {
            modified: None,
            len: 0,
        };
        assert_eq!(Stamp::from_parts(None, 0), unknown);
    }

    /// An Untitled backup comes back from its text alone, dirty, and keeps
    /// its number for the next new Untitled.
    #[gpui::test]
    fn untitled_backup_restores_without_disk(cx: &mut gpui::TestAppContext) {
        use crate::hot_exit::store::BackupHeader;
        let (keymap, _) =
            lucerna_settings::Keymap::load(lucerna_settings::Keymap::default_bindings(), None);
        let themes = lucerna_theme::ThemeRegistry::builtin();
        let theme = themes.get_or_default("Lucerna Dark");
        let icon_themes = lucerna_theme::IconThemeRegistry::build(&[]).0;
        let icons = icon_themes.resolve("Lucide");
        cx.set_global(crate::AppState {
            settings: lucerna_settings::SettingsStore::with_defaults(),
            keymap,
            themes,
            theme,
            theme_generation: 0,
            languages: Arc::new(lucerna_syntax::LanguageRegistry::bundled()),
            languages_generation: 0,
            icon_themes,
            icons,
        });
        let dir = tempfile::TempDir::new().unwrap();
        let settings = cx.new(|cx| WindowSettings::new(None, cx));
        let docs = cx.new(|_| Documents::new(dir.path().to_path_buf(), settings));
        let backup = Backup {
            header: BackupHeader {
                path: PathBuf::from("untitled:Untitled-2"),
                mtime: None,
                size: None,
                base_sha256: None,
                encoding: "UTF-8".into(),
                bom: false,
                line_ending: LineEnding::Lf,
                lossy: false,
            },
            text: "kept\n".into(),
        };
        let key = docs.update(cx, |d, cx| d.open_from_backup(backup, cx));
        cx.run_until_parked();
        assert_eq!(&*key, Path::new("untitled:Untitled-2"));
        docs.read_with(cx, |d, _| {
            assert_eq!(d.state(&key), Some(&DocState::Ready));
            assert!(d.is_dirty(&key));
            assert_eq!(d.dirty_paths(), vec![key.clone()]);
            assert_eq!(d.snapshot_of(&key).unwrap().text(), "kept\n");
        });
        let next = docs.update(cx, |d, cx| d.new_untitled(cx));
        assert_eq!(&*next, Path::new("untitled:Untitled-1"));
        let next = docs.update(cx, |d, cx| d.new_untitled(cx));
        assert_eq!(&*next, Path::new("untitled:Untitled-3"));
        assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());
    }
}
