# Replace in project — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a Replace row to the Search tab (⇧⌘H), with a confirmed Replace All, per-line and per-file replace, Dismiss, and an inline preview. Closed files are written to disk off the UI thread; open files go through `Documents`.

**Architecture:**
- One gpui-free matching rule lives in `lucerna-project` (`search_regex` + `line_matches`). Search, the preview and the replace job all use it, and whole word is decided by `editor.wordSeparators` via `lucerna_text::words`.
- A gpui-free job, `lucerna_project::replace_files`, replaces only the shown match lines and writes closed files atomically with a hash check.
- The workbench (`search_view/replace.rs`) splits the shown files:
  - closed files go to the job;
  - open documents get edits computed in the background and applied as one undo step, and clean ones are saved with an autosave-like save that never prompts.

**Tech Stack:** Rust, gpui (Zed rev `b5cd310`), `regex`, `grep-searcher`, `lucerna-text` (decode/encode/atomic write).

**Spec:** `docs/superpowers/specs/2026-11-18-project-replace-design.md`

## Global Constraints

- Every string the app shows is English, copied exactly from the spec. Specs and comments may be Vietnamese.
- Only `lucerna-workbench` may depend on gpui. `lucerna-project` and `lucerna-text` stay gpui-free.
- The UI thread never waits on file I/O or on matching whole files.
- Never replace text that the results do not show: only the given lines, and only matches that start within the shown prefix (`line_text.len()`, at most `MAX_LINE_CHARS` = 1000 chars). Skip empty matches and matches containing `\n`.
- Never write over a file that changed since it was read (stamp/hash check). Never auto-save a document that had unsaved changes. Never write a lossy-decoded file.
- Reuse `ui.rs` components: `search_bar_row`, `search_query_column`, `input_box`, `toggle_button`, `bar_icon_button`, `bar_separator`, `centered_message`. Icons: `icons::REPLACE`, `icons::REPLACE_ALL`, `icons::X`.
- Shared files are edited append-only: `default-keybindings.json` (one block at the end), `app_menu.rs` (one line), `CHANGELOG.md`, `docs/keyboard-shortcuts.md`.
- VS Code code may be learned from with credit in a comment. Zed (GPL) and Helix (MPL) are never copied.
- Cargo:
  - Prefix commands with `PATH="/opt/homebrew/opt/rustup/bin:$PATH"`.
  - Run `scripts/prune-target.sh` before building.
  - Never set `CARGO_TARGET_DIR`.
  - `touch crates/*/src/lib.rs` if stale artifacts cause impossible errors.
