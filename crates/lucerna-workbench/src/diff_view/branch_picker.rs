//! The branch picker of the Changes since tab: the local and remote
//! branches (`git for-each-ref`) in Quick Open's list, fuzzy-filtered,
//! with the ref the tab compares with selected. Enter or a click picks
//! the row; the tab then compares with it and the workspace keeps it.
//! The workbench hosts it as a modal overlay, like the Open Recent picker.

use gpui::{
    AnyElement, App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable,
    IntoElement, KeyDownEvent, ParentElement, Render, ScrollHandle, Styled, Subscription, Window,
    div, prelude::*, px, svg,
};
use lucerna_git::GitError;

use crate::icons;
use crate::layout::color;
use crate::quick_open::picker::{
    fuzzy_filter, highlighted, picker_input_row, picker_list, picker_row, wrap_index,
};
use crate::tasks::guarded;
use crate::text_input::TextInput;
use crate::ui::{FILE_ICON_SIZE, GRID, UI_FONT_SIZE, popover};
use crate::{AppState, Workbench};

const PLACEHOLDER: &str = "Select a branch to compare with";

/// What the host does as the picker is used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchPickerEvent {
    /// Enter or a click chose this ref.
    Pick(String),
    Dismiss,
}

pub struct BranchPicker {
    input: Entity<TextInput>,
    branches: Vec<String>,
    /// Matching branches: index into `branches` and matched char positions.
    rows: Vec<(usize, Vec<u32>)>,
    selected: usize,
    scroll: ScrollHandle,
}

impl EventEmitter<BranchPickerEvent> for BranchPicker {}

impl BranchPicker {
    /// Lists `branches` with `current` selected.
    pub fn new(branches: Vec<String>, current: Option<&str>, cx: &mut Context<Self>) -> Self {
        let this = cx.weak_entity();
        let input = cx.new(|cx| {
            let mut input = TextInput::new(cx);
            input.set_placeholder(PLACEHOLDER);
            input.on_change(move |text, _, cx| {
                this.update(cx, |picker, cx| picker.query_changed(text, cx))
                    .ok();
            });
            input
        });
        let rows = fuzzy_filter(branches.iter().map(String::as_str), "");
        let selected = current
            .and_then(|c| branches.iter().position(|b| b == c))
            .unwrap_or(0);
        let scroll = ScrollHandle::new();
        scroll.scroll_to_item(selected);
        BranchPicker {
            input,
            branches,
            rows,
            selected,
            scroll,
        }
    }

    /// The refs listed, in order.
    pub fn row_texts(&self) -> Vec<String> {
        self.rows
            .iter()
            .map(|(i, _)| self.branches[*i].clone())
            .collect()
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    fn query_changed(&mut self, text: &str, cx: &mut Context<Self>) {
        self.rows = fuzzy_filter(self.branches.iter().map(String::as_str), text.trim());
        self.select(0, cx);
    }

    fn select(&mut self, row: usize, cx: &mut Context<Self>) {
        self.selected = row;
        self.scroll.scroll_to_item(row);
        cx.notify();
    }

    fn pick(&mut self, row: usize, cx: &mut Context<Self>) {
        if let Some(&(index, _)) = self.rows.get(row) {
            cx.emit(BranchPickerEvent::Pick(self.branches[index].clone()));
        }
    }

    fn key_down(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let m = &ev.keystroke.modifiers;
        if m.control || m.alt || m.shift || m.platform {
            return;
        }
        match ev.keystroke.key.as_str() {
            "up" => self.select(wrap_index(self.selected, self.rows.len(), -1), cx),
            "down" => self.select(wrap_index(self.selected, self.rows.len(), 1), cx),
            "enter" => self.pick(self.selected, cx),
            // Unless a key binding closed the picker first.
            "escape" => cx.emit(BranchPickerEvent::Dismiss),
            _ => return,
        }
        cx.stop_propagation();
    }

    fn render_row(&self, row: usize, cx: &mut Context<Self>) -> AnyElement {
        let (index, positions) = &self.rows[row];
        let ui = &cx.global::<AppState>().theme.ui;
        picker_row("branch-picker-row", row, row == self.selected, cx)
            .gap(px(2. * GRID))
            .on_click(cx.listener(move |picker, _, _, cx| picker.pick(row, cx)))
            .child(
                svg()
                    .data(icons::GIT_BRANCH)
                    .size(px(FILE_ICON_SIZE))
                    .flex_none()
                    .text_color(color(ui.icon_muted)),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(UI_FONT_SIZE))
                    .child(highlighted(
                        self.branches[*index].clone().into(),
                        positions,
                        cx,
                    )),
            )
            .into_any_element()
    }
}

