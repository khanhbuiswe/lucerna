# Replace in project (Search tab)

Date: 2026-11-18. Status: approved.

## Purpose

People moving from VS Code are used to pressing ⇧⌘H to change a word across a whole project. Lucerna
already has the Search tab (⇧⌘F) and Find & Replace within one file (⌥⌘F), but cannot replace across
multiple files. To do that, they have to reopen VS Code. This spec adds a Replace row to the Search
tab, together with Replace All, replace per file and replace per line.

Lucerna users let agents (Claude Code, Codex) write code, and agents read files from disk. So the
replaced text must be on disk right after the replace. The agent may also be writing those very
files, so replace must never overwrite text the agent has just written.

Behavior, commands and keys follow VS Code; the UI follows Zed.

## Goals

- ⇧⌘H (`workbench.action.replaceInFiles`) opens the Search tab with the Replace row.
- The Replace box has a Preserve Case toggle (`AB`). The Replace string syntax is the same as the
  in-file find bar: `$1`, `$&`, `\n`, `\u`… when regex is on (reusing `lucerna_project::replace`).
- When the Replace box has text, the search results show the replacement before it is applied: the
  old text struck through, immediately followed by the new text.
- Replace All has a confirmation dialog worded as in VS Code.
- Replace per line and per file with a button shown on hover, or ⇧⌘1. Remove a line or a file from
  the results (Dismiss, ⌘⌫).
- Writing files follows option A, which the user chose:
  - a closed file is written straight to disk;
  - an open, saved file gets one undo step and is then saved;
  - a file with unsaved edits is replaced only in the editor.
- The UI never waits on I/O. Reading, re-searching and writing run on a background thread.
- Whole word in the Search tab follows `editor.wordSeparators`, like the in-file find bar, so the
  word that gets replaced is exactly the word shown in the results.

## Non-goals (this time)

- A single undo step for the whole project (VS Code has "Workspace Edit"). A closed file cannot be
  undone inside the app; users go back through git.
- Opening a diff preview when selecting a match (`search.useReplacePreview`).
- A multi-line Replace box (Ctrl+Enter inserts a newline); to get a newline, use `\n` in regex mode.
- Replace string history, or saving the Replace string in workspace state.
- Replacing each match separately within a line that has several matches. Lucerna's results are
  counted per line.
- Matches that cross a line boundary. The Search tab searches line by line, so it cannot show this
  kind of match.

## Must not happen

- Overwriting text that another program (an agent) wrote to the file after replace read that file.
- Silently saving the part the user is still editing (VS Code does this; Lucerna does not).
- Replacing text that is not shown in the results. For example: a whole-word rule that differs
  between search time and replace time, or a match that crosses lines.
- Corrupting a file: losing the BOM, changing the encoding, or writing a lossily decoded file
  (unknown bytes already turned into U+FFFD).
- The UI hanging on Replace All across thousands of files.
- Replacing in the wrong place because a range in the results is stale.

## References

Read from the main branch of `microsoft/vscode` (2026-11-18) and Zed at the pinned GPUI revision
(`b5cd310`). Zed is GPL, so only its design is studied.

### VS Code

The code lives in `src/vs/workbench/contrib/search/browser/`: `searchActionsRemoveReplace.ts`,
`searchActionsNav.ts`, `searchWidget.ts`, `searchView.ts`, `replaceService.ts`,
`searchResultsView.ts` and `searchTreeModel/*`. The string replacement part is in
`services/search/common/replace.ts`.

