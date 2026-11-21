# Phase 4: full code editing

Date 2026-11-11. Status: 4.1, 4.2, 4.3 and 4.4 implemented; 4.5, 4.6 not done.

## 1. Goals

Lucerna moves from viewer to editor. Users can edit the files an agent (Claude Code, Codex) is working on without opening VS Code: typing (with Vietnamese IME), undo/redo, multi-cursor, completion, rename, format. When an agent writes a file while the user is editing, Lucerna never silently loses either side's changes.

**Current state (checked in the code, rev `7b979ef`):**
- `lucerna-text::Buffer` only has `load`, `from_bytes`, `reload`, `snapshot`; **there is no edit API**. The rope keeps the file's `\r\n` as is.
- `CodeView` has one selection (`Option<Selection>`, measured in `Point`) and Vim's block selection; there is no blinking cursor, no cursor movement commands outside Vim, and it does not receive typed text. `TextInput` (the single-line input box) already has `EntityInputHandler` with IME.
- `Documents` reloads when the file changes on disk, keeping the scroll position through `Reload` (a line diff). `SyntaxState::edited` shifts the old tree-sitter tree by `Reload`, then re-parses in the background.
- `lucerna-lsp::DocumentSync::change` always sends the **whole** content.
- `lucerna-extension` reads `line_comments` from `config.toml`, copies the `indents.scm` query, and only **validates the syntax** of snippets (there is no runtime yet; the Extensions page says "(Phase 4)").

**Settled (proposed, awaiting approval, see §19):**

| # | Decision | Reason |
|---|---|---|
| D1 | **No CRDT.** Keep `ropey`, add edit, `Patch`, `History` (inverse diffs + selections before/after) to `lucerna-text`. | Lucerna is single-user. Zed needs a CRDT for collaboration; VS Code uses inverse diffs. Cloning a `ropey` rope is O(1), so Zed's snapshot model still works. |
| D2 | **New crate `lucerna-editor` (no gpui)**: selection, movement, and every edit command as pure functions `(snapshot, selections, config) → EditPlan`. | Testable without GPUI; Vim V2 calls the same functions (§13). |
| D3 | **Undo grouping by VS Code's rules** (`EditOperationType`), not time-based grouping like Zed. IME follows Zed: a whole composition is one step. | Behavior follows VS Code; predictable. |
| D4 | **The rope stores `\n`**, remembers the file's line ending style and restores it on save. A file mixing `\r\n` and `\n` is normalized to the majority style on save. | Same as both VS Code and Zed; columns and LSP positions are simpler. |
| D5 | **Conflicts with the agent:** a clean buffer reloads (undoable); a dirty buffer whose disk change does not overlap is **auto-merged** (undoable); an overlap puts a warning dot on the tab and a **bar at the top of the editor** with Reload / Keep Mine. | An agent writing a file while the user edits is the main scenario. VS Code stays silent until save and Zed only shows a dot on the tab: both make it easy for the user to keep editing a stale version. Auto-merge follows VS Code's new `reconcileExternalChanges`. |
| D6 | **No Compare** in Phase 4. "Reload" is an undoable step, so it is not destructive: ⌘Z brings your version back. | Compare needs a diff UI, which belongs to Phase 5. |
| D7 | **Atomic file writes:** a temp file in the same directory + fsync + rename; write in place when the target is a symlink or has multiple hardlinks. | Zed overwrites in place, which can leave a half-written file on a crash; agents read files constantly, so a half-written file is dangerous. |
| D8 | **UI follows Zed:** a dirty/warning dot on the tab, a completion menu as a 280–540 px popover with docs on the right, a one-line rename box under the symbol. Commands, keys, settings follow VS Code. | `design-principles.md` §1.1. |
| D9 | **Format has a hard 5 s timeout** (Zed), without VS Code's "Saving…" + Skip notice. | No progress UI needed; being agent-first, formatting is usually done by the agent. |
| D10 | **Vim:** Phase 4 adds a minimal Insert mode (`i a I A o O`, Escape) so people with Vim on can type; operators, `.`, registers, `u` stay in V2. | Without it, Vim users could edit nothing in Phase 4. |
| D11 | **Split into six separately delivered steps** (§17). Step 4.1 includes the minimal data-loss protection. | The phase is too large to do at once. |

**Success when:**
- Open a Rust file in a trusted project: typing (including Telex), ⌘Z / ⇧⌘Z group like VS Code, ⌘S saves preserving encoding, BOM and line endings; the tab shows a dot when dirty and the dot disappears when undo returns to the saved version.
- While the user has unsaved changes at the top of the file and the agent edits the end of the file: Lucerna merges the agent's change into the buffer, reports it on the status bar, and ⌘Z removes the merged part. When the agent edits the very line the user is editing: the tab gets a warning dot and the bar appears; ⌘S asks before overwriting.
- ⌘D, ⌥⌘↓, ⇧⌥I, ⌥-click create multiple cursors; typing, deleting, paste and IME work on every cursor.
- Typing `pri` in a Rust file with `rust-analyzer`: the completion menu appears ≤ 10 ms after the server replies, Enter inserts `println!` as a snippet, Tab walks through the placeholders.
- F2 renames a function used in three files; ⇧⌥F and format-on-save call the server's formatter.
- Typing in a 10,000-line file with highlighting: every key shows up in the next frame (§15).

**Out of scope:** unnamed files (⌘N), Save As; hot exit (`files.hotExit`) and keeping undo after closing a file (`files.restoreUndoStack`); a single undo step across multiple files; Compare / diff (Phase 5); `formatOnSaveMode: modifications` (needs Git, Phase 5); format on type / on paste; code action, quick fix, signature help, inlay hint, linked editing; rename preview (⌘Enter); user-written snippets; glob-based read-only mode (`files.readonlyInclude`); `editor.columnSelection` (column selection mode toggle); Vim operators (V2).

## 2. References

Details are in `docs/research/2026-11-11-editing.md`. Summary of the points that decide the design:
- **VS Code:** undo is a list of elements, and the last element stays "open" until an undo stop; a stop occurs when the operation type changes, when the cursor moves, before Enter / paste / cut, and on save. Undo restores the selection from before the group. Reload and Revert are diff edits, undoable. A dirty buffer is never reloaded; saving when the file on disk is newer reports an error with Compare / Overwrite. `reconcileExternalChanges` (new) three-way merges external changes into a dirty buffer. The multi-cursor, movement, line, comment, completion, rename and format commands and keys are as listed in §14.
- **Zed:** transactions have depth, "finalize" blocks merging, IME merges into one step, format is one step and is cancelled if the user edits while waiting (5 s timeout). A Warning dot on the tab on conflict, an Overwrite / Discard Edits / Cancel prompt on save. The completion menu is 280–540 px wide, 3–12 rows, with docs on the right at 260–500 px. Rename is a single-line editor under the symbol, in bold. After an edit: `tree.edit` immediately, synchronous parse for 1 ms, otherwise parse in the background.
- **Lucerna departs from both** in: auto-merging non-overlapping changes and the conflict bar (D5); no Compare (D6); atomic writes (D7); a hard format timeout instead of Skip (D9); completion labels colored by kind instead of icons (§10.3).

