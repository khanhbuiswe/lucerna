//! The status-bar vim mode indicator: `-- NORMAL --`, and the pending
//! keys after it in the editor font.

use gpui::{
    Context, Entity, IntoElement, ParentElement, Render, Styled, Subscription, Window, div,
    prelude::*, px,
};

use crate::AppState;
use crate::code_view::CodeView;
use crate::documents::DocState;
use crate::editor_groups::EditorGroupsView;
use crate::fonts::EDITOR_FONT_FAMILY;
use crate::layout::color;
use crate::ui::{GRID, UI_FONT_FAMILY};
use crate::vim::VimLayer;

pub(crate) struct ModeIndicator {
    vim: Entity<VimLayer>,
    groups: Entity<EditorGroupsView>,
    _subscriptions: [Subscription; 2],
}

impl ModeIndicator {
    pub(crate) fn new(
        vim: Entity<VimLayer>,
        groups: Entity<EditorGroupsView>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = [
            cx.observe(&vim, |_, _, cx| cx.notify()),
            cx.observe(&groups, |_, _, cx| cx.notify()),
        ];
        ModeIndicator {
            vim,
            groups,
            _subscriptions: subscriptions,
        }
    }
}

impl Render for ModeIndicator {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Shown only for a code view vim takes keys in.
        let view = self
            .groups
            .read(cx)
            .active_body()
            .and_then(|body| body.downcast::<CodeView>().ok())
            .filter(|v| matches!(v.read(cx).doc_state(), DocState::Ready | DocState::Deleted));
        let vim = self.vim.read(cx);
        let parts = view.and_then(|v| vim.indicator_parts(v.entity_id()));
        let muted = color(cx.global::<AppState>().theme.ui.text_muted);
        div().when_some(parts, |d, (label, pending)| {
            let pending = pending.to_string();
            d.debug_selector(|| "vim-mode-indicator".into())
                .flex()
                .gap(px(2. * GRID))
                .font_family(UI_FONT_FAMILY)
                .text_color(muted)
                .child(label)
                .when(!pending.is_empty(), |d| {
                    d.child(div().font_family(EDITOR_FONT_FAMILY).child(pending))
                })
        })
    }
}