- **Commands and keys (macOS; Windows/Linux in parentheses):**

  | Command | Key | When |
  |---|---|---|
  | `workbench.action.replaceInFiles` "Replace in Files" | ⇧⌘H (Ctrl+Shift+H) | always; in the Edit menu |
  | `search.action.replace` "Replace" | ⇧⌘1 (Ctrl+Shift+1) | `replaceActive && matchFocus && isEditableItem` |
  | `search.action.replaceAllInFile` "Replace All" | ⇧⌘1, ⇧⌘Enter | `replaceActive && fileMatchFocus` |
  | `search.action.replaceAll` "Replace All" | ⌥⌘Enter (Ctrl+Alt+Enter) | `searchViewletVisible && replaceActive && findWidgetNotVisible` |
  | `search.action.remove` "Dismiss" | ⌘⌫ (Delete) | `fileMatchOrMatchFocus` |
  | `toggleSearchPreserveCase` | ⌥⌘P (Alt+P) | `searchViewletFocus` |
  | `closeReplaceInFilesWidget` | Esc | `replaceInputBoxFocus` |

  - There is no command to toggle the Replace row; only a "Toggle Replace" chevron button on the
    left edge.
  - ⇧⌘H opens the view, turns on the Replace row, fills the query from the selection, then focuses.
  - `replaceActive` is true only when the Replace row is showing **and** there are results.
  - In the Replace box: Enter re-runs the search ("press Enter to preview"); Ctrl+Enter inserts a
    newline; Tab goes to the query box's toggles.
- **Replace All:**
  - With no results, it does nothing.
  - With results it **always** asks again; no setting removes this step. Title "Replace All",
    button "Replace", Cancel is the default. The question has 8 variants (singular/plural):
    `Replace {n} occurrence(s) across {m} file(s) with '{value}'?`, or without the `with` part when
    the string is empty. A string longer than 10 lines is cut to 10 lines with `...` added.
  - After replacing, all results are cleared and it shows
    `Replaced {n} occurrence(s) across {m} file(s) with '{value}'.`
- **Writing files** (`ReplaceService.replace`):
  - Each match is one edit. All edits are in one bulk edit, so every file is opened as a model and
    the whole project has **one undo step** ("Workspace Edit").
  - Afterwards VS Code **saves every replaced file**, including files that had unsaved edits.
  - Stale results: an open file is searched again 250 ms after each change. A closed file uses the
    range from search time with no checks, so if the file changed on disk the edit lands in the
    wrong place.
  - Only matches that are in the results are replaced (bounded by `search.maxResults`).
- **Replacing from the results tree:**
  - Inline buttons: Replace (codicon `replace`) on a match, "Replace All" on a file or folder (shown
    only when `replaceActive`), and Dismiss (codicon `close`).
  - Replacing one match saves that file, then re-searches the matches on that line. Replacing one
    file removes the file from the tree. Focus moves to the next element of the same kind.
- **Preview:**
  - In the tree, the old text is struck through on a `diffEditor.removedTextBackground`
    background, and the new text sits on a `diffEditor.insertedTextBackground` background. The new
    text is expanded per match (regex groups have real values).
  - `search.useReplacePreview` (on by default) opens the diff editor
    `{file} ↔ {file} (Replace Preview)` when a match is selected.
- **Widget:** a chevron on the left edge, rows indented 18 px. The Replace box has the placeholder
  "Replace", the Preserve Case toggle sits inside the box, and an icon button `replace-all` with the
  tooltip "Replace All (Submit Search to Enable)" when disabled.
- **Replacement string:** its own parser in the style of JS `String.replace`, with added case ops.
  Each match is expanded by re-running the regex on the matched segment itself.

### Zed

The code is in `crates/search/src/project_search.rs`, `search_bar.rs` and
`crates/project/src/project_search.rs`.

- **`ProjectSearchBar` layout:** query row, error line, replace row (when on), filter row, filter
  error line.
  - The toggle button is the `Replace` icon, tooltip "Toggle Replace", lit when on. It comes after
    the Filter button in the query row's button group. Turning it on focuses the replace box;
    turning it off focuses back to the query box.
  - The replace row has a spacer to line up with the collapse button. Next is the replace box
    (placeholder "Replace in project…", as wide as the query box), then two icon buttons
    `ReplaceNext` "Replace Next Match" (disabled while searching) and `ReplaceAll` "Replace All
    Matches". Tooltips show the key.
- **Keys:** ⇧⌘H outside the tab is `pane::DeploySearch { replace_enabled: true }`; inside the tab it
  is `search::ToggleReplace`. In the replace box: Enter is ReplaceNext, ⌘Enter is ReplaceAll.
