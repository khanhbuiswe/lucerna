# Theme Lucerna — thiết kế

Ngày: 2026-11-14. Trạng thái: đã duyệt.

## Ý định

Lucerna đang mặc định dùng One Dark / One Light lấy từ Zed. Trước bản đầu
tiên, Lucerna cần một theme của riêng mình. Người dùng của Lucerna đến từ
Claude (app desktop, Claude Code) và đọc diff trên GitHub mỗi ngày. Vì vậy theme
mới phải cho họ cảm giác **quen ngay**: màu code theo code theme "Claude" của
Claude desktop, các màu còn thiếu (chẩn đoán, git, terminal) theo GitHub. Bản
sắc của Lucerna nằm ở tên theme và ở accent xanh thương hiệu `#4f8cff`
(`assets/brand/README.md`).

## Mục tiêu

- Hai theme **Lucerna Dark** và **Lucerna Light** trong một file JSON định dạng
  Zed, là theme mặc định và là theme duy nhất đi kèm Lucerna.
- Mọi màu dùng cho chữ đạt **WCAG AA, 4.5:1** trên nền nó hiện lên, kể cả
  comment và số dòng. Có test bảo đảm điều này.
- Phủ **đủ khoá màu** của schema Zed mà One có (141 khoá style, `players`,
  `syntax`), gồm `terminal.ansi.*`, `version_control.*` và các màu trạng thái
  `created/modified/deleted/conflict/ignored/renamed`. Nhờ vậy các nhánh Git và
  Terminal merge vào là có màu ngay.
- Lucerna tự đổi theme theo chế độ sáng/tối của hệ thống
  (`window.autoDetectColorScheme`, **mặc định bật**).

## Không làm

- Không giữ One. `one.json` và `LICENSE-one` bị xoá. Ai muốn One thì thả file
  vào thư mục themes của người dùng hoặc cài extension; cả hai đã chạy được.
- Không làm theme high contrast hay `window.autoDetectHighContrast`.
- Không làm rainbow brackets. Claude tô ngoặc theo cấp lồng, nhưng Lucerna chỉ
  có một màu cho `punctuation.bracket`.
- Không thêm lệnh "Toggle Light/Dark Theme".
- Không đổi màu logo.

## Phản mục tiêu

- Theme trông như One hay Zed được đổi tên: người dùng phải nhận ra màu Claude.
- Chỗ nào màu chữ dưới 4.5:1 (trừ chữ disabled, mà WCAG miễn).
- UI loè loẹt: accent chỉ dùng cho focus, con trỏ, link và chấm chưa lưu.
  Khung UI gần như đơn sắc.

## Ràng buộc

- Định dạng JSON và tên khoá theo Zed (giao diện theo Zed). Settings và hành vi
  theo VS Code (`docs/design-principles.md` §1).
- Giấy phép: chỉ học bảng màu của Claude (không phải mã mở) bằng cách đo màu
  trên ảnh chụp; không có file nào của Claude được chép. Màu lấy từ GitHub
  Primer / `primer/github-vscode-theme` (MIT) phải ghi nguồn trong
  `THIRD_PARTY_NOTICES.md` và `scripts/gen-licenses.py`.
- Chỉ `lucerna-workbench` dùng gpui. Việc đọc appearance của hệ thống nằm ở
  workbench; `lucerna-theme` vẫn là dữ liệu thuần.
- Không có giả định chỉ có macOS: GPUI có `window_appearance()` trên cả ba nền
  tảng.

## Tham khảo

Chi tiết ở `docs/research/2026-11-14-default-theme.md`. Tóm tắt:

| | Zed | VS Code | Lucerna chọn |
|---|---|---|---|
| Định dạng | Family JSON `$schema` v0.2.0, `themes[]` có `appearance` + `style` | `colors` + `tokenColors` (TextMate) | Zed |
| Theme mặc định | One Dark / One Light | Dark Modern / Light Modern (main mới: Dark 2026 / Light 2026) | Lucerna Dark / Lucerna Light |
| Theo hệ thống | Setting `theme` mặc định `{mode: "system", light, dark}` | `window.autoDetectColorScheme`, mặc định `false`; `workbench.preferredDarkColorTheme` / `preferredLightColorTheme` | Tên setting của VS Code, mặc định bật như Zed |
| Tương phản | APCA, `minimum_contrast` cho terminal | `terminal.integrated.minimumContrastRatio` 4.5 | WCAG 4.5 ngay trong bảng màu, có test |
| Comment | One Dark 2.32:1, One Light 2.41:1 | Dark Modern 4.95:1 | ≥ 4.5:1 |

