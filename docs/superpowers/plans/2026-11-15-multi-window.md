# Một tiến trình, nhiều cửa sổ — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Mọi cửa sổ Lucerna chạy trong một tiến trình `luc` (Dock một icon), mỗi cửa sổ vẫn có settings workspace, trust, terminal, language server riêng.

**Architecture:** `AppState.settings` chỉ còn lớp default và user; mỗi `Workbench` sở hữu một entity `WindowSettings` giữ lớp workspace và truyền handle xuống entity con. Một `Router` global giữ danh sách cửa sổ, mọi đường mở (CLI socket, Finder, menu, Welcome, restore) đi qua một hàm `router::open`. `luc` giành Unix socket trong thư mục data: giành được thì là app (mặc định tách khỏi terminal), không thì gửi đường dẫn cho app đang chạy.

**Tech Stack:** Rust, GPUI (`b5cd310`), `std::os::unix::net`, `libc` (đã có trong `crates/lucerna`), Python stdlib cho bench.

**Spec:** `docs/superpowers/specs/2026-11-15-multi-window-design.md` (đọc kèm; mã quyết định M1–M18 và mục § dưới đây trỏ vào spec). Nghiên cứu: `docs/research/2026-11-15-multi-window.md`.

## Global Constraints

- Nói chuyện với người dùng bằng tiếng Việt; code, comment, commit message bằng tiếng Anh như repo.
- `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"` trước cargo. Chạy `scripts/prune-target.sh` trước mỗi lần build/test. Không đặt `CARGO_TARGET_DIR`, không `--target-dir`. Giữ một profile trên đĩa (xoá `target/release` trước khi test debug, và ngược lại; không xoá khi `pgrep -x cargo` thấy cargo của session khác). Đĩa trống không dưới ~1.2 GB.
- Chỉ `lucerna-workbench` dùng GPUI. Không giả định macOS ngoài code nền tảng (`#[cfg(unix)]` cho socket; Windows để sau).
- Luồng UI không chờ I/O, trừ lần đọc đầu `.lucerna/settings.json` khi tạo workbench (như hôm nay `watch_config` đọc đồng bộ).
- Không chép code Zed (GPL). VS Code (MIT) tham khảo được, ghi nguồn nếu dùng lại.
- `cargo deny check` phải qua; `cargo clippy --workspace --all-targets -- -D warnings` và `cargo fmt --check` qua sau mỗi task.
- Commit: stage theo đường dẫn cụ thể, commit với `-- <paths>`, trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Không merge, không push, không rebase.
- Tên khóa và giá trị từ spec: `window.restoreWindows` mặc định `"all"`, nhận `"preserve" | "all" | "folders" | "one" | "none"`; lệnh `workbench.action.newWindow`, phím `cmd+shift+n` (macOS) / `ctrl+shift+n` (khác); file `<data dir>/windows.json`; socket `<data dir>/luc-<CARGO_PKG_VERSION>.sock`, thêm `-dev` khi `debug_assertions`; cờ ẩn `--foreground`.

## Review Focus

1. **Một entity con quên nhận `WindowSettings` và đọc settings user thay vì workspace** — người dùng đặt `editor.tabSize` trong `.lucerna/settings.json` mà một view (minimap, terminal, explorer) không theo. Test: Task 6 `window_settings_reach_every_reader` đặt một khóa workspace cho từng nhóm (editor, terminal, explorer, search, lsp, vim) và kiểm từng nơi đọc.
2. **Thư mục có symlink hoặc chữ hoa/thường khác** — `luc ./proj` và Finder mở `/Users/x/Proj` phải focus cùng một cửa sổ. Test: Task 7 `choose_focuses_canonical_root` (đường dẫn đã canonical ở `router::open`; test GPUI mở qua symlink).
3. **Đóng cửa sổ trong lúc hộp hỏi lưu của ⌘Q đang mở ở cửa sổ khác** — không panic, không thoát khi chưa ai đồng ý. Test: Task 9 `quit_survives_window_closed_during_prompt`.
4. **`luc` gọi liên tiếp nhanh khi app chưa chạy** (`luc a & luc b`) — chỉ một tiến trình app, cả hai thư mục mở. Test: Task 11 `claim_second_caller_becomes_client` (unit, socket tạm) và mục chạy tay.
5. **`windows.json` hỏng hoặc từ phiên bản lạ** — khởi động như danh sách rỗng, không panic, ghi đè bằng bản đúng ở lần ghi sau. Test: Task 10 `session_load_corrupt_is_empty`.

---

## File Structure

| File | Trách nhiệm |
|---|---|
| `crates/lucerna-settings/src/store.rs` | `WorkspaceLayer`, `SettingsView`, bỏ `Layer::Workspace` |
| `crates/lucerna-settings/src/scope.rs` (mới) | `is_app_scope(key)` |
| `crates/lucerna-workbench/src/window_settings.rs` (mới) | entity `WindowSettings`: lớp workspace, watcher `.lucerna`, lỗi, registry ngôn ngữ cửa sổ |
| `crates/lucerna-workbench/src/config_files.rs` | chỉ còn thư mục user |
| `crates/lucerna-workbench/src/window_choice.rs` (mới, thay `open_requests.rs`) | hàm thuần chọn cửa sổ (§6.3), `file_paths(urls)` |
| `crates/lucerna-workbench/src/router.rs` (mới, tách từ `app.rs`) | `Router` global, `open`, danh sách cửa sổ, `last_active`, hàng đợi restore |
| `crates/lucerna-workbench/src/session.rs` (mới) | `windows.json`: đọc, ghi, lọc theo `window.restoreWindows` |
| `crates/lucerna-workbench/src/cli_socket.rs` (mới) | giao thức, `claim`, `send`, `serve` |
| `crates/lucerna-workbench/src/app.rs` | `run`, `start`, khởi động, `QuitMode`, `on_reopen` |
| `crates/lucerna-workbench/src/process.rs` | **xoá** |
| `crates/lucerna-workbench/src/open_requests.rs` | **xoá** (thay bởi `window_choice.rs`) |
| `crates/lucerna/src/main.rs`, `crates/lucerna/src/launch.rs` (mới) | giành socket, client, tách khỏi terminal, `--foreground` |
| `bench/compare/multiwindow.py` (mới) + test | đo nhiều cửa sổ |
| `crates/lucerna-workbench/tests/multi_window.rs` (mới) | test GPUI nhiều cửa sổ |

