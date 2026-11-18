# Nghiên cứu: Image preview

Ngày 2026-11-18. Nguồn:
- VS Code: `microsoft/vscode` nhánh `main`. Commit `e7d9908` (2026-10-20, #341900) xóa extension `extensions/media-preview` và chuyển trình xem ảnh vào core, ở `src/vs/workbench/contrib/mediaPreview/` (`common/mediaPreview.ts`, `mediaPreview.contribution.ts`, `browser/imageViewer.ts`, `browser/imagePreviewEditor.ts`, `browser/mediaPreviewStatusBar.ts`, `media/*.css`). Id và hành vi không đổi. Bản extension cũ xem ở commit `3f98d21`.
- Zed: revision GPUI đang pin `b5cd310`, các crate `image_viewer` (`image_viewer.rs`, `image_info.rs`) và `project/src/image_store.rs`, cùng `gpui` (`platform.rs`, `color.rs`, `elements/img.rs`). Zed phần lớn là GPL: chỉ ghi lại thiết kế, không chép code.
- Helix: `helix-term/src/ui/picker.rs`, `helix-term/src/commands.rs`.

## 1. VS Code

- **Editor:** id `imagePreview.previewEditor`, priority `builtin`. Nhận các file khớp glob `*.{jpg,jpe,jpeg,png,bmp,gif,ico,webp,avif,svg}`. **SVG mặc định mở ra ảnh.** Người dùng đổi qua `workbench.editorAssociations`.
- **Chuyển qua lại với mã nguồn:** khi đang xem `.svg` dạng ảnh, thanh tiêu đề editor có `imagePreview.reopenAsText`. Khi đang xem dạng text thì có `imagePreview.reopenAsPreview`. Cả hai gọi "Reopen Editor With". Không có phím tắt mặc định.
- **Kích thước khi mở:** chế độ `fit` (`max-width/height: 100%`). Ảnh lớn thu nhỏ cho vừa khung, ảnh nhỏ không bị phóng to.
- **Zoom:**
  - Click để phóng to, Alt-click (macOS) để thu nhỏ.
  - Lăn chuột kèm Alt hoặc Ctrl, hoặc pinch trên trackpad, cũng zoom được.
  - Các mức zoom cố định từ 0.1 đến 20. Từ 3× trở lên ảnh vẽ kiểu `pixelated`.
  - Lệnh `imagePreview.zoomIn` và `imagePreview.zoomOut` không có phím mặc định.
- **Thanh trạng thái (bên phải):**
  - Mức zoom (`Whole Image` hoặc `NN%`), bấm vào để mở danh sách chọn mức.
  - Kích thước `WxH`.
  - Dung lượng `N.NNKB` (cơ số 1024, 2 chữ số thập phân).
- **Nền:** ô cờ `conic-gradient` 16px, chỉ nằm sau ảnh. Màu rgb(230) với theme sáng, rgb(20) với theme tối. Ảnh có viền 1px.
- **Tải lại:** theo dõi file, debounce 100 ms, đọc lại byte, giữ nguyên zoom và vị trí cuộn. Đọc lỗi giữa chừng (file đang được ghi) thì vẫn giữ ảnh cũ.
- **Lỗi:** hiện "An error occurred while loading the image." trong khung xem. Bản cũ có thêm link mở bằng text editor. File con trỏ Git LFS có thông báo riêng.
- **Setting:** không có setting nào cho ảnh.
- **Lệnh khác:** `imagePreview.copyImage`.

## 2. Zed

- **File mở ra ảnh:** các đuôi trong `gpui::Img::extensions()` (avif, jpg, jpeg, png, gif, webp, tif, tiff, tga, dds, bmp, ico, hdr, exr, pnm…, qoi, svg), **trừ SVG: SVG mở ra text.**
- **Kích thước khi mở:** `min(scale_x, scale_y, 1.0)`, nghĩa là chỉ thu nhỏ, không phóng to.
- **Zoom và kéo ảnh:**
  - ⌘ kèm lăn chuột để zoom quanh con trỏ. Lăn chuột thường hoặc kéo chuột để di chuyển ảnh.
  - Phím: ⌘= và ⌘- để zoom, ⌘0 về 100%, ⌘1 xem đúng kích thước gốc, ⌘⇧0 vừa khung.
  - Có toolbar riêng cho zoom.
- **Nền:** `gpui::checkerboard(panel_background, 32 * zoom)`, ô cờ to nhỏ theo zoom. Ảnh có viền 1px.
- **Thanh trạng thái (`ImageInfo`):** các phần nối bằng " • ", ví dụ `1920x1080 • 1.2 MiB • 4 channels, 8 bits per pixel • PNG`.
- **Setting:** `image_viewer.unit` nhận `binary` (mặc định, KiB) hoặc `decimal` (KB).
- **Tải ảnh:** `ImageItem` giữ một `Arc<gpui::Image>` tạo từ byte. Định dạng đoán bằng `image::guess_format`, không đoán được thì thử SVG. View giải mã ở nền trước khi vẽ để không bị nháy.
- **Tải lại:** khi worktree báo file đổi thì `ReloadNeeded`, đọc lại byte, giải mã xong mới thay ảnh. Sau đó giải phóng ảnh cũ bằng `window.drop_image` và `Image::remove_asset`. Tải lại lỗi thì giữ ảnh cũ và chỉ ghi log.
- **Tab:** tiêu đề là tên file, tô màu theo trạng thái git. Icon theo loại file.

## 3. Helix

Helix không xem được ảnh. Picker hiện `<Binary file>`. Mở URL `file:` trỏ tới file binary thì Helix giao cho chương trình mặc định của hệ điều hành.

## 4. GPUI ở revision đang pin

- `ImageFormat` có Png, Jpeg, Webp, Gif, Svg, Bmp, Tiff, Ico, Pnm. GIF có chuyển động.
- `img(path)` lưu cache theo đường dẫn. **Ghi đè file không làm cache mất hiệu lực**, nên ảnh cũ vẫn hiện.
- Hai cách ép tải lại:
  - Dùng `ImageSource::Image(Arc<Image>)` với `Image::from_bytes`. Id của ảnh là hash của byte, nên nội dung mới tự có key cache mới.
  - Hoặc gọi `remove_asset` rồi `window.drop_image`.
- Markdown preview của Lucerna đang tải lại ảnh bằng cách tạo lại `RetainAllImageCache`.
- `gpui::checkerboard(color, size)` có sẵn ở `color.rs`.

## 5. So sánh

| | VS Code | Zed | Helix |
|---|---|---|---|
| SVG mặc định | Ảnh | Text | n/a |
| Kích thước / zoom | Vừa khung, chỉ thu nhỏ; zoom 0.1–20 | Vừa khung, chỉ thu nhỏ; zoom, kéo, phím | không |
| Thanh trạng thái | Zoom, `WxH`, dung lượng | `WxH • dung lượng • kênh • định dạng` | không |
| Nền ô cờ | Có, 16px cố định | Có, 32px theo zoom | không |
| Tải lại | Watcher, 100 ms, giữ zoom | Đổi trên đĩa thì đọc lại, giải mã xong mới thay | không |
| Setting | Không | `image_viewer.unit` | không |
