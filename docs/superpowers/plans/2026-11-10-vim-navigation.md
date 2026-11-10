# Lucerna V1 (Vim Navigation) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A native, navigation-only vim mode for the read-only code view. It covers motions, search, scrolling, counts, marks, visual select and yank, and folds. It is enabled with `"vim.enabled": true`.

**Architecture:**
- A new gpui-free crate, `lucerna-vim`, holds a per-view state machine. It is fed typed characters, keeps the cursor and the visual anchor itself, and returns side-effect `Action`s.
- `lucerna-workbench` gets a `VimLayer` entity.
  - It runs in `Workbench::dispatch_key` before the keymap.
  - It reads the active `CodeView` through a `ViewInfo` adapter.
  - It applies actions to the view: cursor overlay, selection, scroll, folds, clipboard and highlights.
  - It drives a status-bar mode indicator.
- `/` and `?` reuse the find widget through a small event hook.

**Tech Stack:** Rust 2024, gpui (workbench only), `lucerna-text` (`BufferSnapshot`, `Point`, `Reload`). There are no new external dependencies.

**Spec:** `docs/superpowers/specs/2026-11-10-vim-navigation-design.md`. Background research: `docs/research/2026-11-09-vim-mode.md` §5. Read the spec section each task cites before starting it.

## Global Constraints

- **Licensing:**
  - The project is licensed MIT OR Apache-2.0.
  - **Never copy code from Zed's `crates/vim`**, which is GPL-3. Its design may be read.
  - VSCodeVim test cases (MIT) may be adapted. Each adapted test carries a comment of the form `// Adapted from VSCodeVim <file>: "<test title>"`, and Task 11 adds the notice.
- **Crate boundaries:**
  - Only `lucerna-workbench` uses gpui.
  - `lucerna-vim` depends only on `lucerna-text`, plus `std`.
- **Positions:**
  - `Point { row: u32, column: u32 }`, where `column` is a **UTF-8 byte offset**.
  - Every move steps by `char`. Never split a char.
  - Horizontal reads stop at 10 000 chars per line (`MAX_LINE_CHARS = 10_000`, which equals `code_view::MAX_RENDERED_LINE_CHARS`).
- **Cursor:** in Normal mode the cursor sits on a char, never past the last char of the line. On an empty line it is at column 0.
- **Settings and defaults:**

  | Key | Default |
  |---|---|
  | `vim.enabled` | false |
  | `vim_mode` | alias, read only when `vim.enabled` is unset |
  | `vim.ignorecase` | true |
  | `vim.wrapscan` | true |
  | `vim.startofline` | true |
  | `vim.scroll` | 0 (half page) |
  | `editor.cursorSurroundingLines` | 0 |

- **Count:** capped at 99 999.
- **Mode indicator text, exact:** `-- NORMAL --`, `-- VISUAL --`, `-- VISUAL LINE --`, `-- VISUAL BLOCK --`.
- **Context keys, exact:**
  - `vim.active` (bool)
  - `vim.mode`, with values `'Normal'`, `'Visual'`, `'VisualLine'` or `'VisualBlock'`
- **Other sessions share this checkout:**
  - Run `git pull --rebase` before starting each task.
  - Stage by explicit path only.
  - Commit on `main`.
  - The trailer is `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- **Toolchain and disk:**
  - Cargo needs `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"`.
  - Keep at least 1.2 GB of disk free (`df -h /`).
- **Talk to the user in Vietnamese.** Code and comments are in English.

## Review Focus

1. **Input sources and IME.** With the Vietnamese Telex input source active, `KeyDownEvent`s may come with `key_char: None`. This happens during composition. Expected: the vim layer ignores such keys (returns `Unhandled`, no panic), and plain `j`/`k` still work when `key_char` is present. Test: `Task 9: ime_key_without_char_is_ignored`.
2. **A pending key across views.** Type `g` in one split, then focus another group and type `g`. Expected: the second view does not run `gg`, because pending state is per view. Test: `Task 9: pending_state_is_per_view`.
3. **Reload during Visual mode removes the anchor's rows.** Expected: the anchor and head are clamped to valid rows and chars, the mode stays Visual, and nothing panics. Test: `Task 9: reload_in_visual_clamps_selection`.
4. **Disabling vim while in Visual mode.** Expected: the selection is cleared, the cursor overlay disappears, the indicator hides, and `j` reaches the keymap again. Test: `Task 9: disabling_vim_clears_overlay_and_selection`.
5. **Close a tab and reopen the same file.** Expected: the new view starts in Normal mode, marks set before still work (they are keyed by path), and the dropped view's `Vim` entry is removed. Test: `Task 9: reopened_file_keeps_marks_new_state`.

---

## File Structure

```
crates/lucerna-vim/                 (new crate)
  Cargo.toml
  src/lib.rs        pub API re-exports: Key, Mode, Action, Outcome, ViewInfo, Settings,
                    SearchQuery, VimSelection, FoldAction, Vim, VimGlobals
  src/state.rs      Vim struct, feed(), pending keys, count, goal, mode switching
  src/classify.rs   CharClass, line access helpers (capped line, char stepping)
  src/motion.rs     pure motion fns over &BufferSnapshot
  src/scroll.rs     ctrl-d/u/f/b, zz/zt/zb, H/M/L (screen-relative)
  src/marks.rs      FileMarks, VimGlobals.marks, remap on Reload
  src/visual.rs     VimSelection, yank text
  src/search.rs     * # n N, search_submitted
  src/test_util.rs  #[cfg(test)] TestView + assert_keys/parse/render helpers
crates/lucerna-workbench/src/
  vim.rs            VimLayer entity: key conversion, ViewInfo adapter, match cache,
                    applying actions, settings, context keys
  vim_indicator.rs  ModeIndicator status item
  code_view/mod.rs  cursor overlay, block selection, set_cursor, fold_at,
                    selection_generation, matching_bracket, CodeViewEvent::Reloaded
  code_view/element.rs, code_view/element/paint.rs   paint overlay + block selection
  find_widget.rs    vim hook: open_for_vim, FindWidgetEvent, query getter
  workbench.rs      call VimLayer in dispatch_key; context keys; status item
crates/lucerna-workbench/tests/vim.rs   integration tests
```

---

### Task 1: The `lucerna-vim` crate, its core types, and Normal-mode basics

Spec: §2.1, §2.2, §3, §4.1, and the `hjkl 0 ^ $` rows of §4.2.

