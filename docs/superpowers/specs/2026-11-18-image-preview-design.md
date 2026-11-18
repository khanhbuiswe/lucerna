# Image preview — thiết kế

Ngày: 2026-11-18. Trạng thái: chờ duyệt.

## 1. Ý định

Repo có nhiều asset: icon, logo, screenshot, ảnh minh hoạ trong docs. Người
dùng bấm vào một file ảnh trong Explorer hoặc Quick Open để biết nó trông thế
nào, mà không phải chuyển sang Finder hay Preview.

Hiện giờ Lucerna mở `.png` thì báo "The file appears to be binary." kèm nút
"Open as Text". Còn `.svg` thì mở ra mã nguồn.

Tính năng coi là thành công khi:
- Bấm một file ảnh là thấy ảnh trong tab ngay, kèm kích thước tính bằng pixel
  và dung lượng file.
- Agent ghi đè file ảnh thì tab tự cập nhật.
- Đóng tab thì bộ nhớ của ảnh được giải phóng.

## 2. Mục tiêu, không làm, phản mục tiêu

**Mục tiêu**
- Mở các file PNG, JPEG, GIF (có chuyển động), WebP, BMP, TIFF, ICO, SVG
  trong một tab ảnh.
- SVG mặc định mở ra ảnh, giống VS Code. Có lệnh, nút và ⌘⇧V để chuyển qua
  lại với mã nguồn.
- Tab ảnh hoạt động như tab file:
  - mở dạng preview tab (chữ nghiêng),
  - giữ lại tab bằng double-click,
  - chia sang ô bên cạnh,
  - lưu và khôi phục cùng workspace,
  - đi theo file khi file bị đổi tên.
- Ảnh được căn giữa. Ảnh lớn thu nhỏ cho vừa khung, ảnh nhỏ giữ nguyên kích
  thước gốc. Phía sau ảnh có nền ô cờ để thấy rõ phần trong suốt.
- Thanh trạng thái hiện `W × H • dung lượng`.
- File đổi trên đĩa thì tab tự tải lại.

**Không làm (bản này)**
- Zoom, kéo ảnh, xem đúng kích thước gốc khi ảnh lớn hơn khung.
- Sửa ảnh, copy ảnh (`imagePreview.copyImage`).
- So sánh ảnh trong git diff (Uncommitted Changes, `git.openChange`).
- Video và âm thanh. Các định dạng GPUI không đọc được (AVIF, TGA, HDR,
  EXR…).
- Ảnh trong hover và trong Quick Open.

**Phản mục tiêu** (chạy được nhưng vẫn coi là thất bại):
- Luồng UI bị đứng khi đọc hoặc giải mã ảnh lớn.
- Đóng tab mà RAM không giảm, hoặc tải lại liên tục làm RAM tăng dần.
- Ảnh cũ vẫn hiện sau khi file trên đĩa đã đổi.
- Ảnh nháy trắng mỗi lần agent ghi lại file.
- Thêm một crate mới vào `Cargo.lock`, hoặc binary tăng quá 200 KB.
- Chạm vào luồng xử lý văn bản (`Documents`, lưu file, hot exit).

## 3. Tham khảo

Chi tiết ở `docs/research/2026-11-18-image-preview.md`. Tóm tắt:

| Mặt | VS Code | Zed | Lucerna |
|---|---|---|---|
| SVG mặc định | ảnh | text | **VS Code** (ảnh) |
| Chọn editor | `imagePreview.previewEditor`, priority builtin, đổi qua `workbench.editorAssociations` | danh sách đuôi cố định | **VS Code** |
| Chuyển SVG ↔ mã nguồn | `imagePreview.reopenAsText` / `reopenAsPreview`, nút trên tiêu đề | không | lệnh **VS Code**, nút theo kiểu **Zed** (icon trên thanh đường dẫn, như markdown), thêm ⌘⇧V (lệch, §4.2) |
| Kích thước khi mở | vừa khung, chỉ thu nhỏ | như VS Code | như nhau |
| Nền | ô cờ 16px sau ảnh, viền 1px | `gpui::checkerboard`, viền 1px | **Zed** (`gpui::checkerboard`) |
| Thanh trạng thái | zoom, `WxH`, KB | `WxH • size • kênh • định dạng` | **Zed** về hình thức, nội dung như **VS Code**: `W × H • size` |
| Tải lại | watcher, giữ ảnh cũ nếu lỗi | đọc lại byte, giải mã xong mới thay, giải phóng ảnh cũ | **Zed** |
| Helix | không có | | |

