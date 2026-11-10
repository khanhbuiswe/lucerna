# Phase 3: hiểu code qua LSP

Ngày 2026-11-10. Trạng thái: chờ duyệt.

## 1. Mục tiêu

Lucerna đọc được code như một IDE. Người dùng nhảy tới định nghĩa, xem tham chiếu, hover xem kiểu và tài liệu, thấy lỗi của compiler ngay trong file, và tìm symbol trong file hay cả project, mà không phải mở VS Code. Lucerna vẫn chỉ đọc: completion, rename, format, code action thuộc Phase 4.

Đã chốt với người dùng:

| # | Quyết định | Lý do |
|---|---|---|
| D1 | **Server lấy từ PATH qua adapter viết sẵn**: rust-analyzer, gopls, pyright / basedpyright, typescript-language-server, clangd. Không tự tải server. | Đây là các ngôn ngữ chính. Zed viết sẵn server cho chúng bằng Rust (GPL), không qua extension. |
| D2 | **Host WASM cho extension Zed để sang Phase 3b**, với spec riêng. | Host WASM không đem lại Rust/Go/Python/TS, và cần `cargo` + `wasm32-wasip2` trên máy người dùng để build `extension.wasm` (Lucerna không dùng `api.zed.dev`). Ước lượng 2–4 tuần riêng phần host. |
| D3 | **Không cho tự khai báo server** chỉ bằng settings. | Giống cả VS Code và Zed. Ngôn ngữ khác chờ Phase 3b. |
| D4 | **Crate `lucerna-lsp` không dùng gpui**, tự viết transport, kiểu dữ liệu từ `lsp-types`. | Test được bằng fake server; không kéo tokio. |
| D5 | **Nhiều kết quả và peek mở picker có preview** (theo `lsp_locations` của Zed); `⇧⌥F12` mở **tab References** dùng chung giao diện với tab Search. Không có vùng peek chèn trong editor. | Giao diện theo Zed. |
| D6 | **Symbol lấy từ LSP `documentSymbol`**; khi không có server thì dùng `outline.scm` của extension ngôn ngữ (nếu có). | Chính xác như VS Code, vẫn có outline khi không có server. |
| D7 | **Outline chỉ có modal `@` và breadcrumbs**, không có panel. | Gọn. |
| D8 | **Diagnostics:** gạch lượn sóng (luôn hiện dù chỉ đọc), icon ở gutter, thông báo inline (tắt mặc định), vạch trên scrollbar, status bar, F8. Không có tab Problems, không tô màu Explorer. | Người dùng chọn. |
| D9 | **Tin cậy thư mục như Zed**: thư mục mới ở Restricted Mode, không chạy server cho tới khi người dùng tin cậy. | Server chạy code của project (build.rs, proc-macro, `go list`). |
| D10 | Server crash thì **không tự restart** (như Zed); người dùng bấm Restart. | Thấy rõ lỗi, không vòng lặp crash. |

**Thành công khi:**
- Mở một crate Rust đã tin cậy, có `rust-analyzer` trên PATH: sau khi index xong, F12 trên một hàm nhảy tới định nghĩa; ⌘-click cũng vậy; ⇧F12 ra picker tham chiếu có preview; hover hiện chữ ký và doc; lỗi `cargo check` hiện gạch đỏ, icon ở gutter và số đếm ở status bar; ⇧⌘O và ⌘T tìm được symbol; breadcrumbs theo con trỏ.
- Agent sửa file trên đĩa: diagnostics và symbol cập nhật theo, không cần thao tác gì.
- Không có server trên PATH: Lucerna vẫn như cũ, nút ⚡ báo rõ thiếu gì và cách cấu hình.
- Mục tiêu hiệu năng của Phase 1 vẫn giữ (khung hình đầu dưới 200 ms, RAM rỗi dưới 150 MB khi chưa mở file nào có server).

**Ngoài phạm vi:** host WASM (3b); tự tải server; server tự khai báo; tab Problems; tô màu Explorer theo lỗi; pull diagnostics (chỉ dùng push); semantic tokens, inlay hint, document highlight, code lens; mọi thứ thuộc về sửa code; workspace nhiều root.

## 2. Tham khảo

Chi tiết ở `docs/research/2026-11-10-lsp.md`. Tóm tắt những điểm quyết định thiết kế:
- **VS Code:** lệnh và phím `editor.action.revealDefinition` (F12), `peekDefinition` (⌥F12), `goToReferences` (⇧F12, nhiều kết quả thì peek), `references-view.findReferences` (⇧⌥F12), `marker.nextInFiles` (F8), `gotoSymbol` (⇧⌘O, `@`), `showAllSymbols` (⌘T, `#`), `navigateBack` (⌃-). Hover trễ 300 ms, sticky. Kết quả trùng chỗ con trỏ thì chạy Find References. Không có icon diagnostics ở gutter. **Bẫy chỉ đọc:** `editor.renderValidationDecorations` mặc định `editable` nên VS Code ẩn squiggle trong editor chỉ đọc; Lucerna luôn hiện.
- **Zed:** transport JSON-RPC tự viết, request có timeout 120 s và tự gửi `$/cancelRequest` khi future bị drop; server khởi động lười theo (worktree, server); shutdown 5 s rồi exit rồi kill; không tự restart. Hover gửi request ở nửa thời gian trễ. Không có peek: nhiều kết quả mở multibuffer hoặc picker có preview (`lsp_locations`). Outline và breadcrumbs mặc định từ `outline.scm`. Status bar có nút LSP (⚡) với chấm trạng thái và popover từng server; log là view chỉ đọc. Restricted Mode cho worktree chưa tin cậy.
- **Lucerna lệch khỏi cả hai** ở: luôn hiện diagnostics dù chỉ đọc (khác VS Code); có icon ở gutter (khác cả hai, người dùng chọn); Restricted Mode chỉ chặn server và khóa `lsp` của workspace, không chặn mọi setting workspace (khác Zed, xem §5).

Giấy phép: các crate `lsp`, `project`, `editor`, `language` của Zed là GPL-3, chỉ học thiết kế. `lsp-types` (MIT), `pulldown-cmark` (MIT) dùng làm dependency. Icon từ Lucide (ISC).

## 3. Kiến trúc

```
lucerna-lsp (mới, không gpui)
  transport ── server ── registry ── adapters
                 │          │
              sync      diagnostics, symbols
lucerna-syntax: outline.scm → cây symbol
lucerna-settings: đọc setting bỏ qua lớp workspace
lucerna-theme: màu diagnostics, link
lucerna-workbench: language_servers (cầu nối), diagnostics_view, hover, goto, locations_picker,
                   references_view, excerpt_list, symbols (@, #, breadcrumbs), lsp_status, trust, nav_history
```

### 3.1 `lucerna-lsp`

Crate mới, phụ thuộc `lsp-types`, `serde`, `serde_json`, `futures`, `crossbeam-channel`, `globset`, `tracing`, `lucerna-text` (snapshot để quy đổi vị trí), `lucerna-syntax` (kiểu cây symbol). Không gpui, không tokio.

