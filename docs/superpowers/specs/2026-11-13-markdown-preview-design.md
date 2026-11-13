# Markdown preview — thiết kế

Ngày: 2026-11-13. Trạng thái: đã duyệt.

## 1. Ý định

Người dùng Lucerna để agent viết code, và agent viết rất nhiều `.md`: README,
plan, spec, ghi chú. Đọc những file đó ở dạng mã nguồn thì mệt: bảng thành
dãy `|`, task list thành `- [ ]`, ảnh chỉ là đường dẫn. Lucerna cần một
preview markdown **vẽ bằng GPUI** để đọc chúng, cập nhật ngay khi agent ghi
lại file.

Vì đọc là việc chính (`design-principles.md` §3), file `.md` **mặc định mở
ra preview**. Mã nguồn chỉ cách một phím (⌘⇧V). Ai muốn như VS Code thì tắt
bằng setting.

Thành công: mở một spec 2000 dòng do agent viết, đọc được ngay với bảng,
task list và ảnh; agent ghi đè file thì preview đổi theo trong vòng chưa
tới nửa giây mà không mất chỗ đang đọc; binary tăng không quá vài trăm KB.

## 2. Mục tiêu, không làm, phản mục tiêu

**Mục tiêu**
- Mở preview tại chỗ (thay tab nguồn) hoặc ở nhóm bên cạnh.
- File `.md` mở mặc định ra preview, trừ khi nhảy tới một dòng cụ thể.
- Tự cập nhật khi buffer đổi: gõ phím, undo, và file bị ghi đè trên đĩa.
- Hiển thị: heading, đoạn, nhấn mạnh, gạch ngang, code inline, code block
  có tô màu, danh sách (có thứ tự, bắt đầu từ số bất kỳ), task list, trích
  dẫn, đường kẻ, bảng (có căn lề), ảnh local, link.
- Cuộn đồng bộ hai chiều với editor của cùng file.
- Lệnh "Open in Browser" cho file `.html`/`.htm`.

**Không làm (bản này)**
- Preview HTML (xem §4.3).
- Ảnh `http(s)` (cần HTTP client cho gpui; hiện alt text).
- Mermaid, công thức toán, footnote, raw HTML trong markdown (raw HTML bị
  bỏ qua như hover hiện nay), front matter dạng bảng (front matter bị ẩn).
- Preview "locked" (`markdown.showLockedPreviewToSide`,
  `markdown.preview.toggleLock`), bấm checkbox để sửa file, tìm trong
  preview (⌘F), chọn và copy văn bản trong preview, `markdown.styles`.

**Phản mục tiêu** (tính năng chạy được nhưng vẫn là thất bại nếu):
- Binary tăng hơn 500 KB, hoặc thêm một crate mới vào `Cargo.lock`.
- Luồng UI đứng khi file lớn đổi liên tục (agent đang ghi).
- Preview nhảy về đầu trang mỗi khi file đổi.
- Kéo webview vào.

## 3. Tham khảo

Chi tiết ở `docs/research/2026-11-13-markdown-preview.md`. Tóm tắt:

| Mặt | VS Code | Zed | Lucerna |
|---|---|---|---|
| Render | webview + markdown-it | phần tử GPUI, pulldown-cmark, parse nền | **Zed** (GPUI, pulldown-cmark, parse nền) |
| Phím | ⌘⇧V `markdown.togglePreview` (tại chỗ, bấm lại về nguồn), ⌘K V `showPreviewToSide` | ⌘⇧V mở / trong preview đóng, ⌘K V bên cạnh | **VS Code** |
| Nút | editor title, Alt = tại chỗ | icon Eye trên toolbar pane, Alt = split | **Zed** (icon trên thanh đường dẫn), hành vi bấm theo VS Code |
| Preview bên cạnh | dynamic: đi theo editor md active | lệnh Follow riêng | **VS Code** (dynamic) |
| Mở md ra preview | `workbench.editorAssociations` (mặc định tắt) | `open_markdown_files_in_preview` (mặc định tắt) | key của **VS Code**, **mặc định bật** (lệch, §4.1) |
| Cập nhật | throttle 300 ms | debounce 200 ms qua buffer | qua buffer, 200 ms |
| Đồng bộ | theo vị trí cuộn, hai chiều, nội suy theo dòng | theo con trỏ và click | **VS Code** (theo cuộn) |
| Dựng tài liệu | DOM | một phần tử cho cả tài liệu | `gpui::list`, mỗi khối gốc một item (hơn Zed về hiệu năng) |
| Task list | không có trong core | checkbox bấm được | checkbox chỉ hiển thị |
| Ảnh | trong workspace hoặc thư mục file | tương đối theo file, `/x` theo worktree | như Zed, giới hạn như VS Code |

