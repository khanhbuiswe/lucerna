# Phase 4.3 Multi-cursor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Multi-cursor like VS Code: ⌘D and related commands, ⌥⌘↑/↓, ⇧⌥I, column selection, ⌥-click, ⌥-drag, ⇧⌥-drag, two-step Escape, `spread`/`full` paste, selection counts in the status bar. Remove `block_selection`. When Vim is on, multi-cursor commands put Vim into Insert.

**Architecture:**
- `lucerna-editor` (pure) gains `find` (buffer search shared by the Find bar and ⌘D, whole word by `wordSeparators`) and `multicursor` (functions `SelectionSet → SelectionSet`). The clipboard is fixed to follow VS Code.
- `lucerna-workbench`: commands, keys, mouse, session state (⌘D, the ⌥⌘↑/↓ stack, column selection) in `CodeView`; Vim listens for the new event `CodeViewEvent::MultiCursor` to enter Insert.
- Column selection and `<C-v>` are real multiple selections; the block-drawing path is deleted.

**Tech Stack:** Rust 2024, `lucerna-text`, `lucerna-editor` (+ `regex`, already in the workspace), `lucerna-vim`, gpui (workbench only).

**Spec:** `docs/superpowers/specs/2026-11-17-multi-cursor-design.md` (read all of it). Parent spec: `docs/superpowers/specs/2026-11-11-lucerna-phase4-editing-design.md` §8, §14. Research: `docs/research/2026-11-11-editing.md` §3.

## Global Constraints

- **App text is English only**, verbatim: `The number of cursors has been limited to N.` (N is a number), `N selections`, `N selections (M characters selected)`, `Ln x, Col y (M selected)`. Vietnamese appears only in test data.
- **Crates:** only `lucerna-workbench` uses gpui. `lucerna-editor` adds `regex.workspace = true` and no other dependency.
- **License:** do not copy Zed code (GPL). Code adapted from VS Code carries `//! Adapted from microsoft/vscode <path> (MIT)` at the top of the module, like `clipboard.rs`: `multicursor.rs` ← `src/vs/editor/contrib/multicursor/browser/multicursor.ts`, `src/vs/editor/common/cursor/cursorMoveCommands.ts`, `src/vs/editor/common/cursor/cursorColumnSelection.ts`; `find.rs` ← `src/vs/editor/common/model/textModelSearch.ts` (`isValidMatch`).
- **Positions:** UTF-8 byte offsets, LF-normalised like `SelectionSet`; never split a character.
- **New settings (defaults):** `editor.multiCursorModifier: "alt"` (`"alt"` | `"ctrlCmd"`), `editor.multiCursorMergeOverlapping: true`, `editor.multiCursorPaste: "spread"` (`"spread"` | `"full"`), `editor.multiCursorLimit: 10000`. Do **not** add `editor.selectedTextMatchMode`.
- **Keys:** exactly the §5.2 table of the spec, added to `crates/lucerna-settings/assets/default-keybindings.json`.
- **Build:**
  - `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"`.
  - Run `scripts/prune-target.sh` before **every** `cargo test` or build.
  - Do not set `CARGO_TARGET_DIR` or `--target-dir`. If the cargo lock is held, wait.
  - Keep one profile: delete `target/release` before testing, unless `pgrep -x cargo` shows another cargo running.
