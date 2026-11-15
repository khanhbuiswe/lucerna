# Review các flow cơ bản trước release v0.1.0

Ngày: 2026-11-15. Spec: [2026-11-15-release-flow-review-design.md](../specs/2026-11-15-release-flow-review-design.md). Plan: [2026-11-15-release-flow-review.md](../plans/2026-11-15-release-flow-review.md).

**Cách làm.** Năm agent đọc code theo năm nhóm flow. Mỗi lỗi tìm được do một agent khác đọc lại code để kiểm chứng; agent này không được xem lập luận của agent trước. Không có lỗi nào được kiểm chứng bằng cách chạy app hay chạy test. Mọi kết luận đều đến từ việc đọc code tại commit `ff0498b`.

## Tóm tắt

| Mức | Số lỗi |
|---|---|
| P0 (mất/hỏng dữ liệu, crash) | 2 |
| P1 (flow cơ bản không làm được) | 0 |
| P2 (sai nhưng có cách khác) | 11 |
| P3 (nhỏ) | 13 |

Có 2 mục bị loại, 1 mục bị gộp vì trùng.

**Nên sửa trước release:**
- **T2-1 [P0]:** Quit từ Dock, Log Out hoặc Restart máy thoát ngay mà không hỏi lưu, nên mất thay đổi chưa lưu.
- **T2-2 [P2]:** đổi tên chỉ khác hoa/thường để lại một mục "ma" trong Explorer. Nếu chọn Delete Permanently trên mục đó thì file thật bị xoá.
- **T1-1 [P0]:** file có byte sai encoding bị ghi thành `U+FFFD` khi lưu, mà không có cảnh báo nào.

## Danh sách lỗi

### T1-1 [P0] Byte sai encoding bị thay bằng U+FFFD khi lưu, kể cả chỗ không sửa

- **Flow:** lưu file không phải UTF-8 hoặc có BOM.
- **Vị trí:** [load.rs:88](../../../crates/lucerna-text/src/load.rs), [write.rs:145](../../../crates/lucerna-text/src/write.rs).
- **Tái hiện:** mở file UTF-8 có BOM nhưng lẫn byte `0xE9`, hoặc file UTF-16 có số byte lẻ. Sửa chỗ khác rồi lưu: byte hỏng thành `EF BF BD`.
- **Biến thể:** với encoding nhiều byte (Shift_JIS, GBK…) thì lưu thất bại ở một dòng người dùng không sửa.
- **Nguyên nhân:** `decode_without_bom_handling` bỏ cờ `had_errors`.
- **Mức độ:** theo thang của spec, đổi byte người dùng không sửa là hỏng dữ liệu, nên xếp P0. Lỗi hiếm gặp: file UTF-8 hợp lệ và file Latin-1 không bị ảnh hưởng, và VS Code cũng ghi U+FFFD. Agent kiểm chứng đề xuất P2 vì lý do đó.
- **Hướng sửa:** dùng `had_errors` để cảnh báo, hoặc mở chỉ đọc.

### T2-1 [P0] Thoát từ Dock, Log Out/Restart hoặc AppleScript bỏ qua hộp thoại lưu

- **Flow:** thoát app khi còn file chưa lưu.
- **Vị trí:** [workbench.rs:2907](../../../crates/lucerna-workbench/src/workbench.rs), [workbench.rs:1673](../../../crates/lucerna-workbench/src/workbench.rs), [workbench.rs:860](../../../crates/lucerna-workbench/src/workbench.rs).
- **Tái hiện:** sửa một file, không lưu. Chuột phải icon Dock → Quit, hoặc Apple menu → Restart. App tắt ngay. Mở lại thì thay đổi đã mất.
- **Nguyên nhân:**
  - Chỉ action `Quit` của Lucerna (Cmd+Q, menu; [app_menu.rs:570](../../../crates/lucerna-workbench/src/app_menu.rs)) và việc đóng cửa sổ mới đi qua `confirm_quit`.
  - Delegate macOS của GPUI (rev `b5cd310`, `gpui_macos/src/platform.rs:91`) chỉ đăng ký `applicationWillTerminate:`, không đăng ký `applicationShouldTerminate:`. Vì vậy `-[NSApp terminate:]` thoát ngay.
  - Hook `on_app_quit` của Lucerna chỉ lưu layout và tắt LSP.
  - Không có hot exit hay bản sao lưu nào.