---

### Task 1: Đo mốc trước khi chuyển

**Files:**
- Create: `bench/compare/multiwindow.py`, `bench/compare/tests/test_multiwindow.py`
- Modify: `bench/compare/README.md` (mục mới), `docs/benchmarks.md` (mục "Nhiều cửa sổ")

**Interfaces:**
- Produces: `python3 -I bench/compare/multiwindow.py --windows 1,2,3 --out <json>` và `--mode before|after`. Task 13 chạy lại với `--mode after`.

Đo ba số (§7.1) trên bản release dựng từ commit gốc nhánh (`git merge-base multi-window main`, hiện `2a9c1e3` + commit docs), `HOME` riêng như `editors.launch`:
- `ram_mb[n]`: tổng `footprint` (dùng parser có sẵn trong `metrics.py`) của mọi tiến trình `luc` thuộc phiên bench sau khi mở n cửa sổ trên n repo khác nhau và đợi `wait_quiet`. Mode `before`: mỗi cửa sổ là một lần chạy `luc <repo>`. Mode `after`: lần đầu `luc --foreground <repo1>`, sau đó `luc <repoK>` (client).
- `second_window_ms`: từ lúc gọi lệnh mở cửa sổ thứ hai tới khung hình đầu có nội dung của cửa sổ đó (`bench-helper watch ... coverage`, như `startup_ms`).
- `drop_pct_while_busy`: tỷ lệ khung hình rớt khi cuộn cửa sổ A (`bench-helper scroll`) trong lúc B mở một repo lớn (repo lớn của bench), so với khi B đứng yên. Hai cửa sổ cùng pid: đặt khung cửa sổ A (`frame`) lớn hơn B để helper chọn A. Nếu helper vẫn không phân biệt được, thêm `--window-index` vào `bench-helper` (Swift) và ghi trong README.

- [ ] **Step 1: Test phần thuần** (`test_multiwindow.py`): `summarize(rounds)` trả median/min/max cho từng chỉ số; `luc_commands("after", repos)` trả `[[bin, "--foreground", r1], [bin, r2], ...]`, `"before"` trả `[[bin, r1], [bin, r2], ...]`.
- [ ] **Step 2:** `python3 -m unittest discover -s bench/compare/tests -v` → FAIL (module chưa có).
- [ ] **Step 3:** Viết `multiwindow.py` (dùng lại `editors`, `metrics`, `env` có sẵn; 5 vòng mỗi chỉ số).
- [ ] **Step 4:** Unit test PASS.
- [ ] **Step 5:** Dựng release ở commit gốc (`scripts/bundle-app.sh`, theo quy tắc một profile), chạy `--mode before`, ghi JSON vào `bench/compare/results/multiwindow-before.json` và bảng vào `docs/benchmarks.md` mục "Nhiều cửa sổ — trước". Nếu không có quyền Screen Recording/Accessibility thì dừng và báo người dùng (chỉ RAM chạy được không cần quyền màn hình).
- [ ] **Step 6: Commit** `bench: multi-window measurement and the baseline before one process`.

### Task 2: `WorkspaceLayer` và `SettingsView` trong `lucerna-settings`

**Files:**
- Modify: `crates/lucerna-settings/src/store.rs`, `crates/lucerna-settings/src/lib.rs`
- Create: `crates/lucerna-settings/src/scope.rs`
- Test: trong `store.rs`/`scope.rs` (`#[cfg(test)]`)

**Interfaces:**
- Produces:
  - `pub struct WorkspaceLayer` (map đã parse; `Default` = rỗng) với `pub fn parse(jsonc: &str) -> Result<WorkspaceLayer, SettingsError>` và `pub fn app_scope_keys(&self) -> Vec<String>` (khóa phạm vi app có trong lớp, để ghi log).
  - `pub struct SettingsView<'a>` (`Copy`) với `pub fn get<T: DeserializeOwned>(&self, key: &str, language: Option<&str>) -> Option<T>` và `pub fn get_trusted<T>(..)`.
  - `impl SettingsStore { pub fn view<'a>(&'a self, workspace: Option<&'a WorkspaceLayer>) -> SettingsView<'a> }`. `SettingsStore::get`/`get_trusted` giữ nguyên = `view(None)`.
  - `pub fn is_app_scope(key: &str) -> bool` (scope.rs): `true` cho `workbench.colorTheme`, `workbench.preferredDarkColorTheme`, `workbench.preferredLightColorTheme`, `workbench.iconTheme`, `workbench.startupEditor`, mọi khóa bắt đầu `window.` hoặc `extensions.`.
  - `Layer` chỉ còn `Default`, `User` (bỏ `Workspace`: lỗi biên dịch dẫn các chỗ cần sửa).
- `SettingsView::get` khi lớp workspace có khóa phạm vi app: bỏ qua giá trị đó (tra như không có).

- [ ] **Step 1: Test** — `view_prefers_workspace`: user `{"editor.tabSize":4}`, workspace `{"editor.tabSize":2}` → `view(Some(&ws)).get::<u32>("editor.tabSize", None) == Some(2)`, `view(None)` → `Some(4)`. `view_language_override_order`: workspace `[rust]` > user `[rust]` > workspace > user (port các test thứ tự hiện có sang `view`). `view_merges_objects` (R26 qua ba lớp). `trusted_skips_workspace`. `app_scope_ignored_in_workspace`: workspace `{"workbench.colorTheme":"X","window.restoreWindows":"none"}` → `view(Some(&ws)).get::<String>("workbench.colorTheme", None)` bằng giá trị user/default. `is_app_scope` cho từng khóa trên và `false` cho `editor.tabSize`, `files.associations`.
- [ ] **Step 2:** `cargo test -p lucerna-settings` → FAIL.
- [ ] **Step 3:** Cài đặt; chuyển các test cũ dùng `Layer::Workspace` trong crate này sang `WorkspaceLayer`.
- [ ] **Step 4:** `cargo test -p lucerna-settings` → PASS. (`lucerna-workbench` chưa biên dịch: Task 3 sửa.)
- [ ] **Step 5: Commit** `settings: workspace layer as a separate value, views over user + workspace`. Có thể gộp commit với Task 3 nếu workspace không biên dịch giữa chừng; thứ tự review vẫn tách.