## 4. Quyết định

### 4.1 Mở `.md` mặc định ra preview

Dùng setting của VS Code `workbench.editorAssociations` (object glob →
editor id). Lucerna chỉ hiểu một editor id: `vscode.markdown.preview.editor`
và id đặc biệt `default` (mã nguồn). Giá trị mặc định của Lucerna:

```json
"workbench.editorAssociations": { "*.md": "vscode.markdown.preview.editor", "*.markdown": "vscode.markdown.preview.editor" }
```

Glob khớp theo tên file (không có `/`) hoặc theo đường dẫn tuyệt đối (có
`/`), như VS Code. Người dùng xóa key hoặc đặt `"*.md": "default"` để quay
về mã nguồn.

Lệch VS Code (mặc định của VS Code là rỗng) vì người dùng Lucerna đọc
markdown nhiều hơn sửa.

Cách mở nào đi đâu:

| Cách mở | Kết quả với file khớp association |
|---|---|
| Click trong Explorer, ⌘P (không có `:dòng`), Open Recent, `luc file.md`, khôi phục phiên, Open File… | preview |
| Kết quả Search, Go to Definition, References, diagnostics, ⌘P có `:dòng`, link `file#L10`, Go Back/Forward | mã nguồn (cần đúng dòng) |

Quy tắc trong code: `open_path_at` hiện gọi `open_path`, và Go Back
(`Workbench::navigate`) gọi `open_path` rồi `go_to_and_scroll_when_opened`.
Vì vậy tách hai lối vào: một hàm mở **mã nguồn** (dùng bởi `open_path_at`,
`jump_to`, `navigate` và mọi chỗ có điểm đến), và `open_path` (không điểm)
tra association rồi gọi hàm kia hoặc mở preview. Preview tab (chữ nghiêng,
`workbench.editor.enablePreview`) áp dụng y như tab file.

**File đã mở sẵn trong nhóm** (theo VS Code: dùng lại editor đã mở):
- Mở không có điểm: nếu nhóm đã có tab mã nguồn hoặc preview tĩnh của file
  thì kích hoạt tab đó (ưu tiên tab đang active, rồi tab dùng gần nhất), không
  tạo tab mới. Chưa có thì theo association.
- Mở có điểm: dùng tab mã nguồn của file nếu nhóm đã có; chưa có thì mở tab
  mã nguồn mới. Preview tĩnh đang mở của file đó giữ nguyên.

**Preview là một tab của file.** `Tab::path()` trả về đường dẫn cho cả
`MarkdownPreview`, nên preview tham gia mọi chỗ đang dựa vào nó: chấm
"chưa lưu", lời hỏi lưu khi đóng tab cuối của file chưa lưu, ghim tự động
preview tab của file chưa lưu, `disk_bar`, Explorer làm nổi bật, recent
files. ⌘S và auto-save khi preview có focus lưu buffer của file đó.
Preview hiển thị buffer, kể cả phần chưa lưu. Đóng tab mã nguồn cuối của
file chưa lưu mà preview của file còn mở thì **không** hỏi (file vẫn còn
tab), giống đóng một trong hai tab mã nguồn của cùng file hôm nay. Preview
dynamic thì không tính là tab của file (nó đổi file liên tục): `path()`
trả `None`, không chấm, không hỏi lưu.

### 4.2 Lệnh, phím, nút

Lệnh (ID của VS Code, có trong command palette):

