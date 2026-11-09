# Lucerna — Giai đoạn 2: Host extension Zed

- **Ngày:** 2026-11-09
- **Trạng thái:** Bản nháp chờ duyệt
- **Spec trước:** `2026-11-08-lucerna-phase1-design.md` (lộ trình ở §1.3, giao diện ở §12–§13)
- **Nghiên cứu nền:** `docs/research/2026-11-09-extensions.md` (so sánh Zed và VS Code)

## 1. Mục tiêu

### 1.1 Kết quả

Người dùng cài được extension từ registry của Zed ngay trong Lucerna, và những gì extension mang theo dùng được ngay, không cần khởi động lại:

- theme;
- icon theme;
- ngôn ngữ, gồm grammar tree-sitter và các query;
- snippet: được cài và kiểm tra, nhưng chưa dùng tới. Phần chèn snippet thuộc giai đoạn 4.

Extension được tự build từ repo `zed-industries/extensions`. Lucerna **không** gọi `api.zed.dev`, vì dịch vụ này không có điều khoản sử dụng công khai.

### 1.2 Tiêu chí thành công

1. Từ panel Extensions hoặc bằng `luc ext install`, cài được và dùng được ba loại extension sau, không phải khởi động lại:
   - một theme (Catppuccin);
   - một icon theme (ví dụ `material-icon-theme`);
   - một ngôn ngữ có grammar riêng (Zig).
2. Một extension ngôn ngữ cần injection, như Vue hoặc Svelte, được tô màu cả phần nhúng: `<script>`, `<style>`, biểu thức trong template.
3. Mọi mục tiêu hiệu năng của giai đoạn 1 (§6) vẫn đạt.
4. Một extension hỏng, dù ở lúc cài hay lúc chạy, không làm sập hay treo editor.

### 1.3 Những gì đã quyết định khi brainstorm

| # | Quyết định | Lý do |
|---|---|---|
| D1 | **Chưa có host WASM component** (WIT, `extension.wasm`, các hàm host) ở giai đoạn này. Để sang giai đoạn 3, cùng với LSP. | Bốn thứ trong scope đều là dữ liệu. Chỉ language server cần chạy code của extension. |
| D2 | **Grammar từ extension được biên dịch sang WASM** bằng wasi-sdk và chạy qua `WasmStore` của tree-sitter trên wasmtime. Grammar bundled vẫn native. | Có sandbox: grammar lỗi chỉ gây trap, không làm sập app. Zed cũng làm như vậy. Giai đoạn 3 vẫn cần wasmtime. |
| D3 | **Panel Extensions trong app**, cộng thêm lệnh CLI `luc ext …`. | Trải nghiệm giống VS Code; CLI tiện cho script và test. |
| D4 | **Snippet** được cài và kiểm tra, nhưng chưa dùng. | Lucerna vẫn chỉ đọc cho tới giai đoạn 4. |
| D5 | **Cập nhật:** tự kiểm tra định kỳ, người dùng bấm để cập nhật. | Không âm thầm tải và build code khi người dùng không yêu cầu. |
| D6 | **Injection tổng quát** theo quy ước của Zed. | Khoảng một phần ba extension ngôn ngữ cần injection. |
| D7 | **Thứ tự ưu tiên khi trùng:** người dùng, rồi extension, rồi built-in. Khi hai extension trùng nhau, extension có id lớn hơn theo alphabet thắng, kèm cảnh báo trong log. | Đi theo hướng chung của Zed và VS Code, nhưng luật được ghi rõ và có tính quyết định. |
| D8 | **Lấy dữ liệu bằng `git` và `curl` có sẵn trên macOS.** Không thêm crate mạng hay git. | Gọn. Giống `extension_builder` của Zed. Chạy được với mọi host git. |
| D9 | **Registry chỉ có id và version.** Tên và mô tả lấy từ manifest khi extension được cài hoặc khi người dùng mở mục đó. | Không phải fetch hơn 1.590 repo. |

### 1.4 Không có trong giai đoạn 2

- Host WASM và các hàm host (`download-file`, `process`, `nodejs`, `github`…), capability prompt, language server, slash command, context server. Phần nào thuộc giai đoạn 3 thì làm ở giai đoạn 3; các tính năng AI thì không bao giờ làm.
- Chèn snippet.
- Các query `outline`, `indents`, `textobjects`, `runnables`, `overrides`, `redactions`, `debugger`. Chúng được chép vào lúc cài nhưng chưa dùng.
- Grammar có scanner viết bằng C++ (`scanner.cc`). Zed cũng không hỗ trợ.
- Extension của VS Code, Open VSX, Microsoft Marketplace.
- Bật hoặc tắt extension theo workspace, gợi ý extension cho workspace.

## 2. Kiến trúc

### 2.1 Crate mới: `lucerna-extension`

Rust thuần, không phụ thuộc GPUI, `lucerna-syntax` hay `lucerna-theme`. Crate này chỉ trả về **dữ liệu**: đường dẫn và mô tả của những gì đã cài. Việc nạp dữ liệu đó vào các registry do `lucerna-workbench` (hoặc CLI) đảm nhận.

