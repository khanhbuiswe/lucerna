# Bảng language server (kiểu Helix)

Ngày 2026-11-11. Trạng thái: đã triển khai (commit 92d55bd..6d668f9, cộng commit docs). Khác spec khi triển khai: `languages` của clangd là `["c","c++","objective-c"]` vì id extension là tên chữ thường; relay chờ vtsls đang `Starting` thay vì trả `null` ngay; lệnh `probe` của Health chạy trong thư mục root. Tiếp nối Phase 3 (`2026-11-10-lucerna-phase3-lsp-design.md`).

## 1. Mục tiêu

PHP, Ruby, Java, Kotlin, Swift, Lua, Elixir, Zig, Dart, Vue, C#, Haskell… có LSP trong Lucerna khi người dùng **tự cài server lên PATH**, không cần host WASM và không cần Lucerna tải gì. Cách làm theo Helix: một bảng server mặc định nằm trong lõi, người dùng ghi đè hoặc thêm server bằng settings JSON.

Đã chốt với người dùng:

| # | Quyết định | Lý do |
|---|---|---|
| T1 | **Mở rộng key `lsp` sẵn có** thay vì thêm key mới. Bảng mặc định và settings của người dùng cùng một hình dạng. | Một chỗ cấu hình cho một server; giữ tương thích với các key kiểu Zed đã có (`binary`, `initialization_options`, `settings`). |
| T2 | **Khớp cả theo đuôi/tên file** (`file_types`), không chỉ theo id ngôn ngữ. | File Ruby, PHP… vẫn có LSP khi chưa cài extension ngôn ngữ (chỉ thiếu tô màu). Helix cũng khai báo file-types trong bảng. |
| T3 | **Vue làm đủ, có relay** `tsserver/request` giữa vue-language-server và vtsls. | Không có relay thì phần `<script>` không có hover hay đi tới định nghĩa. Zed làm relay trong lõi; Helix không. |
| T4 | **Popover ⚡ + tab Health + `luc lsp health`.** | Người dùng biết thiếu server nào và lệnh cài, như `hx --health`. |
| T5 | **Bảng là dữ liệu JSON** (`servers.json`), một struct `ServerDef` thay cho trait `Adapter` và 6 adapter Rust. Hành vi đặc biệt bật bằng dữ liệu (biến `{data_dir}`, `"relay"`). | Một đường code; bảng dễ đọc, dễ ghi nguồn từng dòng; người dùng ghi đè bằng đúng hình dạng đó. |
| T6 | **Không chép `languages.toml` của Helix** (MPL-2.0, copyleft theo file). Mỗi dòng viết từ tài liệu upstream của chính server, ghi `homepage`; đối chiếu được với nvim-lspconfig (Apache-2.0). | Lệnh, tham số, root marker là dữ kiện; tránh đưa file MPL vào repo. |
| T7 | **Sửa D3 của Phase 3** (xem §8). | Chưa có host WASM, đây là cách rẻ nhất để có LSP cho mọi ngôn ngữ; rủi ro chạy lệnh tùy ý đã được cơ chế tin cậy thư mục chặn. |

**Thành công khi:**
- Cài `intelephense` (npm) rồi mở một file `.php` trong thư mục đã tin cậy, kể cả khi chưa cài extension PHP: có diagnostics, hover, F12, `@`.
- Mở `.vue` khi đã cài `@vue/language-server` và `@vtsls/language-server`: hover trong `<script setup>` hiện kiểu TypeScript.
- Thiếu server: popover ⚡ ghi "not found on PATH" kèm lệnh cài và nút Copy; tab Health và `luc lsp health` liệt kê mọi ngôn ngữ trong bảng.
- Người dùng thêm một server bảng chưa có (ví dụ `"lsp": { "mylang-ls": { "command": "mylang-ls", "file_types": ["ml"] } }`) là chạy, không cần build lại Lucerna.
- Mọi test của Phase 3 vẫn xanh.

