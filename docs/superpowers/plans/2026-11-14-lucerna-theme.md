# Theme Lucerna Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Thay One Dark/One Light bằng Lucerna Dark/Lucerna Light làm theme duy nhất đi kèm, đạt WCAG AA, và tự đổi theo chế độ sáng/tối của hệ thống.

**Architecture:** Một file `lucerna.json` định dạng Zed trong `lucerna-theme`, có test khoá và test tương phản. Workbench nhớ appearance của hệ thống trong `AppState`, chọn tên theme bằng một hàm thuần từ settings, và cập nhật khi cửa sổ báo appearance đổi. Picker ghi vào setting preferred khi đang tự theo hệ thống.

**Tech Stack:** Rust, GPUI (rev `b5cd310`), serde_json/jsonc, Python 3 cho `gen-licenses.py`.

**Spec:** `docs/superpowers/specs/2026-11-14-lucerna-theme-design.md` (đã duyệt). Executor đọc cả spec, nhất là các bảng màu.

## Global Constraints

- Làm trong worktree `.claude/worktrees/lucerna-theme`, nhánh `claude/lucerna-theme`. Không merge, không push.
- Cargo: `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"`. Dùng `CARGO_TARGET_DIR=/Users/duykhanh/Projects.nosync/lucerna/target` (chung với checkout chính, chỉ một profile). Chạy `scripts/prune-target.sh` trước mỗi lần build/test. Trước `cargo test` thì xoá `target/release` nếu có, trừ khi `pgrep -x cargo` cho thấy session khác đang build.
- Chỉ `lucerna-workbench` dùng gpui. `lucerna-theme` là dữ liệu thuần.
- Không giả định chỉ có macOS.
- Tên theme chính xác: `"Lucerna Dark"`, `"Lucerna Light"`; family `"Lucerna"`.
- Ngưỡng: chữ ≥ 4.5:1 (WCAG 2.x), thành phần không phải chữ ≥ 3:1.
- Setting: `window.autoDetectColorScheme` (mặc định `true`), `workbench.colorTheme` (`"Lucerna Dark"`), `workbench.preferredDarkColorTheme` (`"Lucerna Dark"`), `workbench.preferredLightColorTheme` (`"Lucerna Light"`).
- Chữ placeholder của picker, đúng từng chữ: `Select Color Theme for System Dark Mode`, `Select Color Theme for System Light Mode`, `Select Color Theme (detect system color mode disabled)`.
- Commit: stage theo đường dẫn, `git commit -- <paths>`, trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. `cargo deny check` phải qua.

## Review Focus

1. Người dùng đã có `"workbench.colorTheme": "One Dark"` trong settings: theme không còn tồn tại, nên phải rơi về Lucerna Dark mà không lỗi. Test ở Task 2.
2. Tự theo hệ thống bật, nhưng `preferredLightColorTheme` trỏ tới theme không có: dùng Lucerna Dark (mặc định của registry), không panic. Test ở Task 2.
3. Hệ thống đổi chế độ trong lúc picker đang xem trước: không được đè lên bản xem trước (`reapply_configured_theme` đã bỏ qua khi `theme_preview`). Test ở Task 3.
4. Theme của người dùng thiếu khoá: vẫn dùng `UiColors::fallback`, không đổi hành vi. Các test fallback hiện có phải còn qua sau Task 1.
5. Đổi `window.autoDetectColorScheme` từ `true` sang `false` khi đang chạy: theme đổi ngay sang `workbench.colorTheme`. Test ở Task 2.

---

### Task 1: `lucerna.json` thay `one.json`, test khoá và test tương phản

**Files:**
- Create: `crates/lucerna-theme/assets/lucerna.json`
- Create: `crates/lucerna-theme/assets/LICENSE-primer`
- Create: `crates/lucerna-theme/tests/lucerna_theme.rs`
- Delete: `crates/lucerna-theme/assets/one.json`, `crates/lucerna-theme/assets/LICENSE-one`
- Modify: `crates/lucerna-theme/src/registry.rs` (hằng, `builtin`, doc comment)
- Modify: `crates/lucerna-theme/tests/theme.rs` (các test đang dùng One)
- Modify: mọi chuỗi `"One Dark"`/`"One Light"` trong `crates/lucerna-workbench/src/**` và `crates/lucerna-workbench/tests/**`, gồm `app.rs:610`, `excerpt_list/render.rs`, `quick_open/picker.rs:164`, `config_files.rs:396` (doc comment), test trong `theme_picker.rs`