**Files:**
- Create: `crates/lucerna-vim/Cargo.toml`, `src/lib.rs`, `src/state.rs`, `src/classify.rs`, `src/motion.rs`, `src/test_util.rs`
- Modify: root `Cargo.toml` (`[workspace.dependencies]` gets `lucerna-vim = { path = "crates/lucerna-vim" }`)

**Interfaces (produced, used by every later task):**

```rust
pub enum Key { Char(char), Ctrl(char), Escape, Enter }
pub enum Mode { Normal, Visual, VisualLine, VisualBlock }   // Copy, Eq, Debug
impl Mode { pub fn label(self) -> &'static str; pub fn context_value(self) -> &'static str }
// label: "-- NORMAL --" ...; context_value: "Normal" | "Visual" | "VisualLine" | "VisualBlock"

pub struct Settings { pub ignorecase: bool, pub wrapscan: bool, pub startofline: bool,
                      pub scroll: u32, pub scrolloff: u32 }       // Default = spec defaults
pub struct SearchQuery { pub pattern: String, pub is_regex: bool,
                         pub case_sensitive: bool, pub whole_word: bool }   // Clone, Eq, Hash
pub enum FoldAction { Toggle, Close, Open }

pub trait ViewInfo {
    fn path(&self) -> &std::sync::Arc<std::path::Path>;
    fn snapshot(&self) -> &BufferSnapshot;
    fn top_row(&self) -> u32;
    fn visible_rows(&self) -> u32;          // 0 before first paint → callers use 30
    fn is_row_hidden(&self, row: u32) -> bool;
    fn tab_size(&self) -> u32;
    fn matching_bracket(&self, offset: usize) -> Option<usize>;
    fn matches(&self, query: &SearchQuery) -> Option<std::sync::Arc<[std::ops::Range<usize>]>>;
}

pub enum Action {
    CursorMoved { jump: bool },   // cursor/selection changed; jump → center if off-screen
    ScrollTopRow(u32),
    OpenSearch { backward: bool },
    Highlight(SearchQuery),
    Yank(String),
    Fold { row: u32, action: FoldAction },
    ModeChanged(Mode),
}
pub enum Outcome { Handled(Vec<Action>), Unhandled, Retry(SearchQuery) }

pub struct Vim { /* mode, cursor: Point, goal: Goal, anchor: Point, pending, count */ }
impl Vim {
    pub fn new(cursor: Point) -> Vim;
    pub fn feed(&mut self, key: Key, view: &dyn ViewInfo, g: &mut VimGlobals, s: &Settings) -> Outcome;
    pub fn mode(&self) -> Mode;
    pub fn cursor(&self) -> Point;
    pub fn set_cursor(&mut self, p: Point);          // external move: clamps, resets goal
    pub fn pending(&self) -> &str;                    // e.g. "5", "g", "2f"
    pub fn reset(&mut self, cursor: Point);           // → Normal, clear pending, clamp cursor
    pub fn clamp(&mut self, snapshot: &BufferSnapshot); // after reload: clamp cursor + anchor
}
#[derive(Default)] pub struct VimGlobals { /* filled by Tasks 3, 4, 5, 7 */ }

// classify.rs
pub(crate) enum CharClass { Word, Punct, Space }
pub(crate) fn class(c: char, big_word: bool) -> CharClass;  // Word = is_alphanumeric() || '_'
pub(crate) const MAX_LINE_CHARS: usize = 10_000;
pub(crate) fn line(s: &BufferSnapshot, row: u32) -> String;  // without '\n'/'\r\n', capped
pub(crate) fn display_col(line: &str, byte_col: u32, tab: u32) -> u32;
pub(crate) fn byte_col_for_display(line: &str, display: u32, tab: u32) -> u32; // clamps to last char
pub(crate) fn last_char_col(line: &str) -> u32;              // 0 for empty
```

Rules:
- `feed` snapshots `self` (derive `Clone`) at entry. When an inner step needs matches that are not ready, it restores the snapshot and returns `Outcome::Retry(query)`, which Task 7 uses.
- Any printable char that is not a command returns `Handled(vec![])` and clears the pending keys.
- `Escape` behaves as follows:
  - With keys pending: clear them and return `Handled`.
  - In Normal mode with nothing pending: return `Unhandled`.
  - In a Visual mode: go back to Normal.
- `Ctrl(c)` for a letter that has no binding returns `Unhandled`, so `ctrl+g` and `ctrl+tab` reach the keymap.
- `Enter` moves down `count` rows to the first non-blank, as Vim's `+` does.
- The goal column is kept as `Goal::Display(u32)` or `Goal::Eol`. `j`/`k` skip rows for which `view.is_row_hidden(row)` is true.

- [ ] **Step 1: Write `test_util.rs`.**
  - `TestView::new(text)` takes a text with a single `|` cursor marker. Its defaults are: `top_row` 0, `visible_rows` 30, no hidden rows, `tab_size` 4, and `path` `"test.rs"`.
  - `matching_bracket` is a naive scan over `()[]{}` that ignores strings.
  - `matches` is a literal search: ASCII-case-folded unless `case_sensitive`, and with `\b` boundaries implemented by checking `class` when `whole_word` is set.
  - `assert_keys(start: &str, keys: &str, end: &str)` and `assert_keys_with(settings, view_setup: impl FnOnce(&mut TestView), ...)`.
  - Key notation in `keys`:
    - plain chars as they are;
    - `<C-d>` for `Ctrl('d')`;
    - `<Esc>`;
    - `<CR>` for `Enter`.
- [ ] **Step 2: Write failing tests in `state.rs` and `motion.rs`.** These include the following VSCodeVim adaptations:
  - `te|xt text\ntext\ntext` with `3$` puts the cursor on the last `t` of row 2. *Adapted from VSCodeVim `test/mode/normalModeTests/motions.test.ts`: "Can handle $ with a count".*
  - `te|xt text text` with `3$` gives `text text tex|t`. *Same file: "Can handle $ with a count at end of file".*
  - `|one one one\ntwo\nthree` with `$jj` gives `thre|e`. *Same file: "maintains column position correctly with $".*
  - Lucerna-only cases:
    - `l` stops at the last char.
    - `h` stops at column 0.
    - `j` across a shorter line restores the goal column on the longer line.
    - `10j` moves 10 rows: the `0` is part of the count.
    - `0` alone goes to column 0.
    - `^` on `"   "` goes to the last char.
    - `^` on `"\tfoo"` goes to `f`.
    - `l` over `"aé😀b"` lands on the byte columns 1, 3 and 7.
    - `j` with row 1 hidden (`view.hidden = vec![1..2]`) goes from row 0 to row 2.
    - On a 20 000-char line, `$` goes to char 9 999.
    - On an empty file, every key keeps the cursor at (0,0).
    - `x` returns `Handled` and moves nothing.
    - `<C-g>` returns `Unhandled`.
    - `<Esc>` in Normal mode returns `Unhandled`.
    - `5<Esc>j` moves 1 row.
    - `<CR>` goes to the next row's first non-blank.