| Module | Trách nhiệm |
|---|---|
| `manifest` | Đọc `extension.toml` (schema 0 và 1) và `extension.json` cũ. Tự điền đường dẫn mặc định theo bố cục thư mục (§3.2). Đọc `config.toml` của từng ngôn ngữ (§3.3). |
| `registry` | Clone và fetch repo `zed-industries/extensions`. Ghép `extensions.toml`, `.gitmodules` và `git ls-tree` thành danh sách `RegistryEntry`. Cache ra `registry.json`. |
| `process` | Chạy tiến trình con (`git`, `curl`, `tar`, clang) có timeout, hủy được, giữ lại stderr. |
| `toolchain` | Đảm bảo có wasi-sdk 34: tải về, kiểm tra SHA-256, giải nén. Ưu tiên `WASI_SDK_PATH` nếu biến này trỏ tới một `bin/clang` có thật. |
| `builder` | Fetch mã extension, fetch và biên dịch grammar, kiểm tra tài nguyên, dựng thư mục trong `staging/`. |
| `validate` | Kiểm tra JSON của theme, icon theme và snippet; kiểm tra cú pháp body snippet theo TextMate. Việc nạp grammar và biên dịch query được gọi qua một trait `GrammarValidator`, do bên gọi cung cấp (§2.3). |
| `store` | Quản lý các extension đã cài: `install`, `uninstall`, `update`, `index()`. Dựng `index.json`. Phát sự kiện. Mỗi id chỉ có một thao tác tại một thời điểm. |

### 2.2 Các crate cũ được mở rộng

- **`lucerna-syntax`**
  - Bật feature `wasm` của tree-sitter (kéo theo `wasmtime-c-api-impl` 48).
  - `Language` có thêm nguồn gốc `Extension`.
  - Thêm hai thứ mới: injection tổng quát (§4.3) và `files.associations` (§4.2).
  - Cung cấp `WasmGrammarValidator`, một bản cài đặt của `GrammarValidator`.
- **`lucerna-theme`**
  - `ThemeRegistry` nạp theme theo từng lớp (§4.4).
  - Thêm `IconTheme` và `IconThemeRegistry` (§4.5).
  - Bảng icon Lucide hiện có trở thành icon theme built-in tên "Lucide". Phần dữ liệu ánh xạ chuyển sang `lucerna-theme`; các byte SVG vẫn nằm trong `lucerna-workbench`.
- **`lucerna-settings`:** thêm các khóa mới ở §6.
- **`lucerna-workbench`**
  - Panel Extensions và nút trên status bar.
  - Hoán đổi các registry khi có sự kiện `ExtensionsChanged`.
  - Vẽ icon theme bằng `img()`.
- **`lucerna` (bin `luc`):** thêm lệnh con `luc ext`.

### 2.3 Interface giữa các crate

- **`extension::Store`**
  - `open(root) -> Store`
  - `index() -> Arc<ExtensionIndex>`
  - `install(id, progress: impl Fn(InstallProgress), cancel: CancelToken) -> Result<InstalledExtension, InstallError>`
  - `uninstall(id)`, `update(id)`
  - `install_dev(dir)`
  - `subscribe() -> Receiver<StoreEvent>`
- **`ExtensionIndex`** chứa, cho từng extension:
  - manifest đã điền đủ;
  - commit;
  - danh sách `LanguageEntry { name, extension_id, dir, grammar: Option<GrammarRef { name, wasm_path }>, matcher: { path_suffixes, first_line_pattern }, hidden, queries: QueryFlags }`;
  - danh sách `ThemeEntry { name, appearance, extension_id, path }`;
  - danh sách `IconThemeEntry { name, appearance, extension_id, path, icons_root }`;
  - snippet: số lượng theo từng ngôn ngữ.
- **`trait GrammarValidator`**
  - `fn load(&self, name, wasm: &[u8]) -> Result<(), String>`
  - `fn check_query(&self, name, wasm, kind, source) -> Result<(), String>`

  `lucerna-syntax` cung cấp bản cài đặt thật. Test dùng bản giả.
- **`syntax::LanguageRegistry::build(bundled, extensions: &[LanguageEntry], associations: &Associations) -> LanguageRegistry`**
- **`theme::ThemeRegistry::build(builtin, extension_themes, user_dir)`**, **`theme::IconThemeRegistry::build(builtin_lucide, extension_icon_themes)`**

## 3. Cài đặt

### 3.1 Bố cục trên đĩa

Tất cả nằm trong `~/Library/Application Support/Lucerna/extensions/`:

```
registry/                 clone nông của zed-industries/extensions
registry.json             cache danh sách RegistryEntry + thời điểm fetch
toolchain/wasi-sdk-34/    wasi-sdk đã giải nén
work/<id>/src/            mã extension tại commit đã ghim (cache, xóa được)
work/<id>/grammars/<g>/   mã grammar tại rev đã ghim (cache, xóa được)
staging/<id>-<uuid>/      đang build
installed/<id>/
  extension.toml          manifest đã điền đủ
  lucerna.json            { commit, version, installed_at, source: "registry" | "dev" }
  grammars/<g>.wasm
  languages/<dir>/{config.toml,*.scm}
  themes/*.json
  icon_themes/*.json
  icons/**
  snippets/…              giữ nguyên đường dẫn tương đối như trong manifest
index.json                ExtensionIndex
logs/<id>.log             log của lượt cài gần nhất
```

### 3.2 Manifest

- **`extension.toml`**
  - **Bắt buộc:** `id`, `name`, `version`, `schema_version`.
  - **Không bắt buộc:**
    - `description`, `repository`, `authors`;
    - `lib`;
    - `themes[]`, `icon_themes[]`, `languages[]`;
    - `grammars.<name> = { repository, rev (hoặc commit), path? }`;
    - `snippets` (một chuỗi hoặc một mảng);
    - `language_servers`, `context_servers`, `slash_commands`, `capabilities`… (giữ lại ở dạng thô cho giai đoạn 3).
  - Khóa không nhận ra được bỏ qua, không báo lỗi.