### Task 3: Entity `WindowSettings`; `ConfigState` chỉ còn user

**Files:**
- Create: `crates/lucerna-workbench/src/window_settings.rs`
- Modify: `config_files.rs`, `app.rs` (gọi `watch_config` một lần trong `start`), `workbench.rs` (tạo entity), `lib.rs` (export), status item lỗi cấu hình (trong `config_files.rs`)
- Test: `crates/lucerna-workbench/tests/window_settings.rs`, sửa `tests/config_files.rs`, `tests/trust.rs`, `tests/lsp_health.rs`, `tests/terminal_lifecycle.rs`, `src/git/tests.rs` (các chỗ `Layer::Workspace`)

**Interfaces:**
- Consumes: Task 2.
- Produces:
  - `pub struct WindowSettings`; `impl WindowSettings`:
    - `pub fn new(root: Option<&Path>, cx: &mut Context<Self>) -> Self` — `root` `None` cho cửa sổ file riêng. Đọc `<root>/.lucerna/settings.json` đồng bộ; observe global `AppState` để `cx.notify()` khi lớp user đổi; watcher `<root>/.lucerna` (không chạy khi `cx.background_executor().is_test()`), debounce `CONFIG_DEBOUNCE`.
    - `pub fn view<'a>(&'a self, cx: &'a App) -> SettingsView<'a>`
    - `pub fn get<T: DeserializeOwned>(&self, key: &str, language: Option<&str>, cx: &App) -> Option<T>`; `pub fn get_trusted<T>(..)` tương tự.
    - `pub fn reload(&mut self, cx: &mut Context<Self>)` — đọc lại file (luồng nền), cập nhật lớp, lỗi, `generation`, notify.
    - `pub fn set_workspace_json(&mut self, jsonc: &str, cx: &mut Context<Self>) -> Result<(), SettingsError>` — cho test.
    - `pub fn error(&self) -> Option<&ConfigError>`; `pub fn generation(&self) -> u64`; `pub fn settings_path(&self) -> Option<&Path>`.
  - `impl Workbench { pub fn settings(&self) -> &Entity<WindowSettings> }` — tạo trong `Workbench::build` trước mọi entity con.
  - `ConfigPaths` bỏ trường `workspace_settings` và `with_user_dir(_, root)`/`for_root`; còn `user_only()` (đổi tên `ConfigPaths::default_user()` không cần — giữ `user_only`, `user_only_with_user_dir`). `ConfigFile::WorkspaceSettings` chuyển sang `ConfigError` của `WindowSettings` (giữ `status_text` ".lucerna/settings.json: …").
  - Status item lỗi cấu hình: hiện lỗi `ConfigState` (user) rồi `WindowSettings::error()` của cửa sổ.
  - Lưu `.lucerna/settings.json` mở trong editor: workbench gọi `WindowSettings::reload` (thay cho việc `reload_config` thấy đường dẫn workspace).
- Bước này **chưa** đổi các chỗ đọc (Task 4–6): mọi chỗ vẫn đọc `AppState.settings` (giờ chỉ user), nên settings workspace tạm thời không có tác dụng ở các view. Các test hỏng vì lý do đó được đánh dấu trong ledger và phải xanh lại sau Task 6.

- [ ] **Step 1: Test** (`tests/window_settings.rs`): `reads_workspace_file` (tempdir có `.lucerna/settings.json` `{"editor.tabSize":2}` → `get::<u32>("editor.tabSize")==Some(2)`); `user_change_notifies` (`reload_config` user → observer của entity chạy, giá trị mới thấy được); `reload_picks_up_edit`; `broken_file_keeps_previous_and_reports` (`error()` có `status_text` bắt đầu `.lucerna/settings.json:`); `single_file_has_no_workspace` (`new(None)`); `two_entities_independent` (hai root, hai giá trị).
- [ ] **Step 2:** Chạy `cargo test -p lucerna-workbench --test window_settings` → FAIL.
- [ ] **Step 3:** Cài đặt; sửa `config_files.rs`, `app.rs` (`watch_config(ConfigPaths::user_only_with_user_dir(..))` một lần trong `start`/`Router` không gọi lại khi đổi thư mục), `workbench.rs`; sửa các test dùng `Layer::Workspace` sang ghi file `.lucerna/settings.json` trước khi tạo workbench hoặc `wb.settings().update(cx, |s, cx| s.set_workspace_json(..., cx))`.
- [ ] **Step 4:** `cargo test -p lucerna-workbench --test window_settings --test config_files` → PASS; `cargo build -p lucerna` qua.
- [ ] **Step 5: Commit** `workbench: per-window workspace settings entity; config state keeps the user files`.

### Task 4: Đọc settings theo cửa sổ — editor (documents, code view, excerpt list, vim, save, goto, fonts, symbols, editor groups, markdown preview)

**Files:**
- Modify: `documents.rs`, `code_view/mod.rs`, `code_view/transact.rs`, `code_view/minimap.rs`, `code_view/element.rs` (nếu đọc settings), `excerpt_list/render.rs`, `excerpt_list/mod.rs`, `vim.rs`, `save.rs`, `goto.rs`, `fonts.rs`, `symbols/breadcrumbs.rs`, `editor_groups/view.rs`, `markdown_preview/{mod,sync,commands}.rs`, `workbench.rs` (truyền handle)
- Test: `tests/multi_window.rs` (tạo file), sửa `tests/code_view*.rs`, `tests/excerpt_list*.rs` theo chữ ký mới