**Interfaces:**
- Produces: `pub const DEFAULT_DARK_THEME: &str = "Lucerna Dark"` và `pub const DEFAULT_LIGHT_THEME: &str = "Lucerna Light"`, cả hai re-export từ `lucerna_theme`. `ThemeRegistry::get_or_default` trả "Lucerna Dark" khi tên không có.

- [ ] **Step 1: Chép tập khoá Zed ra test trước khi xoá `one.json`**

Trong `tests/lucerna_theme.rs`, tạo `const STYLE_KEYS: &[&str]` gồm đúng 141 khoá trong `style` của One Dark (trừ `syntax`, `players`, `accents`), và `const SYNTAX_KEYS: &[&str]` gồm 46 capture trong `style.syntax`. Lấy từ `one.json` bằng `python3 -I -c` rồi dán vào; đây là bản chép một lần.

- [ ] **Step 2: Viết test hỏng**

```rust
// lucerna_theme.rs (đọc lucerna.json qua include_str! và serde_json; test giữ code tính WCAG riêng)
fn contrast(fg: &str, bg: &str) -> f64 // WCAG 2.x; fg có alpha thì trộn lên bg trước khi tính

#[test] fn builtin_is_only_lucerna()
// ThemeRegistry::builtin().names() == ["Lucerna Dark", "Lucerna Light"];
// get("Lucerna Dark").appearance == Dark, get("Lucerna Light").appearance == Light;
// get_or_default("One Dark").name == "Lucerna Dark".

#[test] fn every_zed_key_is_present()
// Với mỗi theme: mọi STYLE_KEYS có trong style, mọi SYNTAX_KEYS có trong style.syntax,
// players có đúng 8 phần tử. Báo theme + khoá thiếu trong thông báo assert.

#[test] fn text_colors_meet_aa_on_editor_background()
// ≥ 4.5 trên editor.background cho: text, text.muted, text.placeholder, text.accent,
// editor.foreground, editor.line_number, editor.active_line_number, error, warning, info,
// hint, success, created, modified, deleted, conflict, renamed, ignored, terminal.foreground,
// mọi màu trong style.syntax, và terminal.ansi.{,bright_}<màu> trừ: black (Dark);
// white và bright_white (Light). Không kiểm dim_*.

#[test] fn list_text_meets_aa_on_selected_row()
// ≥ 4.5 trên element.selected: text, text.muted, created, modified, deleted, conflict, renamed, ignored.

#[test] fn syntax_meets_aa_on_active_line()
// Mọi màu syntax ≥ 4.5 trên editor.active_line.background (trộn alpha nếu có).

#[test] fn non_text_meets_3_to_1()
// ≥ 3.0 trên editor.background: border.focused, players[0].cursor,
// version_control.added, version_control.modified, version_control.deleted.

#[test] fn spec_palette_values()
// Ghim các giá trị [C]/[B] của spec, ví dụ: Dark editor.background "#191919ff",
// syntax keyword "#cc7bf4ff", function "#70b8ffff", comment "#818898ff",
// players[0].cursor "#4f8cffff"; Light editor.background "#ffffffff",
// keyword "#8101c2ff", comment "#677080ff", number "#007a7aff".
```

- [ ] **Step 3: Chạy test và thấy hỏng**

Chạy: `cargo test -p lucerna-theme --test lucerna_theme`
Kỳ vọng: FAIL, vì `lucerna.json` chưa có (include_str! báo lỗi) hoặc `builtin_is_only_lucerna` hỏng.

- [ ] **Step 4: Viết `lucerna.json`**