- **Tự điền khi thiếu:**
  - `languages/<dir>/` có `config.toml`;
  - `themes/*.json`;
  - `icon_themes/*.json`;
  - `snippets.json` ở thư mục gốc.

  Mục khai báo tường minh được giữ nguyên; mục tìm thấy tự động được thêm vào nếu chưa có.
- **Schema 0:** xóa `languages`, `grammars` và `themes` rồi dựng lại từ thư mục. Đọc thêm `grammars/*.toml` (`repository`, `commit`, `path?`).
- **`extension.json` cũ:** id lấy theo tên thư mục, schema được coi là 0.
- **Từ chối** khi `schema_version` lớn hơn 1, hoặc khi tên grammar không phải snake_case.

### 3.3 `config.toml` của ngôn ngữ

Các trường được dùng:

- `name`;
- `grammar`;
- `path_suffixes`: khớp phần mở rộng, tên file đầy đủ, hoặc đuôi `.<suffix>`;
- `first_line_pattern`;
- `line_comments`, `block_comment`;
- `brackets`;
- `hidden`;
- `tab_size`.

Các trường khác được bỏ qua.

**Các file query được nhận:** `highlights`, `injections`, `brackets`, `outline`, `indents`, `textobjects`, `overrides`, `redactions`, `runnables`, `debugger` (đuôi `.scm`). File `.scm` không có trong danh sách này bị bỏ qua và ghi cảnh báo.

### 3.4 Làm mới registry

1. **Lần đầu:** `git clone --depth 1 --filter=blob:none --no-checkout https://github.com/zed-industries/extensions registry/`.
   **Các lần sau:** `git -C registry fetch --depth 1 origin main`, rồi `git -C registry update-ref HEAD FETCH_HEAD`.
2. Đọc ba nguồn và ghép theo id:
   - `git show HEAD:extensions.toml` cho `[<id>] submodule, version, path?`;
   - `git show HEAD:.gitmodules` cho URL theo submodule;
   - `git ls-tree HEAD extensions/` cho commit theo submodule.

   Mục nào thiếu URL hoặc commit thì bỏ qua và ghi cảnh báo.
3. Ghi `registry.json` gồm `fetched_at` và danh sách `RegistryEntry { id, version, url, commit, path }`. `name` và `description` được điền khi có (D9).
4. **Khi nào chạy:**
   - khi mở panel và `fetched_at` đã quá 24 giờ;
   - khi khởi động, nếu `extensions.autoCheckUpdates` bật và đã quá 24 giờ;
   - khi người dùng chạy lệnh "Extensions: Check for Updates".

**Lấy mô tả của một mục** (D9): khi người dùng mở rộng một dòng chưa cài, Lucerna fetch nông repo đó tại commit vào `work/<id>/src`. Bước này cũng là bước đầu của việc cài, nên không phí công. Sau đó đọc `extension.toml` và lưu `name` và `description` vào `registry.json`.

### 3.5 Cài một extension

**Kiểm tra trước:**
- Tìm thấy `<id>` trong registry.
- `git --version` chạy được.
- Ổ đĩa còn trên 1,2 GB.
- Không có thao tác nào khác đang chạy trên cùng id.

**Các bước**, mỗi bước phát `InstallProgress`:
1. **`Fetching`:**
   - Chạy `git init`, `git fetch --depth 1 <url> <commit>` và `git checkout FETCH_HEAD` trong `work/<id>/src`.
   - Nếu thư mục đã có sẵn ở đúng commit thì bỏ qua bước này.
   - Áp dụng `path`.
2. **`ReadingManifest`:**
   - Đọc và điền manifest.
   - `version` phải trùng với version trong registry.
   - Ghi nhận các phần chưa hỗ trợ để hiển thị trên panel.
3. **`PreparingToolchain`**, chỉ chạy khi extension có grammar:
   - Tải `https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-34/wasi-sdk-34.0-<arm64|x86_64>-macos.tar.gz` bằng `curl -fL`.
   - So SHA-256 với hằng số ghim trong code.
   - Giải nén bằng `tar` vào `toolchain/`.
4. **`BuildingGrammar(name, i, n)`:** với từng grammar:
   - Chạy `git init`, `git fetch --depth 1 <repository> <rev>` và `git checkout` vào `work/<id>/grammars/<name>`.
   - Áp dụng `path`. Nếu thiếu `src/parser.c` thì báo lỗi. Nếu có `src/scanner.cc` mà không có `scanner.c` thì báo lỗi "C++ scanner không được hỗ trợ".
   - Biên dịch:
     ```
     <wasi-sdk>/bin/clang -fPIC -shared -Os -Wl,--export=tree_sitter_<name> \
       -Wl,--max-memory=4294967296 -o <staging>/grammars/<name>.wasm \
       -I src src/parser.c [src/scanner.c]
     ```
   - Có cache: nếu `work/<id>/grammars/<name>.wasm` đã có, cùng rev và mới hơn mã nguồn, thì chép lại file đó thay vì biên dịch.
