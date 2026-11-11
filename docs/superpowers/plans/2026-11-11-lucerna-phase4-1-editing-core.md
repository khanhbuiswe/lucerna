# Lucerna Phase 4.1 (Editing Core and Save) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the read-only code view into an editor. The user can type (IME included), move the cursor, delete, use the clipboard, undo and redo, and save. The work keeps the encoding, BOM and line ending, and never loses the user's text when an agent rewrites the file.

**Architecture:**
- `lucerna-text` gains:
  - LF-normalised storage;
  - `Patch` (offset mapping);
  - `Buffer::edit`, with a transaction `History` that stores inverse edits and opaque selection tokens;
  - dirty tracking by history state;
  - an atomic `write`.
- A new gpui-free crate, `lucerna-editor`, owns `SelectionSet`, the motions, and the edit commands as pure functions that return an `EditPlan`, plus VS Code's undo-stop rule.
- `lucerna-workbench`:
  - applies plans through `Documents`, which own the `Buffer`;
  - lets views remap through `Patch`;
  - adds an `EntityInputHandler` and a caret to `CodeView`;
  - adds save, close and quit prompts, autosave, the minimal conflict guard, and a minimal Vim Insert mode.
- Language servers get incremental `didChange` and `didSave`.

**Tech Stack:** Rust 2024; gpui (workbench only); `ropey`, `encoding_rs`, `similar` (Apache-2.0, already in `Cargo.lock`, add to `[workspace.dependencies]`), `tempfile`, `unicode-segmentation`.

**Spec:** `docs/superpowers/specs/2026-11-11-lucerna-phase4-editing-design.md`, step **4.1** in §17. Research is in `docs/research/2026-11-11-editing.md`. Read the spec section each task cites before starting it.

**Delivery split:** Tasks 1–13 are **4.1a**: the core, typing, undo, save and data-loss guard. They can ship on their own. Tasks 14–17 are **4.1b**: close and quit prompts, autosave, Vim Insert, and docs. If 4.1a runs past two weeks, ship it and plan 4.1b separately (spec §20).

## Global Constraints

- **Licensing:**
  - The project is licensed MIT OR Apache-2.0, and `cargo deny check` must pass.
  - **Never copy code from Zed's `text`, `editor`, `language`, `workspace` or `project` crates (GPL-3).** Their design may be read.
  - Code adapted from VS Code (MIT) carries the comment `// Adapted from microsoft/vscode <path> (MIT)`, and Task 17 lists each such file in `THIRD_PARTY_NOTICES.md`.
- **Crate boundaries:**
  - Only `lucerna-workbench` uses gpui.
  - `lucerna-editor` depends only on `lucerna-text`, `unicode-segmentation` and `std`.
- **Offsets:**
  - Buffer offsets are UTF-8 byte offsets into the **LF-normalised** text.
  - `Point.column` stays a UTF-8 byte column.
  - Never split a char: every motion and delete steps by grapheme cluster (`unicode-segmentation`).
- **Undo stops follow VS Code (spec §4.2).** IME composition and a Vim Insert session are one step each.
- **Settings added in this plan (exact keys and defaults):**

  | Key | Default |
  |---|---|
  | `editor.insertSpaces` | `true` |
  | `editor.trimAutoWhitespace` | `true` |
  | `editor.wordSeparators` | `` "`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?" `` |
  | `editor.cursorBlinking` | `"blink"` |
  | `editor.cursorStyle` | `"line"` |
  | `editor.emptySelectionClipboard` | `true` |
  | `files.eol` | `"auto"` |
  | `files.encoding` | `"utf8"` |
  | `files.insertFinalNewline` | `false` |
  | `files.trimFinalNewlines` | `false` |
  | `files.trimTrailingWhitespace` | `false` |
  | `files.autoSave` | `"off"` |
  | `files.autoSaveDelay` | `1000` |
  | `files.saveConflictResolution` | `"askUser"` |

- **Exact UI copy:**
  - Conflict prompt on save: message `This file has changed on disk since you started editing it. Do you want to overwrite it?`, buttons `Overwrite`, `Reload`, `Cancel`.
  - Close prompt: message `Do you want to save the changes you made to {name}?`, detail `Your changes will be lost if you don't save them.`, buttons `Save`, `Don't Save`, `Cancel`.
  - Quit or window-close prompt: message `Do you want to save the changes to the following {n} files?`, with the file names one per line in the detail, followed by `Your changes will be lost if you don't save them.`, and buttons `Save All`, `Don't Save`, `Cancel`. With one file, use the close prompt's wording.
  - Status bar after a failed save: `Failed to save '{name}': {reason}`.
  - When text cannot be encoded: `{reason}` = `{encoding} cannot encode the character at Ln {row+1}, Col {col+1}`.
  - Vim indicator: `-- INSERT --`.
- **Context keys (exact):** `editorReadonly`, `editorHasSelection`, `editorHasMultipleSelections`, `activeEditorIsDirty`.
- **Other sessions share this checkout:**
  - Run `git pull --rebase` before each task.
  - Stage by explicit path only.
  - Commit on `main`.
  - End every commit message with the trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- **Toolchain and disk:**
  - Run `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"` before cargo.
  - Keep at least 1.2 GB free (`df -h /`). Prefer `cargo test -p <crate>` over workspace-wide builds.
- **Each task ends green:** `cargo test -p <touched crates>`, `cargo clippy -p <touched crates> -- -D warnings`, and `cargo fmt --check`.
- **Language:** talk to the user in Vietnamese. Code, comments and commit messages are in English.

## Review Focus

1. **Typing while a reload is in flight.** The background reload result is computed against an older version and must be discarded and recomputed, so the user's keystrokes survive. The test is in Task 10: `reload_racing_an_edit_keeps_the_edit`.
2. **Byte-exact round trip.** Opening and saving an unedited UTF-16LE file with a BOM and CRLF endings must write identical bytes, and the same goes for UTF-8 with a BOM, Shift-JIS and Latin-1. `encoding_rs` cannot *encode* UTF-16, so Lucerna must do it itself. The test is in Task 4: `write_round_trips_every_encoding`.
3. **Two tabs on one file.** An edit in one view shifts the other view's selection when it lies after the edit, and leaves it alone when it lies before. The test is in Task 11: `edit_in_one_view_remaps_the_other`.
4. **Edits made while a save is writing.** The buffer must stay dirty after the save completes, because only the captured state becomes clean. The test is in Task 13: `edit_during_save_stays_dirty`.
5. **A save that fails half-way** (read-only directory, or a symlink to an unwritable target). The original file must be untouched, the buffer must stay dirty, and the status bar must show the reason. The tests are in Task 4 (`failed_atomic_write_leaves_original_and_no_temp`) and Task 13 (`failed_save_keeps_dirty_and_reports`).

---

### Task 1: LF-normalised storage with encoding facts

Spec §3.1 (normalisation) and D4.

**Files:**
- Modify: `crates/lucerna-text/src/load.rs` (`decode`)
- Modify: `crates/lucerna-text/src/buffer.rs`
- Modify: `crates/lucerna-text/src/lib.rs`
- Test: `crates/lucerna-text/src/buffer.rs` (`mod tests`)

**Interfaces:**
- Produces:
  - `pub struct Decoded { pub text: String /* LF only, lone \r kept */, pub encoding: &'static Encoding, pub has_bom: bool, pub line_ending: LineEnding, pub hash: u64 }`
  - `pub fn decode_file(bytes: &[u8], force_text: bool) -> Result<Decoded, LoadError>`. This is public so that `Documents` can decode off the main thread.
  - `Buffer::has_bom(&self) -> bool`, `Buffer::disk_hash(&self) -> u64` (the hash of the bytes last loaded or saved), `Buffer::from_decoded(Decoded) -> Buffer`, and `Buffer::set_line_ending(&mut self, LineEnding)`.
  - `Documents` (Task 13) calls `set_line_ending` for a file that has no line break at all. It uses `files.eol`: `"\n"` or `"\r\n"` as given, and `"auto"` means `\n` on unix and `\r\n` on Windows. `files.encoding` is only added to the defaults: `decode` always produces a guess, so the setting is not consulted in 4.1.
  - `LineEnding` is unchanged, and `Buffer::line_ending()` keeps its signature.
  - `hash` uses `std::hash::DefaultHasher::new()` over the raw bytes. It is deterministic within one process, and it is never persisted.

