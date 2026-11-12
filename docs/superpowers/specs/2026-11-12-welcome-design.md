# Màn Welcome và Open Recent

Ngày 2026-11-12. Trạng thái: chờ duyệt.

## 1. Mục tiêu

Mở Lucerna từ Dock hoặc Finder mà không kèm thư mục thì hiện **màn Welcome** thay cho hộp thoại chọn thư mục, nơi bấm Cancel là app thoát. Từ Welcome và từ trong một cửa sổ đang mở thư mục, người dùng mở lại thư mục gần đây bằng **Open Recent** (⌃R) như trong VS Code. Khi không còn tab nào, vùng editor gợi ý vài phím tắt.

Hiện trạng:
- `app.rs` `pick_folder` hiện hộp thoại ngay sau 300 ms chờ yêu cầu mở từ Finder.
- App chưa có danh sách thư mục gần đây. `RecentFiles` chỉ phục vụ Quick Open bên trong một project.
- `editor_groups/view.rs` `empty_hint` đã hiện "Lucerna", "Go to file ⌘P" và "Find in project ⇧⌘F".

Quyết định (W1–W4 là phạm vi người dùng đã đồng ý; W5–W12 do nghiên cứu đề xuất, cần duyệt):

| # | Quyết định | Lý do |
|---|---|---|
| W1 | Welcome là **root view riêng** của cửa sổ không có thư mục (`WelcomeView`), gồm title bar và một cột nội dung căn giữa. Chọn thư mục thì cửa sổ đổi root sang `Workbench` (`window.replace_root`), không mở cửa sổ mới. | `Workbench` luôn cần root, và cấu hình workspace là global gắn với một root. Zed vẽ Welcome trong workspace không có worktree, còn Lucerna không có workbench rỗng. Đổi root ngay trong cửa sổ thì không bị nháy. |
| W2 | Lệnh `workbench.action.openRecent`, phím `ctrl+r`. Picker dùng lại khung picker sẵn có (`popover`, `picker_input_row`, `picker_list`, `picker_row`, `fuzzy_filter`). | Lệnh và phím theo VS Code, giao diện dùng chung với Quick Open và theme picker. |
| W3 | Vùng editor trống hiện watermark gồm các lệnh và phím, lấy nhãn từ keymap của người dùng. Bật/tắt bằng `workbench.tips.enabled`. | VS Code làm vậy. Zed để trống pane khi có thư mục; Lucerna lệch khỏi Zed ở đây vì người dùng đến từ VS Code cần thấy phím. Watermark đã có, chỉ mở rộng thêm. |
| W4 | Setting `workbench.startupEditor`, mặc định `"welcomePage"`. | Theo VS Code (§5). |
| W5 | Danh sách lưu ở `<data_dir>/recent.json`, tối đa 100 mục, mới nhất đứng đầu. Đường dẫn không còn tồn tại thì **bị ẩn khi đọc** (như Zed) và **bị loại khỏi file ở lần ghi kế tiếp**. | "Tự bỏ" đúng như yêu cầu. Kiểm tra tồn tại chạy ở luồng nền. |
| W6 | Danh sách ghi **cấp cửa sổ**: một thư mục mở thành cửa sổ, hoặc một file mở riêng (`single_file`). File mở thành tab bên trong một thư mục thì không ghi (Quick Open đã có `RecentFiles` cho việc đó). | VS Code ghi cả file. Lucerna chỉ để màn Welcome và Open Recent đưa người dùng về đúng cửa sổ; danh sách file trong project đã có chỗ khác. |
| W7 | Trong picker, Enter mở **thay cửa sổ hiện tại**, ⌘Enter mở **cửa sổ mới**. Ở Welcome, bấm chuột mở trong cửa sổ đó, giữ ⌘ khi bấm thì mở cửa sổ mới. | Theo VS Code. |
| W8 | "Thay cửa sổ hiện tại" khi đang mở một thư mục: hỏi lưu như lúc đóng cửa sổ (`confirm_quit`), lưu layout, **khởi động `luc <path>` ở tiến trình mới rồi đóng cửa sổ này**. "Cửa sổ mới" thì chỉ khởi động tiến trình mới. | Mỗi tiến trình chỉ có một cửa sổ (`open_requests.rs`). Bỏ một `Workbench` đang chạy (LSP, watcher, task) ngay trong tiến trình là rủi ro rò tài nguyên. Tiến trình mới thì sạch, và đường `Route::NewProcess` đã có sẵn. Riêng Welcome không có trạng thái nặng nên đổi root ngay tại chỗ (W1). |
| W9 | Welcome trên macOS có "Open…" (⌘O, `workbench.action.files.openFileFolder`, hộp thoại chọn được cả file lẫn thư mục) và "Open Recent…" (⌃R). Trên Linux và Windows thì thay "Open…" bằng "Open File…" và "Open Folder…". | Theo VS Code ở từng nền tảng. Phím và nhãn theo nền tảng nằm ở `cfg`, nên không thêm giả định chỉ có macOS vào chỗ khác. |
| W10 | Thêm `workbench.action.closeFolder` (`cmd+k f`): hỏi lưu, rồi đưa về Welcome bằng cách chạy `luc` không kèm đường dẫn ở tiến trình mới với `cwd = /` và đóng cửa sổ hiện tại. | VS Code có lệnh này. Có Welcome mà không quay lại được thì thiếu. |
| W11 | Menu **File** của app: Open…, Open Recent ▸ (tối đa 10 mục, More…, Clear Recently Opened), Close Folder. | Người dùng macOS tìm Open Recent trong menu File. GPUI có submenu. |
| W12 | Không có nội dung AI, đăng nhập, onboarding, New File hay Clone Repository. | Lucerna không có các tính năng đó. Untitled file chưa có. |

