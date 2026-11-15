# Onboarding lần chạy đầu ("Get Started"): plan triển khai

> **Cho agent thực thi:** BẮT BUỘC dùng sub-skill superpowers:subagent-driven-development (khuyên dùng) hoặc superpowers:executing-plans để làm plan này theo từng task. Các bước dùng checkbox (`- [ ]`) để theo dõi.

**Mục tiêu:** lần đầu mở Lucerna mà không có gì để mở lại thì hiện trang Get Started, nơi chỉnh theme (Light/Dark/System), font và cỡ chữ editor, Vim mode, và mở `settings.json`. Mỗi thay đổi ghi ngay vào settings người dùng. Mở lại được bằng `lucerna.openOnboarding`, Help ▸ Get Started và nút trên Welcome.

**Kiến trúc:**
- Phần thuần (không gpui): ghi nhiều khóa JSONC giữ comment (`settings_edit.rs`), cờ `onboarded` trong `<data_dir>/state.json` (`app_state_file.rs`), chọn màn khởi động (`startup::startup_screen`), ánh xạ Light/Dark/System ra khóa settings (`onboarding/model.rs`).
- `OnboardingPage` là một view, có `Host::{Window, Tab}`. Ở `Window` nó là root của cửa sổ chưa mở thư mục (như `WelcomeView`, `Router` đổi root qua lại). Ở `Tab` nó là nội dung của `TabItem::Onboarding` (singleton, không lưu), giống `LspHealthView`.
- Component mới nằm trong `ui.rs`. `FontPicker` dựng như `RecentPicker`/`ThemePicker`.

**Tech stack:** Rust, GPUI (Zed rev `b5cd310`), `jsonc-parser` (đã dùng trong `theme_picker.rs`), serde_json, tempfile cho test.

**Spec:** `docs/superpowers/specs/2026-11-15-onboarding-design.md`. Nghiên cứu: `docs/research/2026-11-15-onboarding.md`. Đọc spec trước mỗi task; mỗi task ghi các mục § nó triển khai.

## Ràng buộc chung

- Chỉ đổi `crates/lucerna-workbench` (và `crates/lucerna-settings/assets/default-keybindings.json`). Không thêm dependency. `settings_edit.rs`, `app_state_file.rs`, `onboarding/model.rs`, `startup.rs` không dùng gpui.
- Không thêm giả định chỉ có macOS ngoài `cfg(target_os = "macos")`.
- Luồng UI không chờ I/O: đọc/ghi `state.json` (trừ lần đọc đồng bộ lúc khởi động trong `open_without_folder`) và lấy danh sách font chạy qua `cx.background_spawn`. Ghi `settings.json` đồng bộ như `set_theme_setting` hiện nay (một file nhỏ, cùng cách theme picker đang làm).
- Trước mỗi `cargo test`/build: `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"`, `scripts/prune-target.sh`; chỉ giữ một profile; không xóa profile khi `pgrep -x cargo` cho thấy session khác đang build; dung lượng trống không dưới ~1.2 GB. Dùng thư mục `target` chung, **không** đặt `CARGO_TARGET_DIR` hay `--target-dir`.
- Commit kết thúc bằng `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`, stage theo đường dẫn cụ thể (`git commit … -- <paths>`).
- Lệnh test: `cargo test -p lucerna-workbench <filter>` từ gốc repo.
- Chuỗi hiển thị nguyên văn từ spec: "Welcome to Lucerna", "Set up the basics. Change them any time.", "Finish", "THEME", "EDITOR", "Light", "Dark", "System", "Font Family", "Font Size", "Vim Mode", "Enable Vim keybindings (vim.enabled)", "Open Settings (JSON)", tab "Get Started", lệnh "Help: Get Started", mục menu "Get Started".
- Giá trị cố định: cột tối đa 640 px; padding `8×GRID`; khoảng cách khối `6×GRID`; cỡ chữ 6–100, bước 1; fallback theme "Lucerna Light"/"Lucerna Dark"; font fallback "Lilex".
- ID lệnh: `lucerna.openOnboarding` (không phím), `lucerna.onboarding.finish` (`cmd+enter` khi `onboardingFocus`).

## Review Focus