- [ ] **Step 3:** Run `cargo test -p lucerna-vim`. Expected: compile errors or failures.
- [ ] **Step 4:** Implement the crate per the interfaces above.
- [ ] **Step 5:** Run `cargo test -p lucerna-vim`. Expected: all tests pass.
- [ ] **Step 6:** Run `cargo clippy -p lucerna-vim --all-targets -- -D warnings`. Expected: clean.
- [ ] **Step 7:** Commit with message `feat(vim): lucerna-vim crate with normal-mode core`.

---

### Task 2: Word motions

Spec: the `w b e ge` and `W B E gE` rows of §4.2.

**Files:** Modify `crates/lucerna-vim/src/motion.rs` and `state.rs`, which binds `w b e W B E` and `ge gE` through the `g` pending state.

**Interfaces:** Produces `pub(crate) fn word_motion(s: &BufferSnapshot, from: Point, kind: WordMotion, big: bool, count: u32) -> Point`, where `enum WordMotion { NextStart, NextEnd, PrevStart, PrevEnd }`.

Rules:
- An empty line is a stop for `w`/`b`/`ge`. A line of only whitespace is skipped.
- `e`/`E` always advance at least one char first, and skip empty lines.
- `w` on the last word of the file clamps to the last char (VSCodeVim goes one past it; Normal mode cannot).
- If a count step makes no progress, break out of the loop.

- [ ] **Step 1: Write failing tests.** The cases are adapted from VSCodeVim. The `motions.test.ts` ones carry the title in the comment.
  - `one |two three four` + `w` → `one two |three four` ("w moves to next word").
  - `one |two,.;/three four` + `w` → `one two|,.;/three four` ("w stops at punctuation").
  - `one two|,.;/three four` + `w` → `one two,.;/|three four` ("w on punctuation jumps over punctuation").
  - `o|ne  \n two three` + `w` → `one  \n |two three` ("w goes over EOL").
  - `|one two three four` + `2w` → `one two |three four` ("[count]w").
  - From `test/motion.test.ts`, on the text `if (true) {\n  return true;\n} else {\n\n  return false;\n  \n} // endif`. Positions are (row, col):

    | Start | Keys | End | Title |
    |---|---|---|---|
    | (0,10) | `w` | (1,2) | "last word should move to next line" |
    | (2,7) | `w` | (3,0) | "stops on empty line" |
    | (4,14) | `w` | (6,0) | "skips whitespace only line" |
    | (6,6) | `w` | (6,9) | "last word on last line should go to end of document"; Lucerna clamps to the last char |
    | (6,0) | `b` | (4,14) | |
    | (2,7) | `e` | (4,7) | |
    | (0,3) | `W` | (0,10) | |
    | (0,4) | `E` | (0,8) | |

  - `one two\nthree\nfour fiv|e` + `gegegegege` → `|one two`. *Adapted from VSCodeVim `test/mode/normalModeTests/commands.test.ts`: "Can handle 'ge' in multiple lines case2".*
  - Lucerna-only cases:
    - `|Xin chào thế giới` + `w` → `Xin |chào thế giới`, and a second `w` lands on `thế`.
    - `e` from `X` lands on `n`.
    - `a|😀b c` + `w` stops on `b`, because the emoji is in the punctuation class.
    - `B` from `c` in `a.b c` goes to `a`.
- [ ] **Step 2:** Run `cargo test -p lucerna-vim word`. Expected: FAIL.
- [ ] **Step 3:** Implement `word_motion` and bind the keys.
- [ ] **Step 4:** Run `cargo test -p lucerna-vim`. Expected: PASS.
- [ ] **Step 5:** Commit with message `feat(vim): word motions`.

---

### Task 3: File, paragraph, bracket and find-char motions, plus `H M L`

Spec: the `gg G`, `{ }`, `%`, `H M L`, `f F t T ; ,` rows of §4.2. These motions are jumps, and the jump flag is recorded in Task 5.

**Files:** Modify `motion.rs`, `state.rs`, and `src/scroll.rs` (create it for `H M L`).

**Interfaces:**
- `VimGlobals` gains `pub(crate) last_find: Option<FindChar>`, where `struct FindChar { ch: char, forward: bool, till: bool }`.
- Motions return `(Point, bool /*jump*/)` to `feed`. `feed` emits `Action::CursorMoved { jump }`.

Rules:
- **`gg`, `G`, `Ngg`, `NG`:** clamp to the last row. With `startofline`, the cursor goes to the first non-blank; otherwise it keeps the goal.
- **`N%`** (1..=100): go to row `(N*lines + 99)/100 - 1`, then to the first non-blank.
- **`%` without a count:**
  1. Find the first char of `()[]{}` at or after the cursor on the line.
  2. Get its partner with `view.matching_bracket(byte offset)`.
  3. If there is no bracket on the line or no partner, the cursor stays.
- **`{` / `}`:** only rows of length 0 are boundaries. From inside a run of empty rows, first leave the run. If no boundary is found, `}` goes to the last char of the last row and `{` goes to (0,0).
- **`H`, `M`, `L`:** use the visible, non-hidden rows starting at `top_row`. The margin is `scrolloff`, except that no margin applies at the file top (for `H`) or bottom (for `L`). The column follows `startofline`.
- **`;` after `t`/`T`:** if the repeat would not move, search with count + 1.
- **`f`/`t` matching:** an exact char compare; `fế` must work.