Licenses: Zed's `text`, `editor`, `language`, `workspace`, `project` are GPL-3, so we only learn the design. Adaptations from VS Code (MIT) include the `shouldPushStackElementBetween` rule, `fuzzyScore`, the word separator character list and the snippet parser; each place records its source in a comment and in `THIRD_PARTY_NOTICES.md`. New dependencies: `similar` (Apache-2.0, already in `Cargo.lock`), `tempfile` (MIT/Apache, already present).

## 3. Architecture

```
lucerna-text      Buffer (edit, transaction, undo), Patch, History, file writing, EOL normalization
lucerna-editor    (new, no gpui) Selection, SelectionSet, movement, edit commands → EditPlan,
                  auto-close, indentation, comment, multi-cursor, snippet, completion scoring, three-way merge
lucerna-syntax    SyntaxState::edited(Patch), indents.scm, nodes for smart select
lucerna-extension reads brackets / autoclose_before / block_comment; loads snippets
lucerna-lsp       incremental didChange, didSave, completion, rename, formatting, WorkspaceEdit
lucerna-settings  new settings and keys
lucerna-vim       minimal Insert mode
lucerna-workbench Document holds the Buffer; CodeView receives text (IME), draws cursors; save, prompt, autosave;
                  conflict bar; completion menu; rename box
```

### 3.1 `lucerna-text`

- **Line ending normalization:** `from_bytes` drops the `\r` before `\n` when loading into the rope and records the majority `LineEnding` (`dominant_line_ending` already exists). A lone `\r` is kept as is. Writing the file converts `\n` to the remembered style.
- **Edit:** `Buffer::edit(edits: impl IntoIterator<Item = (Range<usize>, Arc<str>)>) -> Patch`. Ranges are in bytes on the snapshot *before* the edit, non-overlapping; applied from the end to the start. Inserted text is normalized `\r\n` → `\n`.
- **`Patch`:** a sorted list of `PatchEdit { old: Range<usize>, new: Range<usize> }`, with these functions:
  - `map_offset(offset, Bias) -> usize` to shift selections, marks, highlights, diagnostics;
  - `map_row(row) -> u32` replacing `Reload::adjust_row` (folds, scroll);
  - `compose(&Patch) -> Patch` to merge several edits in one transaction;
  - `input_edits(old, new)` for tree-sitter, `lsp_changes(old_snapshot)` for LSP.
  The existing `Reload` becomes a `Patch` computed by a line diff.
