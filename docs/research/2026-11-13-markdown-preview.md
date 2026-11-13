# Nghiên cứu: Markdown preview

Ngày 2026-11-13. Nguồn:
- VS Code: `microsoft/vscode` nhánh `main` (commit `08e8bbf`, 2026-11-13), thư mục `extensions/markdown-language-features/` (`package.json`, `package.nls.json`, `src/preview/*.ts`, `src/markdownEngine.ts`, `src/commands/*.ts`, `src/util/resources.ts`, `preview-src/*.ts`).
- Zed: revision GPUI đang pin `b5cd310b3b54dfc659fcc06453e6358843dfe216` (2026-11-07), các crate `markdown_preview`, `markdown`, `mermaid_render`, `gpui`, `zed/src/zed/quick_action_bar/preview.rs`, `assets/keymaps/default-macos.json`, `assets/settings/default.json`. Zed phần lớn là GPL: chỉ ghi lại thiết kế, không chép code.

## 1. VS Code

### 1.1 Lệnh, phím, menu
Lệnh (`package.json` → `contributes.commands`, cài trong `src/commands/`):

| Lệnh | Tiêu đề | Ghi chú |
|---|---|---|
| `markdown.showPreview` | Open Preview | mở preview trong cùng nhóm editor |
| `markdown.showPreviewToSide` | Open Preview to the Side | mở ở nhóm bên cạnh, focus giữ ở editor |
| `markdown.showLockedPreviewToSide` | Open Locked Preview to the Side | preview khoá vào file này |
| `markdown.showSource` | Show Source | từ preview quay về editor nguồn |
| `markdown.preview.refresh` | Refresh Preview | |
| `markdown.preview.toggleLock` | Toggle Preview Locking | |
| `markdown.showPreviewSecuritySelector` | Change Preview Security Settings | |
| `markdown.reopenAsPreview` / `markdown.reopenAsSource` | Open as Preview / Open as Source | dùng `reopenActiveEditorWith` |
| `markdown.togglePreview` | Toggle Preview | thay tab nguồn bằng preview hoặc ngược lại, **trong cùng tab** |

Phím (mới nhất trên `main`):
- `cmd+k v` (`ctrl+k v`) → `markdown.showPreviewToSide`, `when`: `editorFocus && editorLangId =~ /^(markdown|prompt|instructions|chatagent|skill)$/ && !notebookEditorFocused`.
- `cmd+shift+v` (`ctrl+shift+v`) → **`markdown.togglePreview`** (bản cũ là `markdown.showPreview`), `when`: `!terminalFocus && ((editorFocus && resourceLangId =~ markdown… ) || activeCustomEditorId == 'vscode.markdown.preview.editor')`. Tức là ⌘⇧V bấm trong editor thì chuyển sang preview, bấm trong preview thì về nguồn.

Menu:
- `editor/title`, group `navigation@1`: nút `markdown.showPreviewToSide` (icon `$(open-preview)`), `alt` = `markdown.showPreview` (giữ Alt khi bấm thì mở tại chỗ). Chỉ hiện với editor markdown.
- `editor/title` khi preview active: `markdown.showSource` (icon `$(file-code)`), và trong menu "…" (group `1_markdown`): Refresh, Toggle Lock, Security.
- `explorer/context` và `editor/title/context`: Open Preview.

### 1.2 Preview là gì
- Preview là **webview** (`markdown.preview`), HTML do markdown-it render rồi gửi vào webview; script `preview-src/index.ts` chạy trong webview lo cuộn, click, link.
- Có thêm **custom editor** `vscode.markdown.preview.editor` (`contributes.customEditors`, priority `option` cho `*.md, *.markdown, …`). Vì là custom editor nên dùng được "Reopen Editor With…" và `workbench.editorAssociations` (vd. `{"*.md": "vscode.markdown.preview.editor"}` để mở md thẳng ra preview). Loại này là **static preview** (`StaticMarkdownPreview`), luôn gắn với một file.
- `src/preview/previewManager.ts`: preview mở bằng lệnh là **dynamic preview**. Nếu không khoá, nó **đi theo editor markdown đang active** (`onDidChangeActiveTextEditor` → `update(uri, dòng đang thấy)`). Preview khoá (`locked`) giữ nguyên file. Toggle Lock chuyển qua lại; tiêu đề tab đổi thành "[Preview] x.md" khi khoá.
- Mở lại preview của cùng file trong cùng cột thì dùng lại preview cũ (`matchesResource(resource, column, locked)`).

