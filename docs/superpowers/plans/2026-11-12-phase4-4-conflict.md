# Lucerna Phase 4.4 (Conflicts with Agents) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When an agent rewrites a file the user is editing, Lucerna merges the agent's change into the unsaved buffer when the two do not touch, and otherwise shows a conflict bar with Reload / Keep Mine; a dirty file deleted on disk gets a bar with Close / Keep.

**Architecture:**
- `lucerna-editor::merge` (new, pure): a line-based three-way merge `base → ours`, `base → theirs` with `similar`; theirs' hunks become edits on ours, or the whole merge fails.
- `lucerna-text`: `BufferSnapshot::from_text` and `Buffer::clear_saved` (a merged buffer is never clean until saved or reloaded).
- `lucerna-workbench::Documents`: keeps `base` (the disk text as last loaded, reloaded or saved); the reload job runs the merge in the background when the buffer is dirty; `conflict: bool` becomes `Option<Conflict>` holding the disk text, so Keep Mine can adopt it. New queries `disk_notice`, actions `keep_mine`, `keep_deleted`.
- `conflict_bar.rs` (new): the bar, drawn by `EditorGroupsView` right under the path bar for each group's active file tab, built from `ui.rs` (`search_toolbar`, `search_bar_row`, `warning_row`, `text_button`).

**Tech Stack:** Rust 2024; gpui (workbench only); `similar` (already a workspace dependency).

**Spec:** `docs/superpowers/specs/2026-11-11-lucerna-phase4-editing-design.md` — §7 (whole), §6.1–6.2 (save flow, autosave), §14.2 (`lucerna.externalChanges.autoMerge`, `files.saveConflictResolution`), §16 (binary / decode errors), §17 row 4.4, §18. Research: `docs/research/2026-11-11-editing.md` §5.

## Global Constraints