**`transport`**
- `Transport::spawn(command, args, env, cwd) -> Result<Transport>`: chạy tiến trình con với stdin/stdout/stderr dạng pipe, và ba thread có tên (`lsp-read-<server>`, `lsp-write-<server>`, `lsp-stderr-<server>`), mỗi thread bọc `catch_panic`.
- Thread ghi: nhận chuỗi JSON qua kênh không giới hạn, ghi `Content-Length: N\r\n\r\n` + body, flush mỗi message.
- Thread đọc: đọc header tới `\r\n\r\n` (chấp nhận `Content-Type`, thứ tự bất kỳ), `read_exact` body, phân loại: response (có `id`, không có `method`) thì gửi vào oneshot đang chờ; request hoặc notification từ server thì đưa vào kênh sự kiện có giới hạn 128.
- Thread stderr: đọc từng dòng, đưa vào log của server.
- EOF ở stdout: mọi request đang chờ nhận `Err(Closed)`, phát sự kiện `Exited { code }`.

**`server`**
- `LanguageServer`: một tiến trình đã `initialize` xong.
  - `request::<R: lsp_types::request::Request>(params) -> Request<R::Result>`. `Request` là future: có timeout (mặc định 120 s; hover và ⌘-hover 10 s), và **drop trước khi có kết quả thì gửi `$/cancelRequest`**.
  - `notify::<N>(params)`.
  - `capabilities() -> &ServerCapabilities`, `position_encoding()`.
  - `shutdown()`: `shutdown` (chờ tối đa 5 s) → `exit` → chờ thoát 1 s → kill. `Drop` gọi shutdown trên một thread riêng.
- `initialize` gửi:
  - `processId`, `rootUri`, `workspaceFolders` = [root], `clientInfo { name: "Lucerna", version }`, `initializationOptions` từ adapter gộp với `lsp.<server>.initialization_options`.
  - `general.positionEncodings = ["utf-8", "utf-16"]`.
  - Capability chỉ đọc: `definition`, `typeDefinition`, `implementation`, `declaration` (đều `linkSupport`); `references`; `hover` (`contentFormat: ["markdown", "plaintext"]`); `documentSymbol` (`hierarchicalDocumentSymbolSupport`); `publishDiagnostics` (`relatedInformation`, `versionSupport`, `tagSupport [1, 2]`, `codeDescriptionSupport`); `synchronization` (không `willSave`, không `didSave`); `workspace.symbol`; `workspace.configuration`; `workspace.workspaceFolders`; `workspace.didChangeWatchedFiles` (`dynamicRegistration`, `relativePatternSupport`); `window.workDoneProgress`; `window.showMessage`.
  - Sau `initialized`, gửi `workspace/didChangeConfiguration { settings: lsp.<server>.settings }`.
- Server gửi request sang thì tự trả lời:

  | Request | Trả lời |
  |---|---|
  | `workspace/configuration` | Với mỗi item: `lsp.<server>.settings` (cả object, hoặc theo `section` dạng `a.b`), không có thì `null` |
  | `window/workDoneProgress/create` | `null` (OK) |
  | `client/registerCapability`, `client/unregisterCapability` | OK; ghi lại đăng ký `workspace/didChangeWatchedFiles` |
  | `workspace/workspaceFolders` | `[root]` |
  | `window/showMessageRequest` | `null` (không chọn action), message vào log |
  | `workspace/applyEdit`, `window/showDocument` | `{ applied: false }` / `{ success: false }` |
  | khác | lỗi `MethodNotFound` |

- Notification từ server: `textDocument/publishDiagnostics` → kho diagnostics; `$/progress` → trạng thái; `window/logMessage` và `window/showMessage` → log (showMessage loại Error còn hiện ở popover ⚡); khác thì bỏ.

**`adapters`**

| Server | Ngôn ngữ (id của Lucerna) | Binary và tham số | Root | Ghi chú |
|---|---|---|---|---|
| `rust-analyzer` | `rust` | `rust-analyzer` | Thư mục **ngoài cùng** có `Cargo.toml`, tính từ file lên tới root workspace; không có thì root workspace | Chạy thử `--version` (proxy rustup trả lỗi khi chưa cài component) |
| `gopls` | `go` | `gopls` | Thư mục gần nhất có `go.work`, rồi `go.mod`; không có thì root workspace | |
| `basedpyright` | `python` | `basedpyright-langserver --stdio` | Thư mục gần nhất có `pyproject.toml`, `setup.py`, `setup.cfg`, `pyrightconfig.json`; không có thì root workspace | Mặc định thử trước pyright |
| `pyright` | `python` | `pyright-langserver --stdio` | như trên | |
| `typescript-language-server` | `typescript`, `typescriptreact`, `javascript`, `javascriptreact` | `typescript-language-server --stdio` | Thư mục gần nhất có `tsconfig.json`, `jsconfig.json`, `package.json`; không có thì root workspace | |
| `clangd` | `c`, `cpp` | `clangd` | Thư mục gần nhất có `compile_commands.json`, `.clangd`; không có thì root workspace | Chỉ có tác dụng khi một extension cung cấp ngôn ngữ C/C++ |

- `Adapter` là trait: `name`, `language_ids`, `default_command`, `root_markers` + kiểu tìm (gần nhất / ngoài cùng), `language_id_for(lang)` (id gửi trong `didOpen`), `initialization_options()` mặc định, `probe_args` (mặc định `--version`, rỗng nghĩa là không chạy thử).
- **Tìm binary:** `lsp.<server>.binary.path` nếu có (dùng nguyên, không chạy thử), kèm `binary.arguments` (thay tham số mặc định) và `binary.env`. Nếu không thì tìm tên binary trong PATH của **môi trường login shell** rồi chạy thử `probe_args` với timeout 5 s. Root không thay đổi kết quả tìm, nên kết quả được cache theo tên server cho tới khi settings đổi.
- **Môi trường login shell** (`env.rs`, mã riêng cho từng nền tảng): macOS và Linux chạy `$SHELL -l -i -c 'env -0'` một lần, nền, timeout 5 s, cache cho cả tiến trình. Lỗi hoặc timeout thì dùng môi trường của tiến trình. Windows (sau này) dùng môi trường của tiến trình. Tiến trình server nhận môi trường này, gộp `binary.env` lên trên.

**`registry`**
- `ServerKey = (root: Arc<Path>, server: &'static str)`. Mỗi key có một `ServerSlot`: `Starting`, `Running(Arc<LanguageServer>)`, `Failed { message, stderr_tail }`, `Stopped`, `NotFound`.
- `servers_for(language_id, path) -> Vec<ServerKey>`: lấy danh sách theo `lucerna.lsp.languageServers` của ngôn ngữ (§8), lọc adapter phục vụ ngôn ngữ đó, tính root theo adapter.
- Server khởi động lười khi file đầu tiên của nó được mở. Khi đang `Starting`, các `didOpen` được xếp hàng và gửi ngay sau `initialized`.
- Settings đổi (`lsp`, `lucerna.lsp.*`) thì tính lại: server không còn được chọn thì shutdown; server có `binary`/`initialization_options` đổi thì restart; chỉ `settings` đổi thì gửi `didChangeConfiguration`.
- `restart(key)`, `stop(key)`, `shutdown_all()` (song song, mỗi server tối đa 5 s).

