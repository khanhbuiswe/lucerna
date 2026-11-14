//! Draws the [`ExplorerModel`] as a virtualized list and turns clicks into
//! `Workbench::open_path` calls.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use gpui::{
    AnchoredPositionMode, App, Bounds, ClickEvent, Context, Div, Entity, FocusHandle, Focusable,
    FontWeight, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, ParentElement, Pixels,
    Point, Render, ScrollStrategy, Stateful, Styled, Subscription, Task, UniformListScrollHandle,
    WeakEntity, Window, anchored, deferred, div, point, prelude::*, px, uniform_list,
};
use lucerna_project::fs_ops::{self, NameError};
use lucerna_project::{ChangeKind, EntryKind, WorktreeEvent, WorktreeSnapshot};
use lucerna_theme::{IconSource, ResolvedIcons};

use super::menu::build_menu;
use super::model::{EditKind, EditState, ExplorerModel, Row, SelectedKind, Selection};
use crate::layout::color;
use crate::text_input::TextInput;
use crate::ui::{
    ContextMenu, DeployedMenu, GRID, ScrollHandleSource, ScrollbarState, TREE_ROW_HEIGHT,
    UI_FONT_SIZE, UI_FONT_SIZE_SMALL, deploy_context_menu, file_icon_sized, render_deployed,
};
use crate::{AppState, Documents, EditorGroupsView, OpenMode, Workbench, status_message};

/// Indentation per tree level: one icon slot, so each level's indent
/// guide runs under its parent's chevron.
const INDENT: f32 = 4. * GRID;
const ICON_SLOT: f32 = 4. * GRID;
/// Left padding of every row, before indentation.
const ROW_PADDING: f32 = 2. * GRID;
const CHEVRON_SIZE: f32 = 12.;
const TYPE_ICON_SIZE: f32 = 14.;

/// Left padding of a row at `depth`, before its chevron or icon.
fn row_indent(depth: u16) -> f32 {
    ROW_PADDING + INDENT * f32::from(depth)
}

/// Where a row's label starts, from the row's left edge: after the
/// indent, the chevron slot of a directory and the icon slot.
fn label_x(depth: u16, is_dir: bool) -> f32 {
    row_indent(depth) + if is_dir { ICON_SLOT + GRID } else { 0. } + ICON_SLOT + GRID
}

/// The name being typed in the tree (the model holds where).
struct NameEdit {
    input: Entity<TextInput>,
    /// Why the name cannot be used, shown under the input.
    error: Option<String>,
    /// The file system operation is running; the input takes no new confirm.
    busy: bool,
    /// Confirms or cancels when the input loses focus.
    _blur: Subscription,
}

/// A rename started by `confirm_rename`, for its end (`renamed`).
struct Rename {
    abs_from: PathBuf,
    abs_to: PathBuf,
    /// The new path, root-relative.
    target: Arc<Path>,
    /// The paths given to `Workbench::expect_own_changes`.
    expected: Vec<Arc<Path>>,
    /// The renamed folder was expanded: expand it at its new path.
    expand: bool,
}

/// A path just created: once the worktree has it, select it, reveal it and,
/// for a folder, expand it.
struct PendingReveal {
    path: Arc<Path>,
    expand: bool,
}

/// The file tree in the sidebar's Explorer slot.
pub struct ExplorerView {
    model: ExplorerModel,
    root_label: String,
    /// Single-file mode: the tree holds only the file, and the folder row
    /// is not drawn, as Zed shows a lone file.
    single_file: bool,
    /// Tab paths may be absolute; the tree is relative to this.
    root: PathBuf,
    workbench: WeakEntity<Workbench>,
    /// Read when validating a rename: a name an open document holds is
    /// taken.
    documents: WeakEntity<Documents>,
    scroll: UniformListScrollHandle,
    scrollbar: ScrollbarState,
    /// The active tab of the active group, root-relative; its row is
    /// highlighted.
    active: Option<Arc<Path>>,
    /// Repaints when the soonest change decoration expires.
    expiry: Option<Task<()>>,
    /// The open context menu.
    context_menu: Option<DeployedMenu>,
    /// Counts deployed menus, so a menu that closes late does not clear its
    /// replacement.
    menu_generation: u64,
    /// The sidebar's focus handle: menus give focus back to it.
    focus: FocusHandle,
    name_edit: Option<NameEdit>,
    pending_reveal: Option<PendingReveal>,
    /// The input of an edit ended because its folder vanished: if it still
    /// held the focus, the next frame gives the focus to the sidebar.
    refocus_sidebar: Option<FocusHandle>,
    _active_tab: Subscription,
}

impl ExplorerView {
    pub(crate) fn new(
        root: PathBuf,
        single_file: bool,
        snapshot: WorktreeSnapshot,
        workbench: WeakEntity<Workbench>,
        editor_groups: &Entity<EditorGroupsView>,
        focus: FocusHandle,
        cx: &mut Context<Self>,
    ) -> Self {
        ExplorerView {
            model: ExplorerModel::new(snapshot),
            root_label: root
                .file_name()
                .unwrap_or(root.as_os_str())
                .to_string_lossy()
                .into_owned(),
            single_file,
            root,
            workbench,
            documents: WeakEntity::new_invalid(),
            scroll: UniformListScrollHandle::new(),
            scrollbar: ScrollbarState::default(),
            active: None,
            expiry: None,
            context_menu: None,
            menu_generation: 0,
            focus,
            name_edit: None,
            pending_reveal: None,
            refocus_sidebar: None,
            _active_tab: cx.observe(editor_groups, |this, groups, cx| {
                this.sync_active_tab(&groups, cx)
            }),
        }
    }

