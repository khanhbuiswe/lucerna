# Fix the bugs still open after the 0.1.0 release, then V2b

Date 2026-11-17. Status: implemented, merged into `main` (9a95859).

## 1. Intent

Before moving on with features, clear out the bugs still open from the [pre-release review](../reviews/2026-11-15-release-flow-review.md): one P0 bug (T1-1), the remaining hot exit limits and the P3 bugs. After that, do V2b (Vim macros and the `:` command line) following the existing spec [2026-11-13-vim-editing-design.md](2026-11-13-vim-editing-design.md), §6.4, §8, §9, §12.

The user chose this order: fix bugs first, V2b after.

## 2. Goals, non-goals, anti-goals

- **Goals:** every bug listed in §5 is fixed and has a test. The review is marked "Fixed" item by item. The roadmap table §1.3 is updated.
- **Non-goals:** "Reopen with Encoding" / "Save with Encoding" (picking the encoding by hand), keeping the original bytes on save (approach C was rejected), new features other than V2b.
- **Anti-goals:** a fix that slows the file open path or the save path (re-measure `docs/perf-checklist.md` if `decode_file` is touched); a bar or dialog appearing for a valid UTF-8 file.

## 3. Constraints

- Behavior follows VS Code, UI follows Zed (`docs/design-principles.md`). Displayed strings are in English.
- Only `lucerna-workbench` uses gpui. No macOS dependence outside platform code.
- Tight disk: one shared `target` folder for all worktrees (`CLAUDE.local.md`), run `scripts/prune-target.sh` before each build or test. Four parallel branches have to wait for each other on the cargo lock, not make their own target.

## 4. Approaches

| Approach | Trade-off | Result |
|---|---|---|
| **One branch per group of bugs, run in parallel, merged into `claude/p3-fixes`; V2b after the merge** | Fast; the groups are split by code area so they do not conflict. Needs a merge step. | **Chosen.** Same as the `claude/p2-fixes` round. |
| Sequentially on one branch | Simple but slow, while the four groups are nearly independent. | Rejected. |
| V2b in parallel with the bug fixes | T5-4 and V2b's `:` command line both edit `quick_open/query.rs`. | Rejected. |

For T1-1, the user chose **approach A** (warn, then allow editing) of three: A warn, B open read-only, C keep the original bytes. Reason: the bug is rare, and A blocks silent file corruption at a small cost. C would need a structural change to `lucerna-text`.

## 5. Checklist

Each item: the settled fix direction, and the test needed. For location and reproduction, see the review.

### Group 1: Saving files (branch `claude/p3-save`)

- [x] **T1-1 [P0] Wrong-encoding bytes** (design in §6).
- [x] **T1-2 Autosave recreates a file the agent just deleted:** with `SaveReason::Auto`, if the file is no longer on disk (NotFound) do not write; the tab waits for the watcher event as usual. A manual save (Cmd+S) still recreates the file like VS Code. Test: autosave after deleting the file does not create the file.
- [x] **T1-3 Stamp taken after the rename:** take the metadata from the temp file's handle before the rename. For the in-place write case (`write_in_place`), take it from the handle after writing. Test: the returned stamp matches the temp file.
- [x] **T1-5 Long names:** the temp file name stays within 255 bytes. If `.{name}.luc~XXXXXXXX` goes over, cut `name` at a UTF-8 boundary (not in the middle of a character). Test: save a file with a 250-byte name, a name with Vietnamese.
- [x] **T1-6 Keep Mine does not reschedule autosave:** on receiving `ConflictChanged{conflict:false}` while the text is still dirty and `files.autoSave` is `afterDelay`, reschedule the timer. Test.
- [x] **T1-7 `trimFinalNewlines` with an all-blank-lines file:** check against `TrimFinalNewLinesParticipant` in the VS Code source and follow it (the review predicts an empty file). Fix the existing `\n` test to match.
- [x] **T2-4 Temp file left behind on Quit:** *(my decision)*
  - Lucerna's `quit()` path (Cmd+Q, menu), both the Don't Save branch and hot exit: wait for saves in progress to finish, at most 5 seconds; past that, delete the known temp files and quit.
  - The platform path (Dock Quit, Log Out, Restart) does not go through `quit()`, and GPUI only gives the `on_app_quit` hook at most 200 ms (`SHUTDOWN_TIMEOUT`). On this path only delete the known temp files in the hook, without patching GPUI. The original file stays intact, except a file with hardlinks: that file is written in place (`write_in_place`), with no temp file, so if cut off midway it may be truncated. This is a pre-existing limit, unchanged this round.
  - Test with a simulated slow save.
