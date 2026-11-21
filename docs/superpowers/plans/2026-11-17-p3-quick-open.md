# Fixing the P3 bugs of group 3 (Quick Open, search, recent): plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the four P3 bugs of group 3 (T3-3, T5-1, T5-4, T5-5) on the branch `claude/p3-quick-open`, each with a test, then mark them "Fixed" in the review.

**Architecture:** Four independent fixes, each at the exact place the review points to. T5-5 touches only `lucerna-project` (the matcher and the searcher both use CRLF). T5-4 changes Quick Open's parser to `i64` and lets the view compute the real line from the editor's line count. T5-1 makes the `reveal` closure of the Search tab finish even when the line no longer exists. T3-3 stops filtering when writing and when displaying the recent list, then does what VS Code does when an entry that is no longer on disk is chosen: remove the entry from the list and report "Path does not exist" with a dialog that has only an OK button.

**Tech Stack:** Rust, GPUI (only in `lucerna-workbench`), `grep-regex` 0.1.14 / `grep-searcher` 0.1.17 / `grep-matcher`.

**Spec:** `docs/superpowers/specs/2026-11-17-p3-fixes-design.md`, §5 "Group 3". Location and reproduction: `docs/superpowers/reviews/2026-11-15-release-flow-review.md`, items T3-3, T5-1, T5-4, T5-5.

## Global Constraints

- Text shown in the app is in English only (`docs/design-principles.md` §3). Vietnamese appears only in test data.
- Only `lucerna-workbench` uses gpui. Make no macOS assumptions outside platform-specific code.
- Behavior follows VS Code, UI follows Zed. Use the existing prompt and status message, and add no new kind of UI.
- Cargo:
  - Run `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"` first.
  - Run `scripts/prune-target.sh` before every build or test.
  - Before `cargo test`, delete `target/release` if it exists, but only when `pgrep -x cargo` prints nothing.
  - Never set `CARGO_TARGET_DIR` or pass `--target-dir`; every worktree shares the main checkout's `target` (`scripts/target-dir.sh`). If the cargo lock is held, wait.
  - Keep more than 1.2 GB of disk free.
- Commit: stage each file by its explicit path (`git add <path>…`, `git commit -- <paths>`), never `git add -A`. Trailer: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- `cargo deny check` must pass (this plan adds no dependency).
- Do not touch `lucerna-vim` or the V2b `:` command-line parser. `parse_query` is the meaning **when Vim is off**; V2b writes its own parse function (spec §5 T5-4, vim spec §8).

## References (VS Code, `microsoft/vscode` branch `main`, read on 2026-11-17)

- **`src/vs/editor/contrib/quickAccess/browser/gotoLineQuickAccess.ts`, `parsePosition`:**
  - Splits with `value.split(/,|:|#/)`, takes `parts[0]` as the line and `parts[1]` as the column; the rest is dropped.
  - A negative line: `lineNumber = (maxLine + 1) + lineNumber`, then clamped into `[1, maxLine]`. So `:0` becomes line 1 and `:-1` is the last line.
  - A negative column also counts from the end of the line. Our spec does not mention it; this plan keeps the column non-negative (see "Notes").
  - `::N` is offset mode, which Lucerna does not have.
  - Lucerna counts lines by `BufferSnapshot::line_count()`: a trailing newline in the file does not add a line. So `:-1` is the last line with text, while VS Code goes to the empty line after that newline. Keep Lucerna's convention, the same way `:999` is clamped today.
- **A recent entry that no longer exists:**
  - VS Code does not filter when displaying: Welcome, the menu and the picker show every entry.
  - When an entry that no longer exists is opened, `windowsMainService.ts` (`doExtractPathsFromAPI`) shows a box of `type: 'info'` with only an **OK** button, with the message "Path does not exist" and the detail "The path '{0}' does not exist on this computer." (the path with `~`).
  - The entry is removed automatically, without asking: `resolveOpenable` calls `removeRecentlyOpened`, and `menubar.ts` also removes it when `open` fails.
  - Lucerna does exactly that (the controller decided on 2026-11-17 and fixed the spec: the Remove / Cancel sentence in the spec was a mistake). The dialog has the message "Path does not exist" and the detail "The path '{0}' does not exist on this computer.", where `{0}` is the path with `~`.

