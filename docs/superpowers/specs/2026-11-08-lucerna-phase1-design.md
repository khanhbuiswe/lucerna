# Lucerna — Giai đoạn 1: Trình xem lõi

- **Ngày:** 2026-11-08
- **Trạng thái:** Bản nháp chờ duyệt
- **Tên sản phẩm:** Lucerna (Latin: "ngọn đèn")
- **Lệnh CLI:** `luc`
- **Crate gốc:** `lucerna`

## 1. Bối cảnh và mục tiêu

### 1.1 Sản phẩm là gì

Lucerna là một bản VS Code gọn hơn và nhanh hơn, viết bằng Rust, phát hành mã nguồn mở với giấy phép thoáng (MIT OR Apache-2.0).

Người dùng mục tiêu viết code chủ yếu qua các agent chạy trong terminal, như Claude Code hay Codex. Họ mở editor phần lớn để **xem**: duyệt project, đọc code, theo dõi những gì agent vừa thay đổi. Những ai muốn code trực tiếp trên Lucerna vẫn sửa được đầy đủ, nhưng phần sửa thuộc giai đoạn sau.

### 1.2 Những gì đã quyết định

- **Không có tính năng AI** trong editor: không chat, không ghost text, không MCP.
- **Trải nghiệm giống VS Code:** bố cục, Quick Open, `settings.json`, `keybindings.json`, phím tắt mặc định.
- **Extension dùng định dạng của Zed** (WASM, tree-sitter, theme JSON), không dùng định dạng VS Code. Phần này thuộc giai đoạn 2.
- **UI viết bằng GPUI** (Apache-2.0). Mọi phần còn lại tự viết, không phụ thuộc crate GPL nào của Zed.

### 1.3 Lộ trình tổng thể

Spec này **chỉ** mô tả giai đoạn 1. Mỗi giai đoạn sau có spec riêng.

| # | Giai đoạn | Kết quả |
|---|---|---|
| **1** | **Trình xem lõi** (spec này) | Thay được VS Code cho việc xem code |
| 2 | Host extension Zed | Theme, icon theme, ngôn ngữ và grammar, snippet từ extension của Zed |
| 3 | Hiểu code qua LSP | Định nghĩa, tham chiếu, hover, diagnostics, outline, breadcrumbs, Quick Open `@` và `#` |
| 4 | Sửa code đầy đủ | Multi-cursor, undo, xử lý xung đột với agent, completion, rename, format |
| 5 | Mở rộng | Git gutter, xem diff, terminal tích hợp, giao diện settings |

### 1.4 Tiêu chí thành công của giai đoạn 1

Giai đoạn 1 xong khi tác giả dùng Lucerna thay VS Code để đọc code hằng ngày trong một tuần mà không phải mở lại VS Code để xem code, và đạt các mục tiêu hiệu năng ở mục 6.

## 2. Phạm vi

### 2.1 Có trong giai đoạn 1

- Chạy `luc [đường-dẫn]` để mở một thư mục hoặc một file. Mỗi lần chạy mở một cửa sổ.
- **Bố cục:**
  - Activity bar có hai mục, Explorer và Search.
  - Sidebar có thể ẩn hoặc hiện bằng Cmd+B.
  - Vùng editor chia được thành nhiều ô (Cmd+\), mỗi ô có tab.
  - Status bar.
- **Explorer:**
  - Cây thư mục tôn trọng `.gitignore` và `files.exclude`.
  - Tự cập nhật khi file trên đĩa thay đổi.
  - Đánh dấu file vừa bị thay đổi từ bên ngoài, tắt được qua `explorer.decorations.externalChanges`.
  - Tự làm nổi bật file đang mở, theo `explorer.autoReveal`.
- **Tab:**
  - Preview tab: chữ nghiêng, bị thay bởi file mở tiếp theo. Double-click để giữ lại tab.
  - Cmd+W đóng tab.
  - Ctrl+Tab chuyển qua lại giữa các tab gần đây.