- **Behavior:**
  - No confirmation dialog, no per-file or per-match button, no preview.
  - Replace edits the buffers in the results multibuffer. Replace All is one transaction across all
    files, one undo step. **No save**: the buffers are dirty, and saving the Search tab saves all of
    them.
  - If a search is running, replace waits for the results to finish. If the query differs from the
    last search, it searches again before replacing. Matches are anchors, so they stay correct after
    the buffer is edited.
  - Limits of 5,000 files and 10,000 matches ("Search Limits Reached"). Replace All only replaces
    what was found.
  - Replace runs synchronously on the main thread.

### Helix

There is no project-wide replace. `global_search` (`Space /`) is just a picker. Users use an
external tool (`sed`, `sd`, scooter), or open each file and run `%` `s` `c`.

### What Lucerna chooses

| Part | Follows | Why |
|---|---|---|
| Commands, keys, context keys, dialog and message wording | VS Code | Behavior. |
| Replace All confirmation dialog | VS Code | Behavior. Replacing on disk cannot be undone across files, so asking again is all the more needed. |
| Strikethrough preview in the results | VS Code | Zed has none. Users reviewing an agent's code need to see the text that will be replaced beforehand. A deviation from the Zed UI, with a reason. |
| Per-line and per-file buttons, Dismiss | VS Code | Behavior. The button on the file header uses the spot of the "Open File" button that Zed already has. |
| Replace row layout, icon-based Toggle Replace button | Zed | UI. VS Code puts a chevron on the left edge. |
| Only a Replace All button, no Replace Next | VS Code | VS Code has no Replace Next; Enter in the Replace box re-searches. |
| Write to disk and save | VS Code | User's choice (option A): agents read from disk. |
| Do not save a file with unsaved edits | Lucerna | A deviation from VS Code: saving the user's half-finished edits for them is a surprise. |
| No cross-file undo | Lucerna | Lucerna has no workspace-wide undo yet; later. |
| Re-search on the current content at replace time, check the hash before writing | Lucerna | VS Code writes to the old range when the file changed on disk; agents write files often, so Lucerna must not do that. |
| Run on a background thread | Zed and principle §3 | Both VS Code and Zed replace on the main thread. |

## Design

### 1. Search tab UI

Use the existing components in `ui.rs`: `search_bar_row`, `search_query_box`,
`search_query_column`, `input_box`, `toggle_button`, `bar_icon_button`, `bar_separator`.

```
[⌄⌃] [Search………………………  Aa ab .*] [⊟] [⇄] [↻] │ ‹ › 19 results
     [Replace……………………  AB         ] │ [replace-all]
     [files to include      ] [files to exclude      ]
───────────────────────────────────────────────────────────
 ▾ foo.rs  src/                              [⇄] [×]   ← hover
   12  let o̶l̶d̶N̶a̶m̶e̶newName = 1;                  [⇄] [×]   ← hover
```

- **Query row:** add a `bar_icon_button` Toggle Replace (icon `replace`, already present from the
  in-file find bar) right after the filter button. Tooltip `Toggle Replace`; the button is lit when
  on.
- **Replace row:** shown only when on, between the query row and the filter row.
  - On the left is a spacer lined up with the collapse button, like the filter row.
  - The Replace box uses `input_box` in `search_query_column`, as wide as the query box. Placeholder
    `Replace`. The `AB` toggle (`Preserve Case`) sits inside the box.
  - After the box come a `bar_separator` and a `bar_icon_button` with icon `replace-all`, tooltip
    `Replace All`. The button is disabled when there are no results, while a search is running, or
    while replacing.
- **The Replace string** is kept when the Replace row is hidden and shown again (it lives in
  `SearchView`).