## Notes for the controller (differences from or gaps in the spec)

1. **T3-3, follow VS Code instead of the spec.** The first version of the spec said a Remove / Cancel dialog, but VS Code does not ask. The controller decided to follow VS Code and fixed the spec: remove the entry at once, then report with a dialog that has only **OK** (see "References").
2. **T3-3, the Open Recent menu when no window exists** (macOS, every window closed): there is no window to show a dialog in, because GPUI only has `Window::prompt`. The plan removes the entry like VS Code's `menubar.ts`, opening nothing and reporting nothing. The spec does not mention this case.
3. **T5-5, `.crlf(true)` alone breaks every search.** A matcher with `crlf(true)` reports a CRLF line terminator. `Searcher` uses LF by default, so it returns `ConfigError::MismatchedLineTerminators` for every file. `search_file` logs this error only at debug level, so the result is empty with no warning. So `SearcherBuilder::line_terminator(LineTerminator::crlf())` must be set as well, like `rg --crlf`. Lines are still split on `\n`, so LF files do not change.
   **Superseded:** this approach is wrong, a CRLF searcher breaks `$` on LF files. The shipped version (commit 45319d7) builds the matcher with `multi_line(true)`, `crlf(true)`, `line_terminator(Some(b'\n'))` and keeps the LF searcher.
4. **T5-4, a negative column** (`:5:-1`) is still treated as no column, that is, go to the start of the line. The spec only talks about lines.
5. **T5-1, a doc that failed to load:** the plan also removes the observer when the doc is in `DocState::Error`. Otherwise a later successful reload would make the cursor jump. The spec only mentions `Ready`.

## Review Focus

1. **A drive is pulled, the user chooses that entry, then plugs the drive back in:** the entry has been removed from the list and does not come back by itself. This is VS Code's behavior. The entry only returns when the folder is opened again (Open Folder, `luc`), and then it is at the top of the list. → test `missing_folder_is_removed_and_reported` in Task 5.
2. **`:0`, `:-N` larger than the line count, `:12,` or `:,5`:** no panic, the line is clamped into `[1, line count]`, and the label shows the line it will really go to. → tests in Task 2.
3. **An LF file and a non-regex search after the searcher switches to CRLF:** still gives the same results as before. → `literal_and_lf_searches_still_match` in Task 1.
4. **A search result that points into a file that can no longer be read (Error):** the observer is removed, so when the file becomes readable again the cursor does not jump. → test in Task 3.
5. **A recent entry of kind File (not a folder) that no longer exists, in a workbench window:** it is also removed and reported like a folder, and no empty tab opens. → test `missing_file_in_picker_is_removed_and_reported` in Task 5.

---

### Task 1: T5-5, regex `$` and `^` match CRLF files in the Search tab

**Files:**
- Modify: `crates/lucerna-project/src/search.rs` (imports at lines 9–11; `RegexMatcherBuilder` at lines 137–139; `SearcherBuilder` at lines 184–188)
- Test: `crates/lucerna-project/tests/search.rs`

**Interfaces:**
- Consumes: none.
- Produces: no new API. `search(...)` keeps its signature.

- [ ] **Step 1: Write the failing tests** in `tests/search.rs`, using the existing helpers `write`, `query`, `run`, `files`:

```rust
#[test]
fn regex_anchors_match_crlf_lines() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "crlf.txt", "foo\r\nbar foo\r\n");
    let mut q = query("foo$");
    q.is_regex = true;
    let f = files(dir.path(), &run(dir.path(), &q));
    assert_eq!(f.len(), 1);
    let got: Vec<_> = f[0].1.iter()
        .map(|l| (l.line_number, l.line_text.clone(), l.ranges.clone()))
        .collect();
    assert_eq!(got, vec![(1, "foo".into(), vec![0..3]), (2, "bar foo".into(), vec![4..7])]);

    let mut q = query("^bar");
    q.is_regex = true;
    let f = files(dir.path(), &run(dir.path(), &q));
    assert_eq!(f[0].1.iter().map(|l| l.line_number).collect::<Vec<_>>(), [2]);
}

#[test]
fn literal_and_lf_searches_still_match() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "crlf.txt", "foo\r\nbar\r\n");
    write(dir.path(), "lf.txt", "foo\nbar\n");
    // Literal, both files.
    let f = files(dir.path(), &run(dir.path(), &query("bar")));
    assert_eq!(f.iter().map(|(n, l)| (n.as_str(), l.len())).collect::<Vec<_>>(),
               [("crlf.txt", 1), ("lf.txt", 1)]);
    // Regex `$` on the LF file.
    let mut q = query("foo$");
    q.is_regex = true;
    let f = files(dir.path(), &run(dir.path(), &q));
    assert_eq!(f.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["crlf.txt", "lf.txt"]);
}
```

- [ ] **Step 2: Run, see them fail.** `cargo test -p lucerna-project --test search regex_anchors_match_crlf_lines` → FAIL (`f.len()` is 0). The second test passes at this point; it guards Step 3.

- [ ] **Step 3: Fix `search()` in `src/search.rs`.**
  - Add `.crlf(true)` to `RegexMatcherBuilder::new()`.
  - Add `.line_terminator(LineTerminator::crlf())` to `SearcherBuilder::new()`, with `use grep_matcher::LineTerminator;`.
  - **Superseded:** the steps below are no longer right (a CRLF searcher breaks `$` on LF files). The shipped version (commit 45319d7) builds the matcher with `multi_line(true)`, `crlf(true)`, `line_terminator(Some(b'\n'))` and keeps the LF searcher.
  - Add one comment line: the matcher and the searcher must use the same line terminator, otherwise the searcher rejects every file (Note 3).
  - `line_match` already trims `\n` and `\r` before taking the range, so nothing to fix there.

- [ ] **Step 4: Run.** `cargo test -p lucerna-project --test search` → every test PASSES, including both new tests and old ones such as `regex_mode`, `context_*`, `unsaved_buffer_*`.

- [ ] **Step 5: Commit.**

```bash
git add crates/lucerna-project/src/search.rs crates/lucerna-project/tests/search.rs
git commit -m "fix(search): regex anchors match CRLF lines (T5-5)" -- crates/lucerna-project/src/search.rs crates/lucerna-project/tests/search.rs
```
(the message carries the trailer from Global Constraints; later commits too.)

---

### Task 2: T5-4, Quick Open accepts `:-N`, `:N,M`, `:N#M`

**Files:**
- Modify: `crates/lucerna-workbench/src/quick_open/query.rs` (`QuickOpenQuery::GotoLine`, `parse_query`, add `resolve_line`)
- Modify: `crates/lucerna-workbench/src/quick_open/mod.rs:11` (re-export), `:148`, `:171` (`Sources`)
- Modify: `crates/lucerna-workbench/src/quick_open/view.rs` (`Sources::has_editor` at line 54, field `QuickOpen::has_editor` at 138/195, the `GotoLine` branch at 413–419)
- Modify: `crates/lucerna-workbench/src/lib.rs:125-127` (export `resolve_line`)
- Test: `crates/lucerna-workbench/tests/quick_open_query.rs`, `crates/lucerna-workbench/tests/quick_open.rs`

