# Lucerna — V1: Vim navigation

- **Date:** 2026-11-10
- **Status:** Implemented
- **Roadmap:** item V1 in §1.3 of `2026-11-08-lucerna-phase1-design.md`
- **Background research:** `docs/research/2026-11-09-vim-mode.md`. Section 5 of that note was checked against the Zed source `crates/vim` at `b5cd310` and VSCodeVim at `28f0275`.

## 1. Goals

### 1.1 Outcome

Someone used to Vim reads code in Lucerna entirely from the keyboard: moving, searching, scrolling, setting marks, selecting and copying. Lucerna stays a read-only viewer. (Note: a minimal Insert already exists since Phase 4.1; V2 builds on the contract in §13 of the Phase 4 spec.) V1 has no insert mode and no editing operators yet.

### 1.2 Success criteria

1. Setting `"vim.enabled": true` takes effect immediately, with no restart. The status bar shows `-- NORMAL --` and the cursor is a block.
2. Every command in §4 behaves as described. The cases adapted from VSCodeVim (§9.1) all pass.
3. Typing into Quick Open, the find widget, the Search box and the Extensions page never has keys swallowed by vim.
4. When vim is off (the default), all Lucerna behavior stays as it was.
5. The Phase 1 performance goals (§6 of the Phase 1 spec) are still met. A vim key is handled in under 1 ms on a 100k-line file, except the first `n`/`N`/`*`/`#` on a large file (see §8).

### 1.3 What was decided during brainstorming

| Question | Decision |
|---|---|
| Architecture | A pure crate `lucerna-vim`, no gpui, plus a thin glue layer in the workbench (option A) |
| Reading keys | Vim reads the **typed character** (`key_char`), the way VSCodeVim overrides the `type` command, instead of declaring each key in the keymap |
| Input box for `/` and `?` | Open the **find widget**, like Zed. Only a small hook is added to `find_widget.rs` |
| Behavior | Follows VSCodeVim, which also follows Vim. Deviations are noted in §4 |

### 1.4 Not in V1

- Insert mode, replace mode, the operators `d`/`c`/`y` with a motion, text objects, `.`, macros, registers, `u`/`Ctrl+r` and the `:` command line. These belong to V2, after Phase 4.
- Remapping vim keys (`vim.normalModeKeyBindings`…).
- The jumplist `Ctrl+o`/`Ctrl+i`.
- Uppercase marks (global marks) and saving marks across sessions.
- `gj`/`gk` to move by display line when word wrap is on.
- Status bar color by mode (`vim.statusBarColorControl`).

## 2. Architecture

### 2.1 New crate: `lucerna-vim`

This crate does not use gpui and depends only on `lucerna-text`. License MIT OR Apache-2.0.

```
crates/lucerna-vim/src/
  lib.rs       // pub use; Key, Mode, Action, ViewInfo
  state.rs     // Vim: state machine, pending keys, count, goal column
  motion.rs    // pure motions on BufferSnapshot
  classify.rs  // character classes: word / punctuation / whitespace
  search.rs    // * #, n N over the match list
  marks.rs     // Marks: a–z, the ' mark, remap on reload
  visual.rs    // VimSelection, text on yank
  tests/…      // string-based tests with a cursor marker
```

### 2.2 Interface