**Interfaces:**
- Consumes: `WindowSettings` (Task 3).
- Produces (chữ ký mới, các task sau và test dùng):
  - `Documents::new(root: PathBuf, settings: Entity<WindowSettings>) -> Self`
  - `CodeView::new(path: Arc<Path>, settings: Entity<WindowSettings>, cx: &mut Context<Self>) -> Self`
  - `ExcerptList::new(root: PathBuf, id_prefix: &'static str, settings: Entity<WindowSettings>, cx) -> Self`
  - `VimLayer::new(settings: Entity<WindowSettings>, cx) -> Self`
  - `MarkdownPreview::new(.., settings: Entity<WindowSettings>, ..)` (thêm tham số cuối trước `cx`)
  - `fonts::editor_font_family(settings: SettingsView<'_>) -> ..` (đổi tham số từ `cx: &App`)
  - Hàm tự do khác đang nhận `cx: &App` chỉ để đọc settings: đổi sang nhận `SettingsView<'_>`; caller lấy `self.settings.read(cx).view(cx)`.
  - `observe_global::<AppState>` dùng để nghe settings (vim.rs:172, code_view/mod.rs:449, workbench.rs:718) đổi sang `cx.observe(&settings, ..)`; chỗ nghe theme giữ `observe_global`.
- Ngôn ngữ của tài liệu (`Documents::language_for`) vẫn dùng `AppState.languages` ở task này; Task 5 đổi sang registry của cửa sổ.

- [ ] **Step 1: Test** (`tests/multi_window.rs`, helper `open_two(cx, ws_a: &str, ws_b: &str) -> (WindowHandle<Workbench>, WindowHandle<Workbench>)` mở hai `Workbench::new` ở hai cửa sổ trên hai tempdir có `.lucerna/settings.json` khác nhau): `editor_settings_per_window` — A `{"editor.tabSize":2,"editor.wordWrap":"on"}`, B `{"editor.tabSize":8}`; mở `a.rs` ở mỗi cửa sổ; tab size của indent (`Documents` indentation inputs) là 2 và 8; `CodeView` của A wrap, của B không. `workspace_change_stays_in_window` — `set_workspace_json` ở A đổi tabSize → B không đổi, A đổi.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** Truyền handle và đổi các chỗ đọc trong các file của task.
- [ ] **Step 4:** `cargo test -p lucerna-workbench --test multi_window` và các test editor bị ảnh hưởng (`code_view`, `documents`, `editing`, `vim*`, `markdown_preview`, `excerpt_list*`) → PASS.
- [ ] **Step 5: Commit** `workbench: editor views read the window's settings`.

### Task 5: Đọc settings theo cửa sổ — language servers, git, trust, `files.associations`

**Files:**
- Modify: `language_servers/{mod,sync,lifecycle}.rs`, `lsp_log.rs`, `lsp_health.rs`, `lsp_status.rs` (nếu đọc), `git/*` (khóa `git.*`, `scm.*`), `trust.rs:459`, `window_settings.rs` (registry), `documents.rs::language_for`, `extensions/service.rs` (registry chung chỉ dùng user; báo cửa sổ dựng lại khi registry chung đổi)
- Test: `tests/multi_window.rs`, sửa `tests/lsp_*.rs`, `src/git/tests.rs`

**Interfaces:**
- Consumes: Task 3, 4.
- Produces:
  - `LanguageServers::new(.., settings: Entity<WindowSettings>, ..)` (thêm tham số); `lsp`, `lucerna.lsp.enabled` đọc qua `get_trusted`/`get` của cửa sổ.
  - `GitStore` nhận `Entity<WindowSettings>` (đọc `git.enabled`, `git.path`, `git.statusLimit`, `git.diffBase`, `scm.*`).
  - `impl WindowSettings { pub fn languages(&self, cx: &App) -> Arc<LanguageRegistry> }` — `AppState.languages` nếu lớp workspace không có `files.associations`; ngược lại registry riêng dựng (luồng nền) từ cùng đầu vào extension với associations gộp user + workspace; dựng lại khi `AppState.languages_generation` đổi hoặc lớp workspace đổi. Thêm `pub fn languages_generation(&self) -> u64` để `Documents` biết tra lại.
  - Để dựng registry riêng: `extensions::service` cung cấp `pub(crate) fn registry_with_associations(assoc: Associations, cx: &App) -> Task<LanguageRegistry>` dùng lại `RegistryInputs` hiện có.

- [ ] **Step 1: Test:** `lsp_settings_per_window` — A workspace `{"lucerna.lsp.enabled":false}`, B không; dùng fake server của `tests/support/lsp.rs`, mở `a.rs` ở cả hai (cả hai trusted) → chỉ B khởi động server. `lsp_binary_ignores_workspace` (đã có ý trong test hiện tại; chuyển sang `WindowSettings`). `associations_per_window` — A `{"files.associations":{"*.foo":"rust"}}` → `x.foo` ở A là Rust, ở B là plain text. `git_disabled_per_window` — A `{"git.enabled":false}` → `GitStore` của A không quét, B quét.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** Cài đặt.
- [ ] **Step 4:** `cargo test -p lucerna-workbench` cho `multi_window`, `lsp_*`, `git_*`, `trust` và unit test trong `src/git` → PASS.
- [ ] **Step 5: Commit** `workbench: language servers, git and file associations follow the window's settings`.

### Task 6: Đọc settings theo cửa sổ — terminal, explorer, search, panel, phần còn lại của workbench; test kiến trúc

**Files:**
- Modify: `terminal/{settings,commands,links,panel,view}.rs`, `explorer/{view,menu,commands}.rs`, `search_view/view.rs`, `layout/panel.rs`, `workbench.rs` (1636, 2349, 2804, 3166), `onboarding/mod.rs` (chỉ đọc user: giữ `AppState`, Get Started không có cửa sổ workbench; trong tab của workbench thì đọc qua handle)
- Create: `crates/lucerna-workbench/tests/architecture_settings.rs`
- Test: `tests/multi_window.rs`, sửa `tests/terminal_*.rs`, `tests/explorer_*.rs`

