//! Renders the editor groups side by side, each with a tab bar over the
//! active tab's body, separated by draggable dividers.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{
    AnyView, App, ClickEvent, Context, DragMoveEvent, Entity, IntoElement, MouseButton,
    ParentElement, Pixels, Render, SharedString, Styled, Window, div, prelude::*, px, relative,
    svg,
};
use lucerna_theme::{Rgba, UiColors};

use super::model::{EditorGroups, GroupsState, Tab, TabId, TabItem};
use crate::AppState;
use crate::documents::{DocState, Documents};
use crate::icons;
use crate::layout::color;
use crate::layout::sidebar::SashGhost;
use crate::symbols::breadcrumbs::{
    BreadcrumbSettings, Crumbs, breadcrumb_segments, breadcrumbs_row, segments_text,
};
use crate::ui::{
    BLOCKS_ICON, CLOSE_ICON, GRID, PATH_BAR_HEIGHT, SEARCH_ICON, TAB_HEIGHT, TAB_PADDING_X,
    UI_FONT_SIZE_SMALL,
};

/// Square slot that holds a tab's close button, kept even while the
/// button is hidden so tabs do not change width on hover.
const CLOSE_SLOT: Pixels = px(16.);
const SASH_WIDTH: Pixels = px(4.);
const MIN_GROUP_WIDTH: Pixels = px(120.);

/// Produces the view shown under the tab bar for a tab. It is called once
/// per tab; the view is kept until the tab closes.
pub type TabBodyFactory = Rc<dyn Fn(&Tab, &mut Window, &mut App) -> AnyView>;

/// Called with a group's index when its breadcrumbs row is clicked.
pub(crate) type BreadcrumbsClick = Rc<dyn Fn(usize, &mut Window, &mut App)>;

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
    /// The workspace root: path bars show paths relative to it.
    root: Option<PathBuf>,
    /// The user's home folder, shown as `~` in path bars; read when the
    /// view is made and again when its root is set.
    home: Option<PathBuf>,
    /// Shown as the Search tab's title in place of "Search".
    search_title: Option<SharedString>,
    /// Shown as the References tab's title in place of "References".
    references_title: Option<SharedString>,
    /// The breadcrumbs of file tabs, as the workbench last computed them.
    breadcrumbs: HashMap<TabId, Crumbs>,
    on_breadcrumbs_click: Option<BreadcrumbsClick>,
}

impl Default for EditorGroupsView {
    fn default() -> Self {
        EditorGroupsView {
            model: EditorGroups::default(),
            widths: vec![1.0],
            bodies: HashMap::new(),
            make_body: Rc::new(placeholder_body),
            hovered_tab: None,
            documents: None,
            root: None,
            home: std::env::home_dir(),
            search_title: None,
            references_title: None,
            breadcrumbs: HashMap::new(),
            on_breadcrumbs_click: None,
        }
    }
}

impl EditorGroupsView {
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

