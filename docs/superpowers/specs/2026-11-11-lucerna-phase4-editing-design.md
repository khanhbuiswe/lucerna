# Phase 4: sửa code đầy đủ

Ngày 2026-11-11. Trạng thái: 4.1 và 4.2 đã triển khai; 4.3–4.6 chưa làm.

## 1. Mục tiêu

Lucerna chuyển từ trình xem sang editor. Người dùng sửa được file mà agent (Claude Code, Codex) đang làm việc trên đó mà không phải mở VS Code: gõ chữ (có IME tiếng Việt), undo/redo, multi-cursor, completion, rename, format. Khi agent ghi file trong lúc người dùng đang sửa, Lucerna không bao giờ âm thầm làm mất thay đổi của bên nào.

**Hiện trạng (đã kiểm tra trong code, rev `7b979ef`):**
- `lucerna-text::Buffer` chỉ có `load`, `from_bytes`, `reload`, `snapshot`; **không có API sửa**. Rope giữ nguyên `\r\n` của file.
- `CodeView` có một selection (`Option<Selection>`, tính bằng `Point`) và selection khối của Vim; không có con trỏ nhấp nháy, không có lệnh di chuyển con trỏ ngoài Vim, không nhận chữ gõ. `TextInput` (ô nhập một dòng) đã có `EntityInputHandler` với IME.
- `Documents` reload khi file đổi trên đĩa, giữ vị trí cuộn bằng `Reload` (diff theo dòng). `SyntaxState::edited` dịch cây tree-sitter cũ theo `Reload` rồi parse lại ở nền.
- `lucerna-lsp::DocumentSync::change` luôn gửi **toàn bộ** nội dung.
- `lucerna-extension` đọc `line_comments` của `config.toml`, chép query `indents.scm`, và chỉ **kiểm tra cú pháp** snippet (chưa có runtime; trang Extensions ghi "(Phase 4)").

**Đã chốt (đề xuất, chờ duyệt — xem §19):**

| # | Quyết định | Lý do |
|---|---|---|
| D1 | **Không CRDT.** Giữ `ropey`, thêm edit, `Patch`, `History` (diff ngược + selection trước/sau) vào `lucerna-text`. | Lucerna một người dùng. Zed cần CRDT cho cộng tác; VS Code dùng diff ngược. Clone `ropey` là O(1) nên mô hình snapshot của Zed vẫn dùng được. |
| D2 | **Crate mới `lucerna-editor` (không gpui)**: selection, di chuyển, và mọi lệnh sửa dưới dạng hàm thuần `(snapshot, selections, cấu hình) → EditPlan`. | Test được không cần GPUI; Vim V2 gọi cùng các hàm này (§13). |
| D3 | **Nhóm undo theo quy tắc của VS Code** (`EditOperationType`), không nhóm theo thời gian như Zed. IME theo Zed: cả lần soạn là một bước. | Hành vi theo VS Code; dự đoán được. |
| D4 | **Rope lưu `\n`**, nhớ kiểu xuống dòng của file và trả lại khi lưu. File có lẫn `\r\n` và `\n` sẽ được chuẩn hóa về kiểu chiếm đa số khi lưu. | Giống cả VS Code và Zed; cột và vị trí LSP đơn giản hơn. |
| D5 | **Xung đột với agent:** buffer sạch thì reload (undo được); buffer bẩn mà thay đổi trên đĩa không chồng lấn thì **tự gộp** (undo được); chồng lấn thì chấm cảnh báo trên tab và **thanh báo ở đầu editor** với Reload / Keep Mine. | Agent ghi file khi người dùng đang sửa là tình huống chính. VS Code im lặng tới lúc lưu, Zed chỉ có chấm trên tab: cả hai dễ để người dùng sửa tiếp trên bản cũ. Tự gộp lấy theo `reconcileExternalChanges` mới của VS Code. |
| D6 | **Không có Compare** trong Phase 4. "Reload" là bước undo được nên không phá hủy: ⌘Z lấy lại bản của mình. | Compare cần giao diện diff, thuộc Phase 5. |
| D7 | **Ghi file atomic:** file tạm cùng thư mục + fsync + rename; ghi tại chỗ khi đích là symlink hoặc có nhiều hardlink. | Zed ghi đè tại chỗ, có thể để lại file dở khi crash; agent đọc file liên tục nên file dở là nguy hiểm. |
| D8 | **Giao diện theo Zed:** chấm bẩn/cảnh báo trên tab, menu completion dạng popover 280–540 px với tài liệu bên phải, ô rename một dòng dưới symbol. Lệnh, phím, settings theo VS Code. | `design-principles.md` §1.1. |
| D9 | **Format có timeout cứng 5 s** (Zed), không có thông báo "Saving…" + Skip của VS Code. | Không cần UI tiến trình; agent-first nên format thường do agent làm. |
| D10 | **Vim:** Phase 4 thêm Insert mode tối thiểu (`i a I A o O`, Escape) để người bật Vim gõ được; toán tử, `.`, register, `u` vẫn thuộc V2. | Không có nó, người dùng Vim không sửa được gì trong Phase 4. |
| D11 | **Chia thành sáu bước giao riêng** (§17). Bước 4.1 gồm cả phần chống mất dữ liệu tối thiểu. | Phase quá lớn để làm một lần. |

**Thành công khi:**
- Mở một file Rust trong project đã tin cậy: gõ chữ (cả Telex), ⌘Z / ⇧⌘Z đúng nhóm như VS Code, ⌘S lưu giữ nguyên encoding, BOM và kiểu xuống dòng; tab hiện chấm khi bẩn và hết chấm khi undo về bản đã lưu.
- Trong lúc người dùng có thay đổi chưa lưu ở đầu file, agent sửa cuối file: Lucerna gộp thay đổi của agent vào buffer, báo trên status bar, ⌘Z gỡ được phần gộp. Agent sửa đúng dòng người dùng đang sửa: tab có chấm cảnh báo, thanh báo hiện ra; ⌘S hỏi trước khi ghi đè.
- ⌘D, ⌥⌘↓, ⇧⌥I, ⌥-click tạo nhiều con trỏ; gõ, xóa, paste, IME chạy trên mọi con trỏ.
- Gõ `pri` trong file Rust có `rust-analyzer`: menu completion hiện sau ≤ 10 ms kể từ khi server trả lời, Enter chèn `println!` dạng snippet, Tab đi qua các placeholder.
- F2 đổi tên một hàm dùng ở ba file; ⇧⌥F và format-on-save gọi formatter của server.
- Gõ trong file 10.000 dòng có tô màu: mỗi phím hiện lên trong khung hình kế tiếp (§15).

**Ngoài phạm vi:** file chưa có tên (⌘N), Save As; hot exit (`files.hotExit`) và giữ undo sau khi đóng file (`files.restoreUndoStack`); undo một bước xuyên nhiều file; Compare / diff (Phase 5); `formatOnSaveMode: modifications` (cần Git, Phase 5); format on type / on paste; code action, quick fix, signature help, inlay hint, linked editing; xem trước rename (⌘Enter); snippet do người dùng tự viết; chế độ chỉ đọc theo glob (`files.readonlyInclude`); `editor.columnSelection` (chế độ chọn cột bật/tắt); toán tử Vim (V2).

## 2. Tham khảo

Chi tiết ở `docs/research/2026-11-11-editing.md`. Tóm tắt những điểm quyết định thiết kế:
- **VS Code:** undo là danh sách phần tử, phần tử cuối "mở" tới khi có undo stop; stop khi đổi loại thao tác, khi di chuyển con trỏ, trước Enter / paste / cut, khi lưu. Undo khôi phục selection trước nhóm. Reload và Revert là edit diff, undo được. Buffer bẩn không bao giờ bị reload; lưu khi file trên đĩa mới hơn thì báo lỗi với Compare / Overwrite. `reconcileExternalChanges` (mới) gộp ba chiều thay đổi bên ngoài vào buffer bẩn. Lệnh và phím multi-cursor, di chuyển, dòng, comment, completion, rename, format như liệt kê ở §14.
- **Zed:** transaction có độ sâu, "finalize" chặn gộp, IME gộp một bước, format là một bước và hủy nếu người dùng sửa trong lúc chờ (timeout 5 s). Chấm Warning trên tab khi xung đột, prompt Overwrite / Discard Edits / Cancel khi lưu. Menu completion 280–540 px, 3–12 dòng, tài liệu bên phải 260–500 px. Rename là editor một dòng dưới symbol, chữ đậm. Sau edit: `tree.edit` ngay, parse đồng bộ 1 ms, quá thì parse ở nền.
- **Lucerna lệch khỏi cả hai** ở: tự gộp thay đổi không chồng lấn và thanh báo xung đột (D5); không có Compare (D6); ghi atomic (D7); timeout format cứng thay cho Skip (D9); nhãn completion tô theo loại thay vì icon (§10.3).

