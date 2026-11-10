# Project Search Tab Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the sidebar Search panel with a Zed-style Project Search editor tab whose results are syntax-highlighted excerpts with context lines.

**Architecture:** `lucerna_project::search` streams per-file excerpts (match lines plus context). A pure `search_view::model` turns them into a flat row list; `search_view::highlight` parses a result file on a background thread and returns per-line capture spans; `search_view::view::SearchView` is one entity rendered as the body of a singleton `TabItem::Search`, like the Extensions tab.

**Tech Stack:** Rust, GPUI (only in `lucerna-workbench`), `grep-searcher`, tree-sitter via `lucerna-syntax`.

**Spec:** `docs/superpowers/specs/2026-11-10-project-search-tab-design.md` (research: `docs/research/2026-11-10-project-search-ui.md`).

## Global Constraints

- Cargo needs `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"`. Keep ≥ 1.2 GB free disk.
- Only `lucerna-workbench` may depend on gpui.
- No code copied from Zed (GPL); learn the design only.
- Commands, keys and settings use VS Code IDs; UI follows Zed.
- The UI thread never reads files or parses: search and highlighting run in the background.
- Commit on `main`, staging files by explicit path; trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Before finishing: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, `cargo deny check` all pass.
- Copy to keep verbatim: placeholder `"Search"`, include/exclude placeholders `"e.g. src/**, *.rs"` / `"e.g. target, *.min.js"`, `"Search is unavailable when a single file is opened"`, `"Searching…"`, `"No results found"`, `"N results in M files"` with ` (truncated at 20,000)`.

## Review Focus

- A match on line 1 or the last line of a file (context window clipped at file edges) must not panic or produce empty excerpts — tested in Task 1.
- Two matches whose context windows touch (gap of exactly `2 * context_lines` lines) must render as one excerpt with no duplicate lines — tested in Task 1.
- A file edited between search and highlighting (lines shifted) must show plain text, never colors from the wrong line — tested in Task 3.
- Restarting with a saved Search tab in single-file mode or with an empty query must not start a search or crash — tested in Task 7.
- Closing the Search tab while it holds focus must move focus to the editor area so keys keep working — tested in Task 4.

---

### Task 1: Search returns excerpts with context lines

**Files:**
- Modify: `crates/lucerna-project/src/search.rs`, `crates/lucerna-project/src/lib.rs`, `crates/lucerna-workbench/src/search_panel.rs` (compile-only adaptation)
- Test: `crates/lucerna-project/tests/search.rs`

**Interfaces:**
- Produces:
  - `SearchQuery { …, pub context_lines: u32 }` (default 0).
  - `pub struct Excerpt { pub lines: Vec<LineMatch> }` — consecutive `line_number`s; a line with empty `ranges` is context. `impl Excerpt { pub fn match_count(&self) -> usize }`.
  - `SearchEvent::File { path: Arc<Path>, excerpts: Vec<Excerpt> }` (replaces `matches`).
  - `lib.rs` re-exports `Excerpt`.

- [ ] **Step 1: Update the test helpers and write failing tests**

Change `files()` in `tests/search.rs` to flatten match lines (`excerpts.iter().flat_map(|e| &e.lines).filter(|l| !l.ranges.is_empty())`) so every existing test keeps asserting on match lines only. Add a helper `excerpts(root, events) -> Vec<(String, Vec<Vec<u32>>)>` giving each file's excerpts as line-number lists, and these tests (file content `"l1\nl2\n…"` built from a list, `needle` on chosen lines, `context_lines` as given):

```rust
#[test] fn context_zero_gives_one_line_excerpts()      // matches on 2,5 → [[2],[5]]
#[test] fn context_lines_surround_match()              // ctx 1, match 5 of 9 → [[4,5,6]]; line 4,6 ranges empty
#[test] fn context_clipped_at_file_edges()             // ctx 2, matches 1 and 9 of 9 → [[1,2,3],[7,8,9]]
#[test] fn touching_context_merges()                   // ctx 1, matches 2 and 4 → [[1,2,3,4,5]]
#[test] fn adjacent_after_gap_merges()                 // ctx 1, matches 2 and 5 (lines 3,4 are context) → [[1,2,3,4,5,6]]
#[test] fn separate_groups_split()                     // ctx 1, matches 2 and 8 of 10 → [[1,2,3],[7,8,9]]
#[test] fn cap_counts_only_match_lines()               // ctx 1, MAX_RESULTS+5 matches in one file → Done total == MAX_RESULTS, truncated, match lines == MAX_RESULTS
#[test] fn context_lines_are_cut_like_matches()        // ctx 1, a context line of 1,500 chars → its line_text has 1,000 chars
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p lucerna-project --test search`
Expected: compile error on `excerpts` / `context_lines`.