| Lệnh | Tiêu đề | Làm gì |
|---|---|---|
| `markdown.togglePreview` | Markdown: Toggle Preview | Đang ở mã nguồn md → thay tab đó bằng preview tại chỗ. Đang ở preview tại chỗ → thay bằng mã nguồn. Đang ở preview bên cạnh → mở mã nguồn của file đó trong nhóm này. |
| `markdown.showPreview` | Markdown: Open Preview | Mở preview tại chỗ (không đổi lại). |
| `markdown.showPreviewToSide` | Markdown: Open Preview to the Side | Mở (hoặc dùng lại) preview dynamic ở nhóm bên phải; tạo nhóm nếu chưa có. Focus ở lại editor. |
| `markdown.showSource` | Markdown: Show Source | Từ preview, mở mã nguồn tại chỗ. |
| `markdown.preview.refresh` | Markdown: Refresh Preview | Parse lại. |
| `workbench.action.openInBrowser` | Open in Browser | §4.3. |

Phím mặc định (thêm vào `default-keybindings.json`):

```json
{ "key": "cmd+shift+v", "command": "markdown.togglePreview", "when": "editorLangId == markdown || activeEditor == 'markdownPreview'" },
{ "key": "cmd+k v", "command": "markdown.showPreviewToSide", "when": "editorLangId == markdown" }
```

Context key mới `activeEditor` (giá trị `markdownPreview` khi tab active là
preview, tĩnh hay dynamic). Hôm nay `editorLangId` lấy từ `CodeView` active
hoặc `tab.path()`; cần sửa để khi preview active, `editorLangId` là
`markdown` (ngôn ngữ của file đang xem trước) và `resourceExtname` là đuôi
file đó, để ⌘K V chạy từ preview. `when.rs` đã có `||`, `==` và chuỗi
trong nháy.

Phím trong preview (focus ở preview): ↑/↓ cuộn một dòng chữ, PageUp/Down,
Space/Shift+Space một trang, Home/End hoặc ⌘↑/⌘↓ đầu/cuối, như cuộn trong
editor. Vim mode không áp dụng vào preview (bản này).

**Nút** (UI theo Zed): thanh đường dẫn (path bar, dưới thanh tab) có một
`icon_button` ở mép phải:
- Tab mã nguồn `.md`: icon Lucide `eye` (chưa có trong
  `crates/lucerna-workbench/assets/icons`: lấy `eye.svg` từ repo Lucide,
  thêm vào `icons.rs` như các icon khác), tooltip "Open Preview to the Side
  (⌘K V)". Bấm → `showPreviewToSide`; Alt+bấm → `togglePreview` (như VS Code
  `navigation@1`, Alt = tại chỗ).
- Tab preview: icon Lucide `file-code`, tooltip "Show Source (⌘⇧V)". Bấm →
  `togglePreview`.

### 4.3 HTML: chỉ "Open in Browser"

Không có preview HTML. Lệnh `workbench.action.openInBrowser` ("Open in
Browser") mở file đang active bằng trình duyệt mặc định (`cx.open_url` với
`file://` URL). Lệnh chỉ hiện trong palette khi tab active là file
`.html`/`.htm` (`resourceExtname`), không gán phím. File chưa lưu thì mở
bản trên đĩa (ghi chú trong tooltip/palette không cần).

Lý do không làm preview HTML bằng webview (`wry` / WKWebView):
- Phải gắn một cửa sổ native vào cây view GPUI: z-order, focus, cắt khung
  khi cuộn hay chia nhóm đều phải tự xử lý.
- Linux cần WebKitGTK, Windows cần WebView2: thêm dependency hệ thống cho
  mỗi nền tảng, trái với việc Lucerna sẽ ra Linux và Windows.
- Mỗi webview tốn thêm khoảng 50–100 MB RAM, gấp nhiều lần cả editor.
- Trình duyệt của người dùng đã làm việc này tốt hơn, kể cả DevTools.

VS Code và Zed cũng không có preview HTML dựng sẵn.

### 4.4 Preview tại chỗ và preview bên cạnh

Hai loại tab mới trong `TabItem`:

- `MarkdownPreview(Arc<Path>)`: preview **tĩnh** của một file. Do
  association, `togglePreview` hay `showPreview` tạo ra. Tiêu đề tab: tên
  file (`README.md`), icon là icon `eye` thay cho icon file. Mỗi nhóm có
  tối đa một tab như vậy cho một đường dẫn; có thể cùng tồn tại với tab mã
  nguồn của cùng file. Có thể là preview tab (nghiêng). Không phải
  singleton.