    /// The open documents, read when validating a rename.
    pub(crate) fn set_documents(&mut self, documents: WeakEntity<Documents>) {
        self.documents = documents;
    }

    /// The root row's label: the project folder's name, as on disk.
    pub fn root_label(&self) -> &str {
        &self.root_label
    }

    /// The scrollbar thumb as last painted, in window coordinates; `None`
    /// while the rows fit.
    pub fn scrollbar_thumb(&self) -> Option<Bounds<Pixels>> {
        self.scrollbar.thumb()
    }

    /// Whether the mouse is over the scrollbar strip.
    pub fn scrollbar_hovered(&self) -> bool {
        self.scrollbar.is_hovered()
    }

    pub fn selection(&self) -> &Selection {
        self.model.selection()
    }

    /// The open context menu, if any.
    pub fn context_menu(&self) -> Option<&Entity<ContextMenu>> {
        self.context_menu.as_ref().map(|d| &d.menu)
    }

    /// Whether the Explorer's context menu is open.
    pub fn context_menu_open(&self) -> bool {
        self.context_menu.is_some()
    }

    /// The selected resource's kind (see [`ExplorerModel::selected_kind`]).
    /// In single-file mode `Root` is the lone file.
    pub fn selected_kind(&self) -> SelectedKind {
        match self.model.selected_kind() {
            SelectedKind::Root if self.single_file => SelectedKind::File,
            kind => kind,
        }
    }

    /// Absolute path of the selection: the root itself for `Root` (in
    /// single-file mode the lone file).
    pub(crate) fn selected_abs_path(&self) -> PathBuf {
        match self.model.selection() {
            Selection::Path(path) => self.root.join(path),
            Selection::Root if self.single_file => self
                .model
                .visible_rows()
                .first()
                .map_or_else(|| self.root.clone(), |row| self.root.join(&*row.path)),
            Selection::Root => self.root.clone(),
        }
    }

    pub fn model(&self) -> &ExplorerModel {
        &self.model
    }

    /// Whether a name is being typed in the tree.
    pub fn editing(&self) -> bool {
        self.model.edit().is_some()
    }

    /// The error shown under the name being typed, if any.
    pub fn edit_error(&self) -> Option<&str> {
        self.name_edit.as_ref()?.error.as_deref()
    }

    /// The input of the name being typed, if any.
    pub fn name_input(&self) -> Option<&Entity<TextInput>> {
        self.name_edit.as_ref().map(|e| &e.input)
    }

    /// New File… / New Folder…: types the name in a placeholder row inside
    /// the selected folder, or the selected file's folder.
    pub fn start_new(&mut self, kind: EditKind, window: &mut Window, cx: &mut Context<Self>) {
        if self.single_file || self.editing() || kind == EditKind::Rename {
            return;
        }
        let parent: Arc<Path> = match (self.model.selection().clone(), self.model.selected_kind()) {
            (Selection::Path(path), SelectedKind::Dir) => path,
            (Selection::Path(path), _) => Arc::from(path.parent().unwrap_or(Path::new(""))),
            (Selection::Root, _) => Arc::from(Path::new("")),
        };
        let edit = EditState {
            kind,
            parent,
            original: None,
        };
        self.begin_edit(edit, "", 0..0, window, cx);
    }

    /// Selects `selection` (the target of a menu command run with
    /// `args.path`).
    pub(crate) fn select(&mut self, selection: Selection, cx: &mut Context<Self>) {
        self.pending_reveal = None;
        self.model.select(selection);
        cx.notify();
    }

    /// Where the selection goes once `rel` is deleted: its next sibling,
    /// else its previous sibling, else its parent folder, as VS Code
    /// focuses a neighbour. `None` when the selection is not at or under
    /// `rel`. Read before the delete runs: the watcher may drop `rel` from
    /// the tree (moving the selection up) before the delete's result.
    pub(crate) fn neighbour_after_delete(&self, rel: &Path) -> Option<Selection> {
        let Selection::Path(selected) = self.model.selection() else {
            return None;
        };
        if !selected.starts_with(rel) {
            return None;
        }
        let parent = rel.parent().unwrap_or(Path::new(""));
        let siblings: Vec<&Arc<Path>> = self
            .model
            .visible_rows()
            .iter()
            .filter(|r| !r.edit && r.path.parent() == Some(parent))
            .map(|r| &r.path)
            .collect();
        let neighbour = siblings.iter().position(|p| &***p == rel).and_then(|ix| {
            siblings
                .get(ix + 1)
                .or_else(|| ix.checked_sub(1).and_then(|prev| siblings.get(prev)))
        });
        Some(match neighbour {
            Some(path) => Selection::Path((*path).clone()),
            None if parent.as_os_str().is_empty() => Selection::Root,
            None => Selection::Path(Arc::from(parent)),
        })
    }

    /// `rel` was deleted: selects `neighbour` (from
    /// [`Self::neighbour_after_delete`]) while the selection is still at or
    /// under `rel`, or where the tree moved it (an ancestor of `rel`). A
    /// neighbour gone meanwhile gives way to its nearest remaining folder.
    pub(crate) fn select_after_delete(
        &mut self,
        rel: &Path,
        neighbour: Selection,
        cx: &mut Context<Self>,
    ) {
        let moved_by_delete = match self.model.selection() {
            Selection::Root => true,
            Selection::Path(selected) => selected.starts_with(rel) || rel.starts_with(selected),
        };
        if !moved_by_delete {
            return;
        }
        let selection = match neighbour {
            Selection::Path(path) if !self.model.contains(&path) => path
                .ancestors()
                .skip(1)
                .find(|a| !a.as_os_str().is_empty() && self.model.contains(a))
                .map_or(Selection::Root, |a| Selection::Path(Arc::from(a))),
            other => other,
        };
        self.model.select(selection);
        cx.notify();
    }

