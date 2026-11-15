# Mở lại workspace gần nhất khi khởi động — thiết kế ngắn

Ngày: 2026-11-15. Trạng thái: người dùng chọn "làm như Zed".

## Ý định

Mở một folder, thoát Lucerna, mở lại từ Dock thì Lucerna hiện Welcome
thay vì folder vừa dùng. Người dùng muốn như Zed: khởi động mà không có
đường dẫn thì mở lại workspace gần nhất.

## Tham khảo

- **Zed:** `restore_on_startup`, mặc định `"last_workspace"`: mở lại
  workspace dùng gần nhất. `"last_session"` mở lại mọi cửa sổ, `"none"`
  mở cửa sổ trống.
- **VS Code:** `window.restoreWindows`, mặc định `"all"`: mở lại mọi cửa
  sổ. `"one"` mở lại cửa sổ dùng gần nhất, `"folders"`, `"preserve"`,
  `"none"`. Welcome (`workbench.startupEditor`) chỉ hiện khi không có gì
  để mở lại.

## Thiết kế

- Hành vi theo Zed (`last_workspace`), tên setting theo VS Code (nguyên
  tắc dự án): `window.restoreWindows`, mặc định `"one"`.
  - `"none"`: như trước (Welcome hoặc hộp chọn folder theo
    `workbench.startupEditor`).
  - Giá trị khác (`"one"`, `"all"`, `"folders"`, `"preserve"`): mở lại
    **một** workspace, vì mỗi cửa sổ Lucerna là một tiến trình. Mở lại
    nhiều cửa sổ để sau.
- "Workspace gần nhất" là mục đầu tiên còn tồn tại trong `recent.json`
  (folder hoặc file mở riêng), mở qua đúng đường mở thường
  (`Router::open_path`).
- Chỉ áp dụng khi khởi động không có đường dẫn (Dock, Finder mở app) và
  không có yêu cầu mở file nào trong thời gian chờ ban đầu. `luc <path>`
  và "Open With" vẫn mở đúng đường dẫn được yêu cầu.
- Không có mục nào còn tồn tại: hiện Welcome như trước.

## Giới hạn đã biết

Close Folder rồi thoát thì lần sau vẫn mở lại folder đó (danh sách gần
đây không ghi việc đóng folder). VS Code `"one"` sẽ mở cửa sổ trống.

## Test

`startup.rs`: `"none"` không mở lại; mặc định và các giá trị khác mở lại;
chọn mục mới nhất còn tồn tại, bỏ qua mục đã bị xoá; danh sách rỗng thì
không có gì.