Giấy phép: `text`, `editor`, `language`, `workspace`, `project` của Zed là GPL-3, chỉ học thiết kế. Chuyển thể từ VS Code (MIT) gồm quy tắc `shouldPushStackElementBetween`, `fuzzyScore`, danh sách ký tự phân cách từ và parser snippet; mỗi chỗ ghi nguồn trong comment và `THIRD_PARTY_NOTICES.md`. Dependency mới: `similar` (Apache-2.0, đã có trong `Cargo.lock`), `tempfile` (MIT/Apache, đã có), `aho-corasick` (MIT/Unlicense, đã có).

## 3. Kiến trúc

```
lucerna-text      Buffer (edit, transaction, undo), Patch, History, ghi file, chuẩn hóa EOL
lucerna-editor    (mới, không gpui) Selection, SelectionSet, di chuyển, lệnh sửa → EditPlan,
                  auto-close, thụt lề, comment, multi-cursor, snippet, chấm điểm completion, gộp ba chiều
lucerna-syntax    SyntaxState::edited(Patch), indents.scm, node cho smart select
lucerna-extension đọc brackets / autoclose_before / block_comment; nạp snippet
lucerna-lsp       didChange tăng dần, didSave, completion, rename, formatting, WorkspaceEdit
lucerna-settings  settings và phím mới
lucerna-vim       Insert mode tối thiểu
lucerna-workbench Document giữ Buffer; CodeView nhận chữ (IME), vẽ con trỏ; lưu, prompt, autosave;
                  thanh xung đột; menu completion; ô rename
```

### 3.1 `lucerna-text`

- **Chuẩn hóa xuống dòng:** `from_bytes` bỏ `\r` trước `\n` khi nạp vào rope và ghi nhận `LineEnding` chiếm đa số (đã có `dominant_line_ending`). `\r` đứng một mình giữ nguyên. Ghi file thì đổi `\n` thành kiểu đã nhớ.
- **Edit:** `Buffer::edit(edits: impl IntoIterator<Item = (Range<usize>, Arc<str>)>) -> Patch`. Các khoảng tính bằng byte trên snapshot *trước* edit, không chồng nhau; áp từ cuối lên đầu. Văn bản chèn vào được chuẩn hóa `\r\n` → `\n`.
- **`Patch`:** danh sách `PatchEdit { old: Range<usize>, new: Range<usize> }` đã sắp, cùng hàm:
  - `map_offset(offset, Bias) -> usize` để dịch selection, mark, highlight, diagnostic;
  - `map_row(row) -> u32` thay `Reload::adjust_row` (folds, cuộn);
  - `compose(&Patch) -> Patch` để gộp nhiều edit trong một transaction;
  - `input_edits(old, new)` cho tree-sitter, `lsp_changes(old_snapshot)` cho LSP.
  `Reload` hiện có trở thành một `Patch` tính bằng diff theo dòng.
- **`Transaction`:** `{ id: TransactionId(u64), edits: Vec<InverseEdit>, kind: EditKind, selections_before: Option<SelectionsToken>, selections_after: Option<SelectionsToken>, open: bool }`. `InverseEdit` lưu khoảng mới và văn bản cũ (đủ để undo), khoảng cũ và văn bản mới (đủ để redo). `SelectionsToken` là dữ liệu mờ do `lucerna-editor` gắn vào (crate text không biết selection là gì).
- **`History`:**
  - `start_transaction(kind)` / `end_transaction()` có đếm độ sâu; lồng nhau thì gộp vào transaction ngoài; transaction rỗng bị bỏ.
  - `push_stop()` đóng transaction đang mở (undo stop của VS Code). `group_until(id)` gộp mọi transaction sau `id` vào `id` (IME, Vim insert).
  - `undo()` / `redo()` trả về `(Patch, Option<SelectionsToken>)`; edit mới xóa stack redo.
  - Không giới hạn số bước trong Phase 4; history mất khi document đóng.
- **Bẩn:** `saved: Option<TransactionId>` ghi id đỉnh stack undo lúc lưu hoặc reload. `is_dirty()` khi id đỉnh hiện tại khác `saved` (undo về đúng chỗ đã lưu thì sạch, giống "alternative version id" của VS Code). Redo qua chỗ đã lưu cũng sạch lại.
- **`version: u64`** tăng sau mỗi edit, undo, redo; kết quả nền (parse, format, completion) mang version và bị bỏ nếu cũ.
- **Ghi file:** `Buffer::write(path, &WriteOptions) -> Result<Stamp>`:
  1. Mã hóa snapshot theo `encoding` (thêm BOM nếu file gốc có BOM), đổi `\n` theo `LineEnding`.
  2. Nếu `path` là symlink hoặc `nlink > 1`: ghi tại chỗ (truncate + write + fsync).
  3. Còn lại: file tạm `.<tên>.luc~<ngẫu nhiên>` trong cùng thư mục, chép quyền của file cũ, ghi, fsync, `rename` đè lên; lỗi ở bất kỳ bước nào thì xóa file tạm và trả lỗi, file cũ còn nguyên.
  4. Trả `Stamp` (mtime, độ dài) và hash nội dung đã ghi.
  Encoding không biểu diễn được ký tự (ví dụ Latin-1 gặp chữ Việt) thì trả `WriteError::Unencodable { first: Point }`, không ghi gì.

### 3.2 `lucerna-editor` (crate mới, không gpui)

Phụ thuộc `lucerna-text`, `lucerna-syntax` (node cho smart select và thụt lề), `aho-corasick`, `similar`, `unicode-segmentation`.

- **`Selection { id, anchor: usize, head: usize, goal: Goal }`**: offset byte; `Goal` giữ cột hiển thị khi đi lên/xuống (`None | Column(f32) | Block { start, end }`). `SelectionSet { selections: Vec<Selection> (đã sắp, không chồng), primary: SelectionId }`.
  - `normalize(merge_overlapping)`: theo `editor.multiCursorMergeOverlapping`; gộp khi chồng lấn, cùng điểm đầu, hoặc con trỏ rỗng chạm biên selection khác; hai selection không rỗng chỉ chạm nhau thì giữ riêng (theo Zed).
  - `remap(&Patch)`: dịch qua edit của view khác hoặc của agent.
  - Giới hạn `editor.multiCursorLimit` (10000).
- **Di chuyển:** hàm thuần `move_selections(snapshot, set, Motion, extend: bool, &Layout) -> SelectionSet`. `Layout` là trait do workbench cài (đổi offset ↔ vị trí hiển thị khi word wrap, folding, tab), để `lucerna-editor` không cần biết cách vẽ. `Motion` gồm Left, Right, Up, Down, WordStartLeft, WordEndRight, Home (thông minh: lần đầu tới ký tự khác trắng đầu dòng, lần hai tới cột 0), End, LineStart, LineEnd, Top, Bottom, PageUp, PageDown. Từ theo `editor.wordSeparators` với ba loại ký tự như VS Code.
- **Lệnh sửa:** mỗi lệnh là `fn(&Ctx, &SelectionSet) -> EditPlan`, trong đó `Ctx` có snapshot, cấu hình ngôn ngữ (cặp ngoặc, comment), settings sửa (tab size, insert spaces, auto-close…), và syntax nếu có. `EditPlan { edits: Vec<(Range<usize>, Arc<str>)>, selections_after: SelectionSet, kind: EditKind, autoclose: Vec<AutocloseRegion> }`. `EditKind` = `TypingOther | TypingFirstSpace | TypingConsecutiveSpace | DeletingLeft | DeletingRight | Other`, dùng cho quy tắc undo stop (§4.2).
- **`UndoStops`:** hàm `should_stop_between(prev: EditKind, cur: EditKind) -> bool` chuyển thể từ VS Code; workbench gọi trước mỗi edit.
- **Auto-close, thụt lề, comment, dòng, multi-cursor, snippet, completion, gộp ba chiều:** các module riêng trong crate, mô tả ở §5, §7–§10.
- **Ký hiệu test:** `marked("fn |a«bc»|")` dựng snapshot và selection từ chuỗi có đánh dấu (`|` con trỏ, `«»` selection), viết mới cho Lucerna.