**Ngoài phạm vi:** tự tải server; `only-features` / `except-features` của Helix; host WASM (3b, theo phương án d sau Phase 4); giao diện sửa bảng (người dùng sửa JSON); workspace nhiều root.

## 2. Tham khảo

Chi tiết ở `docs/research/2026-11-11-lsp-server-table.md`. Tóm tắt:
- **Helix:** `[language-server.<name>]` (`command`, `args`, `config`, `timeout`, `environment`, `required-root-patterns`) và `[[language]]` (`language-servers`, `roots`, `language-id`, `file-types`). Gộp 3 lớp (mặc định → người dùng → `.helix/languages.toml`): bảng gộp theo key, mảng bị thay nguyên. `config` gửi làm `initializationOptions` và dùng trả lời `workspace/configuration`. `hx --health` chỉ chạy `which`, không khởi động server.
- **Zed:** `lsp.<name>.{binary{path,arguments,env}, initialization_options, settings}` và `languages.<L>.language_servers` với `!`/`...`; **không** định nghĩa được server mới chỉ bằng settings (mọi server mới phải đến từ extension). Vue relay nằm trong lõi (`vue_language_server_ext.rs`).
- **VS Code:** không có client LSP chung trong lõi; mỗi extension tự cấu hình. Các extension "generic client" (glspc…) dùng key riêng, không có chuẩn chung.
- **Wu** (`dstrml/wu`, fork GPL của Zed) vẫn tải extension từ `api.zed.dev`. Lucerna không theo hướng này: điều khoản Zed (§2.2, §2.4(b)) chỉ cho truy cập qua cơ chế Zed chỉ định và cấm API không công khai.
- **Lucerna lệch khỏi Zed và VS Code** ở chỗ cho định nghĩa server mới bằng settings (§8, theo Helix).

## 3. Mô hình dữ liệu

### 3.1 Một mục server

`lsp.<tên>` có các trường (trường mới đánh dấu *):

| Trường | Kiểu | Ý nghĩa |
|---|---|---|
| `command`* | chuỗi | Tên binary tìm trên PATH của login shell, hoặc đường dẫn tuyệt đối |
| `args`* | mảng chuỗi | Tham số; hỗ trợ biến (§3.3) |
| `env`* | object chuỗi → chuỗi | Biến môi trường thêm vào (gộp lên môi trường login shell) |
| `languages`* | mảng id ngôn ngữ Lucerna | Ví dụ `"php"`, `"vue.js"`, `"c++"` (id extension = tên chữ thường, dấu cách thành `-`) |
| `file_types`* | mảng | Đuôi file (`"rb"`) hoặc tên file đầy đủ (`"Gemfile"`); so khớp không phân biệt hoa thường với đuôi, phân biệt với tên file |
| `language_id`* | chuỗi, hoặc object id Lucerna → id LSP | `languageId` trong `didOpen`. Không có thì dùng id ngôn ngữ Lucerna; file không có ngôn ngữ thì dùng giá trị chuỗi, hoặc phần tử đầu của `languages` |
| `root_markers`* | mảng | Tên file/thư mục đánh dấu root; hỗ trợ glob `*.sln`, `*.cabal` |
| `root_search`* | `"nearest"` \| `"outermost"` \| `"ordered"` | Như `RootSearch` hiện có (`ordered` = `NearestByMarker`) |
| `probe`* | mảng chuỗi hoặc `false` | Lệnh chạy thử khi tìm binary (mặc định `["--version"]`) |
| `enabled`* | bool | Mặc định `true`; server thay thế trong bảng mặc định là `false` |
| `relay`* | `"vue-tsserver"` | Bật relay của Vue (§5) |
| `install`* | chuỗi, hoặc object `macos`/`linux`/`windows` → chuỗi | Lệnh cài hiển thị trong Health |
| `homepage`* | URL | Tài liệu upstream; cũng là nguồn của dòng này |
| `note`* | chuỗi | Ghi chú một dòng hiển thị trong tab Health (ví dụ server ghi vào thư mục project) |
| `binary` | object `{path, arguments, env}` | Có sẵn (kiểu Zed); `path` thay `command`, `arguments` thay `args`, `env` gộp lên `env` |
| `initialization_options` | object | Có sẵn |
| `settings` | object | Có sẵn (trả lời `workspace/configuration`, `didChangeConfiguration`) |

