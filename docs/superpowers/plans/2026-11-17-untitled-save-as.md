# New Untitled File and Save As Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** ⌘N mở tab Untitled; ⌘S / ⇧⌘S lưu nó (hoặc file bất kỳ) ra đường dẫn chọn trong hộp thoại; Untitled đi qua đóng tab, thoát, Save All và hot exit mà không mất chữ.

**Architecture:** Untitled là một tài liệu bình thường trong `Documents` với khóa giả tương đối `untitled:Untitled-N`. Module mới `untitled.rs` giữ mọi quy tắc thuần (nhận diện, đánh số, tên tab, tên gợi ý). Mọi chỗ chạm đĩa hay dịch vụ ngoài kiểm tra `untitled::is_untitled` và bỏ qua. Save As ghi ra đích rồi dùng đường đổi tên sẵn có (`Workbench::paths_renamed`) để chuyển tài liệu và tab sang đường dẫn thật.

**Tech Stack:** Rust, GPUI (test platform: `simulate_new_path_selection`, `did_prompt_for_new_path`, `simulate_prompt_answer`), crate `lucerna-workbench`, `lucerna-settings`.

**Spec:** `docs/superpowers/specs/2026-11-17-untitled-save-as-design.md`

## Global Constraints

- Cargo: `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"`; `scripts/prune-target.sh` trước mỗi lần build/test; không đặt `CARGO_TARGET_DIR`; build dùng chung `target/` ở checkout chính.
- Chỉ `lucerna-workbench` dùng gpui. Không giả định riêng macOS ngoài code nền tảng (hộp thoại dùng API GPUI chung).
- Mọi chuỗi hiện ra là tiếng Anh. Tên cố định: `Untitled-N`; lệnh `workbench.action.files.newUntitledFile` "File: New Untitled Text File" (`cmd+n`, menu "New File"); `workbench.action.files.saveAs` "File: Save As..." (`shift+cmd+s`, menu "Save As…"); setting `files.defaultLanguage` mặc định `""`.
- Tên tab: dòng đầu không rỗng, cắt khoảng trắng, tối đa 40 ký tự, thêm `…` khi cắt; rỗng → `Untitled-N`.
- Tên gợi ý khi lưu: tên tab bỏ `/ \ : * ? " < > |` và ký tự điều khiển, cắt khoảng trắng; rỗng → `Untitled-N`; không thêm đuôi.
- ⌘N không chạy ở Welcome / không có cửa sổ (đã đúng sẵn nhờ `WELCOME_COMMANDS` / `NO_WINDOW_COMMANDS`; chỉ cần test giữ nó).
- Không có file nào tên bắt đầu bằng `untitled:` được tạo trên đĩa, trong bất kỳ test nào.
- Commit: stage từng file theo đường dẫn; trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. Lưu Untitled vào đường dẫn **đang mở ở tab khác**: không còn hai tab cùng đường dẫn, tab Untitled giữ lại — test `save_untitled_over_open_file` (Task 5).
2. Ghi Save As thất bại (thư mục chỉ đọc): tab vẫn là Untitled, vẫn dirty, không mất chữ — test `save_as_write_error_keeps_untitled` (Task 5).
3. Gõ vào Untitled rồi xóa hết: đóng tab không hỏi, hot exit không giữ backup rỗng — test `emptied_untitled_closes_silently` (Task 6).
4. Hai Untitled, đóng `Untitled-1` rồi ⌘N: tab mới là `Untitled-1` — test `untitled_numbers_are_reused` (Task 4).
5. Untitled khi thư mục là repo git có `.gitignore` / watcher chạy thật: không đường dẫn `untitled:` nào tới git hay watcher — test `untitled_never_reaches_git_or_watcher` (Task 3).

---

### Task 1: Module `untitled` và khóa đi qua nguyên

**Files:**
- Create: `crates/lucerna-workbench/src/untitled.rs` (đăng ký `pub mod untitled;` trong `lib.rs`, thứ tự abc)
- Modify: `crates/lucerna-workbench/src/workspace_state.rs:123` (`absolute_path`)

