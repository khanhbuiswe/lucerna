//! The editor area: groups of tabs laid out left to right.

mod closed;
mod model;
mod view;

pub use closed::{ClosedTab, TabViewState};
pub use model::{
    BRANCH_CHANGES_TAB_STATE, EXTENSIONS_TAB_STATE, EditorGroups, FileView, GIT_CHANGES_TAB_STATE,
    GIT_CHANGES_TAB_TITLE, Group, GroupId, GroupState, GroupsState, IMAGE_PREVIEW_STATE_PREFIX,
    MARKDOWN_PREVIEW_DYNAMIC_STATE_PREFIX, MARKDOWN_PREVIEW_STATE_PREFIX, OpenMode,
    SEARCH_TAB_STATE, Tab, TabId, TabItem,
};
pub(crate) use model::{branch_changes_title, file_diff_title, prefixed, split_preview_state};
pub use view::{EditorGroupsView, TabBodyFactory, placeholder_body};
pub(crate) use view::{path_bar_text, untitled_title};

use gpui::{App, Context, Window};
use serde_json::Value;

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
        |wb, args, window, cx| {
            let Some(id) = target_tab(wb, args, cx) else {
                return;
            };
            let groups = wb.editor_groups().clone();
            groups.update(cx, |view, cx| view.request_close(id, window, cx));
        },
    );
    commands.register(
        "workbench.action.closeAllEditors",
        "View: Close All Editors",
        |wb, _, window, cx| {
            let groups = wb.editor_groups().clone();
            groups.update(cx, |view, cx| view.request_close_all(window, cx));
        },
    );
    commands.register(
        "workbench.action.quickOpenPreviousRecentlyUsedEditorInGroup",
        "View: Quick Open Previous Recently Used Editor in Group",
        |wb, _, _, cx| {
            wb.update_editor_groups(cx, EditorGroups::cycle_mru);
        },
    );
    commands.register(
        "workbench.action.navigateBack",
        "Go: Back",
        |wb, _, window, cx| {
            wb.navigate(true, cx);
            wb.focus_editor_area(window, cx);
        },
    );
    commands.register(
        "workbench.action.navigateForward",
        "Go: Forward",
        |wb, _, window, cx| {
            wb.navigate(false, cx);
            wb.focus_editor_area(window, cx);
        },
    );
    commands.register(
        "workbench.action.reopenClosedEditor",
        "View: Reopen Closed Editor",
        |wb, _, window, cx| wb.reopen_closed_editor(window, cx),
    );
    let close: [(&'static str, &'static str, CommandHandler); 5] = [
        (
            "workbench.action.closeOtherEditors",
            "View: Close Other Editors in Group",
            |wb, args, w, cx| close_set(wb, args, CloseSet::Others, w, cx),
        ),
        (
            "workbench.action.closeEditorsToTheRight",
            "View: Close Editors to the Right in Group",
            |wb, args, w, cx| close_set(wb, args, CloseSet::Right, w, cx),
        ),
        (
            "workbench.action.closeEditorsToTheLeft",
            "View: Close Editors to the Left in Group",
            |wb, args, w, cx| close_set(wb, args, CloseSet::Left, w, cx),
        ),
        (
            "workbench.action.closeUnmodifiedEditors",
            "View: Close Saved Editors in Group",
            |wb, args, w, cx| close_set(wb, args, CloseSet::Saved, w, cx),
        ),
        (
            "workbench.action.closeEditorsInGroup",
            "View: Close All Editors in Group",
            |wb, args, w, cx| close_set(wb, args, CloseSet::Group, w, cx),
        ),
    ];
    for (id, title, handler) in close {
        commands.register(id, title, handler);
    }
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

/// The tab a close command acts on: the one its `{"tab": <id>}` argument
/// names (the tab bar's menu), else the active tab of the active group.
/// `None` when the named tab is gone: closing around another tab could
/// close the wrong ones.
fn target_tab(wb: &Workbench, args: Option<&Value>, cx: &App) -> Option<TabId> {
    let model = wb.editor_groups().read(cx).model();
    match args.and_then(|a| a.get("tab")) {
        Some(tab) => tab.as_u64().and_then(|raw| model.tab_from_u64(raw)),
        None => model.active_tab().map(|t| t.id),
    }
}

/// Which tabs of the target's group a close command closes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CloseSet {
    Others,
    Right,
    Left,
    /// Those without unsaved changes; tabs that are not files count.
    Saved,
    Group,
}

/// Closes `set` around the command's target tab, asking once about unsaved
/// changes. Close Others, Left and Right first pin and activate the target,
/// whose group becomes the active one.
fn close_set(
    wb: &mut Workbench,
    args: Option<&Value>,
    set: CloseSet,
    window: &mut Window,
    cx: &mut Context<Workbench>,
) {
    let Some(target) = target_tab(wb, args, cx) else {
        return;
    };
    if matches!(set, CloseSet::Others | CloseSet::Right | CloseSet::Left) {
        wb.update_editor_groups(cx, |m| {
            m.pin(target);
            m.activate(target);
        });
    }
    let model = wb.editor_groups().read(cx).model();
    let ids = match set {
        CloseSet::Others => model.others(target),
        CloseSet::Right => model.to_the_right(target),
        CloseSet::Left => model.to_the_left(target),
        CloseSet::Group => model.group_tabs(target),
        CloseSet::Saved => {
            let docs = wb.documents().read(cx);
            let group = model.group_tabs(target);
            model
                .groups()
                .iter()
                .flat_map(|g| &g.tabs)
                .filter(|t| group.contains(&t.id))
                .filter(|t| !t.path().is_some_and(|p| docs.is_dirty(p)))
                .map(|t| t.id)
                .collect()
        }
    };
    let groups = wb.editor_groups().clone();
    groups.update(cx, |view, cx| view.request_close_tabs(ids, window, cx));
}
