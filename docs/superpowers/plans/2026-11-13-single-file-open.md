# Open a lone file like Zed — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Opening a lone file shows only that file: no scan of the parent folder, no opening the whole git repo.

**Architecture:** `resolve_target` always returns single-file mode (R27, already present) for a file. A Workbench in single-file mode shows the file name in the title bar and does not draw the root (folder) row in the sidebar, so the sidebar has only the file's row.

**Tech Stack:** Rust, GPUI (only in `lucerna-workbench`).

**Spec:** `docs/superpowers/specs/2026-11-13-single-file-open-design.md`

## Global Constraints

- Cargo: `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"`.
- Before every build/test: `scripts/prune-target.sh`; if `target/release` exists and no other cargo is running (`pgrep -x cargo`), delete it.
- Only `lucerna-workbench` uses gpui. No macOS-only assumptions.
- Commit by explicit path (`git commit -- <paths>`), trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Never rebase main; check `.git/MERGE_HEAD` before committing.
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` must be clean.

## Review Focus

- A file in a git repo (including a git worktree with a `.git` file) opens alone, not the repo — Task 1 has tests.
- A file in a plain folder such as `~/Downloads` opens alone — Task 1 has tests.
- Opening a folder (including `$HOME`, a folder with `.git`) still opens it as a folder — Task 1 has tests.
- Opening a folder in folder mode still has the root row and the title is the folder name — the existing Task 2 test `root_row_replaces_uppercase_header` stays unchanged.
- File path is a symlink: the root is the parent folder of the canonicalized path (current behavior, unchanged) — no test added.

**Note on deviation from the spec (decided while planning):** the spec says "the sidebar root row shows the file name". Since the one-file worktree already has the file's row, a root row named after the file would repeat the name twice; Zed shows only one row. The plan drops the root row in single-file mode. The spec is amended accordingly in Task 2.

---

### Task 1: `resolve_target` always opens a lone file

**Files:**
- Modify: `crates/lucerna/src/target.rs` (doc of `Target::file`, `resolve_target_with_home`)
- Test: `crates/lucerna/tests/cli.rs`

**Interfaces:**
- Produces: `resolve_target_with_home(path, home) -> Result<Target, String>`: for a file returns `Target { root: <canonical parent folder>, file: Some(<canonical file>), single_file: true }`. The signature stays the same; the `home` parameter is no longer used for files (kept so the public API does not change; rename to `_home` if clippy complains).
- `Target::file` is no longer used in the crate: delete it if nothing calls it any more (`grep -rn "Target::file" crates`).

- [ ] **Step 1: Change the tests in `crates/lucerna/tests/cli.rs`**

Replace the three tests `resolve_file_in_git_repo_uses_repo_root`, `resolve_file_in_git_worktree_uses_dir_with_git_file`, `resolve_file_outside_git_uses_parent` and `resolve_loose_file_in_home_is_single_file` with:

```rust
/// A file opens alone, as in Zed: its folder is never scanned.
fn lone(file: &Path) -> Target {
    let file = canonical(file);
    Target { root: file.parent().unwrap().to_path_buf(), file: Some(file), single_file: true }
}

#[test]
fn resolve_file_in_git_repo_opens_alone() {
    // repo/.git, repo/src/deep/main.rs
    assert_eq!(resolve_target(Some(file.clone())), Ok(lone(&file)));
}

#[test]
fn resolve_file_in_git_worktree_opens_alone() {
    // outer/.git, outer/worktree/.git (file), outer/worktree/src/lib.rs
    assert_eq!(resolve_target(Some(file.clone())), Ok(lone(&file)));
}

#[test]
fn resolve_file_in_plain_folder_opens_alone() {
    // home/Downloads/notes.md, with home as $HOME
    let target = resolve_target_with_home(Some(file.clone()), Some(home.path())).unwrap();
    assert_eq!(target, lone(&file));
}

