# Nghiên cứu: hệ thống extension của Zed và VS Code

- **Ngày:** 2026-11-09
- **Trạng thái:** ghi chú nghiên cứu, làm nền cho spec Phase 2.
- **Nguồn:**
  - Zed: mã nguồn tại rev `b5cd310` (các crate `extension`, `extension_host`, `extension_cli`, `language`, `theme`, `file_icons`, `snippet_provider`) và `docs/src/extensions/`. Các crate này là GPL-3: chỉ học thiết kế và định dạng dữ liệu, không chép code.
  - VS Code: `microsoft/vscode` (MIT) tại commit `142e5584`, tài liệu `code.visualstudio.com/api`, wiki Open VSX.
- Mục nào chưa kiểm chứng trực tiếp trong mã nguồn được đánh dấu **[chưa kiểm chứng]**.

## 1. Kiến trúc tổng thể

| | Zed | VS Code |
|---|---|---|
| Code của extension | Component WASM (`extension.wasm`, WIT `since_v*`), chạy trong wasmtime, cùng tiến trình | JavaScript trong một tiến trình Node riêng (extension host) |
| Phần khai báo (không chạy code) | theme, icon theme, ngôn ngữ, grammar, snippet | `contributes.themes`, `iconThemes`, `languages`, `grammars` (TextMate), `snippets`, `configurationDefaults`… |
| Extension chỉ có dữ liệu | Không nạp `extension.wasm` | Không bao giờ được kích hoạt, thêm và gỡ được khi đang chạy, luôn được phép kể cả ở workspace chưa tin cậy |

**Kết luận cho Lucerna:** cả bốn thứ trong scope Phase 2 đều là dữ liệu. Phase 2 không cần chạy code của extension. Host WASM để sang Phase 3, cùng với LSP.

## 2. Phân phối và registry

- **Zed:**
  - Client chỉ gọi `api.zed.dev` để lấy danh sách, cập nhật và tải `archive.tar.gz` đã build sẵn.
  - Repo `zed-industries/extensions` có:
    - `extensions.toml`, mỗi mục là `[<id>] submodule = "extensions/<id>"`, `version = "x.y.z"`, `path = "subdir"` (không bắt buộc);
    - các git submodule đã ghim theo commit.
  - CI của repo đó build extension bằng `extension_cli`. Repo không chứa archive build sẵn.
  - Muốn không dùng `api.zed.dev` thì phải tự làm các bước: đọc `extensions.toml` và `.gitmodules`, lấy commit của submodule, fetch về, áp dụng `path`, rồi tự build.
- **VS Code:**
  - Gói `.vsix` là một file zip. Gallery là `POST {serviceUrl}/extensionquery`.
  - Bản OSS không cấu hình gallery nào.
  - Điều khoản của Microsoft Marketplace chỉ cho phép dùng với các sản phẩm của Microsoft. VSCodium và các editor bên thứ ba dùng **Open VSX** (Eclipse Foundation, có giới hạn tần suất truy cập).
  - Mỗi gói có chữ ký `.sigzip` và có danh sách chặn extension độc hại.
- **Kết luận:**
  - Lucerna tự build từ repo `zed-industries/extensions`, đúng như roadmap.
  - Không dùng Microsoft Marketplace. Nếu sau này cần extension VS Code, nguồn hợp lệ là Open VSX.

## 3. Bố cục trên đĩa, cài đặt và gỡ

- **Zed:** thư mục `~/Library/Application Support/Zed/extensions/` gồm:
  - `installed/<id>/`: bản đã build, có `extension.toml` đầy đủ, `grammars/<name>.wasm`, `themes/`, `icon_themes/`, `icons/`, `languages/<dir>/`, snippet;
  - `build/`: chứa wasi-sdk;
  - `staging/`: nơi giải nén trước khi đổi tên sang `installed/<id>`;
  - `index.json`: chỉ mục theme, icon theme và ngôn ngữ theo tên, kèm matcher và các file query có mặt. Được dựng lại khi mtime của `installed/` mới hơn.

  Extension dev là một symlink. Gỡ extension thì xóa thư mục rồi nạp lại.
