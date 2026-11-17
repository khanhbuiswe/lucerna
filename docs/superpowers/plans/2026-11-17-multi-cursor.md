# Phase 4.3 Multi-cursor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Multi-cursor như VS Code: ⌘D và các lệnh liên quan, ⌥⌘↑/↓, ⇧⌥I, chọn cột, ⌥-click, ⌥-kéo, ⇧⌥-kéo, Escape hai bước, paste `spread`/`full`, đếm selection ở status bar. Bỏ `block_selection`. Khi Vim bật, lệnh multi-cursor đưa Vim vào Insert.

**Architecture:**
- `lucerna-editor` (thuần) có thêm `find` (tìm trong buffer dùng chung cho thanh Find và ⌘D, cả từ theo `wordSeparators`) và `multicursor` (hàm `SelectionSet → SelectionSet`). Clipboard sửa theo VS Code.
- `lucerna-workbench`: lệnh, phím, chuột, trạng thái phiên (⌘D, stack ⌥⌘↑/↓, chọn cột) trong `CodeView`; Vim nghe sự kiện mới `CodeViewEvent::MultiCursor` để vào Insert.
- Chọn cột và `<C-v>` là nhiều selection thật; đường vẽ khối bị xóa.

**Tech Stack:** Rust 2024, `lucerna-text`, `lucerna-editor` (+ `regex`, đã có trong workspace), `lucerna-vim`, gpui (chỉ workbench).

**Spec:** `docs/superpowers/specs/2026-11-17-multi-cursor-design.md` (đọc hết). Spec cha: `docs/superpowers/specs/2026-11-11-lucerna-phase4-editing-design.md` §8, §14. Nghiên cứu: `docs/research/2026-11-11-editing.md` §3.

## Global Constraints

- **Chữ trong app chỉ tiếng Anh**, nguyên văn: `The number of cursors has been limited to N.` (N là số), `N selections`, `N selections (M characters selected)`, `Ln x, Col y (M selected)`. Tiếng Việt chỉ có trong dữ liệu test.
- **Crate:** chỉ `lucerna-workbench` dùng gpui. `lucerna-editor` thêm `regex.workspace = true`, không thêm dependency nào khác.
- **Giấy phép:** không chép code Zed (GPL). Code phỏng theo VS Code ghi `//! Adapted from microsoft/vscode <path> (MIT)` ở đầu module như `clipboard.rs`: `multicursor.rs` ← `src/vs/editor/contrib/multicursor/browser/multicursor.ts`, `src/vs/editor/common/cursor/cursorMoveCommands.ts`, `src/vs/editor/common/cursor/cursorColumnSelection.ts`; `find.rs` ← `src/vs/editor/common/model/textModelSearch.ts` (`isValidMatch`).
- **Vị trí:** offset byte UTF-8, LF-normalised như `SelectionSet`; không cắt đôi ký tự.
- **Setting mới (mặc định):** `editor.multiCursorModifier: "alt"` (`"alt"` | `"ctrlCmd"`), `editor.multiCursorMergeOverlapping: true`, `editor.multiCursorPaste: "spread"` (`"spread"` | `"full"`), `editor.multiCursorLimit: 10000`. **Không** thêm `editor.selectedTextMatchMode`.
- **Phím:** đúng bảng §5.2 của spec, thêm vào `crates/lucerna-settings/assets/default-keybindings.json`.
- **Build:**
  - `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"`.
  - Chạy `scripts/prune-target.sh` trước **mọi** lần `cargo test` hay build.
  - Không đặt `CARGO_TARGET_DIR` hay `--target-dir`. Cargo lock bị giữ thì chờ.
  - Giữ một profile: xóa `target/release` trước khi test, trừ khi `pgrep -x cargo` cho thấy cargo khác đang chạy.
