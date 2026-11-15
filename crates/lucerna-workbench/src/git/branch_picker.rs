//! The branch picker, in two modes. Compare mode (the Changes since tab):
//! the local and remote branches in Quick Open's list, fuzzy-filtered,
//! with the ref the tab compares with selected; Enter or a click picks the
//! row. Checkout mode (the title bar branch button): the current branch
//! with a check, the other local branches, then the remote ones, each with
//! its last commit time, and a Create row for a name that is not a branch
//! yet. The workbench hosts it as a modal overlay, like the Open Recent
//! picker.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use gpui::{
    AnyElement, App, AppContext, Context, Div, Entity, EventEmitter, FocusHandle, Focusable,
    IntoElement, KeyDownEvent, ParentElement, Render, ScrollHandle, Styled, Subscription, Window,
    div, prelude::*, px, svg,
};
use lucerna_git::{BranchRef, GitError, SwitchTarget, sanitize_branch_name};

use crate::git::checkout::CheckoutAnchor;
use crate::icons;
use crate::layout::color;
use crate::quick_open::picker::{
    fuzzy_filter, highlighted, picker_input_row, picker_list, picker_row, wrap_index,
};
use crate::tasks::guarded;
use crate::text_input::TextInput;
use crate::ui::{
    FILE_ICON_SIZE, GRID, UI_FONT_SIZE, anchored_popover_overlay, popover, popover_overlay,
    relative_time,
};
use crate::{AppState, Workbench};

const COMPARE_PLACEHOLDER: &str = "Select a branch to compare with";
const CHECKOUT_PLACEHOLDER: &str = "Switch to a branch or type to create one";
const INVALID_NAME: &str = "Invalid branch name";

/// What the picker is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchPickerMode {
    /// Pick the ref the Changes since tab compares with.
    Compare,
    /// Pick the branch to switch to, or a name to create.
    Checkout,
}

/// What the host does as the picker is used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchPickerEvent {
    /// Compare mode: Enter or a click chose this ref.
    Pick(String),
    /// Checkout mode: Enter or a click chose this switch. Choosing the
    /// current branch is a `Dismiss`.
    Checkout(SwitchTarget),
    Dismiss,
}

/// Whether a branch name passes git's ref-format rules; a pure check, so
/// the picker never runs git.
pub type NameCheck = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// The Create row of Checkout mode.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CreateRow {
    /// The sanitized query.
    name: String,
    valid: bool,
}

pub struct BranchPicker {
    mode: BranchPickerMode,
    input: Entity<TextInput>,
    /// The names listed, parallel to `refs` in Checkout mode.
    branches: Vec<String>,
    /// Checkout mode: the branches, current first, then local, then remote.
    refs: Vec<BranchRef>,
    is_valid_name: NameCheck,
    /// Unix time the picker opened at, for the relative times.
    now: i64,
    /// Matching branches: index into `branches` and matched char positions.
    rows: Vec<(usize, Vec<u32>)>,
    /// Checkout mode: the row after `rows` for a name that is no branch yet.
    create: Option<CreateRow>,
    selected: usize,
    scroll: ScrollHandle,
}

impl EventEmitter<BranchPickerEvent> for BranchPicker {}

impl BranchPicker {
    /// Compare mode: lists `branches` with `current` selected.
    pub fn new(branches: Vec<String>, current: Option<&str>, cx: &mut Context<Self>) -> Self {
        let selected = current
            .and_then(|c| branches.iter().position(|b| b == c))
            .unwrap_or(0);
        let mut picker = Self::build(
            BranchPickerMode::Compare,
            branches,
            Vec::new(),
            Arc::new(|_| true),
            COMPARE_PLACEHOLDER,
            cx,
        );
        picker.select(selected, cx);
        picker
    }

    /// Checkout mode over `branches` (any order). `is_valid_name` is a pure
    /// check of a sanitized name, run on every keystroke.
    pub fn new_checkout(
        mut branches: Vec<BranchRef>,
        is_valid_name: NameCheck,
        cx: &mut Context<Self>,
    ) -> Self {
        // Current branch, other local branches, remote branches; newest
        // first in each group (the sort is stable).
        branches.sort_by_key(|b| {
            (
                !b.is_head,
                b.remote.is_some(),
                std::cmp::Reverse(b.committed),
            )
        });
        let names = branches.iter().map(|b| b.name.clone()).collect();
        Self::build(
            BranchPickerMode::Checkout,
            names,
            branches,
            is_valid_name,
            CHECKOUT_PLACEHOLDER,
            cx,
        )
    }

