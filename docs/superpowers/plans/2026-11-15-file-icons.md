# Icon file có màu (Seti) và icon trên tab: plan triển khai

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Đưa icon theme Seti của VS Code vào làm icon theme mặc định, có màu theo loại file và màu riêng cho theme sáng. Thêm icon file lên tab editor, kèm setting `workbench.editor.showIcons`.

**Architecture:**
- `scripts/gen-seti.py` chạy một lần. Nó đọc seti-ui và vscode đã tải về, rồi sinh SVG đã chuẩn hóa viewBox cùng hai file Rust dữ liệu, cả hai commit vào repo.
- `lucerna-theme` thêm loại theme "Seti" với thứ tự khớp riêng giống VS Code, và một biến thể `IconSource::Glyph` mang tên glyph cùng màu tối/sáng.
- `lucerna-workbench` nhúng SVG và tô màu theo appearance của color theme. Explorer, các danh sách file và tab đều vẽ qua `ui::file_icon_sized`.

**Tech Stack:**
- Rust, GPUI (`svg()` dùng SVG làm mặt nạ rồi tô `text_color`).
- Python 3 cho script sinh, dùng `svgelements` (MIT) để tính khung vẽ. Thư viện này chỉ cài trong venv tạm, không phải dependency của app.

**Spec:** `docs/superpowers/specs/2026-11-15-file-icons-design.md`. Tham khảo thêm: `docs/research/2026-11-15-file-icons.md`.

## Global Constraints

- Làm trên worktree nhánh `claude/file-icons`. Không merge, không push.
- Cargo: `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"`. Dùng thư mục `target` chung; **không** đặt `CARGO_TARGET_DIR` hay `--target-dir`.
- Chạy `scripts/prune-target.sh` trước mỗi lần build hay `cargo test`. Trước `cargo test` thì xóa `target/release`, trừ khi có cargo của session khác đang chạy (`pgrep -x cargo`). Ổ đĩa phải còn ≥ 1,2 GB.
- Chỉ `lucerna-workbench` được dùng gpui. `lucerna-theme` không giữ byte SVG.
- Không có giả định riêng cho macOS ngoài code platform.
- Ghim nguồn: seti-ui commit `2d6c5e68` (commit mà `vs-seti-icon-theme.json` ghi). Lấy vscode ở một commit cụ thể và ghi commit đó vào đầu file sinh ra.
- Màu theme sáng: tương phản WCAG ≥ 3:1 với `#ffffff`. Theme tối dùng đúng màu VS Code.
- Tên theme là `"Seti"`, bí danh là `"vs-seti"`. Setting `workbench.iconTheme` mặc định là `"Seti"`, `workbench.editor.showIcons` mặc định là `true`.
- License: MIT OR Apache-2.0; `cargo deny check` phải qua. Commit kèm trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Stage file bằng đường dẫn cụ thể.
- Zed là GPL: không chép code.

## Review Focus

1. **Tên viết hoa khác thường** (`DOCKERFILE`, `Readme.MD`, `.ENV`) phải khớp như VS Code, tức không phân biệt hoa thường. Test nằm ở Task 2.
2. **Tên file nhiều dấu chấm hoặc bắt đầu bằng dấu chấm** (`.env`, `.env.local`, `a.d.ts`, `x.test.ts`, `archive.tar.gz`) phải ra icon của đuôi dài nhất mà Seti có, và không bao giờ panic. Test nằm ở Task 2.
3. **Đổi color theme sáng/tối khi app đang chạy** thì màu icon trên Explorer và tab đổi theo ngay, không cần mở lại. Test nằm ở Task 4.
4. **Người dùng đã đặt `"workbench.iconTheme": "Lucide"`** vẫn giữ Lucide sau khi nâng cấp. Tên lạ thì ra Seti mà không ghi đè setting. Test nằm ở Task 3.
5. **Tab của file bị xóa, tab preview (chữ nghiêng) và tab diff** vẫn có icon file. Tắt `showIcons` thì không còn icon nào trên tab, kể cả tab Search. Test nằm ở Task 5.

---

### Task 0: Worktree và ảnh "trước"

**Files:**
- Create: `docs/research/file-icons-screenshots/before-dark.png`, `before-light.png`

