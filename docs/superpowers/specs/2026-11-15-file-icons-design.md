# Icon file có màu (Seti) và icon trên tab

Ngày: 2026-11-15. Trạng thái: đã duyệt (2026-11-15).
Tham khảo: `docs/research/2026-11-15-file-icons.md`.

## Mục đích

Người dùng chuyển từ VS Code quen nhìn màu và hình icon để nhận ra loại file ngay, cả trong cây file lẫn trên hàng tab (icon TS xanh, cá voi Docker, bánh răng cho `.env`). Lucerna hiện vẽ mọi icon một màu nhạt với 10 hình chung chung, và tab file không có icon. Tính năng này đưa bộ icon Seti của VS Code vào làm mặc định và thêm icon vào tab file.

## Mục tiêu

1. Icon theme dựng sẵn mới tên **"Seti"**, là mặc định (`workbench.iconTheme: "Seti"`). Mỗi loại file có hình và màu như Seti của VS Code, kèm bộ màu riêng cho theme sáng.
2. Tab file hiện icon của icon theme đang dùng, bên trái tên. Setting `workbench.editor.showIcons` (mặc định `true`) tắt được.
3. Mọi chỗ đang vẽ icon file qua `ResolvedIcons::file` (Explorer, ô tạo/đổi tên file, picker Locations, danh sách excerpt trong Search/References) tự có màu, không cần sửa từng chỗ.
4. Lucide vẫn chọn được trong picker icon theme. Icon theme từ extension Zed vẫn chạy như cũ.

## Không làm

- Không thêm theme "Minimal" hay lựa chọn "None" của VS Code.
- Không thêm icon file vào breadcrumbs. Breadcrumbs hiện chỉ có icon loại symbol.
- Không thêm `tabs.git_status` hay icon chẩn đoán trên tab kiểu Zed.
- Không có language ID lúc chạy. Ánh xạ theo ngôn ngữ được đổi sẵn thành tên file, đuôi file và mẫu tên file khi sinh dữ liệu.
- Không đổi Quick Open. Nó hiện chưa có icon.

## Không được xảy ra

- Icon nhạt đến khó thấy trên theme sáng. Màu dùng cho theme sáng phải có tương phản ít nhất 3:1 với nền Lucerna Light (`#ffffff`).
- Mỗi lần vẽ hàng Explorer hay tab lại phải parse hay tra bảng chậm. Bảng Seti chỉ dựng một lần.
- Chép code từ crate GPL của Zed.

## Ràng buộc

- Giấy phép: SVG lấy từ `jesseweed/seti-ui` (MIT), bảng màu và ánh xạ lấy từ `microsoft/vscode/extensions/theme-seti` (MIT). Giữ nguyên văn hai file license trong thư mục asset, thêm hai dòng vào `ASSETS` trong `scripts/gen-licenses.py`, và ghi nguồn trong `THIRD_PARTY_NOTICES.md` nếu repo đang dùng file đó.
- Chỉ `lucerna-workbench` được dùng GPUI. `lucerna-theme` chỉ giữ tên icon và màu, không giữ byte SVG, giống cách Lucide đang làm (`IconSource::Builtin(&'static str)` → `icons::lucide_svg`).
- Không có giả định riêng cho macOS.
- Asset nhúng bằng `include_bytes!` như Lucide. Tổng SVG Seti khoảng 175 KB, chấp nhận được.

## Các cách đã cân nhắc

| Cách | Được | Mất |
|---|---|---|
| **A. Đóng gói Seti làm mặc định** (đã chọn) | Giống VS Code, đúng ảnh người dùng gửi; mỗi icon một màu nên khớp cách GPUI tô `svg()`; có bộ màu sáng/tối; MIT | Thêm khoảng 170 SVG và một bảng ánh xạ cần sinh bằng script |
| B. Tô màu icon Lucide theo loại file | Ít việc nhất, nét vẽ đồng nhất | Chỉ 10 hình; `.ts` và `.rs` cùng hình, chỉ khác màu; không giống VS Code |
| C. Đóng gói Material Icon Theme | Icon rất đầy đủ, nhiều màu | Hơn 1000 file, nặng vài MB, rối mắt, ngược với UI tối giản |

