# Nghiên cứu: bảng language server theo dữ liệu (Helix, Zed, VS Code)

- **Ngày:** 2026-11-11
- **Trạng thái:** ghi chú nghiên cứu, làm nền cho việc "language server cho mọi ngôn ngữ" (Phase 3b / sau Phase 4). Phần "Lucerna chọn gì" (§9) chỉ là **đề xuất ban đầu**, sẽ chốt trong spec sau khi thảo luận với người dùng.
- **Nguồn:**
  - Helix: `helix-editor/helix` nhánh `master` lấy ngày 2026-11-11 (bản tag mới nhất là 25.07.1, 2025-07-18); tài liệu https://docs.helix-editor.com/languages.html, wiki https://github.com/helix-editor/helix/wiki/Language-Server-Configurations. Helix là MPL-2.0.
  - Zed: checkout cục bộ `~/.cargo/git/checkouts/zed-a70e2ad075855582/b5cd310/` (`docs/src/configuring-languages.md`, `assets/settings/default.json`, `crates/settings_content/src/project.rs`, `crates/project/src/lsp_store/vue_language_server_ext.rs`). Các crate này là **GPL-3**: chỉ học thiết kế, không chép code.
  - VS Code: https://code.visualstudio.com/api/language-extensions/language-server-extension-guide và các extension generic client. Sublime LSP: https://lsp.sublimetext.io/client_configuration/.
  - Wu: https://github.com/dstrml/wu. Điều khoản Zed: https://zed.dev/terms.
  - Tài liệu upstream của từng server (URL ở cuối file).
- Mục nào chưa kiểm chứng với nguồn gốc trong ngày được đánh dấu **(chưa kiểm chứng)**.
- Bối cảnh: `crates/lucerna-lsp/src/settings.rs` đã có shape của Zed `lsp.<server>.{binary{path,arguments,env}, initialization_options, settings}` cho 5 adapter viết sẵn (rust-analyzer, gopls, pyright/basedpyright, typescript-language-server, clangd). Tài liệu này trả lời câu hỏi: làm sao thêm hàng loạt ngôn ngữ khác mà không viết một adapter Rust cho mỗi server.

## 1. Tóm tắt

| Chủ đề | Helix | Zed | VS Code / Sublime | Lucerna nghiêng về |
|---|---|---|---|---|
| Định nghĩa server mới bằng config | Có: `[language-server.<name>]` + `[[language]]` | **Không**: `lsp.<name>` chỉ chỉnh server do adapter hoặc extension đăng ký | Lõi VS Code không có; extension generic dùng key riêng; Sublime có `clients` | Có, theo shape JSON của Zed + vài key bổ sung |
| Gắn server với ngôn ngữ | `language-servers` trong `[[language]]`, hỗ trợ `only-features` / `except-features` | `languages.<L>.language_servers` với `!name` và `"..."` | Sublime: `selector` (scope) | Giữ `languages.<L>.language_servers` kiểu Zed |
| Merge cấu hình | Mặc định, user, project; gộp theo `name`, mảng bị thay cả | Settings nhiều lớp; mảng `language_servers` của user thay hẳn mặc định | Tùy extension | Gộp object theo key, thay mảng, `!`/`...` chỉ cho `language_servers` |
| Cấu hình server | Một khối `config` dùng cho cả `initializationOptions` và `workspace/configuration` | Tách `initialization_options` và `settings` | Tách (client option và settings) | Tách hai khối như Zed |
| Tin cậy file project | Master (2026-06): hỏi trust cho `.helix/languages.toml`, gắn hash SHA-256 | Restricted Mode | Workspace Trust | File project không được đặt `command`/`binary.path`/`env` khi chưa trust |
| Kiểm tra cài đặt | `hx --health [lang]`: chỉ `which(command)` | Không có view tương đương | Không | View "LSP health" dò `--version`, kèm gợi ý cài |
| Mặc định trong binary | `languages.toml` (MPL-2.0) | Adapter Rust + extension | Extension | Bảng riêng của Lucerna, viết từ tài liệu upstream, không chép file Helix |
| Kéo extension từ `api.zed.dev` | Wu vẫn làm | Có | Không | **Không**: vướng ToS §2.2 và §2.4(b) |

## 2. Helix

Nguồn: docs `languages.html`; `languages.toml` (5.657 dòng), `helix-loader/src/lib.rs` (`merge_toml_values`), `helix-loader/src/config.rs` (`user_lang_config`), `helix-loader/src/workspace_trust.rs`, `helix-term/src/health.rs`, `helix-term/src/application.rs`, `helix-lsp/src/client.rs`.

### 2.1 `[language-server.<name>]`

Trong file mặc định, mọi server nằm trong một bảng `[language-server]` dưới dạng inline table (ví dụ `bash-language-server = { command = "bash-language-server", args = ["start"] }`). Người dùng có thể viết `[language-server.<name>]` như thường.

| Key | Kiểu | Ý nghĩa |
|---|---|---|
| `command` | string | Tên binary (tìm trong `$PATH`) hoặc đường dẫn. Bắt buộc. |
| `args` | string[] | argv sau command. |
| `config` | TOML bất kỳ | Gửi làm `initializationOptions`; đồng thời là nguồn trả lời `workspace/configuration`. Sub-table `format` dùng làm option khi format. |
| `timeout` | số giây | Thời gian tối đa của một request, mặc định 20. |
| `environment` | bảng string sang string | Biến môi trường cho tiến trình. |
| `required-root-patterns` | string[] (glob) | Server chỉ chạy nếu có ít nhất một glob khớp trong workspace. Dùng cho trường hợp như tailwind chỉ chạy khi có file config. |

Không có `roots`, `file-types` hay `language-id` theo từng server; các thứ đó nằm ở ngôn ngữ. Nhờ vậy một server dùng lại được cho nhiều ngôn ngữ (`vscode-css-language-server` cho css/scss/less).

### 2.2 `[[language]]` (phần liên quan LSP)

