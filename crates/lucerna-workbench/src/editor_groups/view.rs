//! Renders the editor groups side by side, each with a tab bar over the
//! active tab's body, separated by draggable dividers.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyView, App, ClickEvent, Context, Div, DragMoveEvent, Entity, IntoElement, MouseButton,
    ParentElement, Pixels, Render, SharedString, Styled, Window, div, prelude::*, px, relative,
};
use lucerna_settings::SettingsView;
use lucerna_theme::{IconSource, Rgba, UiColors};

use super::model::{EditorGroups, GroupsState, Tab, TabId, TabItem};
use crate::conflict_bar::{self, BarActions};
use crate::documents::{DiskNotice, DocState, Documents};
use crate::icons;
use crate::layout::color;
use crate::markdown_preview::{self, SHOW_PREVIEW_TO_SIDE, TOGGLE_PREVIEW};
use crate::save::{self, SaveChoice};
use crate::symbols::breadcrumbs::{
    BreadcrumbSettings, Crumbs, breadcrumb_segments, breadcrumbs_row, segments_text,
};
use crate::ui::{
    self, ClickHandler, GRID, PATH_BAR_HEIGHT, SashGhost, TAB_PADDING_X, UI_FONT_SIZE,
    UI_FONT_SIZE_SMALL, icon_button, tooltip,
};
use crate::{AppState, WindowSettings};

/// What a file tab's dot says about its document.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TabMark {
    /// Unsaved changes.
    Dirty,
    /// The file changed on disk under unsaved changes.
    Conflict,
}

/// The deleted state and the dot of each tab's document, per group.
type TabStates = Vec<Vec<(bool, Option<TabMark>)>>;
const SASH_WIDTH: Pixels = px(ui::SASH_SIZE);
const MIN_GROUP_WIDTH: Pixels = px(120.);

/// Produces the view shown under the tab bar for a tab. It is called once
/// per tab; the view is kept until the tab closes.
pub type TabBodyFactory = Rc<dyn Fn(&Tab, &mut Window, &mut App) -> AnyView>;

/// Called with a group's index when its breadcrumbs row is clicked.
pub(crate) type BreadcrumbsClick = Rc<dyn Fn(usize, &mut Window, &mut App)>;

/// Runs the workbench command with this id.
pub(crate) type RunCommand = Rc<dyn Fn(&'static str, &mut Window, &mut App)>;

/// The default tab body until the code view exists: the file's path (the
/// tab title for the Extensions page).
pub fn placeholder_body(tab: &Tab, _: &mut Window, cx: &mut App) -> AnyView {
    let text = match tab.path() {
        Some(path) => path.display().to_string(),
        None => tab.title(),
    };
    cx.new(|_| PathPlaceholder { text }).into()
}

struct PathPlaceholder {
    text: String,
}

impl Render for PathPlaceholder {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = &cx.global::<AppState>().theme.ui;
        div()
            .flex()
            .size_full()
            .justify_center()
            .items_center()
            .text_color(color(ui.text_muted))
            .child(self.text.clone())
    }
}

/// Drag payload of the divider to the right of group `.0`.
struct GroupSash(usize);

/// The editor area: owns the [`EditorGroups`] model and each tab's body.
pub struct EditorGroupsView {
    model: EditorGroups,
    /// Fraction of the area each group takes; sums to 1.
    widths: Vec<f32>,
    bodies: HashMap<TabId, AnyView>,
    make_body: TabBodyFactory,
    /// The tab under the mouse; its close button shows. Tab elements are
    /// keyed by tab id, so a tab moving into another's place starts with
    /// fresh hover state and GPUI reports its hover on the next frame.
    hovered_tab: Option<TabId>,
    /// Consulted for each tab's title: deleted files are struck through.
    documents: Option<Entity<Documents>>,
    /// The window's settings the tabs, breadcrumbs and empty groups read.
    settings: Entity<WindowSettings>,
    /// The workspace root: path bars show paths relative to it.
    root: Option<PathBuf>,
    /// The user's home folder, shown as `~` in path bars; read when the
    /// view is made and again when its root is set.
    home: Option<PathBuf>,
    /// Shown as the Search tab's title in place of "Search".
    search_title: Option<SharedString>,
    /// Shown as the References tab's title in place of "References".
    references_title: Option<SharedString>,
    /// Shown as the Changes since tab's title in place of "Changes since
    /// default branch".
    branch_title: Option<SharedString>,
    /// The breadcrumbs of file tabs, as the workbench last computed them.
    breadcrumbs: HashMap<TabId, Crumbs>,
    on_breadcrumbs_click: Option<BreadcrumbsClick>,
    /// Runs a workbench command for a path bar button.
    on_command: Option<RunCommand>,
}