- Family `"Lucerna"`, `$schema` `https://zed.dev/schema/themes/v0.2.0.json`, hai theme theo thứ tự Dark rồi Light.
- Comment đầu file ghi nguồn: màu code đo từ code theme "Claude" của Claude desktop; chẩn đoán, git và ANSI theo GitHub Primer / `primer/github-vscode-theme` (MIT, xem `LICENSE-primer`).
- Mọi giá trị lấy từ các bảng của spec. Hex viết dạng `#rrggbbaa` như `one.json`.
- ANSI lấy từ `primer/github-vscode-theme` (GitHub Dark Default / GitHub Light Default). Kiểm lại với nguồn; nếu nguồn khác các giá trị dưới đây thì dùng nguồn rồi chạy lại test. Các giá trị trượt AA được chỉnh như sau:
  - Dark: black `#484f58`, red `#ff7b72`, green `#3fb950`, yellow `#d29922`, blue `#58a6ff`, magenta `#bc8cff`, cyan `#39c5cf`, white `#b1bac4`; bright: black `#7c848f` (Primer `#6e7681`, chỉnh), red `#ffa198`, green `#56d364`, yellow `#e3b341`, blue `#79c0ff`, magenta `#d2a8ff`, cyan `#56d4dd`, white `#ffffff`.
  - Light: black `#24292f`, red `#cf222e`, green `#116329`, yellow `#4d2d00`, blue `#0969da`, magenta `#8250df`, cyan `#1b7c83`, white `#6e7781`; bright: black `#57606a`, red `#a40e26`, green `#1a7f37`, yellow `#633c01`, blue `#0070ea` (Primer `#218bff`, chỉnh), magenta `#8a4ef7` (Primer `#a475f9`, chỉnh), cyan `#2a7e92` (Primer `#3192aa`, chỉnh), white `#8c959f`.
  - `dim_*` là màu thường với alpha `0xb3`.
- Các khoá spec để builder quyết định: giữ khung UI trung tính (xám không ngả màu), chọn sao cho các test qua.

- [ ] **Step 5: Đổi registry, xoá One, thêm license**

- `registry.rs`: `const LUCERNA_JSON = include_str!("../assets/lucerna.json")`; thêm hai hằng `pub const` ở Interfaces; `builtin()` có doc comment "The bundled Lucerna family"; câu `expect` nhắc `lucerna.json`; doc của `get_or_default` ghi "Lucerna Dark".
- `lib.rs`: re-export hai hằng, doc crate bỏ chữ "Zed theme loading" nếu nó ngụ ý One (giữ "Zed-format theme loading").
- `git rm` `one.json` và `LICENSE-one`.
- `LICENSE-primer`: văn bản MIT nguyên văn từ `https://github.com/primer/github-vscode-theme/blob/main/LICENSE`.

- [ ] **Step 6: Sửa test cũ và mọi chuỗi One**

- `tests/theme.rs`: `builtin_has_one_dark_and_light` → kiểm hai tên Lucerna. `ui_colors_present_for_one_dark` → giá trị Lucerna Dark: editor bg `#191919`, cursor `#4f8cff`, cùng các giá trị bạn đặt cho `text`, `editor.line_number`, selection, status bar. Các test `*_from_theme_*`, `new_ui_colors_read_from_one_dark`, `error_color_*`, `scrollbar_*`, `editor_subheader_*` đọc "Lucerna Dark"/"Lucerna Light" và so với giá trị trong `lucerna.json`. `unknown_name_defaults_to_one_dark` → `_to_lucerna_dark`. `style_for_falls_back`: giữ ý nghĩa; nếu `string.escape` và `string` cùng màu thì so một capture khác có màu riêng.
- Workbench: thay mọi `"One Dark"` → `"Lucerna Dark"`, `"One Light"` → `"Lucerna Light"`. Riêng `config_files.rs` test (`tests/config_files.rs`) chỉ đổi tên ở task này; phần auto-detect sửa ở Task 2.
- Kiểm: `rg -n "One Dark|One Light|one\.json" crates` không còn kết quả nào.

- [ ] **Step 7: Chạy test**

Chạy: `cargo test -p lucerna-theme && cargo test -p lucerna-workbench`
Kỳ vọng: PASS. Nếu một màu trượt test tương phản, chỉnh màu đó trong `lucerna.json` (giữ hue, đổi lightness) và cập nhật bảng trong spec. Không hạ ngưỡng.

- [ ] **Step 8: Commit**

```bash
git add crates/lucerna-theme crates/lucerna-workbench docs/superpowers/specs/2026-11-14-lucerna-theme-design.md
git commit -m "feat(theme): Lucerna Dark and Lucerna Light replace One" -- crates/lucerna-theme crates/lucerna-workbench docs/superpowers/specs/2026-11-14-lucerna-theme-design.md
```