- **Preview in the results:** when the Replace row is showing **and** the Replace box has text, each
  match range of a line is drawn as the old text struck through on a `diff.removed` background,
  immediately followed by the new text on a `diff.inserted` background.
  - Which theme color to use (`editor.diff.*` or `version_control.*`) is up to the implementer, as
    long as it already exists in the theme and is readable on both light and dark themes.
  - The new text is expanded for that exact line: `$1` shows its real value, with Preserve Case
    applied. How it is computed: run the regex on the line's `line_text` (already cut at 1,000
    characters), then take the replacement text for each range. A range that cannot be matched again
    (cut off, lookaround) is only struck through, with no new text.
  - The preview is enabled only for the Search tab's list. `ExcerptList` is shared with the diff
    view, so add a render option, off by default.
  - The new text is computed at render time, only for the rows being shown, and cached by (Replace
    string, toggle). It is not computed for every line on every keystroke.
- **Buttons on hover** (only when the Replace row is showing and the search has finished):
  - Match line: `[⇄]` (`replace`, tooltip `Replace`) and `[×]` (`x`, tooltip `Dismiss`). Context
    lines have no buttons.
  - File header: `[⇄]` (`replace-all`, tooltip `Replace All`) and `[×]` (tooltip `Dismiss`), placed
    before the Open File button.
  - Dismiss also shows when the Replace row is hidden, like VS Code.
- **The list's status line:** while replacing, the counter reads `Replacing…`. When replacing is
  done the list is empty and shows the result message in the center, where `centered_message` shows
  "No results found".

### 2. Commands, keys, context keys, menu

| Command | Label | Key (macOS) | When |
|---|---|---|---|
| `workbench.action.replaceInFiles` | `Search: Replace in Files` | ⇧⌘H | always |
| `search.action.replaceAll` | `Search: Replace All` | ⌥⌘Enter | `searchViewletFocus && replaceActive` |
| `search.action.replace` | `Search: Replace` | ⇧⌘1 | `searchViewletFocus && replaceActive && matchFocus` |
| `search.action.replaceAllInFile` | `Search: Replace All in File` | ⇧⌘1 | `searchViewletFocus && replaceActive && fileMatchFocus` |
| `search.action.replaceAllInFile` | | ⇧⌘Enter | `searchViewletFocus && replaceActive && fileMatchFocus` |
| `search.action.remove` | `Search: Dismiss` | ⌘⌫ | `searchViewletFocus && fileMatchOrMatchFocus` |
| `toggleSearchPreserveCase` | `Search: Toggle Preserve Case` | ⌥⌘P | `searchViewletFocus` |
| `closeReplaceInFilesWidget` | `Search: Close Replace Widget` | Escape | `searchViewletFocus && replaceInputBoxFocus` |

- **New context keys**, placed alongside `searchViewletFocus` in `workbench.rs`:
  - `replaceActive`: the Replace row is showing and there is at least one result (as in VS Code).
  - `replaceInputBoxFocus`: the Replace box has focus.
  - `matchFocus`: the list has focus and the selected row is a match line.
  - `fileMatchFocus`: the list has focus and a file header is selected.
- **Selectable file headers:** `ExcerptList` currently lets ↑/↓ skip headers. This spec makes ↑/↓
  stop at headers when the tab is the Search tab (the diff view stays as before), to enable
  `fileMatchFocus`, ⇧⌘Enter, Dismiss for a whole file, and selecting the next element after
  replacing. Enter on a header opens or collapses the file as clicking the header does. F4 and
  `‹ ›` still go through the match lines only.
  - `fileMatchOrMatchFocus`: either of the two.
- **The find bar's `togglePreserveCase` (⌥⌘P)** currently has `editorFocus && !searchViewletFocus`,
  so it does not conflict with the new keys.