**Interfaces:**
- Consumes: `BufferSnapshot::line_count(&self) -> u32` (`lucerna-text`, does not count a trailing newline), `CodeView::snapshot()`.
- Produces:
  - `QuickOpenQuery::GotoLine { line: Option<i64>, column: Option<u32> }`.
  - `pub fn resolve_line(line: i64, line_count: u32) -> u32`: the real 1-based line. A negative number counts from the end (`line_count + 1 + line`), then is clamped into `[1, max(line_count, 1)]`.
  - `Sources::editor_lines: Option<u32>` replaces `has_editor: bool`. The field `QuickOpen::editor_lines` likewise. `None` means there is no code view.
  - `Row::GoToLine` and `QuickOpenEvent::GoToLine { line: u32, column: Option<u32> }` stay as they are; `line` is the resolved line.

- [ ] **Step 1: Fix and add tests in `tests/quick_open_query.rs`.**
  - `colon_line`, `colon_line_col`: change `Some(12)` to `Some(12i64)` to match the new type.
  - In `colon_garbage_none`, `":-3"` is now `line: Some(-3), column: None`.
  - Add:

```rust
#[test]
fn colon_separators_and_negative_lines() {
    let g = |line, column| QuickOpenQuery::GotoLine { line, column };
    assert_eq!(parse_query(":-1"), g(Some(-1), None));
    assert_eq!(parse_query(":12,5"), g(Some(12), Some(5)));
    assert_eq!(parse_query(":12#5"), g(Some(12), Some(5)));
    assert_eq!(parse_query(": 12 , 5 "), g(Some(12), Some(5)));
    assert_eq!(parse_query(":12:5:9"), g(Some(12), Some(5)));
    assert_eq!(parse_query(":12,"), g(Some(12), None));
    assert_eq!(parse_query(":,5"), g(None, Some(5)));
    assert_eq!(parse_query(":5:-1"), g(Some(5), None));
    assert_eq!(parse_query(":99999999999999999999"), g(None, None));
}

#[test]
fn resolve_line_counts_from_the_end_and_clamps() {
    assert_eq!(resolve_line(-1, 100), 100);
    assert_eq!(resolve_line(-100, 100), 1);
    assert_eq!(resolve_line(-500, 100), 1);
    assert_eq!(resolve_line(0, 100), 1);
    assert_eq!(resolve_line(50, 100), 50);
    assert_eq!(resolve_line(999, 100), 100);
    assert_eq!(resolve_line(i64::MIN, 100), 1);
    assert_eq!(resolve_line(-1, 0), 1);
}
```

- [ ] **Step 2: Add the workbench test** `goto_line_negative_and_comma` in `tests/quick_open.rs`, using the existing `open`, `open_long_txt`, `keys`, `type_text`, `rows`, `cursor` (`long.txt` has 100 lines):
  - `ctrl-g`, type `-1`: `rows == ["Go to line 100"]`. Enter: `cursor == Point { row: 99, column: 0 }`.
  - `ctrl-g`, type `12,5`: `rows == ["Go to line 12, column 5"]`. Enter: `Point { row: 11, column: 4 }`.
  - `ctrl-g`, type `-500`: `rows == ["Go to line 1"]`.
  - `ctrl-g`, type `0`: `rows == ["Go to line 1"]`.

  The test `goto_line_without_editor` stays as it is ("Open a file to go to a line").

- [ ] **Step 3: Run, see them fail.** `cargo test -p lucerna-workbench --test quick_open_query --test quick_open` → compile error or FAIL (`resolve_line` does not exist yet, `:-1` gives `None`).

- [ ] **Step 4: Fix.**
  - `parse_query`: split the part after `:` with `rest.split([':', ',', '#'])`. Part 1 parses as `i64`, part 2 as `u32`, both after `trim()`. Ignore the third part onward.
  - The doc comment of `parse_query` says clearly: this is the meaning when Vim is off, following VS Code's `gotoLineQuickAccess` (MIT, only the behavior is referenced). Vim's `:` command (V2b) uses its own parser.
  - Write `resolve_line` in `query.rs`, computing in `i64` with saturating arithmetic so `i64::MIN` does not overflow. Export it in `quick_open/mod.rs` and `lib.rs`.
  - `Workbench::show_quick_open` (`mod.rs:148`): `editor_lines: self.active_code_view(cx).map(|v| v.read(cx).snapshot().line_count())`. `show_extension_ids` (`:171`): `editor_lines: None`.
  - In `QuickOpen::query_changed`:
    - `editor_lines` is `None` → "Open a file to go to a line".
    - `Some(n)` and there is a line → `Row::GoToLine { line: resolve_line(line, n), column }`.
    - No line → "Type a line number…".
  - `Workbench::go_to_line` stays as it is; it still clamps, in case the file changes while Quick Open is open.