**Thành công khi:**
- Mở `Lucerna.app` từ Dock: hiện Welcome có danh sách thư mục gần đây. Bấm một mục thì cửa sổ thành workbench của thư mục đó, không có cửa sổ thứ hai.
- ⌃R ở Welcome hay trong workbench đều mở picker. Gõ để lọc. Enter mở thư mục, ⌘Enter mở cửa sổ mới. Bấm × thì mục biến mất khỏi picker và khỏi `recent.json`.
- Xóa (hoặc đổi tên) một thư mục từng mở: lần sau nó không còn trong Welcome hay picker, và không còn trong `recent.json` sau lần ghi kế tiếp.
- Đóng hết tab: vùng editor hiện watermark có phím đúng theo keymap người dùng. Đặt `"workbench.tips.enabled": false` thì watermark biến mất.
- `"workbench.startupEditor": "none"` thì khởi động như hiện nay (hộp thoại chọn thư mục).
- Mọi test cũ vẫn xanh. `cargo clippy`, `cargo fmt --check`, `cargo deny check` đều sạch.

**Ngoài phạm vi:**
- Trang Welcome dạng tab trong cửa sổ có thư mục (`startupEditor: "welcomePage"` của VS Code mở Welcome cả khi không khôi phục tab nào; Lucerna để watermark làm việc này).
- Giá trị `readme`, `newUntitledFile`, `terminal`.
- Dock menu recent (`NSApplication` dock menu).
- Workspace nhiều thư mục; remote.
- Đồng bộ danh sách giữa các tiến trình đang chạy: mỗi tiến trình đọc file khi cần nên luôn thấy bản mới nhất.

## 2. Tham khảo

Chi tiết ở `docs/research/2026-11-12-welcome.md`. Tóm tắt:
- **VS Code:**
  - Welcome là tab, có Start (Open… / Open File… / Open Folder…) và Recent (5 thư mục, nút × khi hover, link More…).
  - `workbench.startupEditor` có `none | welcomePage | readme | newUntitledFile | welcomePageInEmptyWorkbench | terminal`.
  - `workbench.action.openRecent` dùng `ctrl+r`; picker chia nhóm "folders" và "files", có nút ×; Enter mở cùng cửa sổ, ⌘Enter mở cửa sổ mới.
  - Danh sách là JSON trong storage cấp ứng dụng, tối đa 500 mục; đường dẫn mất vẫn giữ lại.
  - Watermark bật/tắt bằng `workbench.tips.enabled`.