- License MIT OR Apache-2.0; `cargo deny check` must pass. Never copy Zed code (GPL); the merge algorithm is written fresh.
- Only `lucerna-workbench` uses gpui. `lucerna-editor` may depend on `lucerna-text`, `unicode-segmentation`, `similar`, std.
- Offsets are UTF-8 byte offsets into LF-normalised text.
- Cargo needs `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"`. Run `scripts/prune-target.sh` before every build or test. Keep only one build profile (debug while working: delete `target/release` first unless another cargo runs — `pgrep -x cargo`).
- A parallel session works on step 4.2 in `lucerna-editor` command modules (`commands.rs`, `plan.rs`, …). **Do not edit them.** Changes to shared files (`crates/lucerna-editor/src/lib.rs`, `crates/lucerna-editor/Cargo.toml`, `default-settings.json`) stay one or two lines.
- New workbench tests go in a new file `crates/lucerna-workbench/tests/external_changes.rs` (copy the helpers it needs from `tests/save.rs`), so `tests/save.rs` stays as it is except where an existing assertion must change.
- Commit trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`; stage files by explicit path.
- Exact copy (spec §7):
  - conflict bar: `"{name} changed on disk and conflicts with your unsaved changes."`, buttons **Reload**, **Keep Mine**;
  - unreadable disk file (binary, too large, decode or read error): `"{name} changed on disk and can't be read: {err}"`, same buttons;
  - deleted: `"{name} was deleted on disk."`, buttons **Close**, **Keep**;
  - status message after a merge: `"Merged changes from disk"` (status_message, 4 s).
  - `{name}` is `save::display_name(path)`.

## Review Focus

1. **The user types while the merge job runs.** Expected: the stale merge is dropped and recomputed; never applied to the wrong text. Test in Task 3 (`typing_during_merge_job_recomputes`).
2. **Undo after a merge back past the user's edits.** Expected: the text is the old disk text and it shows dirty (the disk now has the agent's text), never clean. Test in Task 3 (`undo_past_merge_stays_dirty`).
3. **Keep Mine then save.** Expected: writes without prompting; a further agent write after Keep Mine merges against the agent's text, not the old base. Tests in Task 4.
4. **The agent writes the same text the user has.** Expected: no conflict; the document becomes clean. Test in Task 3 (`disk_equal_to_buffer_cleans`).
5. **Insertions at the very start/end and files without a final newline.** Expected: merge or conflict, never a corrupted line. Tests in Task 2 (table rows) plus the randomized property test.

---

### Task 1: `lucerna-text` support

**Files:**
- Modify: `crates/lucerna-text/src/buffer.rs`, `crates/lucerna-text/src/history.rs`
- Test: unit tests in those files

**Interfaces:**
- Produces:
  - `BufferSnapshot::from_text(text: &str) -> BufferSnapshot` (text is already LF-normalised; no conversion).
  - `Buffer::clear_saved(&mut self)`: no history state counts as saved; `is_dirty()` is true until the next `set_saved`.
  - `BufferSnapshot::text_eq(&self, other: &BufferSnapshot) -> bool` (rope equality, no copy).

- [x] **Step 1: Failing tests** — `from_text_round_trips` (`from_text("a\nb").text_for_range(0..3) == "a\nb"`, `line_count() == 2`); `clear_saved_is_dirty_until_set_saved` (fresh buffer clean → `clear_saved()` → dirty; undo/redo stays dirty; `set_saved(history_state())` → clean); `text_eq_compares_content`.
- [x] **Step 2: Run** `cargo test -p lucerna-text` — expect compile failure.
- [x] **Step 3: Implement.** `History.saved` becomes `Option<HistoryState>`; `is_dirty` is `self.saved != Some(self.state())`; `set_saved` stores `Some`; `clear_saved` stores `None`.
- [x] **Step 4: Run** `cargo test -p lucerna-text` — PASS.
- [x] **Step 5: Commit** `feat(text): snapshots from text and a buffer with no saved state`.

### Task 2: `lucerna-editor::merge`

**Files:**
- Create: `crates/lucerna-editor/src/merge.rs`
- Modify: `crates/lucerna-editor/src/lib.rs` (one line `pub mod merge;`), `crates/lucerna-editor/Cargo.toml` (`similar.workspace = true`)

**Interfaces:**
- Produces:
  ```rust
  pub struct MergeConflict; // Debug, Clone, Copy, PartialEq, Eq
  /// Edits (sorted, non-overlapping, byte ranges in `ours`) that bring
  /// `theirs`' changes from `base` into `ours`.
  pub fn merge(base: &str, ours: &str, theirs: &str)
      -> Result<Vec<(Range<usize>, Arc<str>)>, MergeConflict>;
  ```

Algorithm (spec §7.2):
1. `TextDiff::configure().algorithm(Myers).deadline(now + 1 s).diff_lines` for `base→ours` and `base→theirs`. Collect non-Equal ops as hunks `(base_lines: Range<usize>, side_lines: Range<usize>)`, merging a delete adjacent to an insert into one hunk (as `lucerna_text::diff_edits` does).
2. A theirs hunk `[a, b)` conflicts with an ours hunk `[c, d)` when `a <= d && c <= b` (overlap **or touch**; an empty range is a point). Any conflict → `Err(MergeConflict)` (no partial merge).
3. Every ours hunk is now strictly before or strictly after each theirs hunk. Ours line of base line `a` = `a + Σ(len(side) - len(base))` over ours hunks with `d < a`. Edit = ours byte range of those lines (via ours line offsets) → theirs' text for `side_lines`.
4. Shortcut: `ours == theirs` → `Ok(vec![])` is **not** done here (the caller handles it; see Task 3). `ours == base` → the theirs hunks as edits (falls out of the algorithm).

- [x] **Step 1: Failing table tests** (each asserts the text after applying the edits to `ours`, or `Err`):

| test | base | ours | theirs | expected |
|---|---|---|---|---|
| `disjoint_changes_merge` | `"a\nb\nc\nd\ne\n"` | `"A\nb\nc\nd\ne\n"` | `"a\nb\nc\nd\nE\n"` | `"A\nb\nc\nd\nE\n"` |
| `same_line_conflicts` | `"a\nb\n"` | `"x\nb\n"` | `"y\nb\n"` | `Err` |
| `adjacent_lines_conflict` | `"a\nb\nc\n"` | `"A\nb\nc\n"` | `"a\nB\nc\n"` | `Err` |
| `one_line_apart_merges` | `"a\nb\nc\n"` | `"A\nb\nc\n"` | `"a\nb\nC\n"` | `"A\nb\nC\n"` |
| `theirs_appends_at_end` | `"a\nb\nc\n"` | `"A\nb\nc\n"` | `"a\nb\nc\nd\n"` | `"A\nb\nc\nd\n"` |
| `theirs_deletes_at_end` | `"a\nb\nc\nd\n"` | `"A\nb\nc\nd\n"` | `"a\nb\nc\n"` | `"A\nb\nc\n"` |
| `insert_after_ours_shifts` | `"a\nb\nc\nd\n"` | `"a\nx\ny\nb\nc\nd\n"` (insert at 1) | `"a\nb\nc\nD\n"` | `"a\nx\ny\nb\nc\nD\n"` |
| `both_insert_at_start_conflict` | `""` | `"x\n"` | `"y\n"` | `Err` |
| `no_final_newline` | `"a\nb\nc"` | `"A\nb\nc"` | `"a\nb\nc\nd"` | `"A\nb\nc\nd"` |
| `unchanged_ours_takes_theirs` | `"a\n"` | `"a\n"` | `"b\n"` | `"b\n"` |
| `multibyte` | `"xin chào\nb\nđ\n"` | `"xin chào!\nb\nđ\n"` | `"xin chào\nb\nđường\n"` | `"xin chào!\nb\nđường\n"` |

- [x] **Step 2: Failing property test** `merge_keeps_both_sides` — 2000 cases from a small seeded LCG (no new dependency): base of 0–12 lines from a 4-word alphabet; build ours and theirs by applying 0–3 random line replace/insert/delete each. Assert: (a) `merge(base, ours, theirs).is_ok() == merge(base, theirs, ours).is_ok()`; (b) when both are `Ok`, applying the first to `ours` gives the same text as applying the second to `theirs` (both sides' hunks kept whole); (c) the result equals `theirs` when ours == base and `ours` when theirs == base.
- [x] **Step 3: Run** `cargo test -p lucerna-editor merge` — fail.
- [x] **Step 4: Implement** as above. Doc comment names spec §7.2.
- [x] **Step 5: Run** `cargo test -p lucerna-editor` — PASS. `cargo clippy -p lucerna-editor --all-targets -- -D warnings`.
- [x] **Step 6: Commit** `feat(editor): three-way line merge for external changes`.

### Task 3: Auto-merge on disk change

**Files:**
- Modify: `crates/lucerna-workbench/src/documents.rs`, `crates/lucerna-workbench/Cargo.toml` (if `lucerna-editor` is not yet a dependency, it is — check), `crates/lucerna-settings/assets/default-settings.json` (one line: `"lucerna.externalChanges.autoMerge": true,` next to the `files.*` save keys)
- Create test: `crates/lucerna-workbench/tests/external_changes.rs`

**Interfaces:**
- Consumes: Task 1 (`from_text`, `clear_saved`, `text_eq`), Task 2 (`merge`).
- Produces (inside `documents.rs`):
  - `Document.base: Option<BufferSnapshot>` — the disk text as last loaded (`take_buffer`), saved (`finish_save` `Written`: the written snapshot), or made the file's (`apply_disk`: buffer snapshot after the edit).
  - `pub(crate) struct Conflict { disk: Option<DiskText>, reason: Option<String> }`, `struct DiskText { text: BufferSnapshot, hash: u64, stamp: Stamp, decoded meta needed by set_disk_state }` (store the `Decoded` minus its `text` or keep `Arc<Decoded>`; implementer's choice). `Document.conflict: Option<Conflict>`; `set_conflict(doc, key, Option<Conflict>, cx)` emits `ConflictChanged` when `is_some()` flips. `has_conflict` unchanged in meaning.
  - `Job::Reload` gains `merge_base: Option<BufferSnapshot>` (set when the buffer is dirty, `lucerna.externalChanges.autoMerge` is true for the document's language, and `base` is `Some`) and `dirty: bool`; rename its `base` field to `ours`.
  - `Reloaded` gains `theirs: BufferSnapshot` and `outcome: ReloadOutcome` where `enum ReloadOutcome { Apply(Edits) /* clean: diff ours→theirs, or ours == theirs → empty */, Merge(Edits), Conflict }`. The job: same hash → `Apply(vec![])`; ours text == theirs text → `Apply(vec![])`; not dirty → `Apply(diff_edits)`; dirty with `merge_base` → `merge(...)` → `Merge` / `Conflict`; dirty without → `Conflict`.

`finish_reload` order:
1. hash equal to `base_hash` → as today.
2. `buffer.version() != base_version` or `buffer.is_dirty() != reloaded.dirty` → `reload_again`, return.
3. `Apply(edits)` → `apply_disk` (clean; also covers "agent wrote what the user has").
4. `Merge(edits)` → `apply_merge`: `push_stop`, edit, `push_stop`, `set_disk_state(decoded)`, `clear_saved()`, `base = theirs`, `base_hash`, `stamp`, `disk_epoch += 1`, `encoding`/`line_ending`, `set_conflict(None)`, `DocState::Ready` (+ `announce_return` if it was deleted and the patch is empty), `commit_edit`, `status_message::show("Merged changes from disk")`.
5. `Conflict` → `set_conflict(Some(Conflict { disk: Some(theirs…), reason: None }))`, state Ready, `announce_return` if it was deleted.

`reload_failed` sets `Conflict { disk: None, reason: Some(err.to_string()) }`.

- [x] **Step 1: Failing tests** in `tests/external_changes.rs` (helpers copied from `tests/save.rs`: `init`, `settle`, `editor`, `input`, `keys`, `text_of`, `is_dirty`, `has_conflict`, `external_write`, `message`, `disk`, `documents`):
  - `non_overlapping_change_merges`: file `"a\nb\nc\nd\ne\n"`; type `x` at start (`"xa\n…"`); external write `"a\nb\nc\nd\nE\n"` → text `"xa\nb\nc\nd\nE\n"`, dirty, no conflict, `message() == Some("Merged changes from disk")`.
  - `merge_is_one_undo_step`: after the above, `cmd-z` → `"xa\nb\nc\nd\ne\n"`; `cmd-z` again → `"a\nb\nc\nd\ne\n"`.
  - `undo_past_merge_stays_dirty`: after two undos, `is_dirty` is true and no conflict.
  - `save_after_merge_writes_without_prompt`: after the merge, `cmd-s` → no pending prompt, disk `"xa\nb\nc\nd\nE\n"`, clean.
  - `overlapping_change_conflicts`: file `"abc\n"`, type `x`, write `"disk\n"` → conflict, text `"xabc\n"` (4.1 behaviour kept).
  - `second_agent_write_merges_against_new_base`: after `non_overlapping_change_merges`, external write `"a\nb\nC\nd\nE\n"` → `"xa\nb\nC\nd\nE\n"`.
  - `auto_merge_off_conflicts`: user settings `{"lucerna.externalChanges.autoMerge": false}`; the first scenario ends in conflict, text unchanged.
  - `disk_equal_to_buffer_cleans`: file `"abc\n"`, type `x`, external write `"xabc\n"` → clean, no conflict.
  - `typing_during_merge_job_recomputes`: type `x`; write the disk file and send the `Modified` change **without** settling; type `y` (via `cx.simulate_input`, no settle); then `settle` → text `"xya\nb\nc\nd\nE\n"`, no conflict. (Use the first scenario's file.)
  - `conflict_clears_when_disk_merges_again`: overlapping conflict, then the agent writes a version whose change no longer touches the user's line (`"abc\nmore\n"` against base `"abc\n"`… choose: base `"abc\nz\nq\n"`, user types `x` on line 0, agent writes `"abd\nz\nq\n"` → conflict; agent then writes `"abc\nz\nQ\n"` → merged `"xabc\nz\nQ\n"`, no conflict).
- [x] **Step 2: Run** `cargo test -p lucerna-workbench --test external_changes` — fail.
- [x] **Step 3: Implement** as specified above.
- [x] **Step 4: Run** `cargo test -p lucerna-workbench --test external_changes --test save --test documents` — PASS (the existing conflict tests in `tests/save.rs` must still pass unchanged).
- [x] **Step 5: Commit** `feat(workbench): merge an agent's non-overlapping changes into unsaved text`.