- [x] **Hot exit (a): the single-file window's file has been deleted:** the window is still reopened. The tab is restored **as normal**, exactly per hot-exit spec §5.4: the backup text, unsaved, no bar shown, saving recreates the file. No change for folder windows (§5.4 already works that way). *(My decision, keeping to the approved hot exit spec.)*
- [x] **Hot exit (b): startup with a path:** a **single-file** window with a backup is always reopened, even when starting with `luc <path>` or from Finder; the requested path is opened in addition as normal. Folder windows still follow `window.restoreWindows` as in §5.4: the backup waits until the folder is reopened. VS Code also only always reopens empty windows that have backups ("Empty windows with backups are always restored"); Lucerna's single-file window is the equivalent of that window. *(My decision.)*
- [x] **T1-1 through hot exit:** see §6.2, the `lossy` flag is recorded in the backup.
- [x] Update `docs/features.md`: remove the hot exit "Known limits" paragraph.

### Group 2: Explorer and watcher (branch `claude/p3-explorer`)

- [x] **T2-5 New File `foo/`:** a name ending in `/` creates a folder; `a/b/c.rs` also creates the intermediate folders (VS Code does both). Test both.
- [x] **T4-4 Watcher fails to start:** the status bar shows a warning entry (warning icon, tooltip "File watching is off for <folder name>: <reason>. Changes made outside Lucerna won't show."), for both the worktree watcher and the `.git` one. No dialog.
- [x] **T4-5 An ignored file whose parent directory is deleted:** watch the nearest ancestor directory that still exists; when the parent directory reappears, move the watch down. Test: `rm -rf dist`, open/close another tab, recreate `dist/bundle.js` → the tab reloads.

### Group 3: Quick Open, search, recent (branch `claude/p3-quick-open`)

- [x] **T3-3 Recent lost when a drive is unplugged:** the real bug is in the filtering at write time (`record`, `remove` in `recent.rs`); drop the filtering there. Do it like VS Code *(my decision, following the behavior-follows-VS-Code rule)*: Welcome, the Open Recent menu and the picker show **every** entry, no longer calling `.existing()`; choosing an entry that no longer exists removes it from the list and shows a notification dialog with only an OK button, using VS Code's exact wording (`windowsMainService.ts`): "Path does not exist" / "The path '<path>' does not exist on this computer." When there is no window (the File menu) it removes without notifying, like VS Code's menubar. *(Corrected 2026-11-17 while writing the plan: the earlier version wrongly said VS Code asks Remove / Cancel.)*
- [x] **T5-1 Leftover search observer:** when the doc is Ready but the target line no longer exists, remove the observer and put the cursor on the last line. Test.
- [x] **T5-4 `:-N` and `:N,M`:** parse the line as `i64`, a negative number counts from the end of the file (`:-1` is the last line); the line and column parts are separated by `:`, `,` or `#`. Check against VS Code's `gotoLineQuickAccess`. This is the meaning when **Vim is off**. When Vim is on (V2b), `-1` and `12,5` are not "numbers" per vim spec §8.1 so they are read as Vim ranges (`-1` relative to the cursor, `12,5` a line range). So the two meanings use two separate parse functions; V2b does not reuse this one. *(My decision.)*
- [x] **T5-5 Regex `$` with CRLF:** the matcher turns on `multi_line(true)` and `crlf(true)` but forces `line_terminator(Some(b'\n'))`; the searcher keeps the default LF. *(Corrected during implementation: setting CRLF on the searcher as the earlier version of the spec did made LF files get skipped.)* Test a CRLF file and an LF file.

### Group 4: `luc`, socket, multi-window (branch `claude/p3-cli`)

- [x] **M1-2 `luc` while the app is quitting:** the EOF part was fixed together with M1-1 (`claim_and_send` reclaims on `SendError::Dropped`). What remains is the "quitting" reply: an app that is shutting down still accepts, so reclaiming immediately would meet it again. Add a structured `Quitting` reply type to the protocol (instead of a free-form error string); a client that gets it retries claiming every 100 ms, at most 5 seconds, then starts a new app; past the deadline it reports an error as it does now. *(My decision.)*
- [x] **M1-3 Open fails after "will open when ready":** show a status message for `Source::Cli` too, not only `Source::Menu`.
- [x] **M1-4 App running in a terminal when there is no socket:** still `spawn_detached` the app (without a socket) and then `luc` exits 0, with a warning as now. Multi-window spec §6.5 is updated to match.
- [x] **M1-5 Versioned socket name:** drop the version from the socket name, keep the `-dev` suffix. An app of a different version but the same protocol version accepts requests normally. A protocol newer than the app is still refused like the current `reply_for_version`, but the message changes to "Lucerna was updated. Quit Lucerna and open it again." Multi-window spec M13/§10 is updated to match. *(My decision.)*
- [x] **M1-6 `--trace-startup` when the app is already running:** if the socket is already held, report a clear error ("Lucerna is already running; quit it to measure startup.") and exit with a nonzero code. The user's `--foreground` does the same. The child process that `detach()` starts uses a separate hidden flag so its behavior (and the M1-1 fix) does not change. *(My decision.)*
- [x] **M2-3 Clicking the Dock during restore:** `reopen()` is skipped while `restoring`.
- [x] **M3-4 jdtls sharing the data directory:** sharing one server when `(server, root)` is the same across windows is a big job; instead add a "slot" number to the hash key of `data_dir_for`: the slot is the smallest number not yet used by any running server for the same `(server, root)`. The first window always gets slot 0, so the data directory is reused across runs and does not change for someone who only opens one window. *(My decision.)*

