# Màn onboarding lần chạy đầu ("Get Started")

Ngày 2026-11-15. Trạng thái: người dùng đã duyệt (2026-11-15), chưa triển khai.

Spec này sửa mục W12 của `2026-11-12-welcome-design.md`: Welcome vẫn không có AI, đăng nhập, New File hay Clone Repository, nhưng **Lucerna có onboarding**, là một trang riêng chứ không phải một phần của Welcome.

## 1. Ý định

Zed có một trang onboarding riêng hiện ở lần chạy đầu, cho chọn theme và Vim. Lucerna sắp ra 0.1, và người dùng muốn người mới có một chỗ chỉnh vài thứ cơ bản ngay khi mở app: theme, font, Vim. Hiện nay người mới phải tự tìm `settings.json` hoặc Command Palette mới làm được.

**Mục tiêu:**
- Lần đầu mở Lucerna (Dock, Finder, `luc` không đối số) thì hiện trang "Get Started", chỉnh được theme Sáng / Tối / Theo hệ thống, font và cỡ chữ editor, Vim mode, và có nút mở `settings.json`.
- Mỗi thay đổi được ghi ngay vào `settings.json` của người dùng và thấy kết quả ngay.
- Trang chỉ tự hiện một lần. Mở lại được bằng lệnh, từ menu Help, và từ Welcome.
- Hợp với `window.restoreWindows`: người đã có workspace để mở lại thì không bị chặn bởi onboarding.

**Không làm:** chọn keymap gốc (Lucerna chỉ theo phím VS Code), AI, đăng nhập, telemetry, import settings từ editor khác, font UI, chọn họ theme (Lucerna mới có một cặp theme mặc định).

**Thất bại dù vẫn chạy được, nếu:**
- trang hiện lại ở mỗi lần khởi động, hoặc chen vào trước workspace người dùng muốn mở lại;
- đổi setting trên trang mà làm mất comment hay định dạng trong `settings.json`;
- trang hiện giá trị khác với setting thật (ví dụ người dùng đã sửa tay file).

## 2. Tham khảo

Chi tiết ở `docs/research/2026-11-15-onboarding.md`. Tóm tắt:
- **Zed:** onboarding là một tab trong workspace trống, một trang cuộn (từ bản 0.208), cột tối đa 780 px. Đầu trang có logo, "Welcome to Zed" và nút "Finish Setup" (`cmd-enter`), bấm thì tab được thay bằng Welcome. Các mục: Theme (toggle Light/Dark/System và ba ô xem trước vẽ bằng màu theme), Base Keymap, AI, Import Settings, Vim Mode, Trust, Telemetry. Mỗi control ghi ngay vào `settings.json`, không có Cancel. Tự hiện khi không có gì để khôi phục và khóa `first_open` chưa có; khóa được ghi ngay lúc trang mở. Mở kèm đường dẫn thì không hiện. Mở lại bằng `zed: open onboarding`; Welcome có nút "Return to Onboarding".
- **VS Code:** walkthrough "Get started with VS Code" trên trang Welcome có bước chọn theme bằng thẻ ảnh. Overlay onboarding mới là modal phủ Welcome, chỉ hiện khi storage ứng dụng mới. Chọn theme khi `window.autoDetectColorScheme` bật thì ghi vào khóa theme ưa thích theo chế độ hệ thống.
- **Lucerna chọn:** điều kiện "lần đầu", cách ghi settings ngay, nút Finish và bố cục trang theo Zed; tên setting và lệnh mở settings theo VS Code. Lệch khỏi Zed:
  - chỉ một ô xem trước theme thay vì ba, vì Lucerna mới có một cặp theme;
  - có phần font editor (bản Zed cũ có, bản mới bỏ), theo yêu cầu người dùng;
  - có mục Help ▸ Get Started (Zed không có), để người dùng VS Code tìm được.

## 3. Quyết định

