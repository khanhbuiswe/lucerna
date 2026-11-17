# Phase 4.3: Multi-cursor

Ngày 2026-11-17. Trạng thái: thiết kế, chờ duyệt. Đã qua builder check.

Spec cha: [2026-11-11-lucerna-phase4-editing-design.md](2026-11-11-lucerna-phase4-editing-design.md) §8, §14, §17. Nghiên cứu VS Code / Zed: [docs/research/2026-11-11-editing.md](../../research/2026-11-11-editing.md) §3. Spec này chốt phạm vi và cách làm của bước 4.3; chỗ nào khác spec cha thì spec này thắng.

## 1. Ý định

Bước tiếp theo trên roadmap (§1.3 spec Phase 1): V2 đã xong, và 4.5 (completion, snippet) cần multi-cursor cho placeholder cùng số. Người dùng chủ yếu đọc và chỉnh nhẹ code do agent viết; ⌘D, ⌥-click, chọn cột là những thao tác sửa hàng loạt nhanh nhất mà người quen VS Code dùng hằng ngày.

## 2. Mục tiêu, không làm, phản mục tiêu

**Mục tiêu**
- Mọi lệnh 4.3 trong bảng §14.1 của spec cha chạy như VS Code, với phím mặc định macOS của VS Code.
- ⌥-click thêm / bỏ con trỏ, ⌥-kéo thêm một vùng chọn, ⇧⌥-kéo chọn cột.
- Escape: nhiều selection → còn selection chính; một selection không rỗng → con trỏ.
- Paste theo `editor.multiCursorPaste`.
- Bốn setting: `editor.multiCursorModifier`, `editor.multiCursorMergeOverlapping`, `editor.multiCursorPaste`, `editor.multiCursorLimit` (mặc định như §14.2 spec cha; xem §5.3 về `selectedTextMatchMode`).
- Status bar có nhiều selection thì hiện như VS Code: `N selections`, thêm ` (M characters selected)` khi có chữ chọn; một selection có chữ thì `Ln x, Col y (M selected)`.
- `block_selection` trong `CodeView` bị bỏ. Chọn cột (cả `<C-v>` của Vim) là nhiều selection thật.

**Không làm (bước này)**
- Toán tử Vim trên Visual Block (`d`, `c`, `I`, `A` trên khối), register Blockwise. Người dùng đã chọn để thành bước riêng. `<C-v>` vẫn chỉ chọn và `y` như hiện nay, chỉ đổi cách lưu và vẽ.
- `editor.columnSelection` (chế độ chọn cột bật tắt), chuột giữa kéo chọn cột, `editor.action.selectAllMatches` (⌥Enter trong thanh Find), lệnh `cursorRedo`.
- Vim chạy motion / toán tử trên nhiều con trỏ (như Zed). Phần này làm lại lõi `lucerna-vim`, gộp vào bước Visual Block sau. Bước này chỉ làm cách A ở §5.2 "Vim".

**Phản mục tiêu**
- Gõ chữ chậm đi khi có nhiều con trỏ (mục tiêu: 1000 con trỏ vẫn gõ không giật; xem §15 spec cha).
- ⌘D treo luồng UI trên file lớn.
- Thanh Find và ⌘D khớp chữ khác nhau với cùng tùy chọn.
- Hành vi một con trỏ hiện nay (click, kéo, ⌘-click đi tới định nghĩa, Vim) thay đổi khi không dùng phím bổ trợ của multi-cursor.

## 3. Ràng buộc

- Hành vi, phím, setting theo VS Code; giao diện theo Zed (`docs/design-principles.md`). Chuỗi hiển thị tiếng Anh.
- Chỉ `lucerna-workbench` dùng gpui. Logic selection nằm trong `lucerna-editor`, test không cần cửa sổ.
- Không chép code GPL của Zed. Code phỏng theo VS Code (MIT) ghi dòng `Adapted from microsoft/vscode …` như `clipboard.rs`.
- `cargo deny check` phải qua; không thêm dependency mới (regex đã có trong workspace).

## 4. Cách làm (Approaches)

