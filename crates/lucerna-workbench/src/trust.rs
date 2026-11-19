//! Folder trust (spec §5, after Zed's Restricted Mode): language servers
//! run only in folders the user trusted. The choices live in
//! `<data dir>/trusted.json`; the title bar's "Restricted Mode" button and
//! the `workbench.trust.manage` command open the trust modal, which also
//! opens by itself the first time a file whose language server is
//! installed (on `PATH`, or set in the User `lsp` setting) is opened in an
//! untrusted folder, or when the folder is in a git repository (spec
//! §4.2 of the git view design). The User setting `session.trustAllWorktrees`
//! trusts every folder without writing the file.
//!
//! Trust is per window: its root, which in single-file mode is the file's
//! folder. Reading and writing `trusted.json` run off the UI thread.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use futures::FutureExt;
use futures::channel::oneshot;
use futures::future::Shared;

use gpui::{
    App, BorrowAppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    Global, IntoElement, KeyDownEvent, Render, Subscription, Task, Window, div, prelude::*, px,
    svg,
};
use serde::{Deserialize, Serialize};

use crate::documents::{DocumentEvent, Documents};
use crate::editor_groups::path_bar_text;
use crate::icons;
use crate::layout::color;
use crate::tasks::guarded;
use crate::ui::{GRID, UI_FONT_SIZE_SMALL, checkbox, popover, primary_text_button, text_button};
use crate::{AppState, CommandRegistry, Workbench};

/// The file under the data directory.
const TRUST_FILE: &str = "trusted.json";
/// The User setting that trusts every folder.
const TRUST_ALL_KEY: &str = "session.trustAllWorktrees";
const MODAL_WIDTH: f32 = 440.;
/// What Restricted Mode prevents, one bullet each in the modal.
const PREVENTS: [&str; 2] = [
    "Language servers and Git from running",
    "lsp settings in .lucerna/settings.json from being applied",
];

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
struct TrustFile {
    /// Trusted folders, each on its own.
    #[serde(default)]
    folders: Vec<PathBuf>,
    /// Folders whose every descendant is trusted.
    #[serde(default)]
    parents: Vec<PathBuf>,
}

/// The trusted folders, saved as `<data dir>/trusted.json`:
/// `{ "folders": [..], "parents": [..] }`. Reads and writes block: keep
/// them off the UI thread.
#[derive(Debug, Clone)]
pub struct TrustStore {
    path: PathBuf,
    file: TrustFile,
}

impl TrustStore {
    /// The store in `data_dir`. A missing or unreadable file is an empty
    /// store, written over on the next change.
    pub fn load(data_dir: &Path) -> TrustStore {
        let path = data_dir.join(TRUST_FILE);
        let file = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|err| {
                tracing::warn!(path = %path.display(), %err, "invalid trust file ignored");
                TrustFile::default()
            }),
            Err(err) if err.kind() == io::ErrorKind::NotFound => TrustFile::default(),
            Err(err) => {
                tracing::warn!(path = %path.display(), %err, "cannot read the trust file");
                TrustFile::default()
            }
        };
        TrustStore { path, file }
    }

    /// Whether `folder` is in `folders`, or is (or is inside) one of the
    /// `parents`.
    pub fn is_trusted(&self, folder: &Path) -> bool {
        self.file.folders.iter().any(|f| f == folder)
            || self.file.parents.iter().any(|p| folder.starts_with(p))
    }

    /// Trusts `folder`, or with `parent` every folder in `folder`'s parent
    /// (`folder` itself when it has none), and saves.
    ///
    /// # Errors
    /// When the file cannot be written.
    pub fn trust(&mut self, folder: &Path, parent: bool) -> io::Result<()> {
        let (list, entry) = match folder.parent().filter(|_| parent) {
            Some(parent) => (&mut self.file.parents, parent),
            None => (&mut self.file.folders, folder),
        };
        if !list.iter().any(|f| f == entry) {
            list.push(entry.to_path_buf());
        }
        self.save()
    }

    /// Forgets every folder and saves.
    ///
    /// # Errors
    /// When the file cannot be written.
    pub fn clear(&mut self) -> io::Result<()> {
        self.file = TrustFile::default();
        self.save()
    }

    /// Writes a temporary file next to it, then renames it over.
    fn save(&self) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_vec_pretty(&self.file).map_err(io::Error::other)?;
        let mut tmp = self.path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &self.path).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
    }
}

