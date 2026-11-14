# Thanh cuộn dùng chuột: VS Code và Zed

Tra ngày 2026-11-14. VS Code ở nhánh `main` (MIT). Zed ở nhánh `main`, crate `ui` (GPL): chỉ học hành vi, không chép code.

## VS Code

Nguồn: `src/vs/base/browser/ui/scrollbar/abstractScrollbar.ts`, `scrollbarState.ts`, `src/vs/editor/common/config/editorOptions.ts`.

- **Kéo thumb (slider).** Lúc nhấn, VS Code lưu vị trí chuột và một bản sao trạng thái thanh cuộn. Mỗi lần chuột di chuyển, độ lệch so với điểm nhấn được cộng vào vị trí slider ban đầu (`getDesiredScrollPositionFromDelta`). Nhờ vậy điểm nắm luôn nằm dưới con trỏ, và sai số không cộng dồn.
  - Chỉ trên Windows: kéo lệch ra ngang quá 140 px (`POINTER_DRAG_RESET_DISTANCE`) thì cuộn về chỗ cũ. macOS và Linux không có.
- **Bấm rãnh.**
  - `editor.scrollbar.scrollByPage` mặc định `false`: nhảy tới chỗ bấm, slider căn giữa vào điểm bấm (`getDesiredScrollPositionFromOffset` trừ nửa chiều cao slider).
  - Khi bật `true`: bấm trên slider thì lên một trang, dưới slider thì xuống một trang (`...FromOffsetPaged`).
  - Sau cú bấm rãnh, nếu giữ chuột thì kéo tiếp được, vì `_sliderPointerDown` được gọi luôn.
  - **Không có** hành vi đảo khi giữ Shift trong code hiện tại.
- **Rê chuột.** Chỉ đổi màu bằng CSS (`scrollbarSlider.hoverBackground`; khi kéo thì `scrollbarSlider.activeBackground`, qua class `active`). Kích thước không đổi.
- Slider ngắn nhất 20 px (`MINIMUM_SLIDER_SIZE`).

## Zed

Nguồn: `crates/ui/src/components/scrollbar.rs`.

- **Ba trạng thái thumb:** Inactive, Hover, Dragging. Hover và Dragging chỉ đổi màu, dùng màu hover và màu active riêng; thumb không rộng ra.
- **Bấm rãnh:** thumb căn giữa vào điểm bấm, kẹp trong hai đầu rãnh. Không lật trang.
- **Kéo:** lưu chỗ nắm bên trong thumb, rồi giữ chỗ đó dưới con trỏ.
- **Khi kéo:** chỉ notify entity trạng thái của thanh cuộn (và entity cha nếu có đăng ký). Chuột bị capture suốt lúc kéo, con trỏ là mũi tên.
- **Kích thước:** thumb thường rộng 6 px, đệm 4 px mỗi bên; thumb editor rộng 15 px. Thumb ngắn nhất 25 px.
- **Tự ẩn:** ẩn sau 1 giây không dùng, fade 400 ms. Lucerna chưa làm phần này.

## Lucerna chọn gì

- **Hành vi theo VS Code:**
  - kéo giữ điểm nắm;
  - bấm rãnh thì nhảy tới chỗ bấm, có setting `editor.scrollbar.scrollByPage`;
  - giữ chuột sau khi bấm rãnh thì kéo tiếp được;
  - không có Shift;
  - không có reset 140 px, vì macOS là nền tảng chính và Zed cũng không có.
- **Giao diện theo Zed:**
  - thumb 6 px như hiện tại;
  - vùng nhận chuột rộng 12 px;
  - đổi màu khi rê và khi kéo, theo key theme của Zed là `scrollbar.thumb.hover_background` và `scrollbar.thumb.active_background`;
  - một component thanh cuộn dùng chung giữa editor và danh sách, như component `Scrollbar` của Zed.
