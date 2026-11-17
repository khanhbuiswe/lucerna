# Zoom cỡ chữ code (⌘+ / ⌘− / ⌘0)

Ngày 2026-11-17. Trạng thái: đã duyệt, chưa triển khai. Mục đầu tiên của tầng 1 trong `docs/roadmap.md`.

## 1. Mục đích

Người chuyển từ VS Code bấm ⌘+ để đọc code to hơn (màn hình lớn, trình chiếu, mỏi mắt). Hiện Lucerna
không có lệnh nào, phải sửa `editor.fontSize` bằng tay.

**Mục tiêu:** ⌘= / ⌘+ / ⌘− / ⌘0 phóng to, thu nhỏ, đặt lại cỡ chữ code ở mọi cửa sổ.

**Ngoài phạm vi:** zoom toàn bộ giao diện (`window.zoomLevel`); ⌘ + lăn chuột (`editor.mouseWheelZoom`);
lưu mức zoom qua lần mở app sau.

**Anti-goal:** zoom làm thay đổi `settings.json` của người dùng.

## 2. Tham khảo

- **Zed** (`assets/keymaps/default-macos.json`): `cmd-=` / `cmd-+` → `zed::IncreaseBufferFontSize`,
  `cmd--` → `DecreaseBufferFontSize`, `cmd-0` → `ResetBufferFontSize`, đều `{ "persist": false }`:
  không ghi settings, mất khi khởi động lại. Ở Welcome và Onboarding, các phím này đổi cỡ chữ UI.
  Terminal của Zed dùng `buffer_font_size` khi không đặt cỡ riêng.
- **VS Code:** ⌘= / ⌘− zoom cả cửa sổ (`workbench.action.zoomIn`, ghi `window.zoomLevel`). Riêng
  editor có `editor.action.fontZoomIn` / `fontZoomOut` / `fontZoomReset`, không có phím mặc định,
  không lưu qua lần khởi động.
- **Helix:** chạy trong terminal, không có zoom.

## 3. Các cách đã cân nhắc

| Cách | Đánh đổi | Kết quả |
|---|---|---|
| A. Zoom toàn UI như VS Code | Đúng nguyên tắc "hành vi theo VS Code", nhưng ~634 chỗ `px(...)` cố định trong `lucerna-workbench` phải đổi sang một hệ số chung | Bỏ, để sau nếu có người xin |
| **B. Chỉ cỡ chữ code như Zed** | Nhỏ, chỉ đụng chỗ đọc `editor.fontSize`. Lệch VS Code ở phím ⌘+ (VS Code zoom cả cửa sổ) | **Chọn** |

Lệch VS Code có chủ ý: phím ⌘+ gọi lệnh font zoom của editor thay vì zoom cửa sổ, vì Lucerna chưa
có zoom cửa sổ và người bấm ⌘+ gần như luôn muốn đọc code to hơn.

## 4. Thiết kế

- **Lệnh** (ID của VS Code): `editor.action.fontZoomIn`, `editor.action.fontZoomOut`,
  `editor.action.fontZoomReset`. Có trong command palette với tên "Editor Font Zoom In",
  "Editor Font Zoom Out", "Editor Font Zoom Reset".
- **Phím mặc định** (theo Zed, đổi được trong `keybindings.json`): `cmd+=` và `cmd++` → in,
  `cmd+-` → out, `cmd+0` → reset. Không có `when`, chạy ở mọi nơi trong cửa sổ.
- **Trạng thái:** một độ lệch nguyên (px) dùng chung cho cả tiến trình, mọi cửa sổ. Cỡ chữ thật =
  `editor.fontSize` + độ lệch, kẹp trong 6–100 (cùng giới hạn với bước cỡ chữ ở onboarding,
  `FONT_SIZE_MIN` / `FONT_SIZE_MAX`). Mỗi lần bấm ±1 px. Đã chạm giới hạn thì không làm gì.
- **Không lưu:** độ lệch chỉ sống trong bộ nhớ; không ghi `settings.json`, không vào app state.
- **Settings thắng:** khi `editor.fontSize` hiệu lực đổi (người dùng sửa settings), độ lệch về 0.
- **Áp dụng cho:** mọi chỗ vẽ code theo `editor.fontSize` (editor, tab diff, các danh sách excerpt),
  và terminal khi `terminal.integrated.fontSize` là `null`. Cỡ chữ UI (sidebar, tab, status bar,
  Quick Open) không đổi.
- **Phản hồi:** sau mỗi lần bấm, status bar hiện tạm "Font Size: N" qua cơ chế `status_message`
  sẵn có.
- **Đổi cỡ giữ vị trí:** dòng ở đầu khung nhìn vẫn ở đầu khung nhìn sau khi đổi cỡ (như khi đổi
  `editor.fontSize` hiện nay; nếu hiện chưa giữ thì làm cho giữ).
- **Vim:** không ảnh hưởng, các phím đều có ⌘.

## 5. Để người làm quyết định

- Cách đưa cỡ chữ đã zoom tới các view: hàm đọc chung thay cho các chỗ đang đọc thẳng
  `editor.fontSize`, hay một global GPUI rồi `notify` các cửa sổ.
- Khi terminal đã có cỡ riêng (`terminal.integrated.fontSize` khác `null`) thì terminal không zoom.
  Đây là quyết định của người viết spec (như Zed: terminal theo buffer chỉ khi không đặt riêng).

## 6. Kiểm tra

- Unit test: bước ±1, kẹp 6–100, reset, settings đổi thì độ lệch về 0.
- Thủ công trên app: ⌘+ ở editor, diff tab, terminal; hai cửa sổ cùng đổi; khởi động lại thì về cỡ
  trong settings; `settings.json` không đổi.
