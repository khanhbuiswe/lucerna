# Explorer context menu: design

Date: 2026-11-13. Status: approved.

## Intent

Right-clicking a row in the Explorer opens a small menu. The menu has commands for reading code (open to the side, reveal in Finder, open a terminal, search in a folder, copy a path) and commands that edit the file tree (create a file or folder, rename, delete to Trash). The user wants **both kinds**.

This is Lucerna's first right-click menu. The menu component must be a shared component in `ui.rs`, so that the tab, editor and path-bar menus can reuse it later.

## References

The details are in `docs/research/2026-11-13-explorer-context-menu.md`. Summary:

- **VS Code** (`MenuId.ExplorerContext`):
  - Groups in this order: create new → open or reveal → compare → search → cut, copy, paste → copy path → rename, delete.
  - Items that do not apply are hidden.
  - Both create and rename take the name right in the tree.
  - Delete moves the file to the Trash and asks for confirmation first.
  - Commands: `explorer.newFile`, `explorer.newFolder`, `explorer.openToSide`, `revealFileInOS`, `openInTerminal`, `filesExplorer.findInFolder`, `copyFilePath`, `copyRelativeFilePath`, `renameFile`, `moveFileToTrash`, `deleteFile`.
- **Zed** (`project_panel::deploy_context_menu`, `ui::ContextMenu`):
  - The item order is nearly the same as VS Code.
  - The menu is a custom-drawn popover placed at the mouse position (`deferred(anchored())`). It has its own focus and is navigated with `↑↓`, `Enter`, `Esc`.
  - Each item shows its shortcut on the right. The menu is at least 200 px wide.
  - Disabled items are dimmed, and separators use the `border_variant` color.
  - The row that opened the menu stays highlighted until the menu closes.
  - `shift-f10` opens the menu at the selected row.
- **Which one Lucerna follows:**
  - Commands, IDs, keys, settings and behavior follow VS Code.
  - Menu look and drawing follow Zed.
  - Differs from both: "Open in Terminal" opens an **external terminal**, because Lucerna has no integrated terminal. This is also where users run CLI agents.

## Goals

1. A shared `ContextMenu` component in `ui.rs`, drawn after Zed.
2. The Explorer has its own **selected row**, can open the menu with a right-click or `shift+f10`, and has the VS Code shortcuts for the commands below.
3. Read-only commands: Open to the Side, Reveal in Finder, Open in Terminal, Find in Folder…, Copy Path, Copy Relative Path.
4. Editing commands: New File…, New Folder…, Rename…, Delete (move to Trash), and Delete Permanently (shortcut only, not in the menu).

## Non-goals

- Cut, Copy, Paste, Duplicate file; drag and drop; Undo/Redo of file operations.
- Multi-row selection. The menu always acts on exactly one row.
- Moving through the tree with the arrow keys. That is a separate feature, but the selected row is designed so it can be used for that later.
- Compare, Open With…, the Git items, Add/Remove Folder.
- Menus for the tab, the editor and the path bar. The component is ready for these menus, but building them belongs to a later spec.
- A "Do not ask me again" checkbox in the delete dialog. GPUI's native dialog has no checkbox, so users turn it off with the `explorer.confirmDelete` setting.
- Submenus, items with a check mark, headers inside the menu. The component does not need these for now.

## Must not happen (anti-goals)

- Data loss: deleting without asking while `explorer.confirmDelete` is on; deleting permanently when the user only chose Trash; overwriting an existing file when creating or renaming.
- A UI thread that waits on I/O. Every file operation runs on a background thread.
- The tab of a renamed file losing its unsaved edits.
- Lucerna's own operations being marked as "changed externally" in the Explorer (see `2026-11-13-explorer-own-save-decoration-design.md`).
- A VS Code-style or native menu. The menu must look like Zed's.

## Constraints

- Only `lucerna-workbench` may use GPUI. File operations (create, rename, move to Trash, delete) live in `lucerna-project` and have tests that need no UI.
- Moving to the Trash uses the `trash` crate (MIT license; Zed uses it too). Run `cargo deny check` after adding the crate.
- No macOS-only code outside the platform-specific parts:
  - Reveal uses `cx.reveal_path`.
  - The external terminal is chosen per platform, as below.
  - The label shown to the user: "Reveal in Finder" on macOS, "Open Containing Folder" on Linux, "Reveal in File Explorer" on Windows (same as VS Code).
- Don't copy Zed's code, because Zed is GPL. Learn the design only.