Người dùng chọn A.

Lệch khỏi Zed: Zed mặc định dùng icon một màu và tắt icon trên tab (`tabs.file_icons: false`). Lucerna theo VS Code ở cả hai điểm vì người dùng yêu cầu rõ.

## Thiết kế

### Dữ liệu Seti

- **Nguồn** (ghim đúng phiên bản VS Code đang dùng): `vs-seti-icon-theme.json` của VS Code và SVG của seti-ui ở commit `2d6c5e68`, là commit mà JSON đó ghi.
  - JSON có `iconDefinitions` (mỗi định nghĩa gồm mã glyph và `fontColor`), `fileExtensions`, `fileNames`, `languageIds` và khối `light`.
  - **Tên định nghĩa chính là tên SVG.** Bỏ `_` ở đầu, bỏ `_<n>` và `_light` ở cuối, ví dụ `_typescript_1` → `typescript.svg`. Cả 383 định nghĩa đều khớp được với file trong `seti-ui/icons/`.
  - `_light` không phải một bộ màu chọn tay. Script của VS Code chỉ nhân mỗi kênh màu tối với 0,9. Trên nền trắng, xám mặc định (`#bfc2c1`) chỉ đạt 1,8:1 và vàng đạt 2,1:1.
- **Màu cho theme sáng** (người dùng chọn): bắt đầu từ màu `_light` của VS Code. Nếu tương phản với `#ffffff` dưới 3:1, script làm đậm dần màu đó, giữ nguyên sắc độ (hue): giảm L trong HSL hoặc OKLCH, builder chọn. Script dừng ở giá trị đầu tiên đạt 3:1. Màu nào đã đạt 3:1 thì giữ nguyên. Theme tối dùng đúng màu VS Code. Theme sáng hay tối là theo `appearance` của color theme đang dùng, kể cả theme từ extension.
- **Ánh xạ theo ngôn ngữ.** Nhiều loại phổ biến chỉ có trong `languageIds`, không có trong `fileExtensions`/`fileNames`: `ts`, `rs`, `md`, `json`, `yml`, `Dockerfile`, `docker-compose.yml`, `.gitignore`, `.env`. Script sinh làm như `build/update-icon-theme.js` của VS Code:
  - đọc `contributes.languages` trong `extensions/*/package.json` của vscode ở cùng commit (`extensions`, `filenames`, `filenamePatterns`), và các danh sách cứng trong script đó;
  - đổi mỗi language ID thành đuôi file, tên file và mẫu tên file.
  `lucerna-syntax` không dùng được cho việc này, vì nó không có language ID của VS Code.
- **Thứ tự khớp, giống VS Code:**
  1. tên file của Seti;
  2. đuôi file của Seti, đuôi dài nhất trước (`spec.ts` trước `ts`);
  3. tên file lấy từ ngôn ngữ;
  4. mẫu tên file lấy từ ngôn ngữ;
  5. đuôi file lấy từ ngôn ngữ;
  6. `_default`.

  Mục lấy từ ngôn ngữ luôn thua mục riêng của Seti. Mẫu tên file chỉ hỗ trợ hai dạng `tiền tố.*` và `*.đuôi`. Đó là các dạng mà ngôn ngữ dựng sẵn của VS Code đang dùng, ví dụ `.env.*` → dotenv, `Dockerfile.*` → dockerfile. Nhờ vậy `.env.example` ra `_config` như VS Code. Đây là quyết định của tôi.
- **SVG:** 169 file. Phần lớn có viewBox `0 0 32 32`, nhưng có 10 file `0 0 1200 1000` (trong đó có `default` và `lock`), 17 file không có viewBox, và vài file có khung lệch. Trong font, VS Code co mọi glyph về cùng cỡ. Vì vậy script sinh **đặt lại viewBox theo khung thật của hình vẽ** (căn giữa, vuông), nên icon nào cũng cùng cỡ và nằm giữa. Script không đổi đường vẽ. Chín icon nhiều màu (`go`, `elm`, `twig`…) thành một màu vì GPUI dùng SVG làm mặt nạ; font của VS Code cũng một màu, nên kết quả giống VS Code.
- **Script sinh**: `scripts/gen-seti.py`. Script đọc bản seti-ui và vscode đã tải về (ghi rõ commit vào đầu file sinh ra), chép các SVG thực sự dùng (đã chỉnh viewBox) vào `crates/lucerna-workbench/assets/icons/seti/`, và sinh một file dữ liệu commit vào repo. File dữ liệu là Rust `const` hoặc JSON nhúng; builder chọn. Nó gồm:
  - định nghĩa: tên glyph, màu tối, màu sáng;
  - các bảng khớp ở trên (khớp tên không phân biệt hoa thường, như VS Code).
  Build không cần mạng. Script chỉ chạy khi muốn cập nhật Seti.
