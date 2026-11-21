# Reopen Closed Editor and Close Others Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** ⇧⌘T reopens the last closed tab (or batch of tabs) in its group, at its position, with its cursor and scroll. Close Others, Close to the Left/Right, Close Saved and Close All in Group work from commands, keys and a right-click menu on tabs.

**Architecture:** The closed-tab history lives in the pure `EditorGroups` model, in a new `editor_groups/closed.rs`. Every user close goes through `close`/`close_many`, which record entries; code-driven closes use `remove`, which does not. `EditorGroupsView::update_model` attaches cursor and scroll to new entries while the closed tab's `CodeView` body still exists. One `request_close_tabs` replaces `request_close` and `request_close_all` and asks a single Save All / Don't Save / Cancel prompt. The tab menu reuses `ContextMenu` / `deploy_context_menu` the way Explorer does.

**Tech Stack:** Rust, GPUI, crate `lucerna-workbench` (tests in `crates/lucerna-workbench/tests/`).

**Spec:** `docs/superpowers/specs/2026-11-18-reopen-close-others-design.md`

## Global Constraints

- Cargo commands are prefixed `PATH="/opt/homebrew/opt/rustup/bin:$PATH"`. Run `scripts/prune-target.sh` before each build or test. Never set `CARGO_TARGET_DIR` or `--target-dir`; if another cargo holds the lock, wait. If you hit impossible errors (a method missing that exists in source), `touch crates/*/src/lib.rs`.
- Every string shown in the app is English, with titles exactly as in the spec:
  - Command Palette: "View: Reopen Closed Editor", "View: Close Other Editors in Group", "View: Close Editors to the Right in Group", "View: Close Editors to the Left in Group", "View: Close Saved Editors in Group", "View: Close All Editors in Group".
  - Menu labels: "Close", "Close Others", "Close to the Left", "Close to the Right", "Close Saved", "Close All", and the File menu entry "Reopen Closed Editor".
- History holds at most 20 entries.
- Only `TabItem::File` and `TabItem::MarkdownPreview` are recorded, never one whose path is untitled (`crate::untitled::is_untitled`).
- Edit shared files append-only, so this branch merges cleanly with the Replace-in-project branch: `default-keybindings.json` (one block at the end), `app_menu.rs` (one line), `CHANGELOG.md`, `docs/keyboard-shortcuts.md`, `docs/roadmap.md`.
- Commit with explicit paths (`git commit -- <paths>`). Every commit message ends with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Never copy code from Zed or Helix.

## Review Focus

1. **Close All across two groups, then ⇧⌘T.** Emptied groups are removed, so the old `GroupId`s are gone. Every tab must come back into the active group, in index order, with none lost. Test in Task 1 (`reopen_into_missing_group_goes_to_active_group`).
2. **A batch where only some files still exist.** Reopen the files that exist and skip the deleted ones. Fall back to the previous batch only when none exist. Test in Task 1 (`reopen_skips_missing_files_and_falls_back_a_batch`).
3. **A dirty file that is also shown in a tab the command does not close.** No prompt, and the document stays dirty and open. Test in Task 3 (`close_others_does_not_ask_for_document_shown_elsewhere`).
4. **Cancel at the shared prompt.** No tab closes and no history entry is written. Test in Task 3 (`cancel_closes_nothing_and_records_nothing`).
5. **Right-clicking a tab in a group that is not active.** The command acts on that tab's group, and that group becomes active. Test in Task 5 (`menu_targets_right_clicked_tab_in_other_group`).

---

### Task 1: Closed-tab history in the model

**Files:**
- Create: `crates/lucerna-workbench/src/editor_groups/closed.rs`
- Modify: `crates/lucerna-workbench/src/editor_groups/model.rs` (`close`, `replace_item`, `retarget`, new methods)
- Modify: `crates/lucerna-workbench/src/editor_groups/mod.rs` (module and exports)
- Modify: `crates/lucerna-workbench/src/lib.rs` (re-export, if `EditorGroups` is re-exported there)
- Modify: `crates/lucerna-workbench/src/workbench.rs:1395-1410` and `:1440-1452` (`m.close` → `m.remove`)
- Modify: `crates/lucerna-workbench/src/onboarding/mod.rs:700` (`m.close` → `m.remove`)
- Test: `crates/lucerna-workbench/tests/closed_tabs_model.rs` (new file, so it does not conflict with `editor_groups_model.rs`)