    /// Rename…: types the selected entry's new name in its own row, the
    /// name before its last extension selected. Not for the root, nor in
    /// single-file mode.
    pub fn start_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.single_file || self.editing() {
            return;
        }
        let Selection::Path(path) = self.model.selection().clone() else {
            return;
        };
        let Some(row) = self.model.visible_rows().iter().find(|r| r.path == path) else {
            return;
        };
        let is_dir = row.kind == EntryKind::Dir;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let edit = EditState {
            kind: EditKind::Rename,
            parent: Arc::from(path.parent().unwrap_or(Path::new(""))),
            original: Some(path.clone()),
        };
        let selected = fs_ops::stem_range(&name, is_dir);
        self.begin_edit(edit, &name, selected, window, cx);
    }

    /// Shows the name input for `edit`, holding `text` with `selected`
    /// (bytes) selected, and gives it the focus.
    fn begin_edit(
        &mut self,
        edit: EditState,
        text: &str,
        selected: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.model.start_edit(edit);
        let view = cx.entity().downgrade();
        let input = cx.new(|cx| {
            let mut input = TextInput::new(cx);
            if !text.is_empty() {
                input.set_text(text, window, cx);
                input.select_range(selected, window, cx);
            }
            input.on_change(move |text, _, cx| {
                let _ = view.update(cx, |this, cx| this.name_changed(text, cx));
            });
            input
        });
        let focus = input.focus_handle(cx);
        let blur = cx.on_blur(&focus, window, |this, window, cx| {
            this.name_blurred(window, cx)
        });
        window.focus(&focus, cx);
        self.name_edit = Some(NameEdit {
            input,
            error: None,
            busy: false,
            _blur: blur,
        });
        if let Some(ix) = self.model.visible_rows().iter().position(|r| r.edit) {
            self.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
        }
        cx.notify();
    }

    /// Checks `name` against the tree (never the disk) as it is typed.
    /// A rename also refuses a target inside the entry itself (`src` to
    /// `src/x`) and a name an open document holds (the tab of a file that
    /// is gone), so two tabs never share a path.
    fn validate_name(&self, name: &str, cx: &App) -> Option<Result<PathBuf, NameError>> {
        let edit = self.model.edit()?;
        let result =
            fs_ops::validate_new_name(&edit.parent, name, edit.original.as_deref(), |path| {
                self.model.contains(path)
            });
        let (Ok(target), Some(original)) = (&result, edit.original.as_deref()) else {
            return Some(result);
        };
        if target == original {
            return Some(result);
        }
        let shown = || {
            target
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        if target.starts_with(original) {
            return Some(Err(NameError::Invalid(shown())));
        }
        let held = self
            .documents
            .upgrade()
            .is_some_and(|d| d.read(cx).has_document_under(&self.root.join(target)));
        if held {
            return Some(Err(NameError::Exists(shown())));
        }
        Some(result)
    }

    fn name_changed(&mut self, text: &str, cx: &mut Context<Self>) {
        let error = match self.validate_name(text, cx) {
            Some(Err(e)) => Some(e.to_string()),
            _ => None,
        };
        if let Some(edit) = &mut self.name_edit {
            edit.error = error;
            cx.notify();
        }
    }

    fn name_key_down(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let m = &ev.keystroke.modifiers;
        if m.platform || m.control || m.alt || m.shift || m.function {
            return;
        }
        let busy = self.name_edit.as_ref().is_some_and(|e| e.busy);
        match ev.keystroke.key.as_str() {
            "enter" => self.confirm_name(window, cx),
            // While the file system works, the result still decides.
            "escape" if busy => {}
            "escape" => self.end_name_edit(true, window, cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    /// As VS Code: losing focus keeps a valid name and drops any other.
    /// Runs on the input's blur and, since a row scrolled out of the list
    /// gets no blur when the focus later moves on, after every frame too.
    fn name_blurred(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = &self.name_edit else {
            return;
        };
        // Still focused: the row was only not drawn (scrolled out of the
        // list), which GPUI reports as a blur. An inactive window keeps
        // the focus it will get back.
        if edit.busy || edit.input.focus_handle(cx).is_focused(window) || !window.is_window_active()
        {
            return;
        }
        let text = edit.input.read(cx).text().to_string();
        match self.validate_name(&text, cx) {
            Some(Ok(_)) => self.confirm_name(window, cx),
            _ => self.end_name_edit(false, window, cx),
        }
    }

    /// The sidebar is being hidden: cancels a name being typed and closes
    /// the menu, neither of which may hold the focus undrawn.
    pub(crate) fn dismiss_transient(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing() || self.name_edit.is_some() {
            self.end_name_edit(false, window, cx);
        }
        if self.context_menu.take().is_some() {
            cx.notify();
        }
    }

    /// Ends the name edit; `refocus` gives the focus back to the Explorer.
    fn end_name_edit(&mut self, refocus: bool, window: &mut Window, cx: &mut Context<Self>) {
        // Dropping the edit first drops its blur listener.
        self.name_edit = None;
        self.model.end_edit();
        if refocus {
            self.focus_sidebar(window, cx);
        }
        cx.notify();
    }

    /// Creates the typed file or folder in the background, unless the name
    /// has an error.
    fn confirm_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(name_edit) = &self.name_edit else {
            return;
        };
        if name_edit.busy {
            return;
        }
        let text = name_edit.input.read(cx).text().to_string();
        let Some(result) = self.validate_name(&text, cx) else {
            return;
        };
        let target = match result {
            Ok(target) => target,
            // Nobody can fix the name: drop it, as a blur does.
            Err(_) if !self.name_focused(window, cx) => {
                self.end_name_edit(false, window, cx);
                return;
            }
            Err(e) => {
                if let Some(name_edit) = &mut self.name_edit {
                    name_edit.error = Some(e.to_string());
                }
                cx.notify();
                return;
            }
        };
        let Some(edit) = self.model.edit().cloned() else {
            return;
        };
        if !self.model.is_dir(&edit.parent) {
            return;
        }
        let folder = match edit.kind {
            EditKind::NewFile => false,
            EditKind::NewFolder => true,
            EditKind::Rename => {
                if let Some(original) = edit.original {
                    self.confirm_rename(original, target, window, cx);
                }
                return;
            }
        };
        // Every path this creates, so the watcher's events for them are
        // not marked as another program's. Recorded before the operation
        // starts: the watcher may report it before the result comes back.
        let created: Vec<Arc<Path>> = target
            .ancestors()
            .take_while(|a| **a != *edit.parent)
            .filter(|a| !self.model.contains(a))
            .map(Arc::from)
            .collect();
        let _ = self
            .workbench
            .update(cx, |wb, _| wb.expect_own_changes(created.iter().cloned()));
        if let Some(name_edit) = &mut self.name_edit {
            name_edit.busy = true;
        }
        let abs = self.root.join(&target);
        let parent_abs = self.root.join(&edit.parent);
        let op = cx.background_spawn(async move {
            // Never re-create a parent folder deleted meanwhile: only the
            // missing levels below it.
            if !parent_abs.is_dir() {
                Err(std::io::ErrorKind::NotFound.into())
            } else if folder {
                fs_ops::create_dir(&abs)
            } else {
                fs_ops::create_file(&abs)
            }
        });
        let target: Arc<Path> = Arc::from(target);
        cx.spawn_in(window, async move |this, cx| {
            let result = op.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.created(target, created, folder, result, window, cx)
            });
        })
        .detach();
    }

    /// Renames `original` to `target` (both root-relative, `target`
    /// validated) in the background. An unchanged name just ends the edit.
    fn confirm_rename(
        &mut self,
        original: Arc<Path>,
        target: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if *target == *original {
            let focused = self.name_focused(window, cx);
            self.end_name_edit(focused, window, cx);
            return;
        }
        let (abs_from, abs_to) = (self.root.join(&original), self.root.join(&target));
        // The old and new paths, and the folders a multi-level name adds,
        // so the watcher's events for them are not marked as another
        // program's. Recorded before the operation starts: the watcher may
        // report it before the result comes back.
        let mut expected: Vec<Arc<Path>> = vec![original.clone()];
        expected.extend(
            target
                .ancestors()
                .take_while(|a| !a.as_os_str().is_empty() && !self.model.contains(a))
                .map(Arc::from),
        );
        let _ = self
            .workbench
            .update(cx, |wb, _| wb.expect_own_changes(expected.iter().cloned()));
        if let Some(name_edit) = &mut self.name_edit {
            name_edit.busy = true;
        }
        let expand = self
            .model
            .visible_rows()
            .iter()
            .any(|r| r.path == original && r.expanded);
        // A save of a document in it still running would write the old
        // path again after the rename: let it end first.
        let saves = self
            .documents
            .upgrade()
            .map(|d| d.read(cx).saves_done_under(&abs_from));
        let (from, to) = (abs_from.clone(), abs_to.clone());
        let rename = Rename {
            abs_from,
            abs_to,
            target: Arc::from(target),
            expected,
            expand,
        };
        cx.spawn_in(window, async move |this, cx| {
            if let Some(saves) = saves {
                saves.await;
            }
            let result = cx
                .background_spawn(async move { fs_ops::rename(&from, &to) })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.renamed(rename, result, window, cx)
            });
        })
        .detach();
    }

    /// The end of [`Self::confirm_rename`]'s operation. On success the
    /// documents, tabs and history follow first, before the watcher's
    /// event for it can arrive.
    fn renamed(
        &mut self,
        rename: Rename,
        result: std::io::Result<()>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Rename {
            abs_from,
            abs_to,
            target,
            expected,
            expand,
        } = rename;
        let focused = self.name_focused(window, cx);
        let name = target
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        match result {
            Err(e) => {
                let _ = self
                    .workbench
                    .update(cx, |wb, _| wb.forget_own_changes(&expected));
                if e.kind() == std::io::ErrorKind::AlreadyExists && focused {
                    self.keep_editing_taken_name(name, cx);
                } else {
                    self.end_name_edit(focused, window, cx);
                    let old = abs_from
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    status_message::show(format!("Unable to rename '{old}': {e}"), cx);
                }
            }
            Ok(()) => {
                let _ = self
                    .workbench
                    .update(cx, |wb, cx| wb.paths_renamed(&abs_from, &abs_to, cx));
                self.end_name_edit(focused, window, cx);
                self.model.select(Selection::Path(target.clone()));
                self.pending_reveal = Some(PendingReveal {
                    path: target,
                    expand,
                });
                self.reveal_pending();
            }
        }
    }

    /// Someone else took `name` after the check: the edit stays open with
    /// the error, for the name to be fixed.
    fn keep_editing_taken_name(&mut self, name: String, cx: &mut Context<Self>) {
        if let Some(name_edit) = &mut self.name_edit {
            name_edit.busy = false;
            name_edit.error = Some(NameError::Exists(name).to_string());
        }
        cx.notify();
    }

    /// Whether the name input has the focus.
    fn name_focused(&self, window: &Window, cx: &App) -> bool {
        self.name_edit
            .as_ref()
            .is_some_and(|e| e.input.focus_handle(cx).is_focused(window))
    }

    /// The end of [`Self::confirm_name`]'s operation.
    fn created(
        &mut self,
        target: Arc<Path>,
        expected: Vec<Arc<Path>>,
        folder: bool,
        result: std::io::Result<()>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if result.is_err() {
            let _ = self
                .workbench
                .update(cx, |wb, _| wb.forget_own_changes(&expected));
        }
        let focused = self.name_focused(window, cx);
        let name = target
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        match result {
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && focused => {
                self.keep_editing_taken_name(name, cx);
            }
            Err(e) => {
                self.end_name_edit(focused, window, cx);
                let kind = if folder { "folder" } else { "file" };
                status_message::show(format!("Unable to create {kind} '{name}': {e}"), cx);
            }
            Ok(()) => {
                self.end_name_edit(focused, window, cx);
                self.model.select(Selection::Path(target.clone()));
                self.pending_reveal = Some(PendingReveal {
                    path: target.clone(),
                    expand: folder,
                });
                self.reveal_pending();
                if !folder {
                    let _ = self
                        .workbench
                        .update(cx, |wb, cx| wb.open_path(target, OpenMode::Pinned, cx));
                }
            }
        }
    }

    /// Selects, reveals and expands a just-created path once the tree has it.
    fn reveal_pending(&mut self) {
        let Some(pending) = self
            .pending_reveal
            .take_if(|p| self.model.contains(&p.path))
        else {
            return;
        };
        if let Some(ix) = self.model.reveal(&pending.path) {
            self.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
        }
        if pending.expand {
            self.model.expand(&pending.path);
        }
        self.model.select(Selection::Path(pending.path));
    }

    /// The icon the active icon theme gives the visible row at `path`
    /// (root-relative); `None` when no such row is shown.
    pub fn row_icon(&self, path: &Path, cx: &App) -> Option<IconSource> {
        let row = self
            .model
            .visible_rows()
            .iter()
            .find(|r| &*r.path == path)?;
        Some(row_icon(&cx.global::<AppState>().icons, row))
    }

    /// Takes the worktree's latest `snapshot` and marks the added and
    /// modified paths that `decorate` (given a root-relative path) accepts
    /// as recently changed. The active tab's file is never marked.
    pub(crate) fn on_worktree_event(
        &mut self,
        ev: &WorktreeEvent,
        snapshot: WorktreeSnapshot,
        decorate: &dyn Fn(&Path) -> bool,
        cx: &mut Context<Self>,
    ) {
        self.model.set_snapshot(snapshot);
        // The model dropped an edit whose folder is gone: drop its input
        // (and blur listener) too, without creating anything.
        if self.model.edit().is_none()
            && let Some(edit) = self.name_edit.take()
        {
            self.refocus_sidebar = Some(edit.input.focus_handle(cx));
        }
        self.reveal_pending();
        match ev {
            WorktreeEvent::Changed(changes) => {
                let paths: Vec<Arc<Path>> = changes
                    .iter()
                    .filter(|c| c.kind != ChangeKind::Removed)
                    .filter(|c| self.active.as_deref() != Some(&*c.path))
                    .filter(|c| decorate(&c.path))
                    .map(|c| c.path.clone())
                    .collect();
                if !paths.is_empty() {
                    self.model.mark_changed(&paths, Instant::now());
                    self.schedule_expiry(cx);
                }
            }
            // The active tab may have been opened before the tree had it.
            WorktreeEvent::ScanComplete => {
                if let Some(path) = self.active.clone() {
                    self.auto_reveal(&path, false, cx);
                }
            }
            _ => {}
        }
        cx.notify();
    }

    /// Marks `path` (absolute or root-relative) as changed on disk by
    /// another program, unless it is the active tab's file.
    pub(crate) fn mark_external_change(&mut self, path: &Arc<Path>, cx: &mut Context<Self>) {
        let path = self.relative(path);
        if self.active.as_ref() == Some(&path) {
            return;
        }
        self.model.mark_changed(&[path], Instant::now());
        self.schedule_expiry(cx);
        cx.notify();
    }

    fn sync_active_tab(&mut self, groups: &Entity<EditorGroupsView>, cx: &mut Context<Self>) {
        let active = groups
            .read(cx)
            .model()
            .active_tab()
            .and_then(|tab| tab.path())
            .map(|path| self.relative(path));
        if active == self.active {
            return;
        }
        self.active = active.clone();
        if let Some(path) = active {
            self.model.clear_changed(&path);
            self.auto_reveal(&path, true, cx);
        }
        cx.notify();
    }

    /// Reveals `path`; selects it when `force`, else only when the selection
    /// is the root or already `path` (a late scan must not undo a click).
    fn auto_reveal(&mut self, path: &Path, force: bool, cx: &mut Context<Self>) {
        let enabled = cx
            .global::<AppState>()
            .settings
            .get::<bool>("explorer.autoReveal", None)
            .unwrap_or(true);
        if !enabled {
            return;
        }
        // Reveal first: `select` keeps a hidden path, but its kind is only
        // known once the row is visible.
        if let Some(ix) = self.model.reveal(path) {
            self.scroll.scroll_to_item(ix, ScrollStrategy::Center);
        }
        let keep = match self.model.selection() {
            Selection::Root => true,
            Selection::Path(p) => &**p == path,
        };
        if force || keep {
            self.model.select(Selection::Path(Arc::from(path)));
        }
    }

    fn relative(&self, path: &Arc<Path>) -> Arc<Path> {
        match path.strip_prefix(&self.root) {
            Ok(rel) if path.is_absolute() => Arc::from(rel),
            _ => path.clone(),
        }
    }

    /// Schedules one repaint for the soonest expiring decoration; each
    /// repaint schedules the next.
    fn schedule_expiry(&mut self, cx: &mut Context<Self>) {
        let now = Instant::now();
        let Some(at) = self.model.next_expiry(now) else {
            self.expiry = None;
            return;
        };
        let delay = at.saturating_duration_since(now);
        self.expiry = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update(cx, |this, cx| {
                this.schedule_expiry(cx);
                cx.notify();
            });
        }));
    }

    fn focus_sidebar(&self, window: &mut Window, cx: &mut Context<Self>) {
        let _ = self
            .workbench
            .update(cx, |wb, cx| wb.focus_sidebar(window, cx));
    }

    /// Opens the context menu for the selection at `position`, or just
    /// below the selected row, left-aligned with its label, when `None`.
    pub(crate) fn open_context_menu(
        &mut self,
        position: Option<Point<Pixels>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let position = position.unwrap_or_else(|| self.selected_row_anchor());
        let kind = self.selected_kind();
        let (single_file, owner) = (self.single_file, self.focus.clone());
        // Root-relative; empty for the root itself.
        let path = self
            .selected_abs_path()
            .strip_prefix(&self.root)
            .map(|rel| rel.to_string_lossy().into_owned())
            .unwrap_or_default();
        let menu = cx.new(|cx| build_menu(kind, single_file, path, owner, cx));
        self.menu_generation += 1;
        let generation = self.menu_generation;
        self.context_menu = Some(deploy_context_menu(
            menu,
            position,
            window,
            cx,
            move |this, command, window, cx| {
                if this.menu_generation == generation {
                    this.context_menu = None;
                    cx.notify();
                }
                if let Some(command) = command {
                    // Run once this update ends: commands may update the
                    // Explorer themselves.
                    let workbench = this.workbench.clone();
                    window.defer(cx, move |window, cx| {
                        let _ = workbench.update(cx, |wb, cx| {
                            wb.run_command(command.command, command.args.as_ref(), window, cx)
                        });
                    });
                }
            },
        ));
        cx.notify();
    }

    /// Just below the selected row, at the left edge of its label.
    fn selected_row_anchor(&self) -> Point<Pixels> {
        let list = self.scroll.0.borrow().base_handle.clone();
        let (bounds, offset) = (list.bounds(), list.offset());
        let rows = self.model.visible_rows();
        let ix = match self.model.selection() {
            Selection::Path(path) => rows.iter().position(|row| row.path == *path),
            Selection::Root if self.single_file => (!rows.is_empty()).then_some(0),
            Selection::Root => None,
        };
        match ix.map(|ix| (ix, &rows[ix])) {
            Some((ix, row)) => Point::new(
                bounds.left() + px(label_x(row.depth, row.kind == EntryKind::Dir)),
                bounds.top() + offset.y + px(TREE_ROW_HEIGHT * (ix + 1) as f32),
            ),
            // The root row sits just above the list.
            None => Point::new(bounds.left() + px(label_x(0, false)), bounds.top()),
        }
    }

    /// Right-click on a row or the root: select it and open its menu.
    fn right_clicked(
        &mut self,
        selection: Selection,
        ev: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        self.pending_reveal = None;
        self.model.select(selection);
        self.open_context_menu(Some(ev.position), window, cx);
    }

    /// Right-click below the last row: the root, or in single-file mode
    /// (which has no root row) the lone file.
    fn empty_space_right_clicked(
        &mut self,
        ev: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selection = match self.model.visible_rows().first() {
            Some(row) if self.single_file => Selection::Path(row.path.clone()),
            _ => Selection::Root,
        };
        self.right_clicked(selection, ev, window, cx);
    }

    fn root_clicked(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pending_reveal = None;
        self.model.select(Selection::Root);
        self.focus_sidebar(window, cx);
        cx.notify();
    }

    fn row_clicked(
        &mut self,
        path: Arc<Path>,
        kind: EntryKind,
        ev: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending_reveal = None;
        self.model.select(Selection::Path(path.clone()));
        self.focus_sidebar(window, cx);
        if kind == EntryKind::Dir {
            // The second click of a double click would undo the first.
            if ev.click_count() == 1 {
                self.model.toggle(&path);
                cx.notify();
            }
            return;
        }
        let mode = if ev.click_count() >= 2 {
            OpenMode::Pinned
        } else {
            OpenMode::Preview
        };
        self.model.clear_changed(&path);
        cx.notify();
        let _ = self
            .workbench
            .update(cx, |wb, cx| wb.open_path(path, mode, cx));
    }

    fn render_rows(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<Stateful<Div>> {
        let ui = cx.global::<AppState>().theme.ui.clone();
        let icons = cx.global::<AppState>().icons.clone();
        let now = Instant::now();
        let hover = color(ui.ghost_element_hover);
        let menu_open = self.context_menu.is_some();
        let rows = self.model.visible_rows();
        let end = range.end.min(rows.len());
        let rows = &rows[range.start.min(end)..end];
        rows.iter()
            .map(|row| {
                if row.edit {
                    return self.render_edit_row(row, cx);
                }
                let path = row.path.clone();
                let kind = row.kind;
                let selected =
                    matches!(self.model.selection(), Selection::Path(p) if *p == row.path);
                let changed = self.model.is_recently_changed(&row.path, now);
                let name = row
                    .path
                    .file_name()
                    .unwrap_or(row.path.as_os_str())
                    .to_string_lossy()
                    .into_owned();
                let is_dir = kind == EntryKind::Dir;
                let type_icon = row_icon(&icons, row);
                // Selectors are formatted lazily: GPUI only calls these
                // closures in test builds, so release frames skip the
                // per-row and per-guide `format!`.
                let selector_path = row.path.clone();
                let selected_path = row.path.clone();
                let outline_path = row.path.clone();
                let right_path = row.path.clone();
                let guides = (0..row.depth).map(|k| {
                    let path = row.path.clone();
                    indent_guide(k, ui.border_variant)
                        .debug_selector(move || format!("explorer-guide-{}-{k}", path.display()))
                });
                div()
                    .id(row.path.clone())
                    .debug_selector(move || format!("explorer-row-{}", selector_path.display()))
                    .relative()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(GRID))
                    .h(px(TREE_ROW_HEIGHT))
                    .w_full()
                    .pl(px(row_indent(row.depth)))
                    .pr(px(2. * GRID))
                    .cursor_pointer()
                    .when(selected, |d| d.bg(color(ui.element_selected)))
                    .when(!selected, |d| d.hover(move |s| s.bg(hover)))
                    .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                        this.row_clicked(path.clone(), kind, ev, window, cx)
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                            this.right_clicked(Selection::Path(right_path.clone()), ev, window, cx)
                        }),
                    )
                    .children(guides)
                    .when(selected && menu_open, |d| {
                        d.child(
                            div()
                                .debug_selector(move || {
                                    format!("explorer-row-outline-{}", outline_path.display())
                                })
                                .absolute()
                                .inset_0()
                                .border_1()
                                .border_color(color(ui.border_focused)),
                        )
                    })
                    .when(selected, |d| {
                        // Marker for tests: a 0-size node that exists in release builds too
                        // (only its selector is test-only).
                        d.child(
                            div()
                                .debug_selector(move || {
                                    format!("explorer-row-selected-{}", selected_path.display())
                                })
                                .absolute()
                                .size_0(),
                        )
                    })
                    // Directories: chevron, then folder. Files: their type
                    // icon alone, in the chevron's slot.
                    .when(is_dir, |d| {
                        let chevron = icons.chevron(row.expanded);
                        d.child(icon_slot(&chevron, CHEVRON_SIZE, ui.icon_muted))
                    })
                    .child(icon_slot(&type_icon, TYPE_ICON_SIZE, ui.icon_muted))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .when(changed, |d| {
                                d.text_color(color(ui.text)).font_weight(FontWeight::BOLD)
                            })
                            .child(name),
                    )
                    .when(changed, |d| {
                        d.child(
                            div()
                                .flex_none()
                                .size(px(1.5 * GRID))
                                .rounded_full()
                                .bg(color(ui.text)),
                        )
                    })
            })
            .collect()
    }
}

