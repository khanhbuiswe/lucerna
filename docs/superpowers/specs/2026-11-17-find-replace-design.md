# Find & Replace in a file

Date: 2026-11-17. Status: approved.

## Purpose

People moving from VS Code are used to pressing ⌥⌘F to replace text in the open file. Lucerna only has
the find bar (⌘F) so far, so they have to go back to VS Code. This spec adds a Replace box to the
existing find bar, along with Replace (one match) and Replace All. Behavior, commands and keys follow
VS Code; the UI follows Zed.

The replacement logic (reading the Replace string, expanding `$1`, preserving case) lives outside the
UI and does not use gpui, so the next item on the roadmap, Replace in project (the Search tab), can
reuse it.

## Goals

- ⌥⌘F (`editor.action.startFindReplaceAction`) opens the find bar with a Replace row.
- Replace one match (`editor.action.replaceOne`) and Replace All (`editor.action.replaceAll`), each
  command is one undo step.
- The Replace string follows VS Code syntax in regex mode: `$0` `$&` `$1…$99` `$$` `\n` `\t` `\\`
  `\u` `\U` `\l` `\L`.
- A Preserve Case toggle (`AB`) like VS Code.
- Shortcuts for the toggles: ⌥⌘C, ⌥⌘W, ⌥⌘R, ⌥⌘P.

## Non-goals (this time)

- Find in Selection (⌥⌘L) and limiting Replace All to the selection.
- Replace string history (↑/↓ in the box), saving toggles per workspace.
- A multi-line Replace box (Ctrl+Enter inserts a newline). To get a newline, use `\n` in regex mode.
- The `editor.find.*` settings.
- Replace in project: a separate spec, reusing the module here.

## Must not happen

- A Replace All that takes several undo presses to get back.
- Replacing in the wrong place because the match list is stale (the file just changed, or the
  background search has not finished).
- The UI freezing during Replace All on a large file.
- Replacing text in a read-only view (a picker preview, an unreadable file).

## References

Read from the main branch of `microsoft/vscode`, `zed-industries/zed` and `helix-editor/helix` (2026-11-17).

### VS Code

