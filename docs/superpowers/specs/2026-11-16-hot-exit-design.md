# Hot exit: giữ thay đổi chưa lưu khi thoát app

Ngày: 2026-11-16. Nguồn: lỗi P0 T2-1 trong [review trước release](../reviews/2026-11-15-release-flow-review.md). Nghiên cứu: [docs/research/2026-11-16-hot-exit.md](../../research/2026-11-16-hot-exit.md).

## 1. Ý định

Hiện nay, thoát Lucerna bằng Quit trên Dock, Log Out, Restart máy hoặc AppleScript `quit` sẽ làm mất mọi thay đổi chưa lưu mà không hỏi gì. Nguyên nhân:
- Hộp thoại "Save changes?" chỉ chạy trong action `Quit` của Lucerna (Cmd+Q, menu) và khi đóng cửa sổ.
- Các đường thoát kể trên đi qua `-[NSApp terminate:]`. GPUI (`b5cd310`) không cài `applicationShouldTerminate:`, nên chỉ còn các hook `on_app_quit`. Các hook này không huỷ được việc thoát, và chỉ có 200 ms (`SHUTDOWN_TIMEOUT`).

Cách sửa là **hot exit**, giống VS Code (`files.hotExit: "onExit"`, mặc định) và Zed (`session.restore_unsaved_buffers`):
- Lucerna luôn giữ một bản sao lưu của mọi file chưa lưu.
- Thoát app không hỏi gì.
- Lần mở sau, các tab hiện lại đúng nội dung chưa lưu.

Thành công nghĩa là:
- Thoát app bằng **bất kỳ** đường nào, kể cả khi app crash, cũng không mất quá khoảng 1 giây gõ phím cuối cùng.
- Mở lại workspace thì thấy lại đúng các tab chưa lưu, có chấm "chưa lưu", với đúng nội dung.
- Lưu một file đã bị đổi trên đĩa trong lúc app tắt thì được hỏi trước khi ghi đè. Không bao giờ âm thầm ghi đè bản của agent.

## 2. Mục tiêu, không làm, phản mục tiêu

**Mục tiêu**
- Sao lưu liên tục nội dung của mọi document chưa lưu, theo từng workspace.
- Setting `files.hotExit`: `"onExit"` (mặc định) hoặc `"off"`.
- Thoát (Cmd+Q, menu, Welcome Quit, Dock Quit, Log Out/Restart) không hỏi lưu khi `onExit`.
- Khôi phục khi mở lại workspace, kể cả khi khởi động app.
- Dọn bản sao lưu khi không còn cần.

**Không làm**
- `files.hotExit: "onExitAndWindowClose"`. Giá trị này được đọc như `"onExit"`.
- Giữ lịch sử undo qua lần thoát (`files.restoreUndoStack`). Cả VS Code lẫn Zed đều không giữ.
- Hiện các workspace có bản sao lưu chưa khôi phục trong Welcome hay Open Recent (VS Code có mục "More…" cho việc này).
- Tạo lại bản sao lưu cho file chưa có tên. Lucerna chưa có file untitled.
- Linux và Windows: code không giả định macOS, nhưng chỉ kiểm tra trên macOS.

**Phản mục tiêu** (làm được nhưng vẫn là thất bại)
- Âm thầm ghi đè bản trên đĩa mà agent đã đổi trong lúc app tắt.
- Thoát app chậm đi thấy rõ, hoặc gõ phím bị giật vì ghi bản sao lưu.
- Bản sao lưu cũ "sống lại" sau khi người dùng đã lưu, đã Revert hoặc đã chọn Don't Save.
- Bản sao lưu của workspace này hiện ra ở workspace khác.

## 3. Ràng buộc

- Chỉ `lucerna-workbench` được dùng gpui. Phần định dạng và đọc/ghi file sao lưu nên nằm ở chỗ không phụ thuộc gpui, để test được bằng unit test.
- `on_app_quit` không huỷ được việc thoát và chỉ có 200 ms. Dock Quit và Log Out không đi qua action `Quit` của Lucerna.
- Một process có nhiều cửa sổ (spec multi-window). Mỗi cửa sổ có `Documents` riêng.
- Ghi file theo kiểu atomic (temp rồi rename), giống các file trạng thái khác (`workspace_state::write_atomic`).
- Ổ đĩa của tác giả nhỏ. Bản sao lưu phải được dọn, không được tích tụ.
- Hành vi theo VS Code, giao diện theo Zed (`docs/design-principles.md`).