- **Hướng sửa:** chặn terminate ở tầng platform: `applicationShouldTerminate:` trả `NSTerminateCancel` rồi chạy `quit(cx)`. GPUI là code của Zed, nên cần patch hoặc hook riêng. Một lựa chọn khác là ghi bản sao lưu các buffer chưa lưu trong hook quit.

### T2-2 [P2] Đổi tên chỉ khác hoa/thường để lại mục "ma" trong Explorer

- **Flow:** Explorer Rename, và agent chạy `mv`/`git mv`, trên APFS không phân biệt hoa thường. Lỗi này gộp T4-3.
- **Vị trí:** [worktree.rs:471](../../../crates/lucerna-project/src/worktree.rs), [watch.rs:126](../../../crates/lucerna-project/src/watch.rs), [explorer/view.rs:794](../../../crates/lucerna-workbench/src/explorer/view.rs).
- **Tái hiện:** đổi tên `foo.rs` thành `Foo.rs`. Explorer hiện cả hai.
  - Bấm `foo.rs` thì mở ra document thứ hai cho cùng một file.
  - Delete `foo.rs` thì xoá file thật. Mặc định là vào Thùng rác; nếu chọn Delete Permanently thì mất hẳn.
- **Nguyên nhân:** `symlink_metadata("foo.rs")` vẫn thành công, nên đường dẫn cũ bị coi là Modified thay vì Removed.
- **Hướng sửa:** đường dẫn đã biết mà "vẫn tồn tại" thì so tên thật trong `read_dir` của thư mục cha. Nếu khác thì coi là Removed.

### T2-3 [P2] Thoát khi Paste (copy) đang chạy để lại bản copy dở

- **Flow:** Explorer copy/paste.
- **Vị trí:** [explorer/clipboard.rs:263](../../../crates/lucerna-workbench/src/explorer/clipboard.rs), [workbench.rs:1676](../../../crates/lucerna-workbench/src/workbench.rs).
- **Tái hiện:** Paste một folder vài GB rồi Cmd+Q ngay. App thoát mà không hỏi. Bản `X copy` còn lại trên đĩa thì thiếu file, hoặc có file bị cụt.
- **Nguyên nhân:** task copy bị `detach`, và `confirm_quit` chỉ xét buffer chưa lưu và terminal. Không có thao tác file nào được theo dõi lúc thoát: `deletes_in_flight` ([explorer/commands.rs:288](../../../crates/lucerna-workbench/src/explorer/commands.rs)) cũng không được đọc ở đó, nên Cut hay Delete đang chạy cũng không được bảo vệ.
- **Hướng sửa:** đếm các thao tác copy đang chạy và hỏi hoặc đợi khi thoát. Hoặc copy vào tên tạm rồi rename khi xong.

### T3-1 [P2] Open With hoặc thả lên Dock một thư mục con của folder đang mở không làm gì

- **Flow:** mở từ Finder hoặc Dock.
- **Vị trí:** [open_requests.rs:77](../../../crates/lucerna-workbench/src/open_requests.rs), [app.rs:445](../../../crates/lucerna-workbench/src/app.rs).
- **Tái hiện:** chạy `luc ~/proj`. Trong Finder, Open With Lucerna trên `~/proj/src`: chỉ kích hoạt cửa sổ hiện có.
- **Kỳ vọng:** docs ghi "anything else opens in a new window".
- **Hướng sửa:** trong `route`, chỉ xếp file và chính root vào `ThisWindow`. Thư mục con thì trả `NewProcess`.

### T3-2 [P2] Ở chế độ mở một file, Open Recent ẩn folder cha

- **Flow:** Open Recent (Ctrl+R và menu).
- **Vị trí:** [workbench.rs:961](../../../crates/lucerna-workbench/src/workbench.rs).
- **Tái hiện:** mở `~/proj` một lần. Sau đó chạy `luc ~/proj/README.md` rồi bấm Ctrl+R: không thấy `~/proj`.
- **Nguyên nhân:** `opened_paths` luôn có `root`, mà ở chế độ một file thì root chính là thư mục cha.
- **Hướng sửa:** ở chế độ một file, chỉ trả về chính file đó.

### T3-4 [P2] Mở FIFO thì tab kẹt Loading; `luc /dev/zero` dùng hết RAM rồi crash

