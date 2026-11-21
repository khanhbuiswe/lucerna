# Research: project-wide search UI (Zed and VS Code)

Date 2026-11-10. Prompted by the request "build the search topbar like Zed instead of a search panel; UI follows Zed first".

Zed sources: the GPUI build Lucerna currently pins (`~/.cargo/git/checkouts/zed-*/b5cd310`, commit dated 2026-11-07), the crates `search`, `title_bar` and `picker`, the file `assets/keymaps/default-macos.json`, and `docs/src/finding-navigating.md`. These crates are GPL, so we only learn the design.

## 1. Zed

### 1.1 The title bar has no search box
`crates/title_bar/src/title_bar.rs` only renders: the application menu (Linux/Windows), restricted mode, the remote host, the project name (click it to open the recent projects popover), the worktree and git branch (click to open the branch picker), the collaborator list, the connection status, the update button, the sign-in button and the user menu. There is no search box or command center.

### 1.2 Project Search is a tab (`pane::DeploySearch`, `cmd-shift-f`)
- `ProjectSearchView` implements `Item`, so it opens as a **tab in the editor pane**, not as a panel in the sidebar. The tab title is the query.
- A compact search bar (`ProjectSearchBar`, a `ToolbarItemView`) sits at the top of the tab: the query box, the toggle buttons (case, whole word, regex, include ignored), the replace button, and the filter button (`cmd-shift-j`) that shows include/exclude.
- Results appear in a **multibuffer**: each file is an excerpt with a few lines of context, and it is directly editable. The search only runs when you press Enter.
- Inside the tab: `escape` moves focus between the query box and the results, `cmd-shift-f` refocuses the search box, and `cmd-shift-enter` expands/collapses all.
- The status bar has a magnifier button (`SearchButton`, can be hidden with `search.button`) that opens Project Search.

### 1.3 Text Finder: a text-search popup (`project_search::OpenTextFinder`, `alt-cmd-f`)
A new Zed feature (`crates/search/src/text_finder*`). The docs describe it as: quickly find a string across the whole project, then open the file; for finer filtering, switch to Project Search.
- It is a **modal picker floating at the top center of the window**, like the file finder. Placeholder `"Search all files…"`.
- To the right of the input are the case / whole word / regex / include ignored toggles, a divider, and an indicator that the project is being scanned.
- The result list is **grouped by file**: a header row with a file icon, the file name and a dimmed directory; the header is collapsible (`text_finder::Fold`/`Unfold`/`ToggleFoldAll`). Below the header are the individual matching lines, with the matched text highlighted.
- The picker has a **preview** (`try_get_preview_data_for_match`): when preview is on, the picker grows to a "telescope" size and shows the file around the matching line on the right or below.
- The Actions menu in the bottom right corner: Split left/right/up/down, Open File (or Open Multiple when several are selected), and **Open as Tab** (`ToProjectSearch`), which moves the query into Project Search.
- It remembers the last query and options per workspace (stored in the DB).
- Results stream into the picker while the search runs, and the number of matches per overly long line is capped.

## 2. VS Code

- **Search view** (`workbench.view.search`, `workbench.action.findInFiles`, `cmd-shift-f`): by default a view in the sidebar; the old `search.location` setting allowed placing it in the panel. It can be dragged to the panel or secondary sidebar. There is "Open in Editor" (`search.action.openNewEditor`), which creates a text-style **Search Editor** tab.
- **Command Center** (`window.commandCenter`, on by default since 1.77): a fake box in the middle of the title bar showing the workspace name and a magnifier. Clicking it opens **Quick Open** (`workbench.action.quickOpen`) in file-picking mode, hinting at the prefixes (`>` commands, `:` line, `@` symbol, `%` text...). It has back/forward buttons on both sides. It does not search text by itself.
- **Quick Search** (`workbench.action.quickTextSearch`, prefix `%` in Quick Open, since 1.83): searches text inside Quick Open; each result line shows the file name, and there is a button to open it in the Search view. It has no default shortcut.

## 3. Comparison

| | Zed | VS Code |
|---|---|---|
| Search box on the title bar | No | Yes (Command Center → Quick Open) |
| Quick text search as a popup | Text Finder, `alt-cmd-f`, grouped by file, with preview | Quick Search, `%`, flat list |
| Full text search | Project Search tab (multibuffer) | Search view in the sidebar (+ Search Editor tab) |
| `cmd-shift-f` | Opens the Project Search tab | Opens the Search view in the sidebar |

## 4. What Lucerna chooses

Waiting for the user to choose (see the spec). By principle: UI follows Zed, commands/shortcuts follow VS Code (`workbench.action.findInFiles` = `cmd-shift-f`, `workbench.action.quickTextSearch`).

## 5. Find in file (find widget)