**Interfaces:**
- Consumes: Task 3–5.
- Produces: hàm `terminal::settings::*`, `terminal::commands::skips_shell`, `explorer::commands::*` nhận `SettingsView<'_>` thay vì `&App`. `TerminalPanel::new(settings: Entity<WindowSettings>, window, cx)`.

- [ ] **Step 1: Test:** `window_settings_reach_every_reader` trong `multi_window.rs`: A workspace đặt `terminal.integrated.scrollback: 123`, `explorer.confirmDelete: false`, `workbench.panel.defaultLocation: "right"`, `vim.enabled: true`, `files.autoSave: "afterDelay"`; B không đặt. Kiểm từng giá trị qua API công khai của panel terminal, explorer (xoá không hỏi), vị trí panel, vim layer bật, auto-save — A theo workspace, B theo mặc định. `architecture_settings.rs`: quét `src/**/*.rs`, không file nào ngoài `config_files.rs`, `app.rs`, `router.rs`, `startup.rs`, `session.rs`, `extensions/service.rs`, `theme_picker.rs`, `onboarding/mod.rs`, `welcome.rs`, `window_settings.rs`, `app_menu.rs` chứa `AppState>().settings` hoặc `AppState>()\n            .settings`; không file nào chứa `process::spawn`/`SpawnRequest` (phần này bật ở Task 7: để `#[ignore]` kèm ghi chú tới khi Task 7 xong).
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** Cài đặt.
- [ ] **Step 4:** `cargo test -p lucerna-workbench` (toàn bộ) → PASS, kể cả các test đã ghi là tạm hỏng ở Task 3.
- [ ] **Step 5: Commit** `workbench: terminal, explorer and panels read the window's settings; guard test`.

### Task 7: Router nhiều cửa sổ và `router::open`

**Files:**
- Create: `window_choice.rs`, `router.rs`
- Delete: `process.rs`, `open_requests.rs`
- Modify: `app.rs` (Router chuyển ra `router.rs`; `run` dùng `router::open`), `open_commands.rs` (bỏ `SpawnRequest`, `spawn_or_report`, `WorkbenchEvent::Replace`), `welcome.rs`, `onboarding/mod.rs` (`OpenSettingsInNewWindow`), `recent_picker.rs`/`open_commands.rs` (⌘Enter), `lib.rs`, `crates/lucerna/src/main.rs` (bỏ `resolver`)
- Test: `window_choice.rs` (unit), `tests/multi_window.rs`, sửa tests dùng `WorkbenchEvent`/`Spawner`/`open_launch_for_test`/`router_shown_root_for_test`

**Interfaces:**
- Produces:
  - `window_choice.rs` (thuần, không GPUI):
    - `pub enum WindowKind { Welcome, Folder(PathBuf), File(PathBuf) }` (`Welcome` gồm cả Get Started)
    - `pub struct WindowInfo { pub kind: WindowKind, pub last_active: u64 }`
    - `pub enum Target { Auto, NewWindow, ReplaceIn(usize) }`
    - `pub enum Choice { Focus(usize), OpenTab(usize), ReplaceRoot(usize), NewWindow }`
    - `pub fn choose(windows: &[WindowInfo], path: &Path, is_dir: bool, target: Target) -> Choice` — quy tắc 1–4 §6.3 (index là vị trí trong slice).
    - `pub fn file_paths(urls: &[String]) -> Vec<PathBuf>` (chuyển từ `open_requests`).
  - `router.rs`:
    - `pub struct Launch` giữ nguyên trường; thêm `pub fn Launch::for_path(path: PathBuf) -> Result<Launch, String>` (thư mục → folder; file → single file với root là cha; không tồn tại → `"path not found: …"`), thay `Resolve`.
    - `pub enum Source { Cli, Finder, Menu, Restore }`
    - `pub struct OpenRequest { pub paths: Vec<PathBuf>, pub target: Target, pub source: Source }`
    - `pub fn open(request: OpenRequest, cx: &mut App) -> Result<(), String>` — canonical hoá, gọi `choose` cho từng đường dẫn, thực hiện, ghi recent, `app_menu::refresh`, (Task 10 thêm ghi `windows.json`). Lỗi gộp thành một chuỗi cho `Source::Cli`.
    - `pub fn open_welcome(target: Target, cx: &mut App)`; `pub fn open_onboarding(cx: &mut App)`.
    - `pub fn windows(cx: &App) -> Vec<(AnyWindowHandle, WindowKind)>` theo `last_active` giảm dần (dùng cho quit, session, test).
    - Router là `Global` (tạo lười bằng `default_global` nếu chưa có, để test dựng `Workbench::new` trực tiếp vẫn chạy); `last_active` cập nhật qua `observe_window_activation` của mỗi cửa sổ router mở; `on_window_closed` bỏ mục.
  - `Workbench`: `replace_with(Replacement, window, cx)` với `pub enum Replacement { Path(PathBuf), Welcome }` — sau `confirm_quit` gọi thẳng `router::open(.. Target::ReplaceIn(this window) ..)` hoặc `open_welcome(ReplaceIn)`; thư mục đã mở ở cửa sổ khác thì focus cửa sổ đó, không hỏi lưu (quy tắc 1 kiểm trước `confirm_quit`). `WorkbenchEvent` bị xoá.
  - Lệnh mới `workbench.action.newWindow` ("New Window"), phím trong `default-keybindings.json`, mục menu File đầu nhóm Open, mục Dock menu (`cx.set_dock_menu`) "New Window". Chạy được không cần cửa sổ: Task 9 đăng ký handler cấp app; ở task này đăng ký như lệnh workbench + Welcome.
  - Test hooks thay cho cũ: `#[doc(hidden)] pub fn router::open_for_test(paths: Vec<PathBuf>, target: Target, cx: &mut App)` và `router::windows`.