## Approaches considered

1. **Follow Zed: the menu is an entity with its own focus, and the name is typed right in the tree.** This is the chosen approach. It matches both VS Code and Zed, the user sees where the new file will go, and the menu component is reusable.
2. Same as approach 1, but type the name in a popover at the top of the window, Quick Open style. Dropped because it differs from both. It is lighter only because it avoids inserting an input box into the virtualized list.
3. Use the operating system's native menu. Dropped because GPUI does not support native right-click menus, and this approach cannot be reused for tabs or the editor.

The user chose approach 1. The work is split into two parts, done in order, each with its own plan:

- **Part 1:** the menu component, the selected row, the read-only commands.
- **Part 2:** New File/Folder, Rename, Delete.

## Design

### 1. The `ContextMenu` component (`ui.rs`)

**Data.** A GPUI entity with its own `FocusHandle` that emits `DismissEvent`. The menu is a list of items, each one of two kinds:

- `Entry { label, command: &'static str, args: Option<Value>, disabled: bool }`
- `Separator`

An item runs a command by its **`CommandRegistry` command ID**, not a GPUI action. This way the same command runs from the menu, from a shortcut and from the Command Palette.

The builder has `entry(label, command)`, `entry_with_args(...)`, `disabled_when(cond)`, `separator()`, `when(cond, f)`. The builder drops a separator at the start of the menu, at the end of the menu, and the second of two adjacent separators, so call sites can use `when` freely.

**Showing.** The owning view keeps the menu in `Option<DeployedMenu { menu, position, _subscription }>`. When the menu emits `DismissEvent`, the view sets it back to `None`.

The menu is drawn with `deferred(anchored().position(pos).child(menu))`. `anchored` moves the menu automatically so it does not overflow the window, and the menu stays at least 8 px from the window edge. A shared function `deploy_context_menu(...)` lets any view open a menu with a single call.