```rust
pub enum Key { Char(char), Ctrl(char), Escape, Enter }

pub enum Mode { Normal, Visual, VisualLine, VisualBlock }

/// What the vim layer may read from the view. Vim keeps the cursor and the
/// visual anchor itself; the view's selection is only an output.
pub trait ViewInfo {
    fn path(&self) -> &Arc<Path>;              // marks are kept per file
    fn snapshot(&self) -> &BufferSnapshot;
    fn top_row(&self) -> u32;                  // first visible buffer row
    fn visible_rows(&self) -> u32;             // 0 before the first paint
    fn is_row_hidden(&self, row: u32) -> bool; // inside a closed fold
    fn tab_size(&self) -> u32;
    /// The partner bracket of the one at `offset`, syntax-aware when possible.
    fn matching_bracket(&self, offset: usize) -> Option<usize>;
    /// Every match of `query`, sorted byte ranges; `None` while a background
    /// search for a large file is still running. The workbench implements it
    /// with `find_in_snapshot` and a cache keyed by (revision, query).
    fn matches(&self, query: &SearchQuery) -> Option<Arc<[Range<usize>]>>;
}

pub struct SearchQuery { pub pattern: String, pub is_regex: bool,
                         pub case_sensitive: bool, pub whole_word: bool }

pub struct Settings {
    pub ignorecase: bool,      // vim.ignorecase, default true
    pub wrapscan: bool,        // vim.wrapscan, default true
    pub startofline: bool,     // vim.startofline, default true
    pub scroll: u32,           // vim.scroll, default 0 (half page)
    pub scrolloff: u32,        // editor.cursorSurroundingLines, default 0
}

/// Side effects; the cursor and the visual selection are read back from
/// `Vim::cursor()` and `Vim::selection()` after every key.
pub enum Action {
    CursorMoved { jump: bool },   // jump: centre if off-screen
    ScrollTopRow(u32),
    OpenSearch { backward: bool },
    Highlight(SearchQuery),
    Yank(String),
    Fold { row: u32, action: FoldAction },  // Toggle / Close / Open
    ModeChanged(Mode),
}

/// `Retry`: the matches for `query` are not ready; the state is unchanged
/// and the workbench feeds the same key again once they are.
pub enum Outcome { Handled(Vec<Action>), Unhandled, Retry(SearchQuery) }

impl Vim {
    pub fn feed(&mut self, key: Key, view: &dyn ViewInfo,
                globals: &mut VimGlobals, settings: &Settings) -> Outcome;
    pub fn mode(&self) -> Mode;
    pub fn cursor(&self) -> Point;
    pub fn selection(&self, snapshot: &BufferSnapshot) -> Option<VimSelection>;
    /// The pending keys for the indicator: "5", "g", "2f", "m"...
    pub fn pending(&self) -> &str;
    /// Called when the search started by `/` or `?` is submitted.
    pub fn search_submitted(&mut self, query: SearchQuery, backward: bool, origin: Point,
                            view: &dyn ViewInfo, globals: &mut VimGlobals,
                            settings: &Settings) -> Outcome;
    /// A mouse click or a selection made outside vim: back to Normal.
    pub fn reset(&mut self, cursor: Point);
}
```

- A `Vim` lives per CodeView.
- `VimGlobals` lives per workbench and holds:
  - `last_find`: the character, the direction, and whether it was `f` or `t`;
  - `last_search`: the pattern, direction and flags;
  - `scroll`: the number of lines `Ctrl+d` currently uses;
  - the marks of each file, keyed by `Arc<Path>`.

### 2.3 Glue layer in the workbench

New file `crates/lucerna-workbench/src/vim.rs`:

- **`VimLayer`**
  - Holds a `HashMap<EntityId, Vim>` for each CodeView. A view's entry is removed when that view is destroyed.
  - Holds `VimGlobals`, plus the `Settings` read from `AppState`. Settings are re-read every time `AppState` changes.
- **Call site:** `Workbench::dispatch_key` calls `VimLayer::handle_key` **before** the keymap. The vim layer receives a key only when all of these hold:
  1. vim is on, see §6;
  2. `editorFocus && !textInputFocus && !inQuickOpen`;
  3. the key has no `cmd`, and the keymap is not in the middle of a chord such as `cmd+k …`;
  4. the open tab is a `CodeView` in state `DocState::Ready` or `Deleted`.
- **Keys are converted to `Key`:**
  - Has ctrl, and `key` is a letter → `Ctrl(c)`.
  - `escape` → `Escape`; `enter` → `Enter`.
  - Has a `key_char` of exactly one character, and neither ctrl nor alt is held → `Char(c)`. On macOS, `alt` produces a different character, so keys with alt are skipped and go down to the keymap. For example `alt+z` toggles word wrap.
  - The arrow keys are converted to `h`/`j`/`k`/`l`.
  - Every other key returns `Unhandled`.
- **Result:**
  - `Handled(actions)`: run the actions on the CodeView, then call `stop_propagation`.
  - `Unhandled`: the key continues into the keymap as before.
- **Keys with no meaning in normal mode yet.** A printable character that is not a vim command, for example `i`, `x`, `d`, is still treated as `Handled`, does nothing, and clears the pending keys. This way no character slips down to the keymap and runs a different command by mistake. Real Vim does the same.
- **Context keys for `when`**, named after VSCodeVim:
  - `vim.active`: true when vim is on.
  - `vim.mode`: `'Normal'`, `'Visual'`, `'VisualLine'` or `'VisualBlock'`. The value is the mode of the open view.