Một mục **dùng được** khi có `command` (hoặc `binary.path`) và có ít nhất một trong `languages` / `file_types`. Mục chỉ ghi đè vài trường của một server mặc định (ví dụ chỉ `settings`) vẫn hợp lệ vì được gộp với dòng mặc định.

### 3.2 Gộp lớp

- Ba lớp: **mặc định** (`crates/lucerna-lsp/assets/servers.json`, nhúng bằng `include_str!`) → **User** → **Workspace**. Lớp User và Workspace là giá trị `lsp` mà `SettingsStore` đã gộp (object gộp đệ quy, lớp trên thắng; giá trị không phải object thay nguyên).
- Gộp mặc định với settings cùng quy tắc: object gộp theo key đệ quy; mảng (`args`, `languages`, `file_types`, `root_markers`, `probe`) bị thay nguyên.
- **Tin cậy:** khi thư mục chưa tin cậy, Lucerna không chạy server nào và không đọc `lsp` của lớp Workspace (như Phase 3, `get_trusted`). Khi đã tin cậy, lớp Workspace được đọc đầy đủ, kể cả `command`.
- Chọn server cho một ngôn ngữ vẫn dùng `lucerna.lsp.languageServers` ghi đè theo ngôn ngữ (`"[php]": { … }`), với `!tên` và `"..."`. `"..."` gồm mọi server **đang bật** khớp với file (§4.1), theo thứ tự tên.
- Kết quả gộp là `ServerTable { servers: BTreeMap<Arc<str>, ServerDef>, errors: Vec<TableError> }`. Mục sai kiểu hoặc thiếu trường bắt buộc vào `errors` (kèm tên key và lý do) và bị bỏ qua; các mục khác vẫn dùng được.

### 3.3 Biến trong `args` và `env`

| Biến | Giá trị |
|---|---|
| `{root}` | Root của server (§4.2) |
| `{data_dir}` | `<data dir Lucerna>/lsp/<tên server>/<hash ngắn của root>`; tạo nếu chưa có |
| `{npm_root}` | Kết quả `npm root -g` (chạy nền một lần mỗi tiến trình, timeout 5 s, dùng môi trường login shell) |

Biến không biết, hoặc `{npm_root}` thất bại, thì server đó chuyển sang `Failed` với thông báo nêu biến. Không có biến nào trong `command`.

## 4. Khớp file và vòng đời

### 4.1 Chọn server cho một file

1. Nếu file có ngôn ngữ Lucerna (id `L`) và có server trong bảng (bật hay tắt) liệt kê `L` trong `languages`: các server đó.
2. Nếu không có ngôn ngữ, hoặc không server nào liệt kê `L` (extension đặt tên khác bảng, ví dụ "Terraform Vars" → `terraform-vars` cho `.tfvars`, "OCaml Interface" → `ocaml-interface` cho `.mli`): server có đuôi file hoặc tên file trong `file_types`.
3. Lọc theo `lucerna.lsp.languageServers` của `L` (hoặc mặc định `["..."]` khi không có ngôn ngữ) và `enabled`.
4. `languageId` trong `didOpen` theo `language_id` (§3.1).

`Documents` phát `DocumentEvent::Opened` cả khi `language_id` là `None`; `LanguageServers` dùng đường dẫn để khớp `file_types`. Kiểm tra "file quá lớn" và "chưa tin cậy" giữ nguyên.

### 4.2 Root, khởi động, nhiều server