- **Git (shared folder):** `git status` before each commit; if a merge is in progress or there are unfamiliar staged files, stop and report BLOCKED. Commit by path: `git commit -m "…" -- <paths>`. No rebase / stash / amend. Trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo deny check` pass at the end of every task that touches code.

## Review Focus

1. **⌘D on Vietnamese text and multi-byte text.** The word `chào` appears three times (once inside `chàobạn`): ⌘D ×3 selects exactly the two standalone words, then wraps around without adding more, with no panic at character boundaries. Test: `Task 2: add_next_vietnamese_whole_word`.
2. **⌘D when an agent overwrites the file between two presses.** The reload remaps the selection; the next ⌘D searches the new snapshot and does not use the old text at the old offset. Test: `Task 6: cmd_d_after_external_reload`.
3. **⇧⌘L on a file > 1 MB with tens of thousands of results.** Capped at `multiCursorLimit`, shows a message, and the UI does not hang (runs in the background). Test: `Task 6: select_highlights_limit_large_file`.
4. **Column selection across lines with tabs and empty lines.** Columns are counted by display width; a short line gets an empty cursor at its end; typing then one undo step restores the original text. Tests: `Task 4: column_select_tabs_and_short_lines`, `Task 7: shift_alt_drag_then_type_and_undo`.
5. **Vim on, ⌘D then type then `u`.** One undo step, back to Normal with one cursor. Test: `Task 8: vim_cmd_d_insert_escape_undo`.

---

## Decisions settled in the plan (spec for the builder)

- **The selection merge flag** lives in `SelectionSet` (`merge_overlapping: bool`, default `true`), so that `remap` and every later merge follow the setting without passing a parameter through every function. `CodeView` sets the flag each time it installs a new set.
- **`find_next` is for literal text only** (⌘D is always literal). It scans in 256 KiB windows starting at a line start, overlapping by `4 × the pattern's character count` bytes, so it does not copy the whole buffer. `find_all` (Find bar, ⇧⌘L) still copies once as it does today.
- **The background threshold of ⇧⌘L** = `BACKGROUND_SEARCH_BYTES` (1 MiB), same as the Find bar.
- **Column selection** follows buffer lines and display columns that account for tabs (`tab_size`), even when word wrap is on. ⌥⌘↑/↓ use the existing `Layout::vertical`, so they follow display lines.
- **The character count in the status bar** is cached by `selection_generation` in `CodeView`.

---

### Task 1: Shared buffer find (`lucerna-editor::find`)

**Files:**
- Create: `crates/lucerna-editor/src/find.rs`
- Modify: `crates/lucerna-editor/src/lib.rs`, `crates/lucerna-editor/Cargo.toml`, `crates/lucerna-workbench/src/find_widget.rs:30-72`, `crates/lucerna-workbench/src/lib.rs:101`, `crates/lucerna-workbench/src/vim.rs` (import)
- Test: unit tests in `find.rs`; `crates/lucerna-workbench/tests/find_widget.rs` must pass

**Interfaces:**
- Produces:
  ```rust
  pub struct FindQuery<'a> { pub pattern: &'a str, pub is_regex: bool, pub case_sensitive: bool, pub whole_word: bool, pub word_separators: &'a str }
  pub fn build_regex(pattern: &str, is_regex: bool, case_sensitive: bool) -> Result<regex::Regex, String>
  pub fn find_all(snap: &BufferSnapshot, q: &FindQuery) -> Result<Vec<Range<usize>>, String>
  pub fn find_next(snap: &BufferSnapshot, q: &FindQuery, from: usize, backwards: bool) -> Option<Range<usize>>
  pub fn is_whole_word(snap: &BufferSnapshot, range: Range<usize>, separators: &str) -> bool
  ```
  `lucerna_workbench::find_in_snapshot(snapshot, pattern, is_regex, case, whole_word)` keeps its old signature and calls `find_all` with `DEFAULT_WORD_SEPARATORS` (for Vim and the old tests).

- [ ] **Step 1: Write the failing tests in `find.rs`**
  - `whole_word_uses_separators`: text `$foo foo-bar $foo`, the separators of `test_config()` **without** `$` and `-` → searching `$foo` as a whole word gives 2 results `[0..4, 13..17]`; searching `foo-bar` as a whole word gives `[5..12]`.
  - `whole_word_default_separators`: default separators, `foo` as a whole word in `foo foobar (foo)` gives `[0..3, 12..15]`.
  - `find_next_wraps_and_backwards`: `a x a x a`, pattern `a`, `from=3` forward → `4..5`; `from=9` forward → wraps to `0..1`; backward from `4` → `0..1`.
  - `find_next_across_window_boundary`: a 600 KiB text of all `x`, with `needle` inserted at offset 262_140 (straddling the 256 KiB boundary) → `find_next(.., 0, false) == Some(262_140..262_146)`.
  - `find_next_case_folding_multibyte`: `CHÀO chào`, case-insensitive, from 0 → `0..5`, from 5 → `6..11`.
  - `invalid_regex_errors`: `find_all` with `(` as a regex → `Err`.