    fn build(
        mode: BranchPickerMode,
        branches: Vec<String>,
        refs: Vec<BranchRef>,
        is_valid_name: NameCheck,
        placeholder: &'static str,
        cx: &mut Context<Self>,
    ) -> Self {
        let this = cx.weak_entity();
        let input = cx.new(|cx| {
            let mut input = TextInput::new(cx);
            input.set_placeholder(placeholder);
            input.on_change(move |text, _, cx| {
                this.update(cx, |picker, cx| picker.query_changed(text, cx))
                    .ok();
            });
            input
        });
        let rows = fuzzy_filter(branches.iter().map(String::as_str), "");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        BranchPicker {
            mode,
            input,
            branches,
            refs,
            is_valid_name,
            now,
            rows,
            create: None,
            selected: 0,
            scroll: ScrollHandle::new(),
        }
    }

    /// The rows listed, in order: Compare mode the refs; Checkout mode
    /// `✓ main  2 days ago`, ... and the Create row last.
    pub fn row_texts(&self) -> Vec<String> {
        let mut texts: Vec<String> = self
            .rows
            .iter()
            .map(|(i, _)| match self.refs.get(*i) {
                Some(b) => format!(
                    "{}{}  {}",
                    if b.is_head { "✓ " } else { "" },
                    b.name,
                    relative_time(b.committed, self.now)
                ),
                None => self.branches[*i].clone(),
            })
            .collect();
        if let Some(create) = &self.create {
            let mut text = format!("+ Create branch \"{}\"", create.name);
            if !create.valid {
                text.push_str("  ");
                text.push_str(INVALID_NAME);
            }
            texts.push(text);
        }
        texts
    }

    pub fn mode(&self) -> BranchPickerMode {
        self.mode
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    /// The input, for tests.
    #[doc(hidden)]
    pub fn input(&self) -> &Entity<TextInput> {
        &self.input
    }

    /// Checkout mode: the Create row's name and whether it is a valid
    /// branch name.
    #[doc(hidden)]
    pub fn create_row(&self) -> Option<(String, bool)> {
        self.create.as_ref().map(|c| (c.name.clone(), c.valid))
    }

    fn row_count(&self) -> usize {
        self.rows.len() + usize::from(self.create.is_some())
    }

    fn query_changed(&mut self, text: &str, cx: &mut Context<Self>) {
        self.rows = fuzzy_filter(self.branches.iter().map(String::as_str), text.trim());
        self.create = self.create_for(text);
        self.select(0, cx);
    }

    /// The Create row for `query`, when it names no branch yet.
    fn create_for(&self, query: &str) -> Option<CreateRow> {
        if self.mode != BranchPickerMode::Checkout {
            return None;
        }
        let name = sanitize_branch_name(query);
        if name.is_empty() {
            return None;
        }
        let taken = self.refs.iter().any(|b| match b.remote {
            None => b.name == name,
            Some(_) => b.branch_part() == name,
        });
        if taken {
            return None;
        }
        let valid = (self.is_valid_name)(&name);
        Some(CreateRow { name, valid })
    }

    fn select(&mut self, row: usize, cx: &mut Context<Self>) {
        self.selected = row;
        self.scroll.scroll_to_item(row);
        cx.notify();
    }

    fn pick(&mut self, row: usize, cx: &mut Context<Self>) {
        if let Some(&(index, _)) = self.rows.get(row) {
            match self.refs.get(index) {
                None => cx.emit(BranchPickerEvent::Pick(self.branches[index].clone())),
                Some(b) if b.is_head => cx.emit(BranchPickerEvent::Dismiss),
                Some(b) => {
                    let target = match &b.remote {
                        None => SwitchTarget::Local(b.name.clone()),
                        Some(remote) => SwitchTarget::Remote {
                            remote: remote.clone(),
                            branch: b.branch_part().to_string(),
                        },
                    };
                    cx.emit(BranchPickerEvent::Checkout(target));
                }
            }
        } else if row == self.rows.len()
            && let Some(create) = &self.create
            && create.valid
        {
            cx.emit(BranchPickerEvent::Checkout(SwitchTarget::Create(
                create.name.clone(),
            )));
        }
    }

    fn key_down(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let m = &ev.keystroke.modifiers;
        if m.control || m.alt || m.shift || m.platform {
            return;
        }
        match ev.keystroke.key.as_str() {
            "up" => self.select(wrap_index(self.selected, self.row_count(), -1), cx),
            "down" => self.select(wrap_index(self.selected, self.row_count(), 1), cx),
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
        let branch = self.refs.get(*index);
        let mut el = picker_row("branch-picker-row", row, row == self.selected, cx)
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
            );
        if let Some(branch) = branch {
            if branch.is_head {
                el = el.child(
                    svg()
                        .data(icons::CHECK)
                        .size(px(FILE_ICON_SIZE))
                        .flex_none()
                        .text_color(color(ui.icon_muted)),
                );
            }
            el = el.child(div().flex_1()).child(
                div()
                    .flex_none()
                    .text_size(px(UI_FONT_SIZE))
                    .text_color(color(ui.text_muted))
                    .child(relative_time(branch.committed, self.now)),
            );
        }
        el.into_any_element()
    }

    fn render_create_row(&self, create: &CreateRow, cx: &mut Context<Self>) -> AnyElement {
        let row = self.rows.len();
        let ui = &cx.global::<AppState>().theme.ui;
        let text_color = if create.valid {
            ui.text
        } else {
            ui.text_disabled
        };
        let mut el = picker_row("branch-picker-row", row, row == self.selected, cx)
            .gap(px(2. * GRID))
            .on_click(cx.listener(move |picker, _, _, cx| picker.pick(row, cx)))
            .child(
                svg()
                    .data(icons::PLUS)
                    .size(px(FILE_ICON_SIZE))
                    .flex_none()
                    .text_color(color(ui.icon_muted)),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(UI_FONT_SIZE))
                    .text_color(color(text_color))
                    .child(format!("Create branch \"{}\"", create.name)),
            );
        if !create.valid {
            el = el.child(div().flex_1()).child(
                div()
                    .flex_none()
                    .text_size(px(UI_FONT_SIZE))
                    .text_color(color(ui.text_muted))
                    .child(INVALID_NAME),
            );
        }
        el.into_any_element()
    }
}