- [ ] **Step 1: Tạo worktree.** Tạo `claude/file-icons` từ `main` bằng skill `superpowers:using-git-worktrees`, đặt dưới `.claude/worktrees/`. Chép spec, research và plan này (chưa commit trên main) vào worktree, rồi commit trên nhánh: `git add docs/superpowers/specs/2026-11-15-file-icons-design.md docs/research/2026-11-15-file-icons.md docs/superpowers/plans/2026-11-15-file-icons.md`, `git commit -m "docs(file-icons): spec, research, plan" -- <3 đường dẫn>`. Không commit những file này lên main.
- [ ] **Step 2: Dựng thư mục mẫu.** Tạo `/private/tmp/.../scratchpad/icons-demo/` (trong scratchpad của session) gồm: `src/app.ts`, `src/app.test.ts`, `src/main.rs`, `Dockerfile`, `docker-compose.yml`, `.env`, `.env.example`, `.gitignore`, `package.json`, `tsconfig.json`, `README.md`, `LICENSE`, `Cargo.toml`, `notes.txt`, `logo.png` (ảnh 1×1), và `Makefile`.
- [ ] **Step 3: Chụp ảnh trước.** Build debug từ code main hiện tại (chưa sửa gì), chạy `luc <thư mục mẫu>`, mở tab `app.ts`, `Dockerfile` và `.env`, bung thư mục `src`. Chụp cửa sổ ở Lucerna Dark và Lucerna Light: đổi bằng `workbench.colorTheme` và `"window.autoDetectColorScheme": false` trong settings người dùng tạm. Dùng `screencapture -l <windowid>` hoặc công cụ computer-use. Lưu hai ảnh vào thư mục trên. Commit.

### Task 1: Script sinh dữ liệu Seti, SVG và license

**Files:**
- Create: `scripts/gen-seti.py`
- Create (sinh ra): `crates/lucerna-theme/src/seti_data.rs`, `crates/lucerna-workbench/src/seti_svgs.rs`, `crates/lucerna-workbench/assets/icons/seti/*.svg`
- Create: `crates/lucerna-workbench/assets/icons/seti/LICENSE-seti-ui` (nguyên văn `LICENSE.md` của seti-ui), `crates/lucerna-workbench/assets/icons/seti/LICENSE-vscode` (nguyên văn `LICENSE.txt` ở gốc vscode), `crates/lucerna-workbench/assets/icons/seti/README.md` (nguồn, hai commit, ghi chú "viewBox đã được đặt lại; đường vẽ giữ nguyên")
- Modify: `scripts/gen-licenses.py` (`ASSETS`: thêm `("Seti icons (seti-ui)", ".../seti/LICENSE-seti-ui")` và `("Seti icon theme mapping (VS Code)", ".../seti/LICENSE-vscode")`), rồi chạy lại để cập nhật `THIRD_PARTY_NOTICES.md`

**Interfaces — Produces** (`seti_data.rs`, sinh ra, `pub(crate)`; đầu file có `// @generated by scripts/gen-seti.py — seti-ui <commit>, vscode <commit>. Do not edit.`):

```rust
pub(crate) struct SetiDef { pub glyph: &'static str, pub dark: u32, pub light: u32 } // 0xRRGGBB
pub(crate) const DEFAULT_DEF: &str = "_default";
pub(crate) const DEFS: &[(&str, SetiDef)];            // khóa "_typescript_1", sắp theo khóa
pub(crate) const FILE_NAMES: &[(&str, &str)];         // Seti fileNames, tên viết thường → def
pub(crate) const FILE_EXTENSIONS: &[(&str, &str)];    // Seti fileExtensions, viết thường, không có dấu chấm đầu
pub(crate) const LANG_FILE_NAMES: &[(&str, &str)];    // từ contributes.languages[].filenames
pub(crate) const LANG_NAME_PREFIXES: &[(&str, &str)]; // filenamePatterns "X.*" → "x."
pub(crate) const LANG_NAME_SUFFIXES: &[(&str, &str)]; // filenamePatterns "*.x" → ".x"
pub(crate) const LANG_EXTENSIONS: &[(&str, &str)];    // contributes.languages[].extensions, bỏ dấu chấm đầu
```

