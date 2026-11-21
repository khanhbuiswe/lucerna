# Explorer context menu: VS Code and Zed

Looked up on 2026-11-13. VS Code: branch `main` of `microsoft/vscode`. Zed: the GPUI revision Lucerna pins (`b5cd310`), read in `~/.cargo/git/checkouts/zed-*/b5cd310`.

## 1. VS Code

### Menu entries

The menu is `MenuId.ExplorerContext`. Each entry belongs to a group, and a separator line sits between two groups. The order below is the order shown on macOS, with a single-root project.

| Group | Entry | Command ID | Shown when | Key (macOS) |
|---|---|---|---|---|
| navigation | New File… | `explorer.newFile` | is a folder | — |
| navigation | New Folder… | `explorer.newFolder` | is a folder | — |
| navigation | Open to the Side | `explorer.openToSide` | is a file | `ctrl+enter` |
| navigation | Open With… | `explorer.openWith` | is a file and several editors exist | — |
| navigation | Reveal in Finder | `revealFileInOS` | always | `alt+cmd+r` |
| navigation | Open in Integrated Terminal | `openInIntegratedTerminal` | always | — |
| navigation | Open in External Terminal | `openInTerminal` | `terminal.explorerKind` is `external` or `both` | — |
| 2_workspace | Add Folder to Workspace… / Remove Folder from Workspace | | is a root folder | — |
| 3_compare | Select for Compare / Compare with Selected | | is a file | — |
| 4_search | Find in Folder… | `filesExplorer.findInFolder` | is a folder | `alt+shift+f` |
| 5_cutcopypaste | Cut / Copy / Paste | `filesExplorer.cut` / `.copy` / `.paste` | not a root | `cmd+x` / `cmd+c` / `cmd+v` |
| 6_copypath | Copy Path | `copyFilePath` | always | `alt+cmd+c` |
| 6_copypath | Copy Relative Path | `copyRelativeFilePath` | always | `alt+cmd+shift+c` |
| 7_modification | Rename… | `renameFile` | not a root | `enter` (Windows/Linux: `f2`) |
| 7_modification | Delete | `moveFileToTrash` (Delete Permanently is `deleteFile`, shown only when moving to Trash is not possible) | not a root | `cmd+backspace`; permanent delete `alt+cmd+backspace` |

All the keys above work only when the Explorer has focus (`filesExplorerFocus`).

### Behavior

