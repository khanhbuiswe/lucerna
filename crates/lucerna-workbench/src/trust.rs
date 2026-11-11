//! Folder trust (spec §5, after Zed's Restricted Mode): language servers
//! run only in folders the user trusted. The choices live in
//! `<data dir>/trusted.json`; the title bar's "Restricted Mode" button and
//! the `workbench.trust.manage` command open the trust modal, which also
//! opens by itself the first time a file whose language server is
//! installed (on `PATH`, or set in the User `lsp` setting) is opened in an
//! untrusted folder. The User setting `session.trustAllWorktrees`
//! trusts every folder without writing the file.
//!
//! Trust is per window: its root, which in single-file mode is the file's
//! folder. Reading and writing `trusted.json` run off the UI thread.

use std::io;
use std::path::{Path, PathBuf};

use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, IntoElement,
    KeyDownEvent, Render, Subscription, Task, Window, div, prelude::*, px, svg,
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
                    .child(bullet("Language servers from running"))
                    .child(bullet(
                        "lsp settings in .lucerna/settings.json from being applied",
                    )),
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

/// A window's trust state, kept by the [`Workbench`].
pub(crate) struct Trust {
    /// Where `trusted.json` is; `None` keeps choices for this session only.
    data_dir: Option<PathBuf>,
    /// Whether the store trusts the root (or the user just did); `None`
    /// until the store is read.
    stored: Option<bool>,
    /// The modal opened by itself, or the user stayed in Restricted Mode:
    /// it does not open by itself again this session.
    prompted: bool,
    /// A file with a server opened before the store was read.
    prompt_waiting: bool,
    modal: Option<ShownTrustModal>,
    /// The last write of `trusted.json`; the next one waits for it.
    write: Option<Task<()>>,
    _load: Option<Task<()>>,
}

struct ShownTrustModal {
    modal: Entity<TrustModal>,
    _events: Subscription,
    focus_before: Option<FocusHandle>,
}

impl Trust {
    pub(crate) fn new(data_dir: Option<PathBuf>) -> Self {
        Trust {
            stored: data_dir.is_none().then_some(false),
            data_dir,
            prompted: false,
            prompt_waiting: false,
            modal: None,
            write: None,
            _load: None,
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
            let root = self.root().to_path_buf();
            let read = cx.background_executor().spawn(async move {
                guarded("trust-load", move || {
                    TrustStore::load(&dir).is_trusted(&root)
                })
                .unwrap_or(false)
            });
            self.trust._load = Some(cx.spawn_in(window, async move |wb, cx| {
                let trusted = read.await;
                wb.update_in(cx, |wb, window, cx| wb.trust_loaded(trusted, window, cx))
                    .ok();
            }));
        }
        cx.subscribe_in(documents, window, Self::trust_document_event)
    }

    /// Whether the window is in Restricted Mode: its servers do not run.
    pub fn is_restricted(&self, cx: &App) -> bool {
        !self.language_servers().read(cx).is_trusted()
    }

    /// The shown trust modal, if any.
    pub fn trust_modal(&self) -> Option<&Entity<TrustModal>> {
        self.trust.modal.as_ref().map(|shown| &shown.modal)
    }

    fn trust_loaded(&mut self, trusted: bool, window: &mut Window, cx: &mut Context<Self>) {
        // The user chose (or cleared) while it was read.
        if self.trust.stored.is_some() {
            return;
        }
        self.trust.stored = Some(trusted);
        self.apply_trust(cx);
        if std::mem::take(&mut self.trust.prompt_waiting) {
            self.prompt_trust(window, cx);
        }
    }

    /// Trusted when the store trusts the root or the User layer sets
    /// `session.trustAllWorktrees`. Called after the store is read and
    /// after every settings change.
    pub(crate) fn apply_trust(&mut self, cx: &mut Context<Self>) {
        let all = cx
            .global::<AppState>()
            .settings
            .get_trusted::<bool>(TRUST_ALL_KEY, None)
            == Some(true);
        let trusted = all || self.trust.stored == Some(true);
        let servers = self.language_servers().clone();
        if servers.read(cx).is_trusted() != trusted {
            servers.update(cx, |servers, cx| servers.set_trusted(trusted, cx));
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
            language_id: Some(language),
            ..
        } = ev
        else {
            return;
        };
        if self.trust.prompted || !self.is_restricted(cx) {
            return;
        }
        let found = self
            .language_servers()
            .update(cx, |servers, cx| servers.server_binary_found(language, cx));
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
        self.write_trust("trust-save", cx, move |store| store.trust(&root, parent));
    }

    /// `workbench.trust.clear`: forgets every trusted folder and puts this
    /// window back in Restricted Mode (unless `session.trustAllWorktrees`).
    /// The modal may open by itself again.
    pub fn clear_trust(&mut self, cx: &mut Context<Self>) {
        self.trust.stored = Some(false);
        self.trust.prompted = false;
        self.apply_trust(cx);
        self.write_trust("trust-clear", cx, TrustStore::clear);
        cx.notify();
    }

    /// Reads the store again and changes it with `change`, off the UI
    /// thread and after the previous write.
    fn write_trust(
        &mut self,
        name: &'static str,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut TrustStore) -> io::Result<()> + Send + 'static,
    ) {
        let Some(dir) = self.trust.data_dir.clone() else {
            return;
        };
        let previous = self.trust.write.take();
        self.trust.write = Some(cx.background_executor().spawn(async move {
            if let Some(previous) = previous {
                previous.await;
            }
            guarded(name, move || {
                let mut store = TrustStore::load(&dir);
                if let Err(err) = change(&mut store) {
                    tracing::error!(%err, path = %store.path.display(), "cannot save the trust file");
                }
            });
        }));
    }
}