- [ ] **Step 2: Run, confirm they fail**
  Run: `cargo test -p lucerna-editor find::`
  Expected: FAIL (the module does not exist yet).

- [ ] **Step 3: Implement**
  - `build_regex`: `regex::escape` when literal, `case_insensitive(!case_sensitive)`, `multi_line(true)` (as now, minus the `\b` part).
  - `is_whole_word` follows VS Code's `isValidMatch`: the left side is valid when `start == 0`, or the character before `start` is not `Regular` (`words::classify`), or the first character of the result is not `Regular`; symmetric for the character after `end` and the last character of the result.
  - `find_all`: like the old `find_in_snapshot`, filter out empty matches, filter by `is_whole_word` when `whole_word`.
  - `find_next`: `debug_assert!(!q.is_regex)`; windows as in the "Decisions" section; skip empty / non-whole-word results; after one full loop with nothing found → `None`.
  - The Find bar calls `find_all` with `word_separators` taken from the view's `edit_config`.

- [ ] **Step 4: Run the tests**
  Run: `cargo test -p lucerna-editor find:: && cargo test -p lucerna-workbench --test find_widget --test vim --test vim_editing`
  Expected: PASS. If an old Find test fails because of `\b` → fix the expectation, and write the reason in the commit message.

- [ ] **Step 5: Commit**
  `git commit -m "refactor(find): shared buffer find in lucerna-editor, whole word by word separators" -- <the files above>`

---

### Task 2: ⌘D, ⌘K ⌘D, ⇧⌘L (`multicursor`, the find part)

**Files:**
- Create: `crates/lucerna-editor/src/multicursor.rs`
- Modify: `crates/lucerna-editor/src/lib.rs`, `crates/lucerna-editor/src/selection.rs` (merge flag)
- Test: unit tests in `multicursor.rs`, `selection.rs`

**Interfaces:**
- Consumes: Task 1 `FindQuery`, `find_next`, `find_all`.
- Produces:
  ```rust
  #[derive(Clone, Copy, Debug, PartialEq)] pub struct MatchOptions { pub case_sensitive: bool, pub whole_word: bool }
  #[derive(Clone, Debug, PartialEq)] pub struct MatchSession { pub text: String, pub options: MatchOptions }
  pub struct Step { pub set: SelectionSet, pub session: Option<MatchSession>, pub seeded_word: Option<String>, pub limited: bool }
  pub fn add_match(snap: &BufferSnapshot, set: &SelectionSet, session: Option<&MatchSession>, find: MatchOptions, separators: &str, backwards: bool, limit: usize) -> Option<Step>
  pub fn move_match(snap: &BufferSnapshot, set: &SelectionSet, session: Option<&MatchSession>, find: MatchOptions, separators: &str, backwards: bool) -> Option<Step>
  pub fn select_all_matches(snap: &BufferSnapshot, set: &SelectionSet, session: Option<&MatchSession>, find: MatchOptions, separators: &str, limit: usize) -> Option<Step>
  // selection.rs
  impl SelectionSet { pub fn set_merge_overlapping(&mut self, merge: bool); pub fn merge_overlapping(&self) -> bool; pub fn newest(&self) -> &Selection }
  ```
  `find` is the Find bar's current options (default `false/false` when never opened). `seeded_word` is the word just opened from an empty cursor (the workbench fills it into the Find bar if it is open).