- `MarkdownPreviewDynamic`: preview **đi theo** editor markdown active,
  tạo bởi `showPreviewToSide`. Tiêu đề: `Preview README.md`. Tối đa một
  trong toàn cửa sổ (singleton): gọi lại thì đổi file và kích hoạt tab đó.
  Khi tab active của nhóm khác là mã nguồn hoặc preview tĩnh của một file
  markdown khác, preview dynamic chuyển sang file đó, giữ vị trí tương ứng
  với editor (cuộn đồng bộ). Tab không phải markdown thì nó giữ nguyên.

`togglePreview` thay tab tại chỗ: tab mới đứng đúng vị trí tab cũ, giữ trạng
thái ghim. Vị trí đọc được giữ qua lần đổi: preview mở ra cuộn tới chỗ ứng
với dòng trên cùng của editor, và ngược lại (dùng ánh xạ ở §5.4).

Trạng thái phiên (`workspace_state`): lưu như các tab đặc biệt khác, chuỗi
tương đối `lucerna:markdown-preview:<đường dẫn tuyệt đối>` và
`lucerna:markdown-preview-dynamic:<đường dẫn tuyệt đối>`. Khôi phục mở lại
đúng loại tab; file không còn thì bỏ tab.

### 4.5 Cuộn đồng bộ

Theo VS Code, hai setting (mặc định `true`):
- `markdown.preview.scrollPreviewWithEditor`: cuộn editor → preview cuộn.
- `markdown.preview.scrollEditorWithPreview`: cuộn preview → editor cuộn.

Chỉ áp dụng giữa preview và editor **đang hiện** của cùng file ở nhóm khác
(trường hợp ⌘K V). Preview tại chỗ không có editor hiện cùng lúc nên chỉ
dùng ánh xạ khi đổi qua lại (§4.4).

`markdown.preview.doubleClickToSwitchToEditor` (Lucerna mặc định `true`;
VS Code bản mới mặc định `false`): double-click một khối trong preview →
mở mã nguồn tại dòng đầu của khối đó (tại chỗ nếu preview là tĩnh; ở nhóm
có editor của file nếu là dynamic, không có thì tại chỗ). Lệch VS Code vì
với preview mặc định, đây là cách nhanh nhất để sửa đúng chỗ đang đọc.

`markdown.preview.markEditorSelection` không làm bản này.

### 4.6 Kiểu chữ và bố cục

Theo Zed về giao diện: nền `editor_background`, chữ UI (`UI_FONT_FAMILY`),
code dùng font editor. Nội dung căn giữa, rộng tối đa 800 px, padding
ngang 32 px, dọc 24 px. Setting kiểu chữ theo VS Code:
`markdown.preview.fontSize` (mặc định 14), `markdown.preview.lineHeight`
(1.6), `markdown.preview.fontFamily` (mặc định rỗng = font UI).

Kiểu từng khối (màu từ theme, không màu cứng):
- Heading: cỡ chữ h1 2.0×, h2 1.5×, h3 1.25×, h4–h6 1×; đậm; h1 và h2 có
  đường kẻ dưới `border_variant`. Khoảng trên 24 px, dưới 12 px.
- Code block: nền `elevated_surface_background` (theme không có
  `surface_background`; nếu hover đang dùng màu khác cho code thì dùng
  chung màu đó), bo góc `GRID`, padding 12 px, cuộn ngang khi dòng dài
  (không ngắt dòng).
- Trích dẫn: vạch trái 3 px `border`, chữ `text_muted`.
- Bảng: viền 1 px `border_variant`, hàng đầu đậm và cùng nền với code
  block, căn lề theo `:--`/`:-:`/`--:`, cột co theo nội
  dung; bảng rộng hơn khung thì cuộn ngang riêng bảng.
- Task list: `ui::checkbox` (đã có trong `ui.rs`) thay cho dấu đầu dòng,
  không bấm được.
- Ảnh: rộng tối đa bằng khung nội dung, giữ tỉ lệ, không phóng to quá cỡ
  thật.

Hover giữ cỡ chữ và khoảng cách hiện tại: preview truyền một bộ thông số
kiểu riêng vào renderer (§5.2).

## 5. Thiết kế kỹ thuật

### 5.1 Parser (`markdown.rs`, dùng chung với hover)