### After merging

- [x] Review: mark each item "Fixed" with its commit, update the summary line.
- [x] Roadmap §1.3: line 5 records Git and branch switching as done (merged); add a line for this round of fixes.
- [x] Merge `claude/p3-fixes` into `main`.
- [ ] Write the V2b plan from the existing spec, work on the branch `claude/vim-v2b`.

## 6. T1-1 design: warn about wrong-encoding bytes

### 6.1 References

- **VS Code:** opens it, the wrong bytes become `U+FFFD`, saving writes `EF BF BD`, says nothing.
- **Zed:** reports an error and does not open; the `ForceOpen` command opens regardless, accepting that the broken bytes are replaced (commits `b629b1f`, `44abaed` in the Zed repo).
- **Lucerna (approach A):** opens and edits like VS Code, but warns like Zed, and asks before the first save.

### 6.2 Detection

- `Decoded` gets a `lossy: bool` field, taken from `encoding_rs`'s `had_errors` flag in both decode branches (with a BOM, and by `chardetng`). The valid-UTF-8 branch is always `false`.
- The flag travels with the document like `encoding` and `has_bom`, and is recomputed after every reload from disk.
- **Hot exit:** `BackupHeader` gets a `lossy` field, otherwise the restored text contains `U+FFFD` with no flag and Cmd+S overwrites silently again. The consent in §6.4 is not stored: after a restore, the first save asks again. An old backup without this field is treated as `false`. *(My decision.)*

### 6.3 The bar (following Zed, in the style of `conflict_bar.rs`)

- When `lossy` is `true`, show a thin bar above the editor, using the same component and warning color as the conflict bar:
  > This file contains bytes that aren't valid {encoding}. Saving will replace them with �.
- The **Dismiss** button hides the bar until the tab is reopened or the file reloads with the error still present.
- If the file is both conflicted and lossy, the conflict bar takes priority; the lossy bar shows again after the conflict is resolved.

### 6.4 The first save

- **Manual save** (Cmd+S, Save All, `:w` later, saving when closing a tab or quitting the app): the first time for that tab, ask with a warning dialog:
  - Title: "Save '{file}' and replace invalid bytes?"
  - Body: "Bytes that aren't valid {encoding} will be written as �. This can't be undone."
  - Buttons: **Save** / **Cancel** (Cancel is the default). With the current GPUI on macOS, the first button receives Return, so the order is `[Cancel, Save]`; Space still lands on Save. The implementer tries Return, Escape and Space by hand.
  - Once consent is given, later saves of that tab do not ask again; after that save the file is clean so `lossy` also becomes `false`.
- **Autosave:** skips a lossy tab that has not been consented to; the tab keeps its unsaved dot. Hot exit still backs up the text like any tab.
- **Save Without Formatting** and Save All use the same flow.

### 6.5 Tests

- `decode_file` returns `lossy` for: UTF-8 with a BOM mixed with `0xE9`; UTF-16 with an odd byte count; Shift_JIS with bad bytes. Returns `false` for valid UTF-8 and Latin-1.
- Workbench: a lossy tab shows the bar; Cmd+S asks, Cancel does not write; Save writes and does not ask next time; autosave skips it.
- Re-measure the file open time per `docs/perf-checklist.md`: it must not be slower.

## 7. Left to the implementer

- Splitting tasks in the plan, the number of commits, function names.
- The specific position of the watcher warning entry on the status bar.
- The T1-1 variant "saving fails with a multi-byte encoding" (Unencodable on a line that was not edited) is resolved along the way since if `U+FFFD` cannot be encoded: the implementer checks, and if `U+FFFD` cannot be encoded in that encoding, the §6.4 dialog still shows, but saving reports an Unencodable error as it does now.
