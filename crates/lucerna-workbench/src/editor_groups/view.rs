//! Renders the editor groups side by side, each with a tab bar over the
//! active tab's body, separated by draggable dividers.

use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyView, App, Context, DragMoveEvent, IntoElement, MouseButton, ParentElement, Pixels, Render,
    Styled, Window, div, prelude::*, px, relative,
};

use super::model::{EditorGroups, Tab, TabId};
use crate::AppState;
use crate::layout::color;
use crate::layout::sidebar::SashGhost;

const TAB_BAR_HEIGHT: Pixels = px(35.);
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
}

impl Default for EditorGroupsView {
    fn default() -> Self {
        EditorGroupsView {
            model: EditorGroups::default(),
            widths: vec![1.0],
            bodies: HashMap::new(),
            make_body: Rc::new(placeholder_body),
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
        let n = self.model.groups().len();
        if self.widths.len() != n {
            self.widths = vec![1.0 / n as f32; n];
        }
        cx.notify();
        result
    }

    /// Fraction of the editor area's width each group takes, left to right.
    pub fn group_widths(&self) -> &[f32] {
        &self.widths
    }

    /// Replaces how tab bodies are made; existing bodies are rebuilt.
    pub fn set_tab_body_factory(&mut self, make_body: TabBodyFactory, cx: &mut Context<Self>) {
        self.make_body = make_body;
        self.bodies.clear();
        cx.notify();
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
                let close = div()
                    .id(("tab-close", ti))
                    .debug_selector(move || format!("tab-close-{gi}-{ti}"))
                    .px_1()
                    .rounded_sm()
                    .cursor_pointer()
                    .hover(|s| s.bg(color(ui.element_selected)))
                    .on_click(cx.listener(move |view, _, _, cx| {
                        cx.stop_propagation();
                        view.update_model(cx, |m| m.close(id));
                    }))
                    .child("×");
                div()
                    .id(("tab", ti))
                    .debug_selector(move || format!("tab-{gi}-{ti}"))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_2()
                    .pl_3()
                    .pr_1()
                    .h_full()
                    .border_r_1()
                    .border_color(color(ui.border))
                    .bg(color(if active {
                        ui.tab_active_background
                    } else {
                        ui.tab_inactive_background
                    }))
                    .text_color(color(fg))
                    .cursor_pointer()
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.update_model(cx, |m| m.activate(id));
                    }))
                    .on_mouse_up(
                        MouseButton::Middle,
                        cx.listener(move |view, _, _, cx| {
                            view.update_model(cx, |m| m.close(id));
                        }),
                    )
                    .child(div().when(!tab.pinned, |d| d.italic()).child(title))
                    .child(close)
            });
            let tab_bar = div()
                .id(("tab-bar", gi))
                .flex()
                .flex_row()
                .flex_none()
                .w_full()
                .h(TAB_BAR_HEIGHT)
                .overflow_x_scroll()
                .bg(color(ui.tab_inactive_background))
                .border_b_1()
                .border_color(color(ui.border))
                .children(tabs);
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
                    .when(gi > 0, |d| d.border_l_1().border_color(color(ui.border)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |view, _, _, cx| {
                            view.update_model(cx, |m| m.focus_group(gi));
                        }),
                    )
                    .child(tab_bar)
                    .child(div().flex_1().min_h_0().overflow_hidden().children(body))
                    .children(sash),
            );
        }
        row
    }
}