- [ ] **Step 1: Failing tests** (use `marked`/`render`)
  - `add_first_expands_words`: `«ab»c x|y |z` → `«ab»c «xy» «z»`, `seeded_word == None` (several words), `session == None`.
  - `add_first_single_cursor_seeds`: `fo|o bar foo` → `«foo» bar foo`, `seeded_word == Some("foo")`, `session == Some({text: "foo", options: {case_sensitive: true, whole_word: true}})`.
  - `add_next_wraps_skips_overlap`: `foo «foo»* foo` (primary in the middle) → adds the third `foo`; again → wraps around and adds the first; again → `None`.
  - `add_from_selection_uses_find_options`: `«Foo» foo FOO`, `find={false,false}` → adds `foo` (position 4); `find={true,false}` → `None`.
  - `add_mismatched_text_noop`: `«ab» «cd»` → `None`.
  - `add_previous`: `foo foo «foo»` backwards → adds position 4.
  - `move_replaces_newest`: `«foo» «foo» foo` (newest = the second) → `«foo» foo «foo»`.
  - `select_all_keeps_primary`: `x foo *«foo» foo` → three selections, the primary stays in the middle.
  - `add_next_vietnamese_whole_word`: `chào| chàobạn chào` ×3 calls → selections at `0..5` and `17..22`, the third call `None`.
  - `limit_caps`: `select_all_matches` with `limit=2` over 5 results → 2 selections, `limited == true`.
  - `merge_flag_off_keeps_touching`: `set_merge_overlapping(false)` then `remap` a patch that makes two selections overlap → still 2.

- [ ] **Step 2: Run, confirm they fail** — `cargo test -p lucerna-editor multicursor:: selection::` → FAIL.

- [ ] **Step 3: Implement** per spec §5.1. `newest()` = the selection with the largest id. `remap` and the places that call `from_selections(…, true)` in `lucerna-editor` use `self.merge_overlapping`; `from_selections` keeps its parameter and the new set inherits that value. A new session when starting from self-selected text: `{text, options: find}`.

- [ ] **Step 4: Run the tests** → PASS.
- [ ] **Step 5: Commit** `feat(editor): add/move/select-all find match selections`.

---

### Task 3: ⌥⌘↑/↓, ⇧⌥I, Escape, ⌥-click (`multicursor`, the cursor part)

**Files:** Modify `crates/lucerna-editor/src/multicursor.rs`. Tests in the same file.

**Interfaces:**
- Consumes: `Layout` (`motion.rs`), `SelectionSet::newest`.
- Produces:
  ```rust
  pub fn insert_cursors_vertical(snap: &BufferSnapshot, set: &SelectionSet, up: bool, layout: &dyn Layout, limit: usize) -> (SelectionSet, Vec<u64>, bool /*limited*/)
  pub fn remove_ids(set: &SelectionSet, ids: &[u64]) -> SelectionSet
  pub fn cursors_at_line_ends(snap: &BufferSnapshot, set: &SelectionSet, limit: usize) -> Option<(SelectionSet, bool)>
  pub fn remove_secondary(set: &SelectionSet) -> Option<SelectionSet>
  pub fn cancel_selection(set: &SelectionSet) -> Option<SelectionSet>
  pub fn toggle_cursor_at(set: &SelectionSet, offset: usize, limit: usize) -> SelectionSet
  pub fn add_selection(set: &SelectionSet, anchor: usize, head: usize, limit: usize) -> (SelectionSet, u64)
  ```
  `Vec<u64>` holds the ids of the cursors just added (one layer of the stack). `add_selection` is used for ⌥-drag and ⌥ double/triple-click; it returns the id so the drag updates exactly that selection.

- [ ] **Step 1: Failing tests**
  - `insert_below_every_selection_keeps_goal`: `ab|cdef\nx\nabcdef\n` (2 times down) → cursors at (1,1) then (2,2) with the goal keeping column 2; every initial selection gets one added (the case of 2 initial cursors on lines 0 and 3 → 4 cursors).
  - `insert_then_remove_layer`: `remove_ids` with the ids of the layer just added returns the initial set.
  - `insert_at_edge_noop`: the cursor on the last line, `up=false` → the set is unchanged, `ids` is empty.
  - `line_ends_vscode`: `«ab\ncd\nef»gh` → cursors at the end of `ab`, the end of `cd`, and at the end of the selection (`ef|gh`); `«ab\n»cd` (stopping at column 0) → only one cursor at the end of `ab`; empty selections are dropped; all empty → `None`.
  - `escape_two_steps`: `«a» «b»*` → `remove_secondary` leaves `«b»`; `cancel_selection` → `b|`; a single empty cursor → both `None`.
  - `toggle_adds_and_removes`: adding a new cursor makes it primary; clicking on a secondary cursor removes it; clicking on the only selection → unchanged.

