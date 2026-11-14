//! Drawing a diff tab: the toolbar, the centered message and the list.

use gpui::{
    Context, IntoElement, KeyDownEvent, ParentElement, Render, Styled, Window, div, prelude::*, px,
};

use super::{DiffKind, DiffView};
use crate::AppState;
use crate::git::GitState;
use crate::icons::{CHEVRONS_DOWN_UP, CHEVRONS_UP_DOWN, REFRESH_CW};
use crate::layout::color;
use crate::ui::{
    CHEVRON_LEFT_ICON, CHEVRON_RIGHT_ICON, GRID, UI_FONT_SIZE_SMALL, bar_icon_button,
    bar_separator, centered_message, search_bar_row, search_toolbar, text_button,
};

impl DiffView {
    /// Keys in the list: `↑` / `↓` and `Enter`.
    fn list_key_down(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.list.update(cx, |list, cx| list.key_down(ev, cx)) {
            cx.stop_propagation();
        }
    }

    /// The toolbar: `{n} files  +A −R` on the left; previous / next
    /// change, collapse all and refresh on the right.
    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = cx.global::<AppState>().theme.ui.clone();
        let (files, added, removed) = self.totals();
        let all_collapsed = self
            .list
            .read(cx)
            .results()
            .files()
            .iter()
            .all(|f| f.collapsed);
        let collapse_icon = if all_collapsed {
            CHEVRONS_UP_DOWN
        } else {
            CHEVRONS_DOWN_UP
        };
        let files_text = match files {
            1 => "1 file".to_string(),
            n => format!("{n} files"),
        };
        let summary = div()
            .debug_selector(|| "git-diff-summary".into())
            .flex()
            .flex_1()
            .min_w_0()
            .gap(px(2. * GRID))
            .overflow_hidden()
            .whitespace_nowrap()
            .text_size(px(UI_FONT_SIZE_SMALL))
            .text_color(color(ui.text_muted))
            .child(files_text)
            .child(
                div()
                    .flex()
                    .gap(px(GRID))
                    .child(
                        div()
                            .text_color(color(ui.vc_added))
                            .child(format!("+{added}")),
                    )
                    .child(
                        div()
                            .text_color(color(ui.vc_deleted))
                            .child(format!("\u{2212}{removed}")),
                    ),
            );
        let bar = search_bar_row()
            .child(summary)
            .child(
                bar_icon_button("git-diff-previous", CHEVRON_LEFT_ICON, false, cx)
                    .on_click(cx.listener(|view, _, _, cx| view.step_change(false, cx))),
            )
            .child(
                bar_icon_button("git-diff-next", CHEVRON_RIGHT_ICON, false, cx)
                    .on_click(cx.listener(|view, _, _, cx| view.step_change(true, cx))),
            )
            .child(bar_separator(cx))
            .child(
                bar_icon_button("git-diff-collapse-all", collapse_icon, false, cx)
                    .on_click(cx.listener(|view, _, _, cx| view.toggle_all_collapsed(cx))),
            )
            .child(
                bar_icon_button("git-diff-refresh", REFRESH_CW, false, cx)
                    .on_click(cx.listener(|view, _, window, cx| view.refresh(window, cx))),
            );
        search_toolbar(cx)
            .debug_selector(|| "git-diff-toolbar".into())
            .child(bar)
    }

    /// The centered line shown instead of the list, with the "Manage
    /// Workspace Trust" button in Restricted Mode.
    fn render_message(&self, text: String, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = cx.global::<AppState>().theme.ui.clone();
        let restricted =
            self.kind == DiffKind::Uncommitted && self.git.read(cx).state() == GitState::Restricted;
        let trust = restricted.then(|| {
            text_button("git-diff-trust", "Manage Workspace Trust", cx).on_click(cx.listener(
                |view, _, window, cx| {
                    if let Some(workbench) = view.workbench.upgrade() {
                        workbench.update(cx, |wb, cx| {
                            wb.run_command("workbench.trust.manage", None, window, cx)
                        });
                    }
                },
            ))
        });
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(3. * GRID))
            .child(
                centered_message("git-diff-message", text, color(ui.text_muted))
                    .size_auto()
                    .flex_none(),
            )
            .children(trust)
    }
}

impl Render for DiffView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = cx.global::<AppState>().theme.ui.clone();
        let body = match self.message(cx) {
            Some(text) => self.render_message(text, cx).into_any_element(),
            None => self.list.clone().into_any_element(),
        };
        let toolbar = (!self.files.is_empty()).then(|| self.render_toolbar(cx));
        div()
            .id("git-diff-view")
            .debug_selector(|| "git-diff-view".into())
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .size_full()
            .bg(color(ui.editor_background))
            .children(toolbar)
            .child(
                div()
                    .id("git-diff-list")
                    .debug_selector(|| "git-diff-list".into())
                    .track_focus(&self.list_focus)
                    .on_key_down(cx.listener(Self::list_key_down))
                    .flex_1()
                    .min_h_0()
                    .child(body),
            )
    }
}