- [ ] **Step 1: Unit test `choose`:** `choose_focuses_open_folder`, `choose_focuses_open_single_file`, `choose_deepest_root_wins` (roots `/a` và `/a/b`, file `/a/b/c.rs` → cửa sổ `/a/b`), `choose_single_file_window_is_not_a_root` (cửa sổ file `/a/x.rs`, mở `/a/y.rs`, có cửa sổ thư mục `/z` dùng gần nhất → `OpenTab(z)`), `choose_loose_file_goes_to_last_active_folder`, `choose_loose_file_reuses_welcome`, `choose_loose_file_new_window_when_none`, `choose_folder_reuses_welcome` (Welcome dùng gần nhất trong hai Welcome), `choose_new_window_still_focuses_open` (`Target::NewWindow` + thư mục đã mở → `Focus`), `choose_replace_in` (`ReplaceIn(1)` thư mục chưa mở → `ReplaceRoot(1)`), `choose_focuses_canonical_root` (caller truyền đường dẫn canonical; test xác nhận so sánh đường dẫn đúng từng thành phần, `/a/bc` không nằm trong `/a/b`).
- [ ] **Step 2: GPUI test** (`multi_window.rs`): `open_two_folders_one_app` (hai `open_for_test` → `router::windows` có 2, cả hai `Folder`); `reopen_focuses_existing` (mở lại thư mục 1 → vẫn 2, cửa sổ 1 là `last_active` lớn nhất); `open_through_symlink_focuses` (symlink tới thư mục 1); `file_inside_b_opens_in_b` (A active, mở `b/src/x.rs` → tab ghim ở B, A không đổi); `new_window_then_pick_folder` (New Window → 3 cửa sổ, một `Welcome`; `WelcomeEvent::Open` thư mục mới trên Welcome đó → vẫn 3, mục đó thành `Folder`); `open_recent_cmd_enter_new_window`; `replace_releases_old_workbench` (giữ test W8: workbench cũ không upgrade được); `same_file_two_windows_reload` (M17: `a/x.rs` mở ở A và ở cửa sổ file riêng B; lưu ở A → B nhận thay đổi đĩa và nạp lại, như với mọi thay đổi ngoài).
- [ ] **Step 3:** FAIL.
- [ ] **Step 4:** Cài đặt; xoá `process.rs`, `open_requests.rs`, `Spawner`; bật phần `process::spawn` của test kiến trúc (Task 6).
- [ ] **Step 5:** `cargo test -p lucerna-workbench` → PASS; `cargo build -p lucerna`.
- [ ] **Step 6: Commit** `workbench: one router for every window; New Window; no more processes per window`.

### Task 8: `status_message` theo cửa sổ

**Files:**
- Modify: `status_message.rs` và 27 caller (`grep -rn "status_message::show"`), status bar đọc `current`
- Test: `tests/multi_window.rs`, sửa test đang dùng `status_message::` (13 chỗ)

**Interfaces:**
- Produces: `pub fn show(window: WindowId, text: impl Into<SharedString>, cx: &mut App)`; `pub fn current(window: WindowId, cx: &App) -> Option<SharedString>`; `observe` giữ nguyên chữ ký. Map `WindowId → (text, generation)`; mục bị bỏ khi cửa sổ đóng (router gọi `status_message::forget(window, cx)`). Caller trong workbench dùng `window.window_handle().window_id()`; caller không có `Window` (router) truyền cửa sổ đích; không có cửa sổ thì ghi log thay vì hiện.

- [ ] **Step 1: Test** `status_message_stays_in_window`: `show(a, "hi")` → `current(a) == Some("hi")`, `current(b) == None`; status bar của B không render "hi".
- [ ] **Step 2:** FAIL. **Step 3:** Cài đặt. **Step 4:** PASS toàn crate.
- [ ] **Step 5: Commit** `workbench: status messages belong to their window`.

### Task 9: Thoát, đóng cửa sổ cuối, Dock, action cấp app

**Files:**
- Modify: `app.rs` (`start`: bỏ observer quit; `run`: `application().with_quit_mode(QuitMode::Default)`, `on_reopen` → `router::open_welcome(Target::NewWindow)`), `workbench.rs::quit` (sắp theo `router::windows`, chịu cửa sổ đóng giữa chừng), `app_menu.rs` (handler cấp app cho M10 khi không có cửa sổ; trạng thái bật/tắt mục khi không có cửa sổ)
- Test: `tests/multi_window.rs`, `tests/app_menu.rs`

**Interfaces:**
- Consumes: `router::windows`, `router::open`, `router::open_welcome` (Task 7).
- Produces: `pub(crate) fn quit(cx: &mut App)` giữ tên; Task 10 thêm bước ghi `windows.json` đồng bộ ngay trước `cx.quit()` (để móc `session::save_now(cx)` ở đúng chỗ đó, task này để comment chỗ móc).
- Hành vi không cửa sổ (M10): `workbench.action.newWindow` → Welcome mới; `workbench.action.files.openFileFolder`/`openFolder`/`openFile` → hộp chọn của app (`cx.prompt_for_paths`) rồi `router::open(Target::NewWindow, Source::Menu)`; mục Open Recent của menu (`OpenRecentEntry`) → `router::open(Target::Auto)`; `workbench.action.openSettingsJson`/`openGlobalKeybindingsFile` (id có trong `config_files::register_commands`) → `router::open` file user ở `Target::NewWindow`; About như nay; Quit như nay.

- [ ] **Step 1: Test:** `quit_cancel_in_second_window_keeps_all` (hai cửa sổ có file bẩn; Quit; cửa sổ dùng gần nhất hỏi trước — `simulate_prompt_answer` "Don't Save" ở nó, "Cancel" ở cửa sổ kia → không thoát, hai cửa sổ còn); `quit_dont_save_both_quits` (dùng cờ test: thay `cx.quit` bằng hook ghi nhận, vì `TestPlatform::quit` là no-op); `quit_survives_window_closed_during_prompt` (đang hỏi ở A thì đóng B bằng `remove_window` → không panic; trả lời A → thoát); `close_last_window_keeps_app` (`QuitMode::Explicit` trong test; đóng hết → app vẫn còn, `router::windows` rỗng, menu: mục New Window bật, mục cần workbench tắt); `menu_open_folder_without_window` (`simulate_path_prompt_response` → một cửa sổ thư mục mới); `reopen_opens_welcome` (gọi hàm handler `app::on_reopen_for_test(cx)` → một Welcome).
- [ ] **Step 2:** FAIL. **Step 3:** Cài đặt. **Step 4:** PASS toàn crate.
- [ ] **Step 5: Commit** `workbench: quit asks every window, the app outlives its last window on macOS, Dock reopens Welcome`.