- **Sync with the mouse.** When a CodeView's selection is changed by the mouse or by `go_to` (Quick Open, search results, go-to-line), the vim layer calls `reset()` and the cursor is clamped to the last character of the line. CodeView reports this with a counter `selection_generation`, incremented every time the selection changes. The vim layer remembers the counter from its own last write, and calls `reset()` when it sees a different value.

### 2.4 Changes in CodeView

- **Drawing the cursor.** CodeView does not draw a cursor today.
  - Add `set_cursor_overlay(Option<CursorOverlay>)`, default `None` to keep the old behavior. In V1, `CursorShape` has only `Block`.
  - In normal mode, draw a block in the color `editor.foreground` with alpha about 0.4, exactly as wide as the character under the cursor: CJK characters and emoji are twice as wide, a tab is as wide as its column count. On an empty line, the block is half a character wide.
  - In visual mode, the block is drawn over the selection.
  - The color comes from the theme: `editor.foreground`, or `terminal.ansiBrightBlack` if the theme has it.
- **Visual-style selection.**
  - `VimSelection::Char` and `Line` use the existing `Selection`. The inclusive boundary is converted to exclusive in the glue layer. For `Line`, the selection runs from column 0 of the first line to the end of the last line, including the newline character.
  - `VimSelection::Block { rows, start_col, end_col }` computes columns as **display columns**, accounting for tabs. When drawing, each line in the block is converted to a byte range. Add a field `block_selection: Option<Vec<Range<Point>>>` to CodeView and draw it with the `selection` color. `copy_selection` joins these ranges with `\n`.
- **Scroll API**, serving `zz`/`zt`/`zb`, `Ctrl+d`/`Ctrl+u`, `Ctrl+f`/`Ctrl+b` and keeping the cursor in the viewport:
  - `set_cursor(p)`: set the cursor without centering as `go_to` does. If the cursor leaves the viewport, scroll just enough to bring it back, taking `editor.cursorSurroundingLines` into account. This function also opens a fold that hides the target line.
  - `scroll_top_row` / `set_scroll_top_row`: already exist.
  - If `visible_rows()` returns 0 before the first paint, screen-based motions such as H/M/L and `Ctrl+d` temporarily use 30.
- **Fold.** Add `fold_at(row, FoldAction)`. The function finds the smallest fold containing `row` in `fold_ranges`, then toggles, closes or opens that fold.

### 2.5 Changes in the find widget

The session `lucerna-search-ui` is reworking `find_widget.rs`, so the change here is kept as small as possible and was announced to that session beforehand.

- Add `FindWidgetEvent::Submitted` (on Enter) and `FindWidgetEvent::Dismissed` (on Esc), emitted through `EventEmitter`.
- `open` takes an extra parameter `vim_backward: Option<bool>`.
  - When it has a value, the widget opens with an empty input box and does not take the selection.
  - Enter does not jump to the next match inside the widget as in normal use, but emits `Submitted`, then closes the widget and returns focus to the CodeView.
- The vim layer reads the widget's query, the case/word/regex flags and `matches()`.

## 3. Mode model and pending keys

| Pending state | Starts with | Next key |
|---|---|---|
| count | `1`–`9`, and `0` once a count exists | more digits; capped at 99999 |
| `g` | `g` | `g`, `e`, `E`; any other key discards |
| `z` | `z` | `z` `t` `b` `a` `c` `o`; any other key discards |
| find | `f` `F` `t` `T` | any one character, including Vietnamese typed through an IME |
| mark | `m` | `a`–`z`; any other key discards |
| jump | `'` or `` ` `` | `a`–`z`, `'` or `` ` ``; any other key discards |

- `Escape` while something is pending clears the pending part.
- `Escape` in normal mode, with nothing pending, returns `Unhandled`, so Esc still closes the find widget, Quick Open and other popups as before.
- `Escape` in visual mode returns to normal mode.
- **Mode transitions:**
  - `v`, `V`, `Ctrl+v` go from normal to the matching visual mode.
  - In a visual mode, pressing that mode's own key again returns to normal; pressing the key of another visual mode switches to that mode.
  - A newly opened tab always starts in normal mode.

## 4. Behavior of each command

Behavior follows VSCodeVim, which also follows Vim. Every deviation is noted. In all motions:
- the cursor is a `Point` with the column counted in bytes;
- every step follows character (`char`) boundaries and never cuts in the middle of a UTF-8 character;
- a motion with no target leaves the cursor in place, with no error and no beep.