/// What the trust modal's buttons and keys chose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustModalEvent {
    /// "Trust and Continue" (Enter); `parent` is the checkbox.
    Trust { parent: bool },
    /// "Stay in Restricted Mode" (Escape), or a click outside.
    Stay,
}

/// The trust modal (spec §5): the folder, what Restricted Mode prevents,
/// a checkbox to trust the parent folder, and the two choices.
pub struct TrustModal {
    /// The folder as shown: `~` for the home folder.
    folder: String,
    /// The parent folder as shown; `None` for a root such as `/`.
    parent: Option<String>,
    trust_parent: bool,
    focus: FocusHandle,
}

impl EventEmitter<TrustModalEvent> for TrustModal {}

impl TrustModal {
    pub fn new(folder: &Path, cx: &mut Context<Self>) -> Self {
        let home = std::env::home_dir();
        let shown = |path: &Path| path_bar_text(path, None, home.as_deref());
        TrustModal {
            folder: shown(folder),
            parent: folder.parent().map(shown),
            trust_parent: false,
            focus: cx.focus_handle(),
        }
    }

    /// The bullets under "Restricted Mode prevents:".
    pub fn prevents(&self) -> [&'static str; 2] {
        PREVENTS
    }

    fn toggle_parent(&mut self, cx: &mut Context<Self>) {
        self.trust_parent = !self.trust_parent;
        cx.notify();
    }

    fn trust(&mut self, cx: &mut Context<Self>) {
        cx.emit(TrustModalEvent::Trust {
            parent: self.trust_parent,
        });
    }

    fn key_down(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let m = &ev.keystroke.modifiers;
        if m.control || m.alt || m.shift || m.platform {
            return;
        }
        match ev.keystroke.key.as_str() {
            "enter" => self.trust(cx),
            "escape" => cx.emit(TrustModalEvent::Stay),
            _ => return,
        }
        cx.stop_propagation();
    }
}

impl Focusable for TrustModal {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for TrustModal {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = cx.global::<AppState>().theme.ui.clone();
        let muted = color(ui.text_muted);
        let bullet = |text: &'static str| {
            div()
                .flex()
                .flex_row()
                .gap(px(2. * GRID))
                .pl(px(GRID))
                .child(div().flex_none().text_color(muted).child("•"))
                .child(text)
        };
        let header = div()
            .flex()
            .flex_col()
            .gap(px(GRID))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(2. * GRID))
                    .child(
                        svg()
                            .data(icons::TRIANGLE_ALERT)
                            .flex_none()
                            .size(px(16.))
                            .text_color(color(ui.warning)),
                    )
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Unrecognized Project"),
                    ),
            )
            .child(
                div()
                    .debug_selector(|| "trust-folder".into())
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(px(UI_FONT_SIZE_SMALL))
                    .text_color(muted)
                    .child(self.folder.clone()),
            );
        let body = div()
            .flex()
            .flex_col()
            .gap(px(2. * GRID))
            .text_size(px(UI_FONT_SIZE_SMALL))
            .child("Untrusted projects are opened in Restricted Mode to protect your system.")
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(GRID))
                    .child(div().text_color(muted).child("Restricted Mode prevents:"))
                    .children(PREVENTS.map(bullet)),
            );
        let parent_row = self.parent.clone().map(|parent| {
            div()
                .id("trust-parent-checkbox")
                .debug_selector(|| "trust-parent-checkbox".into())
                .flex()
                .flex_row()
                .items_center()
                .gap(px(2. * GRID))
                .cursor_pointer()
                .text_size(px(UI_FONT_SIZE_SMALL))
                .on_click(cx.listener(|modal, _, _, cx| modal.toggle_parent(cx)))
                .child(checkbox(self.trust_parent, cx))
                .child(
                    div()
                        .min_w_0()
                        .child(format!("Trust all projects in the {parent} folder")),
                )
        });
        let footer = div()
            .flex()
            .flex_row()
            .justify_end()
            .gap(px(2. * GRID))
            .px(px(4. * GRID))
            .py(px(3. * GRID))
            .border_t_1()
            .border_color(color(ui.border_variant))
            .child(
                text_button("trust-stay", "Stay in Restricted Mode", cx)
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(TrustModalEvent::Stay))),
            )
            .child(
                primary_text_button("trust-continue", "Trust and Continue", cx)
                    .on_click(cx.listener(|modal, _, _, cx| modal.trust(cx))),
            );
        popover(cx)
            .id("trust-modal")
            .debug_selector(|| "trust-modal".into())
            .track_focus(&self.focus)
            .max_w(px(MODAL_WIDTH))
            .occlude()
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(TrustModalEvent::Stay)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4. * GRID))
                    .p(px(4. * GRID))
                    .child(header)
                    .child(body)
                    .children(parent_row),
            )
            .child(footer)
    }
}

