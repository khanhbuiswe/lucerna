# Explorer: lần lưu của chính Lucerna không bị đánh dấu "đổi từ bên ngoài"

Thiết kế ngắn cho một lỗi. Người dùng đã duyệt ngày 2026-11-13.

## Lỗi

Video của người dùng: sửa `flashcards.ts` (Vim `ciw`, Esc), lưu, rồi `u` và lưu lại. Sau mỗi lần lưu, tab trở lại nút × (đúng), nhưng hàng `flashcards.ts` trong Explorer in đậm và có chấm trắng, và giữ như vậy tới khi chuyển tab khác rồi quay lại.

Chấm đó là đánh dấu "file vừa đổi trên đĩa từ bên ngoài" của Phase 1 (`explorer.decorations.externalChanges`, `CHANGE_DECORATION` = 10 s). Phase 1 chưa sửa được file, nên `ExplorerView::on_worktree_event` coi mọi `WorktreeEvent::Changed` (trừ `Removed`) là thay đổi bên ngoài. Từ Phase 4, Lucerna tự ghi file khi lưu, và watcher báo lại chính lần ghi đó, nên nó bị đánh dấu nhầm. Dấu chỉ bị xóa khi tab active *đổi* sang file đó, nên file đang active giữ dấu suốt 10 s.

## Tham khảo

- VS Code: Explorer chỉ tô theo git (`explorer.decorations.colors`/`badges`) và lỗi; không có dấu "vừa đổi từ bên ngoài". Lưu file không đổi gì trong cây. Trạng thái chưa lưu chỉ ở tab và Open Editors.
- Zed: project panel tô theo git status và diagnostics; không có dấu thay đổi ngoài.
- Dấu này là tính năng riêng của Lucerna (báo khi agent sửa file), nên chỉ cần nó báo đúng: chỉ thay đổi thật sự từ bên ngoài, và không báo cho file người dùng đang nhìn.

## Mục tiêu

- Lần lưu của Lucerna (lưu tay, autosave, lưu khi đóng) không đánh dấu file.
- Ghi lại đúng nội dung đã có (`touch`, agent ghi lại y nguyên) không đánh dấu file đang mở.
- File ở tab active không bị đánh dấu, kể cả khi agent sửa nó (người dùng đang nhìn nó; spec Phase 1: mở file thì xóa dấu).
- File chưa mở, hoặc mở ở tab nền, vẫn được đánh dấu khi bị sửa từ bên ngoài, như trước.

Ngoài phạm vi: đổi giao diện của dấu, thêm git decoration, dấu chưa lưu trong Explorer.

## Các cách đã cân nhắc

1. **Để `Documents` quyết định cho file đang mở (chọn).** `Documents` đã đọc lại file sau mỗi sự kiện `Modified` và so hash với `base_hash` (`finish_reload`); bằng nhau nghĩa là lần lưu của chính nó hoặc ghi y nguyên. Chỉ khi khác (`Apply`, `Merge`, `Conflict`) mới là thay đổi bên ngoài thật. Dựa vào nội dung nên không chạy đua với thời điểm sự kiện; `start_job` cũng đã chờ lần ghi đang chạy xong (`reload_after_save`).
2. Ghi nhớ "Lucerna vừa ghi file này" lúc lưu rồi lọc sự kiện. Phải tự lo thứ tự giữa lúc ghi xong và lúc watcher báo, và lặp lại việc `Documents` đã làm. Bỏ.
3. Chỉ bỏ qua file ở tab active. Không sửa trường hợp lưu file ở tab nền (Save All, autosave khi đổi focus). Bỏ; giữ như một quy tắc phụ.

## Thiết kế

- `DocumentEvent::ChangedOnDisk { path }`: `Documents` phát khi một lần đọc lại tìm ra nội dung khác `base_hash`, tức nhánh `Apply`, `Merge` hoặc `Conflict` của `finish_reload`.
- `Documents::follows_disk(path) -> bool`: tài liệu đang mở và đã có buffer (có lần đọc lại để so). Tài liệu chưa nạp xong hay lỗi nạp thì không, để Explorer tự đánh dấu như cũ.
- `Workbench::on_worktree_event`: khi tính các đường dẫn cần đánh dấu, bỏ các đường dẫn mà `Documents::follows_disk` nhận; `ExplorerView::on_worktree_event` nhận một bộ lọc thay cho `decorate: bool`.
- `Workbench` nghe `ChangedOnDisk` và, nếu setting bật, gọi `ExplorerView::mark_external_change(path)`.
- `ExplorerView` không đánh dấu đường dẫn đang là tab active, ở cả hai lối vào.

## Test

Trong `crates/lucerna-workbench/tests/`:
- Sửa rồi Cmd+S, watcher báo `Modified`: không có dấu.
- File mở ở tab nền bị agent ghi nội dung khác: có dấu.
- Cùng trường hợp nhưng ghi y nguyên nội dung: không có dấu.
- File active bị agent ghi: không có dấu.
- Setting tắt: `ChangedOnDisk` không đánh dấu.
- Các test cũ (`changed_event_decorates_until_opened`, `changed_decoration_respects_setting`) vẫn qua.