- **VS Code:**
  - Thư mục `~/.vscode/extensions/<publisher>.<name>-<version>`, kèm `extensions.json` liệt kê các extension đã cài.
  - Cài đặt nguyên tử: giải nén vào `.<uuid>` rồi đổi tên.
  - Gỡ được hoãn lại qua file `.obsolete`: bỏ qua khi quét, xóa thật ở lần khởi động sau. Nhờ vậy cửa sổ đang chạy vẫn đọc được file cũ.
- **Kết luận:**
  - Dùng `staging/` rồi `rename` để cài đặt nguyên tử.
  - Có một file chỉ mục để khởi động nhanh mà không phải đọc lại mọi `config.toml`.
  - Gỡ và cập nhật cũng qua bước đổi tên. Vì Lucerna đọc grammar vào bộ nhớ khi nạp, xóa ngay là an toàn.

## 4. Build grammar

- **Zed (`extension_builder`):**
  - Lấy mã grammar: `git init`, `git fetch --depth 1 origin <rev>`, `git checkout <rev>` vào `<ext>/grammars/<name>/`.
  - Biên dịch:

    ```
    <wasi-sdk>/bin/clang -fPIC -shared -Os -Wl,--export=tree_sitter_<name> -Wl,--max-memory=4294967296 \
      -o grammars/<name>.wasm -I src src/parser.c [src/scanner.c]
    ```
  - wasi-sdk **34**, tải từ `github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-34/wasi-sdk-34.0-<arm64|x86_64>-macos.tar.gz`. Biến `WASI_SDK_PATH` được ưu tiên nếu có.
  - Chỉ hỗ trợ `scanner.c`. **Không hỗ trợ `scanner.cc` (C++).**
  - Tên grammar phải là snake_case.
- **Nạp lúc chạy:**
  - Có một `wasmtime::Engine` dùng chung cho toàn tiến trình.
  - Mỗi `Parser` trong pool có một `WasmStore` riêng.
  - Gọi `store.load_language(name, bytes)` một lần, rồi cache `tree_sitter::Language`.
  - Khi trùng tên, grammar native luôn thắng grammar từ extension.
- **VS Code:** dùng grammar TextMate (JSON/plist), không cần biên dịch. Không áp dụng được cho Lucerna.

## 5. Ngôn ngữ và query

- **`config.toml` của Zed:** các trường có ích cho trình xem là:
  - `name`, `grammar`;
  - `path_suffixes` (khớp phần mở rộng, tên file đầy đủ, hoặc đuôi `.<suffix>`);
  - `first_line_pattern`;
  - `line_comments`, `block_comment`;
  - `brackets`;
  - `hidden` (ngôn ngữ chỉ dùng để inject);
  - `tab_size`.
- **Các file query hợp lệ:** `highlights`, `brackets`, `outline`, `indents`, `injections`, `overrides`, `redactions`, `runnables`, `debugger`, `textobjects` (đuôi `.scm`). **Không có `folds.scm`.**
- **Thứ tự chọn ngôn ngữ:**
  - Zed: glob `file_types` của người dùng, sau đó path suffix khớp dài nhất, cuối cùng mới tới `first_line_pattern`.
  - VS Code: `files.associations` của người dùng, sau đó tên file chính xác, rồi glob dài nhất, rồi phần mở rộng dài nhất, cuối cùng `firstLine`. Khi hòa, ngôn ngữ đăng ký sau thắng.
- **Highlight trong Zed:**
  - Các capture được xếp chồng theo thứ tự lặp. Chỉ capture nào ánh xạ được sang style trong theme mới được tính.
  - Với cùng một node, **capture sau cùng có style thắng.** Trong một pattern `@a @b`, Zed thử `@b` trước. Giữa các pattern thì pattern sau thắng **[chưa kiểm chứng thứ tự capture của tree-sitter]**.
  - Tên capture lùi dần theo dấu chấm khi tìm style.
  - Capture có tiền tố `_` bị bỏ qua.
  - Predicate riêng chỉ có `#has-parent?` và `#not-has-parent?`.