### Task 2: Settings và chọn theme theo appearance của hệ thống

**Files:**
- Modify: `crates/lucerna-settings/assets/default-settings.json:40` (khối settings đúng như spec mục "Settings")
- Modify: `crates/lucerna-workbench/src/app_state.rs` (trường mới)
- Modify: `crates/lucerna-workbench/src/config_files.rs` (`watch_config`, `load` nhánh `ConfigFile::Themes`, `apply_configured_theme`)
- Modify: `crates/lucerna-workbench/src/extensions/service.rs` (`theme_files`, `extension_theme_files`)
- Modify: `crates/lucerna-workbench/src/app.rs` (mỗi `cx.open_window` gọi `follow_system_appearance`)
- Test: `crates/lucerna-workbench/tests/config_files.rs`, test unit trong `config_files.rs`

**Interfaces:**
- Consumes: `lucerna_theme::{Appearance, DEFAULT_DARK_THEME, DEFAULT_LIGHT_THEME}` (Task 1).
- Produces:
  - `AppState.system_appearance: lucerna_theme::Appearance`, khởi tạo `Dark` nơi `AppState` được dựng.
  - `pub(crate) fn configured_theme_name(settings: &SettingsStore, system: Appearance) -> String` trong `config_files.rs`.
  - `pub(crate) fn color_theme_setting_key(settings: &SettingsStore, system: Appearance) -> &'static str`, trả `"workbench.colorTheme"`, `"workbench.preferredDarkColorTheme"` hoặc `"workbench.preferredLightColorTheme"`.
  - `pub(crate) fn follow_system_appearance(window: &mut Window, cx: &mut App)`.
  - `pub(crate) fn to_appearance(a: gpui::WindowAppearance) -> Appearance`: `Dark`/`VibrantDark` → `Dark`, còn lại → `Light`.

- [ ] **Step 1: Viết test hỏng**

Test unit trong `config_files.rs` (dựng `SettingsStore` từ JSON như các test sẵn có):
```rust
#[test] fn theme_name_follows_system_when_auto_detect_is_on()
// defaults: system Dark → "Lucerna Dark"; system Light → "Lucerna Light".
#[test] fn theme_name_uses_color_theme_when_auto_detect_is_off()
// {"window.autoDetectColorScheme": false, "workbench.colorTheme": "X"} → "X" với cả Dark và Light.
#[test] fn preferred_settings_override_defaults()
// {"workbench.preferredLightColorTheme": "Y"} + Light → "Y".
#[test] fn setting_key_matches_mode()
// auto on + Dark → "workbench.preferredDarkColorTheme"; auto on + Light → "...Light..."; auto off → "workbench.colorTheme".
```
Test tích hợp trong `tests/config_files.rs`:
```rust
#[gpui::test] fn system_appearance_change_switches_theme(cx)
// Settings mặc định, mở cửa sổ test; theme == "Lucerna Light" (nền tảng test báo Light).
// cx.simulate_window_appearance(Dark) hoặc TestWindow::simulate_appearance_change(Dark) → theme == "Lucerna Dark".
#[gpui::test] fn turning_auto_detect_off_uses_color_theme(cx)
// Ghi {"window.autoDetectColorScheme": false, "workbench.colorTheme": "Lucerna Dark"} khi hệ thống Light → "Lucerna Dark".
#[gpui::test] fn stale_one_dark_setting_falls_back(cx)
// {"window.autoDetectColorScheme": false, "workbench.colorTheme": "One Dark"} → "Lucerna Dark", không có lỗi config.
#[gpui::test] fn unknown_preferred_theme_falls_back(cx)
// {"workbench.preferredLightColorTheme": "Nope"} + Light → "Lucerna Dark".
```

- [ ] **Step 2: Chạy test và thấy hỏng**

Chạy: `cargo test -p lucerna-workbench --lib config_files && cargo test -p lucerna-workbench --test config_files`
Kỳ vọng: FAIL (các hàm chưa có).

- [ ] **Step 3: Cài đặt**

