# Nhóm 3 của đợt sửa P3 (Quick Open, search, recent): plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Sửa bốn lỗi P3 của nhóm 3 (T3-3, T5-1, T5-4, T5-5) trên nhánh `claude/p3-quick-open`, mỗi lỗi có test, rồi đánh dấu "Đã sửa" trong review.

**Architecture:** Bốn bản sửa độc lập, mỗi bản ở đúng chỗ review chỉ ra. T5-5 chỉ đụng `lucerna-project` (matcher và searcher cùng dùng CRLF). T5-4 đổi parser của Quick Open sang `i64` và để view tính dòng thật theo số dòng của editor. T5-1 cho closure `reveal` của tab Search kết thúc cả khi dòng không còn. T3-3 bỏ lọc khi ghi và khi hiển thị danh sách recent, rồi làm như VS Code khi chọn một mục không còn trên đĩa: gỡ mục khỏi danh sách và báo "Path does not exist" bằng hộp thoại chỉ có nút OK.

**Tech Stack:** Rust, GPUI (chỉ trong `lucerna-workbench`), `grep-regex` 0.1.14 / `grep-searcher` 0.1.17 / `grep-matcher`.

**Spec:** `docs/superpowers/specs/2026-11-17-p3-fixes-design.md`, §5 "Nhóm 3". Vị trí, cách tái hiện: `docs/superpowers/reviews/2026-11-15-release-flow-review.md`, các mục T3-3, T5-1, T5-4, T5-5.

## Global Constraints

- Chữ hiển thị trong app chỉ bằng tiếng Anh (`docs/design-principles.md` §3). Tiếng Việt chỉ có trong dữ liệu test.
- Chỉ `lucerna-workbench` dùng gpui. Không giả định macOS ngoài code riêng cho platform.
- Hành vi theo VS Code, giao diện theo Zed. Dùng prompt và status message có sẵn, không thêm loại UI mới.
- Cargo:
  - Chạy `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"` trước.
  - Chạy `scripts/prune-target.sh` trước mỗi lần build hoặc test.
  - Trước `cargo test`, xoá `target/release` nếu có, nhưng chỉ khi `pgrep -x cargo` không ra gì.
  - Không bao giờ đặt `CARGO_TARGET_DIR` hay truyền `--target-dir`; mọi worktree dùng chung `target` của checkout chính (`scripts/target-dir.sh`). Lock cargo bị giữ thì chờ.
  - Giữ hơn 1,2 GB đĩa trống.
- Commit: stage từng file bằng đường dẫn cụ thể (`git add <path>…`, `git commit -- <paths>`), không `git add -A`. Trailer: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- `cargo deny check` phải pass (plan này không thêm dependency).
- Không đụng `lucerna-vim` hay parser dòng lệnh `:` của V2b. `parse_query` là nghĩa **khi Vim tắt**; V2b viết hàm parse riêng (spec §5 T5-4, vim spec §8).

## Tham khảo (VS Code, `microsoft/vscode` nhánh `main`, đọc ngày 2026-11-17)

- **`src/vs/editor/contrib/quickAccess/browser/gotoLineQuickAccess.ts`, `parsePosition`:**
  - Tách bằng `value.split(/,|:|#/)`, lấy `parts[0]` là dòng và `parts[1]` là cột; phần sau bị bỏ.
  - Dòng âm: `lineNumber = (maxLine + 1) + lineNumber`, rồi kẹp vào `[1, maxLine]`. Vì vậy `:0` thành dòng 1, `:-1` là dòng cuối.
  - Cột âm cũng đếm từ cuối dòng. Spec của mình không nói tới; plan này giữ cột không âm (xem "Ghi chú").
  - `::N` là chế độ offset, Lucerna không có.
  - Lucerna đếm dòng theo `BufferSnapshot::line_count()`: newline ở cuối file không thêm dòng. Nên `:-1` là dòng có chữ cuối cùng, còn VS Code đi tới dòng trống sau newline đó. Giữ theo quy ước của Lucerna, giống cách `:999` đang kẹp.
