# Markdown Preview Follows a Rename Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** After an Explorer rename, every open Markdown Preview of a moved file shows it at its new path, and renaming back to the old name is allowed.

**Architecture:** `Workbench::paths_renamed` already moves documents, tab items and CodeViews. It gains one call that points each `MarkdownPreview` view whose path is at or under `from` to its moved path via `MarkdownPreview::set_path`. This runs inside the same update as `Documents::rename`, so it runs before the deferred `DocumentEvent::Closed` reaches `reopen_if_dropped`, which then finds the document at the preview's new path and no longer reopens the old one.

**Tech Stack:** Rust, GPUI (`lucerna-workbench`), `#[gpui::test]` integration tests.

**Spec:** `docs/superpowers/specs/2026-11-16-markdown-preview-follows-rename-design.md`

## Global Constraints

- Only `lucerna-workbench` may use gpui.
- UI strings are English only. This change adds none.
- Cargo: `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"`. Run `scripts/prune-target.sh` before every `cargo test` or build. Before a debug build or `cargo test`, delete `target/release`, but only if `pgrep -x cargo` shows no other session building. Never set `CARGO_TARGET_DIR` or pass `--target-dir`.
- Stage files by explicit path. Commit trailer: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. Renaming a **folder** above the previewed file. The preview must move too, because `moved_path` handles descendants. Test: `rename_folder_moves_preview`.
2. A **dynamic** preview (`markdown.showPreviewToSide`). Its path must follow as well. Test: `rename_moves_dynamic_preview`.
3. Renaming **back to the old name** must leave no error in the Explorer. Test: `rename_previewed_file_and_back`.
4. A preview of a file that is **not** under `from` must be left untouched. Covered by the `moved_path` guard. No separate test is needed.

---

### Task 1: Previews follow a rename

**Files:**
- Modify: `crates/lucerna-workbench/src/markdown_preview/commands.rs`. Add `retarget_previews` beside the private `previews` at lines ~197-208, and reuse `previews`.
- Modify: `crates/lucerna-workbench/src/markdown_preview/mod.rs:20`. Re-export `retarget_previews` in the `pub(crate) use commands::{...}` list.
- Modify: `crates/lucerna-workbench/src/workbench.rs:1241-1270`. In `paths_renamed`, call it right after `update_editor_groups(... retarget ...)`.
- Test: `crates/lucerna-workbench/tests/explorer_edit.rs`, which already has the rename helpers `click`, `keys`, `type_text`, `wait_until`, `run`, `open_file` and `root_of`.

**Interfaces:**
- Produces: `pub(crate) fn retarget_previews(wb: &Workbench, from: &Path, to: &Path, cx: &mut App)`. `from` and `to` are absolute. For each preview in `previews(wb, cx)` with `moved_path(preview.path(), from, to) == Some(new)`, it calls `preview.update(cx, |p, cx| p.set_path(Arc::from(new), cx))`. Collect the matches first, then update, so that `wb` is not borrowed across the update. If borrowing `wb` and `cx` together does not fit, take the list from `previews` before the loop. Any signature that keeps this behaviour is acceptable.
- Consumes: `crate::workspace_state::moved_path(path, from, to) -> Option<PathBuf>`, `MarkdownPreview::path()` and `MarkdownPreview::set_path(Arc<Path>, cx)`.

- [ ] **Step 1: Write the failing tests** in `tests/explorer_edit.rs`.

  Add a test helper `previews(wb, cx) -> Vec<Entity<MarkdownPreview>>`. It walks every group's tabs where `t.previewed_path().is_some()` and collects `editor_groups().read(cx).body_of(t.id)?.downcast::<MarkdownPreview>().ok()`. Import `lucerna_workbench::MarkdownPreview`.

  `rename_previewed_file_and_back`:
  - Write `doc.md` (`"# hi\n"`) into `project()`. `open_file(&wb, "doc.md", cx)`, then `run(&wb, "markdown.showPreview", cx)`. Let `preview` be the only entry of `previews`.
  - `click("explorer-row-doc.md")`, `keys("enter")`, `type_text("x")` (the stem is selected, so the name becomes `x.md`), `keys("enter")`. Then `wait_until` `root.join("x.md").is_file()` and `!editing`.
  - Assert `preview.read(cx).path().as_ref() == root.join("x.md")`.
  - Assert `wb.documents().read(cx).state(&root.join("doc.md")).is_none()`. **This is the bug.**
  - `wait_until` the worktree snapshot contains `x.md`, then `draw`. `click("explorer-row-x.md")`, `keys("enter")`, `type_text("doc")`.
  - Assert `explorer().read(cx).edit_error().is_none()`.
  - `keys("enter")`, then `wait_until` `root.join("doc.md").is_file()`. Assert the preview path is `root.join("doc.md")`.

  `rename_folder_moves_preview`:
  - Write `src/guide.md`, open it and run `markdown.showPreview`.
  - Rename the `src` row to `lib`, as in `rename_folder_moves_open_tabs`.
  - Assert the preview path is `root.join("lib/guide.md")` and `state(&root.join("src/guide.md")).is_none()`.

  `rename_moves_dynamic_preview`:
  - Write `doc.md`, `open_file` it and run `markdown.showPreviewToSide`. Let `preview` be the entry of `previews` whose `is_dynamic()` is true.
  - Rename `doc.md` to `x.md` through the Explorer. Focus the Explorer row with `click` first.
  - Assert the preview path is `root.join("x.md")` and `state(&root.join("doc.md")).is_none()`.

- [ ] **Step 2: Run the tests and confirm they fail**

  Run: `cargo test -p lucerna-workbench --test explorer_edit -- rename_previewed_file_and_back rename_folder_moves_preview rename_moves_dynamic_preview`
  Expected: FAIL. The preview path is still the old one, `state(old)` is `Some(..)`, and the first test's `edit_error` contains "already exists".

- [ ] **Step 3: Implement `retarget_previews`, re-export it, and call it from `paths_renamed`** with the absolute `from` and `to` that the function already computes. Place the call after `update_editor_groups(cx, |m| m.retarget(&from, &to))`.

- [ ] **Step 4: Run the tests and confirm they pass**

  Run the Step 2 command. Expected: 3 passed.
  Then run `cargo test -p lucerna-workbench --test explorer_edit --test markdown_preview --test explorer_clipboard`. Expected: all pass.

- [ ] **Step 5: Lint**

  Run: `cargo clippy -p lucerna-workbench --tests -- -D warnings` and `cargo fmt --check`. Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add -- crates/lucerna-workbench/src/markdown_preview/commands.rs crates/lucerna-workbench/src/markdown_preview/mod.rs crates/lucerna-workbench/src/workbench.rs crates/lucerna-workbench/tests/explorer_edit.rs
git commit -m "fix(markdown-preview): follow an Explorer rename

A preview kept its old path, reopened the gone file after the rename,
and so held its name: renaming back said it already exists.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>" -- crates/lucerna-workbench/src/markdown_preview/commands.rs crates/lucerna-workbench/src/markdown_preview/mod.rs crates/lucerna-workbench/src/workbench.rs crates/lucerna-workbench/tests/explorer_edit.rs
```