impl EditorGroupsView {
    /// An editor area of one empty group, reading the window's `settings`.
    pub fn new(settings: Entity<WindowSettings>) -> Self {
        EditorGroupsView {
            model: EditorGroups::default(),
            widths: vec![1.0],
            bodies: HashMap::new(),
            make_body: Rc::new(placeholder_body),
            hovered_tab: None,
            documents: None,
            settings,
            root: None,
            home: std::env::home_dir(),
            search_title: None,
            references_title: None,
            branch_title: None,
            breadcrumbs: HashMap::new(),
            on_breadcrumbs_click: None,
            on_command: None,
        }
    }
}

/// `workbench.editor.showIcons`: icons on every tab, default on.
fn show_tab_icons(settings: SettingsView<'_>) -> bool {
    settings
        .get("workbench.editor.showIcons", None)
        .unwrap_or(true)
}

/// The icon of a tab: the icon theme's file icon for file tabs and file
/// diffs, a Lucide icon for the special tabs.
fn tab_icon_source(item: &TabItem, cx: &App) -> Option<IconSource> {
    let file = |p: &Path| cx.global::<AppState>().icons.file(p);
    Some(match item {
        TabItem::File(p) | TabItem::FileDiff(p) => file(p),
        TabItem::Extensions => IconSource::Builtin("blocks"),
        TabItem::Search => IconSource::Builtin("search"),
        TabItem::LspLog(_) | TabItem::LspHealth => IconSource::Builtin("zap"),
        TabItem::References => IconSource::Builtin("file-code"),
        TabItem::Onboarding => IconSource::Builtin("settings"),
        TabItem::GitChanges | TabItem::BranchChanges => IconSource::Builtin("git-branch"),
        TabItem::MarkdownPreview(_) | TabItem::MarkdownPreviewDynamic(_) => {
            IconSource::Builtin("eye")
        }
    })
}

impl EditorGroupsView {
    /// The icon of tab `ti` in group `gi`; `None` when the tab does not
    /// exist or `workbench.editor.showIcons` is off.
    pub fn tab_icon(&self, gi: usize, ti: usize, cx: &App) -> Option<IconSource> {
        if !show_tab_icons(self.settings_view(cx)) {
            return None;
        }
        let tab = self.model.groups().get(gi)?.tabs.get(ti)?;
        tab_icon_source(&tab.item, cx)
    }

    pub fn model(&self) -> &EditorGroups {
        &self.model
    }