### 3.3 `lucerna-syntax`

- `SyntaxState::edited(old, new, &Patch)` thay bản nhận `Reload`; reload và edit đi cùng một đường.
- Sau edit, workbench thử parse đồng bộ trong `SYNC_PARSE_BUDGET` hiện có (giữ nguyên giá trị, đo lại ở §15), quá thì parse tăng dần ở nền; kết quả mang `version`.
- `indent_suggestion(snapshot, row) -> Option<IndentDelta>` từ `indents.scm` (capture `@indent`, `@end`, `@outdent` như Zed dùng).
- `enclosing_nodes(offset_range) -> Vec<Range<usize>>` cho smart select.

### 3.4 `lucerna-extension`

`LanguageConfig` đọc thêm từ `config.toml` của extension ngôn ngữ Zed: `brackets` (`start`, `end`, `close`, `newline`, `not_in`), `autoclose_before`, `block_comment` (cả dạng mảng `["/* ", " */"]` lẫn dạng bảng mới `{ start, end, prefix, tab_size }`), `word_characters`. Thiếu khóa thì dùng mặc định: cặp `()` `[]` `{}` tự đóng, không có dấu nháy. Nạp snippet hợp lệ (định dạng JSON của VS Code: `prefix`, `body`, `description`) thành `Vec<Snippet>` theo ngôn ngữ.

### 3.5 `lucerna-lsp`

- **`DocumentSync::change(path, old_snapshot, new_snapshot, &Patch)`**: server `INCREMENTAL` nhận các thay đổi theo khoảng (tính từ `Patch` theo position encoding đã thương lượng); server `FULL` vẫn nhận toàn bộ. Gửi ngay khi transaction kết thúc (không gộp theo frame), nên mọi request sau đó thấy văn bản mới nhất.
- `did_save(path, text: Option)` theo capability `save` / `includeText`.
- Request mới: `completion`, `completionItem/resolve`, `prepareRename`, `rename`, `documentHighlight` (để làm mờ khi rename), `formatting`, `rangeFormatting`. Khai báo capability tương ứng trong `initialize` (`snippetSupport: true`, `resolveSupport` cho `documentation`, `detail`, `additionalTextEdits`; `prepareSupport` cho rename).
- `WorkspaceEdit` → `Vec<(PathBuf, Option<i32> version, Vec<TextEdit>)>`. Thao tác tạo / đổi tên / xóa file trong `documentChanges` không được hỗ trợ: cả edit bị từ chối kèm thông báo.

### 3.6 `lucerna-settings`

Thêm settings và phím mặc định ở §14 vào `default-settings.json` và `default-keybindings.json`.

### 3.7 `lucerna-workbench`

- **`Documents`:** `Document` giữ `Buffer` (đã có), thêm `base: BufferSnapshot` (nội dung trên đĩa lúc nạp / reload / lưu gần nhất, dùng cho gộp ba chiều), `base_hash`, `conflict: Option<Conflict>`. Sự kiện mới `DocumentEvent::Edited { path, patch, snapshot, version }` (LSP, diagnostics, view khác), `DirtyChanged`, `Saved`, `ConflictChanged`.
  - Reload hiện mang `Buffer` sang luồng khác. Phase 4 đổi thành: luồng nền đọc và giải mã file, tính diff giữa snapshot hiện tại và nội dung mới; luồng chính áp diff như một edit nếu `version` chưa đổi (giống Zed), ngược lại tính lại.
- **`CodeView`:** thay `selection` và `block_selection` bằng `SelectionSet`; cài `EntityInputHandler` (marked text như `TextInput`, mỗi con trỏ một vùng soạn); vẽ con trỏ dạng thanh 2 px nhấp nháy (`editor.cursorBlinking`, `editor.cursorStyle`) cho mọi selection; ngừng nhấp nháy khi gõ. Mọi lệnh sửa đi qua `CodeView::transact(kind, |plan| …)`: xin undo stop theo `UndoStops`, gọi `Documents::apply(path, plan)`, gắn `SelectionsToken`, cuộn tới con trỏ chính.
- **Tab:** chấm thay nút × khi bẩn (hover hiện ×), màu `text.accent` của theme Zed; chấm màu `warning` khi xung đột; tên gạch ngang khi file đã bị xóa. Context key mới (§14).
- **Prompt:** dùng `window.prompt` của GPUI (hộp thoại hệ thống trên macOS, có bản vẽ sẵn của GPUI trên nền tảng khác).
- **Module mới:** `editing.rs` (đăng ký lệnh sửa), `save.rs` (luồng lưu, autosave, prompt đóng), `conflict_bar.rs`, `completion_menu.rs`, `snippet_session.rs`, `rename.rs`, `format.rs`.

## 4. Mô hình edit và undo

### 4.1 Đường đi của một phím

1. GPUI gọi `replace_text_in_range` (chữ gõ, IME) hoặc keymap chạy một lệnh (`deleteLeft`, `tab`…).
2. `CodeView` dựng `Ctx`, gọi hàm của `lucerna-editor`, nhận `EditPlan`.
3. `UndoStops`: nếu `should_stop_between(prev_kind, plan.kind)` thì `history.push_stop()`.
4. `Documents::apply` → `Buffer::edit` → `Patch`; `version` tăng.
5. Transaction đang mở nhận `InverseEdit` và `selections_after`; transaction mới thì nhận cả `selections_before`.
6. Phát `Edited`: view khác `remap` selection, folds, highlight tìm kiếm, mark Vim; diagnostics dịch theo `Patch` cho tới khi server gửi bản mới; `SyntaxState::edited` rồi parse; LSP `didChange`.
7. View vẽ lại trong khung hình kế tiếp với cây cú pháp đã dịch (màu gần đúng) nếu parse chưa xong.

### 4.2 Undo stop (theo VS Code)

| Sự kiện | Stop |
|---|---|
| Gõ ký tự, loại khác loại trước theo `should_stop_between` | trước |
| Enter | trước |
| Paste, cut, surround, tab / outdent, lệnh dòng, comment | trước và sau |
| ⌫ / ⌦ khi loại trước khác, hoặc xóa qua nhiều dòng | trước |
| Mọi di chuyển con trỏ (phím, click, ⌘D…) | trước |
| Lưu file | trước (để ⌘Z quay về đúng bản đã lưu) |
| Reload, gộp từ đĩa, áp completion, rename, format | trước và sau (bước riêng, không gộp) |
| Một lần soạn IME | cả lần soạn là một bước (`group_until`) |
| Vim Insert mode | cả lần insert là một bước |

Undo / redo khôi phục selection trước / sau của transaction vào view đang thực hiện undo. `cursorUndo` (⌘U) giữ 50 trạng thái selection gần nhất của view, xóa khi nội dung đổi, như VS Code.

### 4.3 Nhiều view cùng một file

Mỗi tab có `CodeView` và `SelectionSet` riêng; `Buffer` và `History` thuộc `Document` (undo theo file, như cả VS Code và Zed). Edit ở view này dịch selection của view kia qua `Patch`.

## 5. Gõ chữ và lệnh sửa cơ bản