Mở rộng `parse_markdown`, giữ API hover:
- Bật `ENABLE_TABLES | ENABLE_STRIKETHROUGH | ENABLE_TASKLISTS |
  ENABLE_YAML_STYLE_METADATA_BLOCKS | ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS
  | ENABLE_GFM` (metadata bật chỉ để ẩn front matter; `GFM` cho alert
  `> [!NOTE]`, hiển thị như trích dẫn có tiêu đề in đậm). Hover dùng cùng
  cờ.
- `Block` thêm: `Table { aligns, head: Vec<Vec<Inline>>, rows:
  Vec<Vec<Vec<Inline>>> }`, `Image { url, alt, title }` (khi một đoạn chỉ
  có một ảnh: ảnh khối), task marker trên mục danh sách
  (`items: Vec<ListItem>` với `checked: Option<bool>`), danh sách có số bắt
  đầu (`start: Option<u64>`).
- `Inline` thêm `Strike(Vec<Inline>)`, `Image { url, alt }` (ảnh giữa dòng).
- Mỗi **khối gốc** (con trực tiếp của tài liệu) mang khoảng dòng nguồn
  `lines: Range<u32>` (0-based, dòng cuối không tính), lấy từ
  `Parser::into_offset_iter()` rồi đổi offset byte → dòng bằng một bảng vị
  trí `\n` dựng một lần. Khối lồng không cần dòng.
- `MAX_MARKDOWN_BYTES` (100 KB) giữ cho hover. Preview dùng giới hạn riêng
  **5 MB**; lớn hơn thì preview hiện thông báo "File too large to preview"
  kèm nút Show Source, không parse.

### 5.2 Renderer

Tách `Renderer` thành hàm render **một khối** với một `MarkdownStyle`
(cỡ chữ gốc, line height, khoảng cách khối, cỡ heading, độ rộng tối đa của
ảnh). Hover gọi nó cho mọi khối trong một `div` như hiện nay; preview gọi
nó cho từng item của list. Highlight code block dùng lại `code_runs` có
cache. Link dùng `LinkHandler` hiện có, cộng thêm luật ở §5.6.

### 5.3 View `MarkdownPreview` (`markdown_preview.rs` mới)

- Giữ `Arc<MarkdownBlocks>` hiện tại, `ListState` của `gpui::list` (mỗi
  khối gốc một item, `ListAlignment::Top`, overdraw vừa đủ), đường dẫn
  file và loại (tĩnh hay dynamic).
- Giữ document mở trong `Documents` khi không có editor nào của file:
  `Documents` cần một cách "giữ" document không qua `CodeView` (handle
  thả ra thì document được thu như view đóng). Cách cụ thể do người làm
  chọn; giữ một `CodeView` passive không vẽ là chấp nhận được nếu gọn hơn.
  Nhờ đó file bị agent ghi đè vẫn được reload và preview vẫn nghe được.
- Trạng thái tải: document đang tải → khung trống (không hiện spinner trong
  200 ms đầu); lỗi đọc / file bị xóa → dùng cùng thông báo và `disk_bar`
  như tab mã nguồn.

### 5.4 Cập nhật

- Subscribe `DocumentEvent::Edited` (và `Opened`) của đúng đường dẫn. Lần
  đầu parse ngay; sau đó gom: nếu đã có lần parse đang chờ thì bỏ qua, nếu
  không thì hẹn 200 ms rồi parse snapshot **mới nhất** lúc đó.
- Parse chạy trên luồng nền (`cx.background_spawn`) với `BufferSnapshot`
  bất biến; kết quả cũ hiển thị tới khi kết quả mới về. Kết quả về muộn
  hơn một kết quả mới hơn thì bỏ (so `revision`).
- Giữ chỗ đang đọc khi thay khối: trước khi thay, lấy khối trên cùng
  (`logical_scroll_top`) và dòng nguồn tương ứng (§5.5); sau khi thay,
  `ListState::reset` rồi cuộn tới khối chứa dòng đó với cùng tỉ lệ trong
  khối. Agent ghi đè toàn bộ file thì vẫn giữ được theo dòng.
- Đổi theme, font, `languages_generation`: chỉ vẽ lại (cache runs đã theo
  generation).
- Ảnh đổi trên đĩa: không theo dõi bản này; `markdown.preview.refresh`
  tải lại (xóa cache ảnh của view).

### 5.5 Ánh xạ dòng ↔ vị trí cuộn