    /// Changes the model and redraws. Groups get equal widths whenever
    /// their number changes.
    pub fn update_model<R>(
        &mut self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut EditorGroups) -> R,
    ) -> R {
        let result = f(&mut self.model);
        // A closed tab's element is gone and never reports hover-leave.
        if let Some(hovered) = self.hovered_tab {
            let exists = self
                .model
                .groups()
                .iter()
                .any(|g| g.tabs.iter().any(|t| t.id == hovered));
            if !exists {
                self.hovered_tab = None;
            }
        }
        let n = self.model.groups().len();
        if self.widths.len() != n {
            self.widths = vec![1.0 / n as f32; n];
        }
        cx.notify();
        result
    }

    /// Closes tab `id` for the user (the tab's ×, a middle click,
    /// `workbench.action.closeActiveEditor`). When it is the last tab
    /// showing a document with unsaved changes, asks first (spec §6.3):
    /// Save closes it once saved (and still clean), Don't Save closes it
    /// and drops the document with its history, Cancel keeps it.
    pub fn request_close(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        let Some((path, documents)) = self.last_tab_of_dirty(id, cx) else {
            self.update_model(cx, |m| m.close(id));
            return;
        };
        let answer = save::prompt_unsaved(&save::display_name(&path), window, cx);
        cx.spawn_in(window, async move |view, cx| {
            match SaveChoice::of(answer.await) {
                SaveChoice::Save => {
                    // An edit typed while the save wrote is not saved: the
                    // tab stays, still dirty.
                    let saved = save::save_each(documents.clone(), vec![path.clone()], cx).await
                        && !documents.read_with(cx, |d, _| d.is_dirty(&path));
                    if saved {
                        view.update(cx, |view, cx| view.update_model(cx, |m| m.close(id)))
                            .ok();
                    }
                }
                SaveChoice::DontSave => {
                    view.update(cx, |view, cx| view.update_model(cx, |m| m.close(id)))
                        .ok();
                    documents.update(cx, |d, cx| d.discard(&path, cx));
                }
                SaveChoice::Cancel => {}
            }
        })
        .detach();
    }

    /// Close on a deleted file's bar: closes tab `id` without asking and
    /// drops the document at `path` (with its unsaved text and history)
    /// when no other tab shows it, as Don't Save does.
    fn close_deleted(&mut self, id: TabId, path: &Path, cx: &mut Context<Self>) {
        self.update_model(cx, |m| m.close(id));
        let shown = self
            .model
            .groups()
            .iter()
            .flat_map(|g| &g.tabs)
            .any(|t| t.path().is_some_and(|p| **p == *path));
        if !shown && let Some(documents) = self.documents.clone() {
            documents.update(cx, |d, cx| d.discard(path, cx));
        }
    }

    /// The conflict or deleted-file bar (spec §7.3) for group `group`,
    /// whose active tab is `tab`; `None` unless it is a file tab or a
    /// Markdown preview (static or dynamic) whose document has a
    /// [`DiskNotice`].
    fn disk_bar(&self, group: usize, tab: &Tab, cx: &mut Context<Self>) -> Option<Div> {
        let path = tab.path().or(tab.previewed_path())?.clone();
        let documents = self.documents.clone()?;
        let notice = documents.read(cx).disk_notice(&path)?;
        let id = tab.id;
        let actions = match notice {
            DiskNotice::Conflict { .. } => {
                let (docs, p) = (documents.clone(), path.clone());
                let reload: ClickHandler = Box::new(move |_, _, cx| {
                    docs.update(cx, |d, cx| d.revert(&p, cx).detach());
                });
                let (docs, p) = (documents, path.clone());
                let keep_mine: ClickHandler = Box::new(move |_, _, cx| {
                    docs.update(cx, |d, cx| d.keep_mine(&p, cx));
                });
                BarActions {
                    primary: reload,
                    secondary: keep_mine,
                }
            }
            DiskNotice::Deleted => {
                let p = path.clone();
                let close = cx.listener(move |view, _: &ClickEvent, _, cx| {
                    view.close_deleted(id, &p, cx);
                });
                let (docs, p) = (documents, path.clone());
                let keep: ClickHandler = Box::new(move |_, _, cx| {
                    docs.update(cx, |d, cx| d.keep_deleted(&p, cx));
                });
                BarActions {
                    primary: Box::new(close),
                    secondary: keep,
                }
            }
        };
        Some(conflict_bar::render(
            group,
            &save::display_name(&path),
            &notice,
            actions,
            cx,
        ))
    }

    /// The path and documents of tab `id` when it shows a document with
    /// unsaved changes that no other tab, in any group, shows.
    fn last_tab_of_dirty(&self, id: TabId, cx: &App) -> Option<(Arc<Path>, Entity<Documents>)> {
        let tabs = || self.model.groups().iter().flat_map(|g| &g.tabs);
        let path = tabs().find(|t| t.id == id)?.path()?.clone();
        let documents = self.documents.clone()?;
        let shown_elsewhere = tabs().any(|t| t.id != id && t.path() == Some(&path));
        (!shown_elsewhere && documents.read(cx).is_dirty(&path)).then_some((path, documents))
    }

    /// Replaces the layout with the one saved in `state` (see
    /// [`EditorGroups::restore`]) and the saved group `widths`. Widths
    /// that do not fit the restored groups (another count, say after an
    /// empty group was dropped, or not positive) become equal; others are
    /// scaled to sum to 1. Tab bodies are made again, through the factory,
    /// as they are drawn.
    pub fn restore(&mut self, state: &GroupsState, widths: &[f32], cx: &mut Context<Self>) {
        self.model = EditorGroups::restore(state);
        self.bodies.clear();
        self.hovered_tab = None;
        let n = self.model.groups().len();
        let total: f32 = widths.iter().sum();
        let fits = widths.len() == n && widths.iter().all(|w| w.is_finite() && *w > 0.);
        self.widths = if fits && total.is_finite() {
            widths.iter().map(|w| w / total).collect()
        } else {
            vec![1.0 / n as f32; n]
        };
        cx.notify();
    }

    /// The body of tab `id`, if it has been drawn.
    /// The bodies made for tabs that are not the active tab of their
    /// group, so not on screen.
    pub(crate) fn hidden_bodies(&self) -> impl Iterator<Item = &AnyView> + '_ {
        let active: Vec<TabId> = self
            .model
            .groups()
            .iter()
            .filter_map(|g| g.active)
            .collect();
        self.bodies
            .iter()
            .filter(move |(id, _)| !active.contains(id))
            .map(|(_, body)| body)
    }

    /// The view made for tab `id`, once it has been drawn (or made).
    pub fn body_of(&self, id: TabId) -> Option<AnyView> {
        self.bodies.get(&id).cloned()
    }

    /// Fraction of the editor area's width each group takes, left to right.
    pub fn group_widths(&self) -> &[f32] {
        &self.widths
    }

    /// The body of the focused group's active tab, once it has been drawn.
    pub fn active_body(&self) -> Option<AnyView> {
        let tab = self.model.active_tab()?;
        self.bodies.get(&tab.id).cloned()
    }

    /// The body of the focused group's active tab, made now if it has not
    /// been drawn yet.
    pub(crate) fn ensure_active_body(
        &mut self,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyView> {
        let tab = self.model.active_tab()?.clone();
        Some(self.body(&tab, window, cx))
    }

    /// The bodies of every group's active tab, made now if they have not
    /// been drawn yet.
    pub(crate) fn ensure_active_bodies(&mut self, window: &mut Window, cx: &mut App) {
        let active: Vec<Tab> = self
            .model
            .groups()
            .iter()
            .filter_map(|g| g.tabs.iter().find(|t| Some(t.id) == g.active).cloned())
            .collect();
        for tab in &active {
            self.body(tab, window, cx);
        }
    }

    /// Replaces how tab bodies are made; existing bodies are rebuilt.
    pub fn set_tab_body_factory(&mut self, make_body: TabBodyFactory, cx: &mut Context<Self>) {
        self.make_body = make_body;
        self.bodies.clear();
        cx.notify();
    }

    /// Strikes through the titles of tabs whose file `documents` reports
    /// deleted.
    pub(crate) fn set_documents(&mut self, documents: Entity<Documents>) {
        self.documents = Some(documents);
    }

    /// The window's settings.
    fn settings_view<'a>(&'a self, cx: &'a App) -> SettingsView<'a> {
        self.settings.read(cx).view(cx)
    }

    /// The Search tab's title: `title` when `Some`, else "Search".
    pub fn set_search_title(&mut self, title: Option<SharedString>, cx: &mut Context<Self>) {
        if self.search_title != title {
            self.search_title = title;
            cx.notify();
        }
    }

    /// The title [`EditorGroupsView::set_search_title`] set; `None` while the
    /// Search tab says "Search".
    pub fn search_title(&self) -> Option<&SharedString> {
        self.search_title.as_ref()
    }

    /// The References tab's title: `title` when `Some`, else "References".
    pub fn set_references_title(&mut self, title: Option<SharedString>, cx: &mut Context<Self>) {
        if self.references_title != title {
            self.references_title = title;
            cx.notify();
        }
    }

    /// The title [`EditorGroupsView::set_references_title`] set.
    pub fn references_title(&self) -> Option<&SharedString> {
        self.references_title.as_ref()
    }

    /// The Changes since tab's title, naming its ref.
    pub fn set_branch_title(&mut self, title: Option<SharedString>, cx: &mut Context<Self>) {
        if self.branch_title != title {
            self.branch_title = title;
            cx.notify();
        }
    }

    /// The title [`EditorGroupsView::set_branch_title`] set.
    pub fn branch_title(&self) -> Option<&SharedString> {
        self.branch_title.as_ref()
    }

    /// Path bars show paths relative to `root`.
    pub(crate) fn set_root(&mut self, root: PathBuf) {
        self.root = Some(root);
        self.home = std::env::home_dir();
    }

    /// What the path bar of group `group` shows: its active tab's path,
    /// or the file a preview shows (see [`path_bar_text`]); `None` when the
    /// group has no tab or shows the Extensions page.
    pub fn path_bar_text(&self, group: usize) -> Option<String> {
        let group = self.model.groups().get(group)?;
        let tab = group.tabs.iter().find(|t| Some(t.id) == group.active)?;
        Some(path_bar_text(
            tab.path().or(tab.previewed_path())?,
            self.root.as_deref(),
            self.home.as_deref(),
        ))
    }

    /// Sets the breadcrumbs of the file tabs in `rows`. Other tabs keep
    /// theirs, shown again as they were when they become active.
    pub(crate) fn set_breadcrumbs(&mut self, rows: HashMap<TabId, Crumbs>, cx: &mut Context<Self>) {
        let mut changed = false;
        for (id, row) in rows {
            if self.breadcrumbs.get(&id) != Some(&row) {
                self.breadcrumbs.insert(id, row);
                changed = true;
            }
        }
        if changed {
            cx.notify();
        }
    }

    /// Forgets every tab's breadcrumbs.
    pub(crate) fn clear_breadcrumbs(&mut self, cx: &mut Context<Self>) {
        if !self.breadcrumbs.is_empty() {
            self.breadcrumbs.clear();
            cx.notify();
        }
    }

    pub(crate) fn set_on_breadcrumbs_click(&mut self, on_click: BreadcrumbsClick) {
        self.on_breadcrumbs_click = Some(on_click);
    }

    /// How the path bar buttons run their commands.
    pub(crate) fn set_on_command(&mut self, run: RunCommand) {
        self.on_command = Some(run);
    }

    /// The button at the right end of group `gi`'s path bar when its
    /// active tab is a markdown file (spec §4.2): on the source, the eye
    /// opens the preview to the side (Alt: in place); on a preview, the
    /// code icon goes back to the source. Tooltips name the user's keys.
    fn markdown_button(&self, gi: usize, tab: &Tab, cx: &mut Context<Self>) -> Option<Div> {
        let source = match &tab.item {
            TabItem::File(path) => {
                let root = self.root.as_deref().unwrap_or(Path::new(""));
                if !markdown_preview::is_markdown(root, path, cx) {
                    return None;
                }
                true
            }
            TabItem::MarkdownPreview(_) | TabItem::MarkdownPreviewDynamic(_) => false,
            _ => return None,
        };
        let (icon, title, key_command) = if source {
            (icons::EYE, "Open Preview to the Side", SHOW_PREVIEW_TO_SIDE)
        } else {
            (icons::FILE_CODE, "Show Source", TOGGLE_PREVIEW)
        };
        let tip = match cx.global::<AppState>().keymap.label_for(key_command) {
            Some(keys) => format!("{title} ({keys})"),
            None => title.to_string(),
        };
        let id = format!("md-preview-toggle-{gi}");
        let button = icon_button(SharedString::from(id.clone()), icon, false, cx)
            .debug_selector(move || id.clone())
            .tooltip(tooltip(tip))
            // Focus stays in the editor.
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
            .on_click(cx.listener(move |view, ev: &ClickEvent, window, cx| {
                let command = match (source, ev.modifiers().alt) {
                    (true, false) => SHOW_PREVIEW_TO_SIDE,
                    _ => TOGGLE_PREVIEW,
                };
                if let Some(run) = view.on_command.clone() {
                    window.defer(cx, move |window, cx| run(command, window, cx));
                }
            }));
        Some(div().flex_none().px(px(GRID)).child(button))
    }

    /// The breadcrumbs of group `group` when its active tab is a file and
    /// they are enabled; the path alone until the workbench computes them.
    fn crumbs(&self, group: usize, settings: &BreadcrumbSettings, cx: &App) -> Option<Crumbs> {
        if !settings.enabled {
            return None;
        }
        let g = self.model.groups().get(group)?;
        let tab = g.tabs.iter().find(|t| Some(t.id) == g.active)?;
        let TabItem::File(_) = tab.item else {
            return None;
        };
        if let Some(crumbs) = self.breadcrumbs.get(&tab.id) {
            return Some(crumbs.clone());
        }
        let text = self.path_bar_text(group)?;
        Some(Crumbs {
            segments: breadcrumb_segments(&text, &[], settings.file_path, settings.symbol_path),
            font: crate::fonts::editor_font_family(self.settings_view(cx), None),
        })
    }

    /// What the bar under the tab bar of group `group` shows, as text:
    /// the breadcrumbs joined by ` › `, or the plain path (see
    /// [`EditorGroupsView::path_bar_text`]) when they are disabled or the
    /// tab is not a file.
    pub fn breadcrumbs_text(&self, group: usize, cx: &App) -> Option<String> {
        match self.crumbs(group, &BreadcrumbSettings::read(self.settings_view(cx)), cx) {
            Some(crumbs) => Some(segments_text(&crumbs.segments)),
            None => self.path_bar_text(group),
        }
    }

    /// The title colour of tab `tab` in group `group` under `ui` (see
    /// [`title_color`]).
    pub fn tab_title_color_in(&self, group: usize, tab: usize, ui: &UiColors) -> Option<Rgba> {
        let g = self.model.groups().get(group)?;
        let t = g.tabs.get(tab)?;
        let active = g.active == Some(t.id);
        Some(title_color(active, group == self.model.active_group(), ui))
    }

    /// Moves the divider right of group `index` to `fraction` of the area,
    /// keeping both neighbours at least `min` wide.
    fn resize(&mut self, index: usize, fraction: f32, min: f32, cx: &mut Context<Self>) {
        if index + 1 >= self.widths.len() {
            return;
        }
        let left: f32 = self.widths[..index].iter().sum();
        let pair = self.widths[index] + self.widths[index + 1];
        let min = min.min(pair / 2.);
        let width = (fraction - left).clamp(min, pair - min);
        self.widths[index] = width;
        self.widths[index + 1] = pair - width;
        cx.notify();
    }

    fn set_hovered(&mut self, tab: TabId, hovered: bool, cx: &mut Context<Self>) {
        if hovered {
            self.hovered_tab = Some(tab);
        } else if self.hovered_tab == Some(tab) {
            self.hovered_tab = None;
        }
        cx.notify();
    }

    fn body(&mut self, tab: &Tab, window: &mut Window, cx: &mut App) -> AnyView {
        let make_body = Rc::clone(&self.make_body);
        self.bodies
            .entry(tab.id)
            .or_insert_with(|| make_body(tab, window, cx))
            .clone()
    }
}