| # | Quyết định | Lý do |
|---|---|---|
| O1 | Trang là một view `OnboardingPage` dùng được ở **hai chỗ**: (a) làm **root view** của cửa sổ chưa mở thư mục, như Welcome; (b) làm **tab** `TabItem::Onboarding` trong workbench. Phần vẽ nội dung là một. | Người dùng chọn (cách A). Zed để trang trong tab; Lucerna không có workbench rỗng nên ở cửa sổ chưa mở thư mục trang phải là root, như Welcome (W1). |
| O2 | Tự hiện khi khởi động **không có đường dẫn** (không đối số, không yêu cầu mở từ Finder trong thời gian chờ), **không có workspace để mở lại** (`window.restoreWindows` là `"none"` hoặc không có mục gần đây còn tồn tại), và **chưa có cờ** `onboarded`. | Theo Zed. Người cũ có workspace để mở lại thì không bị chặn. |
| O3 | Cờ `onboarded: true` được ghi **ngay khi trang tự hiện**, không chờ bấm Finish. | Theo Zed: đóng cửa sổ hay thoát app thì lần sau không hiện lại. |
| O4 | Onboarding đứng trước `workbench.startupEditor` ở lần đầu: khi O2 đúng thì hiện onboarding kể cả khi `startupEditor` là `"none"`. | Lần đầu thường chưa có settings. Người đặt `"none"` trước khi chạy lần đầu chỉ thấy trang một lần. |
| O5 | Finish (`⌘Enter`): ở root thì đổi root sang Welcome (`window.replace_root`, như W1); ở tab thì đóng tab. | Zed thay trang bằng Welcome. Trong workbench Lucerna không có tab Welcome, nên đóng tab. |
| O6 | Mỗi control ghi ngay vào `settings.json` người dùng, giữ comment (hàm sẵn có trong `theme_picker.rs`), rồi nạp lại cấu hình. Trang vẽ theo setting hiện hành sau khi gộp các lớp. Không có Undo/Cancel. | Theo Zed. Sửa tay file thì trang đổi theo. |
| O7 | Theme: cụm ba nút **Light / Dark / System**. System ghi `window.autoDetectColorScheme: true`. Light/Dark ghi `window.autoDetectColorScheme: false` và `workbench.colorTheme` bằng giá trị đang có của `workbench.preferredLightColorTheme` / `workbench.preferredDarkColorTheme` (mặc định "Lucerna Light" / "Lucerna Dark"). | Khóa theo VS Code, như người dùng yêu cầu. Giữ theme ưa thích người dùng đã đặt. |
| O8 | Lệnh `lucerna.openOnboarding`, tiêu đề "Help: Get Started", không có phím mặc định. Có trong menu Help ("Get Started", đứng đầu) và là một nút "Get Started" trên Welcome. | VS Code không có lệnh tương ứng (onboarding mới của nó ở nhóm Developer), nên dùng tiền tố `lucerna.` như `lucerna.openLogsFolder`. Zed có nút "Return to Onboarding" trên Welcome. |
| O9 | Font family chọn bằng picker có lọc mờ, liệt kê font trên máy (`cx.text_system().all_font_names()`), Lilex đứng đầu. Font size bằng nút − / + từ 6 tới 100, bước 1. | Gõ tên font tay dễ sai. Khung picker đã có. |
| O10 | Vim mode là một **công tắc** (switch) ghi `vim.enabled`. | Zed dùng switch. |
| O11 | Không chọn keymap gốc, AI, đăng nhập, telemetry, import, font UI. | Phạm vi người dùng đã chốt. |

**Thành công khi:**
- Xóa `<data_dir>` (hoặc dùng một `HOME` mới), mở `Lucerna.app` từ Dock: hiện trang Get Started chiếm cả cửa sổ. Bấm Dark: app đổi sang Lucerna Dark ngay, `settings.json` có `"window.autoDetectColorScheme": false` và `"workbench.colorTheme": "Lucerna Dark"`, comment cũ còn nguyên.
- Bấm + ở Font size: dòng mẫu font trên trang và editor ở các cửa sổ khác to lên ngay, file có `"editor.fontSize": 15`.
- Bật Vim mode: file có `"vim.enabled": true`; mở một thư mục thì editor ở Normal mode.
- ⌘Enter: cửa sổ đổi sang Welcome, không có cửa sổ thứ hai.
- Thoát rồi mở lại từ Dock khi chưa mở thư mục nào: hiện Welcome, không hiện onboarding.
- Đã mở một thư mục, thoát, mở lại từ Dock: mở lại thư mục đó (restoreWindows), không hiện onboarding.
- `luc ~/code` ở lần chạy đầu: mở thư mục, không hiện onboarding, cờ chưa ghi.
- Trong workbench, chạy "Help: Get Started": mở tab Get Started; chạy lần nữa thì nhảy về tab đó. ⌘Enter đóng tab.
- Sửa tay `"editor.fontSize": 20` trong `settings.json` khi trang đang mở: ô cỡ chữ hiện 20.

