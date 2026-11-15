# Nghiên cứu: một tiến trình, nhiều cửa sổ

Ngày 2026-11-15. Nghiên cứu cho việc Lucerna chuyển từ "mỗi cửa sổ một tiến trình `luc`" sang "một tiến trình, nhiều cửa sổ".

Nguồn:
- Zed: `~/.cargo/git/checkouts/zed-a70e2ad075855582/b5cd310/` (rev `b5cd310b3b`, cùng rev Lucerna pin cho gpui). Các crate Zed trừ gpui là GPL: ở đây chỉ ghi thiết kế, tên định danh và vài dòng ngắn.
- GPUI (Apache-2.0): `crates/gpui`, `crates/gpui_macos` ở cùng rev.
- VS Code: `microsoft/vscode` nhánh `main`, lấy ngày 2026-11-15 (file gốc dưới `src/vs/...`, đường dẫn ghi bên dưới).

Đường dẫn bắt đầu bằng `crates/` ở dưới là tương đối so với bản checkout của Zed.

Lưu ý quan trọng về rev Zed này: Zed giờ có **`MultiWorkspace`** làm view gốc của mọi cửa sổ. Một cửa sổ có thể chứa nhiều `Workspace` (project), chuyển qua một sidebar ("threads sidebar"). Vì vậy "mở thư mục" trong Zed có thể là (a) cửa sổ mới, (b) thêm một workspace mới vào sidebar của cửa sổ hiện tại, hoặc (c) kích hoạt một workspace có sẵn. Zed cũ (và VS Code) chỉ có (a) và "thay thế cửa sổ". Lucerna không cần sidebar; khi sidebar không liên quan, đọc `MultiWorkspace` bên dưới như "cửa sổ".

---

## 1. Zed: app giữ nhiều cửa sổ thế nào

### 1.1 Trạng thái dùng chung và theo từng cửa sổ

- `AppState` (`crates/workspace/src/workspace.rs:1329`) là một `Arc` lưu như global của GPUI (`GlobalAppState`). Mọi cửa sổ dùng chung: `languages: Arc<LanguageRegistry>`, `client`, `user_store`, `workspace_store`, `fs`, `build_window_options: fn(Option<Uuid>, &mut App) -> WindowOptions`, `node_runtime`, `session: Entity<AppSession>`.
- `WorkspaceStore` (`workspace.rs:1361`) chỉ là một tập `(AnyWindowHandle, WeakEntity<Workspace>)` dùng cho cộng tác/follow; nó **không** phải nguồn sự thật cho "những cửa sổ nào đang tồn tại". Mọi chỗ cần liệt kê cửa sổ đều dùng `cx.windows()` rồi downcast sang `WindowHandle<MultiWorkspace>` (ví dụ `workspace_windows_for_location`, `workspace.rs:11094`).
- Theo từng cửa sổ: `MultiWorkspace` (`crates/workspace/src/multi_workspace.rs:305`) → một hoặc nhiều `Workspace` → mỗi `Workspace` sở hữu một entity `Project`.
- Theo từng project (thực tế là theo cửa sổ): `Project` sở hữu `BufferStore`, `LspStore`, `WorktreeStore`, `terminals`, git store, task store (`crates/project/src/project.rs:247-255`). Hai cửa sổ trên các thư mục chồng lên nhau có hai `Project` độc lập, nên có hai buffer store và hai bộ language server.

### 1.2 `workspace::open_paths` và cách chọn cửa sổ

`open_paths(abs_paths, app_state, OpenOptions, cx)` (`workspace.rs:11436`) là điểm vào duy nhất cho CLI, mở từ Finder/Dock, hộp thoại Open và khôi phục session. Các bước:

1. `find_existing_workspace` (`workspace.rs:11158`) duyệt `cx.windows()` → mọi workspace trong mọi cửa sổ, hỏi project qua `visibility_for_paths` / `visibility_for_subpaths`, giữ kết quả khớp nhất. Độ rộng của việc khớp lấy từ `OpenOptions.workspace_matching` (`WorkspaceMatching`, `workspace.rs:11254`): `None` (luôn mở mới), `MatchExact` (mặc định: đường dẫn là gốc worktree hoặc là file/thư mục nằm trong một worktree), `MatchSubpaths` (nằm trong, nhưng không phải chính gốc), `MatchSubdirectory` (`zed -a`: nằm trong bất kỳ worktree nào, nếu không thì lùi về cửa sổ bất kỳ).
2. Nếu không khớp và **mọi đường dẫn đều là file**, `preferred_target_for_files` (`workspace.rs:11118`) chọn workspace của cửa sổ đang active, không thì cửa sổ khác bất kỳ. Nên file lẻ không bao giờ sinh cửa sổ mới khi đã có cửa sổ.
3. Nếu không khớp và các đường dẫn có thư mục, cờ `add_dirs_to_sidebar` (lấy từ setting) quyết định thư mục thành workspace mới trong cửa sổ active hay mở cửa sổ mới.
4. Còn lại thì `Workspace::new_local(..., requesting_window, ..., open_mode)` (`workspace.rs:2160`) tạo `Project` mới và đặt vào `requesting_window` (`OpenMode::Activate`/`Add`) hoặc mở cửa sổ OS mới (`OpenMode::NewWindow`, `workspace.rs:1690`).

`OpenOptions` (`workspace.rs:11272`): `visible`, `focus`, `workspace_matching`, `add_dirs_to_sidebar`, `wait`, `requesting_window`, `open_mode`, `env`, `open_in_dev_container`. `should_reuse_existing_window()` trả false với `WorkspaceMatching::None | MatchSubpaths` hoặc `OpenMode::NewWindow`.