5. **`Validating`:**
   - Nạp từng grammar bằng `GrammarValidator::load`.
   - Biên dịch từng file `.scm` với grammar của nó.
   - Đọc JSON của theme và icon theme. Kiểm tra mọi `path` của icon nằm trong thư mục extension.
   - Đọc snippet.
   - Tài nguyên nào lỗi thì bỏ riêng tài nguyên đó và ghi vào `warnings`. Ngôn ngữ có grammar lỗi bị bỏ cả ngôn ngữ.
   - Nếu sau khi lọc không còn tài nguyên nào dùng được, lượt cài thất bại với lỗi `NothingUsable`.
6. **`Installing`:**
   - Chép tài nguyên vào `staging/<id>-<uuid>/` theo bố cục ở §3.1.
   - Ghi `extension.toml` (bản đã điền đủ) và `lucerna.json`.
   - Nếu đã có bản cài cũ, đổi tên `installed/<id>` thành `installed/.old-<id>-<uuid>`.
   - `rename` staging sang `installed/<id>`.
   - Dựng lại `index.json` và ghi nguyên tử: ghi ra file tạm rồi `rename`.
   - Xóa thư mục `.old-…` ở nền.
   - Phát `StoreEvent::Changed`.
7. **`Done { warnings }`**, hoặc **`Failed(InstallError)`**. Khi thất bại, Lucerna xóa `staging/` và giữ nguyên bản cài cũ.

**Hủy:** `CancelToken` kill tiến trình con đang chạy, xóa `staging/`, rồi trả `Failed(Cancelled)`.

**Timeout:** mỗi tiến trình `git` hoặc `curl` có 120 giây; mỗi lần biên dịch grammar có 300 giây.

**Log:** toàn bộ stdout và stderr của các tiến trình con được ghi vào `logs/<id>.log`, ghi đè ở mỗi lượt cài.

### 3.6 Cập nhật, gỡ, cài từ thư mục local

- **Có bản cập nhật** khi registry có `version` khác hoặc `commit` khác so với `lucerna.json` của bản đã cài. Cập nhật đi qua đúng luồng ở §3.5.
- **Gỡ:**
  - Đổi tên `installed/<id>` thành `installed/.trash-<id>-<uuid>`.
  - Dựng lại index, phát sự kiện.
  - Xóa thư mục `.trash-…` ở nền.
  - `work/<id>` được giữ lại làm cache.
- **Khởi động:** xóa mọi thư mục `.old-*`, `.trash-*` và `staging/*` còn sót lại.
- **`luc ext install --dev <dir>`:** build từ `<dir>` theo §3.5, bỏ bước 1 và bỏ kiểm tra version với registry, rồi chép vào `installed/<id>` với `source: "dev"`. Lucerna không dùng symlink. Muốn cập nhật thì chạy lại lệnh.
- **"Extensions: Clear Build Cache":** xóa `work/`.

### 3.7 Index

- `index.json` được dựng lại từ `installed/*`, bỏ qua các mục có tên bắt đầu bằng `.`.
- Với mỗi extension, Lucerna đọc `extension.toml`, `lucerna.json`, `config.toml` của từng ngôn ngữ (lấy matcher, grammar và cờ query), tên và `appearance` của theme và icon theme, và số snippet theo ngôn ngữ.
- **Khi khởi động:**
  - đọc `index.json`;
  - nếu file không có, hỏng, hoặc mtime của `installed/` mới hơn file, thì dựng lại ở nền;
  - trong lúc chờ, giao diện dùng index cũ hoặc index rỗng.
- Thư mục cài hỏng (thiếu hoặc lỗi `extension.toml`) được đánh dấu `broken` trong index, kèm lý do.
- **Watcher** cho `installed/` là một watcher `notify` riêng, debounce 200 ms. Nó nằm ngoài worktree, nên không vi phạm R21. Nhờ watcher, một lệnh `luc ext` chạy ở terminal khác cũng làm cửa sổ đang mở cập nhật theo.

## 4. Tích hợp lúc chạy

### 4.1 Grammar WASM

- Một `wasmtime::Engine` toàn cục, tạo lười khi lần đầu cần.
- Mỗi `SyntaxState`, và mỗi parser của một layer injection, dùng một `Parser`. Một `WasmStore` chỉ được gắn vào parser khi grammar của nó là WASM.
- Grammar được nạp lười. Lần đầu mở một file của ngôn ngữ đó, Lucerna đọc file `.wasm` và gọi `load_language` ở luồng nền, sau đó cache `tree_sitter::Language` trong `GrammarCache` theo khóa `(extension_id, grammar_name)`.
- **Tách namespace:** tên grammar chỉ có nghĩa trong phạm vi extension của nó. Hai extension cùng có grammar `toml` không ghi đè nhau.
- **Lỗi:**
  - Nạp lỗi, hoặc gặp trap lúc parse (tree-sitter trả lỗi hoặc không có cây): ngôn ngữ đó bị đánh dấu `failed` cho tới lần khởi động sau.
  - File hiển thị không tô màu và ghi log.
  - Panic được `catch_unwind` bắt lại (R3).

### 4.2 Chọn ngôn ngữ cho một file

`LanguageRegistry` gồm ba lớp: bundled, extension, associations. Thứ tự xét:

1. **`files.associations`:**
   - Áp dụng glob theo kiểu VS Code cho tên file. Nếu pattern có `/` thì áp dụng cho đường dẫn tương đối.
   - Giá trị là id ngôn ngữ (ví dụ `rust`, `typescriptreact`) hoặc tên ngôn ngữ của extension, không phân biệt hoa thường.
   - Nếu nhiều glob cùng khớp, glob dài nhất thắng.