**Interfaces:**
- Produces:
  - `pub const PREFIX: &str = "untitled:";`
  - `pub fn is_untitled(path: &Path) -> bool` — đúng khi `path` tương đối, đúng một thành phần, bắt đầu bằng `PREFIX`.
  - `pub fn path(n: u32) -> Arc<Path>` — `untitled:Untitled-{n}`.
  - `pub fn number(path: &Path) -> Option<u32>`.
  - `pub fn name(path: &Path) -> String` — `Untitled-N` (phần sau `PREFIX`).
  - `pub fn next_number(used: impl IntoIterator<Item = u32>) -> u32` — số nhỏ nhất từ 1 chưa dùng.
  - `pub fn label(text_first_lines: &str) -> Option<String>` — tên tab theo dòng đầu không rỗng (quy tắc ở Global Constraints), `None` khi không có dòng nào có chữ.
  - `pub fn suggested_file_name(label: &str) -> Option<String>` — tên gợi ý, `None` khi rỗng sau khi lọc.
- `workspace_state::absolute_path(root, path)` trả `path` nguyên khi `is_untitled(path)`.

- [ ] **Step 1: Unit test trong `untitled.rs`**

```rust
#[test] fn recognizes_only_relative_single_component() {
    assert!(is_untitled(Path::new("untitled:Untitled-1")));
    assert!(!is_untitled(Path::new("/tmp/untitled:Untitled-1")));
    assert!(!is_untitled(Path::new("dir/untitled:x")));
    assert!(!is_untitled(Path::new("src/main.rs")));
}
#[test] fn numbers() {
    assert_eq!(&*path(3), Path::new("untitled:Untitled-3"));
    assert_eq!(number(&path(3)), Some(3));
    assert_eq!(name(&path(3)), "Untitled-3");
    assert_eq!(next_number([]), 1);
    assert_eq!(next_number([1, 3]), 2);
    assert_eq!(next_number([2, 1]), 3);
}
#[test] fn labels() {
    assert_eq!(label(""), None);
    assert_eq!(label("\n   \n"), None);
    assert_eq!(label("\n  fn main() {}\nx").as_deref(), Some("fn main() {}"));
    let long = "a".repeat(50);
    assert_eq!(label(&long).unwrap(), format!("{}…", "a".repeat(40)));
}
#[test] fn suggested_names() {
    assert_eq!(suggested_file_name("a/b:c*?.rs").as_deref(), Some("abc.rs"));
    assert_eq!(suggested_file_name(" <>|\"\\ "), None);
}
```

Và trong `workspace_state.rs` tests: `absolute_path(Path::new("/r"), Path::new("untitled:Untitled-1"))` == `Path::new("untitled:Untitled-1")`.

- [ ] **Step 2: Chạy, thấy fail** — `cargo test -p lucerna-workbench --lib untitled workspace_state` → lỗi biên dịch.
- [ ] **Step 3: Viết `untitled.rs` theo Interfaces; sửa `absolute_path`.** `label` đếm theo `char`, không theo byte.
- [ ] **Step 4: Chạy, thấy pass** — cùng lệnh → PASS.
- [ ] **Step 5: Commit** — `feat: untitled document naming rules`

---

### Task 2: `Documents` tạo và giữ tài liệu Untitled

**Files:**
- Modify: `crates/lucerna-workbench/src/documents.rs` — `open_view` (~643), `is_dirty` (1075), `dirty_paths` (1266), `save` (1305), `revert`/`revert_now` (1679/1698), `on_worktree_event` Rescanned (~1992), `open_from_backup` (2140), `restore` (2949), `language_for` (2825)
- Modify: `crates/lucerna-settings/assets/default-settings.json` (thêm `"files.defaultLanguage": ""` cạnh các `files.*`), `docs/settings.md`
- Test: `crates/lucerna-workbench/tests/untitled.rs` (mới; copy `init`/`editor`/`settle`/`run` từ `tests/save.rs:28-133`)

**Interfaces:**
- Consumes: Task 1.
- Produces:
  - `pub fn Documents::new_untitled(&mut self, cx: &mut Context<Self>) -> Arc<Path>` — chọn `untitled::next_number` theo các khóa Untitled đang có trong `docs`, tạo tài liệu rỗng ở trạng thái Ready, không job, ngôn ngữ theo `files.defaultLanguage` (`LanguageRegistry::language_by_name`, không thấy → Plain Text), phát `Opened`. Trả khóa.
  - `Documents::open(path)` với khóa Untitled đã có trả view mới của tài liệu đó; khóa Untitled chưa có thì tạo rỗng (dùng cho restore), không bao giờ đọc đĩa.
  - `is_dirty(untitled)` = buffer không rỗng; `dirty_paths()` gồm các Untitled không rỗng.
  - `save(untitled, ..)` trả `Ok(false)` ngay, không ghi gì (đường lưu Untitled là Task 5).
  - `revert(untitled)`: không làm gì. `on_worktree_event`: bỏ qua khóa Untitled. `language_for(untitled)`: theo `files.defaultLanguage`.
  - `open_from_backup` với header Untitled: không đọc đĩa, dựng tài liệu từ text của backup, dirty.

