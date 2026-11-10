# Nghiên cứu: giao diện tìm kiếm toàn project (Zed và VS Code)

Ngày 2026-11-10. Phục vụ yêu cầu "làm phần search topbar như zed thay vì search panel, UI ưu tiên theo zed".

Nguồn Zed: bản GPUI mà Lucerna đang pin (`~/.cargo/git/checkouts/zed-*/b5cd310`, commit ngày 2026-11-07), các crate `search`, `title_bar`, `picker`, file `assets/keymaps/default-macos.json` và `docs/src/finding-navigating.md`. Các crate này là GPL, nên chỉ học thiết kế.

## 1. Zed

### 1.1 Title bar không có ô tìm kiếm
`crates/title_bar/src/title_bar.rs` chỉ render: menu ứng dụng (Linux/Windows), restricted mode, host remote, tên project (bấm vào mở popover recent projects), worktree và nhánh git (bấm vào mở branch picker), danh sách cộng tác, trạng thái kết nối, nút cập nhật, nút đăng nhập và menu người dùng. Không có ô search hay command center nào.

### 1.2 Project Search là một tab (`pane::DeploySearch`, `cmd-shift-f`)
- `ProjectSearchView` implement `Item`, nên mở thành **tab trong editor pane**, không phải panel ở sidebar. Tiêu đề tab là query.
- Thanh search gọn (`ProjectSearchBar`, là `ToolbarItemView`) nằm trên đầu tab: ô query, các nút toggle (case, whole word, regex, include ignored), nút replace, nút filter (`cmd-shift-j`) để hiện include/exclude.
- Kết quả hiện trong một **multibuffer**: mỗi file là một excerpt có vài dòng ngữ cảnh, sửa trực tiếp được. Nhấn Enter mới chạy search.
- Trong tab: `escape` chuyển focus giữa ô query và kết quả, `cmd-shift-f` focus lại ô search, `cmd-shift-enter` mở/gập tất cả.
- Status bar có nút kính lúp (`SearchButton`, ẩn được bằng `search.button`) để mở Project Search.

### 1.3 Text Finder: popup tìm text (`project_search::OpenTextFinder`, `alt-cmd-f`)
Tính năng mới của Zed (`crates/search/src/text_finder*`). Docs mô tả: tìm nhanh một chuỗi trong cả project rồi mở file; muốn lọc kỹ hơn thì chuyển sang Project Search.
- Là **modal picker nổi ở trên giữa cửa sổ**, giống file finder. Placeholder `"Search all files…"`.
- Bên phải ô nhập có các nút toggle case / whole word / regex / include ignored, một vạch ngăn, và chỉ báo đang quét project.
- Danh sách kết quả **gom theo file**: dòng header có icon file, tên file, thư mục mờ; header gập được (`text_finder::Fold`/`Unfold`/`ToggleFoldAll`). Dưới header là từng dòng khớp, chữ khớp được tô.
- Picker có **preview** (`try_get_preview_data_for_match`): khi bật preview, picker phình to sang khổ "telescope" và hiện file quanh dòng khớp ở bên phải hoặc bên dưới.
- Menu Actions ở góc phải dưới: Split trái/phải/trên/dưới, Open File (hoặc Open Multiple khi chọn nhiều), **Open as Tab** (`ToProjectSearch`) chuyển query sang Project Search.
- Nhớ query và option lần cuối theo từng workspace (lưu trong DB).
- Kết quả stream vào picker trong lúc search chạy, và giới hạn số match trên một dòng quá dài.

## 2. VS Code

- **Search view** (`workbench.view.search`, `workbench.action.findInFiles`, `cmd-shift-f`): mặc định là view trong sidebar; `search.location` cũ cho phép đặt ở panel. Có thể kéo sang panel/secondary sidebar. Có "Open in Editor" (`search.action.openNewEditor`) tạo **Search Editor** dạng tab text.
- **Command Center** (`window.commandCenter`, mặc định bật từ 1.77): ô giả ở giữa title bar hiện tên workspace và kính lúp. Bấm vào mở **Quick Open** (`workbench.action.quickOpen`) ở chế độ chọn file, gợi ý các prefix (`>` lệnh, `:` dòng, `@` symbol, `%` text…). Hai bên có nút back/forward. Nó không tự tìm text.
- **Quick Search** (`workbench.action.quickTextSearch`, prefix `%` trong Quick Open, từ 1.83): tìm text trong Quick Open, kết quả từng dòng có tên file, có nút mở sang Search view. Không có phím tắt mặc định.

## 3. So sánh

| | Zed | VS Code |
|---|---|---|
| Ô search trên title bar | Không | Có (Command Center → Quick Open) |
| Tìm text nhanh dạng popup | Text Finder, `alt-cmd-f`, gom theo file, có preview | Quick Search, `%`, danh sách phẳng |
| Tìm text đầy đủ | Tab Project Search (multibuffer) | Search view ở sidebar (+ Search Editor tab) |
| `cmd-shift-f` | Mở tab Project Search | Mở Search view ở sidebar |

## 4. Lucerna chọn gì

Chờ người dùng chọn (xem spec). Theo nguyên tắc: giao diện theo Zed, lệnh/phím tắt theo VS Code (`workbench.action.findInFiles` = `cmd-shift-f`, `workbench.action.quickTextSearch`).