## 4. Lần đầu và luồng khởi động

`app.rs` `open_without_folder` hiện làm: đọc cấu hình người dùng → nếu `restores_last_workspace` và có `last_workspace` thì mở nó → nếu không thì `startup_choice` (Welcome hoặc hộp chọn thư mục). Thêm một bước:

```
open_without_folder:
  watch_config(user_only)
  nếu restores_last_workspace && có last_workspace → mở nó; return
  nếu !onboarded(data_dir)                         → mark_onboarded(data_dir); Router::open_onboarding; return
  startup_choice → FolderPrompt | Welcome
```

- **Cờ** nằm ở `<data_dir>/state.json`: `{ "version": 1, "onboarded": true }`. Module thuần `app_state_file.rs` (không gpui) có `load(path) -> AppFlags`, `mark_onboarded(data_dir)`. File không có hoặc thiếu khóa thì `onboarded = false`. File hỏng hoặc khác version thì đổi tên thành `.bak` như `recent.json`, coi là `onboarded = false`. Ghi bằng `write_atomic` sẵn có. Đọc và ghi đều ở luồng nền như `recent.rs`, trừ lần đọc lúc khởi động (một file nhỏ, đọc đồng bộ trước khi có cửa sổ, như `recent::load` ở `open_without_folder`).
- Không có `HOME` (`default_data_dir()` là `None`): không hiện onboarding, đi thẳng `startup_choice`. Không có chỗ ghi cờ thì hiện mỗi lần sẽ gây phiền.
- Ghi cờ lỗi: `tracing::warn` và vẫn hiện trang. Lần sau có thể hiện lại; chấp nhận được.
- Các đường khác (`luc <path>`, Open With, `open_urls`) không đổi: không xét onboarding và không ghi cờ.
- Chọn root lúc khởi động tách thành hàm thuần để test: `startup_screen(restore: Option<&RecentEntry>, onboarded: bool, has_data_dir: bool, choice: Startup) -> StartupScreen::{Restore(path), Onboarding, Welcome, FolderPrompt}`. Hàm `restores_last_workspace`, `last_workspace`, `startup_choice` trong `startup.rs` giữ nguyên; hàm mới nằm cạnh chúng.

## 5. Trang Get Started

### 5.1 Bố cục (theo Zed `basics_page.rs`, bỏ các mục ở O11)

```
┌──────────────────────────────────────────────────────────────┐
│ ● ● ●                       Lucerna                          │  title bar 34 px (chỉ khi là root)
├──────────────────────────────────────────────────────────────┤
│      [logo] Welcome to Lucerna                 [ Finish  ⌘↵ ]│  header
│             Set up the basics. Change them any time.         │  muted
│      ─────────────────────────────────────────────────────── │  đường kẻ border_variant
│      THEME ───────────────────────────── [Light|Dark|System] │  section_header + segmented
│      ┌──────────────────────────────┐                        │
│      │ ▌▬▬  ▬▬▬▬▬                    │  theme_preview          │
│      │ ▌▬   ▬▬ ▬▬▬▬                  │  (System: chia đôi)     │
│      └──────────────────────────────┘                        │
│      Lucerna Dark                                            │  tên theme, muted
│      EDITOR ─────────────────────────────────────────────────│
│      Font Family                          [ Lilex        ▾ ] │  setting_row + dropdown_button
│      Font Size                            [ −   14   + ]     │  setting_row + number_stepper
│      Vim Mode                                         ( ●)   │  setting_row + switch
│      Enable Vim keybindings (vim.enabled)                    │  mô tả muted dưới nhãn
│      ─────────────────────────────────────────────────────── │
│      [⚙] Open Settings (JSON)                       ⌘,       │  list_button
└──────────────────────────────────────────────────────────────┘
```

- Nền `editor_background`, không có status bar khi là root. Cột rộng tối đa **640 px** (giữa 512 px của Welcome và 780 px của Zed, vì Lucerna bỏ bớt mục), căn giữa ngang, padding `8×GRID`, các khối cách nhau `6×GRID`, cuộn dọc khi cửa sổ thấp (thanh cuộn chung của Lucerna).
- Tiêu đề và logo dùng lại cách vẽ của Welcome (`mark-mono.svg`, cỡ 45 px, tiêu đề 20 px). Dòng phụ cỡ `UI_FONT_SIZE_SMALL`, màu `text_muted`.
- Nút Finish dùng `primary_text_button` với phím hiển thị bên phải.
- `section_header` và `list_button` dùng lại từ `ui.rs`.
- Nút "Open Settings (JSON)" hiện phím của `workbench.action.openSettingsJson` theo keymap người dùng.

