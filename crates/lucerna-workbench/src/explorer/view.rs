//! Draws the [`ExplorerModel`] as a virtualized list and turns clicks into
//! `Workbench::open_path` calls.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use gpui::{
    ClickEvent, Context, Div, Entity, FontWeight, IntoElement, ParentElement, Render,
    ScrollStrategy, Stateful, Styled, Subscription, Task, UniformListScrollHandle, WeakEntity,
    Window, div, prelude::*, px, svg, uniform_list,
};
use lucerna_project::{ChangeKind, EntryKind, WorktreeEvent, WorktreeSnapshot};

use super::model::ExplorerModel;
use crate::layout::color;
use crate::ui::{CHEVRON_DOWN_ICON, CHEVRON_RIGHT_ICON, FILE_ICON, GRID, TREE_ROW_HEIGHT};
use crate::{AppState, EditorGroupsView, OpenMode, Workbench};

/// Indentation per tree level.
const INDENT: f32 = 3. * GRID;
const ICON_SLOT: f32 = 4. * GRID;

/// The file tree in the sidebar's Explorer slot.
pub struct ExplorerView {
    model: ExplorerModel,
    /// Tab paths may be absolute; the tree is relative to this.
    root: PathBuf,
    workbench: WeakEntity<Workbench>,
    scroll: UniformListScrollHandle,
    /// The active tab of the active group, root-relative; its row is
    /// highlighted.
    active: Option<Arc<Path>>,
    /// Repaints when the soonest change decoration expires.
    expiry: Option<Task<()>>,
    _active_tab: Subscription,
}

impl ExplorerView {
    pub(crate) fn new(
        root: PathBuf,
        snapshot: WorktreeSnapshot,
        workbench: WeakEntity<Workbench>,
        editor_groups: &Entity<EditorGroupsView>,
        cx: &mut Context<Self>,
    ) -> Self {
        ExplorerView {
            model: ExplorerModel::new(snapshot),
            root,
            workbench,
            scroll: UniformListScrollHandle::new(),
            active: None,
            expiry: None,
            _active_tab: cx.observe(editor_groups, |this, groups, cx| {
                this.sync_active_tab(&groups, cx)
            }),
        }
    }

    pub fn model(&self) -> &ExplorerModel {
        &self.model
    }

    /// Takes the worktree's latest `snapshot` and, when `decorate` is set,
    /// marks added and modified paths as recently changed.
    pub(crate) fn on_worktree_event(
        &mut self,
        ev: &WorktreeEvent,
        snapshot: WorktreeSnapshot,
        decorate: bool,
        cx: &mut Context<Self>,
    ) {
        self.model.set_snapshot(snapshot);
        match ev {
            WorktreeEvent::Changed(changes) if decorate => {
                let paths: Vec<Arc<Path>> = changes
                    .iter()
                    .filter(|c| c.kind != ChangeKind::Removed)
                    .map(|c| c.path.clone())
                    .collect();
                self.model.mark_changed(&paths, Instant::now());
                self.schedule_expiry(cx);
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

    fn sync_active_tab(&mut self, groups: &Entity<EditorGroupsView>, cx: &mut Context<Self>) {
        let active = groups
            .read(cx)
            .model()
            .active_tab()
            .map(|tab| self.relative(&tab.path));
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
                let icon = match (row.kind, row.expanded) {
                    (EntryKind::Dir, true) => CHEVRON_DOWN_ICON,
                    (EntryKind::Dir, false) => CHEVRON_RIGHT_ICON,
                    _ => FILE_ICON,
                };
                let selector = format!("explorer-row-{}", row.path.display());
                div()
                    .id(row.path.clone())
                    .debug_selector(move || selector)
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(GRID))
                    .h(px(TREE_ROW_HEIGHT))
                    .w_full()
                    .pl(px(2. * GRID + INDENT * f32::from(row.depth)))
                    .pr(px(2. * GRID))
                    .cursor_pointer()
                    .when(selected, |d| d.bg(color(ui.element_selected)))
                    .when(!selected, |d| d.hover(move |s| s.bg(hover)))
                    .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                        this.row_clicked(path.clone(), kind, ev, cx)
                    }))
                    .child(
                        div()
                            .flex()
                            .flex_none()
                            .justify_center()
                            .items_center()
                            .size(px(ICON_SLOT))
                            .child(
                                svg()
                                    .data(icon)
                                    .size(px(if kind == EntryKind::Dir { 12. } else { 14. }))
                                    .text_color(color(ui.icon_muted)),
                            ),
                    )
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

impl Render for ExplorerView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .debug_selector(|| "explorer".into())
            .size_full()
            .child(
                uniform_list(
                    "explorer-rows",
                    self.model.visible_rows().len(),
                    cx.processor(|this, range, _, cx| this.render_rows(range, cx)),
                )
                .size_full()
                .track_scroll(&self.scroll),
            )
    }
}