- **License**: `assets/icons/seti/LICENSE-seti-ui` (từ seti-ui) và `LICENSE-vscode` (từ `LICENSE.txt` ở gốc vscode, vì `theme-seti` không có file license riêng), nguyên văn. Thêm vào `ASSETS` trong `scripts/gen-licenses.py`. Script này sinh `THIRD_PARTY_NOTICES.md`. Ghi chú license nói rõ SVG đã được đặt lại viewBox.

### `lucerna-theme`

- Thêm một biến thể `IconSource` cho icon dựng sẵn có màu, ví dụ `IconSource::Glyph { name: &'static str, dark: Rgba, light: Rgba }`. Builder đặt tên.
- `seti_icon_theme()` dựng một `IconTheme` tên "Seti" từ dữ liệu sinh ra:
  - tên file → `stems`, đuôi file → `suffixes`, glyph → `icons`;
  - thứ tự khớp: tên file đầy đủ, rồi đuôi dài nhất, rồi mặc định. Đây là thứ tự của VS Code. `IconTheme::file_icon` hiện đã làm gần như vậy; chỗ nào lệch thì sửa cho đúng VS Code mà không đổi hành vi của Lucide và theme extension;
  - thư mục và chevron để trống, nên rơi về Lucide. Đây là quyết định của tôi: Seti không có icon thư mục, nhưng giữ icon folder Lucide màu nhạt thì cây vẫn có bố cục kiểu Zed.
- `IconThemeRegistry`:
  - có sẵn hai theme dựng sẵn, Seti và Lucide; `names()` trả "Seti", "Lucide", rồi theme extension;
  - `resolve(name)` với tên lạ trả **Seti** (mặc định mới); Lucide vẫn là lớp dự phòng cuối cho thư mục, chevron và icon thiếu;
  - nhận thêm tên `"vs-seti"` như bí danh của "Seti", để settings chép từ VS Code vẫn chạy. Đây là quyết định của tôi;
  - extension không được ghi đè tên "Seti", như với Lucide.

### `lucerna-settings`

- `default-settings.json`: `"workbench.iconTheme": "Seti"`, thêm `"workbench.editor.showIcons": true`. Cập nhật schema hoặc mô tả settings nếu có.
- Ai đã đặt `"Lucide"` thì vẫn giữ Lucide.

### `lucerna-workbench`

- `icons.rs`: nhúng SVG Seti (macro giống `lucide!`) và thêm `seti_svg(name)`.
- `ui::file_icon_sized`: với `IconSource::Glyph` thì vẽ `svg()` bằng SVG Seti, tô màu `dark` hoặc `light` theo appearance của color theme đang dùng. Tham số `color` chỉ áp cho Lucide. Hàm cần biết appearance: builder chọn cách truyền (đọc `AppState` qua `cx`, hoặc thêm tham số).
- **Explorer**: không đổi bố cục. Icon file lấy màu Seti; folder và chevron vẫn Lucide với `icon_muted`. Hàng đang chọn hay hover vẫn giữ màu icon, như VS Code.
- **Tab file** (`editor_groups/view.rs`): `TabItem::File(path)` lấy `state.icons.file(path)` và vẽ bằng `file_icon_sized` ở 14 px, cùng ô icon và khoảng cách với các tab đặc biệt hiện có. Selector `tab-icon-{gi}-{ti}` dùng chung. Tab file bị xóa (gạch ngang) vẫn hiện icon. Tab diff của một file (`TabItem::FileDiff`) cũng hiện icon của file như VS Code; tab Git Changes và Branch Changes giữ icon nhánh. Tab xem trước Markdown giữ icon mắt.
- **`workbench.editor.showIcons: false`**: ẩn icon trên mọi tab, cả tab đặc biệt, như VS Code. Đổi setting thì tab vẽ lại ngay, không cần khởi động lại.
- Picker icon theme (`theme_picker.rs`): danh sách có "Seti" đầu tiên; xem trước bằng mũi tên vẫn chạy.
- Đổi color theme giữa sáng và tối thì màu icon đổi theo ngay.