- **Recent không còn tồn tại:**
  - VS Code không lọc khi hiển thị: Welcome, menu và picker hiện mọi mục.
  - Khi mở một mục không còn, `windowsMainService.ts` (`doExtractPathsFromAPI`) hiện hộp `type: 'info'`, chỉ có nút **OK**, với message "Path does not exist" và detail "The path '{0}' does not exist on this computer." (đường dẫn có `~`).
  - Mục đó bị gỡ tự động, không hỏi: `resolveOpenable` gọi `removeRecentlyOpened`, và `menubar.ts` cũng gỡ khi `open` thất bại.
  - Lucerna làm đúng như vậy (controller chốt 2026-11-17, sửa spec: câu Remove / Cancel trong spec là nhầm). Hộp thoại có message "Path does not exist" và detail "The path '{0}' does not exist on this computer.", với `{0}` là đường dẫn có `~`.

## Ghi chú cho controller (lệch hoặc thiếu so với spec)

1. **T3-3, theo VS Code thay cho spec.** Bản spec đầu ghi hộp thoại Remove / Cancel, nhưng VS Code không hỏi. Controller đã chốt theo VS Code và sửa spec: gỡ mục ngay, rồi báo bằng hộp thoại chỉ có **OK** (xem "Tham khảo").
2. **T3-3, menu Open Recent khi không có cửa sổ nào** (macOS, mọi cửa sổ đã đóng): không có cửa sổ để hiện hộp thoại, vì GPUI chỉ có `Window::prompt`. Plan gỡ mục như `menubar.ts` của VS Code, không mở gì và không báo. Spec không nói tới trường hợp này.
3. **T5-5, `.crlf(true)` một mình làm hỏng mọi lần tìm.** Matcher có `crlf(true)` báo line terminator CRLF. `Searcher` mặc định dùng LF, nên nó trả `ConfigError::MismatchedLineTerminators` cho mọi file. `search_file` chỉ ghi lỗi này ở mức debug, nên kết quả rỗng mà không báo gì. Vì vậy phải đặt thêm `SearcherBuilder::line_terminator(LineTerminator::crlf())`, như `rg --crlf`. Dòng vẫn tách theo `\n`, nên file LF không đổi.
4. **T5-4, cột âm** (`:5:-1`) vẫn coi là không có cột, tức đi tới đầu dòng. Spec chỉ nói về dòng.
5. **T5-1, doc load lỗi:** plan cũng gỡ observer khi doc ở `DocState::Error`. Nếu không, một lần reload thành công về sau sẽ làm con trỏ nhảy. Spec chỉ nói tới `Ready`.

## Review Focus

1. **Ổ bị rút, người dùng chọn mục đó, rồi cắm lại ổ:** mục đã bị gỡ khỏi danh sách và không tự quay lại. Đây là hành vi của VS Code. Mục chỉ có lại khi folder được mở lại (Open Folder, `luc`), và khi đó đứng đầu danh sách. → test `missing_folder_is_removed_and_reported` trong Task 5.
2. **`:0`, `:-N` lớn hơn số dòng, `:12,` hay `:,5`:** không panic, dòng kẹp vào `[1, số dòng]`, và nhãn hiện đúng dòng sẽ tới. → test trong Task 2.
3. **File LF và tìm không dùng regex sau khi searcher chuyển sang CRLF:** vẫn ra kết quả như cũ. → `literal_and_lf_searches_still_match` trong Task 1.
4. **Kết quả search trỏ vào file đã không đọc được (Error):** observer bị gỡ, nên khi file đọc được lại thì con trỏ không nhảy. → test trong Task 3.
5. **Mục recent kiểu File (không phải folder) không còn trong cửa sổ workbench:** cũng bị gỡ và báo như folder, không mở tab rỗng. → test `missing_file_in_picker_is_removed_and_reported` trong Task 5.