- Commit with explicit paths (`git commit -- <paths>`). The trailer is `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. **An agent rewrites a shown file between the search and Replace All.** That file is skipped (`changed since the search`, or `changed on disk` if the change lands after the read), and the other files are still replaced. Task 3 tests both windows; Task 7 tests the summary.
2. **A file shown in the results is open and dirty.** The search ran on its buffer, so its line numbers are the buffer's. The replace must check lines against the buffer, edit the buffer as one undo step, leave it dirty, and leave the disk untouched. Task 7.
3. **A file is open and clean.** It must be edited through `Documents` (undo works, no reload flash) and then saved. It must never be written behind the document's back by the closed-file path. Task 7.
4. **CRLF, UTF-16 BOM and Windows-1252 files that are closed.** The bytes are re-encoded with the same encoding, BOM and dominant line ending, and the line comparison works on the LF-normalised text. Task 3.
5. **An empty Replace string.** The confirm message and the replaced message use the variants without `with '…'`; the preview shows only the struck-through text; matches are deleted. Tasks 5 and 7.

---

## File Structure

- `crates/lucerna-text/src/words.rs` (new): `CharClass`, `classify`, `is_word_bounded`. These move here from `lucerna-editor`.
- `crates/lucerna-editor/src/words.rs`: re-exports `CharClass` and `classify` from `lucerna_text::words` and keeps the motions.
- `crates/lucerna-editor/src/find.rs`: `is_whole_word` calls `lucerna_text::words::is_word_bounded`.
- `crates/lucerna-project/Cargo.toml`: adds `lucerna-text`.
- `crates/lucerna-project/src/search.rs`:
  - `SearchQuery.word_separators`;
  - `build_pattern` loses `\b`;
  - `search_regex` and `line_matches`;
  - the sink filters matches by whole word;
  - `search_text` for re-searching one file.
- `crates/lucerna-project/src/replace_files.rs` (new): the replace job.
- `crates/lucerna-workbench/src/excerpt_list/{mod.rs,model.rs,render.rs}`: header selection (opt-in), `dismiss_line`, `dismiss_file`, the replace-preview render option, and hover action buttons.
- `crates/lucerna-workbench/src/search_view/model.rs`: the dialog and message strings (pure).
- `crates/lucerna-workbench/src/search_view/replace.rs` (new): the workbench side of a replace run (open documents plus the job).
- `crates/lucerna-workbench/src/search_view/view.rs`:
  - the replace row, toggle and Preserve Case;
  - the commands and keys;
  - the replace state.
- `crates/lucerna-workbench/src/documents.rs`: `is_lossy(path)`.
- `crates/lucerna-workbench/src/workbench.rs`: context keys.
- `crates/lucerna-workbench/src/find_widget.rs`: `build_regex` for the Search tab goes through `search_regex`.
- `crates/lucerna-settings/assets/default-keybindings.json`, `app_menu.rs`, docs.
- Tests:
  - `crates/lucerna-project/tests/replace_files.rs` (new);
  - unit tests in `search.rs` and `words.rs`;
  - `crates/lucerna-workbench/tests/search_replace.rs` (new; copy the helpers it needs from `tests/search_view.rs`, which keeps its own).

---

### Task 1: One whole-word rule in `lucerna-text`

**Files:**
- Create: `crates/lucerna-text/src/words.rs`
- Modify:
  - `crates/lucerna-text/src/lib.rs` (`pub mod words;`)
  - `crates/lucerna-editor/src/words.rs` (remove `CharClass`/`classify`, add `pub use lucerna_text::words::{CharClass, classify};`)
  - `crates/lucerna-editor/src/find.rs:140-160`

**Interfaces:**
- Produces:
  - `lucerna_text::words::CharClass` (unchanged variants `Regular`, `Whitespace`, `Separator`);
  - `lucerna_text::words::classify(c: char, separators: &str) -> CharClass`;
  - `lucerna_text::words::is_word_bounded(before: Option<char>, first: Option<char>, last: Option<char>, after: Option<char>, separators: &str) -> bool`.
- Semantics of `is_word_bounded`:
  - `before` and `after` are the chars just outside the match; `None` means the text edge.
  - `first` and `last` are the match's own edge chars.
  - `\n` and `\r` count as non-regular.
  - The rule is exactly today's `is_whole_word`: a side is bounded if it is at the edge, if its neighbour is not regular, or if the match's edge char is not regular.

- [ ] **Step 1: Write failing tests in `crates/lucerna-text/src/words.rs`**

```rust
#[test]
fn bounded_by_separators_and_edges() {
    let seps = "`~!@#$%^&*()-=+[{]}\\|;:'\",.<>/?";
    assert!(is_word_bounded(None, Some('f'), Some('o'), Some(' '), seps));
    assert!(!is_word_bounded(Some('a'), Some('f'), Some('o'), None, seps));
    assert!(is_word_bounded(Some('-'), Some('f'), Some('o'), Some('-'), seps));
    assert!(!is_word_bounded(Some('-'), Some('f'), Some('o'), Some('-'), ""));
    assert!(is_word_bounded(Some('\n'), Some('f'), Some('o'), Some('\r'), seps));
    // The match's own edge char counts: "-foo" inside "a-foo".
    assert!(is_word_bounded(Some('a'), Some('-'), Some('o'), None, seps));
    assert!(!is_word_bounded(Some('x'), Some('đ'), Some('g'), Some(' '), " "));
}
```

- [ ] **Step 2:** Run `cargo test -p lucerna-text words`. Expected: FAIL (module missing).
- [ ] **Step 3:** Create the module and move `CharClass` and `classify` (with their doc comments and VS Code credit) from `lucerna-editor/src/words.rs`. Implement `is_word_bounded`. Rewrite `lucerna_editor::find::is_whole_word` to collect the four chars from the snapshot and call it.
- [ ] **Step 4:** Run `cargo test -p lucerna-text -p lucerna-editor`. Expected: PASS, including every existing `find` and `words` test.
- [ ] **Step 5:** Commit `feat(text): whole-word rule shared by find and project search`.

---

### Task 2: Project search matches by `editor.wordSeparators`

**Files:**
- Modify:
  - `crates/lucerna-project/Cargo.toml` (`lucerna-text.workspace = true`)
  - `crates/lucerna-project/src/search.rs`
  - `crates/lucerna-project/src/lib.rs` (exports)
  - `crates/lucerna-workbench/src/search_view/view.rs` (`run_search` fills `word_separators`)
  - `crates/lucerna-workbench/src/search_view/model.rs` (`first_match`)
  - `crates/lucerna-workbench/src/find_widget.rs:64-75` (`build_regex` for the Search tab: delegate to `search_regex`, drop the `whole_word` param; fix its callers in `view.rs`)

**Interfaces:**
- Produces:
  - `SearchQuery { …, pub word_separators: String }`. The `Default` is an empty string; whole word with empty separators means only whitespace and line edges bound a word.
  - `pub fn build_pattern(pattern: &str, is_regex: bool) -> String`: escapes unless regex, never adds `\b`.
  - `pub fn search_regex(query: &SearchQuery) -> Result<regex::Regex, String>`: case flag, `multi_line(true)`, `crlf(true)`.
  - `pub fn line_matches(regex: &regex::Regex, query: &SearchQuery, line: &str) -> Vec<Range<usize>>`: the non-empty matches in one line (no `\n`), filtered by whole word via `is_word_bounded` when `query.whole_word`. This is the one place that decides "a match".
  - `pub fn search_text(query: &SearchQuery, text: &str) -> Result<Vec<Excerpt>, String>`: the excerpts of one text, as `search` would report the file (same context lines, same `MAX_LINE_CHARS` cut, no `MAX_RESULTS` across files).
- `SearchView` reads `editor.wordSeparators` from `settings.get::<String>("editor.wordSeparators", None)`. If missing, it uses `code_view`'s default constant (make it `pub(crate)` if needed).

- [ ] **Step 1: Write failing tests** in `search.rs` tests (and a temp-dir test through `search` for the sink):
  - `build_pattern("log", false) == "log"`; `build_pattern("a.b", false) == r"a\.b"` (replace the old `whole_word_wraps` test).
  - `line_matches` with `whole_word` and seps `"-"`:
    - `"foo-bar foo"` gives `[0..3, 8..11]`;
    - `"foobar"` gives `[]`;
    - with seps `""`, `"foo-bar"` gives `[]`.
  - `line_matches` ignores empty matches (regex `x*` on `"ab"` gives `[]`).
  - `search` on a file `"foobar\nfoo\n"` with `whole_word` reports exactly one match line, line 2. A line whose every match is filtered is neither counted nor reported as a match.
  - `search_text` on `"a\nfoo\nb"` with `context_lines: 1` gives one excerpt with lines 1–3 and ranges only on line 2.
- [ ] **Step 2:** Run `cargo test -p lucerna-project search`. Expected: FAIL.
- [ ] **Step 3:** Implement.
  - The grep matcher is built from `build_pattern` (no `\b`).
  - In `ExcerptSink::matched`, compute ranges with `line_matches`. If none survive, push the line as context (`is_match = false`) and do not count it.
  - Extract the per-buffer path of `search_file` so `search_text` reuses it.
- [ ] **Step 4:** Run `cargo test -p lucerna-project` and `cargo test -p lucerna-workbench --test search_view`. Expected: PASS. Update any `search_view` test that asserted `\b` behaviour, and say so in the commit.
- [ ] **Step 5:** Commit `feat(project): search whole word by editor.wordSeparators`.

---

### Task 3: The replace job (`lucerna_project::replace_files`)

**Files:**
- Create: `crates/lucerna-project/src/replace_files.rs`
- Create: `crates/lucerna-project/tests/replace_files.rs`
- Modify: `crates/lucerna-project/src/lib.rs`

**Interfaces:**
- Consumes:
  - `search_regex`, `line_matches`, `SearchQuery` (Task 2);
  - `replace::{ReplacePattern, replace_ranges}`;
  - `lucerna_text::{decode_file, check_regular, check_size, Buffer::encode, BufferSnapshot::from_text, write_bytes_tracked, TempFiles, WriteError}`.
- Produces:

```rust
pub struct ShownLine { pub line_number: u32, pub text: String }   // 1-based, text as in LineMatch::line_text
pub struct ReplaceSpec { pub query: SearchQuery, pub pattern: ReplacePattern, pub preserve_case: bool }
pub enum SkipReason {
    ChangedSinceSearch, ChangedOnDisk, Lossy, Unreadable, Unwritable(String),
    Unencodable(&'static str), ChangedWhileReplacing, Missing, Conflict,
}
impl SkipReason { pub fn message(&self) -> String }  // exact spec strings, e.g. "changed since the search",
                                                      // "could not be written: {e}", "cannot encode the replacement in {enc}"
pub enum FileOutcome { Replaced(usize), Skipped(SkipReason) }
pub struct ReplaceProgress { pub done: usize, pub total: usize }

/// Edits for `text` (LF-normalised): `None`-free core shared by open documents and files.
pub fn edits_for_text(text: &str, lines: &[ShownLine], spec: &ReplaceSpec)
    -> Result<Vec<(Range<usize>, String)>, SkipReason>;
/// One closed file: read, check, edit, encode, re-check, write.
pub fn replace_file(path: &Path, lines: &[ShownLine], spec: &ReplaceSpec, temps: &TempFiles) -> FileOutcome;
/// Every file, on up to `available_parallelism().min(8)` threads; blocking. Stops taking files once `cancel` is set.
pub fn replace_files(files: Vec<(PathBuf, Vec<ShownLine>)>, spec: &ReplaceSpec, temps: &TempFiles,
    cancel: &AtomicBool, progress: &dyn Fn(ReplaceProgress)) -> Vec<(PathBuf, FileOutcome)>;
```

- Rules for `edits_for_text`:
  - Every shown line must equal the first `line.text.len()` bytes of the text's line `line_number`, else `Err(ChangedSinceSearch)`. A missing line counts as changed.
  - Matches come from `line_matches` on the full line. Keep those whose start is `< line.text.len()`.
  - Edits are computed with `replace_ranges` on the whole text, so captures and lookarounds see the real context. Byte offsets are whole-text offsets.
- Rules for `replace_file`:
  - Read the stamp and bytes.
  - Skip with `Unreadable` if `check_regular`, `check_size` or `decode_file(bytes, false)` fails (binary included), and with `Lossy` if `lossy`.
  - Call `edits_for_text`; with no edits, return `Replaced(0)` without writing.
  - Apply the edits to a `String`, encode with the decoded encoding, BOM and line ending; `Unencodable` becomes `Skipped(Unencodable(name))`.
  - Re-check the stamp; if it differs, hash the bytes; if the hash differs, return `Skipped(ChangedOnDisk)`.
  - Write with `write_bytes_tracked`; a missing file is `Missing`.
  - Add a `#[doc(hidden)] pub fn replace_file_with_hook(…, before_write: &dyn Fn())` so the test can modify the file between the read and the write. `replace_file` calls it with a no-op.

- [ ] **Step 1: Write failing tests** in `tests/replace_files.rs` (tempdir). Each asserts the bytes on disk and the outcome:
  - `replaces_only_shown_lines`: file `"foo\nfoo\nfoo\n"`, lines `[(1,"foo"),(3,"foo")]`, pattern `"bar"` gives `"bar\nfoo\nbar\n"` and `Replaced(2)`.
  - `skips_file_when_a_line_changed`: shown `(2,"foo")`, but the file's line 2 is `"fob"`. Gives `Skipped(ChangedSinceSearch)` and the bytes are unchanged.
  - `skips_file_changed_before_write`: the hook rewrites the file. Gives `Skipped(ChangedOnDisk)`, and the hook's bytes stay.
  - `keeps_crlf_bom_and_encoding`: CRLF UTF-8, UTF-16LE with BOM, and Windows-1252 `"café foo\r\n"` all round-trip with only `foo` changed to `bar`.
  - `skips_lossy_and_binary`: invalid UTF-16 BOM file gives `Lossy`; NUL bytes give `Unreadable`; both are byte-identical afterwards.
  - `unencodable_replacement`: Windows-1252 file, replacement `"đường"`, gives `Skipped(Unencodable("windows-1252"))`.
  - `ignores_matches_past_shown_prefix`: line `"x".repeat(1000) + "foo"` shown with `text` of 1000 chars gives `Replaced(0)` and the file is unchanged.
  - `regex_groups_and_preserve_case`:
    - `(\w+)_id` → `$1Id` on `"user_id"` gives `"userId"`;
    - preserve case `foo` → `bar` on `"Foo FOO"` gives `"Bar BAR"`;
    - `"Đường"` with preserve case and replacement `"phố"` gives `"Phố"`.
  - `cross_line_matches_not_replaced`: regex `a\nb` on `"a\nb"` gives `Replaced(0)`.
  - `cancel_stops_remaining_files`: with `cancel` set before the call, no file is touched and the result is empty.
  - `progress_reports_each_file`: three files give final progress `done == total == 3`.
- [ ] **Step 2:** Run `cargo test -p lucerna-project --test replace_files`. Expected: FAIL.
- [ ] **Step 3:** Implement `replace_files.rs` per the interfaces.
- [ ] **Step 4:** Run `cargo test -p lucerna-project`. Expected: PASS.
- [ ] **Step 5:** Commit `feat(project): replace job for closed files`.

---

### Task 4: Excerpt list: selectable headers and Dismiss

**Files:**
- Modify:
  - `crates/lucerna-workbench/src/excerpt_list/model.rs`
  - `crates/lucerna-workbench/src/excerpt_list/mod.rs`
  - `crates/lucerna-workbench/src/excerpt_list/render.rs` (header selected background)
- Test: unit tests in `model.rs`, plus `crates/lucerna-workbench/tests/excerpt_list_diff.rs` stays green.

**Interfaces:**
- Produces:
  - `Results::dismiss_line(&mut self, r: LineRef)`:
    - removes that line;
    - an excerpt left with no match line is dropped;
    - a file left with no excerpt is dropped;
    - `match_count` and `revision` update.
  - `Results::dismiss_file(&mut self, file: usize)`.
  - `Results::occurrence_count(&self) -> usize`: the sum of non-empty ranges, plus 1 for a match line whose only range is empty (cut).
  - `pub enum ListSelection { Line(LineRef), Header(usize) }`.
  - `ExcerptList::set_headers_selectable(&mut self, on: bool)` (Search tab: true; diff view: default false).
  - `ExcerptList::selection(&self) -> Option<ListSelection>`. `selected()` keeps returning only a line.
  - `ExcerptList::select_header(&mut self, file: usize, cx)`.
  - `ExcerptList::dismiss_selected(&mut self, cx)`. Afterwards it selects the next row of the same kind, else the previous, else nothing.
  - `ExcerptListEvent::ActivateHeader(usize)`: Enter on a selected header. The Search tab toggles the file's fold, as a header click does.
- `↑`/`↓` with headers selectable visit headers and lines (never separators or notes). F4 and `‹ ›` (`step_match`) still visit only match lines.

- [ ] **Step 1: Write failing tests** (model unit tests):
  - `dismiss_line_drops_empty_excerpt_and_file`;
  - `dismiss_line_keeps_context_of_other_matches`;
  - `occurrence_count_counts_ranges`: two ranges on one line plus one cut line gives 3.
  - Then a gpui test in `tests/search_replace.rs` (create the file with helpers): `arrows_stop_on_headers_in_search`. Results of two files: `↓` from nothing selects the header of file 0, then line 1, and so on.
- [ ] **Step 2:** Run the tests. Expected: FAIL.
- [ ] **Step 3:** Implement. Store the header selection as the file's path (as `LineAnchor` stores a path) so streamed inserts do not move it.
- [ ] **Step 4:** Run `cargo test -p lucerna-workbench --test search_view --test excerpt_list_diff --test search_replace` plus the lib unit tests. Expected: PASS.
- [ ] **Step 5:** Commit `feat(search): selectable file headers and dismiss in the results list`.

---

### Task 5: Replace row, toggle, ⇧⌘H, preview

**Files:**
- Modify:
  - `crates/lucerna-workbench/src/search_view/view.rs`
  - `crates/lucerna-workbench/src/excerpt_list/{mod.rs,render.rs}` (preview option)
  - `crates/lucerna-workbench/src/workbench.rs` (context keys)
  - `crates/lucerna-settings/assets/default-keybindings.json`
  - `crates/lucerna-workbench/src/app_menu.rs:277` (add `command("Replace in Files", "workbench.action.replaceInFiles")` after Find in Files)
- Test: `crates/lucerna-workbench/tests/search_replace.rs`

**Interfaces:**
- Consumes: `search_regex`, `line_matches` (Task 2); `ReplacePattern`, `replacement_at`.
- Produces on `SearchView`:
  - `replace_input(&self) -> &Entity<TextInput>`;
  - `replace_visible(&self) -> bool`;
  - `preserve_case(&self) -> bool`;
  - `set_replace_visible(&mut self, on: bool, window, cx)` (focus as spec: on → Replace input, off → query);
  - `toggle_preserve_case(&mut self, cx)`;
  - `replace_input_focused(&self, window, cx) -> bool`;
  - `replace_active(&self, cx) -> bool` (visible and `results.match_count() > 0`).
- `pub struct ReplacePreview { regex: Regex, query: SearchQuery, pattern: ReplacePattern, preserve: bool }` in `excerpt_list` (or `search_view/model.rs`), with `fn replacements(&self, line: &LineMatch) -> Vec<Option<String>>`, one entry per `line.ranges` entry (`None` when `replacement_at` fails on `line_text`).
- `ExcerptList::set_replace_preview(&mut self, preview: Option<Arc<ReplacePreview>>, cx)`. Render: for each range, the old text gets `strikethrough` on the diff-removed background, followed by the replacement on the diff-inserted background. Cache per line keyed by `Arc::as_ptr(preview)`. It is set only by the Search tab, when the row is visible and the Replace text is non-empty.
- Commands (register in `search_view::register_commands`):
  - `workbench.action.replaceInFiles` "Search: Replace in Files": `find_in_files`, then `set_replace_visible(true)`, focusing Replace if the query is non-empty, else the query.
  - `toggleSearchPreserveCase` "Search: Toggle Preserve Case".
  - `closeReplaceInFilesWidget` "Search: Close Replace Widget": hide and focus the query.
- Context keys (`workbench.rs`, next to `searchViewletFocus`): `replaceActive`, `replaceInputBoxFocus`. (`matchFocus`, `fileMatchFocus` and `fileMatchOrMatchFocus` come in Task 8.)
- Keybindings block, appended (Task 7 and Task 8 append their rows to the same block):

```json
{ "key": "shift+cmd+h", "command": "workbench.action.replaceInFiles" },
{ "key": "alt+cmd+p", "command": "toggleSearchPreserveCase", "when": "searchViewletFocus" },
{ "key": "escape", "command": "closeReplaceInFilesWidget", "when": "searchViewletFocus && replaceInputBoxFocus" }
```

- UI details:
  - Toggle button `search-toggle-replace` (icon `REPLACE`, tooltip `Toggle Replace`) after the filter button.
  - Replace row `search-replace-row` between the query and filter rows:
    - a spacer `w(6*GRID)`;
    - the Replace input (`search-replace`, placeholder `Replace`) with toggle `search-toggle-preserve-case` (`AB`, tooltip `Preserve Case`);
    - `bar_separator`;
    - `bar_icon_button("search-replace-all", REPLACE_ALL, …)` (tooltip `Replace All`), disabled unless `replace_active` and the search is done and no replace is running.
  - Tab in the query moves to Replace when the row is visible; Shift+Tab in Replace goes back to the query.
  - Enter in Replace runs `run_search` now.
  - Typing in Replace or toggling `AB` rebuilds the preview only; it never searches.

- [ ] **Step 1: Write failing gpui tests** in `search_replace.rs`:
  - `cmd_shift_h_opens_replace_row`:
    - keys `cmd-shift-h` show `search-replace-row`;
    - focus is in the query (empty);
    - with an editor selection `foo`, the query is `foo` and focus is in Replace.
  - `toggle_replace_button_and_escape`: clicking `search-toggle-replace` hides the row; `escape` in Replace hides it and focuses the query.
  - `tab_moves_between_query_and_replace`.
  - `preview_strikes_old_and_shows_new`:
    - query `(\w+)_id` (regex), replace `$1Id`, file `"user_id"`;
    - a debug selector `search-preview-0-0-0` exists;
    - `ReplacePreview::replacements` for that line is `[Some("userId")]`.
  - `diff_view_has_no_preview`: an existing diff view test helper; no preview selector is drawn.
  - `typing_replace_does_not_search`: the search generation is unchanged after typing in Replace.
  - `alt_cmd_p_toggles_preserve_case_in_search_only`: in the Search tab `alt-cmd-p` flips `preserve_case()`; in a code view it still runs the find bar's `togglePreserveCase`.
- [ ] **Step 2:** Run `cargo test -p lucerna-workbench --test search_replace`. Expected: FAIL.
- [ ] **Step 3:** Implement per the interfaces.
- [ ] **Step 4:** Run `cargo test -p lucerna-workbench --test search_replace --test search_view --test find_widget --test app_menu`. Expected: PASS.
- [ ] **Step 5:** Commit `feat(search): replace row, preview and replaceInFiles`.

---

### Task 6: Dialog and message strings

**Files:**
- Modify: `crates/lucerna-workbench/src/search_view/model.rs` (pure functions and unit tests)

**Interfaces:**
- Produces (credit VS Code `searchView.ts` `buildReplaceAllConfirmationMessage` / `buildAfterReplaceAllMessage`, MIT):
  - `pub fn replace_all_confirmation(occurrences: usize, files: usize, value: &str) -> String`
  - `pub const REPLACE_ALL_TRUNCATED_DETAIL: &str = "The results only contain a subset of all matches. Files not in the results are not changed.";`
  - `pub fn replaced_message(occurrences: usize, files: usize, value: &str) -> String`
  - `pub fn skipped_message(skipped: &[(String, String)]) -> Option<String>`: `(file name, reason message)`, at most 5 then `…`.
  - `pub fn unsaved_message(files: usize) -> Option<String>`

- [ ] **Step 1: Write failing tests**, with exact strings:

```rust
assert_eq!(replace_all_confirmation(1, 1, "x"), "Replace 1 occurrence across 1 file with 'x'?");
assert_eq!(replace_all_confirmation(2, 1, "x"), "Replace 2 occurrences across 1 file with 'x'?");
assert_eq!(replace_all_confirmation(1, 3, "x"), "Replace 1 occurrence across 3 files with 'x'?");
assert_eq!(replace_all_confirmation(5, 3, ""), "Replace 5 occurrences across 3 files?");
let long = (1..=12).map(|i| i.to_string()).collect::<Vec<_>>().join("\n");
assert_eq!(replace_all_confirmation(2, 2, &long),
    format!("Replace 2 occurrences across 2 files with '{}\n...'?", (1..=10).map(|i| i.to_string()).collect::<Vec<_>>().join("\n")));
assert_eq!(replaced_message(1, 1, "x"), "Replaced 1 occurrence across 1 file with 'x'.");
assert_eq!(replaced_message(4, 2, ""), "Replaced 4 occurrences across 2 files.");
assert_eq!(replaced_message(4, 2, &long), format!("Replaced 4 occurrences across 2 files with '{long}'."));
assert_eq!(skipped_message(&[("a.rs".into(), "changed on disk".into())]).as_deref(),
    Some("Skipped 1 file: a.rs (changed on disk)"));
assert_eq!(unsaved_message(2).as_deref(), Some("2 files with unsaved changes were not saved."));
assert_eq!(unsaved_message(1).as_deref(), Some("1 file with unsaved changes was not saved."));
```