- **Khung xem code:**
  - Chỉ đọc.
  - Số dòng, tô màu cú pháp, cuộn mượt, word wrap bật tắt được.
  - Chọn văn bản và copy (Cmd+C).
  - Tìm trong file (Cmd+F), có tùy chọn phân biệt hoa thường, cả từ và regex.
  - Minimap, tắt được qua `editor.minimap.enabled`.
  - Folding theo thụt lề.
  - Tô sáng cặp ngoặc tương ứng.
- **Quick Open:**
  - Một widget duy nhất.
  - Không tiền tố: tìm file (Cmd+P).
  - `>`: chạy lệnh (Cmd+Shift+P).
  - `:`: nhảy tới dòng (Ctrl+G).
- **Search** (Cmd+Shift+F):
  - Tìm trong toàn project, có tùy chọn phân biệt hoa thường, cả từ, regex, và glob include/exclude.
  - Kết quả hiển thị trong sidebar, nhóm theo file.
- **Settings và keybindings:**
  - `settings.json` và `keybindings.json` theo ngữ nghĩa của VS Code.
  - Lệnh mở hai file này có trong command palette.
- **Theme:**
  - Đọc theme JSON theo schema Zed v0.2.0.
  - Đi kèm One Dark và One Light (MIT, lấy từ thư mục `assets/themes` của Zed, kèm file license).
- **Ngôn ngữ:**
  - Đi kèm sẵn grammar tree-sitter cho Rust, TypeScript, TSX, JavaScript, Python, Go, JSON, Markdown (gồm cả inline), TOML, YAML, Bash, HTML và CSS.
  - Query lấy từ repo gốc của từng grammar.
- **Lưu trạng thái workspace:** bố cục, kích thước các ô, tab đang mở, vị trí cuộn.

### 2.2 Không có trong giai đoạn 1

Sửa văn bản, LSP, extension, git, terminal, debug, giao diện chỉnh settings, đa nền tảng, remote, mở nhiều thư mục gốc trong một cửa sổ, auto-update, telemetry (Lucerna không bao giờ có telemetry).

### 2.3 Nền tảng

Chỉ macOS 13 trở lên, chạy trên Apple Silicon và Intel. Linux và Windows làm sau giai đoạn 1.

## 3. Kiến trúc

### 3.1 Nguyên tắc

1. **Luồng UI không bao giờ chờ** I/O, parse hay tìm kiếm. Việc nặng chạy ở luồng nền trên snapshot bất biến, rồi gửi kết quả về UI.
2. **Chỉ crate `workbench` phụ thuộc GPUI.** Các crate còn lại là Rust thuần, kiểm thử được mà không cần cửa sổ.
3. **Không dùng code GPL.** Chỉ đọc định dạng dữ liệu (theme JSON, file `.scm`, cấu trúc keymap) như một interface, không chép mã nguồn GPL của Zed. CI chặn điều này bằng `cargo-deny`.
4. **Viết sẵn để sau này sửa được.** `text` dùng rope ngay từ đầu, dù giai đoạn 1 chỉ đọc. Nhờ vậy giai đoạn 4 không phải đổi cấu trúc dữ liệu.

### 3.2 Các crate

Tất cả nằm trong một Cargo workspace, thư mục `crates/`.