- [ ] **Step 1: Write failing tests.** Cases adapted from VSCodeVim:
  - From `motions.test.ts`:
    - ` 123456\n 123|456\n 123456` + `9gg` → row 2, col 1 ("[count]gg (startofline=true), count greater than last line").
    - The same start + `1G` with `startofline=false` → row 0, col 4 ("[count]G (startofline=false)").
    - `text tex|t` + `fe,` → `text t|ext`.
    - `|a a a a a` + `fa;;;,,,` → `a |a a a a`.
    - `text tex|t` + `^tttt` → `te|xt text` ("Can handle 't' twice").
    - `text tex|t` + `$TtTt` → `text t|ext` ("Can handle 'T' twice").
  - From `test/register/repeatableMovement.test.ts`: `|abc abc abc` + `tc;` → `abc a|bc abc`.
  - From `test/mode/normalModeTests/matchingBracket.test.ts`:
    - `|one (two)` + `%` → `one (two|)`.
    - `((( ))|)` + `%` → `|((( )))`.
    - `|foo->bar(baz);` + `%` → `foo->bar(baz|);`.
  - From `test/motion.test.ts` "paragraph motion", on the text `this text has\n\nmany\nparagraphs\n\n\nin it.\n\nWOW`:

    | Start | Keys | End |
    |---|---|---|
    | (2,0) | `}` | (4,0) |
    | (4,0) | `}` | (7,0) |
    | (7,0) | `}` | (8,2) (Lucerna clamps to the last char) |
    | (5,0) | `{` | (1,0) |

  - Lucerna-only cases:
    - `}` treats a whitespace-only row as text, not a boundary.
    - `50%` on 10 rows goes to row 4.
    - `3H` with `top_row` 10 goes to row 12.
    - `L` with `scrolloff` 2, `top_row` 10 and `visible_rows` 20 goes to row 27.
    - `M` on a 5-row file goes to row 2.
    - `fế` on `Xin chào |thế` stops on `ế`.
    - `f` with no match does not move.
- [ ] **Step 2:** Run `cargo test -p lucerna-vim`. Expected: the new tests FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -p lucerna-vim`. Expected: PASS.
- [ ] **Step 5:** Commit with message `feat(vim): file, paragraph, bracket, find-char and screen motions`.

---

### Task 4: Scrolling and folds

Spec: §4.4.

**Files:** Modify `scroll.rs` and `state.rs`.

**Interfaces:**
- `VimGlobals` gains `pub(crate) scroll: u32`, where 0 means "use `Settings::scroll`, else half the visible rows".
- Emits `Action::ScrollTopRow(row)`, followed by `Action::CursorMoved { jump: false }` when the cursor moved.
- Emits `Action::Fold { row: cursor.row, action }`.

Rules:
- **`Ctrl+d` / `Ctrl+u`:** move the top row and the cursor by `n` visible rows, where:
  - `n` is the count; a count also sets `globals.scroll = count`.
  - Otherwise `n = globals.scroll`.
  - Otherwise `n = settings.scroll`.
  - Otherwise `n = max(1, visible / 2)`.

  The cursor is clamped to the file and keeps its goal. At the top of the file, `Ctrl+u` still moves the cursor up.
- **`Ctrl+f` / `Ctrl+b`:** scroll by `max(1, visible - 2)` rows, then clamp the cursor into the new view.
- **`zz` / `zt` / `zb`:** only emit `ScrollTopRow`. The cursor and goal are unchanged.
  - `zt` puts the cursor row at the top.
  - `zz` puts `visible/2` visible rows above the cursor row.
  - `zb` puts the cursor row at the bottom.
  - Skip hidden rows when counting.
- **`za` / `zc` / `zo`:** emit `Fold` only. The workbench sets the cursor afterwards (Task 9).

- [ ] **Step 1: Write failing tests.** These assert both the cursor and the emitted `ScrollTopRow`.
  - Adapted from VSCodeVim `test/mode/modeNormal.test.ts`:
    - `ab|c\ndef\nghi\njkl\nmno\npqr\nstu` + `2<C-d><C-d>` → `|mno` ("[count]<C-d> sets and adheres to scroll option").
    - `\t hello world\nhello\nhi hello\nvery long line |at the bottom` + `<C-u>`, with `visible_rows` 30 → cursor `\t |hello world` ("can handle <C-u> when first line is visible and starting column is in the middle").
  - Adapted from `motions.test.ts`: `very long line of text....|.\nshort line` + `jzzk` → unchanged ("Preserves desired cursor position when pressing zz").
  - Lucerna-only cases:
    - `<C-f>` with `visible_rows` 20 on 100 rows emits `ScrollTopRow(18)`, and the cursor ends at row ≥ 18.
    - `zt` at row 40 emits `ScrollTopRow(40)`.
    - `zb` at row 40 with `visible_rows` 20 emits `ScrollTopRow(21)`.
    - `za` emits `Fold { row, action: Toggle }`.
- [ ] **Step 2:** Run the tests. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -p lucerna-vim`. Expected: PASS.
- [ ] **Step 5:** Commit with message `feat(vim): scrolling and fold keys`.

---

### Task 5: Marks and jumps

Spec: §4.5.

**Files:** Create `marks.rs`. Modify `state.rs`.

**Interfaces:**
- `VimGlobals` gains `marks: HashMap<Arc<Path>, FileMarks>`, where `FileMarks { letters: [Option<Point>; 26], last_jump: Option<Point>, visual: Option<(Point, Point)> }`.
- `pub fn remap_marks(&mut self, path: &Path, reload: &Reload, snapshot: &BufferSnapshot)` on `VimGlobals`. It maps rows with `Reload::adjust_row` and clamps the column to the last char of the new row.

Rules:
- Every motion that returns `jump = true` stores the pre-jump cursor in `last_jump` before moving. This covers the Task 3 motions and, later, Task 7's searches.
- A mark jump is itself a jump.
- `'x` goes to the first non-blank of the mark's row. `` `x `` goes to the exact position, clamped.
- `''` and ` `` ` use `last_jump`.
- An unset mark is a no-op, returning `Handled(vec![])`.
- `m` followed by a non-letter cancels.
- The `<`/`>` marks are written by Task 6. Reading them with `` `< `` / `` `> `` and `'<` / `'>` is implemented here.

- [ ] **Step 1: Write failing tests.**
  - `|hello world and mars` + ``wma2w`a`` → `hello |world and mars`. *Adapted from VSCodeVim `test/marks.test.ts`: "Can jump to lowercase mark".*
  - Lucerna-only cases:
    - `'a` lands on the first non-blank.
    - `G''` returns to the start.
    - `` ` `` + an unset letter does not move.
    - `remap_marks` after a reload that inserts 3 rows above shifts the mark down by 3.
    - When the reload deletes the mark's row, the mark is clamped.
    - Marks live under the view's `path()`. Two `TestView`s with different paths have independent marks.
- [ ] **Step 2:** Run the tests. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -p lucerna-vim`. Expected: PASS.
- [ ] **Step 5:** Commit with message `feat(vim): marks and jump-back`.