Khung UI theo Zed: các vùng (sidebar, tab bar, status bar, editor) cùng một màu
nền và chỉ tách nhau bằng viền 1 px. Claude desktop cũng làm như vậy (sidebar
cùng màu với editor).

Hành vi picker khi bật tự theo hệ thống, theo VS Code
(`themes.contribution.ts`, `themeConfiguration.ts`):
- `workbench.colorTheme` bị bỏ qua. Theme đang dùng là
  `preferredDarkColorTheme` hoặc `preferredLightColorTheme` tuỳ chế độ hệ thống.
- Picker có tiêu đề "Select Color Theme for System Dark Mode" (hoặc Light Mode)
  và ghi lựa chọn vào setting preferred ứng với chế độ hiện tại.
- Khi tắt tự theo hệ thống, tiêu đề picker là
  "Select Color Theme (detect system color mode disabled)", và picker ghi vào
  `workbench.colorTheme` như hiện nay.

## Cách làm đã cân nhắc

1. **Một file JSON định dạng Zed, viết tay, có test tương phản** (chọn). Giống
   cách Zed tổ chức theme. Người dùng copy file làm mẫu cho theme của mình được.
   Test chặn việc tụt tương phản và việc thiếu khoá.
2. Bảng màu khoảng 30 token cộng script sinh JSON. Nhất quán hơn, nhưng phải
   giữ thêm một script và một bước build cho một file ít khi sửa. Bỏ.
3. Định nghĩa theme trong code Rust. Lệch khỏi Zed, và người dùng không có file
   mẫu. Bỏ.

Màu code: **giữ gần như y hệt Claude**, chỉ chỉnh khi trượt AA. Người dùng chọn
cách này để người đến từ Claude thấy quen ngay.

## Thiết kế

### Bảng màu

Các màu dưới đây đo từ ảnh chụp code theme "Claude Light" / "Claude Dark" của
Claude desktop (`[C]`), lấy từ GitHub Primer (`[G]`), hoặc là màu thương hiệu
(`[B]`). Tỷ lệ trong ngoặc là tương phản WCAG so với nền editor. Hàng không đánh
dấu là quyết định của builder (xem cuối spec).

**Nền và chữ**

| Vai trò | Dark | Light |
|---|---|---|
| Nền chung (editor, sidebar, tab bar, status bar, title bar) | `#191919` [C] | `#ffffff` [C] |
| Nền nổi (popover, Quick Open, menu) | `#222222` | `#ffffff` + viền |
| Dòng hiện tại | `#1f1f1f` (Claude `#252525`, nhạt bớt để comment đạt 4.5) | `#fafafa` |
| Hàng đang chọn (Explorer, list) | `#2a2a2a` (Claude `#302f2f`, nhạt bớt) | `#ebebeb` (Claude `#e6e6e6`, nhạt bớt) |
| Hàng dưới chuột | `#252424` [C] | `#f0f0f0` |
| Viền / viền phụ | `#2e2e2e` / `#252525` | `#e5e5e5` / `#ededed` |
| Chữ UI | `#efefef` [C] (15.3) | `#191919` [C] |
| Chữ editor | `#d3d7de` [C] (12.2) | `#2b303b` [C] (13.2) |
| Chữ mờ (`text.muted`, số dòng) | `#9c9c9c` [C] (6.4) | `#666666` [C] (5.7) |
| Số dòng hiện tại | `#efefef` | `#191919` |
| Accent: viền focus, con trỏ, chấm chưa lưu | `#4f8cff` [B] (5.5) | `#4f8cff` [B] (viền, con trỏ: 3.2, đủ 3:1 cho thành phần không phải chữ) |
| Accent dạng chữ (`text.accent`, link, `renamed`) | `#5c96ff` (xanh thương hiệu sáng hơn một chút, 6.0; 4.96 trên hàng chọn) | `#1a5dcc` (xanh thương hiệu đậm hơn, 5.9; 5.06 trên hàng chọn) |
| Vùng chọn | `#4f8cff` alpha ≈ 0x40 | `#4f8cff` alpha ≈ 0x33 |

**Syntax**

