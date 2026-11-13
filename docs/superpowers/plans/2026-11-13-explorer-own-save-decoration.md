# Explorer Own-Save Decoration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The Explorer's "changed on disk" mark (bold name + dot) appears only for real external changes, never for Lucerna's own saves or for the active tab's file.

**Architecture:** For open, loaded documents the Explorer stops decorating on the raw watcher event; `Documents` emits `DocumentEvent::ChangedOnDisk` when a reload finds content different from `base_hash`, and the Workbench forwards that to the Explorer. The Explorer never marks the active tab's path.

**Tech Stack:** Rust, GPUI (`lucerna-workbench`).

**Spec:** `docs/superpowers/specs/2026-11-13-explorer-own-save-decoration-design.md`

## Global Constraints

- Only `lucerna-workbench` uses gpui; no macOS-only code.
- Setting `explorer.decorations.externalChanges` (default `true`) still gates both paths.
- Run `scripts/prune-target.sh` before `cargo test`; cargo needs `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"`.
- Commit only your paths: `git commit -m … -- <paths>`; trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

- Save while the watcher event arrives during the write: `start_job` defers via `reload_after_save`; the reload then sees `hash == base_hash` → no mark. Covered by the own-save test (event sent after save settles) — reviewer checks no other path marks.
- Agent rewrites an open background file with identical bytes → no mark (test `same_bytes_rewrite_of_open_file_is_not_marked`).
- Agent change merges into a dirty background file, or conflicts → marked (Merge/Conflict branches emit too; test `agent_change_merging_into_dirty_background_file_is_marked`).
- Document still loading (no buffer) when the file changes → Explorer marks immediately as before (`follows_disk` false).
- Setting off → `ChangedOnDisk` marks nothing (test `changed_on_disk_respects_setting`).

---

### Task 1: Decorate only real external changes

**Files:**
- Modify: `crates/lucerna-workbench/src/documents.rs` (`DocumentEvent`, `finish_reload`, new `follows_disk`)
- Modify: `crates/lucerna-workbench/src/explorer/view.rs` (`on_worktree_event`, new `mark_external_change`)
- Modify: `crates/lucerna-workbench/src/workbench.rs` (`on_worktree_event`, documents subscription)
- Modify: exhaustive `DocumentEvent` matches (`language_servers/sync.rs`, `symbols/source.rs`, others the compiler names): ignore the new variant.
- Test: `crates/lucerna-workbench/tests/explorer_decorations.rs` (new; copy the `init`/`settle`/`open`/`keys`/`input` helpers from `tests/external_changes.rs`)

**Interfaces:**
- `DocumentEvent::ChangedOnDisk { path: Arc<Path> }` — absolute key; emitted from `finish_reload` in the `Apply`, `Merge` and `Conflict` branches (after the `base_hash` check).
- `Documents::follows_disk(&self, path: &Path) -> bool` — open and has a buffer; `path` relative to root or absolute.
- `ExplorerView::on_worktree_event(&mut self, ev, snapshot, decorate: &dyn Fn(&Path) -> bool, cx)` — `decorate` takes a root-relative path; replaces the `bool`.
- `ExplorerView::mark_external_change(&mut self, path: &Arc<Path>, cx)` — absolute or relative; no-op for the active path.

- [ ] **Step 1: Write the failing tests** (workbench on a temp dir with `a.txt`, `b.txt`; `recently_changed(wb, "a.txt")` reads `explorer().model().is_recently_changed`; `report(wb, name)` sends `WorktreeEvent::Changed(Modified)`)
  - `own_save_is_not_marked`: open `a.txt`, `input("x")`, `keys("cmd-s")`, `report("a.txt")`, settle → `!recently_changed("a.txt")`.
  - `agent_change_to_background_open_file_is_marked`: open `a.txt`, then `b.txt`; write `"other\n"` to `a.txt`, report, settle → `recently_changed("a.txt")`.
  - `same_bytes_rewrite_of_open_file_is_not_marked`: as above but rewrite the same text → `!recently_changed("a.txt")`.
  - `agent_change_merging_into_dirty_background_file_is_marked`: `a.txt` = `"a\nb\nc\nd\ne\n"`, type `x` in it, open `b.txt`, write `"a\nb\nc\nd\nE\n"` → marked.
  - `agent_change_to_active_file_is_not_marked`: open `a.txt` only, external write, report, settle → not marked.
  - `changed_on_disk_respects_setting`: settings `{"explorer.decorations.externalChanges": false}`, background-file scenario → not marked.
  - `unopened_file_is_marked_at_once`: report `b.txt` without opening → marked before settling.

- [ ] **Step 2: Run** `cargo test -p lucerna-workbench --test explorer_decorations` — expect `own_save_is_not_marked`, `same_bytes…`, `agent_change_to_active_file…` to FAIL.

- [ ] **Step 3: Implement** the interfaces above. In `Workbench::on_worktree_event`, before updating the explorer, collect the root-relative paths in the event for which `documents.follows_disk` is true (avoid borrowing `documents` inside `explorer.update`), and pass `&|p| setting && !followed.contains(p)`. Explorer also skips `self.active`. In the documents subscription, on `ChangedOnDisk` call `explorer.mark_external_change` when the setting is on.

- [ ] **Step 4: Run** the new test file, `--test explorer_view`, `--test external_changes`, `--test save`, then `cargo clippy -p lucerna-workbench --all-targets -- -D warnings` — all pass.

- [ ] **Step 5: Commit** `fix(explorer): don't mark Lucerna's own saves as external changes` with the paths above.