impl Render for EditorGroupsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<AppState>().theme.clone();
        let ui = &theme.ui;

        let live: Vec<TabId> = self
            .model
            .groups()
            .iter()
            .flat_map(|g| g.tabs.iter().map(|t| t.id))
            .collect();
        self.bodies.retain(|id, _| live.contains(id));
        let active_tabs: Vec<Option<Tab>> = self
            .model
            .groups()
            .iter()
            .map(|g| g.tabs.iter().find(|t| Some(t.id) == g.active).cloned())
            .collect();
        let bodies: Vec<Option<AnyView>> = active_tabs
            .iter()
            .map(|tab| tab.as_ref().map(|t| self.body(t, window, cx)))
            .collect();

        let states: TabStates = self
            .model
            .groups()
            .iter()
            .map(|g| {
                g.tabs
                    .iter()
                    .map(|t| {
                        let Some((path, docs)) = t.path().zip(self.documents.as_ref()) else {
                            return (false, None);
                        };
                        let docs = docs.read(cx);
                        let deleted = docs.state(path) == Some(&DocState::Deleted);
                        let mark = if docs.has_conflict(path) {
                            Some(TabMark::Conflict)
                        } else if docs.is_dirty(path) {
                            Some(TabMark::Dirty)
                        } else {
                            None
                        };
                        (deleted, mark)
                    })
                    .collect()
            })
            .collect();
        let count = self.model.groups().len();
        let focused_group = self.model.active_group();
        let crumbs_settings = BreadcrumbSettings::read(self.settings_view(cx));
        let show_icons = show_tab_icons(self.settings_view(cx));
        let appearance = cx.global::<AppState>().theme.appearance;
        self.breadcrumbs.retain(|id, _| live.contains(id));
        let mut row = div()
            .id("editor-groups")
            .debug_selector(|| "editor-groups".into())
            .flex()
            .flex_row()
            .size_full()
            .on_drag_move(cx.listener(|view, ev: &DragMoveEvent<GroupSash>, _, cx| {
                let index = ev.drag(cx).0;
                let width = ev.bounds.size.width;
                let fraction = (ev.event.position.x - ev.bounds.left()) / width;
                view.resize(index, fraction, MIN_GROUP_WIDTH / width, cx);
            }));

        for (gi, (group, body)) in self.model.groups().iter().zip(bodies).enumerate() {
            let tabs = group.tabs.iter().enumerate().map(|(ti, tab)| {
                let id = tab.id;
                let active = group.active == Some(id);
                let fg = title_color(active, gi == focused_group, ui);
                let title = match &tab.item {
                    TabItem::Search => self.search_title.as_ref(),
                    TabItem::References => self.references_title.as_ref(),
                    TabItem::BranchChanges => self.branch_title.as_ref(),
                    _ => None,
                };
                let title = title.map_or_else(|| tab.title(), |t| t.to_string());
                let icon = show_icons
                    .then(|| tab_icon_source(&tab.item, cx))
                    .flatten()
                    .map(|src| {
                        div()
                            .debug_selector(move || format!("tab-icon-{gi}-{ti}"))
                            .flex_none()
                            .child(ui::file_icon_sized(
                                &src,
                                px(14.),
                                color(ui.icon_muted).into(),
                                appearance,
                            ))
                    });
                let (deleted, mark) = states[gi][ti];
                // As in Zed: a dot takes the ×'s place until the tab is
                // hovered.
                let hovered = self.hovered_tab == Some(id);
                let show_close = hovered || (active && mark.is_none());
                let dot = mark.filter(|_| !hovered).map(|mark| {
                    let (fill, kind) = match mark {
                        TabMark::Dirty => (ui.text_accent, "dirty"),
                        TabMark::Conflict => (ui.warning, "conflict"),
                    };
                    (fill, format!("tab-{kind}-{gi}-{ti}"))
                });
                let close = show_close.then(|| {
                    let on_click = cx.listener(move |view, _: &ClickEvent, window, cx| {
                        view.request_close(id, window, cx);
                    });
                    ui::TabClose {
                        id: ("tab-close", id.as_u64()).into(),
                        selector: format!("tab-close-{gi}-{ti}"),
                        on_click: Box::new(on_click),
                    }
                });
                ui::tab(
                    ui::TabProps {
                        id: ("tab", id.as_u64()).into(),
                        selector: format!("tab-{gi}-{ti}"),
                        active,
                        title: title.into(),
                        title_color: fg,
                        icon: icon.map(IntoElement::into_any_element),
                        italic: !tab.pinned,
                        strike: deleted.then(|| format!("tab-deleted-{gi}-{ti}")),
                        close,
                        dot,
                    },
                    cx,
                )
                .on_hover(cx.listener(move |view, hovered: &bool, _, cx| {
                    view.set_hovered(id, *hovered, cx);
                }))
                .on_click(cx.listener(move |view, ev: &ClickEvent, _, cx| {
                    view.update_model(cx, |m| {
                        m.activate(id);
                        // VS Code: a double click keeps a preview open.
                        if ev.click_count() >= 2 {
                            m.pin(id);
                        }
                    });
                }))
                .on_mouse_up(
                    MouseButton::Middle,
                    cx.listener(move |view, _, window, cx| {
                        view.request_close(id, window, cx);
                    }),
                )
            });
            let tab_bar = ui::tab_bar(("tab-bar", gi), cx)
                .w_full()
                .children(tabs)
                .child(ui::tab_bar_filler(cx));
            let bar = || {
                div()
                    .debug_selector(move || format!("path-bar-{gi}"))
                    .flex()
                    .flex_none()
                    .items_center()
                    .w_full()
                    .h(px(PATH_BAR_HEIGHT))
                    .bg(color(ui.editor_background))
                    .border_b_1()
                    .border_color(color(ui.border_variant))
                    .text_size(px(UI_FONT_SIZE_SMALL))
                    .text_color(color(ui.text_muted))
            };
            let md_button = active_tabs[gi]
                .as_ref()
                .and_then(|tab| self.markdown_button(gi, tab, cx));
            let path_bar = match self.crumbs(gi, &crumbs_settings, cx) {
                Some(crumbs) => {
                    let row = breadcrumbs_row(gi, &crumbs, crumbs_settings.icons, ui).on_click(
                        cx.listener(move |view, _: &ClickEvent, window, cx| {
                            if let Some(on_click) = view.on_breadcrumbs_click.clone() {
                                window.defer(cx, move |window, cx| on_click(gi, window, cx));
                            }
                        }),
                    );
                    Some(bar().child(row))
                }
                None => self.path_bar_text(gi).map(|text| {
                    // The Markdown button, when there is one, ends the row
                    // with its own padding.
                    let bar = bar().pl(px(TAB_PADDING_X));
                    let bar = if md_button.is_none() {
                        bar.pr(px(TAB_PADDING_X))
                    } else {
                        bar
                    };
                    bar.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(text),
                    )
                }),
            };
            let path_bar = path_bar.map(|bar| bar.children(md_button));
            let disk_bar = active_tabs[gi]
                .as_ref()
                .and_then(|tab| self.disk_bar(gi, tab, cx));
            let body = match body {
                Some(body) => body.into_any_element(),
                None => empty_hint(ui, self.settings_view(cx), cx),
            };
            let sash = (gi + 1 < count).then(|| {
                div()
                    .id(("group-sash", gi))
                    .debug_selector(move || format!("group-sash-{gi}"))
                    .absolute()
                    .top_0()
                    .right_0()
                    .w(SASH_WIDTH)
                    .h_full()
                    .cursor_col_resize()
                    // Over the scrollbar strip of the content on this edge: the
                    // sash wins on its own few px.
                    .occlude()
                    .on_drag(GroupSash(gi), |_, _, _, cx: &mut App| cx.new(|_| SashGhost))
            });
            row = row.child(
                div()
                    .id(("editor-group", gi))
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .h_full()
                    .min_w_0()
                    .w(relative(self.widths.get(gi).copied().unwrap_or(0.)))
                    .when(gi > 0, |d| {
                        d.border_l_1().border_color(color(ui.border_variant))
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |view, _, _, cx| {
                            view.update_model(cx, |m| m.focus_group(gi));
                        }),
                    )
                    .child(tab_bar)
                    .children(path_bar)
                    .children(disk_bar)
                    .child(div().flex_1().min_h_0().overflow_hidden().child(body))
                    .children(sash),
            );
        }
        row
    }
}