| Capture | Dark | Light |
|---|---|---|
| `keyword`, `preproc`, `variable.special`, `label` | tím `#cc7bf4` [C] (6.5) | tím `#8101c2` [C] (7.7) |
| `function` (gồm method, macro) | xanh `#70b8ff` [C] (8.4) | xanh `#0051c2` [C] (7.1) |
| `type`, `constructor`, `enum`, `variant`, `variable.parameter`, `attribute`, `selector` | cam `#f7ab5f` [C] (9.5) | cam đất `#b34a02` [C] (5.4) |
| `string`, `text.literal` | xanh lá `#9be963` [C] (11.9) | xanh lá `#027900` [C] (5.6) |
| `number`, `boolean`, `constant`, `string.escape`, `string.regex`, `string.special`, `link_uri` | cyan `#5eeded` [C] (12.4) | teal `#007a7a` (Claude `#008080`, đậm một chút để đạt 4.5 trên dòng hiện tại) |
| `property`, `tag` | hồng đỏ `#f37c85` [C] (6.7) | đỏ `#b80918` [C] (6.8) |
| `punctuation.bracket` | vàng `#ffd012` [C] (12.0) | nâu `#945c00` [C] (5.5) |
| `comment`, `comment.doc` | `#818898` [C] (4.95; 4.64 trên dòng hiện tại) | `#677080` (Claude `#6d7686`, đậm một chút; 4.78 trên dòng hiện tại) |
| `title` (heading markdown) | xanh hàm, đậm | xanh hàm, đậm |
| `link_text` | accent dạng chữ | accent dạng chữ |
| `emphasis` / `emphasis.strong` | nghiêng / đậm | nghiêng / đậm |
| `variable`, `namespace`, `operator`, `punctuation` còn lại, `embedded`, `primary` | chữ editor | chữ editor |
| `hint`, `predictive` | chữ mờ, nghiêng | chữ mờ, nghiêng |
| `diff.plus` / `diff.minus` | màu `created` / `deleted` | màu `created` / `deleted` |

**Chẩn đoán** [G]: error `#f85149` / `#d1242f`, warning `#d29922` / `#9a6700`,
info = accent dạng chữ, hint `#9198a1` / `#59636e`, success `#3fb950` /
`#1a7f37`. Nền `<severity>.background` là màu đó với alpha thấp (dark) hoặc tint
nhạt (light). Viền `<severity>.border` đậm hơn nền một bậc.

**Git**

| Khoá | Dark | Light |
|---|---|---|
| `modified` (Explorer) | `#e1c08d` [C] | `#895503` [C] |
| `created` | `#7ee787` [G] | `#116329` [G] |
| `deleted` | `#ff7b72` [G] | `#c21d29` (Primer `#cf222e`, đậm một chút cho hàng chọn) |
| `conflict` | `#f0883e` [G] | `#a84400` (Primer `#bc4c00`, đậm một chút cho hàng chọn) |
| `renamed` | accent dạng chữ | accent dạng chữ |
| `ignored`, `hidden` | chữ mờ | chữ mờ |
| `version_control.added` (gutter) | `#37ac42` [C] | `#218635` [C] |
| `version_control.deleted` | `#ff707b` [C] | `#b7224c` [C] |
| `version_control.modified` | `#4f8cff` [B] | `#4f8cff` [B] |
| Nền dòng thêm / dòng xoá (`editor.diff_hunk.*`, `created.background`, `deleted.background`) | `#253a24` / `#2e1e1f` [C] | `#e7f3e7` / `#feeeef` [C] |
| Chữ thêm / chữ xoá trong dòng (`version_control.word_added` / `word_deleted`) | `#2b5a2c` / `#782633` [C] | `#cce7ce` / `#f8d2d8` [C] |

**Terminal**: `terminal.ansi.*` (thường, bright, dim) lấy theo bảng ANSI của
`primer/github-vscode-theme` [G] cho GitHub Dark và GitHub Light. Nền và chữ
terminal dùng nền và chữ editor.

**Players**: player 0 là con trỏ accent cùng vùng chọn ở trên. Bảy player còn
lại lấy các màu syntax để Zed schema đủ, Lucerna hiện chỉ đọc player 0.

### Quy tắc tương phản (được test)

`lucerna-theme` có test tính tương phản WCAG 2.x cho cả hai theme:

- Chữ UI, chữ mờ, `text.placeholder`, `text.accent`, số dòng (thường và hiện
  tại), mọi màu syntax, màu chẩn đoán, màu trạng thái git và
  `terminal.foreground` đạt ≥ 4.5:1 trên `editor.background`.