### 4.1 Cursor and goal column

- In normal mode, the cursor sits on a character, never after the last character of the line. On an empty line, the cursor is at column 0.
- The **goal column** (`goal`) is measured in display columns, accounting for tabs.
  - `j`, `k`, `Ctrl+d`/`Ctrl+u`/`Ctrl+f`/`Ctrl+b`, `H`/`M`/`L`, `G`/`gg` when `startofline=false`, and `zz`/`zt`/`zb` keep `goal` unchanged.
  - Every horizontal motion resets `goal`.
  - `$` sets `goal = ∞`, so a later `j`/`k` still sticks to the end of the line.
- `j`/`k` skip over lines hidden by a fold, like Zed. VSCodeVim opens the fold by default when passing through; Lucerna follows Zed because there is no reason to open folds when only reading.
- With word wrap on, `j`/`k` still move by buffer line as in Vim. `gj`/`gk` are left to V2.

### 4.2 Motions

| Key | Behavior |
|---|---|
| `h` `l` | One character left/right, never crossing to another line. Count is supported |
| `j` `k` | Line below/above, going to `goal`, clamped to the last character |
| `w` `b` `e` `ge` | Characters fall into 3 classes: **word** (`char::is_alphanumeric()` or `_`, so accented Vietnamese letters are word too), **whitespace** and **punctuation** (everything else, including emoji). An empty line is a stop for `w` and `b`. A line with only whitespace is skipped. `e` skips empty lines. `w` on the last word of the file goes to the final character (only in visual mode can it pass the end of the line) |
| `W` `B` `E` `gE` | As above, but punctuation counts as word |
| `0` | Column 0. If a count already exists, `0` is a digit of the count |
| `^` | The first non-blank character. On a line of only whitespace, goes to the last character |
| `$` | End of line. `N$` goes down N−1 lines first. Sets `goal = ∞` |
| `gg` `G` | `gg` goes to line 1, `G` to the last line. With a count N, goes to line N, clamped to the last line. Per `vim.startofline` (default true) the cursor goes to the first non-blank character, otherwise it keeps `goal`. **Differs from Zed** (Zed keeps the column); follows VS Code/Vim. A jump |
| `{` `}` | Up/down to the next **truly empty** line, passing over every non-empty line (a line with only whitespace is not a boundary). If standing inside a block of empty lines, leave that block first. At the end of the file with none found, `}` goes to the last character of the last line and `{` goes to (0,0). A jump |
| `%` | Find the first character in `()[]{}` from the cursor onward on the current line, then go to its partner through `ViewInfo::matching_bracket`, so brackets inside strings or comments are skipped when a syntax tree exists. If none, stay in place. `N%` (1–100) goes to line `(N*lines + 99) / 100`. A jump |
| `H` `M` `L` | `H`: first line of the viewport + (count−1); `L`: last line − (count−1); `M`: the middle of the visible lines. The scroll margin is `editor.cursorSurroundingLines`, unless the viewport already touches the start or end of the file. Lines hidden by a fold do not count. **Has a count like Vim/Zed** (VSCodeVim ignores the count, a known VSCodeVim bug). Follows `startofline`. A jump |
| `f` `F` `t` `T` | Find the Nth character on the current line. `t`/`T` stop one character before/after it. Matching is exact, case-sensitive |
| `;` `,` | Repeat the last find, in the same or the opposite direction. With `t`/`T`, if the repeat would not move (because the target character is right next to the cursor), search once more, like `cpo` without `;` |

### 4.3 Search

- **`/` and `?`**
  - Open the find widget in vim mode with an empty input box, and save the current cursor position.
  - Enter: from the saved position, jump to the first match **after** (for `/`) or **before** (for `?`) the cursor, wrapping if `vim.wrapscan`. If there is no match, the cursor stays in place. Save `last_search` with the pattern, the widget's flags and the direction. This is a jump.
  - Esc: the cursor stays where it was.
  - A count before `/`, for example `3/foo`, jumps to the third match.
- **`n` / `N`:** the next match in the direction of `last_search` / the opposite direction. Has a count. With `vim.wrapscan=false`, stops at the last match. This is a jump.
  - Match highlights stay after the find widget closes, until the next search or the next reload. `Action::Highlight` calls the existing `set_match_highlights`.