/// A tab title's colour: `text` on the focused group's active tab, halfway
/// between `text` and `text.muted` on another group's active tab, and
/// `text.muted` on inactive tabs.
fn title_color(active: bool, focused_group: bool, ui: &UiColors) -> Rgba {
    match (active, focused_group) {
        (true, true) => ui.text,
        (true, false) => mix(ui.text, ui.text_muted, 0.5),
        (false, _) => ui.text_muted,
    }
}

/// `a` moved `t` of the way towards `b`.
fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let lerp = |x: f32, y: f32| x + (y - x) * t;
    Rgba {
        r: lerp(a.r, b.r),
        g: lerp(a.g, b.g),
        b: lerp(a.b, b.b),
        a: lerp(a.a, b.a),
    }
}

/// `path` as a path bar shows it: relative to `root` inside it, otherwise
/// (or with no root) absolute, with `~` for the `home` folder.
pub(crate) fn path_bar_text(path: &Path, root: Option<&Path>, home: Option<&Path>) -> String {
    if let Some(rel) = root.and_then(|root| path.strip_prefix(root).ok())
        && !rel.as_os_str().is_empty()
    {
        return rel.display().to_string();
    }
    crate::workspace_state::tilde_path(path, home)
}

/// Commands listed in the empty editor area, in display order.
const WATERMARK_COMMANDS: [(&str, &str); 5] = [
    ("Show All Commands", "workbench.action.showCommands"),
    ("Go to File", "workbench.action.quickOpen"),
    ("Find in Files", "workbench.action.findInFiles"),
    ("Toggle Sidebar", "workbench.action.toggleSidebarVisibility"),
    ("Open Recent", "workbench.action.openRecent"),
];