Hàm thuần, test được không cần gpui, đặt trong `markdown.rs` hoặc
`markdown_preview/sync.rs`:

- `line → (item, fraction)`: khối cuối cùng có `lines.start ≤ line`;
  `fraction = (line − start) / max(1, next_start − start)` với `next_start`
  là dòng bắt đầu của khối kế (hoặc số dòng tài liệu), kẹp vào [0, 1).
  Dòng trống giữa hai khối thuộc khối trước.
- `(item, offset_px, item_height) → line` (số thực): ngược lại, `start +
  offset/height × (next_start − start)`.

Editor → preview: khi `CodeView` của cùng file (đang hiện, nhóm khác) đổi
`scroll_top_row`, preview `scroll_to(ListOffset { item_ix, offset_in_item:
fraction × chiều cao item })`. Chiều cao item chưa đo thì dùng 0 và sửa ở
frame sau. Preview → editor: khi `ListState` cuộn do người dùng, tính dòng
rồi `set_scroll_top_row(floor)`. Chống vòng lặp: bên nhận cuộn đặt cờ "cuộn
do đồng bộ" và bỏ qua sự kiện cuộn của chính nó cho tới frame sau. Không
throttle thêm: mỗi frame tối đa một lần. (Handler cuộn của `gpui::list`
chỉ chạy khi người dùng cuộn bằng chuột/trackpad, không chạy khi gọi
`scroll_to`, nên chiều editor → preview không tự gây vòng lặp; cờ chủ yếu
cần cho chiều preview → editor.)

`CodeView` cần một sự kiện (hoặc observe) báo `scroll_top_row` đổi; nếu
chưa có thì thêm, không đổi hành vi hiện tại. Dòng bị fold: dùng dòng
buffer thật, không dùng dòng hiển thị.

### 5.6 Ảnh và link

Ảnh (`img()` của gpui, đã đọc png/jpg/gif/webp/svg/bmp/tiff/ico; không
thêm crate):
- `http(s):`, `data:` → không tải; hiện alt text trong khung viền mờ kèm
  icon Lucide `image` (đã có).
- Đường dẫn tương đối → theo thư mục file md; bắt đầu bằng `/` → theo gốc
  workspace (chế độ một file: thư mục của file). Decode `%xx`, chuẩn hóa
  `..` theo từ vựng.
- Chỉ tải ảnh nằm trong gốc workspace, hoặc trong thư mục của file md nếu
  file nằm ngoài workspace (như `localResourceRoots` của VS Code). Ngoài
  vùng đó hoặc không tồn tại → khung alt text như trên.
- Cache ảnh theo view (`RetainAllImageCache` hoặc tương đương), thả ra khi
  tab đóng. Ảnh trên 20 MB (kích thước file) không tải, hiện alt text kèm
  "Image too large".

Link: hôm nay `link_target` chỉ nhận URL tuyệt đối (`url::Url::parse`), nên
link tương đối và `mailto:` đều trượt, và `open_markdown_link` luôn gọi
`jump_to`. Preview cần một hàm giải link **có đường dẫn gốc** (file md đang
xem) và trả về loại đích; `LinkHandler` của preview giữ đường dẫn đó trong
closure, hover giữ hành vi cũ. Luật:
- `http(s):`, `mailto:` → trình duyệt / app mặc định.
- `#slug` → cuộn tới heading có slug đó (slug kiểu GitHub: chữ thường, bỏ
  dấu câu, khoảng trắng → `-`, trùng thì thêm `-1`, `-2`).
- Đường dẫn tương đối hoặc `file://` tới file `.md`/`.markdown` → mở preview
  (`markdown.preview.openMarkdownLinks`, mặc định `inPreview`; `inEditor`
  thì mở mã nguồn). Mở **như click trong Explorer** (`open_path`, tab
  nghiêng), không chuyển file trong chính tab đang xem như VS Code, vì
  Lucerna chưa có lịch sử trong tab preview. Từ preview dynamic thì đổi
  file của chính nó (nó vốn đi theo file). Có `#L10` → mở mã nguồn tại dòng
  (luật §4.1); có `#slug` → mở preview và cuộn tới heading.
- File khác → mở như `open_path` (Explorer click).
- Hover link: con trỏ tay; URL hiện ở tooltip.