- **Flow:** mở file đặc biệt.
- **Vị trí:** [documents.rs:2576](../../../crates/lucerna-workbench/src/documents.rs), [worktree.rs:270](../../../crates/lucerna-project/src/worktree.rs), [target.rs:74](../../../crates/lucerna/src/target.rs).
- **Tái hiện:**
  - Chạy `mkfifo proj/pipe`, rồi bấm vào `pipe` trong Explorer: tab kẹt Loading.
  - Chạy `luc /dev/zero`: app đọc tới khi hết bộ nhớ.
- **Nguyên nhân:** `metadata.len()` bằng 0 nên lọt qua `check_size`. Code không kiểm `is_file()`.
- **Mức độ:** crash do hết RAM theo thang là P0. Mình hạ xuống P2 vì cần đầu vào cố ý (`/dev/zero`); FIFO trong project chỉ làm treo tab.
- **Hướng sửa:** nếu `!meta.is_file()` thì trả `LoadError` ("Not a regular file").

### T3-5 [P2] Folder gốc không đọc được thì Explorer trống mà không báo

- **Flow:** mở folder khi macOS TCC từ chối quyền, hoặc folder `chmod 000`.
- **Vị trí:** [worktree.rs:661](../../../crates/lucerna-project/src/worktree.rs).
- **Tái hiện:** chạy `chmod 000 proj; luc proj`: cây trống. Người dùng tưởng folder rỗng.
- **Nguyên nhân:** `let Ok(dirent) = result else { continue }` bỏ qua mọi lỗi khi duyệt thư mục.
- **Hướng sửa:** lỗi ở chính root thì gửi một sự kiện lỗi để Explorer hiện thông báo.

### T4-1 [P2] Thư mục gốc bị đổi tên, vào Thùng rác hoặc ổ bị eject: không ai phát hiện

- **Flow:** theo dõi agent.
- **Vị trí:** [coalesce.rs:49](../../../crates/lucerna-project/src/coalesce.rs).
- **Tái hiện:** mở `proj`, rồi từ terminal khác chạy `mv proj proj2`. Cây và tab vẫn như cũ, không có thông báo. Lưu thì báo lỗi.
- **Nguyên nhân:**
  - Sự kiện có đường dẫn tương đối rỗng bị bỏ.
  - notify tạo stream không có cờ `WatchRoot`.
  - Không chỗ nào kiểm tra `root.exists()`.
- **Mức độ:** không mất dữ liệu, vì save kiểm tra đĩa trước khi ghi.
- **Hướng sửa:** coi sự kiện ở root là tín hiệu rescan. Rescan thấy root không còn thì báo và đánh dấu các tab là Deleted.

### T4-2 [P2] Tab mở qua symlink không tự reload khi file đích đổi

- **Flow:** reload khi agent sửa file.
- **Vị trí:** [workbench.rs:1264](../../../crates/lucerna-workbench/src/workbench.rs), [documents.rs:476](../../../crates/lucerna-workbench/src/documents.rs).
- **Tái hiện:** trong repo có `AGENTS.md -> CLAUDE.md`. Mở `AGENTS.md` từ Explorer, sau đó agent sửa `CLAUDE.md`: tab vẫn hiện nội dung cũ.
- **Nguyên nhân:**
  - Key của document không resolve symlink.
  - `OpenFileWatcher` (vốn so theo `real_path`) bỏ qua mọi file có trong snapshot.
- **Mức độ:** save vẫn kiểm tra đĩa trước khi ghi. Mở bằng `luc` thì không bị, vì đường đó có canonicalize.
- **Hướng sửa:** đưa các tab là symlink vào `OpenFileWatcher`, hoặc giữ một map từ real path về key.

### T5-2 [P2] Group bên trái rỗng không tự đóng

- **Flow:** split editor.
- **Vị trí:** [editor_groups/model.rs:605](../../../crates/lucerna-workbench/src/editor_groups/model.rs).
- **Tái hiện:** mở `a.rs`, bấm Cmd+\, rồi Cmd+W ở group trái: một nửa màn hình trống. Không có lệnh đóng group, và trạng thái này còn lại cả sau khi khôi phục workspace.
- **Nguyên nhân:** điều kiện `g > 0`.
- **Kỳ vọng:** VS Code mặc định `workbench.editor.closeEmptyGroups: true`.
- **Hướng sửa:** bỏ group 0 khi nó rỗng mà vẫn còn group khác.