**Interfaces:**
- Produces, in `closed.rs`, re-exported from `lucerna_workbench`:
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub struct TabViewState { pub cursor: lucerna_text::Point, pub scroll_top_row: u32 }
  #[derive(Debug, Clone, PartialEq)]
  pub struct ClosedTab {
      pub item: TabItem, pub group: GroupId, pub index: usize,
      pub was_active: bool, pub batch: u64, pub tab: TabId,
      pub view: Option<TabViewState>,
  }
  pub(crate) struct ClosedHistory { entries: VecDeque<ClosedTab>, next_batch: u64 } // cap 20
  ```
- Produces, on `EditorGroups`:
  - `pub fn close(&mut self, id: TabId) -> Option<TabId>`: unchanged signature. It now records the tab as a batch of its own.
  - `pub fn close_many(&mut self, ids: &[TabId]) -> Option<TabId>`: one batch. Each `index` is the tab's position before the first close of the batch. Returns the active tab afterwards.
  - `pub fn remove(&mut self, id: TabId) -> Option<TabId>`: what `close` did before, records nothing.
  - `pub fn closed(&self) -> &VecDeque<ClosedTab>` (oldest first).
  - `pub fn set_closed_view(&mut self, tab: TabId, view: TabViewState)`.
  - `pub fn reopen_closed(&mut self, exists: impl Fn(&Path) -> bool) -> Vec<(TabId, Option<TabViewState>)>`.
  - `pub(crate) fn tab_from_u64(&self, raw: u64) -> Option<TabId>`: the live tab with that id.

- [ ] **Step 1: Write the failing tests** in `tests/closed_tabs_model.rs`.

  Use helpers like the ones in `editor_groups_model.rs` (`p`, `tabs`). Each test builds a model with `open(p("a.rs"), OpenMode::Pinned, true)` and so on.
  - `close_records_entry`: after closing `b` in [a,b,c] (b active), `closed().len() == 1`, entry `index == 1`, `was_active == true`, `item == TabItem::File(p("b.rs"))`.
  - `history_caps_at_20`: open and close 25 files; `closed().len() == 20`; the oldest kept is file #5.
  - `reclosing_same_item_in_group_keeps_one_entry`: close a, reopen, close a again → 1 entry.
  - `not_recorded`, one assertion per case, each leaving `closed()` empty:
    - a preview replaced by a preview open;
    - `replace_item(id, TabItem::MarkdownPreview(..))`;
    - closing `TabItem::File(untitled::path(1))`;
    - closing `open_search()`;
    - `remove(id)`.
  - `close_many_is_one_batch_with_pre_batch_indexes`: [A,B,C,D], `close_many(&[a,b,d])` → indexes A=0, B=1, D=3, all with the same `batch`.
  - `reopen_restores_batch_in_place`: then `reopen_closed(|_| true)` → tabs `[A,B,C,D]`, all pinned. Active stays C, because no entry was active. Returns 3 ids. `closed()` is empty.
  - `reopen_single_activates_it`: [a,b,c], b active, close b, then activate a. After reopen, active is b and the order is [a,b,c].
  - `reopen_goes_to_old_group`: two groups (`split_active`), close a tab in group 0 while group 1 is active. After reopen, the tab is back in group 0, and group 0 is the active group.
  - `reopen_into_missing_group_goes_to_active_group`: two groups with [a,b] and [c]. Close all with `close_many` over every id; the model is left with one empty group. After reopen, the one group holds all three tabs, a and b in index order, with c at `min(0, len)`. Nothing is lost.
  - `reopen_skips_missing_files_and_falls_back_a_batch`:
    - Batch 1 closes x. Batch 2 closes y and z. With `exists = |p| p != "z.rs"`, reopen gives y only.
    - Second case: batch 2 = [z] only. Reopen skips it and reopens x from batch 1.
    - With no history left, reopen returns an empty vec and changes nothing.
  - `reopen_skips_tab_already_open_in_group`: close a, open a again, reopen → nothing new, and the entry is consumed.
  - `retarget_updates_history`: close `/w/a.rs`, then `retarget("/w/a.rs", "/w/b.rs")`; the entry's item is `File("/w/b.rs")`.
  - `set_closed_view_attaches_state`: close a (tab id `t`), then `set_closed_view(t, TabViewState{cursor: Point::new(3,2), scroll_top_row: 1})`; reopen returns `Some(that state)`.

- [ ] **Step 2: Run them to confirm they fail.**

  Run: `PATH="/opt/homebrew/opt/rustup/bin:$PATH" cargo test -p lucerna-workbench --test closed_tabs_model`

  Expected: compile errors (`ClosedTab`, `close_many`, … not found).

- [ ] **Step 3: Implement `closed.rs` and the model methods.**
  - `ClosedHistory::record(entry)`:
    - drop non-recordable items, which are anything other than `File` or `MarkdownPreview`, and untitled paths;
    - remove any existing entry with the same `item` and `group`;
    - push to the back, then `pop_front` while the length is over 20.
  - `close_many`:
    - takes `batch = next_batch`;
    - before closing anything, collects `(id, group id, index, was_active)` for each live id;
    - then closes each through the internal remove;
    - then records each entry in `ids` order.
  - `close(id)` is `close_many(&[id])`.
  - `replace_item` uses `remove` where it closes a duplicate.
  - `retarget` also rewrites paths in the history with `crate::workspace_state::moved_path`.
  - `reopen_closed`:
    - pop the newest batch;
    - filter it: `exists(path)` is checked for path items; drop an entry when the target group already has a tab with that item;
    - if nothing is left, loop to the previous batch;
    - insert per target group in ascending `index` at `min(index, len)`, pinned, with new ids;
    - add each new tab to its group's MRU: behind the active tab, or at the front for `was_active`;
    - apply the activation rule from the spec (§"History in the model", step 4).

    Target group: the group whose `id == entry.group`, else the active group.

    Inserting requires `Group::tabs.insert` and `mru` handling; keep that in `model.rs`.
  - Switch the three programmatic closes to `remove`:
    - `Workbench::close_file_tabs` (around line 1405);
    - the deleted-paths close (around line 1449);
    - `finish_onboarding` in `onboarding/mod.rs:700`.

- [ ] **Step 4: Run the tests and confirm they pass.**

  Run:
  ```bash
  scripts/prune-target.sh && PATH="/opt/homebrew/opt/rustup/bin:$PATH" cargo test -p lucerna-workbench --test closed_tabs_model --test editor_groups_model
  ```
  Expected: all PASS. The existing model tests are unchanged.

- [ ] **Step 5: Commit**
  ```bash
  git add crates/lucerna-workbench/src/editor_groups/closed.rs crates/lucerna-workbench/tests/closed_tabs_model.rs
  git commit -m "feat(tabs): closed-tab history in the editor groups model" -- crates/lucerna-workbench/src/editor_groups/closed.rs crates/lucerna-workbench/src/editor_groups/model.rs crates/lucerna-workbench/src/editor_groups/mod.rs crates/lucerna-workbench/src/lib.rs crates/lucerna-workbench/src/workbench.rs crates/lucerna-workbench/src/onboarding/mod.rs crates/lucerna-workbench/tests/closed_tabs_model.rs
  ```

### Task 2: Tab sets for the close commands

**Files:**
- Modify: `crates/lucerna-workbench/src/editor_groups/model.rs`
- Test: `crates/lucerna-workbench/tests/closed_tabs_model.rs`

**Interfaces:**
- Produces on `EditorGroups` (each returns tab ids in tab order; an empty vec when `target` is not a live tab):
  - `pub fn others(&self, target: TabId) -> Vec<TabId>`
  - `pub fn to_the_right(&self, target: TabId) -> Vec<TabId>`
  - `pub fn to_the_left(&self, target: TabId) -> Vec<TabId>`
  - `pub fn group_tabs(&self, target: TabId) -> Vec<TabId>`: every tab of `target`'s group, `target` included.
- `pub fn activate(&mut self, id)` already exists and moves the active group too. Task 4 calls `pin(t)` and `activate(t)` for Others, Left and Right.

- [ ] **Step 1: Write the failing test `tab_sets`.**

  [a,b,c,d] in group 0 and [e] in group 1, target c. Expected:
  - `others(c) == [a,b,d]`
  - `to_the_right(c) == [d]`
  - `to_the_left(c) == [a,b]`
  - `group_tabs(c) == [a,b,c,d]`
  - `to_the_right(d)` is empty
  - `others(e)` is empty

  For an id that was closed, all four return empty.
- [ ] **Step 2: Run it and confirm it fails** (`--test closed_tabs_model tab_sets`; the methods do not exist yet).
- [ ] **Step 3: Implement the four methods** using `group_of` and `position`.
- [ ] **Step 4: Run it and confirm it passes.**
- [ ] **Step 5: Commit**
  ```bash
  git commit -m "feat(tabs): tab sets for close commands" -- crates/lucerna-workbench/src/editor_groups/model.rs crates/lucerna-workbench/tests/closed_tabs_model.rs
  ```

### Task 3: One close flow, with view state captured

**Files:**
- Modify: `crates/lucerna-workbench/src/editor_groups/view.rs` (`update_model`, `request_close`, `request_close_all`, `last_tab_of_dirty`)
- Test: `crates/lucerna-workbench/tests/tab_close_commands.rs` (new). Copy the `init` / `editor` / `settle` / `run` helpers you need from `tests/save.rs`; temp folder files make the documents real and dirty-able.

**Interfaces:**
- Consumes: `close_many`, `closed`, `set_closed_view` (Task 1).
- Produces on `EditorGroupsView`:
  - `pub fn request_close_tabs(&mut self, ids: Vec<TabId>, window: &mut Window, cx: &mut Context<Self>)`: no-op on an empty vec.
  - `request_close(id)` = `request_close_tabs(vec![id])`.
  - `request_close_all` = `request_close_tabs(every id)`.

- [ ] **Step 1: Write the failing tests.**

  Each test uses files in a `TempDir` with two or three `.txt` files open pinned in one group. A file is made dirty by typing into its focused view.
  - `closing_tab_captures_cursor_and_scroll`: put the cursor at (5,0) and set the scroll top row to 2 (`set_cursor` and `set_scroll_top_row` on the `CodeView`); then run `closeActiveEditor`. Read the model through `wb.read(cx).editor_groups().read(cx).model().closed()`. The last entry has `view == Some(TabViewState{cursor: Point::new(5,0), scroll_top_row: 2})`.
  - `close_many_dirty_asks_once`:
    - a and b are dirty, c is clean. Call `request_close_tabs([a,b,c])` through the view.
    - `cx.pending_prompt()` message is "Do you want to save the changes to the following 2 files?".
    - Answer "Save All": both files are on disk, every tab is closed, and `closed()` has one batch of 3.
  - `dont_save_closes_and_discards`: same setup, answer "Don't Save". Tabs are closed, the documents are no longer dirty, and the disk is unchanged.
  - `cancel_closes_nothing_and_records_nothing`: same setup, answer "Cancel". All 3 tabs stay and `closed()` is empty.
  - `close_others_does_not_ask_for_document_shown_elsewhere`: a is dirty, `split_active` puts a copy of a in group 1. Then `request_close_tabs` on group 0's tabs: no `pending_prompt()`, group 0's tabs close, and a is still dirty in group 1.
  - The existing single-tab close-prompt tests in `tests/save.rs`, `tests/untitled.rs` and `tests/hot_exit.rs` must still pass unchanged.
- [ ] **Step 2: Run the new tests and confirm they fail.**

  Run: `PATH="/opt/homebrew/opt/rustup/bin:$PATH" cargo test -p lucerna-workbench --test tab_close_commands`

  Expected: FAIL. `request_close_tabs` is missing and `view` is `None`.
- [ ] **Step 3: Implement.**
  - **`update_model`:**
    - Before calling `f`, note `self.model.closed().back().map(|e| (e.batch, e.tab))` and the count.
    - After `f`, for every entry newer than that, take `self.bodies.get(&entry.tab)` and try `downcast::<CodeView>()`.
    - If it is a `CodeView`, call `set_closed_view(entry.tab, TabViewState{cursor: v.cursor(), scroll_top_row: v.scroll_top_row()})`. Bodies are still present here, because they are only pruned in `render`.
    - Compare by tab id, not by count, because the history may have hit the cap.
  - **`request_close_tabs`:**
    - Generalize `last_tab_of_dirty`: the paths to ask about are the dirty paths of tabs in `ids` that no tab outside `ids` shows. Sort and dedup them.
    - Then follow the existing `request_close_all` body, with `close_many(&ids)` in place of the loop of `close`.
    - Ask with `save::prompt_unsaved_all`.
    - Delete `last_tab_of_dirty` if nothing else uses it.
- [ ] **Step 4: Run the tests and confirm they pass.**

  Run:
  ```bash
  scripts/prune-target.sh && PATH="/opt/homebrew/opt/rustup/bin:$PATH" cargo test -p lucerna-workbench --test tab_close_commands --test save --test untitled --test hot_exit --test vim_command_line
  ```
  Expected: all PASS.
- [ ] **Step 5: Commit**
  ```bash
  git add crates/lucerna-workbench/tests/tab_close_commands.rs
  git commit -m "feat(tabs): one close flow for many tabs, remember cursor and scroll" -- crates/lucerna-workbench/src/editor_groups/view.rs crates/lucerna-workbench/tests/tab_close_commands.rs
  ```

### Task 4: Commands, keys and the File menu

**Files:**
- Modify: `crates/lucerna-workbench/src/editor_groups/mod.rs` (`register_commands`)
- Modify: `crates/lucerna-workbench/src/workbench.rs` (new `reopen_closed_editor`, next to `navigate`)
- Modify: `crates/lucerna-settings/assets/default-keybindings.json` (one block appended at the end)
- Modify: `crates/lucerna-workbench/src/app_menu.rs` (one line after "Close Editor")
- Test: `crates/lucerna-workbench/tests/tab_close_commands.rs`

**Interfaces:**
- Consumes:
  - `reopen_closed` and `tab_from_u64` (Task 1);
  - `others`, `to_the_right`, `to_the_left`, `group_tabs` (Task 2);
  - `request_close_tabs` (Task 3).
- Produces: commands registered under the ids and titles from Global Constraints.
  - Each close command, and `closeActiveEditor` too, takes optional args `{"tab": <u64>}`.
  - Target: the tab named by `args["tab"]` via `tab_from_u64`, else the active tab of the active group, else nothing (no-op).
  - `pub(crate) fn reopen_closed_editor(&mut self, cx: &mut Context<Self>)` on `Workbench`.

- [ ] **Step 1: Write the failing tests.**
  - `reopen_restores_tab_cursor_and_scroll`:
    - Open a.txt (30 lines), b.txt and c.txt, with b active. In b, set the cursor to (12,3) and the scroll top row to 4.
    - `closeActiveEditor`, then `keys("cmd-shift-t")`.
    - Tabs are [a,b,c] and b is active.
    - b's new `CodeView` (fetched after `settle`) has `cursor() == Point::new(12,3)` and `scroll_top_row() == 4`.
  - `reopen_walks_back_and_stops`: close c, then b. ⇧⌘T → b is back. ⇧⌘T → c is back. A third ⇧⌘T changes nothing.
  - `reopen_skips_deleted_file`: close b, `fs::remove_file(b.txt)`, then ⇧⌘T. No b tab appears.
  - `close_others_key_and_pin`:
    - Open a pinned, b pinned, and c as a preview (`OpenMode::Preview`), with c active. Press `keys("alt-cmd-t")`.
    - Only c is left, it is pinned, and it is active.
    - ⇧⌘T then reopens a and b at indexes 0 and 1, and c stays active.
  - `close_to_right_and_left_with_tab_arg`:
    - [a,b,c,d] with d active. `run_command("workbench.action.closeEditorsToTheRight", Some(&json!({"tab": b_raw})))` → [a,b], and b is active.
    - Same setup with `closeEditorsToTheLeft` on c → [c,d], and c is active.
  - `close_saved_keeps_dirty`: a is dirty, b and c are clean. `keys("cmd-k u")` → only a remains. There is no prompt.
  - `close_group_key`: two groups. `keys("cmd-k w")` in group 1 closes only group 1's tabs, and group 1 is removed. `keys("cmd-k cmd-w")` closes everything.
  - `registers_commands`: every new id is in the registry with its exact title. Check this the way `registers_editor_group_commands` in `tests/editor_groups_view.rs` does.
- [ ] **Step 2: Run the tests and confirm they fail** (unknown commands, no bindings).
- [ ] **Step 3: Implement.**
  - **`reopen_closed_editor`:**
    - Call `update_editor_groups(cx, |m| m.reopen_closed(|p| p.exists()))`. That is one `stat` per entry on the key press, the same as `navigate`.
    - For each returned `(id, Some(view))` whose tab shows a `File`: if the tab is the active one and its body exists, call `docs.go_to_and_scroll(&view, cursor, row, cx)`. Otherwise call `docs.go_to_and_scroll_when_opened(path, cursor, row)`.
    - Then `focus_editor_area`.
  - **Close command handlers:**
    1. Resolve the target.
    2. For Others, Left and Right: `pin(t)` then `activate(t)` through `update_editor_groups`.
    3. Compute the set:

       | Command | Set |
       |---|---|
       | Others | `others(t)` |
       | Right | `to_the_right(t)` |
       | Left | `to_the_left(t)` |
       | Close Saved | `group_tabs(t)` filtered to tabs whose `path()` is not dirty in `Documents`; non-file tabs count as saved |
       | In Group | `group_tabs(t)` |

    4. Call `request_close_tabs`.
  - **Keybindings:** append this block at the end of `default-keybindings.json`. Add a trailing comma on the previous last row.
    ```json
    { "key": "shift+cmd+t", "command": "workbench.action.reopenClosedEditor" },
    { "key": "alt+cmd+t", "command": "workbench.action.closeOtherEditors" },
    { "key": "cmd+k u", "command": "workbench.action.closeUnmodifiedEditors" },
    { "key": "cmd+k w", "command": "workbench.action.closeEditorsInGroup" },
    { "key": "cmd+k cmd+w", "command": "workbench.action.closeAllEditors" }
    ```
  - **File menu:** add `command("Reopen Closed Editor", "workbench.action.reopenClosedEditor")` right after the "Close Editor" entry.
- [ ] **Step 4: Run the tests and confirm they pass.**

  Run:
  ```bash
  scripts/prune-target.sh && PATH="/opt/homebrew/opt/rustup/bin:$PATH" cargo test -p lucerna-workbench --test tab_close_commands --test editor_groups_view --test app_menu
  ```
  Expected: all PASS. If `tests/app_menu.rs` asserts the File menu's exact entries, add the new entry to that expectation.
- [ ] **Step 5: Commit**
  ```bash
  git commit -m "feat(tabs): reopen closed editor and close-others commands with keys" -- crates/lucerna-workbench/src/editor_groups/mod.rs crates/lucerna-workbench/src/workbench.rs crates/lucerna-settings/assets/default-keybindings.json crates/lucerna-workbench/src/app_menu.rs crates/lucerna-workbench/tests/tab_close_commands.rs crates/lucerna-workbench/tests/app_menu.rs
  ```

### Task 5: Right-click menu on tabs

**Files:**
- Modify: `crates/lucerna-workbench/src/editor_groups/view.rs` (field `tab_menu: Option<DeployedMenu>`, `menu_generation: u64`, right-mouse handler on each tab, `render_deployed` in `render`)
- Test: `crates/lucerna-workbench/tests/tab_close_commands.rs`

**Interfaces:**
- Consumes: the `ContextMenu` builder (`entry_with_args`, `disabled_when`, `separator`), `deploy_context_menu` and `render_deployed` from `crate::ui`. Also the commands from Task 4 and `Workbench::run_command`, reached through `self.workbench` (`WeakEntity<Workbench>`, already set).
- Produces:
  - `pub fn deploy_tab_menu(&mut self, id: TabId, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>)`.
  - `pub fn tab_menu(&self) -> Option<Entity<ContextMenu>>`, for tests.

- [ ] **Step 1: Write the failing tests.**
  - `tab_menu_items_and_disabled_states`: [a,b,c] in one group. Right-click tab 0 (`cx.simulate_mouse_down`/`up` with `MouseButton::Right` on the `tab-0-0` selector, or call `deploy_tab_menu` directly). The menu `items()`, as `(label, command, disabled)` with separators, is:
    ```
    ("Close","workbench.action.closeActiveEditor",false), ("Close Others","workbench.action.closeOtherEditors",false), Sep,
    ("Close to the Left","workbench.action.closeEditorsToTheLeft",true), ("Close to the Right","workbench.action.closeEditorsToTheRight",false), Sep,
    ("Close Saved","workbench.action.closeUnmodifiedEditors",false), ("Close All","workbench.action.closeEditorsInGroup",false)
    ```
    Each entry's args are `{"tab": <id of a>}`. The right-click did not change the active tab.
  - With a single tab, "Close Others", "Close to the Left" and "Close to the Right" are all disabled.
  - `menu_targets_right_clicked_tab_in_other_group`:
    - Groups [a,b] and [c,d], group 1 active with d. Open the menu on tab a (group 0), choose "Close Others" (select it with keys `down` and `enter`, or emit its `MenuCommand`), then `settle`.
    - Group 0 is [a], group 1 is [c,d], the active group is 0 and the active tab is a.
- [ ] **Step 2: Run the tests and confirm they fail.**
- [ ] **Step 3: Implement.**
  - Copy the Explorer pattern (`explorer/view.rs` around line 1240):
    - keep a generation counter;
    - in `on_close`, clear the menu if the generation matches, then `window.defer` → `workbench.update(|wb| wb.run_command(command, args.as_ref(), window, cx))`.
  - The owner focus handle is the one the tab bar uses. If the view has none, pass `window.focused(cx)` or the editor focus handle the Workbench uses.
  - Attach `.on_mouse_down(MouseButton::Right, …)` to each tab next to the existing middle-click handler, with the position from the event. Do not change the active tab.
- [ ] **Step 4: Run the tests and confirm they pass**, along with `--test editor_groups_view` and `--test context_menu`.
- [ ] **Step 5: Commit**
  ```bash
  git commit -m "feat(tabs): right-click menu with close commands" -- crates/lucerna-workbench/src/editor_groups/view.rs crates/lucerna-workbench/tests/tab_close_commands.rs
  ```

### Task 6: Docs and full verification

**Files:**
- Modify: `docs/keyboard-shortcuts.md` (rows appended after "Cmd+W | Close tab")
- Modify: `CHANGELOG.md` (under `## [Unreleased]` → `### Added`, appended as the last bullet)
- Modify: `docs/roadmap.md` (`- [ ] Reopen the tab you just closed (⇧⌘T), Close Others` → `- [x]`)