### Hiệu năng

- Bảng Seti dựng một lần khi khởi động, cùng lúc với `IconThemeRegistry::build`, và dùng chung qua `Arc`.
- Mỗi lần tra chỉ là vài lần tra HashMap theo tên và đuôi, giống Lucide. Không cấp phát thêm khi vẽ, ngoài chuỗi chữ thường mà `file_icon` đang tạo.
- `svg()` của GPUI cache ảnh raster theo dữ liệu và kích cỡ, nên 14 px nhiều màu không tốn thêm so với Lucide. Test perf hiện có (`tests/perf.rs`, `terminal_perf.rs`) phải vẫn qua.

## Test

- `lucerna-theme`:
  - Seti khớp đúng như VS Code cho một bảng mẫu: `app.ts`, `a.spec.ts`, `a.d.ts`, `Dockerfile`, `docker-compose.yml`, `.env`, `.env.example`, `.gitignore`, `Cargo.toml`, `README.md`, `LICENSE`, `main.rs`, `package.json`, `tsconfig.json`, file không đuôi, file lạ → `_default`. Mỗi mẫu kiểm tên glyph và màu tối/sáng. Kết quả mong đợi lấy từ VS Code thật (bảng mẫu ghi trong test).
  - Khớp tên file không phân biệt hoa thường; mục riêng của Seti thắng mục lấy từ ngôn ngữ; `.env.local` và `Dockerfile.dev` khớp theo mẫu tên.
  - Mọi SVG Seti nhúng có viewBox vuông.
  - `resolve("vs-seti")` cho Seti; tên lạ cho Seti; `names()` có Seti đầu tiên.
  - Extension đặt tên "Seti" không ghi đè được theme dựng sẵn.
- `lucerna-workbench`:
  - Mọi tên glyph trong dữ liệu đều có SVG nhúng (giống `lucide_svg_resolves_every_bundled_name`).
  - Tab file có `tab-icon-{gi}-{ti}`. Đặt `workbench.editor.showIcons: false` thì không còn icon nào trên tab.
  - Explorer với Seti: hàng `.ts` có icon `Glyph` với màu đúng, theme sáng thì màu sáng đã sinh.
  - Test dữ liệu: mọi màu sáng có tương phản ≥ 3:1 với `#ffffff`, và màu `_light` nào đã đạt 3:1 thì không đổi.
  - Picker icon theme liệt kê Seti trước Lucide.
- Test đang dựa vào mặc định Lucide (`resolve("Lucide")` trong khoảng 57 chỗ ở tests) vẫn chạy được, vì chúng gọi tên Lucide rõ ràng. Test nào kiểm mặc định (`lucerna-settings/tests/store.rs`, `icon_theme_picker.rs`) thì sửa sang Seti.
- `cargo deny check` và `scripts/gen-licenses.py` chạy qua, và output có Seti.

## Ảnh chụp

Chụp `luc` mở cùng một thư mục mẫu (có `.ts`, `Dockerfile`, `docker-compose.yml`, `.env`, `.json`, `.md`, `.rs`) và mở vài tab. Chụp trước (main hiện tại) và sau, ở cả Lucerna Dark và Lucerna Light. Lưu vào `docs/research/file-icons-screenshots/`.

## Để builder quyết

- Định dạng file dữ liệu sinh ra (Rust `const`, hoặc JSON nhúng rồi parse một lần).
- Cách nối mã glyph font với tên SVG trong `gen-seti.py`.
- Tên biến thể `IconSource` và cách `file_icon_sized` biết appearance.
- Cách tính khung thật của hình vẽ SVG trong script (thư viện Python hay tự parse path).
