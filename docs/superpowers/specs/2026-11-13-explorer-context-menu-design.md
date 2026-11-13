# Context menu của Explorer — thiết kế

Ngày: 2026-11-13. Trạng thái: đã duyệt.

## Ý định

Bấm chuột phải vào một dòng trong Explorer sẽ mở một menu nhỏ. Menu có các lệnh để đọc code (mở bên cạnh, hiện trong Finder, mở terminal, tìm trong thư mục, chép đường dẫn) và các lệnh sửa cây file (tạo file hoặc thư mục, đổi tên, xóa vào Trash). Người dùng muốn làm **đủ cả hai loại**.

Đây là menu chuột phải đầu tiên của Lucerna. Component menu phải là component dùng chung trong `ui.rs`, để sau này menu của tab, của editor và của thanh path dùng lại được.

## Tham khảo

Phần chi tiết nằm ở `docs/research/2026-11-13-explorer-context-menu.md`. Tóm tắt:

- **VS Code** (`MenuId.ExplorerContext`):
  - Chia nhóm theo thứ tự: tạo mới → mở hoặc reveal → so sánh → tìm kiếm → cắt, chép, dán → chép đường dẫn → đổi tên, xóa.
  - Mục không áp dụng thì bị ẩn.
  - Tạo mới và đổi tên đều nhập tên ngay trong cây.
  - Delete đưa file vào Trash và hỏi lại trước khi xóa.
  - Có các lệnh `explorer.newFile`, `explorer.newFolder`, `explorer.openToSide`, `revealFileInOS`, `openInTerminal`, `filesExplorer.findInFolder`, `copyFilePath`, `copyRelativeFilePath`, `renameFile`, `moveFileToTrash`, `deleteFile`.
- **Zed** (`project_panel::deploy_context_menu`, `ui::ContextMenu`):
  - Thứ tự các mục gần như giống VS Code.
  - Menu là một popover tự vẽ nằm ở vị trí chuột (`deferred(anchored())`). Nó có focus riêng và đi được bằng `↑↓`, `Enter`, `Esc`.
  - Mỗi mục hiện phím tắt ở bên phải. Menu rộng tối thiểu 200 px.
  - Mục bị tắt thì làm mờ, đường kẻ phân cách màu `border_variant`.
  - Dòng đang mở menu được tô sáng cho tới khi menu đóng.
  - `shift-f10` mở menu tại dòng đang chọn.
- **Lucerna theo bên nào:**
  - Lệnh, ID, phím, settings và hành vi theo VS Code.
  - Giao diện menu và cách vẽ theo Zed.
  - Lệch khỏi cả hai: "Open in Terminal" mở **terminal ngoài**, vì Lucerna không có terminal tích hợp. Đây cũng là chỗ người dùng chạy agent CLI.

## Mục tiêu

1. Có component `ContextMenu` dùng chung trong `ui.rs`, vẽ theo Zed.
2. Explorer có **dòng đang chọn** riêng, mở được menu bằng chuột phải hoặc `shift+f10`, và có các phím tắt của VS Code cho các lệnh dưới đây.
3. Có các lệnh chỉ đọc: Open to the Side, Reveal in Finder, Open in Terminal, Find in Folder…, Copy Path, Copy Relative Path.
4. Có các lệnh sửa: New File…, New Folder…, Rename…, Delete (đưa vào Trash), và Delete Permanently (chỉ có phím tắt, không có trong menu).

## Không làm

- Cut, Copy, Paste, Duplicate file; kéo thả; Undo/Redo thao tác file.
- Chọn nhiều dòng. Menu luôn tác động lên đúng một dòng.
- Đi trong cây bằng phím mũi tên. Đó là một tính năng riêng, nhưng dòng đang chọn được thiết kế để sau này dùng được cho việc đó.
- Compare, Open With…, các mục Git, Add/Remove Folder.
- Menu cho tab, editor và thanh path. Component sẵn sàng cho các menu này, nhưng việc làm chúng thuộc spec sau.
- Ô "Do not ask me again" trong hộp thoại xóa. Hộp thoại native của GPUI không có checkbox, nên người dùng tắt bằng setting `explorer.confirmDelete`.
- Submenu, mục có dấu check, header trong menu. Component không cần những thứ này lúc này.