- [ ] **Step 1: Add the docs text.**
  - **Shortcut rows:**
    - `Cmd+Shift+T | Reopen closed tab`
    - `Alt+Cmd+T | Close other tabs`
    - `Cmd+K U | Close saved tabs`
    - `Cmd+K W | Close tabs in group`
    - `Cmd+K Cmd+W | Close all tabs`
  - **CHANGELOG bullet:** "Cmd+Shift+T reopens the last closed tab, in its group and place, with its cursor and scroll; press again to go further back. Close Others (Alt+Cmd+T), Close to the Left/Right, Close Saved (Cmd+K U) and Close All in Group (Cmd+K W) are in the Command Palette and in a new right-click menu on tabs; unsaved files are asked about once for all."
- [ ] **Step 2: Run the full verification.**
  ```bash
  scripts/prune-target.sh && PATH="/opt/homebrew/opt/rustup/bin:$PATH" cargo test -p lucerna-workbench
  ```
  ```bash
  PATH="/opt/homebrew/opt/rustup/bin:$PATH" cargo test -p lucerna-settings
  ```
  ```bash
  PATH="/opt/homebrew/opt/rustup/bin:$PATH" cargo clippy -p lucerna-workbench --all-targets -- -D warnings
  ```
  ```bash
  PATH="/opt/homebrew/opt/rustup/bin:$PATH" cargo fmt --all -- --check
  ```
  ```bash
  PATH="/opt/homebrew/opt/rustup/bin:$PATH" cargo deny check
  ```
  Expected: 0 test failures, no clippy warnings, fmt clean, deny "ok". A failure in a test this branch did not touch: rerun it alone once (the shared target dir can leave stale binaries); if it still fails, check it on `main` in another checkout and report it rather than fixing it here.
- [ ] **Step 3: Commit**
  ```bash
  git commit -m "docs: reopen closed editor and close others" -- docs/keyboard-shortcuts.md CHANGELOG.md docs/roadmap.md
  ```