### T5-3 [P2] Quick Open không khớp tên file tiếng Việt dạng NFD, highlight lệch

- **Flow:** Quick Open.
- **Vị trí:** [fuzzy.rs:144](../../../crates/lucerna-project/src/fuzzy.rs), [quick_open/picker.rs:73](../../../crates/lucerna-workbench/src/quick_open/picker.rs).
- **Tái hiện:** file `ghi-chú.md` có tên lưu dạng NFD. Cmd+P, gõ `chú`: không có kết quả. Gõ `chu` thì ra.
- **Nguyên nhân:**
  - Không chỗ nào chuẩn hoá NFC.
  - nucleo gộp grapheme, và trả vị trí theo grapheme, trong khi picker coi đó là chỉ số char.
- **Hướng sửa:** chuẩn hoá NFC cả đường dẫn lẫn query, rồi map vị trí về chỉ số char.

### T5-6 [P2] Tìm trong project bỏ qua thay đổi chưa lưu, click nhảy sai dòng

- **Flow:** Cmd+Shift+F.
- **Vị trí:** [search.rs:107](../../../crates/lucerna-project/src/search.rs), [search_view/view.rs:658](../../../crates/lucerna-workbench/src/search_view/view.rs).
- **Tái hiện:** chèn 10 dòng vào đầu `a.rs`, không lưu. Tìm một từ ở dòng 50 cũ, rồi click kết quả: con trỏ tới dòng 50 của buffer, không có highlight.
- **Kỳ vọng:** VS Code tìm cả trong editor chưa lưu.
- **Hướng sửa:** với file chưa lưu, tìm trên snapshot của buffer.

### T1-2 [P3] Autosave có thể tạo lại file mà agent vừa `rm`/`mv`

- **Flow:** `files.autoSave` khi agent xoá hoặc đổi tên file.
- **Vị trí:** [save.rs:274](../../../crates/lucerna-workbench/src/save.rs).
- **Tái hiện:** đặt `files.autoSave: afterDelay`, rồi gõ vào `a.rs`. Agent chạy `rm a.rs` đúng lúc timer sắp nổ. File `a.rs` xuất hiện lại.
- **Nguyên nhân:** `changed_on_disk` coi NotFound là "không đổi". Vì vậy autosave chạy trong khoảng 50–150 ms trước khi watcher báo sẽ ghi lại file. Kết quả: `rm` của agent bị huỷ, còn `mv` thì sinh ra file trùng.
- **Hướng sửa:** với `SaveReason::Auto`, gặp NotFound thì không ghi.

### T1-3 [P3] Stamp sau khi lưu lấy sau bước rename

- **Flow:** lưu file khi agent ghi ngay sau đó.
- **Vị trí:** [save.rs:263](../../../crates/lucerna-workbench/src/save.rs).
- **Tái hiện:** Cmd+S, agent ghi file trong vài micro giây sau bước rename, rồi người dùng lưu lại trước khi watcher báo. Bản của agent bị ghi đè mà không hỏi. Gần như chỉ xảy ra trên lý thuyết.
- **Hướng sửa:** lấy metadata từ file handle của file tạm trước khi rename.

### T1-5 [P3] Tên file dài hơn khoảng 241 byte thì không lưu được

- **Flow:** sửa và lưu.
- **Vị trí:** [write.rs:263](../../../crates/lucerna-text/src/write.rs).
- **Tái hiện:** mở file có tên 245 byte, sửa, Cmd+S. Status bar báo "File name too long".
- **Nguyên nhân:** tên file tạm `.{name}.luc~XXXXXXXX` dài hơn tên gốc 14 byte.
- **Hướng sửa:** cắt ngắn hoặc hash phần tên trong tên file tạm.

### T1-6 [P3] Sau Keep Mine, autosave không được hẹn lại

- **Flow:** conflict với `files.autoSave: afterDelay`.
- **Vị trí:** [documents.rs:1063](../../../crates/lucerna-workbench/src/documents.rs), [save.rs:404](../../../crates/lucerna-workbench/src/save.rs).
- **Tái hiện:** đặt autoSave afterDelay. Gõ vào file, agent ghi đè cùng dòng nên thành conflict. Chờ quá thời gian delay rồi bấm Keep Mine. Không có lần lưu nào chạy cho tới lần gõ tiếp theo; tab vẫn có chấm "chưa lưu".
- **Hướng sửa:** hẹn lại timer khi nhận `ConflictChanged{conflict:false}` mà văn bản vẫn chưa lưu.