---

### Task 6: Visual modes and yank

Spec: §3 (mode switching) and §4.6.

**Files:** Create `visual.rs`. Modify `state.rs`.

**Interfaces:**

```rust
pub enum VimSelection {
    Char { start: Point, end_inclusive: Point },
    Line { start_row: u32, end_row: u32 },
    Block { rows: Range<u32>, display_cols: Range<u32>, to_eol: bool },
}
impl Vim { pub fn selection(&self, s: &BufferSnapshot) -> Option<VimSelection>; }
pub fn exclusive_range(sel: &VimSelection, s: &BufferSnapshot) -> Option<Range<Point>>;
// Char: start .. byte after end char (past '\n' when end is the line end); Line: (r0,0)..(r1+1,0)
// or end of buffer; Block → None (caller uses block_ranges)
pub fn block_ranges(sel: &VimSelection, s: &BufferSnapshot, tab: u32) -> Vec<Range<Point>>;
pub fn yank_text(sel: &VimSelection, s: &BufferSnapshot, tab: u32) -> String;
```

Rules:
- `v`, `V` and `<C-v>` switch modes as spec §3 describes, each emitting `ModeChanged`.
- In Visual mode the motions move the head. `$` in Visual (char) mode includes the line break.
- `o` swaps the anchor and the head.
- `y`:
  - Emits `Yank(text)`, then `ModeChanged(Normal)` and `CursorMoved { jump: false }`.
  - Moves the cursor to the selection start: column 0 of the first row for Line, the top-left for Block.
  - Stores the `visual` marks as `(start, end)`.
  - Yank text:
    - Line: lines joined, each ending in `\n`, so a final `\n` is added even if the file has none.
    - Block: the rows' slices joined with `\n`; rows shorter than the left edge give an empty slice.
- `Esc` goes back to Normal. The cursor stays on the head and the goal is kept.

- [ ] **Step 1: Write failing tests.**
  - Adapted from VSCodeVim `test/mode/modeVisualLine.test.ts`: `rocinante\nnau|voo\nanubis\ncanterbury` + `Vj<Esc>` → `anu|bis` ("Exiting via <Esc> returns cursor to original column").
  - Adapted from VSCodeVim `test/mode/normalModeTests/motions.test.ts`: `one\nXx|x\ntwo\n  three` + `vjjv'<` → `|Xxx` ("Jump (line) to visual start '<").
  - Adapted from VSCodeVim `test/marks.test.ts`: `one\nt|wo\nthree` + ``vjlo<Esc>gg`>`` → `th|ree` ("'> set by Visual mode (backward selection)").
  - Lucerna-only cases:

    | Start | Keys | Yank | Cursor after |
    |---|---|---|---|
    | `a|bc def` | `vey` | `"bc"` | (0,1) |
    | `abc\nd|ef\nghi` | `Vjy` | `"def\nghi\n"` | (1,0) |
    | `ab|cd\nef\nghij` | `<C-v>jjly` | `"cd\n\nij"` | (0,2) |

    - Block yank with a tab: `"\tab\n    cd"` with tab 4, block on display columns 4..6 → `"ab\ncd"`.
    - `vV<C-v>v` ends in Normal mode with the `ModeChanged` sequence `[Visual, VisualLine, VisualBlock, Normal]`.
    - `exclusive_range` for `Char { (0,0), (0,2) }` on `"abc\n"` is `(0,0)..(0,3)`. With `$` in Visual mode it is `(0,0)..(1,0)`.
- [ ] **Step 2:** Run the tests. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -p lucerna-vim`. Expected: PASS.
- [ ] **Step 5:** Commit with message `feat(vim): visual, visual-line, visual-block and yank`.

---

### Task 7: Search inside the crate: `* # n N` and submitted searches

Spec: §4.3.

**Files:** Create `search.rs`. Modify `state.rs`.

**Interfaces:**
- `VimGlobals` gains `pub(crate) last_search: Option<(SearchQuery, bool /*backward*/)>`.
- `impl Vim { pub fn search_submitted(&mut self, query: SearchQuery, backward: bool, origin: Point, view: &dyn ViewInfo, g: &mut VimGlobals, s: &Settings) -> Outcome; }`
  - It takes the count typed before `/`, which is stored when `OpenSearch` was emitted.
- `/` and `?` in `feed` emit `OpenSearch { backward }` and remember the count.

Rules:
- **`*` / `#`:**
  - The word is the Word-class run under the cursor. If the cursor is on Space or Punct, use the next Word run on the same line. If there is none, return `Handled(vec![])`.
  - The query uses the regex-escaped word: `is_regex: true`, `whole_word: true`, `case_sensitive: !s.ignorecase`.
  - `#` searches backward from the start of the current word.
  - Both set `last_search` and emit `Highlight(query)`.
- **`n` / `N`:** follow the direction of `last_search` or its reverse.
  - The count picks the Nth match.
  - The next match is a binary search (`partition_point`) on match starts against the cursor's byte offset.
  - Wrapping follows `wrapscan`. Without wrap and no match ahead, the cursor does not move.
  - These are jumps.
- **`view.matches()` returning `None`:** restore the state and return `Outcome::Retry(query)`.

- [ ] **Step 1: Write failing tests.** The `TestView::matches` from Task 1 is enough for literal and whole-word patterns. Escaped regex words in the tests stay alphanumeric.
  - Adapted from VSCodeVim `motions.test.ts`:
    - `|blah blahblah duh blah blah` + `*` → `blah blahblah duh |blah blah`.
    - `|test TEST test` + `*` with `ignorecase=true` → `test |TEST test` ("* ignores smartcase").
    - `one  |   \none two one two` + `*` → no move ("* does not go over line boundaries").
    - `abc abcdef| abc` + `#` → `|abc abcdef abc` ("Can handle # on whitespace").
    - `one o|ne` + `#` → `|one one` ("Can handle # already on the word").
  - For the `search_submitted` cases, drive the call directly with a literal `SearchQuery`:
    - `|one two two two` + query `"tw"` with count 5 → `one two |two two`. *Adapted from VSCodeVim `motions.test.ts`: "…count exceeding max number of matches and wrapscan".*
    - `one two two thre|e` + backward query `"two"`, then `n` → `one |two two three`. *Adapted from VSCodeVim `motions.test.ts`: "Can run a reverse and find next search".*
    - `x end\nx\nx\n|start` + backward query `"x"`, then `nn` → `|x end`. *Adapted from VSCodeVim `test/search/search.test.ts`: "?, match at EOL, walk over matches".*
  - Lucerna-only cases:
    - `n` with `wrapscan=false` at the last match does not move.
    - `#` then `n` goes backward.
    - A view whose `matches` returns `None` makes `n` return `Retry(query)` and leaves the cursor, pending keys and count unchanged.
    - `*` emits `Highlight` with `whole_word: true`.