- **Gõ:** chèn tại mọi selection, thay phần đang chọn. Không phải ASCII (chữ Việt, emoji) đi qua cùng đường.
- **IME (theo Zed):** chữ đang soạn được chèn thật vào buffer và tô gạch chân; `marked_text_range` trả vùng của con trỏ chính; các con trỏ phụ nhận cùng chữ. `unmark_text` khi Escape, click, hoặc undo. `bounds_for_range` trả vị trí con trỏ chính để đặt cửa sổ gợi ý.
- **Xóa:** `deleteLeft`, `deleteRight`, `deleteWordLeft`, `deleteWordRight`, `deleteAllLeft`, `deleteAllRight`. ⌫ trong vùng thụt lề bằng dấu cách lùi về tab stop trước (Zed và VS Code cùng làm khi `insertSpaces`).
- **Enter:** giữ thụt lề dòng hiện tại (`editor.autoIndent` ≥ `keep`); giữa cặp ngoặc có `newline = true` thì chèn hai dòng và đặt con trỏ ở dòng giữa đã thụt thêm (≥ `brackets`); `full` thêm gợi ý từ `indents.scm` (gõ `}` thì lùi). `editor.trimAutoWhitespace`: rời dòng chỉ có thụt lề tự chèn thì xóa thụt lề đó.
- **Tab / ⇧Tab:** không chọn gì thì chèn tới tab stop kế (dấu cách hoặc `\t` theo `editor.insertSpaces`); có chọn nhiều dòng thì thụt / lùi các dòng.
- **Auto-close (bước 4.2):** theo `editor.autoClosingBrackets` / `autoClosingQuotes` và dữ liệu `brackets` của ngôn ngữ: chỉ đóng khi ký tự kế tiếp thuộc `autoclose_before` hoặc là cuối dòng / khoảng trắng; dấu nháy không đóng ngay sau chữ; `not_in` (ví dụ trong string) chặn. `autoClosingOvertype: auto` chỉ gõ đè ký tự do editor tự chèn (ghi trong `AutocloseRegion`, mất khi con trỏ rời vùng). `autoClosingDelete: auto`. `autoSurround`.
- **Thụt lề khi mở file (bước 4.2):** `editor.detectIndentation` đoán tab / dấu cách và độ rộng từ nội dung (thuật toán như VS Code: đếm hiệu thụt lề giữa các dòng liền nhau). Status bar thêm mục "Spaces: 4" / "Tab Size: 4" (chỉ hiện, chưa bấm được).
- **Comment (bước 4.2):** `commentLine` dùng `line_comments[0]`; nếu mọi dòng có nội dung đã có comment thì bỏ, ngược lại thêm ở cột thụt lề nhỏ nhất. `blockComment` dùng `block_comment`. Ngôn ngữ không có dữ liệu thì lệnh không làm gì.
- **Dòng (bước 4.2):** move / copy lines up/down, deleteLines, insertLineAfter / Before, indentLines / outdentLines, joinLines, trimTrailingWhitespace.
- **Smart select (bước 4.2):** expand / shrink theo node tree-sitter (`enclosing_nodes`), có stack để shrink quay lại; không có cây thì theo từ → dòng → cả file.
- **Clipboard:** ⌘X / ⌘C / ⌘V. Không chọn gì thì copy / cut cả dòng; dán đoạn đó với con trỏ rỗng thì chèn lên trên dòng hiện tại. Copy từ nhiều selection ghi metadata số đoạn vào `ClipboardItem` để paste `spread`.

## 6. Lưu file

### 6.1 Luồng lưu (`save.rs`)

1. Nếu document đang có `conflict` chưa được "Keep Mine", hỏi (§7.3). Hủy thì dừng.
2. `history.push_stop()`.
3. Các bước trước khi lưu, gom thành **một** transaction (một bước undo): `files.trimTrailingWhitespace` → format (nếu `editor.formatOnSave` và đây không phải lần tự lưu, §12) → `files.insertFinalNewline` → `files.trimFinalNewlines`.
4. Chụp snapshot và `version`; ghi ở luồng nền (`Buffer::write`).
5. Thành công: `saved = id đỉnh stack tại version đã chụp` (sửa trong lúc ghi vẫn bẩn, như Zed), `base = snapshot đã ghi`, cập nhật `stamp` và `base_hash`, `did_save` cho LSP, phát `Saved`.
6. Lỗi: thông báo trong status bar kèm lý do (không có quyền, đĩa đầy, ký tự không mã hóa được ở `Ln x, Col y`); buffer vẫn bẩn.

Watcher sẽ báo thay đổi do chính Lucerna ghi: nội dung đọc về có hash bằng `base_hash` thì bỏ qua.

Lệnh: `workbench.action.files.save` (⌘S, ghi cả khi không bẩn, như VS Code), `workbench.action.files.saveAll` (⌥⌘S), `workbench.action.files.saveWithoutFormatting` (⌘K S, bỏ qua bước 3), `workbench.action.files.revert` (đọc lại từ đĩa, là bước undo được).

### 6.2 Tự lưu

`files.autoSave`: `off` (mặc định) | `afterDelay` (`files.autoSaveDelay`, 1000 ms, đếm lại sau mỗi edit) | `onFocusChange` (editor mất focus) | `onWindowChange` (cửa sổ mất focus). Tự lưu không format, không lưu file đang xung đột hoặc đã bị xóa.

### 6.3 Đóng và thoát

- Đóng tab cuối cùng của một file bẩn: "Do you want to save the changes you made to {tên}?" / "Your changes will be lost if you don't save them." với **Save / Don't Save / Cancel** (VS Code). "Don't Save" bỏ thay đổi (document đóng, history mất).
- Đóng cửa sổ hoặc thoát (⌘Q) khi có file bẩn: một prompt liệt kê các file, **Save All / Don't Save / Cancel**.
- Không có hot exit trong Phase 4 (§19 Q4).

## 7. Xung đột khi agent ghi file

### 7.1 Trạng thái

Watcher (đã có) báo file đổi. `Documents` đọc file ở nền, so hash với `base_hash`:
- **Bằng nhau:** do chính Lucerna ghi, hoặc không đổi nội dung. Bỏ qua.
- **Buffer sạch:** reload: diff `snapshot → disk` áp như edit (một bước undo), `base = disk`, `saved` = transaction reload. Giữ cuộn và con trỏ (đã có, nay qua `Patch`).
- **Buffer bẩn, `lucerna.externalChanges.autoMerge` bật (mặc định):** gộp ba chiều (§7.2). Gộp được thì áp phần của đĩa vào buffer như một bước undo (kind `External`), `base = disk`, buffer vẫn bẩn (phần sửa của người dùng chưa lưu); status bar hiện "Merged changes from disk" trong 4 s. Không gộp được thì sang xung đột.
- **Xung đột:** `conflict = Some(Conflict { disk: snapshot, stamp })`. Không đụng vào buffer.
- **File bị xóa khi buffer bẩn:** tab gạch ngang; thanh báo "{tên} was deleted on disk." với "Close" và "Keep" (lưu sẽ tạo lại file, như VS Code). File sạch bị xóa: giữ hành vi Phase 1 (`DocState::Deleted`, nội dung cuối còn hiện).

### 7.2 Gộp ba chiều (`lucerna-editor::merge`)

- Đầu vào: `base`, `ours` (buffer), `theirs` (đĩa). Diff theo dòng `base → ours` và `base → theirs` bằng `similar`.
- Mỗi hunk của `theirs` được giữ nếu khoảng dòng của nó trong `base` **không chồng và không kề sát** bất kỳ hunk nào của `ours` (kề sát cũng coi là xung đột, cho an toàn). Có một hunk vi phạm thì cả lần gộp thất bại (không gộp một phần).
- Kết quả là danh sách edit trên `ours`, dịch qua các hunk của `ours` đứng trước.
- Chạy ở luồng nền; áp ở luồng chính nếu `version` chưa đổi, ngược lại chạy lại.

### 7.3 Giao diện (theo Zed, thêm thanh báo)

**Màn Zed tương ứng:** tab có chấm màu Warning thay nút ×; không có banner; lưu thì prompt "This file has changed on disk since you started editing it. Do you want to overwrite it?" với Overwrite / Discard Edits / Cancel (`workspace/src/pane.rs`).

Lucerna:
- **Tab:** chấm màu `warning` của theme (bẩn thường dùng `text.accent`).
- **Thanh xung đột** (`conflict_bar.rs`): một dòng ở đầu vùng editor, ngay dưới path bar, cao `SEARCH_BAR_HEIGHT`, dựng bằng `search_toolbar` + `search_bar_row` của `ui.rs` để cùng chiều cao và nền với thanh tìm; nội dung là `warning_row`: "{tên} changed on disk and conflicts with your unsaved changes." cùng hai `text_button` **Reload** và **Keep Mine**.
  - **Reload:** diff `buffer → disk` áp như một bước undo; buffer sạch; ⌘Z lấy lại bản của người dùng (khi đó lại bẩn).
  - **Keep Mine:** `base = disk`, `stamp` mới, `conflict = None`; thanh ẩn; lần lưu sau ghi đè không hỏi.
  - Đĩa đổi tiếp trong lúc thanh đang hiện: tính lại với `disk` mới.
- **Lưu khi đang xung đột:** prompt theo chữ của Zed, nút **Overwrite / Reload / Cancel** (Zed gọi là Discard Edits; Lucerna dùng cùng chữ với thanh báo).
- `files.saveConflictResolution: overwriteFileOnDisk` (VS Code) bỏ qua prompt khi lưu; thanh báo vẫn hiện.

