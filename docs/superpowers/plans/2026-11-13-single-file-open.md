# Mở file lẻ như Zed — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Mở một file lẻ thì chỉ có file đó: không quét thư mục cha, không mở cả repo git.

**Architecture:** `resolve_target` luôn trả chế độ một file (R27, đã có) cho một file. Workbench ở chế độ một file hiện tên file trên thanh tiêu đề và không vẽ dòng gốc (thư mục) trong sidebar, nên sidebar chỉ còn dòng của file.

**Tech Stack:** Rust, GPUI (chỉ trong `lucerna-workbench`).

**Spec:** `docs/superpowers/specs/2026-11-13-single-file-open-design.md`

## Global Constraints

- Cargo: `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"`.
- Trước mỗi build/test: `scripts/prune-target.sh`; nếu có `target/release` và không có cargo khác chạy (`pgrep -x cargo`), xoá nó.
- Chỉ `lucerna-workbench` dùng gpui. Không giả định riêng macOS.
- Commit theo đường dẫn cụ thể (`git commit -- <paths>`), trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Không rebase main; kiểm tra `.git/MERGE_HEAD` trước khi commit.
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` phải sạch.

## Review Focus

- File trong repo git (kể cả git worktree có file `.git`) mở lẻ, không mở repo — Task 1 có test.
- File trong thư mục thường như `~/Downloads` mở lẻ — Task 1 có test.
- Mở thư mục (kể cả `$HOME`, thư mục có `.git`) vẫn mở như folder — Task 1 có test.
- Mở thư mục ở chế độ folder vẫn có dòng gốc và tiêu đề là tên thư mục — Task 2 test hiện có `root_row_replaces_uppercase_header` giữ nguyên.
- Đường dẫn file là symlink: gốc là thư mục cha của đường dẫn đã canonicalize (hành vi hiện tại, không đổi) — không thêm test.

**Ghi chú lệch spec (quyết định khi lập kế hoạch):** spec nói "dòng gốc sidebar hiện tên file". Vì worktree một file đã có dòng của file, dòng gốc mang tên file sẽ lặp tên hai lần; Zed chỉ hiện một dòng. Kế hoạch bỏ dòng gốc ở chế độ một file. Spec được sửa theo trong Task 2.

---

### Task 1: `resolve_target` luôn mở file lẻ

**Files:**
- Modify: `crates/lucerna/src/target.rs` (doc của `Target::file`, `resolve_target_with_home`)
- Test: `crates/lucerna/tests/cli.rs`

**Interfaces:**
- Produces: `resolve_target_with_home(path, home) -> Result<Target, String>`: với file trả `Target { root: <thư mục cha canonical>, file: Some(<file canonical>), single_file: true }`. Chữ ký giữ nguyên; tham số `home` không còn dùng cho file (giữ để không đổi API công khai; đổi tên thành `_home` nếu clippy báo).
- `Target::file` không còn được dùng trong crate: xoá nó nếu không còn nơi nào gọi (`grep -rn "Target::file" crates`).

- [ ] **Step 1: Sửa test trong `crates/lucerna/tests/cli.rs`**

Thay ba test `resolve_file_in_git_repo_uses_repo_root`, `resolve_file_in_git_worktree_uses_dir_with_git_file`, `resolve_file_outside_git_uses_parent` và `resolve_loose_file_in_home_is_single_file` bằng:

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

Giữ lại phần dựng thư mục tạm của các test cũ cho từng trường hợp.

- [ ] **Step 2: Chạy test, xác nhận ba test file thất bại**

Run: `cargo test -p lucerna --test cli resolve_`
Expected: FAIL ở `resolve_file_in_git_repo_opens_alone`, `resolve_file_in_git_worktree_opens_alone`, `resolve_file_in_plain_folder_opens_alone` (`single_file: false`).

- [ ] **Step 3: Sửa `resolve_target_with_home`**

Với file: bỏ tìm `.git` và điều kiện `$HOME`/`/`; trả `single_file: true`, `root` = thư mục cha. Cập nhật doc comment (bỏ "nearest ancestor holding `.git`", nêu: một file luôn mở lẻ như Zed và VS Code; xem spec). Sửa doc của trường `Target::single_file` cho khớp.

- [ ] **Step 4: Chạy test**

Run: `cargo test -p lucerna`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git commit -m "feat: open a lone file alone, never its folder or repo" -- crates/lucerna/src/target.rs crates/lucerna/tests/cli.rs
```

### Task 2: Tiêu đề và sidebar ở chế độ một file

**Files:**
- Modify: `crates/lucerna-workbench/src/workbench.rs` (`root_name`, chỗ tạo `ExplorerView` quanh dòng 447)
- Modify: `crates/lucerna-workbench/src/explorer/view.rs` (`ExplorerView::new`, vẽ dòng gốc quanh dòng 306)
- Modify: `docs/superpowers/specs/2026-11-13-single-file-open-design.md` (dòng "Sidebar")
- Test: `crates/lucerna-workbench/tests/explorer_view.rs`

**Interfaces:**
- Consumes: `Workbench::single_file(root, &file, data_dir, window, cx)` (có sẵn).
- Produces: `ExplorerView::new(root: PathBuf, single_file: bool, snapshot, workbench, editor_groups, cx)`; `Workbench::root_name()` trả tên file ở chế độ một file.

- [ ] **Step 1: Viết test trong `tests/explorer_view.rs`**

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

Mở cửa sổ như `open_files.rs:210` làm (`cx.add_window_view(... Workbench::single_file(root, &file, None, window, cx))`), rồi vẽ một frame (`cx.run_until_parked()`).

- [ ] **Step 2: Chạy test, xác nhận thất bại**

Run: `cargo test -p lucerna-workbench --test explorer_view single_file_shows_only_the_file`
Expected: FAIL ở `explorer-root` (dòng gốc vẫn được vẽ).

- [ ] **Step 3: Sửa mã**

- `ExplorerView` nhận `single_file: bool`; khi `true` không vẽ phần tử `explorer-root`.
- `Workbench::root_name()`: khi `self.single_file` là `Some(file)`, trả `file.file_name()`.
- Sửa dòng "Sidebar" trong spec: ở chế độ một file không có dòng gốc, sidebar chỉ có dòng của file; thanh tiêu đề hiện tên file.

- [ ] **Step 4: Chạy toàn bộ test workbench và kiểm tra**

Run: `cargo test -p lucerna-workbench && cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings`
Expected: tất cả PASS, không cảnh báo. `root_row_replaces_uppercase_header` vẫn qua.

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(workbench): a lone file shows its name, not its folder" -- crates/lucerna-workbench/src/workbench.rs crates/lucerna-workbench/src/explorer/view.rs crates/lucerna-workbench/tests/explorer_view.rs docs/superpowers/specs/2026-11-13-single-file-open-design.md
```
