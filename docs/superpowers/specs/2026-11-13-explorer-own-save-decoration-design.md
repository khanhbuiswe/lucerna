# Explorer: Lucerna's own saves are not marked "changed externally"

A short design for one bug. Approved by the user on 2026-11-13.

## Bug

From the user's video: edit `flashcards.ts` (Vim `ciw`, Esc), save, then `u` and save again. After each save the tab goes back to the × button (correct), but the `flashcards.ts` row in the Explorer turns bold and gets a white dot, and stays that way until the user switches to another tab and back.

That dot is the Phase 1 "file just changed on disk from outside" mark (`explorer.decorations.externalChanges`, `CHANGE_DECORATION` = 10 s). Phase 1 could not edit files, so `ExplorerView::on_worktree_event` treats every `WorktreeEvent::Changed` (except `Removed`) as an external change. Since Phase 4, Lucerna writes files itself when saving, and the watcher reports that very write, so it gets marked by mistake. The mark is cleared only when the active tab *changes* to that file, so a file that is already active keeps the mark for the full 10 s.

## References

- VS Code: the Explorer colors only by git (`explorer.decorations.colors`/`badges`) and errors; there is no "just changed from outside" mark. Saving a file changes nothing in the tree. The unsaved state shows only on the tab and in Open Editors.
- Zed: the project panel colors by git status and diagnostics; there is no external-change mark.
- This mark is a Lucerna-specific feature (it signals when an agent edits a file), so it only needs to be accurate: only real external changes, and no mark for a file the user is looking at.

## Goals

- Lucerna's own saves (manual save, autosave, save on close) do not mark the file.
- Rewriting the exact same content (`touch`, an agent rewriting the file unchanged) does not mark an open file.
- A file in the active tab is not marked, even when an agent edits it (the user is looking at it; Phase 1 spec: opening a file clears the mark).
- A file that is not open, or is open in a background tab, is still marked when edited from outside, as before.

Out of scope: changing how the mark looks, adding git decorations, an unsaved mark in the Explorer.

## Approaches considered

1. **Let `Documents` decide for open files (chosen).** `Documents` already re-reads the file after every `Modified` event and compares the hash with `base_hash` (`finish_reload`); equal means its own save or an identical rewrite. Only when it differs (`Apply`, `Merge`, `Conflict`) is it a real external change. Because it depends on content, it does not race with event timing; `start_job` also already waits for a write in progress to finish (`reload_after_save`).
2. Remember "Lucerna just wrote this file" at save time, then filter events. This has to handle the ordering between the write finishing and the watcher reporting, and it repeats what `Documents` already does. Dropped.
3. Only skip the file in the active tab. This does not fix saving a file in a background tab (Save All, autosave on focus change). Dropped; kept as a secondary rule.

## Design

- `DocumentEvent::ChangedOnDisk { path }`: `Documents` emits it when a re-read finds content different from `base_hash`, that is, the `Apply`, `Merge` or `Conflict` branch of `finish_reload`.
- `Documents::follows_disk(path) -> bool`: the document is open and already has a buffer (there is a re-read to compare against). A document that has not finished loading, or failed to load, does not count, so the Explorer marks it as before.
- `Workbench::on_worktree_event`: when computing the paths to mark, drop the paths that `Documents::follows_disk` accepts; `ExplorerView::on_worktree_event` takes a filter in place of `decorate: bool`.
- `Workbench` listens for `ChangedOnDisk` and, if the setting is on, calls `ExplorerView::mark_external_change(path)`.
- `ExplorerView` does not mark the path of the active tab, in both entry points.

## Tests

In `crates/lucerna-workbench/tests/`:
- Edit then Cmd+S, the watcher reports `Modified`: no mark.
- A file open in a background tab is written with different content by an agent: marked.
- Same case but the content is written unchanged: no mark.
- The active file is written by an agent: no mark.
- Setting off: `ChangedOnDisk` does not mark.
- The old tests (`changed_event_decorates_until_opened`, `changed_decoration_respects_setting`) still pass.