`seti_svgs.rs` (sinh ra): `pub(crate) const SETI: &[(&str, &[u8])] = &[("typescript", include_bytes!(...)), …];`, sắp theo tên, chỉ gồm glyph mà `DEFS` có dùng.

- [ ] **Step 1: Tải nguồn về scratchpad.** Tải seti-ui ở `2d6c5e68` và vscode (lấy commit `main` hiện tại rồi ghi lại) vào hai thư mục riêng, mới, trống trong scratchpad, bằng `gh api .../tarball/<sha>` hoặc `git clone --depth`. Tạo venv trong scratchpad và `pip install svgelements`. Chạy script bằng `python -I`, truyền đường dẫn qua tham số: `scripts/gen-seti.py --seti <dir> --vscode <dir>`.
- [ ] **Step 2: Viết `gen-seti.py`.** Script làm các việc sau:
  - đọc `extensions/theme-seti/icons/vs-seti-icon-theme.json`;
  - với mỗi khóa def không có `_light`: glyph = bỏ `_` đầu, bỏ `_<số>` cuối; màu tối là `fontColor` của nó; màu sáng là `fontColor` của `<khóa>_light`, rồi chỉnh tương phản. Thiếu bản `_light` thì lấy màu tối nhân 0,9. Thiếu `fontColor` (như `_todo`) thì lấy màu của `_default`;
  - kiểm tra khối `light` khớp với khối chính theo quy tắc `def` → `def + "_light"`; sai thì dừng với thông báo lỗi;
  - đi qua `extensions/*/package.json` → `contributes.languages[]`, nối `id` với `languageIds` của theme rồi điền vào các bảng `LANG_*`. Làm giống `extensions/theme-seti/build/update-icon-theme.js` ở cùng commit, kể cả các danh sách cứng trong file đó, nếu chúng ảnh hưởng tới ánh xạ. `filenamePatterns` không thuộc hai dạng `X.*` và `*.x` thì bỏ qua và in cảnh báo;
  - mọi khóa đều viết thường. Trùng khóa trong cùng một bảng thì giữ mục đầu tiên theo thứ tự tên extension, để kết quả ổn định;
  - **SVG:** với mỗi glyph được dùng, đọc `seti-ui/icons/<glyph>.svg` và tính khung vẽ thật bằng `svgelements` (có tính transform). Đặt `viewBox` mới là hình vuông cạnh `max(w,h) * (1 + 2*PAD)` căn giữa khung, với `PAD = 0.0625`. Bỏ `width`/`height`, giữ nguyên các phần còn lại. Ghi ra `assets/icons/seti/<glyph>.svg`;
  - **chỉnh tương phản sáng:** dùng công thức độ chói tương đối WCAG. Nếu `contrast(c, #ffffff) < 3.0` thì giảm L của HSL mỗi lần 0,01, giữ H và S, đến giá trị đầu tiên đạt ≥ 3.0;
  - in thống kê: số def, số glyph, các màu sáng đã chỉnh (trước → sau).
- [ ] **Step 3: Chạy script và kiểm tra bằng mắt.** Mở vài SVG (`default`, `lock`, `typescript`, `docker`, `babel`) trong browser pane hoặc Quick Look: phải vuông và nằm giữa.
- [ ] **Step 4: Thêm license và cập nhật `gen-licenses.py`.** Sửa `ASSETS`, chạy `scripts/gen-licenses.py THIRD_PARTY_NOTICES.md` (đọc docstring của script để biết đường ra đúng), rồi kiểm tra `grep -n "seti" THIRD_PARTY_NOTICES.md`.
- [ ] **Step 5: Commit.** Gồm `scripts/gen-seti.py`, hai file `.rs` sinh ra (chưa gắn vào `mod`, Task 2 và 4 gắn), thư mục `assets/icons/seti/`, `scripts/gen-licenses.py` và `THIRD_PARTY_NOTICES.md`. Message: `feat(icons): vendor Seti icons and generate the mapping`.

### Task 2: Theme Seti trong `lucerna-theme`

**Files:**
- Create: `crates/lucerna-theme/src/seti.rs`
- Modify: `crates/lucerna-theme/src/lib.rs` (`mod seti; mod seti_data;`, re-export `seti_icon_theme`)
- Modify: `crates/lucerna-theme/src/icon_theme.rs`
- Test: `crates/lucerna-theme/tests/icon_theme.rs`, `crates/lucerna-theme/tests/seti.rs`