1. **`settings.json` có comment, dấu phẩy cuối, hoặc khóa trùng** — mọi lần ghi từ trang phải giữ nguyên phần còn lại và sửa bản cuối của khóa trùng (Task 1 có test).
2. **`settings.json` đang hỏng cú pháp khi bấm một control** — không ghi gì, báo lỗi cấu hình, control giữ giá trị cũ (Task 6 có test).
3. **Bấm nhanh nhiều lần + hoặc −** — mỗi lần đọc giá trị đã gộp mới nhất, không mất bước, không vượt 6–100 (Task 4 và Task 6 có test).
4. **`state.json` hỏng hoặc thư mục dữ liệu không ghi được** — vẫn hiện trang, không panic; file hỏng thành `.bak` (Task 2 có test).
5. **Mở app kèm đường dẫn ở lần đầu** — không hiện onboarding, không ghi cờ (Task 2 kiểm `startup_screen` không được gọi trên đường này; Task 7 có test router).

---

### Task 1: Ghi nhiều khóa JSONC (`settings_edit.rs`)

Spec §5.3 (ghi hai khóa một lần), O6.

**Files:**
- Create: `crates/lucerna-workbench/src/settings_edit.rs`
- Modify: `crates/lucerna-workbench/src/theme_picker.rs` (`set_string_setting`, `write_string_setting`, `write_atomically` chuyển sang module mới; `theme_picker` gọi lại), `src/lib.rs` (`mod settings_edit;`)

**Interfaces:**
- Produces:
  - `pub(crate) fn set_settings(text: &str, edits: &[(&str, serde_json::Value)]) -> Result<String, SettingsError>` — áp lần lượt từng khóa top-level; giá trị chuỗi, bool, số.
  - `pub(crate) fn write_settings(path: &Path, edits: &[(&str, serde_json::Value)]) -> Result<(), SettingsError>` — đọc file (không có thì `USER_SETTINGS_TEMPLATE`), `set_settings`, ghi nguyên tử một lần.
  - `pub(crate) fn save_user_settings(edits: &[(&str, serde_json::Value)], cx: &mut App) -> Result<(), ()>` — như `save_setting` hiện có (không có `ConfigState` thì `Ok(())` + warn; lỗi thì `record_error`), rồi `reload_config(&[user_settings], cx)`.
  - `set_string_setting(text, key, value)` giữ chữ ký, gọi `set_settings(text, &[(key, Value::String)])`.

- [ ] **Step 1: Viết test hỏng** trong `settings_edit.rs` `mod tests`:
  - `writes_bool_and_number`: `set_settings("{\n  // c\n}\n", &[("vim.enabled", json!(true)), ("editor.fontSize", json!(15))])` parse lại ra `vim.enabled == true`, `editor.fontSize == 15`, và còn chứa `// c`.
  - `replaces_in_place_keeping_comments`: text có `"editor.fontSize": 14, // size` → kết quả chứa `"editor.fontSize": 15, // size`.
  - `two_keys_one_existing`: `{"window.autoDetectColorScheme": true}` + edits autoDetect=false, colorTheme="Lucerna Dark" → cả hai khóa đúng, autoDetect chỉ xuất hiện một lần.
  - `edits_last_duplicate`, `trailing_comma_kept_valid`, `empty_text`, `invalid_text_is_error` (`"{ \"a\": "` và `"[]"` là `Err`).
  - Các test cũ của `set_string_setting` (chuyển sang đây) vẫn chạy.
- [ ] **Step 2:** `cargo test -p lucerna-workbench settings_edit` → FAIL (chưa có hàm).
- [ ] **Step 3:** Viết `set_settings` bằng cách lặp logic của `set_string_setting` hiện có cho từng edit (parse lại text sau mỗi edit), giá trị serialize bằng `serde_json::to_string(&value)`. Chuyển `write_atomically` sang đây. Viết `write_settings`, `save_user_settings`; `theme_picker::save_setting` gọi `save_user_settings` (không reload hai lần: `set_theme_setting` hiện tự reload, bỏ đoạn reload đó).
- [ ] **Step 4:** `cargo test -p lucerna-workbench settings_edit theme_picker icon_theme_picker` → PASS.
- [ ] **Step 5:** Commit `refactor(settings): write several JSONC settings at once`.