    pub(crate) fn body_of(&self, id: TabId) -> Option<AnyView> {
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

    /// Path bars show paths relative to `root`.
    pub(crate) fn set_root(&mut self, root: PathBuf) {
        self.root = Some(root);
        self.home = std::env::home_dir();
    }

    /// What the path bar of group `group` shows: its active tab's path
    /// (see [`path_bar_text`]); `None` when the group has no tab or shows
    /// the Extensions page.
    pub fn path_bar_text(&self, group: usize) -> Option<String> {
        let group = self.model.groups().get(group)?;
        let tab = group.tabs.iter().find(|t| Some(t.id) == group.active)?;
        Some(path_bar_text(
            tab.path()?,
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
            font: crate::fonts::editor_font_family(None, cx),
        })
    }

    /// What the bar under the tab bar of group `group` shows, as text:
    /// the breadcrumbs joined by ` › `, or the plain path (see
    /// [`EditorGroupsView::path_bar_text`]) when they are disabled or the
    /// tab is not a file.
    pub fn breadcrumbs_text(&self, group: usize, cx: &App) -> Option<String> {
        match self.crumbs(group, &BreadcrumbSettings::read(cx), cx) {
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

        let deleted: Vec<Vec<bool>> = self
            .model
            .groups()
            .iter()
            .map(|g| {
                g.tabs
                    .iter()
                    .map(|t| {
                        let docs = self.documents.as_ref();
                        t.path().zip(docs).is_some_and(|(path, d)| {
                            d.read(cx).state(path) == Some(&DocState::Deleted)
                        })
                    })
                    .collect()
            })
            .collect();
        let count = self.model.groups().len();
        let focused_group = self.model.active_group();
        let crumbs_settings = BreadcrumbSettings::read(cx);
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
                let title = match (&tab.item, &self.search_title, &self.references_title) {
                    (TabItem::Search, Some(title), _) => title.to_string(),
                    (TabItem::References, _, Some(title)) => title.to_string(),
                    _ => tab.title(),
                };
                let icon = match tab.item {
                    TabItem::Extensions => Some(BLOCKS_ICON),
                    TabItem::Search => Some(SEARCH_ICON),
                    TabItem::LspLog(_) => Some(icons::ZAP),
                    TabItem::References => Some(icons::FILE_CODE),
                    TabItem::File(_) => None,
                };
                let icon = icon.map(|icon| {
                    svg()
                        .debug_selector(move || format!("tab-icon-{gi}-{ti}"))
                        .data(icon)
                        .flex_none()
                        .size(px(14.))
                        .text_color(color(ui.icon_muted))
                });
                let show_close = active || self.hovered_tab == Some(id);
                let deleted = deleted[gi][ti];
                let hover = color(ui.ghost_element_hover);
                let close = show_close.then(|| {
                    div()
                        .id(("tab-close", id.as_u64()))
                        .debug_selector(move || format!("tab-close-{gi}-{ti}"))
                        .flex()
                        .justify_center()
                        .items_center()
                        .size_full()
                        .rounded(px(GRID))
                        .cursor_pointer()
                        .hover(move |s| s.bg(hover))
                        .on_click(cx.listener(move |view, _, _, cx| {
                            cx.stop_propagation();
                            view.update_model(cx, |m| m.close(id));
                        }))
                        .child(
                            svg()
                                .data(CLOSE_ICON)
                                .size(px(10.))
                                .text_color(color(ui.icon_muted)),
                        )
                });
                div()
                    .id(("tab", id.as_u64()))
                    .debug_selector(move || format!("tab-{gi}-{ti}"))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(1.5 * GRID))
                    .px(px(TAB_PADDING_X))
                    .h_full()
                    .border_r_1()
                    .border_color(color(ui.border_variant))
                    // The bar's bottom border runs under every tab but the
                    // active one, which opens onto the editor below.
                    .when(!active, |d| d.border_b_1())
                    .bg(color(if active {
                        ui.editor_background
                    } else {
                        ui.tab_inactive_background
                    }))
                    .when(!active, |d| d.hover(move |s| s.bg(hover)))
                    .text_color(color(fg))
                    .text_size(px(UI_FONT_SIZE_SMALL))
                    .cursor_pointer()
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
                        cx.listener(move |view, _, _, cx| {
                            view.update_model(cx, |m| m.close(id));
                        }),
                    )
                    .children(icon)
                    .child(
                        div()
                            .when(!tab.pinned, |d| d.italic())
                            .when(deleted, |d| {
                                d.line_through()
                                    .debug_selector(move || format!("tab-deleted-{gi}-{ti}"))
                            })
                            .child(title),
                    )
                    .child(div().flex_none().size(CLOSE_SLOT).children(close))
            });
            let tab_bar = div()
                .id(("tab-bar", gi))
                .flex()
                .flex_row()
                .flex_none()
                .w_full()
                .h(px(TAB_HEIGHT))
                .overflow_x_scroll()
                .bg(color(ui.tab_bar_background))
                .children(tabs)
                .child(
                    div()
                        .flex_1()
                        .h_full()
                        .border_b_1()
                        .border_color(color(ui.border_variant)),
                );
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
                    bar().px(px(TAB_PADDING_X)).child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(text),
                    )
                }),
            };
            let body = match body {
                Some(body) => body.into_any_element(),
                None => empty_hint(ui, cx).into_any_element(),
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
    if let Some(rel) = home.and_then(|home| path.strip_prefix(home).ok()) {
        return Path::new("~").join(rel).display().to_string();
    }
    path.display().to_string()
}

/// Shown in a group with no tabs: the app name and the shortcuts to find
/// something to open, labelled from the user's keymap.
fn empty_hint(ui: &UiColors, cx: &App) -> impl IntoElement + use<> {
    let keymap = &cx.global::<AppState>().keymap;
    let lines = [
        ("Go to file", "workbench.action.quickOpen"),
        ("Find in project", "workbench.action.findInFiles"),
    ]
    .into_iter()
    .filter_map(|(label, command)| {
        let keys = keymap.label_for(command)?;
        Some(
            div()
                .flex()
                .flex_row()
                .gap(px(3. * GRID))
                .child(label)
                .child(keys),
        )
    });
    div()
        .debug_selector(|| "editor-empty-hint".into())
        .flex()
        .flex_col()
        .size_full()
        .justify_center()
        .items_center()
        .gap(px(GRID))
        .text_color(color(ui.text_muted))
        .child(div().mb(px(2. * GRID)).text_size(px(20.)).child("Lucerna"))
        .children(lines)
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