- [ ] **Step 3: Implement**

Build the searcher with `.before_context(n).after_context(n)`. Replace the `Lossy` sink with a private `ExcerptSink` implementing `grep_searcher::Sink`: `matched` and `context` both decode the line lossily, strip `\r\n`, cut to `MAX_LINE_CHARS` (reuse `line_match`; context lines get no ranges), and append to the current excerpt, starting a new one when `context_break` fired or the line number is not the previous + 1. `matched` returns `Ok(match_count < MAX_RESULTS && !cancel)`. Replace the per-file cap with `fn truncate_matches(excerpts: &mut Vec<Excerpt>, allowed: usize)`: keep everything up to and including the `allowed`-th match line, drop the rest (trailing context included) and empty excerpts.

Keep the workspace compiling: in `search_panel.rs` (deleted in Task 4) set `context_lines: 0` and turn `SearchEvent::File { excerpts, .. }` back into `matches` by flattening the lines with non-empty `ranges`. No behaviour change.

- [ ] **Step 4: Run tests**

Run: `cargo test -p lucerna-project && cargo test -p lucerna-workbench --test search_panel`
Expected: PASS (old and new).

- [ ] **Step 5: Commit**

```bash
git add crates/lucerna-project/src/search.rs crates/lucerna-project/src/lib.rs crates/lucerna-project/tests/search.rs crates/lucerna-workbench/src/search_panel.rs
git commit -m "feat(project): search streams excerpts with context lines"
```

---

### Task 2: Pure results model

**Files:**
- Create: `crates/lucerna-workbench/src/search_view/mod.rs` (just `pub(crate) mod model;` for now), `crates/lucerna-workbench/src/search_view/model.rs`
- Modify: `crates/lucerna-workbench/src/lib.rs` (add `mod search_view;`)
- Test: unit tests inside `model.rs`

**Interfaces:**
- Consumes: `Excerpt`, `LineMatch`, `MAX_RESULTS` (Task 1).
- Produces (in `search_view::model`):
  - `pub struct LineRef { pub file: usize, pub excerpt: usize, pub line: usize }` (Copy, Eq).
  - `pub enum Row { Header(usize), Line(LineRef), Separator { file: usize } }`.
  - `pub struct FileResult { pub path: Arc<Path>, pub excerpts: Vec<Excerpt>, pub collapsed: bool }` with `match_count()`.
  - `pub struct Results` with: `clear()`, `add_file(path: Arc<Path>, excerpts: Vec<Excerpt>)` (sorted by path; an existing path gets its excerpts appended), `files() -> &[FileResult]`, `rows() -> &[Row]`, `line(LineRef) -> Option<&LineMatch>`, `toggle_collapsed(file)`, `set_all_collapsed(bool)`, `expand(file)`, `row_of(LineRef) -> Option<usize>`, `next_match(from: Option<LineRef>, forward: bool) -> Option<LineRef>`, `match_count() -> usize`.
  - Free functions moved from `search_panel.rs` unchanged: `globs`, `thousands`, `first_match`, plus `pub fn summary(matches: usize, files: usize, searched: bool, done: Option<bool>) -> Option<String>` (same strings as `SearchPanel::summary`, without the single-file and error cases).

Rows: per file a `Header`, then (unless collapsed) each excerpt's lines as `Line`, with `Separator { file }` between two excerpts of the same file. `rows` is rebuilt by every mutating call.

- [ ] **Step 1: Write failing tests**