- **`Transaction`:** `{ id: TransactionId(u64), edits: Vec<InverseEdit>, kind: EditKind, selections_before: Option<SelectionsToken>, selections_after: Option<SelectionsToken>, open: bool }`. `InverseEdit` stores the new range and the old text (enough to undo), the old range and the new text (enough to redo). `SelectionsToken` is opaque data attached by `lucerna-editor` (the text crate does not know what a selection is).
- **`History`:**
  - `start_transaction(kind)` / `end_transaction()` count depth; nested ones merge into the outer transaction; an empty transaction is dropped.
  - `push_stop()` closes the open transaction (VS Code's undo stop). `group_until(id)` merges every transaction after `id` into `id` (IME, Vim insert).
  - `undo()` / `redo()` return `(Patch, Option<SelectionsToken>)`; a new edit clears the redo stack.
  - No limit on the number of steps in Phase 4; history is lost when the document closes.
- **Dirty:** `saved: Option<TransactionId>` records the id at the top of the undo stack at save or reload. `is_dirty()` when the current top id differs from `saved` (undoing back to exactly the saved point is clean, like VS Code's "alternative version id"). Redoing past the saved point is also clean again.
- **`version: u64`** increases after every edit, undo, redo; background results (parse, format, completion) carry the version and are dropped if stale.
- **Writing the file:** `Buffer::write(path, &WriteOptions) -> Result<Stamp>`:
  1. Encode the snapshot per `encoding` (add a BOM if the original file had one), convert `\n` per `LineEnding`.
  2. If `path` is a symlink or `nlink > 1`: write in place (truncate + write + fsync).
  3. Otherwise: a temp file `.<name>.luc~<random>` in the same directory, copy the old file's permissions, write, fsync, `rename` over it; on an error at any step, delete the temp file and return the error, leaving the old file intact.
  4. Return a `Stamp` (mtime, length) and the hash of the content written.
  If the encoding cannot represent a character (for example Latin-1 meeting Vietnamese text), return `WriteError::Unencodable { first: Point }` and write nothing.

### 3.2 `lucerna-editor` (new crate, no gpui)

Depends on `lucerna-text`, `lucerna-syntax` (nodes for smart select and indentation), `regex`, `similar`, `unicode-segmentation`.

- **`Selection { id, anchor: usize, head: usize, goal: Goal }`**: byte offsets; `Goal` keeps the display column when going up/down (`None | Column(f32) | Block { start, end }`). `SelectionSet { selections: Vec<Selection> (sorted, non-overlapping), primary: SelectionId }`.
  - `normalize(merge_overlapping)`: per `editor.multiCursorMergeOverlapping`; merges on overlap, same start point, or an empty cursor touching another selection's edge; two non-empty selections that merely touch stay separate (following Zed).
  - `remap(&Patch)`: shift across edits from another view or from the agent.
  - Limit `editor.multiCursorLimit` (10000).
- **Movement:** the pure function `move_selections(snapshot, set, Motion, extend: bool, &Layout) -> SelectionSet`. `Layout` is a trait implemented by the workbench (converts offset ↔ display position under word wrap, folding, tabs), so `lucerna-editor` does not need to know how drawing works. `Motion` includes Left, Right, Up, Down, WordStartLeft, WordEndRight, Home (smart: the first press goes to the first non-whitespace character of the line, the second to column 0), End, LineStart, LineEnd, Top, Bottom, PageUp, PageDown. Words follow `editor.wordSeparators` with three character classes as in VS Code.
- **Edit commands:** each command is `fn(&Ctx, &SelectionSet) -> EditPlan`, where `Ctx` has the snapshot, the language config (bracket pairs, comments), the edit settings (tab size, insert spaces, auto-close…), and syntax if available. `EditPlan { edits: Vec<(Range<usize>, Arc<str>)>, selections_after: SelectionSet, kind: EditKind, autoclose: Vec<AutocloseRegion> }`. `EditKind` = `TypingOther | TypingFirstSpace | TypingConsecutiveSpace | DeletingLeft | DeletingRight | Other`, used for the undo stop rule (§4.2).
- **`UndoStops`:** the function `should_stop_between(prev: EditKind, cur: EditKind) -> bool` adapted from VS Code; the workbench calls it before every edit.
- **Auto-close, indentation, comment, lines, multi-cursor, snippet, completion, three-way merge:** separate modules in the crate, described in §5 and §7–§10.
- **Test notation:** `marked("fn |a«bc»|")` builds a snapshot and selections from a marked string (`|` cursor, `«»` selection), newly written for Lucerna.

### 3.3 `lucerna-syntax`

- `SyntaxState::edited(old, new, &Patch)` replaces the version that took `Reload`; reload and edit take the same path.
- After an edit, the workbench tries a synchronous parse within the existing `SYNC_PARSE_BUDGET` (value unchanged, re-measured in §15); beyond it, parse incrementally in the background; the result carries `version`.
- `indent_suggestion(snapshot, row) -> Option<IndentDelta>` from `indents.scm` (captures `@indent`, `@end`, `@outdent` as Zed uses).
- `enclosing_nodes(offset_range) -> Vec<Range<usize>>` for smart select.

### 3.4 `lucerna-extension`

`LanguageConfig` additionally reads from the Zed language extension's `config.toml`: `brackets` (`start`, `end`, `close`, `newline`, `not_in`), `autoclose_before`, `block_comment` (both the array form `["/* ", " */"]` and the new table form `{ start, end, prefix, tab_size }`), `word_characters`. Missing keys use defaults: `()` `[]` `{}` pairs auto-close, no quotes. Valid snippets (VS Code JSON format: `prefix`, `body`, `description`) are loaded into a per-language `Vec<Snippet>`.

### 3.5 `lucerna-lsp`

- **`DocumentSync::change(path, old_snapshot, new_snapshot, &Patch)`**: an `INCREMENTAL` server receives range-based changes (computed from the `Patch` in the negotiated position encoding); a `FULL` server still receives everything. Sent as soon as the transaction ends (not merged per frame), so every later request sees the latest text.
- `did_save(path, text: Option)` per the `save` / `includeText` capability.
- New requests: `completion`, `completionItem/resolve`, `prepareRename`, `rename`, `documentHighlight` (to dim during rename), `formatting`, `rangeFormatting`. Declare the matching capabilities in `initialize` (`snippetSupport: true`, `resolveSupport` for `documentation`, `detail`, `additionalTextEdits`; `prepareSupport` for rename).
- `WorkspaceEdit` → `Vec<(PathBuf, Option<i32> version, Vec<TextEdit>)>`. File create / rename / delete operations in `documentChanges` are not supported: the whole edit is rejected with a message.

### 3.6 `lucerna-settings`

Add the settings and default keys of §14 to `default-settings.json` and `default-keybindings.json`.

### 3.7 `lucerna-workbench`

- **`Documents`:** `Document` holds the `Buffer` (already), and adds `base: BufferSnapshot` (the on-disk content at the latest load / reload / save, used for the three-way merge), `base_hash`, `conflict: Option<Conflict>`. New events `DocumentEvent::Edited { path, patch, snapshot, version }` (LSP, diagnostics, other views), `DirtyChanged`, `Saved`, `ConflictChanged`.
  - Reload currently moves a `Buffer` across threads. Phase 4 changes this to: a background thread reads and decodes the file and computes the diff between the current snapshot and the new content; the main thread applies the diff as an edit if `version` has not changed (like Zed), otherwise recomputes.
- **`CodeView`:** replaces `selection` and `block_selection` with `SelectionSet`; implements `EntityInputHandler` (marked text as in `TextInput`, one composition region per cursor); draws a blinking 2 px bar cursor (`editor.cursorBlinking`, `editor.cursorStyle`) for every selection; stops blinking while typing. Every edit command goes through `CodeView::transact(kind, |plan| …)`: ask for an undo stop per `UndoStops`, call `Documents::apply(path, plan)`, attach the `SelectionsToken`, scroll to the primary cursor.
- **Tab:** a dot replaces the × button when dirty (hover shows ×), in the Zed theme's `text.accent` color; a `warning` colored dot on conflict; a strikethrough name when the file has been deleted. New context keys (§14).
- **Prompt:** uses GPUI's `window.prompt` (a system dialog on macOS, GPUI's own drawn version on other platforms).
- **New modules:** `editing.rs` (registers edit commands), `save.rs` (save flow, autosave, close prompt), `conflict_bar.rs`, `completion_menu.rs`, `snippet_session.rs`, `rename.rs`, `format.rs`.

## 4. Edit and undo model

### 4.1 The path of a keystroke

1. GPUI calls `replace_text_in_range` (typed text, IME) or the keymap runs a command (`deleteLeft`, `tab`…).
2. `CodeView` builds a `Ctx`, calls the `lucerna-editor` function, and receives an `EditPlan`.
3. `UndoStops`: if `should_stop_between(prev_kind, plan.kind)` then `history.push_stop()`.
4. `Documents::apply` → `Buffer::edit` → `Patch`; `version` increases.
5. The open transaction receives the `InverseEdit` and `selections_after`; a new transaction also receives `selections_before`.
6. Emit `Edited`: other views `remap` selections, folds, search highlights, Vim marks; diagnostics shift by the `Patch` until the server sends new ones; `SyntaxState::edited` then parse; LSP `didChange`.
7. The view redraws in the next frame with the shifted syntax tree (approximate colors) if the parse has not finished.

### 4.2 Undo stops (following VS Code)

| Event | Stop |
|---|---|
| Typing a character, a different kind from the previous per `should_stop_between` | before |
| Enter | before |
| Paste, cut, surround, tab / outdent, line commands, comment | before and after |
| ⌫ / ⌦ when the previous kind differs, or deleting across several lines | before |
| Any cursor movement (key, click, ⌘D…) | before |
| Saving the file | before (so ⌘Z returns exactly to the saved version) |
| Reload, merge from disk, applying completion, rename, format | before and after (a separate step, not merged) |
| One IME composition | the whole composition is one step (`group_until`) |
| Vim Insert mode | the whole insert is one step |

Undo / redo restore the transaction's before / after selection into the view performing the undo. `cursorUndo` (⌘U) keeps the view's 50 most recent selection states, cleared when the content changes, as in VS Code.

### 4.3 Multiple views of one file

Each tab has its own `CodeView` and `SelectionSet`; the `Buffer` and `History` belong to the `Document` (undo per file, as in both VS Code and Zed). An edit in one view shifts the other view's selections through the `Patch`.

## 5. Typing and basic edit commands

- **Typing:** insert at every selection, replacing what is selected. Non-ASCII (Vietnamese text, emoji) goes through the same path.
- **IME (following Zed):** the text being composed is actually inserted into the buffer and underlined; `marked_text_range` returns the primary cursor's region; secondary cursors receive the same text. `unmark_text` on Escape, click, or undo. `bounds_for_range` returns the primary cursor's position to place the suggestion window.
- **Delete:** `deleteLeft`, `deleteRight`, `deleteWordLeft`, `deleteWordRight`, `deleteAllLeft`, `deleteAllRight`. ⌫ inside space indentation goes back to the previous tab stop (Zed and VS Code both do this when `insertSpaces`).
- **Enter:** keeps the current line's indentation (`editor.autoIndent` ≥ `keep`); between a bracket pair with `newline = true` it inserts two lines and puts the cursor on the middle line with extra indentation (≥ `brackets`); `full` adds suggestions from `indents.scm` (typing `}` dedents). `editor.trimAutoWhitespace`: leaving a line that has only auto-inserted indentation removes that indentation.
- **Tab / ⇧Tab:** with nothing selected, inserts up to the next tab stop (spaces or `\t` per `editor.insertSpaces`); with several lines selected, indents / dedents the lines.
- **Auto-close (step 4.2):** per `editor.autoClosingBrackets` / `autoClosingQuotes` and the language's `brackets` data: only closes when the next character is in `autoclose_before` or is the end of line / whitespace; a quote does not close right after a word; `not_in` (for example inside a string) blocks it. `autoClosingOvertype: auto` only types over characters the editor inserted itself (recorded in `AutocloseRegion`, lost when the cursor leaves the region). `autoClosingDelete: auto`. `autoSurround`.
- **Indentation on file open (step 4.2):** `editor.detectIndentation` guesses tabs / spaces and width from the content (the algorithm as in VS Code: count indentation differences between consecutive lines). The status bar gains a "Spaces: 4" / "Tab Size: 4" item (display only, not clickable yet).
- **Comment (step 4.2):** `commentLine` uses `line_comments[0]`; if every non-empty line already has a comment it removes them, otherwise adds them at the smallest indentation column. `blockComment` uses `block_comment`. A language with no data makes the command do nothing.
- **Lines (step 4.2):** move / copy lines up/down, deleteLines, insertLineAfter / Before, indentLines / outdentLines, joinLines, trimTrailingWhitespace.
- **Smart select (step 4.2):** expand / shrink by tree-sitter node (`enclosing_nodes`), with a stack so shrink can go back; with no tree, word → line → whole file.
- **Clipboard:** ⌘X / ⌘C / ⌘V. With nothing selected, copy / cut takes the whole line; pasting that with an empty cursor inserts above the current line. Copying from several selections records the number of fragments as metadata in the `ClipboardItem` so paste can `spread`.

## 6. Saving

### 6.1 Save flow (`save.rs`)

1. If the document has a `conflict` not yet resolved with "Keep Mine", ask (§7.3). Cancel stops.
2. `history.push_stop()`.
3. The pre-save steps, grouped into **one** transaction (one undo step): `files.trimTrailingWhitespace` → format (if `editor.formatOnSave` and this is not an autosave, §12) → `files.insertFinalNewline` → `files.trimFinalNewlines`.
4. Capture the snapshot and `version`; write on a background thread (`Buffer::write`).
5. On success: `saved = id at the top of the stack at the captured version` (edits made during the write stay dirty, as in Zed), `base = the snapshot written`, update `stamp` and `base_hash`, `did_save` for LSP, emit `Saved`.
6. On error: a status bar message with the reason (no permission, disk full, a character that cannot be encoded at `Ln x, Col y`); the buffer stays dirty.

The watcher will report the change Lucerna itself wrote: if the content read back has a hash equal to `base_hash`, ignore it.

Commands: `workbench.action.files.save` (⌘S, writes even when not dirty, as in VS Code), `workbench.action.files.saveAll` (⌥⌘S), `workbench.action.files.saveWithoutFormatting` (⌘K S, skips step 3), `workbench.action.files.revert` (re-reads from disk, an undoable step).

### 6.2 Autosave

`files.autoSave`: `off` (default) | `afterDelay` (`files.autoSaveDelay`, 1000 ms, restarted after every edit) | `onFocusChange` (the editor loses focus) | `onWindowChange` (the window loses focus). Autosave does not format, and does not save a file that is in conflict or has been deleted.

### 6.3 Closing and quitting

- Closing the last tab of a dirty file: "Do you want to save the changes you made to {name}?" / "Your changes will be lost if you don't save them." with **Save / Don't Save / Cancel** (VS Code). "Don't Save" discards the changes (the document closes, history is lost).
- Closing the window or quitting (⌘Q) with dirty files: one prompt listing the files, **Save All / Don't Save / Cancel**.
- No hot exit in Phase 4 (§19 Q4). Hot exit is done separately, see [2026-11-16-hot-exit-design.md](2026-11-16-hot-exit-design.md).

## 7. Conflicts when the agent writes a file

### 7.1 States

The watcher (already exists) reports a file change. `Documents` reads the file in the background and compares the hash with `base_hash`:
- **Equal:** written by Lucerna itself, or the content did not change. Ignore.
- **Clean buffer:** reload: the `snapshot → disk` diff is applied as an edit (one undo step), `base = disk`, `saved` = the reload transaction. Keeps scroll and cursor (already so, now via `Patch`).
- **Dirty buffer, `lucerna.externalChanges.autoMerge` on (default):** three-way merge (§7.2). If it merges, the disk's part is applied to the buffer as one undo step (kind `External`), `base = disk`, the buffer stays dirty (the user's unsaved edits); the status bar shows "Merged changes from disk" for 4 s. If it cannot merge, it becomes a conflict.
- **Conflict:** `conflict = Some(Conflict { disk: snapshot, stamp })`. The buffer is not touched.
- **File deleted while the buffer is dirty:** the tab is struck through; a bar "{name} was deleted on disk." with "Close" and "Keep" (saving recreates the file, as in VS Code). A clean file that is deleted: keeps the Phase 1 behavior (`DocState::Deleted`, the last content stays visible).

