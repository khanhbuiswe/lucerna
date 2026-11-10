# Project Search dạng tab (thay Search panel ở sidebar)

Ngày 2026-11-10. Trạng thái: chờ duyệt spec.

## 1. Mục tiêu

Người dùng yêu cầu: "làm phần search topbar như zed thay vì search panel, UI ưu tiên theo zed".

Đã làm rõ với người dùng:
- Zed không có ô search trên title bar. Thứ người dùng muốn là **Project Search của Zed: một tab trong editor**, thanh search gọn ở đầu tab, kết quả là các đoạn code (excerpt) có ngữ cảnh và tô màu cú pháp.
- **Bỏ Search panel ở sidebar.** Sidebar chỉ còn Explorer.
- **Giữ nút kính lúp ở status bar**, nhưng nó mở tab Search (như `SearchButton` của Zed).
- Kết quả: **excerpt + ngữ cảnh + tô màu cú pháp**.
- Hướng kỹ thuật đã duyệt: **A** (mục 3).

Thành công khi: `⇧⌘F` mở tab Search, gõ là ra kết quả gom theo file với ngữ cảnh và màu cú pháp, bấm một dòng thì nhảy tới chỗ khớp; sidebar không còn Search; không có ảnh hưởng tới độ mượt khi cuộn hàng chục nghìn kết quả.

Ngoài phạm vi: replace, sửa trực tiếp trong kết quả, nhiều tab Search cùng lúc, popup Text Finder (`⌥⌘F`), ô search trên title bar.

## 2. Tham khảo

Chi tiết ở `docs/research/2026-11-10-project-search-ui.md`. Tóm tắt:
- **Zed:** `ProjectSearchView` là một `Item` (tab). `ProjectSearchBar` là toolbar ở đầu tab: query, toggle case / word / regex / include ignored, nút filter (`cmd-shift-j`) hiện include/exclude. Kết quả là multibuffer, có ngữ cảnh, sửa được. `escape` đổi focus giữa query và kết quả. Status bar có nút kính lúp mở search.
- **VS Code:** Search view ở sidebar; Search Editor là phiên bản dạng tab, có setting `search.searchEditor.defaultNumberOfContextLines` (mặc định 1). Search chạy khi gõ (`search.searchOnType`). Phím: `alt+cmd+c/w/r` cho toggle, `shift+cmd+j` cho chi tiết query, `F4`/`shift+F4` cho kết quả kế/trước.
- **Lucerna chọn:** giao diện theo Zed (tab, thanh search gọn, excerpt có màu). Lệnh, phím, setting và hành vi (search khi gõ, debounce) theo VS Code. Lệch khỏi cả hai: excerpt chỉ đọc (Lucerna là viewer); chỉ một tab Search.

## 3. Kiến trúc

### 3.1 `lucerna-project::search`: trả về excerpt
- `SearchQuery` thêm `context_lines: u32`.
- `SearchEvent::File` đổi thành `{ path, excerpts: Vec<Excerpt> }`, trong đó:
  - `Excerpt { lines: Vec<LineMatch> }`, các dòng liên tiếp (dùng lại `LineMatch { line_number, line_text, ranges }` có sẵn);
  - dòng có `ranges` rỗng là dòng ngữ cảnh.
- Dùng `before_context`/`after_context` của `grep-searcher`. Hai cụm khớp có ngữ cảnh chồng hoặc kề nhau thì gộp thành một excerpt.
- `MAX_RESULTS` (20.000) vẫn đếm **dòng khớp**, không đếm dòng ngữ cảnh. `MAX_LINE_CHARS` áp cho cả dòng ngữ cảnh.
- `build_pattern`, hủy qua `AtomicBool`, stream theo file: giữ nguyên.

### 3.2 Model tab
- `TabItem::Search`: tối đa một trong mọi group, không bao giờ là preview, giống `TabItem::Extensions`.
- Lưu trong `GroupState::tabs` bằng chuỗi `lucerna:search`. Query và các option (case, word, regex, include, exclude, filter hiện hay ẩn) lưu trong workspace state; mở lại folder thì tab khôi phục và search lại.
- Workspace state cũ có `sidebar_view = "search"` thì đọc thành Explorer.