### Task 10: `windows.json`, `window.restoreWindows: "all"`, hàng đợi khi restore

**Files:**
- Create: `session.rs`
- Modify: `startup.rs` (`restores_last_workspace` → `restore_windows`), `app.rs::first_screen` và `open_without_folder`, `router.rs` (ghi session sau mỗi thay đổi; cờ `restoring`, hàng đợi), `workbench.rs::quit` (ghi đồng bộ), `crates/lucerna-settings/assets/default-settings.json` (`"window.restoreWindows": "all"`), `docs/superpowers/specs/2026-11-15-restore-last-workspace-design.md` (ghi chú: thay bởi spec nhiều cửa sổ)
- Test: `session.rs` (unit), `app.rs` tests (`first_screen_*`), `tests/multi_window.rs`

**Interfaces:**
- Produces:
  - `pub struct SessionWindow { pub kind: RecentKind, pub path: PathBuf }` (dùng `RecentKind::{Folder, File}` có sẵn); `pub struct Session { pub windows: Vec<SessionWindow> }` (dùng gần nhất trước).
  - `pub fn session_path(data_dir: &Path) -> PathBuf` (`windows.json`); `pub fn load(path: &Path) -> Session` (thiếu/hỏng/`version` ≠ 1 → rỗng, log); `pub fn save(path: &Path, session: &Session) -> io::Result<()>` (ghi nguyên tử, tên tạm theo pid như `recent.rs`).
  - `pub enum RestoreWindows { Preserve, All, Folders, One, None }`; `pub fn restore_windows(settings: &SettingsStore) -> RestoreWindows` (giá trị lạ → `All`).
  - `pub fn to_restore(session: &Session, mode: RestoreWindows, exists: impl Fn(&Path) -> bool) -> Vec<SessionWindow>` (đã lọc, thứ tự dùng gần nhất trước).
  - `StartupScreen::Restore(Vec<SessionWindow>)` thay `Restore(PathBuf)`; danh sách rỗng → fallback `recent.json` như cũ (một mục) trừ `None`.
  - `router::save_session(cx)` (nền) và `session::save_now(cx)` (đồng bộ, gọi trong `quit`).
  - Khởi động có đường dẫn: `Preserve` mở lại rồi mở đường dẫn; khác chỉ mở đường dẫn.

- [ ] **Step 1: Unit test:** `session_round_trip`; `session_load_corrupt_is_empty` (rác, `{"version":2}`, thiếu file); `to_restore_all_skips_missing`; `to_restore_folders_only`; `to_restore_one_takes_first`; `to_restore_none_is_empty`; `restore_windows_default_is_all` (với `SettingsStore::with_defaults()`); `first_screen_restores_all_windows`; `first_screen_empty_session_falls_back_to_recent`; `first_screen_none_ignores_session`.
- [ ] **Step 2: GPUI test:** `session_follows_open_and_close` (mở 2 → file có 2 theo thứ tự dùng gần nhất; đóng 1 → còn 1; Welcome không vào file); `restore_opens_in_reverse_order` (session 3 mục, 1 đã xoá → 2 cửa sổ, cửa sổ dùng gần nhất có `last_active` lớn nhất sau restore); `requests_wait_for_restore` (đặt `restoring`, `router::open` thư mục nằm trong session → xếp hàng; restore xong → không có cửa sổ trùng, đúng một cửa sổ cho thư mục đó).
- [ ] **Step 3:** FAIL. **Step 4:** Cài đặt. **Step 5:** PASS toàn crate.
- [ ] **Step 6: Commit** `workbench: remember open windows and reopen them all at startup`.

### Task 11: `luc` qua socket, tách khỏi terminal, `--foreground`

**Files:**
- Create: `crates/lucerna-workbench/src/cli_socket.rs`, `crates/lucerna/src/launch.rs`
- Modify: `crates/lucerna/src/main.rs`, `crates/lucerna/src/lib.rs`, `app.rs::run` (nhận listener), `router.rs` (kênh yêu cầu CLI, trả lời), `bench/compare/editors.py` (thêm `--foreground` khi `editor == "lucerna"`), `scripts/bundle-app.sh` (comment shim: `luc` tự tách khỏi terminal), `bench/compare/multiwindow.py` (mode after)
- Test: `cli_socket.rs` (unit), `crates/lucerna/tests/cli_socket.rs` (`#[ignore]`), `bench/compare/tests`