### T1-7 [P3] `files.trimFinalNewlines` với file toàn dòng trống còn lại `\n`

- **Flow:** lưu với `files.trimFinalNewlines`.
- **Vị trí:** [save.rs:212](../../../crates/lucerna-workbench/src/save.rs).
- **Tái hiện:** bật `files.trimFinalNewlines`, lưu file có nội dung `\n\n\n`. File còn lại `\n`.
- **Ghi chú:** VS Code cho ra file rỗng; agent nói chắc khoảng 80%. Test hiện tại khẳng định hành vi `\n`.
- **Hướng sửa:** đối chiếu với `TrimFinalNewLinesParticipant` của VS Code.

### T2-4 [P3] Cmd+Q chọn Don't Save khi đang ghi thì để lại file tạm `.name.luc~XXXX`

- **Flow:** thoát app khi đang lưu dở.
- **Vị trí:** [write.rs:253](../../../crates/lucerna-text/src/write.rs), [workbench.rs:1696](../../../crates/lucerna-workbench/src/workbench.rs).
- **Tái hiện:** Cmd+S một file lớn, Cmd+Q ngay, chọn Don't Save. Còn lại file `.<tên>.luc~xxxxxxxx` cạnh file gốc.
- **Ghi chú:** file gốc vẫn nguyên. Ngoại lệ hiếm: file có hardlink đi qua `write_in_place`, khi đó file có thể bị cụt.
- **Hướng sửa:** trước khi thoát, đợi các lần lưu đang chạy xong.

### T2-5 [P3] New File với tên `foo/` tạo file thay vì folder

- **Flow:** Explorer New File.
- **Vị trí:** [fs_ops.rs:79](../../../crates/lucerna-project/src/fs_ops.rs).
- **Tái hiện:** Explorer → New File → gõ `components/` → Enter. Kết quả là file rỗng tên `components`.
- **Ghi chú:** VS Code tạo folder trong trường hợp này. Agent chưa kiểm lại trong source VS Code.
- **Hướng sửa:** tên kết thúc bằng dấu phân cách thì coi là New Folder.

### T3-3 [P3] Mục recent không còn tồn tại bị xoá vĩnh viễn ở lần ghi kế tiếp

- **Flow:** Open Recent, Welcome page.
- **Vị trí:** [recent.rs:127](../../../crates/lucerna-workbench/src/recent.rs).
- **Tái hiện:** mở `/Volumes/SSD/proj`, rút ổ, mở folder khác, rồi cắm lại ổ. `proj` đã mất khỏi danh sách recent.
- **Ghi chú:** đây là hành vi cố ý theo doc comment, nhưng khác VS Code.
- **Hướng sửa:** chỉ lọc các mục không tồn tại lúc hiển thị, không lọc lúc ghi.

### T4-4 [P3] Watcher không khởi động được thì chỉ ghi log

- **Flow:** theo dõi agent.
- **Vị trí:** [worktree.rs:312](../../../crates/lucerna-project/src/worktree.rs), [git/mod.rs:762](../../../crates/lucerna-workbench/src/git/mod.rs).
- **Tái hiện:** watcher của worktree hoặc của `.git` không khởi động được, ví dụ trên volume mạng, hoặc hết inotify watch trên Linux. Auto-reload và màu git ngừng hoạt động mà không có thông báo nào. Trên macOS hiếm gặp.
- **Hướng sửa:** hiện thông báo trên status bar.

### T4-5 [P3] File bị ignore có thư mục cha bị xoá rồi tạo lại có thể không được watch lại

- **Flow:** reload file bị ignore (`dist/`, `target/`).
- **Vị trí:** [open_files.rs:76](../../../crates/lucerna-workbench/src/open_files.rs).
- **Tái hiện:**
  1. Mở `dist/bundle.js`.
  2. Agent chạy `rm -rf dist`.
  3. Người dùng mở hoặc đóng một tab khác cũng nằm ngoài cây, ví dụ `.env`.
  4. Agent build lại.

  Kết quả: tab vẫn ở trạng thái Deleted.
