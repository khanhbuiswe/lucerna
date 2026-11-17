# LSP: root mặc định là thư mục ngoài cùng

Thay đổi nhỏ. Ngày 2026-11-17.

## Vì sao

Mở dự án Laravel (`noti.one`, code ở `apps/api`) rồi go-to-definition vào
`apps/api/vendor/laravel/framework/...` làm Lucerna khởi động intelephense thứ
hai. Lý do: root của server là thư mục **gần nhất** có marker, mà mỗi gói trong
`vendor/` đều có `composer.json`, nên root thành `vendor/laravel/framework`.
Server thứ hai index lại từ đầu, không thấy code app, và hiện thành dòng "trùng"
trong popover LSP.

Không riêng PHP. Mọi hệ sinh thái tải thư viện vào trong dự án, mỗi gói kèm
marker, đều bị:

- `node_modules/<gói>/package.json`: typescript-language-server, vtsls,
  vue-language-server, vscode-css, vscode-html.
- `deps/<gói>/mix.exs`: elixir-ls.
- `.build/checkouts/<gói>/Package.swift`: sourcekit-lsp.
- `build/packages/<gói>/gleam.toml`: gleam.
- `_build/default/lib/<gói>/rebar.config`: elp.

VS Code, Zed và Helix đều không chọn "gần nhất" (xem Tham khảo). Lucerna là
ngoại lệ, và các spec cũ không ghi lý do chọn "gần nhất".

## Mục tiêu

- Nhảy vào code thư viện nằm trong dự án không sinh thêm server.
- Root mặc định khớp Helix: thư mục ngoài cùng có marker, từ file lên tới root
  workspace.

## Không làm

- Dùng chung một process cho nhiều workspace folder (`didChangeWorkspaceFolders`,
  như Helix). Lớn hơn, để sau.
- Setting chia monorepo thành nhiều root kiểu `workspace-lsp-roots` của Helix.
- Danh sách thư mục thư viện cần bỏ qua (cách A); không cần nữa khi chọn ngoài
  cùng.
- Sửa popover để phân biệt hai dòng trùng tên server.

## Cách làm

Các cách đã cân nhắc:

- **A.** Bỏ qua `vendor/`, `node_modules/`, `deps/`... khi tìm root. Sửa mọi
  server nhưng phải giữ danh sách thư mục viết sẵn.
- **B.** Dùng lại server đang chạy nếu root của nó chứa file. Phụ thuộc thứ tự
  mở file.
- **C.** Chỉ cho intelephense/phpactor chọn ngoài cùng. Sửa một dòng dữ liệu,
  nhưng JS/TS, Elixir, Swift... vẫn bị.
- **D.** Đổi mặc định của mọi server sang ngoài cùng, như Helix. Một chỗ sửa,
  không cần danh sách thư mục. Đánh đổi: monorepo JS/Python chạy một server cho
  cả repo (VS Code và Helix cũng vậy). **Người dùng chọn cách này.**

Thay đổi:

- `RootSearch` trong `crates/lucerna-lsp/src/adapters.rs`: `#[default]` chuyển
  từ `Nearest` sang `Outermost`. `Nearest` vẫn giữ, người dùng đặt được qua
  `lsp.<server>.root_search: "nearest"` (setting `lsp` merge vào bảng mặc định).
- `servers.json`: gopls giữ `"ordered"` (ưu tiên `go.work` rồi `go.mod`, đúng ý
  Go). rust-analyzer giữ dòng `"outermost"` dù giờ trùng mặc định,
  vì nó nói rõ ý định (quyết định của mình).
- Test trong `crates/lucerna-lsp/tests/adapters.rs`:
  - Sửa `ts_nearest_package_json`: monorepo có `package.json` ở gốc và
    `packages/web` giờ ra root là gốc. Đổi tên theo hành vi mới.
  - Sửa vế thứ hai của `glob_marker_finds_sln`: có cả `a.sln` ở gốc và
    `src/App/App.csproj` thì giờ ra gốc.
  - Mới: intelephense, file trong `apps/api/vendor/laravel/framework/` và file
    trong `apps/api/app/` ra cùng root, cả khi có và không có `.git` ở gốc.
  - Mới: typescript-language-server, file trong
    `node_modules/<gói>/` ra cùng root với file của dự án.
  - Mới: đặt `root_search: "nearest"` qua setting thì vẫn ra thư mục gần nhất.
- Quyết định của mình: không sửa các spec cũ (lịch sử). Ghi một dòng vào
  `docs/research/2026-11-11-lsp-server-table.md` rằng Helix chọn marker ngoài
  cùng, để lần sau khỏi tra lại.

Hệ quả cần biết:

- `.git` chỉ còn là marker khi nó là marker duy nhất của server (yaml, json,
  bash). Mười lăm server có `.git` kèm marker dự án (intelephense, phpactor,
  ruby-lsp, jdtls, sourcekit-lsp, lua, css/html...) bỏ `.git`, vì với cách tìm
  ngoài cùng, `.git` ở gốc repo sẽ luôn thắng `composer.json`, `Gemfile`,
  `pom.xml` ở thư mục con. Không có marker nào thì vẫn dùng root workspace, như
  trước. (Phát hiện khi review; test `git_is_only_ever_a_sole_root_marker`.)
- Elixir umbrella lấy `mix.exs` ở gốc umbrella, Maven/Gradle nhiều module lấy
  `pom.xml`/`settings.gradle` cha. Đó là root mà các server này muốn.
- Server đang chạy với root cũ không tự đổi; người dùng cần Restart hoặc mở lại
  app. Bảng mặc định nằm trong binary nên thay đổi đến cùng bản build mới.

## Tham khảo

- **VS Code:** extension intelephense (`src/extension.ts`) tạo một
  `LanguageClient` cho cả cửa sổ, không truyền root riêng; file trong `vendor/`
  đi vào cùng server đó.
- **Zed:** `crates/project/src/manifest_tree.rs`.
  - Server không khai báo manifest dùng root của worktree
    (`root_for_path_or_worktree_root`). Extension PHP không khai báo manifest,
    nên mỗi worktree có một intelephense.
  - Server có manifest chọn thư mục **ngoài cùng**: `CargoManifestProvider`
    (`crates/languages/src/rust.rs`), `PyprojectTomlManifestProvider` (đổi sang
    ngoài cùng ở PR #53781 cho workspace uv/Poetry).
  - Không có luật riêng cho `vendor/`/`node_modules/`.
- **Helix:** `find_lsp_workspace` trong `helix-lsp/src/lib.rs`.
  - Đi từ thư mục của file lên, mỗi marker gặp được ghi đè kết quả trước, nên
    thư mục **cao nhất** có marker thắng. Dừng ở root workspace hoặc ở một thư
    mục trong `workspace-lsp-roots`. Không có marker thì dùng root workspace.
  - Không có luật riêng cho thư mục thư viện.
  - `try_add_doc` trong `helix-lsp/src/client.rs`: root trùng client đang chạy
    thì dùng lại; root khác mà server hỗ trợ nhiều workspace folder thì thêm
    folder qua `didChangeWorkspaceFolders` thay vì mở process mới.