**`sync`**
- `DocumentSync` theo dõi với mỗi (server, uri): version hiện tại và snapshot đã gửi.
- `open(path, language, snapshot)`: gửi `didOpen` (version 0, cả văn bản) tới mọi server của file.
- `change(path, snapshot)`: version + 1. Server `Full` nhận cả văn bản; server `Incremental` nhận một thay đổi phủ toàn bộ văn bản cũ (range từ đầu tới cuối snapshot cũ). Server `None` không nhận gì.
- `close(path)`: `didClose`, xóa diagnostics của file nếu server không còn gửi cho file chưa mở (diagnostics của file chưa mở vẫn giữ; xem §7.1).
- Không `didOpen` khi file vượt `lucerna.largeFile.maxBytes` / `maxLines`, hoặc có dòng dài hơn 20 000 ký tự.
- `didChangeWatchedFiles`: các đăng ký `workspace/didChangeWatchedFiles` được dịch sang `globset` (pattern tương đối theo `baseUri`). Mỗi sự kiện của worktree (tạo, sửa, xóa) khớp glob thì gửi tới server đó, gộp theo lô 100 ms.

**`position`**
- `to_lsp(snapshot, Point, encoding) -> lsp::Position` và `from_lsp(snapshot, lsp::Position, encoding) -> Point`, tính trên dòng của rope. UTF-16 đếm theo code unit; UTF-8 theo byte. Vị trí ngoài phạm vi thì cắt về cuối dòng hoặc cuối file, không panic.

**`diagnostics`**
- `DiagnosticStore`: `HashMap<PathBuf, HashMap<ServerKey, FileDiagnostics>>`, `FileDiagnostics { version: Option<i32>, items: Vec<Diagnostic> }`.
- `Diagnostic { range: Range<Point>, severity, message, source, code, code_href, tags: { unnecessary, deprecated } }`. Range quy đổi bằng snapshot đang mở; file chưa mở thì giữ `lsp::Range` thô và quy đổi khi mở.
- `publishDiagnostics` có `version` khác version đã gửi gần nhất của file thì bỏ (Lucerna chỉ đọc, server sẽ gửi lại sau `didChange`).
- `summary() -> (errors, warnings)` của cả project; `for_file(path)` sắp theo vị trí; `next(from: (path, Point), forward, scope: File | Files)` cho F8 / ⌥F8: chỉ tính error, warning, info (bỏ hint, như VS Code); thứ tự file theo đường dẫn; vòng lại đầu.
- Một sự kiện `DiagnosticsChanged(paths)` cho mỗi đợt.

**`symbols`**
- `SymbolTree`: `Vec<Symbol>`, `Symbol { name, detail, kind: SymbolKind, range: Range<Point>, selection_range: Range<Point>, children: Vec<Symbol> }`. `SymbolKind` theo LSP.
- Từ LSP: `DocumentSymbol[]` (phân cấp) dùng nguyên; `SymbolInformation[]` (phẳng) dựng cây theo việc range này chứa range kia.
- `path_at(tree, Point) -> Vec<&Symbol>`: chuỗi symbol chứa điểm đó, sâu dần, cho breadcrumbs.
- `flatten(tree) -> Vec<(depth, ancestors, &Symbol)>`: cho `@`.

### 3.2 `lucerna-syntax`: `outline.scm`

- `Language` đọc thêm `outline.scm` của extension (lười, như `highlights.scm`). Ngôn ngữ bundled chưa có.
- `SyntaxState::outline(snapshot) -> Option<SymbolTree>`: chạy query trên cây hiện có. Mỗi match `@item` là một symbol; `@name` là tên; `@context` (trừ phần trùng tên) ghép thành `detail`; quan hệ cha con theo range chứa nhau. `kind` đoán từ chữ đầu của `@context` (`fn`/`def`/`func`/`function` → Function, `struct`/`class` → Struct/Class, `enum`, `trait`/`interface`, `impl`, `mod`/`module`/`namespace`, `const`, `type`); không đoán được thì `Object`.
- Kiểu `SymbolTree`/`Symbol`/`SymbolKind` định nghĩa ở `lucerna-syntax` (để `lucerna-lsp` dùng lại), không phụ thuộc `lsp-types`.

### 3.3 `lucerna-settings`

- `SettingsStore::get_trusted::<T>(key, language)`: như `get` nhưng bỏ qua lớp `Workspace`. Dùng cho khóa `lsp` khi thư mục chưa tin cậy (§5).

### 3.4 `lucerna-theme`

`UiColors` thêm (đọc từ theme Zed, thiếu thì dùng màu dự phòng theo appearance):
- `warning`, `info`, `hint` (đã có `error`);
- `error_background`, `warning_background`, `info_background`, `hint_background`;
- `error_border`, `warning_border`, `info_border`, `hint_border`;
- `link_text_hover` (`link_text.hover`);
- `editor_highlighted_line_background` (`editor.highlighted_line.background`);
- `element_hover` (`element.hover`).

Màu dự phòng lấy theo One Dark / One Light (ghi trong research §6.2), viết tay giá trị.

### 3.5 `lucerna-workbench`

Mỗi module một việc, file nhỏ:

| Module | Việc |
|---|---|
| `language_servers.rs` | Entity `LanguageServers` (một mỗi workbench): giữ `Registry`, `DocumentSync`, `DiagnosticStore`, cache `SymbolTree` theo document; nhận sự kiện từ server qua một vòng `cx.spawn` (giống pump của worktree); nghe `Documents` (mở, reload, đóng) và `on_worktree_event`; phát sự kiện cho các view. API async cho view: `definition(path, point, kind)`, `references`, `hover`, `document_symbols`, `workspace_symbols(query)`. |
| `trust.rs` | Danh sách thư mục tin cậy, modal tin cậy, nút "Restricted Mode" ở title bar. |
| `nav_history.rs` | Lịch sử điều hướng theo group, Go Back / Forward. |
| `code_view/diagnostics.rs` | Dữ liệu vẽ diagnostics theo dòng cho `CodeView` (gạch chân, gutter, inline, scrollbar). |
| `code_view/link.rs` | ⌘-hover / ⌘-click. |
| `hover.rs` | Popover hover và diagnostic. |
| `markdown.rs` | Render markdown (pulldown-cmark) thành phần tử gpui; khối code tô màu bằng `lucerna-syntax`. Component dùng chung. |
| `goto.rs` | Lệnh đi tới định nghĩa / kiểu / implementation / declaration, tham chiếu; quyết định nhảy thẳng, mở picker hay chuyển sang references. |
| `locations_picker.rs` | Picker vị trí có preview. |
| `excerpt_list/` | Danh sách file → đoạn code tách ra từ `search_view` (dữ liệu hàng, header file, dòng code có gutter, tô cú pháp nền, chọn bằng bàn phím). Tab Search và tab References cùng dùng. |
| `references_view.rs` | Tab References. |
| `symbols/` | Nguồn symbol (LSP rồi `outline.scm`), Quick Open `@` và `#`, breadcrumbs. |
| `lsp_status.rs` | Nút ⚡ ở status bar, popover server, chữ tiến trình, tab log. |
| `diagnostics_status.rs` | Số lỗi / cảnh báo và thông báo tại con trỏ ở status bar. |

## 4. Vòng đời server

