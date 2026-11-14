# Context menu của Explorer: VS Code và Zed

Tra ngày 2026-11-13. VS Code: nhánh `main` của `microsoft/vscode`. Zed: revision GPUI Lucerna đang pin (`b5cd310`), đọc trong `~/.cargo/git/checkouts/zed-*/b5cd310`.

## 1. VS Code

### Các mục trong menu

Menu là `MenuId.ExplorerContext`. Mỗi mục thuộc một group, và giữa hai group có một đường kẻ. Thứ tự dưới đây là thứ tự hiện trên macOS, với một project một thư mục gốc.

| Group | Mục | Command ID | Hiện khi | Phím (macOS) |
|---|---|---|---|---|
| navigation | New File… | `explorer.newFile` | là thư mục | — |
| navigation | New Folder… | `explorer.newFolder` | là thư mục | — |
| navigation | Open to the Side | `explorer.openToSide` | là file | `ctrl+enter` |
| navigation | Open With… | `explorer.openWith` | là file và có nhiều editor | — |
| navigation | Reveal in Finder | `revealFileInOS` | luôn | `alt+cmd+r` |
| navigation | Open in Integrated Terminal | `openInIntegratedTerminal` | luôn | — |
| navigation | Open in External Terminal | `openInTerminal` | `terminal.explorerKind` là `external` hoặc `both` | — |
| 2_workspace | Add Folder to Workspace… / Remove Folder from Workspace | | là thư mục gốc | — |
| 3_compare | Select for Compare / Compare with Selected | | là file | — |
| 4_search | Find in Folder… | `filesExplorer.findInFolder` | là thư mục | `alt+shift+f` |
| 5_cutcopypaste | Cut / Copy / Paste | `filesExplorer.cut` / `.copy` / `.paste` | không phải gốc | `cmd+x` / `cmd+c` / `cmd+v` |
| 6_copypath | Copy Path | `copyFilePath` | luôn | `alt+cmd+c` |
| 6_copypath | Copy Relative Path | `copyRelativeFilePath` | luôn | `alt+cmd+shift+c` |
| 7_modification | Rename… | `renameFile` | không phải gốc | `enter` (Windows/Linux: `f2`) |
| 7_modification | Delete | `moveFileToTrash` (Delete Permanently là `deleteFile`, chỉ hiện khi không đưa vào Trash được) | không phải gốc | `cmd+backspace`; xóa hẳn `alt+cmd+backspace` |

Tất cả phím ở trên chỉ có tác dụng khi Explorer đang có focus (`filesExplorerFocus`).

### Hành vi

