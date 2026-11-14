# Git 5.2 (diff views) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Three read-only, one-column diff tabs:
- **Uncommitted Changes** (`git.viewChanges`).
- **Changes since ‹branch›** (`lucerna.git.viewBranchChanges`), with a branch picker.
- **‹file› (Changes)** (`git.openChange`), for a single file. A click on a gutter bar opens it at that hunk.

**Architecture:**
- The excerpt list learns diff lines: added, deleted and context. It gets line backgrounds, deleted rows without numbers, syntax colors taken from in-memory texts (so deleted lines are colored from the base blob), per-file header stats, and hunk navigation.
- One `DiffView` entity serves all three tabs. A `DiffKind` decides the file list and the base.
- The file list comes from `GitStore`, and each file's excerpts are built off the UI thread from the base text and the current text (the buffer when open, else the disk) with `lucerna_git::diff_hunks`.

**Tech Stack:** Rust 2024, gpui (workbench), `lucerna-git` (plan 5.1). No new external crates.

**Spec:** `docs/superpowers/specs/2026-11-13-git-view-design.md` §4.7, §4.8, §4.9 and the gutter click in §4.6. Research: `docs/research/2026-11-13-git.md` §5–§6. **Plan 5.1 must be merged first** (`2026-11-14-git-5-1-core.md`).

## Global Constraints

- Everything in plan 5.1's Global Constraints applies.
- The tabs are **read-only**. They have no stage, restore, commit or split toggle.
- **Context:** 2 lines on each side of a hunk. Excerpts that overlap or touch are merged.
- **File order:**
  1. conflicts;
  2. tracked (`Modified`, `Added`, `Deleted`, `TypeChanged`);
  3. new (`Untracked`).

  Within a group, files are sorted by path.
- At most **16** files load or diff at once.
- Deleted files start collapsed. A binary or >8 MB file shows only its header plus one note row: "Binary file" or "File too large to diff".
- **Exact copy:**
  - Tab titles: "Uncommitted Changes", "Changes since {ref}" and "{file name} (Changes)".
  - Empty states: "No uncommitted changes", "No changes since {ref}" and "No default branch found".
  - Unavailable states: "Git is not available", "Not a Git repository" and "Git is off in Restricted Mode".
  - Header labels: "new" and "deleted".
- `{ref}` is shown as git names it, e.g. `origin/main` or `main`.
- **Theme:**
  - added rows use `diff_added_background` plus a `vc_added` bar in the list gutter;
  - deleted rows use `diff_deleted_background` plus a `vc_deleted` bar.

  Both come from plan 5.1, Task 4.

## Review Focus

1. **Agent writes while the tab is open.** Only the changed file's excerpts are rebuilt. The selected line and the scroll position stay put, anchored by path and line as the excerpt list already does. Test: Task 3 `one_file_change_keeps_selection`.
2. **Hundreds of changed files.** The headers appear first and the hunks stream in, with never more than 16 jobs in flight. Test: Task 3 `streams_with_bounded_concurrency`.
3. **Deleted-line anchors.** Several deleted lines share the `line_number` of the row after them. Clicking any of them opens the file at that row. Selection does not jump between them after a refresh. Test: Task 1 `deleted_lines_resolve_to_following_row`.
4. **A branch base that disappears** (the branch is deleted, or the remote is pruned). The tab shows "No default branch found" with the picker button, and it does not crash or keep stale hunks. Test: Task 5 `missing_base_ref_shows_empty_state`.
5. **Search and References keep working.** The existing excerpt list tests must pass unchanged, apart from the mechanical `kind: LineKind::Normal` field. Verified in Task 1, Step 4.

---

## File map