### 1.3 "Open folder" từ trong app

Action `Open` (`workspace.rs:~997`, `prompt_and_open_paths`): cờ `create_new_window` lấy từ action hoặc từ setting `default_open_behavior` (`existing_window` mặc định | `new_window`).
- Không phải cửa sổ mới → `multi_workspace.open_project(paths, OpenMode::Activate)` (thêm vào cửa sổ hiện tại).
- Cửa sổ mới → `Workspace::open_workspace_for_paths(OpenMode::NewWindow, …)` (`workspace.rs:4072`). Hàm này có một quy tắc đáng học: **nếu workspace hiện tại trống (không worktree, không remote, không item dirty) thì hạ xuống `Activate`, tức là dùng lại cửa sổ trống này thay vì để nó lại.**
- "Replace window" (hành vi Zed cũ, vẫn gọi được qua `zed -r` / `OpenBehavior::Reuse`) đóng project hiện tại bằng `CloseIntent::ReplaceWindow` và nạp project mới vào cùng cửa sổ. Test: `test_replace_window_without_worktrees_prompts`, `test_replace_window_with_worktrees_hot_exits` (`workspace.rs:13857/13895`).

### 1.4 Cửa sổ mới (cmd-shift-n)

- Keymap: `cmd-shift-n` → `workspace::NewWindow` (`assets/keymaps/default-macos.json:722`), `ctrl-shift-n` trên Linux.
- Handler (`crates/zed/src/zed.rs:1337`): `open_new(Default::default(), …)` → `Workspace::new_local(vec![], …)` với `Project` rỗng, rồi `initialize_new_window` (`zed.rs:2418`) theo `on_new_window`: `launchpad` (mặc định: trang recent-projects trong pane trống), `empty_tab` (một buffer untitled), `readme`.
- Mục "New Window" của menu Dock đặt bằng `cx.set_dock_menu` (`zed.rs:2340`).
- Kích thước/vị trí cho cửa sổ mới (`workspace.rs:~2320`): override từ env → bounds và display đã lưu của chính workspace (project mở lại) → `read_default_window_bounds` (bounds gần nhất của bất kỳ cửa sổ nào) → mặc định của GPUI (xếp chồng bậc thang).

## 2. Zed: CLI `zed` và app đang chạy

File: `crates/cli/src/cli.rs`, `crates/cli/src/main.rs`, `crates/zed/src/zed/open_listener.rs`, `crates/zed/src/zed/mac_only_instance.rs`, `crates/zed/src/main.rs`.