| Cách | Đánh đổi | Kết quả |
|---|---|---|
| **A. Logic trong `lucerna-editor::multicursor`, tìm bằng regex dùng chung với thanh Find, bỏ `block_selection`** | Một mô hình selection; ⌘D khớp giống thanh Find (như VS Code); test thuần Rust. Phải sửa chỗ Vim gọi `set_block_selection` và đường vẽ khối. | **Người dùng chọn.** |
| B. Như A nhưng tìm bằng Aho-Corasick (như Zed, spec cha ghi vậy) | Nhanh hơn khi rất nhiều kết quả, nhưng thêm dependency và khớp khác thanh Find. Regex literal đã đủ nhanh. | Loại. |
| C. Giữ `block_selection`, đổi thành selection khi bắt đầu sửa | Ít thay đổi, nhưng hai mô hình selection song song, trái hợp đồng §13.5 spec cha, làm khó bước Visual Block sau. | Loại. |

## 5. Thiết kế

### 5.1 `lucerna-editor`

**Tìm kiếm dùng chung.** Chuyển `find_in_snapshot` và `build_regex` từ `lucerna-workbench/src/find_widget.rs` vào module mới `lucerna-editor::find` (thêm `regex` vào `lucerna-editor`, đã có trong workspace). Thanh Find và ⌘D cùng gọi module này. *(Quyết định của Claude.)*
- **Cả từ** theo ranh giới từ của editor (`words.rs`, ngữ nghĩa `editor.wordSeparators` của VS Code; VS Code dùng cách này cho cả Find lẫn ⌘D từ bản 1.44), không theo `\b` của regex: lấy kết quả của regex không có `\b` rồi lọc kết quả có hai đầu là ranh giới từ. Nhờ vậy `$foo` (PHP) hay `foo-bar` (CSS) khớp được. Thanh Find đổi theo: đây là thay đổi có chủ đích, gần VS Code hơn; test Find hiện có với chữ thường phải qua, test nào dựa vào `\b` thì sửa kỳ vọng và ghi lý do. Tìm trong project (`lucerna-project::search::build_pattern`, ripgrep) giữ nguyên `\b`.
- **API mới cho ⌘D:** `find_next(snap, query, from, backwards) -> Option<Range>` tìm từ một offset, quay vòng, dừng ở kết quả hợp lệ đầu tiên, đọc theo chunk của rope, không copy cả buffer. `find_in_snapshot` hiện copy cả buffer và gom mọi kết quả nên không dùng cho ⌘D. Với regex literal trên rope, một lần ⌘D trên file vài chục MB vẫn dưới một frame nếu kết quả gần; trường hợp xấu (kết quả duy nhất ở xa trong file rất lớn) builder đo, vượt ngưỡng thì chạy nền như thanh Find (`BACKGROUND_SEARCH_BYTES`).
- **⌘D và thanh Find:** khi thanh Find đang mở ở view đó, ⌘D (lần đầu từ con trỏ rỗng) đặt chữ tìm của thanh Find thành từ vừa chọn, như VS Code; thanh Find đóng thì không đụng. Tùy chọn của thanh Find không bị đổi. *(Quyết định của Claude.)*

**Module `multicursor`.** Hàm thuần nhận `(snapshot, &SelectionSet, …)` trả về `SelectionSet` mới (hoặc `None` khi không có gì đổi). Mỗi selection mới lấy id từ `next_id()`, nên "selection mới nhất" là id lớn nhất.

- `add_next_match(snap, set, opts) / add_previous_match`
  - Có selection rỗng → mỗi selection rỗng mở thành từ dưới nó, selection có chữ giữ nguyên; lần này không tìm thêm (VS Code). Phiên bắt đầu như vậy khớp cả từ + phân biệt hoa thường.
  - Mọi selection có chữ: chữ tìm là chữ của selection mới nhất; các selection không cùng chữ thì không làm gì (VS Code). Phiên bắt đầu từ chữ người dùng tự chọn dùng tùy chọn hoa thường / cả từ hiện tại của thanh Find (không lấy regex: chữ luôn là literal), như VS Code.
  - Tìm từ cuối (hoặc đầu, chiều ngược) selection mới nhất bằng `find_next`, quay vòng, bỏ kết quả chồng selection có sẵn. Hết kết quả mới → không đổi.
