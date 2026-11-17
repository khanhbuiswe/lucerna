# Nguyên tắc thiết kế Lucerna

Các nguyên tắc dưới đây áp dụng cho mọi tính năng và mọi giai đoạn. Chúng đứng trên từng spec riêng lẻ.

## 1. Luôn tham khảo VS Code, Zed và Helix trước khi lên plan

Lucerna muốn là một bản VS Code gọn hơn với giao diện tối giản. Nó nhắm tới người để AI agent viết code, dù agent chạy dưới dạng CLI hay app desktop. Lucerna chạy trên macOS trước, rồi tới Linux và Windows. Vì vậy, **trước khi brainstorm, viết spec hay lập plan cho bất kỳ tính năng nào**, phải xem cả ba đã làm tính năng đó ra sao.

1. **VS Code.** Đọc tài liệu người dùng, tài liệu API hoặc contribution points, và mã nguồn ở `microsoft/vscode`. Nếu tính năng nằm trong một extension phổ biến (ví dụ VSCodeVim), đọc luôn extension đó. Cần nắm được:
   - người dùng thấy tính năng thế nào;
   - tên khóa settings, ID lệnh và phím tắt;
   - các trường hợp biên mà VS Code đã gặp phải.
2. **Zed.** Đọc tài liệu và mã nguồn ở `zed-industries/zed`, ưu tiên đúng revision GPUI mà Lucerna đang pin. Cần nắm được:
   - kiến trúc;
   - cách làm cho nhanh;
   - phong cách giao diện.
3. **Helix.** Đọc tài liệu và mã nguồn ở `helix-editor/helix`. Helix là editor Rust gọn, cấu hình bằng dữ liệu, nên đặc biệt đáng xem cho LSP, tree-sitter, cấu hình ngôn ngữ và modal editing. Cần nắm được:
   - thuật toán và cách xử lý trường hợp biên;
   - cách mô tả tính năng bằng dữ liệu cấu hình thay vì code.
4. **Ghi lại kết quả** vào `docs/research/YYYY-MM-DD-<chủ-đề>.md`, hoặc vào mục "Tham khảo" trong spec của tính năng đó. Phần ghi lại gồm:
   - mỗi bên làm thế nào;
   - điểm giống nhau và khác nhau giữa các bên;
   - Lucerna chọn theo bên nào, và vì sao.
5. **Thứ tự ưu tiên khi các bên khác nhau:**
   - Hành vi, settings, lệnh và phím tắt: theo **VS Code**, để người dùng VS Code chuyển sang không phải học lại.
   - Giao diện và hiệu năng: theo **Zed**.
   - Helix không thay đổi hai luật trên. Nó là góc nhìn thứ ba khi VS Code và Zed khác nhau ở phần bên trong (thuật toán, cấu trúc dữ liệu, định dạng cấu hình), hoặc khi một trong hai không có tính năng đó.
   - Nếu lệch khỏi cả VS Code và Zed thì phải ghi rõ lý do trong spec.

### 1.1 Giao diện theo Zed ngay từ thiết kế đầu tiên

Bài học từ Search: Lucerna làm Search panel ở sidebar theo kiểu VS Code, rồi phải làm lại thành tab và thanh tìm kiếm theo Zed. Để không lặp lại:

- **Bố cục và component lấy theo Zed ngay từ spec đầu tiên.** Tính năng gì cũng bắt đầu bằng câu hỏi "màn này Zed trông thế nào": nó là tab, panel, toolbar hay modal; nó dùng những component nào. Không làm bản giao diện kiểu VS Code trước rồi chỉnh sau. Chỉ lệnh, phím, settings và hành vi là theo VS Code.
- **Dùng chung component giữa các tính năng**, như Zed làm. Mọi ô nhập tìm kiếm (thanh tìm trong file, tab Search, sau này là picker hay filter) đều dùng cùng một khung ô query: viền 1 px, bo góc, các toggle `Aa` `ab` `.*` nằm trong ô. Chúng cũng dùng cùng chiều cao thanh, cùng cụm nút `‹ ›` và bộ đếm, cùng màu nền. Thêm tính năng mới thì tìm component có sẵn trong `ui.rs` trước; chưa có thì tạo ở đó để các tính năng sau dùng lại.
- **Spec phải có ảnh hoặc mô tả màn tương ứng của Zed** (đọc mã nguồn ở revision đang pin, hoặc chụp màn hình Zed). Spec cũng ghi rõ chỗ nào Lucerna khác Zed và vì sao.
- **Review giao diện so với Zed** trước khi coi tính năng là xong: chạy app, đặt cạnh Zed, kiểm tra khoảng cách, chiều cao, màu nền, viền, hover và trạng thái trống.

## 2. Giấy phép khi tham khảo

| Nguồn | Giấy phép | Được làm gì |
|---|---|---|
| Zed: phần lớn các crate (`editor`, `vim`, `extension_host`, `language`, `theme`…) | GPL-3 / AGPL | **Chỉ học thiết kế, không chép code.** Định dạng dữ liệu (theme JSON, `extension.toml`, query `.scm`, keymap) thì dùng được. |
| Zed: `gpui*`, `sum_tree`, `zed_extension_api`, file WIT | Apache-2.0 | Được dùng làm dependency hoặc tham khảo. |
| Helix (`helix-editor/helix`) | MPL-2.0 | **Chỉ học thiết kế, không chép code** (chép vào sẽ buộc file đó theo MPL). Dữ liệu như `languages.toml` và query `.scm` cũng mang MPL, nên chỉ đối chiếu, không chép nguyên văn. |
| VS Code (`microsoft/vscode`) và các extension MIT | MIT | Được tham khảo hoặc dùng lại, nhưng **phải ghi nguồn** trong `THIRD_PARTY_NOTICES.md` hoặc trong comment. |
| Font, icon (Lilex, IBM Plex, Lucide…) | OFL / ISC / MIT | Lấy từ repo gốc, kèm file giấy phép nguyên văn. |

Mã nguồn Lucerna dùng giấy phép MIT OR Apache-2.0. `cargo-deny` sẽ chặn dependency có giấy phép GPL, AGPL hoặc LGPL.

## 3. Các nguyên tắc khác đã chốt

- Lucerna hướng tới người viết code bằng agent CLI (Claude Code, Codex). Việc **đọc code** là ưu tiên số một. Không có tính năng AI.
- Luồng UI không bao giờ được chờ I/O hay phân tích cú pháp. Việc nặng chạy ở luồng nền trên snapshot bất biến.
- Chỉ crate `lucerna-workbench` được phụ thuộc vào GPUI.
- Settings và keybindings theo ngữ nghĩa của VS Code: gộp object qua các lớp, override theo ngôn ngữ, và thứ tự ưu tiên giống VS Code.
- Mỗi tính năng có test, review từng task, rồi review toàn bộ trước khi merge.
- **Trong app chỉ dùng tiếng Anh.** Mọi chữ người dùng thấy (menu, lệnh, nhãn, thông báo, hộp thoại, dòng in ra terminal, tooltip, lỗi) đều bằng tiếng Anh, theo cách VS Code đặt tên. Không có chuỗi tiếng Việt trong code ngoài test (dữ liệu kiểm tra gõ tiếng Việt, IME, Unicode thì được). Spec và tài liệu nội bộ viết tiếng Việt vẫn được, nhưng chữ trong spec mô tả giao diện phải ghi đúng bản tiếng Anh sẽ hiện trong app.