## 4. Các cách đã cân nhắc

**Hướng xử lý lỗi T2-1** (người dùng chọn):

| Cách | Đánh đổi | Kết quả |
|---|---|---|
| A. Gắn `applicationShouldTerminate:` vào delegate của GPUI để mọi đường thoát đều hỏi lưu | Nhỏ, nhưng chặn Log Out/Restart chờ người dùng trả lời; khác mặc định của VS Code và Zed | Không chọn |
| **B. Hot exit** | Lớn hơn, nhưng giống cả VS Code lẫn Zed, không chặn Log Out, và cứu được cả khi crash | **Chọn** |

**File trên đĩa đã đổi trong lúc app tắt** (người dùng chọn):

| Cách | Đánh đổi | Kết quả |
|---|---|---|
| Merge ba bên khi khôi phục, dùng cơ chế merge đang có | Gộp được thay đổi của agent, nhưng bản sao lưu lớn gấp đôi và phức tạp hơn | Không chọn |
| **Như VS Code:** khôi phục nội dung của người dùng; chỉ báo khi lưu | Đơn giản; không mất dữ liệu vì lưu sẽ hỏi trước khi ghi đè | **Chọn** |

**Nơi lưu** (quyết định của mình): mỗi document một file trong thư mục data, giống VS Code. Cách của Zed (một database SQLite) phải thêm dependency mới, trong khi mọi trạng thái khác của Lucerna đều là file JSON.

## 5. Thiết kế

### 5.1 Khi nào ghi bản sao lưu