/// Held while `trusted.json` is read, changed and written, by any window.
static TRUST_FILE_LOCK: Mutex<()> = Mutex::new(());

/// A window's trust state, kept by the [`Workbench`].
pub(crate) struct Trust {
    /// Where `trusted.json` is; `None` keeps choices for this session only.
    data_dir: Option<PathBuf>,
    /// The window whose status bar shows a failed save.
    window: gpui::WindowId,
    /// Whether the store trusts the root (or the user just did); `None`
    /// until the store is read.
    stored: Option<bool>,
    /// Set when saving this window's choice failed: the choice holds for
    /// the session, and a re-read of the (unchanged) file does not undo it.
    /// The next save from this window that works clears it.
    session_override: Option<bool>,
    /// The modal opened by itself, or the user stayed in Restricted Mode:
    /// it does not open by itself again this session.
    prompted: bool,
    /// A file with a server opened before the store was read.
    prompt_waiting: bool,
    modal: Option<ShownTrustModal>,
    /// The current press on the Restricted Mode button found the modal
    /// open: that press closes it (a press outside), the click must not
    /// open it again.
    press_closed: bool,
    /// Done when the last write of `trusted.json` has finished; the next
    /// one waits for it. The writes themselves are detached, so closing
    /// the window right after a choice does not lose it.
    write: Option<Shared<oneshot::Receiver<()>>>,
    _load: Option<Task<()>>,
    /// The latest re-read after another window changed `trusted.json`.
    _reload: Option<Task<()>>,
}

/// Counts the writes of `trusted.json` made by any window in this app; a
/// window that observes it reads the file again for its own root.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TrustGeneration(pub u64);

impl Global for TrustGeneration {}

/// Tells every window that `trusted.json` changed.
fn bump_trust_generation(cx: &mut App) {
    if cx.has_global::<TrustGeneration>() {
        cx.update_global::<TrustGeneration, _>(|generation, _| generation.0 += 1);
    } else {
        cx.set_global(TrustGeneration(1));
    }
}

struct ShownTrustModal {
    modal: Entity<TrustModal>,
    _events: Subscription,
    focus_before: Option<FocusHandle>,
}

impl Trust {
    pub(crate) fn new(data_dir: Option<PathBuf>, window: gpui::WindowId) -> Self {
        Trust {
            window,
            session_override: None,
            stored: data_dir.is_none().then_some(false),
            data_dir,
            prompted: false,
            prompt_waiting: false,
            modal: None,
            press_closed: false,
            write: None,
            _load: None,
            _reload: None,
        }
    }
}