### Task 2: Cờ `onboarded` và chọn màn khởi động

Spec §4, O2–O4, §10 (người dùng nâng cấp).

**Files:**
- Create: `crates/lucerna-workbench/src/app_state_file.rs`
- Modify: `src/startup.rs`, `src/lib.rs`

**Interfaces:**
- Consumes: `workspace_state::write_atomic(path, bytes)`.
- Produces:
  - `pub fn state_path(data_dir: &Path) -> PathBuf` → `<data_dir>/state.json`.
  - `pub fn onboarded(data_dir: &Path) -> bool` — file không có/thiếu khóa → `false`; hỏng hoặc `version != 1` → đổi thành `state.json.bak`, `false`.
  - `pub fn mark_onboarded(data_dir: &Path) -> io::Result<()>` — đọc lại, đặt `onboarded: true`, giữ khóa lạ, ghi `{ "version": 1, "onboarded": true }` nguyên tử.
  - `startup.rs`: `pub enum StartupScreen { Restore(PathBuf), Onboarding, Welcome, FolderPrompt }` và `pub fn startup_screen(restore: Option<&RecentEntry>, onboarded: Option<bool>, choice: Startup) -> StartupScreen` (`onboarded: None` nghĩa là không có data dir).

- [ ] **Step 1: Test hỏng.**
  - `app_state_file`: `missing_file_is_not_onboarded`; `mark_then_read`; `corrupt_file_becomes_bak` (`"{"` → `false`, có `state.json.bak`); `other_version_becomes_bak`; `no_tmp_left` sau `mark_onboarded`; `mark_creates_data_dir` (thư mục chưa tồn tại).
  - `startup`: `restore_wins` (`Some(entry)`, `Some(false)` → `Restore(path)`); `first_run_onboarding` (`None`, `Some(false)`, `Startup::Welcome` → `Onboarding`); `first_run_beats_folder_prompt` (`None`, `Some(false)`, `FolderPrompt` → `Onboarding`); `onboarded_follows_choice` (→ `Welcome` / `FolderPrompt`); `no_data_dir_follows_choice` (`onboarded: None` → theo `choice`).
- [ ] **Step 2:** `cargo test -p lucerna-workbench app_state_file startup` → FAIL.
- [ ] **Step 3:** Viết hai module theo Interfaces (`.bak` theo cách `recent::load`).
- [ ] **Step 4:** Chạy lại → PASS.
- [ ] **Step 5:** Commit `feat(startup): remember the first run in state.json`.

### Task 3: Component mới trong `ui.rs`

Spec §5.2.

**Files:**
- Modify: `crates/lucerna-workbench/src/ui.rs`
- Test: `crates/lucerna-workbench/tests/ui_controls.rs` (mới)

**Interfaces:**
- Produces (mỗi hàm đặt `debug_selector` bằng `id`, phần tử con dùng `"{id}-{suffix}"`):
  - `pub fn segmented_control(id: &'static str, options: &[&'static str], selected: usize, cx: &App) -> Vec<Stateful<Div>>` — trả các nút để gắn `on_click`; selector con `"{id}-{index}"`; bọc bằng `segmented_group(cx) -> Div` (viền 1 px `border`, bo `GRID`, cao `6×GRID`). Nút chọn nền `element_selected`, khác hover `ghost_element_hover`.
  - `pub fn switch(id: &'static str, on: bool, cx: &App) -> Stateful<Div>` — `7×GRID`×`4×GRID`, bo tròn; bật: nền `element_selected`, núm phải; tắt: nền `element_background`, núm trái.
  - `pub fn number_stepper(id: &'static str, value: u32, at_min: bool, at_max: bool, cx: &App) -> (Div, Stateful<Div>, Stateful<Div>)` — (khung, nút `"{id}-minus"`, nút `"{id}-plus"`); nút ở giới hạn mờ và không nhận click. Dùng `icon_button`. Thêm icon `minus`/`plus` vào `icons.rs` nếu chưa có (Lucide, ISC, đã có giấy phép trong repo).
  - `pub fn dropdown_button(id: &'static str, label: impl Into<SharedString>, cx: &App) -> Stateful<Div>` — viền như `input_box`, `CHEVRON_DOWN_ICON` bên phải.
  - `pub fn setting_row(label: &str, description: Option<&str>, control: impl IntoElement, cx: &App) -> Div`.
  - `pub fn theme_preview(id: &'static str, left: &Theme, right: Option<&Theme>, cx: &App) -> Stateful<Div>` — vẽ bằng màu theme truyền vào; `right: Some` thì chia đôi (System). Cao `30×GRID`, bo 8 px, viền 1 px. Bảng thanh mã giả là `const PREVIEW_LINES: &[(indent: u8, &[(width: u8, Token)])]` cố định, ~8 dòng; `Token` ánh xạ sang màu cú pháp keyword/function/string/comment/type/variable của theme.