- [ ] **Step 2:** Run the tests. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -p lucerna-vim`, then `cargo clippy -p lucerna-vim --all-targets -- -D warnings`. Expected: PASS and clean.
- [ ] **Step 5:** Commit with message `feat(vim): star, hash, n/N and submitted searches`.

---

### Task 8: CodeView support: cursor overlay, block selection, `set_cursor`, `fold_at`, events

Spec: §2.4. This task does not use the vim crate.

**Files:**
- Modify `crates/lucerna-workbench/src/code_view/mod.rs`, `code_view/element.rs`, `code_view/element/paint.rs` and `code_view/layout.rs` (if the scroll math lives there).
- Test: `crates/lucerna-workbench/tests/code_view.rs`.

**Interfaces (produced for Task 9):**

```rust
pub enum FoldOp { Toggle, Close, Open }
pub enum CursorShape { Block }
pub struct CursorOverlay { pub at: Point, pub shape: CursorShape }
pub enum CodeViewEvent { Reloaded { reload: Reload } }   // impl EventEmitter<CodeViewEvent>
impl CodeView {
    pub fn set_cursor_overlay(&mut self, overlay: Option<CursorOverlay>, cx);
    pub fn set_block_selection(&mut self, ranges: Option<Vec<Range<Point>>>, cx); // clears normal selection
    pub fn set_cursor(&mut self, p: Point, cx);   // collapse selection; unfold; scroll minimally w/ editor.cursorSurroundingLines
    pub fn reveal_centered(&mut self, p: Point, cx); // = current go_to body; go_to calls it
    pub fn clear_selection(&mut self, cx);
    pub fn fold_at(&mut self, row: u32, op: FoldOp, cx) -> Option<u32>; // header row acted on
    pub fn selection_generation(&self) -> u64;    // bumped by every set_selection / set_buffer / reload / clear
    pub fn matching_bracket(&self, offset: usize) -> Option<usize>; // the closure in update_brackets, extracted
    pub fn is_row_hidden(&self, row: u32) -> bool;
}
```

Rules:
- `FoldOp` is defined in `code_view`, and `vim.rs` maps `FoldAction` → `FoldOp`. This keeps `code_view` free of `lucerna-vim`.
- `apply_reload` emits `CodeViewEvent::Reloaded` after the update.
- **Overlay painting:**
  - Fill the char cell `x_for(col)..x_for(next char col)` on the overlay's display row. Use half a char width on an empty line or at the end of the line.
  - The color is `editor.foreground` with alpha 0.4, added to `FrameColors`.
  - Paint it after the selection and before the text layer, so the glyph stays readable.
  - When `cursor_row` drives the active-line highlight, prefer the overlay row when an overlay is set.
- **Block selection:** paint each range with `range_bounds` in the selection color. `copy_selection()` returns the block ranges' text joined with `\n` when a block is set.
- **`set_cursor` scrolling:** if the row is above `top + margin`, set the top to `row - margin`. If it is below `top + visible - 1 - margin`, scroll so the row sits at the bottom margin. Count visible rows with `folds.next_visible`. Before the first paint (`visible_rows() == 0`), behave like `go_to`.

- [ ] **Step 1: Write failing tests in `tests/code_view.rs`:**
  - `set_cursor_scrolls_minimally`: with 200 rows, after draw, `set_cursor((150,0))` makes `scroll_top_row() == 150 - visible_rows() + 1`.
  - `set_cursor_respects_surrounding_lines`: with `editor.cursorSurroundingLines: 3`, `set_cursor` at the row below the view puts the top row 3 rows earlier than without it.
  - `set_cursor_inside_view_does_not_scroll`.
  - `block_selection_copies_rows`: ranges `[(0,1)..(0,3), (1,1)..(1,3)]` on `"abcd\nefgh"` make `copy_selection() == Some("bc\nfg")`.
  - `fold_at_toggles_innermost`: Rust text with nested braces; `fold_at(inner_row, Close)` returns the inner header, and `is_row_hidden(inner_row + 1)` is true.
  - `selection_generation_bumps_on_mouse_select`: reuse the existing mouse-drag test pattern; the generation increases.
  - `reload_emits_event`: subscribe and call `apply_reload`; the event arrives.
  - `cursor_overlay_paints_without_panic`: set an overlay at the end of a line, on an empty line, and on an emoji, then draw.
- [ ] **Step 2:** Run `cargo test -p lucerna-workbench --test code_view`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -p lucerna-workbench --test code_view`, then all of `cargo test -p lucerna-workbench`, so no existing test regresses. Expected: PASS.
- [ ] **Step 5:** Commit with message `feat(code-view): cursor overlay, block selection, set_cursor, fold_at`.

---

### Task 9: Workbench `VimLayer`: dispatch, settings, actions and context keys

Spec: §2.3, §5 and §6.

**Files:**
- Create: `crates/lucerna-workbench/src/vim.rs`.
- Modify:
  - `workbench.rs`: hold `vim: Entity<VimLayer>`; call it in `dispatch_key` before the keymap; add `vim.active` and `vim.mode` in `context_keys`.
  - `lib.rs`: `mod vim; pub use vim::VimLayer;`.
  - `crates/lucerna-workbench/Cargo.toml`: add `lucerna-vim`.
  - `crates/lucerna-settings/assets/default-settings.json`: add `vim.ignorecase`, `vim.wrapscan`, `vim.startofline`, `vim.scroll` and `editor.cursorSurroundingLines` with their defaults. **Do not** add `vim.enabled`, or the `vim_mode` alias can never apply.
- Test: create `crates/lucerna-workbench/tests/vim.rs`, using the helpers copied from `tests/code_view.rs` (`init`, `workbench`, `open`, `draw`).

