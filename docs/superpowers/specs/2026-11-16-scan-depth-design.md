# Giới hạn độ sâu khi quét cây file — thiết kế ngắn

Ngày: 2026-11-16. Trạng thái: đã duyệt. Người dùng chọn "làm như Zed".

## Ý định

Mở `~` (hoặc một thư mục lớn không phải repo) thì Lucerna quét toàn bộ cây
ngay lập tức: tốn CPU, RAM, và đọc vào mọi thư mục con. Người dùng muốn xử
lý giống Zed.

## Tham khảo

Nghiên cứu ngày 2026-11-16 (nhánh main của Zed, VS Code):

- **Zed** (`crates/worktree/src/worktree.rs`, PR #62583, 2026-08): quét
  eager toàn cây, nhưng thư mục **nằm ngoài git repo** ở độ sâu ≥
  `file_scan_depth` (mặc định 5) được ghi là `UnloadedDir` và không quét
  vào. Thư mục đó được tải khi người dùng expand nó, hoặc khi mở một file
  bên trong. Chạm giới hạn thì hiện một thông báo ở thanh trạng thái. Khi
  đi ngược lên tìm `.git`/`.gitignore`, Zed dừng ở `$HOME`.
- Zed **không** chặn hộp thoại quyền của macOS (TCC): `~/Desktop`,
  `~/Library`, `~/Pictures/Photos Library.photoslibrary` chỉ sâu 1–2 cấp
  nên vẫn bị quét. Issue #20913, #48746 vẫn mở.
- **VS Code**: Explorer chỉ đọc thư mục khi expand; không có setting giới
  hạn độ sâu. Find in Files và Cmd+P vẫn duyệt toàn cây bằng ripgrep.

## Lucerna hôm nay

- `lucerna-project/src/worktree.rs`: `Worker::scan` duyệt toàn cây bằng
  `walk_builder` (`ignore::WalkBuilder`), không giới hạn độ sâu.
- `walk_builder` bật `.parents(true)`: đọc `.gitignore` ở mọi thư mục cha,
  tới tận `/`.
- `search.rs` tự duyệt toàn cây bằng cùng `walk_builder`.
- Watcher là `notify` đệ quy (FSEvents trên macOS).

## Cách làm đã cân nhắc

1. **Như Zed: quét eager, giới hạn độ sâu ngoài repo** (chọn). Ít đổi mô
   hình nhất: snapshot vẫn là danh sách đầy đủ của phần đã tải.
2. Như VS Code: chỉ đọc khi expand. Quick Open và search phải duyệt riêng,
   đổi nhiều; không phải hướng người dùng chọn.
3. Như (1) cộng loại trừ thư mục TCC của macOS. Zed không làm; để sau nếu
   người dùng muốn.

## Thiết kế

- **Setting `files.scanDepth`**, mặc định `5`. VS Code không có setting
  tương đương, nên tên theo kiểu `files.*` của VS Code (quyết định của
  người viết spec). `0` hoặc âm: không giới hạn.
- **Quét**: một thư mục ở độ sâu ≥ `files.scanDepth` tính từ root, và
  không nằm trong git repo nào, được ghi vào snapshot với kiểu mới
  `EntryKind::UnloadedDir` và không duyệt vào trong. Thư mục trong git repo
  (root nằm trong repo, hoặc chính nó/thư mục cha của nó trong root có
  `.git`) luôn quét đủ.
- **Tải khi cần**:
  - Expand một `UnloadedDir` trong Explorer: worker đọc thư mục đó một
    cấp; thư mục con tiếp tục là `UnloadedDir` nếu vẫn vượt giới hạn.
  - Mở một file bên trong (CLI, Reveal, nhảy tới định nghĩa): tải các thư
    mục trên đường dẫn tới file đó.
  - Đã tải thì giữ cho tới khi rescan toàn bộ; rescan tải lại những thư
    mục đang expand.
- **Quick Open, Find in Files**: chỉ thấy phần đã tải, như Zed. Search
  dùng cùng quy tắc độ sâu (bỏ qua `UnloadedDir`) thay vì duyệt toàn cây.
- **Watcher**: giữ FSEvents đệ quy; sự kiện bên trong `UnloadedDir` bị bỏ
  qua, trừ việc tạo/xoá chính thư mục đó.
- **Thư mục cha**: khi root nằm dưới `$HOME`, tìm `.gitignore` và git repo
  ở thư mục cha dừng tại `$HOME` (gồm cả `$HOME`), không đi lên tiếp.
- **Thông báo**: lần đầu một lượt quét gặp giới hạn, thanh trạng thái hiện
  `Some folders deeper than 5 levels load when expanded`
  (số lấy theo setting).
- Mọi phần trên không phụ thuộc nền tảng.

## Không làm

- Không chặn hộp thoại quyền macOS. Mở `~` vẫn có thể bật hộp thoại Photos,
  Desktop… như Zed. Nếu muốn, làm cách (3) ở spec sau.
- Không đổi việc mở lại `~` khi khởi động (Zed cũng mở lại).

## Test

- `worktree.rs`: cây sâu 7 cấp không có `.git` → cấp 5 trở xuống là
  `UnloadedDir`; cùng cây có `.git` → quét đủ; `files.scanDepth: 0` → quét
  đủ; expand một `UnloadedDir` tải đúng một cấp; tải theo đường dẫn file.
- `search.rs`: file trong `UnloadedDir` không có trong kết quả.
- Thư mục cha: `.gitignore` phía trên `$HOME` (HOME giả trong test) không
  áp dụng.
- Explorer: `UnloadedDir` hiện như thư mục đóng và expand được.

## Để người làm quyết

Cách đánh dấu "trong git repo" khi duyệt song song, cách thay
`.parents(true)` để dừng ở `$HOME`, và cách hiện thông báo bằng thành phần
có sẵn trong `ui.rs`/`status_message.rs`.