pub(crate) fn register_commands(commands: &mut CommandRegistry) {
    commands.register(
        "workbench.trust.manage",
        "Workspaces: Manage Workspace Trust",
        |wb, _, window, cx| wb.show_trust_modal(window, cx),
    );
    commands.register(
        "workbench.trust.clear",
        "Workspaces: Clear Trusted Folders",
        |wb, _, _, cx| wb.clear_trust(cx),
    );
}

impl Workbench {
    /// Reads the store off the UI thread, then applies it, and follows
    /// opened documents to open the modal. Called once, by `build`.
    pub(crate) fn start_trust(
        &mut self,
        documents: &Entity<Documents>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Subscription {
        if let Some(dir) = self.trust.data_dir.clone() {
            let read = self.read_trust(dir, None, cx);
            self.trust._load = Some(cx.spawn_in(window, async move |wb, cx| {
                let trusted = read.await;
                wb.update_in(cx, |wb, window, cx| wb.trust_loaded(trusted, window, cx))
                    .ok();
            }));
        }
        // Without a store (`stored` is already known) nothing is read.
        self.apply_trust(cx);
        cx.subscribe_in(documents, window, Self::trust_document_event)
    }

    /// Whether the store in `dir` trusts this window's root, read off the
    /// UI thread after `after` (a write of this window) is done.
    fn read_trust(
        &self,
        dir: PathBuf,
        after: Option<Shared<oneshot::Receiver<()>>>,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        let root = self.root().to_path_buf();
        // A lone file is also trusted inside a trusted folder, such as
        // its repository opened earlier as a project.
        let lone = self.is_single_file();
        cx.background_executor().spawn(async move {
            if let Some(after) = after {
                let _ = after.await;
            }
            guarded("trust-load", move || {
                let _file = TRUST_FILE_LOCK
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                let store = TrustStore::load(&dir);
                if lone {
                    root.ancestors().any(|folder| store.is_trusted(folder))
                } else {
                    store.is_trusted(&root)
                }
            })
            .unwrap_or(false)
        })
    }

    /// Another window (or this one) changed `trusted.json`: reads it again
    /// for this root. A folder no longer trusted goes back to Restricted
    /// Mode, as `workbench.trust.clear` does.
    pub(crate) fn trust_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dir) = self.trust.data_dir.clone() else {
            return;
        };
        let read = self.read_trust(dir, self.trust.write.clone(), cx);
        self.trust._reload = Some(cx.spawn_in(window, async move |wb, cx| {
            let trusted = read.await;
            wb.update_in(cx, |wb, window, cx| wb.trust_reloaded(trusted, window, cx))
                .ok();
        }));
    }

    fn trust_reloaded(&mut self, trusted: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.trust.session_override.is_some() {
            // The file could not take this window's choice; keep it.
        } else if self.trust.stored.is_none() {
            self.trust_loaded(trusted, window, cx);
        } else if self.trust.stored != Some(trusted) {
            self.trust.stored = Some(trusted);
            if !trusted {
                self.trust.prompted = false;
            }
            self.apply_trust(cx);
            cx.notify();
        }
    }

    /// Whether the window is in Restricted Mode: its servers do not run.
    pub fn is_restricted(&self, cx: &App) -> bool {
        !self.language_servers().read(cx).is_trusted()
    }

    /// Whether the title bar shows the Restricted Mode button: trust is
    /// known to be false, and something it holds back is there (language
    /// servers, or a repository for Git).
    pub fn shows_restricted_mode(&self, cx: &App) -> bool {
        let servers = self.language_servers().read(cx);
        servers.shows_restricted_mode()
            || (self.has_repo_hint() && servers.trust_known() && !servers.is_trusted())
    }

    /// The shown trust modal, if any.
    pub fn trust_modal(&self) -> Option<&Entity<TrustModal>> {
        self.trust.modal.as_ref().map(|shown| &shown.modal)
    }

    fn trust_loaded(&mut self, trusted: bool, window: &mut Window, cx: &mut Context<Self>) {
        // Unless the user chose (or cleared) while it was read.
        if self.trust.stored.is_none() {
            self.trust.stored = Some(trusted);
            self.apply_trust(cx);
            if std::mem::take(&mut self.trust.prompt_waiting) {
                self.prompt_trust(window, cx);
            }
        }
        // Git runs only in trusted folders: a repository asks, as a file
        // with a language server does.
        if self.is_restricted(cx) && self.has_repo_hint() && self.git().read(cx).is_enabled() {
            self.server_file_opened(window, cx);
        }
        self.open_restored_terminal(window, cx);
    }

    /// Trusted when the store trusts the root or the User layer sets
    /// `session.trustAllWorktrees`. Called after the store is read and
    /// after every settings change.
    pub(crate) fn apply_trust(&mut self, cx: &mut Context<Self>) {
        let all = self
            .settings()
            .read(cx)
            .get_trusted::<bool>(TRUST_ALL_KEY, None, cx)
            == Some(true);
        let trusted = all || self.trust.stored == Some(true);
        let known = all || self.trust.stored.is_some();
        // No `git` runs until the folder is trusted (spec §4.2). While the
        // store is still being read the store stays Off (not started), not
        // Restricted: a trusted folder's diff tab would flash Restricted Mode.
        if known {
            self.git.update(cx, |git, cx| git.set_trusted(trusted, cx));
        }
        let servers = self.language_servers().clone();
        if servers.read(cx).is_trusted() != trusted {
            servers.update(cx, |servers, cx| servers.set_trusted(trusted, cx));
            cx.notify();
        }
        if servers.read(cx).trust_known() != known {
            servers.update(cx, |servers, cx| servers.set_trust_known(known, cx));
            cx.notify();
        }
    }

    fn trust_document_event(
        &mut self,
        _: &Entity<Documents>,
        ev: &DocumentEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let DocumentEvent::Opened {
            path, language_id, ..
        } = ev
        else {
            return;
        };
        // An untitled document has no file a server could start for.
        if crate::untitled::is_untitled(path) || self.trust.prompted || !self.is_restricted(cx) {
            return;
        }
        let found = self.language_servers().update(cx, |servers, cx| {
            servers.server_binary_found(language_id.as_deref(), path, cx)
        });
        cx.spawn_in(window, async move |wb, cx| {
            if found.await {
                wb.update_in(cx, |wb, window, cx| wb.server_file_opened(window, cx))
                    .ok();
            }
        })
        .detach();
    }

    /// A file whose server can start was opened: prompts now, or once the
    /// store is read.
    fn server_file_opened(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.trust.stored.is_none() {
            self.trust.prompt_waiting = true;
        } else {
            self.prompt_trust(window, cx);
        }
    }

    /// Opens the modal by itself, once per session.
    fn prompt_trust(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.trust.prompted || !self.is_restricted(cx) {
            return;
        }
        self.trust.prompted = true;
        self.show_trust_modal(window, cx);
    }

    /// A press on the Restricted Mode button: notes whether the modal is
    /// open, so the press (outside it) closes it.
    pub(crate) fn restricted_button_pressed(&mut self) {
        self.trust.press_closed = self.trust.modal.is_some();
    }

    /// A click on the Restricted Mode button opens the modal, unless its
    /// press closed it (so it is not closed and opened again).
    pub(crate) fn restricted_button_clicked(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if std::mem::take(&mut self.trust.press_closed) {
            return;
        }
        self.show_trust_modal(window, cx);
    }

    /// Shows the trust modal, closing any other modal overlay.
    pub fn show_trust_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.trust.modal.is_some() {
            return;
        }
        self.close_modal_overlays(window, cx);
        let root = self.root().to_path_buf();
        let modal = cx.new(|cx| TrustModal::new(&root, cx));
        let events = cx.subscribe_in(&modal, window, Self::on_trust_modal_event);
        let focus_before = window.focused(cx);
        window.focus(&modal.focus_handle(cx), cx);
        self.trust.modal = Some(ShownTrustModal {
            modal,
            _events: events,
            focus_before,
        });
        cx.notify();
    }

    pub(crate) fn close_trust_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(shown) = self.trust.modal.take() else {
            return;
        };
        let focused = window.focused(cx);
        if focused.is_none() || shown.modal.focus_handle(cx).contains_focused(window, cx) {
            match shown.focus_before {
                Some(focus) => window.focus(&focus, cx),
                None => self.focus_editor_area(window, cx),
            }
        }
        cx.notify();
    }

    fn on_trust_modal_event(
        &mut self,
        _: &Entity<TrustModal>,
        ev: &TrustModalEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match *ev {
            TrustModalEvent::Trust { parent } => self.trust_root(parent, cx),
            TrustModalEvent::Stay => self.trust.prompted = true,
        }
        self.close_trust_modal(window, cx);
    }

    /// Trusts the root (or with `parent`, its parent folder): servers
    /// start for the open documents, and the choice is saved.
    fn trust_root(&mut self, parent: bool, cx: &mut Context<Self>) {
        self.trust.stored = Some(true);
        self.apply_trust(cx);
        let root = self.root().to_path_buf();
        self.write_trust("trust-save", true, cx, move |store| {
            store.trust(&root, parent)
        });
    }

    /// `workbench.trust.clear`: forgets every trusted folder and puts this
    /// window back in Restricted Mode (unless `session.trustAllWorktrees`).
    /// The modal may open by itself again.
    pub fn clear_trust(&mut self, cx: &mut Context<Self>) {
        self.trust.stored = Some(false);
        self.trust.prompted = false;
        self.apply_trust(cx);
        self.write_trust("trust-clear", false, cx, TrustStore::clear);
        cx.notify();
    }

    /// Reads the store again and changes it with `change`, off the UI
    /// thread and after the previous write.
    fn write_trust(
        &mut self,
        name: &'static str,
        trusted: bool,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut TrustStore) -> io::Result<()> + Send + 'static,
    ) {
        let Some(dir) = self.trust.data_dir.clone() else {
            return;
        };
        let previous = self.trust.write.take();
        let (done, done_rx) = oneshot::channel::<()>();
        let (failed, failed_rx) = oneshot::channel::<String>();
        self.trust.write = Some(done_rx.shared());
        cx.background_executor()
            .spawn(async move {
                if let Some(previous) = previous {
                    let _ = previous.await;
                }
                guarded(name, move || {
                    // Windows share the file: one read-change-write at a time.
                    let _file = TRUST_FILE_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
                    let mut store = TrustStore::load(&dir);
                    if let Err(err) = change(&mut store) {
                        tracing::error!(%err, path = %store.path.display(), "cannot save the trust file");
                        let _ = failed.send(err.to_string());
                    }
                });
                let _ = done.send(());
            })
            .detach();
        // Once written, every window reads the file again. When it could
        // not be written the file is unchanged: reading it again would put
        // this window back to what the file says, so it keeps its choice
        // for the session and says so.
        let written = self.trust.write.clone();
        let window = self.trust.window;
        cx.spawn(async move |this, cx| {
            if let Some(written) = written {
                let _ = written.await;
            }
            match failed_rx.await {
                Ok(err) => {
                    this.update(cx, |wb, _| wb.trust.session_override = Some(trusted))
                        .ok();
                    let rest = if trusted {
                        ""
                    } else {
                        ". The folder stays trusted in other windows and the next time it opens"
                    };
                    cx.update(|cx| {
                        crate::status_message::show(
                            window,
                            format!("Couldn't save the trusted folders list: {err}{rest}"),
                            cx,
                        );
                    });
                }
                Err(_) => {
                    this.update(cx, |wb, _| wb.trust.session_override = None)
                        .ok();
                    cx.update(bump_trust_generation);
                }
            }
        })
        .detach();
    }
}