- [ ] **Step 1: Test hỏng** (`tests/ui_controls.rs`, vẽ trong `VisualTestContext`, kiểm bằng `debug_bounds`):
  - `segmented_draws_each_option`: 3 option → có `seg-0`, `seg-1`, `seg-2`.
  - `stepper_buttons_exist`; `switch_draws`; `dropdown_draws`; `theme_preview_split_has_two_halves` (`preview-left`, `preview-right` khi `right: Some`, chỉ `preview-left` khi `None`).
- [ ] **Step 2:** `cargo test -p lucerna-workbench --test ui_controls` → FAIL.
- [ ] **Step 3:** Viết các hàm. Trước khi tạo, kiểm `ui.rs` xem có thứ tương đương để dùng lại (`toggle_button`, `checkbox`, `input_box`).
- [ ] **Step 4:** Chạy lại → PASS.
- [ ] **Step 5:** Commit `feat(ui): segmented control, switch, stepper, dropdown and theme preview`.

### Task 4: Mô hình settings của trang (`onboarding/model.rs`)

Spec §5.3, §5.4, O7, O9.

**Files:**
- Create: `crates/lucerna-workbench/src/onboarding/mod.rs` (tạm chỉ `pub mod model;`), `src/onboarding/model.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: `SettingsStore::get`, `ThemeRegistry::get_or_default`, hằng `AUTO_DETECT_KEY`, `COLOR_THEME_KEY`, `PREFERRED_DARK_KEY`, `PREFERRED_LIGHT_KEY` (đổi sang `pub(crate)` trong `config_files.rs` nếu đang private).
- Produces:
  - `#[derive(Clone, Copy, PartialEq, Eq, Debug)] pub enum ThemeMode { Light, Dark, System }`.
  - `pub fn current_mode(settings: &SettingsStore, themes: &ThemeRegistry) -> ThemeMode` — autodetect (mặc định `true`) → `System`; ngược lại theo appearance của theme `workbench.colorTheme` (không tìm thấy → `get_or_default`).
  - `pub fn preferred_theme(settings: &SettingsStore, appearance: Appearance) -> String` — đọc **trực tiếp** `PREFERRED_LIGHT_KEY`/`PREFERRED_DARK_KEY`, fallback "Lucerna Light"/"Lucerna Dark".
  - `pub fn mode_edits(mode: ThemeMode, settings: &SettingsStore) -> Vec<(&'static str, serde_json::Value)>` — System: `[(autoDetect, true)]`; Light: `[(autoDetect, false), (colorTheme, preferred_theme(Light))]`; Dark tương tự.
  - `pub fn preview_themes(mode: ThemeMode, settings: &SettingsStore) -> (String, Option<String>)` — Light/Dark: (`workbench.colorTheme` sau khi áp mode, None); System: (preferred Light, Some(preferred Dark)).
  - `pub const FONT_SIZE_MIN: u32 = 6; pub const FONT_SIZE_MAX: u32 = 100;`
  - `pub fn current_font_size(settings: &SettingsStore) -> u32` (làm tròn, kẹp 6–100, mặc định 14).
  - `pub fn stepped_font_size(current: u32, delta: i32) -> Option<u32>` — `None` khi không đổi (đã ở giới hạn).
  - `pub fn current_font_family(settings: &SettingsStore) -> String` (fallback "Lilex").