2. **Tên file chính xác**, xét lớp extension trước lớp bundled.
3. **Suffix dài nhất:** so phần mở rộng và các đuôi `.<suffix>` theo khoảng cách. Xét lớp extension trước lớp bundled. Nếu hai extension cùng khớp ở cùng độ dài, extension có id lớn hơn theo alphabet thắng, và Lucerna ghi cảnh báo một lần.
4. **`first_line_pattern`:** khớp với dòng đầu tiên, tối đa 256 byte. Chỉ xét khi các bước trên không ra kết quả, và chỉ áp dụng cho ngôn ngữ từ extension.

**Ngôn ngữ ẩn:** ngôn ngữ có `hidden = true` không tham gia bước 2–4. Nó chỉ dùng được cho injection và cho `files.associations`.

**Id ngôn ngữ:**
- Ngôn ngữ từ extension dùng id là tên viết thường, với khoảng trắng thay bằng `-`. Ví dụ `Zig` thành `zig`.
- Id này là giá trị của context key `editorLangId` và của khóa override `"[zig]"`.
- Nếu id trùng với một ngôn ngữ bundled, ngôn ngữ của extension thay thế ngôn ngữ đó (D7).

### 4.3 Highlight và injection

**Precedence**
- Ngôn ngữ từ extension dùng `LastPatternWins`.
- Một capture chỉ được tính nếu `Theme::style_for` trả về style. Nhờ đó luật `@a @b` của Zed (thử `@b` trước) và việc bỏ qua capture có tiền tố `_` được giữ đúng.
- Ngôn ngữ bundled giữ precedence hiện tại.

**Predicate**
- Các predicate chuẩn do tree-sitter xử lý.
- Thêm `#has-parent?` và `#not-has-parent?`.
- Predicate lạ: pattern đó bị coi là không khớp, và Lucerna ghi cảnh báo một lần.

**Injection**
- Đọc `injections.scm`:
  - `@injection.content` (hoặc `@content`);
  - `@injection.language` (hoặc `@language`);
  - `#set! injection.language "<tên>"`;
  - `#set! injection.combined`.
- **Tra tên ngôn ngữ:**
  - Không phân biệt hoa thường, theo tên ngôn ngữ, id, rồi tới path suffix.
  - Nếu chuỗi có dấu `.`, chỉ lấy phần sau dấu chấm cuối. Ví dụ `foo.rs` thành `rs`.
- **Cây layer:**
  - `SyntaxState` giữ layer gốc và các layer con.
  - Mỗi layer con có ngôn ngữ, các khoảng byte (`included_ranges`) và một `Tree` riêng.
  - Nhiều vùng `combined` được gộp chung một layer.
  - Tối đa 4 tầng, và tối đa 1.000 layer con cho mỗi file. Vượt giới hạn thì không inject thêm.
- **Parse:**
  - Sau khi layer cha được parse, Lucerna chạy `injections.scm` trên toàn bộ cây cha, rồi parse các layer con.
  - Khi file thay đổi, các layer con được dựng lại.
  - Mọi việc này nằm trong cùng một lượt parse nền như hiện nay, và chịu chung ngân sách 1 ms chạy đồng bộ.
- **Highlight:**
  - Với các dòng đang hiển thị, Lucerna chạy `highlights.scm` của từng layer có giao với vùng đó.
  - Các span được gộp lại; layer sâu hơn thắng khi chồng lên nhau.
- **Ngôn ngữ inject chưa có:** vùng đó bỏ trống. Khi registry đổi, file được parse lại.
- **Markdown:**
  - Cơ chế inline viết tay ở Phase 1 bị bỏ, thay bằng `injections.scm` của `tree-sitter-md`: inline và code fence.
  - Query injection của các grammar bundled lấy từ crate grammar nếu crate có xuất (`INJECTIONS_QUERY`), nếu không thì từ repo upstream, kèm ghi chú trong `THIRD_PARTY_NOTICES.md`.

**Cặp ngoặc**
- Nếu ngôn ngữ có `brackets.scm` (yêu cầu `@open` và `@close`), Lucerna dùng query này để tìm cặp ngoặc.
- Nếu không có, Lucerna dùng thuật toán của Phase 1.

### 4.4 Theme

- **Thứ tự nạp vào `ThemeRegistry`:** built-in (One), rồi theme của các extension theo thứ tự id tăng dần, rồi `~/.config/lucerna/themes/*.json`. Lớp nạp sau thắng khi trùng tên. Lucerna ghi cảnh báo khi hai extension trùng tên theme.
- Theme JSON đọc theo schema v0.2.0 của Zed, như Phase 1, ở chế độ lenient: chấp nhận comment và dấu phẩy thừa.
- **Khi `workbench.colorTheme` trỏ tới một theme chưa có:**
  - Lucerna dùng One Dark nhưng giữ nguyên setting.
  - Mỗi khi registry được dựng lại, Lucerna áp dụng lại theme trong setting. Vì vậy cài extension xong là theme có hiệu lực ngay.

### 4.5 Icon theme

- **Schema** theo v0.3.0 của Zed: `{ name, author, themes: [{ name, appearance, directory_icons, named_directory_icons, chevron_icons, file_stems, file_suffixes, file_icons }] }`.
- **Đường dẫn icon** tính từ `installed/<id>/`. Đường dẫn nào ra ngoài thư mục đó bị từ chối.
- **Thứ tự tra một file:**
  1. tên file đầy đủ;
  2. các đuôi ghép từ dài tới ngắn (`a.b.c` thử `b.c`, rồi `c`);
  3. phần trước dấu chấm đầu tiên;
  4. `default`.

  Ở mỗi bước, tra `file_stems` trước, rồi `file_suffixes`. Kết quả là một type, rồi `file_icons[type].path`. Khi thiếu, lùi về icon theme "Lucide".