**Interfaces:**
- Produces:
  - `cli_socket.rs` (`#[cfg(unix)]`):
    - `pub const PROTOCOL_VERSION: u32 = 1;`
    - `#[derive(Serialize, Deserialize)] pub struct CliRequest { pub version: u32, pub cwd: PathBuf, pub paths: Vec<PathBuf>, #[serde(default)] pub new_window: bool }`; `pub struct CliReply { pub ok: bool, #[serde(default)] pub error: Option<String> }`.
    - `pub fn socket_path(data_dir: &Path) -> PathBuf` — `luc-<ver>[-dev].sock`; nếu `as_os_str().len() > 100` thì `std::env::temp_dir().join(format!("lucerna-{:016x}.sock", hash(data_dir)))`.
    - `pub enum Claim { Server(UnixListener), Client(UnixStream), Unavailable(io::Error) }`; `pub fn claim(path: &Path) -> Claim` — bind; `EADDRINUSE` → connect (timeout 1 s) → `Client`; connect `ECONNREFUSED` → xoá file, bind lại một lần.
    - `pub fn send(stream: UnixStream, request: &CliRequest, timeout: Duration) -> Result<CliReply, String>`.
    - `pub fn serve(listener: UnixListener, tx: futures::channel::mpsc::UnboundedSender<(CliRequest, futures::channel::oneshot::Sender<CliReply>)>)` — chạy trên `std::thread`.
  - `app::run(launch: Option<Launch>, listener: Option<UnixListener>)` (bỏ `Resolve`). Router nhận từ kênh, gọi `router::open(Source::Cli, Target::Auto)` (`paths` rỗng → `[cwd]`), trả `CliReply`; `version > PROTOCOL_VERSION` → `ok:false`, `error:"this Lucerna is older than luc; update Lucerna"`. Thoát app xoá file socket (`on_app_quit`).
  - `crates/lucerna/src/launch.rs`: `pub fn spawn_detached(exe: &Path, args: &[OsString], cwd: &Path) -> io::Result<()>` — `Command` + `pre_exec(libc::setsid)`, stdio `/dev/null`, không đợi (thread reap như `process.rs` cũ). `pub fn send_with_retry(path: &Path, request: &CliRequest, total: Duration) -> Result<CliReply, String>` (thử nối mỗi 50 ms tới 10 s).
  - `main.rs`: thêm `#[arg(long, hide = true)] foreground: bool`; `--trace-startup` bật `foreground`. Luồng: parse → `ext`/`lsp` như cũ → resolve đường dẫn (lỗi → in, mã 1) → `claim(socket_path)`:
    - `Client` → `send` → in lỗi/thoát mã 0 hoặc 1.
    - `Server` và `foreground` (hoặc cwd `/` không đối số: mở từ Finder/Dock) → `app::run(.., Some(listener))`.
    - `Server` và không `foreground` → thả listener (xoá file), `spawn_detached(current_exe, ["--foreground", <path>], cwd)`, `send_with_retry`; trả mã theo trả lời (hết giờ: in "Lucerna is starting; could not confirm the window opened", mã 1).
    - `Unavailable` → cảnh báo stderr, `app::run(.., None)` tại chỗ.

- [ ] **Step 1: Unit test** (`cli_socket.rs`, socket trong `tempfile::tempdir`): `claim_first_is_server`; `claim_second_caller_becomes_client` (server đang nghe → `Client`); `claim_stale_socket_rebinds` (file socket không ai nghe → `Server`); `request_round_trip` (server giả trả `ok`); `unknown_fields_ignored`; `newer_version_rejected` (hàm thuần `reply_for_version`); `long_data_dir_uses_temp` (đường dẫn > 100 byte).
- [ ] **Step 2: Test tích hợp** `crates/lucerna/tests/cli_socket.rs` `#[ignore]`: `HOME` tạm; `luc <dir1>` thoát < 10 s, còn đúng một tiến trình `luc --foreground`; `luc <dir2>` mã 0, số tiến trình không đổi; `luc /nope` mã 1, stderr có `path not found`; kill tiến trình app cuối test.
- [ ] **Step 3:** Unit FAIL. **Step 4:** Cài đặt. **Step 5:** `cargo test -p lucerna-workbench cli_socket` và `cargo test -p lucerna` PASS; chạy tay `cargo test -p lucerna --test cli_socket -- --ignored` trên máy có màn hình (ghi kết quả trong ledger). `python3 -m unittest discover -s bench/compare/tests -v` PASS.
- [ ] **Step 6: Commit** `luc: hand paths to the running app over a socket and leave the terminal`.

### Task 12: Trust đồng bộ giữa cửa sổ

**Files:**
- Modify: `trust.rs` (sự kiện app-level), `workbench.rs` (nghe và đọc lại trust)
- Test: `tests/multi_window.rs`, `tests/trust.rs`

**Interfaces:**
- Produces: global `TrustChanged { generation: u64 }` (hoặc `cx.observe_global::<TrustGeneration>`); cửa sổ ghi `trusted.json` xong thì tăng; mỗi `Workbench` observe, đọc lại trust của gốc mình ở luồng nền (`TrustStore` có sẵn) và gọi `trust_loaded` nếu đổi.

- [ ] **Step 1: Test** `trust_in_a_reaches_b`: A gốc `/t`, B gốc `/t/sub`, cả hai chưa tin; A tin `/t` cùng thư mục cha (`parents`) → sau `run_until_parked`, `LanguageServers` của B `is_trusted()`. `trust_other_folder_leaves_b`.
- [ ] **Step 2:** FAIL. **Step 3:** Cài đặt. **Step 4:** PASS.
- [ ] **Step 5: Commit** `workbench: trusting a folder in one window updates the others`.

### Task 13: Đo sau, ngưỡng, tài liệu

**Files:**
- Modify: `docs/benchmarks.md`, `bench/compare/results/multiwindow-after.json`, `docs/superpowers/specs/2026-11-12-welcome-design.md` (W1, W8, ghi "thay bởi 2026-11-15-multi-window-design.md" ở chỗ nói một tiến trình một cửa sổ), `README.md` nếu có câu về nhiều tiến trình, `crates/lucerna-workbench/src/lib.rs`/module docs nói "a process shows one window"
- Test: không thêm

- [ ] **Step 1:** Dựng release ở đầu nhánh; chạy `multiwindow.py --mode after` 5 vòng; chạy `bench/compare` `startup_ms` và `idle_ram_mb` một cửa sổ (mốc: cùng lệnh ở Task 1 — nếu Task 1 chưa đo hai số này thì đo ở commit gốc trước).
- [ ] **Step 2:** So ngưỡng §7.2. Không đạt → dừng, ghi số, báo lại (không coi là xong). Đạt → ghi bảng trước/sau vào `docs/benchmarks.md`.
- [ ] **Step 3:** `grep -rn "one window\|own process\|a process shows" crates docs README.md` và sửa câu cũ.
- [ ] **Step 4:** `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, `cargo deny check` → qua.
- [ ] **Step 5: Chạy tay trên macOS** (ghi kết quả vào ledger): Dock một icon với 3 cửa sổ mở từ menu, Welcome ⌘bấm, Finder Open With, `luc` trong terminal; ⌘` đi qua cả 3; App Exposé đủ 3; ⌘Q với file bẩn hai cửa sổ; đóng hết cửa sổ rồi bấm Dock → Welcome; thoát rồi mở lại → đủ cửa sổ.
- [ ] **Step 6: Commit** `docs: multi-window measurements and notes`.
