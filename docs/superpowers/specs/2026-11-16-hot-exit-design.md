# Hot exit: keep unsaved changes when the app quits

Date: 2026-11-16. Source: P0 bug T2-1 in the [pre-release review](../reviews/2026-11-15-release-flow-review.md). Research: [docs/research/2026-11-16-hot-exit.md](../../research/2026-11-16-hot-exit.md).

## 1. Intent

Today, quitting Lucerna with Quit on the Dock, Log Out, restarting the machine or the AppleScript `quit` loses every unsaved change without asking. The cause:
- The "Save changes?" dialog only runs in Lucerna's own `Quit` action (Cmd+Q, menu) and when a window is closed.
- The exit paths above go through `-[NSApp terminate:]`. GPUI (`b5cd310`) does not install `applicationShouldTerminate:`, so only the `on_app_quit` hooks remain. These hooks cannot cancel the quit, and get only 200 ms (`SHUTDOWN_TIMEOUT`).

The fix is **hot exit**, like VS Code (`files.hotExit: "onExit"`, the default) and Zed (`session.restore_unsaved_buffers`):
- Lucerna always keeps a backup copy of every unsaved file.
- Quitting the app asks nothing.
- On the next launch, the tabs reappear with exactly the unsaved content.

Success means:
- Quitting the app by **any** path, even when the app crashes, loses no more than about 1 second of the last typing.
- Reopening the workspace brings back exactly the unsaved tabs, with the "unsaved" dot, with exactly the right content.
- Saving a file that changed on disk while the app was off asks before overwriting. Never silently overwrites the agent's version.

## 2. Goals, non-goals, anti-goals

**Goals**
- Continuously back up the content of every unsaved document, per workspace.
- A `files.hotExit` setting: `"onExit"` (default) or `"off"`.
- Quitting (Cmd+Q, menu, Welcome Quit, Dock Quit, Log Out/Restart) does not ask to save when `onExit`.
- Restore when the workspace is reopened, including at app startup.
- Clean up backups when they are no longer needed.

**Non-goals**
- `files.hotExit: "onExitAndWindowClose"`. This value is read as `"onExit"`.
- Keeping undo history across a quit (`files.restoreUndoStack`). Neither VS Code nor Zed keeps it.
- Showing workspaces with unrestored backups in Welcome or Open Recent (VS Code has a "More…" entry for this).
- Recreating backups for untitled files. Lucerna has no untitled files yet.
- Linux and Windows: the code does not assume macOS, but is only tested on macOS.

**Anti-goals** (achievable but still a failure)
- Silently overwriting the on-disk version that the agent changed while the app was off.
- Quitting the app noticeably slower, or typing stutter caused by writing backups.
- An old backup "coming back to life" after the user saved, reverted or chose Don't Save.
- This workspace's backups showing up in another workspace.

## 3. Constraints

- Only `lucerna-workbench` may use gpui. The format and the reading/writing of backup files should live somewhere that does not depend on gpui, so unit tests can cover them.
- `on_app_quit` cannot cancel the quit and has only 200 ms. Dock Quit and Log Out do not go through Lucerna's `Quit` action.
- One process has several windows (multi-window spec). Each window has its own `Documents`.
- Write files atomically (temp then rename), like the other state files (`workspace_state::write_atomic`).
- The author's disk is small. Backups must be cleaned up, not accumulate.
- Behavior follows VS Code, UI follows Zed (`docs/design-principles.md`).

## 4. Approaches considered