- **Git (thư mục dùng chung):** `git status` trước mỗi commit; merge dở hoặc file staged lạ thì dừng, báo BLOCKED. Commit theo đường dẫn: `git commit -m "…" -- <paths>`. Không rebase / stash / amend. Trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo deny check` qua ở cuối mỗi task chạm code.

## Review Focus

1. **⌘D trên chữ tiếng Việt và chữ nhiều byte.** Từ `chào` xuất hiện ba lần (một lần trong `chàobạn`): ⌘D ×3 chọn đúng hai từ đứng riêng rồi quay vòng không thêm, không panic ở biên ký tự. Test: `Task 2: add_next_vietnamese_whole_word`.
2. **⌘D khi agent ghi đè file giữa hai lần bấm.** Reload làm selection remap; lần ⌘D kế tìm trên snapshot mới, không dùng chữ cũ nằm ở offset cũ. Test: `Task 6: cmd_d_after_external_reload`.
3. **⇧⌘L trên file > 1 MB với hàng chục nghìn kết quả.** Bị cắt ở `multiCursorLimit`, hiện thông báo, UI không treo (chạy nền). Test: `Task 6: select_highlights_limit_large_file`.
4. **Chọn cột qua dòng có tab và dòng rỗng.** Cột tính theo độ rộng hiển thị; dòng ngắn được con trỏ rỗng ở cuối dòng; gõ rồi undo một bước trả lại nguyên văn. Test: `Task 4: column_select_tabs_and_short_lines`, `Task 7: shift_alt_drag_then_type_and_undo`.
5. **Vim bật, ⌘D rồi gõ rồi `u`.** Một bước undo, về Normal một con trỏ. Test: `Task 8: vim_cmd_d_insert_escape_undo`.

---

## Quyết định chốt trong plan (spec để builder)

- **Cờ gộp selection** nằm trong `SelectionSet` (`merge_overlapping: bool`, mặc định `true`), để `remap` và mọi chỗ gộp sau đó theo setting mà không phải truyền tham số qua mọi hàm. `CodeView` đặt cờ mỗi khi cài một set mới.
- **`find_next` chỉ cho chữ literal** (⌘D luôn literal). Nó quét theo cửa sổ 256 KiB bắt đầu ở đầu dòng, chồng nhau `4 × số ký tự của mẫu` byte, nên không copy cả buffer. `find_all` (thanh Find, ⇧⌘L) vẫn copy một lần như hiện nay.
- **Ngưỡng nền của ⇧⌘L** = `BACKGROUND_SEARCH_BYTES` (1 MiB), giống thanh Find.
- **Chọn cột** theo dòng buffer và cột hiển thị có tính tab (`tab_size`), kể cả khi word wrap bật. ⌥⌘↑/↓ dùng `Layout::vertical` hiện có nên theo dòng hiển thị.
- **Số ký tự ở status bar** cache theo `selection_generation` trong `CodeView`.

---

### Task 1: Tìm trong buffer dùng chung (`lucerna-editor::find`)

**Files:**
- Create: `crates/lucerna-editor/src/find.rs`
- Modify: `crates/lucerna-editor/src/lib.rs`, `crates/lucerna-editor/Cargo.toml`, `crates/lucerna-workbench/src/find_widget.rs:30-72`, `crates/lucerna-workbench/src/lib.rs:101`, `crates/lucerna-workbench/src/vim.rs` (import)
- Test: unit test trong `find.rs`; `crates/lucerna-workbench/tests/find_widget.rs` phải qua

**Interfaces:**
- Produces:
  ```rust
  pub struct FindQuery<'a> { pub pattern: &'a str, pub is_regex: bool, pub case_sensitive: bool, pub whole_word: bool, pub word_separators: &'a str }
  pub fn build_regex(pattern: &str, is_regex: bool, case_sensitive: bool) -> Result<regex::Regex, String>
  pub fn find_all(snap: &BufferSnapshot, q: &FindQuery) -> Result<Vec<Range<usize>>, String>
  pub fn find_next(snap: &BufferSnapshot, q: &FindQuery, from: usize, backwards: bool) -> Option<Range<usize>>
  pub fn is_whole_word(snap: &BufferSnapshot, range: Range<usize>, separators: &str) -> bool
  ```
  `lucerna_workbench::find_in_snapshot(snapshot, pattern, is_regex, case, whole_word)` giữ chữ ký cũ, gọi `find_all` với `DEFAULT_WORD_SEPARATORS` (cho Vim và test cũ).

- [ ] **Step 1: Viết test hỏng trong `find.rs`**
  - `whole_word_uses_separators`: text `$foo foo-bar $foo`, separators của `test_config()` **bỏ** `$` và `-` → tìm `$foo` cả từ ra 2 kết quả `[0..4, 13..17]`; tìm `foo-bar` cả từ ra `[5..12]`.
  - `whole_word_default_separators`: separators mặc định, `foo` cả từ trong `foo foobar (foo)` ra `[0..3, 12..15]`.
  - `find_next_wraps_and_backwards`: `a x a x a`, mẫu `a`, `from=3` tiến → `4..5`; `from=9` tiến → quay về `0..1`; lùi từ `4` → `0..1`.
  - `find_next_across_window_boundary`: text 600 KiB toàn `x`, chèn `needle` tại offset 262_140 (vắt qua biên 256 KiB) → `find_next(.., 0, false) == Some(262_140..262_146)`.
  - `find_next_case_folding_multibyte`: `CHÀO chào`, không phân biệt hoa thường, từ 0 → `0..5`, từ 5 → `6..11`.
  - `invalid_regex_errors`: `find_all` với `(` regex → `Err`.

- [ ] **Step 2: Chạy, xác nhận hỏng**
  Run: `cargo test -p lucerna-editor find::`
  Expected: FAIL (module chưa có).

- [ ] **Step 3: Cài đặt**
  - `build_regex`: `regex::escape` khi literal, `case_insensitive(!case_sensitive)`, `multi_line(true)` (như hiện tại, bỏ phần `\b`).
  - `is_whole_word` theo `isValidMatch` của VS Code: trái hợp lệ khi `start == 0`, hoặc ký tự trước `start` không phải `Regular` (`words::classify`), hoặc ký tự đầu của kết quả không phải `Regular`; phải đối xứng với ký tự sau `end` và ký tự cuối kết quả.
  - `find_all`: như `find_in_snapshot` cũ, lọc rỗng, lọc `is_whole_word` khi `whole_word`.
  - `find_next`: `debug_assert!(!q.is_regex)`; cửa sổ như mục "Quyết định"; bỏ kết quả rỗng / không cả từ; hết một vòng không thấy → `None`.
  - Thanh Find gọi `find_all` với `word_separators` lấy từ `edit_config` của view.

- [ ] **Step 4: Chạy test**
  Run: `cargo test -p lucerna-editor find:: && cargo test -p lucerna-workbench --test find_widget --test vim --test vim_editing`
  Expected: PASS. Nếu một test Find cũ hỏng vì `\b` → sửa kỳ vọng, ghi lý do trong commit message.

- [ ] **Step 5: Commit**
  `git commit -m "refactor(find): shared buffer find in lucerna-editor, whole word by word separators" -- <các file trên>`

---

### Task 2: ⌘D, ⌘K ⌘D, ⇧⌘L (`multicursor`, phần tìm)

**Files:**
- Create: `crates/lucerna-editor/src/multicursor.rs`
- Modify: `crates/lucerna-editor/src/lib.rs`, `crates/lucerna-editor/src/selection.rs` (cờ gộp)
- Test: unit test trong `multicursor.rs`, `selection.rs`

**Interfaces:**
- Consumes: Task 1 `FindQuery`, `find_next`, `find_all`.
- Produces:
  ```rust
  #[derive(Clone, Copy, Debug, PartialEq)] pub struct MatchOptions { pub case_sensitive: bool, pub whole_word: bool }
  #[derive(Clone, Debug, PartialEq)] pub struct MatchSession { pub text: String, pub options: MatchOptions }
  pub struct Step { pub set: SelectionSet, pub session: Option<MatchSession>, pub seeded_word: Option<String>, pub limited: bool }
  pub fn add_match(snap: &BufferSnapshot, set: &SelectionSet, session: Option<&MatchSession>, find: MatchOptions, separators: &str, backwards: bool, limit: usize) -> Option<Step>
  pub fn move_match(snap: &BufferSnapshot, set: &SelectionSet, session: Option<&MatchSession>, find: MatchOptions, separators: &str, backwards: bool) -> Option<Step>
  pub fn select_all_matches(snap: &BufferSnapshot, set: &SelectionSet, session: Option<&MatchSession>, find: MatchOptions, separators: &str, limit: usize) -> Option<Step>
  // selection.rs
  impl SelectionSet { pub fn set_merge_overlapping(&mut self, merge: bool); pub fn merge_overlapping(&self) -> bool; pub fn newest(&self) -> &Selection }
  ```
  `find` là tùy chọn hiện tại của thanh Find (mặc định `false/false` khi chưa mở). `seeded_word` là từ vừa mở từ con trỏ rỗng (workbench điền vào thanh Find nếu đang mở).

- [ ] **Step 1: Test hỏng** (dùng `marked`/`render`)
  - `add_first_expands_words`: `«ab»c x|y |z` → `«ab»c «xy» «z»`, `seeded_word == None` (nhiều từ), `session == None`.
  - `add_first_single_cursor_seeds`: `fo|o bar foo` → `«foo» bar foo`, `seeded_word == Some("foo")`, `session == Some({text: "foo", options: {case_sensitive: true, whole_word: true}})`.
  - `add_next_wraps_skips_overlap`: `foo «foo»* foo` (primary giữa) → thêm `foo` thứ ba; lần nữa → quay vòng thêm cái đầu; lần nữa → `None`.
  - `add_from_selection_uses_find_options`: `«Foo» foo FOO`, `find={false,false}` → thêm `foo` (vị trí 4); `find={true,false}` → `None`.
  - `add_mismatched_text_noop`: `«ab» «cd»` → `None`.
  - `add_previous`: `foo foo «foo»` backwards → thêm vị trí 4.
  - `move_replaces_newest`: `«foo» «foo» foo` (newest = thứ hai) → `«foo» foo «foo»`.
  - `select_all_keeps_primary`: `x foo *«foo» foo` → ba selection, primary vẫn ở giữa.
  - `add_next_vietnamese_whole_word`: `chào| chàobạn chào` ×3 lần gọi → selection ở `0..5` và `17..22`, lần ba `None`.
  - `limit_caps`: `select_all_matches` với `limit=2` trên 5 kết quả → 2 selection, `limited == true`.
  - `merge_flag_off_keeps_touching`: `set_merge_overlapping(false)` rồi `remap` một patch làm hai selection chồng → vẫn 2.

- [ ] **Step 2: Chạy, xác nhận hỏng** — `cargo test -p lucerna-editor multicursor:: selection::` → FAIL.

- [ ] **Step 3: Cài đặt** theo §5.1 spec. `newest()` = selection có id lớn nhất. `remap` và các chỗ gọi `from_selections(…, true)` trong `lucerna-editor` dùng `self.merge_overlapping`; `from_selections` giữ tham số và set mới thừa hưởng giá trị đó. Session mới khi bắt đầu từ chữ tự chọn: `{text, options: find}`.

- [ ] **Step 4: Chạy test** → PASS.
- [ ] **Step 5: Commit** `feat(editor): add/move/select-all find match selections`.

---

### Task 3: ⌥⌘↑/↓, ⇧⌥I, Escape, ⌥-click (`multicursor`, phần con trỏ)

**Files:** Modify `crates/lucerna-editor/src/multicursor.rs`. Test trong cùng file.

**Interfaces:**
- Consumes: `Layout` (`motion.rs`), `SelectionSet::newest`.
- Produces:
  ```rust
  pub fn insert_cursors_vertical(snap: &BufferSnapshot, set: &SelectionSet, up: bool, layout: &dyn Layout, limit: usize) -> (SelectionSet, Vec<u64>, bool /*limited*/)
  pub fn remove_ids(set: &SelectionSet, ids: &[u64]) -> SelectionSet
  pub fn cursors_at_line_ends(snap: &BufferSnapshot, set: &SelectionSet, limit: usize) -> Option<(SelectionSet, bool)>
  pub fn remove_secondary(set: &SelectionSet) -> Option<SelectionSet>
  pub fn cancel_selection(set: &SelectionSet) -> Option<SelectionSet>
  pub fn toggle_cursor_at(set: &SelectionSet, offset: usize, limit: usize) -> SelectionSet
  pub fn add_selection(set: &SelectionSet, anchor: usize, head: usize, limit: usize) -> (SelectionSet, u64)
  ```
  `Vec<u64>` là id các con trỏ vừa thêm (một lớp của stack). `add_selection` dùng cho ⌥-kéo và ⌥ double/triple-click; trả id để kéo cập nhật đúng selection đó.

- [ ] **Step 1: Test hỏng**
  - `insert_below_every_selection_keeps_goal`: `ab|cdef\nx\nabcdef\n` (2 lần dưới) → con trỏ ở (1,1) rồi (2,2) với goal giữ cột 2; mọi selection ban đầu đều được thêm (ca 2 con trỏ ban đầu ở dòng 0 và 3 → 4 con trỏ).
  - `insert_then_remove_layer`: `remove_ids` với id lớp vừa thêm trả lại set ban đầu.
  - `insert_at_edge_noop`: con trỏ ở dòng cuối, `up=false` → set không đổi, `ids` rỗng.
  - `line_ends_vscode`: `«ab\ncd\nef»gh` → con trỏ cuối `ab`, cuối `cd`, và tại cuối selection (`ef|gh`); `«ab\n»cd` (dừng ở cột 0) → chỉ một con trỏ cuối `ab`; selection rỗng bị bỏ; toàn rỗng → `None`.
  - `escape_two_steps`: `«a» «b»*` → `remove_secondary` còn `«b»`; `cancel_selection` → `b|`; một con trỏ rỗng → cả hai `None`.
  - `toggle_adds_and_removes`: thêm con trỏ mới thành primary; click lên con trỏ phụ xóa nó; click lên selection duy nhất → không đổi.

- [ ] **Step 2–4:** chạy hỏng → cài → chạy qua (`cargo test -p lucerna-editor multicursor::`).
- [ ] **Step 5: Commit** `feat(editor): vertical cursors, cursors at line ends, escape and toggle`.

---

### Task 4: Chọn cột (`multicursor::column_select`)

**Files:** Modify `crates/lucerna-editor/src/multicursor.rs`. Test cùng file.

**Interfaces:**
- Produces:
  ```rust
  #[derive(Clone, Copy, Debug, PartialEq)] pub struct ColumnPos { pub row: u32, pub col: u32 /* cột hiển thị, tab đã mở rộng */ }
  pub fn visual_column(snap: &BufferSnapshot, offset: usize, tab_size: u32) -> u32
  pub fn column_select(snap: &BufferSnapshot, from: ColumnPos, to: ColumnPos, tab_size: u32, limit: usize) -> (SelectionSet, bool)
  pub enum ColumnMove { Left, Right, Up, Down, PageUp, PageDown }
  pub fn column_move(snap: &BufferSnapshot, from: ColumnPos, to: ColumnPos, mv: ColumnMove, page_rows: u32) -> ColumnPos /* `to` mới */
  ```
- [ ] **Step 1: Test hỏng**
  - `column_select_tabs_and_short_lines`: `"\tab\nx\n    abcd"`, tab 4, từ (0, 4) tới (2, 6) → dòng 0 `«ab»`, dòng 1 con trỏ rỗng sau `x`, dòng 2 `«ab»` (cột 4..6); primary là dòng 2.
  - `column_select_reversed`: từ (2,6) tới (0,4) → mọi selection đảo chiều, primary dòng 0.
  - `column_select_unicode`: `"chào\nabcd"` cột 1..3 → `«hà»`, `«bc»`.
  - `column_move_right_up`: `column_move` Right tăng `to.col` 1; Up giảm `to.row`, chặn ở 0.
- [ ] **Step 2–4:** hỏng → cài → qua.
- [ ] **Step 5: Commit** `feat(editor): column selection`.

---

### Task 5: Clipboard theo VS Code và setting paste

**Files:** Modify `crates/lucerna-editor/src/clipboard.rs`, `crates/lucerna-editor/src/plan.rs` (`EditConfig`), `crates/lucerna-editor/src/test_util.rs` (`test_config`), `crates/lucerna-workbench/src/code_view/transact.rs:60-100` (đọc setting), `crates/lucerna-settings/assets/default-settings.json`.

**Interfaces:**
- Produces: `setting_enum!(MultiCursorPaste { Spread = "spread", Full = "full" })`; `EditConfig.multi_cursor_paste: MultiCursorPaste`. Các khóa setting mới cho cả bốn setting được thêm vào `default-settings.json` ở task này (Task 6, 7 chỉ đọc).

- [ ] **Step 1: Test hỏng**
  - `copy_mixed_empty_gives_empty_piece`: `«ab» c|d` → `pieces == ["ab", ""]`, `whole_line == false`.
  - `copy_all_empty_whole_lines` (giữ hành vi cũ) qua.
  - `cut_mixed_empty_deletes_only_ranges`: `«ab» c|d` → văn bản ` cd`.
  - `paste_external_lines_spread`: clipboard `"x\ny"` không có `pieces`, hai con trỏ → mỗi con trỏ nhận một dòng.
  - `paste_full`: `multi_cursor_paste = Full`, clipboard có `pieces` 2 mảnh, 2 con trỏ → mỗi chỗ nhận cả `text`.
  - `paste_lines_count_mismatch_full_text`: 3 dòng, 2 con trỏ → mỗi chỗ nhận cả chữ.
- [ ] **Step 2–4:** hỏng → cài (`sources`: chỉ khi mọi selection rỗng mới lấy cả dòng) → qua (`cargo test -p lucerna-editor clipboard::`, và `cargo test -p lucerna-workbench --test editing`).
- [ ] **Step 5: Commit** `fix(editor): copy mixed selections as VS Code; multiCursorPaste`.

---

### Task 6: Bỏ `block_selection`; lệnh và phím trong workbench

**Files:**
- Create: `crates/lucerna-workbench/src/code_view/multi_cursor.rs` (phương thức `CodeView` cho multi-cursor + trạng thái phiên)
- Modify: `crates/lucerna-workbench/src/code_view/mod.rs` (bỏ trường `block_selection`, `set_block_selection`, `has_block_selection`, nhánh khối trong `copy_selection`, remap ở ~652), `code_view/transact.rs:332-345`, `code_view/element.rs` (`block_selection` trong frame, `visible_selections`), `code_view/element/paint.rs:63`, `crates/lucerna-workbench/src/vim.rs` (`sync_view` ~1623, chỗ tắt Vim ~604), `crates/lucerna-workbench/src/editing.rs` (đăng ký lệnh), `crates/lucerna-settings/assets/default-keybindings.json`
- Test: `crates/lucerna-workbench/tests/multi_cursor.rs` (mới, theo helper của `tests/editing.rs`); `tests/vim.rs`, `tests/vim_editing.rs` phải qua không sửa kỳ vọng

**Interfaces:**
- Consumes: Task 2–5.
- Produces (trên `CodeView`):
  ```rust
  pub fn add_next_match(&mut self, backwards: bool, cx) ; pub fn move_next_match(&mut self, backwards: bool, cx)
  pub fn select_all_matches(&mut self, cx)          // nền khi > BACKGROUND_SEARCH_BYTES
  pub fn insert_cursor_vertical(&mut self, up: bool, cx)
  pub fn cursors_at_line_ends(&mut self, cx)
  pub fn column_select_move(&mut self, mv: ColumnMove, cx)
  pub fn remove_secondary_cursors(&mut self, cx) ; pub fn cancel_selection(&mut self, cx)
  pub fn set_column_selection(&mut self, from: ColumnPos, to: ColumnPos, cx)   // Vim <C-v> và ⇧⌥-kéo
  pub fn multi_cursor_limit(&self, cx: &App) -> usize
  ```
  Trạng thái trong `CodeView`: `match_session: Option<MatchSession>`, `cursor_layers: Vec<(bool /*up*/, Vec<u64>)>`, `column_select: Option<(ColumnPos, ColumnPos)>`. Cả ba bị xóa trong `set_selections_inner` khi selection đổi mà không do chính lệnh của chúng (cờ `multi_cursor_running` bật trong lúc lệnh chạy).
  Mỗi lệnh tạo selection: đi qua `set_selections(set, reveal_newest, cx)` (đường `Move`, nên ⌘U hoàn tác được), rồi `cx.emit(CodeViewEvent::MultiCursor)` (biến thể mới, dùng ở Task 8). Bị cắt → `set_goto_message`/status message `The number of cursors has been limited to N.`.
  `seeded_word` → nếu `find_widget()` có thì đặt chữ ô tìm.

- [ ] **Step 1: Test hỏng** (`tests/multi_cursor.rs`)
  - `cmd_d_three_times_then_type`: `foo bar foo baz foo`, con trỏ trong `foo` đầu, `keys("cmd-d cmd-d cmd-d")`, gõ `x` → `x bar x baz x`; ⌘Z một lần → văn bản cũ.
  - `cmd_k_cmd_d_skips`: ⌘D ⌘D ⌘K ⌘D gõ `x` → `x bar foo baz x`.
  - `cmd_u_undoes_cmd_d`: ⌘D ⌘D ⌘U → còn một selection.
  - `shift_cmd_l_and_cmd_f2`: ⇧⌘L chọn mọi `foo`; ⌘F2 tương tự.
  - `alt_cmd_down_then_up_removes`: ⌥⌘↓ ⌥⌘↓ ⌥⌘↑ → 2 con trỏ.
  - `escape_two_steps`: ⌘D ⌘D, Escape → 1 selection không rỗng; Escape → con trỏ; context `editorHasMultipleSelections` đúng ở từng bước.
  - `escape_closes_find_first`: thanh Find mở + 2 selection, Escape đóng Find, vẫn 2 selection.
  - `cmd_d_explorer_still_duplicates`: focus Explorer, ⌘D chạy `filesExplorer.duplicate` (kiểm lệnh được chọn bằng `keymap` + context, như test explorer hiện có).
  - `cmd_d_seeds_open_find`: mở ⌘F, ⌘D trên `foo` → ô tìm là `foo`.
  - `cmd_d_after_external_reload`: ⌘D, ghi đè file trên đĩa (chèn dòng đầu), chờ reload, ⌘D → selection mới đúng chữ `foo` trên snapshot mới.
  - `select_highlights_limit_large_file`: setting `editor.multiCursorLimit: 100`, file 2 MiB toàn `foo `, ⇧⌘L → `run_until_parked` → đúng 100 selection, status message có `limited to 100`.
  - `alt_shift_cmd_arrows_column`: ⇧⌥⌘→ ×2, ⇧⌥⌘↓ → 2 selection 2 ký tự.
  - `vim_block_yank_unchanged`: test `<C-v>` + `y` hiện có trong `tests/vim.rs` qua nguyên vẹn.
- [ ] **Step 2: Chạy, xác nhận hỏng** — `cargo test -p lucerna-workbench --test multi_cursor` → FAIL.
- [ ] **Step 3: Cài đặt**: lệnh trong `editing.rs` với id VS Code (§5.2 spec; `addSelectionToPreviousFindMatch`/`moveSelectionToPreviousFindMatch` có tiêu đề palette `Add Selection to Previous Find Match` / `Move Last Selection to Previous Find Match`; các lệnh ⌘D, ⇧⌘L, ⌥⌘↑/↓, ⇧⌥I cũng có tiêu đề palette như VS Code: `Add Selection to Next Find Match`, `Move Last Selection to Next Find Match`, `Select All Occurrences of Find Match`, `Change All Occurrences`, `Add Cursor Above`, `Add Cursor Below`, `Add Cursors to Line Ends`). Phím theo bảng spec, `when` của ⌘D là `editorFocus`. Vim `<C-v>` dùng `set_column_selection`; chỗ tắt Vim ~604 gộp chỉ khi Vim đang ở `Mode::Visual`, `Mode::VisualLine` hoặc `Mode::VisualBlock`.
- [ ] **Step 4: Chạy** `--test multi_cursor --test editing --test vim --test vim_editing --test find_widget` → PASS.
- [ ] **Step 5: Commit** `feat(workbench): multi-cursor commands and keys; selections replace block_selection`.

---

### Task 7: Chuột

**Files:** Modify `crates/lucerna-workbench/src/code_view/element.rs:1169-1235` (`mouse_down`, `mouse_drag`, `Drag`), `code_view/mod.rs` (`Drag` thêm biến thể). Test: `tests/multi_cursor.rs`.

**Interfaces:**
- Consumes: `toggle_cursor_at`, `add_selection`, `set_column_selection`, setting `editor.multiCursorModifier`.
- Produces: `enum Drag { Text, AddSelection { id: u64, anchor: usize }, Column { from: ColumnPos } }` (giữ các biến thể cũ khác).

- [ ] **Step 1: Test hỏng** (dùng `cx.simulate_mouse_down/up/move` với `Modifiers`, như `tests/explorer_git.rs`)
  - `alt_click_adds_then_removes`: ⌥-click hai chỗ → 3 con trỏ; ⌥-click lại chỗ thứ hai → 2.
  - `alt_drag_adds_selection`: selection có sẵn giữ nguyên, thêm một vùng theo chuột.
  - `shift_alt_drag_then_type_and_undo`: ⇧⌥-kéo qua 3 dòng (một dòng ngắn, có tab) rồi gõ `z` → mỗi dòng có `z`; ⌘Z một lần trả nguyên văn.
  - `ctrl_cmd_modifier_swaps`: setting `"ctrlCmd"`: ⌘-click thêm con trỏ; ⌥-click (không ⌘) chạy go to definition (kiểm như test link hiện có: request định nghĩa được gửi / `link_mouse_down` nhận điểm).
  - `cmd_alt_click_definition_to_side_both_modes`: ⌘⌥-click không thêm con trỏ ở cả hai chế độ.
  - `alt_double_click_adds_word`: 1 con trỏ, ⌥ double-click lên `bar` → thêm selection `«bar»`, con trỏ cũ giữ.
  - `plain_click_collapses`: 3 con trỏ, click thường → 1.
- [ ] **Step 2–4:** hỏng → cài → qua.
- [ ] **Step 5: Commit** `feat(workbench): alt-click, alt-drag and shift-alt-drag cursors`.

---

### Task 8: Vim vào Insert sau lệnh multi-cursor

**Files:** Modify `crates/lucerna-vim/src/state.rs` (thêm hàm), `crates/lucerna-workbench/src/vim.rs` (nghe `CodeViewEvent::MultiCursor` ở ~668; nhánh `selection_generation != generation` ở ~822 không reset khi đã vào Insert). Test: `crates/lucerna-workbench/tests/vim_editing.rs`.

**Interfaces:**
- Consumes: `CodeViewEvent::MultiCursor` (Task 6, 7 phát).
- Produces: `impl Vim { pub fn enter_insert_here(&mut self, cursor: Point) }` (mode Insert, không count, không `insert_lines`, anchor = cursor); `VimLayer::start_multi_cursor_insert(&mut self, view: &Entity<CodeView>, cx)` — giống `start_insert` nhưng **không** đặt lại con trỏ (giữ `SelectionSet`), mở bước undo như phiên Insert thường, overlay thanh ở head chính, `set_typed_events(true)`.

- [ ] **Step 1: Test hỏng**
  - `vim_cmd_d_insert_escape_undo`: Vim bật, `foo bar foo`, Normal ở `f`; ⌘D ⌘D → mode `Insert`, status `-- INSERT --`; gõ `x` → `x bar x`; Escape → Normal, một selection; `u` → `foo bar foo`.
  - `vim_alt_click_enters_insert`: ⌥-click khi Normal → Insert, 2 con trỏ, gõ ở cả hai.
  - `vim_visual_then_cmd_d`: `v` chọn `foo` rồi ⌘D → Insert, 2 selection (không bị Vim gộp).
  - `vim_ctrl_v_stays_visual_block`: `<C-v>` `j` → mode Visual Block, không Insert.
  - `vim_off_keeps_user_cursors`: Vim tắt khi đang có 3 con trỏ do ⌥-click lúc Vim tắt → bật rồi tắt Vim, vẫn 3 (chỉ Visual mới gộp).
- [ ] **Step 2–4:** hỏng → cài → qua (`--test vim_editing --test vim`).
- [ ] **Step 5: Commit** `feat(vim): multi-cursor commands enter Insert mode`.

---

### Task 9: Status bar

**Files:** Modify `crates/lucerna-workbench/src/documents.rs:2031-2045`, `code_view/mod.rs` (cache). Test: `crates/lucerna-workbench/tests/documents.rs`.

**Interfaces:** Produces `CodeView::selection_summary(&mut self) -> (usize /*selections*/, usize /*chars*/)` có cache theo `selection_generation`; `status()` dùng nó (đổi `&CodeView` thành cách đọc phù hợp nếu cần `&mut`, hoặc cache bằng `Cell`).

- [ ] **Step 1: Test hỏng** `status_counts_selections`:
  - 1 con trỏ: `Ln 1, Col 1`.
  - 1 selection `chào`: `Ln 1, Col 5 (4 selected)` (đếm char).
  - 2 con trỏ rỗng: `2 selections`.
  - 2 selection `ab`, `cde`: `2 selections (5 characters selected)`.
- [ ] **Step 2–4:** hỏng → cài → qua.
- [ ] **Step 5: Commit** `feat(status): selection counts as VS Code`.

---

### Task 10: Đo hiệu năng và tài liệu

**Files:** Modify `crates/lucerna-editor/benches/typing.rs`, `docs/perf-checklist.md`, `docs/superpowers/specs/2026-11-08-lucerna-phase1-design.md` (§1.3: 4.3 xong), `docs/superpowers/specs/2026-11-11-lucerna-phase4-editing-design.md` (dòng trạng thái; §8 bỏ "Aho-Corasick", bỏ `selectedTextMatchMode` ở §8 và §14.2), `docs/research/2026-11-11-editing.md` §3 (ghi chú `selectedTextMatchMode` không xác nhận), `README.md` (nếu có danh sách tính năng), spec 4.3 (trạng thái: đã triển khai).

- [ ] **Step 1:** Bench `insert_char_1000_cursors` đã có. Thêm `select_all_matches_10k` (file 5 MiB, 10 000 kết quả, `limit` 10 000) và `add_match_far` (`find_next` khi kết quả duy nhất ở cuối file 50 MiB).
- [ ] **Step 2:** Run `cargo bench -p lucerna-editor --bench typing -- 1000_cursors select_all add_match_far` (bench build ở release: xóa `target/debug` trước theo luật một profile, xóa lại `target/release` sau). Expected: `insert_char_1000_cursors` không chậm hơn số cũ trong `docs/perf-checklist.md` quá 10 %; `add_match_far` < 16 ms (nếu không: ⌘D chạy nền khi file > `BACKGROUND_SEARCH_BYTES`, như spec cho phép). Ghi số vào `docs/perf-checklist.md`.
- [ ] **Step 3:** Cập nhật tài liệu như danh sách Files.
- [ ] **Step 4:** Chạy toàn bộ: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo deny check` → PASS.
- [ ] **Step 5: Commit** `docs: phase 4.3 multi-cursor done; perf numbers`.