| Path | Responsibility |
|---|---|
| `crates/lucerna-project/src/search.rs` | `LineKind` on `LineMatch` |
| `crates/lucerna-workbench/src/excerpt_list/{model.rs,render.rs,highlight.rs,mod.rs}` | Diff line rendering, text sources, file meta, change navigation |
| `crates/lucerna-workbench/src/diff_view/build.rs` | Pure: hunks plus texts become excerpts and stats |
| `crates/lucerna-workbench/src/diff_view/mod.rs` | `DiffView` entity: file list, jobs, toolbar, empty states |
| `crates/lucerna-workbench/src/diff_view/branch_picker.rs` | Branch picker modal on `quick_open::picker` |
| `crates/lucerna-workbench/src/editor_groups/{model.rs,view.rs}`, `workspace_state.rs` | New `TabItem`s, titles, icons, saving |
| `crates/lucerna-workbench/src/workbench.rs` | Commands, opening tabs, gutter click |
| `crates/lucerna-workbench/src/code_view/{mod.rs,element.rs}` | `CodeViewEvent::DiffHunkClicked` |
| `crates/lucerna-settings/assets/default-keybindings.json` | Alt+F5 in diff tabs |

---

### Task 1: Excerpt list diff lines

**Files:**
- Modify: `crates/lucerna-project/src/search.rs`:
  - add `#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)] pub enum LineKind { #[default] Normal, Added, Deleted { base_line: u32 } }`;
  - add `pub kind: LineKind` to `LineMatch`;
  - re-export it from `lib.rs`.
- Modify: every `LineMatch { … }` literal (15 sites in `search.rs`, `references_view.rs`, `excerpt_list/{model,highlight}.rs` and `tests/search_view.rs`), adding `kind: LineKind::Normal`.
- Modify: `crates/lucerna-workbench/src/excerpt_list/{model.rs,render.rs,highlight.rs,mod.rs}`
- Test: `crates/lucerna-workbench/tests/excerpt_list_diff.rs`

**Interfaces:**
- Produces:
  - **Deleted lines:** a `LineMatch` with `kind: Deleted { base_line }` has `line_number` equal to the 1-based current row that follows the deletion, and `ranges` empty.
  - `pub struct TextSource { pub current: Option<Arc<str>>, pub base: Option<Arc<str>> }`
  - `ExcerptList::set_text_source(&mut self, path: Arc<Path>, source: TextSource, cx)`. When a file has a source, highlight jobs color `Normal`/`Added` lines from `current` (by `line_number`) and `Deleted` lines from `base` (by `base_line`) instead of reading the disk.
  - In `highlight.rs`:
    - `pub fn highlight_text(text: &str, rel_path: &Path, line_numbers: &[u32], registry, styled) -> FileHighlights`
    - `highlight_file` now loads the file and calls it.
    - `MAX_HIGHLIGHT_BYTES` applies to `text.len()`.
  - `HighlightCache` keys base highlights separately from current ones, for example `(path, Side)`.
  - `pub struct FileMeta { pub added: u32, pub removed: u32, pub label: Option<&'static str>, pub note: Option<&'static str>, pub openable: bool }`
  - `ExcerptList::set_file_meta(&mut self, path: Arc<Path>, meta: FileMeta, cx)`. The header shows `+added −removed` in `vc_added`/`vc_deleted` and the label as a small muted tag. A `note` renders one muted row under the header in place of excerpts. When `openable` is false, the header's open-file button is disabled.
  - `Results::next_change(&self, from: Option<LineRef>, forward: bool) -> Option<LineRef>` returns the first line of the next run of `Added`/`Deleted` lines, across files, wrapping.
  - `ExcerptList::step_change(&mut self, forward: bool, cx) -> Option<LineRef>` selects that line and scrolls to it.
- **Render:**
  - **Row background:** `Added` uses `diff_added_background` and `Deleted` uses `diff_deleted_background`.
  - **Gutter:**
    - `Deleted` rows show no number;
    - a bar of the same width rule as the editor gutter (plan 5.1, Task 8) sits at the gutter's left edge, colored `vc_added` or `vc_deleted`.
  - **Gutter width:** `max_line_number` ignores `Deleted` lines.