## 8. Multi-cursor (bước 4.3)

Lệnh và phím VS Code (§14). Hành vi:
- `addSelectionToNextFindMatch` (⌘D): con trỏ rỗng → chọn từ dưới con trỏ, chế độ cả từ + phân biệt hoa thường; đã có chữ chọn (mọi selection cùng chữ) → tìm chữ đó theo `editor.selectedTextMatchMode` (`findOptions` lấy tùy chọn của thanh tìm). Tìm từ cuối selection mới thêm, quay vòng, bỏ kết quả chồng selection có sẵn (Aho-Corasick trên snapshot, ở nền nếu file lớn). Cuộn tới selection mới.
- `moveSelectionToNextFindMatch` (⌘K ⌘D): bỏ selection mới nhất, chọn kết quả kế.
- `selectHighlights` / `changeAll` (⇧⌘L / ⌘F2): mọi kết quả; selection ban đầu giữ là chính.
- `insertCursorAbove` / `Below` (⌥⌘↑ / ⌥⌘↓): giữ cột mục tiêu qua dòng ngắn; bấm chiều ngược lại xóa con trỏ vừa thêm (stack của Zed).
- `insertCursorAtEndOfEachLineSelected` (⇧⌥I).
- `cursorColumnSelect*` (⇧⌥⌘ + mũi tên), ⇧⌥-kéo: chọn khối, thay `block_selection` hiện có.
- ⌥-click thêm con trỏ; ⌥-click lên con trỏ phụ thì xóa. `editor.multiCursorModifier: ctrlCmd` đổi sang ⌘-click (khi đó ⌥-click làm việc của ⌘-click hiện nay: đi tới định nghĩa).
- Escape: nhiều selection → còn selection chính (`removeSecondaryCursors`); một selection không rỗng → con trỏ (`cancelSelection`).
- Paste `editor.multiCursorPaste: spread | full` như VS Code.
- Mọi lệnh ở §5 đã chạy trên `SelectionSet` từ bước 4.1, nên bước này chỉ thêm lệnh tạo selection và cách vẽ nhiều con trỏ.

## 9. Snippet (bước 4.5)

- **Parser** (chuyển thể ngữ pháp snippet của VS Code / LSP): `$1`, `${1:mặc định}` (lồng nhau), `${1|a,b|}`, `$0`, biến `TM_FILENAME`, `TM_FILENAME_BASE`, `TM_DIRECTORY`, `TM_FILEPATH`, `TM_SELECTED_TEXT`, `TM_CURRENT_LINE`, `TM_CURRENT_WORD`, `TM_LINE_INDEX`, `TM_LINE_NUMBER`, `CLIPBOARD`, `CURRENT_YEAR`, `CURRENT_MONTH`, `CURRENT_DATE`, `UUID`. Biến lạ chèn tên biến (như VS Code). Regex transform chưa hỗ trợ: chèn giá trị không biến đổi.
- **Chèn:** các dòng sau dòng đầu được thụt theo dòng hiện tại; tab trong snippet đổi theo `insertSpaces`. Chèn ở mọi con trỏ.
- **Phiên snippet** (`snippet_session.rs`): placeholder cùng số thành nhiều selection (đồng bộ nhờ multi-cursor). Tab / ⇧Tab (`jumpToNextSnippetPlaceholder` / `jumpToPrevSnippetPlaceholder`) đi qua các placeholder; tới `$0` hoặc Escape (`leaveSnippet`) hoặc con trỏ ra khỏi vùng snippet thì kết thúc. `${1|a,b|}` mở menu completion với các lựa chọn. Context key `inSnippetMode`, `hasNextTabstop`, `hasPrevTabstop`.
- **Nguồn:** `insertText` dạng snippet của LSP; snippet của extension ngôn ngữ (theo `prefix`, hiện trong menu completion theo `editor.snippetSuggestions`).

## 10. Completion (bước 4.5)

### 10.1 Kích hoạt (VS Code)

- Gõ ký tự từ, từ dưới con trỏ không phải số, `editor.quickSuggestions` cho phép theo loại token (`other` bật; `comments`, `strings` tắt; token xác định bằng capture của highlight query): sau `editor.quickSuggestionsDelay` (10 ms).
- Gõ trigger character của server (`editor.suggestOnTriggerCharacters`).
- ⌃Space (`editor.action.triggerSuggest`); khi menu đang mở thì ⌃Space bật / tắt phần tài liệu (`toggleSuggestionDetails`).
- Đang mở mà gõ tiếp: lọc lại tại chỗ nếu kết quả không `isIncomplete`, ngược lại hỏi lại server (request cũ bị hủy). Lùi trái qua điểm bắt đầu hoặc xuống dòng thì đóng.

### 10.2 Nguồn và xếp hạng

- Gửi `completion` tới mọi server của document có `completionProvider`, gộp kết quả. Từ trong các document đang mở cùng ngôn ngữ (`editor.wordBasedSuggestions: matchingDocuments`) chỉ dùng khi không server nào trả kết quả hoặc server lỗi (như `completions.words: fallback` của Zed). Snippet của extension luôn được thêm.
- Chấm điểm bằng `fuzzyScoreGracefulAggressive` chuyển thể từ VS Code (`base/common/filters.ts`, MIT) trên `filterText` hoặc label; `editor.suggest.matchOnWordStartOnly`. Sắp theo điểm giảm dần → `sortText` → label. Lọc ở nền khi danh sách > 2.000 mục.
- Mục được chọn sẵn: mục đầu (`editor.suggestSelection: first`).

### 10.3 Giao diện (theo Zed)

**Màn Zed tương ứng** (`editor/src/code_context_menus.rs`): popover dưới con trỏ, lật lên trên khi thiếu chỗ; rộng 280–540 px; cao 3–12 dòng, danh sách ảo hóa, không thanh cuộn; mỗi dòng là label tô màu theo cú pháp với ký tự khớp in đậm, detail mờ ở bên phải; tài liệu Markdown ở popover thứ hai bên phải (260–500 px), không đủ chỗ thì ở dưới.

Lucerna:
- Dùng `ui::popover` (nền, viền, bo góc `POPOVER_RADIUS` như hover) và `uniform_list` của GPUI. Font của editor cho label, `UI_FONT_SIZE_SMALL` cho detail.
- Label tô theo loại mục (function, method → màu `function` của theme; struct, class, enum, interface → `type`; keyword → `keyword`; biến, field → `variable`; constant → `constant`; snippet → màu chữ thường kèm chữ "snippet" mờ ở cột detail). **Không có icon** (khác VS Code, `editor.suggest.showIcons` bị bỏ qua) vì Zed không dùng icon.
- Tài liệu: `completionItem/resolve` cho mục đang chọn và ±4 mục quanh nó; vẽ bằng `markdown.rs` hiện có (dùng chung với hover), cache 16 mục.
- Trạng thái trống: chỉ khi gọi bằng ⌃Space, hiện "No suggestions" một dòng trong popover rồi tự đóng sau 1,5 s.

### 10.4 Nhận một mục

- Enter (`editor.acceptSuggestionOnEnter: on | smart | off`) và Tab nhận; ⇧Enter / ⇧Tab nhận theo chế độ ngược với `editor.suggest.insertMode` (`insert` mặc định ↔ `replace`). Ký tự commit của mục (`acceptSuggestionOnCommitCharacter`) nhận rồi gõ ký tự đó.
- Áp: undo stop trước; thay khoảng `insert` / `replace` ở mọi con trỏ (dịch theo con trỏ chính); chèn qua snippet engine (văn bản thường được escape); `additionalTextEdits` đã resolve thì áp cùng transaction, chưa resolve thì resolve rồi áp sau nếu người dùng chưa sửa phía trước vị trí đó; undo stop sau. `command` của mục bị bỏ qua (chưa có code action).

## 11. Rename (bước 4.6)

**Màn Zed tương ứng** (`editor.rs` `rename`): editor một dòng chèn thành block ngay dưới dòng của symbol, thụt tới cột của symbol, chữ đậm cùng màu cú pháp, nền trong suốt; các chỗ dùng symbol trong file mờ còn 0,6; Enter xác nhận, Escape hủy.