---

### Task 1: T5-5, regex `$` và `^` khớp file CRLF trong tab Search

**Files:**
- Modify: `crates/lucerna-project/src/search.rs` (import ở dòng 9–11; `RegexMatcherBuilder` ở dòng 137–139; `SearcherBuilder` ở dòng 184–188)
- Test: `crates/lucerna-project/tests/search.rs`

**Interfaces:**
- Consumes: không có.
- Produces: không có API mới. `search(...)` giữ chữ ký.

- [ ] **Step 1: Viết test hỏng** trong `tests/search.rs`, dùng các helper có sẵn `write`, `query`, `run`, `files`:

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

- [ ] **Step 2: Chạy, xem hỏng.** `cargo test -p lucerna-project --test search regex_anchors_match_crlf_lines` → FAIL (`f.len()` là 0). Test thứ hai lúc này đạt; nó canh cho Step 3.

- [ ] **Step 3: Sửa `search()` trong `src/search.rs`.**
  - Thêm `.crlf(true)` vào `RegexMatcherBuilder::new()`.
  - Thêm `.line_terminator(LineTerminator::crlf())` vào `SearcherBuilder::new()`, với `use grep_matcher::LineTerminator;`.
  - Thêm một dòng comment: matcher và searcher phải cùng line terminator, nếu không searcher từ chối mọi file (Ghi chú 3).
  - `line_match` đã cắt `\n` và `\r` trước khi lấy range, nên không phải sửa.

- [ ] **Step 4: Chạy.** `cargo test -p lucerna-project --test search` → mọi test PASS, gồm cả hai test mới và các test cũ như `regex_mode`, `context_*`, `unsaved_buffer_*`.

- [ ] **Step 5: Commit.**

```bash
git add crates/lucerna-project/src/search.rs crates/lucerna-project/tests/search.rs
git commit -m "fix(search): regex anchors match CRLF lines (T5-5)" -- crates/lucerna-project/src/search.rs crates/lucerna-project/tests/search.rs
```
(message kèm trailer ở Global Constraints; các commit sau cũng vậy.)

---

### Task 2: T5-4, Quick Open nhận `:-N`, `:N,M`, `:N#M`

**Files:**
- Modify: `crates/lucerna-workbench/src/quick_open/query.rs` (`QuickOpenQuery::GotoLine`, `parse_query`, thêm `resolve_line`)
- Modify: `crates/lucerna-workbench/src/quick_open/mod.rs:11` (re-export), `:148`, `:171` (`Sources`)
- Modify: `crates/lucerna-workbench/src/quick_open/view.rs` (`Sources::has_editor` ở dòng 54, field `QuickOpen::has_editor` ở 138/195, nhánh `GotoLine` ở 413–419)
- Modify: `crates/lucerna-workbench/src/lib.rs:125-127` (export `resolve_line`)
- Test: `crates/lucerna-workbench/tests/quick_open_query.rs`, `crates/lucerna-workbench/tests/quick_open.rs`

**Interfaces:**
- Consumes: `BufferSnapshot::line_count(&self) -> u32` (`lucerna-text`, không tính newline cuối), `CodeView::snapshot()`.
- Produces:
  - `QuickOpenQuery::GotoLine { line: Option<i64>, column: Option<u32> }`.
  - `pub fn resolve_line(line: i64, line_count: u32) -> u32`: dòng 1-based thật. Số âm đếm từ cuối (`line_count + 1 + line`), rồi kẹp vào `[1, max(line_count, 1)]`.
  - `Sources::editor_lines: Option<u32>` thay cho `has_editor: bool`. Field `QuickOpen::editor_lines` cũng vậy. `None` nghĩa là không có code view.
  - `Row::GoToLine` và `QuickOpenEvent::GoToLine { line: u32, column: Option<u32> }` giữ nguyên; `line` là dòng đã resolve.