- [ ] **Step 1: Test hỏng** (`model.rs` `mod tests`, dựng store bằng `SettingsStore::with_defaults()` + `set_layer(Layer::User, json)`):
  - `defaults_are_system`;
  - `autodetect_off_dark_theme_is_dark`, `…light_theme_is_light` (`"workbench.colorTheme": "Lucerna Light"`);
  - `dark_edits_use_preferred_dark` với user `"workbench.preferredDarkColorTheme": "My Dark"` và autodetect `false` → edits chứa `("workbench.colorTheme", "My Dark")` (bảo vệ lỗi dùng `configured_theme_name`);
  - `light_edits_default_to_lucerna_light`; `system_edits_only_autodetect`;
  - `system_preview_is_split`;
  - `font_size_steps_and_clamps`: `stepped(14, 1) == Some(15)`, `stepped(100, 1) == None`, `stepped(6, -1) == None`;
  - `font_size_rounds_and_clamps`: user `13.6` → 14; `200` → 100;
  - `family_fallback_is_lilex` (user `""` cũng → "Lilex").
- [ ] **Step 2:** `cargo test -p lucerna-workbench onboarding::model` → FAIL.
- [ ] **Step 3:** Viết theo Interfaces.
- [ ] **Step 4:** Chạy lại → PASS.
- [ ] **Step 5:** Commit `feat(onboarding): map theme modes and font settings`.

### Task 5: `FontPicker`

Spec §5.4 (Font Family), O9.

**Files:**
- Create: `crates/lucerna-workbench/src/font_picker.rs`
- Modify: `src/lib.rs`
- Test: `crates/lucerna-workbench/tests/font_picker.rs`

**Interfaces:**
- Consumes: khung picker như `recent_picker.rs` (`popover`, `picker_input_row`, `picker_list`, `picker_row`, `fuzzy_filter`).
- Produces:
  - `pub struct FontPicker`; `pub enum FontPickerEvent { Confirmed(String), Dismissed }`.
  - `FontPicker::new(names: Vec<String>, current: &str, window: &mut Window, cx: &mut Context<Self>) -> Self` — danh sách "Lilex" đứng đầu rồi `names` bỏ trùng (kể cả bỏ "Lilex" trùng), xếp chữ cái không phân biệt hoa thường; chọn sẵn dòng `current`.
  - `pub fn font_names(cx: &App) -> Task<Vec<String>>` — `all_font_names()` lấy ở luồng nền.
  - `row_texts(&self) -> Vec<String>`, `selected_index(&self) -> usize` cho test (như `ThemePicker`).
  - Mỗi dòng vẽ tên bằng chính font đó. ↑/↓ chỉ đổi dòng chọn; Enter → `Confirmed(name)`; Escape → `Dismissed`. Không xem trước.

- [ ] **Step 1: Test hỏng:** `lilex_first_and_sorted_deduped` (`["Menlo", "Lilex", "Fira Code", "Menlo"]` → `["Lilex", "Fira Code", "Menlo"]`); `filters_fuzzily` (gõ "fir" → chỉ "Fira Code"); `enter_confirms_selected`; `escape_dismisses`; `current_is_selected`; `empty_names_only_lilex`.
- [ ] **Step 2:** `cargo test -p lucerna-workbench --test font_picker` → FAIL.
- [ ] **Step 3:** Viết `FontPicker`.
- [ ] **Step 4:** Chạy lại → PASS.
- [ ] **Step 5:** Commit `feat(onboarding): font family picker`.

### Task 6: `OnboardingPage` (root view)

Spec §5.1, §5.3–§5.6, §8, O5, O6, O10.

**Files:**
- Modify: `crates/lucerna-workbench/src/onboarding/mod.rs`, `src/lib.rs` (`pub use onboarding::{OnboardingPage, OnboardingEvent, Host}`)
- Test: `crates/lucerna-workbench/tests/onboarding.rs`

