# Nghiên cứu: màn onboarding lần chạy đầu

Ngày 2026-11-15. Đọc Zed `main` và VS Code `main` (lấy ngày 2026-11-15). Zed là GPL: chỉ ghi thiết kế, không chép code. Số dòng là ước chừng vì cả hai repo đổi liên tục.

## 1. Zed

### 1.1 Khi nào tự hiện

`crates/zed/src/main.rs`, `restore_or_create_workspace`:

1. Có workspace khôi phục được (theo `restore_on_startup`: mặc định `last_session`, hoặc `last_workspace`) thì khôi phục, **không** hiện onboarding.
2. Không có gì để khôi phục và khóa KV `first_open` chưa có thì gọi `onboarding::show_onboarding_view`.
3. Còn lại thì mở workspace trống.

- `show_onboarding_view` ghi `first_open = "false"` **ngay khi mở màn**, không phải khi người dùng bấm xong. Đóng tab, bỏ qua hay thoát app đều không làm màn tự hiện lại.
- Thoát khi tab còn mở: tab được serialize (`OnboardingPage`) nên mở lại cùng workspace.
- Mở kèm đường dẫn (`zed <path>`, Open With) đi đường `open_startup_request`, bỏ qua bước trên, nên không hiện onboarding. Khóa vẫn chưa ghi, nên màn có thể hiện ở một lần mở trần sau đó, nếu khi ấy không có gì để khôi phục.

### 1.2 Hình thức và bố cục