- **Zed:**
  - `WelcomePage` vẽ trong pane trống của workspace không có thư mục. Cột rộng tối đa 512 px, căn giữa. Có logo và tiêu đề; section header VIẾT HOA cỡ XSmall, màu muted, kèm đường kẻ.
  - Các nút rộng hết cột, gồm icon muted, nhãn, và phím ở bên phải. Có 5 dự án gần đây, `cmd-1`…`cmd-5` mở từng dự án.
  - Picker `OpenRecent` dùng `ctrl-r`; nút ở cuối dòng chỉ hiện khi hover.
  - Đường dẫn mất bị ẩn khi đọc. Pane trống trong cửa sổ có thư mục thì để trống.
- **Lucerna:** lệnh, phím và setting theo VS Code; bố cục Welcome, cách hiện nút × khi hover và `cmd-1`…`cmd-5` theo Zed; giữ watermark (lệch khỏi Zed, xem W3).

## 3. Danh sách gần đây (`recent.rs`)

Module thuần, không dùng gpui, để test bằng unit test.

```rust
pub enum RecentKind { Folder, File }
pub struct RecentEntry { pub path: PathBuf, pub kind: RecentKind }
pub struct RecentList { entries: Vec<RecentEntry> } // mới nhất trước
```

File `<data_dir>/recent.json` (`data_dir` là `default_data_dir()`, cùng chỗ với `workspaces/`):

```json
{ "version": 1, "entries": [ { "path": "/Users/me/code/lucerna", "kind": "folder" } ] }
```

Các thao tác:
- `load(path) -> RecentList`:
  - File không có: danh sách rỗng.
  - File hỏng hoặc khác version: đổi tên thành `.bak` như `load_state`, rồi trả danh sách rỗng.
  - Không lọc đường dẫn mất ở bước này.
- `existing(&self) -> Vec<RecentEntry>`: chỉ giữ mục mà `path` còn tồn tại và đúng loại (thư mục với `Folder`, file với `File`). Hàm này gọi `stat`, nên UI luôn chạy nó trên `background_executor`.
- `record(data_dir, entry)`, `remove(data_dir, path)`, `clear(data_dir)`: mỗi hàm đọc lại file, sửa, bỏ mục mất, rồi ghi nguyên tử. Cụ thể:
  - `record` bỏ mục trùng `path` rồi đặt mục mới lên đầu, cắt còn `RECENT_LIMIT = 100`.
  - Ghi nguyên tử dùng chung hàm ghi tạm rồi rename với `save_state`. Tách hàm đó thành `write_atomic(path, bytes)` trong `workspace_state.rs`.
  - Các hàm này cũng chạy ở luồng nền.
- Đường dẫn luôn canonical, vì `Router::open_path` đã canonicalize.

Nhiều tiến trình `luc` cùng ghi một file: mỗi lần ghi đều đọc lại ngay trước khi ghi, nên chỉ mất dữ liệu khi hai lần ghi chồng nhau trong vài mili giây. Chấp nhận được với một danh sách gợi ý; không dùng khóa file.

**Khi nào ghi:** `Router` gọi `record` sau khi `open_workbench` mở được cửa sổ (từ CLI, Finder, Welcome hay picker). Có hai loại mục:
- `Folder(root)`, khi cửa sổ mở một thư mục;
- `File(file)`, khi cửa sổ mở một file riêng (`single_file`).

Mở `luc somefile` trong một repo git thì ghi thư mục root, vì cửa sổ là của thư mục đó.

**Hiển thị:**
- Tên là `file_name()`.
- Mô tả là đường dẫn cha, rút gọn `~` bằng logic sẵn có của `path_bar_text`; đưa logic `~` đó ra một hàm dùng chung.
- Thư mục đang mở trong cửa sổ hiện tại không có trong danh sách hiển thị (giống trang Welcome của VS Code), nên focus luôn đặt ở mục đầu.

## 4. Màn Welcome (`welcome.rs`)

### 4.1 Bố cục (theo Zed `welcome.rs`)