- [ ] **Step 5: Run.** `cargo test -p lucerna-workbench --test quick_open_query --test quick_open` → PASS, including the old `goto_line_moves_view`.

- [ ] **Step 6: Commit** `fix(quick-open): accept :-N, :N,M and :N#M as VS Code does (T5-4)`, with the five source files and the two test files above.

---

### Task 3: T5-1, a search result on a line that no longer exists: leave no observer behind

**Files:**
- Modify: `crates/lucerna-workbench/src/search_view/view.rs:740-776` (the `reveal` closure in `SearchView::open_line`)
- Test: `crates/lucerna-workbench/tests/search_view.rs`

**Interfaces:**
- Consumes: `CodeView::doc_state() -> &DocState`, `CodeView::go_to(Point, cx)`, `CodeView::set_match_highlights(Vec<Range<usize>>, Option<usize>, cx)`, `BufferSnapshot::line_count() -> u32`.
- Produces: no new API. `open_line` keeps its signature.

- [ ] **Step 1: Write the failing test** `stale_result_past_the_end_goes_to_last_line_once`, using the existing `project`, `workbench`, `keys`, `type_text`, `settle_search`, `search`, `click`, `active_view`, `draw`:
  1. `a.rs` has 120 lines `l{i}`, line 100 is `needle`, joined with `\n`.
  2. Search `needle`, then `fs::write` `a.rs` again as 50 lines `l1`…`l50` joined with `\n`, with no trailing newline.
  3. `click("search-line-0-0-<matching line>")`: take the index of the matching line from the excerpt, like `click_result_opens_and_positions`.
  4. `draw`. Assert `cursor == Point { row: 49, column: 0 }` and `match_highlights()` is empty.
  5. Type `enter` 60 times in the editor (`keys`), with a `draw` each time. Assert `cursor().row == 109`. Before the fix, the observer pulls the cursor back to line 99 once the file passes 100 lines.

- [ ] **Step 2: Write the test** `stale_result_in_unreadable_file_drops_the_observer` (`#[cfg(unix)]`, skipping itself when run as root like `unreadable_file_is_not_overwritten` in `recent.rs`):
  1. As in Step 1, but before clicking, `chmod 000` the file `a.rs`. Click, `draw`.
  2. Restore permissions `0o644`, write the 120-line file again, then trigger the tab's reload through the watcher. Use the helper that already exists in `tests/external_changes.rs` for waiting on a reload; if there is none, loop `run_until_parked` like `settle_search`.
  3. Assert the cursor is not at `row: 99`, and there is no highlight.

- [ ] **Step 3: Run, see them fail.** `cargo test -p lucerna-workbench --test search_view stale_result` → both tests FAIL (the cursor goes back to line 99).

- [ ] **Step 4: Fix the `reveal` closure.** Its return value: `true` means done and removes the observer.
  - `DocState::Ready` and `row >= line_count`:
    - `go_to(Point { row: line_count.saturating_sub(1), column: 0 })`;
    - `set_match_highlights(Vec::new(), None, cx)`;
    - one `tracing::debug!` line;
    - return `true`.
  - `DocState::Error(_)`: return `true` and do nothing.
  - `Loading`, `Deleted`: return `false` as before.
  - Fix the doc comment of `open_line` to match.

- [ ] **Step 5: Run.** `cargo test -p lucerna-workbench --test search_view` → the whole file PASSES.