## Không được xảy ra (anti-goals)

- Mất dữ liệu: xóa mà không hỏi khi `explorer.confirmDelete` đang bật; xóa vĩnh viễn khi người dùng chỉ chọn Trash; ghi đè file đã có khi tạo mới hoặc đổi tên.
- Luồng UI chờ I/O. Mọi thao tác file chạy ở luồng nền.
- Tab của file vừa đổi tên bị mất nội dung đang sửa dở.
- Thao tác của chính Lucerna bị đánh dấu là "thay đổi từ bên ngoài" trong Explorer (xem `2026-11-13-explorer-own-save-decoration-design.md`).
- Menu kiểu VS Code hoặc menu native. Menu phải trông như Zed.

## Ràng buộc

- Chỉ `lucerna-workbench` được dùng GPUI. Thao tác file (tạo, đổi tên, đưa vào Trash, xóa) nằm trong `lucerna-project` và có test không cần UI.
- Xóa vào Trash dùng crate `trash` (giấy phép MIT; Zed cũng dùng). Phải chạy `cargo deny check` sau khi thêm crate.
- Không viết code chỉ chạy trên macOS ở ngoài phần riêng cho từng nền tảng:
  - Reveal dùng `cx.reveal_path`.
  - Terminal ngoài chọn theo nền tảng, như ở dưới.
  - Tên hiện cho người dùng: "Reveal in Finder" trên macOS, "Open Containing Folder" trên Linux, "Reveal in File Explorer" trên Windows (giống VS Code).
- Không chép code của Zed vì Zed dùng giấy phép GPL. Chỉ học cách thiết kế.

## Cách làm đã cân nhắc

1. **Theo Zed: menu là entity có focus riêng, nhập tên ngay trong cây.** Đây là cách được chọn. Nó giống cả VS Code lẫn Zed, người dùng thấy file mới sẽ nằm ở đâu, và component menu dùng lại được.
2. Giống cách 1, nhưng nhập tên trong một popover ở đầu cửa sổ, kiểu Quick Open. Bỏ vì nó lệch khỏi cả hai bên. Cách này nhẹ hơn chỉ vì không phải chèn ô nhập vào danh sách ảo hóa.
3. Dùng menu native của hệ điều hành. Bỏ vì GPUI không hỗ trợ menu chuột phải native, và cách này không dùng lại được cho tab hay editor.

Người dùng chọn cách 1. Việc này được chia làm hai phần, làm lần lượt, mỗi phần có plan riêng:

- **Phần 1:** component menu, dòng đang chọn, các lệnh chỉ đọc.
- **Phần 2:** New File/Folder, Rename, Delete.

## Thiết kế

### 1. Component `ContextMenu` (`ui.rs`)

**Dữ liệu.** Một entity GPUI có `FocusHandle` riêng và phát ra `DismissEvent`. Menu là một danh sách các mục, mỗi mục là một trong hai loại:

- `Entry { label, command: &'static str, args: Option<Value>, disabled: bool }`
- `Separator`

Mục chạy lệnh theo **command ID của `CommandRegistry`**, không dùng action của GPUI. Nhờ vậy cùng một lệnh chạy được từ menu, từ phím tắt và từ Command Palette.

Builder có `entry(label, command)`, `entry_with_args(...)`, `disabled_when(cond)`, `separator()`, `when(cond, f)`. Builder tự bỏ đường kẻ ở đầu menu, ở cuối menu và đường kẻ thứ hai khi hai đường kẻ liền nhau, để chỗ gọi viết `when` thoải mái.

**Hiện ra.** View chủ giữ menu trong `Option<DeployedMenu { menu, position, _subscription }>`. Khi menu phát `DismissEvent`, view đặt lại thành `None`.

Menu được vẽ bằng `deferred(anchored().position(pos).child(menu))`. `anchored` tự dời menu để không tràn ra ngoài cửa sổ, và menu cách mép cửa sổ ít nhất 8 px. Có một hàm dùng chung `deploy_context_menu(...)`, để view nào cũng mở menu bằng một lời gọi.