**Interfaces:**
- Consumes: `seti_data::*` (Task 1).
- Produces:
  - `IconSource::Glyph { name: &'static str, dark: u32, light: u32 }`: biến thể mới của `IconSource`, vẫn derive `PartialEq, Eq`.
  - `pub fn seti_icon_theme() -> IconTheme` (tên `"Seti"`, appearance Dark).
  - `IconThemeRegistry::names()` → `["Seti", "Lucide", …extension]`.
  - `IconThemeRegistry::resolve(name)`: `"Seti"` hoặc `"vs-seti"` cho Seti; `"Lucide"` cho Lucide; tên extension cho theme đó; tên lạ cho **Seti**. Lucide luôn là lớp dự phòng của `ResolvedIcons`.
  - `pub const DEFAULT_ICON_THEME: &str = "Seti";` export từ crate.
  - `pub fn seti_glyph_names() -> impl Iterator<Item = &'static str>`: các glyph mà `DEFS` dùng, để workbench kiểm tra SVG đã nhúng đủ.

- [ ] **Step 1: Viết test hỏng** trong `tests/seti.rs`. Bảng mẫu `(đường dẫn, glyph, dark)` lấy kết quả mong đợi từ VS Code thật: tra `vs-seti-icon-theme.json` cùng ngôn ngữ của file, rồi ghi giá trị hex vào test. Phải có tối thiểu:

```rust
#[test] fn seti_matches_vscode_table()        // app.ts→typescript, a.spec.ts & a.test.ts→typescript (def màu cam),
                                              // a.d.ts, Dockerfile, DOCKERFILE, Dockerfile.dev, docker-compose.yml (docker màu hồng),
                                              // .env, .env.example, .env.local (config), .gitignore (git), Cargo.toml, README.md, Readme.MD,
                                              // LICENSE, main.rs, package.json, tsconfig.json, Makefile, notes.txt, archive.tar.gz,
                                              // "noext", "weird.zzz" → glyph "default"
#[test] fn seti_own_entries_beat_language_entries()   // một tên file vừa có trong FILE_NAMES vừa có trong LANG_FILE_NAMES ra def của Seti
#[test] fn longest_extension_wins()                   // "a.spec.ts" lấy "spec.ts" chứ không lấy "ts"
#[test] fn seti_has_no_folder_or_chevron_icons()      // directory_icon/chevron → None, nên ResolvedIcons rơi về Lucide
```

  Unit test trong `src/seti.rs` (vì `seti_data` là `pub(crate)`): `every_light_color_has_3_to_1_contrast` (mọi `DEFS`: `contrast(light, 0xffffff) >= 3.0`) và `defs_referenced_by_tables_exist` (mọi def trong các bảng đều có trong `DEFS`).

  Trong `tests/icon_theme.rs`, sửa `unknown_theme_name_resolves_to_lucide` thành `unknown_theme_name_resolves_to_seti` (`names() == ["Seti","Lucide"]`, `resolve("Gone").name() == "Seti"`), và thêm:

```rust
#[test] fn vs_seti_alias_resolves_to_seti()
#[test] fn extension_cannot_replace_seti()   // extension có theme tên "Seti" bị bỏ qua, như với Lucide
#[test] fn lucide_still_resolvable()         // resolve("Lucide").name() == "Lucide"
```

  Các test Lucide hiện có giữ nguyên.