### 5.2 Component mới trong `ui.rs`

Đặt tên và kích thước theo các component sẵn có (`GRID`, màu theme):
- **`segmented_control(id, options, selected, cx)`**: các nút liền nhau, viền 1 px `border`, bo `GRID`, cao `6×GRID`. Nút chọn nền `element_selected`, nút khác hover `ghost_element_hover`. Trả từng nút có `on_click`.
- **`switch(id, on, cx)`**: rộng `7×GRID`, cao `4×GRID`, bo tròn. Bật: nền `element_selected` (hoặc màu nhấn của theme nếu có), núm bên phải. Tắt: nền `element_background`, núm bên trái.
- **`number_stepper(id, value, cx)`**: nút − / + (`icon_button`) hai bên con số; nút mờ khi chạm giới hạn.
- **`dropdown_button(id, label, cx)`**: ô viền như `input_box`, nhãn và `CHEVRON_DOWN_ICON` bên phải.
- **`setting_row(label, description, control)`**: nhãn cỡ `UI_FONT_SIZE`, mô tả cỡ `UI_FONT_SIZE_SMALL` màu `text_muted` dưới nhãn, control căn phải.
- **`theme_preview(theme, cx)`**: vẽ một workspace thu nhỏ bằng màu của theme truyền vào (không dùng theme đang active): sidebar ~22% có 6 dòng, vùng editor có ~8 thanh "mã giả" tô bằng màu cú pháp (keyword, function, string, comment, type, variable). Độ rộng và thụt lề của thanh là bảng cố định, không ngẫu nhiên. Cao `30×GRID`, bo 8 px, viền 1 px. Ở System thì vẽ hai nửa: trái là theme sáng, phải là theme tối.

Kiểm tra trong `ui.rs` trước khi tạo: nếu đã có component tương đương thì dùng lại.

### 5.3 Theme

- **Trạng thái đang chọn:** `window.autoDetectColorScheme` đúng thì System; sai thì Light hoặc Dark theo appearance của theme `workbench.colorTheme`.
- **Bấm** (O7):
  - System: ghi `window.autoDetectColorScheme: true`.
  - Light: ghi `window.autoDetectColorScheme: false` và `workbench.colorTheme` = giá trị của `workbench.preferredLightColorTheme` đọc **trực tiếp** từ store (không qua `configured_theme_name`, vì hàm đó trả `workbench.colorTheme` khi autodetect tắt), không có thì "Lucerna Light".
  - Dark: tương tự với `preferredDarkColorTheme` / "Lucerna Dark".
  - Bấm nút đang chọn thì không ghi gì.
- **Ô xem trước** vẽ theme sẽ dùng với lựa chọn hiện tại: Light/Dark là `workbench.colorTheme`; System vẽ hai nửa preferred Light và preferred Dark. Dưới ô là tên theme (System: "Lucerna Light / Lucerna Dark").
- Theme có tên không tìm thấy thì `get_or_default` như `apply_configured_theme`.
- **Ghi hai khóa một lần:** viết `set_settings(text, &[(key, serde_json::Value)])` trong `theme_picker.rs` (tách thành `settings_edit.rs` nếu cần) dựa trên `set_string_setting`, mở rộng sang giá trị bool và số, và ghi file một lần để watcher chỉ thấy một thay đổi. `set_string_setting` gọi hàm mới.

### 5.4 Editor

- **Font Family:** nhãn của dropdown là `editor.fontFamily` hiện hành (không có thì "Lilex"). Bấm thì mở `FontPicker` trong `popover_overlay`, dựng như `ThemePicker`/`RecentPicker` (`popover`, `picker_input_row`, `picker_list`, `picker_row`, `fuzzy_filter`):
  - danh sách: "Lilex" đầu tiên, rồi `all_font_names()` đã bỏ trùng, xếp chữ cái; lấy danh sách ở luồng nền;
  - mỗi dòng vẽ tên bằng chính font đó;
  - ↑/↓ chỉ chuyển dòng chọn, **không xem trước** trên editor (store không có lớp tạm trong bộ nhớ; mỗi dòng đã vẽ bằng font của nó). Enter ghi `editor.fontFamily`, Escape đóng mà không ghi.
  - Giá trị là chuỗi đơn giản (một tên font), không dựng chuỗi CSS nhiều font như VS Code.