### 1.3 Cập nhật nội dung
`src/preview/preview.ts`:
- Lắng nghe `workspace.onDidChangeTextDocument` (gõ phím, chưa cần lưu) và `onDidOpenTextDocument`.
- Có `FileSystemWatcher` trên file nguồn; chỉ dùng khi VS Code **không** đang mở document đó (tránh refresh đôi). Vậy thay đổi trên đĩa (vd. agent ghi file) cũng được cập nhật.
- `refresh()`: **lần đầu render ngay**, các lần sau gộp bằng timer **300 ms** (`#delay = 300`). Không phải debounce kiểu "đợi ngừng gõ" mà là throttle: lần gọi đầu đặt timer, các lần trong 300 ms bỏ qua.
- Ảnh cục bộ cũng có watcher riêng (`#updateImageWatchers`): ảnh đổi trên đĩa → refresh.
- Khi webview ẩn mà HTML đã cũ, render lại lúc hiện ra.
- Webview cập nhật bằng cách diff DOM (giữ ảnh đã tải), không thay cả trang.

### 1.4 Đồng bộ cuộn
Cài đặt (`contributes.configuration`, mặc định trên `main`):
- `markdown.preview.scrollPreviewWithEditor` (true): cuộn editor → preview cuộn theo.
- `markdown.preview.scrollEditorWithPreview` (true): cuộn preview → editor cuộn theo.
- `markdown.preview.markEditorSelection` (false trên `main`; bản cũ mặc định true): đánh dấu dòng có con trỏ trong preview (`activeLineMarker.ts`, thêm class lên phần tử).
- `markdown.preview.doubleClickToSwitchToEditor` (false trên `main`): double-click trong preview → mở editor nguồn tại dòng đó (`didClick` → `showSource` + đặt selection + `revealRange(InCenterIfOutsideViewport)`).

Cách ánh xạ dòng:
- `markdownEngine.ts` có plugin `pluginSourceMap`: mọi token block có `token.map` được gắn class `code-line` và `data-line="<dòng bắt đầu, 0-based>"`.
- Webview (`preview-src/scroll-sync.ts`) dựng bảng `[phần tử, dòng]` từ mọi `.code-line` (bỏ `UL/OL` vì item con có cùng dòng; code fence tính thêm dòng kết thúc theo số `\n`). Bảng được cache theo version của document.
- Editor → preview: `topmostLineMonitor.ts` nghe `onDidChangeTextEditorVisibleRanges` (throttle 50 ms), lấy dòng trên cùng (có phần lẻ), gửi vào webview. Webview tìm phần tử trước/sau dòng đó và **nội suy tuyến tính** theo `offsetTop` để ra vị trí cuộn. Chờ ảnh tải xong mới cuộn.
- Preview → editor: sự kiện `scroll` (throttle 50 ms) → từ `scrollY` tìm hai phần tử kẹp quanh → nội suy ra dòng thực → `revealLine` → extension cuộn editor tới dòng đó (`scrollEditorToLine`, hỗ trợ dòng lẻ).
- Chống vòng lặp: mỗi bên có cờ "đang cuộn do bên kia" (`#isScrolling` 200 ms ở extension; `scrollDisabledCount` 50 ms ở webview).
- Dòng con trỏ (`onDidChangeTextEditorSelection`) được gửi riêng cho `markEditorSelection`.