**Look** (after Zed, using Lucerna's measurements):

- The frame uses the background, border and shadow of `popover()`, but is not limited by `POPOVER_MAX_WIDTH`. At least 200 px wide. At most 75% of the window height, scrollable when longer. Padding in `GRID`.
- Each item is 6 × `GRID` tall, with horizontal padding of 2 × `GRID` and a corner radius of `GRID`. The label uses `UI_FONT_SIZE_SMALL` and the `text` color. The shortcut sits on the right in `text_muted`, taken from `keymap.label_for(command)` like `list_button`.
  - Hover or the item selected by keyboard: `ghost_element_hover` background.
  - Disabled item: `text_disabled` color, no hover, clicking does nothing.
- Separator: 1 px tall, `border_variant` color, `GRID` of space above and below.
- The menu has no icons yet. Leave room to add optional icons later.

**Interaction:**

- When it first opens, no item is selected. `↓` selects the first item, `↑` selects the last. Moving skips separators and disabled items. `home` and `end` work.
- `Enter` runs the selected item. A left-click on an item also runs it.
- `Esc` closes the menu. Clicking outside the menu (left or right button) also closes it.
- Running an item first closes the menu, returns focus to the view that opened it, and only then runs the command. This way commands that need the Explorer to have focus (such as Rename) still work.
- When the menu closes without running a command, focus returns to the view that opened it. But only if the menu (or nothing) still holds focus, as in Zed: clicking the editor to close the menu leaves focus in the editor, and the Explorer does not take it back.
- The menu closes when focus leaves it, for example when opening Quick Open (`cmd+p`), the Command Palette (`cmd+shift+p`) or hiding the sidebar (`cmd+b`).
- Right-clicking elsewhere while a menu is open closes the old menu and opens a new one.

**Keys inside the menu are handled by the menu itself**, in its own key handler, the same way the Quick Open input receives keys: `up`, `down`, `home`, `end`, `enter`, `escape`, with no modifiers. Keys with `cmd`, `ctrl`, `alt` or `shift` pass on to the workbench keymap. There is a new context key `contextMenuVisible`. While the menu is open, or while a name is being typed in the tree, `filesExplorerFocus` is false, so `enter` does not run Rename and `cmd+backspace` does not delete.

### 2. The selected row and Explorer focus

`ExplorerModel` gets `selected: Option<Arc<Path>>`, a path relative to the root. `None` means the root folder is selected.

**Updating:**

- A left-click on a row selects that row, then does what it does today (opens the file, or expands/collapses the folder).
- A right-click on a row selects that row and opens its menu.
- A right-click on the root row or the empty space below the tree selects the root folder and opens the root menu.
- When the open tab changes, the selected row follows that tab, as in VS Code. Only when `explorer.autoReveal` is on; when it is off, the selected row stays put.
- When the selected row disappears from the tree (deleted, or its parent folder collapsed), select the nearest parent folder that is still visible.

**Drawing:**

- The selected row has the `element_selected` background, replacing today's highlighting by the open tab. Normally the two coincide.
- While a menu is open, the row that owns the menu also gets a 1 px `border_focused` border, inside the row, as in Zed. The border disappears when the menu closes.
- The root row: clicking it does nothing today. Now clicking it selects the root, and the root row is also highlighted while selected.

**Focus.** The sidebar already has a focus handle (`sidebar_focus`). These context keys are added:

- `filesExplorerFocus` = the sidebar has focus, the sidebar is showing the Explorer, no context menu is open and no name is being typed in the tree.
- `explorerViewletFocus` stays as is, equal to `sideBarFocus`: the sidebar now holds only the Explorer (Search and Extensions became tabs), so the two keys mean the same thing.
- `explorerResourceIsFolder` = the selected row is a folder, or the root.
- `explorerResourceIsRoot` = the root is selected.

The key names follow VS Code. Clicking a row moves focus into the sidebar. This needs rechecking, because clicking a row may not take focus today.

**Single-file mode** (opening a lone file, see `2026-11-13-single-file-open-design.md`): in this mode the worktree does not scan or watch a folder. So every menu here, including a right-click on the root row or the empty space, acts on **the file itself**, not on its parent folder. The menu has only Open to the Side, Reveal in Finder, Open in Terminal (opens at the parent folder), Copy Path and Copy Relative Path (copies the file name). No New File/Folder, Find in Folder, Rename or Delete. This is Claude's decision.

### 3. Commands and menu

Every command is registered in `CommandRegistry` with the VS Code ID. A command acts on the Explorer's **selected row**. When it receives `args` of `{ "path": … }`, the command acts on that path instead, for reuse by the tab menu later. Every menu item carries the `args.path` of the row that opened the menu (`""` for the root), so the command's target is fixed from the moment the menu opens, even if the row disappears while the menu is open.

Names in the Command Palette: Copy Path and Copy Relative Path are "Explorer: Copy Path" and "Explorer: Copy Relative Path", because they act on the selected row, not the open file. Reveal uses the platform label: "File: Reveal in Finder" (macOS), "File: Reveal in File Explorer" (Windows), "File: Open Containing Folder" (Linux).

Running New File, New Folder, Rename or Open Context Menu from the Command Palette while the sidebar is hidden shows the Explorer first (VS Code also focuses the Explorer), so the input box or menu does not hold focus without being drawn. Hiding the sidebar cancels any name being typed and closes the menu.

A row's menu has the order below. Items that do not apply are hidden, as in VS Code. "—" is a separator.

| Item | ID | Shown when | Default key (macOS; Linux and Windows follow VS Code) | Kind |
|---|---|---|---|---|
| New File… | `explorer.newFile` | folder or root | — | edit |
| New Folder… | `explorer.newFolder` | folder or root | — | edit |
| — | | | | |
| Open to the Side | `explorer.openToSide` | file | `ctrl+enter` (Linux/Windows: `ctrl+enter`) | read |
| Reveal in Finder | `revealFileInOS` | always | `alt+cmd+r` (Linux: `ctrl+alt+r`; Windows: `shift+alt+r`) | read |
| Open in Terminal | `openInTerminal` | always | — | read |
| — | | | | |
| Find in Folder… | `filesExplorer.findInFolder` | folder or root | `alt+shift+f` | read |
| — | | | | |
| Copy Path | `copyFilePath` | always | `alt+cmd+c` (Linux: `ctrl+alt+c`; Windows: `shift+alt+c`) | read |
| Copy Relative Path | `copyRelativeFilePath` | always | `alt+cmd+shift+c` (Linux: `ctrl+shift+alt+c`; Windows: `ctrl+k ctrl+shift+c`) | read |
| — | | | | |
| Rename… | `renameFile` | not the root | `enter` (Linux/Windows: `f2`) | edit |
| Delete | `moveFileToTrash` | not the root | `cmd+backspace` (Linux/Windows: `delete`) | edit |

Other keys:

- `deleteFile` (Delete Permanently): `alt+cmd+backspace` / `shift+delete`. Not in the menu.
- Opening the menu by key: `shift+f10`, ID `filesExplorer.openContextMenu`. This is my own proposal, because VS Code has no separate ID for this. The menu opens right below the selected row. The key works in both VS Code and Zed.

Every Explorer key has the `filesExplorerFocus` condition. VS Code lets Reveal, Copy Path and Copy Relative Path run even when the editor does not have focus (`!editorFocus`), and takes the file from the open tab. Lucerna narrows this to run only inside the Explorer; widening it is left for when the tab menu is built. Add these conditions:

- Rename and Delete: `&& !explorerResourceIsRoot`.
- Open to the Side: `&& !explorerResourceIsFolder`.
- Find in Folder: `&& explorerResourceIsFolder`.
- Rename (the `enter` key): add `&& !textInputFocus`, so that while a name is being typed `enter` does not trigger Rename. The `textInputFocus` key already exists, but is computed per frame, from the previous frame. The implementer must make sure an input that has just received focus does not let `enter` slip through to Rename, for example by having the Explorer check its own `editing` state.

The keys are added to `crates/lucerna-settings/assets/default-keybindings.json`.

How each command runs:

- **Open to the Side:** `open_path(path, OpenMode::ToSide)`.
- **Reveal in Finder:** `cx.reveal_path(abs)`. This opens Finder with the file pre-selected.
- **Open in Terminal:** opens at that folder, or at the containing folder if it is a file. It runs according to the platform's setting:
  - macOS: `terminal.external.osxExec` (default `Terminal.app`), runs `open -a <app> <dir>`.
  - Linux: `terminal.external.linuxExec` (default is the system terminal, for example `x-terminal-emulator`), spawned with `cwd = dir`.
  - Windows: `terminal.external.windowsExec`, reserved for the Windows version.

  Lucerna waits for the program that opens the terminal to finish (on a background thread), like VS Code. If it cannot run or it exits with a non-zero code (for example `open -a` with a wrong app name), report it with `status_message`: "Couldn't open the terminal: …".
- **Find in Folder…:** opens or brings the Search tab to the front, fills the include box with the folder's relative path (`src/foo`, like VS Code; leave it empty for the root), then focuses the query box.
  - An include pattern with no glob characters is a path, and also matches `<path>/**`, like VS Code.
  - Folder names with special characters are escaped: `[ ] { } * ? ,` are wrapped into a character class (`app/[slug]` becomes `app/[[]slug[]]`), a leading `!` or `#` gets a `\` added, then `/**` is appended.
  - The include box splits patterns on commas, except commas inside `[]` or `{}`, like VS Code's `splitGlobAware`.
- **Copy Path:** copies the absolute path to the clipboard.
- **Copy Relative Path:** copies the path relative to the root. The separator follows `explorer.copyRelativePathSeparator`: `auto`, `/` or `\`, default `auto` (`/` on macOS and Linux). For the root it copies the root folder's name, like VS Code.

### 4. Typing the name in the tree (New File, New Folder, Rename)

`ExplorerModel` gets an `editing: Option<EditState>` state:

```
EditState {
    kind: NewFile | NewFolder | Rename,
    parent: dir,
    original?,
    error: Option<String>,
}
```

- **Position:**
  - New File/Folder: expand the target folder (the root is always expanded), then insert a **virtual row** as that folder's first child. The virtual row has a file or folder icon and a `TextInput`.
  - Rename: replace the row's own name with a `TextInput`. The input is pre-filled with the old name, with the part **before the last extension** preselected (for `foo.rs`, `foo` is selected). For a folder the whole name is selected.
  - The virtual row stays inside the `uniform_list`, and the row height stays `TREE_ROW_HEIGHT`. The input uses Lucerna's `TextInput`, with a 1 px `border_focused` border, no rounded corners, and a height that fits the row.
- **Confirm and cancel:**
  - `Enter` confirms.
  - `Esc` cancels.
  - Losing focus: confirm if the name is valid, cancel if not. This is VS Code's behavior.
  - While typing, the Explorer's keys do not run.
- **Name validation** on every keystroke. The error appears right below the input, in a small box with an `error` border and an `elevated_surface` background. While there is an error, `Enter` does nothing. The errors:
  - empty or whitespace-only name;
  - starts with `/`;
  - contains a `.` or `..` component;
  - contains a character that is invalid on the current platform;
  - a file or folder with the same name already exists. Checked against the snapshot, then checked again at the actual write.

  The error messages follow VS Code, for example: "A file or folder **x** already exists at this location. Please choose a different name."
- **Multi-level paths:** `a/b/c.rs` also creates the intermediate folders `a` and `b`, like VS Code and Zed.
- **After creating:**
  - A new file opens as a pinned tab, and its row is selected.
  - A new folder is expanded in the tree and selected.
  - The operation runs on a background thread. The new row appears when the worktree reports a change, and at that point the Explorer selects it and scrolls to it.
- **Rename:**
  - A rename that changes only the letter case (`readme.md` to `README.md`) must work on a case-insensitive disk.
  - Renaming to a name that already exists reports an error and does not overwrite.
  - Every tab that has that file open, or a file inside the renamed folder, moves to the new path. The tab keeps its unsaved edits, cursor position and pinned state. The language and LSP are recomputed for the new name.
- **Write errors** (no permission, read-only disk, …): report with `status_message`, and the virtual row closes. The messages are Lucerna's own: "Unable to create {file|folder} '<name>': <error>" and "Unable to rename '<name>': <error>". If the name was just taken by another program after the check and the input still has focus, the input stays with a duplicate-name error.
- **Edge cases:**
  - When the window loses focus (switching to another app), the input stays open, neither confirmed nor canceled.
  - If the row being typed in scrolls out of the viewport, the input is kept. It ends only when focus really moves elsewhere.
  - If the parent folder of the row being typed in disappears (or the row being renamed disappears), the input closes and nothing is created.
  - Rename waits for the in-progress saves of the documents inside to finish before renaming, so a save does not write back to the old path.
  - Rename rejects a name that an open tab holds (the tab of a file that was deleted on disk), so two tabs never share a path.

### 5. Delete

- **`moveFileToTrash`:**
  - If `explorer.confirmDelete` is on (the default), ask with `window.prompt(Warning, …)`:
    - Title: "Are you sure you want to delete '<name>'?"
    - Detail: "You can restore this file from the Trash." (for a folder: "…this folder and its contents…")
    - Buttons: `Move to Trash`, `Cancel`.
  - Then move it to the Trash on a background thread. On macOS this uses `NSFileManager` (`trashItemAtURL`) instead of the `trash` crate's default of asking Finder through AppleScript: that approach pops up a permission dialog for controlling Finder, and if it is denied every Delete fails.
  - If it cannot be moved to the Trash (network drive, external drive without a Trash…), ask again: "Unable to move '<name>' to the Trash. Do you want to permanently delete instead?" with the detail "This action is irreversible!" plus the operating system's error, and buttons `Delete Permanently`, `Cancel`. This is how VS Code does it.
- **`deleteFile`:** always asks, regardless of the setting: "Are you sure you want to permanently delete '<name>'?" with the detail "This action is irreversible!". Then it deletes with `std::fs::remove_file` or `remove_dir_all`.
- **While deleting:**
  - Holding the Delete key does not delete the neighboring rows one after another: a second Delete command for the same path (or a path inside it, or containing it) is ignored while the earlier one is not finished.
  - Delete waits for the in-progress saves of the documents inside to finish before running, so a save does not write the file back.
  - A delete error is reported with `status_message`: "Unable to delete '<name>': <error>" (Lucerna's own message).
- **After deleting:**
  - The tab of the deleted file, or of a file inside the deleted folder, closes if it has no unsaved edits.
  - A tab with unsaved edits is kept, in the "deleted" state as it is today (`DocState::Deleted`). VS Code asks separately when deleting a file with unsaved edits; Lucerna does not ask again.
  - The selected row moves to the next sibling, or the previous one if there is none, or the parent folder if there is neither. VS Code also focuses a neighboring row. This row is computed before the delete, because the watcher may report the row as gone before the delete command returns its result.
- **Not marking its own operations:** every path that Lucerna just created, renamed or deleted is skipped when the Explorer marks "changed externally". The save mechanism (`documents.follows_disk`) applies only to open documents, so there must also be a separate list of paths that Lucerna is waiting to see change. Each path is removed from the list when the worktree reports it added or modified, and at the latest after a short deadline (5 seconds). A "removed" report does not drop the path, because the old path of a Rename or Delete is reported that way.

### 6. New settings

Names, defaults and semantics follow VS Code:

| Key | Default | Notes |
|---|---|---|
| `explorer.confirmDelete` | `true` | |
| `explorer.copyRelativePathSeparator` | `"auto"` | `"/"`, `"\\"`, `"auto"` |
| `terminal.external.osxExec` | `"Terminal.app"` | |
| `terminal.external.linuxExec` | `"x-terminal-emulator"` | VS Code detects it from the desktop environment, Lucerna uses a fixed value |
| `terminal.external.windowsExec` | `"cmd.exe"` | reserved for the Windows version |

### 7. Side additions needed

- **Theme:** `UiColors` does not yet have `text_disabled` and `border_focused`. Add these two fields, read from the keys `text.disabled` and `border.focused` in a Zed-style theme. If the theme does not have them, use `text_muted` and `border`.
- **`TextInput`:** add an API to set the selection (`set_selection(range)`), so Rename can preselect the part of the name before the file extension.

### 8. Tests

- `lucerna-project`:
  - create a file or folder, including multi-level paths;
  - reject when the name already exists;
  - rename, including a case-only change;
  - move to the Trash (the test runs on a temp folder; if the CI environment has no Trash, test the failure behavior);
  - permanent delete;
  - name validity checks (a pure function).
- `ContextMenu`:
  - the builder drops exactly the redundant separators;
  - `↑` `↓` skip separators and disabled items;
  - `Enter` runs the command and closes the menu;
  - `Esc` or a click outside closes the menu and returns focus;
  - shortcuts display correctly.
- Explorer (GPUI tests with `debug_selector`):
  - right-click selects the row and opens the correct set of items for a file, a folder and the root;
  - the border of the row that owns the menu;
  - `shift+f10`;
  - each read-only command calls the right API;
  - New File: the virtual row, duplicate-name error, `Enter` creates and opens a tab, `Esc` cancels;
  - Rename preselects the name part, the tab follows the new path;
  - Delete: asks again, closes a tab with no edits, keeps a tab with unsaved edits;
  - Lucerna's own operations are not marked as external changes.
- Check the look against Zed per §1.1 of design-principles: run the app next to Zed and compare the menu's and input's spacing, height, background color, border and hover.

## Left to the implementer

- The concrete data types of `ContextMenu`, the builder and `EditState`; how to split files in `explorer/`.
- How to put the virtual row into the `uniform_list`.
- How to move a tab to a new path in `documents.rs` and `editor_groups` (change the key, close and reopen the LSP).
- How to tell the Explorer to skip its own paths: reuse or extend the own-save mechanism.
- The specific tests and their names.
- The boundary between part 1 and part 2 in the plan, as long as part 1 can run on its own.

## Claude's decisions (the user reviews them when approving)

- The set of commands and the order in §3, including Find in Folder…. Cut/Copy/Paste/Duplicate, multi-row selection, drag and drop and Undo are left out.
- Open in Terminal opens an external terminal, because Lucerna has no integrated terminal.
- There is Delete Permanently (`deleteFile`), but only as a shortcut, not in the menu.
- `shift+f10` opens the menu by key, using the ID `filesExplorer.openContextMenu`.
- The selected row follows the open tab. Moving through the tree with the arrow keys is not built yet.
- Keys inside the menu are handled by the menu's key handler, with `contextMenuVisible`; `filesExplorerFocus` is off while the menu is open or a name is being typed. The menu closes when focus leaves it, and returns focus to the Explorer only when the menu still holds focus.
- `explorerViewletFocus` stays equal to `sideBarFocus`, because the sidebar holds only the Explorer.
- The selected row follows the open tab only when `explorer.autoReveal` is on.
- Tree-editing commands and Open Context Menu, when run with the sidebar hidden, show the Explorer first; hiding the sidebar cancels any name being typed and closes the menu.
- Find in Folder escapes glob characters and commas in folder names; the include box does not split on commas inside `[]` and `{}`.
- Open in Terminal waits for the program that opens the terminal and reports an error when it exits with a non-zero code.
- The error messages Lucerna wrote itself: "Unable to create {file|folder} '<name>': <error>", "Unable to rename '<name>': <error>", "Unable to delete '<name>': <error>".
- After a delete, select the next sibling, then the previous one, then the parent folder (replacing the original "parent folder").
- The unsaved tab of a deleted file is kept without asking again, unlike VS Code.
- On macOS, the Trash uses `NSFileManager`, not Finder.
- In single-file mode, the menu acts only on that file and has only the read-only commands.
- Color fallback: `text_disabled` falls back to `text_muted`, `border_focused` falls back to `border`.