**Interfaces:**
- Consumes: Task 1 `save_user_settings`; Task 3 components; Task 4 model; Task 5 `FontPicker`, `font_names`.
- Produces:
  - `#[derive(Clone, Copy, PartialEq, Eq)] pub enum Host { Window, Tab }`.
  - `pub enum OnboardingEvent { Finish, OpenSettingsInNewWindow(PathBuf), OpenSettingsInWorkbench }`.
  - `OnboardingPage::new(host: Host, window: &mut Window, cx: &mut Context<Self>) -> Self`; `impl Render, Focusable, EventEmitter<OnboardingEvent>`.
  - `pub fn run_command(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) -> bool` (như `WelcomeView::run_command`; xử lý `lucerna.onboarding.finish`, `workbench.action.openSettingsJson`, và ở `Host::Window` các lệnh trong `WELCOME_COMMANDS` có nghĩa không cần recent list: About, Quit).
  - `pub const ONBOARDING_COMMANDS: &[&str]` — `lucerna.onboarding.finish`, `workbench.action.openSettingsJson`, `lucerna.openOnboarding`, `workbench.action.showAboutDialog`, `workbench.action.quit`, `workbench.action.files.openFileFolder`, `workbench.action.files.openFile`, `workbench.action.files.openFolder`.
  - Debug selectors: `onboarding-finish`, `onboarding-theme-{0,1,2}` (Light, Dark, System), `onboarding-preview`, `onboarding-font-family`, `onboarding-font-size-minus`/`-plus`/`-value`, `onboarding-vim`, `onboarding-font-sample`, `onboarding-open-settings`.
- Hành vi:
  - Vẽ theo `AppState.settings` mỗi frame (không cache), nên sửa tay file thì trang đổi theo (`cx.observe_global::<AppState>` → `notify`).
  - Theme: bấm nút khác nút đang chọn → `save_user_settings(&mode_edits(..))`. Bấm nút đang chọn → không ghi.
  - −/+ → `stepped_font_size` → `Some(n)` thì ghi `editor.fontSize: n`.
  - Vim → ghi `vim.enabled: !current`.
  - Font Family → mở `FontPicker` trong `popover_overlay`; `Confirmed(name)` → ghi `editor.fontFamily: name`.
  - Open Settings: `Host::Tab` → `OpenSettingsInWorkbench`; `Host::Window` → đảm bảo file tồn tại (tạo từ `USER_SETTINGS_TEMPLATE`) rồi `OpenSettingsInNewWindow(path)`; lỗi → `warning_row` ở đầu cột.
  - Phím (§5.6): ở `Host::Window`, `on_key_down` dispatch qua keymap như `WelcomeView::dispatch_key`, context có `onboardingFocus = picker.is_none()`, `inQuickOpen = picker.is_some()`; chỉ chạy `ONBOARDING_COMMANDS`. Picker mở thì phím vào picker trước. Rồi `page_key`: Tab/⇧Tab/↑/↓ đổi focus theo thứ tự Finish → Light → Dark → System → Font Family → − → + → Vim → Open Settings; Enter/Space bấm; ←/→ trong cụm theme đổi lựa chọn. Focus ban đầu: nút theme đang chọn.
  - Title bar (`layout::title_bar`) chỉ vẽ ở `Host::Window`.

- [ ] **Step 1: Test hỏng** (`tests/onboarding.rs`, dùng `watch_config(ConfigPaths::user_only_with_user_dir(tmp))` như `tests/config_files.rs` để có file người dùng thật):
  - `draws_current_settings`: user `{"editor.fontSize": 18, "vim.enabled": true}` → text của `onboarding-font-size-value` là "18", switch bật.
  - `dark_writes_autodetect_and_color_theme`: bấm `onboarding-theme-1` → file có `"window.autoDetectColorScheme": false`, `"workbench.colorTheme": "Lucerna Dark"`, comment template còn; `AppState.theme.name == "Lucerna Dark"`.
  - `system_writes_autodetect_true`.
  - `plus_three_times_writes_17` (từ 14) — Review Focus 3.
  - `plus_at_100_writes_nothing` (file không đổi byte nào).
  - `vim_toggle_writes_true`.
  - `broken_settings_not_written`: file `{ "a": ` → bấm Dark → file không đổi, có lỗi cấu hình ghi nhận — Review Focus 2.
  - `reflects_hand_edit`: ghi tay `"editor.fontSize": 20` + `reload_config` → value "20".
  - `cmd_enter_emits_finish`; `cmd_enter_in_font_picker_does_nothing` (picker mở, ⌘Enter → không có `Finish`).
  - `open_settings_in_window_creates_file`: không có `settings.json` → bấm → file tồn tại, event `OpenSettingsInNewWindow(path)`.
  - `font_picker_confirm_writes_family`.