- [ ] **Step 1: Write failing tests**
  - `crlf_is_normalised_and_remembered`:
    - `Buffer::from_bytes(b"a\r\nb\r\n", false)` gives `snapshot.text_for_range(0..len) == "a\nb\n"` and `line_ending() == CrLf`.
    - `line(0) == "a"`.
  - `lone_cr_is_kept`: `from_bytes(b"a\rb\n")` gives the text `"a\rb\n"`.
  - `mixed_endings_pick_majority`: `b"a\r\nb\r\nc\n"` gives `CrLf`, and the text is all LF.
  - `bom_is_recorded`:
    - `b"\xEF\xBB\xBFx"` gives `has_bom() == true` and text `"x"`.
    - A UTF-16LE BOM gives `has_bom()` and `encoding() == UTF_16LE`.
    - `b"x"` gives `!has_bom()`.
  - `hash_matches_bytes`: two buffers built from the same bytes have equal `disk_hash()`, and different bytes give different hashes.
- [ ] **Step 2: Run the tests and confirm they fail.** Run `cargo test -p lucerna-text`. Expected: compile errors for `has_bom` and `disk_hash`.
- [ ] **Step 3: Implement.**
  - `decode` returns `Decoded`.
  - Normalisation replaces `"\r\n"` with `"\n"` in one pass over the decoded `String`.
  - `dominant_line_ending` runs on the pre-normalised text.
  - `Buffer::load` and `from_bytes` go through `decode_file`.
  - `Buffer::reload` keeps working on top of `Decoded`; Task 10 removes it.
  - `trim_eol` keeps stripping `\r`, because a lone `\r` before `\n` cannot occur any more but a lone `\r` at the end of a line can.
- [ ] **Step 4: Run the tests and confirm they pass.** Also run `cargo test -p lucerna-workbench --test documents --test code_view` to catch CRLF fixtures that assumed raw offsets. Fix any such fixture by asserting on LF text.
- [ ] **Step 5: Commit** `feat(text): store LF text, remember line ending, BOM and disk hash`.

### Task 2: `Patch` and `Bias`

Spec §3.1 (`Patch`).

**Files:**
- Create: `crates/lucerna-text/src/patch.rs`
- Modify: `crates/lucerna-text/src/lib.rs`, `crates/lucerna-text/src/reload.rs`

**Interfaces:**
- Produces:
  - `pub enum Bias { Left, Right }`
  - `pub struct PatchEdit { pub old: Range<usize>, pub new: Range<usize> }`
  - `pub struct Patch`, with these methods:
    - `new(edits: Vec<PatchEdit>) -> Patch`: the edits are sorted by `old.start` and do not overlap; `new` ranges are in post-edit coordinates.
    - `edits(&self) -> &[PatchEdit]`
    - `is_empty(&self) -> bool`
    - `map_offset(&self, offset: usize, bias: Bias) -> usize`
    - `compose(&self, later: &Patch) -> Patch`
    - `reload(&self, old: &BufferSnapshot, new: &BufferSnapshot) -> Reload`, a coarse row summary for the existing fold, mark and scroll users.
  - `Reload::diff` stays for now (Task 10 stops using it).

- [ ] **Step 1: Write failing tests** in `patch.rs`.
  - `map_before_inside_after`: for the patch `[old 2..4 → new 2..7]`:
    - `map_offset(1, Left) == 1`;
    - `map_offset(5, Left) == 8` (after the edit, shifted by +3);
    - `map_offset(3, Left) == 2` and `map_offset(3, Right) == 7` (inside a replaced range, clamp to the matching end);
    - `map_offset(2, Right) == 7` and `map_offset(4, Left) == 7` (the end of the old range maps to the end of the new one).
  - `insertion_bias`: for `[2..2 → 2..5]`, `map_offset(2, Left) == 2` and `map_offset(2, Right) == 5`.
  - `compose_equals_sequential`: two random patches built from random edits on a 200-byte string. For every offset, mapping through `a.compose(&b)` equals mapping through `a` then `b`. Use a fixed seed loop of 500 cases and a tiny local xorshift (no new dependency).
  - `reload_summary_rows`: replacing row 3 of a 10-line text gives `Reload { start_row: 3, old_end_row: 4, new_end_row: 4 }`.
- [ ] **Step 2: Run the tests and confirm they fail.**
- [ ] **Step 3: Implement `Patch`** using a binary search over `edits` in `map_offset`. `compose` walks both edit lists in one merge pass.
- [ ] **Step 4: Run the tests and confirm they pass.**
- [ ] **Step 5: Commit** `feat(text): add Patch for mapping offsets through edits`.

### Task 3: `Buffer::edit` and transaction `History`

Spec §3.1 (`Transaction`, `History`, dirty, `version`). Spec D3: the grouping policy lives in the caller; `History` only knows stops.

**Files:**
- Create: `crates/lucerna-text/src/history.rs`
- Modify: `crates/lucerna-text/src/buffer.rs`, `crates/lucerna-text/src/lib.rs`

**Interfaces:**
- Produces:
  - `pub type SelectionsToken = Arc<dyn Any + Send + Sync>`
  - `pub struct TransactionId(u64)` (Copy, Eq)
  - `pub struct HistoryState(Option<TransactionId>)` (Copy, Eq): the id on top of the undo stack.
  - `pub struct UndoResult { pub patch: Patch, pub selections: Option<SelectionsToken> }`
  - `Buffer` methods:
    - `edit<I: IntoIterator<Item = (Range<usize>, Arc<str>)>>(&mut self, edits: I) -> Patch`. The ranges are against the pre-edit text, sorted, and do not overlap. Inserted text is normalised from `\r\n` to `\n`. When no transaction is open, it opens and ends one.
    - `start_transaction(&mut self, selections_before: Option<SelectionsToken>)`. At depth 0 it reuses the top transaction if that transaction is still *open*; otherwise it pushes a new one carrying `selections_before`.
    - `end_transaction(&mut self, selections_after: Option<SelectionsToken>) -> Option<TransactionId>`. At depth 0 it stores `selections_after` and drops an empty new transaction. The transaction stays open.
    - `push_stop(&mut self)`: closes the top transaction.
    - `undo(&mut self) -> Option<UndoResult>` and `redo(&mut self) -> Option<UndoResult>`. Both push a stop first.
    - `version(&self) -> u64`: incremented by every edit, undo and redo.
    - `history_state(&self) -> HistoryState`
    - `set_saved(&mut self, state: HistoryState)`
    - `is_dirty(&self) -> bool`: `history_state() != saved`.
    - `begin_save(&mut self) -> (BufferSnapshot, HistoryState)`: calls `push_stop`, then captures.
  - Initial state: `saved == HistoryState(None)`, so a freshly loaded buffer is clean.

- [ ] **Step 1: Write failing tests** in `history.rs`.
  - `edit_applies_back_to_front`: `"abcdef"` with `[(1..2, "X"), (4..4, "YY")]` gives `"aXcdYYef"`, and the patch new ranges are `[1..2, 4..6]`.
  - `undo_redo_round_trip`:
    - Two separate transactions with a stop between them.
    - Undo restores the text after the first one; undo again restores the original.
    - Redo twice gives the final text.
    - Each `UndoResult.patch` maps an offset after the edit correctly.
  - `open_transaction_appends`: start/edit/end, then start/edit/end without a stop, is a single undo step.
  - `nested_transactions_join`: start, start, edit, end, end is one step.
  - `selections_tokens_returned`:
    - Undo returns the `selections_before` of the first transaction in the group.
    - Redo returns the `selections_after` of its last edit.
    - Downcast with `Arc::downcast::<u32>`.
  - `dirty_tracks_history_state`:
    - A loaded buffer is clean; an edit makes it dirty.
    - `let (_, s) = begin_save(); set_saved(s)` makes it clean.
    - Another edit makes it dirty; undo makes it clean; redo makes it dirty.
  - `new_edit_after_undo_drops_redo`.
  - `crlf_in_inserted_text_is_normalised`.