- **Right-clicking** selects and focuses the row just clicked. The menu acts on that row. If several rows are selected, it acts on all of them.
- **Right-clicking empty space** below the tree opens the root folder's menu.
- **New File / New Folder** insert a name input right in the tree, under the target folder. `Enter` creates, `Escape` cancels. Typing `a/b/c.txt` also creates the folders in between. A duplicate or invalid name is reported right below the input.
- **Rename** turns that row into an input, with the name part preselected but not the file extension. The open tab for that file follows the new name.
- **Delete** asks again: "Are you sure you want to delete 'x'? You can restore this file from the Trash." There is a "Do not ask me again" checkbox, saved to `explorer.confirmDelete`. The tab of a deleted file closes if the file has no unsaved changes.
- **Copy Path** copies the absolute path. **Copy Relative Path** copies the path relative to the root folder, separated by `/` or `\` according to `explorer.copyRelativePathSeparator` (default `auto`).
- **Open in External Terminal** opens the terminal app per `terminal.external.osxExec` (default `Terminal.app`) at that folder. For a file, it opens at the folder containing it.
- If a row does not qualify, the entry is hidden (`when`), not dimmed. Dimming (`precondition`) is used only when the folder is read-only.

### Related settings

`explorer.confirmDelete` (true), `explorer.enableUndo`, `explorer.copyRelativePathSeparator` (`auto`), `terminal.explorerKind` (`integrated`), `terminal.external.osxExec` (`Terminal.app`), `terminal.external.linuxExec`, `terminal.external.windowsExec`.

## 2. Zed

### The `ContextMenu` component (`crates/ui/src/components/context_menu.rs`, GPL)

- It is its own entity, with its own focus handle, and emits `DismissEvent`. The owning view holds the menu in `Option<DeployedContextMenu { menu, position, anchor, _subscription }>`. On receiving `DismissEvent` it resets to `None`.
- It is drawn with `deferred(anchored().position(pos).child(menu)).with_priority(3)`. This keeps the menu above everything else. `anchored` flips or shifts the menu by itself so it does not overflow the window (`snap_to_window_with_margin(8px)` for submenus).
- The builder has the functions `.action(label, Box<dyn Action>)`, `.entry(label, action, handler)`, `.separator()`, `.header(title)`, `.action_disabled_when(cond, …)`, `.when(cond, |m| …)` and submenus. Each entry shows its shortcut on the right by itself, taken from the keymap via `KeyBinding::for_action_in(action, focus)`.
- Styling includes `elevation_2`: background `elevated_surface.background`, border `border`, `rounded_lg` corners, with a drop shadow. The menu is at least 200 px wide and at most 75% of the window tall (scrollable when longer). Each item is a Dense `ListItem`, horizontal padding about 6 px, gap 1.5 between icon, label and key. The separator line is 1 px tall, color `border_variant`, with about 6 px spacing above and below. A disabled item uses the `text.disabled` color.
- Keys in the menu: `up` / `down` to select (skipping separators and disabled items), `home` / `end`, `enter` to run, `escape` to close. Clicking outside the menu (`on_mouse_down_out`) also closes it. Right after opening, no item is selected; pressing `down` selects the first item.
- Running an item closes the menu, then the action is dispatched back to the view that opened the menu (`menu.context(focus_handle)`).

### The Project Panel menu (`crates/project_panel/src/project_panel.rs`, `deploy_context_menu`)

Order when the project is writable:

1. New File, New Folder
2. Reveal in Finder, Open in Default App, Open in Terminal, (Open Markdown/SVG Preview)
3. Find in Folder… (folder), Fold/Unfold Directory
4. Compare Marked Files
5. Cut, Copy, Duplicate, Paste (disabled when the clipboard is empty), Undo, Redo
6. Copy Path, Copy Relative Path
7. Git: Restore File, Add to .gitignore, View History, Permalink…
8. Rename, Trash, Delete (not on the root folder)
9. Root: Add Folders to Project…, Remove from Project
10. Folder: Expand All, Collapse All

When the project is read-only, the menu keeps only Open Markdown Preview, Open SVG Preview and Search Inside.

- **Right-clicking** selects that row, then opens the menu at the mouse position. `shift-f10` opens the menu right at the selected row, using `context_menu_placement_for_entry`.
- The row that opened the menu stays highlighted until the menu closes.
- Zed's **Open in Terminal** opens the integrated terminal. Zed has no option to open an external terminal.
- Keys in `ProjectPanel` (macOS): `cmd-n` New File, `alt-cmd-n` New Folder, `cmd-alt-c` Copy Path, `alt-cmd-shift-c` Copy Relative Path, `enter` / `f2` Rename, `backspace` / `delete` Trash (with a prompt), `cmd-backspace` Trash (no prompt), `cmd-delete` Delete, `alt-cmd-r` Reveal in Finder, `ctrl-shift-enter` open with the default app, `shift-f10` open the context menu, `escape` cancel.

## 3. Similarities and differences

| Aspect | VS Code | Zed |
|---|---|---|
| Groups and order | Create new → open or reveal → search → cut, copy, paste → copy path → modify | Almost identical |
| Entries that do not apply | Hidden | Hidden. Paste, Undo, Redo are dimmed |
| Copy Path key | `alt+cmd+c` | `cmd-alt-c`, same as VS Code |
| Rename key | `enter` | `enter` or `f2` |
| Delete key | `cmd+backspace` moves to Trash and asks again | `backspace` asks again; `cmd-backspace` does not ask |
| Terminal | Both integrated and external terminals | Integrated terminal only |
| Menu look | The OS's native menu on macOS (`window.titleBarStyle: native`) or a self-drawn menu | Self-drawn popover (`elevation_2`, at least 200 px wide, shortcuts on the right) |
| Keyboard operation in the tree | Per-row selection and focus | Per-row selection |

## 4. Relevance to Lucerna (current state)

- The Explorer (`crates/lucerna-workbench/src/explorer/`) has no selection of its own yet. The highlighted row is the open tab (`active`). There is no keyboard operation in the tree yet. The context key `explorerViewletFocus` currently shares its value with `sideBarFocus`.
- Nothing uses `MouseButton::Right` yet. `ui.rs` already has `popover`, `list_button` (an icon + label + key row) and `tooltip`, but nothing that floats at the mouse position yet.
- `workbench.action.splitEditor` and editor groups already exist, so Open to the Side is doable.
- Lucerna has no integrated terminal yet. If Open in Terminal is added, it has to be an external terminal.
- The code already uses `window.prompt` (save, documents, about) to show native dialogs. Delete can use it to ask again.
- Opening a folder in Finder currently calls `Command::new("open")` in `config_files.rs`. GPUI already has `cx.reveal_path` and `cx.open_with_system`, which work on every platform.