### 1.5 Render
- markdown-it với `html: true`, highlight bằng highlight.js. Cài đặt: `markdown.preview.breaks` (false), `markdown.preview.linkify` (true, `fuzzyLink: false`), `markdown.preview.typographer` (false).
- Kiểu chữ: `markdown.preview.fontFamily` (system-ui…), `markdown.preview.fontSize` (14), `markdown.preview.lineHeight` (1.6). `markdown.styles`: danh sách URL/đường dẫn CSS chèn thêm. Extension có thể góp CSS (`markdown.previewStyles`), script và plugin markdown-it (`markdown.markdownItPlugins`).
- Bảng: markdown-it core có GFM table, strikethrough. **Task list không có trong core**: `- [ ]` hiện ra dưới dạng chữ; muốn checkbox phải cài extension (vd. "Markdown Checkboxes"), và checkbox đó không sửa được file. Mermaid, KaTeX cũng qua extension.
- Front matter: `markdown.preview.frontMatter` = `hide | codeBlock | table` (mặc định `table`), có lệnh "Configure Frontmatter Visibility".
- Heading có id (slug kiểu GitHub) để link `#anchor` hoạt động.

### 1.6 Ảnh và bảo mật
- Đường dẫn ảnh tương đối được resolve theo thư mục chứa file md, rồi đổi sang URI của webview (`asWebviewUri`). `/abs` resolve theo gốc workspace.
- `localResourceRoots` (`src/util/resources.ts`): nếu file thuộc workspace thì là **mọi thư mục workspace**; nếu không thì **thư mục chứa file**. Ảnh ngoài các gốc này không tải được.
- CSP (`documentRenderer.ts`), bốn mức chọn qua "Change Preview Security Settings", lưu trong `globalState` theo từng workspace (không phải setting):
  - **Strict** (mặc định): ảnh/media/font/style chỉ `self`, webview, `https:`, `data:`; script chỉ nonce của VS Code. Không chạy script trong tài liệu, không ảnh `http:`.
  - Allow insecure local content: thêm `http://localhost:*`, `http://127.0.0.1:*`.
  - Allow insecure content: thêm `http:`.
  - Disable: tắt CSP, cho chạy script.
- Khi nội dung bị chặn, preview hiện thanh "Some content has been disabled in this document", bấm vào mở bộ chọn mức bảo mật.

### 1.7 Link
- `http(s):`, `mailto:` mở bằng trình duyệt hệ thống. `#anchor` cuộn trong preview.
- Link tương đối: webview gửi `openLink` về extension. `markdown.preview.openMarkdownLinks` = `inPreview` (mặc định) | `inEditor`: link tới file `.md` khác mở **trong preview** (dynamic preview chuyển sang file đó, cuộn tới fragment); file khác mở trong editor. Hỗ trợ `file.md#L10`.

### 1.8 HTML
VS Code **không có preview HTML dựng sẵn**. Có Simple Browser (`simpleBrowser.show`, nhập URL) và extension Microsoft "Live Preview" (`ms-vscode.live-server`) chạy server cục bộ rồi mở trong Simple Browser.

## 2. Zed

### 2.1 Cấu trúc
- Crate `markdown_preview` chỉ còn 3 file: `markdown_preview.rs` (khai báo action, `init`), `markdown_preview_settings.rs`, `markdown_preview_view.rs` (~5000 dòng, phần lớn là test). Parser và renderer **đã gộp vào crate dùng chung `markdown`** (`markdown.rs`, `parser.rs`, `html/`, `mermaid.rs`) — cũng là crate render hover, agent panel… Các file cũ `markdown_parser.rs`, `markdown_renderer.rs`, `markdown_elements.rs` không còn ở revision này.
- `MarkdownPreviewView` là một workspace item (tab trong pane), serialize được (lưu đường dẫn file + mode trong DB, khôi phục khi mở lại).

