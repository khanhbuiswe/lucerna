# Explorer: Cut/Copy/Paste, Duplicate, Open Preview — thiết kế ngắn

Ngày: 2026-11-15. Trạng thái: yêu cầu do người dùng đặt (qua session chính), chờ duyệt plan.

## Ý định

Người dùng muốn context menu của Explorer có thêm các lệnh sau, hành vi theo VS Code:

1. Cut / Copy / Paste file và thư mục.
2. Duplicate, theo cách Zed làm.
3. Open Preview cho file `.md`.

Spec này nối tiếp `2026-11-13-explorer-context-menu-design.md`. Mọi ràng buộc và anti-goal của spec đó vẫn áp dụng: không ghi đè, luồng UI không chờ I/O, thao tác của chính Lucerna không bị đánh dấu là "thay đổi từ bên ngoài", menu trông như Zed.

## Tham khảo

Chi tiết nằm ở `docs/research/2026-11-13-explorer-context-menu.md`, mục §1 và §2.

- **VS Code:**
  - Các lệnh `filesExplorer.cut`, `filesExplorer.copy`, `filesExplorer.paste`, phím ⌘X / ⌘C / ⌘V khi `filesExplorerFocus`.
  - Paste dán vào thư mục đang chọn, hoặc vào thư mục chứa file đang chọn.
  - Tên trùng được đổi theo `explorer.incrementalNaming` (mặc định `simple`): `a copy.rs`, rồi `a copy 2.rs`.
  - Cut rồi Paste là di chuyển. Sau lần Paste đó clipboard được xóa.
  - Mục Paste bị làm mờ khi clipboard trống. Dòng đang bị Cut hiện mờ trong cây. `Esc` hủy Cut (`filesExplorer.cancelCut`).
  - Không cho dán một thư mục vào chính nó hoặc vào thư mục con của nó.
- **VS Code (extension Markdown):** "Open Preview" (`markdown.showPreview`) có trong context menu của Explorer khi bấm vào file markdown.
- **Zed:**
  - `project_panel::Duplicate` (phím `cmd-d`) chép mục sang cùng thư mục, dùng cùng cách đặt tên " copy" như Paste.
  - Sau khi chép xong, Zed mở ô đổi tên trên bản sao, chọn sẵn phần tên đến hết chữ " copy N".
  - Clipboard của Zed nằm trong panel, không phải clipboard của hệ điều hành.

## Thiết kế

### Lệnh, phím, menu

Phím là phím macOS. Mọi phím đều có điều kiện `filesExplorerFocus && !textInputFocus`.

| Mục | ID | Hiện khi | Phím |
|---|---|---|---|
| Cut | `filesExplorer.cut` | không phải gốc | `cmd+x` |
| Copy | `filesExplorer.copy` | không phải gốc | `cmd+c` |
| Paste | `filesExplorer.paste` | thư mục hoặc gốc (với file thì dán vào thư mục chứa nó). Làm mờ khi clipboard trống. | `cmd+v` |
| Duplicate | `filesExplorer.duplicate` | không phải gốc | `cmd+d` |
| Open Preview | `markdown.showPreview` | file markdown (theo ngôn ngữ của file) | — |
| — | `filesExplorer.cancelCut` | không có trong menu | `escape` khi đang có Cut |

- **Vị trí trong menu (theo VS Code):**
  - Nhóm Cut, Copy, Paste, Duplicate đứng sau nhóm Find in Folder, trước nhóm Copy Path.
  - Open Preview đứng sau Open to the Side.
  - Ở chế độ một file: không có Cut, Paste, Duplicate, Delete. Vẫn có Copy và Open Preview.

### Clipboard

- Là clipboard của Explorer, sống trong workbench: `{ op: Cut | Copy, paths }`. Nó không ghi vào clipboard của hệ điều hành.
- Tạm thời mỗi lần chỉ có một mục, vì Lucerna chưa chọn được nhiều dòng.
- Dòng đang bị Cut có tên hiện mờ (`text_muted`).