### 5.7 Hiệu năng

- Luồng UI không parse: parse nền, UI chỉ vẽ các item đang thấy.
- Mục tiêu: file 5000 dòng (khoảng 200 KB) parse xong dưới 30 ms trên
  luồng nền; cuộn giữ 120 fps trên M1 như editor; một lần cập nhật khi
  agent ghi file không làm mất frame.
- Code block dài (hơn 2000 dòng) vẫn là một item; highlight dùng
  `SYNC_PARSE_BUDGET` như hover, vượt thì vẽ chữ thường.

## 6. Đo kích thước binary và RAM

**Mốc hiện tại**: `luc` bản release ngày 2026-11-13 là **48 543 136 byte**
(`/Applications/Lucerna.app/Contents/MacOS/luc`). Đo lại mốc trên commit
ngay trước khi bắt đầu, cùng profile và cùng `scripts/bundle-app.sh`.

**Cách đo binary**
1. Build release bằng `scripts/bundle-app.sh` trên commit gốc và commit
   cuối của tính năng; so `stat -f %z` của `luc`.
2. `cargo tree -p lucerna --prefix none -e normal | sort -u` trước và sau:
   không được có crate mới.
3. Ngân sách: **tăng ≤ 300 KB**, trần cứng 500 KB. Vượt thì tìm nguyên
   nhân (ví dụ feature mới của `image` bị bật) trước khi merge.

Không cần dependency mới: `pulldown-cmark` đã có, các cờ bảng/task list chỉ
là code trong crate đó; `image`, `resvg` đã vào qua gpui.

**Cách đo RAM** (RSS bằng `footprint <pid>` hoặc `ps -o rss`, sau khi mở 10 s):
- A: mở project Lucerna, một tab mã nguồn `docs/superpowers/specs/2026-11-08-lucerna-phase1-design.md`.
- B: như A nhưng tab đó là preview.
- C: như B thêm một file md có 3 ảnh PNG 1–2 MB.
- Ngân sách: B − A ≤ 10 MB. C tăng thêm xấp xỉ kích thước ảnh đã giải nén
  (rộng × cao × 4 byte) và trở về gần mức A sau khi đóng tab.

Ghi số đo vào phần cuối spec này hoặc vào ghi chú merge.

## 7. Kiểm thử

- Parser: bảng (căn lề, ô trống, inline trong ô), task list lồng, danh sách
  bắt đầu từ 3, ảnh khối và ảnh giữa dòng, front matter bị ẩn, alert GFM,
  khoảng dòng của khối gốc (kể cả code fence, danh sách nhiều đoạn, file
  kết thúc không có `\n`, CRLF). Hover vẫn ra như cũ (test hiện có pass).
- Ánh xạ dòng ↔ cuộn: các ca biên (trước khối đầu, sau khối cuối, khối một
  dòng, tài liệu rỗng), hai chiều xấp xỉ nghịch đảo nhau.
- Association: glob tên file, glob đường dẫn, `default`, key bị xóa; bảng
  "cách mở nào đi đâu" ở §4.1.
- Model nhóm editor: `togglePreview` thay đúng vị trí và trạng thái ghim;
  preview dynamic là singleton và đổi file theo tab active; khôi phục phiên.
- Cập nhật: ghi đè file trên đĩa (như agent) → preview đổi trong 500 ms và
  giữ khối đang đọc; nhiều lần ghi liên tiếp chỉ parse lần cuối.
- Ảnh: tương đối, `/x`, `..` thoát khỏi workspace bị chặn, `%20`, file
  không tồn tại, `http(s)` → alt.
- Link: `#slug`, `.md` mở preview, `#L10` mở mã nguồn, URL ngoài.
- Open in Browser: chỉ hiện với `.html`/`.htm`.
- Giao diện: đặt cạnh Zed (preview của Zed cùng file) và so khoảng cách,
  cỡ chữ, viền, nền, trạng thái trống (`design-principles.md` §1.1).

## 8. Giới hạn đã chấp nhận

- **Go Back/Forward không ghi vị trí trong preview.** `nav_here` chỉ ghi
  khi tab active là `CodeView`. Nhảy từ preview (double-click về mã nguồn,
  click link) không ghi điểm xuất phát, nên Go Back không quay lại preview.
  Thêm loại tab và vị trí cuộn preview vào `NavEntry` để sau.