- [ ] **Step 2: Run the tests and confirm they fail.**
- [ ] **Step 3: Implement.**
  - The history entry is `{ id, edits: Vec<Inverse { range_after: Range<usize>, old_text: Arc<str>, range_before: Range<usize>, new_text: Arc<str> }>, selections_before, selections_after, open: bool }`.
  - Undo applies the inverses last to first through the same `Rope` splice code as `edit`. Redo re-applies them first to last.
  - The returned patch is the composition of the per-edit patches (Task 2).
- [ ] **Step 4: Run the tests and confirm they pass.** Add a criterion bench `benches/edit.rs`:
  - 10 000 single-char inserts into a 10 000-line rope;
  - undo of one transaction holding 10 000 edits.
  The spec §15 target is ≤ 50 ms for the undo. Record the numbers in the commit message.
- [ ] **Step 5: Commit** `feat(text): edits with transaction history, undo/redo and dirty state`.

### Task 4: Writing files

Spec §3.1 (write) and D7. Review Focus 2 and 5.

**Files:**
- Create: `crates/lucerna-text/src/write.rs`
- Modify: `crates/lucerna-text/Cargo.toml`: add `tempfile` to `[dependencies]`. Read the hardlink count with `std::os::unix::fs::MetadataExt::nlink` on unix; elsewhere, treat it as 1.

**Interfaces:**
- Produces:
  - `pub enum WriteError { Unencodable { encoding: &'static str, at: Point }, Io(String), PermissionDenied }`, with `Display` giving the reason strings from Global Constraints.
  - `pub struct Written { pub hash: u64 }`
  - `Buffer::encode(snapshot: &BufferSnapshot, encoding: &'static Encoding, has_bom: bool, line_ending: LineEnding) -> Result<Vec<u8>, WriteError>`
  - `pub fn write_bytes(path: &Path, bytes: &[u8]) -> Result<Written, WriteError>`: atomic, or in place for a symlink or `nlink > 1`.
  - `Buffer::write(&self, snapshot: &BufferSnapshot, path: &Path) -> Result<Written, WriteError>`: encode, then `write_bytes`. It uses the buffer's encoding, BOM and line ending.

- [ ] **Step 1: Write failing tests** in `write.rs` (`tempfile::TempDir`).
  - `write_round_trips_every_encoding`: for each fixture, `from_bytes(fixture)` → `write` → read back gives identical bytes. The fixtures are:
    - UTF-8;
    - UTF-8 with a BOM;
    - UTF-16LE with a BOM and CRLF;
    - UTF-16BE with a BOM;
    - Shift-JIS `"日本語\r\n"`;
    - windows-1252 `"café\n"`.
  - `edited_text_uses_file_line_ending`: a CRLF file with `"x\n"` inserted is written with `\r\n` for every line.
  - `unencodable_reports_position`: a windows-1252 buffer with `"ệ"` inserted at row 1, col 2 returns `Unencodable { at: Point { row: 1, column: 2 } }`, and the file is unchanged.
  - `atomic_write_keeps_permissions`: a file with mode `0o640` keeps it after the write (unix only).
  - `symlink_is_written_through`: writing through a symlink updates the target's bytes, and the link is still a symlink.
  - `hardlink_is_written_in_place`: with two hardlinks, both see the new bytes.
  - `failed_atomic_write_leaves_original_and_no_temp`: the directory is made read-only (`0o555`) after creating the file. The write returns `PermissionDenied`, the original bytes are intact, and no file starting with `.` + name + `.luc~` remains (unix only, and skipped when running as root).
- [ ] **Step 2: Run the tests and confirm they fail.**
- [ ] **Step 3: Implement.**
  - **UTF-16LE and UTF-16BE are encoded by hand** (`str::encode_utf16`, then `to_le_bytes` or `to_be_bytes`), because `encoding_rs` encoders for UTF-16 output UTF-8.
  - Other encodings use `Encoding::new_encoder()` with `encode_from_utf8_without_replacement`. An `Unmappable` result is converted to a `Point` with `offset_to_point`.
  - The BOM is `\u{FEFF}` encoded with the same encoder.
  - The temp file is named `.{file_name}.luc~{random 8 hex}` and created with `tempfile::Builder::new().prefix(..).tempfile_in(parent)`. The steps are: `set_permissions` from the old metadata, `write_all`, `sync_all`, then `persist(path)`.
- [ ] **Step 4: Run the tests and confirm they pass.**
- [ ] **Step 5: Commit** `feat(text): atomic, encoding-preserving file writes`.

### Task 5: `lucerna-editor` crate: selections and undo-stop rule

Spec §3.2 (Selection, SelectionSet, `UndoStops`, marked-text notation) and §4.2.

**Files:**
- Create: `crates/lucerna-editor/{Cargo.toml,src/lib.rs,src/selection.rs,src/undo_stops.rs,src/plan.rs,src/test_util.rs}`
- Modify: the root `Cargo.toml` (add `lucerna-editor` to `[workspace.dependencies]`; add `similar = "2"` and `unicode-segmentation = "1"` there if they are missing)

**Interfaces:**
- Produces:
  - `pub enum Goal { None, Column(f32) }`
  - `pub struct Selection { pub id: u64, pub anchor: usize, pub head: usize, pub goal: Goal }`, with `range() -> Range<usize>`, `is_empty()` and `reversed()`.
  - `pub struct SelectionSet`, with:
    - `cursor(offset) -> Self`
    - `from_selections(Vec<Selection>, primary: u64, merge_overlapping: bool) -> Self`
    - `all(&self) -> &[Selection]` (sorted by start)
    - `primary(&self) -> &Selection`
    - `len()`
    - `remap(&mut self, patch: &Patch)`. The head maps with `Bias::Right` when it is the later end and `Left` otherwise; empty selections use `Right`.
    - `clamp(&mut self, snapshot: &BufferSnapshot)`
    - `next_id(&mut self) -> u64`
  - The merge rule (`from_selections` with `merge = true`) merges when the selections overlap, share a start, or an empty selection touches another's boundary. Two non-empty selections that only touch stay separate. The merged primary is the one containing the old primary.
  - `pub enum EditKind { Other, DeletingLeft, DeletingRight, TypingOther, TypingFirstSpace, TypingConsecutiveSpace }`
  - `pub fn should_stop_between(prev: EditKind, cur: EditKind) -> bool` (adapted from VS Code `cursorTypeEditOperations.ts`, with the attribution comment).
  - `pub fn typing_kind(text: &str, prev: EditKind) -> EditKind`
  - `pub struct EditPlan { pub edits: Vec<(Range<usize>, Arc<str>)>, pub selections: SelectionSet /* post-edit coordinates */, pub kind: EditKind, pub stop_before: bool, pub stop_after: bool }`
  - `pub struct EditConfig { pub tab_size: u32, pub insert_spaces: bool, pub keep_indent: bool, pub trim_auto_whitespace: bool, pub word_separators: Arc<str>, pub empty_selection_clipboard: bool }`
  - `pub struct Ctx<'a> { pub snapshot: &'a BufferSnapshot, pub config: &'a EditConfig }`
  - `test_util::marked(src: &str) -> (BufferSnapshot, SelectionSet)` and `test_util::render(snapshot, &SelectionSet) -> String`. This is `pub` behind `cfg(any(test, feature = "test-util"))`.
    - `|` is a cursor.
    - `«` and `»` delimit a selection whose head is at `»`.
    - `»…«` is reversed, with its head at `»`.
    - The primary is the first selection unless it is marked with `*` before it.