- [ ] **Step 1: Sửa và thêm test trong `tests/quick_open_query.rs`.**
  - `colon_line`, `colon_line_col`: đổi `Some(12)` thành `Some(12i64)` cho khớp kiểu mới.
  - Trong `colon_garbage_none`, `":-3"` giờ là `line: Some(-3), column: None`.
  - Thêm:

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

- [ ] **Step 2: Thêm test workbench** `goto_line_negative_and_comma` trong `tests/quick_open.rs`, dùng `open`, `open_long_txt`, `keys`, `type_text`, `rows`, `cursor` sẵn có (`long.txt` có 100 dòng):
  - `ctrl-g`, gõ `-1`: `rows == ["Go to line 100"]`. Enter: `cursor == Point { row: 99, column: 0 }`.
  - `ctrl-g`, gõ `12,5`: `rows == ["Go to line 12, column 5"]`. Enter: `Point { row: 11, column: 4 }`.
  - `ctrl-g`, gõ `-500`: `rows == ["Go to line 1"]`.
  - `ctrl-g`, gõ `0`: `rows == ["Go to line 1"]`.

  Test `goto_line_without_editor` giữ nguyên ("Open a file to go to a line").

- [ ] **Step 3: Chạy, xem hỏng.** `cargo test -p lucerna-workbench --test quick_open_query --test quick_open` → lỗi biên dịch hoặc FAIL (`resolve_line` chưa có, `:-1` ra `None`).

- [ ] **Step 4: Sửa.**
  - `parse_query`: phần sau `:` tách bằng `rest.split([':', ',', '#'])`. Phần 1 parse `i64`, phần 2 parse `u32`, cả hai sau `trim()`. Bỏ qua phần thứ ba trở đi.
  - Doc comment của `parse_query` ghi rõ: đây là nghĩa khi Vim tắt, theo `gotoLineQuickAccess` của VS Code (MIT, chỉ tham khảo hành vi). Lệnh `:` của Vim (V2b) dùng parser riêng.
  - Viết `resolve_line` trong `query.rs`, tính bằng `i64` có saturating để `i64::MIN` không tràn. Export ở `quick_open/mod.rs` và `lib.rs`.
  - `Workbench::show_quick_open` (`mod.rs:148`): `editor_lines: self.active_code_view(cx).map(|v| v.read(cx).snapshot().line_count())`. `show_extension_ids` (`:171`): `editor_lines: None`.
  - Trong `QuickOpen::query_changed`:
    - `editor_lines` là `None` → "Open a file to go to a line".
    - `Some(n)` và có dòng → `Row::GoToLine { line: resolve_line(line, n), column }`.
    - Không có dòng → "Type a line number…".
  - `Workbench::go_to_line` giữ nguyên; nó vẫn kẹp, phòng khi file đổi lúc Quick Open đang mở.

- [ ] **Step 5: Chạy.** `cargo test -p lucerna-workbench --test quick_open_query --test quick_open` → PASS, gồm `goto_line_moves_view` cũ.

- [ ] **Step 6: Commit** `fix(quick-open): accept :-N, :N,M and :N#M as VS Code does (T5-4)`, kèm năm file nguồn và hai file test ở trên.

---

### Task 3: T5-1, kết quả search ở dòng không còn: không để lại observer

**Files:**
- Modify: `crates/lucerna-workbench/src/search_view/view.rs:740-776` (closure `reveal` trong `SearchView::open_line`)
- Test: `crates/lucerna-workbench/tests/search_view.rs`

**Interfaces:**
- Consumes: `CodeView::doc_state() -> &DocState`, `CodeView::go_to(Point, cx)`, `CodeView::set_match_highlights(Vec<Range<usize>>, Option<usize>, cx)`, `BufferSnapshot::line_count() -> u32`.
- Produces: không có API mới. `open_line` giữ chữ ký.