**Giao diện** (theo Zed, lấy số đo của Lucerna):

- Khung dùng nền, viền và bóng của `popover()`, nhưng không bị giới hạn bởi `POPOVER_MAX_WIDTH`. Rộng tối thiểu 200 px. Cao tối đa 75% cửa sổ, cuộn được khi dài hơn. Padding trong `GRID`.
- Mỗi mục cao 6 × `GRID`, padding ngang 2 × `GRID`, bo góc `GRID`. Nhãn cỡ `UI_FONT_SIZE_SMALL`, màu `text`. Phím tắt nằm bên phải, màu `text_muted`, lấy từ `keymap.label_for(command)` giống `list_button`.
  - Hover hoặc mục đang chọn bằng phím: nền `ghost_element_hover`.
  - Mục bị tắt: màu `text_disabled`, không có hover, bấm không chạy gì.
- Đường kẻ phân cách: cao 1 px, màu `border_variant`, cách trên dưới `GRID`.
- Menu chưa có icon. Chừa sẵn chỗ để thêm icon tùy chọn sau này.

**Tương tác:**

- Lúc vừa mở, chưa có mục nào được chọn. `↓` chọn mục đầu tiên, `↑` chọn mục cuối cùng. Khi di chuyển, bỏ qua đường kẻ và các mục bị tắt. Có `home` và `end`.
- `Enter` chạy mục đang chọn. Bấm chuột trái vào một mục cũng chạy mục đó.
- `Esc` đóng menu. Bấm chuột ra ngoài menu (cả chuột trái lẫn chuột phải) cũng đóng menu.
- Chạy một mục thì trước hết đóng menu, trả focus về view đã mở menu, rồi mới chạy lệnh. Như vậy các lệnh cần Explorer đang có focus (như Rename) vẫn chạy đúng.
- Khi menu đóng mà không chạy lệnh nào, focus quay về view đã mở menu.
- Bấm chuột phải ở chỗ khác khi menu đang mở thì đóng menu cũ và mở menu mới.

**Phím trong menu đi qua keymap**, giống Quick Open (`inQuickOpen`). Lý do: `Workbench::dispatch_key` bắt mọi phím ở pha capture, trước khi view con nhận được. Có context key mới `contextMenuVisible`. Các phím `up`, `down`, `home`, `end`, `enter`, `escape` có điều kiện `contextMenuVisible` và trỏ tới các lệnh nội bộ của menu (ví dụ `lucerna.contextMenu.selectNext`). Khi menu đang mở, `filesExplorerFocus` là false, để `enter` không chạy Rename và `cmd+backspace` không xóa.

### 2. Dòng đang chọn và focus của Explorer

`ExplorerModel` có thêm `selected: Option<Arc<Path>>`, là đường dẫn so với gốc. `None` nghĩa là đang chọn thư mục gốc.

**Cập nhật:**

- Bấm chuột trái vào một dòng thì chọn dòng đó, rồi làm như hiện nay (mở file, hoặc mở/đóng thư mục).
- Bấm chuột phải vào một dòng thì chọn dòng đó và mở menu của nó.
- Bấm chuột phải vào dòng gốc hoặc chỗ trống dưới cây thì chọn thư mục gốc và mở menu của gốc.
- Khi tab đang mở đổi (theo `explorer.autoReveal`), dòng đang chọn cũng đi theo tab đó, giống VS Code.
- Khi dòng đang chọn biến mất khỏi cây (bị xóa, hoặc thư mục cha bị đóng), chọn thư mục cha gần nhất còn hiện.

**Cách vẽ:**

- Dòng đang chọn có nền `element_selected`, thay cho cách hiện nay là tô theo tab đang mở. Bình thường hai cái trùng nhau.
- Khi menu đang mở, dòng của menu có thêm viền 1 px màu `border_focused`, bên trong dòng, theo Zed. Viền này mất khi menu đóng.
- Dòng gốc: hiện nay bấm vào không làm gì. Bây giờ bấm vào sẽ chọn gốc, và dòng gốc cũng được tô khi đang chọn.

