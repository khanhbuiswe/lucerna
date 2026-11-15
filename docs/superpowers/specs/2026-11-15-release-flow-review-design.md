# Review code các flow cơ bản trước release v0.1.0

## Mục đích

Trước khi public repo và phát hành v0.1.0, tìm lỗi ở những thao tác mà mọi người dùng mới đều làm trong 10 phút đầu. Lỗi ở tính năng phụ còn chấp nhận được. Lỗi ở flow cơ bản, nhất là lỗi làm mất dữ liệu, thì không.

## Phạm vi

Các flow, xếp theo mức hậu quả nếu hỏng:

1. **Không mất dữ liệu.** Sửa và lưu: ghi atomic, giữ encoding, BOM và kiểu xuống dòng; `files.autoSave`, trim và final newline. Undo và redo. File đổi trên đĩa khi đang có thay đổi chưa lưu: conflict dot, hỏi trước khi ghi đè. Đóng tab hoặc thoát khi còn file chưa lưu. Explorer: tạo, đổi tên (tab đang mở đi theo), xoá vào Trash hoặc xoá hẳn.
2. **Mở được.** `luc .`, `luc <file>`, mở từ Finder hoặc Dock, Open With, khôi phục workspace, Welcome page, Open Recent, Close Folder. File lớn, file nhị phân, file khác encoding.
3. **Theo dõi agent.** Reload khi file trên đĩa đổi (atomic write, xoá rồi tạo lại, `git checkout` hàng nghìn file), giữ vị trí cuộn và fold, đánh dấu trên cây thư mục, màu git trong Explorer, gutter.
4. **Điều hướng hằng ngày.** Quick Open (`>`, `:`), tìm trong project, tìm trong file, split, tab và preview tab, terminal, IME tiếng Việt.

**Ngoài phạm vi:** Vim, LSP, extension, Markdown preview, diff view, hiệu năng, giao diện, tài liệu, license, và dọn dẹp trước khi public repo.

## Cách làm đã chọn

**A. Đọc code theo từng flow.** Mỗi nhóm flow giao cho một agent đọc code, không sửa gì. Agent lần theo đường đi của code từ thao tác của người dùng (phím, click, sự kiện file system) đến kết quả cuối cùng (ghi đĩa, cập nhật view) và tìm:
- trường hợp biên chưa xử lý,
- race condition giữa watcher, ghi file và thao tác người dùng,
- `unwrap`, `expect` và index có thể panic với dữ liệu thật,
- lỗi bị bỏ qua (`let _ =`, `.ok()`) mà đáng ra phải báo cho người dùng,
- trạng thái không khớp nhau (tab, buffer, cây thư mục).

Mỗi lỗi tìm được sẽ có một agent khác, chưa biết kết luận của agent đầu, kiểm chứng lại bằng cách đọc code. Lỗi nào không đứng vững thì bị loại.

### Các cách đã cân nhắc

| Cách | Đánh đổi | Kết quả |
|---|---|---|
| A. Đọc code theo flow | Nhanh, phủ rộng, không cần build. Không bắt được lỗi chỉ lộ ra khi chạy thật (focus, IME, vẽ sai, timing) | **Chọn** |
| B. Chạy app thật theo checklist | Thấy đúng như người dùng thấy, nhưng chậm, mỗi flow chỉ thử được vài trường hợp, và tốn đĩa để build | Không chọn |
| C. A + thêm test cho nhóm 1 + chạy thật một phần | Kỹ nhất, nhưng tốn nhất | Không chọn |

## Đầu ra

Một báo cáo tại `docs/superpowers/reviews/2026-11-15-release-flow-review.md`. Mỗi lỗi ghi:
- mức độ,
- flow bị ảnh hưởng,
- `file:dòng`,
- cách tái hiện cụ thể,
- hướng sửa gợi ý.

Mức độ:
- **P0:** mất hoặc hỏng dữ liệu, crash.
- **P1:** flow cơ bản không làm được.
- **P2:** sai nhưng có cách khác để làm.
- **P3:** nhỏ.

Báo cáo ghi rõ những gì cách A không kiểm được, để bạn tự thử tay trước khi release.

**Không sửa code trong bước này.** Sau khi có báo cáo, bạn chọn những lỗi phải sửa trước release. Việc sửa đi qua spec hoặc plan riêng.

## Ràng buộc

- Agent chỉ đọc code: không build, không chạy `cargo test`, không tạo worktree. Vì vậy không tốn đĩa.
- Repo này có nhiều session làm cùng lúc, nên mọi thứ (báo cáo, spec) được commit theo đường dẫn cụ thể.
- Đoạn lấy từ code vào báo cáo phải ngắn, chỉ đủ để chỉ ra lỗi.

## Để lại cho người làm (quyết định của mình, bạn sửa được)

- Chia agent: một agent cho mỗi nhóm flow (4 agent). Nhóm 1 lớn và quan trọng nhất, nên tách làm hai: lưu file và conflict là một agent, Explorer và đóng hoặc thoát là agent kia. Tổng cộng 5 agent review, cộng các agent kiểm chứng.
- Tự xác định file nào thuộc flow nào, bắt đầu từ `lucerna-workbench`, `lucerna-text`, `lucerna-project`, `lucerna-editor`, `lucerna-git`, `lucerna-terminal`.
- Nếu một lỗi thuộc phần ngoài phạm vi (ví dụ Vim) nhưng làm hỏng flow cơ bản, thì vẫn ghi vào báo cáo.