1. `Documents` báo một file được mở (đã có văn bản và ngôn ngữ).
2. Nếu `lucerna.lsp.enabled` là `false`, hoặc thư mục chưa tin cậy, thì dừng.
3. `registry.servers_for(language, path)` → các `ServerKey`. Key chưa có slot thì khởi động ở nền: tìm binary → spawn → `initialize` (timeout 60 s) → `initialized` → `didChangeConfiguration`. Trong lúc đó slot là `Starting`.
4. `didOpen` gửi ngay nếu server đã `Running`, không thì xếp hàng.
5. Không tìm thấy binary: slot `NotFound`, nút ⚡ có chấm vàng, popover ghi `rust-analyzer not found on PATH. Install it or set "lsp": { "rust-analyzer": { "binary": { "path": … } } }`. Báo một lần cho mỗi server mỗi phiên (không lặp lại mỗi lần mở file).
6. `initialize` lỗi hoặc tiến trình thoát trước đó: slot `Failed` với lỗi và 50 dòng stderr cuối; chấm đỏ.
7. Server thoát khi đang chạy: request đang chờ trả `Err` (UI không báo lỗi riêng cho từng request), diagnostics của server đó bị xóa, slot `Failed` với mã thoát. Không tự restart.
8. Đóng cửa sổ hoặc thoát app: `shutdown_all` song song, tối đa 5 s. `Drop` của registry cũng làm việc này.

## 5. Tin cậy thư mục (theo Zed)

- Thư mục mở lần đầu ở **Restricted Mode**: không khởi động server nào, và khóa `lsp` của `.lucerna/settings.json` bị bỏ qua (đọc bằng `get_trusted`). Các setting khác của workspace vẫn áp dụng: chúng không chạy được code. Đây là chỗ lệch khỏi Zed (Zed bỏ cả file settings của project).
- Title bar hiện nút chữ "Restricted Mode" có icon cảnh báo (`warning`), như Zed. Bấm vào, hoặc chạy lệnh `workbench.trust.manage`, thì mở **modal tin cậy** (dùng `popover`):
  - tiêu đề có icon cảnh báo: "Unrecognized Project";
  - đường dẫn thư mục, màu mờ;
  - "Untrusted projects are opened in Restricted Mode to protect your system.";
  - "Restricted Mode prevents:" và hai gạch đầu dòng "Language servers from running", "`lsp` settings in .lucerna/settings.json from being applied";
  - checkbox "Trust all projects in the `<parent>` folder";
  - hai nút: "Stay in Restricted Mode" (Esc) và "Trust and Continue" (Enter, nút chính).
- Modal tự hiện lần đầu khi mở một file mà ngôn ngữ của nó có server trên PATH, trong một thư mục chưa tin cậy (không hiện ngay khi mở thư mục, để người chỉ xem file thường không bị hỏi). "Stay in Restricted Mode" thì modal không tự hiện lại trong phiên đó.
- Lựa chọn lưu ở `<data dir>/trusted.json`: `{ "folders": [..], "parents": [..] }`. Thư mục được tin cậy nếu nó có trong `folders` hoặc nằm dưới một mục trong `parents`. Lệnh `workbench.trust.clear` xóa hết và chuyển các cửa sổ về Restricted Mode (dừng mọi server).
- Setting `session.trustAllWorktrees` (mặc định `false`, chỉ đọc ở lớp User): `true` thì mọi thư mục coi như tin cậy, không lưu vào `trusted.json`.
- Single-file mode (`luc file.rs`): thư mục chứa file được xét như thư mục thường.
- Khi người dùng tin cậy, các file đang mở được `didOpen` ngay.

## 6. Đồng bộ document

- **Mở:** khi `Documents` tải xong một file (hoặc file đã có thì khi view đầu tiên mở), `LanguageServers` gọi `sync.open`. Một document chỉ được `didOpen` một lần dù có nhiều tab.
- **Thay đổi trên đĩa:** reload của `Documents` (do agent ghi file, qua `on_worktree_event` sẵn có) gọi `sync.change` với snapshot mới. Không gửi khi reload không đổi nội dung.
- **Đóng:** khi view cuối của document đóng (`collect_closed`), gọi `sync.close`.
- **Đổi ngôn ngữ** (`languages_changed`): `didClose` với server cũ, `didOpen` với server mới nếu khác.
- **File bị xóa:** `didClose`; sự kiện xóa vẫn tới `didChangeWatchedFiles`.
- `Documents` phát sự kiện `DocumentEvent::{Opened, Reloaded, Closed, LanguageChanged}` (entity event của gpui); `LanguageServers` đăng ký nghe.

## 7. Tính năng và giao diện

Mọi tính năng LSP chỉ bật khi file có ít nhất một server `Running` hỗ trợ capability tương ứng; không thì lệnh không làm gì (không báo lỗi), trừ symbol có `outline.scm`.

### 7.1 Diagnostics

**Trong editor** (`CodeView`):
- **Gạch lượn sóng** cao 1 px, dưới range, màu theo mức độ: `error`, `warning`, `info`, `hint`. Range rỗng thì gạch một ký tự (hoặc ký tự cuối dòng). Diagnostic có tag `Unnecessary` và mức độ dưới warning thì không gạch mà làm mờ chữ còn 30% độ đục. Tag `Deprecated` thì gạch ngang chữ.
- **Icon gutter**: mỗi dòng có error / warning / info thì hiện icon của mức cao nhất (Lucide `circle-x`, `triangle-alert`, `info`, 12 px, màu theo mức độ) ở mép trái gutter, trước số dòng. Gutter rộng thêm 16 px khi `lucerna.diagnostics.gutterIcons` bật và file có server. Hint không có icon.
- **Inline** (khi `lucerna.diagnostics.inline.enabled`): sau ký tự cuối của dòng, cách 4 em, hiện dòng đầu của thông báo của diagnostic nặng nhất trên dòng; nền là màu mức độ ở 5% độ đục, chữ màu mức độ, font code, cỡ chữ nhỏ hơn 1 px, bo góc 2 px, padding ngang 4 px. Bị cắt ở mép phải editor. Không hiện cho hint.
- **Scrollbar**: vạch ngang cao 2 px ở nửa phải của rãnh scrollbar, màu theo mức độ (error, warning; info và hint không vẽ), vẽ ở vị trí tỉ lệ của dòng.
- Dữ liệu vẽ (`code_view/diagnostics.rs`) dựng lại khi diagnostics của file đổi hoặc file reload, không dựng theo frame: danh sách theo dòng gồm các đoạn gạch, mức cao nhất của dòng và thông báo inline.

**Hover diagnostic:** rê chuột lên range có diagnostic (range hẹp nhất thắng) thì hover (§7.2) có thêm một khối diagnostic ở trên: nền `<mức>.background`, viền 1 px `<mức>.border`, bo góc, padding 4×8; nội dung là thông báo (markdown nếu server gửi markdown) và dòng mờ `source(code)`; `code` là link nếu có `codeDescription.href`. Nhiều diagnostic chồng nhau thì liệt kê lần lượt.