### Task 4: Keep Mine, Keep and the disk notice

**Files:**
- Modify: `crates/lucerna-workbench/src/documents.rs`, `crates/lucerna-workbench/src/lib.rs` (export `DiskNotice`)
- Test: `crates/lucerna-workbench/tests/external_changes.rs`

**Interfaces:**
- Produces:
  ```rust
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub enum DiskNotice {
      /// `reason` is set when the file on disk could not be read as text.
      Conflict { reason: Option<String> },
      Deleted,
  }
  impl Documents {
      /// `Deleted` when the file is gone, the text is dirty and Keep was not
      /// chosen; else `Conflict` while in conflict; else `None`.
      pub fn disk_notice(&self, path: &Path) -> Option<DiskNotice>;
      /// Keep Mine (spec §7.3): ends the conflict; the disk text becomes the
      /// merge base and its stamp and hash the expected ones, so the next save
      /// writes without asking. Without disk text (unreadable file), the next
      /// write skips the disk check instead.
      pub fn keep_mine(&mut self, path: &Path, cx: &mut Context<Self>);
      /// Keep on a deleted file: hides the notice; the tab stays struck
      /// through and a save makes the file again.
      pub fn keep_deleted(&mut self, path: &Path, cx: &mut Context<Self>);
  }
  ```
  New `Document` fields: `deleted_kept: bool` (cleared whenever the state leaves `Deleted`), `skip_disk_check: bool` (consumed by the next `start_write`, cleared by any later disk event that reaches `finish_reload`).
  Reload on the bar calls the existing `Documents::revert`.