- [ ] **Step 1: Test ở `tests/untitled.rs`**

```rust
#[gpui::test] fn new_untitled_is_empty_and_clean(cx: &mut TestAppContext)
// docs.new_untitled() == "untitled:Untitled-1"; lần hai == Untitled-2;
// snapshot rỗng; is_dirty == false; dirty_paths() rỗng.
#[gpui::test] fn untitled_with_text_is_dirty(cx)
// gõ "x" vào view → is_dirty == true, dirty_paths() == [untitled:Untitled-1].
#[gpui::test] fn untitled_never_touches_disk(cx)
// save(untitled) → Ok(false); revert(untitled); gửi WorktreeEvent::Rescanned;
// sau settle: không có mục nào trong root (fs::read_dir) tên chứa "untitled:".
#[gpui::test] fn default_language_applies(cx)
// settings { "files.defaultLanguage": "rust" } → language_of(untitled) == Some("rust");
// "nope" → Plain Text (None hoặc id plaintext như file .txt hiện nay).
```

- [ ] **Step 2: Chạy, thấy fail** — `cargo test -p lucerna-workbench --test untitled` → lỗi biên dịch.
- [ ] **Step 3: Cài đặt theo Interfaces.** Kiểm tra `untitled::is_untitled(&key)` ở đầu mỗi hàm đã liệt kê trong Files.
- [ ] **Step 4: Chạy, thấy pass** — cùng lệnh, cùng `--test save --test hot_exit` (không vỡ gì) → PASS.
- [ ] **Step 5: Commit** — `feat: untitled documents in Documents`

---

### Task 3: Chặn Untitled khỏi LSP, git, watcher, symbols, preview, recent, workspace state

**Files:**
- Modify: `language_servers/sync.rs:77` (Opened, và Edited/Saved/Closed/LanguageChanged của khóa Untitled); `git/wiring.rs:282` (`git_document_event`); `trust.rs:560`; `symbols/source.rs:175`; `workbench.rs:1471` (`refresh_watched_open_files`) và `1498` (`open_tab_paths`); `markdown_preview/commands.rs:354` (`follow_active`); `quick_open/mod.rs:42` (`RecentFiles::record`); `workbench.rs:1880` (recent khi mở file); `editor_groups/model.rs:853` (`saved_path`: Untitled không vào danh sách tab lưu); `explorer/view.rs:1118` (`sync_active_tab`: không reveal)
- Test: `crates/lucerna-workbench/tests/untitled.rs`

**Interfaces:**
- Consumes: Task 1 (`is_untitled`), Task 2 (`new_untitled`).
- Produces: không có API mới; mỗi chỗ trên trả sớm khi `untitled::is_untitled(path)`.

- [ ] **Step 1: Test**

```rust
#[gpui::test] fn untitled_never_reaches_lsp(cx)
// dùng tests/support/lsp.rs::lsp_workbench; settings files.defaultLanguage = ngôn ngữ của server giả;
// new_untitled + gõ chữ → count(didOpen) == 0, count(didChange) == 0.
#[gpui::test] fn untitled_never_reaches_git_or_watcher(cx)
// tests/support/git.rs::repo; mở Untitled, gõ; workbench.open_tab_paths() không chứa đường dẫn Untitled;
// git store không theo dõi đường dẫn nào bắt đầu bằng "untitled:".
#[gpui::test] fn untitled_not_in_workspace_state_or_recent(cx)
// save_workspace_state → state không có tab Untitled; RecentFiles không có Untitled.
```

Tìm thêm chỗ chạm đĩa theo đường dẫn tài liệu (`grep -n "fs::\|std::fs" src/` trên các hàm nhận đường dẫn tab/tài liệu); chỗ nào thêm thì thêm guard và ghi vào ledger.

- [ ] **Step 2: Chạy, thấy fail** — `cargo test -p lucerna-workbench --test untitled untitled_never untitled_not` → FAIL.
- [ ] **Step 3: Thêm guard ở từng chỗ trong Files.**
- [ ] **Step 4: Chạy, thấy pass** — `cargo test -p lucerna-workbench --test untitled --test lsp_sync --test git_gutter --test workspace_state` → PASS.
- [ ] **Step 5: Commit** — `feat: keep untitled documents away from disk services`

