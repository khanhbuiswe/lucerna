# Nghiên cứu: theme mặc định (dark + light) cho Lucerna

Ngày: 2026-11-14. Mục đích: chuẩn bị spec cho cặp theme mặc định riêng của Lucerna, thay cho One Dark/One Light đang mượn từ Zed (`crates/lucerna-theme/assets/one.json`, giống hệt bản upstream `assets/themes/one/one.json` ở thời điểm kiểm tra).

Nguồn đã đọc: zed.dev/docs/themes, zed.dev/docs/reference/all-settings, mã nguồn Zed (`assets/themes/*`, `crates/theme/src/{schema,fallback_themes,default_colors}.rs`, `crates/ui/src/utils/apca_contrast.rs`). VS Code: code.visualstudio.com/docs/configure/themes, .../accessibility, mã nguồn `extensions/theme-defaults/` và `src/vs/workbench/services/themes/common/{themeConfiguration,workbenchThemeService}.ts`, `terminalConfiguration.ts`. Zed là GPL nên ở đây chỉ mô tả thiết kế, không chép mã.

## 1. Zed

### 1.1 Tổ chức theme có sẵn
- `assets/themes/` có ba họ: `one/` (One Dark, One Light), `ayu/` (Ayu Dark, Ayu Light, Ayu Mirage), `gruvbox/` (6 biến thể), kèm `LICENSES`.
- Mỗi file là một **theme family**: `$schema` (`https://zed.dev/schema/themes/v0.2.0.json`), `name`, `author`, `themes: [...]`. Mỗi theme con có `name`, `appearance` (`dark`/`light`), `style`.
- Trong binary còn có một theme dự phòng viết bằng mã (`fallback_themes.rs`, `zed_default_dark()`), cùng hàm điền màu trạng thái và màu UI mặc định khi file JSON thiếu khoá. Lucerna nên có cơ chế tương tự: theme thiếu khoá vẫn dùng được.

### 1.2 Bộ khoá `style` (One Dark có 141 khoá)
| Nhóm | Khoá tiêu biểu | Ghi chú |
|---|---|---|
| Bề mặt | `background`, `surface.background`, `elevated_surface.background`, `panel.background`, `editor.background`, `title_bar.background`, `status_bar.background`, `tab_bar.background`, `toolbar.background` | 3 tầng: nền app → surface → elevated (popover, menu). |
| Element | `element.{background,hover,active,selected,disabled}` | Nút, input có nền. |
| Ghost element | `ghost_element.{background,hover,active,selected,disabled}` | Phần tử không nền (item trong list, nút icon). |
| Text | `text`, `text.muted`, `text.placeholder`, `text.disabled`, `text.accent` | |
| Icon | `icon`, `icon.muted`, `icon.disabled`, `icon.placeholder`, `icon.accent` | |
| Border | `border`, `border.variant`, `border.focused`, `border.selected`, `border.transparent`, `border.disabled` | |
| Editor | `editor.{foreground,background,gutter.background,subheader.background,active_line.background,highlighted_line.background,line_number,active_line_number,invisible,wrap_guide,active_wrap_guide,document_highlight.read_background,…}` | 14 khoá. |
| Scrollbar, tab, pane, search | `scrollbar.thumb.*`, `tab.{active,inactive}_background`, `pane.focused_border`, `search.match_background`, `drop_target.background`, `link_text.hover` | |
| Terminal | `terminal.{background,foreground,bright_foreground,dim_foreground}`, `terminal.ansi.{black…white}` + `bright_*` + `dim_*` | 28 khoá. |
| Version control | `version_control.{added,modified,deleted,conflict_marker.ours,…}` | 7 khoá. |
| Trạng thái | `created, modified, deleted, conflict, renamed, ignored, hidden, unreachable, predictive, error, warning, info, hint, success` | Mỗi cái có 3 biến thể: `X`, `X.background`, `X.border`. |
| Players | `players: [{cursor, background, selection}]` × 8 | Player 0 là con trỏ/selection của chính người dùng; selection = màu accent alpha ~0x3d. |
| Accents | `accents: [...]` | Màu cho rainbow bracket/indent; One không khai báo (dùng mặc định). |
| Syntax | `syntax.<capture>: {color, font_style, font_weight}` | 46 capture: `comment`, `comment.doc`, `keyword`, `string`, `string.escape/regex/special`, `function`, `type`, `number`, `constant`, `boolean`, `variable`, `variable.special`, `property`, `punctuation.*`, `tag`, `attribute`, `title`, `emphasis*`, `link_*`, `diff.plus/minus`, `hint`, `predictive`… Tên trùng với capture của tree-sitter highlights, tra theo tiền tố dài nhất. |