### 2.2 Action, phím, nút
- `zed_actions::preview::markdown::{OpenPreview, OpenPreviewToTheSide}`; trong crate: `markdown::OpenFollowingPreview`, `CloseAndReturnToEditor`, `ScrollPageUp/Down`, `ScrollUp/Down`, `ScrollUpByItem/DownByItem`, `ScrollToTop/Bottom`.
- Keymap macOS:
  - context `Editor && extension == md`: `cmd-k v` → `OpenPreviewToTheSide`, `cmd-shift-v` → `OpenPreview`.
  - context `MarkdownPreview`: `pageup/pagedown`, `up/down`, `alt-up/alt-down` (theo khối), `cmd-up/cmd-down`, `cmd-shift-v` → `CloseAndReturnToEditor` (⌘⇧V thành phím bật/tắt, giống `togglePreview` của VS Code).
- Nút: `quick_action_bar/preview.rs` thêm `IconButton` biểu tượng **Eye** trên toolbar của pane khi item là file md (cũng dùng cho SVG và CSV). Tooltip "Preview Markdown" kèm phím; bấm = `OpenPreview`, **Alt+click = mở ở split** (`OpenPreviewToTheSide`).
- Menu chuột phải editor và menu tab có "Open Markdown Preview" (`editor/src/mouse_context_menu.rs`, `editor/src/items.rs`).

### 2.3 Chế độ
- `MarkdownPreviewMode::Default`: gắn với một editor cụ thể (giống locked của VS Code).
- `MarkdownPreviewMode::Follow` (`OpenFollowingPreview`): observe workspace, **đi theo editor markdown đang active** (giống dynamic preview).
- `OpenPreview` thêm preview vào **cùng pane** (đã có thì kích hoạt lại). `OpenPreviewToTheSide` thêm vào pane kề (`adjacent_pane_of`, tạo split nếu chưa có), không focus preview mà **trả focus về editor**.
- Setting `markdown_preview.open_markdown_files_in_preview` (false): mở file md là mở preview; nếu tab editor chưa sửa thì thay luôn tab đó.
- Setting khác: `font_size`, `font_family`, `code_font_family`, `heading_font_weight` (600), `limit_content_width` (true) + `max_width` (800 px, căn giữa). Font size đổi được bằng phím zoom chung khi preview có focus. Có `markdown_preview_theme` trong theme settings (preview dùng theme khác editor).

### 2.4 Cập nhật nội dung
- Subscribe sự kiện editor: `Edited`, `BufferEdited`, `BuffersEdited`, `DirtyChanged` → cập nhật có debounce; `FileHandleChanged` (đổi tên/di chuyển) → cập nhật ngay, đổi thư mục gốc.
- Debounce `REPARSE_DEBOUNCE = 200 ms`, kiểu "đã có task chờ thì bỏ qua" (giống throttle 300 ms của VS Code). Lần mở đầu không chờ.
- Không watch file riêng: thay đổi trên đĩa đi qua buffer (buffer tự reload khi file ngoài đổi) → `BufferEdited`.
- Lấy toàn bộ text của buffer → `Markdown::reset(source)`. Crate `markdown` **parse trên background thread** (`cx.background_spawn`), giữ kết quả cũ tới khi xong, rồi `notify`. Bỏ qua nếu source không đổi. Highlight code block dùng language registry (tree-sitter) của Zed.

### 2.5 Render và cuộn
- Render: một `div` `overflow_y_scroll` + `ScrollHandle`, chứa **một `MarkdownElement`** dựng toàn bộ tài liệu. **Không dùng `list`/`ListState` ảo hoá** ở revision này (bản cũ của crate từng dùng `list`). Có scrollbar riêng theo setting editor, padding, giới hạn bề rộng.
- Có tìm kiếm trong preview (`SearchableItem`), chọn văn bản, menu chuột phải "Copy", "Copy as Markdown", "Copy Link".