- [ ] **Step 2–4:** run and see them fail → implement → run and see them pass (`cargo test -p lucerna-editor multicursor::`).
- [ ] **Step 5: Commit** `feat(editor): vertical cursors, cursors at line ends, escape and toggle`.

---

### Task 4: Column selection (`multicursor::column_select`)

**Files:** Modify `crates/lucerna-editor/src/multicursor.rs`. Tests in the same file.

**Interfaces:**
- Produces:
  ```rust
  #[derive(Clone, Copy, Debug, PartialEq)] pub struct ColumnPos { pub row: u32, pub col: u32 /* display column, tabs expanded */ }
  pub fn visual_column(snap: &BufferSnapshot, offset: usize, tab_size: u32) -> u32
  pub fn column_select(snap: &BufferSnapshot, from: ColumnPos, to: ColumnPos, tab_size: u32, limit: usize) -> (SelectionSet, bool)
  pub enum ColumnMove { Left, Right, Up, Down, PageUp, PageDown }
  pub fn column_move(snap: &BufferSnapshot, from: ColumnPos, to: ColumnPos, mv: ColumnMove, page_rows: u32) -> ColumnPos /* the new `to` */
  ```
- [ ] **Step 1: Failing tests**
  - `column_select_tabs_and_short_lines`: `"\tab\nx\n    abcd"`, tab 4, from (0, 4) to (2, 6) → line 0 `«ab»`, line 1 an empty cursor after `x`, line 2 `«ab»` (columns 4..6); the primary is line 2.
  - `column_select_reversed`: from (2,6) to (0,4) → every selection reversed, the primary is line 0.
  - `column_select_unicode`: `"chào\nabcd"` columns 1..3 → `«hà»`, `«bc»`.
  - `column_move_right_up`: `column_move` Right increases `to.col` by 1; Up decreases `to.row`, stopping at 0.
- [ ] **Step 2–4:** fail → implement → pass.
- [ ] **Step 5: Commit** `feat(editor): column selection`.

---

### Task 5: Clipboard like VS Code and the paste setting

**Files:** Modify `crates/lucerna-editor/src/clipboard.rs`, `crates/lucerna-editor/src/plan.rs` (`EditConfig`), `crates/lucerna-editor/src/test_util.rs` (`test_config`), `crates/lucerna-workbench/src/code_view/transact.rs:60-100` (read the setting), `crates/lucerna-settings/assets/default-settings.json`.

**Interfaces:**
- Produces: `setting_enum!(MultiCursorPaste { Spread = "spread", Full = "full" })`; `EditConfig.multi_cursor_paste: MultiCursorPaste`. The new setting keys for all four settings are added to `default-settings.json` in this task (Tasks 6 and 7 only read them).

- [ ] **Step 1: Failing tests**
  - `copy_mixed_empty_gives_empty_piece`: `«ab» c|d` → `pieces == ["ab", ""]`, `whole_line == false`.
  - `copy_all_empty_whole_lines` (keeps the old behavior) passes.
  - `cut_mixed_empty_deletes_only_ranges`: `«ab» c|d` → the text ` cd`.
  - `paste_external_lines_spread`: clipboard `"x\ny"` with no `pieces`, two cursors → each cursor gets one line.
  - `paste_full`: `multi_cursor_paste = Full`, the clipboard has 2 `pieces`, 2 cursors → each place gets the whole `text`.
  - `paste_lines_count_mismatch_full_text`: 3 lines, 2 cursors → each place gets the whole text.
- [ ] **Step 2–4:** fail → implement (`sources`: take whole lines only when every selection is empty) → pass (`cargo test -p lucerna-editor clipboard::`, and `cargo test -p lucerna-workbench --test editing`).
- [ ] **Step 5: Commit** `fix(editor): copy mixed selections as VS Code; multiCursorPaste`.

---

### Task 6: Remove `block_selection`; commands and keys in the workbench