**Status bar** (bên trái, sau các nút panel):
- `⊗ 3  ⚠ 12` (icon Lucide `circle-x` màu `error`, `triangle-alert` màu `warning`, số màu `text.muted`); không có lỗi thì một icon `check` mờ. Chỉ hiện khi có ít nhất một server từng chạy.
- Ngay sau đó: dòng đầu thông báo của diagnostic tại con trỏ, mờ, cắt ở 80 ký tự.
- Bấm số đếm: chạy `editor.action.marker.nextInFiles`. Bấm thông báo: chạy `editor.action.marker.next`.
- Cập nhật theo đợt 50 ms.

**F8:** `editor.action.marker.nextInFiles` (F8) / `prevInFiles` (⇧F8) đi qua các file theo thứ tự đường dẫn, mở file (preview) khi cần; `editor.action.marker.next` (⌥F8) / `prev` (⇧⌥F8) chỉ trong file. Đặt con trỏ ở đầu range, cuộn vào giữa, và mở hover diagnostic tại đó ngay (không trễ). Ghi vào lịch sử điều hướng.

### 7.2 Hover

- **Kích hoạt:** chuột dừng trên một ký tự có chữ. Sau `editor.hover.delay / 2` thì gửi `textDocument/hover` (và tìm diagnostic tại chỗ); sau đủ `editor.hover.delay` thì hiện nếu đã có kết quả, không thì hiện ngay khi kết quả về. Chuột rời ký tự trước khi hiện thì hủy (drop request → `$/cancelRequest`). Chuột vẫn trong range của kết quả đang hiện thì không gửi lại.
- `editor.action.showHover` (⌘K ⌘I): hiện ngay tại con trỏ, không trễ.
- **Ẩn:** chuột rời range. Khi `editor.hover.sticky` bật, rê chuột vào popover giữ nó lại; rời popover hoặc range thì ẩn sau `editor.hover.hidingDelay`. Cuộn editor, gõ phím, Esc, hoặc mất focus thì ẩn ngay.
- **Hình thức** (theo Zed): dùng `popover` của `ui.rs` (nền `elevated_surface.background`, viền, bo góc, bóng) nhưng đặt tại con trỏ: phía trên dòng nếu đủ chỗ, không thì phía dưới; cách mép phải editor 8 px. Padding 8 px. Rộng tối đa min(nửa editor, 120 ký tự), tối thiểu 20 ký tự; cao tối đa min(nửa editor, 16 dòng), cuộn được bằng chuột. Khối diagnostic (nếu có) nằm trên, cách 6 px. Range đang hover có nền `element.hover`.
- **Nội dung:** `MarkupContent` markdown hoặc chữ thường; `MarkedString` cũ thì code block theo ngôn ngữ. Cắt ở 100 000 byte. Render bằng `markdown.rs`:
  - đoạn văn font UI; tiêu đề in đậm, cỡ tăng nhẹ; danh sách; trích dẫn mờ có vạch trái; đường kẻ;
  - code inline: font code, nền `editor.background`;
  - khối code: font code, tô màu bằng `lucerna-syntax` theo tên ngôn ngữ của fence (không có thì theo ngôn ngữ của file), không wrap, cuộn ngang trong khối;
  - link: gạch chân; bấm thì mở trình duyệt (`http`, `https`) hoặc mở file (`file://…#L10`) trong Lucerna;
  - chọn và copy chữ: ngoài phạm vi.
- Settings: `editor.hover.enabled` (`true`), `editor.hover.delay` (300), `editor.hover.sticky` (`true`), `editor.hover.hidingDelay` (300).

### 7.3 Đi tới định nghĩa và ⌘-click

**Lệnh** (`goto.rs`), tất cả lấy vị trí con trỏ của editor đang focus:

| Lệnh | Request | Nhiều kết quả |
|---|---|---|
| `editor.action.revealDefinition` | `definition` | picker |
| `editor.action.peekDefinition` | `definition` | luôn picker (kể cả 1 kết quả) |
| `editor.action.revealDefinitionAside` | `definition` | picker; mở ở group bên cạnh |
| `editor.action.goToTypeDefinition` / `peekTypeDefinition` | `typeDefinition` | picker / luôn picker |
| `editor.action.goToImplementation` / `peekImplementation` | `implementation` | picker / luôn picker |
| `editor.action.revealDeclaration` / `peekDeclaration` | `declaration` | picker / luôn picker |

- Kết quả `Location` hoặc `LocationLink` (dùng `targetSelectionRange`). Gộp từ mọi server hỗ trợ, bỏ trùng.
- **Không có kết quả:** popover nhỏ ở con trỏ "No definition found for '`word`'" (theo kiểu: "No type definition found…", "No implementation found…", "No declaration found…"), tự tắt sau 3 s hoặc khi di chuyển. Khi chạy từ ⌘-click thì không hiện.
- **Kết quả duy nhất trùng vị trí con trỏ** (lệnh không phải peek): chạy `editor.action.goToReferences` thay thế (như VS Code `alternativeDefinitionCommand` và Zed `go_to_definition_fallback`).
- **Một kết quả:** mở file theo `OpenMode::Preview` (setting `workbench.editor.enablePreview`), đặt con trỏ đầu range, cuộn vào giữa. Ghi lịch sử điều hướng trước khi nhảy.
- Setting `editor.gotoLocation.multipleDefinitions`, `multipleTypeDefinitions`, `multipleImplementations`, `multipleDeclarations`, `multipleReferences`: `peek` (mặc định, mở picker), `gotoAndPeek` (nhảy tới kết quả đầu và mở picker), `goto` (chỉ nhảy tới kết quả đầu).

**⌘-hover / ⌘-click** (`code_view/link.rs`):
- Giữ ⌘ (Ctrl trên Linux/Windows) và rê chuột: gửi `definition` cho vị trí đó (hủy request cũ khi đổi vị trí; cache theo range của kết quả). Có kết quả thì range gốc (`originSelectionRange`, không có thì từ trong hover) được gạch chân thẳng 1 px và đổi màu `link_text.hover`; con trỏ chuột thành bàn tay. Nhả ⌘ hoặc rời range thì bỏ.
- ⌘-click: đi tới định nghĩa (như `revealDefinition`); ⌥⌘-click: mở ở group bên cạnh. ⌘-click không trúng range có kết quả thì giữ hành vi click cũ.

**Lịch sử điều hướng** (`nav_history.rs`):
- Mỗi group có một ngăn xếp `back` / `forward` các `(path, Point, scroll_top_row)`, tối đa 50 mục.
- Ghi trước mỗi lần nhảy bằng lệnh: định nghĩa, tham chiếu, symbol (`@`, `#`, breadcrumbs), F8, Quick Open (file, `:dòng`), kết quả Search. Nhảy trong cùng file chỉ ghi khi cách ít nhất 10 dòng (Zed). Ghi mới thì xóa `forward`.
- `workbench.action.navigateBack` (⌃-) và `workbench.action.navigateForward` (⌃⇧-): mở file nếu cần (không preview), đặt con trỏ, khôi phục cuộn. File không còn thì bỏ qua mục đó.

### 7.4 Picker vị trí (`locations_picker.rs`)