### 2.6 Đồng bộ với editor
Zed **không đồng bộ theo vị trí cuộn**, mà theo **con trỏ** và **offset byte**:
- Parser ghi lại `root_block_starts`: offset byte bắt đầu của từng khối gốc (đoạn, heading, list, bảng…).
- Editor → preview: `EditorEvent::SelectionsChanged` → lấy offset của selection cuối → tìm khối gốc chứa offset đó → (1) đánh dấu khối active bằng **vạch bên trái** (`show_root_block_markers`, màu `border` cho khối active, `border_variant` cho khối khác), (2) nếu editor hoặc preview đang focus thì `request_autoscroll_to_source_index` cuộn preview tới khối đó (đợi parse xong nếu đang parse).
- Preview → editor: **click một lần** vào khối (`on_source_click`) → đặt con trỏ editor tại offset nguồn của chỗ click, `Autoscroll::center`, không chuyển focus. Cuộn preview **không** cuộn editor.
- `CloseAndReturnToEditor` đóng preview, focus editor.

### 2.7 Tính năng render
- pulldown-cmark 0.13, `PARSE_OPTIONS` = `TABLES | FOOTNOTES | OLD_FOOTNOTES | STRIKETHROUGH | TASKLISTS | SMART_PUNCTUATION | HEADING_ATTRIBUTES | PLUSES_DELIMITED_METADATA_BLOCKS | GFM | SUPERSCRIPT | SUBSCRIPT`; thêm `YAML_STYLE_METADATA_BLOCKS` khi `render_metadata_blocks`. Cố ý không bật `MATH`, `DEFINITION_LIST`, `WIKILINKS` (có test buộc xem xét mỗi option mới).
- Preview bật `MarkdownOptions { parse_html, render_mermaid_diagrams, parse_heading_slugs, render_metadata_blocks }`.
- **Task list**: checkbox (`ui::Checkbox`) **bấm được**; callback thay đúng đoạn `[ ]` ↔ `[x]` trong buffer qua `editor.edit` (có kiểm tra text cũ), rồi refresh preview. Sửa vào buffer, chưa lưu file.
- **Bảng**: có, kể cả căn lề.
- **HTML trong markdown**: parse bằng `html5ever`, chỉ hỗ trợ một tập thẻ: `img` (src, alt, width, height), `p`, `h1–h6`, `ul/ol/li`, `blockquote`, `table/tr/th/td` (colspan, rowspan, align), `b/strong/i/em/del/ins/a`, một ít `style` (underline, italic, bold, text-align) và `align`. Không chạy script, không CSS tuỳ ý.
- **Mermaid**: có, crate `mermaid_render` (dựa trên `merman`, xuất SVG rồi vẽ bằng svg renderer của gpui); có zoom.
- **Front matter**: YAML `---` và `+++` render thành khối metadata.
- Heading slug: link `#heading` cuộn tới heading.
- Code block: highlight, nút Copy hiện khi hover.

### 2.8 Ảnh và link
- `image_resolver` (`resolve_preview_image`):
  - `data:` → bỏ qua.
  - `http(s)://` → `Resource::Uri` (gpui tải qua `cx.http_client()`).
  - `/x` hoặc `\x` → tính từ gốc worktree đầu tiên; đường dẫn tương đối → từ thư mục của file md; chuẩn hoá `..` theo từ vựng. Có decode `%20`.
  - Project remote (SSH) → `image_store::project_image_source` đọc qua project.
  - Chỉ trả ảnh khi file tồn tại.
- Dùng `RetainAllImageCache` gắn vào view để không tải lại ảnh mỗi frame.
- Link click (`handle_url_click`): `#slug` cuộn trong preview; link tới file md khác mở **trong preview** (dùng lại preview đã có cho buffer đó), có `file.md#L10:5` → mở và đặt con trỏ; file khác mở trong editor (`open_url_or_file`); URL ngoài → trình duyệt. Hover link hiện URL ở góc dưới.

### 2.9 HTML file
Zed **không có preview HTML** (chỉ có preview cho Markdown, SVG, CSV/TSV — `svg_preview`, `tabular_data_preview`).