- Root tìm theo `root_markers` và `root_search` như Phase 3; marker có `*` hoặc `?` là glob trên tên các mục trong từng thư mục cha. File ngoài workspace dùng server đang chạy (như bản sửa cuối Phase 3).
- `ServerKey.server` đổi từ `&'static str` sang `Arc<str>` vì tên server giờ đến từ dữ liệu.
- Một file có thể gắn nhiều server (đã có). Mọi request (hover, định nghĩa, symbol…) gộp như Phase 3.
- Settings đổi: tính lại bảng; server có `command`/`args`/`env`/`binary`/`initialization_options` đổi thì restart, chỉ `settings` đổi thì `didChangeConfiguration`, server bị bỏ hoặc tắt thì stop (logic `SettingsDiff` hiện có, so trên `ServerDef`).

## 5. Vue

- Bảng mặc định: `vue-language-server` (`--stdio`, `languages: ["vue.js"]`, `file_types: ["vue"]`, `language_id: "vue"`, `relay: "vue-tsserver"`) và `vtsls` (`--stdio`, `languages: ["vue.js"]`, `file_types: ["vue"]`, `language_id: "vue"`), với `settings` của vtsls (trả lời `workspace/configuration`, như nvim-lspconfig) khai báo plugin:

  ```json
  { "vtsls": { "tsserver": { "globalPlugins": [ {
      "name": "@vue/typescript-plugin",
      "location": "{npm_root}/@vue/language-server",
      "languages": ["vue"], "configNamespace": "typescript", "enableForWorkspaceTypeScriptVersions": true
  } ] } } }
  ```

  (Biến cũng được thay trong `settings` và `initialization_options`.)
- **Relay** (trong `lucerna-lsp`, bật bởi `relay: "vue-tsserver"`): vue-language-server gửi notification `tsserver/request` với mảng các `[id, command, args]`. Với mỗi phần tử, Lucerna tìm server `vtsls` đang `Running` cùng root (không có thì `typescript-language-server`, như Zed), gửi `workspace/executeCommand { command: "typescript.tsserverRequest", arguments: [command, args, { isAsync: true, lowPriority: true }] }`, rồi gửi lại vue-language-server notification `tsserver/response` với `[[id, body]]` (body là `result.body` của câu trả lời, `null` khi lỗi). Không có server TypeScript cùng root thì trả ngay `[[id, null]]` để vue-language-server không treo; nếu server đó đang `Starting` thì request chờ tới khi nó `Running` (gửi đi) hoặc thất bại/dừng (trả `null`), vì vue-language-server gửi `_vue:projectInfo` ngay sau `didOpen`, lúc vtsls thường chưa khởi động xong (nvim-lspconfig thử lại 10 lần × 100 ms cho cùng lý do). Mỗi request chạy nền và trả lời riêng, không chặn event khác.
- **Hình dạng đã kiểm** (2026-11-11) với `vuejs/language-tools` (MIT): `packages/language-server/lib/server.ts` gửi `sendNotification('tsserver/request', [id, command, args])` và nhận `tsserver/response` bằng `([id, res]) => …`; `vscode-jsonrpc` bọc một tham số mảng vào mảng params, nên trên dây là `[[id, command, args]]` và `[[id, body]]`. Client VS Code (`extensions/vscode/src/extension.ts`) gọi `typescript.tsserverRequest` với `command, args, { isAsync: true, lowPriority: true }` và trả `res?.body`. nvim-lspconfig `lsp/vue_ls.lua` (Apache-2.0) cũng mở hai lớp mảng và trả `{ { id, r and r.body } }`, nhưng bỏ object thứ ba trong `arguments`. Lucerna theo client VS Code; khớp với mô tả ở trên.
- TypeScript/JavaScript thuần vẫn dùng `typescript-language-server` (như Phase 3). vtsls chỉ gắn với `.vue`; người dùng muốn dùng vtsls cho TS thì thêm `"typescript"` vào `languages` của nó và chọn bằng `languageServers`.

## 6. jdtls

Dòng trong bảng: `"command": "jdtls", "args": ["-data", "{data_dir}"]` (script launcher của Homebrew, cần JDK 21+). `{data_dir}` nằm trong data dir của Lucerna nên jdtls không ghi workspace data vào project. Không cần adapter Rust riêng.

## 7. Health