---

### Task 4: Lệnh ⌘N, tên tab, icon, path bar, menu

**Files:**
- Modify: `workbench.rs` (lệnh mới trong `default_registry` ~3463; `pub fn new_untitled_file(&mut self, window, cx)`); `editor_groups/model.rs:165` (`Tab::title`) hoặc `editor_groups/view.rs:809-815` (ghi đè tên cho Untitled theo `untitled::label` của snapshot, không thì `untitled::name`); `editor_groups/view.rs:152` (`tab_icon_source`: Untitled → icon của file `.txt`); `editor_groups/view.rs:578,1015` (path bar: `Untitled-N`); `app_menu.rs:236-257` (File → "New File" ngay trên "New Window"; sửa test `993`, `1011` cho thứ tự mới); `keys.rs:189` (thêm dòng `cmd+n`); `crates/lucerna-settings/assets/default-keybindings.json:133-141`
- Docs: `docs/keyboard-shortcuts.md`, `docs/features.md` (đoạn Editing), `CHANGELOG.md` `[Unreleased]` → `### Added`
- Test: `tests/untitled.rs`, `tests/app_menu.rs`

**Interfaces:**
- Consumes: Task 2 `Documents::new_untitled`.
- Produces: `Workbench::new_untitled_file(&mut self, window: &mut Window, cx: &mut Context<Self>)` — tạo khóa bằng `new_untitled`, mở tab ghim (`OpenMode::Pinned`) trong nhóm active qua `open_file` (không qua `absolute_path` / recent), focus view.

- [ ] **Step 1: Test**

```rust
#[gpui::test] fn cmd_n_opens_untitled_tab(cx)
// cx.simulate_keystrokes("cmd-n") → tab active có path "untitled:Untitled-1", pinned, focus ở code view;
// tiêu đề tab hiển thị == "Untitled-1".
#[gpui::test] fn untitled_title_follows_first_line(cx)
// gõ "\n  hello world\nsecond" → tiêu đề "hello world"; xóa hết → "Untitled-1".
#[gpui::test] fn untitled_numbers_are_reused(cx)
// ⌘N hai lần (1, 2); đóng tab 1 (rỗng, không hỏi); ⌘N → "untitled:Untitled-1".
```

`tests/app_menu.rs`: menu File bắt đầu bằng `["New File", "New Window", …]`; `welcome_disables_workbench_items` thêm `assert!(item(file, "New File").is_disabled())`.

- [ ] **Step 2: Chạy, thấy fail** — `cargo test -p lucerna-workbench --test untitled cmd_n untitled_title untitled_numbers --test app_menu` → FAIL.
- [ ] **Step 3: Cài đặt theo Files/Interfaces.** Lấy tên hiển thị của tab Untitled từ snapshot của `Documents` (`snapshot_of`) — chỉ đọc 1–2 dòng đầu, không duyệt cả buffer.
- [ ] **Step 4: Chạy, thấy pass** — cùng lệnh, cộng `--lib keys` → PASS.
- [ ] **Step 5: Commit** — `feat: Cmd+N opens an untitled tab`

---

### Task 5: Save As (Untitled và file thật)

**Files:**
- Modify: `documents.rs` (hàm ghi ra đích mới); `save.rs` (lệnh `workbench.action.files.saveAs` trong `register_commands` ~635; `save` 636 đi Save As khi active là Untitled; hàm hỏi chỗ lưu); `workbench.rs` (`pub fn save_as`; dùng `paths_renamed` 1278); `vim_ex.rs:51-109` (`:w!`, `:wq`, `:x` trên Untitled đi Save As; `:wq`/`:x` chỉ đóng khi lưu xong); `app_menu.rs` (File → "Save As…" ngay dưới "Save"); `keys.rs` (dòng `shift+cmd+s`); `default-keybindings.json`
- Docs: `docs/keyboard-shortcuts.md`, CHANGELOG
- Test: `tests/untitled.rs`, `tests/save.rs` (Save As file thật), `tests/vim_command_line.rs`