- `default-settings.json`: khối bốn setting kèm comment như spec. Giữ `workbench.iconTheme` ngay sau.
- `watch_config`: trước `load(...)`, đặt `AppState.system_appearance = to_appearance(cx.window_appearance())`.
- `apply_configured_theme`: tên = `configured_theme_name(&state.settings, state.system_appearance)`, rồi `get_or_default` như cũ.
- Nạp lần đầu (`ConfigFile::Themes`, `initial`): đổi `only: Option<&str>` thành `only: Option<&[String]>` ở `theme_files` và `extension_theme_files`. Lần đầu truyền các tên cần: auto-detect bật thì cả `preferredDark` và `preferredLight`; tắt thì `colorTheme`. `icon_theme_files` giữ nguyên.
- `follow_system_appearance`: `window.observe_window_appearance(|window, cx| { đặt AppState.system_appearance = to_appearance(window.appearance()); reapply_configured_theme(cx); }).detach()`. Gọi trong closure của mọi `cx.open_window` ở `app.rs` (cửa sổ workbench và Welcome). Gọi thêm ở helper mở cửa sổ test nếu các test tích hợp không đi qua `app.rs`.
- Sửa test cũ trong `tests/config_files.rs` chờ "Lucerna Dark" theo `workbench.colorTheme`: thêm `"window.autoDetectColorScheme": false` vào settings test, hoặc đổi kỳ vọng thành "Lucerna Light" khi test chỉ cần theme mặc định. Chọn cách giữ đúng ý test.

- [ ] **Step 4: Chạy test**

Chạy: `cargo test -p lucerna-workbench && cargo test -p lucerna-settings`
Kỳ vọng: PASS.

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(theme): follow the system color mode (window.autoDetectColorScheme)" -- crates/lucerna-settings crates/lucerna-workbench
```

### Task 3: Theme picker theo chế độ hệ thống

**Files:**
- Modify: `crates/lucerna-workbench/src/theme_picker.rs` (`show_theme_picker`, `on_theme_picker_event`, `set_theme_setting`, sắp xếp `names`)
- Test: `crates/lucerna-workbench/tests/theme_picker.rs` nếu đã có, nếu không thì `tests/config_files.rs` nơi các test picker hiện nằm

**Interfaces:**
- Consumes: `color_theme_setting_key`, `AppState.system_appearance` (Task 2).

- [ ] **Step 1: Viết test hỏng**

```rust
#[gpui::test] fn picker_placeholder_names_system_mode(cx)
// Mặc định + Light: placeholder của input == "Select Color Theme for System Light Mode".
// Sau simulate Dark: mở lại → "...System Dark Mode".
// autoDetect false → "Select Color Theme (detect system color mode disabled)".
#[gpui::test] fn picker_confirm_writes_preferred_setting(cx)
// Light, chọn "Lucerna Dark", Enter → settings.json có "workbench.preferredLightColorTheme": "Lucerna Dark",
// không có "workbench.colorTheme"; theme đang dùng == "Lucerna Dark".
#[gpui::test] fn picker_confirm_writes_color_theme_when_detect_off(cx)
// autoDetect false → ghi "workbench.colorTheme".
#[gpui::test] fn picker_lists_system_mode_themes_first(cx)
// Light: row_texts()[0] == "Lucerna Light".
#[gpui::test] fn appearance_change_during_preview_keeps_preview(cx)
// Mở picker, xem trước "Lucerna Dark" khi Light, simulate Dark rồi Light → theme vẫn là bản xem trước;
// Esc → theme == configured_theme_name hiện tại.
```

- [ ] **Step 2: Chạy test và thấy hỏng**

Chạy: `cargo test -p lucerna-workbench picker`
Kỳ vọng: FAIL.

- [ ] **Step 3: Cài đặt**

- `show_theme_picker`: placeholder là một trong ba chuỗi ở Global Constraints. `show_picker` nhận placeholder `&'static str` nên ba chuỗi là hằng.
- `names`: các theme có `appearance` khớp `system_appearance` lên trước (khi tắt auto-detect thì dùng appearance của theme đang dùng). Giữ thứ tự tên trong mỗi nhóm. `themes_with_appearance()` đã có sẵn.
- `Confirm` và `set_theme_setting(name, icons: false)`: khoá = `color_theme_setting_key(...)` thay cho `COLOR_THEME_KEY`. Icon theme không đổi.
- Sửa các test unit `set_string_setting` trong `theme_picker.rs` chỉ ở tên theme (đã làm ở Task 1). Hàm ghi chuỗi không đổi.