**Files:**
- Create: `crates/lucerna-workbench/src/code_view/multi_cursor.rs` (the `CodeView` methods for multi-cursor + session state)
- Modify: `crates/lucerna-workbench/src/code_view/mod.rs` (remove the field `block_selection`, `set_block_selection`, `has_block_selection`, the block branch in `copy_selection`, the remap at ~652), `code_view/transact.rs:332-345`, `code_view/element.rs` (`block_selection` in the frame, `visible_selections`), `code_view/element/paint.rs:63`, `crates/lucerna-workbench/src/vim.rs` (`sync_view` ~1623, where Vim is switched off ~604), `crates/lucerna-workbench/src/editing.rs` (command registration), `crates/lucerna-settings/assets/default-keybindings.json`
- Test: `crates/lucerna-workbench/tests/multi_cursor.rs` (new, following the helpers of `tests/editing.rs`); `tests/vim.rs`, `tests/vim_editing.rs` must pass without changing expectations

**Interfaces:**
- Consumes: Task 2–5.
- Produces (on `CodeView`):
  ```rust
  pub fn add_next_match(&mut self, backwards: bool, cx) ; pub fn move_next_match(&mut self, backwards: bool, cx)
  pub fn select_all_matches(&mut self, cx)          // background when > BACKGROUND_SEARCH_BYTES
  pub fn insert_cursor_vertical(&mut self, up: bool, cx)
  pub fn cursors_at_line_ends(&mut self, cx)
  pub fn column_select_move(&mut self, mv: ColumnMove, cx)
  pub fn remove_secondary_cursors(&mut self, cx) ; pub fn cancel_selection(&mut self, cx)
  pub fn set_column_selection(&mut self, from: ColumnPos, to: ColumnPos, cx)   // Vim <C-v> and ⇧⌥-drag
  pub fn multi_cursor_limit(&self, cx: &App) -> usize
  ```
  State in `CodeView`: `match_session: Option<MatchSession>`, `cursor_layers: Vec<(bool /*up*/, Vec<u64>)>`, `column_select: Option<(ColumnPos, ColumnPos)>`. All three are cleared in `set_selections_inner` when the selection changes other than by their own commands (the flag `multi_cursor_running` is on while a command runs).
  Each command that creates selections: goes through `set_selections(set, reveal_newest, cx)` (the `Move` path, so ⌘U can undo it), then `cx.emit(CodeViewEvent::MultiCursor)` (a new variant, used in Task 8). If capped → `set_goto_message`/status message `The number of cursors has been limited to N.`.
  `seeded_word` → if `find_widget()` exists, set the text of the find input.

- [ ] **Step 1: Failing tests** (`tests/multi_cursor.rs`)
  - `cmd_d_three_times_then_type`: `foo bar foo baz foo`, the cursor in the first `foo`, `keys("cmd-d cmd-d cmd-d")`, type `x` → `x bar x baz x`; ⌘Z once → the old text.
  - `cmd_k_cmd_d_skips`: ⌘D ⌘D ⌘K ⌘D type `x` → `x bar foo baz x`.
  - `cmd_u_undoes_cmd_d`: ⌘D ⌘D ⌘U → one selection left.
  - `shift_cmd_l_and_cmd_f2`: ⇧⌘L selects every `foo`; ⌘F2 likewise.
  - `alt_cmd_down_then_up_removes`: ⌥⌘↓ ⌥⌘↓ ⌥⌘↑ → 2 cursors.
  - `escape_two_steps`: ⌘D ⌘D, Escape → 1 non-empty selection; Escape → a cursor; the context `editorHasMultipleSelections` is correct at each step.
  - `escape_closes_find_first`: the Find bar open + 2 selections, Escape closes Find, still 2 selections.
  - `cmd_d_explorer_still_duplicates`: focus on the Explorer, ⌘D runs `filesExplorer.duplicate` (check the chosen command with `keymap` + context, like the existing explorer test).
  - `cmd_d_seeds_open_find`: open ⌘F, ⌘D on `foo` → the find input is `foo`.
  - `cmd_d_after_external_reload`: ⌘D, overwrite the file on disk (insert a first line), wait for the reload, ⌘D → the new selection is exactly the text `foo` on the new snapshot.
  - `select_highlights_limit_large_file`: setting `editor.multiCursorLimit: 100`, a 2 MiB file of all `foo `, ⇧⌘L → `run_until_parked` → exactly 100 selections, the status message has `limited to 100`.
  - `alt_shift_cmd_arrows_column`: ⇧⌥⌘→ ×2, ⇧⌥⌘↓ → 2 selections of 2 characters.
  - `vim_block_yank_unchanged`: the existing `<C-v>` + `y` test in `tests/vim.rs` passes intact.