Theo `lsp_locations` của Zed, dùng `popover`, `picker_input_row`, `picker_list`, `picker_row`:
- Modal ở vị trí như Quick Open, nhưng rộng hơn: min(90% cửa sổ, 1100 px), cao min(70% cửa sổ, 560 px).
- Tiêu đề trong ô nhập (placeholder): "Filter definitions…", "Filter references…", "Filter implementations…", ….
- **Bên trái (40%)**: danh sách gom theo file. Hàng header file (không chọn được): icon file, tên file, thư mục mờ, số kết quả ở bên phải. Hàng kết quả: số dòng mờ căn phải (rộng theo số chữ số lớn nhất), rồi nội dung dòng đã bỏ khoảng trắng đầu, font code, phần trong range có nền `search.match_background`. Đọc nội dung dòng ở nền; chưa có thì hàng chỉ có số dòng.
- **Bên phải (60%)**: preview là một `CodeView` chỉ đọc của file đang chọn (mở qua `Documents`, chia sẻ văn bản và cú pháp với tab đã mở), cuộn để range ở giữa, các dòng của range có nền `editor.highlighted_line.background`. Viền trái 1 px `border`.
- Gõ để lọc mờ trên "tên file + nội dung dòng". `↑`/`↓` đổi kết quả (bỏ qua header), preview theo. `Enter` mở (preview tab), `⌘Enter` mở ở group bên cạnh, `Esc` đóng. Bấm một hàng thì chọn; bấm đúp thì mở.
- Kết quả sắp theo file (file hiện tại đứng đầu, sau đó theo đường dẫn), rồi theo vị trí. Chọn sẵn kết quả đầu tiên.

### 7.5 References

- `editor.action.goToReferences` (⇧F12): `textDocument/references` với `includeDeclaration: true`. Một kết quả thì nhảy; nhiều thì picker (theo `editor.gotoLocation.multipleReferences`); không có thì "No references found for '`word`'".
- `references-view.findReferences` (⇧⌥F12): mở **tab References**:
  - tối đa một tab trong mọi group, dùng lại mỗi lần (giống tab Search); tiêu đề tab: icon + "References to `word`"; không lưu vào workspace state;
  - đầu tab là một hàng tóm tắt cao 32 px theo `search_toolbar`: "12 references in 4 files" và nút gập/mở tất cả, nút refresh (chạy lại request tại vị trí cũ);
  - thân tab là `excerpt_list`: header file + các đoạn code, mỗi tham chiếu có 1 dòng ngữ cảnh trên dưới, dòng tham chiếu tô `search.match_background` ở range;
  - phím và chuột như tab Search: `↑`/`↓`, `Enter` mở, F4 / ⇧F4 (`references-view.next` / `prev`) mở kết quả kế / trước.
- **Tách `excerpt_list` khỏi `search_view`:** phần model hàng (`Row`, `FileResult`, `Results`, gập/mở, bước kế/trước), phần render header file và dòng code, và hàng đợi tô cú pháp nền chuyển vào `excerpt_list/`. Tab Search giữ thanh query, toggle, debounce, stream kết quả. Hành vi và giao diện tab Search không đổi; test hiện có của tab Search phải xanh nguyên. `lucerna-project::Excerpt` và `LineMatch` dùng chung; tab References tự dựng excerpt từ các range (đọc file ở nền, gộp các đoạn chồng hoặc kề nhau).

### 7.6 Symbol, Quick Open `@` và `#`, breadcrumbs

**Nguồn symbol của một document** (`symbols/source.rs`): có server hỗ trợ `documentSymbol` thì dùng nó; không thì `SyntaxState::outline` (nếu ngôn ngữ có `outline.scm`); không thì không có. Kết quả cache theo `(path, revision)`, tính lại sau khi document mở, reload, hoặc server vừa sẵn sàng; request debounce 300 ms. Một số server trả rỗng khi đang index: cache rỗng đó có thời hạn 2 s.

**Quick Open `@`** (`workbench.action.gotoSymbol`, ⇧⌘O; gõ `@` trong Quick Open):
- `parse_query` thêm `QuickOpenQuery::Symbols { text, group_by_kind }` (`@:` thì gom nhóm).
- Hàng: icon loại (Lucide, màu `icon.muted`), tên (font code, các ký tự khớp tô `text.accent` ở 30% độ đục như Zed), `detail` mờ cùng dòng. Thụt lề 12 px mỗi cấp khi không lọc. Khi lọc thì không thụt lề, nhưng có tên tổ tiên mờ ở cuối hàng ("in `impl Foo`").
- Lọc mờ trên "tổ tiên + tên"; chữ khớp phải nằm trong tên khi truy vấn là một từ (như Zed).
- `@:`: các header nhóm theo loại "functions (12)", "structs (3)"…, thứ tự theo `SymbolKind`.
- Không có truy vấn: chọn sẵn symbol sâu nhất chứa con trỏ.
- **Xem trước khi di chuyển:** chọn một hàng thì editor đang mở cuộn tới symbol đó (giữa màn), và các dòng từ đầu tới cuối `range` của symbol có nền `editor.highlighted_line.background`. `Enter` đặt con trỏ ở đầu `selectionRange` và ghi lịch sử điều hướng. `Esc` trả lại vị trí cuộn và con trỏ cũ.
- Trạng thái rỗng: "No symbols found" / đang chờ server: "Loading symbols…" / không có nguồn: "No language server or outline for this file".

**Quick Open `#`** (`workbench.action.showAllSymbols`, ⌘T; gõ `#`):
- `QuickOpenQuery::WorkspaceSymbols(text)`. Truy vấn rỗng: "Type to search symbols".
- Debounce 100 ms, gửi `workspace/symbol { query }` tới mọi server `Running` có capability; truy vấn mới thì drop request cũ (tự hủy). Kết quả về thì lọc mờ lại ở client, tối đa 100, file trong project (không bị ignore) đứng trước file ngoài.
- Hàng hai dòng (theo Zed): dòng 1 icon loại + tên (font code, chữ khớp tô) + `containerName` mờ; dòng 2 đường dẫn tương đối + `:dòng`, cỡ nhỏ, mờ.
- `Enter` mở file (preview), đặt con trỏ; `⌘Enter` mở ở group bên cạnh. Không xem trước khi di chuyển (như VS Code).
- `WorkspaceSymbol` chỉ có `uri` (chưa có range) thì gọi `workspaceSymbol/resolve` nếu server hỗ trợ, không thì mở ở dòng 1.
- Không có server nào chạy: "No language server is running".

**Breadcrumbs** (`symbols/breadcrumbs.rs`):
- Một hàng cao 32 px ngay dưới thanh tab của mỗi group, chỉ cho tab code (không cho tab Search, References, Extensions). Nền `editor.background`, viền dưới 1 px `border_variant`, padding ngang 8 px, chữ cỡ UI nhỏ, màu `text.muted`.
- Nội dung: đường dẫn tương đối của file chia theo thư mục (`crates › lucerna-lsp › src › server.rs`, tên file màu `text`), rồi chuỗi symbol tại con trỏ (`path_at`) với icon loại 12 px và tên font code. Dấu ngăn `›` màu placeholder (`text.muted` ở 60%). Quá 12 đoạn thì giữ 6 đầu, 6 cuối, giữa là `⋯`. Tràn ngang thì cắt bên trái.
- Cả hàng là một nút (hover `ghost_element.hover`): bấm thì mở Quick Open `@`. `breadcrumbs.focusAndSelect` (⇧⌘.) cũng mở `@`.
- Cập nhật khi con trỏ đổi (đợt 50 ms) hoặc cây symbol đổi.
- Settings: `breadcrumbs.enabled` (`true`), `breadcrumbs.filePath` (`on` | `off` | `last`), `breadcrumbs.symbolPath` (`on` | `off` | `last`), `breadcrumbs.icons` (`true`).

