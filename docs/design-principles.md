# Nguyên tắc thiết kế Lucerna

Các nguyên tắc dưới đây áp dụng cho mọi tính năng và mọi giai đoạn. Chúng đứng trên từng spec riêng lẻ.

## 1. Luôn tham khảo VS Code và Zed trước khi lên plan

Lucerna muốn là một bản VS Code gọn hơn, với giao diện theo kiểu Zed. Vì vậy, **trước khi brainstorm, viết spec hay lập plan cho bất kỳ tính năng nào**, phải xem cả hai đã làm tính năng đó ra sao.

1. **VS Code.** Đọc tài liệu người dùng, tài liệu API hoặc contribution points, và mã nguồn ở `microsoft/vscode`. Nếu tính năng nằm trong một extension phổ biến (ví dụ VSCodeVim), đọc luôn extension đó. Cần nắm được:
   - người dùng thấy tính năng thế nào;
   - tên khóa settings, ID lệnh và phím tắt;
   - các trường hợp biên mà VS Code đã gặp phải.
2. **Zed.** Đọc tài liệu và mã nguồn ở `zed-industries/zed`, ưu tiên đúng revision GPUI mà Lucerna đang pin. Cần nắm được:
   - kiến trúc;
   - cách làm cho nhanh;
   - phong cách giao diện.
3. **Ghi lại kết quả** vào `docs/research/YYYY-MM-DD-<chủ-đề>.md`, hoặc vào mục "Tham khảo" trong spec của tính năng đó. Phần ghi lại gồm:
   - mỗi bên làm thế nào;
   - điểm giống nhau và khác nhau giữa hai bên;
   - Lucerna chọn theo bên nào, và vì sao.
4. **Thứ tự ưu tiên khi hai bên khác nhau:**
   - Hành vi, settings, lệnh và phím tắt: theo **VS Code**, để người dùng VS Code chuyển sang không phải học lại.
   - Giao diện và hiệu năng: theo **Zed**.
   - Nếu lệch khỏi cả hai thì phải ghi rõ lý do trong spec.

## 2. Giấy phép khi tham khảo

| Nguồn | Giấy phép | Được làm gì |
|---|---|---|
| Zed: phần lớn các crate (`editor`, `vim`, `extension_host`, `language`, `theme`…) | GPL-3 / AGPL | **Chỉ học thiết kế, không chép code.** Định dạng dữ liệu (theme JSON, `extension.toml`, query `.scm`, keymap) thì dùng được. |
| Zed: `gpui*`, `sum_tree`, `zed_extension_api`, file WIT | Apache-2.0 | Được dùng làm dependency hoặc tham khảo. |
| VS Code (`microsoft/vscode`) và các extension MIT | MIT | Được tham khảo hoặc dùng lại, nhưng **phải ghi nguồn** trong `THIRD_PARTY_NOTICES.md` hoặc trong comment. |
| Font, icon (Lilex, IBM Plex, Lucide…) | OFL / ISC / MIT | Lấy từ repo gốc, kèm file giấy phép nguyên văn. |

Mã nguồn Lucerna dùng giấy phép MIT OR Apache-2.0. `cargo-deny` sẽ chặn dependency có giấy phép GPL, AGPL hoặc LGPL.

## 3. Các nguyên tắc khác đã chốt

- Lucerna hướng tới người viết code bằng agent CLI (Claude Code, Codex). Việc **đọc code** là ưu tiên số một. Không có tính năng AI.
- Luồng UI không bao giờ được chờ I/O hay phân tích cú pháp. Việc nặng chạy ở luồng nền trên snapshot bất biến.
- Chỉ crate `lucerna-workbench` được phụ thuộc vào GPUI.
- Settings và keybindings theo ngữ nghĩa của VS Code: gộp object qua các lớp, override theo ngôn ngữ, và thứ tự ưu tiên giống VS Code.
- Mỗi tính năng có test, review từng task, rồi review toàn bộ trước khi merge.