| Crate | Trách nhiệm | Phụ thuộc chính |
|---|---|---|
| `lucerna-text` | Buffer rope, snapshot rẻ (clone bằng `Arc`), chuyển đổi dòng/cột ↔ byte, nhận diện encoding và dòng kết thúc | `ropey`, `encoding_rs` |
| `lucerna-syntax` | Đăng ký ngôn ngữ (theo phần mở rộng và tên file), parse tree-sitter ở nền, tính highlight cho một khoảng dòng, folding | `tree-sitter`, các crate grammar |
| `lucerna-project` | Quét thư mục ở nền, cây file, watcher, fuzzy tìm file, tìm nội dung | `ignore`, `notify`, `grep-searcher`, `grep-regex`, `nucleo-matcher` |
| `lucerna-settings` | Gộp các lớp settings, schema mặc định, đọc keybindings, đánh giá `when` clause | `serde`, `serde_json`, `jsonc-parser` |
| `lucerna-theme` | Đọc theme JSON của Zed, ánh xạ tên capture tree-sitter sang style | `serde`, `palette` |
| `lucerna-workbench` | Toàn bộ UI: bố cục, explorer, tab, khung xem code, Quick Open, panel search, status bar, lưu trạng thái | `gpui` (git, pin theo revision) |
| `lucerna` (bin `luc`) | Đọc tham số CLI, khởi tạo log, ghép các crate | `clap`, `tracing` |

**Ghim phiên bản GPUI.** `gpui` được lấy qua git từ repo `zed-industries/zed`, pin theo một commit cụ thể. Commit đó được chọn khi bắt đầu triển khai và ghi vào `Cargo.toml`. Chỉ cập nhật bằng một PR riêng, kèm kiểm thử lại.

### 3.3 Interface giữa các crate

- `project::Worktree` cung cấp:
  - `snapshot()`: một danh sách file bất biến.
  - Luồng sự kiện `WorktreeEvent`, gồm `Added`, `Removed`, `Modified` và `Rescanned`.
- `text::Buffer` cung cấp:
  - `load(path)`: trả về `Result<Buffer, LoadError>`.
  - `snapshot()`: trả về `BufferSnapshot`.
  - `reload(new_contents)`: trả về một `Reload` cho biết dòng nào đã đổi, dùng để giữ vị trí cuộn.
- `syntax::SyntaxMap` cung cấp:
  - `parse(snapshot)`: hàm bất đồng bộ.
  - `highlights(snapshot, line_range)`: trả về danh sách `(byte_range, capture_name)`.
- `theme::Theme::style_for(capture_name)`: trả về `HighlightStyle`, đi lùi theo tên capture (`function.method` → `function`).
- `settings::Settings::get::<T>(key, language)`: trả về giá trị đã gộp các lớp. Mỗi khi settings đổi, phát sự kiện thay đổi.
- `workbench` chỉ dùng các interface trên. Nó không biết bên trong rope hay tree-sitter hoạt động thế nào.

## 4. Luồng dữ liệu

### 4.1 Khởi động

1. `luc` đọc tham số, khởi tạo `tracing`, rồi nạp settings (mặc định → user → workspace) và theme.
2. GPUI mở cửa sổ và khôi phục trạng thái workspace từ file JSON. Nếu file không có hoặc hỏng thì dùng bố cục mặc định.
3. `project` bắt đầu quét thư mục ở nền. Cây file hiện dần theo từng đợt khoảng 16 ms trong lúc quét.
4. Tab đang active được đọc file và hiển thị ngay, chưa tô màu. Màu xuất hiện khi parse xong.
5. Các tab khác chỉ được đọc file khi người dùng chuyển sang.

### 4.2 Mở và xem file

1. Đọc file và nhận diện encoding: UTF-8 trước, sau đó kiểm tra BOM, cuối cùng đoán bằng `encoding_rs`. File nhị phân được nhận diện qua byte NUL trong 8 KB đầu.
2. Tạo `Buffer` và `BufferSnapshot`.
3. Parse tree-sitter: chờ đồng bộ tối đa 1 ms, quá thời gian thì để luồng nền làm tiếp và vẽ lại khi xong.
4. Chỉ tính highlight cho các dòng đang hiển thị, cộng thêm 100 dòng đệm trên và dưới.
5. **Ngưỡng file lớn** (đặt được trong settings):
   - File trên 20 MB hoặc trên 300.000 dòng: không tô màu, không có minimap, không folding.
   - Dòng dài hơn 10.000 ký tự: chỉ render phần đầu, kèm dấu `…`.
   - File trên 512 MB: không mở, chỉ hiện thông báo.