- **Thư mục:** tra `named_directory_icons[name]`, rồi `directory_icons`, rồi Lucide. Chevron cũng làm như vậy.
- **Built-in "Lucide":** là bảng `icon_for_path` hiện tại chuyển thành dữ liệu của icon theme. Explorer chỉ hỏi `IconThemeRegistry`.
- **Vẽ:**
  - Icon Lucide dùng `svg()` và màu `icon.muted`, như hiện nay.
  - Icon từ extension dùng `img(path)` để giữ màu gốc của SVG, kích thước như icon hiện tại.
- **Setting:** `workbench.iconTheme`, mặc định `"Lucide"`. Nếu giá trị trỏ tới icon theme chưa có, Lucerna dùng Lucide nhưng giữ nguyên setting, như §4.4.

### 4.6 Snippet

- Lúc cài, Lucerna đọc từng file snippet, định dạng `{ "<tên>": { prefix?, body, description? } }` (chấp nhận chuỗi hoặc mảng), và kiểm tra cú pháp body theo TextMate (`$1`, `${1:x}`, `${1|a,b|}`, `$0`, biến).
- Ngôn ngữ của một file snippet lấy theo stem của file, viết thường: `snippets` nghĩa là dùng chung, các tên khác là tên ngôn ngữ.
- `index.json` lưu số snippet hợp lệ theo từng ngôn ngữ. Lucerna chưa có runtime cho snippet.

### 4.7 Hoán đổi registry

- `workbench` nhận `StoreEvent::Changed` qua một task nền đọc từ `Store::subscribe()`.
- Ở luồng nền, nó dựng lại `LanguageRegistry`, `ThemeRegistry` và `IconThemeRegistry` từ index mới, rồi trên luồng UI thay các `Arc` trong `AppState`.
- Sau khi thay:
  - mỗi tab đang mở tra lại ngôn ngữ, và nếu khác thì parse lại;
  - theme và icon theme được áp dụng lại theo setting;
  - Explorer vẽ lại.
- Không cần khởi động lại.

## 5. Giao diện

### 5.1 Status bar

> Thay đổi 2026-11-09: người dùng chọn để Extensions là **trang riêng trong tab editor** giống Zed, thay cho panel ở sidebar. Mục 5.1 và 5.2 dưới đây là bản đã sửa.

- Thêm nút thứ ba ở góc trái, icon Lucide `blocks`, ngay sau Explorer và Search. Bấm vào thì mở (hoặc focus) tab Extensions. Nút này không bật/tắt sidebar.
- Nút có badge số khi có bản cập nhật, và có spinner khi đang cài.
- Phím tắt: Cmd+Shift+X, lệnh `workbench.view.extensions`.

### 5.2 Trang Extensions

- **Một tab duy nhất** tên "Extensions", mở trong nhóm editor đang active. Nếu tab đã mở ở bất kỳ nhóm nào thì chỉ focus nó. Tab không phải preview tab. Tab được lưu trong trạng thái workspace và mở lại khi khôi phục.
- Khi mở hoặc focus tab, con trỏ nằm trong ô tìm kiếm.
- Context key `extensionsViewletFocus` đúng khi tab Extensions có focus.
- **Đầu trang:**
  - Ô tìm kiếm lớn. Fuzzy match bằng `nucleo-matcher` trên id, cùng tên và mô tả nếu đã có.
  - Hàng nút lọc: **All · Installed · Updates · Themes · Icon Themes · Languages**. Vẫn gõ được `@installed`, `@updates`, `@themes`, `@icons`, `@languages`. Ba bộ lọc cuối chỉ áp dụng cho extension đã cài, vì registry không có thông tin về những gì extension cung cấp.
- **Danh sách thẻ**, ảo hóa (GPUI `list`, chiều cao thay đổi được). Extension đã cài đứng trước, rồi tới các mục chưa cài. Mỗi thẻ gồm:
  - dòng 1: tên (hoặc id nếu chưa biết tên), version, id bằng chữ mờ;
  - dòng 2: mô tả. Với mục chưa cài, mô tả được lấy theo §3.4 khi thẻ hiện lên màn hình lần đầu; trong lúc lấy hiện "Loading description…";
  - dòng 3: những gì extension cung cấp, ví dụ "2 themes · 1 language · 14 snippets (Phase 4)", kèm "Language server: Phase 3" nếu có; với mục chưa cài là repository và commit rút gọn;
  - bên phải: nút Install / Update / Uninstall / Reinstall, hoặc Cancel khi đang cài;
  - khi đang cài: bước hiện tại, ví dụ "Building grammar 2/3: zig";
  - khi xong mà có cảnh báo: danh sách cảnh báo;
  - khi lỗi: lỗi ngắn gọn bằng màu `status.error` và nút "Show Log", mở `logs/<id>.log` trong một preview tab.
- Bấm Install trong lúc đang lấy mô tả của cùng id: việc cài được xếp hàng chạy ngay sau khi lấy mô tả xong, nút chuyển sang trạng thái đang cài.
- **Trạng thái rỗng và lỗi chung:**
  - chưa có registry: "Đang tải danh sách…";
  - thiếu `git`: hướng dẫn chạy `xcode-select --install`;
  - fetch registry lỗi: thông báo kèm nút Retry, và vẫn hiện các extension đã cài.