```
┌──────────────────────────────────────────────────────────────┐
│ ● ● ●                       Lucerna                          │  title bar 34 px (layout::title_bar)
├──────────────────────────────────────────────────────────────┤
│                                                              │
│                [logo 45px]  Welcome to Lucerna               │
│                             A minimal code editor            │  muted, nhỏ
│                                                              │
│                GET STARTED ───────────────────────────       │  section header
│                [📂] Open…                          ⌘O        │
│                [🕘] Open Recent…                   ⌃R        │
│                                                              │
│                RECENT ────────────────────────────────       │
│                [▢] lucerna    ~/Projects.nosync     ⌘1  ×    │  × chỉ khi hover
│                [▢] zed        ~/src                 ⌘2       │
│                [▢] notes.md   ~/Desktop             ⌘3       │  kind File: icon file
│                More…                                         │  chỉ khi > 5 mục
│                                                              │
└──────────────────────────────────────────────────────────────┘
```

- Nền là `editor_background`, cả trang không có status bar. Cột nội dung rộng tối đa 512 px, căn giữa cả hai chiều, padding `8×GRID`, các khối cách nhau `6×GRID`, cuộn dọc nếu cửa sổ thấp.
- Đầu trang:
  - logo Lucerna (`assets/brand/mark.svg`) cạnh tiêu đề "Welcome to Lucerna", cỡ 20 px;
  - dòng phụ cỡ `UI_FONT_SIZE_SMALL`, màu `text_muted`.
  - Khi đã có mục gần đây thì tiêu đề là "Welcome back to Lucerna", như Zed.
- **Section header** (component mới trong `ui.rs`: `section_header(title, cx)`): tiêu đề viết hoa, cỡ `UI_FONT_SIZE_CAPTION`, font editor, màu `text_muted`, theo sau là đường kẻ 1 px `border_variant` chiếm phần còn lại.
- **Nút hàng** (component mới trong `ui.rs`: `list_button(id, icon, label, keys, cx)`):
  - cao `7×GRID` (bằng `PICKER_ROW_HEIGHT`), rộng hết cột, bo `GRID`;
  - hover nền `ghost_element_hover`, khi có focus thì nền `element_selected`;
  - icon 14 px màu `icon_muted`, nhãn cỡ `UI_FONT_SIZE`, cắt bớt nếu dài;
  - phím ở bên phải, cỡ `UI_FONT_SIZE_STATUS`, màu `text_muted`.
  - Dòng gần đây có thêm mô tả (đường dẫn cha, màu muted, cắt bớt) và nút × (`icon_button` với `CLOSE_ICON`), chỉ hiện khi hover hoặc khi dòng có focus.
- Mục RECENT có tối đa 5 dòng. Khi danh sách rỗng thì không vẽ section này (Zed cũng ẩn). Dòng "More…" mở picker.

### 4.2 Hành vi

- **Bàn phím:**
  - ↑/↓ và Tab/⇧Tab chuyển focus giữa các nút; Enter bấm nút đang focus; ⌘Enter mở cửa sổ mới.
  - Lúc đầu focus nằm ở nút "Open…".
  - ⌘1…⌘5 mở mục gần đây thứ 1–5 (Zed).
- **Phím toàn cục** trong cửa sổ Welcome đi qua keymap của người dùng (`AppState.keymap`), như workbench làm trong `keys.rs`. Chỉ các lệnh trong §6 có tác dụng; phím khác bị bỏ qua.
- **Open… / Open File… / Open Folder…:** gọi `cx.prompt_for_paths`. Bấm Cancel thì vẫn ở Welcome, không thoát app. Đường dẫn chọn được đi qua `Router::open_path`, như yêu cầu mở từ Finder.
- **Bấm một mục gần đây:**
  - Nếu đường dẫn đã mất (bị xóa sau lúc nạp danh sách): gỡ mục khỏi danh sách và hiện một dòng `warning_row` "‹path› no longer exists" ở đầu cột.
  - Nếu không: mở như "Open…".
  - Bấm × thì `remove` và vẽ lại.
- **Mở thư mục từ Welcome:**
  - `watch_config(ConfigPaths::for_root(root))`;
  - `window.replace_root(...)` sang `Workbench`, cửa sổ giữ nguyên vị trí và kích thước;
  - `Router` cập nhật handle và `OpenWindow`;
  - `record`.