### 4.3 Khi file thay đổi từ bên ngoài

1. `notify` (FSEvents trên macOS) gửi sự kiện. `project` gộp các sự kiện trong 50 ms.
2. Nếu `notify` báo tràn hàng đợi, hoặc có hơn 1.000 sự kiện trong một đợt (ví dụ khi agent chạy `git checkout`), `project` quét lại toàn bộ ở nền và phát `Rescanned`.
3. Cây file được cập nhật và file vừa đổi được đánh dấu. Dấu này tự mất sau 10 giây hoặc khi người dùng mở file.
4. Với file đang mở:
   - Đọc lại file.
   - Gọi `Buffer::reload`.
   - Giữ dòng trên cùng của khung nhìn, dịch theo số dòng bị thêm hoặc bớt phía trên.
   - Parse lại.
5. File đang mở mà bị xóa: tab chuyển sang trạng thái "đã xóa" (gạch ngang tên), nội dung vẫn còn để xem. Nếu file xuất hiện lại thì tab trở về bình thường.
6. Mục tiêu: dưới 100 ms từ lúc file được ghi tới lúc màn hình cập nhật.

### 4.4 Quick Open

- **Cmd+P:**
  - Fuzzy match bằng `nucleo-matcher` trên snapshot danh sách file.
  - Chạy ở nền, trả 50 kết quả đầu tiên. Mỗi lần gõ phím thì hủy lượt match trước.
  - Thứ tự ưu tiên: file mở gần đây, sau đó điểm match, sau đó đường dẫn ngắn.
- **`>`:** liệt kê lệnh trong command registry, hiển thị kèm phím tắt.
- **`:N` hoặc `:N:C`:** nhảy tới dòng N, cột C, trong file đang mở.
- Enter mở file dưới dạng preview tab. Cmd+Enter mở file trong ô bên cạnh.

### 4.5 Tìm kiếm toàn project

- Chạy trên nhiều luồng bằng `ignore::WalkParallel` kết hợp `grep-searcher`. Tôn trọng `.gitignore`, `search.exclude` và glob nhập tay.
- Kết quả đổ dần vào panel, nhóm theo file.
- Giới hạn 20.000 kết quả, kèm thông báo khi bị cắt.
- Đổi truy vấn thì hủy lượt tìm cũ (debounce 150 ms).
- Bấm vào một kết quả: mở file và tô sáng chỗ khớp.

## 5. Settings, keybindings và lệnh

### 5.1 Đường dẫn

| Loại | Đường dẫn |
|---|---|
| User settings | `~/.config/lucerna/settings.json` |
| User keybindings | `~/.config/lucerna/keybindings.json` |
| User themes | `~/.config/lucerna/themes/*.json` |
| Workspace settings | `<root>/.lucerna/settings.json` |
| Trạng thái workspace | `~/Library/Application Support/Lucerna/workspaces/<sha256(root)>.json` |
| Log | `~/Library/Logs/Lucerna/` |

### 5.2 Settings

- Định dạng JSON có comment (JSONC).
- Gộp theo thứ tự: mặc định → user → workspace. Override theo ngôn ngữ bằng khóa `"[rust]": { … }`.
- Tên khóa giữ đúng như VS Code khi có khái niệm tương đương. Danh sách khóa giai đoạn 1:
  - `editor.fontFamily`, `editor.fontSize`, `editor.lineHeight`, `editor.wordWrap`, `editor.minimap.enabled`, `editor.renderWhitespace`, `editor.tabSize`
  - `files.exclude`, `search.exclude`
  - `workbench.colorTheme`, `workbench.editor.enablePreview`
  - `explorer.autoReveal`, `explorer.decorations.externalChanges`
  - `lucerna.largeFile.maxBytes`, `lucerna.largeFile.maxLines`