### 1.3 Chọn theme theo giao diện hệ thống
- Setting `theme` nhận chuỗi (tên theme) hoặc object `{ "mode": "system" | "light" | "dark", "light": "...", "dark": "..." }`.
- Mặc định: `{ mode: "system", light: "One Light", dark: "One Dark" }`, nghĩa là **mặc định theo hệ thống**.
- Lệnh toggle (`cmd-k cmd-shift-t`) đổi chuỗi tĩnh sang dạng object rồi chỉ đổi `mode`.
- `theme_overrides` (theo tên theme) cho phép ghi đè từng khoá, kể cả `syntax` và `accents`.

### 1.4 Xử lý độ tương phản
- Zed dùng **APCA**, không dùng WCAG 2.x. `crates/ui/src/utils/apca_contrast.rs` có `apca_contrast(text, bg)` (Lc khoảng −108..106) và `ensure_minimum_contrast(fg, bg, min_lc)`. Hàm này chỉnh lightness trước, giữ hue và saturation, rồi mới chỉnh saturation, và chỉ về đen/trắng khi buộc phải vậy.
- Setting dùng nó: `terminal.minimum_contrast` (mặc định **45**, "tối thiểu cho chữ lớn") và `minimum_contrast_for_highlights` (mặc định **45**, cho chữ editor nằm trên nền highlight). Giá trị 0 tắt điều chỉnh.
- Ngưỡng ghi trong code: Lc 45 cho chữ lớn, 60 cho nội dung khác, 75 cho body text, 90 là mức nên đạt cho body text.
- Bản thân theme One **không** đạt chuẩn với comment (xem §3).

## 2. VS Code

### 2.1 Theme mặc định: đã đổi sang "2026"
- Trên nhánh `main` hiện tại, `ThemeSettingDefaults` là `COLOR_THEME_DARK = 'Dark 2026'`, `COLOR_THEME_LIGHT = 'Light 2026'`, `HC_DARK = 'Default High Contrast'`, `HC_LIGHT = 'Default High Contrast Light'`. File nằm ở `extensions/theme-defaults/themes/2026-{dark,light}.json` và kế thừa `dark_modern` / `light_modern`.
- Docs vẫn ghi Dark Modern / Light Modern là mặc định; trang accessibility còn ghi "Dark+", tức đã cũ hơn nữa.
- Chuỗi kế thừa: `2026-dark → dark_modern → dark_plus → dark_vs`, `light_modern → light_plus → light_vs`. Mỗi tầng thêm `colors` (UI) hoặc `tokenColors` (TextMate scope); tầng sau ghi đè tầng trước.
- `workbench.colorTheme` mặc định là theme dark (trên web là light).

### 2.2 Bảng màu chính
| Khoá | Dark Modern | Light Modern | 2026 Dark | 2026 Light |
|---|---|---|---|---|
| editor.background | #1F1F1F | #FFFFFF | #121314 | #FFFFFF |
| editor.foreground | #CCCCCC | #3B3B3B | #BBBEBF | #202020 |
| sideBar/statusBar/activityBar/titleBar/panel | #181818 | #F8F8F8 | #191A1B | #FAFAFD |
| tab.activeBackground | #1F1F1F | #FFFFFF | #121314 | #FFFFFF |
| editorWidget.background | #202020 | #F8F8F8 | #202122 | #FAFAFD |
| input.background | #313131 | #FFFFFF | #191A1B | #FFFFFF |
| border (widget/sideBar) | #313131 / #2B2B2B | #E5E5E5 | #2A2B2C | #E2E2E5 / #F0F1F2 |
| focusBorder | #0078D4 | #005FB8 | #3994BCB3 | #0069CC |
| button.background | #0078D4 | #005FB8 | #297AA0 | #0069CC |
| descriptionForeground (muted) | #9D9D9D | #3B3B3B | #8C8C8C | #606060 |