### Paste

- **Thư mục đích:** thư mục được chọn. Nếu dòng được chọn là file thì dùng thư mục chứa file đó. Lấy từ `args.path` khi lệnh chạy từ menu.
- **Đặt tên khi trùng:**
  - File: `stem copy.ext`, rồi `stem copy 2.ext`, `stem copy 3.ext`…
  - Thư mục: `name copy`, `name copy 2`…
  - Kiểm tra trùng trên snapshot, rồi kiểm tra lại khi ghi bằng thao tác không ghi đè. Nếu lúc ghi mới phát hiện trùng thì thử số tiếp theo.
- **Copy:**
  - File được chép với `create_new`, nên không bao giờ ghi đè.
  - Thư mục được chép đệ quy. Symlink được chép thành symlink, không đi theo nó.
  - Từ chối khi đích nằm trong chính thư mục nguồn, và báo bằng status message.
- **Cut (di chuyển):**
  - Dùng `fs_ops::rename`, loại không ghi đè.
  - Nếu trùng tên ở đích thì đặt tên kiểu " copy" như trên. Không ghi đè, không hỏi.
  - Dán vào đúng thư mục đang chứa mục đó thì không làm gì.
  - Tab, tài liệu, LSP, lịch sử điều hướng và recent đi theo đường dẫn mới, dùng lại `Workbench::paths_renamed` của Rename.
  - Chờ các lần lưu đang chạy dở xong rồi mới di chuyển.
  - Sau khi dán, clipboard được xóa.
- **Sau khi dán:** chọn mục mới, mở thư mục đích ra, và cuộn tới mục đó. Nếu là file thì mở trong tab ghim, theo Zed `auto_open.on_paste` mặc định bật.
- **Thao tác của chính mình:** đường dẫn mới được đưa vào `expect_own_changes` trước khi ghi, và gỡ ra bằng `forget_own_changes` nếu lỗi.
- **Lỗi:** báo bằng status message `Unable to paste '{name}': {err}`. Câu này do Claude tự đặt.
- **Luồng:** mọi I/O chạy ở luồng nền.

### Duplicate

- Giống Copy rồi Paste vào chính thư mục chứa mục đó, nhưng không đụng tới clipboard.
- Sau khi chép xong, mở ô đổi tên trên bản sao. Ô chọn sẵn phần tên đến hết " copy N", không chọn phần đuôi file, theo Zed.

### Open Preview

- `markdown.showPreview` nhận thêm `args.path`. Khi có `args.path`, lệnh mở file đó ở chế độ preview (`FileView::Preview`) trong tab ghim. Khi không có, lệnh làm như hiện nay, tức là chuyển tab đang mở sang preview.
- Mục menu chỉ hiện khi file có ngôn ngữ `markdown` (`files.associations` vẫn áp dụng).

## Không làm

- Chọn nhiều dòng; Cut/Copy nhiều mục cùng lúc.
- Dán file chép từ Finder hoặc từ clipboard của hệ điều hành.
- Ghi vào clipboard của hệ điều hành.
- Hộp thoại "Replace?" khi trùng tên: luôn đặt tên " copy", không hỏi.
- Undo, kéo thả.
- Setting `explorer.incrementalNaming` (chỉ có kiểu `simple`).

## Quyết định của Claude (người dùng xem lại)

- **Cut trùng tên ở đích:** đổi tên kiểu " copy" thay vì hỏi có ghi đè không như VS Code. Lý do: anti-goal là không bao giờ ghi đè, và người dùng muốn đặt tên kiểu " copy" khi trùng.
- **ID Duplicate:** `filesExplorer.duplicate`, vì VS Code không có lệnh này. Phím `cmd+d` khi Explorer có focus, theo Zed.
- **Sau khi dán file thì mở file**, theo Zed. VS Code không mở.
- **Clipboard chỉ là clipboard nội bộ**, như Zed.