- Settings được nạp lại tự động khi file thay đổi trên đĩa.

### 5.3 Keybindings

- Định dạng giống VS Code: mảng các phần tử `{ "key": "cmd+p", "command": "workbench.action.quickOpen", "when": "…" }`.
- Lệnh có tiền tố `-` để gỡ một binding mặc định.
- **`when` clause:**
  - Hỗ trợ `!`, `&&`, `||`, `==`, `!=`, dấu ngoặc và context key kiểu boolean hoặc chuỗi.
  - Context key giai đoạn 1: `editorFocus`, `sideBarVisible`, `sideBarFocus`, `explorerViewletFocus`, `searchViewletFocus`, `inQuickOpen`, `findWidgetVisible`, `resourceExtname`, `editorLangId`.
- Hỗ trợ chord, ví dụ `cmd+k cmd+t`.
- Rule nằm sau và có `when` đúng thì thắng, giống VS Code.
- ID lệnh dùng tên của VS Code khi có lệnh tương đương, để người dùng chép keybinding cũ sang được.

### 5.4 Phím tắt mặc định (macOS)

| Phím | Lệnh |
|---|---|
| Cmd+P | Quick Open |
| Cmd+Shift+P | Command palette |
| Ctrl+G | Đi tới dòng |
| Cmd+Shift+F | Tìm trong project |
| Cmd+F | Tìm trong file |
| Cmd+B | Ẩn/hiện sidebar |
| Cmd+Shift+E | Mở Explorer |
| Cmd+\ | Chia ô editor |
| Cmd+1…9 | Chuyển tới ô thứ N |
| Cmd+W | Đóng tab |
| Ctrl+Tab | Tab gần đây |
| Cmd+K Cmd+T | Chọn theme |
| Cmd+, | Mở settings.json |
| Alt+Z | Bật/tắt word wrap |
| Cmd+= / Cmd+- | Phóng to / thu nhỏ chữ |

## 6. Hiệu năng

Đo trên MacBook Pro M-series, bản build release.

| Chỉ số | Mục tiêu |
|---|---|
| Từ chạy `luc` tới khung hình đầu có nội dung | < 200 ms |
| Mở repo khoảng 80.000 file (Linux kernel) | Cây file dùng được < 300 ms; quét xong < 5 s |
| Quick Open trên 80.000 file, mỗi phím gõ | < 16 ms tới khi hiện kết quả |
| Cuộn file 10 MB | 120 fps, không rớt khung hình |
| Khi rỗi, repo trung bình (khoảng 5.000 file, 10 tab) | < 150 MB RAM |
| File thay đổi trên đĩa → màn hình cập nhật | < 100 ms |

## 7. Xử lý lỗi

Nguyên tắc: một lỗi không bao giờ làm sập toàn bộ editor.

| Tình huống | Cách xử lý |
|---|---|
| Không có quyền đọc, file không tồn tại | Tab hiện thông báo lỗi và nút "Thử lại" |
| File nhị phân | Tab hiện "File nhị phân" và nút "Mở dạng text" |
| Encoding lạ | Đoán bằng `encoding_rs`; status bar hiện encoding đang dùng |
| `settings.json` hoặc `keybindings.json` sai cú pháp | Giữ cấu hình hợp lệ gần nhất; status bar báo lỗi kèm số dòng; bấm vào thì mở file tại dòng đó |
| Theme hỏng hoặc không tìm thấy | Dùng One Dark, ghi cảnh báo vào log |
| Parse tree-sitter lỗi hoặc quá thời gian | Hiển thị không tô màu, ghi log |
| Watcher tràn hàng đợi | Quét lại toàn bộ (mục 4.3) |
| Panic trong tác vụ nền | `catch_unwind` bắt lại, hủy tác vụ đó, ghi log; UI vẫn chạy |
| File trạng thái workspace hỏng | Bỏ qua, dùng bố cục mặc định, đổi tên file hỏng thành `.bak` |

