# Reopen Closed Editor and Close Others

Date: 2026-11-18. Status: draft, awaiting approval.

## Purpose

People moving from VS Code have two habits with tabs that Lucerna does not support yet:

1. After an accidental ⌘W, press ⇧⌘T to reopen the tab just closed, in the same place, with the
   cursor and scroll as before. Pressing it again steps back through earlier closed tabs.
2. Cleaning up tabs in bulk: Close Others (⌥⌘T), close the tabs to the right/left, close the saved
   tabs, close the whole group. They do it with keys or by right-clicking a tab.

This is a tier 1 item in `docs/roadmap.md`: the things that block newcomers from VS Code. Commands,
keys and behavior follow VS Code; the menu UI follows Zed.

## Goals

- `workbench.action.reopenClosedEditor` (⇧⌘T): reopen the most recent close, into its old group, at
  the exact position, with cursor and scroll. Pressing again steps back further. The history
  remembers 20 tabs.
- Bulk close, each command targeting one tab (the active tab, or the right-clicked tab):

  | Command | Title in the Command Palette | Key (mac) |
  |---|---|---|
  | `workbench.action.closeOtherEditors` | View: Close Other Editors in Group | ⌥⌘T |
  | `workbench.action.closeEditorsToTheRight` | View: Close Editors to the Right in Group | — |
  | `workbench.action.closeEditorsToTheLeft` | View: Close Editors to the Left in Group | — |
  | `workbench.action.closeUnmodifiedEditors` | View: Close Saved Editors in Group | ⌘K U |
  | `workbench.action.closeEditorsInGroup` | View: Close All Editors in Group | ⌘K W |
  | `workbench.action.closeAllEditors` (already exists) | View: Close All Editors | ⌘K ⌘W (key added) |
  | `workbench.action.reopenClosedEditor` | View: Reopen Closed Editor | ⇧⌘T |

- A right-click menu on tabs, with only the close group of commands (order and grouping follow Zed):

  ```
  Close
  Close Others
  ─────────────
  Close to the Left
  Close to the Right
  ─────────────
  Close Saved
  Close All
  ```

  The menu targets the right-clicked tab. An item with no effect is dimmed: Close Others when the
  group has only 1 tab; Close to the Left on the first tab; Close to the Right on the last tab.
  "Close All" here is Close All Editors **in Group** (`closeEditorsInGroup`), like VS Code and Zed.
- Closing several tabs that have unsaved documents: ask **one shared dialog** (the existing
  `request_close_all` flow).

## Non-goals (this time)

- The other items of the tab menu: Copy Path, Copy Relative Path, Reveal in Explorer, Pin/Keep Open,
  Split.
- Sticky tabs (VS Code-style pinning). Lucerna only has preview tabs and normal tabs, so the bulk
  close commands have no tab that must be spared.
- Saving the closed-tab history across the next app launch.
- Reopening a discarded Untitled tab (see below).
- The context key `canReopenClosedEditor`, selecting several tabs and closing them at once.
- Restoring a deleted group: after Close All with 2 groups, ⇧⌘T opens everything into the active
  group.

## Must not happen

- Some path that closes a tab (⌘W, ×, middle click, `:q`, `:qa`, Close All, the Close button of the
  deleted-file bar, the new commands) does not go into the history, making ⇧⌘T "do nothing" in a
  confusing way.
- A bulk close that loses unsaved changes without asking, or asks a separate dialog for each file.
- Pressing Cancel in the dialog while some tab is still closed.
- ⇧⌘T opens an empty tab for a deleted file, or opens another tab duplicating an existing one.
- A blocked UI: checking that the file still exists is just one `stat` per entry when the key is
  pressed, like Go Back.

## References

Read from the source of the main/master branch, November 2026.

### VS Code

- **Reopen.** `ReopenClosedEditorAction` (`browser/parts/editor/editorActions.ts`), ⇧⌘T, calls
  `HistoryService.reopenLastClosedEditor()` (`services/history/browser/historyService.ts`).
  - `MAX_RECENTLY_CLOSED_EDITORS = 20`. The history is shared across the whole window; beyond 20, the
    oldest entry is dropped.
  - Each entry (`IRecentlyClosedEditor`) keeps: the untyped form of the editor, resource, `index`,
    `sticky`, `batchId`. It does **not** keep the group or the preview flag: it always reopens into
    the active group, as pinned, at the exact old `index`.
  - **Batch** (new on main): closes within the same synchronous run share a `batchId`; one ⇧⌘T
    reopens the whole batch (after Close Others or Close All).
  - A `REPLACE` close (a preview replaced by another preview) and `MOVE` are not recorded. Closing
    the same editor again first removes the old entry, so there is no duplicate.
  - Untitled is recorded but its content is not kept: reopening gives an empty file.
  - If the active group already contains that editor, it is not opened again. If no entry of a batch
    can be opened, it steps back to the previous batch.
  - A deleted file (watcher or a delete action) is removed from the history; a renamed file has its
    entry updated.
  - Cursor and scroll are not in the entry but in a per-group `EditorMemento`, keyed by resource (100
    entries), enabled by `workbench.editor.restoreViewState`.