- [x] **Step 1: Failing tests:**
  - `keep_mine_saves_without_prompt`: overlapping conflict (`"abc\n"`, `x`, disk `"disk\n"`); `keep_mine` → `has_conflict` false, still dirty, `disk_notice == None`; `cmd-s` → no prompt, disk `"xabc\n"`.
  - `keep_mine_then_agent_writes_again_merges`: base `"abc\nz\nq\n"`, user `x` on line 0, agent `"abd\nz\nq\n"` → conflict; `keep_mine`; agent writes `"abd\nz\nQ\n"` → text `"xabc\nz\nQ\n"`, no conflict (base is now the agent's first version).
  - `keep_mine_on_unreadable_file_overwrites`: dirty `x`, disk rewritten as `b"\0\x01bin"` → `disk_notice == Some(Conflict { reason: Some(_) })`; `keep_mine`; `cmd-s` → no prompt, disk `"xabc\n"`.
  - `reload_takes_disk_undoably`: overlapping conflict; `revert` → `"disk\n"`, clean, no notice; `cmd-z` → `"xabc\n"`.
  - `deleted_dirty_file_notice`: dirty `x`, file removed (`ChangeKind::Removed`) → `disk_notice == Some(Deleted)`; `keep_deleted` → `None`, state still `Deleted`; `cmd-s` recreates the file with `"xabc\n"`.
  - `deleted_clean_file_has_no_notice`: clean file removed → `disk_notice == None`, state `Deleted`.
  - `autosave_skips_kept_deleted_file`: `files.autoSave: afterDelay`, dirty, removed, `keep_deleted`, advance clock 2 s → file still absent.
  - `overwrite_setting_keeps_the_notice`: `files.saveConflictResolution: overwriteFileOnDisk`; overlapping conflict → notice `Conflict`; `cmd-s` writes without prompt and the notice goes away.
- [x] **Step 2: Run** — fail.
- [x] **Step 3: Implement.**
- [x] **Step 4: Run** `cargo test -p lucerna-workbench --test external_changes --test save` — PASS.
- [x] **Step 5: Commit** `feat(workbench): Keep Mine, Keep, and the disk notice of a document`.

### Task 5: The conflict bar

**Files:**
- Create: `crates/lucerna-workbench/src/conflict_bar.rs` (`mod conflict_bar;` in `lib.rs`)
- Modify: `crates/lucerna-workbench/src/editor_groups/view.rs` (insert the bar between `path_bar` and the body, only for a group whose active tab is a file tab with a notice)
- Test: `crates/lucerna-workbench/tests/external_changes.rs`

**Interfaces:**
- Consumes: Task 4 (`disk_notice`, `keep_mine`, `keep_deleted`, `revert`).
- Produces: `pub(crate) fn message(name: &str, notice: &DiskNotice) -> String` (the copy) and `pub(crate) fn render(group: usize, name: &str, notice: &DiskNotice, actions: BarActions, cx: &App) -> Div` where `BarActions` holds two `ClickHandler`s (`ui::ClickHandler`): primary (Reload / Close) and secondary (Keep Mine / Keep). The view builds the handlers:
  - Reload → `documents.revert(path)` (detach the task);
  - Keep Mine → `documents.keep_mine(path)`;
  - Close → `model.close(tab_id)`, then `documents.discard(path)` when no other tab shows the path (as `request_close`'s Don't Save branch does; no prompt);
  - Keep → `documents.keep_deleted(path)`.

Layout (spec §7.3, Zed look): `search_toolbar(cx).child(search_bar_row().child(warning_row(…).flex_1().min_w_0()).child(text_button(primary)).child(text_button(secondary)))`. The message is one line: `whitespace_nowrap`, `text_ellipsis`, `UI_FONT_SIZE_SMALL`, `text` color. Debug selectors: `conflict-bar-{group}`, buttons `conflict-bar-reload-{group}`, `conflict-bar-keep-mine-{group}`, `deleted-bar-close-{group}`, `deleted-bar-keep-{group}`. Clicking a button leaves focus in the editor (`text_button` already prevents default on mouse down).

- [x] **Step 1: Failing tests** (use `drawn` and `cx.debug_bounds(sel)` + `cx.simulate_click(bounds.center(), Modifiers::none())`):
  - `bar_shows_on_conflict_and_hides_after_merge`: no bar when clean; merged change → no bar; overlapping conflict → `conflict-bar-0` and `conflict-bar-message-0` drawn. The exact copy is pinned by a unit test in `conflict_bar.rs`: `message(name, notice) -> String` returns the three Global Constraints strings.
  - `bar_reload_button`: click `conflict-bar-reload-0` → text `"disk\n"`, bar gone.
  - `bar_keep_mine_button`: click `conflict-bar-keep-mine-0` → bar gone, still dirty, `cmd-s` without prompt.
  - `deleted_bar_keep_and_close`: dirty file removed → `deleted-bar-close-0` drawn; click `deleted-bar-keep-0` → bar gone, tab still `tab-deleted-0-0`; in a second test, click `deleted-bar-close-0` → the tab is gone, no prompt, `documents.state("file.txt") == None`.
  - `bar_only_in_group_showing_the_file`: split right with another file active in group 1 → `conflict-bar-0` drawn, `conflict-bar-1` not.
- [x] **Step 2: Run** — fail.
- [x] **Step 3: Implement.** Make sure the groups view redraws on `ConflictChanged` and on `disk_notice` changes (the test proves it).
- [x] **Step 4: Run** `cargo test -p lucerna-workbench --test external_changes --test editor_groups_view --test save` — PASS.
- [ ] **Step 5: Manual look** (record in the commit body, not a blocker for tests): `cargo run -p lucerna -- <folder>`; edit a file, make an overlapping change from a shell; compare the bar's height, background and border with the find bar (⌘F) — they must match. (bỏ qua: chưa kiểm tra tay)
- [x] **Step 6: Commit** `feat(workbench): conflict bar with Reload / Keep Mine and deleted-file bar`.

### Task 6: Docs and final check

**Files:**
- Modify: spec status line (line 3: "4.1 và 4.4 đã triển khai"), `README.md` (feature list: auto-merge of agent changes, conflict bar), roadmap table §1.3 of `docs/superpowers/specs/2026-11-08-lucerna-phase1-design.md` (4.4 delivered), this plan's checkboxes.

- [x] **Step 1:** Update the docs above (Vietnamese, as the existing text).
- [x] **Step 2:** `scripts/prune-target.sh`, then `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all --check`, `cargo deny check` — all pass.
- [x] **Step 3: Commit** `docs: phase 4.4 conflicts with agents delivered`.