- [ ] **Step 1: Write failing tests.**
  - `marked_round_trips`: for `"a|b«cd»e"`, `render(marked(s)) == s`.
  - Merge cases (with `merge = true`):
    - `"«ab»|c"` becomes `"«ab»c"`: an empty selection touching a boundary merges into it;
    - `"«ab»«cd»"` stays two selections: two non-empty selections that only touch are kept;
    - `"«a«b»c»"` becomes `"«abc»"`.
  - `remap_after_insert`: the cursor in `"ab|cd"` with an insert of `"X"` at 0 gives `"Xab|cd"`.
  - `remap_at_insert_point_moves_right`: a cursor at 2 with an insert at 2 moves to after the insert.
  - `stop_rule_matches_vscode` (each case in the table holds):
    - Typing `abc def` is the kind sequence `TypingOther×3, TypingFirstSpace, TypingOther×3`, which must give the groups `abc` and ` def`. Assert `should_stop_between(TypingOther, TypingFirstSpace) == true` and `should_stop_between(TypingFirstSpace, TypingOther) == false`.
    - Two spaces: `(TypingFirstSpace, TypingConsecutiveSpace) == false`, and then `(TypingConsecutiveSpace, TypingOther) == true`. This gives three groups for `abc  def`.
    - `(TypingOther, DeletingLeft) == true`, `(DeletingLeft, DeletingLeft) == false` and `(DeletingLeft, DeletingRight) == true`.
- [ ] **Step 2: Run the tests and confirm they fail.**
- [ ] **Step 3: Implement** the files above. `lib.rs` re-exports the public items.
- [ ] **Step 4: Run the tests, `cargo clippy -p lucerna-editor` and `cargo deny check`, and confirm they pass.**
- [ ] **Step 5: Commit** `feat(editor): new crate with selections, edit plans and VS Code undo-stop rule`.

### Task 6: Motions

Spec §3.2 (motions) and §14.1 (movement rows).

**Files:**
- Create: `crates/lucerna-editor/src/motion.rs`, `crates/lucerna-editor/src/words.rs`

**Interfaces:**
- Produces:
  - `pub enum Motion { Left, Right, Up, Down, WordStartLeft, WordEndRight, Home, End, LineStart, LineEnd, Top, Bottom, PageUp, PageDown }`
  - `pub trait Layout { fn vertical(&self, snapshot: &BufferSnapshot, offset: usize, goal: Goal, rows: i32) -> (usize, Goal); fn page_rows(&self) -> u32; fn home(&self, snapshot: &BufferSnapshot, offset: usize) -> usize; fn end(&self, snapshot: &BufferSnapshot, offset: usize) -> usize; }`. `home` and `end` are display-line boundaries, so with word wrap they act on the wrapped line, as in VS Code.
  - `pub struct PlainLayout { pub tab_size: u32, pub page_rows: u32 }`, which implements `Layout` without wrapping. It is used by the tests, and the workbench uses it when the frame is not laid out yet.
  - `pub fn move_selections(snapshot: &BufferSnapshot, set: &SelectionSet, motion: Motion, extend: bool, layout: &dyn Layout, word_separators: &str) -> SelectionSet`
  - `words::CharClass { Regular, Whitespace, Separator }` and `words::classify(c: char, separators: &str) -> CharClass` (adapted from VS Code `wordCharacterClassifier.ts`).
- Behaviour:
  - Left or Right with a non-empty selection and `!extend` collapses to the selection's start or end.
  - Up or Down keeps `Goal::Column` (display column in chars, with tabs expanded) across short lines.
  - Home is smart: the first press goes to the first non-whitespace char of the display line; if already there, it goes to column 0.
  - `WordEndRight` is VS Code's `cursorWordEndRight` (end of the next word); `WordStartLeft` is `cursorWordLeft`.
  - Top and Bottom go to offset 0 and the end of the buffer.

- [ ] **Step 1: Write failing tests** with the `marked` notation, one per motion, plus:
  - `vertical_keeps_goal`: `"abcd|e\nx\nabcdef"` → Down → Down lands at col 4 of row 2.
  - `left_collapses_selection`: `"a«bc»d"` → Left gives `"a|bcd"`.
  - `extend_keeps_anchor`: `"a|bcd"` → Right with extend twice gives `"a«bc»d"`.
  - `word_end_right_vscode`: repeated `WordEndRight` from `"|foo.bar baz"` visits `foo|.bar baz` → `foo.|bar baz` → `foo.bar| baz` → `foo.bar baz|`. A separator run counts as a word.
  - `word_start_left_vscode`: repeated `WordStartLeft` from `"foo.bar baz|"` visits `foo.bar |baz` → `foo.|bar baz` → `foo|.bar baz` → `|foo.bar baz`.
  - These two sequences are this plan's reading of VS Code's `cursorWordEndRight` and `cursorWordLeft`. Step 3 checks them against the source.
  - `smart_home`: `"  ab|c"` gives `"  |abc"`, then `"|  abc"`.
  - `graphemes_not_split`: `"a|e\u{301}b"` → Right gives `"ae\u{301}|b"`.
- [ ] **Step 2: Run the tests and confirm they fail.**
- [ ] **Step 3: Implement.** If a word-motion expectation disagrees with VS Code, read `src/vs/editor/common/cursor/cursorWordOperations.ts` (MIT) and make the test match VS Code, noting the source in the test's comment.
- [ ] **Step 4: Run the tests and confirm they pass.**
- [ ] **Step 5: Commit** `feat(editor): cursor motions with VS Code word boundaries`.

### Task 7: Edit commands

Spec §5 (4.1 parts: typing, delete, Enter keeping indent, Tab, clipboard) and §4.2 (stop flags).

**Files:**
- Create: `crates/lucerna-editor/src/commands.rs`, `crates/lucerna-editor/src/clipboard.rs`

**Interfaces:**
- Consumes: `Ctx`, `EditPlan`, `EditKind`, `typing_kind` (Task 5); `words::classify` (Task 6).
- Produces (each returns an `EditPlan` with the `kind`, `stop_before` and `stop_after` given):

  | Function | kind | stop_before / stop_after |
  |---|---|---|
  | `insert(ctx, set, text: &str, prev: EditKind) -> EditPlan` | `typing_kind(text, prev)`; a multi-char `text` (IME commit or a paste-like insert) is `Other` | from the rule / false |
  | `delete_left(ctx, set) -> EditPlan` | `DeletingLeft` | true when the delete spans lines / false |
  | `delete_right(ctx, set) -> EditPlan` | `DeletingRight` | true when the delete spans lines / false |
  | `delete_word_left`, `delete_word_right`, `delete_all_left`, `delete_all_right` | `Other` | true / true |
  | `newline(ctx, set) -> EditPlan` | `TypingOther` | true / false |
  | `tab(ctx, set) -> EditPlan` | `Other` | true / true |
  | `outdent(ctx, set) -> EditPlan` | `Other` | true / true |
  | `line_break_insert(ctx, set) -> EditPlan` | `Other` | true / true |
  | `clipboard::copy(ctx, set) -> ClipboardText` | — | — |
  | `clipboard::cut(ctx, set) -> (EditPlan, ClipboardText)` | `Other` | true / true |
  | `clipboard::paste(ctx, set, &ClipboardText) -> EditPlan` | `Other` | true / true |

  - `pub struct ClipboardText { pub text: String, pub pieces: Option<Vec<String>> /* one per selection when copied from several */, pub whole_line: bool }`
- Behaviour pinned by the spec:
  - `delete_left` with the cursor inside leading spaces deletes back to the previous tab stop: `((col - 1) / tab_size) * tab_size`.
  - `delete_left` on an empty selection at offset 0 is a no-op plan (empty edits).
  - `newline` copies the current line's leading whitespace, capped at the cursor column, when `keep_indent`.
  - `trim_auto_whitespace` (reduced scope, spec §5): when the cursor's line is whitespace-only and the cursor is at its end, the whitespace is removed as part of the same plan.
  - `tab` with an empty selection inserts up to the next tab stop: `tab_size - (display_col % tab_size)` spaces, or `"\t"` when `!insert_spaces`. With a selection spanning several lines, it indents every touched line by one unit.
  - `outdent` removes up to one indent unit from each touched line.
  - `copy` and `cut` with an empty selection and `empty_selection_clipboard` take the whole line including `\n` and set `whole_line`.
  - `paste` of a `whole_line` clipboard at an empty cursor inserts at the line start, above the current line.
  - `paste` with `pieces.len() == set.len()` gives each selection its own piece. Otherwise every selection gets the full text; the spread-by-lines mode arrives in 4.3.
  - In every plan, the selections after the edit sit at the end of the inserted text.