- `Documents::discard` (Don't Save khi đóng) bỏ document ngay cả khi còn
  view. Với luật ở §4.1, lời hỏi chỉ hiện khi đóng tab cuối của file (mã
  nguồn hoặc preview tĩnh), nên không còn preview nào mất document.

## 9. Việc để người làm tự quyết

- Cách `Documents` giữ document cho preview (§5.3).
- Cách `CodeView` báo đổi `scroll_top_row` (§5.5).
- Cấu trúc file trong `lucerna-workbench` (một file hay thư mục
  `markdown_preview/`).
- Overdraw của `gpui::list` và cách đo chiều cao item lần đầu.
- Thứ tự task trong plan; gợi ý: parser → renderer dùng chung → view và
  tab → association → cuộn đồng bộ → ảnh/link → Open in Browser → đo.

## 10. Số đo

Đo ngày 2026-11-13, cùng profile `release`, build bằng `cargo build --release -p lucerna`
(không dùng `scripts/bundle-app.sh` vì nó giả định `target/` trong repo, còn target dir
được chia sẻ giữa các worktree). Mốc là `7b79e1b` (chỉ có spec và plan), đầu tính năng là `b7f54ef`.

| Mục | Kết quả |
|---|---|
| `luc` mốc `7b79e1b` | 48 792 336 byte |
| `luc` đầu tính năng `b7f54ef` | 49 215 760 byte |
| Chênh lệch | **+423 424 byte (413 KB)**: vượt ngân sách 300 KB, dưới trần cứng 500 KB |
| `git diff 7b79e1b HEAD -- Cargo.lock` | rỗng, không có crate mới |
| Parse `parse_markdown_document`, 5014 dòng, 191 KB | trung vị 2,2 ms, tối đa 3,0 ms (ngân sách 30 ms), `tests/perf.rs::parse_5000_lines` |
| RAM A: tab mã nguồn (bản sao `.txt` của spec phase 1) | footprint 107 MB (RSS 113,6 MB) |
| RAM B: cùng nội dung mở ở preview (`.md`) | footprint 109 MB (RSS 117,4 MB), B − A = +2 MB, trong ngân sách 10 MB |
| RAM C: file md có 3 ảnh PNG 700×700 (mỗi ảnh 1,4 MB) | footprint 124 MB (RSS 130,2 MB), +15 MB so với B, ảnh giải nén khoảng 5,9 MB |

Ghi chú: mỗi ca là một tiến trình `luc <file>` chạy riêng, đo sau 10 s (`footprint <pid>`).
CLI chỉ mở được một file, nên C không có sẵn tab B; "trở về gần A sau khi đóng tab"
chưa đo, cần chạy tay: mở C, đóng tab, đợi 10 s, đo lại.

Nguyên nhân chênh 413 KB (so `size -m` và `nm -m` của hai binary, tên symbol giải bằng
`c++filt`, gộp theo module): `__text` +207 KB, bảng unwind/exception (`__eh_frame`,
`__gcc_except_tab`, `__unwind_info`) +62 KB, `__DATA_CONST` +16 KB, còn khoảng +115 KB là
bảng symbol trong `__LINKEDIT` (binary không strip; +936 symbol, bảng tên +106 KB). Trong
`__text`, các hàm có tên chứa `markdown_preview`/`MarkdownPreview` chiếm 118 KB: code của
module (+54 KB: view, `ScrollSync`, `follow_link`, `resolve_link`, lệnh) cùng các bản
generic của gpui sinh cho `MarkdownPreview` (`Context::spawn`/`listener`/`subscribe`,
drop glue của `Div`). Phần mở rộng parser/renderer trong `markdown.rs` thêm khoảng 28 KB
(`parse` +13 KB, `Renderer::block` +10 KB). Phần còn lại là inline xếp lại giữa các crate
(ví dụ `lsp_types` −15 KB, `serde_json::value` +19 KB), cộng lại gần bằng không.
`pulldown_cmark` không lớn thêm (−2,6 KB), không có crate hay feature mới, không thấy một
generic nào bị nhân bản bất thường, nên không tối ưu. Vượt ngân sách 300 KB nhưng dưới trần
500 KB: cần người dùng chấp nhận trước khi merge.