```rust
#[test] fn rows_have_header_lines_and_separators()   // 2 excerpts of 3 lines → [Header(0), L×3, Separator, L×3]
#[test] fn collapsed_file_shows_only_header()
#[test] fn files_stay_sorted_and_merge()              // add b, a, then b again → files [a, b]; b has both batches
#[test] fn next_match_skips_context_and_wraps()       // forward from last match → first match; backward from first → last
#[test] fn next_match_crosses_collapsed_files()       // returns a LineRef in a collapsed file; after expand(file) row_of is Some
#[test] fn next_match_from_none_is_first_or_last()
#[test] fn summary_strings()                          // (0,0,true,None) "Searching…"; (0,0,true,Some(false)) "No results found"; (1,1,..) "1 result in 1 file"; (20000,3,true,Some(true)) "20,000 results in 3 files (truncated at 20,000)"; searched=false → None
```
Move the existing `globs_split_on_commas` and `thousands_separators` tests here.

- [ ] **Step 2: Run** `cargo test -p lucerna-workbench --lib search_view::model` — expected FAIL (items not defined). `search_panel.rs` keeps its own copies of the moved helpers until Task 4 deletes it.

- [ ] **Step 3: Implement** the model as specified.

- [ ] **Step 4: Run tests** — PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/lucerna-workbench/src/search_view/mod.rs crates/lucerna-workbench/src/search_view/model.rs crates/lucerna-workbench/src/lib.rs
git commit -m "feat(search): pure results model for the search tab"
```

---

### Task 3: Background excerpt highlighting

**Files:**
- Create: `crates/lucerna-workbench/src/search_view/highlight.rs`
- Modify: `crates/lucerna-workbench/src/search_view/mod.rs`
- Test: unit tests inside `highlight.rs`

**Interfaces:**
- Consumes: `Excerpt` (Task 1); `Buffer::load`, `BufferSnapshot`, `LanguageRegistry::language_for_file`, `SyntaxState::{new, parse, highlights}`.
- Produces:
  - `pub const MAX_HIGHLIGHT_BYTES: u64 = 2 * 1024 * 1024;`
  - `pub type LineSpans = Vec<(Range<usize>, Arc<str>)>;` — byte ranges within `LineMatch::line_text`, and capture names.
  - `pub type FileHighlights = HashMap<u32, LineSpans>;` keyed by 1-based `line_number`.
  - `pub fn highlight_file(abs_path: &Path, rel_path: &Path, excerpts: &[Excerpt], registry: &Arc<LanguageRegistry>, styled: &dyn Fn(&str) -> bool) -> FileHighlights` — blocking; call on a background thread.

Behaviour: empty map when the file is over `MAX_HIGHLIGHT_BYTES`, fails to load, has no language, or the grammar fails. Otherwise parse once (`parse(&snapshot, None)`), and per excerpt call `highlights(snapshot, first_row..last_row + 1, styled)` (rows are 0-based = `line_number - 1`). Split each span by row, convert to in-line byte ranges, clip to `line_text.len()` and char boundaries, drop empty ranges. A line is included only when the snapshot's row text (without `\r\n`) starts with `line_text` — otherwise the file changed and that line stays plain.

- [ ] **Step 1: Write failing tests** (temp dir, bundled registry `Arc::new(LanguageRegistry::bundled())`, `styled = |_| true`, a `.rs` file):

```rust
#[test] fn rust_keyword_gets_a_capture()       // "fn main() {}" line 1 → spans contain 0..2 with a capture starting "keyword"
#[test] fn multi_line_comment_colors_inner_line()  // "/*\nneedle\n*/" excerpt of line 2 only → line 2 has a capture starting "comment"
#[test] fn changed_line_is_left_plain()        // excerpt line_text "old" but file line is "new" → no entry for that line
#[test] fn unknown_language_is_empty()         // "x.unknownext"
#[test] fn oversized_file_is_empty()           // file of MAX_HIGHLIGHT_BYTES + 1 bytes
#[test] fn spans_are_clipped_to_cut_line()     // line_text is a 10-char prefix of a longer line → every range.end <= 10
```

- [ ] **Step 2: Run** `cargo test -p lucerna-workbench --lib search_view::highlight` — expected FAIL.
- [ ] **Step 3: Implement** `highlight_file`.
- [ ] **Step 4: Run tests** — PASS.
- [ ] **Step 5: Commit**

```bash
git add crates/lucerna-workbench/src/search_view/highlight.rs crates/lucerna-workbench/src/search_view/mod.rs
git commit -m "feat(search): highlight result excerpts off the UI thread"
```

---

### Task 4: Search tab replaces the sidebar panel

**Files:**
- Create: `crates/lucerna-workbench/src/search_view/view.rs`
- Delete: `crates/lucerna-workbench/src/search_panel.rs`, `crates/lucerna-workbench/tests/search_panel.rs`
- Create: `crates/lucerna-workbench/tests/search_view.rs`
- Modify: `crates/lucerna-workbench/src/editor_groups/model.rs`, `editor_groups/view.rs` (tab icon), `src/workbench.rs`, `src/workspace_state.rs`, `src/layout/panel_toggles.rs`, `src/layout/sidebar.rs` (if it matches on `SidebarView::Search`), `src/lib.rs`, `crates/lucerna-workbench/tests/editor_groups_model.rs`, `tests/workspace_state.rs`, `tests/chrome.rs` (toggle assertions)

**Interfaces:**
- Consumes: Tasks 1–2.
- Produces:
  - `TabItem::Search`; `pub const SEARCH_TAB_STATE: &str = "lucerna:search";` `EditorGroups::open_search(&mut self) -> TabId`, `EditorGroups::search_tab(&self) -> Option<TabId>` (mirror `open_extensions` / `extensions_tab`; restore keeps only the first Search tab, pinned; `WorkspaceState::without_missing_files` keeps it like the Extensions tab).
  - `Tab::title()` returns `"Search"` for the Search tab (the model does not know the query). `EditorGroupsView` gains `pub fn set_search_title(&mut self, title: Option<SharedString>, cx: &mut Context<Self>)`; the tab strip shows that title instead of `"Search"` when it is `Some` (Task 7 calls it).
  - `SearchView` (in `search_view::view`, re-exported from `search_view`): `new(root: PathBuf, workbench: WeakEntity<Workbench>, searchable: bool, cx) -> Self`; `query_input()`, `include_input()`, `exclude_input()` -> `&Entity<TextInput>`; `results() -> &Results`; `summary() -> Option<String>` (single-file → the unavailable copy; error → `None`); `error() -> Option<&str>`; `run_search(cx)`; `focus_query(text: Option<String>, window, cx)`; `open_line(LineRef, window, cx)`; `toggle_collapsed(file, cx)`. Implements `Focusable` and `Render`.
  - `Workbench::search_view() -> &Entity<SearchView>`, `Workbench::open_search(window, cx)` (opens/activates the tab, `focus_query(None)`).
  - `SidebarView` keeps only `Explorer`; state name `"search"` now restores the Explorer.

Behaviour to port from `search_panel.rs` unchanged: debounce `SEARCH_DEBOUNCE` (150 ms), cancel via `AtomicBool`, fresh channel + generation per search (R17), `Enter` in the query reruns now, `search.exclude` + `files.exclude`, open-at-line with column re-matched on the decoded buffer line (R24) via `first_match`, `Drop` cancels. New: query passes `context_lines` read from setting `search.searchEditor.defaultNumberOfContextLines` (`settings.get::<u32>(…, None).unwrap_or(1).min(5)`; add `"search.searchEditor.defaultNumberOfContextLines": 1` to `crates/lucerna-settings/assets/default-settings.json`). Clicking a context line opens at that line with no highlight; clicking a header toggles collapse.

Layout (spec §4), rows plain-colored for now (Task 5 adds syntax colors):
- Search bar row 32 px: `input_box` with query + `toggle_button`s `Aa` / `ab` / `.*` (ids `search-toggle-case|word|regex`), then a 1 px `border_variant` divider, then `icon_button`s `search-filters` (Lucide `filter`/`list-filter` icon), `search-refresh` (`refresh-cw`), `search-collapse-all` (`chevrons-down-up`). Add any missing Lucide SVGs to `assets/icons/` with the ISC notice already covering Lucide.
- Filter row (hidden until `search-filters` is on): include and exclude `input_box`es side by side.
- Summary line: `UI_FONT_SIZE_SMALL`, `text_muted`, or the error in the error color the panel used.
- `uniform_list("search-results", …)` with every row the editor line height (`editor.fontSize` × `editor.lineHeight` like `CodeView::line_height`). Header: chevron, file icon via `cx.global::<AppState>().icons.file(path)` + `file_icon_sized`, name in `text`, dir in `text_muted`, match count right-aligned; background `element_background`. Line: gutter of line number right-aligned in `text_muted` (width = digits of the largest line number), text in `editor.fontFamily`, whitespace preserved (no trim), match ranges with `search_match_background`. Separator: `┈` in the gutter, `text_muted`. Debug selectors: `search-file-{file}`, `search-line-{file}-{excerpt}-{line}`, `search-separator-{file}-{excerpt}`.

Workbench wiring: the tab body factory returns `search_view` for `TabItem::Search`; `findInFiles` and `workbench.view.search` call `open_search` then `focus_query(selection)`; remove `search_panel` field, `search_view` sidebar slot, `SidebarView::Search` arms; the status-bar search toggle becomes `icon_button("panel-toggle-search", SEARCH_ICON, search_tab_active)` running `workbench.action.findInFiles`; `searchViewletFocus` = Search tab open and `search_view.focus_handle().contains_focused`; closing the Search tab while focused moves focus to the editor area (generalise `extensions_tab_closed`); tab-strip icon `SEARCH_ICON` for Search tabs.

- [ ] **Step 1: Port and extend the GPUI tests in `tests/search_view.rs`** — port every test from `tests/search_panel.rs` to drive the tab (open with `cmd-shift-f`, read `wb.search_view()`), renaming selectors (`search-match-{f}-{i}` → `search-line-{f}-{e}-{l}`). Add:

```rust
#[gpui::test] fn find_in_files_opens_search_tab()            // cmd-shift-f → active tab item == TabItem::Search, query focused; again → still one Search tab
#[gpui::test] fn results_include_context_lines()             // ctx default 1: match on line 3 of 5 → rows Header, lines 2,3,4
#[gpui::test] fn click_context_line_opens_at_that_line()
#[gpui::test] fn status_bar_search_button_opens_tab()         // click panel-toggle-search
#[gpui::test] fn sidebar_has_only_explorer()                  // cmd-shift-e works; no SidebarView::Search exists (compile-level) and state "search" restores Explorer
#[gpui::test] fn closing_focused_search_tab_focuses_editor_area()
```
And in `tests/editor_groups_model.rs`: `open_search_is_singleton_and_pinned`, `restore_keeps_first_search_tab`; in `tests/workspace_state.rs`: `search_tab_survives_without_missing_files`.

- [ ] **Step 2: Run** `cargo test -p lucerna-workbench --test search_view --test editor_groups_model --test workspace_state` — expected FAIL.
- [ ] **Step 3: Implement** the view and wiring as above; delete `search_panel.rs` and its test file.
- [ ] **Step 4: Run** `cargo test -p lucerna-workbench` — PASS (whole crate, since chrome/shell tests touch the toggles).
- [ ] **Step 5: Commit**

```bash
git add crates/lucerna-workbench/src/search_view/view.rs crates/lucerna-workbench/src/search_view/mod.rs crates/lucerna-workbench/tests/search_view.rs crates/lucerna-workbench/src/editor_groups/model.rs crates/lucerna-workbench/src/editor_groups/view.rs crates/lucerna-workbench/src/workbench.rs crates/lucerna-workbench/src/workspace_state.rs crates/lucerna-workbench/src/layout/panel_toggles.rs crates/lucerna-workbench/src/lib.rs crates/lucerna-settings/assets/default-settings.json
git rm crates/lucerna-workbench/src/search_panel.rs crates/lucerna-workbench/tests/search_panel.rs
# plus every other file this task modified, by explicit path (sidebar.rs, tests/*.rs, assets/icons/*.svg)
git commit -m "feat(search): project search opens as an editor tab"
```

---

### Task 5: Syntax colors in the results

**Files:**
- Modify: `crates/lucerna-workbench/src/search_view/view.rs`
- Test: `crates/lucerna-workbench/tests/search_view.rs`

**Interfaces:**
- Consumes: `highlight_file`, `FileHighlights` (Task 3); `Results` (Task 2).
- Produces: `SearchView::highlights(&self, path: &Path) -> Option<&FileHighlights>` (test accessor) and `pub const MAX_HIGHLIGHT_JOBS: usize = 3;`.

Behaviour: `render_rows` records which files have a row in the visible range. Each file not yet highlighted or queued is queued; at most `MAX_HIGHLIGHT_JOBS` run at once on `cx.background_spawn` (wrapped in `guarded`), each calling `highlight_file` with `AppState.languages` and a `styled` closure over a clone of `AppState.theme` (`theme.style_for(c).is_some()`). Results are stored per path with the search generation; a new search clears the map and queue and drops late results of older generations. A theme change (`theme_generation`) clears the map so lines re-highlight. Line rendering builds `StyledText` highlights from the line's spans (`style_for(capture)` → color/italic/bold) and overlays `search_match_background` on match ranges (match background wins where they overlap; keep runs non-overlapping by splitting).

- [ ] **Step 1: Write failing tests**

```rust
#[gpui::test] fn visible_rust_result_gets_highlights()      // search "main" in a .rs file → after settling, highlights(path) has an entry for the match line
#[gpui::test] fn new_search_drops_old_highlights()           // search a, settle, search b → highlights of a's file gone until re-fetched; no stale generation applied
#[gpui::test] fn at_most_three_highlight_jobs_run()          // 10 result files visible → in-flight count never exceeds 3 (expose `highlight_jobs_in_flight()` cfg(test)/pub(crate))
```
- [ ] **Step 2: Run** `cargo test -p lucerna-workbench --test search_view` — FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** — PASS.
- [ ] **Step 5: Commit**

```bash
git add crates/lucerna-workbench/src/search_view/view.rs crates/lucerna-workbench/tests/search_view.rs
git commit -m "feat(search): syntax-color result excerpts"
```

---

### Task 6: Search commands and keys

**Files:**
- Modify: `crates/lucerna-workbench/src/search_view/view.rs`, `crates/lucerna-settings/assets/default-keybindings.json`
- Test: `crates/lucerna-workbench/tests/search_view.rs`

**Interfaces:**
- Consumes: `Results::{next_match, expand, row_of, set_all_collapsed}` (Task 2), `SearchView::open_line` (Task 4).
- Produces: commands registered by `search_view::register_commands(&mut CommandRegistry)`:
  `toggleSearchCaseSensitive`, `toggleSearchWholeWord`, `toggleSearchRegex`, `workbench.action.search.toggleQueryDetails`, `search.action.focusNextSearchResult`, `search.action.focusPreviousSearchResult`, `search.action.refreshSearchResults`, `search.action.collapseSearchResults`. `SearchView::selected() -> Option<LineRef>`.

Keybindings to add:
```json
{ "key": "alt+cmd+c", "command": "toggleSearchCaseSensitive", "when": "searchViewletFocus" },
{ "key": "alt+cmd+w", "command": "toggleSearchWholeWord", "when": "searchViewletFocus" },
{ "key": "alt+cmd+r", "command": "toggleSearchRegex", "when": "searchViewletFocus" },
{ "key": "shift+cmd+j", "command": "workbench.action.search.toggleQueryDetails", "when": "searchViewletFocus" },
{ "key": "f4", "command": "search.action.focusNextSearchResult" },
{ "key": "shift+f4", "command": "search.action.focusPreviousSearchResult" }
```
F4 / Shift+F4 work from anywhere once a search has results: select the next/previous match line (`next_match` from `selected`), expand its file, scroll it into view, and `open_line` it (preview). The list has its own focus handle: `↑`/`↓` move `selected` over `Line` rows (skipping headers and separators), `Enter` opens the selection, `Esc` in the query focuses the list and `Esc` in the list focuses the query (only when the find widget is not involved — keep `escape_in_search_panel_keeps_find_widget` behaviour). The selected line gets `ghost_element_selected` (or `element_selected`) background. Refresh = `run_search`; collapse = `set_all_collapsed(true)` (the collapse-all button toggles between all collapsed and all expanded).

- [ ] **Step 1: Write failing tests**

```rust
#[gpui::test] fn alt_cmd_c_toggles_case_in_search_tab()     // also: same keys in an editor do nothing to the search
#[gpui::test] fn shift_cmd_j_shows_filter_row()             // include input becomes visible (debug selector search-include)
#[gpui::test] fn f4_walks_matches_across_files_and_wraps()  // opens file of each match in turn; after last → first
#[gpui::test] fn f4_expands_collapsed_file()
#[gpui::test] fn arrows_and_enter_open_selected_line()
#[gpui::test] fn escape_swaps_focus_between_query_and_list()
#[gpui::test] fn collapse_all_then_expand_all()
```
- [ ] **Step 2: Run** — FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p lucerna-workbench --test search_view` and `cargo test -p lucerna-settings` — PASS.
- [ ] **Step 5: Commit**