- **Khi cài xong một extension có theme hoặc icon theme:** một thông báo nhỏ ở góc dưới phải hỏi "Dùng theme X?" kèm nút "Set Theme". Nút này ghi vào user settings.
- Phong cách giống Zed: lưới 4 px, nền hover là `ghost_element.hover`, thẻ có viền 1 px `border.variant`, bo góc 6 px.

### 5.3 Lệnh

| ID lệnh | Tên hiển thị |
|---|---|
| `workbench.view.extensions` | View: Show Extensions |
| `workbench.extensions.action.installExtension` | Extensions: Install Extension… (Quick Open liệt kê registry) |
| `workbench.extensions.action.checkForUpdates` | Extensions: Check for Updates |
| `workbench.extensions.action.updateAllExtensions` | Extensions: Update All |
| `lucerna.extensions.clearBuildCache` | Extensions: Clear Build Cache |
| `workbench.action.selectIconTheme` | Preferences: File Icon Theme |

Theme picker (`workbench.action.selectTheme`) có thêm theme từ extension. Icon theme picker dùng cùng widget, có xem trước khi di chuyển lựa chọn.

### 5.4 CLI

```
luc ext search <query>
luc ext list [--installed] [--updates]
luc ext install <id>... | --dev <dir>
luc ext remove <id>...
luc ext update [<id>...]
```

- Lệnh `luc ext` chạy không cần GUI, dùng `lucerna-extension` cùng `WasmGrammarValidator`.
- Tiến độ in ra stderr, mỗi bước một dòng.
- Mã thoát khác 0 khi có lỗi.

## 6. Settings mới

| Khóa | Mặc định | Ý nghĩa |
|---|---|---|
| `workbench.iconTheme` | `"Lucide"` | Tên icon theme |
| `files.associations` | `{}` | Glob → id hoặc tên ngôn ngữ, giống VS Code |
| `extensions.autoCheckUpdates` | `true` | Kiểm tra registry định kỳ (24 giờ) |

Context key mới cho `when` clause: `extensionsViewletFocus`.

## 7. Xử lý lỗi

| Tình huống | Cách xử lý |
|---|---|
| Thiếu `git`, mất mạng, fetch quá 120 s | Lượt cài thất bại, xóa `staging/`, panel hiện lỗi, log ở `logs/<id>.log` |
| Ổ đĩa còn dưới 1,2 GB | Chặn trước khi tải hoặc build |
| wasi-sdk sai SHA-256 | Xóa file đã tải, báo lỗi, không bao giờ chạy |
| Grammar biên dịch lỗi hoặc chỉ có `scanner.cc` | Bỏ các ngôn ngữ dùng grammar đó, phần còn lại vẫn được cài, panel hiện cảnh báo |
| Query không biên dịch được | Bỏ file query đó, kèm cảnh báo. Nếu là `highlights.scm` thì ngôn ngữ vẫn được cài nhưng không có màu. |
| Icon trỏ ra ngoài thư mục extension | Bỏ icon đó, kèm cảnh báo |
| Manifest sai version so với registry, hoặc `schema_version` lớn hơn 1 | Lượt cài thất bại |
| Grammar lỗi khi nạp hoặc parse | Ngôn ngữ bị đánh dấu `failed`, file hiển thị không tô màu, ghi log |
| `index.json` hỏng | Dựng lại từ `installed/*` |
| Thư mục trong `installed/` hỏng | Đánh dấu `broken` trong index, panel có nút Reinstall |
| `registry.json` hỏng | Xóa và fetch lại |
| Panic trong tác vụ cài hoặc dựng registry | `catch_unwind` (R3), chuyển sang `Failed`, editor vẫn chạy |
| Hai thao tác trên cùng một id | Thao tác sau bị từ chối với `Busy` |

## 8. Hiệu năng

- **Khởi động** chỉ đọc `index.json`, không mở `.wasm` hay `.scm`. Mục tiêu khung hình đầu dưới 200 ms vẫn giữ.
- **Nạp grammar WASM lần đầu** (Cranelift biên dịch) chạy ở nền. File hiện chữ trước, có màu sau.
- **Wasmtime** chỉ được khởi tạo khi lần đầu cần một grammar WASM. Mục tiêu RAM khi rỗi dưới 150 MB vẫn giữ.
- **Injection** chỉ tính highlight cho các dòng đang hiển thị. Thêm benchmark cho HTML có `<script>` lớn và Markdown có nhiều code fence, với cùng ngưỡng hồi quy 15% như các benchmark khác.
- **Benchmark mới:** parse và highlight một grammar WASM, so với bản native của cùng grammar (dùng fixture JSON). Chỉ ghi nhận, không đặt ngưỡng.

## 9. Giấy phép

- Thêm `Apache-2.0 WITH LLVM-exception` vào `deny.toml` (wasmtime, cranelift và các crate liên quan).
- wasi-sdk được tải về lúc chạy, không đóng gói. Lucerna không phân phối extension; người dùng tự build trên máy mình.
- **Không dùng code GPL của Zed.** Chỉ đọc định dạng dữ liệu (`extension.toml`, `config.toml`, các file `.scm`, schema của theme và icon theme) như một interface. Các tham chiếu tới mã nguồn Zed trong tài liệu nghiên cứu chỉ để hiểu hành vi.
- Query injection của các grammar bundled lấy từ crate grammar hoặc repo upstream của chúng, ghi trong `THIRD_PARTY_NOTICES.md`.
- File `.wasm` dùng làm fixture test được build từ một grammar MIT, kèm file license trong `tests/fixtures`.

