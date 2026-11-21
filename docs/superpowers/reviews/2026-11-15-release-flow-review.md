# Review of the basic flows before the v0.1.0 release

Date: 2026-11-15. Spec: [2026-11-15-release-flow-review-design.md](../specs/2026-11-15-release-flow-review-design.md). Plan: [2026-11-15-release-flow-review.md](../plans/2026-11-15-release-flow-review.md).

**Addendum 2026-11-16:** also reviewed the multi-window merge (`d765a50`), see the section [Addendum](#addendum-multi-window-merge-d765a50). Older bugs affected by the merge were updated in place.

**Method.** Five agents read the code, one per group of flows. Each bug found was checked by another agent reading the code again; that agent was not allowed to see the earlier agent's reasoning. No bug was verified by running the app or running tests. Every conclusion comes from reading the code at commit `ff0498b`.

## Summary

| Level | Number of bugs |
|---|---|
| P0 (lost/corrupted data, crash) | 2 |
| P1 (a basic flow cannot be done) | 0 |
| P2 (wrong, but there is another way) | 14, all fixed on branch `claude/p2-fixes` |
| P3 (minor) | 21 (13 old, 8 new) |

3 items were dropped and 2 were merged as duplicates. T3-1 was fixed in the multi-window merge. T2-1 was fixed with hot exit; T2-2, T2-3, T3-2, T3-4, T3-5, T4-1, T4-2, T5-2, T5-3, T5-6, M1-1, M2-1, M3-1, M3-2 (and M2-2) were fixed on branch `claude/p2-fixes`. Every remaining item (T1-1 and the P3s: T1-2, T1-3, T1-5, T1-6, T1-7, T2-4, T2-5, T3-3, T4-4, T4-5, T5-1, T5-4, T5-5, M1-2, M1-3, M1-4, M1-5, M1-6, M2-3, M3-4) was fixed on branch `claude/p3-fixes`, following the [spec](../specs/2026-11-17-p3-fixes-design.md). No item is left unfixed.

**Should be fixed before release:**
- **T2-1 [P0]:** Quit from the Dock, Log Out or Restart exits at once without asking to save, so unsaved changes are lost.
- **T2-2 [P2]:** a rename that only changes case leaves a "ghost" entry in the Explorer. If Delete Permanently is chosen on that entry, the real file is deleted.
- **T1-1 [P0]:** a file with bytes in the wrong encoding is written as `U+FFFD` on save, with no warning.

**From the multi-window merge, the most worth looking at:** M3-2 (if trust cannot be written, the window silently goes back to Restricted Mode), M2-1 (Cmd+Q does not show the save dialog when the window is minimized), M3-1 (the preview theme gets stuck), M1-1 (two `luc` at the same time).

## Bug list

### T1-1 [P0] Bytes in the wrong encoding are replaced by U+FFFD on save, even where they were not edited

- **Flow:** saving a file that is not UTF-8 or has a BOM.
- **Location:** [load.rs:88](../../../crates/lucerna-text/src/load.rs), [write.rs:145](../../../crates/lucerna-text/src/write.rs).
- **Reproduction:** open a UTF-8 file with a BOM but with a stray `0xE9` byte, or a UTF-16 file with an odd number of bytes. Edit somewhere else and save: the broken byte becomes `EF BF BD`.
- **Variant:** with multi-byte encodings (Shift_JIS, GBK…), the save fails on a line the user did not edit.
- **Cause:** `decode_without_bom_handling` drops the `had_errors` flag.
- **Severity:** by the spec's scale, changing bytes the user did not edit is data corruption, so it is ranked P0. The bug is rare: valid UTF-8 files and Latin-1 files are not affected, and VS Code also writes U+FFFD. The verifying agent proposed P2 for that reason.
- **Suggested fix:** use `had_errors` to warn, or open read-only.
- **Fixed** (commits 85d3211, 24ea121, d661116, e93a21d): decoding reports a flag for invalid bytes, the editor warns with a bar on the file, the first save asks before replacing the broken bytes with U+FFFD, and the flag is kept in the hot exit backup.

### T2-1 [P0] Quitting from the Dock, Log Out/Restart or AppleScript bypasses the save dialog

- **Flow:** quitting the app while files are unsaved.
- **Location:** [workbench.rs:2907](../../../crates/lucerna-workbench/src/workbench.rs), [workbench.rs:1673](../../../crates/lucerna-workbench/src/workbench.rs), [workbench.rs:860](../../../crates/lucerna-workbench/src/workbench.rs).
- **Reproduction:** edit a file, do not save. Right-click the Dock icon → Quit, or Apple menu → Restart. The app shuts down at once. Reopen and the changes are gone.
- **Cause:**
  - Only Lucerna's `Quit` action (Cmd+Q, the menu; [app_menu.rs:570](../../../crates/lucerna-workbench/src/app_menu.rs)) and closing a window go through `confirm_quit`.
  - GPUI's macOS delegate (rev `b5cd310`, `gpui_macos/src/platform.rs:91`) only registers `applicationWillTerminate:`, not `applicationShouldTerminate:`. So `-[NSApp terminate:]` quits at once.
  - Lucerna's `on_app_quit` hook only saves the layout and shuts down LSP.
  - There is no hot exit or any backup.
- **Fixed with hot exit** on branch `claude/hot-exit` (commits 5d402f7..7ee56bb): quitting any way keeps unsaved changes in the backup, with no need to ask. Still waiting for hand tests of Dock Quit, Log Out and `kill -9`.
- **Suggested fix:** block terminate at the platform layer: `applicationShouldTerminate:` returns `NSTerminateCancel` and then runs `quit(cx)`. GPUI is Zed's code, so this needs a patch or a separate hook. Another option is to write a backup of the unsaved buffers in the quit hook.
- **After the multi-window merge:** still present. GPUI is still at `b5cd310`. `quit` is now at [workbench.rs:3018](../../../crates/lucerna-workbench/src/workbench.rs) and asks every window, but Dock Quit and Log Out still go straight through `terminate:`. The multi-window spec §6.6 records this as a known limitation.

### T2-2 [P2] A rename that only changes case leaves a "ghost" entry in the Explorer

- **Flow:** Explorer Rename, and an agent running `mv`/`git mv`, on a case-insensitive APFS. This bug absorbs T4-3.
- **Location:** [worktree.rs:471](../../../crates/lucerna-project/src/worktree.rs), [watch.rs:126](../../../crates/lucerna-project/src/watch.rs), [explorer/view.rs:794](../../../crates/lucerna-workbench/src/explorer/view.rs).
- **Reproduction:** rename `foo.rs` to `Foo.rs`. The Explorer shows both.
  - Clicking `foo.rs` opens a second document for the same file.
  - Deleting `foo.rs` deletes the real file. By default it goes to the Trash; if Delete Permanently is chosen it is gone for good.
- **Cause:** `symlink_metadata("foo.rs")` still succeeds, so the old path is treated as Modified instead of Removed.
- **Suggested fix:** for a known path that "still exists", compare the real name in the parent's `read_dir`. If it differs, treat it as Removed.
- **Fixed** (commits 1bae091, 9d65392 — the tab follows when an agent renames): a path where one name in it exists on disk only with different case is treated as Removed (a folder with its whole subtree), for both Explorer Rename and an agent's `mv`/`git mv`.

### T2-3 [P2] Quitting while a Paste (copy) is running leaves a partial copy

- **Flow:** Explorer copy/paste.
- **Location:** [explorer/clipboard.rs:263](../../../crates/lucerna-workbench/src/explorer/clipboard.rs), [workbench.rs:1676](../../../crates/lucerna-workbench/src/workbench.rs).
- **Reproduction:** Paste a folder of a few GB and then Cmd+Q at once. The app quits without asking. The `X copy` left on disk is missing files, or has truncated files.
- **Cause:** the copy task is `detach`ed, and `confirm_quit` only looks at unsaved buffers and terminals. No file operation is tracked at quit: `deletes_in_flight` ([explorer/commands.rs:288](../../../crates/lucerna-workbench/src/explorer/commands.rs)) is not read there either, so a running Cut or Delete is not protected either.
- **Suggested fix:** count the running copy operations and ask or wait at quit. Or copy to a temporary name and rename when done.
- **After the multi-window merge:** still present with Cmd+Q (`confirm_quit` at [workbench.rs:1773](../../../crates/lucerna-workbench/src/workbench.rs)). Closing a single window on macOS no longer causes the bug, because the process stays alive so the copy finishes.
- **Fixed** (commit a5f9802): the copy writes to a hidden name `.luc-copy~*` next to the destination and only renames when done (the Explorer does not show this hidden name), and Cmd+Q or closing a window while a Paste/Duplicate/Cut/Delete is running asks "A file operation is still running" (Quit / Cancel).

### T3-1 [P2, fixed in `d765a50`] Open With or dropping onto the Dock a subfolder of the open folder does nothing

- **Flow:** opening from Finder or the Dock.
- **Location:** `open_requests.rs:77` (at `ff0498b`, the file has since been deleted), [app.rs:445](../../../crates/lucerna-workbench/src/app.rs).
- **Reproduction:** run `luc ~/proj`. In Finder, Open With Lucerna on `~/proj/src`: it only activates the existing window.
- **Expected:** the docs say "anything else opens in a new window".
- **Suggested fix:** in `route`, put only files and the root itself into `ThisWindow`. A subfolder returns `NewProcess`.
- **After the multi-window merge:** gone. `open_requests.rs` was deleted; [window_choice.rs:71](../../../crates/lucerna-workbench/src/window_choice.rs) only focuses when `root == path`, a subfolder opens a new window or replaces the Welcome window.

### T3-2 [P2] In single-file mode, Open Recent hides the parent folder

- **Fixed** (commit 58aca46): in single-file mode, Open Recent (picker and menu) only hides the file itself, not the parent folder.

- **Flow:** Open Recent (Ctrl+R and the menu).
- **Location:** [workbench.rs:961](../../../crates/lucerna-workbench/src/workbench.rs).
- **Reproduction:** open `~/proj` once. Then run `luc ~/proj/README.md` and press Ctrl+R: `~/proj` is not shown.
- **Cause:** `opened_paths` always includes `root`, and in single-file mode the root is the parent folder.
- **Suggested fix:** in single-file mode, return only the file itself.
- **After the multi-window merge:** still present. `opened_paths` is now at [workbench.rs:1029](../../../crates/lucerna-workbench/src/workbench.rs). The bug also affects the File ▸ Open Recent menu ([app_menu.rs:705](../../../crates/lucerna-workbench/src/app_menu.rs)).

### T3-4 [P2] Opening a FIFO leaves the tab stuck on Loading; `luc /dev/zero` uses up all RAM and then crashes

- **Fixed** (commit 77df9d9): a FIFO or device opens with the error "Not a regular file" at once, without reading; `luc /dev/zero` reports an error and exits with code 1.

- **Flow:** opening a special file.
- **Location:** [documents.rs:2576](../../../crates/lucerna-workbench/src/documents.rs), [worktree.rs:270](../../../crates/lucerna-project/src/worktree.rs), [target.rs:74](../../../crates/lucerna/src/target.rs).
- **Reproduction:**
  - Run `mkfifo proj/pipe`, then click `pipe` in the Explorer: the tab is stuck on Loading.
  - Run `luc /dev/zero`: the app reads until memory runs out.
- **Cause:** `metadata.len()` is 0 so it slips past `check_size`. The code does not check `is_file()`.
- **Severity:** a crash from running out of RAM is P0 by the scale. I lowered it to P2 because it needs deliberate input (`/dev/zero`); a FIFO in a project only hangs the tab.
- **Suggested fix:** if `!meta.is_file()`, return a `LoadError` ("Not a regular file").

### T3-5 [P2] An unreadable root folder leaves the Explorer empty with no notice

- **Flow:** opening a folder when macOS TCC denies permission, or a `chmod 000` folder.
- **Location:** [worktree.rs:661](../../../crates/lucerna-project/src/worktree.rs).
- **Reproduction:** run `chmod 000 proj; luc proj`: the tree is empty. The user thinks the folder is empty.
- **Cause:** `let Ok(dirent) = result else { continue }` skips every error while walking directories.
- **Suggested fix:** an error at the root itself sends an error event so the Explorer shows a message.
- **Fixed** (commit 38933d7): each scan checks the root first; if it cannot be listed the tree is empty and the Explorer shows "Can't read folder <name>: <reason>", and a later successful rescan returns to normal.

### T4-1 [P2] The root folder is renamed, moved to the Trash or its drive is ejected: nobody notices

- **Flow:** watching the agent.
- **Location:** [coalesce.rs:49](../../../crates/lucerna-project/src/coalesce.rs).
- **Reproduction:** open `proj`, then from another terminal run `mv proj proj2`. The tree and tabs stay as before, with no notice. Saving reports an error.
- **Cause:**
  - An event whose relative path is empty is dropped.
  - notify creates the stream without the `WatchRoot` flag.
  - Nothing checks `root.exists()`.
- **Severity:** no data loss, because save checks the disk before writing.
- **Suggested fix:** treat an event at the root as a rescan signal. If the rescan finds the root gone, report it and mark the tabs as Deleted.
- **Fixed** (commit 38933d7): an event at the root (empty path) now causes a rescan; if the root is gone every tab changes to Deleted (keeping the unsaved text), git marks are dropped and the Explorer shows "Folder <name> no longer exists"; when renamed back, the tabs, tree and git return to normal.

### T4-2 [P2] A tab opened through a symlink does not reload when the target file changes

- **Fixed** (commit 8f24c26): a tab opened through a symlink is added to the open-file watcher and the target file's folder is also watched, so it reloads when the target file changes. Saving through the symlink still writes to the target file.

- **Flow:** reload when an agent edits a file.
- **Location:** [workbench.rs:1264](../../../crates/lucerna-workbench/src/workbench.rs), [documents.rs:476](../../../crates/lucerna-workbench/src/documents.rs).
- **Reproduction:** in a repo there is `AGENTS.md -> CLAUDE.md`. Open `AGENTS.md` from the Explorer, then an agent edits `CLAUDE.md`: the tab still shows the old content.
- **Cause:**
  - The document's key does not resolve the symlink.
  - `OpenFileWatcher` (which compares by `real_path`) skips every file that is in the snapshot.
- **Severity:** save still checks the disk before writing. Opening with `luc` does not have this bug, because that path canonicalizes.
- **Suggested fix:** put symlink tabs into `OpenFileWatcher`, or keep a map from real path to key.

### T5-2 [P2] An empty left group does not close itself

- **Fixed** (commit 09c8f81): an empty group is dropped while other groups remain, including the first group; the restored layout also does not keep empty groups.

- **Flow:** split editor.
- **Location:** [editor_groups/model.rs:605](../../../crates/lucerna-workbench/src/editor_groups/model.rs).
- **Reproduction:** open `a.rs`, press Cmd+\, then Cmd+W in the left group: half the screen is empty. There is no close-group command, and this state remains even after restoring the workspace.
- **Cause:** the condition `g > 0`.
- **Expected:** VS Code defaults to `workbench.editor.closeEmptyGroups: true`.
- **Suggested fix:** drop group 0 when it is empty and other groups remain.

### T5-3 [P2] Quick Open does not match NFD Vietnamese file names, and the highlight is off

- **Fixed** (commits 3be5141, ff586be): paths and the query are NFC-normalized before matching, and highlight positions are computed in chars of the displayed string; ASCII paths keep the fast path (debug measurement on 80k files: 95.4 ms before, 95.9 ms after).

- **Flow:** Quick Open.
- **Location:** [fuzzy.rs:144](../../../crates/lucerna-project/src/fuzzy.rs), [quick_open/picker.rs:73](../../../crates/lucerna-workbench/src/quick_open/picker.rs).
- **Reproduction:** a file `ghi-chú.md` whose name is stored as NFD. Cmd+P, type `chú`: no results. Type `chu` and it shows.
- **Cause:**
  - Nothing normalizes to NFC.
  - nucleo merges graphemes and returns positions by grapheme, while the picker treats them as char indices.
- **Suggested fix:** NFC-normalize both the path and the query, then map positions back to char indices.

### T5-6 [P2] Project search ignores unsaved changes, and clicking jumps to the wrong line

- **Fixed** (commit ac9452f): an open file with unsaved changes is searched on the buffer content instead of the disk.

- **Flow:** Cmd+Shift+F.
- **Location:** [search.rs:107](../../../crates/lucerna-project/src/search.rs), [search_view/view.rs:658](../../../crates/lucerna-workbench/src/search_view/view.rs).
- **Reproduction:** insert 10 lines at the top of `a.rs`, do not save. Search a word on the old line 50, then click the result: the cursor goes to line 50 of the buffer, with no highlight.
- **Expected:** VS Code also searches unsaved editors.
- **Suggested fix:** for an unsaved file, search on the buffer's snapshot.

### T1-2 [P3] Autosave can recreate a file the agent just `rm`'d/`mv`'d

- **Flow:** `files.autoSave` when an agent deletes or renames a file.
- **Location:** [save.rs:274](../../../crates/lucerna-workbench/src/save.rs).
- **Reproduction:** set `files.autoSave: afterDelay`, then type in `a.rs`. The agent runs `rm a.rs` just as the timer is about to fire. The file `a.rs` appears again.
- **Cause:** `changed_on_disk` treats NotFound as "unchanged". So an autosave that runs in the 50–150 ms before the watcher reports will rewrite the file. Result: the agent's `rm` is undone, and a `mv` produces a duplicate file.
- **Suggested fix:** for `SaveReason::Auto`, do not write on NotFound.
- **Fixed** (commit 400054a): autosave does not rewrite a file that has been deleted.

### T1-3 [P3] The post-save stamp is taken after the rename step

- **Flow:** saving a file while an agent writes right after.
- **Location:** [save.rs:263](../../../crates/lucerna-workbench/src/save.rs).
- **Reproduction:** Cmd+S, the agent writes the file within a few microseconds after the rename step, and then the user saves again before the watcher reports. The agent's version is overwritten without asking. Practically only theoretical.
- **Suggested fix:** take the metadata from the temp file's file handle before the rename.
- **Fixed** (commit e5f0ed9): the post-save stamp is taken from the temp file's file handle, before the rename step.

### T1-5 [P3] A file name longer than about 241 bytes cannot be saved

- **Flow:** edit and save.
- **Location:** [write.rs:263](../../../crates/lucerna-text/src/write.rs).
- **Reproduction:** open a file with a 245-byte name, edit, Cmd+S. The status bar reports "File name too long".
- **Cause:** the temp file name `.{name}.luc~XXXXXXXX` is 14 bytes longer than the original name.
- **Suggested fix:** truncate or hash the name part in the temp file name.
- **Fixed** (commit 90cb304): the temp file name is kept within 255 bytes.

### T1-6 [P3] After Keep Mine, autosave is not rescheduled

- **Flow:** conflict with `files.autoSave: afterDelay`.
- **Location:** [documents.rs:1063](../../../crates/lucerna-workbench/src/documents.rs), [save.rs:404](../../../crates/lucerna-workbench/src/save.rs).
- **Reproduction:** set autoSave afterDelay. Type in a file, the agent overwrites the same line so it becomes a conflict. Wait longer than the delay and then click Keep Mine. No save runs until the next typing; the tab still has the "unsaved" dot.
- **Suggested fix:** reschedule the timer on `ConflictChanged{conflict:false}` when the text is still unsaved.
- **Fixed** (commit 2391aad): Keep Mine reschedules autosave afterDelay.

### T1-7 [P3] `files.trimFinalNewlines` on an all-blank-lines file leaves `\n`

- **Flow:** saving with `files.trimFinalNewlines`.
- **Location:** [save.rs:212](../../../crates/lucerna-workbench/src/save.rs).
- **Reproduction:** turn on `files.trimFinalNewlines`, save a file whose content is `\n\n\n`. The file is left with `\n`.
- **Note:** VS Code produces an empty file; the agent was about 80% sure. The current test asserts the `\n` behavior.
- **Suggested fix:** check against VS Code's `TrimFinalNewLinesParticipant`.
- **Fixed** (commit 404b591): an all-blank-lines file now becomes an empty file, as in VS Code.

### T2-4 [P3] Cmd+Q choosing Don't Save while a write is in progress leaves a temp file `.name.luc~XXXX`

- **Flow:** quitting the app while a save is in progress.
- **Location:** [write.rs:253](../../../crates/lucerna-text/src/write.rs), [workbench.rs:1696](../../../crates/lucerna-workbench/src/workbench.rs).
- **Reproduction:** Cmd+S a large file, Cmd+Q at once, choose Don't Save. A file `.<name>.luc~xxxxxxxx` is left next to the original file.
- **Note:** the original file is intact. A rare exception: a file with a hardlink that goes through `write_in_place`, where the file can be truncated.
- **Suggested fix:** before quitting, wait for running saves to finish.
- **Fixed** (commit a4ceb09): at quit, wait up to 5 s for running saves and then delete their temp files.

### T2-5 [P3] New File with the name `foo/` creates a file instead of a folder

- **Flow:** Explorer New File.
- **Location:** [fs_ops.rs:79](../../../crates/lucerna-project/src/fs_ops.rs).
- **Reproduction:** Explorer → New File → type `components/` → Enter. The result is an empty file named `components`.
- **Note:** VS Code creates a folder in this case. The agent did not re-check this in the VS Code source.
- **Suggested fix:** a name ending in a separator is treated as New Folder.
- **Fixed** (commit d48a109): a name ending in `/` creates a folder, as in VS Code; `a/b/c.rs` creates the intermediate folders too (already correct before, now with a test).

### T3-3 [P3] A recent entry that no longer exists is permanently deleted on the next write

- **Flow:** Open Recent, Welcome page.
- **Location:** [recent.rs:127](../../../crates/lucerna-workbench/src/recent.rs).
- **Reproduction:** open `/Volumes/SSD/proj`, unplug the drive, open another folder, then plug the drive back in. `proj` is gone from the recent list.
- **Note:** this is deliberate per the doc comment, but differs from VS Code.
- **Suggested fix:** only filter the entries that do not exist at display time, not at write time.
- **Fixed** (commits 30d9e37, 7bae7da): the recent list keeps entries that are no longer on disk, so Welcome, Open Recent and the menu show every entry. Choosing an entry that no longer exists removes it from the list and shows a "Path does not exist" / "The path '…' does not exist on this computer." dialog with only an OK button, as in VS Code; the menu when no window exists removes it without notice.

### T4-4 [P3] A watcher that fails to start is only logged

- **Flow:** watching the agent.
- **Location:** [worktree.rs:312](../../../crates/lucerna-project/src/worktree.rs), [git/mod.rs:762](../../../crates/lucerna-workbench/src/git/mod.rs).
- **Reproduction:** the worktree's or `.git`'s watcher fails to start, for example on a network volume, or when inotify watches run out on Linux. Auto-reload and git colors stop working with no notice. Rare on macOS.
- **Suggested fix:** show a notice in the status bar.
- **Fixed** (commits 328019d, 43bd55f, 021390d): when the worktree's or `.git`'s watcher fails to start, the status bar shows a warning icon, with the tooltip "File watching is off for <folder>: <reason>. Changes made outside Lucerna won't show."

### T4-5 [P3] An ignored file whose parent folder is deleted and recreated may not be watched again

- **Flow:** reloading an ignored file (`dist/`, `target/`).
- **Location:** [open_files.rs:76](../../../crates/lucerna-workbench/src/open_files.rs).
- **Reproduction:**
  1. Open `dist/bundle.js`.
  2. The agent runs `rm -rf dist`.
  3. The user opens or closes another tab that is also outside the tree, for example `.env`.
  4. The agent rebuilds.

  Result: the tab stays in the Deleted state.
- **Note:** a single Rescanned would rescue it by itself.
- **Suggested fix:** watch the nearest ancestor folder that still exists.
- **Fixed** (commits 3314e15, 288c127, df6711d, caf3dfe): the open-file watcher follows the nearest ancestor folder that still exists and moves down when the parent folder reappears; a file that was written in the meantime is also reported.

### T5-1 [P3] Clicking a search result past the end of the file leaves an observer, and later the cursor jumps

- **Flow:** project search, click a result.
- **Location:** [search_view/view.rs:672](../../../crates/lucerna-workbench/src/search_view/view.rs).
- **Reproduction:**
  1. Search with a result on line 100 of `a.rs`.
  2. The agent rewrites `a.rs`, leaving only 50 lines.
  3. Click the result.
  4. Later, typing, or the agent writing, makes the file longer than 100 lines.

  The cursor jumps once to line 100.
- **Suggested fix:** when the doc is already Ready but the line does not exist, remove the observer.
- **Fixed** (commit 08ba3d4): if the line does not exist the cursor goes to the last line once, the highlight is dropped and the observer is removed; the observer is also removed when the doc fails to load, so a later reload does not highlight again. (If an unreadable file later becomes readable, the jump to the line from the original open still runs, because that is the open request completing, not the observer.)

### T5-4 [P3] Quick Open does not accept `:-N` and `:N,M`

- **Flow:** Quick Open, go to line.
- **Location:** [quick_open/query.rs:42](../../../crates/lucerna-workbench/src/quick_open/query.rs).
- **Reproduction:** Cmd+P, type `:-1` or `:12,5`, then Enter. Nothing happens. `:N` and `:N:M` still work.
- **Suggested fix:** parse `i64`, negative numbers count from the end of the file; split the line/column part on `[:,#]`.
- **Fixed** (commit 5e87b50): `:-N` counts from the end of the file; line and column are separated by `:`, `,` or `#`; the line is clamped to `[1, number of lines]` as in VS Code. Applies when Vim is off.

### T5-5 [P3] Project search with a regex containing `$` does not match CRLF files

- **Flow:** project search, regex mode.
- **Location:** [search.rs:122](../../../crates/lucerna-project/src/search.rs).
- **Reproduction:** a CRLF file with a line `foo`. Turn on regex, search `foo$`: no results. Cmd+F in that same file finds it.
- **Suggested fix:** add `.crlf(true)` to `RegexMatcherBuilder`.
- **Fixed** (commit 45319d7): the regex matcher is built with `multi_line(true)`, `crlf(true)` and `line_terminator(Some(b'\n'))`, and the searcher keeps LF. `$` and `^` match at both `\r\n` and `\n`; LF files and literal search are unchanged.

## Addendum: multi-window merge (`d765a50`)

On 2026-11-16 the `multi-window` branch was merged into `main`. This branch moved from "one process per window" to "one process, many windows", and `luc` now passes paths over a Unix socket. This part was reviewed with the same method as above, but on `git diff ff0498b..d765a50`: three agents read the code, and each bug was verified by another agent. The branch's spec: [2026-11-15-multi-window-design.md](../specs/2026-11-15-multi-window-design.md).

**No new P0 or P1 bug was found.** The agents checked and found no bug in these cases:
- **The same file open in two windows:** there is no path that loses data. Each window has its own `Documents`, and before writing they compare stamp and hash, so a change from the other window becomes a conflict.
- **Settings:** nothing leaks from one window to another. Checked `files.*`, `editor.*`, `terminal.integrated.*`, `git.*`, `lsp`.
- **Quit with many windows:** Cancel in any window cancels the whole quit.
- **`WindowHandle::update` on a closed window:** all handled, no panic.

### M1-1 [P2] Two `luc` commands run at the same time when the app is not open: the second can fail

- **Fixed** (commit 53828ad): when `luc` hits a connection closed before any reply, it claims the socket again once.

- **Flow:** opening with `luc` (for example an agent runs `luc a & luc b`).
- **Location:** [main.rs:173](../../../crates/lucerna/src/main.rs), [cli_socket.rs:289](../../../crates/lucerna-workbench/src/cli_socket.rs), [cli_socket.rs:367](../../../crates/lucerna-workbench/src/cli_socket.rs).
- **Reproduction:** the app is not running, run `luc a & luc b`. Sometimes `b` reports "no reply from Lucerna", exits with code 1, and opens nothing.
- **Cause:**
  1. The first `luc` binds the socket and then releases it (`release`) so the child app can bind again.
  2. If the second `luc` connects exactly in that window, its connection sits in the backlog of a listener that is about to be dropped.
  3. The listener is dropped, the connection is reset, and the second `luc` does not retry.
- **Suggested fix:** let the child app inherit the listener's fd instead of releasing and rebinding; or have the client claim again once on EOF or an error.

### M2-1 [P2] Cmd+Q when a window with unsaved files is minimized: the save dialog does not appear

- **Fixed** (commit 036bb30): before every question at quit, the window is brought to the front. Un-minimizing on macOS still needs a hand test.

- **Flow:** quitting the app with many windows.
- **Location:** [workbench.rs:3028](../../../crates/lucerna-workbench/src/workbench.rs), [save.rs:105](../../../crates/lucerna-workbench/src/save.rs).
- **Reproduction:** open folders A and B. Edit a file in B, do not save, then minimize B (or move B to another Space). From A press Cmd+Q: nothing happens. Pressing Cmd+Q a second time does nothing either, because the quit is canceled silently.
- **Cause:** `quit()` asks each window but does not bring that window to the front. GPUI macOS shows the dialog as a sheet on that window (`beginSheetModalForWindow`) and does not un-minimize.
- **Severity:** no data loss, because the quit is canceled rather than dropping the file.
- **Suggested fix:** call `activate_window()` on the window before calling `confirm_quit`, as Zed does.

### M3-1 [P2] Closing a window while the theme picker is open leaves the preview theme stuck in every window

- **Fixed** (commit baa0342): when the window closes, the Workbench is released, or the window loses focus while the theme/icon picker is open, the picker closes with a revert; reverting the icon theme no longer turns the preview flag back on.

- **Flow:** choosing a theme with many windows.
- **Location:** [workbench.rs:1844](../../../crates/lucerna-workbench/src/workbench.rs), [theme_picker.rs:247](../../../crates/lucerna-workbench/src/theme_picker.rs), [config_files.rs:295](../../../crates/lucerna-workbench/src/config_files.rs).
- **Reproduction:** there are windows A and B. In A open the Color Theme picker, use the arrow keys to move to another theme, then click the red button to close A.
  - B keeps the theme being previewed.
  - After that, editing `workbench.colorTheme`, or changing the system Light/Dark, has no effect.
  - The icon theme gets stuck the same way.
- **Cause:** the `theme_preview`/`icon_preview` flag is global, and only `close_theme_picker` clears it. The window-close path does not call this function. Before multi-window, closing the window quit the process, so it never came up.
- **Workaround:** open and then Esc the picker in another window.
- **Suggested fix:** close the picker with a revert in `should_close_window` and in the Workbench's `on_release`; or tie the preview flag to a `WindowId`.

### M3-2 [P2] If `trusted.json` cannot be written, the window that just clicked Trust is put back in Restricted Mode with no notice

- **Fixed** (commits 31b6754, e38026c): a write error does not bump TrustGeneration, the window keeps trust (or untrust) for the whole session even when another window writes later, and there is a status message.

- **Flow:** trusting a folder.
- **Location:** [trust.rs:712](../../../crates/lucerna-workbench/src/trust.rs), [trust.rs:473](../../../crates/lucerna-workbench/src/trust.rs).
- **Reproduction:** the data folder is not writable (disk full, or read-only). Open a folder, click "Trust and Continue". LSP and git start and then stop at once, and the Restricted Mode button reappears. The error is only in the log.
- **Cause:**
  1. `write_trust` ignores the write error but still bumps `TrustGeneration`.
  2. Every window, including the one that just clicked, rereads the old file and sets `stored=false`.

  This is a regression: previously trust was kept in memory for the whole session.
- **Suggested fix:** only bump on a successful write. On a write error show a status message and keep trust for the session.

### M1-2 [P3] `luc` just as the app is quitting reports an error instead of opening a new app

- **Fixed** (commits c315fe7, 8c5162b): the EOF part was fixed together with M1-1; a quitting app returns `status: "quitting"`, and `luc` claims again every 100 ms for up to 5 s and then opens in the new app.

- **Flow:** opening with `luc`.
- **Location:** [cli_socket.rs:478](../../../crates/lucerna-workbench/src/cli_socket.rs), [app.rs:146](../../../crates/lucerna-workbench/src/app.rs).
- **Reproduction:** Cmd+Q, and while the app is shutting down type `luc ~/proj`. `luc` prints "Lucerna is quitting" or "no reply", and exits with code 1.
- **Suggested fix:** on the client side treat "quitting" and EOF as a signal to claim the socket again.

### M1-3 [P3] App busy for more than 5 seconds: if the open later fails, there is no notice

- **Fixed** (commits 68ff94f, 6223843): when `luc` has stopped waiting ("will open when ready") and the open later fails, a status message "Could not open …" is shown in the most recently used window; when `luc` is still waiting, the error is only printed to the terminal.

- **Flow:** opening with `luc` while the app is restoring windows.
- **Location:** [router.rs:460](../../../crates/lucerna-workbench/src/router.rs).
- **Reproduction:** the app is restoring many windows, type `luc ~/x/file`. After 5 seconds `luc` reports "will open when ready" and exits 0. If the file is deleted in the meantime there is no notice.
- **Cause:** only `Source::Menu` shows a status message. Reporting "will open when ready" and exiting 0 is correct per spec §6.5.
- **Suggested fix:** show a status message for `Source::Cli` too.

### M1-4 [P3] If the socket cannot be used the app runs right in the terminal; closing the terminal loses unsaved files

- **Fixed** (commit 6f920e3): if the socket cannot be used, `luc` still starts the app detached from the terminal (without a socket), warns, and exits 0.

- **Flow:** `luc` when the data folder belongs to another user, or when there is no HOME.
- **Location:** [main.rs:160](../../../crates/lucerna/src/main.rs).
- **Reproduction:**
  1. The folder `~/Library/Application Support/Lucerna` belongs to another user (for example `sudo luc` was run once).
  2. Run `luc ~/proj`: it prints the warning "running without the luc socket", and the app runs right in the terminal.
  3. Edit a file, then close the terminal tab: the app dies, with no save prompt.
- **Note:** spec §6.5 deliberately runs without the socket with a warning, but does not say it has to be detached from the terminal.
- **Suggested fix:** still `spawn_detached` the app without a socket and then exit.

### M1-5 [P3] The socket name contains the version: after an update while the old app is still running there are two apps

- **Fixed** (commit 2aab070): the socket is `luc.sock` / `luc-dev.sock`; a protocol newer than the app reports "Lucerna was updated. Quit Lucerna and open it again."

- **Flow:** updating Lucerna.
- **Location:** [cli_socket.rs:136](../../../crates/lucerna-workbench/src/cli_socket.rs).
- **Reproduction:** app 0.1.0 is running, install 0.1.1, run `luc ~/proj`: a second process opens. Whichever process quits later, `windows.json` keeps only that process's windows.
- **Note:** deliberate per spec M13 and §10. Every write is atomic so no file is corrupted.
- **Suggested fix:** keep the `-dev` suffix but remove the version from the socket name (the protocol already has a `version` field).

### M1-6 [P3] `luc --trace-startup`/`--foreground` when an app of the same version is running only forwards the request

- **Fixed** (commit ba9e4c7): when the socket is already held, `--trace-startup` reports "Lucerna is already running; quit it to measure startup.", `--foreground` reports "Lucerna is already running; quit it to run in the foreground.", and both exit 1; `luc`'s child process uses the hidden flag `--detached-child`.

- **Flow:** dev tools and bench.
- **Location:** [main.rs:143](../../../crates/lucerna/src/main.rs).
- **Reproduction:** open Lucerna, then run `luc --trace-startup ~/proj` with the same binary: exits 0 at once, without printing `startup_ms`. A bench would measure wrongly with no warning.
- **Suggested fix:** when `--trace-startup` is given and the socket is already held, report a clear error.

### M2-2 [P3] Cmd+Q while another window has a save dialog is canceled silently

- **Fixed** together with M2-1 (commit 036bb30): the window that has the dialog is brought to the front.

- **Flow:** quitting the app with many windows. This bug absorbs M3-5.
- **Location:** [workbench.rs:1781](../../../crates/lucerna-workbench/src/workbench.rs).
- **Reproduction:** B has unsaved files. Click B's red button, the "Save changes?" sheet appears. Switch to A and press Cmd+Q: nothing happens.
- **Note:** blocking a second dialog is deliberate (there is a comment in the code). Only the feedback to the user is missing.
- **Suggested fix:** bring the window that is asking to the front, or wait for that dialog to be answered.

### M2-3 [P3] Clicking the Dock while the app is restoring windows adds an extra Welcome window

- **Fixed** (commit 5ceaff2): `reopen()` is skipped while restoring.

- **Flow:** starting the app.
- **Location:** [router.rs:577](../../../crates/lucerna-workbench/src/router.rs).
- **Reproduction:** `windows.json` has 2 folders. Click the Lucerna icon to start, then click again at once. The result is 2 folder windows and 1 extra Welcome window.
- **Cause:** `reopen()` does not look at the `restoring` flag, while the `luc` path does. Spec M14 requires every request to wait until restore is done.
- **Suggested fix:** `reopen()` is skipped while `restoring`.

### M3-4 [P3] Two windows share jdtls's data folder when their project roots coincide

- **Fixed** (commit 24a2628): `{data_dir}` gets an extra slot (the smallest number not yet used for the same server and root); the first window is always slot 0, keeping the old folder.

- **Flow:** a language server in two windows with nested roots.
- **Location:** [lifecycle.rs:594](../../../crates/lucerna-workbench/src/language_servers/lifecycle.rs), [vars.rs:126](../../../crates/lucerna-lsp/src/vars.rs).
- **Reproduction:** window A opens `/mono`, window B opens `/mono/svc` (which has a `pom.xml`). Open the same Java file in both: jdtls in the second window reports the workspace is locked.
- **Cause:** `data_dir_for` only hashes by the server's root.
- **Note:** the bug existed before multi-window. Only jdtls is affected. LSP is outside the scope of "basic flows".
- **Suggested fix:** add a window identifier to the hash key, or share one server when `(server, root)` coincide.

## Dropped

- **T1-4** (a file with mixed CRLF/LF is normalized to the majority style on save): deliberate behavior. See the [phase4 spec](../specs/2026-11-11-lucerna-phase4-editing-design.md) and the test `mixed_endings_pick_majority`. VS Code does the same.
- **T4-6** (`git init` after opening a folder does not turn on Git): Git still turns on by itself, because `git init` creates `.git/info/exclude` and this triggers a rescan. It only gets stuck when there is no template. The git-view spec also says "no repo means off, no notice".
- **T4-3:** duplicate of T2-2, merged into it.
- **M3-3** (Copy in the Explorer in window A, Paste in window B does nothing): deliberate. The Explorer file ops spec says the clipboard is an internal clipboard, like Zed, and before multi-window cross-window paste did not work either.
- **M3-5:** duplicate of M2-2, merged into it.

## To test by hand before release

Reading the code cannot check the following:

- [ ] Quit from the Dock and Restart the machine with unsaved files (confirm T2-1 on the real app).
- [ ] Typing Telex/VNI in Quick Open, Find and Search: does Enter to commit the word also select the result row? Try it with Apple's input method and with EVKey/OpenKey.
- [ ] Terminal with "fake backspace" style input methods (EVKey/OpenKey).
- [ ] Whether Finder sends NFC or NFD paths. Open `luc` with an NFC path while the name on disk is NFD.
- [ ] FSEvents when the root folder itself is deleted or renamed, and when the drive is remounted.
- [ ] After Close Folder or Open Folder, do the old folder's watcher, scan and LSP stop?
- [ ] Does closing a terminal tab kill the shell?
- [ ] Trash with paths that have spaces or Vietnamese, and on a volume with no Trash.
- [ ] Quitting the app while a large folder is being deleted.
- [ ] Does GPUI's `window.prompt` dialog block other actions (Delete in parallel with Rename)?
- [ ] Merge or reload exactly while typing with an IME, or while in Vim Insert mode.
- [ ] Saving a file owned by another user, or a file with xattr/ACL (does the atomic rename lose those attributes?).
- [ ] Opening the same file in two windows.
- [ ] Two `luc` running at the same time when the app is not open (M1-1), and a full backlog (macOS may return ECONNREFUSED, causing the live app's socket to be treated as stale and deleted).
- [ ] Cmd+Q when a window with unsaved files is minimized or in another Space (M2-1).
- [ ] Closing a window or changing root: do the old workbench's watcher, terminal PTY and git really stop?
- [ ] The fallback socket path when HOME is long, and TMPDIR differing between an app opened from Finder and from a terminal, tmux or ssh.
- [ ] Installing or removing an extension with many windows; drag and drop between windows.
- [ ] Clicking the Dock when every window is minimized.