- **Replace string** (`src/vs/editor/contrib/find/browser/replacePattern.ts`, `parseReplaceString`):
  - Only parsed when regex is on. With regex off, the string is literal text, including `\n` and `$1`.
  - `$$` → `$`. `$0` and `$&` → the whole match. `$1`…`$9` are groups.
  - Two digits are read greedily: `$NN` is group NN. If there is no group NN then group N followed by
    the digit `0` is used, so `$10` with only one group becomes group 1 + `"0"`.
  - If no group matches the index, `$` and the digits are kept as is (`$5` with 2 groups → `"$5"`).
    A group that exists but did not take part in the match → empty string.
  - A `$` at the end of the string, or a `$` followed by another character, is kept.
  - `\\` → `\`, `\n` → LF, `\t` → tab. An unknown escape like `\q` keeps `\q`; a trailing `\` is kept too.
  - `\u` `\l` change the case of one character, `\U` `\L` change the whole part; they act on the
    `$n`/`$&` piece right after them and may stack (`\u\L$1`). They produce no text themselves.
- **Preserve case** (`src/vs/base/common/search.ts`, `buildReplaceStringWithCasePreserved`), looking
  at the matched text:
  1. If both the match and the replacement contain `-` with the same number of segments, handle each
     segment and rejoin with `-`. Likewise for `_`. If both `-` and `_` qualify, skip this step.
  2. Match is all uppercase → the replacement is fully uppercased.
  3. Match is all lowercase → fully lowercased.
  4. The match's first character is uppercase → uppercase the replacement's first character, keep the rest.
  5. The first character is lowercase → lowercase the first character.
  6. Otherwise: keep as is.

  Preserve case also runs when regex is off.
- **Replace one match** (`findModel.ts` `replace()`): take the next match counting from the start of
  the selection.
  - If the selection coincides exactly with that match, replace it (one undo step), then select the
    match after the inserted text.
  - Otherwise just select that match. So the first press selects, the second replaces.
- **Replace All** (`replaceAll()`): find every match, with no limit on the count, then bundle them into
  one command and one undo step. The selection is kept across the edits. With 19999 or more matches,
  it replaces the whole file using `String.replace`.
- **Commands and keys (macOS):**

  | Command | Key | When |
  |---|---|---|
  | `editor.action.startFindReplaceAction` | ⌥⌘F | |
  | `editor.action.replaceOne` | ⇧⌘1 | `findWidgetVisible` |
  | `editor.action.replaceOne` | Enter | `replaceInputFocussed` |
  | `editor.action.replaceAll` | ⌥⌘Enter | `findWidgetVisible` |
  | `editor.action.replaceAll` | ⌘Enter | `replaceInputFocussed` |
  | `toggleFindCaseSensitive` | ⌥⌘C | `editorFocus` |
  | `toggleFindWholeWord` | ⌥⌘W | `editorFocus` |
  | `toggleFindRegex` | ⌥⌘R | `editorFocus` |
  | `togglePreserveCase` | ⌥⌘P | `editorFocus` |

  On Windows and Linux: Ctrl+H, Ctrl+Shift+1, Ctrl+Alt+Enter, Alt+C/W/R/P.
- **Widget** (`findWidget.ts`):
  - A chevron button on the left opens or collapses the Replace row.
  - ⌥⌘F focuses the Replace box if the Find box has focus or was just filled from the selection;
    otherwise it focuses the Find box.
  - ⌘F when the bar is open keeps the Replace row; when the bar is hidden it opens without the Replace row.
  - Read-only editor: Replace cannot be opened, the chevron button is disabled.
  - The Replace and Replace All buttons are disabled when the Find box is empty.
  - The Replace string is kept per editor across closing/opening, and is never filled from the selection.
  - Tab in the Replace box moves focus to the toggles in the Find box; Shift+Tab moves back to the Find box.

### Zed

- **Layout** (`crates/search/src/buffer_search.rs`):
  - Row 1: the query box with the CaseSensitive / WholeWord / Regex toggles inside the box. Next are
    icon buttons: Toggle Replace (`IconName::Replace`, shows its on state), Search Selection, Select
    All Matches. Then the `‹ › n/m` group, and the Close button on the right.
  - Row 2 (when replace is on): the replace box (placeholder "Replace with…") as wide as the query
    box, then two **icon** buttons: `ReplaceNext` ("Replace Next Match") and `ReplaceAll` ("Replace
    All Matches").
- **Keys:** in the replace box, Enter is `search::ReplaceNext` and ⌘Enter is `search::ReplaceAll`.
  ⇧⌘H is `search::ToggleReplace`. ⌥⌘C/W/X toggle the toggles; ⌥⌘X for regex differs from VS Code's ⌥⌘R.
- **Behavior:**
  - `replace_next` always replaces the active match, with no need for the selection to coincide with
    the match, then selects the next match.
  - Both replace one match and replace all are a single transaction.
  - String expansion: with regex off it is literal text. With regex on it only unescapes `\\` `\n`
    `\t`, then calls `fancy_regex`'s `Captures::expand`.
  - Zed has no case modifiers and no preserve case.

### Helix

Helix has no find/replace UI. The user selects the whole file (`%`), selects every regex match (`s`),
then types the new text into all cursors (`c`). Since the text is typed rather than expanded, there
are no group references.

### The `regex` crate

`Captures::expand` differs from VS Code in these ways:

- A group name is the longest string of `[_0-9A-Za-z]`, so `$1a` is a group named "1a" (empty), while
  VS Code reads it as group 1 + `a`.
- A nonexistent group → empty, while VS Code keeps `$5`.
- There is no `$&`, `\n` or `\u`.

### Lucerna's choice

| Part | Follows | Why |
|---|---|---|
| Replace string syntax, Preserve Case | VS Code | Behavior follows VS Code; Zed lacks case modifiers and Preserve Case. Write our own parser (do not copy VS Code code, cite the source in a comment), not using `expand`. |
| Replace one match | VS Code | Behavior. In Lucerna the find bar already selects the active match, so in practice the first press already replaces, matching Zed's feel. |
| One undo step per command | Both | |
| Commands, keys, key contexts | VS Code | ⌥⌘R for regex (not Zed's ⌥⌘X). |
| Two-row layout, toggle buttons after the query box, two icon buttons | Zed | UI. VS Code puts the chevron on the left edge. |
| The `AB` toggle in the Replace box | VS Code | Zed has no Preserve Case; placed inside the box like `Aa` `ab` `.*`. |

## Design

### 1. Module `lucerna_project::replace` (no gpui)

Placed next to `build_pattern` (this crate already uses `regex` and is where the project Search
lives). It contains:

- `ReplacePattern::parse(text: &str, is_regex: bool) -> ReplacePattern`. With regex off it is static
  text. With regex on it is a list of pieces (text, group N, the whole match, plus case ops), exactly
  following the VS Code rules above.
- `ReplacePattern::expand(&self, caps: &regex::Captures) -> String`, with the `$NN` → group N + digit
  fallback, and keeping `$N` when the group does not exist.
- `ReplacePattern::is_static()`: needs no captures, for the fast path.
- `preserve_case(matched: &str, replacement: &str) -> String`: the six rules above.
- `replace_all(text: &str, regex: &Regex, pattern: &ReplacePattern, preserve_case: bool)
  -> Vec<(Range<usize>, String)>`: each **non-empty** match gives one edit, in order, with byte
  coordinates from before the edit. Empty matches are dropped to agree with what the find bar
  highlights and counts (`find_in_snapshot` drops them too). This is Lucerna's decision.
- `replacement_at(text, regex, range, pattern, preserve_case) -> Option<String>`: the replacement for
  a known match. Returns `None` if the regex no longer matches exactly `range`.

The regex comes from the existing `build_regex` (the same case, whole word and multi-line flags as
find). Unit tests cover each VS Code rule in the References section, including a Vietnamese example
for Preserve Case (`Đường` → capitalize the first letter).

### 2. The find bar (`find_widget.rs`)

`FindWidget` adds: `replace_input: Entity<TextInput>` (placeholder `Replace`),
`replace_visible: bool` and `preserve_case: bool`. The Replace string lives in the widget, so it is
kept across closing/opening of that view.

Layout, using `search_toolbar`, `search_bar_row`, `search_query_box`, `toggle_button`,
`bar_icon_button` and `bar_separator` in `ui.rs`:

```
[Find…………………  Aa ab .*] [⇄] │ ‹ › 3/19          │ ×
[Replace……………  AB       ] │ [replace next] [replace all]
```

- **Row 1** stays as it is now, adding only a Toggle Replace icon button right after the query box
  (tooltip `Toggle Replace`, lit when on). A read-only view does not have this button.
- **Row 2** only shows when `replace_visible`:
  - The Replace box uses `input_box` through `search_query_column`, so it is as wide as the query box.
    The `AB` toggle (`Preserve Case`) sits inside the box.
  - Then a `bar_separator` and two `bar_icon_button`s: `Replace Next Match` and
    `Replace All Matches`. Both buttons are disabled (not clickable, dim color) when the Find box is
    empty, the regex is invalid or there is no match.
- **Icons:** add two unmodified Lucide files `replace.svg` and `replace-all.svg` to
  `assets/icons/` (ISC, `LICENSE-lucide` already exists).
  - `replace` is for Toggle Replace, `replace-all` for Replace All.
  - Replace Next needs another Lucide icon (suggestion: `arrow-right-to-line`); the implementer
    picks, as long as it differs from the toggle icon.
- **Vim:** when the bar is opened for `/` or `?`, the Replace row and the toggle button are hidden.

### 3. Behavior

- **⌥⌘F** (`editor.action.startFindReplaceAction`, label `Replace`):
  - If the view is read-only, act like ⌘F.
  - Otherwise, open the bar and set `replace_visible = true`. The Find box is filled from a
    one-line selection, like ⌘F.
  - Focus goes to the Replace box if the Find box already had focus beforehand or was just filled from
    the selection; otherwise focus the Find box.
- **⌘F:** when the bar is hidden, open it with `replace_visible = false`. When the bar is open, keep
  the Replace row as it is.
- **Toggle Replace button:** flips `replace_visible`. Turning on focuses the Replace box, turning off
  focuses the Find box.
- **Tab / Shift+Tab:** Tab in the Find box moves to the Replace box (when the Replace row is shown);
  Shift+Tab in the Replace box moves back to the Find box. VS Code goes through the toggles too;
  Lucerna only jumps between the two boxes, since the toggles already have shortcuts.
- **Replace one match** (`editor.action.replaceOne`, label `Replace`):
  - Does nothing if the view is read-only, the Replace row is hidden, or a background search is running.
  - Take the active match. If the selection coincides exactly with its range, compute the replacement
    with `replacement_at` on the current snapshot, then apply one `EditPlan` with `stop_before` and
    `stop_after`.
  - After that the active match is the first match starting from the end of the inserted text
    onward; if none is left, wrap to the first match. This match is selected and scrolled into view.
  - If the selection does not coincide (the user moved the cursor), just select the nearest match,
    like `next`.
- **Replace All** (`editor.action.replaceAll`, label `Replace All`):
  - Call `replace_all` on the current snapshot, then apply a single `EditPlan` with `stop_before`
    and `stop_after`.
  - The selection is carried through the edits, like VS Code keeping the selection. How is up to the
    implementer, as long as the cursor does not jump around.
  - A file larger than `BACKGROUND_SEARCH_BYTES` computes the edits on the background executor and
    applies them only if the view's `revision` has not changed. If it has changed, drop them and
    replace nothing.
  - No limit on the number of matches.
  - After replacing, the bar searches again (automatically when the revision changes) and will
    usually show `0/0`.
- **Toggles:** `toggleFindCaseSensitive`, `toggleFindWholeWord`, `toggleFindRegex` and
  `togglePreserveCase` change the toggle on the active view's widget. If the view has no widget yet,
  create the widget without showing it, so the next open uses the new state, like VS Code keeping
  state while the widget is hidden.
- **Read-only:** both replace commands do nothing (`transact` already blocks it too). The Toggle
  Replace button is not shown.

### 4. Commands, keys, key contexts, menu

- **New key context:** `replaceInputFocussed` (name follows VS Code), on when the Replace box has
  focus. Placed with `findWidgetVisible` in `workbench.rs`.
- **Append to the end of `default-keybindings.json`**, as one block:

  ```json
  { "key": "alt+cmd+f", "command": "editor.action.startFindReplaceAction", "when": "editorFocus" },
  { "key": "shift+cmd+1", "command": "editor.action.replaceOne", "when": "editorFocus && findWidgetVisible" },
  { "key": "alt+cmd+enter", "command": "editor.action.replaceAll", "when": "editorFocus && findWidgetVisible" },
  { "key": "enter", "command": "editor.action.replaceOne", "when": "editorFocus && replaceInputFocussed" },
  { "key": "cmd+enter", "command": "editor.action.replaceAll", "when": "editorFocus && replaceInputFocussed" },
  { "key": "alt+cmd+c", "command": "toggleFindCaseSensitive", "when": "editorFocus" },
  { "key": "alt+cmd+w", "command": "toggleFindWholeWord", "when": "editorFocus" },
  { "key": "alt+cmd+r", "command": "toggleFindRegex", "when": "editorFocus" },
  { "key": "alt+cmd+p", "command": "togglePreserveCase", "when": "editorFocus" }
  ```

  - ⌥⌘C/W/R are already used for `searchViewletFocus` and `filesExplorerFocus`. `editorFocus` does
    not overlap with those two contexts, so there is no conflict. The implementer re-checks with a
    keymap test.
  - Enter in the Replace box must win over the existing Enter handling in `FindWidget::key_down`,
    which is only for the Find box.
  - If `editorFocus` does not cover focus inside the find bar (needs checking with a test), use
    exactly the context that `closeFindWidget` relies on.
- **Edit menu:** add `Replace` (`editor.action.startFindReplaceAction`) right after `Find`. Add just
  one line, because the ⌘N session is also editing `app_menu.rs`.
- **Labels in the Command Palette:** `Replace`, `Replace One`, `Replace All`,
  `Toggle Case Sensitive`, `Toggle Whole Word`, `Toggle Use Regular Expression`,
  `Toggle Preserve Case`. VS Code uses the label `Replace` for `replaceOne` too; Lucerna changes it to
  `Replace One` so the palette does not have two `Replace` lines.
- **Docs:**
  - `docs/keyboard-shortcuts.md` and `CHANGELOG.md`: only add lines, do not edit old lines, because the
    ⌘N session also edits these two files.
  - `docs/roadmap.md`: mark this item done.
  - `docs/design-principles.md` §1.1: drop the example sentence "drop Replace because Lucerna is only
    for viewing code", since it is outdated.

### 5. Tests

- **`lucerna-project`:** unit tests for `ReplacePattern` (every rule in the References section),
  `preserve_case` and `replace_all` (empty matches, Unicode, multiple lines).
- **`lucerna-workbench`** (gpui tests, in the style of the existing find tests):
  - ⌥⌘F shows the Replace row; ⌘F when the bar is hidden does not.
  - Replace one match replaces exactly the active match, moves to the next match, and one undo
    restores it.
  - A selection that does not coincide with a match only selects.
  - Replace All replaces everything and one undo restores everything.
  - `$1` with regex on; with Preserve Case.
  - A read-only view does not change.
  - Enter and ⌘Enter in the Replace box.
  - The toggles via keys.
  - Replace All on a file larger than 1 MB.

## Still left to the implementer

- The Lucide icon for Replace Next.
- How to carry the selection through Replace All's edits.
- Function names and the internal structure of the `replace` module.