- [ ] **Step 1: Write the failing tests** in `tests/excerpt_list_diff.rs`:
  - `deleted_rows_have_no_number_and_backgrounds`, using the debug selectors already used by `search_view` tests and a test-only `ExcerptList::row_background(r) -> Option<Hsla>`;
  - `deleted_lines_resolve_to_following_row`: two deleted lines before row 5 both activate at `line_number` 5;
  - `text_source_colors_deleted_lines_from_base`: a Rust base text where the deleted line is `fn old() {}`; after highlighting, that row has a keyword span;
  - `next_change_skips_context_and_wraps`;
  - `file_meta_header_and_note`;
  - `unopenable_header_button_disabled`.
- [ ] **Step 2: Run** `scripts/prune-target.sh && cargo test -p lucerna-workbench --test excerpt_list_diff`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test -p lucerna-project` and `cargo test -p lucerna-workbench --test excerpt_list_diff --test search_view --test references`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(excerpt-list): diff lines, text sources, file meta and change navigation`.

### Task 2: Build excerpts from hunks

**Files:**
- Create: `crates/lucerna-workbench/src/diff_view/build.rs`, with unit tests in the file
- Create: `crates/lucerna-workbench/src/diff_view/mod.rs`, a stub with `pub(crate) mod build;`
- Modify: `crates/lucerna-workbench/src/lib.rs` (`mod diff_view;`)

**Interfaces:**
- Consumes: `Hunk`, `HunkKind` and `diff_hunks` (plan 5.1, Task 2), and `LineKind` (Task 1).
- Produces:
  - `pub(crate) struct BuiltFile { pub excerpts: Vec<Excerpt>, pub added: u32, pub removed: u32 }`
  - `pub(crate) fn build_file(base: &str, current: &str, context: u32, full_file: bool) -> BuiltFile`
    - With `full_file`, it returns one excerpt with every current line plus the deleted lines interleaved.
    - Otherwise each hunk gets `context` lines on each side, and overlapping or touching ranges merge.
    - In the output, every `Modified` hunk is its deleted base lines followed by its added current lines.
    - `added` and `removed` are line counts.
  - `pub(crate) const CONTEXT_LINES: u32 = 2;`
- [ ] **Step 1: Write the failing tests:**
  - `modified_hunk_is_deleted_then_added`;
  - `context_two_lines_and_merge_touching`: hunks 5 lines apart merge, hunks 6 apart do not;
  - `untracked_all_added`: the base is `""`;
  - `deleted_file_all_deleted`: the current text is `""`;
  - `full_file_includes_unchanged_lines`;
  - `stats_count_lines`.
- [ ] **Step 2: Run** `cargo test -p lucerna-workbench diff_view::build`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the same command. Expected: PASS.
- [ ] **Step 5: Commit** `feat(git): build diff excerpts from hunks`.

### Task 3: `DiffView` and the Uncommitted Changes tab

**Files:**
- Modify: `crates/lucerna-workbench/src/diff_view/mod.rs`
- Modify: `crates/lucerna-workbench/src/editor_groups/model.rs`:
  - add `TabItem::GitChanges`;
  - title "Uncommitted Changes";
  - singleton;
  - saved as `GIT_CHANGES_TAB_STATE = "lucerna:git-changes"` in `saved_path` and restored in `workspace_state.rs`.
- Modify: `crates/lucerna-workbench/src/editor_groups/view.rs` (icon: an existing diff-like icon from `icons.rs`, or add Lucide `git-compare` with its ISC notice already covered by the Lucide entry)
- Modify: `crates/lucerna-workbench/src/workbench.rs` (command `git.viewChanges`, "Git: Open Changes (Uncommitted)")
- Modify: `crates/lucerna-settings/assets/default-keybindings.json`. Add `alt+f5` and `shift+alt+f5` → `workbench.action.editor.nextChange` / `previousChange` with `"when": "gitDiffViewFocus"`, and set that context key on the view's focus.
- Test: `crates/lucerna-workbench/tests/git_diff_view.rs`