- **Ghi chú:** một lần Rescanned sẽ tự cứu được.
- **Hướng sửa:** watch thư mục tổ tiên gần nhất còn tồn tại.

### T5-1 [P3] Click kết quả search vượt cuối file để lại observer, về sau con trỏ nhảy

- **Flow:** tìm trong project, click kết quả.
- **Vị trí:** [search_view/view.rs:672](../../../crates/lucerna-workbench/src/search_view/view.rs).
- **Tái hiện:**
  1. Tìm có kết quả ở dòng 100 của `a.rs`.
  2. Agent ghi lại `a.rs`, chỉ còn 50 dòng.
  3. Click kết quả.
  4. Sau đó gõ, hoặc agent ghi, làm file dài quá 100 dòng.

  Con trỏ nhảy một lần về dòng 100.
- **Hướng sửa:** khi doc đã Ready mà dòng không còn tồn tại thì gỡ observer.

### T5-4 [P3] Quick Open không nhận `:-N` và `:N,M`

- **Flow:** Quick Open, đi tới dòng.
- **Vị trí:** [quick_open/query.rs:42](../../../crates/lucerna-workbench/src/quick_open/query.rs).
- **Tái hiện:** Cmd+P, gõ `:-1` hoặc `:12,5`, rồi Enter. Không có gì xảy ra. `:N` và `:N:M` thì vẫn chạy.
- **Hướng sửa:** parse `i64`, số âm thì tính từ cuối file; tách phần dòng/cột theo `[:,#]`.

### T5-5 [P3] Tìm trong project bằng regex có `$` không khớp file CRLF

- **Flow:** tìm trong project, chế độ regex.
- **Vị trí:** [search.rs:122](../../../crates/lucerna-project/src/search.rs).
- **Tái hiện:** file CRLF có dòng `foo`. Bật regex, tìm `foo$`: không có kết quả. Cmd+F trong chính file đó thì tìm thấy.
- **Hướng sửa:** thêm `.crlf(true)` vào `RegexMatcherBuilder`.

## Đã loại

- **T1-4** (file lẫn CRLF/LF bị chuẩn hoá theo kiểu đa số khi lưu): hành vi cố ý. Xem [phase4 spec](../specs/2026-11-11-lucerna-phase4-editing-design.md) và test `mixed_endings_pick_majority`. VS Code cũng làm vậy.
- **T4-6** (`git init` sau khi mở folder không bật Git): Git vẫn tự bật, vì `git init` tạo `.git/info/exclude` và việc này kích hoạt rescan. Chỉ kẹt khi không có template. Spec git-view cũng ghi "không có repo thì tắt, không báo".
- **T4-3:** trùng T2-2, đã gộp vào đó.

## Cần thử tay trước release

Đọc code không kiểm được những điều sau:

- [ ] Quit từ Dock và Restart máy khi có file chưa lưu (xác nhận T2-1 trên app thật).
- [ ] Gõ Telex/VNI trong Quick Open, Find và Search: Enter để chốt chữ có chọn luôn dòng kết quả không? Thử với bộ gõ của Apple và với EVKey/OpenKey.
- [ ] Terminal với bộ gõ kiểu "backspace giả" (EVKey/OpenKey).
- [ ] Finder gửi đường dẫn NFC hay NFD. Mở `luc` bằng đường dẫn NFC trong khi tên trên đĩa là NFD.
- [ ] FSEvents khi chính thư mục gốc bị xoá hoặc đổi tên, và khi ổ được mount lại.
- [ ] Sau Close Folder hoặc Open Folder, watcher, scan và LSP của folder cũ có dừng không.
- [ ] Đóng tab terminal có kill shell không.
- [ ] Trash với đường dẫn có dấu cách hoặc tiếng Việt, và trên volume không có Trash.
- [ ] Thoát app khi đang xoá một folder lớn.
- [ ] Hộp thoại `window.prompt` của GPUI có chặn thao tác khác không (Delete song song với Rename).
- [ ] Merge hoặc reload đúng lúc đang gõ IME, hoặc đang ở Insert mode của Vim.
- [ ] Lưu file thuộc user khác, hoặc file có xattr/ACL (rename atomic có làm mất các thuộc tính đó không).
- [ ] Mở cùng một file ở hai cửa sổ.