- [ ] **Step 1: Write failing tests** as marked-text tables, one test per function, each with single-cursor and two-cursor rows. For example:
  - `insert`: `"a|b"` + `"x"` gives `"ax|b"`;
  - `insert` over a selection: `"a«bc»d"` + `"x"` gives `"ax|d"`;
  - `delete_left` at a tab stop: `"        |x"` (8 spaces, tab size 4) gives `"    |x"`;
  - `newline` keeps indent: `"    ab|c"` gives `"    ab\n    |c"`;
  - `newline` trims auto whitespace: `"    |"` gives `"\n    |"`;
  - `tab` to a tab stop: `"ab|"` gives `"ab  |"`;
  - two cursors: `"|a\n|b"` + `"x"` gives `"x|a\nx|b"`;
  - `cut` of an empty selection takes the line: `"a\nb|c\nd"` gives text `"a\nd"`, with the clipboard holding `"bc\n"` and `whole_line`;
  - `paste_whole_line_above`: `"x|y"` with `"bc\n"` gives `"bc\nx|y"`.
- [ ] **Step 2: Run the tests and confirm they fail.**
- [ ] **Step 3: Implement.** Each function maps every selection to `(range, text)` and builds the post-edit `SelectionSet` by mapping through the plan's own patch (`Patch` from the edit list against the snapshot).
- [ ] **Step 4: Run the tests and confirm they pass.**
- [ ] **Step 5: Commit** `feat(editor): typing, delete, newline, tab and clipboard commands`.

### Task 8: Syntax follows `Patch`

Spec §3.3.

**Files:**
- Modify: `crates/lucerna-syntax/src/syntax_state.rs` (`edited`, `input_edits`)

**Interfaces:**
- Produces: `SyntaxState::edited(&self, old: &BufferSnapshot, new: &BufferSnapshot, patch: &Patch) -> SyntaxState`, which replaces the `Reload` parameter. The `InputEdit`s are built from each `PatchEdit` (byte offsets, plus `Point`s from `offset_to_point` on old and new).

- [ ] **Step 1: Write a failing test** `edited_tree_shifts_highlights_after_insert`: parse `fn a() {}\nfn b() {}`, insert `"// x\n"` at 0, and call `edited`. The highlight span for `b` (a function name) is on row 2 before any reparse.
- [ ] **Step 2: Run the test and confirm it fails.**
- [ ] **Step 3: Implement.** Update the callers in `lucerna-workbench` (`documents.rs`, `code_view/mod.rs`) to pass `&Patch`. Until Task 10, reloads build their patch with a temporary `Patch::from_reload(old, new, &reload)` helper in `lucerna-text`, which Task 10 deletes.
- [ ] **Step 4: Run `cargo test -p lucerna-syntax -p lucerna-workbench` and confirm it passes.**
- [ ] **Step 5: Commit** `refactor(syntax): edit trees from a Patch`.

### Task 9: Incremental `didChange` and `didSave`

Spec §3.5 (sync bullets).

**Files:**
- Modify: `crates/lucerna-lsp/src/sync.rs`, `crates/lucerna-lsp/src/capabilities.rs`
- Test: `crates/lucerna-workbench/tests/lsp_sync.rs` (it already has fake-server sync tests; extend it) and the unit tests in `sync.rs`

**Interfaces:**
- Produces:
  - `DocumentSync::change(&mut self, path: &Path, snapshot: &BufferSnapshot, patch: Option<&Patch>, servers: Servers<'_>)`. With `Some(patch)` and an `INCREMENTAL` server, it sends one `TextDocumentContentChangeEvent` per `PatchEdit`, in order. Each event's range is computed against the text *as the server has it after the previous events*, which means walking the edits last to first in the old snapshot. With `None` it sends the full text, as it does today.
  - `DocumentSync::did_save(&mut self, path: &Path, snapshot: &BufferSnapshot, servers: Servers<'_>)`, gated by the server's `save` capability (`TextDocumentSyncSaveOptions.include_text`).
  - The client capabilities set `synchronization.didSave = true`.

- [ ] **Step 1: Write failing tests** in `sync.rs`.
  - `incremental_change_ranges_utf16`: the old text `"aé\nb"`, an insert of `"X"` after `é` and a delete of `b`. Expected events, applied last to first: range `(1,0)-(1,1)` with text `""`, then `(0,2)-(0,2)` with text `"X"`, both in UTF-16.
  - `full_server_gets_full_text`.
  - `did_save_respects_include_text`.
- [ ] **Step 2: Run the tests and confirm they fail.**
- [ ] **Step 3: Implement.** Keep `Doc.snapshot` as the base for the range computation. Positions use the existing `to_lsp(snapshot, point, encoding)`.
- [ ] **Step 4: Run `cargo test -p lucerna-lsp` and confirm it passes.**
- [ ] **Step 5: Commit** `feat(lsp): incremental didChange and didSave`.

### Task 10: `Documents` own edits, undo and reload-as-edit

Spec §3.7 (`Documents`), §4.1, and §7.1 (4.1 subset: clean → reload as an edit; dirty → conflict flag, no merge). Review Focus 1.

**Files:**
- Modify: `crates/lucerna-workbench/src/documents.rs` and `crates/lucerna-workbench/src/language_servers/sync.rs` (handle `Edited` and `Saved`)
- Modify: `crates/lucerna-text/src/{buffer.rs,reload.rs,lib.rs}` (remove `Buffer::reload` and `Patch::from_reload`; add `diff_edits`)
- Create: `crates/lucerna-text/src/diff.rs`
- Test: `crates/lucerna-workbench/tests/documents.rs`

