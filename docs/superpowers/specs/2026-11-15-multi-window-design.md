# Một tiến trình, nhiều cửa sổ — thiết kế

Ngày: 2026-11-15. Trạng thái: người dùng đã duyệt (2026-11-15).

**Các điểm người viết spec tự quyết** (người dùng kiểm khi duyệt): M5 (file
ngoài thư mục đang mở vào cửa sổ dùng gần nhất), M7, M10, M13 (giành socket
trước), M16, M17; §6.1.3 danh sách khóa phạm vi app và `files.associations`
theo cửa sổ; §6.3 cửa sổ file riêng không nhận thêm file; §6.5 không chở
env, không dùng `open -a`; §6.6 định dạng `windows.json`, fallback
`recent.json`, Welcome không được mở lại; §7.2 mốc đo là bước 1.

Nghiên cứu: `docs/research/2026-11-15-multi-window.md` (Zed ở revision GPUI
đang pin `b5cd310`, VS Code nhánh `main`).

## 1. Ý định

Hiện nay mỗi cửa sổ Lucerna là một tiến trình `luc` riêng (spec Welcome W1,
W8): settings workspace là global gắn với một thư mục gốc, nên một tiến trình
chỉ giữ được một cửa sổ. Hậu quả người dùng thấy: mở hai cửa sổ thì Dock có
hai icon Lucerna, App Exposé (Ctrl+↓) và ⌘` chỉ thấy cửa sổ của một tiến
trình. VS Code và Zed đều là một app với nhiều cửa sổ. Lucerna sắp ra 0.1 và
cần làm giống vậy.

Thành công nghĩa là:

- Dock luôn chỉ có một icon Lucerna, dù mở bao nhiêu cửa sổ, mở từ đâu
  (menu, Welcome, Finder, `luc` trong terminal).
- ⌘` đi vòng qua mọi cửa sổ; App Exposé hiện đủ.
- Mỗi cửa sổ vẫn hành xử như một workspace độc lập: settings workspace,
  trust, terminal, language server, layout.
- Một cửa sổ không chậm hơn bản hiện tại (§9).

## 2. Mục tiêu, không làm, phản mục tiêu

**Mục tiêu**

- Mọi cửa sổ trong một tiến trình. Bỏ `process::spawn` khỏi mọi đường mở.
- Settings: lớp default và user dùng chung; lớp workspace theo cửa sổ.
- Router quản lý nhiều cửa sổ; một hàm mở duy nhất cho mọi nguồn.
- `luc <path>` gửi đường dẫn tới tiến trình đang chạy qua Unix socket.
- Thoát và đóng cửa sổ cuối như VS Code.
- `window.restoreWindows: "all"`: mở lại mọi cửa sổ khi khởi động.
- Test GPUI cho nhiều cửa sổ.
- Đo hiệu năng trước và sau.

**Không làm (để sau 0.1)**

- Cờ CLI `--new-window`, `--reuse-window`, `--add`, `--goto`, `--wait`.
  Giao thức socket chừa chỗ cho chúng (§6.3).
- `window.openFoldersInNewWindow`, `window.openFilesInNewWindow`: hành vi
  cố định theo mặc định của VS Code (§5.3), chưa có setting.
- Khôi phục vị trí và kích thước từng cửa sổ. Cửa sổ vẫn mở phóng to như nay.
- Hot exit (lưu ngầm file chưa lưu khi thoát).
- Tab cửa sổ native của macOS (`tabbing_identifier` để `None`).
- Workspace nhiều thư mục (multi-root), sidebar nhiều project của Zed.
- Dùng chung language server giữa các cửa sổ.
- Linux/Windows: socket và `QuitMode` viết sao cho mở rộng được, nhưng chỉ
  test trên macOS.

**Phản mục tiêu** (chạy được nhưng vẫn là thất bại)

- Một cửa sổ đọc nhầm settings workspace của cửa sổ khác, kể cả ở việc nền
  (auto-save, format, language server, terminal).
- Việc nặng của một cửa sổ (quét repo, git, language server) làm rớt khung
  hình cửa sổ khác.