**Interfaces:**
- Consumes:
  - `GitStore::{snapshot, state, hunks}`, `Repo::read_blob` and `GitStoreEvent` (plan 5.1);
  - `ExcerptList` with `set_text_source`, `set_file_meta` and `step_change` (Task 1);
  - `build_file` (Task 2);
  - `Documents::snapshot_of`.
- Produces:
  - `pub enum DiffKind { Uncommitted, Branch { base_ref: Option<String> }, File(Arc<Path>) }`
  - `pub struct DiffView`, created with `DiffView::new(kind, git: Entity<GitStore>, documents: Entity<Documents>, root: PathBuf, cx)`.
    - `fn kind(&self) -> &DiffKind`
    - `fn title(&self) -> String`
    - `fn step_change(&mut self, forward: bool, cx)`
    - `fn excerpt_list(&self) -> &Entity<ExcerptList>`, for tests.
  - `Workbench::open_git_changes(&mut self, window, cx)`
- **Uncommitted behavior:**
  - The file list is `Repo::status(None)` against HEAD, independent of `git.diffBase`. Use a dedicated `GitStore::uncommitted_statuses()` job result, cached and refreshed with the snapshot, so the tab works when `diffBase` is `"defaultBranch"`.
  - The base text is `read_blob(HEAD, path)`.
  - The current text comes from the open buffer if there is one, else from the disk.
  - The jobs run on the background executor, at most 16 at a time.
  - Files stream in through `ExcerptList::add_files` in the order from Global Constraints. Headers come first, as empty excerpt lists with `FileMeta { note: Some("…") }` while loading, and are filled as each file finishes.
  - On `SnapshotChanged` the view diffs the set of files: it removes vanished files, adds new ones and rebuilds those whose status changed.
  - On `HunksChanged(path)`, or a disk change event for a listed path, it rebuilds only that file.
- **Toolbar** (`search_toolbar` / `search_bar_row` from `ui.rs`, like the References tab):
  - left: `{n} files  +A −R`;
  - right: previous and next change (`bar_icon_button`, the chevrons used by Search), collapse all / expand all (`CHEVRONS_DOWN_UP` / `CHEVRONS_UP_DOWN`), and refresh (`REFRESH_CW`, which runs `git.refresh`).
- **Empty and unavailable states:** one centered muted line, as defined in Global Constraints and mapped from `GitState`. `Restricted` adds a button "Manage Workspace Trust" that runs `workbench.trust.manage`.
- **Activate:** Enter or a click opens the file at the line with `OpenMode::Normal`. A deleted line opens at its `line_number`.
- [ ] **Step 1: Write the failing tests** in `tests/git_diff_view.rs`, using a temp repo, `set_trusted(true)` and `run_until_parked`:
  - `lists_files_in_conflict_tracked_new_order`;
  - `shows_deleted_and_added_rows_for_modified_file`;
  - `unsaved_buffer_text_is_used`;
  - `deleted_file_collapsed_and_not_openable`;
  - `binary_file_shows_note`;
  - `one_file_change_keeps_selection`: select a line in `b.rs`, modify `a.rs`, deliver the events; the selection is still the same anchor;
  - `streams_with_bounded_concurrency`: 100 modified files; `peak_jobs() <= 16` (test-only counter);
  - `empty_state_when_clean`;
  - `restricted_state_has_trust_button`;
  - `alt_f5_steps_hunks_across_files`;
  - `tab_saved_and_restored`, following the existing `workspace_state` tests for Search;
  - `independent_of_diff_base_setting`: with `git.diffBase` set to `"defaultBranch"` and the work committed on a branch, the Uncommitted tab is empty.