- [ ] **Step 2:** `cargo test -p lucerna-workbench --test onboarding` → FAIL.
- [ ] **Step 3:** Viết `OnboardingPage` theo Interfaces và bố cục §5.1 (cột 640 px, logo/tiêu đề như Welcome, `section_header` "THEME"/"EDITOR", `primary_text_button` Finish có phím, `list_button` Open Settings có phím theo keymap, ô `onboarding-font-sample` là `subheader_card` với hai dòng `fn main() {` / `    println!("Hello, Lucerna");` bằng font/cỡ hiện hành và màu cú pháp theme active). Cột cuộn dọc.
- [ ] **Step 4:** Chạy lại → PASS.
- [ ] **Step 5:** Commit `feat(onboarding): the Get Started page`.

### Task 7: Router, Welcome, menu Help

Spec §4, §5.5 (root), §6 (Welcome, Help), O1–O5, O8.

**Files:**
- Modify: `crates/lucerna-workbench/src/app.rs`, `src/welcome.rs`, `src/app_menu.rs`
- Test: `tests/welcome.rs`, `src/app_menu.rs` `mod tests`, `tests/onboarding.rs`

**Interfaces:**
- Consumes: Task 2 `onboarded`, `mark_onboarded`, `startup_screen`; Task 6 `OnboardingPage`, `OnboardingEvent`, `ONBOARDING_COMMANDS`.
- Produces:
  - `app.rs`: `Shown::Onboarding(WindowHandle<OnboardingPage>)`; `Router::open_onboarding(this, cx)` (cửa sổ mới, như `open_welcome`); `Router::replace_with_onboarding(this, cx)` và `replace_with_welcome` dùng được khi root đang là Onboarding; `on_onboarding_event`: `Finish` → `replace_with_welcome`; `OpenSettingsInNewWindow(path)` → `process::spawn` như "cửa sổ mới" của Welcome (lỗi → `tracing::error`).
  - `pub(crate) fn first_screen(data_dir: Option<&Path>, settings: &SettingsStore) -> StartupScreen` — không gpui: đọc `recent::load` + `last_workspace` (chỉ khi `restores_last_workspace`), `onboarded`, `startup_choice`, rồi gọi `startup_screen`.
  - `open_without_folder` gọi `first_screen` rồi `match`; với `Onboarding` thì `mark_onboarded` ở nền (lỗi → `tracing::warn`) rồi `open_onboarding`. Đường `open_urls`/CLI không đổi.
  - `welcome.rs`: nút "Get Started" sau "Open Recent…" (`Button::Command("lucerna.openOnboarding")`), thêm `lucerna.openOnboarding` vào `WELCOME_COMMANDS`, `WelcomeEvent::OpenOnboarding` → router `replace_with_onboarding`.
  - `app_menu.rs`: `WindowKind::Onboarding`; Help bắt đầu bằng `command("Get Started", "lucerna.openOnboarding")`, separator, rồi các mục cũ; ở `Welcome` bật thêm mục này (đã có trong `WELCOME_COMMANDS`); ở `Onboarding` bật theo `ONBOARDING_COMMANDS`. `run_command` của menu thử thêm `WindowHandle::<OnboardingPage>` sau Welcome. `active_window` trả `WindowKind::Onboarding` cho root onboarding.

- [ ] **Step 1: Test hỏng:**
  - `app_menu` unit: `help_starts_with_get_started`; `welcome_enables_get_started`; `onboarding_enables_only_its_commands` (Save disabled, Get Started/About/Quit enabled).
  - `tests/welcome.rs`: `get_started_button_emits_open_onboarding` (bấm nút → `WelcomeEvent::OpenOnboarding`); `ctrl_r` cũ vẫn chạy.
  - `app.rs` unit (`mod tests`, temp dir): `first_screen_fresh_is_onboarding` (không có gì → `Onboarding`); `first_screen_restores_existing_folder` (`recent.json` có thư mục tồn tại → `Restore`); `first_screen_after_onboarded_is_welcome` (`state.json` onboarded, không recent → `Welcome`); `first_screen_restore_none_first_run` (`window.restoreWindows: "none"` + recent có thư mục → `Onboarding`); `first_screen_no_data_dir` (`None` → `Welcome`).