### 7.7 Trạng thái server và log (`lsp_status.rs`)

- **Nút ⚡** ở status bar bên phải (trước ngôn ngữ), dùng `icon_button` với icon Lucide `zap`, chỉ hiện khi có ít nhất một server được chọn cho các file đã mở, hoặc khi thư mục đang Restricted. Chấm 6 px ở góc: đỏ (`error`) khi có server `Failed`; vàng (`warning`) khi `NotFound` hoặc Restricted; không có chấm khi mọi thứ ổn. Tooltip: "Language Servers".
- **Chữ tiến trình** cạnh nút, mờ: tiến trình `$/progress` mới nhất, "rust-analyzer: Indexing 45%" (`title` + `percentage` + `message`, cắt ở 60 ký tự), "+2" nếu có thêm; khi đang khởi động: "Starting rust-analyzer…". Ẩn khi không có gì.
- **Popover** (bấm nút): danh sách server theo root. Mỗi hàng: chấm trạng thái (xanh Running, vàng Starting / NotFound, đỏ Failed, xám Stopped), tên server, root rút gọn mờ, và ở bên phải các nút chữ nhỏ "Restart", "Stop", "Log". Hàng `NotFound` / `Failed` có thêm dòng thông báo lỗi (§4). Cuối popover: "Restart All", "Stop All". Khi Restricted: một hàng "Restricted Mode — Trust this folder…" mở modal tin cậy.
- **Tab log** (`lucerna:lsp-log/<server>`): mỗi server một tab, chỉ đọc, font code, cuộn được, tự cuộn xuống cuối khi đang ở cuối. Giữ 2000 dòng gần nhất: stderr, `window/logMessage`, `window/showMessage`, và các sự kiện vòng đời (đường dẫn binary, tham số, root, phiên bản server, mã thoát). Không lưu workspace state. Copy toàn bộ bằng ⌘A ⌘C.
- Lệnh: `lucerna.lsp.restart` (server của file hiện tại), `lucerna.lsp.restartAll`, `lucerna.lsp.stopAll`, `lucerna.lsp.showLog`.

### 7.8 Vim

Trong normal mode (khi `vim.enabled`):
- `gd`: `editor.action.revealDefinition`; `gD`: `editor.action.revealDeclaration` (VSCodeVim).
- `gh`: `editor.action.showHover` (VSCodeVim).
- `]d` / `[d`: `editor.action.marker.next` / `prev` (Zed).
- `⌃o` / `⌃i` (Vim jump list): `navigateBack` / `navigateForward`.

## 8. Lệnh, phím và settings

**Phím mới** (macOS; `default-keybindings.json`, `when` có `editorTextFocus` cho lệnh editor):

| Lệnh | Phím |
|---|---|
| `editor.action.revealDefinition` | `f12` |
| `editor.action.peekDefinition` | `alt+f12` |
| `editor.action.revealDefinitionAside` | `cmd+k f12` |
| `editor.action.goToImplementation` | `cmd+f12` |
| `editor.action.peekImplementation` | `shift+cmd+f12` |
| `editor.action.goToReferences` | `shift+f12` |
| `references-view.findReferences` | `shift+alt+f12` |
| `editor.action.showHover` | `cmd+k cmd+i` |
| `editor.action.marker.nextInFiles` / `prevInFiles` | `f8` / `shift+f8` |
| `editor.action.marker.next` / `prev` | `alt+f8` / `shift+alt+f8` |
| `workbench.action.gotoSymbol` | `shift+cmd+o` |
| `workbench.action.showAllSymbols` | `cmd+t` |
| `breadcrumbs.focusAndSelect` | `shift+cmd+.` |
| `workbench.action.navigateBack` / `navigateForward` | `ctrl+-` / `ctrl+shift+-` |
| `references-view.next` / `prev` | `f4` / `shift+f4` (khi tab References có focus) |

Không có phím: `goToTypeDefinition`, `peekTypeDefinition`, `revealDeclaration`, `peekDeclaration`, `workbench.trust.manage`, `workbench.trust.clear`, `lucerna.lsp.*`. Tất cả có trong Command Palette.

**Settings mới** (`default-settings.json`):

| Khóa | Mặc định | Ghi chú |
|---|---|---|
| `lucerna.lsp.enabled` | `true` | Ghi đè được theo ngôn ngữ (`"[rust]": { … }`) |
| `lucerna.lsp.languageServers` | `["..."]` | Theo ngôn ngữ. Thứ tự là ưu tiên; `"!tên"` tắt; `"..."` là các server còn lại của ngôn ngữ, trừ server mặc định tắt. Mặc định `[python]`: `["basedpyright", "!pyright", "..."]` |
| `lsp` | `{}` | Object theo shape của Zed: `"lsp": { "<server>": { "binary": { "path", "arguments", "env" }, "initialization_options", "settings" } }`. Lớp workspace bị bỏ qua khi chưa tin cậy |
| `session.trustAllWorktrees` | `false` | Chỉ đọc ở lớp User |
| `editor.hover.enabled` / `delay` / `sticky` / `hidingDelay` | `true` / `300` / `true` / `300` | |
| `editor.gotoLocation.multipleDefinitions` (+ `TypeDefinitions`, `Implementations`, `Declarations`, `References`) | `peek` | `peek` \| `gotoAndPeek` \| `goto` |
| `lucerna.diagnostics.gutterIcons` | `true` | |
| `lucerna.diagnostics.inline.enabled` | `false` | |
| `breadcrumbs.enabled` / `filePath` / `symbolPath` / `icons` | `true` / `on` / `on` / `true` | |

**Lệch khỏi spec đã trình bày khi brainstorm:** khóa chọn server viết là `lucerna.lsp.languageServers` ghi đè theo ngôn ngữ bằng `"[python]": { … }` (cơ chế override có sẵn của Lucerna, theo VS Code), thay cho `languages.<Lang>.language_servers` của Zed; ngữ nghĩa `!` và `...` giữ nguyên. `lsp.enabled` đổi thành `lucerna.lsp.enabled` vì `lsp` đã là khóa object.

**Context key mới:** `referencesViewFocus` (tab References có focus), `lspHoverVisible`.

## 9. Hiệu năng

- Luồng UI không đọc pipe, không parse JSON, không quy đổi vị trí cho cả file: thread đọc parse JSON; quy đổi diagnostics sang `Point` chạy ở background executor rồi mới vào UI, đã nhóm theo dòng.
- `publishDiagnostics` dồn dập (rust-analyzer gửi hàng trăm file một lúc) được gộp theo đợt 50 ms trước khi vẽ lại.
- Hover chỉ gửi request sau nửa thời gian trễ; ⌘-hover cache theo range. Request cũ luôn bị hủy khi có request mới cùng loại.
- `documentSymbol` debounce 300 ms, cache theo revision.
- Server chỉ khởi động khi cần; app chưa mở file nào có server thì không có tiến trình con và không chạy login shell.
- Chưa có benchmark mới; kiểm bằng tay với rust-analyzer trên chính repo Lucerna: cuộn mượt khi file có hàng trăm diagnostics.