Lucerna:
1. F2 (`editor.action.rename`): `prepareRename` nếu server hỗ trợ, ngược lại lấy từ dưới con trỏ. Bị từ chối hoặc không có từ: hiện thông báo một dòng tại con trỏ (dùng `GotoMessage` sẵn có, như "No definition found").
2. Popover không viền gắn ngay dưới symbol, căn trái theo cột symbol, chứa `TextInput` một dòng: font editor, đậm, màu cú pháp của symbol; điền tên cũ, chọn hết (hoặc giữ vị trí con trỏ nếu con trỏ đang ở trong symbol). Bề rộng theo nội dung, tối thiểu 20 ký tự. Context key `renameInputVisible`.
3. Gửi `documentHighlight` (nếu có) để làm mờ các chỗ dùng symbol trong file.
4. Enter: `rename` → `WorkspaceEdit`. Kiểm tra version của từng file đang mở (lệch thì báo "{tên} has changed in the meantime" và không áp gì). Áp:
   - file đang mở: một transaction riêng mỗi file (undo theo từng file, như Zed);
   - file chưa mở: nạp vào `Documents` không có view, áp edit;
   - đổi hơn một file và `files.refactoring.autoSave` bật (mặc định): lưu các file **trước đó không bẩn**; file bẩn sẵn thì để người dùng lưu. Tắt setting: mở mỗi file đã sửa thành tab để thấy chấm bẩn.
5. Escape hoặc mất focus: hủy, không gửi gì.

## 12. Format (bước 4.6)

- `editor.action.formatDocument` (⇧⌥F) gọi `textDocument/formatting`; `editor.action.formatSelection` (⌘K ⌘F) gọi `rangeFormatting` cho mọi selection (rỗng thì dòng hiện tại).
- Chọn server: `editor.defaultFormatter` (theo ngôn ngữ) nhận **tên language server** trong bảng server (ví dụ `"rust-analyzer"`), lệch khỏi VS Code vì Lucerna không có extension id; không đặt thì server đầu tiên của document có capability. Không có server nào: thông báo "No formatter for {ngôn ngữ}".
- `FormattingOptions` lấy từ `tabSize`, `insertSpaces` (sau khi đoán), `files.trimTrailingWhitespace`, `files.insertFinalNewline`, `files.trimFinalNewlines`.
- **Timeout 5 s** (Zed `FORMAT_TIMEOUT`): quá thì bỏ, thông báo "Formatting timed out ({server})"; khi đang lưu thì vẫn lưu, không format.
- **Hủy khi người dùng sửa:** kết quả mang `version`; version đã đổi thì bỏ (cả VS Code và Zed).
- **Áp dạng diff:** edit của server được đổi thành văn bản mới rồi diff theo dòng với văn bản hiện tại (như Zed), để con trỏ, folds, diagnostics không nhảy. Format thủ công là một bước undo; format khi lưu nằm trong bước undo của các thao tác trước khi lưu (§6.1).
- `editor.formatOnSave` (false, theo ngôn ngữ); `editor.formatOnSaveMode` chỉ nhận `file` (giá trị khác được coi như `file`, ghi cảnh báo trong log).

## 13. Vim

**Phase 4 (bước 4.1):**
- Normal / Visual giữ như V1. Phím có ⌘ (⌘Z, ⌘V, ⌘S, ⌘D…) vẫn chạy lệnh sửa của editor.
- Insert mode tối thiểu: `i`, `a`, `I`, `A`, `o`, `O` vào Insert; phím trong Insert đi vào đường gõ chữ của editor (cả IME, completion, snippet); Escape về Normal, con trỏ lùi một ký tự. Con trỏ Insert là thanh, Normal là khối (`CursorOverlay` hiện có thêm `CursorShape::Bar`). Status bar hiện `-- INSERT --`. Cả lần Insert là một bước undo.
- Chưa có: toán tử, `.`, register, `u` / ⌃R, đếm cho insert, `r`, `x`… (V2).

**Hợp đồng cho V2** (để V2 không phải sửa lại mô hình):
1. Mọi thay đổi văn bản của V2 là `EditPlan` dựng bằng hàm của `lucerna-editor` (xóa khoảng, chèn, thay, thụt lề, đổi hoa thường); text object dùng `enclosing_nodes` và hàm từ / câu / đoạn của `lucerna-editor`.
2. `CodeView::transact_group(|cx| …)` cho phép V2 gom nhiều `EditPlan` (ví dụ `3dd`, `ciw` + chữ gõ) thành một bước undo (`group_until`).
3. `.` được V2 cài bằng cách ghi lại phím của lần thay đổi cuối (kể cả chữ gõ trong Insert), như Vim; editor không cần ghi nhớ gì thêm.
4. Register thuộc `VimGlobals`; register không tên đồng bộ với clipboard hệ thống theo `vim.useSystemClipboard` (V2).
5. Selection của Vim chuyển sang `SelectionSet` (visual block là nhiều selection), nên multi-cursor và Vim dùng chung cách vẽ.

## 14. Lệnh, phím và settings

### 14.1 Lệnh và phím mặc định (macOS, theo VS Code)