- [ ] **Step 1: Viết test hỏng** `stale_result_past_the_end_goes_to_last_line_once`, dùng `project`, `workbench`, `keys`, `type_text`, `settle_search`, `search`, `click`, `active_view`, `draw` sẵn có:
  1. `a.rs` có 120 dòng `l{i}`, dòng 100 là `needle`, nối bằng `\n`.
  2. Tìm `needle`, rồi `fs::write` lại `a.rs` thành 50 dòng `l1`…`l50` nối bằng `\n`, không có newline cuối.
  3. `click("search-line-0-0-<dòng khớp>")`: lấy index của dòng khớp từ excerpt, như `click_result_opens_and_positions`.
  4. `draw`. Assert `cursor == Point { row: 49, column: 0 }` và `match_highlights()` rỗng.
  5. Gõ 60 lần `enter` trong editor (`keys`), mỗi lần `draw`. Assert `cursor().row == 109`. Trước khi sửa, observer kéo con trỏ về dòng 99 khi file vượt 100 dòng.

- [ ] **Step 2: Viết test** `stale_result_in_unreadable_file_drops_the_observer` (`#[cfg(unix)]`, tự bỏ qua khi chạy bằng root giống `unreadable_file_is_not_overwritten` trong `recent.rs`):
  1. Như Step 1, nhưng trước khi click thì `chmod 000` file `a.rs`. Click, `draw`.
  2. Trả quyền `0o644`, ghi lại file 120 dòng, rồi gọi reload của tab qua watcher. Dùng helper đang có trong `tests/external_changes.rs` cho việc chờ reload; nếu không có thì `run_until_parked` lặp như `settle_search`.
  3. Assert con trỏ không ở `row: 99`, và không có highlight.

- [ ] **Step 3: Chạy, xem hỏng.** `cargo test -p lucerna-workbench --test search_view stale_result` → FAIL ở cả hai test (con trỏ về dòng 99).

- [ ] **Step 4: Sửa closure `reveal`.** Kết quả trả về: `true` nghĩa là xong và gỡ observer.
  - `DocState::Ready` và `row >= line_count`:
    - `go_to(Point { row: line_count.saturating_sub(1), column: 0 })`;
    - `set_match_highlights(Vec::new(), None, cx)`;
    - `tracing::debug!` một dòng;
    - trả `true`.
  - `DocState::Error(_)`: trả `true` và không làm gì.
  - `Loading`, `Deleted`: trả `false` như cũ.
  - Sửa doc comment của `open_line` cho khớp.

- [ ] **Step 5: Chạy.** `cargo test -p lucerna-workbench --test search_view` → PASS toàn file.

- [ ] **Step 6: Commit** `fix(search): a result past the end of its file goes to the last line once (T5-1)`, kèm `view.rs` và `tests/search_view.rs`.

---

### Task 4: T3-3 phần 1, danh sách recent giữ mục không còn và hiện mọi mục

**Files:**
- Modify: `crates/lucerna-workbench/src/recent.rs`:
  - `record` (123–134) và `remove` (136–144): bỏ `.existing()` và sửa doc comment.
  - Xoá `RecentList::existing` (92–101) cùng test `existing_drops_missing_and_wrong_kind`.
  - Giữ `RecentEntry::exists`: `startup::last_workspace` và Task 5 dùng nó.
- Modify: `crates/lucerna-workbench/src/welcome.rs:180`, `src/open_commands.rs:224`, `src/app_menu.rs:651`: `.existing()` → `.entries`. Sửa doc comment "that existed" / "that still exist" ở `welcome.rs:146` và `open_commands.rs:209`.
- Test: `src/recent.rs` (mod tests), `tests/welcome.rs`, `tests/app_menu.rs`

**Interfaces:**
- Consumes: không có.
- Produces: `recent::record` / `recent::remove` không còn bỏ mục thiếu. `RecentList::existing` không còn. Welcome, picker và menu nhận `RecentList.entries` nguyên vẹn (vẫn bỏ đường dẫn của chính cửa sổ như trước).