```bash
git add crates/lucerna-workbench/src/search_view/view.rs crates/lucerna-workbench/src/search_view/mod.rs crates/lucerna-workbench/src/workbench.rs crates/lucerna-settings/assets/default-keybindings.json crates/lucerna-workbench/tests/search_view.rs
git commit -m "feat(search): VS Code search commands and keys in the search tab"
```

---

### Task 7: Tab title and persisted query

**Files:**
- Modify: `crates/lucerna-workbench/src/search_view/view.rs`, `src/workspace_state.rs`, `src/workbench.rs`
- Test: `crates/lucerna-workbench/tests/search_view.rs`, `tests/workspace_state.rs`

**Interfaces:**
- Consumes: `EditorGroupsView::set_search_title` (Task 4).
- Produces: `#[derive(Serialize, Deserialize, Default, Clone, PartialEq, Debug)] pub struct SearchState { pub query: String, pub case_sensitive: bool, pub whole_word: bool, pub is_regex: bool, pub include: String, pub exclude: String, pub filters_visible: bool }`; `WorkspaceState { …, #[serde(default)] pub search: SearchState }`; `SearchView::state(&self, cx) -> SearchState`, `SearchView::restore(&mut self, state: SearchState, window, cx)`.

Behaviour: the tab title is the query (`"Search"` when empty), updated on every query change. The workbench saves `search_view.state()` with the layout (any search input change calls `layout_changed`), and on startup restores it into the view; when the restored layout has a Search tab and the query is non-empty and the view is searchable, it runs the search once. Single-file mode never searches.