- `luc <path>` lúc app đang khởi động mở thừa một cửa sổ (lỗi Zed #61346).
- Bench `bench/compare` lỡ gửi đường dẫn vào Lucerna đang chạy của người
  dùng thay vì chạy tiến trình riêng.
- Thoát app mất thay đổi chưa lưu mà không hỏi.

## 3. Ràng buộc

- Hành vi, settings, lệnh, phím theo VS Code; giao diện và hiệu năng theo
  Zed (`docs/design-principles.md`). Crate Zed là GPL: chỉ học thiết kế.
- Chỉ `lucerna-workbench` dùng GPUI. Không giả định macOS ngoài code nền
  tảng.
- Luồng UI không chờ I/O. Đọc file settings, `windows.json`, socket đều ở
  luồng nền.
- GPUI ở `b5cd310`: `on_app_quit` không huỷ được việc thoát; test platform
  `unimplemented!()` cho `on_open_urls` và `on_reopen`; `window_stack()` trả
  `None` trong test; `active_window()` có thể `None` khi app ở nền.
- Dùng thư mục `target` chung; không đặt `CARGO_TARGET_DIR`.

## 4. Các cách đã cân nhắc

### 4.1 Theme, icon theme, keymap

| Cách | Đánh đổi | Chọn |
|---|---|---|
| **A. Chung cho app** (Zed): chỉ đọc từ settings user | Giữ nguyên ~140 chỗ đọc màu; mọi cửa sổ trông như nhau. Lệch VS Code: `workbench.colorTheme` trong `.lucerna/settings.json` bị bỏ qua | **Người dùng chọn** |
| B. Theme theo cửa sổ (VS Code) | Phân biệt project bằng màu; phải sửa ~140 chỗ đọc màu và cache theo theme | |

Keymap không có lớp workspace ở cả VS Code lẫn Lucerna, nên chung trong cả
hai cách.

### 4.2 Settings workspace theo cửa sổ

| Cách | Đánh đổi | Chọn |
|---|---|---|
| **1. Entity settings riêng mỗi cửa sổ** (VS Code: `ConfigurationService` mỗi cửa sổ, truyền qua DI) | Rõ cái gì thuộc cửa sổ nào ngay trong kiểu; test hai cửa sổ dễ. Phải truyền handle vào ~50 file | **Người dùng chọn** |
| Kiểu Zed: một store toàn cục, tra theo `SettingsLocation { worktree_id, path }` | Hợp khi có settings cho thư mục con. Lucerna chỉ có một file ở gốc, nên khóa chỉ là thư mục gốc: vẫn phải truyền khóa vào từng entity, thêm một lần tra bảng | |
| 2. Bảng `WindowId` → settings | Ít truyền hơn; entity tạo trước khi có window hoặc trong test dễ đọc nhầm | |
| 3. Một global, đổi lớp workspace khi cửa sổ active đổi | Rẻ nhất nhưng sai cho cửa sổ nền | Loại |

### 4.3 `luc <path>` tới app đang chạy

| Cách | Đánh đổi | Chọn |
|---|---|---|
| **Unix socket trong thư mục data** (VS Code) | Chạy cả với `cargo run` và Linux; bench có `HOME` riêng nên có socket riêng | **Chọn** |
| LaunchServices `open -a` hoặc URL `zed-cli://` (Zed) | Chỉ chạy với bản đóng gói; không chở được cờ hay trả lỗi về terminal | |

## 5. Quyết định

| # | Quyết định | Lý do |
|---|---|---|
| M1 | Mọi cửa sổ chạy trong một tiến trình. `process.rs` (`SpawnRequest`, `Spawner`, `spawn`) và `open_requests::spawn_process` bị xoá. | Mục tiêu chính. |
| M2 | `AppState.settings` chỉ còn lớp default và user. Lớp workspace thuộc `WindowSettings` (§6.1) của từng cửa sổ. | Người dùng chọn cách 1. |
| M3 | Theme, icon theme, keymap, `files.associations` cho việc dựng `LanguageRegistry` chung, và các khóa phạm vi app (§6.1.3) chỉ đọc từ settings user. Khóa phạm vi app trong `.lucerna/settings.json` bị bỏ qua. | Người dùng chọn A. VS Code cũng đọc khóa `application` chỉ từ user. |
| M4 | Mở một thư mục (hoặc file mở riêng) đang có cửa sổ thì focus cửa sổ đó, không mở bản thứ hai. | VS Code `findWindowOnWorkspaceOrFolder`; tránh hai cửa sổ ghi đè workspace state của nhau. |
| M5 | Mở một file: vào cửa sổ có thư mục gốc là cha sâu nhất của file; không có thì vào **cửa sổ workbench dùng gần nhất** thành tab; không có workbench nào thì cửa sổ Welcome đang mở (nếu có) đổi thành cửa sổ file riêng; còn lại thì mở cửa sổ file riêng mới. | VS Code `openFilesInNewWindow: "off"` (mặc định) và `findWindowOnFile`. Đổi so với nay: file ngoài thư mục đang mở không còn mở cửa sổ riêng khi đã có cửa sổ. |
| M6 | Open Folder… và chọn mục trong Open Recent (Enter) **thay** cửa sổ hiện tại (W8 giữ nguyên). ⌘Enter trong Open Recent và trên Welcome, cùng lệnh mới `workbench.action.newWindow` (`cmd+shift+n`), mở **cửa sổ mới trong cùng tiến trình**. | VS Code `openFoldersInNewWindow: "default"`. |
| M7 | `workbench.action.newWindow` mở cửa sổ Welcome mới (không mở Get Started). | VS Code mở cửa sổ trống, hiện Welcome theo `workbench.startupEditor`. |
| M8 | Đóng cửa sổ cuối: macOS app vẫn chạy với menu bar; Linux/Windows thoát. Dùng `QuitMode::Default` của GPUI thay cho observer tự viết trong `start()`. | VS Code `window-all-closed`; Zed `on_last_window_closed: platform_default`. |
| M9 | Bấm icon Dock khi không còn cửa sổ (`on_reopen`): mở Welcome. | VS Code mở cửa sổ trống, không mở lại phiên. |
| M10 | Các lệnh cần chạy được khi không có cửa sổ: Quit, New Window, Open…, Open Folder…, Open File…, Open Recent (mục trong menu), Preferences (settings, keybindings), About. Đăng ký `cx.on_action` cấp app; có cửa sổ thì vẫn chạy trong cửa sổ đó như nay. | GPUI gửi action của menu tới cửa sổ focus, không có thì tới handler cấp app. |
| M11 | ⌘Q: hỏi lưu lần lượt từng cửa sổ, cửa sổ active trước; Cancel ở cửa sổ nào cũng huỷ việc thoát. Ghi danh sách cửa sổ (§6.6) **trước** `cx.quit()`. | Như `workbench::quit` hiện tại; Zed cho cửa sổ active hỏi trước. |
| M12 | `window.restoreWindows` mặc định đổi từ `"one"` sang `"all"`. Nhận `"preserve"`, `"all"`, `"folders"`, `"one"`, `"none"` (§6.6). | Mặc định của VS Code. |
| M13 | `luc` **giành socket trước** (`<data dir>/luc[-dev].sock`), trước cả khi khởi động GPUI. Bind được thì nó là app. `EADDRINUSE` thì nối vào làm client: gửi yêu cầu, đợi trả lời rồi thoát với mã 0 (lỗi thì in ra stderr, mã 1). | VS Code `claimInstance`. Giành trước để hai `luc` chạy gần như cùng lúc không thành hai app. Chỉ hậu tố `-dev` để bản dev không đẩy đường dẫn vào bản release người dùng đang chạy; tên không có version nên app khác version cùng giao thức vẫn nhận request, và cập nhật Lucerna khi app cũ còn chạy không ra hai app. Giao thức mới hơn app thì `luc` in "Lucerna was updated. Quit Lucerna and open it again." và thoát 1. |
| M18 | `luc` trong terminal **tách app khỏi terminal** rồi trả dấu nhắc ngay, như `code .`. Cờ ẩn `--foreground` giữ kiểu cũ (chạy app ngay trong terminal) cho `bench/compare`, `--trace-startup` và debug. | Người dùng chọn. Một tiến trình giữ mọi cửa sổ, nên tắt tab terminal hay Ctrl+C không được kéo theo mọi cửa sổ và bỏ qua bước hỏi lưu. |
| M14 | Yêu cầu từ socket hoặc `on_open_urls` đến khi app chưa mở lại xong cửa sổ thì xếp hàng, xử lý sau khi restore xong. | Lỗi Zed #61346. |
| M15 | Language server, terminal, git, workspace state: theo từng cửa sổ như nay. Trust: kho `trusted.json` chung. Tin một thư mục thì mọi cửa sổ có gốc là đúng thư mục đó chuyển sang tin. Chọn thêm "tin cả thư mục cha" thì mọi cửa sổ có gốc nằm trong thư mục cha đó cũng chuyển sang tin. Cửa sổ có gốc là thư mục con của thư mục được tin một mình thì vẫn ở Restricted Mode. | Cả hai bên làm vậy. |
| M16 | `status_message` hiện ở cửa sổ gây ra nó. | Zed và VS Code đều có thông báo theo cửa sổ. |
| M17 | Một file mở ở hai cửa sổ là hai buffer riêng. Cửa sổ này lưu thì cửa sổ kia nhận qua watcher file như với mọi thay đổi ngoài. | Cả hai bên làm vậy; watcher và xử lý xung đột đã có. |

## 6. Thiết kế

### 6.1 Settings theo cửa sổ

#### 6.1.1 Kiểu

- `lucerna-settings`: tách lớp workspace ra kiểu riêng `WorkspaceLayer`
  (map đã parse). `SettingsStore` giữ default và user; thêm
  `view(workspace: Option<&WorkspaceLayer>)` trả `SettingsView`, kiểu mượn
  store và lớp workspace, có `get(key, language)` và `get_trusted` như
  store. *(Bản đầu của spec gọi là `get_in`; code dùng `SettingsView`.)*
  `SettingsStore::get(key, language)` giữ nguyên nghĩa "không có
  workspace" để code phạm vi app và test cũ không đổi. Thứ tự ưu tiên và
  gộp object (R26) không đổi.
- `lucerna-workbench`: `WindowSettings` là một GPUI entity mỗi `Workbench`
  sở hữu. Nó giữ:
  - đường dẫn `<root>/.lucerna/settings.json` (single-file thì không có);
  - `WorkspaceLayer` hiện tại;
  - lỗi đọc file workspace;
  - watcher thư mục `<root>/.lucerna` (không chạy trên scheduler test, như
    `watch_config` hiện nay);
  - bộ đếm thay đổi.

  API: `get::<T>(key, language, cx)` gộp `AppState.settings` với lớp của
  nó; `get_trusted` bỏ lớp workspace như nay. Nó `cx.notify()` khi lớp
  workspace đổi **và** khi lớp user đổi (nó observe global `AppState`), nên
  người dùng chỉ cần observe một thứ.
- Welcome và Get Started không có `WindowSettings`: chúng chỉ đọc
  `AppState.settings` (user).

#### 6.1.2 Truyền xuống

`Workbench::new` tạo `WindowSettings` trước mọi entity con và truyền handle
vào entity nào cần settings: `Documents`, `LanguageServers`, `TerminalPanel`
và tab terminal, `CodeView`, vim, explorer, editor groups, search, markdown
preview, git store, symbols, find widget, hover, save, panel layout. Mọi chỗ
`cx.global::<AppState>().settings.get(..)` đọc khóa phạm vi cửa sổ đổi sang
handle. 8 chỗ `observe_global::<AppState>` để nghe settings đổi sang
`cx.observe(&settings, ..)`; chỗ nào nghe theme thì vẫn observe `AppState`.

Không giữ lối tắt "đọc global cho nhanh": test kiến trúc (§8.3) bắt các chỗ
đọc khóa phạm vi cửa sổ qua `AppState`.

#### 6.1.3 Phạm vi app (chỉ đọc từ user)

Danh sách cố định trong code (`app_scope_keys`), giống `scope: application`
của VS Code:

- `workbench.colorTheme`, `workbench.preferredDarkColorTheme`,
  `workbench.preferredLightColorTheme`, `window.autoDetectColorScheme`,
  `workbench.iconTheme`;
- `window.*` (gồm `window.restoreWindows`), `workbench.startupEditor`;
- `extensions.*`;
- `files.associations` khi dựng `LanguageRegistry` chung (xem dưới).

Khóa phạm vi app có trong `.lucerna/settings.json` bị bỏ qua và ghi
`tracing::info!` một lần mỗi lần nạp file. *(Cách báo trong UI, ví dụ mờ
khóa trong editor như VS Code, để sau.)*

`files.associations` trong workspace vẫn có tác dụng cho cửa sổ đó:
`WindowSettings` giữ `Arc<LanguageRegistry>` của cửa sổ. Không có
`files.associations` trong workspace thì dùng chung `AppState.languages`;
có thì cửa sổ dựng registry riêng với associations gộp. Lần dựng đầu, khi
tạo `WindowSettings`, chạy ngay trên luồng chính để tài liệu khôi phục lúc
mở cửa sổ có đúng ngôn ngữ từ đầu: việc này chỉ biên dịch glob, không đọc
file, không nạp grammar, nên rẻ. Các lần dựng lại sau đó (file workspace
hay registry của app đổi) chạy ở luồng nền. `Documents` tra ngôn ngữ qua
registry của cửa sổ.

#### 6.1.4 File cấu hình, watcher, lỗi

- `ConfigState` (global) chỉ còn thư mục user: `settings.json`,
  `keybindings.json`, `themes/`. Một watcher cho cả app.
- `watch_config(ConfigPaths)` mất trường `workspace_settings`; gọi một lần
  lúc khởi động. Router không còn gọi lại `watch_config` khi đổi thư mục.
- Mỗi `WindowSettings` có watcher riêng cho `<root>/.lucerna`, debounce như
  hiện nay (`CONFIG_DEBOUNCE`).
- Status item báo lỗi cấu hình của cửa sổ hiện lỗi user (chung) và lỗi
  workspace (riêng cửa sổ).
- `reload_config(changed)` (test và watcher user) giữ nguyên;
  `WindowSettings::reload(cx)` cho watcher workspace. Mở
  `.lucerna/settings.json` trong editor rồi lưu thì chỉ cửa sổ đó đổi.

### 6.2 Router nhiều cửa sổ

`Router` (trong `app.rs`) thay `shown: Option<Shown>` bằng danh sách cửa sổ:

```text
WindowEntry { handle: AnyWindowHandle, shown: Shown, last_active: u64 }
Shown = Welcome | Onboarding | Workbench(OpenWindow)
```

- `last_active` lấy từ một bộ đếm tăng mỗi lần cửa sổ được kích hoạt
  (`observe_window_activation`). "Cửa sổ dùng gần nhất" là `last_active`
  lớn nhất, không phụ thuộc `active_window()` hay `window_stack()`.
- Đóng cửa sổ (`on_window_closed`) thì bỏ khỏi danh sách và ghi lại
  `windows.json` (§6.6).
- Đổi root trong cửa sổ (W8, W10, Welcome → workbench) cập nhật `shown` của
  đúng mục đó.
- Router là global (`Rc<RefCell<..>>` trong một `Global`), để action cấp
  app, socket, `on_open_urls`, `on_reopen` và test cùng gọi.

### 6.3 Một hàm mở duy nhất

```text
open(request: OpenRequest, cx) -> Option<AnyWindowHandle>
OpenRequest { paths: Vec<PathBuf>, target: Target, source: Source }
Target = Auto | NewWindow | ReplaceIn(AnyWindowHandle)
Source = Cli | Finder | Menu | Restore
```

Mọi nguồn gọi hàm này: `luc` (socket), Finder/"Open With"/Dock
(`on_open_urls`), Welcome, Open Recent, Open dialog, New Window, restore.
Hàm thuần trên `&mut App`, nên test gọi trực tiếp được (test platform không
có `on_open_urls`).

Quy tắc với `Target::Auto` cho mỗi đường dẫn (đã canonical):

1. Thư mục đang là gốc của một cửa sổ thư mục, hoặc file đang là file
   riêng của một cửa sổ file riêng → focus cửa sổ đó (M4).
2. File nằm trong gốc của một hay nhiều **cửa sổ thư mục** → cửa sổ có gốc
   sâu nhất, mở file thành tab ghim, focus cửa sổ (M5). Cửa sổ file riêng
   không có gốc ở đây: thư mục cha của nó không được quét nên không tính.
3. File không nằm trong gốc nào: có cửa sổ thư mục → cửa sổ thư mục dùng
   gần nhất, mở thành tab ghim. Không có cửa sổ thư mục mà có Welcome/Get
   Started → đổi root cửa sổ Welcome dùng gần nhất thành cửa sổ file riêng.
   Còn lại → cửa sổ file riêng mới. Cửa sổ file riêng không bao giờ nhận
   thêm file từ `open`, nên nó luôn đúng là "một file" (`windows.json`
   `kind: "file"`, không có settings workspace).
4. Thư mục chưa mở: có Welcome/Get Started → đổi root cửa sổ Welcome dùng
   gần nhất (như Zed: cửa sổ trống được tái dùng). Không có → cửa sổ mới.
5. Đường dẫn không tồn tại → bỏ qua, ghi log; với `Source::Cli` trả lỗi về
   terminal.

`Target::NewWindow` luôn mở cửa sổ mới, trừ quy tắc 1 (đã mở thì focus).
`Target::ReplaceIn(w)` là W8/W10: đổi root của `w` sau khi workbench cũ đã
hỏi lưu.

Sau mỗi lần mở: ghi recent (như `record_recent`), `app_menu::refresh`,
ghi `windows.json`.

Cửa sổ mới mở bằng `maximized_window_options` như nay. Việc đặt cửa sổ mới
lệch khỏi cửa sổ cũ (cascade) để sau.

### 6.4 Các lệnh

| Lệnh | Phím | Hành vi mới |
|---|---|---|
| `workbench.action.newWindow` (mới) | `cmd+shift+n` | Cửa sổ Welcome mới (M7). Có trong menu File, đầu nhóm Open. |
| Open Recent: Enter | | Thư mục: thay cửa sổ này (W8, nhưng nếu thư mục đã mở ở cửa sổ khác thì focus cửa sổ đó, không thay). File: tab ở đây. |
| Open Recent: ⌘Enter | | `Target::NewWindow`. |
| Welcome: bấm mục | | Thay cửa sổ Welcome này. Đã mở ở cửa sổ khác → focus cửa sổ đó, Welcome vẫn còn. |
| Welcome: ⌘bấm | | `Target::NewWindow`. |
| Open Folder… / Open… | | Thay cửa sổ này (W8), trừ thư mục đã mở ở cửa sổ khác → focus. Không có cửa sổ → cửa sổ mới. |
| Close Folder (`cmd+k f`) | | Như W10. |
| Get Started: "Open settings in new window" | | `Target::NewWindow` trên `settings.json` của user. |
| Quit (`cmd+q`) | | M11. |
| Đóng cửa sổ (`cmd+w` khi không còn tab, nút đỏ) | | Hỏi lưu cửa sổ đó như nay; cửa sổ cuối: M8. |

`WorkbenchEvent::Replace(SpawnRequest)` đổi thành
`WorkbenchEvent::Replace(Replacement)` với `Replacement = Path(PathBuf) |
Welcome`, và thêm `WorkbenchEvent::OpenInNewWindow(PathBuf)`.

### 6.5 `luc` từ terminal (socket)

- **Đường dẫn socket:** `<data dir>/luc.sock`, `luc-dev.sock` khi
  `debug_assertions`; tên không có version, mọi version của cùng kênh build
  dùng chung một socket. `sun_path` macOS giới hạn 104 byte: dài
  hơn thì dùng `$TMPDIR/lucerna-<hash của data dir>.sock`.
- **Giành socket** (`crates/lucerna/src/main.rs`, ngay sau khi parse đối
  số, trước `app::run`; `luc ext …` và `luc lsp …` không đụng tới socket):
  - `bind` được: tiến trình này là app. Listener được chuyển vào `app::run`;
    app nhận kết nối ở luồng nền ngay từ đầu, yêu cầu nào tới trong lúc
    khởi động hay restore thì xếp hàng (M14).
  - `EADDRINUSE`: thử `connect` (timeout 1 s). Nối được thì làm client: gửi
    một dòng JSON, đợi một dòng trả lời (timeout 5 s), in lỗi nếu có, thoát.
    `ECONNREFUSED` (file socket còn lại sau crash) thì xoá file và `bind`
    lại một lần.
  - Kết nối bị đóng hoặc lỗi trước khi có trả lời (EOF, reset): giành
    socket lại một lần. Thường là do `luc` khác vừa giữ socket một lúc rồi
    nhả cho app nó khởi động (M1-1). Lần giành thứ hai nối tới app đó, hoặc
    tự bind khi chưa có app. Bước 3 bên dưới cũng nối lại một lần như vậy.
  - App đang thoát trả `{"ok":false,"error":"Lucerna is quitting","status":"quitting"}`
    (nó vẫn giữ socket thêm một lúc). `luc` giành socket lại mỗi 100 ms,
    tối đa 5 s, rồi gửi cho app mới hoặc tự khởi động app; quá hạn thì in
    lỗi, thoát mã 1 (M1-2).
  - Hết thời gian đợi trả lời mà yêu cầu đã gửi đi (app đang bận, ví dụ
    đang khôi phục cửa sổ): in "Lucerna is busy; the path will open when it
    is ready" và thoát mã 0, vì app vẫn mở đường dẫn khi rảnh. Nếu việc
    mở sau đó thất bại, app báo "Could not open …" trong status bar của
    cửa sổ dùng gần nhất, vì `luc` đã thoát (app biết nhờ kết nối socket
    đã đóng); `luc` còn đợi thì lỗi chỉ in ra terminal (M1-3).
  - `luc` không đối số chạy trong `/` khi app đang chạy: gửi yêu cầu
    `activate` (trường `"activate":true`, `paths` rỗng). App không mở `/`
    mà ra trước như khi bấm icon Dock (M9).
  - Thư mục chứa socket được tạo với quyền 0700, socket 0600; app chỉ nhận
    yêu cầu từ cùng user (kiểm tra uid của đầu kia). Đường dẫn dự phòng khi
    quá dài là thư mục riêng của user (`$TMPDIR` trên macOS,
    `$XDG_RUNTIME_DIR` hoặc `lucerna-<uid>` nơi khác), không dùng `/tmp`
    chung.
  - Lỗi khác (không tạo được thư mục data, quyền): in cảnh báo, khởi động
    app tách khỏi terminal như M18 nhưng không có socket (không gửi yêu cầu),
    rồi thoát mã 0; không chặn người dùng. `--foreground`/`--trace-startup`
    thì chạy app tại chỗ, không có socket (M1-4).
- **Tách khỏi terminal** (M18). Khi `luc` (không có `--foreground`) phải
  thành app, nó không chạy app trong tiến trình này mà khởi động một tiến
  trình app tách rời rồi gửi yêu cầu cho nó như client:
  1. Nhả socket vừa giành (để tiến trình app giành).
  2. Khởi động app: chạy chính binary với cờ ẩn `--detached-child`
     (thay cho `--foreground`; tiến trình con gặp app khác đã giữ socket thì
     chuyển yêu cầu cho nó, M1-1, còn không thì chạy app tại chỗ), `setsid`
     (phiên mới, không còn terminal điều khiển), stdin/stdout/stderr về
     `/dev/null` (log vẫn ghi file như nay), `cwd` giữ nguyên. Không dùng
     `open -a`: nó chỉ có với bản đóng gói và dễ mở nhầm bản cài khác cùng
     bundle ID; binary trong `Contents/MacOS` chạy trực tiếp vẫn hiện đúng
     một icon của bundle.
  3. Gửi yêu cầu qua socket, thử lại nối trong tối đa 10 s (app đang khởi
     động), rồi đợi trả lời thêm tối đa 10 s (app giành socket trước cửa sổ
     đầu, nên trả lời đợi cả lúc khởi động); nhận trả lời thì thoát mã 0.
     Hết 10 s mà không nối được thì in lỗi, thoát mã 1 (app có thể vẫn đang
     mở). Nối được mà hết thời gian trả lời thì in thông báo "busy" như trên
     và thoát mã 0.
  Đường dẫn được resolve ở client như nay, nên lỗi "path not found" vẫn
  in ra terminal trước khi khởi động gì cả. Mở từ Finder/Dock (`cwd` là `/`,
  không có đối số) thì đã không có terminal: chạy app tại chỗ như nay.
- **`--foreground`** (ẩn khỏi `--help`): giành socket và chạy app trong
  tiến trình này như hành vi hiện nay. `bench/compare` và
  `scripts/` khởi chạy `luc` để đo thì thêm cờ này (pidfile vẫn đúng
  tiến trình app). `--trace-startup` cũng chạy app tại chỗ như
  `--foreground`. Cả hai chỉ giành socket, không bao giờ chuyển request:
  socket đã bị giữ thì thoát mã 1 và in "Lucerna is already running; quit it
  to measure startup." (`--trace-startup`) hoặc "Lucerna is already running;
  quit it to run in the foreground." (`--foreground`) (M1-6).
- **Phía app:** mỗi kết nối: đọc một dòng ở luồng nền, gửi vào kênh của
  router trên luồng chính, trả lời sau khi `open` xong. Thoát app thì xoá
  file socket.
- **Môi trường:** yêu cầu không chở biến môi trường của shell gọi `luc`.
  Language server và terminal vẫn lấy môi trường từ login shell như nay
  (`lucerna_lsp::env::login_shell_env`). Chở env theo cửa sổ (như Zed, VS
  Code) để sau 0.1; giao thức có `version` nên thêm trường được.
- **Giao thức** (một dòng JSON mỗi chiều, có `version`):

  ```json
  {"version":1,"cwd":"/abs/dir","paths":["/abs/a"],"new_window":false}
  {"version":1,"cwd":"/","paths":[],"new_window":false,"activate":true}
  {"ok":true}
  {"ok":false,"error":"path not found: /abs/a"}
  {"ok":false,"error":"Lucerna is quitting","status":"quitting"}
  ```

  `paths` rỗng (`luc` không đối số) nghĩa là thư mục `cwd`, như nay.
  `new_window` để dành cho `-n` sau 0.1; 0.1 luôn gửi `false`. `activate`
  mặc định `false` (thêm sau, không tăng `version`). `status` trong trả lời
  cũng thêm sau, không tăng `version`; `luc` cũ bỏ qua nó và in `error`. Trường lạ
  bị bỏ qua; `version` lớn hơn thì trả lỗi "Lucerna was updated. Quit Lucerna and open it again."
- **Bench:** `bench/compare` đặt `HOME` riêng nên data dir và socket riêng,
  vẫn chạy tiến trình riêng như nay. Thêm một kiểm tra trong `editors.py` để
  chắc điều đó (§8.4).
- Bản đóng gói mở từ Finder/Dock: macOS chỉ cho một instance mỗi bundle, nên
  đường này vẫn đi qua `on_open_urls`; app vẫn bind socket để `luc` trong
  terminal tìm thấy nó.

### 6.6 Thoát, đóng cửa sổ cuối, mở lại cửa sổ

**Danh sách cửa sổ** `<data dir>/windows.json`:

```json
{ "version": 1,
  "windows": [ { "kind": "folder", "path": "/abs/proj" },
               { "kind": "file", "path": "/abs/x.rs" } ] }
```

- Thứ tự: dùng gần nhất trước. Chỉ ghi cửa sổ workbench; Welcome và Get
  Started không ghi.
- Ghi (luồng nền, ghi nguyên tử như `recent.json`) mỗi khi mở, đóng, đổi
  root hay đổi thứ tự dùng gần nhất của cửa sổ; và một lần đồng bộ trong
  Quit trước `cx.quit()`. Ghi liên tục để sau crash vẫn mở lại được.
- Đóng cửa sổ (không phải thoát) thì bỏ nó khỏi danh sách, như Zed và VS
  Code. Đóng cửa sổ cuối rồi thoát thì danh sách rỗng.

**Thoát** (M11): `workbench::quit` giữ thuật toán, thêm: sắp cửa sổ theo
`last_active` giảm dần; sau khi mọi cửa sổ đồng ý thì ghi `windows.json`
đồng bộ, rồi `cx.quit()`. Thoát do hệ thống (logout) chỉ chạy
`on_app_quit`: lúc đó không hỏi được, chỉ lưu layout như nay; danh sách cửa
sổ đã đúng vì được ghi liên tục.

**Đóng cửa sổ cuối** (M8, M9): `QuitMode::Default`. Bỏ observer
`on_window_closed → quit` trong `start()`. Đăng ký `on_reopen` mở Welcome.
Socket, watcher user, extension service vẫn chạy.

**Khởi động không có đường dẫn** (`first_screen` mở rộng):

| `window.restoreWindows` | Mở |
|---|---|
| `"all"` (mặc định), `"preserve"` | mọi cửa sổ trong `windows.json` còn tồn tại |
| `"folders"` | chỉ cửa sổ thư mục |
| `"one"` | cửa sổ đầu danh sách |
| `"none"` | không mở lại |

- Danh sách rỗng hoặc mọi mục đã mất: với mọi giá trị trừ `"none"`, mở mục
  mới nhất còn tồn tại trong `recent.json` (hành vi hiện nay). Không có gì
  thì Get Started (lần đầu), Welcome, hoặc hộp chọn thư mục như nay.
- Mở theo thứ tự ngược (cũ trước) để cửa sổ dùng gần nhất nằm trên cùng,
  như Zed.
- Không có `windows.json` (lần đầu chạy bản mới): như danh sách rỗng.

**Khởi động có đường dẫn** (`luc <path>` khi chưa có app): `"preserve"` mở
lại mọi cửa sổ rồi mở thêm đường dẫn; các giá trị khác chỉ mở đường dẫn,
như VS Code.

**Hàng đợi** (M14): router có cờ `restoring`. Trong lúc đó yêu cầu từ
socket và `on_open_urls` xếp hàng; restore xong thì xử lý lần lượt qua
`open` (quy tắc 1 tránh mở trùng). Thời gian chờ ban đầu
`OPEN_REQUEST_GRACE` cho "Open With" giữ nguyên.

### 6.7 Trust, terminal, language server, workspace state

- **Trust:** `TrustStore` vẫn là file chung, mỗi cửa sổ đánh giá theo gốc
  của nó (đã như vậy). Thêm sự kiện app-level "trust đổi": cửa sổ nào ghi
  `trusted.json` thì phát; mọi workbench đọc lại trust của gốc mình (luồng
  nền) và bật language server nếu giờ đã tin.
- **Language server:** mỗi `Workbench` có `LanguageServers` riêng như nay;
  hai cửa sổ cùng ngôn ngữ chạy hai server. `QuitShutdown` đã gom mọi
  cửa sổ. Đọc `lsp` và `lucerna.lsp.enabled` qua `WindowSettings` của cửa
  sổ (`get_trusted` cho phần chạy được code).
- **Terminal:** `TerminalPanel` theo cửa sổ như nay; settings
  `terminal.integrated.*` đọc qua `WindowSettings`. `DefaultTerminalSpawner`
  và `SystemShellGlobal` không giữ trạng thái cửa sổ, giữ global.
- **Workspace state:** khóa theo gốc như nay. M4 bảo đảm mỗi gốc tối đa một
  cửa sổ, nên không ghi đè nhau.
- **`status_message`:** `StatusMessage` giữ map `WindowId → (text,
  generation)`. `show(window_id, text, cx)`; caller trong workbench truyền
  cửa sổ của nó; router truyền cửa sổ đích.
- **Recent, `RecentStore`, `ExtensionService`, theme, icon, keymap:** global
  như nay.
- **Menu bar:** `app_menu::refresh` đã dựa trên cửa sổ active; khi không có
  cửa sổ, các mục cần workbench bị tắt, các mục M10 vẫn bật.

### 6.8 Crash

GPUI không cô lập panic theo cửa sổ: panic trên luồng chính trong callback
AppKit không unwind được qua FFI và làm sập cả app. Đây là đánh đổi chính
của mô hình một tiến trình. Giảm rủi ro:

- `windows.json` và layout từng cửa sổ được ghi liên tục, nên lần khởi động
  sau mở lại đủ cửa sổ.
- Việc nặng (quét file, git, language server, tìm kiếm) chạy ở luồng nền;
  panic ở đó được bắt (`tasks::spawn_guarded` đã có) và chỉ ghi log.
- `luc` không nối được socket thì tự chạy app, nên app cũ sập không chặn
  người dùng.
- Panic hook hiện có ghi log; giữ nguyên.

## 7. Hiệu năng

### 7.1 Đo

Trên bản release, dùng `bench/compare` (`footprint` cho RAM), trước khi
chuyển (commit gốc của nhánh) và sau:

1. RAM tổng (footprint của tiến trình và con) khi mở 1, 2, 3 cửa sổ trên
   cùng một bộ repo. Bản cũ: cộng các tiến trình.
2. Thời gian mở cửa sổ thứ hai: từ lúc gửi yêu cầu (`luc <path>` thứ hai)
   tới khung hình đầu của cửa sổ mới.
3. Khung hình rớt ở cửa sổ A (đang cuộn) khi cửa sổ B quét một repo lớn
   (mở thư mục mới) và khi language server của B đang index.

Script đo nằm trong `bench/compare` (thêm kịch bản nhiều cửa sổ); kết quả
ghi vào `docs/benchmarks.md`.

### 7.2 Ngưỡng chấp nhận

Mốc so sánh là số đo ở bước 1 (§9) trên commit gốc của nhánh, cùng máy,
cùng bộ repo. Số khởi động đã công bố trước đây đã bị rút lại (commit
`58540ca`), nên không dùng làm mốc.

- Một cửa sổ: thời gian khởi động và RAM không tệ hơn mốc (trong sai số đo
  của `bench/compare`).
- Cửa sổ thứ hai mở nhanh hơn bản cũ (không phải khởi động tiến trình).
- RAM 2–3 cửa sổ: không cao hơn tổng của bản cũ quá 10 %, và 3 cửa sổ phải
  thấp hơn tổng của bản cũ. *(Sửa 2026-11-16 theo quyết định của người dùng;
  bản đầu là "thấp hơn tổng của bản cũ". Bộ đệm vẽ `IOSurface` của mỗi cửa
  sổ chiếm phần lớn RAM và dao động khoảng 20 MB khi app đứng yên, nên năm
  vòng đo không tách được mức tiết kiệm vài MB của việc gộp tiến trình.)*
- Cửa sổ A không rớt khung hình nào nhiều hơn khi B đứng yên, trong sai số.
- Mọi việc nặng của một cửa sổ chạy ở luồng nền.

Không đạt ngưỡng thì không coi là xong.

## 8. Test

### 8.1 Đơn vị

- `lucerna-settings`: `get_in` với và không có lớp workspace; khóa
  `[lang]`; gộp object; `get_trusted` bỏ workspace.
- Quy tắc chọn cửa sổ của `open` (hàm thuần trên danh sách cửa sổ giả):
  focus gốc trùng, gốc sâu nhất, cửa sổ dùng gần nhất, tái dùng Welcome,
  cửa sổ mới.
- `windows.json`: đọc, ghi, file hỏng, phiên bản lạ; lọc theo từng giá trị
  `window.restoreWindows`; fallback `recent.json`.
- Giao thức socket: parse, phiên bản, đường dẫn dài quá 104 byte.

### 8.2 GPUI nhiều cửa sổ (`tests/multi_window.rs`)

Dùng `TestAppContext`, `cx.windows()`, `simulate_close`,
`simulate_prompt_answer`:

1. Mở hai thư mục → hai cửa sổ workbench trong một app; mở lại thư mục thứ
   nhất → vẫn hai cửa sổ, cửa sổ một được kích hoạt.
2. Hai thư mục có `.lucerna/settings.json` khác nhau (`editor.tabSize`) →
   mỗi cửa sổ đọc giá trị của nó; `Documents` và `CodeView` của mỗi cửa sổ
   dùng đúng giá trị. Đổi file của cửa sổ A (`WindowSettings::reload`) → B
   không đổi.
3. Đổi settings user (`reload_config`) → cả hai cửa sổ thấy.
4. Khóa phạm vi app trong workspace (`workbench.colorTheme`) bị bỏ qua.
5. Mở file nằm trong thư mục cửa sổ B khi A active → tab mở ở B.
6. New Window → thêm một cửa sổ Welcome; chọn thư mục trên Welcome đó →
   cửa sổ đó đổi root, số cửa sổ không đổi.
7. Quit với hai cửa sổ có file chưa lưu: trả lời Cancel ở cửa sổ thứ hai →
   không thoát, cả hai còn; Don't Save cả hai → thoát và `windows.json` có
   hai cửa sổ theo thứ tự dùng gần nhất.
8. Đóng một trong hai cửa sổ → `windows.json` còn một; đóng cửa sổ cuối →
   app không thoát (đã đặt `QuitMode` Explicit trong test).
9. Restore: `windows.json` có ba mục, một đã bị xoá → mở hai cửa sổ, đúng
   thứ tự; `"folders"` bỏ mục file; `"one"` mở một.
10. Yêu cầu mở tới trong lúc restore → xếp hàng, không mở trùng.
11. Trust ở cửa sổ A (gốc `/t`): chọn "tin cả thư mục cha" → cửa sổ B
    gốc `/t/sub` chuyển sang tin. Tin `/t` một mình → B vẫn ở Restricted
    Mode, còn cửa sổ khác có gốc đúng `/t` thì chuyển sang tin.
12. `status_message` của A không hiện ở B.
13. Tách settings không rò: mọi workbench cũ được giải phóng khi đóng cửa
    sổ (như test W8).

### 8.3 Kiến trúc

Một test quét mã nguồn `lucerna-workbench/src` (như các test kiến trúc
khác nếu có; không thì thêm): không còn `process::spawn`, không đọc khóa
phạm vi cửa sổ qua `cx.global::<AppState>().settings` ngoài các file được
phép (`config_files.rs`, `app.rs`, `startup.rs`, `extensions/service.rs`,
`theme_picker.rs`).

### 8.4 Tích hợp `luc`

- Đơn vị (không cần màn hình): quyết định giành socket (bind được →
  app; `EADDRINUSE` + nối được → client; `ECONNREFUSED` → xoá rồi bind
  lại) trên socket tạm; client gửi/nhận một dòng với một server giả.
- Tích hợp (process thật, `HOME` tạm, `#[ignore]` vì cần màn hình; chạy tay
  trên macOS trước khi xong): `luc <dir>` trả dấu nhắc trong < 10 s và để
  lại đúng một tiến trình app; `luc <dir2>` lần hai thoát mã 0, không thêm
  tiến trình; `luc <không tồn tại>` thoát mã 1 và in lỗi; `luc
  --foreground <dir>` chạy app trong tiến trình gọi.

## 9. Các bước (khung cho plan)

Mỗi bước build và test được riêng:

1. **Đo trước** trên `main`: kịch bản nhiều cửa sổ trong `bench/compare`,
   ghi số.
2. **`WorkspaceLayer` và `get_in`** trong `lucerna-settings`.
3. **`WindowSettings`** và tách `ConfigState`: workspace layer ra khỏi
   global; workbench tạo entity; vẫn một cửa sổ mỗi tiến trình. Truyền
   handle theo từng module (có thể chia nhiều task: documents/code view,
   language servers, terminal, explorer/editor groups/search, còn lại).
4. **Router nhiều cửa sổ** và `open()`; bỏ `process.rs`; New Window;
   `status_message` theo cửa sổ.
5. **Thoát, đóng cửa sổ cuối, `on_reopen`, action cấp app.**
6. **`windows.json` và `window.restoreWindows: "all"`**, hàng đợi khi
   restore.
7. **Socket cho `luc`, tách khỏi terminal, `--foreground`;** cập nhật
   `bench/compare` và shim trong `scripts/bundle-app.sh` (comment).
8. **Trust đồng bộ giữa cửa sổ.**
9. **Đo sau**, so ngưỡng, cập nhật `docs/benchmarks.md`; cập nhật spec
   Welcome (W1, W8 ghi chú "thay bởi spec này"), spec restore last
   workspace, README nếu có nhắc nhiều tiến trình.

## 10. Rủi ro và giới hạn đã biết

- Một panic làm sập mọi cửa sổ (§6.8).
- Sửa ~90 chỗ đọc settings và ~40 file test: rủi ro lớn nhất là bỏ sót một
  chỗ đọc nhầm global. Test kiến trúc (§8.3) và test 2 (§8.2) để bắt.
- Thay `QuitMode` đổi hành vi: macOS không còn thoát khi đóng cửa sổ cuối.
  Bench nào dựa vào việc đóng cửa sổ để kết thúc tiến trình phải kill thay
  vì đóng cửa sổ (kiểm tra `bench/compare`).
- Bản cũ (nhiều tiến trình) và bản mới cùng chạy trong lúc nâng cấp: app
  0.1.0 dùng `luc-0.1.0.sock`, nên lần cập nhật đầu tiên lên bản không
  version vẫn có thể có hai tiến trình (một lần); từ đó về sau không còn.
- `"preserve"` chỉ khác `"all"` khi khởi động có đường dẫn.

## 11. Để lại cho người viết plan

- Tên module mới (`window_settings.rs`, `windows_file.rs`, `cli_socket.rs`
  …) và cách chia task ở bước 3.
- Cách `Documents` lấy registry ngôn ngữ của cửa sổ (§6.1.3).
- Chi tiết UI của mục "New Window" trong menu File và Dock menu
  (`set_dock_menu`, có thể thêm "New Window" như Zed).
