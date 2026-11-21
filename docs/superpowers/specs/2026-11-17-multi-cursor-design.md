# Phase 4.3: Multi-cursor

Date 2026-11-17. Status: implemented.

Parent spec: [2026-11-11-lucerna-phase4-editing-design.md](2026-11-11-lucerna-phase4-editing-design.md) §8, §14, §17. VS Code / Zed research: [docs/research/2026-11-11-editing.md](../../research/2026-11-11-editing.md) §3. This spec settles the scope and approach of step 4.3; where it differs from the parent spec, this spec wins.

## 1. Intent

The next step on the roadmap (§1.3 of the Phase 1 spec): V2 is done, and 4.5 (completion, snippets) needs multi-cursor for same-numbered placeholders. Users mostly read and lightly edit code written by agents; ⌘D, ⌥-click and column select are the fastest bulk-edit actions that people used to VS Code use every day.

## 2. Goals, non-goals, anti-goals

**Goals**
- Every 4.3 command in the §14.1 table of the parent spec works like VS Code, with VS Code's default macOS keys.
- ⌥-click adds / removes a cursor, ⌥-drag adds a selection range, ⇧⌥-drag does a column select.
- Escape: several selections → only the primary selection remains; one non-empty selection → a cursor.
- Paste follows `editor.multiCursorPaste`.
- Four settings: `editor.multiCursorModifier`, `editor.multiCursorMergeOverlapping`, `editor.multiCursorPaste`, `editor.multiCursorLimit` (defaults as in §14.2 of the parent spec; see §5.3 about `selectedTextMatchMode`).
- With several selections, the status bar shows it like VS Code: `N selections`, plus ` (M characters selected)` when text is selected; one selection with text shows `Ln x, Col y (M selected)`.
- `block_selection` in `CodeView` is removed. Column select (including Vim's `<C-v>`) is real multiple selections.

**Non-goals (this step)**
- Vim operators on Visual Block (`d`, `c`, `I`, `A` on a block), the Blockwise register. The user chose to make it a separate step. `<C-v>` still only selects and does `y` as it does now, only the storage and drawing change.
- `editor.columnSelection` (the toggleable column select mode), middle-mouse drag column select, `editor.action.selectAllMatches` (⌥Enter in the Find bar), the `cursorRedo` command.
- Vim running motions / operators on multiple cursors (like Zed). That reworks the core of `lucerna-vim`, folded into the later Visual Block step. This step only does approach A in §5.2 "With Vim enabled".

**Anti-goals**
- Typing gets slower when there are many cursors (target: 1000 cursors still type without stutter; see §15 of the parent spec).
- ⌘D freezes the UI thread on a large file.
- The Find bar and ⌘D match different text with the same options.
- Today's single-cursor behavior (click, drag, ⌘-click go to definition, Vim) changes when the multi-cursor modifier keys are not used.

## 3. Constraints

- Behavior, keys and settings follow VS Code; UI follows Zed (`docs/design-principles.md`). Displayed strings are English.
- Only `lucerna-workbench` uses gpui. Selection logic lives in `lucerna-editor`, tests need no window.
- Do not copy Zed's GPL code. Code adapted from VS Code (MIT) carries a `Adapted from microsoft/vscode …` line like `clipboard.rs`.
- `cargo deny check` must pass; no new dependencies (regex is already in the workspace).

## 4. Approaches

| Approach | Trade-off | Result |
|---|---|---|
| **A. Logic in `lucerna-editor::multicursor`, search by regex shared with the Find bar, remove `block_selection`** | One selection model; ⌘D matches like the Find bar (like VS Code); pure Rust tests. The place where Vim calls `set_block_selection` and the block drawing path must be changed. | **The user chose this.** |
| B. Like A but search with Aho-Corasick (like Zed, as the parent spec says) | Faster with very many results, but adds a dependency and matches differently from the Find bar. Literal regex is already fast enough. | Rejected. |
| C. Keep `block_selection`, convert it to a selection when editing begins | Fewer changes, but two parallel selection models, against contract §13.5 of the parent spec, and it makes the later Visual Block step harder. | Rejected. |

## 5. Design

### 5.1 `lucerna-editor`

**Shared search.** Move `find_in_snapshot` and `build_regex` from `lucerna-workbench/src/find_widget.rs` into a new module `lucerna-editor::find` (add `regex` to `lucerna-editor`, already in the workspace). The Find bar and ⌘D both call this module. *(Claude's decision.)*
- **Whole word** follows the editor's word boundaries (`words.rs`, the semantics of VS Code's `editor.wordSeparators`; VS Code uses this for both Find and ⌘D since version 1.44), not the regex `\b`: take the results of a regex without `\b` and then filter for results whose two ends are word boundaries. This lets `$foo` (PHP) or `foo-bar` (CSS) match. The Find bar changes accordingly: this is a deliberate change, closer to VS Code; the existing Find tests with lowercase text must pass, and any test that relies on `\b` gets its expectation fixed with the reason noted. Project search (`lucerna-project::search::build_pattern`, ripgrep) keeps `\b`.
- **New API for ⌘D:** `find_next(snap, query, from, backwards) -> Option<Range>` searches from an offset, wraps around, stops at the first valid result, reads rope chunk by chunk, without copying the whole buffer. `find_in_snapshot` currently copies the whole buffer and collects every result, so it is not used for ⌘D. With a literal regex on a rope, one ⌘D on a file of a few tens of MB is still under a frame if the result is near; for the bad case (the only result far away in a very large file) the builder measures, and above the threshold runs in the background like the Find bar (`BACKGROUND_SEARCH_BYTES`).
- **⌘D and the Find bar:** when the Find bar is open in that view, ⌘D (the first time from an empty cursor) sets the Find bar's search text to the word just selected, like VS Code; when the Find bar is closed, it is left alone. The Find bar's options are not changed. *(Claude's decision.)*

**Module `multicursor`.** Pure functions taking `(snapshot, &SelectionSet, …)` and returning a new `SelectionSet` (or `None` when nothing changes). Each new selection gets its id from `next_id()`, so "the newest selection" is the largest id.

- `add_next_match(snap, set, opts) / add_previous_match`
  - With an empty selection → each empty selection expands to the word under it, selections with text stay as they are; no further search this time (VS Code). A session that starts this way matches whole word + case sensitive.
  - Every selection has text: the search text is the text of the newest selection; if the selections do not all have the same text, do nothing (VS Code). A session that starts from text the user selected themselves uses the Find bar's current case / whole-word options (not regex: the text is always literal), like VS Code.
  - Search from the end (or the start, in the backward direction) of the newest selection with `find_next`, wrapping around, skipping results that overlap an existing selection. No new result → no change.
- `move_to_next_match / move_to_previous_match`: as above but dropping the newest selection and then adding the next result. With only one selection it is like ⌘D but replaces that selection.
- `select_all_matches` (⇧⌘L and `changeAll` ⌘F2): every result of the selected text (or the word under the cursor); the selection containing the original primary cursor stays primary. There may be many results: run in the background when the file passes a threshold (the builder picks the threshold, for example as the Find bar does now), cut at `multiCursorLimit`.
- `insert_cursor_above / below`: each selection gets one new cursor on the line above / below its head (like VS Code and Zed), keeping the target display column (`Goal::Column`, counted in tabs) across short lines; duplicates merge. The line is the display line when word wrap is on (VS Code uses the view line); the builder may do buffer lines first if that is much simpler, stating so in the plan. Pressing the opposite direction right after removes the layer of cursors just added (a stack of layers, per the parent spec and Zed; VS Code does not have it). The stack is cleared when the selection changes by another path. *(The layered stack model is Claude's decision.)*
- `cursors_at_line_ends` (⇧⌥I): per VS Code — an empty selection is dropped from the result set; each line of a non-empty selection gets a cursor at the line end, except the last line: the cursor is at the selection end if it stops mid-line, and there is no cursor if it stops at column 1. Nothing left → keep as is.
- `column_select(snap, from: (row, visual_col), to: (row, visual_col))`: one selection per line from `from.row` to `to.row`, by display column (a tab counts its full width). A line shorter than the starting column → an empty cursor at the line end (VS Code). Each selection's direction follows the drag direction. The primary selection is the line with `to`.
  - ⇧⌥⌘ + ←/→/↑/↓/PageUp/PageDown use the state `ColumnSelectData { from, to }` (like VS Code) kept in `CodeView`; the first time `from` = the primary cursor. The state is cleared when the selection changes by another path.
- `remove_secondary(set)`: only the primary selection remains. `cancel_selection(set)`: one selection → a cursor at the head.
- `toggle_cursor_at(set, offset)`: ⌥-click. If a cursor / secondary selection contains `offset` → remove it (never remove the last selection); if none → add a cursor, which becomes primary.
- Every function that creates selections cuts at `multiCursorLimit`; the command returns a "was cut" flag so the workbench can report it.
- `editor.multiCursorMergeOverlapping: false`: `from_selections(…, merge)` already has the parameter, but `remap`, `clear_selection` and other callers hard-code `true`. The setting must reach all those places (for example `SelectionSet` holds the merge flag, or pass it through `EditConfig`; the builder chooses).

**Clipboard.** Fix `copy` / `cut` to follow VS Code: only when **every** selection is empty does it take whole lines; if some selection is non-empty, empty selections contribute an empty string. Needed for column select across short lines and Vim `<C-v>`'s `y`.

**Paste.** `paste` takes `multi_cursor_paste: Spread | Full`. `Spread` (default) keeps the existing behavior: split by `pieces` when the number of pieces equals the number of selections; plus a VS Code case: a clipboard from outside (with no `pieces`) whose line count equals the number of selections is split by line. `Full`: each selection receives the whole text.

### 5.2 `lucerna-workbench`

**Remove `block_selection`.** `set_block_selection` is replaced by a function that sets a multi-line `SelectionSet` (using `column_select` or directly from Vim's `block_ranges`). Delete the block drawing path in `element/paint.rs`, the block branch in `copy_selection`, `clipboard_text`, and the remap part when editing. `visible_selections` draws every selection the same. Vim: `sync_view` sets multiple selections for `VimSelection::Block`, keeps Vim's block cursor in the right place (overlay), and the `has_block_selection()` spot at `vim.rs:604` (when Vim is turned off) becomes "Vim is in Visual Block", so cursors the user added themselves are not merged when Vim is turned off.

**With Vim enabled (the user chose approach A).** Today each Vim key calls `sync_view` and collapses to one cursor, so the secondary cursors vanish at the next key. New rules:
- When Vim is in Normal or Visual (not `<C-v>`), every selection-creating command of this step (⌘D, ⌘K ⌘D, ⇧⌘L, ⌘F2, ⌥⌘↑/↓, ⇧⌥I, column select by keys, ⌥-click, ⌥-drag, ⇧⌥-drag) runs as when Vim is off and then **puts Vim into Insert**, keeping the `SelectionSet` just created. Typing replaces the selected text / inserts at every cursor through the editor's typing path; the whole Insert session is one undo step as it is today. The status bar shows `-- INSERT --`, the cursor is a bar.
- In Insert, the commands above run normally, and Vim does not collapse the selections.
- Escape in Insert: collapse to the primary selection and then go to Normal as now (the cursor steps back one character). One Escape is enough, not two steps as when Vim is off.
- `<C-v>` is still Vim's Visual Block (multiple selections, only `y`), and does not enter Insert.
- `.` and macros: an Insert session that starts from a multi-cursor command is recorded like a normal Insert key; the multi-cursor command itself is not recorded into `.` / macros in this step (a limitation documented in the docs, fixed in the later Vim step).

**Drawing.** Every selection and every caret is already drawn. Add: the secondary carets use the same color as the primary caret (Zed); the cursors blink in sync. Nothing else changes in the UI.

**Mouse** (`element.rs` `mouse_down` / `mouse_drag`):
- The add-cursor key = ⌥ when `multiCursorModifier: alt`, ⌘ when `ctrlCmd`. The other key alone does "go to definition" (today's ⌘-click). ⌘⌥-click keeps the meaning "go to definition, open to the side" in both modes (like VS Code), and does not add a cursor.
- The add-cursor key + click → `toggle_cursor_at`. + drag → add a new selection that follows the mouse (the other selections stay). Double/triple-click with the same key → add a word / line.
- ⇧⌥-drag (always ⇧⌥, like VS Code) → `column_select` from where it was pressed to the mouse.
- A plain click still replaces all selections with one.

**Commands and keys** (added to `default-keybindings.json`, `when` as in VS Code):

| Command | Key |
|---|---|
| `editor.action.addSelectionToNextFindMatch` | ⌘D (`editorFocus`) |
| `editor.action.moveSelectionToNextFindMatch` | ⌘K ⌘D |
| `editor.action.addSelectionToPreviousFindMatch`, `editor.action.moveSelectionToPreviousFindMatch` | — (in the command palette) |
| `editor.action.selectHighlights` | ⇧⌘L |
| `editor.action.changeAll` | ⌘F2 (`editorTextFocus && !editorReadonly`) |
| `editor.action.insertCursorAbove` / `insertCursorBelow` | ⌥⌘↑ / ⌥⌘↓ |
| `editor.action.insertCursorAtEndOfEachLineSelected` | ⇧⌥I |
| `cursorColumnSelectLeft/Right/Up/Down/PageUp/PageDown` | ⇧⌥⌘ + ← → ↑ ↓ PageUp PageDown |
| `removeSecondaryCursors` | Escape (`editorHasMultipleSelections && textInputFocus`) |
| `cancelSelection` | Escape (`editorHasSelection && textInputFocus`) |

New context keys: `editorHasMultipleSelections`, `editorHasSelection`. Escape for Find / hover / Vim keeps the existing priority order (with Find open, Escape closes Find first, like VS Code). When Vim is on, Escape goes into Vim as now; Vim Normal has only one cursor so it does not interfere.

⌘D is only an editor command when the editor has focus; `filesExplorer.duplicate` keeps ⌘D in the Explorer.

**Cursor history.** Every command here changes the selection through the existing path, so ⌘U (`cursorUndo`) can undo ⌘D, ⌥-click…, like VS Code.

**Scrolling.** After ⌘D / ⌘K ⌘D / adding a cursor above-below: scroll to the newest selection if it is off screen (like the existing `reveal`). ⇧⌘L does not scroll.

**Limit.** Past `multiCursorLimit` → keep exactly the limit and show a status bar notice `The number of cursors has been limited to N.` (VS Code's wording).

**Status bar.** `DocumentStatus.cursor` (documents.rs) is computed as in §2. Count characters by char; it runs every frame so only count when the selection changes (cache by `selection_generation`), without copying text.

### 5.3 Settings

Add to `default-settings.json` and read into `EditConfig` / the config view:

| Setting | Default | Values |
|---|---|---|
| `editor.multiCursorModifier` | `"alt"` | `"alt"`, `"ctrlCmd"` |
| `editor.multiCursorMergeOverlapping` | `true` | bool |
| `editor.multiCursorPaste` | `"spread"` | `"spread"`, `"full"` |
| `editor.multiCursorLimit` | `10000` | number ≥ 1 |

`editor.selectedTextMatchMode` (noted in §14.2 of the parent spec and in the research) is dropped: it could not be confirmed that this setting exists in VS Code. The behavior is always "take the Find bar's options" as described in §5.1. *(Claude's decision.)*

## 6. Testing

- `lucerna-editor`: unit tests for each function in §5.1 using `marked` (strings with selection markers) like the 4.2 modules: wrap-around, skipping overlaps, different texts, short lines, tabs, multi-byte Unicode, the limit, copy/cut/paste mixing empty and non-empty, pasting outside lines.
- `lucerna-workbench` (gpui tests): ⌘D three times then type; ⌥-click to add then remove; ⇧⌥-drag across short lines then type; two-step Escape; Vim on: ⌘D ⌘D enters Insert, type in two places, Escape back to Normal with one cursor, `u` undoes the whole typing; `multiCursorModifier: ctrlCmd` swaps the roles of ⌥/⌘-click; ⌘U after ⌘D; Vim `<C-v>` select then `y` gives the same text as before (the existing tests must pass without changing their expected results).
- Performance: measure typing with 1000 cursors and ⇧⌘L on a large file in `editor-benchmark`, recorded in `docs/perf-checklist.md`.

## 7. Left to the builder

- The background threshold for ⇧⌘L, how to cache the character count in the status bar.
- Splitting the plan into tasks (suggestion: shared find → pure multicursor → clipboard/paste → remove `block_selection` + Vim → mouse → commands/keys/settings → status bar → measuring).
- Function names, the module structure in `lucerna-editor`.

## 8. After delivery

Update the roadmap table §1.3 (4.3 done), the status line of the parent spec, the README if it lists features; fix the "Aho-Corasick" sentence in §8 of the parent spec to match §4, remove `editor.selectedTextMatchMode` from §8 and §14.2 of the parent spec and add a note to `docs/research/2026-11-11-editing.md` §3.