- **Injection trong Zed:**
  - `@injection.content` (hoặc `@content`), `@injection.language`, `#set! injection.language`, `injection.combined`.
  - Tên ngôn ngữ được tra không phân biệt hoa thường, theo tên hoặc theo path suffix. Lồng được nhiều tầng. Lớp được inject thắng lớp chủ.
  - Nhiều extension phụ thuộc vào injection: HTML có JS/CSS, Vue, Svelte, PHP, Markdown có code fence.

## 6. Theme

- **Zed:**
  - Định dạng `{ name, author, themes: [{ name, appearance, style }] }`, schema v0.2.0, đọc theo kiểu lenient.
  - Extension theme không được cung cấp thêm thứ gì khác.
  - Khi trùng tên, theme nạp sau thắng.
- **VS Code:**
  - `workbench.colorTheme` lưu id hoặc label. Khi trùng, theme quét thấy đầu tiên thắng.
  - Nếu theme trong settings chưa được cài, VS Code dùng theme mặc định nhưng **không ghi đè setting**. Khi extension cung cấp theme đó được cài, theme tự được áp dụng.
- **Kết luận:**
  - Đặt luật hòa rõ ràng.
  - Giữ hành vi của VS Code: setting không bị xóa, và theme tự áp dụng khi xuất hiện.

## 7. Icon theme

- **Zed (schema v0.3.0):**
  - Định dạng `{ name, author, themes: [{ name, appearance, directory_icons, named_directory_icons, chevron_icons, file_stems, file_suffixes, file_icons{type: {path}} }] }`.
  - Đường dẫn icon tính từ thư mục gốc của extension.
  - Thứ tự tìm icon:
    1. tên file đầy đủ;
    2. các đuôi ghép, từ dài tới ngắn (`a.b.c`: `b.c`, rồi `c`);
    3. stem;
    4. phần mở rộng;
    5. `default`.

    Ở mỗi bước, tra `file_stems` trước rồi tới `file_suffixes`. Thiếu icon thì lùi về icon theme mặc định.
  - **SVG của icon theme được vẽ đủ màu bằng `img()`**, không tô một màu như icon có sẵn (`svg()` cộng `text_color`).
- **VS Code:**
  - `iconDefinitions`, `fileNames`, `fileExtensions`, `folderNames`, `languageIds`, các phần `light`/`highContrast`.
  - Icon có thể là SVG hoặc PNG, hoặc ký tự trong font.
  - Thứ tự tìm: tên file, rồi phần mở rộng dài nhất, rồi languageId, rồi mặc định.
- **Kết luận:**
  - Lucerna đọc schema của Zed và vẽ bằng `img()`.
  - Bộ Lucide hiện tại trở thành icon theme mặc định, có tên "Lucide".

## 8. Snippet

- Cả hai dùng định dạng JSON giống nhau: `{ "<tên>": { prefix, body, description } }`, body theo cú pháp TextMate.
- **Zed:**
  - Tên file quyết định snippet thuộc ngôn ngữ nào: `rust.json` cho Rust, `snippets.json` dùng chung cho mọi ngôn ngữ.
  - Body được kiểm tra bằng parser snippet.
  - Snippet được nạp vào bộ nhớ, không chép vào thư mục snippet của người dùng.

## 9. Cập nhật và tương thích

- **Zed:**
  - `schema_version` ≤ 1.
  - Nếu có `lib`, `wasm_api_version` phải nằm trong khoảng hỗ trợ.
  - Không có ràng buộc về phiên bản Zed.
  - Tự cập nhật khi semver ở registry cao hơn.
  - Có danh sách id bị chặn cứng, ví dụ `snippets`, `ruff`.
  - Chỉ kiểm tra kích thước file tải về, không có checksum.
- **VS Code:**
  - `engines.vscode` là bắt buộc.
  - Các setting `extensions.autoUpdate`, `extensions.autoCheckUpdates`.
  - Kiểm tra chữ ký.
- **Kết luận cho Lucerna:**
  - Ghim theo commit của submodule. Đây là một dạng kiểm tra toàn vẹn qua git hash, mạnh hơn cách của Zed.
  - Kiểm tra định kỳ, cập nhật thủ công, có `extensions.autoCheckUpdates`.
  - Extension có `lib` vẫn cài được phần dữ liệu. Phần language server sẽ hoạt động từ Phase 3.