/// Shown in a group with no tabs: a two-column list of commands with the
/// user's current shortcuts (labels right-aligned, keys left-aligned).
/// Commands without a key are skipped. `workbench.tips.enabled = false`
/// leaves just the area background.
fn empty_hint(ui: &UiColors, settings: SettingsView<'_>, cx: &App) -> gpui::AnyElement {
    let state = cx.global::<AppState>();
    let enabled = settings
        .get::<bool>("workbench.tips.enabled", None)
        .unwrap_or(true);
    if !enabled {
        return div().size_full().into_any_element();
    }
    let rows: Vec<_> = WATERMARK_COMMANDS
        .into_iter()
        .filter_map(|(label, command)| Some((label, command, state.keymap.label_for(command)?)))
        .collect();
    let labels = rows
        .iter()
        .map(|(label, _, _)| div().flex().justify_end().whitespace_nowrap().child(*label));
    let keys = rows.iter().map(|(_, command, keys)| {
        let command = *command;
        div()
            .debug_selector(move || format!("editor-empty-hint-{command}"))
            .whitespace_nowrap()
            .child(keys.clone())
    });
    div()
        .debug_selector(|| "editor-empty-hint".into())
        .flex()
        .flex_row()
        .size_full()
        .justify_center()
        .items_center()
        .gap(px(3. * GRID))
        .text_size(px(UI_FONT_SIZE))
        .text_color(color(ui.text_muted))
        .child(div().flex().flex_col().gap(px(GRID)).children(labels))
        .child(div().flex().flex_col().gap(px(GRID)).children(keys))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lucerna_theme::Appearance;

    #[test]
    fn path_bar_shows_paths_relative_to_the_root() {
        let root = Path::new("/work/lucerna");
        let home = Some(Path::new("/Users/me"));
        assert_eq!(
            path_bar_text(
                Path::new("/work/lucerna/crates/ui/src/lib.rs"),
                Some(root),
                home
            ),
            "crates/ui/src/lib.rs"
        );
        assert_eq!(
            path_bar_text(Path::new("/work/lucerna/README.md"), Some(root), home),
            "README.md"
        );
    }

    #[test]
    fn path_bar_shows_outside_paths_absolute_with_tilde_for_home() {
        let root = Path::new("/work/lucerna");
        let home = Some(Path::new("/Users/me"));
        assert_eq!(
            path_bar_text(Path::new("/etc/hosts"), Some(root), home),
            "/etc/hosts"
        );
        assert_eq!(
            path_bar_text(
                Path::new("/Users/me/.config/lucerna/settings.json"),
                Some(root),
                home
            ),
            "~/.config/lucerna/settings.json"
        );
        // A sibling whose name only starts with the home folder's.
        assert_eq!(
            path_bar_text(Path::new("/Users/meg/a.txt"), Some(root), home),
            "/Users/meg/a.txt"
        );
        assert_eq!(
            path_bar_text(Path::new("/etc/hosts"), Some(root), None),
            "/etc/hosts"
        );
    }

    #[test]
    fn path_bar_without_a_root_strips_nothing() {
        let home = Some(Path::new("/Users/me"));
        assert_eq!(
            path_bar_text(Path::new("/work/lucerna/src/main.rs"), None, home),
            "/work/lucerna/src/main.rs"
        );
        assert_eq!(
            path_bar_text(Path::new("/Users/me/notes.md"), None, home),
            "~/notes.md"
        );
    }

    #[test]
    fn inactive_tab_titles_are_muted() {
        let ui = UiColors::fallback(Appearance::Dark);
        assert_ne!(ui.text, ui.text_muted);
        assert_eq!(title_color(true, true, &ui), ui.text);
        assert_eq!(title_color(false, true, &ui), ui.text_muted);
        assert_eq!(title_color(false, false, &ui), ui.text_muted);
        assert_eq!(
            title_color(true, false, &ui),
            mix(ui.text, ui.text_muted, 0.5)
        );
    }
}