- **Popover ⚡:** hàng server `NotFound` cho các file đang mở hiện thêm dòng lệnh cài theo hệ điều hành hiện tại (font code, mờ) và nút chữ "Copy" (chép lệnh vào clipboard). Lỗi bảng (`TableError`) hiện thành hàng riêng "Invalid `lsp.<tên>`: <lý do>". Cuối popover thêm nút "Show All Languages…" mở tab Health. Dùng lại `popover`, `text_button`, `tooltip` của `ui.rs`.
- **Tab Health** (`lucerna.lsp.health`, `TabItem::LspHealth`, một tab, không lưu workspace state): danh sách chỉ đọc, mỗi dòng một cặp (ngôn ngữ, server) trong bảng, gom theo ngôn ngữ với header như `excerpt_list`. Cột: server, trạng thái (`✓ /đường/dẫn` màu `text.muted`, `✗ not found` màu `warning`, `disabled` mờ), lệnh cài + nút Copy. Đầu tab có hàng tóm tắt "18 of 40 servers found" và nút refresh. `which` và `probe` chạy nền, kết quả cache tới khi bấm refresh hoặc settings đổi.
- **`luc lsp health`:** in cùng bảng ra stdout dạng cột (ngôn ngữ, server, trạng thái, lệnh cài), không cần GUI; mã thoát 0. Dùng `lucerna-lsp` trực tiếp và settings User (không đọc settings Workspace của thư mục hiện tại).
- Server ghi vào thư mục project (ruby-lsp `.ruby-lsp/`, elixir-ls `.elixir_ls/`, metals `.metals/` `.bloop/`, clangd `.cache/clangd/`, sourcekit-lsp `.build/`) có ghi chú một dòng trong tab Health. Không chặn: thư mục đã qua bước tin cậy.

## 8. Thay đổi spec cũ

- **Phase 3, D3** đổi từ "Không cho tự khai báo server chỉ bằng settings" thành: "**Cho tự khai báo server bằng settings** (`lsp.<tên>.command`…), theo Helix. Lệch khỏi VS Code và Zed. Lý do: chưa có host WASM, đây là cách rẻ nhất để có LSP cho mọi ngôn ngữ; rủi ro chạy lệnh tùy ý đã được cơ chế tin cậy thư mục chặn (settings Workspace chỉ được đọc khi đã tin cậy)." Mục "Ngoài phạm vi" của Phase 3 bỏ "server tự khai báo".
- Phase 3 §3.1 `adapters`: bảng adapter Rust thay bằng `servers.json`; giá trị các dòng rust-analyzer, gopls, basedpyright/pyright, typescript-language-server, clangd giữ nguyên.
- Phase 3 §8 settings: `lsp` thêm các trường §3.1.

## 9. Bảng mặc định

Mỗi ngôn ngữ đúng một server bật sẵn; server thay thế có trong bảng nhưng `enabled: false`. Lệnh và root marker lấy từ tài liệu upstream (cột "Nguồn" = `homepage`), đối chiếu nvim-lspconfig. Cột "Ngôn ngữ" là id Lucerna (bundled hoặc từ extension phổ biến); `file_types` liệt kê các đuôi tương ứng.

