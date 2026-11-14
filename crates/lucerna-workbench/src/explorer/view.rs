//! Draws the [`ExplorerModel`] as a virtualized list and turns clicks into
//! `Workbench::open_path` calls.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use gpui::{
    AnyElement, App, Bounds, ClickEvent, Context, Div, Entity, FontWeight, IntoElement,
    ParentElement, Pixels, Point, Render, ScrollStrategy, Stateful, Styled, Subscription, Task,
    UniformListDecoration, UniformListScrollHandle, WeakEntity, Window, div, prelude::*, px,
    uniform_list,
};
use lucerna_project::{ChangeKind, EntryKind, WorktreeEvent, WorktreeSnapshot};
use lucerna_theme::{IconSource, ResolvedIcons, Rgba};

use super::model::{ExplorerModel, Row};
use crate::git::GitSnapshot;
use crate::git::rollup::{GitColor, Rollup};
use crate::layout::color;
use crate::ui::{
    GRID, SCROLLBAR_WIDTH, TREE_ROW_HEIGHT, UI_FONT_SIZE, UI_FONT_SIZE_SMALL, file_icon_sized,
    thumb_geometry,
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
    /// The latest git statuses, rolled up to folders.
    git: GitSnapshot,
    _active_tab: Subscription,
}

impl ExplorerView {
    pub(crate) fn new(
        root: PathBuf,
        single_file: bool,
        snapshot: WorktreeSnapshot,
        workbench: WeakEntity<Workbench>,
        editor_groups: &Entity<EditorGroupsView>,
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
            git: GitSnapshot::default(),
            _active_tab: cx.observe(editor_groups, |this, groups, cx| {
                this.sync_active_tab(&groups, cx)
            }),
        }
    }

    /// The root row's label: the project folder's name, as on disk.
    pub fn root_label(&self) -> &str {
        &self.root_label
    }

    pub fn model(&self) -> &ExplorerModel {
        &self.model
    }

    /// Swaps in a finished git snapshot and repaints.
    pub(crate) fn set_git(&mut self, snapshot: GitSnapshot, cx: &mut Context<Self>) {
        self.git = snapshot;
        cx.notify();
    }

    /// The git color and badge of a row, honoring the decoration settings.
    /// Shared by `render_rows` and the test accessors.
    fn git_decoration(
        &self,
        path: &Path,
        is_dir: bool,
        cx: &App,
    ) -> (Option<Rgba>, Option<&'static str>) {
        let settings = &cx.global::<AppState>().settings;
        let on = |key| settings.get::<bool>(key, None).unwrap_or(true);
        if !on("git.decorations.enabled") {
            return (None, None);
        }
        let ui = &cx.global::<AppState>().theme.ui;
        let rollup: &Rollup = &self.git.rollup;
        let git_color = if is_dir {
            rollup.dir(path)
        } else {
            rollup.file(path)
        };
        let colored = git_color.filter(|_| on("explorer.decorations.colors"));
        let text = colored.map(|c| match c {
            GitColor::Conflict => ui.status_conflict,
            GitColor::Deleted => ui.status_deleted,
            GitColor::Modified => ui.status_modified,
            GitColor::Created => ui.status_created,
            GitColor::Ignored => ui.status_ignored,
        });
        let badges = settings
            .get::<bool>("explorer.decorations.badges", None)
            .unwrap_or(false);
        let badge = if badges && !is_dir {
            self.git.statuses.get(path).map(|s| Rollup::badge(*s))
        } else {
            None
        };
        // The badge shares the name's color; with colors off it is plain.
        (text, badge)
    }

    /// The git color of the visible row at `path`; `None` when uncolored or
    /// not shown.
    #[doc(hidden)]
    pub fn row_text_color(&self, path: &Path, cx: &App) -> Option<gpui::Hsla> {
        let row = self
            .model
            .visible_rows()
            .iter()
            .find(|r| &*r.path == path)?;
        let (text, _) = self.git_decoration(path, row.kind == EntryKind::Dir, cx);
        text.map(|c| color(c).into())
    }

    /// The badge letter of the visible row at `path`.
    #[doc(hidden)]
    pub fn row_badge(&self, path: &Path, cx: &App) -> Option<&'static str> {
        let row = self
            .model
            .visible_rows()
            .iter()
            .find(|r| &*r.path == path)?;
        self.git_decoration(path, row.kind == EntryKind::Dir, cx).1
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
                    self.auto_reveal(&path, cx);
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
            self.auto_reveal(&path, cx);
        }
        cx.notify();
    }

    fn auto_reveal(&mut self, path: &Path, cx: &mut Context<Self>) {
        let enabled = cx
            .global::<AppState>()
            .settings
            .get::<bool>("explorer.autoReveal", None)
            .unwrap_or(true);
        if !enabled {
            return;
        }
        if let Some(ix) = self.model.reveal(path) {
            self.scroll.scroll_to_item(ix, ScrollStrategy::Center);
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

    fn row_clicked(
        &mut self,
        path: Arc<Path>,
        kind: EntryKind,
        ev: &ClickEvent,
        cx: &mut Context<Self>,
    ) {
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
        let rows = self.model.visible_rows();
        let end = range.end.min(rows.len());
        let rows = &rows[range.start.min(end)..end];
        rows.iter()
            .map(|row| {
                let path = row.path.clone();
                let kind = row.kind;
                let selected = self.active.as_deref() == Some(&*row.path);
                let changed = self.model.is_recently_changed(&row.path, now);
                let name = row
                    .path
                    .file_name()
                    .unwrap_or(row.path.as_os_str())
                    .to_string_lossy()
                    .into_owned();
                let is_dir = kind == EntryKind::Dir;
                let (git_text, badge) = self.git_decoration(&row.path, is_dir, cx);
                let badge_color = git_text.unwrap_or(ui.text_muted);
                let type_icon = row_icon(&icons, row);
                // Selectors are formatted lazily: GPUI only calls these
                // closures in test builds, so release frames skip the
                // per-row and per-guide `format!`.
                let selector_path = row.path.clone();
                let guides = (0..row.depth).map(|k| {
                    let path = row.path.clone();
                    div()
                        .debug_selector(move || format!("explorer-guide-{}-{k}", path.display()))
                        .absolute()
                        .top_0()
                        .left(px(ROW_PADDING + INDENT * f32::from(k) + ICON_SLOT / 2.))
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
                    .pl(px(ROW_PADDING + INDENT * f32::from(row.depth)))
                    .pr(px(2. * GRID))
                    .cursor_pointer()
                    .when(selected, |d| d.bg(color(ui.element_selected)))
                    .when(!selected, |d| d.hover(move |s| s.bg(hover)))
                    .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                        this.row_clicked(path.clone(), kind, ev, cx)
                    }))
                    .children(guides)
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
                            .when_some(git_text, |d, c| d.text_color(color(c)))
                            .when(changed, |d| {
                                d.when(git_text.is_none(), |d| d.text_color(color(ui.text)))
                                    .font_weight(FontWeight::BOLD)
                            })
                            .child(name),
                    )
                    .when_some(badge, |d, letter| {
                        d.child(
                            div()
                                .flex_none()
                                .text_size(px(UI_FONT_SIZE_SMALL))
                                .text_color(color(badge_color))
                                .child(letter),
                        )
                    })
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
    /// Clicking it does nothing.
    fn render_root(&self, cx: &App) -> impl IntoElement {
        let state = cx.global::<AppState>();
        let ui = &state.theme.ui;
        let root_icon = state.icons.directory(&self.root_label, true);
        div()
            .debug_selector(|| "explorer-root".into())
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
    }
}