- Là **một tab** (workspace `Item`, tiêu đề "Onboarding") trong một workspace trống mới, không phải modal hay cửa sổ riêng.
- Từ bản 0.208 (tháng 10/2025, PR #39724) là **một trang cuộn duy nhất**. Trước đó có nav trái 220 px với ba trang Basics / Editing / AI Setup (`cmd-1/2/3`).
- Cột rộng tối đa 780 px, căn giữa, padding `p_12`, khoảng cách `gap_6`, nền `editor_background`, có thanh cuộn dọc.
- Đầu trang: logo, "Welcome to Zed", dòng phụ muted in nghiêng. Bên phải đầu trang là nút đầy "Finish Setup" (rộng 200 px, có phím `cmd-enter`). Sau đó là một đường kẻ.
- Phím (context `Onboarding`): `cmd-enter` Finish; `cmd-=`/`cmd--`/`cmd-0` đổi cỡ chữ UI không lưu; `menu::SelectNext`/`SelectPrevious` chuyển focus giữa các control (mỗi control có `tab_index`).
- Finish: thay tab Onboarding bằng tab Welcome ở cùng vị trí.

### 1.3 Nội dung (theo thứ tự)

1. **Theme.** Nhãn "Theme" và nhóm toggle Light / Dark / System bên phải. Dưới là ba ô xem trước theo họ theme (One, Ayu, Gruvbox).
   - Ô không phải ảnh chụp mà là bộ khung một workspace thu nhỏ vẽ bằng màu của theme đó: sidebar ~20–25% có vài dòng, pane có các thanh "mã giả" tô màu cú pháp (keyword, function, string, comment…). Độ rộng, thụt lề và màu chọn tất định.
   - Chế độ System vẽ mỗi ô chia đôi sáng/tối.
   - Ô đang chọn có viền `border_selected`; ô khác mờ 80%, hover thì có viền. Tên họ theme nằm dưới ô. Bo 8 px, viền 2 px.
2. **Base Keymap:** Zed, VS Code, JetBrains, Sublime/Atom, Emacs, Cursor, TextMate.
3. **Agent Setup** (AI).
4. **Import Settings** từ VS Code / Cursor.
5. **Vim Mode** (switch).
6. **Trust All Projects By Default** (switch).
7. Telemetry: metrics, crash reports.

Bản cũ có trang Editing: font UI và font buffer (picker họ font, stepper cỡ chữ), ligatures, format on save, inlay hints, git blame, minimap. Bản hiện tại đã bỏ phần font.

### 1.4 Cách ghi settings

- Mỗi control ghi **ngay** vào `settings.json` của người dùng (`update_settings_file`). View theo dõi `SettingsStore` nên luôn vẽ theo setting hiện hành, kể cả khi người dùng sửa tay ở nơi khác.
- Không có Undo/Cancel: Finish chỉ chuyển trang.
- Khóa: `theme: {mode, light, dark}`, `base_keymap`, `vim_mode`, `telemetry.*`, `session.trust_all_worktrees`; bản cũ có `buffer_font_family`, `buffer_font_size`, `ui_font_*`.
- Toggle minimap bản cũ áp giá trị vào bộ nhớ trước khi ghi file, vì chờ ghi file làm toggle bị trễ.

### 1.5 Khác trang Welcome

- Welcome (`WelcomePage`, `zed::ShowWelcome`) là trang "Get Started": New File, Open Project, Clone Repository, Command Palette, Configure…, và Recent Projects (`cmd-1`…`cmd-5`) ở chế độ launchpad. Welcome có nút "Return to Onboarding".
- Onboarding là trang cấu hình lần đầu. Mở lại bằng lệnh `zed: open onboarding` (`OpenOnboarding`). Menu Help của Zed chỉ có "Show Welcome", không có mục onboarding.

## 2. VS Code

### 2.1 Walkthrough trên trang Welcome

- `welcomeGettingStarted/common/gettingStartedContent.ts`. Walkthrough "Get started with VS Code" (id `Setup`) hiện gồm các bước Copilot, `pickColorTheme` và video. Các bước cũ (sync, extension, command palette…) chuyển sang "Learn the Fundamentals" (id `Beginner`).
- Bước `pickColorTheme`: bốn thẻ ảnh (Dark Modern, Light Modern, Dark/Light High Contrast) và link "See More Themes…" mở quick pick `workbench.action.selectTheme`. Bấm thẻ thì `setColorTheme(id, USER)`; thẻ được đánh dấu khi `workbench.colorTheme` bằng id của nó.
- Tiến độ mỗi bước lưu trong Memento `gettingStartedService` (`{done: true}` theo id bước). Bước xong theo sự kiện `onCommand:`, `onSettingChanged:`, `onContext:`…

### 2.2 Overlay onboarding mới

- `contrib/welcomeOnboarding/`, setting `workbench.welcomePage.experimentalOnboarding` (mặc định true): **modal** "Welcome to Visual Studio Code" phủ lên trang Welcome. Các bước: Sign In, "Make It Yours" (thẻ Color Theme vẽ bằng SVG, chọn keymap của editor đã cài trên máy).
- Chỉ hiện khi storage ứng dụng là mới (`isNew(APPLICATION)`), không phải web, AI không bị ẩn, và khóa `welcomeOnboarding.state` chưa có (ghi khi đóng). Esc bỏ qua; Tab bị giữ trong overlay; phím tắt không tới keybinding service.
- Mở lại bằng `workbench.action.welcomeOnboarding2026` (nhóm Developer).

### 2.3 Khi nào Welcome tự mở, lệnh, menu

- `browser/startupPage.ts`: sau khi khôi phục xong, không phải reload, không có `--skip-welcome`, không có editor nào đang mở; theo `workbench.startupEditor` (mặc định `welcomePage`).
- Trang Welcome cổ điển không có khái niệm "lần đầu"; nó chỉ theo `startupEditor`.
- Help ▸ Welcome (`workbench.action.openWalkthrough`), Help ▸ Open Walkthrough… (`welcome.showAllWalkthroughs`).

### 2.4 Setting theme và font

- `window.autoDetectColorScheme` (VS Code mặc định false; Lucerna mặc định true). Khi bật, `workbench.colorTheme` bị bỏ qua và chọn theme sẽ ghi vào `workbench.preferredDarkColorTheme` hoặc `workbench.preferredLightColorTheme` theo chế độ hệ thống hiện tại. Lucerna đã làm y vậy (`color_theme_setting_key`).
- Font: `editor.fontFamily`, `editor.fontSize`. Không luồng Welcome nào của VS Code đụng tới hai khóa này.

## 3. Giống và khác

| | Zed | VS Code | Lucerna chọn |
|---|---|---|---|
| Hình thức | Tab trong workspace trống | Modal phủ Welcome (mới); walkthrough trong tab Welcome (cũ) | Giao diện theo Zed (trang, không phải modal). Xem spec. |
| Điều kiện lần đầu | Không có gì để khôi phục và chưa có khóa `first_open`; ghi khóa khi mở | Storage ứng dụng mới; ghi khóa khi đóng | Theo Zed |
| Mở kèm đường dẫn | Không hiện | Overlay vẫn hiện trên cửa sổ | Theo Zed: không hiện |
| Ghi settings | Ghi ngay mỗi lần đổi, không Cancel | Ghi ngay (thẻ theme) | Ghi ngay |
| Theme | Light/Dark/System + ô xem trước vẽ bằng màu theme | Thẻ ảnh/SVG | Light/Dark/System theo Zed; khóa theo VS Code |
| Keymap gốc | Có | Có (cài extension keymap) | Không: Lucerna chỉ theo phím VS Code |
| Vim | Switch `vim_mode` | Không | Switch `vim.enabled` |
| Font | Bản cũ có, bản mới bỏ | Không | Chỉ `editor.fontSize`; font family người dùng tự đặt trong `settings.json` |
| AI, đăng nhập, telemetry | Có | Có | Không |
| Mở lại | Lệnh `zed: open onboarding`; Welcome có nút "Return to Onboarding" | Lệnh nhóm Developer | Lệnh + Help menu + nút trên Welcome |

## 4. Nguồn

- Zed: `crates/onboarding/src/{onboarding.rs, basics_page.rs, theme_preview.rs}`, `crates/zed/src/main.rs`, `crates/zed/src/zed/app_menus.rs`, `crates/workspace/src/{welcome.rs, pane.rs}`, `assets/keymaps/default-macos.json`; bản trước refactor ở commit 4dbd1864 (`editing_page.rs`). Release notes 0.208.4: https://zed.dev/releases/stable/0.208.4
- VS Code: `src/vs/workbench/contrib/welcomeGettingStarted/{browser/startupPage.ts, browser/gettingStartedService.ts, browser/gettingStarted.ts, common/gettingStartedContent.ts, common/media/theme_picker.ts}`, `src/vs/workbench/contrib/welcomeOnboarding/*`, `src/vs/workbench/services/themes/common/themeConfiguration.ts`.

Chưa kiểm chứng được: Esc/đóng tab trên onboarding của Zed ngoài cách đóng tab thường; nhãn "Dark Modern" so với id `Dark 2026` của VS Code.