### 7.2 Three-way merge (`lucerna-editor::merge`)

- Input: `base`, `ours` (buffer), `theirs` (disk). Line diffs `base → ours` and `base → theirs` using `similar`.
- Each `theirs` hunk is kept if its line range in `base` **neither overlaps nor is adjacent to** any `ours` hunk (adjacent also counts as a conflict, to be safe). If any hunk violates this, the whole merge fails (no partial merge).
- The result is a list of edits on `ours`, shifted across the `ours` hunks that come before.
- Runs on a background thread; applied on the main thread if `version` has not changed, otherwise rerun.

### 7.3 UI (following Zed, plus a bar)

**The corresponding Zed screen:** the tab has a Warning-colored dot in place of the × button; there is no banner; saving shows the prompt "This file has changed on disk since you started editing it. Do you want to overwrite it?" with Overwrite / Discard Edits / Cancel (`workspace/src/pane.rs`).

Lucerna:
- **Tab:** a dot in the theme's `warning` color (ordinary dirty uses `text.accent`).
- **Conflict bar** (`conflict_bar.rs`): one row at the top of the editor area, right below the path bar, `SEARCH_BAR_HEIGHT` tall, built with `search_toolbar` + `search_bar_row` from `ui.rs` so it has the same height and background as the search bar; the content is a `warning_row`: "{name} changed on disk and conflicts with your unsaved changes." plus two `text_button`s **Reload** and **Keep Mine**.
  - **Reload:** the `buffer → disk` diff is applied as one undo step; the buffer is clean; ⌘Z brings back the user's version (it is then dirty again).
  - **Keep Mine:** `base = disk`, new `stamp`, `conflict = None`; the bar hides; the next save overwrites without asking.
  - If the disk changes again while the bar is showing: recompute with the new `disk`.
- **Saving while in conflict:** a prompt using Zed's wording, with buttons **Overwrite / Reload / Cancel** (Zed calls it Discard Edits; Lucerna uses the same word as the bar).
- `files.saveConflictResolution: overwriteFileOnDisk` (VS Code) skips the prompt on save; the bar still shows.

## 8. Multi-cursor (step 4.3)