- **Danh sách nạp ở luồng nền** khi tạo view: `load` rồi `existing`, sau đó `cx.notify()`. Trong lúc đó mục RECENT chưa vẽ, nên không có khung trống nhấp nháy.
- **Thoát app:** đóng cửa sổ Welcome thì app thoát, như đóng cửa sổ cuối cùng hiện nay. ⌘Q vẫn hoạt động.

### 4.3 Cấu hình khi chưa có thư mục

`watch_config` hiện cần một root để đọc `<root>/.lucerna/settings.json`. Thêm `ConfigPaths::user_only()`, trong đó `workspace_settings: None`: loader bỏ qua lớp workspace và watcher chỉ theo dõi thư mục người dùng. Welcome cần cấu hình này để có theme, keymap và `startupEditor`. Khi đổi sang một thư mục, `watch_config` được gọi lại với `for_root`, thay global `ConfigState` như lúc khởi động.

## 5. `workbench.startupEditor`

Thêm vào `default-settings.json`:

```jsonc
// Controls which editor is shown at startup, if none are restored from the previous session.
// "welcomePage" | "welcomePageInEmptyWorkbench": the Welcome screen when Lucerna starts without a folder.
// "none": ask for a folder instead; Cancel quits.
"workbench.startupEditor": "welcomePage",
```

- `run(None, …)`: sau khi hết thời gian chờ yêu cầu mở từ Finder (300 ms, giữ nguyên), app đọc setting của người dùng. `none` thì gọi `pick_folder` như cũ; mọi giá trị khác thì mở cửa sổ Welcome.
- `welcomePage` và `welcomePageInEmptyWorkbench` cho cùng một kết quả, vì Lucerna không có tab Welcome trong cửa sổ có thư mục. `readme`, `newUntitledFile` và `terminal` chưa hỗ trợ nên được coi như `welcomePage`.
- Thứ tự đọc trước khi mở cửa sổ: `start` → `watch_config(user_only)` → đọc setting → mở Welcome hoặc hiện hộp thoại. Đọc setting là đọc một file nhỏ đồng bộ trước khi có cửa sổ, đúng như `watch_config` vẫn làm lúc khởi động.

## 6. Lệnh, phím, menu

| Lệnh | Tiêu đề (Command Palette) | Phím mặc định | Ở đâu |
|---|---|---|---|
| `workbench.action.openRecent` | File: Open Recent… | `ctrl+r` | Welcome, workbench |
| `workbench.action.files.openFileFolder` | File: Open… | `cmd+o` (macOS) | Welcome, workbench |
| `workbench.action.files.openFile` | File: Open File… | (Linux/Windows `ctrl+o`, khi có keymap riêng cho các nền tảng đó) | Welcome, workbench |
| `workbench.action.files.openFolder` | File: Open Folder… | (Linux/Windows `ctrl+k ctrl+o`) | Welcome, workbench |
| `workbench.action.clearRecentFiles` | File: Clear Recently Opened… | không có | Welcome, workbench, menu |
| `workbench.action.closeFolder` | File: Close Folder | `cmd+k f` | workbench |

- **Trong picker Open Recent** (khi `inRecentPicker` đúng):
  - `ctrl+r` chọn mục kế, `ctrl+shift+r` chọn mục trước;
  - `cmd+enter` mở cửa sổ mới; `escape` đóng picker (dùng `workbench.action.closeQuickOpen` sẵn có).
- **Chỗ sửa trong file dùng chung:**
  - `default-keybindings.json`: thêm 5 dòng (`ctrl+r`, `cmd+o`, `cmd+k f`, và 2 dòng điều hướng trong picker);
  - `default-settings.json`: thêm 2 khóa (`workbench.startupEditor`, `workbench.tips.enabled`);
  - `lib.rs`: thêm 3 dòng `mod`.
  - `cmd+k f` không đụng các chord `cmd+k` đã có (`cmd+k cmd+t`, `cmd+k f12`, `cmd+k s`, `cmd+k cmd+i`).
- **Va chạm với Vim:**
  - Vim V1 không dùng `ctrl+r`.
  - Vim V2 sẽ dùng `<C-r>` để redo. `vim::dispatch` nhận phím trước keymap khi editor có focus, nên `ctrl+r` trong editor sẽ thành redo, ngoài editor vẫn mở Open Recent. VSCodeVim cũng làm vậy, nên không cần `when`.