| Ngôn ngữ (id) | Server bật | Lệnh | Root markers | Cài (macOS) | Tắt sẵn |
|---|---|---|---|---|---|
| rust | rust-analyzer | `rust-analyzer` | `Cargo.toml` (outermost) | `rustup component add rust-analyzer` | |
| go | gopls | `gopls` | `go.work`, `go.mod` (ordered) | `go install golang.org/x/tools/gopls@latest` | |
| python | basedpyright | `basedpyright-langserver --stdio` | `pyproject.toml`, `setup.py`, `setup.cfg`, `pyrightconfig.json` | `npm i -g basedpyright` | pyright |
| typescript, typescriptreact, javascript, javascriptreact | typescript-language-server | `… --stdio` | `tsconfig.json`, `jsconfig.json`, `package.json` | `npm i -g typescript typescript-language-server` | |
| php | intelephense | `intelephense --stdio` | `composer.json`, `.git` | `npm i -g intelephense` | phpactor |
| ruby | ruby-lsp | `ruby-lsp` | `Gemfile`, `.git` | `gem install ruby-lsp` | solargraph |
| java | jdtls | `jdtls -data {data_dir}` | `pom.xml`, `build.gradle`, `build.gradle.kts`, `settings.gradle`, `settings.gradle.kts`, `.git` | `brew install jdtls` | |
| kotlin | kotlin-lsp | `kotlin-lsp --stdio` | `settings.gradle.kts`, `settings.gradle`, `build.gradle.kts`, `build.gradle`, `pom.xml` | `brew install JetBrains/utils/kotlin-lsp` | kotlin-language-server |
| swift | sourcekit-lsp | `sourcekit-lsp` | `Package.swift`, `buildServer.json`, `compile_commands.json`, `.git` | Xcode / Command Line Tools | |
| lua | lua-language-server | `lua-language-server` | `.luarc.json`, `.luarc.jsonc`, `.stylua.toml`, `selene.toml`, `.git` | `brew install lua-language-server` | |
| elixir | elixir-ls | `elixir-ls` | `mix.exs` | `brew install elixir-ls` | expert |
| zig | zls | `zls` | `build.zig`, `zls.json` | `brew install zls` | |
| dart | dart | `dart language-server --protocol=lsp` | `pubspec.yaml` | Dart SDK hoặc Flutter | |
| vue.js | vue-language-server + vtsls | §5 | `package.json` | `npm i -g @vue/language-server @vtsls/language-server` | |
| c, c++, objective-c | clangd | `clangd` | `compile_commands.json`, `compile_flags.txt`, `.clangd`, `.git` | Command Line Tools hoặc `brew install llvm` | |
| c# | csharp-ls | `csharp-ls` | `*.sln`, `*.slnx`, `*.csproj` | `dotnet tool install -g csharp-ls` | roslyn-language-server |
| haskell | haskell-language-server | `haskell-language-server-wrapper --lsp` | `hie.yaml`, `cabal.project`, `stack.yaml`, `*.cabal`, `package.yaml` | `ghcup install hls` | |
| ocaml | ocamllsp | `ocamllsp` | `dune-project`, `dune-workspace`, `*.opam` | `opam install ocaml-lsp-server` | |
| scala | metals | `metals` | `build.sbt`, `build.sc`, `build.mill`, `.scala-build` | `cs install metals` | |
| terraform | terraform-ls | `terraform-ls serve` | `.terraform`, `.git` | `brew install hashicorp/tap/terraform-ls` | |
| dockerfile | docker-langserver | `docker-langserver --stdio` | `Dockerfile`, `.git` | `npm i -g dockerfile-language-server-nodejs` | |
| yaml | yaml-language-server | `yaml-language-server --stdio` | `.git` | `brew install yaml-language-server` | |
| json, jsonc | vscode-json-language-server | `… --stdio` | `.git` | `npm i -g vscode-langservers-extracted` | |
| css, scss, less | vscode-css-language-server | `… --stdio` | `package.json`, `.git` | như trên | |
| html | vscode-html-language-server | `… --stdio` | `package.json`, `.git` | như trên | |
| shellscript | bash-language-server | `bash-language-server start` | `.git` | `brew install bash-language-server` | |
| toml | taplo | `taplo lsp stdio` | `.taplo.toml`, `taplo.toml`, `.git` | `brew install taplo` | |
| markdown | marksman | `marksman server` | `.marksman.toml`, `.git` | `brew install marksman` | |
| nix | nil | `nil` | `flake.nix`, `.git` | `nix profile install nixpkgs#nil` | nixd |
| gleam | gleam | `gleam lsp` | `gleam.toml` | `brew install gleam` | |
| erlang | elp | `elp server` | `rebar.config`, `.elp.toml`, `erlang.mk` | `brew install erlang-language-platform` | |

