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

## 5. Tìm trong file (find widget)

- **VS Code**: find widget (`actions.find`, `cmd-f`) là một overlay nổi ở góc trên bên phải editor, không đẩy nội dung xuống mà che một phần text. Có ô nhập với toggle case / whole word / regex, bộ đếm "3 of 19", nút trước/sau, "find in selection", nút đóng, và vùng replace gập được.
- **Zed**: `BufferSearchBar` là một toolbar item của pane: một hàng full-width nằm dưới breadcrumb, đẩy nội dung editor xuống. Trong hàng có ô query với các toggle Aa / wd / .* nằm trong ô, nút select-all và replace, nút trước/sau, bộ đếm "3/19" và nút đóng. Lỗi regex hiện ngay trên thanh.
- **Lucerna chọn**: giao diện theo Zed (thanh full-width dưới breadcrumb, đẩy nội dung xuống, phủ cả minimap), phím tắt và hành vi theo VS Code (`cmd-f`, `enter` / `shift-enter`, `escape`, các toggle). Chỉ học thiết kế, không chép code Zed (GPL).

## 6. Chi tiết giao diện tab Search và thanh tìm (Zed, revision `b5cd310`)

Đọc ở `crates/search/src/{buffer_search,project_search,search_bar}.rs` và `crates/editor/src/element/header.rs`. Chỉ học thiết kế, không chép code (GPL).

- **Toolbar.** Cả `BufferSearchBar` lẫn `ProjectSearchBar` dùng chung một khung ô query (`input_base_styles`): `min_w_32`, `min_h_8`, viền 1 px, `rounded_md`, các toggle Aa / wd / .* nằm trong ô. Khi regex lỗi, viền ô đổi sang màu `Error`; khi không có kết quả thì chữ query đổi màu `Error`. Sau ô là cụm điều khiển (`min_w_64`), rồi cụm `‹ ›` và bộ đếm, ngăn bằng `border_l_1` màu `border_variant`. Bộ đếm là `"{i}/{n}"`, hoặc `"0/0"` khi chưa có match nào được chọn; nhãn cỡ `Small`. Nút đóng của thanh tìm trong file nằm riêng ở mép phải, sau một vạch `border_variant`. Ở tab Project Search, nút gập/mở hết (`ChevronDownUp` / `ChevronUpDown`) đứng đầu hàng; nút filter là `IconButton` vuông.
- **Header file trong multibuffer.** Cao `FILE_HEADER_HEIGHT = 2` dòng editor, padding `BUFFER_HEADER_PADDING` (0.25 rem). Bên trong là một thẻ `rounded_sm`, `border_1` màu `border`, nền `editor_subheader_background`, hover `element_hover`, chiếm cả hàng. Thẻ có nút chevron gập file, icon file mờ, tên file (font buffer), thư mục mờ cắt đầu, và nút "Open File" bên phải chỉ hiện khi hover hoặc khi file đang được chọn.
- **Header dính (sticky).** Khi cuộn vào giữa một file, Zed vẽ lại header của file đó đè lên đầu vùng editor, nền gradient từ trong suốt sang `editor_background`, có `shadow_md`. Header file kế tiếp tới gần thì đẩy header dính lên trên (dịch `origin.y` theo phần chồng lấn).
- **Trạng thái trống.** Tab chưa tìm hiện ở giữa tiêu đề "Search All Files" cỡ lớn, dòng mờ "Start typing to search. For more options:", rồi các nút có icon, nhãn và phím tắt: include/exclude, find and replace, match with regex, match case, match whole words.

**Lucerna lấy gì:**

- Một component chung trong `ui.rs` cho cả thanh tìm trong file lẫn tab Search: `search_toolbar` (nền `editor.background`, vạch dưới `border.variant`), `search_bar_row` (cao 40 px, lề ngang 8 px), `search_query_box` (ô 28 px, xin khoảng 2/3 hàng qua `flex_basis`, tối thiểu 128 px, viền màu `error` khi không có kết quả hoặc regex lỗi), `bar_separator`, và `match_nav` (`‹ ›` + bộ đếm). Lucerna đổi viền sang màu lỗi cả khi không có kết quả, thay vì đổi màu chữ như Zed, để hai thanh dùng chung một tín hiệu.
- Bộ đếm theo Zed: `"3/19"`, `"0/0"` khi không có kết quả. Riêng tab Search, khi có kết quả mà chưa chọn dòng nào thì bộ đếm ghi `"19 results"` thay cho `"0/19"`, vì dòng tóm tắt "N results in M files" đã bỏ khi search xong.
- Header file theo Zed: cao 2 dòng, padding 4 px, thẻ `rounded_sm` viền `border` trên nền `editor.subheader.background` (key theme mới, fallback như `element.background`), hover `ghost_element.hover`, nút mở file (icon `arrow-up-right`) hiện khi hover hoặc khi file chứa dòng đang chọn.
- Header dính theo Zed, có hiệu ứng bị header kế tiếp đẩy lên. Nền dùng màu `editor.background` đặc thay cho gradient.
- Ranh giới giữa hai excerpt là một vạch 1 px `border.variant` ngang vùng code, cao 1 dòng, thay cho ký tự trong gutter.
- Trạng thái trống theo Zed, bỏ dòng "Find and replace" vì Lucerna chỉ để xem code. Phím tắt lấy từ keymap (`Keymap::label_for`).
- Danh sách kết quả chuyển từ `uniform_list` sang `list` của gpui (`ListState`) để header cao gấp đôi dòng. Chiều cao mỗi hàng biết trước theo loại hàng, nên việc cuộn tới một hàng và tính các hàng đang hiện được tính chính xác cả khi list chưa đo các hàng đó.