- [ ] **Step 2: Run, confirm they fail** — `cargo test -p lucerna-workbench --test multi_cursor` → FAIL.
- [ ] **Step 3: Implement**: commands in `editing.rs` with VS Code ids (spec §5.2; `addSelectionToPreviousFindMatch`/`moveSelectionToPreviousFindMatch` have the palette titles `Add Selection to Previous Find Match` / `Move Last Selection to Previous Find Match`; the commands ⌘D, ⇧⌘L, ⌥⌘↑/↓, ⇧⌥I also have palette titles like VS Code: `Add Selection to Next Find Match`, `Move Last Selection to Next Find Match`, `Select All Occurrences of Find Match`, `Change All Occurrences`, `Add Cursor Above`, `Add Cursor Below`, `Add Cursors to Line Ends`). Keys follow the spec table, the `when` of ⌘D is `editorFocus`. Vim `<C-v>` uses `set_column_selection`; where Vim is switched off (~604), merge only when Vim is in `Mode::Visual`, `Mode::VisualLine` or `Mode::VisualBlock`.
- [ ] **Step 4: Run** `--test multi_cursor --test editing --test vim --test vim_editing --test find_widget` → PASS.
- [ ] **Step 5: Commit** `feat(workbench): multi-cursor commands and keys; selections replace block_selection`.

---

### Task 7: Mouse

**Files:** Modify `crates/lucerna-workbench/src/code_view/element.rs:1169-1235` (`mouse_down`, `mouse_drag`, `Drag`), `code_view/mod.rs` (`Drag` gets a variant). Test: `tests/multi_cursor.rs`.

**Interfaces:**
- Consumes: `toggle_cursor_at`, `add_selection`, `set_column_selection`, setting `editor.multiCursorModifier`.
- Produces: `enum Drag { Text, AddSelection { id: u64, anchor: usize }, Column { from: ColumnPos } }` (keep the other old variants).

- [ ] **Step 1: Failing tests** (use `cx.simulate_mouse_down/up/move` with `Modifiers`, like `tests/explorer_git.rs`)
  - `alt_click_adds_then_removes`: ⌥-click two places → 3 cursors; ⌥-click the second place again → 2.
  - `alt_drag_adds_selection`: the existing selection stays, and a region following the mouse is added.
  - `shift_alt_drag_then_type_and_undo`: ⇧⌥-drag across 3 lines (one short, with a tab) then type `z` → every line has `z`; ⌘Z once restores the original text.
  - `ctrl_cmd_modifier_swaps`: setting `"ctrlCmd"`: ⌘-click adds a cursor; ⌥-click (without ⌘) runs go to definition (check like the existing link test: the definition request is sent / `link_mouse_down` receives the point).
  - `cmd_alt_click_definition_to_side_both_modes`: ⌘⌥-click adds no cursor in either mode.
  - `alt_double_click_adds_word`: 1 cursor, ⌥ double-click on `bar` → adds the selection `«bar»`, the old cursor stays.
  - `plain_click_collapses`: 3 cursors, a plain click → 1.
- [ ] **Step 2–4:** fail → implement → pass.
- [ ] **Step 5: Commit** `feat(workbench): alt-click, alt-drag and shift-alt-drag cursors`.

---

### Task 8: Vim enters Insert after a multi-cursor command

**Files:** Modify `crates/lucerna-vim/src/state.rs` (add a function), `crates/lucerna-workbench/src/vim.rs` (listen for `CodeViewEvent::MultiCursor` at ~668; the `selection_generation != generation` branch at ~822 does not reset once Insert is entered). Test: `crates/lucerna-workbench/tests/vim_editing.rs`.