- [ ] **Step 1: Sửa test.**
  - `recent.rs`: đổi `writes_drop_missing_entries` thành `writes_keep_missing_entries`:
    - record `a`, `remove_dir(a)`, record `b` → `paths == [b, a]`;
    - rồi `remove(&data, &b)` → `paths == [a]`.
  - `tests/welcome.rs`: đổi `skips_missing_folders` thành `lists_missing_folders`:
    - `Fixture::new(&["a", "gone", "b"], &["gone"])` → `recent == [a, gone, b]`;
    - `drawn("welcome-recent-2")`.
  - `tests/app_menu.rs`: thêm `open_recent_menu_lists_missing_entries`, theo mẫu `open_recent_menu_hides_the_window_folder`:
    - record `other` rồi `this`;
    - `remove_dir(&other)` trước `app_menu::install`;
    - assert `shown(&other)`.

- [ ] **Step 2: Chạy, xem hỏng.** `cargo test -p lucerna-workbench --lib recent:: && cargo test -p lucerna-workbench --test welcome --test app_menu` → ba test trên FAIL.

- [ ] **Step 3: Sửa** theo danh sách Files.

- [ ] **Step 4: Chạy lại** lệnh Step 2. Lúc này `vanished_folder_shows_warning` (welcome) và `vanished_folder_in_picker_reports` (open_commands) vẫn đạt: đường mở vẫn kiểm tra tồn tại, Task 5 mới đổi. → PASS.

- [ ] **Step 5: Commit** `fix(recent): keep entries whose path is gone, as VS Code does (T3-3)`, kèm `recent.rs`, `welcome.rs`, `open_commands.rs`, `app_menu.rs`, `tests/welcome.rs`, `tests/app_menu.rs`.

---

### Task 5: T3-3 phần 2, chọn một mục không còn thì gỡ nó và báo "Path does not exist"

**Files:**
- Modify: `crates/lucerna-workbench/src/recent.rs`: thêm `MISSING_TITLE` và `missing_detail`.
- Modify: `crates/lucerna-workbench/src/open_commands.rs`:
  - thêm `report_missing`;
  - `forget` (167) thành `pub(crate)`;
  - sửa `Workbench::open_recent_entry` (308–345).
- Modify: `crates/lucerna-workbench/src/welcome.rs`:
  - `open_entry` (372–399) nhận thêm `window`;
  - xoá field `warning`, `warning()` (233–236), phần render `welcome-warning` (734–737, 779), và import `warning_row` nếu không còn dùng;
  - sửa các chỗ gọi ở `press` (366) và `on_picker_event` (444).
- Modify: `crates/lucerna-workbench/src/app_menu.rs:844-863` (`open_recent_entry`)
- Test: `src/recent.rs` (mod tests), `tests/welcome.rs`, `tests/open_commands.rs`, `tests/app_menu.rs`

**Interfaces:**
- Consumes: `RecentEntry::exists(&self) -> bool`; `tilde_path`; `Window::prompt`; `WelcomeView::forget(&mut self, PathBuf, cx)` (sẵn có, private).
- Produces:
  - `pub(crate) const MISSING_TITLE: &str = "Path does not exist";` trong `recent.rs`.
  - `pub(crate) fn missing_detail(path: &Path, home: Option<&Path>) -> String` trong `recent.rs`. Trả đúng `format!("The path '{}' does not exist on this computer.", tilde_path(path, home))`. Hai chuỗi lấy nguyên văn từ `pathNotExistTitle` / `pathNotExistDetail` trong `windowsMainService.ts` của VS Code (MIT); ghi nguồn bằng comment.
  - `pub(crate) fn report_missing(path: &Path, window: &mut Window, cx: &mut App)` trong `open_commands.rs`. Gọi `window.prompt(PromptLevel::Info, MISSING_TITLE, Some(&missing_detail(path, std::env::home_dir().as_deref())), &["OK"], cx)` và bỏ qua câu trả lời (không cần chờ). Hàm không gỡ mục; người gọi gỡ trước.
  - `pub(crate) fn forget(path: PathBuf, cx: &mut App)` (`open_commands.rs`, trước là private).
  - `WelcomeView::open_entry(&mut self, entry: RecentEntry, new_window: bool, window: &mut Window, cx: &mut Context<Self>)`.