- **`*` / `#`**
  - Take the word (word class) under the cursor. If the cursor is on whitespace or punctuation, take the next word on the line. If there is no word, do nothing.
  - Search for the whole word: `\b` plus the escaped word. Case-insensitive when `vim.ignorecase` is on; smartcase is ignored, following VSCodeVim.
  - `*` goes forward, `#` goes backward, starting from the start of the current word. Saved into `last_search`, so `n` after `#` also goes backward.
- **How searching works:** the crate calls `ViewInfo::matches`. The workbench implements this method with the existing `find_in_snapshot` and caches the result by `(revision, pattern, flags)`. The crate gets the next match by binary search on the cursor position. If `matches` returns `None` (searching on a background thread), `feed` records the pending key; the workbench calls `feed` again with the same key when the result is ready.

### 4.4 Scrolling

| Key | Behavior |
|---|---|
| `Ctrl+d` `Ctrl+u` | Scroll down/up `scroll` lines; the default is half the number of visible lines, and it can be set with `vim.scroll`. The cursor moves by the same number of lines, clamped within the file. `N Ctrl+d` sets `scroll = N` for later uses. When already at the top of the file, `Ctrl+u` still moves the cursor up |
| `Ctrl+f` `Ctrl+b` | Scroll one page minus 2 lines, like Vim. The cursor is clamped into the viewport |
| `zz` `zt` `zb` | Bring the cursor line to the middle/top/bottom of the viewport. The cursor and `goal` do not change |
| `za` `zc` `zo` | Toggle/close/open the smallest fold containing the cursor line. When closing a fold, the cursor goes to the first line of the fold |

Scrolled lines are counted in the buffer lines currently shown, that is, lines hidden by a fold are skipped.

### 4.5 Marks

- **`m{a-z}`:** save the cursor's `Point` for the open file. Marks live only for the session.
- **`` `x ``:** go to the exact marked position.
- **`'x`:** go to the first non-blank character of the marked line.
- An unset mark does nothing.
- **The `'` mark:** every jump (see §4.2–§4.3) records the position *before* jumping into the `'` mark, so `''` and ` `` ` return there.
- **When an agent edits the file and it reloads:** every mark of the file is remapped with `Reload::adjust_row`, and the column is clamped to the new line. The workbench already receives `Reload` in `CodeView::apply_reload`, and will pass it to `VimGlobals::remap_marks(path, &reload, line_count)`.
- A jump to a mark on a line outside the viewport scrolls to center, like `go_to`. Other motions scroll just enough (`set_cursor`).

### 4.6 Visual mode

- **`v` (Visual):** the selection is `anchor` → `head`, including the character under `head`. Every motion in §4.2 and §4.3 moves `head`. In visual mode, `$` includes the newline character.
- **`V` (VisualLine):** select whole lines from the line of `anchor` to the line of `head`.
- **`Ctrl+v` (VisualBlock):** a rectangular block from the `anchor` line to the `head` line, from the min to the max display column. Columns are display columns, so lines with tabs still line up. After `$`, the block runs to the end of each line. Lines shorter than the block's left column are skipped.
- **`o`:** swap `anchor` and `head`. This is a read operation, cheap and familiar.
- **`y`**
  - Write the text to the system clipboard. Lucerna has no registers yet, so it does not read `vim.useSystemClipboard`.
    - Visual: the text from start to end, including the last character.
    - VisualLine: the lines joined, with a trailing newline character.
    - VisualBlock: each line of the block, joined with `\n`.
  - Set the marks `<` and `>`. These marks are used only internally in V1.
  - Back in normal mode, the cursor is at the **start** of the selection. For VisualLine, the cursor is at column 0 of the first line; for VisualBlock, at the top-left corner.
- **`Esc`:** back to normal mode, the cursor at `head`, keeping `goal`.
- **`cmd+c`** in visual mode copies like `y` without leaving visual mode. The keymap handles this through `editor.action.clipboardCopyAction`, because the CodeView's selection is already correct.

## 5. UI

- **Mode indicator**
  - A new status item on the left of the status bar, shown only when vim is on and the open tab is code.
  - The text shown: `-- NORMAL --`, `-- VISUAL --`, `-- VISUAL LINE --` or `-- VISUAL BLOCK --`, in the UI font and the `text_muted` color. When keys are pending, the pending keys are also shown in a mono font, for example `-- NORMAL --  2f`.
  - This text is identical in both Zed and VSCodeVim.
