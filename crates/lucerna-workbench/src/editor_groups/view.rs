//! Renders the editor groups side by side, each with a tab bar over the
//! active tab's body, separated by draggable dividers.

use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyView, App, ClickEvent, Context, DragMoveEvent, Entity, IntoElement, MouseButton,
    ParentElement, Pixels, Render, Styled, Window, div, prelude::*, px, relative, svg,
};
use lucerna_theme::UiColors;

use super::model::{EditorGroups, GroupsState, Tab, TabId};
use crate::AppState;
use crate::documents::{DocState, Documents};
use crate::layout::color;
use crate::layout::sidebar::SashGhost;
use crate::ui::{CLOSE_ICON, GRID, TAB_HEIGHT};

/// Square slot that holds a tab's close button, kept even while the
/// button is hidden so tabs do not change width on hover.
const CLOSE_SLOT: Pixels = px(16.);
const SASH_WIDTH: Pixels = px(4.);
const MIN_GROUP_WIDTH: Pixels = px(120.);

/// Produces the view shown under the tab bar for a tab. It is called once
/// per tab; the view is kept until the tab closes.
pub type TabBodyFactory = Rc<dyn Fn(&Tab, &mut Window, &mut App) -> AnyView>;

/// The default tab body until the code view exists: the file's path.
pub fn placeholder_body(tab: &Tab, _: &mut Window, cx: &mut App) -> AnyView {
    let path = tab.path.clone();
    cx.new(|_| PathPlaceholder { path }).into()
}

struct PathPlaceholder {
    path: Arc<Path>,
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
            .child(self.path.display().to_string())
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
    /// [`EditorGroups::restore`]). Tab bodies are made again, through the
    /// factory, as they are drawn.
    pub fn restore(&mut self, state: &GroupsState, cx: &mut Context<Self>) {
        self.model = EditorGroups::restore(state);
        self.bodies.clear();
        self.hovered_tab = None;
        let n = self.model.groups().len();
        self.widths = vec![1.0 / n as f32; n];
        cx.notify();
    }

    /// The body of tab `id`, if it has been drawn.
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
                        self.documents
                            .as_ref()
                            .is_some_and(|d| d.read(cx).state(&t.path) == Some(&DocState::Deleted))
                    })
                    .collect()
            })
            .collect();
        let count = self.model.groups().len();
        let focused_group = self.model.active_group();
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
                let fg = if active && gi == focused_group {
                    ui.text
                } else {
                    ui.text_muted
                };
                let title = tab
                    .path
                    .file_name()
                    .unwrap_or(tab.path.as_os_str())
                    .to_string_lossy()
                    .into_owned();
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
                    .px(px(3. * GRID))
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
                    .child(div().flex_1().min_h_0().overflow_hidden().child(body))
                    .children(sash),
            );
        }
        row
    }
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