- **Bấm chuột phải** chọn và focus đúng dòng vừa bấm. Menu tác động lên dòng đó. Nếu đang chọn nhiều dòng thì tác động lên tất cả.
- **Bấm chuột phải vào chỗ trống** bên dưới cây sẽ mở menu của thư mục gốc.
- **New File / New Folder** chèn một ô nhập tên ngay trong cây, dưới thư mục đích. `Enter` tạo, `Escape` hủy. Gõ `a/b/c.txt` sẽ tạo luôn các thư mục ở giữa. Tên trùng hoặc không hợp lệ bị báo lỗi ngay dưới ô nhập.
- **Rename** biến dòng đó thành ô nhập, chọn sẵn phần tên mà không chọn đuôi file. Tab đang mở file đó đổi theo tên mới.
- **Delete** hỏi lại: "Are you sure you want to delete 'x'? You can restore this file from the Trash." Có ô "Do not ask me again", lưu vào `explorer.confirmDelete`. Tab của file bị xóa sẽ đóng nếu file đó chưa sửa.
- **Copy Path** chép đường dẫn tuyệt đối. **Copy Relative Path** chép đường dẫn so với thư mục gốc, ngăn cách bằng `/` hoặc `\` theo `explorer.copyRelativePathSeparator` (mặc định `auto`).
- **Open in External Terminal** mở app terminal theo `terminal.external.osxExec` (mặc định `Terminal.app`) tại thư mục đó. Nếu là file thì mở tại thư mục chứa nó.
- Nếu dòng không đủ điều kiện, mục đó bị ẩn (`when`), không bị làm mờ. Làm mờ (`precondition`) chỉ dùng khi thư mục chỉ đọc.

### Settings liên quan

`explorer.confirmDelete` (true), `explorer.enableUndo`, `explorer.copyRelativePathSeparator` (`auto`), `terminal.explorerKind` (`integrated`), `terminal.external.osxExec` (`Terminal.app`), `terminal.external.linuxExec`, `terminal.external.windowsExec`.

## 2. Zed

### Component `ContextMenu` (`crates/ui/src/components/context_menu.rs`, GPL)

- Là một entity riêng, có focus handle riêng, và phát ra `DismissEvent`. View chủ giữ menu trong `Option<DeployedContextMenu { menu, position, anchor, _subscription }>`. Khi nhận `DismissEvent` thì đặt lại thành `None`.
- Được vẽ bằng `deferred(anchored().position(pos).child(menu)).with_priority(3)`. Nhờ vậy menu nằm trên mọi thứ khác. `anchored` tự lật hoặc dời menu để không tràn ra ngoài cửa sổ (`snap_to_window_with_margin(8px)` cho submenu).
- Builder có các hàm `.action(label, Box<dyn Action>)`, `.entry(label, action, handler)`, `.separator()`, `.header(title)`, `.action_disabled_when(cond, …)`, `.when(cond, |m| …)` và submenu. Mỗi entry tự hiện phím tắt ở bên phải, lấy từ keymap qua `KeyBinding::for_action_in(action, focus)`.
- Kiểu dáng gồm `elevation_2`: nền `elevated_surface.background`, viền `border`, bo góc `rounded_lg`, có bóng đổ. Menu rộng tối thiểu 200 px, cao tối đa 75% cửa sổ (cuộn được khi dài hơn). Mỗi mục là một `ListItem` loại Dense, padding ngang khoảng 6 px, gap 1.5 giữa icon, nhãn và phím. Đường kẻ phân cách cao 1 px, màu `border_variant`, cách trên dưới khoảng 6 px. Mục bị tắt dùng màu `text.disabled`.
- Phím trong menu: `up` / `down` để chọn (bỏ qua đường kẻ và mục bị tắt), `home` / `end`, `enter` để chạy, `escape` để đóng. Bấm chuột ra ngoài menu (`on_mouse_down_out`) cũng đóng menu. Ngay khi mở chưa có mục nào được chọn; nhấn `down` thì chọn mục đầu tiên.
- Chạy một mục thì menu đóng, rồi action được dispatch về view đã mở menu (`menu.context(focus_handle)`).

### Menu của Project Panel (`crates/project_panel/src/project_panel.rs`, `deploy_context_menu`)

Thứ tự khi project ghi được:

1. New File, New Folder
2. Reveal in Finder, Open in Default App, Open in Terminal, (Open Markdown/SVG Preview)
3. Find in Folder… (thư mục), Fold/Unfold Directory
4. Compare Marked Files
5. Cut, Copy, Duplicate, Paste (tắt khi clipboard trống), Undo, Redo
6. Copy Path, Copy Relative Path
7. Git: Restore File, Add to .gitignore, View History, Permalink…
8. Rename, Trash, Delete (không có ở thư mục gốc)
9. Gốc: Add Folders to Project…, Remove from Project
10. Thư mục: Expand All, Collapse All

Khi project chỉ đọc, menu chỉ còn Open Markdown Preview, Open SVG Preview và Search Inside.

- **Bấm chuột phải** chọn dòng đó rồi mở menu tại vị trí chuột. `shift-f10` mở menu ngay tại dòng đang chọn, dùng `context_menu_placement_for_entry`.
- Dòng đang mở menu được tô sáng cho tới khi menu đóng.
- **Open in Terminal** của Zed mở terminal tích hợp. Zed không có lựa chọn mở terminal ngoài.
- Phím trong `ProjectPanel` (macOS): `cmd-n` New File, `alt-cmd-n` New Folder, `cmd-alt-c` Copy Path, `alt-cmd-shift-c` Copy Relative Path, `enter` / `f2` Rename, `backspace` / `delete` Trash (có hỏi), `cmd-backspace` Trash (không hỏi), `cmd-delete` Delete, `alt-cmd-r` Reveal in Finder, `ctrl-shift-enter` mở bằng app mặc định, `shift-f10` mở context menu, `escape` hủy.

## 3. Giống và khác

| Điểm | VS Code | Zed |
|---|---|---|
| Nhóm và thứ tự | Tạo mới → mở hoặc reveal → tìm kiếm → cắt, chép, dán → chép đường dẫn → sửa | Gần như y hệt |
| Mục không áp dụng | Ẩn đi | Ẩn đi. Paste, Undo, Redo thì làm mờ |
| Phím Copy Path | `alt+cmd+c` | `cmd-alt-c`, giống VS Code |
| Phím Rename | `enter` | `enter` hoặc `f2` |
| Phím Delete | `cmd+backspace` đưa vào Trash và hỏi lại | `backspace` hỏi lại; `cmd-backspace` không hỏi |
| Terminal | Có cả terminal tích hợp và terminal ngoài | Chỉ có terminal tích hợp |
| Giao diện menu | Menu native của hệ điều hành trên macOS (`window.titleBarStyle: native`) hoặc menu tự vẽ | Popover tự vẽ (`elevation_2`, rộng tối thiểu 200 px, hiện phím tắt bên phải) |
| Thao tác bàn phím trong cây | Có selection và focus riêng cho từng dòng | Có selection riêng cho từng dòng |

## 4. Liên quan tới Lucerna (hiện trạng)

- Explorer (`crates/lucerna-workbench/src/explorer/`) chưa có selection riêng. Dòng được tô sáng là tab đang mở (`active`). Chưa có thao tác bàn phím trong cây. Context key `explorerViewletFocus` đang dùng chung giá trị với `sideBarFocus`.
- Chưa có chỗ nào dùng `MouseButton::Right`. `ui.rs` đã có `popover`, `list_button` (dòng icon + nhãn + phím) và `tooltip`, nhưng chưa có thứ gì nổi lên tại vị trí chuột.
- Đã có `workbench.action.splitEditor` và editor groups, nên làm được Open to the Side.
- Lucerna chưa có terminal tích hợp. Nếu có Open in Terminal thì phải là terminal ngoài.
- Code đã dùng `window.prompt` (save, documents, about) để hiện hộp thoại native. Delete có thể dùng nó để hỏi lại.
- Mở thư mục trong Finder hiện đang gọi `Command::new("open")` ở `config_files.rs`. GPUI có sẵn `cx.reveal_path` và `cx.open_with_system`, chạy được trên mọi nền tảng.