- **Cursor:** see §2.4.
- **Turning vim on or off while running:** when turned off, every `Vim` is dropped, the visual selection disappears and the cursor is no longer drawn. When turned on again, every view starts in normal mode.

## 6. Settings

| Key | Type | Default | Source |
|---|---|---|---|
| `vim.enabled` | bool | false | Lucerna's own: VSCodeVim is turned on by installing the extension, so it has no such key |
| `vim_mode` | bool | — | An alternative name like Zed's. Read only when `vim.enabled` is not set in any settings layer |
| `vim.ignorecase` | bool | true | VSCodeVim. In V1 used only for `*`/`#`. `/` uses the find widget's case button |
| `vim.wrapscan` | bool | true | VSCodeVim |
| `vim.startofline` | bool | true | VSCodeVim |
| `vim.scroll` | number | 0 (half a page) | VSCodeVim |
| `editor.cursorSurroundingLines` | number | 0 | VS Code; used as the scroll margin |

- Settings take effect as soon as `AppState` changes.
- The keys above go into the settings schema or documentation in the same place as the existing keys.

## 7. Error handling and edge cases

| Situation | Behavior |
|---|---|
| Empty file | Every motion stays at (0,0); `y` writes an empty string |
| Last line has no newline character | `G`, `}`, `$` are still correct; VisualLine yank still adds `\n` at the end, like Vim |
| Line longer than 10000 characters | Horizontal motions read at most the first 10000 characters, like the display. The cursor does not go beyond that limit. `$` goes to the 10000th character |
| Vietnamese, emoji, CJK | Each step is one `char`. The cursor block is as wide as the character's display width |
| File reloads while the cursor is on a deleted line | The cursor and anchor are clamped to the nearest valid line and character. Visual mode is kept |
| Loading, Error or Extensions tab | The vim layer receives no keys |
| Invalid `/` regex | The find widget shows the error. Enter does nothing |
| Search finds no match | The cursor stays in place |
| Panic in `feed` | Caught with `catch_panic` like elsewhere on the UI thread; log it, clear the pending keys, treat as `Handled` |

## 8. Performance

- Motions read only the lines they need through `BufferSnapshot::line` and `chunks_in_range`, without rebuilding the whole text.
- `{`/`}` on a 100k-line file with no empty lines must scan the whole file. Each line needs only a length check, so it is still under 5 ms.
- The first `n`/`N`/`*`/`#` calls `find_in_snapshot`, scanning the whole file. The result is cached by `(revision, pattern, flags)`, so later `n` presses are only a binary search.
  - A snapshot larger than `BACKGROUND_SEARCH_BYTES` (1 MB) runs the search on a background thread, like the find widget. The `n` key is recorded and carried out when the result is ready. If another key arrives first, the pending `n` is dropped.
- Drawing the cursor adds only one quad per frame.

## 9. Testing

### 9.1 `lucerna-vim`: unit tests

- **String-based helper:** `assert_keys("one |two three", "w", "one two |three")`.
  - `|` marks the cursor. Visual mode adds `[` for the anchor.
  - There are parameters for the viewport size and folded lines, for the scroll and H/M/L tests.
- **Adapted from VSCodeVim**, about 45 cases. Each case has a comment naming the source file and the original test name:
  - `test/mode/normalModeTests/motions.test.ts`: w across punctuation and across the end of a line, `[count]w`, `$` with a count, `$jj`, `[count]gg` and `[count]G` with `startofline` on or off, `f` then `,`, `fa;;;,,,`, `tttt`, `TtTt`, `5/tw` with wrap, `?two` then `n`, `*` whole word, `*` with `ignorecase`, `*` on whitespace not crossing lines, `#` on whitespace, `#` on a word, `zz` keeping `goal`, `vjjv'<`.
  - `test/motion.test.ts`: w, b, e, W, E across empty lines and whitespace-only lines, w at the end of the file, `{`/`}` on the sample "paragraphs" text.
  - `test/mode/normalModeTests/matchingBracket.test.ts`: `%` before an opening bracket, `%` with nested brackets, `%` after `->`.
  - `test/register/repeatableMovement.test.ts`: `tc;`.
  - `test/mode/modeNormal.test.ts`: `2<C-d><C-d>`, `<C-u>` when the first line is visible.
  - `test/mode/normalModeTests/commands.test.ts`: `gegegegege` across several lines.
  - `test/marks.test.ts`: `` wma2w`a ``, and `` vjlo<Esc>gg`> `` (adapted to use the internal `>` mark).
  - `test/mode/modeVisualLine.test.ts`: `Vj<Esc>` keeps the column.
  - `test/search/search.test.ts`: `?x` then `nn`.