- **Font Size:** hiện `editor.fontSize` hiện hành (làm tròn). − / + ghi giá trị mới, giới hạn 6–100 (VS Code chấp nhận 6–100).
- **Vim Mode:** công tắc theo `vim.enabled`, bấm thì ghi giá trị đảo. Các editor đang mở đổi theo cách sẵn có khi `vim.enabled` thay đổi.
- **Dòng mẫu font:** dưới ba dòng trên là một ô `subheader_card` có hai dòng code mẫu (ví dụ `fn main() {` / `    println!("Hello, Lucerna");`) vẽ bằng `editor.fontFamily` và `editor.fontSize` hiện hành, màu cú pháp của theme đang active. Đây là phản hồi duy nhất cho font khi trang là root (chưa có editor nào mở).
- Các thay đổi này chỉ ghi lớp người dùng. Nếu lớp workspace (`.lucerna/settings.json`) hoặc override ngôn ngữ đặt khóa đó thì trang hiện giá trị đã gộp, và giá trị vừa ghi có thể không thấy hiệu lực. Spec này **không** báo điều đó trên trang, vì `SettingsStore` không cho biết lớp nào đặt khóa và spec chỉ đổi `lucerna-workbench`. Ghi vào giới hạn đã biết (§10).

### 5.5 Open Settings (JSON)

- Ở tab: chạy `workbench.action.openSettingsJson` như Command Palette.
- Ở root (chưa có workbench): mở `settings.json` người dùng ở **cửa sổ mới** (`luc <user settings path>`, như "cửa sổ mới" của W7), để trang onboarding vẫn còn. File chưa tồn tại thì tạo bằng `USER_SETTINGS_TEMPLATE` trước.

### 5.6 Bàn phím

- Thứ tự focus: Finish → Light → Dark → System → Font Family → − → + → Vim Mode → Open Settings (JSON). Tab/⇧Tab và ↑/↓ chuyển focus; Enter hoặc Space bấm control đang focus; ←/→ trong cụm theme chuyển lựa chọn.
- Focus ban đầu: nút theme đang chọn.
- `⌘Enter` (lệnh `lucerna.onboarding.finish`, `when: onboardingFocus`) chạy Finish ở cả hai chế độ. `onboardingFocus` đúng khi focus nằm trong trang, **không** tính lúc FontPicker đang mở: khi đó phím đi vào picker trước (như picker ở Welcome), và ⌘Enter trong picker không làm gì. Binding này không đụng `editor.action.insertLineAfter` (`editorTextFocus`) vì trang không có editor.
- Ở root: phím toàn cục đi qua keymap người dùng như Welcome (`WELCOME_COMMANDS`), thêm `lucerna.openOnboarding`, `workbench.action.openSettingsJson`, `lucerna.onboarding.finish`.
- Ở tab: phím của workbench chạy như ở các tab khác (⌘W đóng tab).

## 6. Mở lại

- **Lệnh** `lucerna.openOnboarding`:
  - Trong workbench: mở hoặc kích hoạt tab `TabItem::Onboarding` (chỉ một bản trong cửa sổ, mở ở group đang active).
  - Ở Welcome: đổi root sang onboarding.
  - Ở onboarding root: không làm gì.
- **Menu Help:** thêm "Get Started" (`lucerna.openOnboarding`) đứng đầu menu Help, sau đó là separator và các mục cũ. Cửa sổ Welcome bật mục này (thêm vào danh sách mục Welcome xử lý được trong `build_menus`).
- **Welcome:** thêm nút "Get Started" vào GET STARTED, sau "Open Recent…", icon riêng (ví dụ `sparkles` hoặc `settings` từ Lucide nếu đã có trong `icons`), phím theo keymap.
- **Tab:** tiêu đề "Get Started", không có dấu chưa lưu, không có path bar. Tab **không** được lưu vào workspace state; mở lại thư mục thì không có tab này.

## 7. Kiến trúc mã