- [ ] **Step 1: Write failing tests**

```rust
#[gpui::test] fn tab_title_follows_query()                    // type "needle" → tab title "needle"; clear → "Search"
#[gpui::test] fn search_state_round_trips_and_reruns()        // set query + regex + include, save, reopen workbench from the store → inputs restored, results present
#[gpui::test] fn restored_search_in_single_file_mode_does_not_search()
#[gpui::test] fn restored_empty_query_does_not_search()
```
`tests/workspace_state.rs`: `old_state_without_search_field_loads` (JSON without `search` → `SearchState::default()`).
- [ ] **Step 2: Run** — FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p lucerna-workbench` — PASS.
- [ ] **Step 5: Commit**

```bash
git add crates/lucerna-workbench/src/search_view/view.rs crates/lucerna-workbench/src/workspace_state.rs crates/lucerna-workbench/src/workbench.rs crates/lucerna-workbench/src/editor_groups/view.rs crates/lucerna-workbench/tests/search_view.rs crates/lucerna-workbench/tests/workspace_state.rs
git commit -m "feat(search): search tab title and restored query"
```

---

### Task 8: Docs and full verification

**Files:**
- Modify: `docs/superpowers/specs/2026-11-10-project-search-tab-design.md` (status → implemented), `README.md` only if it mentions the Search panel (`grep -n -i "search panel\|sidebar search" README.md site/ 2>/dev/null`).

- [ ] **Step 1:** `grep -rn "search_panel\|SearchPanel\|SidebarView::Search" crates docs/superpowers/specs/2026-11-10* README.md` — expected: no code hits.
- [ ] **Step 2:** Run, all must pass:
  ```bash
  export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
  cargo fmt --check
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace
  cargo deny check
  ```
- [ ] **Step 3:** Run the app (`cargo run -p lucerna -- .`), press `⇧⌘F`, search `build_pattern`, confirm excerpts with colors, click a line, press F4; take a screenshot for the user.
- [ ] **Step 4: Commit** docs changes by explicit path: `git commit -m "docs: mark project search tab implemented"`.