- `move_to_next_match / move_to_previous_match`: như trên nhưng bỏ selection mới nhất rồi thêm kết quả kế. Chỉ còn một selection thì giống ⌘D nhưng thay luôn selection đó.
- `select_all_matches` (⇧⌘L và `changeAll` ⌘F2): mọi kết quả của chữ chọn (hoặc từ dưới con trỏ); selection chứa con trỏ chính ban đầu giữ là chính. Có thể nhiều kết quả: chạy ở nền khi file vượt ngưỡng (builder chọn ngưỡng, ví dụ như thanh Find đang làm), cắt ở `multiCursorLimit`.
- `insert_cursor_above / below`: mỗi selection có một con trỏ mới ở dòng trên / dưới head của nó (như VS Code và Zed), giữ cột hiển thị mục tiêu (`Goal::Column`, tính theo tab) qua dòng ngắn; trùng thì gộp. Dòng là dòng hiển thị khi word wrap bật (VS Code dùng dòng view); builder có thể làm theo dòng buffer trước nếu đơn giản hơn nhiều, ghi rõ trong plan. Bấm chiều ngược lại ngay sau đó xóa lớp con trỏ vừa thêm (stack các lớp, theo spec cha và Zed; VS Code không có). Stack bị xóa khi selection đổi bởi đường khác. *(Mô hình stack theo lớp là quyết định của Claude.)*
- `cursors_at_line_ends` (⇧⌥I): theo VS Code — selection rỗng bị bỏ khỏi tập kết quả; mỗi dòng của selection không rỗng được một con trỏ ở cuối dòng, riêng dòng cuối: con trỏ ở cuối selection nếu nó dừng giữa dòng, không có con trỏ nếu nó dừng ở cột 1. Không còn gì → giữ nguyên.
- `column_select(snap, from: (row, visual_col), to: (row, visual_col))`: một selection mỗi dòng từ `from.row` tới `to.row`, theo cột hiển thị (tab tính đủ chiều rộng). Dòng ngắn hơn cột bắt đầu → con trỏ rỗng ở cuối dòng (VS Code). Hướng của từng selection theo chiều kéo. Selection chính là dòng có `to`.
  - ⇧⌥⌘ + ←/→/↑/↓/PageUp/PageDown dùng trạng thái `ColumnSelectData { from, to }` (như VS Code) giữ trong `CodeView`; lần đầu `from` = con trỏ chính. Trạng thái bị xóa khi selection đổi bởi đường khác.
- `remove_secondary(set)`: còn selection chính. `cancel_selection(set)`: một selection → con trỏ ở head.
- `toggle_cursor_at(set, offset)`: ⌥-click. Có con trỏ / selection phụ chứa `offset` → bỏ nó (không bỏ selection cuối cùng); không có → thêm con trỏ, thành chính.
- Mọi hàm tạo selection cắt ở `multiCursorLimit`; lệnh trả về cờ "bị cắt" để workbench báo.
- `editor.multiCursorMergeOverlapping: false`: `from_selections(…, merge)` đã có tham số, nhưng `remap`, `clear_selection` và các chỗ gọi khác đang truyền cứng `true`. Setting phải tới mọi chỗ đó (ví dụ `SelectionSet` giữ cờ merge, hoặc truyền qua `EditConfig`; builder chọn).

**Clipboard.** Sửa `copy` / `cut` theo VS Code: chỉ khi **mọi** selection rỗng mới lấy cả dòng; có selection không rỗng thì selection rỗng góp chuỗi rỗng. Cần cho chọn cột qua dòng ngắn và `y` của `<C-v>`.

**Paste.** `paste` nhận `multi_cursor_paste: Spread | Full`. `Spread` (mặc định) giữ hành vi hiện có: chia theo `pieces` khi số mảnh bằng số selection; thêm trường hợp VS Code: clipboard từ ngoài (không có `pieces`) mà số dòng bằng số selection thì chia theo dòng. `Full`: mỗi selection nhận cả chữ.

### 5.2 `lucerna-workbench`