**Interfaces:**

```rust
pub struct VimLayer { /* enabled, settings: lucerna_vim::Settings, globals: VimGlobals,
                         views: HashMap<EntityId, ViewState{ vim: Vim, generation: u64 }>,
                         match_cache: HashMap<EntityId, (u64 /*revision*/, SearchQuery, Arc<[Range<usize>]>)>,
                         search: Option<VimSearch> /* Task 10 */ } }
impl VimLayer {
    pub fn new(cx: &mut Context<Self>) -> Self;          // observes AppState global → reload settings
    pub fn enabled(&self) -> bool;
    pub fn mode_for(&self, view: EntityId) -> Option<Mode>;
    pub fn indicator_text(&self, view: EntityId) -> Option<String>; // "-- NORMAL --" or "-- NORMAL --  2f"
    /// Returns true when the key was consumed.
    pub(crate) fn handle_key(&mut self, ev: &KeyDownEvent, view: &Entity<CodeView>,
                             window: &mut Window, cx: &mut Context<Self>) -> bool;
}
pub(crate) fn to_vim_key(ev: &KeyDownEvent) -> Option<lucerna_vim::Key>;
```

Rules:
- **When `handle_key` is called.** `Workbench::dispatch_key` calls it only when all of these hold:
  - `vim.enabled()`;
  - the `editorFocus`, `!textInputFocus` and `!inQuickOpen` context flags hold;
  - `pending_chord` is empty;
  - the key has no `cmd`;
  - the active body is a `CodeView` whose `doc_state()` is `Ready` or `Deleted`.

  If it returns true, call `cx.stop_propagation()` and return.
- **`to_vim_key`:**
  - `escape` → `Escape`; `enter` → `Enter`.
  - The arrows map to `h`/`j`/`k`/`l`.
  - ctrl with an ASCII letter `key` → `Ctrl(letter)`.
  - Otherwise, with no ctrl or alt and `key_char` holding exactly one char → `Char(c)`.
  - Everything else → `None`, which is treated as unhandled.
- **Settings:** `enabled = get("vim.enabled").or(get("vim_mode")).unwrap_or(false)`.
  - On a switch to disabled: for every known view, clear the overlay and the block selection, drop `views`, and `cx.notify()` so the indicator hides.
- **Before `feed`:**
  - If the view's `selection_generation()` differs from the stored value, call `vim.reset(view.cursor())`, unless a vim search is open.
  - Run `feed` inside `tasks::catch_panic`. On a panic, log it, reset the view, and consume the key.
- **Applying the outcome:**
  1. `ScrollTopRow` → `set_scroll_top_row`.
  2. `CursorMoved { jump }`: when the mode is Visual, set the selection from `exclusive_range` or `block_ranges`. Otherwise clear the selection and call `set_cursor`, or `reveal_centered` when `jump` and the target is off-screen.
  3. Set the overlay to `vim.cursor()` with `Block` shape.
  4. `Yank(text)` → `cx.write_to_clipboard`.
  5. `Highlight(q)` → compute or get the matches and call `set_match_highlights(matches, None)`.
  6. `Fold` → `fold_at`. When the cursor row is now hidden, `vim.set_cursor((header, 0))`.
  7. `ModeChanged` → `cx.notify()`.
  8. Store the new `selection_generation()`.
- **`Retry(query)`:**
  - For snapshots of at most `BACKGROUND_SEARCH_BYTES`, compute synchronously with `find_in_snapshot`, fill the cache, and re-feed the same key once.
  - For larger snapshots, run `cx.background_spawn` (as in `FindWidget::search`). On completion, fill the cache and re-feed, unless another key arrived first (tracked by a generation counter).
- **`ViewInfo` adapter:**
  - It wraps `&CodeView` together with the cache entry.
  - `matches()` returns the cached `Arc` when `(revision, query)` matches, else `None`. A regex error counts as an empty match list.
- **Cleanup:** on `CodeViewEvent::Reloaded`, subscribed per view when its `ViewState` is created:
  - `globals.remap_marks(path, &reload, snapshot)`;
  - `vim.clamp(snapshot)`;
  - drop that view's cache entry.

  Release the subscription and the entry when the view entity is released (`cx.observe_release`).
- **Context keys:** `vim.active` = enabled; `vim.mode` = `mode_for(active view)`, using `context_value()`, or `""`.