| Key | Ý nghĩa |
|---|---|
| `name` | Id ngôn ngữ của Helix, đồng thời là khóa merge. |
| `language-servers` | Mảng có thứ tự. Mỗi phần tử là tên server (string) hoặc `{ name = "...", only-features = [...] }` / `{ name = "...", except-features = [...] }` (hai key loại trừ nhau). |
| `roots` | File đánh dấu để tìm root LSP, tìm ngược từ file hiện tại đến workspace root. Master chấp nhận cả glob (C# dùng `["*.slnx", "*.sln", "*.csproj"]`). Thư mục **ngoài cùng** có marker thắng: `find_lsp_workspace` ghi đè kết quả mỗi lần gặp marker khi đi lên, dừng ở workspace root hoặc một thư mục trong `workspace-lsp-roots`. Lucerna theo cách này từ 2026-11-17, xem `docs/superpowers/specs/2026-11-17-lsp-outermost-root-design.md`. |
| `language-id` | `TextDocumentItem.languageId`. Mặc định bằng `name`; chỉ đặt khi khác (`javascriptreact`, `typescriptreact`, `terraform`, `terraform-vars`, `dockercompose`). |
| `file-types` | Mảng đuôi file (`"rs"`) hoặc `{ glob = "..." }`. Glob được kiểm trước đuôi file. |
| `workspace-lsp-roots` | Thư mục con coi là root LSP riêng (monorepo). Đặt ở `[editor]` hoặc theo ngôn ngữ. |

Tên feature của `only-features` / `except-features` (chính xác): `format`, `goto-definition`, `goto-declaration`, `goto-type-definition`, `goto-reference`, `goto-implementation`, `signature-help`, `hover`, `document-highlight`, `completion`, `code-action`, `workspace-command`, `document-symbols`, `workspace-symbols`, `diagnostics`, `rename-symbol`, `inlay-hints`.

Cách điều phối (theo docs): với đa số feature, server đầu tiên trong danh sách hỗ trợ request đó sẽ thắng. Riêng `diagnostics`, `code-action`, `completion`, `document-symbols`, `workspace-symbols` được gửi cho mọi server đang bật rồi gộp kết quả. Ví dụ trong bảng mặc định: Erlang liệt kê `erlang-ls` và `elp`, cả hai đều `except-features = ["document-symbols", "workspace-symbols"]`.

### 2.3 Merge ba lớp

`user_lang_config` nạp `languages.toml` biên dịch sẵn trong binary (`include_bytes!`), rồi chồng `~/.config/helix/languages.toml` và (nếu được trust) `<workspace>/.helix/languages.toml`, mỗi lớp bằng `merge_toml_values(left, right, 3)`. Lớp sau thắng. Quy tắc:

- Bảng + bảng, depth > 0: hợp key; key có ở cả hai thì merge đệ quy với depth-1.
- Mảng + mảng, depth > 0: với mỗi phần tử bên phải có key `name`, phần tử cùng `name` bên trái bị bỏ và kết quả đã merge được nối vào cuối. Phần tử không trùng tên thì nối thêm. Vì thế `[[language]] name = "php"` trong file user merge vào mục php mặc định thay vì tạo mục thứ hai.
- depth 0 hoặc các cặp giá trị khác (string, số, mảng string): giá trị bên phải thay thế.

Hệ quả (depth 3 tính từ gốc tài liệu):
- `[language-server.foo]`: từng key của server được merge; `args` bị thay cả mảng; sub-table của `config` sâu hơn depth 3 bị thay, không merge.
- `[[language]]` merge theo `name`; các mảng `language-servers`, `roots`, `file-types` bị thay cả mảng (không có cú pháp append, không có `!`/`...` như Zed). Muốn thêm server phải viết lại cả danh sách.
- Project áp dụng sau cùng nên project thắng user.

### 2.4 Trust

Bản 25.07.1 nạp `.helix/languages.toml` vô điều kiện. Master thêm workspace trust (PR https://github.com/helix-editor/helix/pull/15857, merge 2026-06-20):

- `TrustQuery` gồm: language server, debug adapter, config `.helix/` cục bộ, git config.
- `ImplicitTrustLevel`: `None` (hỏi mọi thứ), `Servers` (mặc định: LSP/DAP từ config **global** chạy ngầm vì binary do người dùng cài và tìm qua PATH; config cục bộ của workspace vẫn cần trust rõ ràng), `Insecure`.
- Trust gắn với hash SHA-256 của cây `.helix/` (kiểu direnv): đổi nội dung sau khi trust thì thành "stale" và phải duyệt lại, nên kéo một nhánh độc hại về không thể lặng lẽ đổi `command`.
- Có cờ `prompt` và glob các đường dẫn tin cậy. `hx --health` chạy với `WorkspaceTrust::fully_trusted()`.

### 2.5 `config`: initializationOptions và workspace/configuration

- `initialize` gửi `initialization_options: self.config.clone()` (`helix-lsp/src/client.rs`).
- Với `workspace/configuration`, Helix lấy chính giá trị `config` rồi đi theo `item.section.split('.')`. Section rỗng trả cả `config`; đường dẫn không có thì trả `null`; bỏ qua `scopeUri` (`helix-term/src/application.rs`).
- Helix khai báo dynamic registration cho `didChangeConfiguration` và gọi nó khi reload config.
- Tức là Helix dùng **một khối** cho cả hai việc. Cách này chạy được với server dùng namespace có tiền tố (`Lua.*`, `elixirLS.*`, `yaml.*`), nên entry mặc định có dạng `config = { elixirLS.dialyzerEnabled = false }`. Zed và VS Code tách riêng; cách tách của Lucerna đúng hơn, giữ nguyên.

### 2.6 `hx --health`

- Phần đầu: `Config file:`, `Language file:`, `Log file:`, thư mục runtime, clipboard provider.
- Bảng cột `Language`, `Language servers`, `Debug adapter`, `Formatter`, rồi mỗi loại query tree-sitter một cột.
- Cột language server in server đầu tiên là `✓ name` hoặc `✘ name`, server phụ nằm ở các dòng tiếp. Kiểm tra chỉ là `which(command)` trên `$PATH`: không spawn server, không chạy `--version`.
- `hx --health <lang>` in `Configured language servers:` kèm `✓ name: /đường/dẫn/đã/resolve` hoặc `✘ name: 'cmd' not found in $PATH`, rồi tương tự cho debug adapter, formatter, file query.
- Không có đối số thì lọc theo `use-grammars`; `all` / `all-languages` hiện tất cả.

Lucerna đã dò bằng `--version` (`probe_args`), chặt hơn Helix. View "LSP health" có thể theo `hx --health <lang>`: server đã cấu hình, đường dẫn đã resolve hoặc "not found on PATH", kèm gợi ý cài từ bảng.

### 2.7 Wiki "Language Server Configurations"

Trang cộng đồng, có hướng dẫn cài và đoạn `languages.toml` cho từng ngôn ngữ. Các điểm dùng ở §7: Phpactor qua `phpactor.phar` với `args = ["language-server"]`; intelephense qua npm, bản premium cần `licenceKey`; jdtls qua `brew install jdtls`; Elixir qua `elixir-ls.zip` và đổi tên `language_server.sh`; Vue cần `@vue/language-server` + `@vue/typescript-plugin` đăng ký làm plugin của `typescript-language-server`; taplo cần bản build có feature `lsp` (`cargo install taplo-cli --locked --features lsp`, hoặc `brew install taplo`); Gleam dùng `gleam lsp` từ v0.21; Docker Compose có server riêng `docker-compose-langserver --stdio`. Wiki là nội dung người dùng đóng góp: chỉ coi là gợi ý, không phải nguồn sự thật.

### 2.8 Giống và khác với Lucerna hiện tại

- Giống: server là một map có tên; ngôn ngữ giữ danh sách server có thứ tự.
- Khác: Helix tách `command` khỏi binding ngôn ngữ và root, Lucerna/Zed gom mọi thứ vào `lsp.<server>`. Helix gộp `config` một khối, Lucerna tách đôi. Helix không có `!`/`...`.

## 3. Giấy phép: lấy bảng mặc định từ Helix

Helix là MPL-2.0 (LICENSE của repo). `languages.toml` không có header riêng (chỉ hai dòng comment trỏ tới docs), nên áp dụng MPL của cả repo. Đây không phải tư vấn pháp lý.

1. **Dữ kiện không có bản quyền.** Tên server, file thực thi, cờ bắt buộc (`--stdio`, `serve`, `lsp stdio`), languageId và file đánh dấu của build tool là dữ kiện do chính dự án server công bố. Luật Mỹ (Feist v. Rural, 1991) và đa số nước không bảo hộ dữ kiện, chỉ bảo hộ cách chọn lọc, sắp xếp và biểu đạt gốc. Phần dễ được bảo hộ nhất của một file TOML ~300 mục là cách sắp xếp, comment và các giá trị tinh chỉnh `config` đặc thù của Helix.
2. **MPL-2.0 là copyleft cấp file.** Nếu Lucerna chép `languages.toml` (hoặc một khối lớn) vào, ví dụ, `crates/lucerna-lsp/assets/servers.toml` thì file đó vẫn là MPL-2.0: phải công bố mã nguồn của file theo MPL, giữ nguyên thông báo, sửa đổi file cũng là MPL. MPL §3.3 cho phép file MPL nằm trong "Larger Work" dưới MIT/Apache nên tương thích, nhưng:
   - crate thành kiểu `MIT OR Apache-2.0` kèm MPL-2.0, `cargo deny` phải cho phép MPL-2.0, trường `license` và NOTICE phải sửa;
   - phá câu chuyện giấy phép gọn "MIT OR Apache-2.0" của dự án.
3. **Cách làm đề xuất:**
   - **Không chép file Helix.** Mỗi mục trong bảng mặc định lấy từ tài liệu upstream của chính server (README, docs), là dữ kiện công bố. Ghi URL upstream vào trường `docs` của mục đó; trường này đồng thời là link gợi ý cài trong UI.
   - Dùng Helix chỉ để kiểm tra chéo "lệnh này đúng chưa"; không chép file, thứ tự, comment hay khối `config` của Helix.
   - Đối chiếu được với nvim-lspconfig (https://github.com/neovim/nvim-lspconfig, Apache-2.0); mason-registry cũng Apache-2.0 (chưa kiểm chứng). Cả hai tương thích giấy phép. Nếu có lúc suy ra máy móc từ một trong hai thì nvim-lspconfig là nguồn an toàn, kèm dòng ghi nguồn trong NOTICE hoặc header bảng ("lệnh server được đối chiếu với nvim-lspconfig, Apache-2.0").
   - Một dòng cảm ơn ("lấy cảm hứng từ `languages.toml` của Helix") trong mục Tham khảo của spec là lịch sự và không tạo nghĩa vụ.
4. Tương đương của Zed (`crates/languages`, extensions) là GPL/Apache hỗn hợp; theo CLAUDE.md chỉ học thiết kế.

## 4. Zed

Nguồn cục bộ: `docs/src/configuring-languages.md`, `assets/settings/default.json`, `crates/settings_content/src/project.rs`.

- `lsp.<server-name>` (`LspSettings`):
  - `binary: { path, arguments, env, ignore_system_version }` (`BinarySettings`): `path` thay binary tải về hoặc trên PATH, `arguments` thay args, `env` thêm biến môi trường.
  - `initialization_options`: JSON merge vào mặc định của adapter rồi gửi trong `initialize`.
  - `settings`: JSON trả cho `workspace/configuration` (và đẩy qua `didChangeConfiguration`).
  - `enable_lsp_tasks` (mặc định true), `fetch.pre_release`.
- `languages.<Language>.language_servers`: danh sách tên có thứ tự; `"!name"` tắt server; `"..."` mở rộng thành mọi server khác đã đăng ký cho ngôn ngữ đó mà chưa được nêu (trừ server tắt theo mặc định nếu không nêu tên). Danh sách của người dùng **thay hẳn** danh sách mặc định (các `!x` mặc định mất nếu không viết lại). `language_servers: ["..."]` ở cấp cao nhất là mặc định toàn cục.
- `global_lsp_settings`: `button`, `request_timeout` (mặc định 120 s, 0 = không timeout), `max_buffer_line_length` (20000), thông báo, rule semantic token.
- **Xác nhận: không thể định nghĩa server mới chỉ bằng settings.** `LspSettings` không có trường nào gắn server với ngôn ngữ, đuôi file hay root marker; `language_servers` chỉ sắp xếp và lọc tên các server do adapter dựng sẵn hoặc extension đăng ký. Một entry `lsp.foo` với `foo` lạ là vô hiệu. Con đường duy nhất là viết extension (Rust biên dịch ra WASM, `language_server_command`).

Giống và khác: Lucerna đã có `ServerSettings { binary, initialization_options, settings }` giống hệt shape này. Khác biệt cần bù là khả năng khai báo server mới (Helix/Sublime có, Zed không).

## 5. VS Code, Sublime LSP, Wu và điều khoản Zed

### 5.1 VS Code và các generic client

Lõi VS Code không có generic LSP client; mỗi server đi kèm một extension dựng trên `vscode-languageclient` (`ServerOptions = { command, args, options: { env, cwd } }`, `LanguageClientOptions = { documentSelector, initializationOptions, synchronize.configurationSection }`). `workspace/configuration` được trả lời từ settings của VS Code theo tên section (`yaml` ra các setting `yaml.*`).

Các extension generic client (không có schema chuẩn):

| Extension | Settings | Shape |
|---|---|---|
| Generic LSP Client "glspc" (llllvvuu/vscode-glspc) | `glspc.serverCommand`, `glspc.serverCommandArguments`, `glspc.languageId`, `glspc.environmentVariables` (mở rộng `$VAR`), `glspc.initializationOptions`, `glspc.trace.server` | một server mỗi workspace |
| vscode-lsp-config (pepebecker) | `lspConfig.servers: [{ language, cmd, args }]` | danh sách |
| Alloglot (MercuryTechnologies) | `alloglot.languages: [{ languageId, serverCommand, formatCommand, onSaveCommand, ... }]`, `alloglot.mergeConfigs` merge `.vscode/alloglot.json` | danh sách, thiên về Haskell |
| langoustine-vscode | `langoustine-vscode.servers` khóa theo đuôi file | danh sách |

### 5.2 Sublime LSP và Neovim

- Sublime LSP `clients.<name>`: `enabled`, `command` (mảng argv), `selector` (scope, ví dụ `source.php`), `env`, `settings`, `initialization_options`, `priority_selector`, `diagnostics_mode`, `disabled_capabilities`, `experimental_capabilities`, `tcp_port`.
- Neovim 0.11+ `vim.lsp.config(name, { cmd, filetypes, root_markers, settings, init_options })` + `vim.lsp.enable(name)` (chưa kiểm chứng), với danh mục nvim-lspconfig (Apache-2.0) làm mặc định.
- **Mẫu số chung** của Helix, Sublime, Neovim, glspc: `command` + `args` (hoặc argv), gắn ngôn ngữ/đuôi file, root marker, `env`, `initialization_options`, `settings`. Đó là schema tối thiểu của Lucerna.

### 5.3 Wu (github.com/dstrml/wu)

Nguồn: README, `assets/settings/default.json`, `crates/http_client/src/http_client.rs`, `crates/extension_host/src/extension_host.rs`.

- README: "a fork of Zed" "diverges heavily from it"; ghi công đội Zed; mô tả "The fast, native code editor and agent workspace"; hỗ trợ sẵn agent Claude Code, Codex, OpenCode; tuyên bố "No account or telemetry".
- Giấy phép: GPL-3.0-or-later kèm phần Apache-2.0 (LICENSE-GPL, LICENSE-APACHE). Repo không phải đối tượng "fork" trên GitHub (`fork: false`), tạo 2026-08-28, push gần nhất 2026-11-10, khoảng 640 sao.
- Extension: **vẫn trỏ về server của Zed theo mặc định.** `assets/settings/default.json` có `"server_url": "https://zed.dev"` (ghi đè bằng env `ZED_SERVER_URL`); `http_client.rs` giữ `build_zed_api_url`, ánh xạ `https://zed.dev` sang `https://api.zed.dev` (và cloud.zed.dev cho endpoint khác). `extension_host.rs` gọi `/extensions/updates` và các endpoint extension khác qua `build_zed_api_url`. Tức gallery extension của Wu chính là registry của Zed phục vụ từ `api.zed.dev`.
- Khác Zed (theo README, chưa audit): đổi thương hiệu (action có namespace `wu:`), tính năng agent workspace, bỏ telemetry và tài khoản.
- **Lucerna không theo hướng này.** Lý do ở §5.4.

### 5.4 Điều khoản của Zed

Nguồn: https://zed.dev/terms ("Last Updated: March 2, 2026", tức 2026-03-02).

- §2.2 (Access Grant): "Customer agrees to access the Service only through the mechanisms designated by Zed."
- §2.4 (Restrictions), mục (b): "(b) seek to access non-public APIs associated with the Service;"

Tài liệu công khai cho endpoint extension của `api.zed.dev`: docs của Zed mô tả quy trình *xuất bản* extension (PR submodule vào `zed-industries/extensions`, CI của Zed đóng gói) và crate `zed_extension_api` cho tác giả (https://zed.dev/docs/extensions/developing-extensions, https://zed.dev/docs/extensions/publishing/prerequisites). **Không thấy** tài liệu công khai cho các endpoint HTTP mà app dùng (`/extensions`, `/extensions/updates`, `/extensions/<id>/download`...); chúng chỉ lộ ra trong mã nguồn Zed. Hiểu chúng là "non-public APIs associated with the Service" là cách đọc thận trọng; editor bên thứ ba gọi chúng tốt nhất là vùng xám, xấu nhất là vi phạm §2.4(b) (điều khoản ràng buộc người chấp nhận). Đây là một lý do để Lucerna dùng PATH + bảng thay vì kéo extension từ `api.zed.dev`. Điều khoản này tồn tại công khai; điều không công khai là tài liệu API của endpoint.

## 6. Các phương án "language server cho mọi ngôn ngữ"

Người dùng đã duyệt: **chọn f trước, d sau Phase 4** (nếu cần "cài là chạy").

| | Phương án | Ưu | Nhược |
|---|---|---|---|
| a | Bắt buộc có `cargo` để build `extension.wasm` từ repo extension | Dùng được hệ extension Zed nguyên bản | Người dùng phải cài Rust toolchain, build chậm, đĩa nặng |
| b | Lucerna tự cài toolchain (rustup, wasm target) | Người dùng không phải làm gì | Tải vài trăm MB ngầm, nhiều điểm hỏng, khó hỗ trợ |
| c | Dùng `api.zed.dev` (như Wu) để lấy archive build sẵn | Dễ nhất, có sẵn kho extension | Vướng ToS §2.2 và §2.4(b); phụ thuộc hạ tầng của Zed |
| d | CI của Lucerna build sẵn `extension.wasm` và tự host (ký ed25519, kiểm giấy phép) | Cài là chạy, độc lập Zed, kiểm soát chất lượng và giấy phép | Phải vận hành CI và hosting, lo bảo mật chuỗi cung ứng; để Phase 3b sau Phase 4 |
| e | Đóng gói sẵn WASM vào app | Offline, không cần mạng | App phình, cập nhật server phải phát hành lại app, vướng giấy phép từng extension |
| f | Bảng server kiểu Helix (dữ liệu trong binary, người dùng ghi đè) | Không cần WASM, nhanh, tính năng LSP đầy đủ, hợp giấy phép | Người dùng tự cài server (có gợi ý cài); không có logic riêng ngoài vài hook (Vue) |
| g | Adapter Rust tự tải server (như adapter native của Zed) | Trải nghiệm tốt nhất cho từng server | Mỗi server một adapter, tốn công bảo trì, phải xử lý tải và cập nhật |
| h | Xin phép Zed dùng API extension | Hợp lệ nếu được đồng ý | Phụ thuộc người khác, không chắc có phản hồi; vẫn bị ràng buộc hạ tầng Zed |

## 7. Bảng server theo ngôn ngữ

languageId lấy từ bảng "Language identifiers" của LSP 3.17 (https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocumentItem) khi có; id đánh dấu `*` không nằm trong bảng spec, là giá trị thực tế các extension VS Code đang dùng. "Node" = cần Node.js trên PATH; "JVM" = cần JDK; "BEAM" = cần Erlang/Elixir.

### 7.1 Bảng

| Ngôn ngữ | Server | Lệnh stdio | Root markers | languageId | Cài trên macOS | Cần Node? |
|---|---|---|---|---|---|---|
| PHP | intelephense | `intelephense --stdio` | `composer.json`, `.git` | `php` | `npm i -g intelephense` | Có |
| PHP | phpactor | `phpactor language-server` | `composer.json`, `.phpactor.json`, `.git` | `php` | `composer global require phpactor/phpactor` hoặc phar (Homebrew core không có formula) | Không (PHP 8.1+) |
| Ruby | ruby-lsp | `ruby-lsp` | `Gemfile`, `.git` | `ruby` | `gem install ruby-lsp` | Không (Ruby) |
| Ruby | solargraph | `solargraph stdio` | `Gemfile`, `.git` | `ruby` | `gem install solargraph` | Không (Ruby) |
| Java | jdtls | `jdtls` (+ `-data <dir>`) | `pom.xml`, `build.gradle(.kts)`, `settings.gradle(.kts)`, `.git` | `java` | `brew install jdtls` | Không (JDK 21+, Python 3) |
| Kotlin | kotlin-lsp (JetBrains) | `kotlin-lsp --stdio` | `settings.gradle(.kts)`, `build.gradle(.kts)`, `pom.xml` | `kotlin`* | `brew install JetBrains/utils/kotlin-lsp` | Không (JVM đi kèm, chưa kiểm chứng) |
| Kotlin | kotlin-language-server (fwcd) | `kotlin-language-server` | như trên | `kotlin`* | `brew install kotlin-language-server` (chưa kiểm chứng) | Không (JVM) |
| Swift | sourcekit-lsp | `sourcekit-lsp` (hoặc `xcrun sourcekit-lsp`) | `Package.swift`, `buildServer.json`, `compile_commands.json`, `.git` | `swift` | đi kèm Xcode / CLT / toolchain Swift | Không |
| Lua | lua-language-server | `lua-language-server` | `.luarc.json`, `.luarc.jsonc`, `.stylua.toml`, `selene.toml`, `.git` | `lua` | `brew install lua-language-server` | Không |
| Elixir | elixir-ls | `elixir-ls` (= `language_server.sh` đổi tên) | `mix.exs` | `elixir` | `brew install elixir-ls` | Không (BEAM) |
| Elixir | expert | `expert --stdio` | `mix.exs` | `elixir` | binary từ GitHub release (`expert_darwin_arm64`) | Không (tự chứa) |
| Elixir | next-ls | `nextls --stdio` | `mix.exs` | `elixir` | binary release (chưa kiểm chứng) | Không |
| Elixir | lexical | `lexical` (`start_lexical.sh`) | `mix.exs` | `elixir` | build từ nguồn / release | Không (BEAM) |
| Zig | zls | `zls` | `build.zig`, `zls.json` | `zig`* | `brew install zls` | Không |
| Dart | dart | `dart language-server` | `pubspec.yaml` | `dart` | Dart hoặc Flutter SDK (tap `dart-lang/dart`) | Không |
| Vue | vue-language-server | `vue-language-server --stdio` | `package.json` | `vue`* | `npm i -g @vue/language-server` | Có |
| Vue (phần TS) | vtsls + plugin | `vtsls --stdio` | `package.json`, `tsconfig.json` | `vue`* | `npm i -g @vtsls/language-server` | Có |
| C/C++ | clangd | `clangd` | `compile_commands.json`, `compile_flags.txt`, `.clangd`, `.git` | `c`, `cpp`, `objective-c`, `objective-cpp` | CLT có `clangd`; hoặc `brew install llvm` (keg-only) | Không |
| C# | roslyn-language-server | `roslyn-language-server --stdio --autoLoadProjects` | `*.sln`, `*.slnx`, `*.csproj` | `csharp` | `dotnet tool install -g roslyn-language-server --prerelease` | Không (.NET SDK, có báo cáo cần 10) |
| C# | csharp-ls | `csharp-ls` | `*.sln`, `*.csproj` | `csharp` | `dotnet tool install -g csharp-ls` | Không (.NET SDK) |
| C# | OmniSharp | `OmniSharp --languageserver` | `*.sln`, `*.csproj` | `csharp` | GitHub release (bản `-net6.0`) | Không (.NET) |
| Haskell | hls | `haskell-language-server-wrapper --lsp` | `hie.yaml`, `cabal.project`, `stack.yaml`, `*.cabal`, `package.yaml` | `haskell` | `ghcup install hls` (hoặc `brew install haskell-language-server`) | Không (GHC) |
| OCaml | ocamllsp | `ocamllsp` | `dune-project`, `dune-workspace`, `*.opam`, `esy.json` | `ocaml` | `opam install ocaml-lsp-server` | Không (opam switch) |
| Scala | metals | `metals` | `build.sbt`, `build.sc`, `build.mill`, `.scala-build`, `pom.xml`, `build.gradle` | `scala` | `cs install metals` (Coursier: `brew install coursier`) | Không (JVM) |
| Terraform | terraform-ls | `terraform-ls serve` | `.terraform`, `.git` | `terraform`* / `terraform-vars`* | `brew install hashicorp/tap/terraform-ls` | Không |
| Dockerfile | docker-langserver | `docker-langserver --stdio` | `Dockerfile`, `.git` | `dockerfile` | `npm i -g dockerfile-language-server-nodejs` | Có |
| Dockerfile | docker-language-server (Docker Inc., Go) | `docker-language-server start --stdio` | như trên | `dockerfile` | GitHub release (chưa kiểm chứng) | Không |
| YAML | yaml-language-server | `yaml-language-server --stdio` | `.git` | `yaml` | `brew install yaml-language-server` hoặc npm | Có |
| JSON | vscode-json-language-server | `vscode-json-language-server --stdio` | `.git` | `json`, `jsonc` | `npm i -g vscode-langservers-extracted` | Có |
| CSS | vscode-css-language-server | `vscode-css-language-server --stdio` | `package.json`, `.git` | `css`, `scss`, `less` | cùng gói | Có |
| HTML | vscode-html-language-server | `vscode-html-language-server --stdio` | `package.json`, `.git` | `html` | cùng gói | Có |
| Bash | bash-language-server | `bash-language-server start` | `.git` | `shellscript` | `brew install bash-language-server` hoặc npm | Có |
| TOML | taplo | `taplo lsp stdio` | `.taplo.toml`, `taplo.toml`, `.git` | `toml`* | `brew install taplo` (bản npm không có LSP) | Không |
| Markdown | marksman | `marksman server` | `.marksman.toml`, `.git` | `markdown` | `brew install marksman` | Không (.NET tự chứa) |
| Nix | nil | `nil` | `flake.nix`, `.git` | `nix`* | `nix profile install nixpkgs#nil` | Không (cần Nix) |
| Nix | nixd | `nixd` | `flake.nix`, `.git` | `nix`* | `nix profile install nixpkgs#nixd` | Không (cần Nix) |
| Gleam | gleam | `gleam lsp` | `gleam.toml` | `gleam`* | `brew install gleam` | Không (BEAM khi build) |
| Erlang | elp | `elp server` | `rebar.config`, `.elp.toml`, `erlang.mk` | `erlang` | `brew install erlang-language-platform` | Không (Erlang/OTP) |
| Erlang | erlang_ls | `erlang_ls` | `rebar.config`, `erlang_ls.config` | `erlang` | `brew install erlang_ls` (chưa kiểm chứng) | Không (Erlang/OTP) |

Server cần Node: intelephense, vue-language-server, vtsls, typescript-language-server, docker-langserver, yaml-language-server, vscode-langservers-extracted (json/css/html), bash-language-server. Formula Homebrew của yaml-language-server và bash-language-server tự kéo `node`.

Root marker của Helix (để đối chiếu, chỉ dữ kiện): php `composer.json`, `index.php`; java `pom.xml`, `build.gradle`, `build.gradle.kts`; kotlin `settings.gradle(.kts)`; swift `Package.swift`; elixir `mix.exs`, `mix.lock`; zig `build.zig`; dart `pubspec.yaml`; haskell `Setup.hs`, `stack.yaml`, `cabal.project`, `hie.yaml`; scala `build.sbt`, `build.sc`, `build.gradle(.kts)`, `pom.xml`, `.scala-build`; erlang `rebar.config`; C# `*.slnx`, `*.sln`, `*.csproj`. Ruby và Nix không có `roots`.

### 7.2 Các quirk quan trọng

**jdtls (Java).**
- `jdtls` trên PATH là launcher Python `bin/jdtls` (`jdtls.py`), cần **Java 21+** (kiểm tra rồi thoát nếu thiếu; `--no-validate-java-version` bỏ qua) và Python 3.
- Thư mục dữ liệu workspace: không truyền `-data` thì launcher dùng `<cache>/jdtls/jdtls-<sha1 của cwd>` (`~/Library/Caches/jdtls/...` trên macOS). Vì vậy cwd khi spawn quan trọng: spawn với cwd = root LSP, hoặc truyền tường minh `-data <cache-của-lucerna>/jdtls/<hash-của-root>`. Thư mục này không được nằm trong project; mỗi root một thư mục, không dùng chung cho hai tiến trình jdtls cùng lúc. **Luôn truyền `-data`** trong cache của Lucerna.
- Launcher thêm cờ Equinox `-configuration`/OSGi, `-Xms1G`, `--add-modules=ALL-SYSTEM`, `--add-opens` cho `java.base`. JVM arg thêm qua `--jvm-arg=...`.
- Lần đầu rất chậm (import Maven/Gradle, có thể vài phút); gửi `language/status` và `$/progress`. Timeout phải rộng.
- Setting `java.import.generatesMetadataFilesAtProjectRoot` (mặc định false ở bản hiện tại, chưa kiểm chứng) quyết định có ghi `.project`/`.classpath`/`.settings` vào project hay không.
- jdtls trả URI `jdt://` (goto-definition vào class thư viện) mà client phải lấy qua `java/classFileContents`. Client chưa resolve được nên bỏ các location này.

**Vue.**
- `@vue/language-server` 2.x có "hybrid mode" tùy chọn; 3.0 bỏ tùy chọn và chỉ còn hybrid. Server Vue lo template/style/cấu trúc SFC; TypeScript trong `<script>` và biểu thức template do tsserver nạp `@vue/typescript-plugin` phục vụ.
- 3.0 thêm cặp notification tùy biến: server Vue gửi `tsserver/request` (params `[[id, command, args]]`, dạng chính xác chưa kiểm chứng) cho client; client phải chuyển cho server TypeScript và trả lại `tsserver/response`. Client bỏ qua thì log "Unknown notification" và tính năng Vue lặng lẽ suy giảm (lsp-mode issue #4838).
- nvim-lspconfig `vue_ls` (Apache-2.0) làm cầu nối trong `on_init`: chuyển sang `vtsls` (hoặc `ts_ls`) qua `workspace/executeCommand` với command `typescript.tsserverRequest`, rồi notify `tsserver/response`. vtsls nhận plugin qua `settings.vtsls.tsserver.globalPlugins = [{ name = "@vue/typescript-plugin", location = <đường dẫn @vue/language-server>, languages = ["vue"], configNamespace = "typescript" }]`.
- Zed làm tương tự trong lõi: `crates/project/src/lsp_store/vue_language_server_ext.rs` (133 dòng) đăng ký handler `tsserver/request` cho server `vue-language-server`, tìm `vtsls` (rơi về `typescript-language-server`) của cùng buffer, gửi `workspace/executeCommand typescript.tsserverRequest`, trả `tsserver/response`. `default.json` của Zed đặt server Vue là `["vue-language-server", "vtsls", "..."]`.
- Mặc định Helix chỉ liệt kê `vuels` cho Vue với `typescript.tsdk = "node_modules/typescript/lib/"`; không thấy xử lý `tsserver/request` trong Helix, nên Vue 3.x trên Helix chỉ có tính năng phía Vue; wiki bảo người dùng thêm `typescript-language-server` với plugin TS.
- Thiết lập tối thiểu hữu ích cho trình xem: chạy `vtsls --stdio` cho `.vue` với global plugin `@vue/typescript-plugin` (location = thư mục cài `@vue/language-server` toàn cục, tìm qua `npm root -g`), cộng `vue-language-server --stdio`, cộng một hook "chuyển tiếp request" nhỏ trong client LSP của Lucerna: nhận `tsserver/request` từ server A thì gọi `workspace/executeCommand(typescript.tsserverRequest, ...)` trên server B cùng root, rồi notify A bằng `tsserver/response`. Đây là phần Rust riêng duy nhất cho Vue; phần còn lại là dữ liệu bảng.
- Vue 2: nhánh 3.x vẫn hỗ trợ Vue 2.7; thiết lập cũ dùng `@vue/language-server@2` với `vue.hybridMode = false` (takeover), đã deprecated.

**Server ghi vào thư mục project** (đáng chú ý với trình xem chỉ-đọc):

| Server | Ghi gì | Giảm thiểu |
|---|---|---|
| ruby-lsp | `.ruby-lsp/` (composed bundle), có thể chạy bundler (chưa kiểm chứng); chạy không kèm `bundle exec` | chấp nhận hoặc ghi chú |
| elixir-ls | `.elixir_ls/`, biên dịch project; dialyzer chậm nên tắt (`elixirLS.dialyzerEnabled = false`) | chấp nhận |
| expert | `.expert/` (chưa kiểm chứng) | chấp nhận |
| metals | `.metals/`, `.bloop/`, `project/metals.sbt`; hỏi import build qua `window/showMessageRequest` | mặc định tắt hoặc hỏi |
| clangd | `.cache/clangd/index/` | `--background-index=false` (tùy chọn) hoặc `.clangd` `Index: Background: Skip` |
| sourcekit-lsp | `.build/index-build/` (background indexing, mặc định bật từ Swift 6.1, chưa kiểm chứng) | chấp nhận |
| jdtls | thư mục dữ liệu ngoài project nếu có `-data`; tùy chọn `.project`/`.classpath` | luôn truyền `-data` trong cache |
| gleam / zls / hls / ocamllsp | thư mục build (`build/`, `.zig-cache`, `dist-newstyle`, `_build`) | chấp nhận, là output build bình thường |

Gợi ý trường trong bảng: `writes_to_project: bool` (hiện ở view health), và `enabled_by_default: false` cho metals.

**Các quirk khác (ngắn):**
- intelephense: đóng nguồn (free + premium). Khóa premium qua `initializationOptions.licenceKey` (viết kiểu Anh) hoặc `~/intelephense/licence.txt`. Phải có `--stdio`, nếu không dùng node-ipc. Mặc định: intelephense trước, phpactor là lựa chọn thay thế (Zed đặt `["phpactor", "!intelephense", ...]` nhưng bản `windows` đảo lại).
- ruby-lsp và solargraph: solargraph cần subcommand `stdio`. Zed cho Ruby `["..."]` mở ra solargraph trước ruby-lsp; Helix đặt ruby-lsp trước.
- kotlin-lsp: Alpha, "một phần đóng nguồn"; cần `--stdio` (không thì nghe TCP, chưa kiểm chứng). Zed ưu tiên `kotlin-lsp` hơn `kotlin-language-server`; fwcd gần như không còn bảo trì.
- sourcekit-lsp: trên macOS `/usr/bin/sourcekit-lsp` là shim xcrun, toolchain phải khớp toolchain build project. SwiftPM chạy ngay; project Xcode cần cầu nối BSP (`xcode-build-server` ghi `buildServer.json`) hoặc `compile_commands.json`.
- lua-language-server: settings dùng namespace `Lua.*`; đặt `Lua.workspace.checkThirdParty = false` để tránh hộp thoại tương tác (chưa kiểm chứng).
- elixir-ls và expert: mặc định `elixir-ls`, expert là lựa chọn; bỏ next-ls, lexical (đã bị thay thế, chưa kiểm chứng lịch sử hợp nhất). Expert (https://github.com/elixir-lang/expert) Alpha, cần `--stdio`.
- zls: phải khớp phiên bản compiler Zig (zls 0.15 cho zig 0.15); có thể chạy `zig build` khi mở (chưa kiểm chứng).
- dart: `dart language-server` mặc định LSP; README yêu cầu client khai báo qua env `DASH__IDE_NAME`, `DASH__IDE_VERSION`, `DASH__PLUGIN_NAME`, `DASH__PLUGIN_VERSION`. `initializationOptions` hữu ích: `onlyAnalyzeProjectsWithOpenFiles` (hợp với trình xem), `outline`, `flutterOutline`, `suggestFromUnimportedLibraries`, `allowOpenUri`. Flutter có `dart` trong `flutter/bin/dart`.
- clangd: không có `compile_commands.json`/`compile_flags.txt` thì đoán cờ.
- roslyn-language-server: `--autoLoadProjects` để tự mở solution (không thì client phải gửi `solution/open`/`project/open`); chỉ hỗ trợ pull diagnostics (`textDocument/diagnostic`). Mọi server C# cần `dotnet restore`. Helix ưu tiên roslyn, omnisharp, csharp-ls; Zed ưu tiên `roslyn`.
- hls: luôn chạy bản **wrapper** để chọn đúng binary theo GHC của project; lần đầu chậm.
- ocamllsp: phải cài trong cùng opam switch với project (hay dùng `_opam` cục bộ), nên PATH từ app GUI thường thiếu; logic lấy PATH từ login shell của Lucerna quan trọng ở đây.
- terraform-ls: root là thư mục module; `.terraform` chỉ có sau `terraform init`.
- yaml-language-server: schema qua `yaml.schemas`; mặc định tải SchemaStore qua mạng (`yaml.schemaStore.enable`), nên để người dùng chọn.
- JSON/CSS/HTML: cùng gói `vscode-langservers-extracted` (MIT, tách từ VS Code). Server JSON cần client đẩy schema qua `json/schemaAssociations` hoặc `json.schemas`, hoặc nhờ client tải qua request tùy biến `vscode/content` (Zed có `json_language_server_ext`); không có thì chỉ kiểm cú pháp. `provideFormatter: true` để bật format.
- bash-language-server: diagnostics từ `shellcheck` nếu có trên PATH; format từ `shfmt`.
- taplo: bản npm `@taplo/cli` và vài bản build thiếu LSP; dùng brew hoặc `cargo install taplo-cli --locked --features lsp`. Phương án khác: `tombi`. Kiến thức Cargo.toml lấy từ SchemaStore (mạng).
- marksman: cần root (`.marksman.toml` hoặc `.git`) để link chéo file; file đơn chạy ở "single-file mode".
- nil (MIT) hoặc nixd (LGPL-3.0, chỉ dùng binary); cả hai cần `nix` cho tính năng đánh giá; format qua `nixfmt` ngoài.
- gleam: `gleam lsp` nằm trong binary compiler; build vào `<root>/build/` (chưa kiểm chứng).
- Erlang: ELP (`elp server`, cấu hình `.elp.toml`); Helix chạy cả erlang_ls và elp, mỗi cái `except-features = ["document-symbols", "workspace-symbols"]`; Zed ưu tiên `erlang-ls` và tắt `elp` theo mặc định.

## 8. Sơ đồ một mục trong bảng của Lucerna

Chỉ để minh họa; key dạng snake_case khớp settings hiện có.

```jsonc
"lsp": {
  "intelephense": {
    "command": "intelephense",          // hoặc binary.path
    "args": ["--stdio"],                // hoặc binary.arguments
    "languages": ["php"],
    "root_markers": ["composer.json", ".git"],
    "language_id": "php",               // mặc định: id ngôn ngữ của Lucerna
    "initialization_options": {},
    "settings": {},
    "enabled": true,
    "install": "npm i -g intelephense", // gợi ý ở view health
    "docs": "https://github.com/bmewburn/intelephense-docs"
  }
},
"languages": {
  "PHP": { "language_servers": ["intelephense", "!phpactor", "..."] }
}
```

Quy tắc merge đề xuất: gộp object theo key (như Zed/Helix), thay mảng (như cả hai), `!`/`...` chỉ cho `language_servers`. File cấp project chỉ được đặt `initialization_options`, `settings`, `language_servers` và `enabled` cho đến khi thư mục được trust.

## 9. Lucerna chọn gì

**Đây là đề xuất ban đầu; sẽ chốt trong spec sau khi thảo luận với người dùng.** Các điểm đã được người dùng duyệt được ghi rõ.

| Chủ đề | Đề xuất | Vì sao |
|---|---|---|
| Hướng chính | Phương án **f**: bảng server kiểu Helix, dữ liệu trong binary, người dùng ghi đè (đã duyệt) | Không cần WASM hay toolchain; LSP đầy đủ; hợp giấy phép |
| Về sau | Phương án **d** (CI Lucerna build sẵn và tự host, ký ed25519) để Phase 3b sau Phase 4 nếu cần "cài là chạy" (đã duyệt) | Độc lập Zed, không vướng ToS |
| Không làm | c (dùng `api.zed.dev`), và không theo Wu | ToS §2.2, §2.4(b); không có tài liệu công khai cho endpoint |
| Shape cấu hình | Giữ `lsp.<server>.{binary, initialization_options, settings}` + thêm `command`/`args`, `languages`, `root_markers`, `language_id`, `enabled`, `install`, `docs` | Tương thích Zed (extension WASM cần đúng shape), mẫu số chung của Helix/Sublime/Neovim |
| Thứ tự server | `languages.<L>.language_servers` với `!` và `...` | Theo Zed; VS Code không có ý kiến trong lõi |
| `config` | Tách `initialization_options` và `settings` | Đúng hơn Helix một khối |
| Giấy phép bảng mặc định | Viết từ tài liệu upstream, ghi `docs`, đối chiếu nvim-lspconfig, không chép `languages.toml` | MPL-2.0 cấp file |
| Trust | Built-in và user settings được spawn binary trên PATH; file project không đặt được `command`/`binary.path`/`env` khi chưa trust; cân nhắc hash file project như Helix | Chặn nhánh độc hại đổi `command` |
| Health | View "LSP health" kiểu `hx --health <lang>` nhưng dò `--version` | Có sẵn `probe_args` |
| Hook riêng | Chuyển tiếp `tsserver/request` cho Vue; jdtls luôn truyền `-data` trong cache Lucerna | Hai chỗ bảng dữ liệu không đủ |
| Server ghi vào project | Cờ `writes_to_project`; metals mặc định tắt hoặc hỏi | Trình xem chỉ-đọc |
| Mặc định chọn server | PHP: intelephense; Ruby: ruby-lsp; Kotlin: kotlin-lsp; Elixir: elixir-ls; C#: roslyn; Dockerfile: docker-langserver; Erlang: elp | Theo mặc định Zed/Helix và độ bảo trì |

## Nguồn

- Helix docs: https://docs.helix-editor.com/languages.html
- Helix languages.toml: https://github.com/helix-editor/helix/blob/master/languages.toml
- Helix loader, merge: https://github.com/helix-editor/helix/blob/master/helix-loader/src/lib.rs
- Helix config và trust: https://github.com/helix-editor/helix/blob/master/helix-loader/src/config.rs, https://github.com/helix-editor/helix/blob/master/helix-loader/src/workspace_trust.rs, PR https://github.com/helix-editor/helix/pull/15857
- Helix health: https://github.com/helix-editor/helix/blob/master/helix-term/src/health.rs
- Helix workspace/configuration: https://github.com/helix-editor/helix/blob/master/helix-term/src/application.rs
- Helix initialization options: https://github.com/helix-editor/helix/blob/master/helix-lsp/src/client.rs
- Helix wiki: https://github.com/helix-editor/helix/wiki/Language-Server-Configurations
- Helix release 25.07.1: https://github.com/helix-editor/helix/releases
- MPL-2.0: https://www.mozilla.org/en-US/MPL/2.0/ và FAQ https://www.mozilla.org/en-US/MPL/2.0/FAQ/
- nvim-lspconfig: https://github.com/neovim/nvim-lspconfig (Vue: https://github.com/neovim/nvim-lspconfig/blob/master/lsp/vue_ls.lua)
- LSP language ids: https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocumentItem
- jdtls launcher: https://github.com/eclipse-jdtls/eclipse.jdt.ls/blob/main/org.eclipse.jdt.ls.product/scripts/jdtls.py
- kotlin-lsp: https://github.com/Kotlin/kotlin-lsp
- Expert: https://github.com/elixir-lang/expert
- ELP: https://whatsapp.github.io/erlang-language-platform/docs/get-started/install/
- Dart LSP: https://github.com/dart-lang/sdk/blob/main/pkg/analysis_server/tool/lsp_spec/README.md
- ruby-lsp: https://shopify.github.io/ruby-lsp/
- roslyn-language-server: https://www.nuget.org/packages/roslyn-language-server.linux-x64, https://github.com/seblyng/roslyn.nvim, https://zed.dev/docs/languages/csharp.html
- Vue language tools: https://github.com/vuejs/language-tools, wiki https://github.com/vuejs/language-tools/wiki/Neovim, README https://cdn.jsdelivr.net/npm/@vue/language-server@3.3.1/README.md, PR https://github.com/vuejs/language-tools/pull/5248, lsp-mode issue https://github.com/emacs-lsp/lsp-mode/issues/4838
- Wu: https://github.com/dstrml/wu
- Zed ToS: https://zed.dev/terms
- Zed extension docs: https://zed.dev/docs/extensions/developing-extensions, https://zed.dev/docs/extensions/publishing/prerequisites
- Zed (cục bộ): `docs/src/configuring-languages.md`, `assets/settings/default.json`, `crates/settings_content/src/project.rs`, `crates/project/src/lsp_store/vue_language_server_ext.rs`
- VS Code LSP guide: https://code.visualstudio.com/api/language-extensions/language-server-extension-guide
- glspc: https://github.com/llllvvuu/vscode-glspc
- vscode-lsp-config: https://github.com/pepebecker/vscode-lsp-config
- Alloglot: https://github.com/MercuryTechnologies/alloglot
- langoustine-vscode: https://github.com/neandertech/langoustine-vscode
- Sublime LSP: https://lsp.sublimetext.io/client_configuration/