**Focus.** Sidebar đã có focus handle (`sidebar_focus`). Có thêm các context key sau:

- `filesExplorerFocus` = sidebar đang có focus, sidebar đang hiện Explorer, và không có context menu nào đang mở.
- `explorerViewletFocus` sửa lại cho đúng nghĩa: chỉ đúng khi sidebar đang hiện Explorer. Hiện nay key này bằng `sideBarFocus`, nên vẫn đúng cả khi sidebar đang hiện Extensions.
- `explorerResourceIsFolder` = dòng đang chọn là thư mục, hoặc là gốc.
- `explorerResourceIsRoot` = đang chọn gốc.

Tên các key theo VS Code. Bấm vào một dòng sẽ đưa focus vào sidebar. Cần kiểm tra lại điều này, vì hiện nay bấm vào dòng có thể không lấy focus.

**Chế độ một file** (mở một file lẻ, xem `2026-11-13-single-file-open-design.md`): ở chế độ này worktree không quét và không theo dõi thư mục. Vì vậy mọi menu ở đây, kể cả bấm chuột phải vào dòng gốc hay chỗ trống, đều tác động lên **chính file đó**, không phải lên thư mục cha. Menu chỉ có Open to the Side, Reveal in Finder, Open in Terminal (mở tại thư mục cha), Copy Path và Copy Relative Path (chép tên file). Không có New File/Folder, Find in Folder, Rename hay Delete. Đây là quyết định của Claude.

### 3. Lệnh và menu

Mọi lệnh được đăng ký trong `CommandRegistry` với ID của VS Code. Lệnh tác động lên **dòng đang chọn** của Explorer. Khi nhận `args` là `{ "path": … }` thì lệnh tác động lên đường dẫn đó, để dùng lại cho menu tab sau này.

Menu của một dòng có thứ tự như sau. Mục không áp dụng thì bị ẩn, theo VS Code. Dấu "—" là đường kẻ phân cách.

| Mục | ID | Hiện khi | Phím mặc định (macOS; Linux và Windows theo VS Code) | Loại |
|---|---|---|---|---|
| New File… | `explorer.newFile` | thư mục hoặc gốc | — | sửa |
| New Folder… | `explorer.newFolder` | thư mục hoặc gốc | — | sửa |
| — | | | | |
| Open to the Side | `explorer.openToSide` | file | `ctrl+enter` (Linux/Windows: `ctrl+enter`) | đọc |
| Reveal in Finder | `revealFileInOS` | luôn | `alt+cmd+r` (Linux: `ctrl+alt+r`; Windows: `shift+alt+r`) | đọc |
| Open in Terminal | `openInTerminal` | luôn | — | đọc |
| — | | | | |
| Find in Folder… | `filesExplorer.findInFolder` | thư mục hoặc gốc | `alt+shift+f` | đọc |
| — | | | | |
| Copy Path | `copyFilePath` | luôn | `alt+cmd+c` (Linux: `ctrl+alt+c`; Windows: `shift+alt+c`) | đọc |
| Copy Relative Path | `copyRelativeFilePath` | luôn | `alt+cmd+shift+c` (Linux: `ctrl+shift+alt+c`; Windows: `ctrl+k ctrl+shift+c`) | đọc |
| — | | | | |
| Rename… | `renameFile` | không phải gốc | `enter` (Linux/Windows: `f2`) | sửa |
| Delete | `moveFileToTrash` | không phải gốc | `cmd+backspace` (Linux/Windows: `delete`) | sửa |

Các phím khác:

- `deleteFile` (Delete Permanently): `alt+cmd+backspace` / `shift+delete`. Không có trong menu.
- Mở menu bằng phím: `shift+f10`, ID `filesExplorer.openContextMenu`. Đây là đề xuất của mình, vì VS Code không có ID riêng cho việc này. Menu mở ngay dưới dòng đang chọn. Phím dùng được ở cả VS Code lẫn Zed.