- CLI là một **binary nhỏ riêng**. Nó tạo `ipc_channel::IpcOneShotServer<IpcHandshake>` và dựng URL `zed-cli://<server_name>` (`cli/src/main.rs:537`).
- macOS: gọi `LSOpenFromURLSpec` với app bundle làm `appURL` và URL `zed-cli://…` làm item, cờ `kLSLaunchDefaults | kLSLaunchDontSwitch` (`cli/src/main.rs:1334`). LaunchServices hoặc khởi động Zed.app, hoặc chuyển URL cho bản đang chạy; cả hai đường đều đến `application:openURLs:` của GPUI → `App::on_open_urls` → `OpenListener` (`zed/src/main.rs:457`). (Bundle chỉ khai báo `zed://`, `crates/zed/Cargo.toml:293`; `zed-cli://` chạy được vì app đích được gọi đích danh.)
- Linux: app bind một `UnixDatagram` tại `<data_dir>/zed-<channel>.sock` (`open_listener.rs:409`), xóa socket cũ khi gặp `ConnectionRefused`; CLI gửi URL `zed-cli://` vào đó, hoặc spawn binary nếu không ai lắng nghe.
- App parse `zed-cli://name` (`open_listener.rs:158`), kết nối ngược lại (`connect_to_cli`, `open_listener.rs:433`) và trao đổi thông điệp có kiểu: `CliRequest::Open { paths, urls, diff_paths, wait, open_behavior, env, cwd, … }` và `CliResponse::{Ping, Stdout, Stderr, Exit{status}}` (`cli/src/cli.rs:55-80`). Nhờ đó CLI nhận được stdout/stderr thật và exit status từ app.
- Cờ → `cli::OpenBehavior` (`cli/src/cli.rs:18`): `-n/--new` `AlwaysNew`, `-a/--add` `Add`, `-e/--existing` `ExistingWindow`, `--classic` (ẩn; thư mục mở cửa sổ mới, file dùng lại cửa sổ: hành vi trước khi có sidebar) và `-r/--reuse` (ẩn; thay workspace của một cửa sổ). Không cờ → `Default`, quyết định từ `cli_default_open_behavior` (`existing_window` | `new_window`); lần đầu dùng Zed thậm chí có thể hỏi và ghi setting (`resolve_open_behavior`, `open_listener.rs:688`). Ánh xạ sang `OpenOptions` nằm ở `open_options_for_behavior` (`open_listener.rs:790`).
- `--wait` (`open_listener.rs:~1005`): với file, thu thập các future `item.on_release`; với thư mục hoặc không có đường dẫn, chờ entity workspace bị release (cửa sổ đóng). CLI giữ kết nối đến lúc đó rồi nhận `Exit`.
- Không có đường dẫn và không có `-n` → `restore_or_create_workspace` (giống Dock reopen).
- **Single instance:** macOS dùng TCP listener trên 127.0.0.1 với port suy ra từ release channel + uid (`mac_only_instance.rs`): nếu nhận lại chuỗi handshake thì đã có instance khác, tiến trình mới in "zed is already running" rồi thoát. Cái này chặn người dùng chạy binary hai lần; CLI thông thường đi qua LaunchServices. Kênh dev và `ZED_STATELESS` bỏ qua bước kiểm tra (`main.rs:354`).
- Race khi khởi động nguội: khi mở nguội, yêu cầu CLI có thể đến lúc session đang được khôi phục; Zed chờ khôi phục xong (hoặc có cửa sổ đầu tiên) rồi mới khớp đường dẫn, nếu không sẽ mở cửa sổ trùng (comment nhắc issue #61346, `main.rs:~985`).

## 3. Zed: settings theo worktree và toàn cục

- Một `SettingsStore` toàn cục (`crates/settings/src/settings_store.rs:154`) với các lớp: default, user, global, extension, server, và `local_settings: BTreeMap<(WorktreeId, Arc<RelPath>), SettingsContent>`. `WorktreeId` là duy nhất trong cả tiến trình, nên một store phục vụ mọi cửa sổ.
- `.zed/settings.json` (`paths::local_settings_file_relative_path`) có thể nằm ở bất kỳ thư mục nào trong worktree; `Project` theo dõi chúng và gọi `set_local_settings(worktree_id, dir, kind, content)` (`project_settings.rs:1658`). Các loại: `Settings`, `Tasks`, `Debug`, `Editorconfig`.
- File local chỉ được parse thành `ProjectSettingsContent` (`settings_store.rs:~1110`), một tập con của `SettingsContent` (`settings_content.rs:174` flatten `project` + `theme` + …). Vì vậy theme, keymap, tùy chọn toàn UI **không** đặt theo project được; chúng là toàn cục.
- Tra cứu: `Settings::get(Some(SettingsLocation { worktree_id, path }), cx)`; `value_for_path` (`settings_store.rs:1653`) duyệt ngược các giá trị local và trả về thư mục sâu nhất có path là tiền tố, nếu không thì dùng giá trị toàn cục đã merge. `get_global(cx)` = `get(None)`. Setting của editor/ngôn ngữ truyền vị trí file của buffer; code toàn app (hành vi đóng cửa sổ, restore) gọi `get_global`.
- Keymap là toàn cục (`cx.bind_keys` sau `cx.clear_key_bindings`), theme cũng là global; menu được dựng lại sau khi nạp lại keymap (`reload_menus`).

## 4. Zed: khôi phục khi khởi động

- Setting `restore_on_startup` (`settings_content/src/workspace.rs:525`): `last_session` (mặc định), `last_workspace`, `empty_tab`, `launchpad` (`assets/settings/default.json:171` vẫn ghi `none`).
- Theo dõi session (`crates/session/src/session.rs`): mỗi lần chạy có một `session_id` UUID; cái trước được giữ làm `last_session_id` trong KV store. `AppSession` poll `cx.window_stack()` và lưu thứ tự cửa sổ (`session_window_stack`), rồi lưu thêm một lần nữa trong `app_will_quit`.
- DB workspace (`crates/workspace/src/persistence.rs`) lưu mỗi workspace cùng `session_id`, `window_id`, `window_bounds`, `display` (uuid), dock, các item đang mở. `prepare_windows_to_quit` flush việc serialize trước khi thoát (`workspace.rs:12112`). Khi đóng một cửa sổ (không phải thoát), workspace bị xóa khỏi session (`remove_from_session`), nên chỉ những cửa sổ còn mở lúc thoát mới được khôi phục.
- `restore_or_create_workspace` (`crates/zed/src/main.rs:1422`): `restorable_workspace_locations` → với `last_session`, `last_session_workspace_locations(db, last_session_id, window_stack)`, rồi **đảo ngược** để cửa sổ trước nhất được mở sau cùng và nằm trên cùng (`main.rs:~1640`); các workspace được gom lại thành cửa sổ theo `window_id` (`persistence.rs:332`). Lùi về `last_workspace` khi không có session trước. Lỗi hiện toast; nếu không cửa sổ nào sống sót thì mở một cửa sổ trống.
- Mỗi cửa sổ khôi phục có bounds và display riêng đã lưu (xem 1.4). Các cửa sổ remote khôi phục dùng `WorkspaceMatching::None` để hai cửa sổ chung một gốc đều được khôi phục.
- Cùng hàm này chạy cho Dock reopen và `zed` không có đường dẫn.

## 5. Zed: thoát và đóng cửa sổ cuối

- `Quit` (`crates/zed/src/zed.rs:1754`): prompt `confirm_quit` tùy chọn; sau đó sắp xếp cửa sổ để **cửa sổ active hỏi trước**, rồi `prepare_windows_to_quit` (`workspace.rs:12075`) chạy `prepare_window_to_close(window, CloseIntent::Quit)` **lần lượt từng cửa sổ**. Bất kỳ lần cancel nào cũng dừng việc thoát (các cửa sổ đã chuẩn bị được flush; thoát bị hủy).
- `Workspace::prepare_to_close` (`workspace.rs:3733`): với `CloseIntent::Quit` được phép hot-exit: buffer dirty được serialize vào DB một cách im lặng và khôi phục lần chạy sau; không có prompt lưu. Với đóng cửa sổ thường, hot-exit chỉ được phép nếu tìm lại được workspace (có worktree hiển thị, hoặc đây là cửa sổ cuối và đóng nó là thoát); nếu không thì hiện prompt lưu/bỏ.
- Nút đóng cửa sổ: `window.on_window_should_close` trả `false` và chạy `MultiWorkspace::close_window` bất đồng bộ (prompt, flush, rồi `window.remove_window()`), `zed.rs:494`, `multi_workspace.rs:555`.
- Đóng cửa sổ cuối: setting `on_last_window_closed` = `platform_default` (macOS: vẫn chạy; nền tảng khác: thoát) | `quit_app` (`settings_content/src/workspace.rs:760`). Cài đặt ở `bind_on_window_closed` (`zed.rs:336`): `cx.on_window_closed(|cx, _| if cx.windows().is_empty() { cx.quit() })`, bind lại khi settings đổi. `closing_last_window_quits_app` (`workspace_settings.rs:53`) là predicate dùng chung (luôn true ngoài macOS).
- `when_closing_with_no_tabs` (`platform_default` = đóng cửa sổ trên macOS, giữ lại ở nơi khác): đóng tab cuối thì đóng cửa sổ.
- Bấm Dock khi không còn cửa sổ: `app.on_reopen` → `restore_or_create_workspace` (`main.rs:467`).

## 6. Zed: terminal, language server, trust

- Terminal: thuộc `Project.terminals`; panel terminal là theo từng `Workspace`. Không chia sẻ giữa các cửa sổ.
- Language server: `LspStore` theo từng `Project` (`LocalLspStore.language_servers`, `lsp_store.rs:~359`). `LanguageRegistry` (adapter, grammar, tải về) dùng chung qua `AppState.languages`, nhưng **tiến trình server không chia sẻ giữa các cửa sổ**: hai cửa sổ Rust chạy hai rust-analyzer.
- Trust (`crates/project/src/trusted_worktrees.rs`): một entity `TrustedWorktrees` toàn cục, khởi tạo một lần từ DB (`fetch_trusted_worktrees`, `main.rs:~487`). Trust tính theo đường dẫn worktree (file đơn < thư mục < override ở đường dẫn cha), được lưu bền và dùng chung cho mọi cửa sổ; mỗi `Workspace` subscribe `TrustedWorktreesEvent::Trusted` và serialize lại. Project bị hạn chế hiện modal bảo mật khi mở (`show_worktree_trust_security_modal`, `workspace.rs:~2440`).

## 7. GPUI ở b5cd310 (Apache-2.0): API nhiều cửa sổ

Tất cả trong `crates/gpui/src/app.rs` trừ khi ghi khác.

| API | Ghi chú |
|---|---|
| `cx.open_window(options, build_root)` (`app.rs:1605`) | trả `WindowHandle<V>`; vẽ một lần trước khi trả về. |
| `cx.windows()` (`:1581`) | mọi cửa sổ đang mở dạng `AnyWindowHandle`; `downcast::<V>()` để lọc theo view gốc. Thứ tự là thứ tự slot-map, không phải z-order. |
| `cx.window_stack()` (`:1593`) | thứ tự từ trước ra sau; là `Option` vì platform có thể không cài (impl mặc định của trait trả `None`, `platform.rs:402`; macOS dùng `NSApp orderedWindows`). |
| `cx.active_window()` (`:1598`) | key window của platform. Có thể là `None` khi app không ở foreground, nên lùi về `window_stack().first()` hoặc cửa sổ focus gần nhất mà bạn tự theo dõi. |
| `cx.on_window_closed(\|cx, WindowId\|)` (`:2867`) | chạy sau khi cửa sổ đã bị gỡ (không truy cập được nữa). |
| `QuitMode` + `Application::with_quit_mode` / `cx.set_quit_mode` (`:526`, `:291`, `:2096`) | `Default` = `Explicit` trên macOS, `LastWindowClosed` ở nơi khác; được kiểm tra ngay sau các observer closed (`:3294`). Lucerna có thể dùng cái này thay cho `on_window_closed` quit tự viết. |
| `Application::on_reopen` (`:369`) | macOS `applicationShouldHandleReopen:hasVisibleWindows:`; GPUI chỉ gọi lại **khi không còn cửa sổ nào hiển thị** (`gpui_macos/src/platform.rs:1373`). |
| `Application::on_open_urls` (`:359`) | macOS `application:openURLs:` (Finder Open With, `open -a`, URL LaunchServices gồm cả `file://`). Phải đăng ký trước `run`. |
| `cx.on_app_quit(\|cx\| future)` (`:2842`) | chạy trong `shutdown`, future được cấp `SHUTDOWN_TIMEOUT`; **không hủy được**. Không có hook "should quit" để veto: Zed veto bằng cách làm mọi prompt trong action `Quit` *trước khi* gọi `cx.quit()`. `Platform::on_quit` nối với `applicationWillTerminate:`. |
| `window.on_window_should_close(cx, \|window, cx\| -> bool)` (`window.rs:7162`) | veto theo cửa sổ cho nút đóng; trả `false` và đóng sau bằng `window.remove_window()` (`window.rs:2599`). |
| `cx.quit()` (`:1343`) | macOS: `[NSApp terminate:]` bất đồng bộ. Lưu ý: thoát do hệ thống (logout, Dock "Quit") bỏ qua action `Quit` của bạn; lúc đó chỉ `on_app_quit` chạy. |
| `cx.set_menus(..)` (`:2902`) | một thanh menu cho cả app; mục menu gửi action đến cửa sổ đang focus, không thì đến handler global `cx.on_action`. Action phải chạy được khi không có cửa sổ (Quit, Open, New Window, Open Recent) cần handler global. |
| `cx.set_dock_menu(..)`, `cx.add_recent_document(path)` | menu Dock / tài liệu gần đây. |
| `cx.activate(ignoring_other_apps)`, `window.activate_window()` | đưa app / một cửa sổ lên trước. |
| `WindowOptions.tabbing_identifier` (`gpui_macos/src/window.rs:1011, 1248, 1290`) | `Some(id)` bật tab tự động của `NSWindow`; cửa sổ mới gia nhập cửa sổ chính như một tab native khi tùy chọn hệ thống "prefer tabs" của người dùng là Always (hoặc InFullScreen khi đang fullscreen). `None` tắt tab toàn cục. Zed phơi ra là `use_system_window_tabs`. |

Hỗ trợ test (`crates/gpui/src/app/test_context.rs`, `crates/gpui/src/platform/test/platform.rs`):
- `TestAppContext::add_window(|window, cx| V)` → `WindowHandle<V>`; `add_window_view` / `add_empty_window` → `&mut VisualTestContext`; `cx.windows()`; `cx.update_window(..)`; `run_until_parked()`.
- `VisualTestContext::simulate_close()` gọi handler should-close của cửa sổ và trả về việc nó có cho đóng hay không.
- `simulate_prompt_answer(button)`, `has_pending_prompt()`, `simulate_path_prompt_response` cho các luồng prompt (ví dụ prompt buffer dirty giữa các cửa sổ).
- `TestPlatform::active_window` là một `RefCell` được đặt khi cửa sổ test được activate (`window.rs:423`), nên `cx.active_window()` chạy được trong test; `window_stack()` trả `None` (impl mặc định).
- `TestPlatform::on_open_urls` và `on_reopen` là **`unimplemented!()`** và `quit()` không làm gì. Giữ logic định tuyến trong các hàm thường nhận `&mut App` (như `handle_open_request`, `restore_or_create_workspace` của Zed) và test chúng; chỉ phần nối mỏng `Application::on_*` là không test.
- `TestAppContext::new_app()` dựng một app thứ hai trên cùng dispatcher (hữu ích cho test kiểu "instance thứ hai").

## 8. VS Code

File (microsoft/vscode `main`): `src/vs/platform/windows/electron-main/windowsMainService.ts`, `windowsFinder.ts`, `windowsStateHandler.ts`, `src/vs/platform/launch/electron-main/launchMainService.ts`, `src/vs/code/electron-main/main.ts`, `app.ts`, `src/vs/platform/lifecycle/electron-main/lifecycleMainService.ts`, `src/vs/code/node/cli.ts`, `src/vs/platform/environment/node/argv.ts`, `src/vs/workbench/browser/workbench.contribution.ts`, `src/vs/workbench/electron-browser/desktop.contribution.ts`, `src/vs/workbench/contrib/files/browser/files.contribution.ts`, `src/vs/workbench/services/workingCopy/electron-browser/workingCopyBackupTracker.ts`.

### 8.1 Mô hình tiến trình
Một tiến trình main Electron sở hữu mọi cửa sổ (`windowsMainService`). Mỗi cửa sổ là một renderer với **extension host riêng**, nên extension và language server chúng khởi động chạy một lần cho mỗi cửa sổ. Pty của terminal tích hợp nằm trong một tiến trình "pty host" dùng chung, nhưng terminal thuộc về một cửa sổ.

### 8.2 Settings chọn cửa sổ
- `window.openFoldersInNewWindow` (`on` | `off` | `default`, mặc định `default`, scope APPLICATION): `on` cửa sổ mới; `off` **thay thế cửa sổ active gần nhất**; `default` = cửa sổ mới, trừ khi chọn từ trong app (menu File/hộp thoại) thì thay thế. Bị bỏ qua khi có `--new-window` / `--reuse-window`.
- `window.openFilesInNewWindow` (`on` | `off` | `default`, mặc định `off`): `off` = cửa sổ đang mở thư mục chứa file, không thì cửa sổ active gần nhất. `default` trên macOS = như `off` trừ khi mở qua Dock hoặc Finder; trên Linux/Windows là cửa sổ mới trừ khi mở từ hộp thoại/menu hoặc từ terminal của chính VS Code (`TERM_PROGRAM=vscode`). Logic: `shouldOpenNewWindow` (`windowsMainService.ts:1432`).
- `window.openWithoutArgumentsInNewWindow` (`on` | `off`, mặc định `off` trên macOS, `on` ở nơi khác): lần chạy thứ hai không có tham số sẽ làm gì (`launchMainService.ts:~171`): cửa sổ trống mới hay focus cửa sổ active gần nhất.
- `window.restoreWindows` (`preserve` | `all` | `folders` | `one` | `none`, mặc định `all`): `preserve` mở lại mọi cửa sổ *và* vẫn mở các thư mục từ CLI thành cửa sổ thêm; `all` mở lại mọi cửa sổ trừ khi dòng lệnh có truyền thư mục/workspace/file; `folders` chỉ các cửa sổ có thư mục; `one` chỉ cửa sổ active gần nhất; `none` một cửa sổ trống. Sau khi cập nhật nó luôn hành xử như `all` (`getRestoreWindowsSetting`, `windowsMainService.ts:1134`).
- `window.closeWhenEmpty` (mặc định false): đóng editor cuối thì đóng cửa sổ, chỉ cho cửa sổ không có thư mục.
- `window.newWindowDimensions` (`default` | `inherit` | `offset` | `maximized` | `fullscreen`), `window.restoreFullscreen`, `window.nativeTabs` (tab native macOS, cần khởi động lại), `window.confirmBeforeClose` (`always` | `keyboardOnly` | `never`, mặc định desktop `never`).

### 8.3 Tìm cửa sổ cho một đường dẫn
`windowsFinder.ts`:
- `findWindowOnFile(windows, fileUri)`: trước hết các workspace multi-root có thư mục chứa file, rồi các cửa sổ một thư mục là cha của file, **thư mục sâu nhất thắng** (sắp theo độ dài đường dẫn).
- `findWindowOnWorkspaceOrFolder`: khớp chính xác URI thư mục hoặc file workspace; dùng để mở một thư mục đã mở thì focus cửa sổ đó.
- Trong `doOpen` (`windowsMainService.ts:~620`): file từ CLI/Dock/desktop/link dùng `findWindowOnFile`, không thì cửa sổ active gần nhất; nếu cửa sổ đó có thư mục, thư mục được "mở lại" (focus) cùng các file; nếu nó trống, file vào thẳng đó; nếu không có cửa sổ, mở cái mới.
- Truyền nhiều thư mục trên CLI thì thành một workspace multi-root untitled.
- Một cửa sổ bị "thay thế" (mở thư mục với `openFoldersInNewWindow: off`): renderer được unload với `ShutdownReason.LOAD` (kiểm tra dirty / hot exit) và nạp thư mục mới trong cùng cửa sổ native.

### 8.4 CLI `code <path>`
- `code` chạy `cli.ts` dưới Electron-as-Node. Trên macOS nó khởi động app bằng `open -a <app> --args …` (env truyền qua `--env`); nơi khác thì spawn binary.
- Tiến trình main mới thử **chiếm main IPC socket** (`environmentMainService.mainIPCHandle`, Unix socket / named pipe; `main.ts:352 claimInstance`). `EADDRINUSE` → kết nối làm client, gọi `launch.start(args, env)` trên instance đang chạy rồi thoát ("Sent env to running instance. Terminating..."). Nếu kết nối lỗi vì socket cũ, nó unlink và thử lại một lần.
- `launchMainService.start` ánh xạ cờ: `-n/--new-window` → `forceNewWindow`, `-r/--reuse-window` → `forceReuseWindow`, `-a/--add` → `addMode` (thêm thư mục vào cửa sổ active gần nhất; bị bỏ qua khi không có cửa sổ hoặc lúc khởi động), `--remove`, `-g/--goto file:line[:col]` → `gotoLineMode`, `-d/--diff`, `--merge`. `preferNewWindow = !reuse && !wait`.
- `-w/--wait`: CLI tạo một file "wait marker" tạm và truyền `--waitMarkerFilePath`; app xóa nó khi editor/cửa sổ đóng; CLI poll `whenDeleted`. Chính instance thứ hai cũng chờ `whenClosedOrLoaded` của cửa sổ được dùng.
- Môi trường từ shell được chuyển tiếp (`userEnv`) và dùng cho cửa sổ (terminal, task).

### 8.5 Cửa sổ cuối trên macOS, Dock, cửa sổ mới
- `lifecycleMainService.ts:300`: `window-all-closed` chỉ thoát nếu có yêu cầu thoát hoặc không phải macOS. Nên trên macOS app vẫn chạy chỉ với thanh menu.
- `app.on('activate', (e, hasVisibleWindows))` (`app.ts:562`): không có cửa sổ hiển thị → `openEmptyWindow({ context: DOCK })`. Nó **không** khôi phục session cuối (khôi phục chỉ xảy ra lúc khởi động).
- `app.on('new-window-for-tab')` → cửa sổ trống (nút "+" của tab macOS).
- `workbench.action.newWindow` (cmd+shift+n / ctrl+shift+n) mở cửa sổ trống (`forceNewWindow`, `forceEmpty`); nó hiện trang Welcome theo `workbench.startupEditor`. Cửa sổ mới kế thừa profile của cửa sổ active gần nhất.
- Bounds cửa sổ: `windowsStateHandler.ts` lưu `windowsState` = `{ lastActiveWindow, openedWindows[] }` với uiState (bounds, mode, display) theo từng thư mục/workspace/backup trống; thư mục mở lại có bounds riêng, cửa sổ mới xếp bậc thang từ cửa sổ active gần nhất (`ensureNoOverlap`). Trên macOS trạng thái của cửa sổ vừa đóng cuối được giữ trong bộ nhớ để dùng cho cửa sổ kế tiếp.

### 8.6 Thoát khi còn editor dirty
- Thoát: Electron `before-quit` → mỗi cửa sổ được unload với `ShutdownReason.QUIT`; bất kỳ cửa sổ nào cũng có thể veto, làm hủy cả việc thoát.
- `files.hotExit` (`off` | `onExit` mặc định | `onExitAndWindowClose`): khi QUIT mọi thứ dirty được backup, **không prompt**, khôi phục lần chạy sau (`workingCopyBackupTracker.ts:217`). Khi đóng cửa sổ (CLOSE) với `onExit`: macOS hoặc nhiều hơn một cửa sổ → prompt (cửa sổ có thư mục chỉ tự backup scratchpad); cửa sổ cuối trên Win/Linux → backup, vì app thoát ngay sau đó. `onExitAndWindowClose` → backup cả cửa sổ có thư mục khi đóng.
- Với `hotExit: off`, mỗi cửa sổ lần lượt prompt trong lúc thoát.

### 8.7 Workspace trust, settings
- Trust tính theo URI thư mục (hoặc thư mục cha), lưu một lần trong application storage và dùng chung cho mọi cửa sổ; mỗi cửa sổ tự đánh giá trust cho các thư mục của mình và phản ứng với thay đổi làm ở cửa sổ khác. Cửa sổ trống theo `security.workspace.trust.emptyWindow`.
- Settings: settings user (theo profile) dùng chung cho mọi cửa sổ; settings workspace (`.vscode/settings.json` hoặc file `.code-workspace`) theo từng cửa sổ; settings folder theo từng gốc trong multi-root. Scope của setting: `application` (chỉ từ settings user, do tiến trình main đọc, ví dụ mọi `window.open*`/`restoreWindows`), `machine`, `window`, `resource`, `language-overridable`.

## 9. Giống và khác

| Chủ đề | VS Code | Zed (b5cd310) |
|---|---|---|
| Đơn vị mỗi cửa sổ | 1 workspace (thư mục hoặc multi-root) | `MultiWorkspace`: N project + sidebar |
| Mở thư mục mặc định | cửa sổ mới; từ menu thì thay cửa sổ hiện tại | thêm vào cửa sổ hiện tại (`existing_window`) |
| Mở file | cửa sổ chứa nó (thư mục sâu nhất), không thì cửa sổ active gần nhất | workspace chứa nó, không thì cửa sổ active |
| Mở thư mục đã mở | focus cửa sổ đó | focus workspace đó (MatchExact) |
| Dùng lại cửa sổ trống | cửa sổ trống nhận file | workspace trống hạ NewWindow→Activate |
| Kênh CLI | main IPC socket do instance đầu tiên chiếm | URL LaunchServices (mac) / unix datagram (Linux) + kênh ngược ipc-channel |
| Cờ CLI | `-n -r -a -g -w --remove` | `-n -a -e -w` (+ ẩn `-r --classic`) |
| Khôi phục | `restoreWindows` all/preserve/folders/one/none | `restore_on_startup` last_session/last_workspace/empty_tab/launchpad |
| Cửa sổ cuối (mac) | app vẫn chạy; Dock → cửa sổ trống | app vẫn chạy; Dock → khôi phục session (hoặc tạo mới) |
| Cửa sổ cuối (Linux/Win) | thoát | thoát (`on_last_window_closed`) |
| Thoát khi còn dirty | hot exit mặc định, không prompt | hot exit vào DB, không prompt; cửa sổ active trước |
| Language server | theo cửa sổ (extension host) | theo project (≈ theo cửa sổ) |
| Terminal | theo cửa sổ, pty host dùng chung | theo project |
| Settings project | `.vscode/settings.json` theo cửa sổ | `.zed/settings.json` theo thư mục worktree, một store toàn cục khóa theo worktree |
| Theme/keymap | cấp user (theme có thể là setting workspace) | chỉ toàn cục |
| Trust | theo thư mục, kho toàn cục | theo đường dẫn worktree, kho toàn cục |

## 10. Lucerna cần chú ý gì

Lucerna hiện tại (xem chỉ-đọc `crates/lucerna-workbench/src/app.rs`, `app_state.rs`, `process.rs`): `Router { shown: Option<…> }` giả định một cửa sổ; `start()` thoát khi đóng cửa sổ cuối; `open_requests::route` có thể trả `Route::NewProcess` và `process::spawn` khởi động thêm một `luc`; `AppState.settings` là một `SettingsStore` với đúng một lớp `workspace`.

1. **Phân lớp settings là cái bẫy lớn nhất.** `AppState.settings` trộn lớp user và *một* lớp workspace ở mức toàn cục. Với hai cửa sổ trên hai thư mục, lớp workspace phải theo từng cửa sổ (hoặc khóa theo gốc như Zed làm với `(WorktreeId, path)`). Mọi chỗ gọi `settings.get(..)` khi đó cần một vị trí (cửa sổ/gốc hoặc đường dẫn file). Theo scope của VS Code: các setting điều khiển cửa sổ (`window.*`, restore, hành vi mở) là scope application, chỉ đọc từ settings user.
2. **Hành vi theo VS Code, cơ chế theo GPUI**: cài `window.openFoldersInNewWindow`, `window.openFilesInNewWindow`, `window.restoreWindows`, `window.closeWhenEmpty`, `window.openWithoutArgumentsInNewWindow`, ngữ nghĩa `files.hotExit`; dùng quy tắc finder của VS Code (thư mục sâu nhất thắng; thư mục đã mở thì focus). Chép quy tắc "cửa sổ trống được dùng lại" của Zed.
3. **Mọi lần mở đi qua một hàm** (như `open_paths`) nhận một struct options và trả về cửa sổ được dùng; Finder, Dock, CLI, menu, danh sách recent, restore đều gọi nó. Giữ nó là `fn(&mut App)` thường để test được: `TestPlatform::on_open_urls`/`on_reopen` panic.
4. **CLI → app đang chạy**: Lucerna đã nhận đường dẫn qua `on_open_urls`. Với `luc <path>` kèm cờ (`-n`, `-r`, `-a`, `-g`, `-w`), CLI cần một kênh mang cờ và env, và một câu trả lời cho `--wait`. Lựa chọn: socket kiểu VS Code (instance đầu tiên chiếm `<data_dir>/luc.sock`; xử lý socket cũ) hoặc URL kiểu Zed với kênh ngược one-shot. Tránh kiểm tra single-instance bằng cổng TCP của Zed; Unix socket trong thư mục data đơn giản hơn và mang sang Linux được. Chú ý race khởi động nguội (issue #61346 của Zed): xếp hàng các yêu cầu CLI cho đến khi restore đã mở xong các cửa sổ.
5. **Thoát**: `on_app_quit` của GPUI không veto được. Làm prompt/hot exit trong action `Quit` trên mọi cửa sổ *trước* `cx.quit()`, cửa sổ active trước, từng cái một; Cancel hủy cả việc thoát. Thoát do hệ thống (logout, Dock Quit) chỉ chạy `on_app_quit`, nên hot exit cũng phải an toàn ở đó (ghi backup đồng bộ trong `SHUTDOWN_TIMEOUT`, hoặc luôn giữ backup cập nhật). Language server: `QuitShutdown` đã theo dõi mọi entity `LanguageServers`; kiểm tra nó vẫn đúng khi có nhiều cửa sổ.
6. **Đóng cửa sổ cuối**: thay `on_window_closed → quit` tự viết trong `start()` bằng `QuitMode::Default` (macOS vẫn chạy, nơi khác thoát) hoặc giữ observer nhưng chỉ ngoài macOS. Rồi cài `on_reopen` (bấm Dock khi không còn cửa sổ) và làm cho action menu chạy được khi không có cửa sổ (`cx.on_action` global cho Quit, Open, Open Recent, New Window, Settings).
7. **`active_window()` có thể là `None`** khi app ở background (ví dụ CLI chạy từ Terminal). Tự theo dõi "cửa sổ workbench active gần nhất" (như `getLastActiveWindow` của VS Code), lùi về `window_stack()`.
8. **Khôi phục**: lưu danh sách cửa sổ và thứ tự trước ra sau lúc thoát (`window_stack`), xóa cửa sổ khỏi session khi người dùng đóng nó, khôi phục theo thứ tự đảo ngược để cửa sổ trước nhất nằm trên cùng, và giữ bounds + display theo từng thư mục. Cần quyết định hành vi Dock reopen: VS Code mở cửa sổ trống, Zed khôi phục session.
9. **Cùng file ở hai cửa sổ**: hai editor có buffer riêng (Zed và VS Code đều làm vậy giữa các cửa sổ). Lucerna cần reload khi đĩa đổi và xử lý xung đột chạy đúng khi cửa sổ kia lưu; cân nhắc focus cửa sổ đã có file (finder của VS Code) để trường hợp này hiếm.
10. **Tài nguyên theo cửa sổ và dùng chung**: language server theo cửa sổ khớp cả hai editor và giữ mọi thứ đơn giản; chia sẻ một server giữa các cửa sổ cần hỗ trợ workspace-folder cho từng server. Giữ `LanguageRegistry`, theme, icon theme, extension, keymap là global của app. Kho trust là global, đánh giá theo gốc của từng cửa sổ; đổi trust ở một cửa sổ phải cập nhật các cửa sổ khác.
11. **Global giả định một cửa sổ**: rà các kiểu `impl Global` (`StatusMessage`, `SystemShellGlobal`, `Spawner`, `DefaultTerminalSpawner`, `QuitShutdown`, `TestRouter`) và trường `Router.shown`. Riêng status message ở cả hai editor đều theo từng cửa sổ.
12. **Tab native macOS**: GPUI chỉ bật khi `WindowOptions.tabbing_identifier` là `Some`; để `None` (`window.nativeTabs` của VS Code mặc định false) trừ khi có setting yêu cầu.
13. **Test**: dùng `TestAppContext::add_window` cho nhiều cửa sổ, `cx.windows()`, `VisualTestContext::simulate_close()`, `simulate_prompt_answer` cho luồng thoát khi còn dirty. `window_stack()` là `None` trong test, nên code không được dựa vào nó để đúng.
14. **Tách riêng code theo nền tảng**: đường dẫn socket, LaunchServices, Dock reopen và "thoát khi đóng cửa sổ cuối" khác nhau theo OS; đặt chúng sau các hàm nhỏ để Linux/Windows theo sau được.

## 11. Lucerna chọn gì

- Một tiến trình `luc` cho mọi cửa sổ; Dock một icon.
- Theme, icon theme, keymap: chung cho cả app, chỉ đọc từ settings user (theo Zed; lệch VS Code vì VS Code cho đặt `workbench.colorTheme` trong workspace). Lý do: giữ nguyên ~140 chỗ đọc màu, mọi cửa sổ trông như nhau.
- Settings: lớp default và user dùng chung; lớp workspace (`.lucerna/settings.json`) theo từng cửa sổ, là một entity riêng mà mỗi `Workbench` sở hữu và truyền xuống các entity con (theo VS Code: mỗi cửa sổ có `ConfigurationService` riêng). Không theo kiểu khóa theo vị trí của Zed vì Lucerna chỉ có một file settings ở thư mục gốc.
- Đóng cửa sổ cuối: macOS app vẫn chạy, bấm Dock mở Welcome (theo VS Code); Linux/Windows thoát.
- `luc <path>`: Unix socket trong thư mục data (theo VS Code), không dùng LaunchServices.
- `window.restoreWindows` mặc định `"all"` như VS Code.
- Mở thư mục đã mở ở cửa sổ khác thì focus cửa sổ đó; file mở ở cửa sổ có thư mục chứa nó sâu nhất (`findWindowOnFile`).
- Language server, terminal: theo từng cửa sổ như cả hai bên. Trust: một kho chung, đánh giá theo gốc của từng cửa sổ, đổi ở cửa sổ này thì cửa sổ khác cập nhật.