Test kiểm id ngôn ngữ bundled của Lucerna (`shellscript`, `json`…) có trong registry và được bảng nêu tên; id extension (`php`, `ruby`, `vue.js`, `c++`, `c#`…) chỉ được kiểm đúng dạng (chữ thường, không dấu cách), không kiểm khớp extension thật vì extension không có sẵn khi test. File có id không server nào liệt kê vẫn có server nhờ `file_types` (§4.1, có test).

## 10. Kiến trúc mã

- `crates/lucerna-lsp/assets/servers.json`: bảng mặc định.
- `crates/lucerna-lsp/src/table.rs` (mới): `ServerDef`, `ServerTable::build(defaults: &Value, user_and_workspace: &Value) -> ServerTable`, gộp lớp, kiểm tra từng mục, `TableError`.
- `crates/lucerna-lsp/src/vars.rs` (mới): thay `{root}`, `{data_dir}`, `{npm_root}`.
- `crates/lucerna-lsp/src/relay.rs` (mới): relay Vue.
- `crates/lucerna-lsp/src/adapters.rs`: bỏ trait `Adapter` và các `Static`; giữ `find_root` (nhận `&ServerDef`) và thêm glob.
- `crates/lucerna-lsp/src/settings.rs`, `registry.rs`: dùng `ServerDef`, `ServerKey.server: Arc<str>`, `select_servers` theo §4.1.
- `crates/lucerna-lsp/src/health.rs` (mới): `health(table, launcher) -> Vec<HealthRow>`, dùng chung cho tab và CLI.
- `crates/lucerna-workbench`: `language_servers` dùng `ServerTable` và khớp `file_types`; `lsp_status.rs` thêm lệnh cài + Copy + lỗi bảng; `lsp_health.rs` (mới) cho tab Health; `TabItem::LspHealth`.
- `crates/lucerna/src/`: lệnh con `luc lsp health` (như `luc ext`).

## 11. Xử lý lỗi

- Mục sai: bỏ qua mục đó, báo trong popover ⚡ và tab Health; không panic, không ảnh hưởng server khác.
- `npm root -g` thất bại hoặc timeout: chỉ server dùng `{npm_root}` chuyển `Failed`, thông báo "npm not found; {npm_root} unavailable".
- Relay không tìm thấy vtsls, hoặc vtsls trả lỗi: trả `[[id, null]]`.
- `servers.json` hỏng là lỗi lúc build (test bắt), không phải lỗi lúc chạy.

## 12. Kiểm thử

- **Bảng mặc định:** parse được; mỗi dòng có `command`, có `languages` hoặc `file_types`, có `install` và `homepage`; mỗi ngôn ngữ đúng một server `enabled`.
- **`table.rs`:** gộp (object gộp, mảng thay, User thắng mặc định, Workspace thắng User); mục sai vào `errors`; ghi đè chỉ `settings` của server mặc định.
- **Khớp:** theo ngôn ngữ; theo đuôi và tên file khi không có ngôn ngữ; `language_id` dạng chuỗi và dạng object; `languageServers` với `!`/`...`.
- **Biến:** `{root}`, `{data_dir}` (tạo thư mục), `{npm_root}` (fake launcher), biến không biết.
- **Root:** glob `*.sln`.
- **Relay:** hai `FakeServer` (vue và vtsls): `tsserver/request` → `workspace/executeCommand` sang vtsls → `tsserver/response` đúng id; không có vtsls thì trả `null`.
- **Workbench:** mở `.rb` khi không có ngôn ngữ Ruby vẫn khởi động server Ruby (fake); popover hiện lệnh cài và Copy chép đúng chuỗi; tab Health liệt kê đủ dòng và trạng thái; server do người dùng khai báo trong settings User chạy được; khai báo trong `.lucerna/settings.json` chỉ chạy khi đã tin cậy.
- **CLI:** snapshot `luc lsp health` với fake launcher.
- **Phase 3:** toàn bộ test cũ xanh.
- **Thử thật:** rust-analyzer, clangd (và server npm/brew có sẵn nếu đã cài) trên máy phát triển.