impl ExplorerView {
    /// The row whose name is being typed: indent, icons, then the input in
    /// the name's place, and any error in a box just below the row.
    fn render_edit_row(&self, row: &Row, cx: &mut Context<Self>) -> Stateful<Div> {
        let state = cx.global::<AppState>();
        let ui = state.theme.ui.clone();
        let icons = state.icons.clone();
        let el = div().id("explorer-edit-row");
        let Some(edit) = &self.name_edit else {
            return el;
        };
        let is_dir = row.kind == EntryKind::Dir;
        let placeholder = self
            .model
            .edit()
            .is_some_and(|e| e.kind != EditKind::Rename);
        let icon = if placeholder {
            // The icon of the name typed so far.
            let text = edit.input.read(cx).text();
            let name = text.rsplit('/').next().unwrap_or("");
            if is_dir {
                icons.directory(name, false)
            } else {
                icons.file(Path::new(name))
            }
        } else {
            row_icon(&icons, row)
        };
        let error = edit.error.clone().map(|error| {
            let width = self.scroll.0.borrow().base_handle.bounds().size.width;
            let message = div()
                .debug_selector(|| "explorer-edit-error".into())
                .when(width > px(0.), |d| d.w(width))
                .px(px(2. * GRID))
                .py(px(GRID))
                .border_1()
                .border_color(color(ui.error))
                .bg(color(ui.elevated_surface_background))
                .text_size(px(UI_FONT_SIZE_SMALL))
                .text_color(color(ui.text))
                .whitespace_normal()
                .occlude()
                .child(error);
            // Anchored to the row's top-left corner, drawn over the rows below.
            div().absolute().top_0().left_0().child(
                deferred(
                    anchored()
                        .position_mode(AnchoredPositionMode::Local)
                        .position(point(px(0.), px(TREE_ROW_HEIGHT)))
                        .child(message),
                )
                .with_priority(1),
            )
        });
        el.debug_selector(|| "explorer-edit-row".into())
            .relative()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(GRID))
            .h(px(TREE_ROW_HEIGHT))
            .w_full()
            .pl(px(row_indent(row.depth)))
            .on_key_down(cx.listener(Self::name_key_down))
            // Not the list's: that would select the root.
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .children((0..row.depth).map(|k| indent_guide(k, ui.border_variant)))
            .when(is_dir, |d| {
                let chevron = icons.chevron(false);
                d.child(icon_slot(&chevron, CHEVRON_SIZE, ui.icon_muted))
            })
            .child(icon_slot(&icon, TYPE_ICON_SIZE, ui.icon_muted))
            .child(
                div()
                    .debug_selector(|| "explorer-edit-input".into())
                    .flex()
                    .items_center()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .px(px(GRID / 2.))
                    .overflow_hidden()
                    .border_1()
                    .border_color(color(ui.border_focused))
                    .bg(color(ui.editor_background))
                    .child(edit.input.clone()),
            )
            .children(error)
    }

    /// The always-expanded root: an open folder and the project's name.
    /// Clicking it selects the root.
    fn render_root(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = *self.model.selection() == Selection::Root;
        let hover = color(cx.global::<AppState>().theme.ui.ghost_element_hover);
        let state = cx.global::<AppState>();
        let ui = &state.theme.ui;
        let root_icon = state.icons.directory(&self.root_label, true);
        div()
            .id("explorer-root")
            .debug_selector(|| "explorer-root".into())
            .relative()
            .flex()
            .flex_none()
            .flex_row()
            .items_center()
            .gap(px(GRID))
            .h(px(TREE_ROW_HEIGHT))
            .pl(px(ROW_PADDING))
            .pr(px(2. * GRID))
            .text_size(px(UI_FONT_SIZE))
            .text_color(color(ui.text))
            .cursor_pointer()
            .when(selected, |d| {
                d.bg(color(ui.element_selected)).child(
                    div()
                        .debug_selector(|| "explorer-root-selected".into())
                        .absolute()
                        .size_0(),
                )
            })
            .when(!selected, |d| d.hover(move |s| s.bg(hover)))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.root_clicked(window, cx)))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                    this.right_clicked(Selection::Root, ev, window, cx)
                }),
            )
            .when(selected && self.context_menu.is_some(), |d| {
                d.child(
                    div()
                        .debug_selector(|| "explorer-root-outline".into())
                        .absolute()
                        .inset_0()
                        .border_1()
                        .border_color(color(ui.border_focused)),
                )
            })
            .child(
                icon_slot(&root_icon, TYPE_ICON_SIZE, ui.icon_muted)
                    .debug_selector(|| "explorer-root-icon".into()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(self.root_label.clone()),
            )
    }
}