| Bước | Lệnh | Phím |
|---|---|---|
| 4.1 | `undo` / `redo` | ⌘Z / ⇧⌘Z |
| 4.1 | `cursorUndo` / `cursorRedo` | ⌘U / — |
| 4.1 | `editor.action.selectAll` | ⌘A |
| 4.1 | `cursorLeft/Right/Up/Down` (+`Select`) | ← → ↑ ↓ (⇧), ⌃B ⌃F ⌃P ⌃N |
| 4.1 | `cursorWordLeft` / `cursorWordEndRight` (+`Select`) | ⌥← / ⌥→ |
| 4.1 | `cursorHome` / `cursorEnd` (+`Select`) | ⌘← / ⌘→, Home / End |
| 4.1 | `cursorLineStart` / `cursorLineEnd` | ⌃A / ⌃E |
| 4.1 | `cursorTop` / `cursorBottom` (+`Select`) | ⌘↑ / ⌘↓ |
| 4.1 | `cursorPageUp` / `cursorPageDown` (+`Select`) | PageUp / PageDown |
| 4.1 | `deleteLeft` / `deleteRight` | ⌫ / ⌦, ⌃H / ⌃D |
| 4.1 | `deleteWordLeft` / `deleteWordRight` | ⌥⌫ / ⌥⌦ |
| 4.1 | `deleteAllLeft` / `deleteAllRight` | ⌘⌫ / ⌘⌦, ⌃K |
| 4.1 | `tab` / `outdent` | Tab / ⇧Tab |
| 4.1 | `lineBreakInsert` | ⌃O |
| 4.1 | `editor.action.clipboardCutAction` / `clipboardPasteAction` | ⌘X / ⌘V |
| 4.1 | `workbench.action.files.save` / `saveAll` / `saveWithoutFormatting` | ⌘S / ⌥⌘S / ⌘K S |
| 4.1 | `workbench.action.files.revert` | — |
| 4.2 | `editor.action.commentLine` / `addCommentLine` / `removeCommentLine` | ⌘/ / ⌘K ⌘C / ⌘K ⌘U |
| 4.2 | `editor.action.blockComment` | ⇧⌥A |
| 4.2 | `editor.action.moveLinesUpAction` / `moveLinesDownAction` | ⌥↑ / ⌥↓ |
| 4.2 | `editor.action.copyLinesUpAction` / `copyLinesDownAction` | ⇧⌥↑ / ⇧⌥↓ |
| 4.2 | `editor.action.deleteLines` | ⇧⌘K |
| 4.2 | `editor.action.insertLineAfter` / `insertLineBefore` | ⌘Enter / ⇧⌘Enter |
| 4.2 | `editor.action.indentLines` / `outdentLines` | ⌘] / ⌘[ |
| 4.2 | `editor.action.joinLines` | ⌃J |
| 4.2 | `editor.action.trimTrailingWhitespace` | ⌘K ⌘X |
| 4.2 | `editor.action.smartSelect.expand` / `shrink` | ⌃⇧⌘→ / ⌃⇧⌘← |
| 4.3 | `editor.action.addSelectionToNextFindMatch` | ⌘D |
| 4.3 | `editor.action.moveSelectionToNextFindMatch` | ⌘K ⌘D |
| 4.3 | `editor.action.addSelectionToPreviousFindMatch` / `moveSelectionToPreviousFindMatch` | — |
| 4.3 | `editor.action.selectHighlights` / `editor.action.changeAll` | ⇧⌘L / ⌘F2 |
| 4.3 | `editor.action.insertCursorAbove` / `insertCursorBelow` | ⌥⌘↑ / ⌥⌘↓ |
| 4.3 | `editor.action.insertCursorAtEndOfEachLineSelected` | ⇧⌥I |
| 4.3 | `cursorColumnSelectLeft/Right/Up/Down/PageUp/PageDown` | ⇧⌥⌘ + ← → ↑ ↓ PageUp PageDown |
| 4.3 | `removeSecondaryCursors` / `cancelSelection` | Escape |
| 4.5 | `editor.action.triggerSuggest` | ⌃Space |
| 4.5 | `acceptSelectedSuggestion` / `acceptAlternativeSelectedSuggestion` | Enter, Tab / ⇧Enter, ⇧Tab |
| 4.5 | `selectNextSuggestion` / `selectPrevSuggestion` | ↓ / ↑, ⌃N / ⌃P |
| 4.5 | `selectNextPageSuggestion` / `selectPrevPageSuggestion` | PageDown / PageUp |
| 4.5 | `hideSuggestWidget` | Escape |
| 4.5 | `toggleSuggestionDetails` | ⌃Space (khi menu mở) |
| 4.5 | `jumpToNextSnippetPlaceholder` / `jumpToPrevSnippetPlaceholder` / `leaveSnippet` | Tab / ⇧Tab / Escape |
| 4.6 | `editor.action.rename` | F2 |
| 4.6 | `acceptRenameInput` / `cancelRenameInput` | Enter / Escape |
| 4.6 | `editor.action.formatDocument` / `formatSelection` | ⇧⌥F / ⌘K ⌘F |

Phím của lệnh chỉ thêm khi bước tương ứng giao. Phím trùng với lệnh hiện có được kiểm tra khi thêm (ví dụ ⌘K ⌘X, ⌘U hiện chưa dùng).

**Context key mới:** `editorReadonly` (false khi file sửa được; true với file lỗi nạp, file nhị phân, view preview của picker), `editorHasSelection`, `editorHasMultipleSelections`, `suggestWidgetVisible`, `inSnippetMode`, `hasNextTabstop`, `hasPrevTabstop`, `renameInputVisible`, `activeEditorIsDirty`.

### 14.2 Settings mới (theo VS Code trừ khi ghi khác)

| Key | Mặc định | Bước |
|---|---|---|
| `editor.insertSpaces` | `true` | 4.1 |
| `editor.trimAutoWhitespace` | `true` | 4.1 |
| `editor.wordSeparators` | `` `~!@#$%^&*()-=+[{]}\|;:'",.<>/? `` | 4.1 |
| `editor.cursorBlinking` | `"blink"` (`solid` tắt nhấp nháy; giá trị khác như `blink`) | 4.1 |
| `editor.cursorStyle` | `"line"` (`line`, `block`, `underline`) | 4.1 |
| `editor.emptySelectionClipboard` | `true` | 4.1 |
| `files.eol` | `"auto"` (chỉ dùng cho file không có dòng nào) | 4.1 |
| `files.encoding` | `"utf8"` (chỉ dùng khi không đoán được) | 4.1 |
| `files.insertFinalNewline` / `files.trimFinalNewlines` / `files.trimTrailingWhitespace` | `false` | 4.1 |
| `files.autoSave` / `files.autoSaveDelay` | `"off"` / `1000` | 4.1 |
| `files.saveConflictResolution` | `"askUser"` | 4.1 |
| `editor.autoIndent` | `"full"` | 4.2 |
| `editor.detectIndentation` | `true` | 4.2 |
| `editor.autoClosingBrackets` / `autoClosingQuotes` | `"languageDefined"` | 4.2 |
| `editor.autoClosingOvertype` / `autoClosingDelete` | `"auto"` | 4.2 |
| `editor.autoSurround` | `"languageDefined"` | 4.2 |
| `editor.multiCursorModifier` | `"alt"` | 4.3 |
| `editor.multiCursorMergeOverlapping` | `true` | 4.3 |
| `editor.multiCursorPaste` | `"spread"` | 4.3 |
| `editor.multiCursorLimit` | `10000` | 4.3 |
| `editor.selectedTextMatchMode` | `"findOptions"` | 4.3 |
| `lucerna.externalChanges.autoMerge` (riêng Lucerna) | `true` | 4.4 |
| `editor.quickSuggestions` | `{ "other": "on", "comments": "off", "strings": "off" }` | 4.5 |
| `editor.quickSuggestionsDelay` | `10` | 4.5 |
| `editor.suggestOnTriggerCharacters` | `true` | 4.5 |
| `editor.acceptSuggestionOnEnter` | `"on"` | 4.5 |
| `editor.acceptSuggestionOnCommitCharacter` | `true` | 4.5 |
| `editor.suggestSelection` | `"first"` (chỉ hỗ trợ `first`) | 4.5 |
| `editor.snippetSuggestions` | `"inline"` | 4.5 |
| `editor.wordBasedSuggestions` | `"matchingDocuments"` | 4.5 |
| `editor.suggest.insertMode` | `"insert"` | 4.5 |
| `editor.suggest.filterGraceful` / `editor.suggest.matchOnWordStartOnly` | `true` / `true` | 4.5 |
| `files.refactoring.autoSave` | `true` | 4.6 |
| `editor.formatOnSave` | `false` | 4.6 |
| `editor.formatOnSaveMode` | `"file"` | 4.6 |
| `editor.defaultFormatter` | `null` (tên language server) | 4.6 |

`editor.quickSuggestions.other` mặc định `"on"` thay vì `"offWhenInlineCompletions"`, và `editor.wordBasedSuggestions` mặc định `"matchingDocuments"` thay vì `"offWithInlineSuggestions"` của VS Code `main`: Lucerna không có inline completion nên các cặp giá trị này cho cùng hành vi; ghi giá trị trực tiếp cho dễ hiểu.

## 15. Hiệu năng

Mục tiêu (MacBook Air M2, bản release):
- Gõ một phím trong file 10.000 dòng có tô màu: từ lúc nhận phím tới khi khung hình mới được gửi đi ≤ 8 ms ở p95. Edit O(log n); không chép toàn văn bản ở luồng chính.
- File 1 MB một dòng hoặc file lớn hơn `lucerna.largeFile.*`: vẫn sửa được (không tô màu, không LSP, như Phase 3); gõ ≤ 16 ms ở p95.
- 1.000 con trỏ gõ một ký tự ≤ 16 ms.
- Undo một transaction 10.000 edit (ví dụ ⇧⌘L rồi gõ) ≤ 50 ms.
- Parse đồng bộ sau edit dùng `SYNC_PARSE_BUDGET` hiện có; đo trên file Rust 10.000 dòng, chỉnh giá trị nếu không đạt mục tiêu trên.
- Lọc completion 5.000 mục ≤ 5 ms ở luồng chính, nhiều hơn thì lọc ở nền.
- Bench mới trong `lucerna-text` và `lucerna-editor` (`cargo bench`, chỉ dùng thư viện đã có giấy phép phù hợp), chạy tay trước khi giao mỗi bước.

## 16. Xử lý lỗi

- Ghi file lỗi: file cũ còn nguyên (atomic), buffer vẫn bẩn, thông báo trên status bar có lý do; `Unencodable` chỉ vị trí ký tự đầu tiên.
- Server trả completion / format / rename lỗi hoặc quá hạn: ghi log server, thông báo một dòng (format, rename) hoặc im lặng (completion tự kích hoạt).
- `WorkspaceEdit` có thao tác file hoặc version lệch: không áp gì, thông báo.
- Gộp ba chiều gặp file nhị phân hoặc giải mã lỗi: coi như xung đột, thanh báo ghi lý do.
- Panic trong lệnh sửa: bắt bằng `catch_panic` sẵn có; transaction đang mở được đóng, buffer giữ trạng thái trước lệnh (edit chỉ được áp sau khi `EditPlan` dựng xong).

## 17. Chia bước giao

Mỗi bước có plan, test và review riêng, giao được độc lập (theo thứ tự phụ thuộc):

| Bước | Nội dung | Phụ thuộc |
|---|---|---|
| **4.1 Lõi sửa và lưu** | `lucerna-text` (edit, Patch, History, chuẩn hóa EOL, ghi atomic); `lucerna-editor` (SelectionSet, di chuyển, gõ, xóa, Enter giữ thụt lề, Tab, clipboard, `UndoStops`); `CodeView` nhận chữ + IME + con trỏ nhấp nháy; undo/redo/cursorUndo; lưu, lưu tất cả, revert, tự lưu, các thao tác `files.*` khi lưu; chấm bẩn, prompt đóng / thoát; LSP `didChange` tăng dần + `didSave`; syntax, folds, diagnostics, highlight tìm kiếm dịch theo `Patch`; reload là bước undo; **chống mất dữ liệu tối thiểu**: không reload khi bẩn, chấm cảnh báo trên tab, prompt Overwrite / Reload / Cancel khi lưu; Vim Insert tối thiểu. | — |
| **4.2 Gõ thông minh** | Auto-close, overtype, surround; `autoIndent` (`brackets`, `indents.scm`); `detectIndentation` + mục status bar; comment dòng / khối; lệnh dòng; smart select; đọc thêm `config.toml`. | 4.1 |
| **4.3 Multi-cursor** | Lệnh tạo selection (§8), ⌥-click, ⇧⌥-kéo, chọn cột, paste `spread`, vẽ nhiều con trỏ. | 4.1 |
| **4.4 Xung đột với agent** | Gộp ba chiều, thanh xung đột, Reload / Keep Mine, file bẩn bị xóa, `files.saveConflictResolution`. | 4.1 |
| **4.5 Completion và snippet** | Parser + phiên snippet, snippet extension, request completion / resolve, menu kiểu Zed, chấm điểm VS Code, word-based. | 4.3 (placeholder cùng số dùng multi-cursor) |
| **4.6 Rename và format** | F2 + ô rename, áp `WorkspaceEdit` nhiều file, `files.refactoring.autoSave`; format document / selection / on save, timeout, áp dạng diff. | 4.1 |

4.2, 4.3, 4.4, 4.6 làm song song được sau 4.1 nếu tách file cẩn thận (4.2 và 4.3 cùng chạm `lucerna-editor`, nên làm tuần tự hoặc chia module rõ).

Sau mỗi bước: cập nhật README và bảng roadmap (§1.3 của spec Phase 1), sửa câu "Khung xem code: Chỉ đọc" trong spec Phase 1 khi 4.1 giao.

## 18. Kiểm thử

- **`lucerna-text`:** edit nhiều khoảng; undo / redo trả đúng văn bản và token selection; lồng transaction; `push_stop`, `group_until`; bẩn / sạch khi undo về và redo qua chỗ đã lưu; `Patch::map_offset` với bias, `compose`; chuẩn hóa EOL (LF, CRLF, lẫn lộn, `\r` lẻ); ghi rồi đọc lại với UTF-8, UTF-8 BOM, UTF-16LE, Shift-JIS, Latin-1; `Unencodable`; ghi atomic giữ quyền; symlink và hardlink ghi tại chỗ; lỗi giữa chừng không để lại file tạm.
- **`lucerna-editor`:** test bảng bằng ký hiệu đánh dấu cho mọi lệnh (§14.1) với một và nhiều selection; bảng `should_stop_between` so với ví dụ của VS Code (`abc def` → hai bước, `abc  def` → ba bước); auto-close theo `autoclose_before`, `not_in`, dấu nháy sau chữ; Enter giữa ngoặc; comment toggle; ⌘D quay vòng, bỏ chồng lấn; gộp selection (chồng, chạm, cùng điểm đầu); gộp ba chiều (không chồng → gộp; chồng hoặc kề → lỗi; thêm / xóa cuối file; file rỗng) và test thuộc tính: kết quả gộp chứa nguyên mọi hunk của hai phía khi không chồng; parser snippet (lồng, escape, biến, lựa chọn); `fuzzyScore` khớp các ví dụ trong test của VS Code (ghi nguồn).
- **`lucerna-lsp`:** fake server: `didChange` tăng dần đúng khoảng với UTF-8 / UTF-16 / UTF-32, `FULL` vẫn gửi toàn bộ; `didSave` theo `includeText`; completion + resolve; rename với `changes` và `documentChanges`, từ chối thao tác file; formatting.
- **`lucerna-workbench` (TestAppContext):** gõ qua input handler (cả marked text), undo nhóm đúng, chấm bẩn xuất hiện / biến mất; lưu ghi đúng byte; prompt đóng tab và thoát (giả lập câu trả lời); file đổi trên đĩa khi sạch → reload, ⌘Z quay lại; khi bẩn không chồng → gộp; chồng → thanh xung đột, Reload, Keep Mine, prompt khi lưu; tự lưu bỏ qua file xung đột; hai view cùng file dịch selection cho nhau; menu completion mở, lọc, nhận, đóng; rename nhiều file với `files.refactoring.autoSave`; format bị hủy khi gõ trong lúc chờ; timeout format khi lưu vẫn lưu.
- **Kiểm tra tay trước khi giao mỗi bước:** đặt cạnh Zed (menu completion, ô rename, chấm trên tab, thanh xung đột: khoảng cách, chiều cao, màu nền, viền, hover, trạng thái trống) và cạnh VS Code (phím, nhóm undo). Kịch bản agent: chạy Claude Code sửa một file trong lúc đang gõ ở file đó, cả hai trường hợp chồng và không chồng.

## 19. Câu hỏi cần người dùng duyệt

Các điểm dưới đây đã có lựa chọn trong spec, nhưng lệch khỏi VS Code hoặc Zed, hoặc là đánh đổi về phạm vi:

| # | Câu hỏi | Spec đang chọn | Phương án khác |
|---|---|---|---|
| Q1 | Xử lý file bẩn bị agent sửa | Tự gộp khi không chồng + thanh báo khi chồng (D5) | Chỉ chấm trên tab và hỏi khi lưu như Zed (ít việc hơn, bỏ được bước 4.4) |
| Q2 | Nhóm undo | Quy tắc VS Code (D3) | Gộp theo 300 ms như Zed (đơn giản hơn, nhưng `abc def` gõ nhanh thành một bước) |
| Q3 | Vim trong Phase 4 | Insert tối thiểu (D10) | Không có gì: người bật Vim phải tắt Vim để sửa tới V2 |
| Q4 | Hot exit, giữ undo sau khi đóng, file mới ⌘N, Save As | Ngoài phạm vi | Thêm một bước 4.7 |
| Q5 | Format khi chậm | Timeout cứng 5 s (D9) | "Saving…" + Skip như VS Code |
| Q6 | Icon trong menu completion | Không icon, tô màu label theo loại (theo Zed) | Icon theo loại như VS Code |

## 20. Rủi ro

- **IME với nhiều con trỏ và với GPUI tại rev đang pin:** `TextInput` đã chạy IME cho một ô; editor nhiều dòng cần `bounds_for_range` đúng khi word wrap. Làm sớm trong 4.1 và thử Telex, VNI, tiếng Nhật.
- **Chuẩn hóa EOL đổi offset:** tìm trong file, hover, diagnostics, vị trí LSP đang tính trên rope có `\r`. Phải rà mọi chỗ dùng `len_bytes`, `point_to_offset` trong 4.1; search toàn project đọc file từ đĩa nên không ảnh hưởng.
- **Gộp ba chiều sai thì mất dữ liệu âm thầm:** chỉ gộp khi không chồng và không kề, gộp là bước undo, có test thuộc tính; có setting tắt.
- **Watcher trễ hoặc gộp sự kiện** làm Lucerna tưởng file do mình ghi là file của agent (hoặc ngược lại): luôn so hash nội dung, không chỉ mtime.
- **Bước 4.1 lớn:** nếu plan cho thấy quá ba tuần, tách 4.1a (lõi edit, gõ, undo, lưu, chống mất dữ liệu) và 4.1b (tự lưu, prompt thoát, Vim Insert, cursorUndo).

## 21. Thay đổi spec cũ (khi giao)

- `2026-11-08-lucerna-phase1-design.md`: §2.1 "Khung xem code: Chỉ đọc" → trỏ sang spec này; bảng §1.3 dòng 4 ghi link spec.
- `2026-11-10-lucerna-phase3-lsp-design.md`: §1 "Lucerna vẫn chỉ đọc" → ghi chú Phase 4.
- `2026-11-10-vim-navigation-design.md`: ghi chú Insert tối thiểu đã có từ Phase 4, V2 dựa trên hợp đồng ở §13.
- Trang Extensions: bỏ chữ "(Phase 4)" sau số snippet khi 4.5 giao.