### 3.3 `search_view/` (thay `search_panel.rs`)
Module mới trong `lucerna-workbench`, chia nhỏ:
- `model.rs` — thuần, không gpui: danh sách file kết quả, cờ gập, dựng **danh sách hàng phẳng** (`Header(file)`, `Line(file, excerpt, line)`, `Separator(file, excerpt)`), tìm hàng khớp kế/trước cho F4, tóm tắt.
- `highlight.rs` — tải highlight nền (3.4).
- `view.rs` — entity `SearchView`: các `TextInput`, toggle, debounce 150 ms, generation guard (R17), `uniform_list`, mở kết quả.
- Một entity `SearchView` duy nhất cho workbench (như Extensions page); tab Search render nó.

### 3.4 Tô màu cú pháp chạy nền
- Khi `uniform_list` render một khoảng hàng, view biết những file nào đang hiện. File nào chưa có highlight và chưa được yêu cầu thì đưa vào hàng đợi.
- Tối đa **3** tác vụ nền cùng lúc. Mỗi tác vụ: đọc file (bỏ qua nếu vượt `check_size` hoặc lớn hơn 2 MB), dựng `BufferSnapshot`, chọn ngôn ngữ (`language_for_file`), `SyntaxState::new` + `parse(None)`, rồi gọi `highlights` cho các khoảng dòng của excerpt.
- Kết quả: với mỗi dòng excerpt, danh sách `(Range<usize> trong dòng, capture)`. Span được cắt theo biên dòng và theo `MAX_LINE_CHARS`.
- **Chống lệch:** nếu text dòng trên đĩa khác `LineMatch::line_text` (file đã đổi) thì dòng đó không có highlight.
- Kết quả gắn với generation của search; search mới thì bỏ hàng đợi và các kết quả cũ. Lỗi đọc/parse hoặc không có ngôn ngữ: dòng hiện chữ trơn, không báo lỗi.
- Nếu file đang mở trong `Documents` thì vẫn tự parse (đơn giản hơn; tối ưu để sau).

### 3.5 Sidebar và status bar
- Bỏ `SidebarView::Search` và `search_panel.rs`. Sidebar chỉ còn Explorer; `cmd+b`, `cmd+shift+e` giữ nguyên.
- Nút kính lúp ở status bar chạy `workbench.action.findInFiles`, sáng khi tab Search đang active trong group có focus.

## 4. Giao diện (theo Zed)

- **Tab:** icon kính lúp + query (rỗng thì "Search").
- **Thanh search** ở đầu tab, cao 32 px, padding theo lưới 4 px:
  - ô query `TextInput` (IME), placeholder "Search";
  - trong ô, bên phải: toggle `Aa`, `ab`, `.*` (dùng `toggle_button` có sẵn);
  - sau ô: vạch ngăn, nút filter (icon), nút refresh, nút gập/mở tất cả. Các nút dùng `icon_button`.
- **Hàng filter** (ẩn mặc định): hai ô Include và Exclude cạnh nhau, placeholder như panel cũ.
- **Dòng tóm tắt** cỡ chữ nhỏ, màu `text.muted`: `12 results in 2 files`, "Searching…", báo đã cắt ở 20.000, hoặc lỗi regex.
- **Header file** (cao bằng một dòng editor, nền `element.background`/nhạt): chevron gập, icon file theo bảng icon hiện có, tên file màu `text`, thư mục màu `text.muted`, số dòng khớp ở bên phải.
- **Dòng code:** font và cỡ chữ editor, gutter số dòng màu `text.muted` căn phải, chữ tô theo theme (`style_for(capture)`), chỗ khớp có nền `search.match_background`. Dòng đang chọn (bàn phím) có nền `ghost_element.hover`/selected.
- **Ngăn excerpt:** một hàng chỉ có `┈` mờ trong gutter.
- Mọi hàng cao bằng nhau (chiều cao dòng editor) để dùng `uniform_list`.
- Single-file mode: tab chỉ hiện "Search is unavailable when a single file is opened".