- **Open… trong workbench:**
  - Chọn file: mở thành tab ghim trong cửa sổ hiện tại, kể cả file ở ngoài root (VS Code mở file trong cửa sổ hiện tại).
  - Chọn thư mục: thay cửa sổ, như W8.
- **Clear Recently Opened:** hỏi lại bằng `window.prompt` ("Do you want to clear all recently opened folders?", nút Clear / Cancel).
- **Menu File** (`install_app_menu`): Open… (⌘O), Open Recent ▸ (10 mục đầu của `existing()`, separator, More…, Clear Recently Opened…), separator, Close Folder.
  - Submenu được dựng lại sau mỗi `record`, `remove` hoặc `clear` trong tiến trình. Danh sách cho menu được nạp ở luồng nền rồi gọi `cx.set_menus`.
  - Trong cửa sổ Welcome thì Close Folder bị tắt.

## 7. Picker Open Recent (`recent_picker.rs`)

`RecentPicker` là một entity giống `ThemePicker`. Nó được hiện trong `popover_overlay`, cả ở Welcome lẫn workbench. Workbench giữ nó như `theme_picker`: thêm vào `other_modal_overlay_shown` và vào các chỗ kiểm tra `theme_picker.is_some()` (phím, focus, `inQuickOpen`) để nó hoạt động như các picker khác.

```
┌──────────────────────────────────────────────────────┐
│ Select to open (⌘Enter for a new window)             │  picker_input_row, placeholder
├──────────────────────────────────────────────────────┤
│ FOLDERS                                              │  subheader, chỉ khi có cả file
│ ▢ lucerna   ~/Projects.nosync                        │  dòng chọn: element_selected
│ ▢ zed       ~/src                         ⧉  ×       │  hover: nút "Open in New Window" và ×
│ FILES                                                │
│ ▤ notes.md  ~/Desktop                                │
└──────────────────────────────────────────────────────┘
```

- Mỗi dòng là `picker_row` với icon thư mục hoặc file, tên được tô ký tự khớp (`highlighted`), và đường dẫn cha màu muted.
- Lọc mờ trên chuỗi `"{name} {parent}"`, nên gõ tên thư mục cha cũng khớp (VS Code `matchOnDescription`). Khi đang lọc thì bỏ header nhóm và xếp theo điểm, như Quick Open.
- Hai nút chỉ hiện khi hover:
  - "Open in New Window" (icon mũi tên ra ngoài, tooltip có ⌘Enter);
  - × "Remove from Recently Opened", gọi `remove` rồi bỏ dòng mà picker vẫn mở.
- Danh sách rỗng: một dòng muted "No recently opened folders".
- Đường dẫn đã mất lúc chọn: xử lý như ở Welcome. Trong workbench, thông báo đi qua `status_message`.

## 8. Watermark vùng editor trống

Viết lại `empty_hint` trong `editor_groups/view.rs`, giữ debug selector `editor-empty-hint`.

- Bỏ chữ "Lucerna" to.
- Thay bằng một lưới hai cột căn giữa: nhãn căn phải, phím căn trái, cách nhau `3×GRID`, chữ cỡ `UI_FONT_SIZE`, màu `text_muted`. Mỗi dòng gồm:
  - Show All Commands — `workbench.action.showCommands`;
  - Go to File — `workbench.action.quickOpen`;
  - Find in Files — `workbench.action.findInFiles`;
  - Toggle Sidebar — `workbench.action.toggleSidebarVisibility`;
  - Open Recent — `workbench.action.openRecent`.
- Lệnh nào không có phím trong keymap người dùng thì bỏ dòng đó, như hiện nay.
- Thêm setting `"workbench.tips.enabled": true`. Khi là `false`, vùng trống chỉ còn nền.

## 9. Kiến trúc mã

