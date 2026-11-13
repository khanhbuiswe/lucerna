//! Draws the [`ExplorerModel`] as a virtualized list and turns clicks into
//! `Workbench::open_path` calls.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use gpui::{
    AnyElement, App, Bounds, ClickEvent, Context, Div, Entity, FocusHandle, Focusable, FontWeight,
    IntoElement, MouseButton, MouseDownEvent, ParentElement, Pixels, Point, Render, ScrollStrategy,
    Stateful, Styled, Subscription, Task, UniformListDecoration, UniformListScrollHandle,
    WeakEntity, Window, div, prelude::*, px, uniform_list,
};
use lucerna_project::{ChangeKind, EntryKind, WorktreeEvent, WorktreeSnapshot};
use lucerna_theme::{IconSource, ResolvedIcons};

use super::menu::build_menu;
use super::model::{ExplorerModel, Row, SelectedKind, Selection};
use crate::layout::color;
use crate::ui::{
    ContextMenu, DeployedMenu, GRID, SCROLLBAR_WIDTH, TREE_ROW_HEIGHT, UI_FONT_SIZE,
    deploy_context_menu, file_icon_sized, render_deployed, thumb_geometry,
};
use crate::{AppState, EditorGroupsView, OpenMode, Workbench};

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
    scroll: UniformListScrollHandle,
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
            scroll: UniformListScrollHandle::new(),
            active: None,
            expiry: None,
            context_menu: None,
            menu_generation: 0,
            focus,
            _active_tab: cx.observe(editor_groups, |this, groups, cx| {
                this.sync_active_tab(&groups, cx)
            }),
        }
    }

    /// The root row's label: the project folder's name, as on disk.
    pub fn root_label(&self) -> &str {
        &self.root_label
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
        let menu = cx.new(|cx| build_menu(kind, single_file, owner, cx));
        self.menu_generation += 1;
        let generation = self.menu_generation;
        let replacing = self.context_menu.is_some();
        let focus = menu.read(cx).focus_handle(cx);
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
                    // The explorer is being updated, so commands cannot
                    // read it: hand them the menu's target as `path`.
                    let mut args = command
                        .args
                        .clone()
                        .unwrap_or_else(|| serde_json::json!({}));
                    if let Some(map) = args.as_object_mut() {
                        map.entry("path").or_insert_with(|| {
                            this.selected_abs_path()
                                .to_string_lossy()
                                .into_owned()
                                .into()
                        });
                    }
                    let _ = this.workbench.update(cx, |wb, cx| {
                        wb.run_command(command.command, Some(&args), window, cx)
                    });
                }
            },
        ));
        if replacing {
            // The replaced menu hands focus back to the owner as it closes;
            // take it again afterwards.
            cx.defer_in(window, move |this, window, cx| {
                if this.menu_generation == generation {
                    window.focus(&focus, cx);
                }
            });
        }
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
                    div()
                        .debug_selector(move || format!("explorer-guide-{}-{k}", path.display()))
                        .absolute()
                        .top_0()
                        .left(px(row_indent(k) + ICON_SLOT / 2.))
                        .w(px(1.))
                        .h_full()
                        .bg(color(ui.border_variant))
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

/// A thin overlay scrollbar for a uniform list: a thumb only, drawn at the
/// right edge while the rows overflow the list.
struct ScrollbarThumb {
    color: gpui::Rgba,
    selector: &'static str,
}

impl UniformListDecoration for ScrollbarThumb {
    fn compute(
        &self,
        _visible_range: Range<usize>,
        bounds: Bounds<Pixels>,
        scroll_offset: Point<Pixels>,
        item_height: Pixels,
        item_count: usize,
        _window: &mut Window,
        _cx: &mut App,
    ) -> AnyElement {
        let viewport = f32::from(bounds.size.height);
        let content = f32::from(item_height) * item_count as f32;
        let scroll_top = -f32::from(scroll_offset.y);
        let Some((top, height)) = thumb_geometry(viewport, content, scroll_top) else {
            return gpui::Empty.into_any_element();
        };
        let selector = self.selector;
        // Decorations are placed in content coordinates, which scroll; the
        // visible top of the list is at `scroll_top`.
        div()
            .w(bounds.size.width)
            .h(bounds.size.height)
            .child(
                div()
                    .debug_selector(move || selector.into())
                    .absolute()
                    .top(px(scroll_top + top))
                    .right_0()
                    .w(px(SCROLLBAR_WIDTH))
                    .h(px(height))
                    .rounded(px(SCROLLBAR_WIDTH / 2.))
                    .bg(self.color),
            )
            .into_any_element()
    }
}

impl Render for ExplorerView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let thumb = color(cx.global::<AppState>().theme.ui.scrollbar_thumb);
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
                    .child(
                        uniform_list(
                            "explorer-rows",
                            self.model.visible_rows().len(),
                            cx.processor(|this, range, _, cx| this.render_rows(range, cx)),
                        )
                        .size_full()
                        .with_decoration(ScrollbarThumb {
                            color: thumb,
                            selector: "explorer-scrollbar-thumb",
                        })
                        .track_scroll(&self.scroll),
                    ),
            )
            .children(self.context_menu.as_ref().map(render_deployed))
    }
}