## 3b. Các cách đã cân nhắc

1. **Thêm một loại tab ảnh riêng** (`TabItem::ImagePreview`,
   `FileView::Image`), không đi qua `Documents`. **Người dùng chọn cách
   này.** Không đụng vào markdown preview, ảnh tách biệt với luồng văn bản.
   Cái giá là lặp lại một ít logic chuyển qua lại giữa mã nguồn và preview.
2. **Gộp với markdown thành một loại "rendered".** Logic chuyển qua lại chỉ
   có ở một chỗ, nhưng phải sửa markdown preview (preview động, đồng bộ
   cuộn) là phần ảnh không cần. Dễ gây lỗi cho tính năng đang chạy ổn.
3. **Vẽ ảnh vào màn hình "binary" của `Documents`.** Ít code nhất, nhưng
   trộn ảnh vào luồng văn bản (encoding, lưu, hot exit). SVG không bao giờ
   tới được nhánh đó. Thông tin trên thanh trạng thái cũng khó gắn vào.

## 4. Quyết định

### 4.1 File nào mở ra ảnh

Thêm `FileView::Image` vào enum đang chọn giữa mã nguồn và preview. Hàm
`editor_for` (`markdown_preview/commands.rs`) trả về:
1. Khi `workbench.editorAssociations` có glob khớp: `Image` nếu id là
   `imagePreview.previewEditor`, `Source` nếu id là `default`, và `Preview`
   cho markdown như hiện nay.
2. Khi không glob nào khớp mà đuôi file (không phân biệt hoa thường) nằm
   trong danh sách ảnh: `Image`. Đây là priority `builtin` của VS Code, nên
   không cần thêm các đuôi ảnh vào giá trị mặc định của setting.

Danh sách đuôi ảnh: `png jpg jpeg jpe gif webp bmp ico tif tiff svg`.

Ai muốn SVG mở ra mã nguồn như trước thì đặt
`"workbench.editorAssociations": { "*.svg": "default" }`. Cũng làm được như
vậy với đuôi khác, nhưng ảnh raster lúc đó sẽ ra màn hình "binary" hiện có.

Mọi đường mở file đều đi qua chỗ này: Explorer, Quick Open, Open Recent,
`luc file`, phiên được khôi phục. Mở kèm số dòng (`file.svg:12`, Go to
Definition, kết quả tìm kiếm) thì vẫn mở mã nguồn, như `.md` đang làm.

### 4.2 Chuyển qua lại giữa SVG và mã nguồn

- **Lệnh** (theo VS Code):
  - `imagePreview.reopenAsText` ("Reopen as Text"): thay tab ảnh đang mở
    bằng tab mã nguồn ở cùng vị trí.
  - `imagePreview.reopenAsPreview` ("Reopen as Image Preview"): làm ngược
    lại.
  - Chỉ hiện trong command palette khi tab đang mở là SVG.
- **Nút:** icon trên thanh đường dẫn, cùng chỗ và cùng kiểu với nút preview
  của markdown.
- **Phím:** ⌘⇧V gọi đúng lệnh còn lại. VS Code không gán phím cho việc này.
  Lucerna gán vì ⌘⇧V đã quen tay ở markdown và người dùng đã chọn cách
  này. Đây là một chỗ lệch có chủ ý.
  - Cách gán: hai dòng keybinding cho ⌘⇧V, mỗi dòng có điều kiện `when`
    theo loại tab (SVG ở dạng mã nguồn, SVG ở dạng ảnh). Dòng của markdown
    giữ nguyên.
  - Trên tab ảnh raster, ⌘⇧V không làm gì.
  - *(Claude quyết định.)* Chuyển đổi luôn diễn ra tại chỗ, giống
    `markdown.togglePreview` đang làm. Nếu nhóm đã có tab ở dạng kia của cùng
    file thì kích hoạt tab đó thay vì tạo tab trùng, giống cách markdown
    xử lý hiện nay.