- [ ] **Step 2: Chạy test và xem nó hỏng.** `cargo test -p lucerna-theme`. Kết quả mong đợi: lỗi biên dịch vì chưa có `seti_icon_theme` và `IconSource::Glyph`.
- [ ] **Step 3: Cài đặt.**
  - Trong `icon_theme.rs`, `IconTheme` có thêm một trường kiểu enum riêng tư: `Mapping` cho dữ liệu hiện tại và `Seti` cho Seti. `file_icon`, `directory_icon` và `chevron` rẽ nhánh theo enum này. Phần `Mapping` giữ nguyên hành vi.
  - `seti.rs` dựng các `HashMap<&'static str, &'static SetiDef>` một lần, từ `const` của `seti_data`.
  - Thứ tự khớp trên tên file viết thường `n`:
    1. `FILE_NAMES[n]`;
    2. với mỗi đuôi theo thứ tự dài trước, tức phần sau mỗi dấu `.` từ trái sang phải (`.env` cho `env`; `a.spec.ts` cho `spec.ts` rồi `ts`): `FILE_EXTENSIONS`;
    3. `LANG_FILE_NAMES[n]`;
    4. tiền tố trong `LANG_NAME_PREFIXES` mà `n` bắt đầu bằng, chọn tiền tố dài nhất; rồi hậu tố trong `LANG_NAME_SUFFIXES` mà `n` kết thúc bằng, chọn hậu tố dài nhất;
    5. các đuôi như bước 2, nhưng tra `LANG_EXTENSIONS`;
    6. `DEFAULT_DEF`.
  - `name.find('.')` an toàn với UTF-8 vì `.` là ASCII.
  - `IconThemeRegistry` giữ `seti: Arc<IconTheme>` bên cạnh `lucide`. `ResolvedIcons` không đổi chữ ký.
- [ ] **Step 4: Chạy test và xem nó qua.** `cargo test -p lucerna-theme`. Mong đợi: PASS hết.
- [ ] **Step 5: Commit.** Message: `feat(theme): built-in Seti icon theme, default for unknown names`.

### Task 3: Setting mặc định và lúc khởi động

**Files:**
- Modify: `crates/lucerna-settings/assets/default-settings.json` (`"workbench.iconTheme": "Seti"`; thêm `"workbench.editor.showIcons": true` cạnh `workbench.editor.enablePreview`)
- Modify: `crates/lucerna-workbench/src/app.rs:731-732` (`resolve(DEFAULT_ICON_THEME)`)
- Modify: `crates/lucerna-workbench/src/extensions/service.rs:236-263` (dự phòng là `DEFAULT_ICON_THEME`; sửa doc comment)
- Modify: `crates/lucerna-workbench/src/app_state.rs:26-29` (doc comment)
- Test: `crates/lucerna-settings/tests/store.rs:223`, `crates/lucerna-workbench/tests/icon_theme_picker.rs`

**Interfaces:** Consumes `lucerna_theme::DEFAULT_ICON_THEME` (Task 2).

- [ ] **Step 1: Sửa test cho đúng hành vi mới.**
  - `store.rs`: mặc định là `Some("Seti")`; thêm assert `workbench.editor.showIcons` mặc định là `Some(true)`.
  - `icon_theme_picker.rs`: danh sách picker là `["Seti","Lucide","Ext Icons"]`. Tên lạ `"Gone"` ra `"Seti"` và setting giữ nguyên. Hàm `init` của test nên resolve theo setting đã cấu hình (gọi `resolve_icons` công khai nếu có, hoặc `resolve(DEFAULT_ICON_THEME)`) thay vì cứng `"Lucide"`.
  - Thêm `#[gpui::test] fn lucide_setting_kept()`: settings `{"workbench.iconTheme":"Lucide"}` → `icon_name == "Lucide"` sau khi mở workbench, và sau khi reload config.
- [ ] **Step 2: Chạy test và xem nó hỏng.** `cargo test -p lucerna-settings --test store` và `cargo test -p lucerna-workbench --test icon_theme_picker`. Mong đợi: FAIL ở các giá trị mặc định.
- [ ] **Step 3: Sửa default, `app.rs` và `service.rs`.** Không cần sửa khoảng 57 test khác đang gọi `.resolve("Lucide")`: chúng gọi tên Lucide rõ ràng.
- [ ] **Step 4: Chạy lại hai lệnh ở Step 2.** Mong đợi: PASS.
- [ ] **Step 5: Commit.** Message: `feat(settings): Seti is the default icon theme; workbench.editor.showIcons`.

### Task 4: Vẽ icon Seti có màu