- **VS Code**: the find widget (`actions.find`, `cmd-f`) is a floating overlay at the top right of the editor. It does not push the content down; it covers part of the text. It has an input with case / whole word / regex toggles, a counter "3 of 19", previous/next buttons, "find in selection", a close button, and a collapsible replace area.
- **Zed**: `BufferSearchBar` is a toolbar item of the pane: a full-width row below the breadcrumb that pushes the editor content down. The row has a query box with the Aa / wd / .* toggles inside the box, select-all and replace buttons, previous/next buttons, a counter "3/19" and a close button. A regex error shows right on the bar.
- **Lucerna chooses**: UI follows Zed (a full-width bar below the breadcrumb, pushing the content down, covering the minimap too), shortcuts and behavior follow VS Code (`cmd-f`, `enter` / `shift-enter`, `escape`, the toggles). We only learn the design and do not copy Zed code (GPL).

## 6. Details of the Search tab UI and the find bar (Zed, revision `b5cd310`)

Read from `crates/search/src/{buffer_search,project_search,search_bar}.rs` and `crates/editor/src/element/header.rs`. We only learn the design and do not copy code (GPL).

- **Toolbar.** Both `BufferSearchBar` and `ProjectSearchBar` share one query-box frame (`input_base_styles`): `min_w_32`, `min_h_8`, a 1 px border, `rounded_md`, with the Aa / wd / .* toggles inside the box. When the regex is invalid, the box border changes to the `Error` color; when there are no results, the query text changes to the `Error` color. After the box comes a control cluster (`min_w_64`), then the `‹ ›` cluster and the counter, separated by a `border_l_1` in the `border_variant` color. The counter is `"{i}/{n}"`, or `"0/0"` when no match is selected yet; the label size is `Small`. The close button of the in-file find bar sits separately at the right edge, after a `border_variant` divider. In the Project Search tab, the collapse/expand-all button (`ChevronDownUp` / `ChevronUpDown`) leads the row; the filter button is a square `IconButton`.
- **File header in the multibuffer.** Height `FILE_HEADER_HEIGHT = 2` editor lines, padding `BUFFER_HEADER_PADDING` (0.25 rem). Inside is a `rounded_sm` card with `border_1` in the `border` color, background `editor_subheader_background`, hover `element_hover`, filling the whole row. The card has a chevron button to fold the file, a dimmed file icon, the file name (buffer font), a dimmed directory truncated at the start, and an "Open File" button on the right that only shows on hover or when the file is selected.
- **Sticky header.** When you scroll into the middle of a file, Zed redraws that file's header over the top of the editor area, with a gradient background from transparent to `editor_background` and a `shadow_md`. When the next file's header comes close, it pushes the sticky header up (shifting `origin.y` by the overlap).
- **Empty state.** A tab with no search yet shows a large "Search All Files" title in the center, a dimmed line "Start typing to search. For more options:", then buttons with icon, label and shortcut: include/exclude, find and replace, match with regex, match case, match whole words.

**What Lucerna takes:**

- A shared component in `ui.rs` for both the in-file find bar and the Search tab: `search_toolbar` (background `editor.background`, bottom line `border.variant`), `search_bar_row` (40 px tall, 8 px horizontal margin), `search_query_box` (28 px box, asks for about 2/3 of the row via `flex_basis`, minimum 128 px, border in the `error` color when there are no results or the regex is invalid), `bar_separator`, and `match_nav` (`‹ ›` + counter). Lucerna changes the border to the error color also when there are no results, instead of changing the text color as Zed does, so both bars share one signal.
- The counter follows Zed: `"3/19"`, `"0/0"` when there are no results. Only in the Search tab, when there are results but no line is selected yet, the counter reads `"19 results"` instead of `"0/19"`, because the summary line "N results in M files" is dropped once the search finishes.
- The file header follows Zed: 2 lines tall, 4 px padding, a `rounded_sm` card with a `border` border on the `editor.subheader.background` background (a new theme key, falling back to something like `element.background`), hover `ghost_element.hover`, and an open-file button (icon `arrow-up-right`) shown on hover or when the file contains the selected line.
- The sticky header follows Zed, including the effect of being pushed up by the next header. The background uses a solid `editor.background` color instead of a gradient.
- The boundary between two excerpts is a 1 px `border.variant` line across the code area, 1 line tall, instead of a character in the gutter.
- The empty state follows Zed, dropping the "Find and replace" line because Lucerna is only for viewing code. Shortcuts come from the keymap (`Keymap::label_for`).
- The result list moves from `uniform_list` to gpui's `list` (`ListState`) so a header can be twice as tall as a line. The height of each row is known in advance from the row kind, so scrolling to a row and computing the visible rows are exact even when the list has not measured those rows yet.