- [ ] **Step 1: Viết test.** Trong các test dưới, `detail(path)` là `format!("The path '{}' does not exist on this computer.", <path, có ~ nếu nằm dưới std::env::home_dir()>)`.
  - `recent.rs` mod tests: `missing_detail_uses_tilde`.

```rust
assert_eq!(MISSING_TITLE, "Path does not exist");
assert_eq!(
    missing_detail(Path::new("/Users/me/code/x"), Some(Path::new("/Users/me"))),
    "The path '~/code/x' does not exist on this computer."
);
```

  - `tests/welcome.rs`: thay `vanished_folder_shows_warning` bằng `missing_folder_is_removed_and_reported`.
    1. `Fixture::new(&["a", "b"], &[])`, rồi `remove_dir(a)`. `click("welcome-recent-0")`, `draw`.
    2. Assert `cx.pending_prompt() == Some(("Path does not exist".into(), detail(a)))`.
    3. Assert ngay khi prompt còn mở: `recent == [b]`, `fixture.stored() == [b]`, không có event. Mục bị gỡ trước khi người dùng bấm OK, như VS Code.
    4. `simulate_prompt_answer("OK")`, `draw`. Assert không còn prompt, danh sách vẫn `[b]`.
    5. `create_dir(a)` (ổ được cắm lại), `draw`. Assert `fixture.stored() == [b]`: mục không tự quay lại.
  - `tests/open_commands.rs`: thay `vanished_folder_in_picker_reports` bằng `missing_folder_in_picker_is_removed_and_reported`.
    1. `remove_dir(other)` **trước** `ctrl-r`. Assert picker `row_texts()` có `other`.
    2. `enter`. Assert prompt `("Path does not exist", detail(other))`. `simulate_prompt_answer("OK")`, `settle`.
    3. Assert `shown_root == root`, `window_count == 1`, `fixture.stored() == [third]`, và status message không chứa "no longer exists".
  - `tests/open_commands.rs`: thêm `missing_file_in_picker_is_removed_and_reported`:
    - viết tay vào `recent.json` một mục `{"path": <base>/gone.txt, "kind": "file"}` (không tạo file), cạnh `third`;
    - `ctrl-r`, chọn `gone.txt`, `enter` → prompt `("Path does not exist", detail(gone.txt))`;
    - `simulate_prompt_answer("OK")`; assert `stored() == [third]` và không có tab mới.
  - `tests/app_menu.rs`: thêm `missing_entry_without_window_is_forgotten`:
    - `app_state`, ghi `recent.json` có `gone` (không tạo) và `kept` (có tạo);
    - `cx.update(|cx| { app_menu::install(cx); cx.dispatch_action(&OpenRecentEntry { entry: gone }) })`, rồi `run_until_parked`;
    - assert `recent::load(..).entries` chỉ còn `kept` và `cx.windows().is_empty()`.

- [ ] **Step 2: Chạy, xem hỏng.** `cargo test -p lucerna-workbench --lib recent:: && cargo test -p lucerna-workbench --test welcome --test open_commands --test app_menu` → các test mới FAIL: chưa có prompt, Welcome còn hiện cảnh báo.