- Mỗi lần nội dung của một document chưa lưu thay đổi, hẹn ghi bản sao lưu sau **1000 ms** (debounce, như `DEFAULT_BACKUP_SCHEDULE_DELAYS.default` của VS Code). Gõ tiếp thì hẹn lại.
- Ghi ở background executor; không chặn luồng giao diện.
- Document hết "chưa lưu" (lưu xong, Revert, Take Theirs làm nó sạch, đóng tab chọn Don't Save) thì huỷ lần ghi đang hẹn và **xoá** bản sao lưu.
- `files.hotExit: "off"` thì không ghi, và xoá các bản sao lưu của workspace khi mở.
- Hook `on_app_quit` ghi ngay (đồng bộ) các bản sao lưu còn đang hẹn. Hook này có thể bị cắt ở 200 ms. Đó là lý do phải ghi liên tục chứ không đợi tới lúc thoát.

### 5.2 Nơi lưu và định dạng

- Thư mục: `<data_dir>/backups/<khoá workspace>/`.
  - Cửa sổ folder: khoá là SHA-256 (hex) của đường dẫn canonical của folder, giống cách `workspace_state::state_path` băm.
  - Cửa sổ mở một file riêng: khoá là SHA-256 của `"file:" + đường dẫn canonical của file`, để không đụng với cửa sổ folder cha.
- Mỗi document một file: `<sha256 của đường dẫn tuyệt đối của file>`.
- Nội dung file:
  - **Dòng đầu là JSON** (một dòng), gồm:
    - `version`
    - `path`: đường dẫn tuyệt đối
    - `mtime` và `size` của file trên đĩa lúc document được đọc hoặc lưu lần cuối (stamp hiện có); `null` nếu file chưa tồn tại
    - `base_sha256`: SHA-256 của các byte trên đĩa mà document dựa trên. Không dùng `base_hash` hiện có, vì `DefaultHasher` không ổn định giữa các phiên bản Rust. *(Quyết định của mình.)*
    - `encoding`, `bom`, `line_ending`: lấy từ buffer, để lưu ra đúng định dạng cũ
  - **Từ dòng thứ hai** là văn bản của buffer ở dạng UTF-8, xuống dòng `\n` (dạng trong bộ nhớ).
- Ghi bằng temp rồi rename trong cùng thư mục.
- File sao lưu không đọc được (JSON hỏng, sai version) thì đổi tên thành `.bak` và bỏ qua, giống `load_state`.

### 5.3 Khi thoát

**`files.hotExit: "onExit"`:**
- **Cmd+Q, menu Quit, Welcome Quit:**
  1. Với mỗi cửa sổ, cửa sổ active trước: lưu thật các file mà `files.autoSave` cho phép lưu (như VS Code).
  2. Ghi ngay mọi bản sao lưu đang hẹn và đợi xong.
  3. Ghi thành công hết thì thoát, không hỏi lưu.
  4. Ghi lỗi (ví dụ đĩa đầy) thì cửa sổ đó quay về hộp thoại "Save changes?" hiện có, chỉ cho các file ghi lỗi. Cancel huỷ cả việc thoát.
- **Terminal đang chạy tiến trình:** vẫn hỏi như hiện nay. Không đổi.
- **Dock Quit, Log Out, Restart, AppleScript:** chỉ chạy hook `on_app_quit`, ghi nốt các bản sao lưu đang hẹn. Không hỏi gì (vì không hỏi được).

**`files.hotExit: "off"`:** giữ nguyên hành vi hiện nay (hỏi lưu ở mọi cửa sổ khi Cmd+Q).

**Các thao tác vẫn hỏi lưu như hiện nay**, kể cả với `"onExit"` (giống VS Code trên macOS):
- Đóng một cửa sổ (nút đỏ, Cmd+Shift+W).
- Close Folder, Open Folder thay cửa sổ.
- Đóng tab.

Với các thao tác này, Don't Save xoá bản sao lưu của các file đó, còn Cancel giữ nguyên.

### 5.4 Khi mở lại

- Khi một workbench mở một workspace (lúc khởi động, Open Recent, `luc`, Finder…), nó đọc thư mục bản sao lưu của workspace đó.
- Với mỗi bản sao lưu, kể cả file nằm **ngoài** root (ví dụ file mở bằng go-to-definition, hoặc tab khác trong cửa sổ một file):
  - Mở tab nếu file chưa có tab. Đặt vào group đầu, không lấy focus, không phải preview tab.
  - Nạp **văn bản của bản sao lưu** thay cho nội dung trên đĩa, với `encoding`, `bom`, `line_ending` từ dòng đầu.
  - Đọc file trên đĩa (nếu còn) để lấy base cho việc merge sau này:
    - **SHA-256 khớp `base_sha256`:** đĩa chưa đổi. Dùng nội dung đó làm `base` của document, cùng `stamp` và `base_hash` tính mới. Nhờ vậy agent ghi tiếp sau khi khôi phục vẫn được tự merge.
    - **Không khớp, hoặc file không còn:** đĩa đã đổi trong lúc tắt. Để `base` trống, `stamp` lấy từ dòng đầu, và đặt `base_hash` sao cho kiểm tra đĩa trước khi lưu luôn coi là "đã đổi" khi file tồn tại.
    - Không hiện gì cho người dùng ở bước này *(quyết định của mình, để giữ được auto-merge mà vẫn đúng lựa chọn "như VS Code" của bạn)*.
  - Đánh dấu document là chưa lưu.
  - Lịch sử undo bắt đầu trống: không undo về được bản trên đĩa.
- Người dùng chọn cách của VS Code: **không báo** lúc khôi phục rằng đĩa đã đổi. Kiểm tra đĩa trước khi lưu đang có sẽ phát hiện file đã đổi và hỏi trước khi ghi đè.
- **File đã bị xoá trong lúc app tắt:** khôi phục bình thường. Lưu sẽ tạo lại file, giống VS Code và Zed.
- **Thay đổi xảy ra sau khi đã khôi phục** (agent ghi tiếp): đi qua đường merge hoặc conflict đang có. Có `base` thì tự merge như bình thường; `base` trống (đĩa đã đổi trong lúc tắt) thì luôn ra conflict bar.
- **Cửa sổ nào được mở lại khi khởi động:**
  - Cửa sổ folder theo `window.restoreWindows` như hiện nay. Bản sao lưu của folder không được mở lại thì được giữ, tới khi người dùng mở lại folder đó.
  - Cửa sổ mở một file riêng mà có bản sao lưu thì luôn được mở lại, giống "empty window" của VS Code. Vì tên thư mục chỉ là hash, mỗi thư mục bản sao lưu có thêm một file `workspace.json` ghi loại cửa sổ (folder hoặc file) và đường dẫn, để lúc khởi động biết cần mở cửa sổ nào *(quyết định của mình)*.

### 5.5 Dọn dẹp

- Xoá bản sao lưu ngay khi document hết "chưa lưu" (§5.1).
- Mọi bản sao lưu trong thư mục của một workspace đều thuộc workspace đó, vì chỉ cửa sổ của nó ghi vào đây, kể cả cho file ngoài root. Không có bước lọc theo đường dẫn. Bản sao lưu có dòng đầu hỏng thì được đổi thành `.bak` (§5.2) và giữ lại để người dùng tự lấy.
- Thư mục `<khoá workspace>` rỗng thì xoá.
- Không xoá bản sao lưu của workspace chưa được mở lại.

### 5.6 Hai cửa sổ cùng mở một file

Mỗi cửa sổ có thư mục bản sao lưu riêng, nên hai bản sao lưu không đụng nhau. Khi mở lại, mỗi cửa sổ khôi phục bản của mình. Lưu ở một cửa sổ thì cửa sổ kia thấy conflict qua đường đang có.

## 6. Kiểm tra

- **Unit test** cho định dạng: ghi rồi đọc lại; dòng đầu hỏng; sai version; đường dẫn có dấu cách và tiếng Việt; file CRLF, có BOM, UTF-16.
- **Test GPUI**:
  - sửa → đợi debounce → file sao lưu có đúng nội dung;
  - lưu → file sao lưu bị xoá;
  - Revert hoặc đóng tab chọn Don't Save → xoá;
  - Cmd+Q với file chưa lưu → không có prompt (`simulate_prompt_answer` không được gọi), app thoát;
  - mở lại → tab chưa lưu, đúng văn bản;
  - đĩa đổi trong lúc tắt → Cmd+S hỏi trước khi ghi đè;
  - ghi bản sao lưu lỗi khi Cmd+Q → hộp thoại lưu;
  - `files.hotExit: "off"` → hành vi cũ.
- **Thử tay:** Dock Quit, Log Out, và `kill -9` trong lúc đang gõ. `TestPlatform` không mô phỏng được `applicationWillTerminate:`.

## 7. Để lại cho người làm

- Gắn vào sự kiện nào của `Documents` để biết nội dung đổi hoặc hết "chưa lưu".
- Tách module ra sao. Gợi ý: `hot_exit.rs` trong workbench cho phần nối với gpui, và phần định dạng ở module không phụ thuộc gpui.
- `files.hotExit` là setting phạm vi app: chỉ đọc từ settings user, như VS Code *(quyết định của mình)*.
- Ghi bản sao lưu đang hẹn phải xong trước khi xoá nó, để một lần ghi trễ không tạo lại bản đã xoá.
- Chia 200 ms của `on_app_quit` với các hook khác: flush bản sao lưu là việc quan trọng nhất.
- Đưa `files.hotExit` vào default settings và `docs/settings.md`. Cập nhật `docs/features.md` và bỏ ghi chú "Không có hot exit" trong spec phase 4.
- Sau khi xong, cập nhật T2-1 trong báo cáo review.

## Tham khảo

- **VS Code** (`microsoft/vscode`, `main`):
  - `files.hotExit` ở `files.contribution.ts`;
  - `workingCopyBackupTracker.ts` (debounce 1000/2000 ms, xoá khi hết dirty, luồng thoát);
  - `workingCopyBackupService.ts` (bố cục `Backups/<workspace>/<scheme>/<hash>`, dòng tiêu đề kèm meta);
  - `textFileEditorModel.ts` (nạp từ backup, lỗi `FILE_MODIFIED_SINCE` khi lưu).

  Chi tiết và số dòng ở [docs/research/2026-11-16-hot-exit.md](../../research/2026-11-16-hot-exit.md).
- **Zed** (`b5cd310`):
  - bảng `editors` lưu `contents` và `mtime` khi buffer dirty, throttle 200 ms;
  - action Quit chỉ hỏi cho các mục không serialize được;
  - `on_app_quit` flush trong 200 ms;
  - khôi phục giữ `saved_mtime` nên lưu sẽ thấy conflict.
- **Lucerna:**
  - [spec multi-window](2026-11-15-multi-window-design.md) §6.6 (quit);
  - `workspace_state.rs` (`state_path`, `write_atomic`);
  - `save.rs` (`changed_on_disk`, hộp thoại lưu);
  - `workbench.rs` (`quit`, `confirm_quit`).