#[test]
fn resolve_folders_stay_folders() {
    // $HOME itself, a folder with .git, a plain sub-folder
    for dir in [home.path().to_path_buf(), repo, sub] {
        let target = resolve_target_with_home(Some(dir.clone()), Some(home.path())).unwrap();
        assert_eq!(target, Target::folder(canonical(&dir)));
    }
}
```

Keep the temp-folder setup of the old tests for each case.

- [ ] **Step 2: Run the tests, confirm the three file tests fail**

Run: `cargo test -p lucerna --test cli resolve_`
Expected: FAIL in `resolve_file_in_git_repo_opens_alone`, `resolve_file_in_git_worktree_opens_alone`, `resolve_file_in_plain_folder_opens_alone` (`single_file: false`).

- [ ] **Step 3: Change `resolve_target_with_home`**

For a file: drop the `.git` search and the `$HOME`/`/` condition; return `single_file: true`, `root` = parent folder. Update the doc comment (drop "nearest ancestor holding `.git`", say: a file always opens alone, like Zed and VS Code; see the spec). Update the doc of the `Target::single_file` field to match.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p lucerna`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git commit -m "feat: open a lone file alone, never its folder or repo" -- crates/lucerna/src/target.rs crates/lucerna/tests/cli.rs
```

### Task 2: Title and sidebar in single-file mode

**Files:**
- Modify: `crates/lucerna-workbench/src/workbench.rs` (`root_name`, where `ExplorerView` is created around line 447)
- Modify: `crates/lucerna-workbench/src/explorer/view.rs` (`ExplorerView::new`, drawing the root row around line 306)
- Modify: `docs/superpowers/specs/2026-11-13-single-file-open-design.md` ("Sidebar" line)
- Test: `crates/lucerna-workbench/tests/explorer_view.rs`

**Interfaces:**
- Consumes: `Workbench::single_file(root, &file, data_dir, window, cx)` (already exists).
- Produces: `ExplorerView::new(root: PathBuf, single_file: bool, snapshot, workbench, editor_groups, cx)`; `Workbench::root_name()` returns the file name in single-file mode.

- [ ] **Step 1: Write the test in `tests/explorer_view.rs`**

```rust
#[gpui::test]
fn single_file_shows_only_the_file(cx: &mut TestAppContext) {
    init(cx, None);
    // dir/notes.md and dir/other.md; open dir/notes.md with Workbench::single_file
    assert!(cx.debug_bounds("explorer-root").is_none(), "no folder row");
    assert!(cx.debug_bounds("explorer-row-notes.md").is_some());
    assert!(cx.debug_bounds("explorer-row-other.md").is_none());
    assert_eq!(wb.read_with(cx, |wb, _| wb.root_name()), "notes.md");
}
```

Open the window the way `open_files.rs:210` does (`cx.add_window_view(... Workbench::single_file(root, &file, None, window, cx))`), then draw one frame (`cx.run_until_parked()`).

- [ ] **Step 2: Run the test, confirm it fails**

Run: `cargo test -p lucerna-workbench --test explorer_view single_file_shows_only_the_file`
Expected: FAIL at `explorer-root` (the root row is still drawn).

- [ ] **Step 3: Change the code**

- `ExplorerView` takes `single_file: bool`; when `true` it does not draw the `explorer-root` element.
- `Workbench::root_name()`: when `self.single_file` is `Some(file)`, return `file.file_name()`.
- Amend the "Sidebar" line in the spec: in single-file mode there is no root row, the sidebar has only the file's row; the title bar shows the file name.

- [ ] **Step 4: Run all workbench tests and checks**

Run: `cargo test -p lucerna-workbench && cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings`
Expected: all PASS, no warnings. `root_row_replaces_uppercase_header` still passes.

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(workbench): a lone file shows its name, not its folder" -- crates/lucerna-workbench/src/workbench.rs crates/lucerna-workbench/src/explorer/view.rs crates/lucerna-workbench/tests/explorer_view.rs docs/superpowers/specs/2026-11-13-single-file-open-design.md
```