- **Lucerna's own cases:**
  - `w`/`e`/`b` on `"Xin chào thế giới"` and on emoji.
  - `f` with a Vietnamese character (`fế`).
  - `$jj` across a line with tabs.
  - A 20000-character line.
  - Empty file.
  - Last line without a newline character.
  - j/k across a fold.
  - A `0` count (`10j`).
  - Mark remap after `Reload`.
  - Block yank with tabs.
  - `Esc` while something is pending.
  - A key with no meaning yet does not slip down to the keymap.

### 9.2 `lucerna-workbench`: integration tests with `TestAppContext`

- `vim.enabled=false`: typing `j` does nothing, the key goes down as before.
- With vim on: `j` moves; `Esc` still closes the find widget; typing `j` in Quick Open, the find widget, the Search box and the Extensions page still goes into the input box; `cmd+p` still opens Quick Open.
- `/foo` then Enter: the cursor jumps, the widget closes, `n` goes to the next match, the highlight remains.
- `vjy` writes the right text to the test clipboard.
- The mode indicator shows `-- NORMAL --` then `-- VISUAL LINE --`, and includes the pending keys.
- Marks are still correct after the file reloads (sharing the test flow with `documents.rs`).
- A mouse click in visual mode returns to normal.
- Turning on `vim_mode` instead of `vim.enabled` also works.
- The context key `vim.mode` can be used in the `when` of a user keybinding.

### 9.3 Manual check after installing

Run `scripts/bundle-app.sh --install`, turn vim on, then read a large Rust file. Check:
- the block cursor is clearly visible on light and dark themes;
- `Ctrl+d` is smooth;
- `/` and Enter work.

## 10. License

- Do not copy code from Zed's `crates/vim` (GPL-3). Only learn the design, exactly as the research note says.
- The test cases adapted from VSCodeVim (MIT): add a "VSCodeVim test cases" entry to `THIRD_PARTY_NOTICES.md`, with `Copyright (c) 2015 VSCodeVim` and the full MIT license text. Each test file has a comment naming the source.
- The new crate depends only on `lucerna-text`. No external dependencies; regex lives in the workbench, behind `ViewInfo::matches`. `cargo deny check` must pass.

## 11. Risks

| Risk | Mitigation |
|---|---|
| Conflict with the `lucerna-search-ui` session in `find_widget.rs` | Only add an event and an open flag. Pull/rebase right before editing. That session was told in advance |
| `key_char` differs across keyboard layouts and IMEs | Use `key_char` for printable characters, correct for every layout. Keys with ctrl use `key`. There are tests for keys with shift (`$`, `{`) |
| The cursor has never been drawn, so it may be off on wrapped lines or tabs | Share the `x_for` that the selection already uses |
| Inclusive ↔ exclusive selection off by one character | Gather the conversion into one function in `visual.rs` with its own tests |

## 12. Planned implementation order

1. Crate `lucerna-vim`: crate skeleton, `Key`/`Mode`/`Action`/`ViewInfo`, test helper, `classify`.
2. Horizontal and vertical motions: `hjkl`, `0 ^ $`, `w b e ge W B E gE`, `goal`, count.
3. Remaining motions: `gg G`, `{ }`, `%`, `f t ; ,`, `H M L`.
4. Scrolling and folds: `Ctrl+d u f b`, `zz zt zb`, `za zc zo`.
5. Marks and jumps: `m`, `'`, `` ` ``, `''`, remap on reload.
6. Visual mode: `v V Ctrl+v`, `o`, `y`, `Esc`.
7. Search in the crate: `*`, `#`, `n`, `N`, `search_submitted`.
8. CodeView: cursor drawing, block-style selection drawing, `set_cursor`, `fold_at`, `selection_generation`.
9. Workbench: `VimLayer`, key conversion, context keys, settings, mode indicator.
10. Find widget: hook for `/` and `?`.
11. Integration tests, `THIRD_PARTY_NOTICES.md`, README and roadmap, `CLAUDE.md` (add `lucerna-vim` to the crate list).