Mọi phím của Explorer có điều kiện `filesExplorerFocus`. VS Code cho Reveal, Copy Path và Copy Relative Path chạy cả khi editor không có focus (`!editorFocus`), và lấy file từ tab đang mở. Lucerna hẹp lại, chỉ cho chạy trong Explorer; mở rộng ra thì để lúc làm menu tab. Thêm các điều kiện sau:

- Rename và Delete: `&& !explorerResourceIsRoot`.
- Open to the Side: `&& !explorerResourceIsFolder`.
- Find in Folder: `&& explorerResourceIsFolder`.
- Rename (phím `enter`): thêm `&& !textInputFocus`, để lúc đang nhập tên thì `enter` không kích hoạt Rename. Key `textInputFocus` đã có sẵn, nhưng được tính theo frame trước. Người làm phải bảo đảm ô nhập vừa nhận focus không làm `enter` lọt sang Rename, ví dụ bằng cách cho Explorer tự kiểm tra trạng thái `editing`.

Các phím được thêm vào `crates/lucerna-settings/assets/default-keybindings.json`.

Cách từng lệnh chạy:

- **Open to the Side:** `open_path(path, OpenMode::ToSide)`.
- **Reveal in Finder:** `cx.reveal_path(abs)`. Hàm này mở Finder và chọn sẵn file.
- **Open in Terminal:** mở tại thư mục đó, hoặc tại thư mục chứa nếu là file. Chạy theo setting của nền tảng:
  - macOS: `terminal.external.osxExec` (mặc định `Terminal.app`), chạy `open -a <app> <dir>`.
  - Linux: `terminal.external.linuxExec` (mặc định là terminal của hệ, ví dụ `x-terminal-emulator`), spawn với `cwd = dir`.
  - Windows: `terminal.external.windowsExec`, để dành cho bản Windows.

  Nếu lỗi thì báo bằng `status_message`.