Log dùng `tracing` cùng `tracing-appender`, xoay vòng theo ngày, giữ 7 ngày. Lệnh "Lucerna: Open Logs Folder" có trong command palette.

## 8. Kiểm thử

- **Unit test** cho từng crate không có UI:
  - `text`: chuyển đổi dòng, cột và byte; reload và tính dòng thay đổi; nhận diện encoding.
  - `syntax`: chọn ngôn ngữ; highlight trên mẫu nhỏ mỗi ngôn ngữ (snapshot test bằng `insta`).
  - `project`: lọc gitignore; điểm fuzzy match; gộp sự kiện.
  - `settings`: gộp các lớp; override theo ngôn ngữ; parser và bộ đánh giá `when` clause; gỡ binding bằng `-`; chord.
  - `theme`: đọc One Dark và One Light; đi lùi theo tên capture.
- **Test tích hợp cho `project`:**
  - Tạo thư mục tạm, rồi giả lập agent: tạo, sửa, xóa, đổi tên file, và ghi 5.000 file trong một đợt.
  - Kiểm tra snapshot và sự kiện phát ra là đúng.
- **Test UI** bằng `gpui::TestAppContext`, chạy không cần cửa sổ: bố cục mặc định, preview tab, chia ô, Quick Open, lưu và khôi phục trạng thái, keybinding có `when`.
- **Benchmark** bằng `criterion`: quét repo lớn, fuzzy match, highlight file 10 MB, reload file. CI báo lỗi khi một chỉ số chậm đi quá 15%.
- **CI** (GitHub Actions, chạy trên macOS): `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, `cargo deny check` (cấm GPL, AGPL, LGPL), benchmark.

## 9. Giấy phép

- **Mã nguồn Lucerna:** MIT OR Apache-2.0.
- **Dependency được phép:** MIT, Apache-2.0, BSD-2/3, ISC, Zlib, Unicode-3.0, MPL-2.0 (dạng thư viện). Được thực thi bằng `deny.toml`.
- **Từ Zed chỉ được dùng:**
  - Các crate Apache-2.0: `gpui` và các crate nền tảng của nó, cùng các dependency nội bộ Apache của chúng.
  - Theme One (MIT), kèm file license.
- **Grammar và query:** lấy từ repo gốc của từng grammar (phần lớn MIT). Liệt kê trong `THIRD_PARTY_NOTICES.md`.

## 10. Rủi ro

| Rủi ro | Cách giảm thiểu |
|---|---|
| GPUI thay đổi API, chưa có bản phát hành mới | Pin theo một commit; cập nhật bằng PR riêng; giữ toàn bộ code GPUI trong `lucerna-workbench` |
| GPUI thiếu widget (cây, input, danh sách ảo) | Tự viết trên `uniform_list` và `div`; tham khảo `gpui-component` sau khi kiểm tra giấy phép |
| Vô tình chép code GPL từ Zed | `cargo-deny` trong CI; quy ước không mở mã nguồn crate GPL khi đang viết module tương ứng |
| Render chữ hoặc fallback font tiếng Việt và CJK không đúng | Đưa file mẫu tiếng Việt, CJK và emoji vào test thủ công trước khi phát hành |
| Quét repo lớn tốn RAM | Lưu đường dẫn dạng `Arc<str>` hoặc dùng chung tiền tố; đo trong benchmark |

## 11. Các bước kế tiếp

Sau khi spec này được duyệt, viết kế hoạch triển khai bằng skill `writing-plans`. Thứ tự dự kiến:

1. Tạo workspace và CI.
2. `settings` và `theme`.
3. `text`.
4. `project`.
5. `syntax`.
6. `workbench`: khung cửa sổ, sau đó explorer, rồi khung xem code.
7. Quick Open và search.
8. Lưu trạng thái workspace.
9. Benchmark và tinh chỉnh.