- **Close commands.** `closeOtherEditors` (⌥⌘T, mac only), `closeEditorsToTheRight`/`Left` (no key),
  `closeUnmodifiedEditors` "Close Saved Editors in Group" (⌘K U), `closeEditorsInGroup` (⌘K W),
  `closeAllEditors` (⌘K ⌘W). All spare sticky tabs. From the tab menu they target the right-clicked
  tab, otherwise the active tab. Close Others and Close to the Right pin the tab that is kept.
  - Unsaved tabs: the per-group commands ask **one file at a time**, the first Cancel stops the whole
    batch; only Close All Editors asks one shared dialog.
  - Tab menu: Close · Close Others · Close to the Right · Close Saved · Close All │ Copy Path · Copy
    Relative Path │ Reopen Editor With… │ Reveal… │ Keep Open · Pin │ Split… There is no Close to
    the Left.

### Zed

- `pane::ReopenClosedItem` (cmd-shift-t): `closed_stack` in each pane's `NavHistory`,
  `MAX_NAVIGATION_HISTORY_LEN = 1024`, not persisted across launches. An entry keeps the item, path,
  navigation data (cursor anchor, scroll position), preview flag. **A closed preview tab is not
  recorded.**
  - Reopens into the active pane, **next to the active tab** (not at the old position), one tab at a
    time. If the item is still open, it activates it; if opening fails, it tries the next entry.
    Untitled cannot be reopened. A deleted file is removed from the history.
- Close commands: `CloseOtherItems` (alt-cmd-t; `CloseInactiveItems` is now an old alias),
  `CloseItemsToTheLeft` (cmd-k e), `CloseItemsToTheRight` (cmd-k t), `CloseCleanItems` (cmd-k u),
  `CloseAllItems` (cmd-k w). By default they spare pinned tabs.
  - Several unsaved tabs: **one shared dialog** "Do you want to save changes to the following
    files?", Save all / Discard all / Cancel; one tab gets the ordinary dialog.
  - Tab menu: Close · Close Others │ Close Left · Close Right │ Close Clean · Close All │ Pin Tab │ …
    │ Copy Path · Copy Relative Path │ Reveal… Items with no effect are dimmed.

### Helix

`:buffer-close-others` (`:bco`), `:buffer-close-all` (`:bca`), with `!` variants. They close the clean
buffers, then report the error "N unsaved buffers remaining" instead of asking. No pinning, no
reopening a closed buffer.

### What Lucerna chooses, and why