**Files:**
- Modify: `crates/lucerna-workbench/src/icons.rs` (`mod` cho `seti_svgs`; `pub fn seti_svg(name: &str) -> Option<&'static [u8]>`, tra nhị phân trên `SETI` đã sắp)
- Modify: `crates/lucerna-workbench/src/lib.rs` (khai báo `mod seti_svgs;` nếu `icons.rs` không phải module cha)
- Modify: `crates/lucerna-workbench/src/ui.rs:84-107`
- Modify: `crates/lucerna-workbench/src/explorer/view.rs:1571-1581` (`icon_slot`), `excerpt_list/render.rs:223`, `locations_picker.rs:419`
- Test: `crates/lucerna-workbench/src/icons.rs` (unit), `crates/lucerna-workbench/tests/explorer_view.rs`

**Interfaces:**
- Consumes: `IconSource::Glyph` (Task 2), `seti_svgs::SETI` (Task 1).
- Produces:
  - `pub fn icon_tint(source: &IconSource, fallback: Hsla, appearance: Appearance) -> Hsla` trong `ui.rs`. Với `Glyph`, trả màu `dark` hoặc `light` theo appearance, alpha 1. Với các loại khác, trả `fallback`.
  - `pub fn file_icon_sized(source: &IconSource, size: Pixels, color: Hsla, appearance: Appearance) -> AnyElement`: thêm tham số `appearance`; `file_icon` cũng thêm như vậy. `Glyph` vẽ bằng `svg().data(seti_svg(name)).text_color(icon_tint(..))`. Glyph không tìm thấy SVG thì vẽ Lucide `file`.
  - Mọi chỗ gọi truyền `cx.global::<AppState>().theme.appearance`.

- [ ] **Step 1: Viết test hỏng.**

```rust
// icons.rs
#[test] fn every_seti_glyph_is_bundled()      // mọi lucerna_theme::seti_glyph_names() có seti_svg(..).is_some()
#[test] fn seti_svgs_have_square_viewbox()    // mỗi SVG trong SETI có viewBox "x y w h" với w == h
// ui.rs (unit test trong module)
#[test] fn glyph_tint_follows_appearance()    // Glyph{dark:0x519aba, light:L} → Dark: hsla của #519aba; Light: hsla của L; Builtin → fallback
// tests/explorer_view.rs
#[gpui::test] fn seti_icons_in_explorer()     // AppState với resolve("Seti"); row_icon("x.ts") là IconSource::Glyph{name:"typescript",..};
                                              // thư mục ra Builtin("folder")/("folder-open"); debug_bounds của icon hàng có mặt sau khi vẽ
#[gpui::test] fn glyph_color_tracks_color_theme() // đổi state.set_theme(Lucerna Light) rồi vẽ lại: icon_tint với appearance hiện tại ra màu light
```

- [ ] **Step 2: Chạy test và xem nó hỏng.** `cargo test -p lucerna-workbench --lib icons ui` và `--test explorer_view`. Mong đợi: lỗi biên dịch hoặc FAIL.
- [ ] **Step 3: Cài đặt.**
  - Explorer giữ `ui.icon_muted` làm `fallback`. Lucide và icon từ extension không đổi.
  - Đổi comment ở `icon_slot`: icon Lucide tô `tint`, glyph Seti tô màu riêng của nó, icon extension giữ màu gốc.
- [ ] **Step 4: Chạy test và xem nó qua.** Chạy lại các lệnh ở Step 2, rồi `cargo test -p lucerna-workbench --test perf`. Mong đợi: PASS.
- [ ] **Step 5: Commit.** Message: `feat(icons): draw Seti glyphs in their colors, light colors on light themes`.

### Task 5: Icon file trên tab và `workbench.editor.showIcons`

**Files:**
- Modify: `crates/lucerna-workbench/src/editor_groups/view.rs:655-677`
- Test: `crates/lucerna-workbench/tests/editor_groups_view.rs`

**Interfaces:** Consumes `file_icon_sized(…, appearance)` (Task 4) và `state.icons.file(path)`.

- [ ] **Step 1: Viết test hỏng** (dùng `init` và `open` có sẵn trong file test; với `init`, đổi `icons` thành `resolve("Seti")` chỉ trong các test mới, bằng một helper `init_seti`):