  - Seven skipped files list five, then `, …`. The prefix is `Skipped 7 files: `.
- [ ] **Step 2:** Run `cargo test -p lucerna-workbench --lib search_view::model`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the same command. Expected: PASS.
- [ ] **Step 5:** Commit `feat(search): replace all messages`.

---

### Task 7: Replace All

**Files:**
- Create: `crates/lucerna-workbench/src/search_view/replace.rs`
- Modify:
  - `crates/lucerna-workbench/src/search_view/{mod.rs,view.rs}`
  - `crates/lucerna-workbench/src/documents.rs` (`pub fn is_lossy(&self, path: &Path) -> bool`)
  - keybindings block
- Test: `crates/lucerna-workbench/tests/search_replace.rs`

**Interfaces:**
- Consumes:
  - `replace_files`, `edits_for_text`, `ShownLine`, `ReplaceSpec`, `FileOutcome`, `SkipReason` (Task 3);
  - the messages (Task 6);
  - `Documents::{snapshot_of, revision_of, state, is_dirty, has_conflict, is_lossy, apply_edits, push_stop, save}`;
  - `save::{SaveOptions, SaveReason, save_temps}`.
- Produces:
  - `pub(crate) struct ReplaceTarget { pub path: Arc<Path> /* root-relative, as in Results */, pub lines: Vec<ShownLine> }`.
  - `pub(crate) struct ReplaceReport { pub replaced: usize, pub files: usize, pub skipped: Vec<(String, SkipReason)>, pub unsaved: usize }`.
  - `pub(crate) fn run_replace(targets: Vec<ReplaceTarget>, spec: ReplaceSpec, documents: Entity<Documents>, root: PathBuf, window: &mut Window, cx: &mut App) -> Task<ReplaceReport>`, with progress through a callback `on_progress: impl Fn(ReplaceProgress, &mut App) + 'static`.
  - `SearchView::replace_all(&mut self, window, cx)`, `SearchView::is_replacing(&self) -> bool`.
- `run_replace` flow:
  1. Split the targets.
     - A path is an **open document** if `documents.state(path)` is `Some(Ready)` and a snapshot exists.
     - An open document that is `Loading` or `Error` is skipped (`Unreadable`).
     - Everything else goes to the closed-file job.
  2. The closed files go to `replace_files` on `cx.background_spawn`. Progress is sent through a channel and applied in batches.
  3. For each open document:
     - Skip it with `Conflict` if `has_conflict`, and with `Lossy` if `is_lossy`.
     - Otherwise take the snapshot and revision, and compute `edits_for_text` on `background_spawn`.
     - Back on the foreground: if the revision is unchanged, `push_stop`, `apply_edits(..)`, `push_stop`. If it changed, recompute once; if it changed again, skip with `ChangedWhileReplacing`.
     - Remap each view's selections through the edits, as `find_widget::apply_replacements` does: build the plan through the active `CodeView`'s `transact` when a view exists, else `Documents::apply_edits`.
     - Apply one document per foreground turn (yield between documents).
  4. For a document that was clean before:
     - `documents.save(path, SaveOptions { reason: SaveReason::Auto, participants: false, overwrite: false }, window, cx)`.
     - If the result is `Ok(false)` and `has_conflict(path)` holds, it is `ChangedOnDisk`. If it is `Ok(false)` and the path does not exist, it is `Missing`. `Err(e)` is `Unwritable(e.to_string())`.
     - In every one of these cases the edit stays in the buffer.
  5. A document that was dirty before is not saved; `unsaved += 1`.
- `SearchView::replace_all`:
  - It does nothing unless `replace_active`, `done.is_some()`, `!is_replacing()` and `!stale`. If `stale`, it calls `run_search` instead.
  - It prompts with `window.prompt(PromptLevel::Warning, &replace_all_confirmation(results.occurrence_count(), files, value), detail, &["Replace", "Cancel"], cx)`. The `detail` is `REPLACE_ALL_TRUNCATED_DETAIL` when `done == Some(true)`.
  - Only answer `0` proceeds.
  - The targets are every file's match lines (`ranges` non-empty) as `ShownLine { line_number, text: line_text }`.
  - While it runs, the counter reads `Replacing… {done}/{total} files`.
  - When it ends, if no search started since (compare `generation`):
    - clear the results;
    - show `centered_message` with `replaced_message(..)`, plus `skipped_message` and `unsaved_message` lines below it;
    - set `last_replace_message: Option<String>`, which is cleared by the next search.
  - Dropping `SearchView` or the window sets the job's cancel flag.
- Keybinding appended: `{ "key": "alt+cmd+enter", "command": "search.action.replaceAll", "when": "searchViewletFocus && replaceActive" }`. Command title `Search: Replace All`. The find bar's `alt+cmd+enter` row must not fire in the Search tab; if the test shows it does, add `&& !searchViewletFocus` to the find bar row (the only allowed edit of an existing row, as Find & Replace did for its toggles).

- [ ] **Step 1: Write failing gpui tests** in `search_replace.rs`:
  - `replace_all_asks_then_writes`:
    - two closed files `a.txt` (`"foo foo\n"`) and `b.txt` (`"foo\n"`), query `foo`, replace `bar`;
    - `alt-cmd-enter` gives `has_pending_prompt()`;
    - `simulate_prompt_answer("Cancel")` leaves the files unchanged;
    - again, then `"Replace"`: the files are `"bar bar\n"` and `"bar\n"`, the results are empty, and the message is `"Replaced 3 occurrences across 2 files with 'bar'."`
  - `replace_all_button_disabled_without_results`.
  - `replace_all_open_clean_document_undoable`:
    - `a.txt` open, clean;
    - after Replace All, the disk has `bar`, the document is not dirty, and one `cmd-z` in its view restores `foo`.
  - `replace_all_dirty_document_not_saved`:
    - `a.txt` open with an unsaved edit adding `"foo\n"` at its end;
    - after replace, the buffer has `bar` everywhere, it is still dirty, the disk is unchanged, and the message has `1 file with unsaved changes was not saved.`;
    - one undo returns the buffer to its earlier dirty text.
  - `replace_all_skips_conflicted_document`: an open document in conflict stays unchanged, and the message has `Skipped 1 file: a.txt (has a conflict with the file on disk)`.
  - `replace_all_skips_file_changed_since_search`: after the search, rewrite `b.txt` on disk to `"x\nfoo\n"`; `b.txt` is unchanged and listed as `changed since the search`; `a.txt` is replaced.
  - `replace_all_empty_value_message`: replace `""` gives `"Replaced 3 occurrences across 2 files."`
  - `new_search_during_replace_keeps_its_results`.
  - `find_bar_alt_cmd_enter_still_works_in_editor`.
- [ ] **Step 2:** Run `cargo test -p lucerna-workbench --test search_replace`. Expected: FAIL.
- [ ] **Step 3:** Implement per the interfaces.
- [ ] **Step 4:** Run `cargo test -p lucerna-workbench --test search_replace --test search_view --test find_widget --test save`. Expected: PASS.
- [ ] **Step 5:** Commit `feat(search): replace all across files`.

---

### Task 8: Per-line and per-file replace, Dismiss, hover buttons

**Files:**
- Modify:
  - `crates/lucerna-workbench/src/search_view/view.rs`
  - `crates/lucerna-workbench/src/excerpt_list/render.rs` (hover buttons, opt-in)
  - `crates/lucerna-workbench/src/workbench.rs`
  - keybindings block
- Test: `crates/lucerna-workbench/tests/search_replace.rs`

**Interfaces:**
- Consumes:
  - `run_replace` (Task 7) with one target;
  - `search_text` (Task 2) to re-search the replaced file;
  - `ExcerptList::{selection, dismiss_selected, set_file, remove_files}` (Task 4).
- Produces:
  - `SearchView::replace_selected(&mut self, window, cx)`:
    - a line selection replaces that line;
    - a header selection replaces the whole file.
  - `SearchView::replace_line(&mut self, r: LineRef, window, cx)`.
  - `SearchView::replace_file(&mut self, file: usize, window, cx)`.
  - `SearchView::dismiss_selected(&mut self, cx)`.
  - `ExcerptList::set_row_actions(&mut self, actions: Option<RowActions>)`, where `RowActions { replace: bool }`:
    - when `replace` is set, hovering a match line shows `[REPLACE] [X]` (selectors `search-line-replace-…`, `search-line-dismiss-…`);
    - when it is not set, only `[X]` shows;
    - headers show `[REPLACE_ALL] [X]` before Open File;
    - clicks emit `ExcerptListEvent::{ReplaceLine(LineRef), ReplaceFile(usize), DismissLine(LineRef), DismissFile(usize)}`.
- After a line replace, the file is re-searched with `search_text` on its current text (the document's snapshot if open, else the disk, read in the background) and `set_file` (or `remove_files` if empty). The selection moves to the next match line, else the previous.
- After a file replace, the file is removed and its successor's first match line is selected.
- A skipped result shows in the status bar via `status_message::show`:
  - `changed since the search` gives `The file has changed since the search. Search again to replace it.`;
  - other reasons give `Could not replace in {name}: {reason}`;
  - a dirty document gives `Replaced in {name}. The file has unsaved changes.`
- Context keys:
  - `matchFocus`: the list is focused and the selection is a line with ranges;
  - `fileMatchFocus`: the list is focused and the selection is a header;
  - `fileMatchOrMatchFocus`.
- Commands:
  - `search.action.replace` "Search: Replace";
  - `search.action.replaceAllInFile` "Search: Replace All in File";
  - `search.action.remove` "Search: Dismiss".
- Keybindings appended:

```json
{ "key": "shift+cmd+1", "command": "search.action.replace", "when": "searchViewletFocus && replaceActive && matchFocus" },
{ "key": "shift+cmd+1", "command": "search.action.replaceAllInFile", "when": "searchViewletFocus && replaceActive && fileMatchFocus" },
{ "key": "shift+cmd+enter", "command": "search.action.replaceAllInFile", "when": "searchViewletFocus && replaceActive && fileMatchFocus" },
{ "key": "cmd+backspace", "command": "search.action.remove", "when": "searchViewletFocus && fileMatchOrMatchFocus" }
```

  Check that the find bar's `shift+cmd+1` row (`editorFocus && findWidgetVisible`) does not win in the Search tab. If it does, add `&& !searchViewletFocus` to it.

- [ ] **Step 1: Write failing gpui tests:**
  - `shift_cmd_1_replaces_selected_line`:
    - file `"foo\nx\nfoo\n"`; select line 1 with ↓;
    - `shift-cmd-1` makes the disk `"bar\nx\nfoo\n"`;
    - the results show one match line (3), which is selected;
    - there is no prompt.
  - `replace_line_refuses_changed_line`: the disk line changed after the search; the file is unchanged and the status bar shows the spec text.
  - `shift_cmd_1_on_header_replaces_file`: the header is selected; the file is replaced and removed from results; the next file's first match is selected.
  - `cmd_backspace_dismisses_line_and_file`: nothing on disk changes; the counts drop.
  - `hover_buttons_only_with_replace_row`: with the row hidden, `search-line-replace-0-0-0` is not drawn but the dismiss button is.
  - `replace_line_in_dirty_document_reports_unsaved`.
- [ ] **Step 2:** Run `cargo test -p lucerna-workbench --test search_replace`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test -p lucerna-workbench --test search_replace --test search_view --test find_widget`. Expected: PASS.
- [ ] **Step 5:** Commit `feat(search): replace and dismiss per line and per file`.

---

### Task 9: Performance check, docs, full verification

**Files:**
- Modify:
  - `crates/lucerna-project/tests/replace_files.rs` (timing test)
  - `docs/keyboard-shortcuts.md`
  - `CHANGELOG.md` (lines appended under Unreleased)
  - `docs/roadmap.md` (`- [x] Replace in project (Search)`)

- [ ] **Step 1:** Add `replace_files_2000_files` (`#[ignore]`-free but fast): 2,000 files of 10 lines each, all shown. Assert every file reads `bar` afterwards, and print the elapsed time with `eprintln!`. The UI-thread budget is covered by design (job on the background executor). Note the measured time in the commit message.
- [ ] **Step 2:** Docs:
  - `keyboard-shortcuts.md`: add rows for ⇧⌘H, ⌥⌘Enter, ⇧⌘1, ⇧⌘Enter, ⌘⌫ and ⌥⌘P (Search), each with its `when`.
  - `CHANGELOG.md`: `- Replace in Files (⇧⌘H): Replace All across the project, per file and per line, with a preview in the Search tab.`
  - Roadmap checkbox.
- [ ] **Step 3:** Run the full verification:

```bash
scripts/prune-target.sh
PATH="/opt/homebrew/opt/rustup/bin:$PATH" cargo test -p lucerna-text -p lucerna-project -p lucerna-editor -p lucerna-workbench
PATH="/opt/homebrew/opt/rustup/bin:$PATH" cargo clippy --workspace --all-targets -- -D warnings
PATH="/opt/homebrew/opt/rustup/bin:$PATH" cargo fmt --all --check
PATH="/opt/homebrew/opt/rustup/bin:$PATH" cargo deny check
```

  Expected: every test passes, with no clippy warnings, no fmt diff, and `cargo deny` reporting ok.
- [ ] **Step 4:** Commit `docs: replace in files shortcuts, changelog, roadmap`.
