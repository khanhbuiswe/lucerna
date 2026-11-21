# Explorer: Cut/Copy/Paste, Duplicate, Open Preview — short design

Date: 2026-11-15. Status: requested by the user (via the main session), awaiting plan approval.

## Intent

The user wants the Explorer context menu to gain the following commands, with VS Code behavior:

1. Cut / Copy / Paste for files and folders.
2. Duplicate, the way Zed does it.
3. Open Preview for `.md` files.

This spec follows on from `2026-11-13-explorer-context-menu-design.md`. All the constraints and anti-goals of that spec still apply: never overwrite, the UI flow never waits on I/O, Lucerna's own operations are not flagged as "changed externally", and the menu looks like Zed's.

## References

Details are in `docs/research/2026-11-13-explorer-context-menu.md`, sections §1 and §2.

- **VS Code:**
  - The commands `filesExplorer.cut`, `filesExplorer.copy`, `filesExplorer.paste`, with keys ⌘X / ⌘C / ⌘V when `filesExplorerFocus`.
  - Paste pastes into the selected folder, or into the folder containing the selected file.
  - Duplicate names are renamed per `explorer.incrementalNaming` (default `simple`): `a copy.rs`, then `a copy 2.rs`.
  - Cut then Paste is a move. After that Paste the clipboard is cleared.
  - The Paste item is dimmed when the clipboard is empty. A row that has been Cut shows dimmed in the tree. `Esc` cancels the Cut (`filesExplorer.cancelCut`).
  - A folder cannot be pasted into itself or into one of its subfolders.
- **VS Code (Markdown extension):** "Open Preview" (`markdown.showPreview`) is in the Explorer context menu when clicking a markdown file.
- **Zed:**
  - `project_panel::Duplicate` (key `cmd-d`) copies the entry into the same folder, using the same " copy" naming as Paste.
  - After copying, Zed opens the rename box on the copy, with the name selected up to the end of " copy N".
  - Zed's clipboard lives in the panel, not the operating system clipboard.

## Design

### Commands, keys, menu

The keys are macOS keys. Every key has the condition `filesExplorerFocus && !textInputFocus`.

| Item | ID | Shown when | Key |
|---|---|---|---|
| Cut | `filesExplorer.cut` | not the root | `cmd+x` |
| Copy | `filesExplorer.copy` | not the root | `cmd+c` |
| Paste | `filesExplorer.paste` | a folder or the root (for a file, it pastes into the folder containing it). Dimmed when the clipboard is empty. | `cmd+v` |
| Duplicate | `filesExplorer.duplicate` | not the root | `cmd+d` |
| Open Preview | `markdown.showPreview` | a markdown file (by the file's language) | — |
| — | `filesExplorer.cancelCut` | not in the menu | `escape` while a Cut is pending |

- **Position in the menu (following VS Code):**
  - The Cut, Copy, Paste, Duplicate group goes after the Find in Folder group and before the Copy Path group.
  - Open Preview goes after Open to the Side.
  - In single-file mode: no Cut, Paste, Duplicate, Delete. Copy and Open Preview remain.

### Clipboard

- It is the Explorer's own clipboard, living in the workbench: `{ op: Cut | Copy, paths }`. It does not write to the operating system clipboard.
- For now there is only one item at a time, because Lucerna cannot yet select multiple rows.
- A row that has been Cut shows its name dimmed (`text_muted`).

### Paste

- **Target folder:** the selected folder. If the selected row is a file, use the folder containing that file. Taken from `args.path` when the command runs from the menu.
- **Naming on collision:**
  - File: `stem copy.ext`, then `stem copy 2.ext`, `stem copy 3.ext`…
  - Folder: `name copy`, `name copy 2`…
  - Check for a collision on the snapshot, then check again when writing using a non-overwriting operation. If a collision is only discovered at write time, try the next number.
- **Copy:**
  - Files are copied with `create_new`, so they never overwrite.
  - Folders are copied recursively. A symlink is copied as a symlink, not followed.
  - Refuse when the destination is inside the source folder itself, and report it with a status message.
- **Cut (move):**
  - Uses `fs_ops::rename`, the non-overwriting kind.
  - If the name collides at the destination, name it in the " copy" style as above. No overwriting, no prompt.
  - Pasting into the folder that already contains the entry does nothing.
  - Tabs, documents, LSP, navigation history and recents follow the new path, reusing Rename's `Workbench::paths_renamed`.
  - Wait for in-flight saves to finish before moving.
  - After pasting, the clipboard is cleared.
- **After pasting:** select the new entry, expand the target folder, and scroll to the entry. If it is a file, open it in a pinned tab, following Zed's `auto_open.on_paste`, on by default.
- **Own operations:** the new path is put into `expect_own_changes` before writing, and removed with `forget_own_changes` on error.
- **Errors:** report with the status message `Unable to paste '{name}': {err}`. This wording was written by Claude.
- **Threading:** all I/O runs on a background thread.

### Duplicate

- Same as Copy then Paste into the folder containing the entry, but without touching the clipboard.
- After copying, open the rename box on the copy. The box has the name preselected up to the end of " copy N", not the file extension, following Zed.

### Open Preview

- `markdown.showPreview` gains `args.path`. When `args.path` is present, the command opens that file in preview mode (`FileView::Preview`) in a pinned tab. When it is absent, the command behaves as it does today, i.e. it switches the open tab to preview.
- The menu item appears only when the file's language is `markdown` (`files.associations` still applies).

## Non-goals

- Selecting multiple rows; Cut/Copy of several entries at once.
- Pasting files copied from Finder or from the operating system clipboard.
- Writing to the operating system clipboard.
- A "Replace?" dialog on a name collision: always name it " copy", never ask.
- Undo, drag and drop.
- The `explorer.incrementalNaming` setting (only the `simple` style exists).

## Claude's decisions (for the user to review)

- **Cut with a name collision at the destination:** rename in the " copy" style instead of asking whether to overwrite as VS Code does. Reason: the anti-goal is never to overwrite, and the user wants " copy" naming on collision.
- **Duplicate ID:** `filesExplorer.duplicate`, because VS Code has no such command. Key `cmd+d` when the Explorer has focus, following Zed.
- **After pasting a file, open the file**, following Zed. VS Code does not open it.
- **The clipboard is only the internal clipboard**, as in Zed.