- **Explorer:** menu chuột phải của file `.svg` có "Open Preview", giống
  `.md`.
- Ảnh raster không có lệnh nào ở trên. Muốn xem dạng text thì dùng nút
  "Open as Text" trên màn hình lỗi (§4.6).

### 4.3 Tab

- Thêm `TabItem::ImagePreview(Arc<Path>)`. Không phải singleton: mở bao nhiêu
  cũng được, giống `File` và `MarkdownPreview`.
- Tiêu đề tab là tên file, icon theo loại file như tab file.
- Preview tab, pin tab, mở ở ô bên cạnh: cùng luật với tab file. Trong một
  nhóm, mở lại một file đang có tab ảnh thì kích hoạt tab đó.
- Lưu vào `GroupState::tabs` với tiền tố `lucerna:image-preview:` cộng đường
  dẫn tuyệt đối, như `MARKDOWN_PREVIEW_STATE_PREFIX`.
- File bị đổi tên: tab đi theo tên mới, như tab file và tab markdown.
- File bị xóa: tab giữ ảnh cuối cùng, tiêu đề tab gạch ngang như tab file.
  Không có thanh Close/Keep, vì thanh đó lấy trạng thái từ `Documents`.
  Khi file xuất hiện lại thì tab tải lại ảnh và bỏ gạch ngang. *(Claude
  quyết định.)*
- Tải lại không phải là thao tác của người dùng. Preview tab (chữ nghiêng)
  vẫn là preview tab sau khi tải lại.
- Tab ảnh không bao giờ ở trạng thái dirty. ⌘S không làm gì. Các lệnh soạn
  thảo (find, go to line, format…) không áp dụng cho tab này.

### 4.4 View ảnh

- Một entity `ImagePreview` mới nằm ở `lucerna-workbench`, trong module
  `image_preview/`. View này không dùng `Documents`.
- **Tải:** đọc byte và giải mã ở nền, dùng `Image::from_bytes`. Định dạng
  đoán từ byte; không đoán được mà đuôi là `.svg` thì thử SVG. Đồng thời đọc
  kích thước pixel. Vẽ bằng `img(ImageSource::Image(Arc<Image>))` để id ảnh
  theo nội dung, không theo đường dẫn.
- Trong lúc đang tải lần đầu, khung để trống, không có spinner, như markdown
  preview.
- **Bố cục:**
  - Ảnh nằm giữa khung. Tỉ lệ là `min(khung / ảnh, 1)` theo cả hai chiều.
  - Khung có lề vài pixel, dùng chung spacing token với các view khác.
  - Nền khung là màu nền editor. Riêng vùng ngay dưới ảnh có
    `gpui::checkerboard` với màu lấy từ theme, cộng viền 1px.
  - Khung đổi kích thước thì ảnh tính lại tỉ lệ.
- **SVG** vẽ ở độ phân giải đúng với kích thước hiển thị và scale factor của
  màn hình, để không bị mờ trên màn hình Retina. Kích thước "gốc" của SVG lấy
  từ `width`/`height`/`viewBox` của file.
  - *(Claude quyết định.)* Ở revision GPUI đang pin, `Image::from_bytes` với
    SVG luôn raster ở kích thước gốc của file (scale 1.0), nên SVG nhỏ vẽ to
    sẽ bị mờ. Vì vậy SVG có đường raster riêng:
    - Parse một lần bằng `usvg`.
    - Raster bằng `resvg` ở kích thước hiển thị nhân scale factor, rồi đưa
      vào GPUI dạng ảnh bitmap.
    - Khung đổi kích thước hoặc scale factor đổi thì raster lại sau 150 ms
      debounce. Raster cũ được giải phóng như §4.5.
    - `resvg` 0.46 đã có trong `Cargo.lock` qua GPUI. Thêm nó làm dependency
      trực tiếp của `lucerna-workbench`, đúng version đó, nên không thêm crate
      mới.
- GIF chuyển động thì chạy, nhờ GPUI.

### 4.5 Tải lại và bộ nhớ

- Theo dõi file qua `WorktreeEvent` sẵn có của workbench. Event này chỉ phủ
  thư mục đang mở. Ảnh nằm ngoài workspace (`luc file /nơi/khác.png`, phiên
  được khôi phục) chỉ được đọc một lần, không tự tải lại. *(Claude quyết
  định.)*