- [ ] **Step 3: Sửa.** Thứ tự ở mọi đường là gỡ trước, báo sau, như `resolveOpenable` của VS Code.
  - Viết `MISSING_TITLE`, `missing_detail`, `report_missing` theo Interfaces.
  - `Workbench::open_recent_entry`: mục không tồn tại → `forget(entry.path.clone(), cx)` rồi `report_missing(&entry.path, window, cx)`. Bỏ status message "no longer exists".
  - `WelcomeView::open_entry`:
    - đổi sang `cx.spawn_in(window, …)` để có `window` trong `update_in`;
    - mục không tồn tại → `view.forget(entry.path.clone(), cx)` (gỡ khỏi trang và khỏi file, rồi `app_menu::refresh`), rồi `report_missing(&entry.path, window, cx)`.
  - `app_menu::open_recent_entry`:
    - nhánh có cửa sổ Welcome gọi `view.open_entry(entry, false, window, cx)`;
    - nhánh không có cửa sổ: `cx.spawn` kiểm tra `entry.exists()` trên `background_spawn`. Có thì `router::open` như cũ; không có thì `open_commands::forget(entry.path, cx)` và không báo gì (Ghi chú 2).
  - Xoá phần cảnh báo của Welcome theo danh sách Files.

- [ ] **Step 4: Chạy lại** lệnh Step 2, cộng `--test recent_picker --test startup` nếu có file đó (`ls crates/lucerna-workbench/tests`) → PASS. Chạy `cargo clippy -p lucerna-workbench --tests` → không có cảnh báo mới (import thừa, field chết).

- [ ] **Step 5: Commit** `fix(recent): drop a recent entry whose path is gone and say so, as VS Code does (T3-3)`, kèm `recent.rs`, `open_commands.rs`, `welcome.rs`, `app_menu.rs` và bốn file test.

---

### Task 6: Đánh dấu "Đã sửa" trong review, kiểm tra toàn bộ

**Files:**
- Modify: `docs/superpowers/reviews/2026-11-15-release-flow-review.md` (các mục T3-3 ở dòng 229, T5-1 ở 258, T5-4 ở 271, T5-5 ở 278)

**Interfaces:**
- Consumes: SHA các commit của Task 1–5 (`git log --oneline main..HEAD`).
- Produces: không có.

- [ ] **Step 1: Kiểm tra toàn bộ.**
  - `cargo test -p lucerna-project -p lucerna-workbench` → PASS.
  - `cargo clippy --workspace --tests` → sạch.
  - `cargo deny check` → pass.

- [ ] **Step 2: Thêm dòng "Đã sửa".** Ghi một dòng cuối mỗi mục, đúng dạng các mục đã sửa trong file (`- **Đã sửa** (commit <sha>): …`):
  - **T3-3** (sha Task 4, Task 5): danh sách recent giữ mục không còn trên đĩa. Welcome, Open Recent và menu hiện mọi mục. Chọn mục không còn thì nó bị gỡ khỏi danh sách và hiện hộp thoại "Path does not exist" / "The path '…' does not exist on this computer." chỉ có nút OK, như VS Code. Menu khi không có cửa sổ thì gỡ mà không báo.
  - **T5-1** (sha Task 3): dòng không còn thì con trỏ về dòng cuối và observer bị gỡ, kể cả khi file không đọc được.
  - **T5-4** (sha Task 2): `:-N` đếm từ cuối; dòng và cột tách bằng `:`, `,` hoặc `#`; dòng kẹp vào `[1, số dòng]` như VS Code. Áp dụng khi Vim tắt.
  - **T5-5** (sha Task 1): matcher `crlf(true)` cùng searcher line terminator CRLF.

  Không sửa bảng tóm tắt hay dòng "Các mục P3 còn lại chưa sửa"; các nhánh nhóm khác cũng sửa file này, nên phần đó cập nhật sau khi gộp vào `claude/p3-fixes` (spec §5 "Sau khi gộp").

- [ ] **Step 3: Commit** `docs(review): mark T3-3, T5-1, T5-4, T5-5 fixed`, chỉ kèm file review.