## 10. Xử lý lỗi

- Mọi thread của transport và mọi callback của `LanguageServers` bọc `catch_panic` / `guarded`; panic thì log và coi như server thoát.
- JSON không đọc được: log mức warn, bỏ message, không ngắt kết nối. Header sai: coi như server thoát.
- Request lỗi (`ResponseError`, timeout, server đóng): lệnh không làm gì, log mức debug; riêng go to definition thì hiện popover "No definition found…" chỉ khi kết quả là rỗng, không phải khi lỗi.
- Vị trí ngoài file: cắt về cuối dòng / cuối file.
- Settings sai kiểu (`lsp.x.binary.path` không phải chuỗi…): bỏ giá trị đó, dùng mặc định, báo trong popover ⚡ như lỗi settings khác.
- `trusted.json` hỏng: coi như rỗng, ghi lại khi có thay đổi.

## 11. Kiểm thử

**`lucerna-lsp`** (không cần server thật):
- Binary `fake-lsp-server` (trong `crates/lucerna-lsp/src/bin/`, chỉ build khi test): đọc kịch bản JSON từ biến môi trường (trả lời theo method, đẩy notification sau `initialized` hoặc sau `didOpen`/`didChange`, ngủ, crash, gửi request ngược). Ghi mọi message nhận được ra file để test kiểm tra.
- Transport: framing (`Content-Type`, nhiều message trong một lần đọc, body lớn), response đúng id, drop future gửi `$/cancelRequest`, timeout, EOF trả `Closed` cho mọi request đang chờ.
- Server: `initialize` gửi đúng capability; tự trả lời `workspace/configuration` theo `section`; `MethodNotFound` cho request lạ; shutdown đúng thứ tự, kill khi server không trả lời.
- Registry: `languageServers` với `!` và `...`; root theo adapter (Cargo workspace lồng nhau, `go.work`); settings đổi thì restart / didChangeConfiguration / stop đúng trường hợp; `NotFound` khi PATH không có; `binary.path` không bị chạy thử.
- Sync: version tăng; `Full` và `Incremental`; không `didOpen` file lớn; `didChangeWatchedFiles` theo glob.
- Position: UTF-16 với emoji và chữ CJK; UTF-8; ngoài phạm vi.
- Diagnostics: bỏ khi lệch version; `next` qua file và vòng lại, bỏ hint; tổng số.
- Symbols: `SymbolInformation` phẳng thành cây; `path_at`.
- Test với `rust-analyzer` thật trên một crate fixture nhỏ: `#[ignore]`, chạy tay (`cargo test -p lucerna-lsp -- --ignored`).

**`lucerna-syntax`:** `outline` với fixture `outline.scm` + grammar WASM JSON đã có (hoặc grammar bundled + query test), cha con theo range, đoán `kind`.

**`lucerna-settings`:** `get_trusted` bỏ lớp workspace, giữ gộp object.

**`lucerna-workbench`** (`TestAppContext`, `fake-lsp-server`):
- Mở file → `didOpen`; agent ghi file → `didChange`; đóng tab cuối → `didClose`.
- Restricted Mode: không có server; modal hiện khi mở file có server; Trust → server chạy; `trustAllWorktrees`.
- Diagnostics: gạch chân, icon gutter, inline (bật / tắt), số đếm status bar, F8 qua hai file, ⌥F8 trong file.
- Hover: trễ, hủy khi rời, sticky, khối diagnostic, markdown có code block.
- F12 một kết quả nhảy; nhiều kết quả mở picker; trùng chỗ con trỏ thì chạy references; không có thì popover; ⌘-click; back / forward.
- Picker: lọc, preview theo chọn, Enter, ⌘Enter, Esc.
- References tab: tóm tắt, header, Enter mở dòng, F4; tab Search vẫn xanh.
- `@`: cây, lọc, `@:`, xem trước và Esc trả lại; fallback `outline.scm` khi không có server.
- `#`: debounce, hủy request cũ, sắp xếp trong project trước.
- Breadcrumbs: theo con trỏ, bấm mở `@`, settings tắt.
- Nút ⚡: NotFound, Failed với stderr, Restart, tab log.
- Vim: `gd`, `gh`, `]d`.

**Review giao diện:** chạy app với rust-analyzer trên repo Lucerna, đặt cạnh Zed: hover, gạch chân, picker, tab References, breadcrumbs, `@`, `#`, nút ⚡, modal tin cậy. Chụp màn hình lưu cạnh plan.

## 12. Thứ tự triển khai

Mỗi bước tự chạy được và có test:
1. `lucerna-lsp`: transport, server, `fake-lsp-server`, position.
2. Adapter, môi trường login shell, registry, settings (`lsp`, `lucerna.lsp.*`, `get_trusted`), tin cậy thư mục, nút ⚡, tab log.
3. Sự kiện của `Documents`, đồng bộ document, kho diagnostics, màu theme, vẽ diagnostics (gạch chân, gutter, inline, scrollbar), status bar, F8.
4. `markdown.rs`, hover.
5. Đi tới định nghĩa (các biến thể), ⌘-click, picker vị trí, lịch sử điều hướng.
6. Tách `excerpt_list` từ tab Search, tab References.
7. `outline.scm`, nguồn symbol, `@`, breadcrumbs, `#`.
8. Vim, review giao diện cạnh Zed, review toàn nhánh, `scripts/bundle-app.sh --install`.

## 13. Rủi ro

| Rủi ro | Cách giảm |
|---|---|
| `lsp-types` bản mới đổi kiểu (`Uri` thay `Url` từ 0.96) | Ghim một bản, bọc quy đổi đường dẫn ↔ URI trong một hàm |
| PATH của app GUI trên macOS thiếu `~/.cargo/bin`, `~/go/bin`, nvm… | Lấy môi trường login shell; `binary.path` là lối thoát |
| rust-analyzer gửi diagnostics `cargo check` cho file chưa mở | Kho diagnostics giữ cả file chưa mở; số đếm và F8 tính cả chúng |
| Server gửi vị trí theo bản văn bản cũ khi agent ghi file liên tục | Bỏ diagnostics lệch version; server sẽ gửi lại |
| Tách `excerpt_list` làm hỏng tab Search | Làm ở bước riêng, test tab Search phải xanh nguyên trước khi thêm References |
| Login shell chậm hoặc in rác (oh-my-zsh) | Timeout 5 s, đọc `env -0` sau dấu phân cách riêng, lỗi thì dùng môi trường tiến trình |
| Ổ đĩa chật | `lsp-types` và `pulldown-cmark` nhỏ; `fake-lsp-server` là binary của crate, không thêm dependency |

## 14. Thay đổi spec cũ

- Spec Phase 1, §1.3: thêm hàng "3b — Host WASM cho extension Zed: language server từ `extension.wasm` (PHP, Ruby, Elixir, Vue…)" giữa giai đoạn 3 và 4.
- Spec Phase 2, D1: "để sang giai đoạn 3" đổi thành "để sang giai đoạn 3b".
- Trang Extensions: dòng "Language server: Phase 3" đổi thành "Language server: Phase 3b".