### 2.10 gpui: ảnh
- `gpui/Cargo.toml` đã phụ thuộc `image`, `resvg`, `usvg`. Workspace bật `image` 0.25 với feature `bmp, dds, exr, ff, gif, hdr, ico, jpeg, png, pnm, qoi, rayon, tga, tiff, webp`; `resvg` 0.46 có `text, system-fonts, memmap-fonts, raster-images`.
- `gpui::ImageFormat`: Png, Jpeg, Webp, Gif (động, `decode_gif_image`), Svg, Bmp, Tiff, Ico, Pnm. `img()` nhận `ImageSource::Resource(Path | Uri | Embedded)`, `Image`, `Render`, hoặc closure.
- `Cargo.lock` của Lucerna đã có `image 0.25.10`, `resvg/usvg 0.46.0`, `pulldown-cmark 0.13.6` (đang dùng ở `crates/lucerna-workbench/src/markdown.rs` cho hover). Thêm ảnh cục bộ **không kéo thêm crate**. Ảnh `http(s)` cần một `HttpClient` cho gpui; Lucerna hiện chưa đặt cái nào, nên URL ngoài sẽ không tải được nếu không thêm.

## 3. Giống và khác

| Mặt | VS Code | Zed |
|---|---|---|
| Kỹ thuật render | webview HTML + markdown-it | phần tử GPUI gốc + pulldown-cmark |
| Phím | ⌘⇧V toggle tại chỗ, ⌘K V bên cạnh | ⌘⇧V mở (trong preview: đóng, về editor), ⌘K V bên cạnh |
| Nút | editor title, Alt = mở tại chỗ | toolbar pane (Eye), Alt = mở split |
| Đi theo editor active | dynamic (mặc định) / locked | Default (khoá) / Follow (lệnh riêng) |
| Mở md thẳng ra preview | `workbench.editorAssociations` | `open_markdown_files_in_preview` |
| Cập nhật | throttle 300 ms, watcher file và ảnh | debounce 200 ms qua buffer, parse nền |
| Đồng bộ | theo **vị trí cuộn** hai chiều, nội suy theo dòng | theo **con trỏ** (editor → preview) và **click** (preview → editor), theo offset |
| Đánh dấu dòng | `markEditorSelection` (tắt mặc định) | vạch trái ở khối active (luôn bật) |
| Task list | không có trong core | có, bấm được, sửa buffer |
| Mermaid | qua extension | dựng sẵn |
| HTML trong md | đầy đủ, chịu CSP | tập thẻ nhỏ |
| Bảo mật | CSP 4 mức | không cần (không có script) |
| Link `.md` | mở trong preview (`openMarkdownLinks`) | mở trong preview |
| Front matter | hide / codeBlock / table | khối metadata |
| Preview HTML | không (Simple Browser, Live Preview) | không |

## 4. Gợi ý cho Lucerna

Theo `docs/design-principles.md`: hành vi, setting, lệnh, phím theo VS Code; UI và hiệu năng theo Zed.