- [ ] **Step 1: Write failing tests in `tests/vim.rs`.** Use `cx.simulate_keystrokes`.
  - Note: check that the test platform fills `key_char` for single-char keystrokes such as `"$"` and `"shift-g"`. If it does not, add a `type_keys(cx, "...")` helper that builds `KeyDownEvent`s with `key_char` set, and use it everywhere.

  | Test | What it checks |
  |---|---|
  | `disabled_by_default_keys_pass_through` | `j` does not move the cursor (no overlay); `cmd-p` still opens Quick Open |
  | `j_moves_cursor_when_enabled` | settings `{"vim.enabled": true}`, text `"a\nb\nc"`, `jj` → `cursor() == (2,0)` |
  | `vim_mode_alias_enables` | `{"vim_mode": true}` |
  | `explicit_vim_enabled_false_beats_alias` | `{"vim.enabled": false, "vim_mode": true}` → disabled |
  | `keys_go_to_quick_open_input` | open Quick Open, type `jk`; the query text is `"jk"` |
  | `escape_still_closes_find_widget` | run `actions.find`, then `escape`; the widget is hidden |
  | `visual_yank_writes_clipboard` | `vjy` on `"abc\ndef"` → clipboard `"abc\nd"` |
  | `mode_indicator_text` | `-- NORMAL --`, `-- VISUAL LINE --` after `V`, `-- NORMAL --  2` after `<Esc>2` |
  | `context_key_vim_mode_in_user_keybinding` | user keybinding `{"key":"ctrl+k","command":"editor.action.toggleWordWrap","when":"vim.mode == 'Normal'"}` toggles wrap |
  | `mouse_click_resets_to_normal` | `v`, then a mouse click → `mode_for` is `Normal` |
  | `ime_key_without_char_is_ignored` | `KeyDownEvent { key: "e", key_char: None }` → no panic, not consumed |
  | `pending_state_is_per_view` | two groups, `g` in one, focus the other, `g` → no move to row 0 there |
  | `reload_in_visual_clamps_selection` | `Vjj` on 5 rows, reload to 1 row → no panic, `mode_for` stays `VisualLine`, cursor row 0 |
  | `disabling_vim_clears_overlay_and_selection` | `v`, set `vim.enabled` false → selection `None`, indicator `None`, `j` unconsumed |
  | `reopened_file_keeps_marks_new_state` | `ma` at row 2, close the tab, reopen, `gg'a` → row 2, mode Normal |
  | `fold_keys_fold_and_move_cursor` | `zc` inside a fn body → the cursor is on the header row |
  | `star_highlights_matches` | `*` → `match_highlights()` is non-empty |
  | `marks_follow_reload` | `ma` at row 5, reload with 2 rows inserted above, `` `a `` → row 7 |
- [ ] **Step 2:** Run `cargo test -p lucerna-workbench --test vim`. Expected: FAIL.
- [ ] **Step 3:** Implement `vim.rs` and the workbench wiring. Add the mode indicator: `vim_indicator.rs` with `ModeIndicator { vim: Entity<VimLayer>, groups: Entity<EditorGroupsView> }`. It renders `indicator_text(active view id)` with `debug_selector "vim-mode-indicator"`, the UI font, and `text_muted`; the pending part uses the mono font. Register it with `add_status_item(StatusBarSide::Left, …)` next to `config_error` in `Workbench::new`. It observes both entities.
- [ ] **Step 4:** Run `cargo test -p lucerna-workbench`. Expected: PASS, with no regressions.
- [ ] **Step 5:** Commit with message `feat(vim): workbench vim layer, settings, mode indicator`.

---

### Task 10: `/` and `?` through the find widget

Spec: §2.5 and §4.3 (`/` `?`).

**Pre-step:** run `git pull --rebase` and read the current `find_widget.rs`. The `lucerna-search-ui` session may have changed it. Keep this diff minimal: no restyling and no reordering.

**Files:** Modify `find_widget.rs`, `vim.rs` and `tests/vim.rs`.

**Interfaces:**

```rust
pub enum FindWidgetEvent { Submitted, Dismissed }      // impl EventEmitter<FindWidgetEvent> for FindWidget
impl FindWidget {
    pub(crate) fn query(&self) -> lucerna_vim::SearchQuery;   // pattern + 3 flags
}
pub(crate) fn open_for_vim(view: &Entity<CodeView>, window: &mut Window, cx: &mut App) -> Entity<FindWidget>;
pub(crate) fn close(view: &Entity<CodeView>, window: &mut Window, cx: &mut App); // existing fn made pub(crate)
```

Rules:
- `FindWidget` gains a `vim: bool` field.
- `open_for_vim`:
  - Sets `vim = true`.
  - Clears the input with `set_text("")`, so there is no selection prefill.
  - Focuses the input.
- `key_down` with Enter and `vim == true`: emit `Submitted` and stop propagation. Do not step.
- `close()`: if the widget's `vim` was true, set it to false and emit `Dismissed` before hiding.
- **`VimLayer` on `OpenSearch { backward }`:**
  - Store `VimSearch { view, backward, origin: vim.cursor(), _sub }`, where `_sub` subscribes to the widget's events.
  - **On `Submitted`:**
    1. Read `query()`.
    2. Set `vim = false`, then `close()` the widget. The `Dismissed` this emits is ignored because `search` is taken first.
    3. Call `vim.search_submitted(query, backward, origin, …)`, handling `Retry` as in Task 9.
    4. Apply the actions, and emit the highlight.
  - **On `Dismissed`:** `vim.set_cursor(origin)`, restore the overlay, and call `set_cursor(origin)` on the view.

- [ ] **Step 1: Write failing tests in `tests/vim.rs`:**
  - `slash_search_jumps_and_n_continues`: on `"foo\nbar foo\nfoo"`, type `/foo` and Enter; the cursor is at (1,4), the widget is hidden, and highlights are non-empty. Then `n` puts the cursor at (2,0).
  - `question_search_goes_backward`: from (2,0), `?foo` and Enter → (1,4); `n` → (0,0).
  - `escape_in_vim_search_restores_cursor`: `/bar` then Escape → the cursor is back at the origin, and the widget is hidden.
  - `count_slash`: `2/foo` and Enter from (0,0) → (2,0).
  - `plain_find_widget_unchanged`: `cmd-f`, type `foo`, Enter → the existing next-match behaviour; no vim event is observed.
- [ ] **Step 2:** Run `cargo test -p lucerna-workbench --test vim`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -p lucerna-workbench`. Expected: PASS, including the existing `tests/find_widget.rs`.
- [ ] **Step 5:** Commit with message `feat(vim): slash and question search via the find widget`. Stage only `find_widget.rs`, `vim.rs` and `tests/vim.rs`.

---

### Task 11: Docs, notices, verification and install

Spec: §9.3, §10, and §12 step 11.

**Files:** Modify:
- `THIRD_PARTY_NOTICES.md`: add a "VSCodeVim test cases" section with `Copyright (c) 2015 VSCodeVim`, the full MIT text, and the files the cases came from.
- `CLAUDE.md`: add `lucerna-vim` to the crate list.
- `README.md`: in the settings and features sections, document `vim.enabled` and the V1 key list in one short table. Mention Zed only for GPUI and extensions, per `CLAUDE.md`.
- The roadmap row V1 in the phase 1 spec §1.3: mark it as done.
- The status line of the vim spec: `Đã triển khai`.

Steps:
- [ ] **Step 1:** Edit the docs listed above.
- [ ] **Step 2:** Run each check and confirm its expected output:

  | Command | Expected |
  |---|---|
  | `cargo fmt --all -- --check` | no output |
  | `cargo clippy --workspace --all-targets -- -D warnings` | clean |
  | `cargo test --workspace` | all pass |
  | `cargo deny check` | `advisories ok, bans ok, licenses ok, sources ok` |

- [ ] **Step 3:** Check that `df -h /` shows more than 1.2 GB free, then run `scripts/bundle-app.sh --install`. Expected: it ends by reporting that the installed `Lucerna.app` is registered.
- [ ] **Step 4:** Commit with message `docs: vim navigation (V1) in README, notices, roadmap`.
- [ ] **Step 5:** Hand the user the manual checklist from spec §9.3, in Vietnamese:
  - the block cursor is visible on a light and a dark theme;
  - `Ctrl+d` is smooth on a large Rust file;
  - `/` with Enter works.