Nhận xét:
- Chrome (sidebar, status bar, activity bar, title bar) dùng **một màu** và tối hơn (dark) hoặc xám hơn (light) editor một bậc, phân tách bằng border 1px. Kiểu "Modern" bỏ status bar xanh đậm.
- 2026 Dark nền gần đen, xám **hơi ngả lạnh** (#121314, #191A1B). 2026 Light có xám pha tím-lam nhẹ (#FAFAFD, #E2E2E5). Accent chuyển sang xanh lơ dịu (#297AA0).
- Token 2026 lấy nhiều từ bảng GitHub Primer: string #a5d6ff / #0a3069, function #d2a8ff / #8250df, comment #8b949e / #6e7781.

### 2.3 Setting liên quan
| Setting | Mặc định | Ý nghĩa |
|---|---|---|
| `workbench.colorTheme` | Dark 2026 (web: Light 2026) | Theme đang dùng. |
| `window.autoDetectColorScheme` | **false** | Bật thì chọn theme theo chế độ sáng/tối của OS. |
| `workbench.preferredDarkColorTheme` / `preferredLightColorTheme` | Dark 2026 / Light 2026 | Dùng khi bật auto-detect. |
| `window.autoDetectHighContrast` | **true** | Theo chế độ high-contrast của OS. |
| `workbench.preferredHighContrastColorTheme` / `…HighContrastLightColorTheme` | Default High Contrast (Light) | |
| `workbench.colorCustomizations`, `editor.tokenColorCustomizations`, `editor.semanticTokenColorCustomizations` | `{}` | Ghi đè, có thể theo theme bằng khoá `"[Tên theme]"`. |
| `terminal.integrated.minimumContrastRatio` | **4.5** (WCAG AA) | Đổi luminance của chữ terminal tới khi đạt tỉ lệ, theo **WCAG 2.x**. |

### 2.4 Chính sách contrast
- Docs không cam kết WCAG cho theme mặc định. Họ dựa vào high-contrast theme (có trên mọi nền tảng, tự bật theo OS) và gợi ý theme bên thứ ba đạt WCAG 2.1.
- Trên thực tế (§3), mọi màu token chính của Modern và 2026 đều ≥ 4.5:1. Chỉ số dòng của Dark Modern đạt 3.59.

## 3. Độ tương phản WCAG 2.x so với `editor.background`

Script: `/private/tmp/claude-501/-Users-duykhanh-Projects-nosync-lucerna/de749e45-9d8b-4878-bf3c-b6ae4400ac50/scratchpad/contrast.py`, chạy bằng `python3 -I`. Màu VS Code được resolve qua chuỗi `include`, lấy scope khớp dài nhất: keyword = `keyword.control`, string = `string.quoted`, type = `entity.name.type`. Với Zed, "muted" là `text.muted`; với VS Code là `descriptionForeground`. ✗ = dưới 4.5:1, (L) = chỉ đạt 3:1 (AA cho chữ lớn).

| Theme (bg) | text | muted | line nr | comment | keyword | string | function | type | number |
|---|---|---|---|---|---|---|---|---|---|
| One Dark (#282c33) | 10.57 | 6.37 | 1.97 ✗ | **2.32 ✗** | 4.33 (L) | 6.98 | 5.93 | 5.97 | 5.15 |
| One Light (#fafafa) | 14.67 | 6.80 | 1.98 ✗ | **2.41 ✗** | 4.88 | 3.03 (L) | 3.80 (L) | 3.99 (L) | 4.00 (L) |
| Dark Modern (#1F1F1F) | 10.26 | 6.08 | 3.59 (L) | 4.95 | 5.92 | 6.24 | 11.66 | 8.09 | 9.70 |
| Light Modern (#FFFFFF) | 11.20 | 11.20 | 4.59 | 5.14 | 5.46 | 7.85 | 6.10 | 4.59 | 4.60 |
| 2026 Dark (#121314) | 9.95 | 5.53 | 5.21 | 6.05 | 6.68 | 12.10 | 9.55 | 9.13 | 10.95 |
| 2026 Light (#FFFFFF) | 16.29 | 6.29 | 6.29 | 4.55 | 5.46 | 12.81 | 5.05 | 4.59 | 4.60 |
| Ayu Dark (#0d1016) | 10.13 | 5.44 | 2.22 ✗ | 3.30 (L) | 8.39 | 11.53 | 10.72 | 9.60 | 9.65 |
| Ayu Mirage (#242835) | 8.95 | 5.21 | 2.20 ✗ | 2.55 ✗ | 7.98 | 12.79 | 10.22 | 8.46 | 9.12 |
| Ayu Light (#fcfcfc) | 6.10 | 3.21 (L) | 2.05 ✗ | 2.13 ✗ | 2.29 ✗ | 2.42 ✗ | 1.89 ✗ | 2.84 ✗ | 3.29 (L) |

APCA |Lc| tham khảo, theo đúng thang Zed dùng: comment One Dark ≈ 17.5, One Light ≈ 46.5, Dark Modern ≈ 39.1, 2026 Dark ≈ 43.4, Light Modern ≈ 74.6, 2026 Light ≈ 71.6. Text One Dark ≈ 83, 2026 Dark ≈ 67.

Kết luận:
- **One Dark/Light** (mặc định hiện tại của Lucerna) có comment khoảng 2.3–2.4:1 và số dòng khoảng 2:1, rất mờ. One Light còn có 4 màu token chỉ đạt 3:1.
- Ayu Light trượt gần hết. Ayu Dark và Mirage có token sáng nhưng comment và số dòng mờ.
- VS Code Modern và 2026 là nhóm duy nhất có **mọi token chính ≥ 4.5:1**, kể cả comment. Với 2026 thì kể cả số dòng. Đây là chỉ dấu VS Code có chủ ý giữ AA. 2026 Dark nâng comment từ 4.95 lên 6.05 và số dòng từ 3.59 lên 5.21 so với Dark Modern.

## 4. Lucerna chọn gì

**Định dạng: theo Zed.** Theme mặc định của Lucerna là một file family JSON theo schema Zed v0.2.0 (`themes[]`, `appearance`, `style`), dùng đúng bộ khoá ở §1.2: surfaces/element/ghost_element/text/icon/border, `editor.*`, `terminal.ansi.*`, `version_control.*`, trạng thái ×3, `players`, `accents`, `syntax`. Đặt cạnh `one.json` trong `crates/lucerna-theme/assets/`. Giữ One Dark/One Light làm theme phụ để chọn, và giữ fallback trong code khi thiếu khoá.

**Hành vi và setting: theo VS Code.**
- `workbench.colorTheme` (mặc định là tên theme dark của Lucerna), `workbench.preferredDarkColorTheme`, `workbench.preferredLightColorTheme`.
- `window.autoDetectColorScheme` là một hành vi **riêng**, cần quyết định trong spec. VS Code mặc định `false`, Zed mặc định `mode: system`. Với một editor tối giản trên macOS, bật mặc định có lợi cho người dùng, nhưng nguyên tắc dự án là hành vi theo VS Code. Spec nên chọn rõ một trong hai và ghi lý do. Nếu theo VS Code thì để `false`.
- Có thể làm sau: `workbench.colorCustomizations` / `editor.tokenColorCustomizations` (ghi đè theo `"[Tên theme]"`), `window.autoDetectHighContrast` cùng theme high-contrast, và `terminal.integrated.minimumContrastRatio` (4.5, WCAG) cho terminal.

**Thực hành đáng học:**
1. **Comment ≥ 4.5:1** (WCAG) hoặc Lc khoảng 60 (APCA). Comment phải lùi xuống nhưng vẫn đọc được. Đây là điểm yếu lớn nhất của One.
2. **Mọi token chính ≥ 4.5:1**, kể cả trên nền light (One Light và Ayu Light trượt ở đây). Số dòng và `text.muted` ≥ 4.5; số dòng ít nhất 3:1.
3. **Xám trung tính pha nhẹ về hue accent.** One dùng xám xanh ~220°, 2026 Light dùng #FAFAFD/#E2E2E5. Độ bão hoà rất thấp để chrome hài hoà với accent mà không bị "màu".
4. **Ít tầng bề mặt, chênh lệch nhỏ.** Chrome (sidebar, status, title, tab bar) dùng một màu, khác editor 1 bậc; phân tách bằng border 1px chứ không bằng khối màu đậm. Bộ ba nền app → surface → elevated của Zed cho popover và menu.
5. **Một accent duy nhất** cho focus border, nút chính, `text.accent`, player 0 và selection (accent với alpha khoảng 0x30–0x40), giống 2026 và One.
6. **Mỗi màu trạng thái có 3 biến thể** (`X`, `X.background`, `X.border`). Created/modified/deleted dùng chung với `version_control.*` để git gutter, explorer và diff nhất quán.
7. **Cặp dark/light cùng hue-map:** cùng hue cho keyword, string, function…, chỉ đổi lightness. Đổi theme vẫn giữ được trí nhớ màu.
8. **Kiểm tra contrast tự động:** thêm test trong `lucerna-theme` tính tỉ lệ WCAG cho các cặp token/nền quan trọng để giữ ngưỡng khi chỉnh màu. Có thể thêm APCA về sau nếu cần điều chỉnh động như Zed (`minimum_contrast_for_highlights`).
