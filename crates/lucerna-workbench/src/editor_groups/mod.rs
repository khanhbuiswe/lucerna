//! The editor area: groups of tabs laid out left to right.

mod model;
mod view;

pub use model::{
    EXTENSIONS_TAB_STATE, EditorGroups, Group, GroupState, GroupsState, OpenMode, SEARCH_TAB_STATE,
    Tab, TabId, TabItem,
};
pub(crate) use view::path_bar_text;
pub use view::{EditorGroupsView, TabBodyFactory, placeholder_body};

use crate::{CommandHandler, CommandRegistry, Workbench};

pub(crate) fn register_commands(commands: &mut CommandRegistry) {
    commands.register(
        "workbench.action.splitEditor",
        "View: Split Editor",
        |wb, _, window, cx| {
            wb.update_editor_groups(cx, |m| m.split_active());
            wb.focus_editor_area(window, cx);
        },
    );
    commands.register(
        "workbench.action.closeActiveEditor",
        "View: Close Editor",
        |wb, _, _, cx| {
            wb.update_editor_groups(cx, |m| m.active_tab().map(|t| t.id).map(|id| m.close(id)));
        },
    );
    commands.register(
        "workbench.action.quickOpenPreviousRecentlyUsedEditorInGroup",
        "View: Quick Open Previous Recently Used Editor in Group",
        |wb, _, _, cx| {
            wb.update_editor_groups(cx, EditorGroups::cycle_mru);
        },
    );
    let focus_group: [(&'static str, &'static str, CommandHandler); 8] = [
        (
            "workbench.action.focusFirstEditorGroup",
            "View: Focus First Editor Group",
            |wb, _, w, cx| focus(wb, 0, w, cx),
        ),
        (
            "workbench.action.focusSecondEditorGroup",
            "View: Focus Second Editor Group",
            |wb, _, w, cx| focus(wb, 1, w, cx),
        ),
        (
            "workbench.action.focusThirdEditorGroup",
            "View: Focus Third Editor Group",
            |wb, _, w, cx| focus(wb, 2, w, cx),
        ),
        (
            "workbench.action.focusFourthEditorGroup",
            "View: Focus Fourth Editor Group",
            |wb, _, w, cx| focus(wb, 3, w, cx),
        ),
        (
            "workbench.action.focusFifthEditorGroup",
            "View: Focus Fifth Editor Group",
            |wb, _, w, cx| focus(wb, 4, w, cx),
        ),
        (
            "workbench.action.focusSixthEditorGroup",
            "View: Focus Sixth Editor Group",
            |wb, _, w, cx| focus(wb, 5, w, cx),
        ),
        (
            "workbench.action.focusSeventhEditorGroup",
            "View: Focus Seventh Editor Group",
            |wb, _, w, cx| focus(wb, 6, w, cx),
        ),
        (
            "workbench.action.focusEighthEditorGroup",
            "View: Focus Eighth Editor Group",
            |wb, _, w, cx| focus(wb, 7, w, cx),
        ),
    ];
    for (id, title, handler) in focus_group {
        commands.register(id, title, handler);
    }
}

/// Focuses group `index` and moves keyboard focus to the editor area; a
/// group that does not exist is ignored.
fn focus(
    wb: &mut Workbench,
    index: usize,
    window: &mut gpui::Window,
    cx: &mut gpui::Context<Workbench>,
) {
    if wb.update_editor_groups(cx, |m| m.focus_group(index)) {
        wb.focus_editor_area(window, cx);
    }
}