**Bỏ `block_selection`.** `set_block_selection` thay bằng một hàm đặt `SelectionSet` nhiều dòng (dùng `column_select` hoặc trực tiếp từ `block_ranges` của Vim). Xóa đường vẽ khối trong `element/paint.rs`, nhánh khối trong `copy_selection`, `clipboard_text`, phần remap khi sửa. `visible_selections` vẽ mọi selection như nhau. Vim: `sync_view` đặt nhiều selection cho `VimSelection::Block`, giữ con trỏ khối của Vim ở đúng chỗ (overlay), và chỗ `has_block_selection()` ở `vim.rs:604` (lúc tắt Vim) đổi thành "Vim đang ở Visual Block", để con trỏ người dùng tự thêm không bị gộp khi tắt Vim.

**Vim bật (người dùng chọn cách A).** Hiện mỗi phím Vim gọi `sync_view` và gộp về một con trỏ, nên con trỏ phụ mất ngay ở phím kế. Quy tắc mới:
- Khi Vim ở Normal hoặc Visual (không phải `<C-v>`), mọi lệnh tạo selection của bước này (⌘D, ⌘K ⌘D, ⇧⌘L, ⌘F2, ⌥⌘↑/↓, ⇧⌥I, chọn cột bằng phím, ⌥-click, ⌥-kéo, ⇧⌥-kéo) chạy như khi tắt Vim rồi **đưa Vim vào Insert**, giữ nguyên `SelectionSet` vừa tạo. Gõ chữ thay chữ chọn / chèn ở mọi con trỏ qua đường gõ của editor; cả lần Insert là một bước undo như hiện có. Status bar hiện `-- INSERT --`, con trỏ là thanh.
- Trong Insert, các lệnh trên chạy bình thường, Vim không gộp selection.
- Escape trong Insert: gộp về selection chính rồi về Normal như hiện nay (con trỏ lùi một ký tự). Một lần Escape là đủ, không cần hai bước như khi tắt Vim.
- `<C-v>` vẫn là Visual Block của Vim (nhiều selection, chỉ `y`), không vào Insert.
- `.` và macro: lần Insert bắt đầu từ lệnh multi-cursor được ghi như phím Insert thường; lệnh multi-cursor không được ghi lại vào `.` / macro ở bước này (hạn chế ghi trong doc, sửa ở bước Vim sau).

**Vẽ.** Đã vẽ mọi selection và mọi caret. Thêm: caret phụ cùng màu caret chính (Zed); con trỏ nhấp nháy đồng bộ. Không đổi gì khác về giao diện.

**Chuột** (`element.rs` `mouse_down` / `mouse_drag`):
- Phím thêm con trỏ = ⌥ khi `multiCursorModifier: alt`, ⌘ khi `ctrlCmd`. Phím còn lại một mình làm việc "đi tới định nghĩa" (⌘-click hiện nay). ⌘⌥-click giữ nghĩa "đi tới định nghĩa, mở bên cạnh" ở cả hai chế độ (như VS Code), không thêm con trỏ.
- Phím thêm con trỏ + click → `toggle_cursor_at`. + kéo → thêm một selection mới đi theo chuột (các selection khác giữ nguyên). Double/triple-click cùng phím → thêm một từ / dòng.
- ⇧⌥-kéo (luôn ⇧⌥, như VS Code) → `column_select` từ chỗ bấm tới chuột.
- Click thường vẫn thay mọi selection bằng một.

**Lệnh và phím** (thêm vào `default-keybindings.json`, `when` như VS Code):

| Lệnh | Phím |
|---|---|
| `editor.action.addSelectionToNextFindMatch` | ⌘D (`editorFocus`) |
| `editor.action.moveSelectionToNextFindMatch` | ⌘K ⌘D |
| `editor.action.addSelectionToPreviousFindMatch`, `editor.action.moveSelectionToPreviousFindMatch` | — (có trong command palette) |
| `editor.action.selectHighlights` | ⇧⌘L |
| `editor.action.changeAll` | ⌘F2 (`editorTextFocus && !editorReadonly`) |
| `editor.action.insertCursorAbove` / `insertCursorBelow` | ⌥⌘↑ / ⌥⌘↓ |
| `editor.action.insertCursorAtEndOfEachLineSelected` | ⇧⌥I |
| `cursorColumnSelectLeft/Right/Up/Down/PageUp/PageDown` | ⇧⌥⌘ + ← → ↑ ↓ PageUp PageDown |
| `removeSecondaryCursors` | Escape (`editorHasMultipleSelections && textInputFocus`) |
| `cancelSelection` | Escape (`editorHasSelection && textInputFocus`) |