- [ ] **Step 6: Commit** `fix(search): a result past the end of its file goes to the last line once (T5-1)`, with `view.rs` and `tests/search_view.rs`.

---

### Task 4: T3-3 part 1, the recent list keeps entries that no longer exist and shows every entry

**Files:**
- Modify: `crates/lucerna-workbench/src/recent.rs`:
  - `record` (123–134) and `remove` (136–144): drop `.existing()` and fix the doc comment.
  - Delete `RecentList::existing` (92–101) together with the test `existing_drops_missing_and_wrong_kind`.
  - Keep `RecentEntry::exists`: `startup::last_workspace` and Task 5 use it.
- Modify: `crates/lucerna-workbench/src/welcome.rs:180`, `src/open_commands.rs:224`, `src/app_menu.rs:651`: `.existing()` → `.entries`. Fix the doc comments "that existed" / "that still exist" at `welcome.rs:146` and `open_commands.rs:209`.
- Test: `src/recent.rs` (mod tests), `tests/welcome.rs`, `tests/app_menu.rs`

**Interfaces:**
- Consumes: none.
- Produces: `recent::record` / `recent::remove` no longer drop missing entries. `RecentList::existing` is gone. Welcome, the picker and the menu receive `RecentList.entries` intact (still dropping the window's own path as before).

- [ ] **Step 1: Fix the tests.**
  - `recent.rs`: rename `writes_drop_missing_entries` to `writes_keep_missing_entries`:
    - record `a`, `remove_dir(a)`, record `b` → `paths == [b, a]`;
    - then `remove(&data, &b)` → `paths == [a]`.
  - `tests/welcome.rs`: rename `skips_missing_folders` to `lists_missing_folders`:
    - `Fixture::new(&["a", "gone", "b"], &["gone"])` → `recent == [a, gone, b]`;
    - `drawn("welcome-recent-2")`.
  - `tests/app_menu.rs`: add `open_recent_menu_lists_missing_entries`, following the pattern of `open_recent_menu_hides_the_window_folder`:
    - record `other` then `this`;
    - `remove_dir(&other)` before `app_menu::install`;
    - assert `shown(&other)`.

- [ ] **Step 2: Run, see them fail.** `cargo test -p lucerna-workbench --lib recent:: && cargo test -p lucerna-workbench --test welcome --test app_menu` → the three tests above FAIL.

- [ ] **Step 3: Fix** following the Files list.

- [ ] **Step 4: Run again** the Step 2 command. At this point `vanished_folder_shows_warning` (welcome) and `vanished_folder_in_picker_reports` (open_commands) still pass: the open path still checks existence, and Task 5 changes that. → PASS.

- [ ] **Step 5: Commit** `fix(recent): keep entries whose path is gone, as VS Code does (T3-3)`, with `recent.rs`, `welcome.rs`, `open_commands.rs`, `app_menu.rs`, `tests/welcome.rs`, `tests/app_menu.rs`.

---

### Task 5: T3-3 part 2, choosing an entry that no longer exists removes it and reports "Path does not exist"

**Files:**
- Modify: `crates/lucerna-workbench/src/recent.rs`: add `MISSING_TITLE` and `missing_detail`.
- Modify: `crates/lucerna-workbench/src/open_commands.rs`:
  - add `report_missing`;
  - make `forget` (167) `pub(crate)`;
  - fix `Workbench::open_recent_entry` (308–345).
- Modify: `crates/lucerna-workbench/src/welcome.rs`:
  - `open_entry` (372–399) takes an extra `window`;
  - delete the field `warning`, `warning()` (233–236), the rendering of `welcome-warning` (734–737, 779), and the `warning_row` import if it is no longer used;
  - fix the call sites in `press` (366) and `on_picker_event` (444).
- Modify: `crates/lucerna-workbench/src/app_menu.rs:844-863` (`open_recent_entry`)
- Test: `src/recent.rs` (mod tests), `tests/welcome.rs`, `tests/open_commands.rs`, `tests/app_menu.rs`

**Interfaces:**
- Consumes: `RecentEntry::exists(&self) -> bool`; `tilde_path`; `Window::prompt`; `WelcomeView::forget(&mut self, PathBuf, cx)` (already exists, private).
- Produces:
  - `pub(crate) const MISSING_TITLE: &str = "Path does not exist";` in `recent.rs`.
  - `pub(crate) fn missing_detail(path: &Path, home: Option<&Path>) -> String` in `recent.rs`. Returns exactly `format!("The path '{}' does not exist on this computer.", tilde_path(path, home))`. Both strings are taken verbatim from `pathNotExistTitle` / `pathNotExistDetail` in VS Code's `windowsMainService.ts` (MIT); cite the source in a comment.
  - `pub(crate) fn report_missing(path: &Path, window: &mut Window, cx: &mut App)` in `open_commands.rs`. Calls `window.prompt(PromptLevel::Info, MISSING_TITLE, Some(&missing_detail(path, std::env::home_dir().as_deref())), &["OK"], cx)` and ignores the answer (no need to wait). The function does not remove the entry; the caller removes it first.
  - `pub(crate) fn forget(path: PathBuf, cx: &mut App)` (`open_commands.rs`, previously private).
  - `WelcomeView::open_entry(&mut self, entry: RecentEntry, new_window: bool, window: &mut Window, cx: &mut Context<Self>)`.

- [ ] **Step 1: Write the tests.** In the tests below, `detail(path)` is `format!("The path '{}' does not exist on this computer.", <path, with ~ if it is under std::env::home_dir()>)`.
  - `recent.rs` mod tests: `missing_detail_uses_tilde`.

```rust
assert_eq!(MISSING_TITLE, "Path does not exist");
assert_eq!(
    missing_detail(Path::new("/Users/me/code/x"), Some(Path::new("/Users/me"))),
    "The path '~/code/x' does not exist on this computer."
);
```

  - `tests/welcome.rs`: replace `vanished_folder_shows_warning` with `missing_folder_is_removed_and_reported`.
    1. `Fixture::new(&["a", "b"], &[])`, then `remove_dir(a)`. `click("welcome-recent-0")`, `draw`.
    2. Assert `cx.pending_prompt() == Some(("Path does not exist".into(), detail(a)))`.
    3. Assert right away, while the prompt is still open: `recent == [b]`, `fixture.stored() == [b]`, no event. The entry is removed before the user presses OK, like VS Code.
    4. `simulate_prompt_answer("OK")`, `draw`. Assert there is no prompt any more, and the list is still `[b]`.
    5. `create_dir(a)` (the drive is plugged back in), `draw`. Assert `fixture.stored() == [b]`: the entry does not come back by itself.
  - `tests/open_commands.rs`: replace `vanished_folder_in_picker_reports` with `missing_folder_in_picker_is_removed_and_reported`.
    1. `remove_dir(other)` **before** `ctrl-r`. Assert the picker's `row_texts()` has `other`.
    2. `enter`. Assert the prompt `("Path does not exist", detail(other))`. `simulate_prompt_answer("OK")`, `settle`.
    3. Assert `shown_root == root`, `window_count == 1`, `fixture.stored() == [third]`, and the status message does not contain "no longer exists".
  - `tests/open_commands.rs`: add `missing_file_in_picker_is_removed_and_reported`:
    - write by hand into `recent.json` an entry `{"path": <base>/gone.txt, "kind": "file"}` (do not create the file), next to `third`;
    - `ctrl-r`, choose `gone.txt`, `enter` → prompt `("Path does not exist", detail(gone.txt))`;
    - `simulate_prompt_answer("OK")`; assert `stored() == [third]` and no new tab.
  - `tests/app_menu.rs`: add `missing_entry_without_window_is_forgotten`:
    - `app_state`, write `recent.json` with `gone` (not created) and `kept` (created);
    - `cx.update(|cx| { app_menu::install(cx); cx.dispatch_action(&OpenRecentEntry { entry: gone }) })`, then `run_until_parked`;
    - assert `recent::load(..).entries` has only `kept` left and `cx.windows().is_empty()`.

- [ ] **Step 2: Run, see them fail.** `cargo test -p lucerna-workbench --lib recent:: && cargo test -p lucerna-workbench --test welcome --test open_commands --test app_menu` → the new tests FAIL: there is no prompt yet, and Welcome still shows a warning.

- [ ] **Step 3: Fix.** The order on every path is remove first, report after, like VS Code's `resolveOpenable`.
  - Write `MISSING_TITLE`, `missing_detail`, `report_missing` per Interfaces.
  - `Workbench::open_recent_entry`: an entry that does not exist → `forget(entry.path.clone(), cx)` then `report_missing(&entry.path, window, cx)`. Drop the status message "no longer exists".
  - `WelcomeView::open_entry`:
    - switch to `cx.spawn_in(window, …)` so `window` is available in `update_in`;
    - an entry that does not exist → `view.forget(entry.path.clone(), cx)` (remove it from the page and from the file, then `app_menu::refresh`), then `report_missing(&entry.path, window, cx)`.
  - `app_menu::open_recent_entry`:
    - the branch with a Welcome window calls `view.open_entry(entry, false, window, cx)`;
    - the branch with no window: `cx.spawn` checks `entry.exists()` on `background_spawn`. If it exists, `router::open` as before; if not, `open_commands::forget(entry.path, cx)` and report nothing (Note 2).
  - Delete Welcome's warning part per the Files list.

- [ ] **Step 4: Run again** the Step 2 command, plus `--test recent_picker --test startup` if those files exist (`ls crates/lucerna-workbench/tests`) → PASS. Run `cargo clippy -p lucerna-workbench --tests` → no new warnings (unused imports, dead fields).

- [ ] **Step 5: Commit** `fix(recent): drop a recent entry whose path is gone and say so, as VS Code does (T3-3)`, with `recent.rs`, `open_commands.rs`, `welcome.rs`, `app_menu.rs` and the four test files.

---

### Task 6: Mark "Fixed" in the review, check everything

**Files:**
- Modify: `docs/superpowers/reviews/2026-11-15-release-flow-review.md` (items T3-3 at line 229, T5-1 at 258, T5-4 at 271, T5-5 at 278)

**Interfaces:**
- Consumes: the SHAs of the Task 1–5 commits (`git log --oneline main..HEAD`).
- Produces: none.

- [ ] **Step 1: Check everything.**
  - `cargo test -p lucerna-project -p lucerna-workbench` → PASS.
  - `cargo clippy --workspace --tests` → clean.
  - `cargo deny check` → pass.

- [ ] **Step 2: Add the "Fixed" line.** Write one line at the end of each item, in the form the fixed items in the file already use (`- **Fixed** (commit <sha>): …`):
  - **T3-3** (sha of Task 4, Task 5): the recent list keeps entries that are no longer on disk. Welcome, Open Recent and the menu show every entry. Choosing an entry that no longer exists removes it from the list and shows the dialog "Path does not exist" / "The path '…' does not exist on this computer." with only an OK button, like VS Code. The menu with no window removes it without reporting.
  - **T5-1** (sha of Task 3): when the line no longer exists, the cursor goes to the last line and the observer is removed, even when the file cannot be read.
  - **T5-4** (sha of Task 2): `:-N` counts from the end; the line and column are separated by `:`, `,` or `#`; the line is clamped into `[1, line count]` like VS Code. Applies when Vim is off.
  - **T5-5** (sha of Task 1): a `crlf(true)` matcher together with a searcher whose line terminator is CRLF.

  Do not edit the summary table or the line "The remaining P3 items not yet fixed"; the other groups' branches also edit this file, so that part is updated after merging into `claude/p3-fixes` (spec §5 "After merge").

- [ ] **Step 3: Commit** `docs(review): mark T3-3, T5-1, T5-4, T5-5 fixed`, with only the review file.