| File | Thay đổi |
|---|---|
| `onboarding.rs` (mới) | `OnboardingPage` (render §5, focus, phím), `OnboardingEvent::{Finish}`, `register_commands`. Biết mình là root hay tab qua một cờ `Host::{Window, Tab}` để vẽ title bar và chọn hành vi Open Settings. |
| `font_picker.rs` (mới) | `FontPicker`, `FontPickerEvent::{Confirmed(name), Dismissed}`. |
| `app_state_file.rs` (mới) | Đọc/ghi `<data_dir>/state.json` (§4). Không gpui. |
| `startup.rs` | Thêm `StartupScreen` và `startup_screen` (§4). |
| `app.rs` | `Shown::Onboarding(WindowHandle<OnboardingPage>)`; `Router::open_onboarding`, đổi root Welcome ↔ Onboarding; `open_without_folder` theo §4. |
| `welcome.rs` | Nút "Get Started"; `WelcomeEvent::OpenOnboarding`. |
| `editor_groups/model.rs`, `editor_groups/view.rs` | `TabItem::Onboarding` (singleton như preview động), vẽ `OnboardingPage` trong pane. |
| `workbench.rs` | Đăng ký lệnh; không lưu tab Onboarding vào workspace state. |
| `app_menu.rs` | Mục Help ▸ Get Started; bật ở Welcome và Onboarding root. |
| `ui.rs` | Các component ở §5.2. |
| `theme_picker.rs` (hoặc `settings_edit.rs`) | `set_settings` ghi nhiều khóa, giá trị JSON bất kỳ (§5.3). |
| `default-keybindings.json` | `cmd+enter` → `lucerna.onboarding.finish` khi `onboardingFocus`. |

Chỉ `lucerna-workbench` thay đổi; không thêm dependency.

## 8. Xử lý lỗi

- `settings.json` đọc không được hoặc không parse được: không ghi, báo lỗi cấu hình như theme picker (`record_error`), control giữ giá trị cũ.
- Chưa có `settings.json`: tạo từ `USER_SETTINGS_TEMPLATE` rồi ghi (như `write_string_setting`).
- `all_font_names()` rỗng: picker chỉ có "Lilex".
- Mở cửa sổ settings (§5.5) thất bại: `tracing::error`, hiện `warning_row` ở đầu cột.

## 9. Kiểm thử

- **Unit:**
  - `app_state_file`: không có file → chưa onboarded; ghi rồi đọc; file hỏng → `.bak`; ghi nguyên tử.
  - `startup_screen`: có restore → Restore; không restore, chưa onboarded → Onboarding; đã onboarded → theo `startupEditor`; không data dir → theo `startupEditor`; `startupEditor: none` lần đầu → Onboarding.
  - `set_settings`: ghi bool, số, chuỗi; nhiều khóa một lần; giữ comment; thay khóa có sẵn; file rỗng.
  - Theme: ánh xạ Light/Dark/System ra các khóa ghi (theo O7), với và không có preferred theme người dùng đặt; trạng thái chọn đọc từ setting.
- **GPUI test:**
  - Trang vẽ đúng giá trị từ setting; sửa layer người dùng thì trang vẽ lại.
  - Bấm Dark/System/+/Vim thì file người dùng có khóa đúng.
  - ⌘Enter ở root phát Finish (Router đổi sang Welcome); ở tab thì đóng tab.
  - `lucerna.openOnboarding` trong workbench mở một tab, gọi lần hai không mở thêm.
  - FontPicker: lọc, Enter ghi, Escape trả lại font cũ.
  - Help menu có Get Started, bật ở cửa sổ Welcome.
- **Thủ công (design-principles §1.1):** chạy app với `HOME` mới, đặt cạnh Zed (`zed: open onboarding`) so khoảng cách, cỡ chữ, nút, hover, switch. Thử chuỗi lần đầu → Finish → thoát → mở lại (Welcome) → mở thư mục → thoát → mở lại (thư mục).

## 10. Giới hạn đã biết

- Không báo khi workspace hoặc override ngôn ngữ đè setting trang vừa ghi (§5.4).
- Font picker không xem trước trên editor.
- Người dùng nâng cấp từ bản trước 0.1: chưa có `state.json`, nên nếu khi khởi động không có gì để mở lại (`restoreWindows: "none"` hoặc mọi mục gần đây đã mất) thì họ thấy onboarding **một lần**, dù đã có `settings.json`. Zed cũng vậy. Chấp nhận, vì Lucerna chưa phát hành và chỉ hiện một lần.

## 11. Để người làm quyết

- Icon cụ thể cho nút Get Started.
- Bảng thanh "mã giả" trong ô xem trước, miễn trông giống code và dùng đủ màu cú pháp.
- Dòng phụ dưới tiêu đề có thể chỉnh chữ.