impl Focusable for BranchPicker {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl Render for BranchPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows: Vec<AnyElement> = (0..self.rows.len())
            .map(|row| self.render_row(row, cx))
            .collect();
        let ui = &cx.global::<AppState>().theme.ui;
        let empty = rows.is_empty().then(|| {
            let message = if self.branches.is_empty() {
                "No branches"
            } else {
                "No matching branches"
            };
            div()
                .debug_selector(|| "branch-picker-empty".to_string())
                .px(px(3. * GRID))
                .py(px(3. * GRID))
                .text_size(px(UI_FONT_SIZE))
                .text_color(color(ui.text_muted))
                .child(message)
        });
        popover(cx)
            .id("branch-picker")
            .occlude()
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(BranchPickerEvent::Dismiss)))
            .child(picker_input_row(&self.input, cx))
            .when(!rows.is_empty(), |p| {
                p.child(picker_list("branch-picker-list", &self.scroll).children(rows))
            })
            .children(empty)
    }
}

/// The shown branch picker.
pub(crate) struct ShownBranchPicker {
    picker: Entity<BranchPicker>,
    _events: Subscription,
    focus_before: Option<FocusHandle>,
}

impl Workbench {
    /// The shown branch picker, if any.
    pub fn branch_picker(&self) -> Option<&Entity<BranchPicker>> {
        self.branch_picker.as_ref().map(|shown| &shown.picker)
    }

    /// Shows the branch picker over the repository's branches, once listed
    /// in the background. Nothing happens without a repository.
    pub fn show_branch_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.branch_picker.is_some() || self.branch_picker_loading.is_some() {
            return;
        }
        let Some(repo) = self.git.read(cx).repo() else {
            return;
        };
        self.close_modal_overlays(window, cx);
        let load = cx.spawn_in(window, async move |wb, cx| {
            let branches = cx
                .background_spawn(async move {
                    guarded("git branches", || repo.branches())
                        .unwrap_or_else(|| Err(GitError::Parse("panicked".into())))
                })
                .await
                .unwrap_or_else(|err| {
                    tracing::warn!(%err, "could not list the branches");
                    Vec::new()
                });
            wb.update_in(cx, |wb, window, cx| {
                wb.branch_picker_loading = None;
                wb.open_branch_picker(branches, window, cx);
            })
            .ok();
        });
        self.branch_picker_loading = Some(load);
    }

    fn open_branch_picker(
        &mut self,
        branches: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Another overlay opened while the list loaded.
        if self.other_modal_overlay_shown() || self.locations_picker.is_some() {
            return;
        }
        let current = self.branch_diff_ref(cx);
        let picker = cx.new(|cx| BranchPicker::new(branches, current.as_deref(), cx));
        let events = cx.subscribe_in(&picker, window, Self::on_branch_picker_event);
        let focus_before = window.focused(cx);
        window.focus(&picker.focus_handle(cx), cx);
        self.branch_picker = Some(ShownBranchPicker {
            picker,
            _events: events,
            focus_before,
        });
        cx.notify();
    }

    /// Closes the branch picker (or stops it from showing) and gives the
    /// focus back.
    pub(crate) fn close_branch_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.branch_picker_loading = None;
        let Some(shown) = self.branch_picker.take() else {
            return;
        };
        let focused = window.focused(cx);
        if focused.is_none() || shown.picker.focus_handle(cx).contains_focused(window, cx) {
            match shown.focus_before {
                Some(focus) => window.focus(&focus, cx),
                None => self.focus_editor_area(window, cx),
            }
        }
        cx.notify();
    }

    fn on_branch_picker_event(
        &mut self,
        _: &Entity<BranchPicker>,
        event: &BranchPickerEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            BranchPickerEvent::Pick(name) => {
                self.close_branch_picker(window, cx);
                let name = name.clone();
                self.branch_base.update(cx, |base, cx| {
                    base.0 = Some(name);
                    cx.notify();
                });
            }
            BranchPickerEvent::Dismiss => self.close_branch_picker(window, cx),
        }
    }
}