VS Code commands and keys (§14). Behavior:
- `addSelectionToNextFindMatch` (⌘D): an empty cursor → selects the word under the cursor, in whole-word + case-sensitive mode; with text already selected (all selections have the same text) → searches for that text with the Find bar's options. Search starts from the end of the most recently added selection, wraps around, and drops results overlapping an existing selection (a literal scan over 256 KiB windows on the snapshot, without copying the whole buffer). Scrolls to the new selection.
- `moveSelectionToNextFindMatch` (⌘K ⌘D): drops the newest selection, selects the next result.
- `selectHighlights` / `changeAll` (⇧⌘L / ⌘F2): every result; the original selection stays the primary.
- `insertCursorAbove` / `Below` (⌥⌘↑ / ⌥⌘↓): keeps the target column across short lines; pressing the opposite direction removes the cursor just added (Zed's stack).
- `insertCursorAtEndOfEachLineSelected` (⇧⌥I).
- `cursorColumnSelect*` (⇧⌥⌘ + arrows), ⇧⌥-drag: block selection, replacing the existing `block_selection`.
- ⌥-click adds a cursor; ⌥-click on a secondary cursor removes it. `editor.multiCursorModifier: ctrlCmd` switches to ⌘-click (⌥-click then does what ⌘-click does today: go to definition).
- Escape: several selections → only the primary selection (`removeSecondaryCursors`); one non-empty selection → a cursor (`cancelSelection`).
- Paste `editor.multiCursorPaste: spread | full` as in VS Code.
- Every command in §5 already runs on `SelectionSet` from step 4.1, so this step only adds the commands that create selections and the multi-cursor drawing.

## 9. Snippets (step 4.5)

- **Parser** (adapted from the VS Code / LSP snippet grammar): `$1`, `${1:default}` (nested), `${1|a,b|}`, `$0`, variables `TM_FILENAME`, `TM_FILENAME_BASE`, `TM_DIRECTORY`, `TM_FILEPATH`, `TM_SELECTED_TEXT`, `TM_CURRENT_LINE`, `TM_CURRENT_WORD`, `TM_LINE_INDEX`, `TM_LINE_NUMBER`, `CLIPBOARD`, `CURRENT_YEAR`, `CURRENT_MONTH`, `CURRENT_DATE`, `UUID`. An unknown variable inserts the variable name (as VS Code does). Regex transforms are not supported yet: the value is inserted untransformed.
- **Insertion:** lines after the first are indented to match the current line; tabs in the snippet change per `insertSpaces`. Inserts at every cursor.
- **Snippet session** (`snippet_session.rs`): placeholders with the same number become multiple selections (synchronized by multi-cursor). Tab / ⇧Tab (`jumpToNextSnippetPlaceholder` / `jumpToPrevSnippetPlaceholder`) walk through the placeholders; reaching `$0`, or Escape (`leaveSnippet`), or the cursor leaving the snippet region ends it. `${1|a,b|}` opens the completion menu with the choices. Context keys `inSnippetMode`, `hasNextTabstop`, `hasPrevTabstop`.
- **Sources:** an LSP `insertText` in snippet format; language extension snippets (by `prefix`, shown in the completion menu per `editor.snippetSuggestions`).

## 10. Completion (step 4.5)

### 10.1 Triggering (VS Code)

- Typing a word character, the word under the cursor is not a number, `editor.quickSuggestions` allows it for the token type (`other` on; `comments`, `strings` off; the token is determined by the highlight query capture): after `editor.quickSuggestionsDelay` (10 ms).
- Typing a server trigger character (`editor.suggestOnTriggerCharacters`).
- ⌃Space (`editor.action.triggerSuggest`); when the menu is open, ⌃Space toggles the documentation part (`toggleSuggestionDetails`).
- While open and typing more: filter again in place if the result is not `isIncomplete`, otherwise ask the server again (the old request is cancelled). Moving left past the start point or to the previous line closes it.

### 10.2 Sources and ranking

- Send `completion` to every server of the document that has `completionProvider`, and merge the results. Words from open documents of the same language (`editor.wordBasedSuggestions: matchingDocuments`) are only used when no server returns results or a server errors (like Zed's `completions.words: fallback`). Extension snippets are always added.
- Scored with `fuzzyScoreGracefulAggressive` adapted from VS Code (`base/common/filters.ts`, MIT) on `filterText` or the label; `editor.suggest.matchOnWordStartOnly`. Sorted by score descending → `sortText` → label. Filtering moves to the background when the list has more than 2,000 items.
- The pre-selected item: the first (`editor.suggestSelection: first`).

### 10.3 UI (following Zed)

**The corresponding Zed screen** (`editor/src/code_context_menus.rs`): a popover below the cursor, flipped above when there is no room; 280–540 px wide; 3–12 rows tall, a virtualized list, no scrollbar; each row is a syntax-colored label with matched characters in bold and muted detail on the right; Markdown documentation in a second popover on the right (260–500 px), or below when there is no room.

Lucerna:
- Uses `ui::popover` (background, border, `POPOVER_RADIUS` corners as for hover) and GPUI's `uniform_list`. The editor font for labels, `UI_FONT_SIZE_SMALL` for detail.
- Labels are colored by item kind (function, method → the theme's `function` color; struct, class, enum, interface → `type`; keyword → `keyword`; variable, field → `variable`; constant → `constant`; snippet → normal text color with a muted "snippet" in the detail column). **No icons** (unlike VS Code, `editor.suggest.showIcons` is ignored) because Zed uses no icons.
- Documentation: `completionItem/resolve` for the selected item and ±4 items around it; drawn with the existing `markdown.rs` (shared with hover), cache of 16 items.
- Empty state: only when invoked with ⌃Space, shows a one-line "No suggestions" in the popover then closes itself after 1.5 s.

### 10.4 Accepting an item

- Enter (`editor.acceptSuggestionOnEnter: on | smart | off`) and Tab accept; ⇧Enter / ⇧Tab accept in the opposite mode of `editor.suggest.insertMode` (`insert` default ↔ `replace`). An item's commit character (`acceptSuggestionOnCommitCharacter`) accepts it and then types that character.
- Apply: an undo stop before; replace the `insert` / `replace` range at every cursor (shifted relative to the primary cursor); insert through the snippet engine (plain text is escaped); `additionalTextEdits` that are already resolved are applied in the same transaction, unresolved ones are resolved and applied afterward if the user has not edited before that position; an undo stop after. The item's `command` is ignored (no code action yet).

## 11. Rename (step 4.6)

**The corresponding Zed screen** (`editor.rs` `rename`): a single-line editor inserted as a block right below the symbol's line, indented to the symbol's column, bold text in the same syntax color, transparent background; uses of the symbol in the file are dimmed to 0.6; Enter confirms, Escape cancels.

Lucerna:
1. F2 (`editor.action.rename`): `prepareRename` if the server supports it, otherwise take the word under the cursor. Rejected or no word: show a one-line message at the cursor (using the existing `GotoMessage`, like "No definition found").
2. A borderless popover attached right below the symbol, left-aligned to the symbol's column, containing a one-line `TextInput`: editor font, bold, the symbol's syntax color; filled with the old name, all selected (or keeps the cursor position if the cursor is inside the symbol). Width follows the content, at least 20 characters. Context key `renameInputVisible`.
3. Send `documentHighlight` (if available) to dim the uses of the symbol in the file.
4. Enter: `rename` → `WorkspaceEdit`. Check the version of each open file (a mismatch reports "{name} has changed in the meantime" and applies nothing). Apply:
   - an open file: one separate transaction per file (undo per file, as in Zed);
   - an unopened file: load into `Documents` with no view, apply the edits;
   - if more than one file changes and `files.refactoring.autoSave` is on (default): save the files that were **not dirty before**; already-dirty files are left for the user to save. With the setting off: open every edited file as a tab so the dirty dot is visible.
5. Escape or losing focus: cancel, send nothing.

## 12. Format (step 4.6)

- `editor.action.formatDocument` (⇧⌥F) calls `textDocument/formatting`; `editor.action.formatSelection` (⌘K ⌘F) calls `rangeFormatting` for every selection (an empty one means the current line).
- Choosing the server: `editor.defaultFormatter` (per language) takes a **language server name** from the server table (for example `"rust-analyzer"`), departing from VS Code because Lucerna has no extension ids; if unset, the document's first server with the capability. No server: the message "No formatter for {language}".
- `FormattingOptions` come from `tabSize`, `insertSpaces` (after detection), `files.trimTrailingWhitespace`, `files.insertFinalNewline`, `files.trimFinalNewlines`.
- **5 s timeout** (Zed `FORMAT_TIMEOUT`): beyond it, give up with the message "Formatting timed out ({server})"; when saving, it still saves, without formatting.
- **Cancel when the user edits:** the result carries `version`; if the version has changed, drop it (both VS Code and Zed).
- **Applied as a diff:** the server's edits are turned into the new text, then line-diffed against the current text (like Zed), so the cursor, folds and diagnostics do not jump. A manual format is one undo step; format on save sits inside the undo step of the pre-save operations (§6.1).
- `editor.formatOnSave` (false, per language); `editor.formatOnSaveMode` only accepts `file` (other values are treated as `file`, with a warning in the log).

## 13. Vim

**Phase 4 (step 4.1):**
- Normal / Visual stay as in V1. Keys with ⌘ (⌘Z, ⌘V, ⌘S, ⌘D…) still run the editor's edit commands.
- Minimal Insert mode: `i`, `a`, `I`, `A`, `o`, `O` enter Insert; keys in Insert go through the editor's typing path (including IME, completion, snippets); Escape returns to Normal, with the cursor moving back one character. The Insert cursor is a bar, Normal is a block (the existing `CursorOverlay` gains `CursorShape::Bar`). The status bar shows `-- INSERT --`. A whole Insert session is one undo step.
- Not yet: operators, `.`, registers, `u` / ⌃R, counts for insert, `r`, `x`… (V2).

**Contract for V2** (so V2 does not have to change the model):
1. Every text change by V2 is an `EditPlan` built with `lucerna-editor` functions (delete range, insert, replace, indent, change case); text objects use `enclosing_nodes` and the word / sentence / paragraph functions of `lucerna-editor`.
2. `CodeView::transact_group(|cx| …)` lets V2 group several `EditPlan`s (for example `3dd`, `ciw` + typed text) into one undo step (`group_until`). Already exists: `CodeView::transact_group<R>(keep_open: bool, cx, f: impl FnOnce(&mut Self, &mut Context<Self>) -> R) -> R`; `keep_open` keeps the step open for a command that ends in Insert, and inside an already-open step `f` just joins that step.
3. `.` is implemented by V2 by recording the keys of the last change (including text typed in Insert), as Vim does; the editor needs to remember nothing more.
4. Registers belong to `VimGlobals`; the unnamed register syncs with the system clipboard per `vim.useSystemClipboard` (V2).
5. Vim's selection moves to `SelectionSet` (visual block is several selections), so multi-cursor and Vim share the drawing.

## 14. Commands, keys and settings

### 14.1 Default commands and keys (macOS, following VS Code)

| Step | Command | Key |
|---|---|---|
| 4.1 | `undo` / `redo` | ⌘Z / ⇧⌘Z |
| 4.1 | `cursorUndo` / `cursorRedo` | ⌘U / — |
| 4.1 | `editor.action.selectAll` | ⌘A |
| 4.1 | `cursorLeft/Right/Up/Down` (+`Select`) | ← → ↑ ↓ (⇧), ⌃B ⌃F ⌃P ⌃N |
| 4.1 | `cursorWordLeft` / `cursorWordEndRight` (+`Select`) | ⌥← / ⌥→ |
| 4.1 | `cursorHome` / `cursorEnd` (+`Select`) | ⌘← / ⌘→, Home / End |
| 4.1 | `cursorLineStart` / `cursorLineEnd` | ⌃A / ⌃E |
| 4.1 | `cursorTop` / `cursorBottom` (+`Select`) | ⌘↑ / ⌘↓ |
| 4.1 | `cursorPageUp` / `cursorPageDown` (+`Select`) | PageUp / PageDown |
| 4.1 | `deleteLeft` / `deleteRight` | ⌫ / ⌦, ⌃H / ⌃D |
| 4.1 | `deleteWordLeft` / `deleteWordRight` | ⌥⌫ / ⌥⌦ |
| 4.1 | `deleteAllLeft` / `deleteAllRight` | ⌘⌫ / ⌘⌦, ⌃K |
| 4.1 | `tab` / `outdent` | Tab / ⇧Tab |
| 4.1 | `lineBreakInsert` | ⌃O |
| 4.1 | `editor.action.clipboardCutAction` / `clipboardPasteAction` | ⌘X / ⌘V |
| 4.1 | `workbench.action.files.save` / `saveAll` / `saveWithoutFormatting` | ⌘S / ⌥⌘S / ⌘K S |
| 4.1 | `workbench.action.files.revert` | — |
| 4.2 | `editor.action.commentLine` / `addCommentLine` / `removeCommentLine` | ⌘/ / ⌘K ⌘C / ⌘K ⌘U |
| 4.2 | `editor.action.blockComment` | ⇧⌥A |
| 4.2 | `editor.action.moveLinesUpAction` / `moveLinesDownAction` | ⌥↑ / ⌥↓ |
| 4.2 | `editor.action.copyLinesUpAction` / `copyLinesDownAction` | ⇧⌥↑ / ⇧⌥↓ |
| 4.2 | `editor.action.deleteLines` | ⇧⌘K |
| 4.2 | `editor.action.insertLineAfter` / `insertLineBefore` | ⌘Enter / ⇧⌘Enter |
| 4.2 | `editor.action.indentLines` / `outdentLines` | ⌘] / ⌘[ |
| 4.2 | `editor.action.joinLines` | ⌃J |
| 4.2 | `editor.action.trimTrailingWhitespace` | ⌘K ⌘X |
| 4.2 | `editor.action.smartSelect.expand` / `shrink` | ⌃⇧⌘→ / ⌃⇧⌘← |
| 4.3 | `editor.action.addSelectionToNextFindMatch` | ⌘D |
| 4.3 | `editor.action.moveSelectionToNextFindMatch` | ⌘K ⌘D |
| 4.3 | `editor.action.addSelectionToPreviousFindMatch` / `moveSelectionToPreviousFindMatch` | — |
| 4.3 | `editor.action.selectHighlights` / `editor.action.changeAll` | ⇧⌘L / ⌘F2 |
| 4.3 | `editor.action.insertCursorAbove` / `insertCursorBelow` | ⌥⌘↑ / ⌥⌘↓ |
| 4.3 | `editor.action.insertCursorAtEndOfEachLineSelected` | ⇧⌥I |
| 4.3 | `cursorColumnSelectLeft/Right/Up/Down/PageUp/PageDown` | ⇧⌥⌘ + ← → ↑ ↓ PageUp PageDown |
| 4.3 | `removeSecondaryCursors` / `cancelSelection` | Escape |
| 4.5 | `editor.action.triggerSuggest` | ⌃Space |
| 4.5 | `acceptSelectedSuggestion` / `acceptAlternativeSelectedSuggestion` | Enter, Tab / ⇧Enter, ⇧Tab |
| 4.5 | `selectNextSuggestion` / `selectPrevSuggestion` | ↓ / ↑, ⌃N / ⌃P |
| 4.5 | `selectNextPageSuggestion` / `selectPrevPageSuggestion` | PageDown / PageUp |
| 4.5 | `hideSuggestWidget` | Escape |
| 4.5 | `toggleSuggestionDetails` | ⌃Space (when the menu is open) |
| 4.5 | `jumpToNextSnippetPlaceholder` / `jumpToPrevSnippetPlaceholder` / `leaveSnippet` | Tab / ⇧Tab / Escape |
| 4.6 | `editor.action.rename` | F2 |
| 4.6 | `acceptRenameInput` / `cancelRenameInput` | Enter / Escape |
| 4.6 | `editor.action.formatDocument` / `formatSelection` | ⇧⌥F / ⌘K ⌘F |

A command's key is only added when its step ships. A key that collides with an existing command is checked when added (for example ⌘K ⌘X and ⌘U are currently unused).

**New context keys:** `editorReadonly` (false when the file is editable; true for a file that failed to load, a binary file, the picker's preview view), `editorHasSelection`, `editorHasMultipleSelections`, `suggestWidgetVisible`, `inSnippetMode`, `hasNextTabstop`, `hasPrevTabstop`, `renameInputVisible`, `activeEditorIsDirty`.

### 14.2 New settings (following VS Code unless noted)

| Key | Default | Step |
|---|---|---|
| `editor.insertSpaces` | `true` | 4.1 |
| `editor.trimAutoWhitespace` | `true` | 4.1 |
| `editor.wordSeparators` | `` `~!@#$%^&*()-=+[{]}\|;:'",.<>/? `` | 4.1 |
| `editor.cursorBlinking` | `"blink"` (`solid` turns off blinking; other values act as `blink`) | 4.1 |
| `editor.cursorStyle` | `"line"` (`line`, `block`, `underline`) | 4.1 |
| `editor.emptySelectionClipboard` | `true` | 4.1 |
| `files.eol` | `"auto"` (only used for a file with no lines) | 4.1 |
| `files.encoding` | `"utf8"` (only used when it cannot be guessed) | 4.1 |
| `files.insertFinalNewline` / `files.trimFinalNewlines` / `files.trimTrailingWhitespace` | `false` | 4.1 |
| `files.autoSave` / `files.autoSaveDelay` | `"off"` / `1000` | 4.1 |
| `files.saveConflictResolution` | `"askUser"` | 4.1 |
| `editor.autoIndent` | `"full"` | 4.2 |
| `editor.detectIndentation` | `true` | 4.2 |
| `editor.autoClosingBrackets` / `autoClosingQuotes` | `"languageDefined"` | 4.2 |
| `editor.autoClosingOvertype` / `autoClosingDelete` | `"auto"` | 4.2 |
| `editor.autoSurround` | `"languageDefined"` | 4.2 |
| `editor.multiCursorModifier` | `"alt"` | 4.3 |
| `editor.multiCursorMergeOverlapping` | `true` | 4.3 |
| `editor.multiCursorPaste` | `"spread"` | 4.3 |
| `editor.multiCursorLimit` | `10000` | 4.3 |
| `lucerna.externalChanges.autoMerge` (Lucerna only) | `true` | 4.4 |
| `editor.quickSuggestions` | `{ "other": "on", "comments": "off", "strings": "off" }` | 4.5 |
| `editor.quickSuggestionsDelay` | `10` | 4.5 |
| `editor.suggestOnTriggerCharacters` | `true` | 4.5 |
| `editor.acceptSuggestionOnEnter` | `"on"` | 4.5 |
| `editor.acceptSuggestionOnCommitCharacter` | `true` | 4.5 |
| `editor.suggestSelection` | `"first"` (only `first` is supported) | 4.5 |
| `editor.snippetSuggestions` | `"inline"` | 4.5 |
| `editor.wordBasedSuggestions` | `"matchingDocuments"` | 4.5 |
| `editor.suggest.insertMode` | `"insert"` | 4.5 |
| `editor.suggest.filterGraceful` / `editor.suggest.matchOnWordStartOnly` | `true` / `true` | 4.5 |
| `files.refactoring.autoSave` | `true` | 4.6 |
| `editor.formatOnSave` | `false` | 4.6 |
| `editor.formatOnSaveMode` | `"file"` | 4.6 |
| `editor.defaultFormatter` | `null` (a language server name) | 4.6 |

`editor.quickSuggestions.other` defaults to `"on"` instead of `"offWhenInlineCompletions"`, and `editor.wordBasedSuggestions` defaults to `"matchingDocuments"` instead of VS Code `main`'s `"offWithInlineSuggestions"`: Lucerna has no inline completion, so these value pairs give the same behavior; the values are written directly for clarity.

## 15. Performance

Targets (MacBook Air M2, release build):
- Typing one key in a 10,000-line file with highlighting: from receiving the key to the new frame being sent ≤ 8 ms at p95. Edits are O(log n); no copying of the whole text on the main thread.
- A 1 MB single-line file, or a file larger than `lucerna.largeFile.*`: still editable (no highlighting, no LSP, as in Phase 3); typing ≤ 16 ms at p95.
- 1,000 cursors typing one character ≤ 16 ms.
- Undoing a transaction of 10,000 edits (for example ⇧⌘L then typing) ≤ 50 ms.
- Synchronous parse after an edit uses the existing `SYNC_PARSE_BUDGET`; measure on a 10,000-line Rust file, and adjust the value if it misses the targets above.
- Filtering 5,000 completion items ≤ 5 ms on the main thread; more than that is filtered in the background.
- New benches in `lucerna-text` and `lucerna-editor` (`cargo bench`, only using libraries with a suitable license), run by hand before shipping each step.

## 16. Error handling

- File write fails: the old file stays intact (atomic), the buffer stays dirty, a status bar message with the reason; `Unencodable` points to the position of the first character.
- The server returns an error or times out on completion / format / rename: log it in the server log, a one-line message (format, rename) or silence (auto-triggered completion).
- A `WorkspaceEdit` with file operations or a version mismatch: apply nothing, show a message.
- A three-way merge that meets a binary file or a decoding error: treated as a conflict, the bar states the reason.
- A panic in an edit command: caught by the existing `catch_panic`; the open transaction is closed, the buffer keeps its pre-command state (edits are only applied after the `EditPlan` is built).

## 17. Delivery steps

Each step has its own plan, tests and review, and ships independently (in dependency order):

| Step | Content | Depends on |
|---|---|---|
| **4.1 Edit and save core** | `lucerna-text` (edit, Patch, History, EOL normalization, atomic write); `lucerna-editor` (SelectionSet, movement, typing, delete, Enter keeping indentation, Tab, clipboard, `UndoStops`); `CodeView` receiving text + IME + blinking cursor; undo/redo/cursorUndo; save, save all, revert, autosave, the `files.*` operations on save; dirty dot, close / quit prompt; LSP incremental `didChange` + `didSave`; syntax, folds, diagnostics, search highlights shifted by `Patch`; reload as an undo step; **minimal data-loss protection**: no reload when dirty, a warning dot on the tab, an Overwrite / Reload / Cancel prompt on save; minimal Vim Insert. | — |
| **4.2 Smart typing** | Auto-close, overtype, surround; `autoIndent` (`brackets`, `indents.scm`); `detectIndentation` + status bar item; line / block comment; line commands; smart select; reading more of `config.toml`. | 4.1 |
| **4.3 Multi-cursor** | Selection-creating commands (§8), ⌥-click, ⇧⌥-drag, column select, `spread` paste, drawing multiple cursors. | 4.1 |
| **4.4 Conflicts with the agent** | Three-way merge, conflict bar, Reload / Keep Mine, a dirty file being deleted, `files.saveConflictResolution`. | 4.1 |
| **4.5 Completion and snippets** | Parser + snippet session, extension snippets, completion / resolve requests, Zed-style menu, VS Code scoring, word-based. | 4.3 (same-number placeholders use multi-cursor) |
| **4.6 Rename and format** | F2 + rename box, applying multi-file `WorkspaceEdit`, `files.refactoring.autoSave`; format document / selection / on save, timeout, applying as a diff. | 4.1 |

4.2, 4.3, 4.4, 4.6 can be done in parallel after 4.1 if files are split carefully (4.2 and 4.3 both touch `lucerna-editor`, so do them sequentially or split modules clearly).

After each step: update the README and the roadmap table (§1.3 of the Phase 1 spec), and fix the sentence "Code view: Read-only" in the Phase 1 spec when 4.1 ships.

## 18. Testing

- **`lucerna-text`:** multi-range edits; undo / redo return the right text and selection token; nested transactions; `push_stop`, `group_until`; dirty / clean when undoing back to and redoing past the saved point; `Patch::map_offset` with bias, `compose`; EOL normalization (LF, CRLF, mixed, lone `\r`); write then read back with UTF-8, UTF-8 BOM, UTF-16LE, Shift-JIS, Latin-1; `Unencodable`; atomic write preserves permissions; symlinks and hardlinks written in place; a mid-way error leaves no temp file.
- **`lucerna-editor`:** table tests using marked notation for every command (§14.1) with one and several selections; the `should_stop_between` table against VS Code's examples (`abc def` → two steps, `abc  def` → three steps); auto-close per `autoclose_before`, `not_in`, a quote after a word; Enter between brackets; comment toggle; ⌘D wraps around, drops overlaps; selection merging (overlap, touch, same start point); three-way merge (no overlap → merges; overlap or adjacent → error; add / delete at end of file; empty file) and a property test: the merge result contains every hunk of both sides whole when they do not overlap; snippet parser (nested, escape, variables, choices); `fuzzyScore` matching the examples in VS Code's tests (source recorded).
- **`lucerna-lsp`:** fake server: incremental `didChange` with the right ranges for UTF-8 / UTF-16 / UTF-32, `FULL` still sends everything; `didSave` per `includeText`; completion + resolve; rename with `changes` and `documentChanges`, rejecting file operations; formatting.
- **`lucerna-workbench` (TestAppContext):** typing through the input handler (including marked text), undo grouped correctly, the dirty dot appears / disappears; save writes the exact bytes; the tab close and quit prompts (simulated answers); a file changed on disk while clean → reload, ⌘Z goes back; while dirty and not overlapping → merge; overlapping → conflict bar, Reload, Keep Mine, prompt on save; autosave skips a conflicting file; two views of the same file shift each other's selections; the completion menu opens, filters, accepts, closes; multi-file rename with `files.refactoring.autoSave`; format cancelled when typing during the wait; format timeout on save still saves.
- **Manual checks before shipping each step:** side by side with Zed (completion menu, rename box, tab dot, conflict bar: spacing, height, background color, border, hover, empty state) and with VS Code (keys, undo grouping). Agent scenario: run Claude Code editing a file while typing in that file, both the overlapping and non-overlapping cases.

## 19. Questions for the user to approve

The points below already have a choice in the spec, but they depart from VS Code or Zed, or are scope trade-offs:

| # | Question | Spec's current choice | Alternative |
|---|---|---|---|
| Q1 | Handling a dirty file edited by the agent | Auto-merge when not overlapping + a bar when overlapping (D5) | Only a dot on the tab and ask on save, like Zed (less work, step 4.4 can be dropped) |
| Q2 | Undo grouping | VS Code's rules (D3) | Group by 300 ms like Zed (simpler, but `abc def` typed fast becomes one step) |
| Q3 | Vim in Phase 4 | Minimal Insert (D10) | Nothing: people with Vim on must turn Vim off to edit until V2 |
| Q4 | Hot exit, keeping undo after close, new file ⌘N, Save As | Out of scope | Add a step 4.7 |
| Q5 | Format when slow | A hard 5 s timeout (D9) | "Saving…" + Skip as in VS Code |
| Q6 | Icons in the completion menu | No icons, labels colored by kind (following Zed) | Icons by kind as in VS Code |

## 20. Risks

- **IME with multiple cursors and with GPUI at the pinned rev:** `TextInput` already runs IME for one box; a multi-line editor needs a correct `bounds_for_range` under word wrap. Do it early in 4.1 and test Telex, VNI, Japanese.
- **EOL normalization changes offsets:** search in file, hover, diagnostics, LSP positions are currently computed on a rope containing `\r`. Every use of `len_bytes`, `point_to_offset` must be reviewed in 4.1; project-wide search reads files from disk, so it is unaffected.
- **A wrong three-way merge silently loses data:** merge only when not overlapping and not adjacent, the merge is an undo step, there is a property test; there is a setting to turn it off.
- **A late or coalesced watcher** makes Lucerna take a file it wrote itself for the agent's (or the reverse): always compare content hashes, not just mtime.
- **Step 4.1 is large:** if the plan shows more than three weeks, split into 4.1a (edit core, typing, undo, save, data-loss protection) and 4.1b (autosave, quit prompt, Vim Insert, cursorUndo).

## 21. Changes to older specs (on shipping)

- `2026-11-08-lucerna-phase1-design.md`: §2.1 "Code view: Read-only" → point to this spec; row 4 of the §1.3 table records a link to the spec.
- `2026-11-10-lucerna-phase3-lsp-design.md`: §1 "Lucerna is still read-only" → add a Phase 4 note.
- `2026-11-10-vim-navigation-design.md`: note that minimal Insert exists from Phase 4, and V2 builds on the contract in §13.
- Extensions page: remove the "(Phase 4)" text after the snippet count when 4.5 ships.