**Interfaces:**
- Consumes: `Buffer::edit`, the history API, `begin_save` (Task 3); `decode_file` and `disk_hash` (Task 1); `Patch` (Task 2).
- Produces:
  - `lucerna_text::diff_edits(old: &BufferSnapshot, new_text: &str) -> Vec<(Range<usize>, Arc<str>)>`: a line diff via `similar::TextDiff::configure().algorithm(Algorithm::Myers).deadline(now + 1 s).diff_lines`.
  - `DocumentEvent::Edited { path: Arc<Path>, old: BufferSnapshot, snapshot: BufferSnapshot, patch: Arc<Patch>, origin: Option<EntityId> }`. It replaces `Reloaded`, and reloads use `origin: None`.
  - `DocumentEvent::DirtyChanged { path, dirty: bool }`, `DocumentEvent::Saved { path }` and `DocumentEvent::ConflictChanged { path, conflict: bool }`.
  - `Documents` methods:
    - `apply_edits(&mut self, path: &Path, edits: Vec<(Range<usize>, Arc<str>)>, selections_before: Option<SelectionsToken>, selections_after: Option<SelectionsToken>, origin: EntityId, cx) -> Option<Patch>`
    - `push_stop(&mut self, path: &Path)`
    - `undo(&mut self, path: &Path, cx) -> Option<Option<SelectionsToken>>` and `redo(...)`
    - `is_dirty(&self, path: &Path) -> bool`
    - `has_conflict(&self, path: &Path) -> bool`
  - `Document` gains `base_hash: u64`, `conflict: bool`, `version: u64` (mirrors the buffer's).
  - **The buffer no longer leaves the main thread.** `Job::Reload` reads the bytes, calls `decode_file` and `diff_edits` against a snapshot taken when the job started, and returns `(Decoded, base_version, edits, stamp)`.
  - On the main thread:
    - If `decoded.hash == doc.base_hash`, nothing changes; only `stamp` is updated.
    - Else, if the buffer is dirty: set `conflict = true`, emit `ConflictChanged`, and leave the text alone.
    - Else, if `buffer.version() != base_version`: restart the job.
    - Else: `push_stop`, then `edit(edits)`, then `push_stop`. After that, set the encoding, BOM, line ending and `base_hash` from `decoded`, call `set_saved(history_state())`, and emit `Edited`.
  - Views get `CodeView::apply_edit(&mut self, old: &BufferSnapshot, snapshot: BufferSnapshot, patch: &Patch, cx)`, which replaces `apply_reload`. It remaps the selections (`SelectionSet::remap` after Task 11; until then, Points via `Patch::map_offset`), the folds and scroll via `patch.reload(old, new)`, the match highlights, the diagnostics (shifted until the server republishes), and the syntax (`edited`).
  - `CodeViewEvent::Reloaded` keeps its `Reload` payload (it is derived from the patch) for vim marks.

- [ ] **Step 1: Write failing tests** in `tests/documents.rs`.
  - `clean_reload_is_undoable`: open a file, rewrite it on disk, and send `WorktreeEvent::Changed`. The view shows the new text and the doc is clean. Then `undo` gives the old text, and the doc is dirty.
  - `reload_keeps_cursor_on_same_line`: a cursor on row 10, with a line inserted at row 2 on disk, ends up on row 11.
  - `dirty_doc_is_not_reloaded_and_conflicts`: `apply_edits` makes the doc dirty; after a disk rewrite and the event, the text is unchanged, `has_conflict` is true, and one `ConflictChanged { conflict: true }` was emitted.
  - `same_bytes_event_is_ignored`: rewriting identical bytes emits no `Edited` and sets no conflict.
  - `reload_racing_an_edit_keeps_the_edit`: start a reload (disk changed), call `apply_edits` before the job finishes, then run until idle. The user's edit is present. The doc is then dirty, so the disk change becomes a conflict.
  - `edit_emits_dirty_changed_once`: two edits produce one `DirtyChanged { dirty: true }`.
  - In `tests/lsp_sync.rs`, `typing_sends_incremental_change`: after `apply_edits` inserting `"x"`, the fake server receives `didChange` with one ranged event.
- [ ] **Step 2: Run the tests and confirm they fail.**
- [ ] **Step 3: Implement.** The language-server bridge handles `Edited` by calling `sync.change(path, &snapshot, Some(&patch), &servers)` (with `None` for a document that just shrank back under the size limit, as today) and handles `Saved` by calling `did_save`. Delete `Reload::diff`, `Buffer::reload` and `Patch::from_reload`.
- [ ] **Step 4: Run `cargo test -p lucerna-text -p lucerna-workbench` and confirm it passes.**
- [ ] **Step 5: Commit** `feat(workbench): documents apply edits with undo; reload is an undoable edit`.

### Task 11: `SelectionSet` in `CodeView`, caret and motion commands

Spec §3.7 (`CodeView`) and §14.1 (4.1 movement rows). Review Focus 3.

**Files:**
- Create: `crates/lucerna-workbench/src/code_view/caret.rs` (blink state)
- Create: `crates/lucerna-workbench/src/code_view/frame_layout.rs` (`impl lucerna_editor::Layout for FrameLayout`, backed by `Frame` and `display_lines`)
- Modify: `crates/lucerna-workbench/src/code_view/{mod.rs,element.rs,selection.rs}`, `crates/lucerna-workbench/src/workbench.rs` (`context_keys`)
- Create: `crates/lucerna-workbench/src/editing.rs` (command registration; `mod editing` in `lib.rs`)
- Modify: `crates/lucerna-settings/assets/default-keybindings.json` and `default-settings.json`
- Test: `crates/lucerna-workbench/tests/editing.rs` (new)

**Interfaces:**
- Consumes: `SelectionSet` and `move_selections` (Tasks 5 and 6); `Documents` events (Task 10).
- Produces:
  - `CodeView` stores `selections: SelectionSet` in place of `selection: Option<Selection>`. `block_selection` stays (it is used only by Vim until 4.3).
  - The existing Point-based API keeps its signatures and meaning, now reading the primary selection:
    - `selection() -> Option<Range<Point>>` (None when empty);
    - `cursor()`, `selection_head()`, `select(range)`, `clear_selection()`, `set_cursor(p)`.
  - New `CodeView` methods:
    - `selections(&self) -> &SelectionSet`
    - `set_selections(&mut self, set: SelectionSet, autoscroll: bool, cx)`: bumps `selection_generation`, resets the caret blink, and calls `Documents::push_stop` (every cursor move is a stop, spec §4.2).
    - `move_cursor(&mut self, motion: Motion, extend: bool, cx)`
  - Commands registered in `editing.rs` with VS Code ids:
    - `cursorLeft`, `cursorRight`, `cursorUp`, `cursorDown`, `cursorWordLeft`, `cursorWordEndRight`, `cursorHome`, `cursorEnd`, `cursorLineStart`, `cursorLineEnd`, `cursorTop`, `cursorBottom`, `cursorPageUp`, `cursorPageDown`;
    - each of these except `cursorLineStart` and `cursorLineEnd` also has a `…Select` variant;
    - `editor.action.selectAll`.
  - Key bindings exactly as spec §14.1, rows marked 4.1 for movement. They take the `when` clause `editorTextFocus && !editorReadonly`, except the pure moves, which use `editorTextFocus`.
  - Context keys:
    - `editorReadonly` is true when the doc state is not `Ready` or `Deleted`, or when the view `is_passive()`;
    - `editorHasSelection`;
    - `editorHasMultipleSelections`;
    - `activeEditorIsDirty`.
  - Caret painting: a 2 px bar (`editor.cursorStyle: line`), a block or a 2 px underline, for **every** selection head, while the view is focused and no Vim `CursorOverlay` is set. Blink is 500 ms on and 500 ms off, restarted by any selection change or edit; `editor.cursorBlinking: "solid"` disables it.

- [ ] **Step 1: Write failing tests** in `tests/editing.rs`. Open a file through the workbench test helpers (copy `init` from `tests/documents.rs`), focus the view, and dispatch keys with `cx.simulate_keystrokes`.
  - `arrow_keys_move_cursor`: `"ab\ncd"` with `right right down` gives `cursor() == Point{row:1,column:2}`.
  - `shift_arrows_extend`: `selection()` covers `0..2` on row 0.
  - `cmd_up_down_go_top_bottom`.
  - `edit_in_one_view_remaps_the_other`: split the editor (`workbench.action.splitEditor`) so two views share the file. Put view B's cursor at row 3, col 1, and in view A insert `"x\n"` at offset 0 (through `Documents::apply_edits` for now). View B's cursor is at row 4, col 1, and a view-B cursor placed before the edit is unchanged.
  - `context_keys_report_selection_and_dirty`.
  - `caret_blinks_and_solid_does_not`: with `editor.cursorBlinking: "solid"`, the caret is visible after advancing the clock by 600 ms. With `"blink"` it is hidden at 600 ms and visible at 1100 ms. Expose `CodeView::caret_visible()` for the test.
- [ ] **Step 2: Run the tests and confirm they fail.**
- [ ] **Step 3: Implement.** Mouse click and drag go through `set_selections`. Double-click (word) and triple-click (line) selection keep their current behaviour, now producing a `SelectionSet`.
- [ ] **Step 4: Run `cargo test -p lucerna-workbench` (whole crate, since the selection API is shared) and confirm it passes.**
- [ ] **Step 5: Commit** `feat(workbench): selection set, caret and cursor movement commands`.

### Task 12: Typing, IME, edit commands, undo and redo

Spec §4.1, §4.2, §5 (4.1 parts), and §14.1 (4.1 edit rows).

**Files:**
- Create: `crates/lucerna-workbench/src/code_view/input.rs` (`impl EntityInputHandler for CodeView`)
- Create: `crates/lucerna-workbench/src/code_view/transact.rs`
- Modify: `crates/lucerna-workbench/src/code_view/element.rs` (register the input handler with `window.handle_input(&focus, ElementInputHandler::new(bounds, view), cx)` during paint, as `text_input.rs` does; paint the marked-text underline)
- Modify: `crates/lucerna-workbench/src/editing.rs`, `default-keybindings.json`, `default-settings.json`
- Test: `crates/lucerna-workbench/tests/editing.rs`

**Interfaces:**
- Consumes: the commands of Task 7; `Documents::{apply_edits, push_stop, undo, redo}` (Task 10).
- Produces:
  - `CodeView::transact(&mut self, plan: EditPlan, cx)`. It:
    - applies `stop_before`, or `should_stop_between(self.last_kind, plan.kind)`, by calling `push_stop`;
    - calls `apply_edits` with `selections_before = Arc::new(self.selections.clone())` and `selections_after = Arc::new(plan.selections.clone())`;
    - sets the selections without a stop;
    - applies `stop_after`;
    - autoscrolls to the primary;
    - stores `last_kind`.
    It is a no-op when `editorReadonly`.
  - `CodeView::edit_config(&self, cx) -> EditConfig`, read from the settings with the language override.
  - IME:
    - `marked: Option<Vec<Range<usize>>>` holds one range per selection.
    - `replace_and_mark_text_in_range` replaces each selection's marked range, or the selection itself, with the composing text, and marks it. It pushes **no** stop while marked, so the whole composition is one undo step.
    - `replace_text_in_range` commits.
    - `unmark_text` clears the marked ranges.
    - Undo, Escape and a mouse click call `unmark_text` first.
    - `bounds_for_range` returns `Frame::char_position` of the primary's range start.
    - UTF-16 conversions go through the snapshot, using the same approach as `text_input.rs`.
  - Commands with their VS Code ids:
    - `deleteLeft`, `deleteRight`, `deleteWordLeft`, `deleteWordRight`, `deleteAllLeft`, `deleteAllRight`, `tab`, `outdent`, `lineBreakInsert`;
    - `undo`, `redo`, `cursorUndo`, `cursorRedo`;
    - `editor.action.clipboardCutAction` and `editor.action.clipboardPasteAction`;
    - `editor.action.clipboardCopyAction`, updated to use `clipboard::copy`, which gives whole-line copy.
  - Enter is bound to a `type` command with arg `{"text": "\n"}`, which calls `commands::newline`.
  - Multi-selection clipboard metadata is stored in `ClipboardItem` metadata as JSON `{"pieces": [...], "wholeLine": bool}`.
  - Undo and redo restore the returned `SelectionsToken` (downcast to `SelectionSet`, then `clamp`) without a stop.
  - `cursorUndo` and `cursorRedo` keep a per-view stack of the last **50** `SelectionSet`s, pushed on every `set_selections` that was not caused by an edit, and cleared by any edit to the document.
  - Settings defaults added: `editor.insertSpaces`, `editor.trimAutoWhitespace`, `editor.wordSeparators` and `editor.emptySelectionClipboard` (Global Constraints).

- [ ] **Step 1: Write failing tests** in `tests/editing.rs`.
  - `typing_inserts_text`: `simulate_input("héllo")` gives the file text `"héllo…"` and makes the doc dirty.
  - `undo_groups_like_vscode`: typing `abc def` then `cmd-z` gives `"abc"`; `cmd-z` again gives `""`; `cmd-shift-z` gives `"abc"`.
  - `cursor_move_closes_undo_group`: type `ab`, press `left`, type `c`. Undo removes only `c`.
  - `undo_back_to_saved_is_clean`.
  - `ime_composition_is_one_step`: `replace_and_mark_text_in_range(None, "a", ..)`, then `"ă"`, then `"ằ"`, then `replace_text_in_range(None, "ằ")`. The text has `ằ`, a single undo removes it all, and `marked_text_range()` is `None` after the commit.
  - `undo_cancels_composition`.
  - `cut_paste_whole_line`.
  - `enter_keeps_indent`.
  - `readonly_view_ignores_typing`: a passive picker preview view does not change.
  - `cursor_undo_restores_previous_selection`.
- [ ] **Step 2: Run the tests and confirm they fail.**
- [ ] **Step 3: Implement.** Typed text arrives only through the input handler. `dispatch_key` already leaves unmatched keys to propagate, so no keymap change is needed for characters.
- [ ] **Step 4: Run `cargo test -p lucerna-workbench` and confirm it passes.** Then run the app (`cargo run -p lucerna -- .`) and type Vietnamese with macOS Telex: the composition is underlined and commits correctly.
- [ ] **Step 5: Commit** `feat(workbench): typing with IME, edit commands, undo/redo`.

### Task 13: Saving, the dirty dot and the conflict guard

Spec §6.1, §7.1 (4.1 subset), §7.3 (tab dot and prompt only), §14.1 (save rows) and §14.2 (`files.*` rows). Review Focus 4 and 5.

**Files:**
- Create: `crates/lucerna-workbench/src/save.rs`, `crates/lucerna-workbench/src/status_message.rs`
- Modify: `crates/lucerna-workbench/src/documents.rs` (`save`), `crates/lucerna-workbench/src/editor_groups/view.rs` (dot), `crates/lucerna-workbench/src/layout/status_bar.rs` (message segment)
- Modify: `default-keybindings.json`, `default-settings.json`
- Test: `crates/lucerna-workbench/tests/save.rs` (new)

**Interfaces:**
- Consumes: `begin_save`, `set_saved`, `Buffer::write` and `WriteError` (Tasks 3 and 4); `diff_edits` (Task 10).
- Produces:
  - `pub enum SaveReason { Explicit, Auto }` and `pub struct SaveOptions { pub reason: SaveReason, pub participants: bool }`.
  - `Documents::save(&mut self, path: &Path, opts: SaveOptions, window: &mut Window, cx) -> Task<Result<bool, WriteError>>`. It returns `Ok(false)` when the user cancelled the conflict prompt.
    1. If `conflict` is set and `files.saveConflictResolution == "askUser"`, show the conflict prompt (exact copy in Global Constraints).
       - `Reload`: read the disk, apply `diff_edits` as one stop-delimited edit, `set_saved`, clear the conflict, and return `Ok(false)`.
       - `Cancel`: return `Ok(false)`.
    2. If `opts.participants`, build one transaction of edits in this order, each enabled by its setting: `trimTrailingWhitespace` (every line), `insertFinalNewline` (append `\n` if missing and the text is non-empty), `trimFinalNewlines` (collapse trailing `\n`s to one). The format slot is a no-op until 4.6.
    3. `begin_save()` gives `(snapshot, state)`. Copy the buffer's encoding, BOM flag and line ending, then call `Buffer::encode` and `write_bytes` on `cx.background_spawn`. The `Buffer` itself stays on the main thread.
    4. On success: `set_saved(state)`, `base_hash = written.hash`, update `stamp` from `fs::metadata`, clear the conflict, and emit `Saved` and `DirtyChanged` if needed.
    5. On error: `status_message::show(format!("Failed to save '{name}': {err}"))`; the buffer stays dirty.
  - Commands:
    - `workbench.action.files.save`: Explicit, with participants; it writes even when the doc is clean.
    - `workbench.action.files.saveAll`: dirty docs only.
    - `workbench.action.files.saveWithoutFormatting`: Explicit, no participants.
    - `workbench.action.files.revert`: reload from disk as an undoable edit and clear the conflict.
  - Keys: ⌘S, ⌥⌘S, ⌘K S.
  - `status_message::show(text: impl Into<SharedString>, cx: &mut App)` puts a left status-bar text segment for 4 s, replaced by the next message.
  - Tab indicator:
    - A dirty doc shows a 6 px dot in place of the × (the × shows on hover), in theme `text.accent`.
    - A conflict shows the dot in the theme's `warning` colour.
    - The existing strikethrough for deleted files stays.

- [ ] **Step 1: Write failing tests** in `tests/save.rs`.
  - `save_writes_bytes_and_cleans`: edit, then `cmd-s`. The disk bytes equal the expected text, the doc is clean, and the tab shows no dot.
  - `save_participants_follow_settings`: with `files.trimTrailingWhitespace` and `files.insertFinalNewline` true, `"a  \nb"` is written as `"a\nb\n"`, and one undo restores `"a  \nb"`.
  - `save_without_formatting_skips_participants`.
  - `own_write_does_not_reload_or_conflict`: after a save, delivering the watcher event for the path leaves no conflict and emits no `Edited`.
  - `edit_during_save_stays_dirty`: start the save, then `apply_edits` before the background write completes. When it completes, the doc is dirty.
  - `failed_save_keeps_dirty_and_reports`: point a document at a file in a directory made read-only. The doc stays dirty, and the status message starts with `Failed to save '`.
  - `conflict_prompt_overwrite`, `conflict_prompt_reload`, `conflict_prompt_cancel`: use `cx.simulate_prompt_answer(i)`. Overwrite writes the user's text. Reload gives the disk text, and undo brings the user's text back. Cancel leaves the disk and buffer as they were.
  - `overwrite_setting_skips_prompt`: with `files.saveConflictResolution: "overwriteFileOnDisk"`, no prompt appears.
  - `tab_dot_states`: no dot when clean, the accent dot when dirty, the warning dot when conflicted. Assert on a `debug_selector` of the form `tab-dirty-{gi}-{ti}` / `tab-conflict-{gi}-{ti}`.
- [ ] **Step 2: Run the tests and confirm they fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run `cargo test -p lucerna-workbench` and confirm it passes.**
- [ ] **Step 5: Commit** `feat(workbench): save, revert, dirty and conflict tab indicators`.

---

**4.1a ends here.** Run the whole-crate tests for every touched crate, `cargo deny check`, and a manual check: edit a file while Claude Code rewrites it, both clean and dirty.

---

### Task 14: Close and quit prompts

Spec §6.3.

**Files:**
- Modify: `crates/lucerna-workbench/src/editor_groups/{view.rs,mod.rs}` (route every close through `request_close`), `crates/lucerna-workbench/src/workbench.rs` (window close and quit), `crates/lucerna-workbench/src/save.rs`
- Test: `crates/lucerna-workbench/tests/save.rs`

**Interfaces:**
- Produces:
  - `EditorGroupsView::request_close(&mut self, id: TabId, window, cx)`, used by the tab ×, the middle click and `workbench.action.closeActiveEditor`.
    - If the tab is the **last** tab showing a dirty document, it shows the close prompt (exact copy). `Save` saves (Explicit) and closes on `Ok(true)`. `Don't Save` closes and discards: the document is dropped with its history, and the next open reads the disk. `Cancel` keeps the tab.
    - Otherwise it calls `model.close(id)`.
  - `Workbench::confirm_quit(window, cx) -> Task<bool>`, used by `workbench.action.quit` and by `window.on_window_should_close`. With dirty docs it shows the quit prompt: `Save All` saves each (Explicit) and quits only if all saved, `Don't Save` quits, and `Cancel` aborts.

- [ ] **Step 1: Write failing tests:** `close_dirty_tab_prompts_save`, `close_dirty_tab_dont_save_discards`, `close_dirty_tab_cancel_keeps`, `close_one_of_two_tabs_on_same_file_does_not_prompt`, `quit_with_dirty_prompts_and_cancel_aborts`.
- [ ] **Step 2: Run the tests and confirm they fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run the tests and confirm they pass.**
- [ ] **Step 5: Commit** `feat(workbench): save prompts on closing dirty tabs and quitting`.

### Task 15: Autosave

Spec §6.2.

**Files:**
- Modify: `crates/lucerna-workbench/src/save.rs`, `default-settings.json`
- Test: `crates/lucerna-workbench/tests/save.rs`

**Interfaces:**
- Produces: `AutoSave`, a workbench-owned controller. It reads `files.autoSave` and `files.autoSaveDelay` per language. It saves with `SaveOptions { reason: Auto, participants: true }`, skipping documents that are conflicted or `Deleted`.
  - `afterDelay` uses one timer per document, restarted on each `Edited`.
  - `onFocusChange` saves when a `CodeView` loses focus to anything outside it.
  - `onWindowChange` saves on window deactivation.
  - The format participant must be skipped for `Auto` (it is wired in 4.6; leave the `reason` check in place now).

- [ ] **Step 1: Write failing tests:** `after_delay_saves_after_1000ms_of_quiet` (with the clock advanced by 999 ms there is no write; at 1000 ms there is one), `after_delay_skips_conflicted`, `on_focus_change_saves_when_focus_leaves`.
- [ ] **Step 2: Run the tests and confirm they fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run the tests and confirm they pass.**
- [ ] **Step 5: Commit** `feat(workbench): files.autoSave`.

### Task 16: Minimal Vim Insert mode

Spec §13 (Phase 4 part) and D10.

**Files:**
- Modify: `crates/lucerna-vim/src/{lib.rs,state.rs}` (add `Mode::Insert` with the label `-- INSERT --`; `i a I A o O` return `Action::EnterInsert(InsertAt)`)
- Modify: `crates/lucerna-workbench/src/vim.rs`, `crates/lucerna-workbench/src/code_view/mod.rs` (`CursorShape::Bar`)
- Test: `crates/lucerna-vim/src/state.rs` (unit), `crates/lucerna-workbench/tests/vim.rs`

**Interfaces:**
- Produces:
  - `pub enum InsertAt { Before, After, LineStart /* first non-blank */, LineEnd, LineBelow, LineAbove }`
  - In Insert mode, `vim::dispatch` returns `false` for every key except Escape, so keys reach the keymap and the input handler.
  - On Escape: `Mode::Normal`, the cursor moves one char left (never past column 0), and the workbench pushes a stop.
  - While Insert is active, `CodeView` suppresses automatic stops, so the whole insert session is one undo step. This uses a `suppress_stops: bool` field, checked in `transact` and `set_selections`.
  - `o` and `O` open the line with `commands::newline`, then enter Insert. Together that is one undo step with the typed text.
  - The cursor is a `CursorShape::Bar` overlay in Insert mode and a `Block` overlay in Normal mode.

- [ ] **Step 1: Write failing tests.**
  - Unit: each of `i a I A o O` yields the right `InsertAt`, and Escape returns `Normal`.
  - Workbench `insert_session_is_one_undo_step`: with `vim.enabled`, `i`, type `abc`, Escape, `cmd-z` restores the original.
  - `escape_moves_left`.
  - `o_opens_line_below`.
  - `normal_mode_still_eats_letters`.
- [ ] **Step 2: Run the tests and confirm they fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run `cargo test -p lucerna-vim -p lucerna-workbench` and confirm it passes.**
- [ ] **Step 5: Commit** `feat(vim): minimal Insert mode on top of the editor`.

### Task 17: Docs, notices and the final check

Spec §17 (after each step) and §21.

**Files:**
- Modify: `THIRD_PARTY_NOTICES.md`: add VS Code (MIT) entries for the files adapted in Tasks 5 and 6, and Task 7 if any.
- Modify: `README.md`: state that Lucerna edits files (typing, undo, save, the conflict guard) and drop "read-only" wording.
- Modify: `docs/superpowers/specs/2026-11-08-lucerna-phase1-design.md`: in §1.3 row 4, link the Phase 4 spec and note "4.1 done". In §2.1, replace "Chỉ đọc" with a pointer to the Phase 4 spec.
- Modify: `docs/superpowers/specs/2026-11-10-lucerna-phase3-lsp-design.md` §1 and `docs/superpowers/specs/2026-11-10-vim-navigation-design.md`: add the one-line notes listed in spec §21.

- [ ] **Step 1:** Make the edits above.
- [ ] **Step 2:** Run the full check:
  ```bash
  export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
  cargo test -p lucerna-text -p lucerna-editor -p lucerna-syntax -p lucerna-lsp -p lucerna-vim -p lucerna-workbench
  cargo clippy --workspace -- -D warnings
  cargo deny check
  cargo bench -p lucerna-text --bench edit
  ```
  Expected: everything passes, and the undo bench is ≤ 50 ms.
- [ ] **Step 3: Manual check** (spec §18, last bullet).
  - Type and select side by side with Zed: the caret, the dirty dot (size and colour) and the tab hover.
  - Undo grouping side by side with VS Code: `abc def`, then a cursor move.
  - Run Claude Code against an open file in two cases:
    - clean: it auto-reloads and ⌘Z restores the old text;
    - dirty: a warning dot appears, and ⌘S prompts.
  - Type with Telex and VNI.
  - Record the results in the commit message.
- [ ] **Step 4: Commit** `docs: Phase 4.1 editing core shipped`.