- **Find in Folder…:** mở hoặc đưa tab Search lên trước, điền ô include bằng đường dẫn tương đối của thư mục (`src/foo`, giống VS Code; gốc thì để trống), rồi focus vào ô query.
- **Copy Path:** chép đường dẫn tuyệt đối vào clipboard.
- **Copy Relative Path:** chép đường dẫn so với gốc. Ngăn cách theo `explorer.copyRelativePathSeparator`: `auto`, `/` hoặc `\`, mặc định `auto` (`/` trên macOS và Linux). Với gốc thì chép tên thư mục gốc, giống VS Code.

### 4. Nhập tên ngay trong cây (New File, New Folder, Rename)

`ExplorerModel` có thêm trạng thái `editing: Option<EditState>`:

```
EditState {
    kind: NewFile | NewFolder | Rename,
    parent: dir,
    original?,
    error: Option<String>,
}
```

- **Vị trí:**
  - New File/Folder: mở thư mục đích ra (gốc thì luôn mở sẵn), rồi chèn một **dòng ảo** làm con đầu tiên của thư mục đó. Dòng ảo có icon file hoặc thư mục và một `TextInput`.
  - Rename: thay tên của chính dòng đó bằng `TextInput`. Ô nhập điền sẵn tên cũ, và chọn sẵn phần tên **trước đuôi cuối** (`foo.rs` thì chọn `foo`). Thư mục thì chọn cả tên.
  - Dòng ảo vẫn nằm trong `uniform_list`, chiều cao dòng giữ nguyên `TREE_ROW_HEIGHT`. Ô nhập dùng `TextInput` của Lucerna, có viền 1 px `border_focused`, không bo góc, cao vừa dòng.
- **Xác nhận và hủy:**
  - `Enter` xác nhận.
  - `Esc` hủy.
  - Mất focus: xác nhận nếu tên hợp lệ, hủy nếu không. Đây là cách của VS Code.
  - Trong lúc đang nhập, các phím của Explorer không chạy.
- **Kiểm tra tên** mỗi lần gõ. Lỗi hiện ngay dưới ô nhập, trong một khung nhỏ viền `error`, nền `elevated_surface`. Khi đang có lỗi thì `Enter` không làm gì. Các lỗi:
  - tên rỗng hoặc chỉ có khoảng trắng;
  - bắt đầu bằng `/`;
  - có thành phần `.` hoặc `..`;
  - có ký tự không hợp lệ trên nền tảng hiện tại;
  - đã có file hoặc thư mục cùng tên. Kiểm tra trên snapshot, rồi kiểm tra lại khi thực sự ghi.

  Câu báo lỗi lấy theo VS Code, ví dụ: "A file or folder **x** already exists at this location. Please choose a different name."
- **Đường dẫn nhiều cấp:** `a/b/c.rs` tạo luôn các thư mục `a` và `b` ở giữa, giống VS Code và Zed.
- **Sau khi tạo:**
  - File mới được mở thành tab ghim, và dòng của nó được chọn.
  - Thư mục mới được mở ra trong cây và được chọn.
  - Thao tác chạy ở luồng nền. Dòng mới xuất hiện khi worktree báo có thay đổi, và lúc đó Explorer chọn và cuộn tới nó.
- **Rename:**
  - Đổi tên chỉ khác hoa thường (`readme.md` thành `README.md`) phải chạy được trên ổ không phân biệt hoa thường.
  - Đổi tên sang tên đã có thì báo lỗi, không ghi đè.
  - Mọi tab mở file đó, hoặc mở file nằm trong thư mục được đổi tên, chuyển sang đường dẫn mới. Tab giữ nguyên nội dung đang sửa dở, vị trí con trỏ và trạng thái ghim. Ngôn ngữ và LSP được tính lại theo tên mới.
- **Lỗi khi ghi** (không có quyền, ổ chỉ đọc…): báo bằng `status_message`, dòng ảo đóng lại.

### 5. Delete

- **`moveFileToTrash`:**
  - Nếu `explorer.confirmDelete` bật (mặc định bật), hỏi bằng `window.prompt(Warning, …)`:
    - Tiêu đề: "Are you sure you want to delete '<tên>'?"
    - Chi tiết: "You can restore this file from the Trash." (với thư mục: "…this folder and its contents…")
    - Nút: `Move to Trash`, `Cancel`.
  - Sau đó gọi `trash::delete` ở luồng nền.
  - Nếu không đưa vào Trash được (ổ mạng, ổ ngoài không có Trash…), hỏi tiếp: "Unable to move '<tên>' to the Trash. Do you want to permanently delete instead?" với nút `Delete Permanently` và `Cancel`. Đây là cách VS Code làm.
- **`deleteFile`:** luôn hỏi, không phụ thuộc setting: "Are you sure you want to permanently delete '<tên>'?" với chi tiết "This action is irreversible!". Rồi xóa bằng `std::fs::remove_file` hoặc `remove_dir_all`.
- **Sau khi xóa:**
  - Tab của file bị xóa, hoặc của file nằm trong thư mục bị xóa, sẽ đóng nếu chưa sửa.
  - Tab đang có sửa dở thì giữ lại, ở trạng thái "đã bị xóa" như hiện nay (`DocState::Deleted`).
  - Dòng đang chọn chuyển sang thư mục cha.
- **Không đánh dấu thao tác của mình:** mọi đường dẫn mà Lucerna vừa tạo, đổi tên hoặc xóa đều được bỏ qua khi Explorer đánh dấu "thay đổi từ bên ngoài". Cơ chế của thao tác lưu (`documents.follows_disk`) chỉ áp dụng cho tài liệu đang mở, nên phải có thêm một danh sách riêng các đường dẫn Lucerna đang chờ thấy thay đổi. Mỗi đường dẫn có thời hạn ngắn và bị xóa khỏi danh sách khi worktree đã báo thay đổi đó.

### 6. Settings mới

Tên, mặc định và ngữ nghĩa theo VS Code:

| Key | Mặc định | Ghi chú |
|---|---|---|
| `explorer.confirmDelete` | `true` | |
| `explorer.copyRelativePathSeparator` | `"auto"` | `"/"`, `"\\"`, `"auto"` |
| `terminal.external.osxExec` | `"Terminal.app"` | |
| `terminal.external.linuxExec` | `"x-terminal-emulator"` | VS Code dò theo môi trường desktop, Lucerna lấy một giá trị cố định |
| `terminal.external.windowsExec` | `"cmd.exe"` | để dành cho bản Windows |

### 7. Phần phụ cần thêm

- **Theme:** `UiColors` chưa có `text_disabled` và `border_focused`. Thêm hai trường này, đọc từ khóa `text.disabled` và `border.focused` trong theme kiểu Zed. Nếu theme không có thì lấy `text_muted` và `border`.
- **`TextInput`:** thêm API để đặt vùng chọn (`set_selection(range)`), để Rename chọn sẵn phần tên trước đuôi file.

### 8. Test

- `lucerna-project`:
  - tạo file hoặc thư mục, kể cả đường dẫn nhiều cấp;
  - từ chối khi tên đã có;
  - đổi tên, kể cả chỉ đổi hoa thường;
  - đưa vào Trash (test chạy trên thư mục tạm; nếu môi trường CI không có Trash thì test hành vi khi lỗi);
  - xóa vĩnh viễn;
  - kiểm tra tên hợp lệ (hàm thuần).
- `ContextMenu`:
  - builder bỏ đúng các đường kẻ thừa;
  - `↑` `↓` bỏ qua đường kẻ và mục bị tắt;
  - `Enter` chạy lệnh và đóng menu;
  - `Esc` hoặc bấm ra ngoài thì đóng menu và trả focus;
  - phím tắt hiện đúng.
- Explorer (test GPUI có `debug_selector`):
  - chuột phải chọn dòng và mở đúng bộ mục cho file, cho thư mục và cho gốc;
  - viền của dòng đang mở menu;
  - `shift+f10`;
  - mỗi lệnh chỉ đọc gọi đúng API;
  - New File: dòng ảo, lỗi trùng tên, `Enter` tạo và mở tab, `Esc` hủy;
  - Rename chọn sẵn phần tên, tab đổi theo đường dẫn mới;
  - Delete: hỏi lại, đóng tab chưa sửa, giữ tab đang sửa dở;
  - thao tác của Lucerna không bị đánh dấu là thay đổi từ bên ngoài.
- Kiểm tra giao diện so với Zed theo §1.1 của design-principles: chạy app đặt cạnh Zed và so khoảng cách, chiều cao, màu nền, viền, hover của menu và ô nhập.

## Để người làm quyết

- Kiểu dữ liệu cụ thể của `ContextMenu`, của builder và của `EditState`; chia file trong `explorer/`.
- Cách đưa dòng ảo vào danh sách của `uniform_list`.
- Cách dời tab sang đường dẫn mới trong `documents.rs` và `editor_groups` (đổi key, đóng mở lại LSP).
- Cách báo cho Explorer bỏ qua đường dẫn của chính mình: dùng lại hoặc mở rộng cơ chế của own-save.
- Test cụ thể và tên của test.
- Ranh giới giữa phần 1 và phần 2 trong plan, miễn là phần 1 tự chạy được.

## Quyết định của Claude (người dùng xem lại khi duyệt)

- Bộ lệnh và thứ tự ở §3, gồm cả Find in Folder…. Bỏ Cut/Copy/Paste/Duplicate, chọn nhiều dòng, kéo thả, Undo.
- Open in Terminal mở terminal ngoài, vì Lucerna không có terminal tích hợp.
- Có Delete Permanently (`deleteFile`), nhưng chỉ bằng phím tắt, không có trong menu.
- `shift+f10` mở menu bằng phím, dùng ID `filesExplorer.openContextMenu`.
- Dòng đang chọn đi theo tab đang mở. Chưa làm đi trong cây bằng phím mũi tên.
- Phím trong menu đi qua keymap với `contextMenuVisible`, và `filesExplorerFocus` tắt khi menu đang mở.
- Sửa `explorerViewletFocus` để chỉ đúng khi sidebar đang hiện Explorer.
- Ở chế độ một file, menu chỉ tác động lên file đó và chỉ có các lệnh chỉ đọc.
- Fallback màu: `text_disabled` lấy `text_muted`, `border_focused` lấy `border`.