- Chữ UI, chữ mờ và màu trạng thái git đạt ≥ 4.5:1 trên `element.selected`.
  Đây là chỗ Explorer vẽ tên file trên hàng đang chọn. Màu nào trượt thì
  builder chỉnh màu đó (và cập nhật spec), không hạ ngưỡng.
- Mọi màu syntax đạt ≥ 4.5:1 trên `editor.active_line.background`.
- Accent, `border.focused` và `version_control.*` đạt ≥ 3:1 (thành phần không
  phải chữ, WCAG 1.4.11).
- Không test `text.disabled`, `terminal.ansi.black` (dark), `terminal.ansi.white`
  (light) và các màu `dim_*`.

Một test khác so tập khoá của mỗi theme Lucerna với danh sách khoá Zed đầy đủ
(chép một lần từ `one.json` vào test trước khi xoá file). Theme thiếu khoá thì
test hỏng.

### Đổi file và mặc định

- Thêm `crates/lucerna-theme/assets/lucerna.json` (family "Lucerna"). Xoá
  `one.json` và `LICENSE-one`. Comment đầu file JSON (jsonc cho phép) ghi nguồn
  màu: Claude (đo từ ảnh) và GitHub Primer (MIT).
- `registry.rs`: nạp `lucerna.json`. Đổi `DEFAULT_THEME` thành "Lucerna Dark"
  và thêm hằng "Lucerna Light". Doc comment không còn nói "One Dark".
- `UiColors::fallback` giữ nguyên vai trò (màu cho theme người dùng thiếu khoá).
  Builder có thể đổi giá trị cho khớp Lucerna, nhưng không bắt buộc.
- `THIRD_PARTY_NOTICES.md`: thay mục One bằng mục GitHub Primer. Thêm file
  `crates/lucerna-theme/assets/LICENSE-primer` (MIT, nguyên văn từ
  `primer/github-vscode-theme`).
- `scripts/gen-licenses.py`: bỏ dòng One trong `ASSETS`, thêm dòng Primer.
  Chạy `python3 -I scripts/gen-licenses.py <tệp tạm>` để kiểm tra.

### Settings

Trong `default-settings.json`, giữ thứ tự và comment theo phong cách file:

```jsonc
// If enabled, automatically select a color theme based on the system color mode:
// `workbench.preferredDarkColorTheme` when it is dark, `workbench.preferredLightColorTheme` when it is light.
"window.autoDetectColorScheme": true,
// The color theme used when `window.autoDetectColorScheme` is disabled.
"workbench.colorTheme": "Lucerna Dark",
// The color theme used when the system color mode is dark and `window.autoDetectColorScheme` is enabled.
"workbench.preferredDarkColorTheme": "Lucerna Dark",
// The color theme used when the system color mode is light and `window.autoDetectColorScheme` is enabled.
"workbench.preferredLightColorTheme": "Lucerna Light",
```

Lucerna khác VS Code ở giá trị mặc định của `window.autoDetectColorScheme`
(`true` thay vì `false`). Lý do: người dùng muốn vậy, và Zed cũng mặc định theo
hệ thống.

### Chọn theme đang dùng

`apply_configured_theme` (`config_files.rs`) chọn tên theme như sau:

1. Nếu `window.autoDetectColorScheme` là `true`: đọc appearance của hệ thống
   (`cx.window_appearance()`). `Dark`/`VibrantDark` dùng
   `workbench.preferredDarkColorTheme`, `Light`/`VibrantLight` dùng
   `workbench.preferredLightColorTheme`.
2. Nếu là `false`: dùng `workbench.colorTheme`.
3. Tên không có trong registry thì dùng "Lucerna Dark" như hiện nay
   (`get_or_default`).

Workbench theo dõi khi appearance của hệ thống đổi (`observe_window_appearance`
trên cửa sổ, hoặc tương đương ở mức app) rồi gọi lại `apply_configured_theme`.
Chỉ vẽ lại khi theme thực sự đổi (đã có kiểm tra `Arc::ptr_eq`). Đổi
`window.autoDetectColorScheme` trong settings có hiệu lực ngay, như các setting
theme khác.

Trạng thái lúc khởi động: theme phải đúng ngay từ khung hình đầu, không nháy
theme tối rồi mới sang sáng.

### Theme picker

- Tự theo hệ thống bật: tiêu đề là "Select Color Theme for System Dark Mode" hoặc
  "… for System Light Mode". Phần xem trước giữ như hiện nay. Enter ghi vào
  `workbench.preferredDarkColorTheme` hoặc `workbench.preferredLightColorTheme`
  ứng với chế độ hiện tại. Esc hoàn lại theme cũ.