- [ ] **Step 4: Chạy test**

Chạy: `cargo test -p lucerna-workbench`
Kỳ vọng: PASS.

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(theme): picker writes the preferred theme for the system mode" -- crates/lucerna-workbench
```

### Task 4: Giấy phép, tài liệu, site

**Files:**
- Modify: `scripts/gen-licenses.py:31` (`ASSETS`)
- Modify: `THIRD_PARTY_NOTICES.md` (mục "Themes")
- Modify: `README.md:67`, `README.md:184`
- Modify: `site/index.html:181`, `site/styles.css:21,36` (chỉ chữ và comment nhắc One; không đổi màu site)

- [ ] **Step 1: Sửa**

- `ASSETS`: bỏ dòng One, thêm `("GitHub Primer colors (Lucerna themes)", "crates/lucerna-theme/assets/LICENSE-primer")`.
- `THIRD_PARTY_NOTICES.md`, mục Themes: `lucerna-theme` bundles Lucerna Dark/Light (`assets/lucerna.json`); màu chẩn đoán, git và ANSI dựa trên GitHub Primer / primer/github-vscode-theme, MIT, Copyright (c) GitHub Inc.; license ở `crates/lucerna-theme/assets/LICENSE-primer`.
- README: liệt kê "the Lucerna Dark and Lucerna Light themes"; ví dụ settings dùng `"Lucerna Dark"`, thêm một dòng `"window.autoDetectColorScheme": true` nếu khối ví dụ liệt kê setting theme.
- Site: "Lucerna Dark and Lucerna Light are built in and follow the system color mode." Comment CSS bỏ "One Dark".

- [ ] **Step 2: Kiểm**

Chạy: `python3 -I scripts/gen-licenses.py "$TMPDIR/licenses.md" && grep -c "GitHub Primer" "$TMPDIR/licenses.md"`
Kỳ vọng: chạy không lỗi, đếm được ≥ 1.
Chạy: `cargo deny check`
Kỳ vọng: `advisories ok, bans ok, licenses ok, sources ok`.
Chạy: `rg -n "One Dark|One Light|LICENSE-one|one\.json" --glob '!docs/**' .`
Kỳ vọng: không còn kết quả.

- [ ] **Step 3: Commit**

```bash
git commit -m "docs: credit GitHub Primer for the Lucerna theme, drop One" -- scripts/gen-licenses.py THIRD_PARTY_NOTICES.md README.md site/index.html site/styles.css
```

### Task 5: Ảnh trước/sau và ghi chú bàn giao

**Files:**
- Create: `docs/research/lucerna-theme-screenshots/{before,after}-{dark,light}.png`
- Create: ghi chú bàn giao trong thư mục ghi chú chung `/Users/duykhanh/Library/Application Support/Claude/side-session-notes/local_fcd9b997-826b-47a7-83f7-6b39fdb9b476/lucerna-theme-handover.md`

- [ ] **Step 1: Chụp "trước"**

Dựng app từ `main` (One) hoặc dùng `/Applications/Lucerna.app` nếu nó là bản trước theme mới. Mở repo Lucerna, mở một file Rust đang có diff git (sửa tạm một dòng rồi bỏ sau khi chụp), Explorer mở. Chụp cửa sổ ở chế độ tối và sáng (`workbench.colorTheme` "One Dark" / "One Light").

- [ ] **Step 2: Chụp "sau"**

`scripts/bundle-app.sh` từ nhánh này (tự lo profile). Cùng file, cùng kích thước cửa sổ. Đổi chế độ hệ thống tối/sáng để kiểm luôn việc tự đổi theme. Chụp hai ảnh.

- [ ] **Step 3: Ghi chú bàn giao**

Ghi chép lại mục "Ghi chú khi merge các nhánh song song" của spec, cộng tên commit của nhánh này.

- [ ] **Step 4: Commit**

```bash
git commit -m "docs: before/after screenshots of the Lucerna theme" -- docs/research/lucerna-theme-screenshots
```
