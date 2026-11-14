# Thanh cuộn dùng chuột

## Mục đích

Hiện thanh cuộn của Lucerna chỉ để hiển thị. Thumb trong editor (`code_view/element.rs`) và trong Explorer (`ScrollbarThumb` trong `explorer/view.rs`) được vẽ ra, nhưng không có code nào xử lý chuột trên đó. Người đọc file dài muốn kéo thumb để cuộn nhanh, hoặc bấm vào rãnh để nhảy tới một chỗ trong file, như cách họ vẫn làm trong VS Code.

## Mục tiêu

- **Kéo thumb để cuộn.** Chỗ nắm trên thumb luôn nằm dưới con trỏ.
- **Bấm vào rãnh** (vùng ngoài thumb) thì nhảy tới chỗ bấm, thumb căn giữa vào điểm bấm. Giữ chuột sau cú bấm thì kéo tiếp được.
- **Setting `editor.scrollbar.scrollByPage`** (bool, mặc định `false`), giống VS Code. Khi bật, bấm rãnh trong editor sẽ lật một trang lên hoặc xuống thay vì nhảy. Setting chỉ áp dụng cho editor.
- **Thumb đổi màu** khi rê chuột lên và khi đang kéo. Kích thước không đổi.
- **Vùng nhận chuột rộng 12 px** ở mép phải, thumb vẫn vẽ 6 px sát mép.
- **Đúng khi có dòng bị gập:** thumb của editor tính theo dòng hiển thị (mỗi dòng buffer chưa bị gập là một đơn vị, như hiện nay). Kéo tới vị trí nào thì ra đúng dòng hiển thị ở vị trí đó.
- **Một component dùng chung** cho editor và Explorer, đặt trong `ui.rs`. Các danh sách sau này dùng lại được.

## Không làm

- Tự ẩn thanh cuộn hoặc hiệu ứng fade (Zed có, Lucerna chưa có).
- Thanh cuộn ngang.
- Đảo hành vi khi giữ Shift (VS Code hiện tại không có).
- Cuộn về chỗ cũ khi kéo lệch quá 140 px (chỉ VS Code trên Windows có).
- Thêm thanh cuộn cho các picker. Component phải làm được việc đó về sau, nhưng việc gắn vào picker không nằm trong lần này.

## Không được xảy ra

- **Kéo bị giật hoặc chậm.** Mỗi lần chuột di chuyển chỉ được đổi vị trí cuộn rồi notify. Không shape lại text, không dựng lại các vạch đánh dấu trên thanh cuộn: cache `scrollbar_marks` không phụ thuộc vào vị trí cuộn.
- **Bấm vào thanh cuộn lại làm đổi vùng chọn** hoặc di chuyển con trỏ trong editor.
- **Minimap** hoặc vùng kéo chọn chữ ngừng hoạt động.

## Ràng buộc

- Chỉ `lucerna-workbench` được dùng gpui.
- Không chép code từ crate `ui` của Zed (giấy phép GPL), chỉ học hành vi. Xem `docs/research/2026-11-14-scrollbar-mouse.md`.
- Phải có test GPUI cho kéo thumb và bấm rãnh, gồm cả trường hợp có dòng bị gập.

## Các cách đã cân nhắc

- **A. Dùng chung phần tính toán, mỗi nơi tự nối chuột.** Các hàm thuần nằm trong `ui.rs`; editor xử lý chuột trong `register_mouse`, Explorer xử lý trong decoration. Không thêm element nào, nhưng phần nối chuột phải viết hai lần.
- **B. Một component thanh cuộn chung, đặt chồng lên nội dung (đã chọn).** Một element tự lo hitbox, trạng thái hover và kéo, và việc vẽ thumb, như component `Scrollbar` của Zed. Chỉ có một chỗ chứa code. Đổi lại, editor cần một lớp chuyển đổi giữa pixel và cách cuộn theo dòng của nó, và phải để ý thứ tự hitbox.