| Point | Choice | Reason |
|---|---|---|
| Commands, keys, titles | VS Code | rule §1 of design-principles |
| Number of entries | 20 (VS Code) | enough for "accidental"; VS Code is the user's habit |
| Batch | reopen the whole batch (VS Code main) | an accidental Close Others is undone with one key |
| Target group | **the old group if it still exists**, otherwise the active group | a deviation from VS Code: the brief asks for "in its group"; with several groups, reopening into a different group makes users lose their place. With a single group it is identical to VS Code |
| Position | the old `index` (VS Code) | Zed places it next to the active tab, which is less exact |
| Preview or pinned | always pinned (VS Code) | |
| A replaced preview | not recorded (both) | |
| Cursor and scroll | kept in the entry (Zed's way) | Lucerna has no per-file memento yet; keeping them in the entry is enough for this feature |
| Unsaved tabs when closing several | one shared dialog (Zed, and VS Code's Close All) | the flow Lucerna already has; asking file by file is very annoying |
| Tab menu | follows Zed, only the close group, with Close to the Left | UI follows Zed; the user chose only the close group |

## Design

### History in the model (`editor_groups/model.rs`)

`EditorGroups` gains a list of closed tabs, oldest first, at most 20 entries:

```rust
pub struct ClosedTab {
    pub item: TabItem,
    pub group: GroupId,       // group at close time
    pub index: usize,         // position in the group before the close (before the whole batch)
    pub was_active: bool,     // was the group's active tab at close time
    pub batch: u64,           // tabs closed in one call share a batch
    pub tab: TabId,           // old id, so the view can attach cursor and scroll
    pub view: Option<TabViewState>,
}
pub struct TabViewState { pub cursor: Point, pub scroll_top_row: u32 }
```

- `close(id)` stays as before and records one entry (its own batch). Add `close_many(ids)`: closes
  each tab in order, all entries sharing one batch. `index` is taken from the position **before** the
  batch starts closing. For example, Close Others on C in [A,B,C,D] records A=0, B=1, D=3, so
  reopening by ascending `index` puts each tab back in its exact old place. Every bulk close command,
  including `request_close_all`, goes through here.
- **Only tabs with a file are recorded:** `TabItem::File` and `TabItem::MarkdownPreview` (the static
  one). Other tabs (Search, References, language server log and Health, Extensions, Uncommitted
  Changes, single-file diff, Changes since, Get Started, dynamic preview) do not go into the history.
  They depend on outside state (search results, a running server, the active file), so reopening
  easily gives something stale or meaningless, and all can be reopened with their own commands. This
  is our own choice; VS Code records almost every editor.
- **Only user-initiated closes are recorded:** `close` and `close_many` record history. Add
  `remove(id)`, which closes a tab without recording, for the times code closes a tab on its own:
  - `Workbench::close_file_tabs` (Save As overwriting an open file);
  - `Workbench::paths_deleted` (deleting a file in Explorer);
  - `finish_onboarding`;
  - the `replace_item` times when it closes a tab itself because a tab with the same item already
    exists.

  The Close button on the deleted-file bar is a user action, so it still records. If the file no
  longer exists, ⇧⌘T skips that entry.
- **Not recorded:**
  - a preview tab replaced by `open_file`;
  - a tab replaced by `replace_item` (toggling between Markdown source/preview);
  - an Untitled tab (`TabItem::File` or `MarkdownPreview` of an `untitled:` path): the document goes
    away with the tab, and reopening only gives an empty file. A small deviation from VS Code, noted
    here.
- Recording an item already in the history with the same group first removes the old entry, so there
  are no duplicate entries. Beyond 20, the oldest entry is dropped. If a batch is larger than 20
  (Close All with 30 tabs), only the last 20 tabs of the batch are kept.
- `retarget(from, to)` (rename, move a file) also fixes the paths in the history.
- `reopen_closed(exists: impl Fn(&Path) -> bool) -> Vec<(TabId, Option<TabViewState>)>`:
  1. Take the newest batch out of the history. Drop the entries for which `exists` returns `false`
     and the entries whose item already has a tab in the target group.
  2. If no entry is left to open:
     - if some entry was dropped because its tab is **already reopened** in the target group,
       activate that tab and stop (the entry is used up, do not step back to an older batch), like
       VS Code and Zed;
     - only when every entry was dropped because the **file no longer exists**, step back to the
       previous batch;
     - when the history is exhausted, return empty and do nothing.
  3. Open each entry into the group with the old `GroupId` if it still exists, otherwise the active
     group, at position `min(index, tab count)`, as pinned. Within one group, open by ascending
     `index` so the tabs return to the exact place.
  4. Active tab afterwards: for a one-tab batch, that tab is activated and its group becomes the
     active group. For a multi-tab batch, the tabs with `was_active` are activated in their groups.
     The active group is the group of the last `was_active` tab; if there is no such tab (Close
     Others), the active tab and group stay unchanged.
  5. Return the new ids of the opened tabs together with their cursor and scroll.
- `set_closed_view(tab: TabId, view: TabViewState)`: attaches cursor and scroll to the entry with the
  old id `tab`.

### Cursor and scroll (`editor_groups/view.rs`)

`EditorGroupsView::update_model` captures the new entries into the history after `f` runs. At this
point the tab's body is still in `bodies` (it is only cleaned up on the next paint). If the body is a
`CodeView`, read `cursor()` and `scroll_top_row()` then call `set_closed_view`. This catches every
close path without editing each call site.

On reopening, the `reopenClosedEditor` command (in `Workbench`) applies cursor and scroll like
`navigate`: if the tab already has a view, `go_to_and_scroll`; if not,
`go_to_and_scroll_when_opened(path, …)`. For a multi-tab batch where one file is open in two tabs at
once, only the active tab is guaranteed; this is left to the implementer.

### Bulk close and the dialog (`editor_groups/view.rs`)

Merge `request_close` and `request_close_all` into one `request_close_tabs(ids: Vec<TabId>, …)`:

- A document that **needs asking** is an unsaved document of a tab in `ids` such that **no** tab
  outside `ids` is showing it (a generalization of `last_tab_of_dirty`).
- No document needs asking: `close_many(ids)` right away.
- Otherwise: `save::prompt_unsaved_all(names)`. For one file this is the Save / Don't Save / Cancel
  dialog as today; for several files it is Save All / Don't Save / Cancel.
  - Save / Save All: `save_each`, then close only if every such document is still clean. Keep the
    existing rule: typing more during the save means no tab is closed.
  - Don't Save: `close_many(ids)` then `discard` each document.
  - Cancel: close nothing.
- `request_close(id)` and `request_close_all` become thin wrappers calling this function. All
  existing tests (`save.rs`, `untitled.rs`, `hot_exit.rs`) must still pass.

The tab set for each command, within the group of the target tab `t`:

| Command | Closes | Notes |
|---|---|---|
| Close Others | every tab except `t` | pins `t`, activates it |
| Close to the Right | the tabs after `t` | pins `t`; if the active tab is closed, activates `t` |
| Close to the Left | the tabs before `t` | as above (VS Code does not pin here; Lucerna does the same as Right for consistency, our own choice) |
| Close Saved | every tab of the group whose document is not dirty; a tab that is not a file counts as saved | never asks |
| Close All in Group | every tab of the group | an empty group is removed if other groups remain, like `close` today |

Active tab and group after a command:
- Close Others, Left and Right: `t` becomes the active tab, its group becomes the active group, even
  when the command runs from the menu on a non-active group. Typing continues into `t`, like clicking
  that tab.
- Close Saved and Close All in Group: the group's active tab is the most recently used remaining tab.
  The active group stays unchanged, unless it is removed (then it follows the rules of `close`).

Close Others, Left and Right pin `t` before asking. If the user presses Cancel, `t` is still pinned,
and we consider that harmless.

### Target tab and command arguments

The commands accept an optional argument `{"tab": <number>}` (the tab's id, `TabId::as_u64`). With no
argument, the target tab is the active tab of the active group; if there is no active tab the command
does nothing. If there is an argument but the id no longer exists (the tab is already closed), the
command does nothing, so that tabs around another tab are not closed by mistake. The right-click menu
passes the id of the clicked tab. `closeActiveEditor` also accepts this argument so that the menu's
"Close" item closes exactly the right-clicked tab.

### Right-click menu on tabs

- `EditorGroupsView` listens to `on_mouse_down(MouseButton::Right)` on each tab and builds a
  `ContextMenu` (`context_menu.rs`) with the items from the Goals section, with `disabled_when` for
  items that have no effect.
- Hold the menu with `DeployedMenu` in the view and draw it with `render_deployed`, like Explorer.
- A selected item runs its command through `Workbench::run_command(command, args)` with
  `{"tab": id}`, after the current update pass finishes (`window.defer`), like Explorer.
- The menu shows each item's shortcut (`shortcut_label`, already in the component).
- Right-click does not change the active tab (VS Code and Zed are the same).

### Keys, app menu, docs

- `default-keybindings.json`, add one block at the end:
  - `shift+cmd+t` → reopenClosedEditor
  - `alt+cmd+t` → closeOtherEditors
  - `cmd+k u` → closeUnmodifiedEditors
  - `cmd+k w` → closeEditorsInGroup
  - `cmd+k cmd+w` → closeAllEditors

  None collides with an existing key (`cmd+k cmd+u` is removeCommentLine, different from `cmd+k u`).
- `app_menu.rs`: add "Reopen Closed Editor" to the File menu, right under "Close Editor" (our own
  choice; VS Code puts it in Open Recent).
- Update `docs/keyboard-shortcuts.md`, `CHANGELOG.md` (Unreleased → Added) and mark this item in
  `docs/roadmap.md`. Only add lines, do not edit existing lines, to make merging with the Replace in
  project branch easy.

## Testing

- **Model unit tests** (`tests/editor_groups_model.rs`):
  - recording history, the limit of 20, no duplicates;
  - not recording a replaced preview, `replace_item`, Untitled, tabs that are not files, `remove`;
  - batch: `close_many` creates one batch, reopening the whole batch;
  - correct group, correct `index`, active tab after reopening;
  - a lost group means opening into the active group;
  - dropping entries whose file no longer exists and stepping back a batch; an entry already
    reopened activates that tab, without stepping back a batch;
  - `retarget` fixes the history.
- **GPUI tests** (`tests/editor_groups_view.rs` or a new file `tests/tab_close_commands.rs`):
  - ⇧⌘T after ⌘W restores the tab, cursor and scroll;
  - each bulk close command, including the version with the `tab` argument;
  - the shared dialog when several files are dirty: Save All, Don't Save, Cancel;
  - a document showing in another group is not asked about;
  - the right-click menu shows the right items, dimmed in the right places, selecting an item runs the
    command with the right-clicked tab;
  - the new commands are in the registry and have keys.
- All `lucerna-workbench` tests, clippy, fmt and `cargo deny check` must be clean.

## Left to the implementer

- Naming functions and splitting files (for example splitting the history out into
  `editor_groups/closed.rs` if `model.rs` gets too long).
- The pixel position of the menu: at the mouse cursor like Explorer.
- The case of one file open in two tabs in the same batch: only the active tab needs the right cursor.
