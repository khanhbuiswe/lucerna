# Project Search as a tab (replaces the sidebar Search panel)

Date 2026-11-10. Status: implemented (commits 641094c..c6fdba9).

## 1. Goals

The user asked: "make the search topbar like zed instead of a search panel, UI follows zed first".

Clarified with the user:
- Zed has no search box in the title bar. What the user wants is **Zed's Project Search: a tab in the editor**, with a compact search bar at the top of the tab and results shown as code excerpts with context and syntax highlighting.
- **Remove the Search panel from the sidebar.** The sidebar keeps only the Explorer.
- **Keep the magnifier button in the status bar**, but it opens the Search tab (like Zed's `SearchButton`).
- Results: **excerpt + context + syntax highlighting**.
- Approved technical direction: **A** (section 3).

Success means: `⇧⌘F` opens the Search tab, typing shows results grouped by file with context and syntax colors, clicking a line jumps to the match; the sidebar no longer has Search; scrolling tens of thousands of results stays smooth.

Out of scope: replace, editing directly in the results, multiple Search tabs at once, the Text Finder popup (`⌥⌘F`), a search box in the title bar.

## 2. References

Details are in `docs/research/2026-11-10-project-search-ui.md`. Summary:
- **Zed:** `ProjectSearchView` is an `Item` (a tab). `ProjectSearchBar` is the toolbar at the top of the tab: query, toggles for case / word / regex / include ignored, and a filter button (`cmd-shift-j`) that shows include/exclude. Results are a multibuffer with context, and are editable. `escape` switches focus between the query and the results. The status bar has a magnifier button that opens search.
- **VS Code:** the Search view lives in the sidebar; Search Editor is the tab version, with the setting `search.searchEditor.defaultNumberOfContextLines` (default 1). Search runs as you type (`search.searchOnType`). Keys: `alt+cmd+c/w/r` for the toggles, `shift+cmd+j` for query details, `F4`/`shift+F4` for the next/previous result.
- **Lucerna chooses:** the look follows Zed (tab, compact search bar, colored excerpts). Commands, keys, settings and behavior (search as you type, debounce) follow VS Code. Deviations from both: excerpts are read-only (Lucerna is a viewer); only one Search tab.

## 3. Architecture

### 3.1 `lucerna-project::search`: return excerpts
- `SearchQuery` gains `context_lines: u32`.
- `SearchEvent::File` becomes `{ path, excerpts: Vec<Excerpt> }`, where:
  - `Excerpt { lines: Vec<LineMatch> }` holds consecutive lines (reusing the existing `LineMatch { line_number, line_text, ranges }`);
  - a line with empty `ranges` is a context line.
- Use `before_context`/`after_context` of `grep-searcher`. Two match clusters whose context overlaps or touches are merged into one excerpt.
- `MAX_RESULTS` (20,000) still counts **matching lines**, not context lines. `MAX_LINE_CHARS` applies to context lines too.
- `build_pattern`, cancellation via `AtomicBool`, per-file streaming: unchanged.

### 3.2 Tab model
- `TabItem::Search`: at most one across all groups, never a preview, like `TabItem::Extensions`.
- Stored in `GroupState::tabs` as the string `lucerna:search`. The query and options (case, word, regex, include, exclude, whether the filter is shown or hidden) are stored in the workspace state; reopening the folder restores the tab and searches again.
- Old workspace state with `sidebar_view = "search"` is read as Explorer.

### 3.3 `search_view/` (replaces `search_panel.rs`)
A new module in `lucerna-workbench`, split into smaller files:
- `model.rs` — pure, no gpui: the list of result files, collapse flags, building the **flat row list** (`Header(file)`, `Line(file, excerpt, line)`, `Separator(file, excerpt)`), finding the next/previous matching row for F4, the summary.
- `highlight.rs` — background highlight loading (3.4).
- `view.rs` — the `SearchView` entity: the `TextInput`s, toggles, 150 ms debounce, generation guard (R17), `uniform_list`, opening results.
- A single `SearchView` entity for the whole workbench (like the Extensions page); the Search tab renders it.

### 3.4 Syntax highlighting in the background
- When `uniform_list` renders a range of rows, the view knows which files are visible. Files that have no highlight yet and have not been requested are put in a queue.
- At most **3** background tasks at once. Each task: read the file (skip it if it exceeds `check_size` or is larger than 2 MB), build a `BufferSnapshot`, pick the language (`language_for_file`), `SyntaxState::new` + `parse(None)`, then call `highlights` for the excerpt's line ranges.
- Result: for each excerpt line, a list of `(Range<usize> within the line, capture)`. Spans are clipped to the line boundary and to `MAX_LINE_CHARS`.
- **Drift protection:** if the line text on disk differs from `LineMatch::line_text` (the file changed), that line gets no highlight.
- Results are tied to the search's generation; on a new search, the queue and the old results are dropped. A read/parse error or no language: the line shows plain text, with no error reported.
- If the file is open in `Documents`, it is still parsed separately (simpler; optimize later).

### 3.5 Sidebar and status bar
- Remove `SidebarView::Search` and `search_panel.rs`. The sidebar keeps only the Explorer; `cmd+b` and `cmd+shift+e` stay as they are.
- The magnifier button in the status bar runs `workbench.action.findInFiles` and lights up when the Search tab is active in the focused group.

## 4. UI (follows Zed)

- **Tab:** magnifier icon + query ("Search" if empty).
- **Search bar** at the top of the tab, 32 px high, padding on a 4 px grid:
  - the query box, a `TextInput` (IME), placeholder "Search";
  - inside the box, on the right: toggles `Aa`, `ab`, `.*` (using the existing `toggle_button`);
  - after the box: a divider, the filter button (icon), the refresh button, the collapse/expand-all button. The buttons use `icon_button`.
- **Filter row** (hidden by default): the Include and Exclude boxes side by side, with the same placeholders as the old panel.
- **Summary line** in a small font, color `text.muted`: `12 results in 2 files`, "Searching…", a notice that results were cut off at 20,000, or a regex error.
- **File header** (as tall as one editor line, background `element.background`/light): collapse chevron, file icon from the existing icon table, file name in `text` color, directory in `text.muted`, the number of matching lines on the right.
- **Code line:** editor font and font size, a right-aligned line-number gutter in `text.muted`, text colored by the theme (`style_for(capture)`), matches with the `search.match_background` background. The selected line (keyboard) has a `ghost_element.hover`/selected background.
- **Excerpt separator:** a row with only a dim `┈` in the gutter.
- All rows have the same height (the editor line height) so that `uniform_list` can be used.
- Single-file mode: the tab only shows "Search is unavailable when a single file is opened".

## 5. Commands, shortcuts and settings (follow VS Code)

| Command | Default key | Behavior |
|---|---|---|
| `workbench.action.findInFiles` | `cmd+shift+f` | Open/focus the Search tab in the active group (if it already exists, focus it in the group that holds it) and focus the query box; if the editor has a selection within one line, fill in the selection. |
| `workbench.view.search` | — | Same as above. |
| `toggleSearchCaseSensitive` | `alt+cmd+c` (when `searchViewletFocus`) | Toggle case. |
| `toggleSearchWholeWord` | `alt+cmd+w` (when `searchViewletFocus`) | Toggle whole word. |
| `toggleSearchRegex` | `alt+cmd+r` (when `searchViewletFocus`) | Toggle regex. |
| `workbench.action.search.toggleQueryDetails` | `shift+cmd+j` (when `searchViewletFocus`) | Hide/show the filter row. |
| `search.action.focusNextSearchResult` | `f4` | Select and open the next matching line (wraps to the start). |
| `search.action.focusPreviousSearchResult` | `shift+f4` | Same as above, backwards. |
| `search.action.refreshSearchResults` | — | Run the search again. |
| `search.action.collapseSearchResults` | — | Collapse all files. |

- The context key `searchViewletFocus` (VS Code's name, already present) now means: focus is inside the Search tab.
- In the tab: search runs as you type (150 ms debounce) and when you press `Enter` in the query box; `↑`/`↓` change the selected line; `Enter` in the list opens the selected line; `Esc` switches focus between the query box and the list.
- Clicking a line: opens the file as a preview (`OpenMode::Preview`), scrolls to the line, and selects the first match on the line (the column is computed by matching again on the decoded line, as in the old panel). Clicking a context line: opens at that line, with no selection. Clicking a header: collapses/expands.
- New setting: `search.searchEditor.defaultNumberOfContextLines` (a number, default 1, clamped to 0–5). `search.exclude` and `files.exclude` still apply as before.

## 6. Performance

- The UI thread does not read files or parse: search and highlighting both run in the background.
- `uniform_list` renders only the visible rows; the row list is rebuilt when a new file arrives or on collapse/expand, not on every frame.
- Highlighting only covers visible files, at most 3 at a time, and is cancelled on a new search.

## 7. Testing

- `lucerna-project`: merging context into excerpts (overlapping, adjacent, start of file, end of file, `context_lines = 0`), and the `MAX_RESULTS` limit counting only matching lines.
- `search_view::model`: building rows (with collapse), F4 across several files and wrapping, the summary.
- `search_view::highlight`: clipping spans by line, dropping lines whose text has drifted.
- `editor_groups::model`: a single Search tab only, saving/restoring `lucerna:search`; old state `sidebar_view = "search"` becomes Explorer.
- GPUI tests: `cmd+shift+f` opens the tab and fills in the selection; typing a query produces results; clicking a line opens the file at the right line; the magnifier button opens the tab; the sidebar no longer has Search; the `alt+cmd+c` key toggle.

## 8. Changes to older specs

Section 12 of the Phase 1 spec ("The panel switch buttons in the status bar … Explorer and Search") is replaced: the status bar has an Explorer button (toggles the sidebar) and a magnifier button (opens the Search tab).