- [ ] **Step 2:** `cargo test -p lucerna-workbench app_menu welcome onboarding` → FAIL.
- [ ] **Step 3:** Viết theo Interfaces.
- [ ] **Step 4:** Chạy lại → PASS; cả `tests/welcome.rs` cũ.
- [ ] **Step 5:** Commit `feat(onboarding): show Get Started on the first run, from Welcome and Help`.

### Task 8: Tab Get Started trong workbench

Spec §6 (lệnh, tab), §5.6 (`onboardingFocus`, ⌘Enter đóng tab), O5.

**Files:**
- Modify: `crates/lucerna-workbench/src/editor_groups/model.rs`, `src/editor_groups/view.rs`, `src/workbench.rs`, `src/onboarding/mod.rs`, `crates/lucerna-settings/assets/default-keybindings.json`
- Test: `tests/onboarding.rs`, `tests/editor_groups_model.rs`

**Interfaces:**
- Consumes: Task 6 `OnboardingPage::new(Host::Tab, …)`.
- Produces:
  - `TabItem::Onboarding` — singleton, never preview, **never saved** (doc comment như `LspHealth`); tiêu đề "Get Started"; icon tùy người làm (spec §11).
  - `EditorGroups::open_onboarding(&mut self) -> TabId` như `open_lsp_health`.
  - `onboarding::register_commands(commands: &mut CommandRegistry)` đăng ký `lucerna.openOnboarding` ("Help: Get Started") và `lucerna.onboarding.finish`; được gọi trong các `register_commands` của workbench.
  - `Workbench`: một `Entity<OnboardingPage>` tạo lười khi mở tab lần đầu; `OnboardingEvent::Finish` → đóng tab Onboarding; `OpenSettingsInWorkbench` → `run_command("workbench.action.openSettingsJson")`.
  - `context_keys`: `flag("onboardingFocus", self.onboarding_focused(window, cx))` — đúng khi focus nằm trong `OnboardingPage` và picker của nó không mở.
  - `default-keybindings.json`: `{ "key": "cmd+enter", "command": "lucerna.onboarding.finish", "when": "onboardingFocus" }`.
  - Workspace state bỏ qua `TabItem::Onboarding` khi lưu.

- [ ] **Step 1: Test hỏng:**
  - `editor_groups_model`: `onboarding_is_singleton` (mở hai lần → một tab, kích hoạt tab cũ); `onboarding_not_saved` (serialize state không có nó).
  - `tests/onboarding.rs`: `command_opens_tab` (chạy `lucerna.openOnboarding` hai lần → một tab "Get Started"); `cmd_enter_closes_tab`; `cmd_enter_in_editor_still_inserts_line` (focus editor, ⌘Enter → dòng mới, tab Onboarding không bị đóng); `dark_from_tab_changes_theme`.
- [ ] **Step 2:** `cargo test -p lucerna-workbench --test onboarding --test editor_groups_model` → FAIL.
- [ ] **Step 3:** Viết theo Interfaces.
- [ ] **Step 4:** Chạy lại → PASS.
- [ ] **Step 5:** Commit `feat(onboarding): the Get Started tab and lucerna.openOnboarding`.

### Task 9: Kiểm tra toàn bộ và so với Zed

Spec §9 (thủ công), design-principles §1.1.

- [ ] **Step 1:** `scripts/prune-target.sh`, rồi `cargo test -p lucerna-workbench` → mọi test xanh.
- [ ] **Step 2:** `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, `cargo deny check` → sạch.
- [ ] **Step 3:** Ghi chú `docs/superpowers/specs/2026-11-15-onboarding-design.md` dòng trạng thái: "đã triển khai (commit …)", và các chỗ khác spec nếu có.
- [ ] **Step 4 (thủ công, cần người dùng hoặc session có app):** build `scripts/bundle-app.sh`, chạy với `HOME` mới: lần đầu thấy trang; Dark/+/Vim ghi đúng; ⌘Enter → Welcome; thoát, mở lại → Welcome; mở thư mục, thoát, mở lại → thư mục; Help ▸ Get Started trong workbench mở tab. Đặt cạnh Zed (`zed: open onboarding`) so khoảng cách, cỡ chữ, nút, switch, hover.
- [ ] **Step 5:** Commit `docs(onboarding): mark the spec implemented`.