- [ ] **Step 2: Run** `cargo test -p lucerna-workbench --test git_diff_view`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the same test plus `--test editor_groups_model --test editor_groups_view --test workspace_state`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(git): Uncommitted Changes tab`.

### Task 4: Single-file diff and gutter click

**Files:**
- Modify: `crates/lucerna-workbench/src/editor_groups/model.rs`:
  - add `TabItem::FileDiff(Arc<Path>)`, one per path;
  - not a preview;
  - not saved (`saved_path` returns `None`);
  - title "{file name} (Changes)".
- Modify: `crates/lucerna-workbench/src/code_view/{mod.rs,element.rs}`. A mouse-down on a painted hunk bar or deleted marker emits `CodeViewEvent::DiffHunkClicked { row: u32 }`, using the hunk's `rows.start`. The cursor over a bar is `CursorStyle::PointingHand`.
- Modify: `crates/lucerna-workbench/src/workbench.rs`. Register the commands `git.openChange` ("Git: Open Changes") and `git.openFile` ("Git: Open File"), and handle `DiffHunkClicked`.
- Test: `crates/lucerna-workbench/tests/git_diff_view.rs` (extend) and `tests/git_gutter.rs` (extend)

**Interfaces:**
- Consumes: `DiffView` with `DiffKind::File` (Task 3) and `CodeView::diff_hunks` (plan 5.1).
- Produces:
  - `Workbench::open_file_diff(&mut self, path: Arc<Path>, row: Option<u32>, window, cx)`. It opens or activates the tab, and selects and scrolls to the first diff line at or after `row` when one is given.
  - **`git.openChange`:** acts on the active file tab's path. With no file tab active it shows the status message "No file to compare".
  - **`git.openFile`:** in a `FileDiff` tab it opens the file. In the other diff tabs it opens the selected line's file.
- **File behavior:**
  - The base is `git.diffBase`'s `base_rev` from the snapshot.
  - `full_file: true`.
  - An untracked file is shown all added. An unchanged file shows its lines with no colored rows.
  - The toolbar has the file path, previous and next, and "Open File".
- **Gutter click:** when `scm.diffDecorationsGutterAction` is `"diff"`, the click calls `open_file_diff(path, Some(row))`. When it is `"none"`, nothing happens and the cursor stays an arrow.
- [ ] **Step 1: Write the failing tests:**
  - `open_change_shows_full_file_with_diff_rows`;
  - `open_change_untracked_all_added`;
  - `open_change_without_file_tab_shows_message`;
  - `gutter_click_opens_file_diff_at_hunk`: click the bar's bounds, read through a test-only `CodeView::hunk_bar_bounds(i)`;
  - `gutter_action_none_does_nothing`;
  - `open_file_from_file_diff`.
- [ ] **Step 2: Run** `cargo test -p lucerna-workbench --test git_diff_view --test git_gutter`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the same command. Expected: PASS.
- [ ] **Step 5: Commit** `feat(git): single-file diff tab and gutter click`.

### Task 5: Changes since ‹branch›

**Files:**
- Create: `crates/lucerna-workbench/src/diff_view/branch_picker.rs`, a modal built from `quick_open::picker` pieces as `recent_picker.rs` does, listing `Repo::branches()`.
- Modify: `crates/lucerna-workbench/src/editor_groups/model.rs`:
  - add `TabItem::BranchChanges`, a singleton;
  - saved as `"lucerna:branch-changes"`;
  - the chosen ref is stored in the workspace state next to it (`workspace_state.rs`, new optional field `branch_diff_base: Option<String>`).
- Modify: `crates/lucerna-workbench/src/diff_view/mod.rs` (the `DiffKind::Branch` path) and `workbench.rs` (command `lucerna.git.viewBranchChanges`, "Git: Open Changes Since Branch").
- Test: `crates/lucerna-workbench/tests/git_diff_view.rs` (extend)

**Interfaces:**
- Consumes: `Repo::{default_branch, merge_base, changed_since, branches, read_blob}` (plan 5.1) and `DiffView` (Task 3).
- Produces: `Workbench::open_branch_changes(&mut self, window, cx)`.
- **Behavior:**
  - `base_ref` is the saved ref, else `GitSnapshot::default_branch`.
  - The merge-base is `merge_base(base_ref)`.
  - The file list is `changed_since(merge_base)`, and base texts are `read_blob(merge_base, path)`.
  - `GitStore` caches it per `(base_ref, HEAD)`. It recomputes on `SnapshotChanged` when HEAD or refs changed, and per path on `paths_changed`.
  - The title is "Changes since {base_ref}".
  - The toolbar adds a button with the ref name and a chevron that opens the picker. Picking a ref reloads in place and saves the choice.
  - With no ref, or a ref that no longer resolves, the tab shows "No default branch found" plus a "Choose Branch" button that opens the picker.
- [ ] **Step 1: Write the failing tests:**
  - `branch_tab_includes_commits_and_uncommitted`: the feature branch has one committed and one uncommitted change, and both are listed;
  - `branch_tab_title_uses_ref`;
  - `picker_changes_base_and_saves`: pick `main`; the title changes and the workspace state has `branch_diff_base == Some("main")`;
  - `missing_base_ref_shows_empty_state`: delete the branch after choosing it, `rescan`, and the empty state appears;
  - `no_default_branch_offers_picker`;
  - `agent_commit_keeps_changes_listed`: commit everything; the Uncommitted tab is empty but the branch tab still lists the files.
- [ ] **Step 2: Run** `cargo test -p lucerna-workbench --test git_diff_view`. Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the same command. Expected: PASS.
- [ ] **Step 5: Commit** `feat(git): Changes since branch tab with branch picker`.

### Task 6: UI review against Zed and wrap-up

- [ ] **Step 1:** Run the app on a repo with changes. Place the Uncommitted Changes tab next to Zed's `git: diff` on the same repo. Check:
  - header height and colors;
  - row backgrounds;
  - the deleted-row gutter;
  - toolbar spacing;
  - the empty state.

  Fix the differences, or note them in the spec's §2 with the reason (design-principles §1.1).
- [ ] **Step 2:** Run the measurement from spec §5 for "200 files changed" (header and visible hunks) on the Zed clone, and add the numbers to `docs/research/2026-11-13-git.md`.
- [ ] **Step 3:** In row 5 of roadmap §1.3 of `2026-11-08-lucerna-phase1-design.md`, mark "Git view (5.1, 5.2) done".
- [ ] **Step 4:** Run `cargo deny check` and the full `cargo test -p lucerna-workbench -p lucerna-git -p lucerna-project`, then commit `docs(git): diff view review and measurements`.

---

## Self-review notes

- **Spec coverage:**
  - §4.6 gutter click: Task 4.
  - §4.7: Tasks 1, 3 and 5.
  - §4.8: Task 4.
  - §4.9 commands: Tasks 3–5. Alt+F5 in the tabs: Task 3.
  - The conflict case (§6): ordering in Task 3. A conflicted file's diff is HEAD against the disk, so the markers show as added lines; covered by `lists_files_in_conflict_tracked_new_order`.
  - The Explorer context-menu entry for `git.openChange` is deferred until the Explorer has a context menu (another spec).
- **Interfaces from plan 5.1 relied on:** `GitStore::{snapshot, state, hunks}`, `GitSnapshot::{base_rev, default_branch, head}`, `GitStoreEvent::{SnapshotChanged, HunksChanged}`, `Repo::{read_blob, branches, merge_base, changed_since, status}`, `diff_hunks`, `load_base_text`, `CodeView::diff_hunks`, `UiColors::{vc_*, diff_*_background}`.
- **New in this plan, added to plan 5.1's `GitStore`:**
  - `uncommitted_statuses()` (Task 3);
  - a per-`(base_ref, HEAD)` branch cache (Task 5).

  Both live in `git/mod.rs`. A Task 3 implementer adds them there.