- Tự theo hệ thống tắt: tiêu đề "Select Color Theme (detect system color mode
  disabled)". Enter ghi `workbench.colorTheme` như hiện nay.
- Danh sách xếp theme cùng chế độ với hệ thống lên trước, như VS Code.

### Test cũ

Khoảng 15 file test trong `lucerna-workbench/tests` và `theme_picker.rs` ghi cứng
"One Dark" / "One Light". Chúng đổi sang "Lucerna Dark" / "Lucerna Light". Các
test của `config_files.rs` về `workbench.colorTheme` cần đặt
`window.autoDetectColorScheme: false` hoặc kiểm tra theo appearance mà nền tảng
test trả về. Builder chọn cách nào gọn hơn. Thêm test cho: chọn theo appearance,
đổi appearance lúc chạy, picker ghi đúng setting preferred.

### Ghi chú khi merge các nhánh song song

- **Git** (`claude/confident-murdock-5e4f69`): nhánh này thêm `vc_*`,
  `status_*`, `diff_*` vào `UiColors` cùng fallback màu VS Code. Theme Lucerna
  đã có đủ các khoá Zed tương ứng nên không phải sửa theme. Nếu test của nhánh
  đó dùng "One Dark" thì phải đổi tên khi merge.
- **Terminal** (`claude/epic-hellman-a76cab`): thêm `TerminalColors` đọc
  `terminal.*`. Theme Lucerna có đủ khoá này. Test `tests/terminal.rs` của nhánh
  đó nếu dựa vào màu cụ thể của One thì phải sửa khi merge.
- **Scrollbar** (`claude/objective-cannon-ee3762`): có thể đọc thêm
  `scrollbar.thumb.hover_background` hoặc khoá tương tự. Theme Lucerna có các khoá
  `scrollbar.*` của Zed.

Ghi các ghi chú này vào ghi chú bàn giao trong thư mục ghi chú chung khi xong.

### Ảnh so sánh

Chụp app trước và sau ở cả chế độ tối và sáng, cùng một file Rust có diff git
và cùng Explorer mở. Lưu ở `docs/research/lucerna-theme-screenshots/`
(`before-dark.png`, `after-dark.png`, `before-light.png`, `after-light.png`).

## Để builder quyết định

- Giá trị chính xác của các màu không đánh dấu `[C]`/`[G]`/`[B]`: nền nổi, viền,
  hover, các alpha, `ghost_element.*`, `element.active`, `scrollbar.*`,
  `drop_target`, `search.match_background`, `editor.document_highlight.*`,
  `editor.wrap_guide`, `editor.invisible`, `title_bar.inactive_background`,
  `pane.focused_border`, `panel.focused_border`, `players` 1–7, `accents`. Ràng
  buộc duy nhất: các quy tắc tương phản ở trên, và giữ khung UI trung tính.
- Map syntax cho các capture hiếm không có trong bảng: theo màu gần nhất về
  ý nghĩa.
- Có cập nhật `UiColors::fallback` cho khớp Lucerna hay không.

## Quyết định đã chốt khi soát spec

- **Nạp theme từ extension lúc khởi động:** lần nạp đầu chỉ đọc các file theme
  extension chứa theme sẽ dùng. Khi tự theo hệ thống bật, đó là theme preferred
  ứng với appearance hiện tại. Nạp cả hai theme preferred, để đổi chế độ hệ thống
  không phải chờ.
- **Không nháy lúc khởi động:** `watch_config` chạy trước khi mở cửa sổ, và
  `cx.window_appearance()` dùng được ở mức app. Theme được chọn ngay ở đó.
- **Tiêu đề picker:** picker chưa có hàng tiêu đề, nên chữ của VS Code đặt ở
  placeholder của ô nhập, giống quick pick của VS Code.
- **Test:** nền tảng test của GPUI luôn báo Light. Các test cũ chờ "Lucerna Dark"
  đặt `window.autoDetectColorScheme: false`. Test mới cho phần tự theo hệ thống
  đặt appearance giả, hoặc kiểm tra theo Light.
- **Chỉnh màu cho đạt quy tắc tương phản:** soát spec phát hiện vài màu trượt
  (comment và teal trên dòng hiện tại, `renamed`/`deleted`/`conflict` trên hàng
  chọn). Các bảng trên đã có giá trị đã chỉnh. Dòng hiện tại và hàng chọn nhạt
  hơn Claude một chút, thay vì làm tối các màu chữ.