```rust
#[gpui::test] fn file_tabs_show_file_icons()       // mở a.ts (preview) và b.rs (pinned): vẽ xong thì có "tab-icon-0-0" và "tab-icon-0-1"
#[gpui::test] fn deleted_file_tab_keeps_icon()     // tab có strike vẫn có "tab-icon-…" (dùng cách các test external_changes đánh dấu file đã xóa)
#[gpui::test] fn file_diff_tab_shows_file_icon()   // TabItem::FileDiff(path): icon ra state.icons.file(path), không phải GIT_BRANCH
                                                   // (kiểm qua `EditorGroupsView::tab_icon(gi, ti, cx) -> Option<IconSource>`, xem Step 3)
#[gpui::test] fn show_icons_false_hides_all_tab_icons() // user settings {"workbench.editor.showIcons": false}: tab file và tab Search đều không có "tab-icon-0-*"
#[gpui::test] fn show_icons_toggle_redraws()       // set_layer(User) đổi true→false, reload hoặc refresh, vẽ lại: icon biến mất mà không mở lại tab
```

- [ ] **Step 2: Chạy test và xem nó hỏng.** `cargo test -p lucerna-workbench --test editor_groups_view`. Mong đợi: FAIL.
- [ ] **Step 3: Cài đặt.**
  - Tách việc chọn icon của tab thành `pub fn tab_icon(&self, gi: usize, ti: usize, cx: &App) -> Option<IconSource>` trên view (dùng cho test). Hàm trả về:
    - `TabItem::File(p)` và `TabItem::FileDiff(p)`: `Some(icons.file(p))`;
    - các tab đặc biệt: `Some(IconSource::Builtin("<tên lucide>"))`, ứng với các hằng hiện có (`blocks`, `search`, `zap`, `file-code`, `settings`, `git-branch`, `eye`);
    - setting `workbench.editor.showIcons` (đọc như `BreadcrumbSettings::read`, mặc định `true`) là `false`: `None`.
  - Phần vẽ dùng `file_icon_sized(&src, px(14.), color(ui.icon_muted).into(), appearance)` trong một `div().flex_none()` có `debug_selector("tab-icon-{gi}-{ti}")`. Như vậy `BLOCKS_ICON` và `SEARCH_ICON` đi qua `icons::lucide_svg`, cùng đường vẽ với icon file.
  - Bố cục `ui::tab` giữ nguyên.
- [ ] **Step 4: Chạy test và xem nó qua.** Chạy `--test editor_groups_view`, `--test search_view`, `--test extensions_page`, `--test markdown_preview`. Ba file sau đã kiểm `tab-icon-0-0`. Mong đợi: PASS.
- [ ] **Step 5: Commit.** Message: `feat(tabs): file icons on editor tabs; workbench.editor.showIcons`.

### Task 6: Ảnh "sau", tài liệu và kiểm tra cuối

**Files:**
- Create: `docs/research/file-icons-screenshots/after-dark.png`, `after-light.png`
- Modify: `docs/research/2026-11-15-file-icons.md` (mục "Lucerna chọn gì": thêm kết quả và ảnh), spec (trạng thái: đã triển khai)

- [ ] **Step 1: Chụp ảnh "sau".** Build, chạy cùng thư mục mẫu, cùng các tab, cùng hai theme như ở Task 0. So hai cặp ảnh:
  - `.ts` có màu xanh, `Dockerfile` có cá voi, `.env` có bánh răng;
  - icon trên tab cùng cỡ với icon trong Explorer;
  - icon trên theme sáng vẫn dễ thấy.
  
  Nếu icon trông to hay nhỏ hơn hẳn Lucide ở 14 px, chỉnh `PAD` trong `gen-seti.py`, chạy lại script, rồi ghi lý do vào plan.
- [ ] **Step 2: Kiểm tra cuối.**
  - `cargo test -p lucerna-theme -p lucerna-settings -p lucerna-workbench`, rồi `cargo test --workspace`.
  - `cargo clippy --workspace --all-targets -- -D warnings`.
  - `cargo deny check`.
  - `python3 scripts/gen-licenses.py <đường ra>`: không có diff ngoài những gì đã commit.
  
  Mong đợi: tất cả qua.
- [ ] **Step 3: Commit ảnh và tài liệu.** Ghi bàn giao vào thư mục ghi chú chung (`file-icons-handover.md`): nhánh, các commit, những điểm cần lưu ý khi merge cùng các nhánh khác đang sửa `editor_groups/view.rs`, `ui.rs` và `default-settings.json`.