## 5. Lệnh, phím tắt và setting (theo VS Code)

| Lệnh | Phím mặc định | Hành vi |
|---|---|---|
| `workbench.action.findInFiles` | `cmd+shift+f` | Mở/focus tab Search ở group đang active (đã có thì focus nó ở group đang chứa), focus ô query; nếu editor có chọn trên một dòng thì điền phần chọn. |
| `workbench.view.search` | — | Như trên. |
| `toggleSearchCaseSensitive` | `alt+cmd+c` (when `searchViewletFocus`) | Đổi case. |
| `toggleSearchWholeWord` | `alt+cmd+w` (when `searchViewletFocus`) | Đổi whole word. |
| `toggleSearchRegex` | `alt+cmd+r` (when `searchViewletFocus`) | Đổi regex. |
| `workbench.action.search.toggleQueryDetails` | `shift+cmd+j` (when `searchViewletFocus`) | Ẩn/hiện hàng filter. |
| `search.action.focusNextSearchResult` | `f4` | Chọn và mở dòng khớp kế tiếp (vòng lại đầu). |
| `search.action.focusPreviousSearchResult` | `shift+f4` | Như trên, ngược lại. |
| `search.action.refreshSearchResults` | — | Chạy lại search. |
| `search.action.collapseSearchResults` | — | Gập tất cả file. |

- Context key `searchViewletFocus` (tên VS Code, đã có) nay nghĩa là: focus nằm trong tab Search.
- Trong tab: search chạy khi gõ (debounce 150 ms) và khi nhấn `Enter` ở ô query; `↑`/`↓` đổi dòng chọn; `Enter` ở danh sách mở dòng chọn; `Esc` đổi focus giữa ô query và danh sách.
- Bấm một dòng: mở file kiểu preview (`OpenMode::Preview`), cuộn tới dòng, bôi chọn chỗ khớp đầu tiên của dòng (tính cột bằng cách khớp lại trên dòng đã giải mã, như panel cũ). Bấm dòng ngữ cảnh: mở ở dòng đó, không bôi chọn. Bấm header: gập/mở.
- Setting mới: `search.searchEditor.defaultNumberOfContextLines` (số, mặc định 1, kẹp 0–5). `search.exclude` và `files.exclude` vẫn áp dụng như cũ.

## 6. Hiệu năng

- Luồng UI không đọc file hay parse: search và highlight đều ở nền.
- `uniform_list` chỉ render các hàng thấy được; danh sách hàng dựng lại khi file mới tới hoặc gập/mở, không theo từng frame.
- Highlight chỉ cho file đang hiện, tối đa 3 cùng lúc, có hủy khi search mới.

## 7. Kiểm thử

- `lucerna-project`: gộp ngữ cảnh thành excerpt (chồng nhau, kề nhau, đầu file, cuối file, `context_lines = 0`), giới hạn `MAX_RESULTS` chỉ đếm dòng khớp.
- `search_view::model`: dựng hàng (có gập), F4 qua nhiều file và vòng lại, tóm tắt.
- `search_view::highlight`: cắt span theo dòng, bỏ dòng có text lệch.
- `editor_groups::model`: một tab Search duy nhất, lưu/khôi phục `lucerna:search`; state cũ `sidebar_view = "search"` thành Explorer.
- Test GPUI: `cmd+shift+f` mở tab và điền phần chọn; gõ query ra kết quả; bấm dòng mở file ở đúng dòng; nút kính lúp mở tab; sidebar không còn Search; toggle phím `alt+cmd+c`.

## 8. Thay đổi spec cũ

Mục 12 của spec Phase 1 ("Nút đổi panel ở status bar … Explorer và Search") được thay: status bar có nút Explorer (bật/tắt sidebar) và nút kính lúp (mở tab Search).