/// The indent guide of tree level `k`: a vertical line under that level's
/// chevron.
fn indent_guide(k: u16, tint: lucerna_theme::Rgba) -> Div {
    div()
        .absolute()
        .top_0()
        .left(px(row_indent(k) + ICON_SLOT / 2.))
        .w(px(1.))
        .h_full()
        .bg(color(tint))
}

/// The icon of a tree row, through the active icon theme.
fn row_icon(icons: &ResolvedIcons, row: &Row) -> IconSource {
    if row.kind == EntryKind::Dir {
        let name = row.path.file_name().map(|n| n.to_string_lossy());
        icons.directory(name.as_deref().unwrap_or(""), row.expanded)
    } else {
        icons.file(&row.path)
    }
}

/// A fixed-size slot holding a centred icon in `tint` (Lucide icons only;
/// extension icons keep their colors).
fn icon_slot(source: &IconSource, size: f32, tint: lucerna_theme::Rgba) -> Div {
    div()
        .flex()
        .flex_none()
        .justify_center()
        .items_center()
        .size(px(ICON_SLOT))
        .child(file_icon_sized(source, px(size), color(tint).into()))
}

impl Render for ExplorerView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(input) = self.refocus_sidebar.take()
            && input.is_focused(window)
        {
            cx.defer_in(window, |this, window, cx| this.focus_sidebar(window, cx));
        }
        if self
            .name_edit
            .as_ref()
            .is_some_and(|e| !e.busy && !e.input.focus_handle(cx).is_focused(window))
        {
            cx.defer_in(window, |this, window, cx| this.name_blurred(window, cx));
        }
        let scrollbar = crate::ui::scrollbar(
            self.scrollbar.clone(),
            ScrollHandleSource {
                handle: self.scroll.0.borrow().base_handle.clone(),
                view: cx.entity_id(),
            },
            cx.entity_id(),
        );
        div()
            .debug_selector(|| "explorer".into())
            .size_full()
            .flex()
            .flex_col()
            .when(!self.single_file, |el| el.child(self.render_root(cx)))
            .child(
                div()
                    .debug_selector(|| "explorer-list".into())
                    .flex_1()
                    .min_h_0()
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                            this.empty_space_right_clicked(ev, window, cx)
                        }),
                    )
                    .relative()
                    .child(
                        uniform_list(
                            "explorer-rows",
                            self.model.visible_rows().len(),
                            cx.processor(|this, range, _, cx| this.render_rows(range, cx)),
                        )
                        .size_full()
                        .track_scroll(&self.scroll),
                    )
                    .child(scrollbar),
            )
            .children(self.context_menu.as_ref().map(render_deployed))
    }
}
