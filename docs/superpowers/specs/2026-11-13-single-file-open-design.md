# Mở file lẻ như Zed — thiết kế ngắn

Ngày: 2026-11-13. Trạng thái: đã duyệt.

## Ý định

Mở một file lẻ (Finder, `luc file`, Open File…, Open Recent) hiện mở cả
thư mục cha (ví dụ `~/Downloads`) hoặc cả repo git chứa file, nên sidebar
liệt kê cả thư mục. Người dùng muốn như Zed: mở một file thì chỉ có file đó.

## Hiện trạng

`resolve_target` (`crates/lucerna/src/target.rs`) chọn gốc cho một file:
repo git gần nhất, nếu không thì thư mục cha. Chế độ một file (R27: không
quét, không theo dõi, sidebar và Quick Open chỉ có file đó, Search tắt)
đã có nhưng chỉ bật khi thư mục cha là `$HOME` hoặc `/`.

## Tham khảo

- Zed (`crates/workspace/src/workspace.rs`, `OpenVisible`): mở file vào
  project trống tạo worktree chỉ gồm file đó (`OpenVisible::All`); mở thêm
  file vào project đã có worktree hiện thì file đó ẩn (`OpenVisible::None`).
  Không tìm repo git, không lấy thư mục cha.
- VS Code: `code file` mở cửa sổ không có folder; Explorer báo chưa mở
  folder. Cũng không tìm repo git.

## Cách làm đã cân nhắc

1. **Mọi file lẻ vào chế độ một file** (chọn). Dùng lại R27, ít mã, giống
   Zed và VS Code.
2. Giữ luật repo git, chỉ file ngoài repo mở lẻ. Bỏ: người dùng chọn làm
   như Zed; khác cả hai editor tham khảo.
3. Chế độ mới với file ẩn kiểu Zed. Bỏ: chế độ một file đã đủ, việc này
   để sau.

## Thiết kế

- `resolve_target`: file luôn trả `single_file: true`, `root` = thư mục cha.
  Bỏ tìm `.git` và điều kiện `$HOME`/`/`. Thư mục vẫn mở như cũ.
- Sidebar: ở chế độ một file không vẽ dòng gốc (thư mục), chỉ còn dòng
  của file, như Zed. Thanh tiêu đề hiện tên file. (Bản duyệt đầu ghi
  "dòng gốc hiện tên file"; đổi khi lập kế hoạch vì sẽ lặp tên hai lần.)
- Mở thêm file: Open File… trong ứng dụng mở tab ở cửa sổ này (như nay);
  Finder/`luc` mở cửa sổ mới (như nay, `open_requests::route`).
- Restricted Mode: tin cậy vẫn theo thư mục chứa file.
- Recent: file lẻ ghi là `RecentKind::File` (đã có).
- Test: sửa test luật repo git trong `target.rs` và `tests/cli.rs`; thêm
  test file trong thư mục thường (kiểu `~/Downloads`) và file trong repo
  đều mở lẻ; test không có dòng gốc và tiêu đề là tên file.

## Không làm

- File thứ hai hiện ẩn trong cùng cửa sổ như Zed.
- Đổi folder trong cửa sổ hiện tại (việc riêng của session Open Folder).

## Để người làm quyết

Cách đưa tên file vào nhãn gốc sidebar và tiêu đề; tên test.