**How to handle bug T2-1** (user's choice):

| Approach | Trade-off | Result |
|---|---|---|
| A. Attach `applicationShouldTerminate:` to GPUI's delegate so every exit path asks to save | Small, but blocks Log Out/Restart waiting for the user's answer; differs from the VS Code and Zed defaults | Not chosen |
| **B. Hot exit** | Larger, but like both VS Code and Zed, does not block Log Out, and also rescues a crash | **Chosen** |

**A file on disk changed while the app was off** (user's choice):

| Approach | Trade-off | Result |
|---|---|---|
| Three-way merge on restore, using the existing merge mechanism | Can merge the agent's changes, but the backup is twice as large and more complex | Not chosen |
| **Like VS Code:** restore the user's content; only warn on save | Simple; no data loss because saving asks before overwriting | **Chosen** |

**Where to store it** (my decision): one file per document in the data directory, like VS Code. Zed's approach (one SQLite database) would need a new dependency, while all other Lucerna state is JSON files.

## 5. Design

### 5.1 When to write backups

- Each time the content of an unsaved document changes, schedule a backup write after **1000 ms** (debounce, like VS Code's `DEFAULT_BACKUP_SCHEDULE_DELAYS.default`). More typing reschedules it.
- Write on the background executor; do not block the UI thread.
- When a document stops being "unsaved" (save done, Revert, Take Theirs makes it clean, closing the tab with Don't Save), cancel the pending write and **delete** the backup.
- `files.hotExit: "off"` writes nothing, and deletes the workspace's backups on open.
- The `on_app_quit` hook immediately (synchronously) writes any backups still pending. This hook may be cut off at 200 ms. That is why backups must be written continuously rather than waiting until exit.

### 5.2 Location and format

- Directory: `<data_dir>/backups/<workspace key>/`.
  - Folder window: the key is the SHA-256 (hex) of the folder's canonical path, the same way `workspace_state::state_path` hashes.
  - A window that opened a single file: the key is the SHA-256 of `"file:" + the file's canonical path`, so it does not collide with the parent folder's window.
- One file per document: `<sha256 of the file's absolute path>`.
- File content:
  - **The first line is JSON** (one line), containing:
    - `version`
    - `path`: the absolute path
    - `mtime` and `size` of the file on disk when the document was last read or saved (the existing stamp); `null` if the file does not exist
    - `base_sha256`: the SHA-256 of the `base` text the document is based on (a UTF-8 string with `\n`, decoded, not the bytes on disk). On restore, decode the file on disk and hash that text. Do not use the existing `base_hash`, because `DefaultHasher` is not stable across Rust versions. *(My decision.)*
    - `encoding`, `bom`, `line_ending`: taken from the buffer, so it saves out in the original format
  - **From the second line on** is the buffer text as UTF-8 with `\n` line endings (the in-memory form).
- Written as temp then rename in the same directory.
- A backup file that cannot be read (corrupt JSON, wrong version) is renamed to `.bak` and skipped, like `load_state`.

### 5.3 On quit

**`files.hotExit: "onExit"`:**
- **Cmd+Q, menu Quit, Welcome Quit:**
  1. For each window, the active window first: really save the files that `files.autoSave` allows saving (like VS Code).
  2. Immediately write every pending backup and wait until done.
  3. If all writes succeed, quit without asking to save.
  4. If a write fails (for example disk full), that window falls back to the existing "Save changes?" dialog, only for the files whose write failed. Cancel cancels the whole quit.
- **A terminal running a process:** still asks as it does today. Unchanged.
- **Dock Quit, Log Out, Restart, AppleScript:** only the `on_app_quit` hook runs, writing out the pending backups. Asks nothing (because it cannot ask).

**`files.hotExit: "off"`:** keeps today's behavior (asks to save in every window on Cmd+Q).

**Actions that still ask to save as today**, even with `"onExit"` (like VS Code on macOS):
- Closing a window (red button, Cmd+Shift+W).
- Close Folder, Open Folder replacing the window.
- Closing a tab.

For these actions, Don't Save deletes the backups of those files, and Cancel leaves them alone.

### 5.4 On reopen

- When a workbench opens a workspace (at startup, Open Recent, `luc`, Finder…), it reads that workspace's backup directory.
- For each backup, including files **outside** the root (for example a file opened through go-to-definition, or another tab in a single-file window):
  - Open a tab if the file has none. Put it in the first group, without taking focus, not as a preview tab.
  - Load the **backup's text** in place of the content on disk, with `encoding`, `bom`, `line_ending` from the first line.
  - Read the file on disk (if it still exists) to get the base for later merging:
    - **SHA-256 matches `base_sha256`:** the disk has not changed. Use that content as the document's `base`, with a freshly computed `stamp` and `base_hash`. This way, if the agent writes again after the restore, it can still be auto-merged.
    - **No match, or the file is gone:** the disk changed while off. Leave `base` empty, take `stamp` from the first line, and set `base_hash` so that the pre-save disk check always treats the file as "changed" when it exists.
    - Show nothing to the user at this step *(my decision, to keep auto-merge while still matching your "like VS Code" choice)*.
  - Mark the document as unsaved.
  - Undo history starts empty: there is no undoing back to the on-disk version.
- The user chose VS Code's approach: **do not warn** at restore time that the disk changed. The existing pre-save disk check will detect that the file changed and ask before overwriting.
- **A file deleted while the app was off:** restore normally. Saving recreates the file, like VS Code and Zed.
- **A change after the restore** (the agent writes again): goes through the existing merge or conflict path. With a `base` it auto-merges as usual; with an empty `base` (disk changed while off) it always yields the conflict bar.
- **Which windows are reopened at startup:**
  - Folder windows follow `window.restoreWindows` as today. A folder's backups, if it is not reopened, are kept until the user reopens that folder.
  - A window that opened a single file and has backups is always reopened, like VS Code's "empty window". Since the directory name is only a hash, each backup directory gets one more file, `workspace.json`, recording the window kind (folder or file) and path, so startup knows which window to open *(my decision)*.
  - A single-file window with backups is always reopened, even when startup comes with a path or when the file has been deleted (fixed in [p3-fixes](2026-11-17-p3-fixes-design.md) §5).

### 5.5 Cleanup

- Delete a backup as soon as the document stops being "unsaved" (§5.1).
- Every backup in a workspace's directory belongs to that workspace, because only its window writes there, including for files outside the root. There is no filtering by path step. A backup with a corrupt first line is renamed to `.bak` (§5.2) and kept so the user can retrieve it by hand.
- Delete an empty `<workspace key>` directory.
- Do not delete the backups of a workspace that has not been reopened.

### 5.6 Two windows with the same file open

Each window has its own backup directory, so the two backups do not collide. On reopen, each window restores its own. Saving in one window makes the other see a conflict through the existing path.

## 6. Testing

- **Unit tests** for the format: write then read back; corrupt first line; wrong version; paths with spaces and Vietnamese; CRLF, BOM, UTF-16 files.
- **GPUI tests**:
  - edit → wait for the debounce → the backup file has the right content;
  - save → the backup file is deleted;
  - Revert or closing a tab with Don't Save → deleted;
  - Cmd+Q with an unsaved file → no prompt (`simulate_prompt_answer` is not called), the app quits;
  - reopen → an unsaved tab, with the right text;
  - disk changed while off → Cmd+S asks before overwriting;
  - the backup write fails on Cmd+Q → the save dialog;
  - `files.hotExit: "off"` → old behavior.
- **Manual:** Dock Quit, Log Out, and `kill -9` while typing. `TestPlatform` cannot simulate `applicationWillTerminate:`.

## 7. Left to the implementer

- Which `Documents` event to hook to know when the content changes or stops being "unsaved".
- How to split the modules. Suggestion: `hot_exit.rs` in the workbench for the gpui glue, and the format in a module that does not depend on gpui.
- `files.hotExit` is an app-scope setting: read only from the user settings, like VS Code *(my decision)*.
- A pending backup write must finish before deleting it, so that a late write does not recreate a deleted backup.
- Split `on_app_quit`'s 200 ms with the other hooks: flushing backups is the most important job.
- Put `files.hotExit` in the default settings and `docs/settings.md`. Update `docs/features.md` and remove the "No hot exit" note in the phase 4 spec.
- When done, update T2-1 in the review report.

## References

- **VS Code** (`microsoft/vscode`, `main`):
  - `files.hotExit` in `files.contribution.ts`;
  - `workingCopyBackupTracker.ts` (1000/2000 ms debounce, delete when no longer dirty, the quit flow);
  - `workingCopyBackupService.ts` (the `Backups/<workspace>/<scheme>/<hash>` layout, a header line with meta);
  - `textFileEditorModel.ts` (load from backup, the `FILE_MODIFIED_SINCE` error on save).

  Details and line numbers in [docs/research/2026-11-16-hot-exit.md](../../research/2026-11-16-hot-exit.md).
- **Zed** (`b5cd310`):
  - the `editors` table stores `contents` and `mtime` when the buffer is dirty, throttled at 200 ms;
  - the Quit action only asks for items that cannot be serialized;
  - `on_app_quit` flushes within 200 ms;
  - restore keeps `saved_mtime`, so saving sees a conflict.
- **Lucerna:**
  - [multi-window spec](2026-11-15-multi-window-design.md) §6.6 (quit);
  - `workspace_state.rs` (`state_path`, `write_atomic`);
  - `save.rs` (`changed_on_disk`, the save dialog);
  - `workbench.rs` (`quit`, `confirm_quit`).