| File | Thay đổi |
|---|---|
| `recent.rs` (mới) | `RecentList`, `RecentEntry`, `load`, `existing`, `record`, `remove`, `clear`, `RECENT_LIMIT`. Không dùng gpui. |
| `welcome.rs` (mới) | `WelcomeView`: render (§4), xử lý phím, nạp danh sách ở luồng nền, hiện `RecentPicker`. |
| `recent_picker.rs` (mới) | `RecentPicker` và `RecentPickerEvent::{Open { entry, new_window }, Dismissed}`. |
| `ui.rs` | Thêm `section_header`, `list_button`. |
| `app.rs` | Đổi `Router.window` thành enum `Shown::{Welcome(handle), Workbench(OpenWindow, handle)}`. Thêm `Router::open_welcome`, `Router::open_in_welcome(path)` (đổi root), gọi `record` sau khi mở, đọc `startupEditor`, thêm menu File. Router là `Rc<RefCell<…>>`, được lưu thành global (`RouterHandle`) để workbench gọi được. |
| `open_requests.rs` | `spawn_process` trả `io::Result<()>` để W8 không đóng cửa sổ khi khởi động tiến trình mới thất bại. Thêm `spawn_welcome_process()` (`cwd = /`, không đối số). |
| `workbench.rs` | Đăng ký lệnh ở §6 qua `register_commands` của module mới (như `theme_picker::register_commands`). Thêm `replace_with(path)` cho W8: `confirm_quit` → `save_workspace_state` → spawn → `window.remove_window()`. |
| `config_files.rs` | `ConfigPaths.workspace_settings: Option<PathBuf>` và `ConfigPaths::user_only()`. |
| `workspace_state.rs` | Tách `write_atomic`; thêm hàm dùng chung rút gọn `~`. |
| `editor_groups/view.rs` | `empty_hint` theo §8. |
| `default-settings.json`, `default-keybindings.json` | Thêm như §5, §6. |

Các phần không đụng tới: `lucerna-editor`, `save.rs` (chỉ gọi `confirm_quit` sẵn có), thanh xung đột, `merge`.

## 10. Xử lý lỗi

- `recent.json` hỏng thì đổi thành `.bak` và dùng danh sách rỗng. Ghi lỗi thì `tracing::warn` và bỏ qua, vì danh sách gần đây không bao giờ được chặn việc mở thư mục.
- Không có `HOME` (`default_data_dir()` là `None`) thì không có danh sách gần đây: Welcome chỉ còn Get Started, và mọi thao tác ghi đều bỏ qua.
- `prompt_for_paths` lỗi thì `tracing::error` và vẫn ở màn hiện tại.
- Khởi động tiến trình mới thất bại (W8, W10) thì giữ cửa sổ hiện tại và báo qua `status_message`.
- Hộp thoại hỏi lưu bị Cancel (W8, W10) thì không làm gì.

## 11. Kiểm thử

- **Unit (`recent.rs`):**
  - `record` đặt mục lên đầu, bỏ trùng, cắt ở 100;
  - `remove`, `clear`;
  - `existing` bỏ thư mục mất và mục sai loại (file thay cho thư mục);
  - ghi lại thì bỏ mục mất;
  - file hỏng hoặc khác version thì thành `.bak` và danh sách rỗng;
  - ghi nguyên tử (không còn `.tmp`).
- **GPUI test (`TestAppContext`):**
  - Welcome vẽ đúng 5 mục đầu cùng "More…", ẩn RECENT khi danh sách rỗng, bỏ mục mất;
  - × gỡ mục; ⌘1 mở mục đầu (kiểm tra qua callback mở giả của Router);
  - picker lọc theo tên và đường dẫn cha; `ctrl+r` chọn mục kế; ⌘Enter phát `Open { new_window: true }`; Escape đóng picker;
  - watermark hiện nhãn theo keymap và ẩn khi `workbench.tips.enabled` là `false`;
  - `startupEditor: "none"` chọn hộp thoại, giá trị khác chọn Welcome (tách logic chọn thành một hàm thuần để test).
- **Thủ công (bắt buộc trước khi coi là xong, theo design-principles §1.1):** chạy `Lucerna.app` từ Dock và đặt cạnh Zed khi chưa mở project để so khoảng cách, cỡ chữ, hover và trạng thái trống. Sau đó thử ⌃R trong workbench: Enter thay cửa sổ, ⌘Enter mở cửa sổ mới, Close Folder quay về Welcome.