## 10. Kiểm thử

- **Unit test:**
  - `lucerna-extension`:
    - manifest schema 0 và 1, `extension.json` cũ, tự điền đường dẫn, tên grammar không hợp lệ;
    - ghép registry từ ba nguồn;
    - so sánh để phát hiện bản cập nhật;
    - kiểm tra JSON của theme, icon theme và snippet;
    - chặn đường dẫn icon ra ngoài thư mục;
    - dựng `index.json`.
  - `lucerna-syntax`:
    - thứ tự chọn ngôn ngữ, gồm `files.associations`, tên file, suffix dài nhất, id hòa, `first_line_pattern`, `hidden`;
    - `LastPatternWins` kết hợp bỏ capture không có style;
    - `#has-parent?`.
  - `lucerna-theme`:
    - thứ tự ghi đè giữa các lớp theme;
    - thứ tự tra icon theme và lùi về Lucide.
- **Test tích hợp `lucerna-extension`, chạy offline:**
  - Repo git giả trong thư mục tạm, truy cập qua URL `file://`: một repo registry có `extensions.toml`, `.gitmodules` và submodule; một extension theme; một extension ngôn ngữ có grammar trỏ tới repo grammar giả.
  - Clang được thay bằng một script giả (qua `WASI_SDK_PATH`) chỉ chép file fixture `.wasm`.
  - Kiểm tra: install, update, uninstall, hủy giữa chừng, lỗi ở từng bước, cài lại khi `broken`, dọn `.old` và `.trash` khi khởi động.
- **Grammar WASM:**
  - Test nạp grammar dùng fixture `.wasm` đã build sẵn, commit trong repo. Test này không cần wasi-sdk.
  - Test build thật bằng clang có `#[ignore]` và chỉ chạy khi có `WASI_SDK_PATH`.
- **Injection:** snapshot test bằng `insta` cho:
  - HTML có `<script>` và `<style>`;
  - Markdown có inline và code fence; trường hợp này phải cho kết quả giống snapshot Phase 1 ở phần inline;
  - một trường hợp `injection.combined`;
  - giới hạn độ sâu.
- **UI** bằng `TestAppContext`:
  - panel lọc và tìm kiếm;
  - trạng thái nút theo từng `InstallProgress`;
  - badge cập nhật;
  - hoán đổi registry làm tab đang mở tô màu lại;
  - đổi icon theme làm Explorer vẽ lại;
  - setting trỏ tới theme chưa có, được áp dụng khi theme xuất hiện.
- **CLI:** test bằng `assert_cmd` cho `luc ext list`, `luc ext install --dev`, và lỗi khi id không tồn tại.
- **Thủ công**, trước khi đóng giai đoạn: các tiêu chí ở §1.2, kiểm tra trên registry thật.

## 11. Rủi ro

| Rủi ro | Cách giảm thiểu |
|---|---|
| Wasmtime làm tăng thời gian build và dung lượng `target/` (khoảng 1 GB), trong khi máy chỉ còn khoảng 12 GB | Không build release trong quá trình phát triển; `profile.dev` giữ `debug = false` cho dependency; kiểm tra dung lượng trống trước các bước nặng |
| Tree-sitter 0.27 với feature `wasm` có thể chưa hỗ trợ đúng như Zed đang dùng (Zed dùng một rev git của tree-sitter) | Task đầu tiên của plan là spike: nạp fixture `.wasm` bằng `WasmStore` trên tree-sitter 0.27. Nếu không được thì dừng lại và báo |
| Phiên bản ABI của grammar trong extension nằm ngoài khoảng tree-sitter 0.27 hỗ trợ | `load_language` báo lỗi rõ ràng; ngôn ngữ bị bỏ kèm cảnh báo khi cài |
| `img()` của GPUI không vẽ được SVG từ đường dẫn file, hoặc vẽ chậm | Kiểm tra trong plan; nếu cần thì rasterize SVG một lần và cache thành ảnh |
| Query của extension dùng predicate hoặc quy ước riêng của Zed mà Lucerna chưa hỗ trợ | Pattern có predicate lạ bị coi là không khớp và ghi cảnh báo; theo dõi trên Catppuccin, Zig, Vue, Svelte |
| Registry đổi định dạng `extensions.toml` | Đọc lenient, bỏ qua mục lỗi kèm cảnh báo, có test với dữ liệu thật đã lưu làm fixture |
| Injection làm chậm file lớn | Giới hạn 4 tầng và 1.000 layer con; tắt hẳn khi vượt ngưỡng file lớn như Phase 1 |

## 12. Thứ tự triển khai dự kiến

1. Spike: tree-sitter `wasm` cùng fixture grammar; `deny.toml`.
2. `lucerna-extension`: manifest, rồi registry, process và toolchain, builder và validate, cuối cùng store và index.
3. `lucerna-syntax`: grammar WASM và lớp ngôn ngữ từ extension, `files.associations`, precedence, sau đó tới injection tổng quát (gồm chuyển Markdown sang cơ chế mới) và `brackets.scm`.
4. `lucerna-theme`: theme theo lớp, icon theme, Lucide thành dữ liệu.
5. `lucerna`: lệnh `luc ext`.
6. `lucerna-workbench`: hoán đổi registry, vẽ icon theme, panel Extensions, lệnh và settings.
7. Benchmark, kiểm thử thủ công theo §1.2.