**Interfaces:**
- Consumes: Task 1 (`suggested_file_name`, `label`, `name`), Task 2, Task 4.
- Produces:
  - `pub fn save_dialog_start(wb: &Workbench, path: &Path) -> (PathBuf, String)` (trong `save.rs`) — thư mục bắt đầu và tên gợi ý: Untitled → (`root`, hoặc thư mục cha của file khi `is_single_file`) và `suggested_file_name(label)` hoặc `name(path)`; file thật → (thư mục cha, tên file).
  - `pub fn Documents::write_as(&mut self, from: &Path, to: &Path, window: &mut Window, cx: &mut Context<Self>) -> Task<Result<FileStamp, WriteError>>` — ghi text hiện tại của `from` ra `to` qua đúng luồng ghi atomic sẵn có (participants theo settings của `to`; Untitled ghi UTF-8 không BOM; file thật giữ encoding / BOM / line ending của nó); không kiểm tra xung đột (hộp thoại đã hỏi Replace). Không đổi khóa. (`FileStamp` = kiểu stamp mà `finish_save` đang dùng.)
  - `pub fn Workbench::save_as(&mut self, path: Arc<Path>, window: &mut Window, cx: &mut Context<Self>) -> Task<bool>` — `cx.prompt_for_new_path(dir, Some(name))`; Cancel / lỗi → `false`, không đổi gì. Chọn đúng `path` hiện tại → như Save thường. Ngược lại: `write_as`; thành công thì (a) nếu `to` đang mở: đóng các tab của `to` và bỏ tài liệu cũ của nó; (b) `paths_renamed(path, to)` (tài liệu, mọi tab, preview, nav); (c) đánh dấu sạch bằng stamp vừa ghi (hàm dùng chung với `finish_save`); trả `true`. Lỗi ghi: báo như lưu thường (`status_message`/prompt sẵn có), không đổi khóa, trả `false`.

- [ ] **Step 1: Test**

```rust
#[gpui::test] fn save_untitled_asks_for_path_and_becomes_file(cx)
// ⌘N, gõ "fn main() {}\n"; cx.simulate_new_path_selection(|dir| { assert_eq!(dir, root); Some(root.join("main.rs")) });
// ⌘S → did_prompt_for_new_path(); disk("main.rs") == "fn main() {}\n"; tab path == root/main.rs (tuyệt đối);
// is_dirty == false; language_of == Some("rust"); cmd-z vẫn hoàn tác được lần gõ.
#[gpui::test] fn save_untitled_suggests_first_line(cx)
// gõ "notes: todo\n" → closure nhận được tên gợi ý "notes todo" (kiểm qua test hook nếu GPUI cho đọc; không thì
// unit-test save_dialog_start trực tiếp).
#[gpui::test] fn save_untitled_cancel_keeps_it(cx)
// simulate_new_path_selection(|_| None) → tab vẫn "untitled:Untitled-1", dirty, root không có file mới.
#[gpui::test] fn save_untitled_over_open_file(cx)
// a.txt đang mở ở tab khác; lưu Untitled thành a.txt → đúng 1 tab có path a.txt, nội dung = của Untitled.
#[gpui::test] fn save_as_write_error_keeps_untitled(cx)
// đích trong thư mục chmod 0o555 → tab vẫn Untitled, dirty, chữ còn nguyên.
#[gpui::test] fn untitled_in_two_groups_both_become_file(cx)
// Untitled mở ở hai nhóm (split) → sau lưu, cả hai tab có path đích.
```

`tests/save.rs`:

```rust
#[gpui::test] fn save_as_moves_file_tab_and_keeps_old_file(cx)
// a.txt "old" trên đĩa, sửa thành "new" (chưa lưu); ⇧⌘S chọn b.txt →
// disk(a.txt) == "old"; disk(b.txt) == "new"; mọi tab của a.txt giờ là b.txt; không dirty; undo còn.
#[gpui::test] fn save_as_same_path_is_save(cx)
```

`tests/vim_command_line.rs`: `:w` trên Untitled gọi hộp thoại; `:wq` với Cancel không đóng tab; `:wq` lưu được thì đóng.

- [ ] **Step 2: Chạy, thấy fail** — `cargo test -p lucerna-workbench --test untitled --test save --test vim_command_line save_` → FAIL.
- [ ] **Step 3: Cài đặt theo Interfaces.** Lệnh `files.save` và `:w` khi đường dẫn active là Untitled gọi `Workbench::save_as`.
- [ ] **Step 4: Chạy, thấy pass** — cùng lệnh, cộng `--lib keys --test app_menu` → PASS.
- [ ] **Step 5: Commit** — `feat: Save As, and saving untitled files`

---

### Task 6: Đóng, thoát, Save All, autosave, hot exit với Untitled