Người dùng chọn B.

## Thiết kế

### Component `ui::Scrollbar`

- **Nguồn cuộn.** Element nhận một nguồn cuộn, là một trait (tạm gọi `ScrollbarSource`) cung cấp:
  - `metrics()`: trả về `viewport`, `content` và `scroll_top`, cùng đơn vị pixel như các tham số của `thumb_geometry`;
  - `set_scroll_top(px)`: đặt vị trí cuộn mới, kèm notify view sở hữu.
- **Trạng thái.** Hover và kéo (gồm chỗ nắm trong thumb) nằm trong một `ScrollbarState` nhỏ, do view sở hữu giữ qua các frame. Cách cụ thể (`Rc<Cell>` hay element state) để người làm quyết.
- **Vị trí.** Element phủ một dải rộng 12 px ở mép phải vùng nội dung. Hitbox của dải này chặn chuột với các phần nằm dưới, nên editor không nhận cú bấm.
  - Bấm trúng thumb thì bắt đầu kéo.
  - Bấm vào rãnh thì nhảy tới chỗ bấm (hoặc lật trang nếu nguồn yêu cầu), rồi kéo tiếp từ giữa thumb.
  - Khi đang kéo, element nhận mouse move và mouse up ở bất kỳ đâu, cho tới khi nhả nút.
- **Màu.** Rãnh không vẽ gì. Thumb dùng `scrollbar.thumb.background`, `scrollbar.thumb.hover_background` khi rê chuột, và `scrollbar.thumb.active_background` khi đang kéo.
  - Hai key mới lấy theo tên key theme của Zed.
  - Nếu theme không có hai key này thì dùng màu thumb thường tăng độ đậm (người làm chọn mức).
- **Hàm tính toán.** Các hàm tính "chỗ bấm thành vị trí cuộn" và "khoảng kéo thành vị trí cuộn" là hàm thuần nằm cạnh `thumb_geometry`, có unit test.

### Editor

- **Nguồn cuộn của `CodeView`:**
  - `content` = (số dòng hiển thị − 1) × chiều cao dòng + viewport;
  - `scroll_top` = số dòng hiển thị phía trên × chiều cao dòng + `scroll.offset`.
  - Hai công thức này giữ đúng như code hiện tại.
- **`set_scroll_top(px)`:** đổi `px` thành dòng hiển thị thứ n, rồi qua `folds` thành dòng buffer làm `top_row`. Phần lẻ được đổi thành `offset` trong dòng đó, có tính số dòng bị wrap. Cuối cùng gọi `report_scroll` và notify.
- **Thay thế.** Bỏ thumb đang vẽ trong `paint.rs`. Các vạch đánh dấu diagnostic vẫn vẽ như cũ, nằm dưới thumb.
- **Test.** `CodeView::scrollbar_thumb()` vẫn trả về bounds của thumb để test dùng.

### Explorer

- Bỏ decoration `ScrollbarThumb`.
- Đặt `ui::Scrollbar` chồng lên list. Nguồn cuộn đọc trực tiếp từ `UniformListScrollHandle`:
  - `offset` và `max_offset` lấy từ `base_handle`;
  - đặt vị trí cuộn bằng `set_offset`.
- Nguồn cuộn dựa trên `ScrollHandle` này cũng dùng được cho các `div` có `overflow_y_scroll` về sau.

### Settings

- Thêm `editor.scrollbar.scrollByPage` (bool, mặc định `false`), đọc theo cùng cơ chế với các setting `editor.*` hiện có.

## Phần để người làm quyết

- Cách giữ `ScrollbarState` qua các frame.
- Mức tăng độ đậm của màu fallback.
- Tên chính xác của các kiểu và hàm.
- Debug selector cho test: giữ `explorer-scrollbar-thumb`, thêm selector cho editor.

## Tham khảo

Xem `docs/research/2026-11-14-scrollbar-mouse.md`.