impl Focusable for BranchPicker {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl Render for BranchPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut rows: Vec<AnyElement> = (0..self.rows.len())
            .map(|row| self.render_row(row, cx))
            .collect();
        if let Some(create) = self.create.clone() {
            rows.push(self.render_create_row(&create, cx));
        }
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
            .debug_selector(|| "branch-picker".to_string())
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
    /// Where a Checkout-mode picker shows; `None` in Compare mode.
    pub(crate) anchor: Option<CheckoutAnchor>,
    _events: Subscription,
    focus_before: Option<FocusHandle>,
}

impl Workbench {
    /// The shown branch picker, if any.
    pub fn branch_picker(&self) -> Option<&Entity<BranchPicker>> {
        self.branch_picker.as_ref().map(|shown| &shown.picker)
    }

    /// The shown branch picker in its overlay: a Checkout-mode picker
    /// opened from the title bar under the branch button, any other at
    /// Quick Open's position.
    pub(crate) fn branch_picker_overlay(&self, window: &Window) -> Option<Div> {
        let shown = self.branch_picker.as_ref()?;
        let picker = shown.picker.clone();
        Some(match (shown.anchor, self.branch_button_bounds.get()) {
            (Some(CheckoutAnchor::TitleBar), Some(anchor)) => {
                anchored_popover_overlay(picker, anchor, window.viewport_size())
            }
            _ => popover_overlay(picker),
        })
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
        self.host_branch_picker(picker, None, window, cx);
    }

    /// Shows `picker` as the modal overlay and gives it the focus; with an
    /// `anchor`, a Checkout-mode picker.
    pub(crate) fn host_branch_picker(
        &mut self,
        picker: Entity<BranchPicker>,
        anchor: Option<CheckoutAnchor>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let events = cx.subscribe_in(&picker, window, Self::on_branch_picker_event);
        let focus_before = window.focused(cx);
        window.focus(&picker.focus_handle(cx), cx);
        self.branch_picker = Some(ShownBranchPicker {
            picker,
            anchor,
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
            BranchPickerEvent::Checkout(target) => {
                self.close_branch_picker(window, cx);
                self.switch_branch(target.clone(), window, cx);
            }
            BranchPickerEvent::Dismiss => self.close_branch_picker(window, cx),
        }
    }
}