- **⇧⌘1** is already used by `editor.action.replaceOne` with `editorFocus && findWidgetVisible`. The
  Search tab is inside `editorFocus`, so a keymap test must verify that in the Search tab the Search
  command wins (use `!searchViewletFocus` on the editor's row if needed, as done with the toggles).
  The same goes for ⌥⌘Enter.
- **Esc in the Replace box** hides the Replace row and focuses the query box. Esc elsewhere keeps the
  current behavior.
- **Enter in the Replace box** runs the search immediately (like Enter in the query box), so the
  preview is refreshed. In fact the preview already updates as you type; Enter only needs to skip
  the debounce.
- **Tab / Shift+Tab:** Tab in the query box goes to the Replace box (when the Replace row is
  showing), Shift+Tab in the Replace box goes back to the query box. Like the in-file find bar.
- **Edit menu:** add `Replace in Files` (`workbench.action.replaceInFiles`) right after
  `Find in Files`, if that item exists; if not, put it after `Replace`. Add only one item.
- **Append to the end of `default-keybindings.json`** as one block, without editing the old rows.

### 3. Behavior

- **⇧⌘H** (`workbench.action.replaceInFiles`):
  - Do what ⇧⌘F does (`find_in_files`: open or focus the Search tab, fill the query from a
    single-line selection), then turn on the Replace row.
  - Focus the Replace box if the query box has text, otherwise focus the query box.
- **⇧⌘H while in the Search tab** does the same, without toggling the state. To hide the Replace row,
  click the Toggle Replace button or press Esc in the Replace box.
- **Toggle Replace button:** toggles the Replace row. Turning it on focuses the Replace box, turning
  it off focuses the query box.
- **Replace All** (button, ⌥⌘Enter, `search.action.replaceAll`):
  1. Do nothing if there are no results, a search is running, another replace is running, or the
     query has changed without being searched again (`stale`). In the `stale` case only, run the
     search right away, without replacing.
  2. Ask with `window.prompt` (`PromptLevel::Warning`), buttons `Replace` and `Cancel` (Cancel is
     the default). The question follows VS Code (8 singular/plural variants, with or without
     `with '{value}'`, string cut at 10 lines). `n` and `m` are the number of matches and files
     currently shown in the results. If the results were cut at `MAX_RESULTS`, add a detail line:
     `The results only contain a subset of all matches. Files not in the results are not changed.`
  3. If the user agrees, run the replace job (section 4) on **exactly the match lines currently
     shown**, minus the lines and files that were Dismissed. Text that is not shown in the results
     is never replaced (see "Replace only what is shown" in section 4).
  4. When done: clear the results and show
     `Replaced {n} occurrence(s) across {m} file(s) with '{value}'.` (string not cut), where `n` and
     `m` are the numbers actually replaced. If any file was skipped, add a line
     `Skipped {k} file(s): {reason for each file}` (see the table in section 4), at most 5 files
     and `…`.
- **Replace per line** (button on the line, ⇧⌘1 when a line is selected):
  - Replaces the matches of that line. The replace job runs for one file, with only this line, under
    the same "Replace only what is shown" rule. If the line has changed, it is not replaced, and the
    status bar reports
    `The file has changed since the search. Search again to replace it.`
  - If the file is open with unsaved edits, the status bar reports
    `Replaced in {name}. The file has unsaved changes.`
  - After replacing, that file alone is searched again and its excerpts are updated: the line just
    replaced disappears if it has no match left. No dialog.
  - The selection moves to the next match line (or the previous line if it was the last), as in
    VS Code.
- **Replace per file** (button on the header, ⇧⌘1 or ⇧⌘Enter when a header is selected): replaces the
  file's currently shown match lines, under the same rules as Replace All, without asking again. A
  skipped file has its reason reported in the status bar. After that, the file is removed from the
  results and the first match line of the next file is selected.
- **Dismiss** (`search.action.remove`, the `×` button, ⌘⌫): removes the line or the file from the
  results, without editing any file. The counter and `replaceActive` are recomputed from the
  remaining results. On the next search everything shows again (VS Code remembers dismissed matches
  within one search; Lucerna does not need to remember across searches).
- **Preserve Case** (`toggleSearchPreserveCase`, the `AB` toggle): flips a flag on `SearchView`; the
  preview is redrawn. No need to search again.
- **Changing the Replace string does not re-run the search.** Only the preview is redrawn.
- **While replacing:** the input boxes can still be edited, but every other replace command is
  blocked until it finishes. A new search (typing a query) is still allowed: the replace result will
  not clear the results of that new search. The "Replaced…" message is shown only if no search
  started after the replace ran.

### 4. The replace job (`lucerna_project::replace_in_files`, no gpui)

`lucerna-project` adds a dependency on `lucerna-text` (reading, decoding, encoding and atomic writing
all live there).

**Input:**

- the search query (pattern, regex, case, whole word, `editor.wordSeparators`);
- the `ReplacePattern` and the Preserve Case flag;
- a list of files: the absolute path, with that file's currently shown match lines, each as
  `(line_number, line_text)` taken from the results (minus Dismissed lines);
- the `TempFiles` of the saves (`save::save_temps`), so temp files are cleaned up when the app exits;
- a cancel flag (`AtomicBool`).

**Replace only what is shown** (a Lucerna decision, to keep the rule "never replace text not shown in
the results"):

- For each file, the job compares each assigned line with the line of the same number in the current
  content (comparing the leading part as long as `line_text`, i.e. at most 1,000 characters).
- If **any one line** no longer matches (the file was edited after the search, a line shifted, the
  file got shorter), skip the **whole file** with the reason `changed since the search`. Replacing
  part of a file after it has changed is more error-prone than skipping; the user searches again and
  continues replacing.
- If everything matches, replace only the matches that sit on those lines and **start within the
  shown part** (the first 1,000 characters). A new match on another line, or one in the cut-off part
  of a line, is not replaced.
- The last file, if cut off midway by `MAX_RESULTS`, is replaced only on the lines already shown.
- So the number in the "Replaced…" message equals the number in the dialog, minus the skipped files.
- The existing functions `replace_ranges` / `replacement_at` of `lucerna_project::replace` accept a
  filtered range; use them instead of `replace_all` (which replaces every match).

**How matches are found when replacing** (shared by the preview, the replace job and search):

- The regex is built like search: case, multi-line for `^`/`$`, no `\b`.
- Drop empty matches.
- Drop matches containing `\n`, because line-based search cannot show them.
- Whole word is filtered by **one single rule** following `editor.wordSeparators`, the same rule as
  `lucerna_editor::find::is_whole_word`.
  - The character-level rule (the boundary based on the characters before and after, plus the
    characters at the edges of the match) is moved down into `lucerna-text`, so `lucerna-editor` (on
    `BufferSnapshot`) and `lucerna-project` (on `&str`) both call it. This rule relies on `classify`
    and `CharClass` in `lucerna-editor/src/words.rs` (also used for word motion), so those two move
    along with it; `lucerna-editor` re-exports so the old code does not change.
  - **The `editor.wordSeparators` value:** the in-file find bar reads it per the file's language.
    The Search tab uses one value for the whole search: the window-level setting, not per language,
    because the background search thread does not know each file's language. A per-language
    `wordSeparators` override is very rare; if there is one, the Search tab may differ from the find
    bar for that file. This is a Lucerna decision. The preview, the replace job and search always use
    the same value, taken when the search starts.
  - There is no second copy of the rule.
- **The tab's search changes accordingly.** `build_pattern` no longer wraps whole word in `\b`. The
  `grep` matcher searches without `\b`, and the sink filters each match by the rule above. A line
  whose matches were all filtered out is not a match: it is not counted or reported (if it falls in
  the context of another match it becomes a context line, or is dropped; the implementer picks the
  simplest way that still keeps the tests correct).
  - `SearchQuery` gains a field `word_separators: String`, taken from the window's
    `editor.wordSeparators` setting.
  - `search_view::model::first_match` and `build_regex` in `find_widget.rs` (used by the Search tab)
    also go through the same rule.
  - The existing `build_pattern` test (`\b(?:…)\b`) is rewritten.

**Per closed file** (run in parallel on background threads, with a bounded thread count):

1. Read the bytes, `check_regular` and `check_size`, `decode_file(bytes, false)`. Keep the hash and
   stamp from read time.
2. Skip and report the reason if: binary, lossy, or unreadable. A file larger than 16 MiB (search's
   whole-file read threshold, `WHOLE_READ_BYTES`) is skipped with the reason `too large to replace`;
   the file size is checked from metadata before reading, because the job runs many files at once
   and each file takes several times its size in memory. Open files are not subject to this limit
   (the text is already in memory).
3. Check the lines per "Replace only what is shown", find matches on those lines, then compute the
   edits with `replace_ranges`. With no edits, do not write.
4. Apply the edits to the text, then `Buffer::encode` with the file's encoding, BOM and line ending.
   If a new character cannot be encoded (`Unencodable`), skip the file.
5. Right before writing, read the stamp again. If it differs from read time, compare the hash; if the
   hash differs, **do not write** and report `changed on disk` (like save's `changed_on_disk`). Then
   `write_bytes_tracked`.
6. Return the file's result: the number of matches replaced, or the reason for skipping.

Note: decoding turns CRLF into LF, and writing back uses the dominant line ending. A file with mixed
CRLF and LF gets unified, the same as opening that file in Lucerna and saving (VS Code does the same).
This behavior is acceptable.

**Files open in `Documents`** (that is, with a document and loaded text): the job does not touch the
disk for these files. The workbench handles them:

1. On the UI thread, take `snapshot_of`, `revision_of`, `is_dirty`, `has_conflict`, and the lossy
   state.
2. Skip if: in conflict (`has_conflict`), lossy, or not finished loading (`Loading`, `Error`).
3. Compute the edits on the snapshot on a background thread, with the same function as the job.
4. Back on the UI thread. If the revision has not changed, `apply_edits` as one undo step
   (`push_stop` before and after). If the revision has changed, compute once more; if it changes
   again, skip with the reason `changed while replacing`.
5. If the file was **clean before replacing**, save through `Documents`' save path, with
   `participants: false` (a flag already in `SaveOptions`): format on save or trim whitespace must
   not run automatically on a file where the user only replaced text.
   - This save **never opens a dialog**. If at write time the file has changed on disk
     (`ChangedOnDisk`), the document enters the conflict state like an autosave that hits a
     conflict: the conflict bar shows, the buffer keeps the replaced text as unsaved, and the file
     is reported with the reason `changed on disk`. A Replace All across many open files must not
     pop up many dialogs.
   - How: add a `SaveReason` (or a flag in `SaveOptions`) for this save, behaving like autosave: no
     conflict prompt, and no re-creating a file that was just deleted (report `no longer exists`).
     The implementer picks the neatest way.
6. If the file **has unsaved edits**, replace only in the buffer, without saving. The file counts as
   replaced; the message line adds `{k} file(s) with unsaved changes were not saved.` The replacement
   is one undo step sitting on top of the unsaved edits; one undo returns the buffer to exactly the
   earlier dirty state.
7. The document's views update themselves through `Documents` like any other edit. The selection is
   carried through the edit like the find bar's Replace All.

An untitled document never appears in search results (search only walks the disk), so it needs no
handling.

**Skip reasons** (text shown in the app):

| Reason | Text |
|---|---|
| Changed on disk between read and write | `changed on disk` |
| Document is in conflict with disk | `has a conflict with the file on disk` |
| Lossy decode | `was not decoded cleanly` |
| Binary, no permission, I/O error | `could not be read` or `could not be written: {error}` |
| Closed file larger than 16 MiB | `too large to replace` |
| Character that cannot be encoded | `cannot encode the replacement in {encoding}` |
| Document changed twice during replacing | `changed while replacing` |
| Assigned line no longer matches the file | `changed since the search` |
| File no longer exists | `no longer exists` |

**Progress and cancel:**

- The job sends progress (files done out of the total) over a channel. The tab's counter reads
  `Replacing… {done}/{total} files`.
- Closing the Search tab does not stop a running replace (finishing is still safer than stopping
  halfway). Closing the window cancels the job. Files already written stay as they are; a file being
  written midway is not corrupted thanks to atomic writes.
- There is no separate cancel button (VS Code has none either).

**Watcher:** a file written by the job emits a worktree event like any change on disk. That file's
document (if it was just opened during replacing) reloads as normal.

### 5. Performance

- The UI thread only does: asking for confirmation, gathering the file list, taking snapshots of the
  open documents, applying precomputed edits, and updating progress in batches (not per file).
- Applying edits to one large document (for example 500k matches) still costs time on the UI thread.
  That is the same cost as the find bar's Replace All, and a known follow-up. This spec does not
  solve it, but the documents are applied one after another across several update passes, not all
  merged into one frame.
- There is a benchmark or timing test: Replace All over 2,000 closed files with 10 matches each
  finishes, and the UI thread is not blocked by the job for more than one frame.

### 6. Tests

- **`lucerna-text`:** the character-level whole-word rule (moved from `lucerna-editor`, keeping the
  old tests).
- **`lucerna-editor`:** the existing `is_whole_word` tests still pass.
- **`lucerna-project`:**
  - Whole-word search by `wordSeparators` (for example `foo` does not match in `foo-bar` when `-` is
    a separator… and the reverse when it is not), including a line whose matches were all filtered
    out.
  - The replace job on a temp directory:
    - UTF-8, UTF-16 with BOM, Windows-1252, CRLF files: keep the encoding, BOM and line ending;
    - lossy or binary files are skipped;
    - a file edited between read and write (using a test hook) is not overwritten;
    - only the assigned lines are replaced; a new match on another line is not replaced;
    - a line that no longer matches makes the whole file skipped;
    - a match starting after 1,000 characters is not replaced;
    - matches that cross lines and empty matches are not replaced;
    - `$1`, Preserve Case;
    - cancel midway;
    - Vietnamese text in the match and in the replacement string.
- **`lucerna-workbench`** (gpui tests, in the style of the existing Search tab tests):
  - ⇧⌘H opens the tab with the Replace row, focuses the right box; the Toggle Replace button; Esc in
    the Replace box.
  - Preview: a line has the struck-through text and the new text (`$1` expanded); the diff view is
    not affected.
  - Replace All: the dialog has the exact wording (singular/plural, empty string, 10-line cut, the
    line reporting cut results); Cancel changes nothing; on agreeing, the files on disk change, the
    results are cleared and the message is right.
  - Open, clean file: changed, saved, one undo restores the old text.
  - Open file with unsaved edits: changed in the buffer, still dirty, disk unchanged.
  - File in conflict: unchanged, listed in the skipped list.
  - Replace per line, per file, Dismiss; ⇧⌘1, ⌘⌫, ⌥⌘Enter through the keymap; the selection moves to
    the next line.
  - Replace per line when the file has changed does not replace and shows the message.
  - The find bar's keys (⇧⌘1, ⌥⌘Enter, ⌥⌘P) in a normal editor still run the find bar's commands.
- **Docs:**
  - `docs/keyboard-shortcuts.md` and `CHANGELOG.md`: only add lines.
  - `docs/roadmap.md`: mark this item done.

## Left to the implementer

- The theme colors for the preview.
- A line whose matches were all filtered out by the whole-word rule: becomes a context line or is
  dropped.
- The internal structure of the job (thread pool, progress batch size), as long as the UI thread
  does not wait.
- Function names and data types inside the new module.

## Known limitations

- A match straddling the 1,000-character mark of a line (starting in the shown part and ending after
  it): the preview does not show new text for that match, but the job still replaces it. So the
  number in the confirmation dialog may differ from the number replaced in the result message.
- The check "the line still matches the results" (`changed since the search`) compares by line
  number and line text, so it cannot tell identical lines apart: if identical lines in the file were
  moved after the search, the file may still pass the check and be replaced at the line carrying the
  old number.
- Cancel only happens between two files, and only when the window is closed (there is no cancel
  button; closing the Search tab does not cancel).
- A non-UTF-8 file larger than 16 MiB: the search result shows lossy text (U+FFFD) because search
  reads it as a stream, and Replace skips that file with the reason `too large to replace`.