- Khi file của tab thay đổi trên đĩa thì đọc lại byte, sau 100 ms debounce
  như VS Code. File
  bị ghi liên tục thì chỉ đọc lần cuối.
- Giải mã xong mới thay ảnh. Đọc hoặc giải mã lỗi trong lúc tải lại thì giữ
  ảnh cũ, không đổi sang màn hình lỗi (file có thể đang được ghi dở). Lần
  thay đổi kế tiếp sẽ thử lại.
- Thay ảnh hoặc đóng tab thì giải phóng ảnh cũ ngay bằng `window.drop_image`
  và `remove_asset`.
- Byte chỉ giữ trong lúc giải mã, sau đó bỏ đi.

### 4.6 Giới hạn và lỗi

- File lớn hơn `MAX_IMAGE_BYTES` (20 MB, dùng chung với markdown preview)
  thì không đọc. Ảnh có `width × height` hơn 100 megapixel cũng không giải
  mã, vì RGBA của nó chiếm trên 400 MB RAM.
- Cả hai trường hợp, và khi không giải mã được, khung hiện một dòng thông
  báo căn giữa (bằng tiếng Anh) kèm nút:
  - `The image is too large to preview (34.2 MB).`
  - `The image is too large to preview (20000 × 12000).`
  - `The image could not be loaded.`
  - Nút "Open as Text" mở tab mã nguồn với `force_text`. Cách này dùng lại
    `Documents::open_as_text` hiện có.
  - Thông báo và nút dùng component có sẵn trong `ui.rs`, giống màn hình
    binary hiện nay.
- File con trỏ Git LFS (file văn bản bắt đầu bằng
  `version https://git-lfs.github.com/spec/`) hiện:
  `This image is stored with Git LFS and has not been downloaded.`

### 4.7 Thanh trạng thái

- Khi tab ảnh đang được chọn, các mục của editor văn bản (vị trí con trỏ,
  encoding, line ending, ngôn ngữ) ẩn đi. Thay vào đó là một mục bên phải:
  `512 × 512 • 3.2 KB`.
- Dung lượng: cơ số 1024, ghi `B`, `KB`, `MB` với một chữ số thập phân. Cách
  ghi giống VS Code; Lucerna không có setting `unit`.
- Chưa tải xong thì chỉ hiện dung lượng. Có lỗi thì cũng chỉ hiện dung
  lượng.

## 5. Kiểm thử

- **Unit** (không cần GPUI):
  - `editor_for` với danh sách đuôi, với association `default` và
    `imagePreview.previewEditor`, với đuôi viết hoa.
  - Định dạng `W × H • size`.
  - Giới hạn byte và megapixel.
  - Nhận diện file con trỏ LFS.
  - Lưu và khôi phục tiền tố `lucerna:image-preview:`.
- **UI** (`gpui::TestAppContext`):
  - Mở `.png` từ Explorer ra tab ảnh dạng preview tab.
  - Mở `.svg` ra ảnh; ⌘⇧V chuyển sang mã nguồn rồi chuyển lại.
  - Mở `file.svg:3` ra mã nguồn.
  - Ghi đè file thì ảnh mới thay ảnh cũ và kích thước trên thanh trạng
    thái đổi theo.
  - Ghi file hỏng thì vẫn giữ ảnh cũ.
  - Đóng tab thì ảnh được giải phóng.
  - Tab ảnh được khôi phục sau khi mở lại workspace.
- **Kiểm tra tay:**
  - Ảnh trong suốt hiện đúng trên cả theme sáng và theme tối.
  - SVG không bị mờ trên màn hình Retina.
  - RAM giảm lại sau khi đóng một ảnh 20 MB.

## 6. Để người làm tự quyết

- Kích thước ô cờ và màu cụ thể lấy từ token nào của theme.
- Chia code giữa `image_preview/` và `markdown_preview/commands.rs`. Hàm
  `editor_for` có thể chuyển ra một module chung nếu cách đó gọn hơn.
- Có dùng `RetainAllImageCache` cho view hay tự quản lý `Arc<Image>`, miễn
  là đạt §4.5.