**Files:**
- Modify: `save.rs:165` (`save_each`: Untitled đi `Workbench::save_as` lần lượt; Cancel ở đóng tab / thoát → dừng cả luồng; ở Save All → bỏ qua cái đó, đi tiếp); `save.rs:469-605` (autosave: `auto_save_paths`, `auto_save_document`, `auto_save_event` bỏ Untitled); `editor_groups/view.rs:215,249` (đóng: Untitled rỗng không hỏi — có sẵn nhờ `is_dirty`, chỉ cần test); `workbench.rs:1941,2038` (thoát với `hotExit: off` dùng `save_each` mới; `auto_save_paths` ở thoát bỏ Untitled); `hot_exit/tracker.rs:137` (`backup_of`: Untitled → `mtime: None, size: None`; Untitled rỗng → xóa backup); `documents.rs:2140,2949` (restore Untitled: không đọc đĩa); `workbench.rs:645-650` (Untitled khôi phục mở ở cuối nhóm đầu qua `open_restored`, giữ đúng `Untitled-N` từ header)
- Test: `tests/untitled.rs`, `tests/hot_exit.rs`

**Interfaces:**
- Consumes: Task 2, Task 5 (`Workbench::save_as`).
- Produces: `save_each` giữ chữ ký hiện tại (`fn save_each(documents, paths, &mut AsyncWindowContext) -> bool`) nhưng nhận thêm cách gọi Save As (tham số `WeakEntity<Workbench>` nếu cần — ghi vào ledger nếu đổi chữ ký).

- [ ] **Step 1: Test**

```rust
#[gpui::test] fn emptied_untitled_closes_silently(cx)
// ⌘N, gõ "x", xóa → ⌘W không có pending prompt; tab đóng.
#[gpui::test] fn closing_untitled_with_text_asks(cx)
// gõ "x"; ⌘W → pending prompt; "Save" + simulate_new_path_selection(Some(root/x.txt)) → file có, tab đóng;
// lần khác "Save" + chọn None → tab vẫn mở; "Don't Save" → tab đóng, không file.
#[gpui::test] fn autosave_skips_untitled(cx)
// files.autoSave "afterDelay", autoSaveDelay 10; gõ; chờ → vẫn Untitled, dirty, không file mới.
#[gpui::test] fn save_all_asks_for_each_untitled(cx)
// hai Untitled có chữ + a.txt dirty; saveAll; chọn đường dẫn cho cái 1, None cho cái 2 →
// a.txt lưu; Untitled-1 thành file; Untitled-2 vẫn dirty.
```

`tests/hot_exit.rs` (dùng `Project`, `open_with_data_dir`, `quit`, `reopen`, `tab_paths`, `text_of`):

```rust
#[gpui::test] fn reopen_restores_untitled(cx)
// ⌘N, gõ "draft"; quit; reopen → có tab "untitled:Untitled-1" ở cuối nhóm đầu, text "draft", dirty.
#[gpui::test] fn empty_untitled_leaves_no_backup(cx)
// ⌘N, gõ rồi xóa; quit → backups() rỗng; reopen → không có tab Untitled.
#[gpui::test] fn hot_exit_off_quit_asks_for_untitled_path(cx)
// files.hotExit "off"; Untitled có chữ; confirm_quit → "Save All" + chọn None → quit bị hủy.
```

- [ ] **Step 2: Chạy, thấy fail** — `cargo test -p lucerna-workbench --test untitled --test hot_exit` → FAIL.
- [ ] **Step 3: Cài đặt theo Files.**
- [ ] **Step 4: Chạy, thấy pass** — `cargo test -p lucerna-workbench` (cả crate) và `cargo clippy -p lucerna-workbench --all-targets -- -D warnings` và `cargo fmt --check` → PASS, sạch. (Nếu `lsp_settings_per_window` / `open_to_side_uses_second_group` chập chờn vì rust-analyzer thật, chạy lại riêng file đó và ghi vào ledger.)
- [ ] **Step 5: Đánh dấu xong** — `docs/roadmap.md`: `- [x] File mới (⌘N) và Save As`; spec: trạng thái "đã triển khai (branch …); kiểm tra thủ công §6 chưa làm".
- [ ] **Step 6: Commit** — `feat: untitled files through close, quit, Save All and hot exit`

---

### Kiểm tra thủ công (sau Task 6)

Chạy bản debug: ⌘N → gõ → ⌘S → lưu `foo.rs` trong project → có màu, LSP, Explorer hiện file; ⇧⌘S một file → file cũ còn nguyên; thoát với Untitled chưa lưu → mở lại còn nguyên; Welcome: menu New File mờ.