1. **Lệnh và phím (VS Code)**: `markdown.showPreview`, `markdown.showPreviewToSide`, `markdown.showLockedPreviewToSide`, `markdown.showSource`, `markdown.togglePreview`, `markdown.preview.toggleLock`, `markdown.preview.refresh`. ⌘⇧V = `togglePreview` (tại chỗ, bấm lại về nguồn — cả VS Code mới và Zed đều cho ⌘⇧V là phím bật/tắt). ⌘K V = to side, focus ở lại editor. `when` dựa trên ngôn ngữ markdown.
2. **Dynamic/locked (VS Code)**: preview mở bằng lệnh đi theo editor md active; "Locked" giữ file. Bỏ qua security selector (không có script).
3. **Nút (UI Zed)**: một icon Eye trên thanh của nhóm editor, chỉ hiện với file md; bấm = to side (như VS Code `navigation@1`), Alt = tại chỗ. Tab preview tên "Preview x.md". Dùng lại component của `ui.rs`.
4. **Render (Zed)**: phần tử GPUI gốc, không webview. Mở rộng `crates/lucerna-workbench/src/markdown.rs` (đang có cho hover) thành renderer dùng chung cho hover và preview, như Zed gộp vào crate `markdown`. Parse trên background, giữ kết quả cũ tới khi xong, bỏ qua nếu text không đổi. Lưu offset/dòng bắt đầu của mỗi khối gốc khi parse (pulldown-cmark `into_offset_iter`).
5. **Cập nhật**: nghe sự kiện sửa buffer, throttle khoảng 200–300 ms, lần đầu render ngay. Thay đổi trên đĩa nên đi qua buffer như Zed (cần kiểm tra luồng reload buffer khi file ngoài đổi của Lucerna phát sự kiện mà preview nghe được); quan trọng vì agent ghi file. Nếu preview mở khi không có buffer (locked, tab đã đóng) thì cần watcher file như VS Code.
6. **Đồng bộ**: tối thiểu như Zed (con trỏ → cuộn preview tới khối + vạch trái; click khối → đặt con trỏ editor). Cài đặt `markdown.preview.scrollPreviewWithEditor` / `scrollEditorWithPreview` của VS Code thì cần ánh xạ dòng trên cùng ↔ khối và nội suy theo chiều cao khối đã layout; đáng làm ở bước sau, vì người dùng Lucerna chủ yếu **đọc** file agent viết (cuộn nhiều hơn gõ). `doubleClickToSwitchToEditor` dễ làm khi đã có ánh xạ.
7. **Hiệu năng (Zed + hơn Zed)**: Zed dựng cả tài liệu trong một phần tử. Với file md dài (log, plan agent sinh ra), nên dùng `gpui::list` với mỗi khối gốc là một item để chỉ layout phần thấy được; `ListState` cũng có sẵn API cuộn tới item, khớp với ánh xạ khối.
8. **Tính năng**: pulldown-cmark với TABLES, STRIKETHROUGH, TASKLISTS, FOOTNOTES, GFM (alert `> [!NOTE]`), HEADING_ATTRIBUTES, YAML metadata. `breaks`, `linkify`, `typographer` theo setting VS Code (`markdown.preview.*`; typographer ↔ `ENABLE_SMART_PUNCTUATION`, linkify phải tự làm hoặc dùng crate `linkify`, breaks = coi SoftBreak là HardBreak). Front matter theo `markdown.preview.frontMatter` (mặc định `table`, có thể hide). Task list: hiển thị checkbox; bấm để sửa buffer như Zed là điểm cộng nhỏ, có thể để sau. Mermaid, math, HTML thô: để ngoài phạm vi đầu; HTML inline chỉ hiện như text hoặc bỏ qua.
9. **Ảnh**: `img()` của gpui đã đọc png/jpg/gif/webp/svg/bmp/tiff/ico, không thêm crate. Resolve: tương đối theo thư mục file md, `/x` theo gốc workspace, chuẩn hoá `..`, decode `%xx`; giới hạn trong thư mục workspace (giống `localResourceRoots`) hoặc thư mục chứa file nếu file ngoài workspace. Dùng `RetainAllImageCache` hoặc tương đương. Ảnh `http(s)` cần HttpClient: để sau, hiện alt text.
10. **Link**: `#anchor` cuộn tới heading (cần slug kiểu GitHub); `.md` mở trong preview theo `markdown.preview.openMarkdownLinks` (mặc định `inPreview`); file khác mở editor; `#L10` đặt con trỏ; URL ngoài mở trình duyệt.
11. **Setting kiểu chữ**: `markdown.preview.fontFamily`, `fontSize` (14), `lineHeight` (1.6) theo VS Code; thêm giới hạn bề rộng nội dung căn giữa như Zed (`max_width` 800) vì trông gọn hơn. `markdown.styles` (CSS) không áp dụng được.
12. **HTML preview**: cả hai không có; Lucerna cũng không cần.