**Interfaces:**
- Consumes: `CodeViewEvent::MultiCursor` (emitted by Tasks 6 and 7).
- Produces: `impl Vim { pub fn enter_insert_here(&mut self, cursor: Point) }` (Insert mode, no count, no `insert_lines`, anchor = cursor); `VimLayer::start_multi_cursor_insert(&mut self, view: &Entity<CodeView>, cx)` — like `start_insert` but it does **not** reset the cursor (keeps the `SelectionSet`), opens an undo step like a normal Insert session, the bar overlay at the primary head, `set_typed_events(true)`.

- [ ] **Step 1: Failing tests**
  - `vim_cmd_d_insert_escape_undo`: Vim on, `foo bar foo`, Normal at `f`; ⌘D ⌘D → mode `Insert`, status `-- INSERT --`; type `x` → `x bar x`; Escape → Normal, one selection; `u` → `foo bar foo`.
  - `vim_alt_click_enters_insert`: ⌥-click in Normal → Insert, 2 cursors, typing goes to both.
  - `vim_visual_then_cmd_d`: `v` selects `foo` then ⌘D → Insert, 2 selections (not merged by Vim).
  - `vim_ctrl_v_stays_visual_block`: `<C-v>` `j` → mode Visual Block, not Insert.
  - `vim_off_keeps_user_cursors`: Vim off while there are 3 cursors made by ⌥-click while Vim was off → switch Vim on then off, still 3 (only Visual merges).
- [ ] **Step 2–4:** fail → implement → pass (`--test vim_editing --test vim`).
- [ ] **Step 5: Commit** `feat(vim): multi-cursor commands enter Insert mode`.

---

### Task 9: Status bar

**Files:** Modify `crates/lucerna-workbench/src/documents.rs:2031-2045`, `code_view/mod.rs` (cache). Test: `crates/lucerna-workbench/tests/documents.rs`.

**Interfaces:** Produces `CodeView::selection_summary(&mut self) -> (usize /*selections*/, usize /*chars*/)` cached by `selection_generation`; `status()` uses it (change `&CodeView` to a suitable way of reading if `&mut` is needed, or cache with a `Cell`).

- [ ] **Step 1: Failing test** `status_counts_selections`:
  - 1 cursor: `Ln 1, Col 1`.
  - 1 selection `chào`: `Ln 1, Col 5 (4 selected)` (counting chars).
  - 2 empty cursors: `2 selections`.
  - 2 selections `ab`, `cde`: `2 selections (5 characters selected)`.
- [ ] **Step 2–4:** fail → implement → pass.
- [ ] **Step 5: Commit** `feat(status): selection counts as VS Code`.

---

### Task 10: Measure performance and docs

**Files:** Modify `crates/lucerna-editor/benches/typing.rs`, `docs/perf-checklist.md`, `docs/superpowers/specs/2026-11-08-lucerna-phase1-design.md` (§1.3: 4.3 done), `docs/superpowers/specs/2026-11-11-lucerna-phase4-editing-design.md` (status line; §8 remove "Aho-Corasick", remove `selectedTextMatchMode` in §8 and §14.2), `docs/research/2026-11-11-editing.md` §3 (note that `selectedTextMatchMode` is unconfirmed), `README.md` (if it has a feature list), spec 4.3 (status: implemented).

- [ ] **Step 1:** The bench `insert_char_1000_cursors` already exists. Add `select_all_matches_10k` (a 5 MiB file, 10,000 results, `limit` 10,000) and `add_match_far` (`find_next` when the only result is at the end of a 50 MiB file).
- [ ] **Step 2:** Run `cargo bench -p lucerna-editor --bench typing -- 1000_cursors select_all add_match_far` (the bench builds in release: delete `target/debug` first per the one-profile rule, delete `target/release` again afterward). Expected: `insert_char_1000_cursors` is no more than 10 % slower than the old number in `docs/perf-checklist.md`; `add_match_far` < 16 ms (if not: ⌘D runs in the background when the file is > `BACKGROUND_SEARCH_BYTES`, as the spec allows). Record the numbers in `docs/perf-checklist.md`.
- [ ] **Step 3:** Update the docs as in the Files list.
- [ ] **Step 4:** Run everything: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo deny check` → PASS.
- [ ] **Step 5: Commit** `docs: phase 4.3 multi-cursor done; perf numbers`.