Context key mới: `editorHasMultipleSelections`, `editorHasSelection`. Escape của Find / hover / Vim giữ thứ tự ưu tiên hiện có (Find đang mở thì Escape đóng Find trước, như VS Code). Khi Vim bật, Escape đi vào Vim như hiện nay; Vim Normal chỉ có một con trỏ nên không đụng.

⌘D chỉ là lệnh editor khi editor có focus; `filesExplorer.duplicate` vẫn giữ ⌘D trong Explorer.

**Lịch sử con trỏ.** Mọi lệnh ở đây đổi selection qua đường hiện có, nên ⌘U (`cursorUndo`) hoàn tác được ⌘D, ⌥-click…, như VS Code.

**Cuộn.** Sau ⌘D / ⌘K ⌘D / thêm con trỏ trên-dưới: cuộn tới selection mới nhất nếu nó ngoài màn hình (như `reveal` hiện có). ⇧⌘L không cuộn.

**Giới hạn.** Vượt `multiCursorLimit` → giữ đúng giới hạn và hiện thông báo status bar `The number of cursors has been limited to N.` (chữ VS Code).

**Status bar.** `DocumentStatus.cursor` (documents.rs) tính như §2. Đếm ký tự theo char, chạy mỗi frame nên chỉ đếm khi selection đổi (cache theo `selection_generation`), không copy chữ.

### 5.3 Setting

Thêm vào `default-settings.json` và đọc vào `EditConfig` / config view:

| Setting | Mặc định | Giá trị |
|---|---|---|
| `editor.multiCursorModifier` | `"alt"` | `"alt"`, `"ctrlCmd"` |
| `editor.multiCursorMergeOverlapping` | `true` | bool |
| `editor.multiCursorPaste` | `"spread"` | `"spread"`, `"full"` |
| `editor.multiCursorLimit` | `10000` | số ≥ 1 |

`editor.selectedTextMatchMode` (spec cha §14.2 và nghiên cứu có ghi) bị bỏ: không xác nhận được setting này có trong VS Code. Hành vi luôn là "lấy tùy chọn của thanh Find" như mô tả ở §5.1. *(Quyết định của Claude.)*

## 6. Kiểm thử

- `lucerna-editor`: unit test cho mỗi hàm ở §5.1 bằng `marked` (chuỗi có đánh dấu selection) như các module 4.2: quay vòng, bỏ chồng, chữ khác nhau, dòng ngắn, tab, Unicode nhiều byte, giới hạn, copy/cut/paste trộn rỗng và không rỗng, paste dòng ngoài.
- `lucerna-workbench` (gpui test): ⌘D ba lần rồi gõ; ⌥-click thêm rồi bỏ; ⇧⌥-kéo qua dòng ngắn rồi gõ; Escape hai bước; Vim bật: ⌘D ⌘D vào Insert, gõ ở hai chỗ, Escape về Normal một con trỏ, `u` hoàn tác cả lần gõ; `multiCursorModifier: ctrlCmd` đổi vai trò ⌥/⌘-click; ⌘U sau ⌘D; Vim `<C-v>` chọn rồi `y` cho cùng chữ như trước (test hiện có phải qua không sửa kết quả mong đợi).
- Hiệu năng: đo gõ với 1000 con trỏ và ⇧⌘L trên file lớn trong `editor-benchmark`, ghi vào `docs/perf-checklist.md`.

## 7. Để builder quyết

- Ngưỡng chạy nền cho ⇧⌘L, cách cache số ký tự ở status bar.
- Chia plan thành task (gợi ý: find dùng chung → multicursor thuần → clipboard/paste → bỏ `block_selection` + Vim → chuột → lệnh/phím/setting → status bar → đo).
- Tên hàm, cấu trúc module trong `lucerna-editor`.

## 8. Sau khi giao

Cập nhật bảng roadmap §1.3 (4.3 xong), dòng trạng thái của spec cha, README nếu có liệt kê tính năng; sửa câu "Aho-Corasick" ở §8 spec cha cho khớp §4, bỏ `editor.selectedTextMatchMode` khỏi §8 và §14.2 spec cha và ghi chú vào `docs/research/2026-11-11-editing.md` §3.
