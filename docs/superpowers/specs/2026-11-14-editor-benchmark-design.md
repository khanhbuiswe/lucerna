# So hiệu năng Lucerna, Zed và VS Code — thiết kế

Ngày: 2026-11-14. Trạng thái: đã duyệt.

## Ý định

Đo Lucerna cạnh Zed và VS Code trên cùng một máy, trong môi trường sạch,
rồi **công bố công khai** kết quả (README, site). Vì công bố, mọi con số phải
tái lập được: script, dữ liệu, phiên bản và cấu hình máy đều công khai, và
cách đo công bằng với cả ba editor.

Đồng thời kiểm tra các mục tiêu hiệu năng ở §6 của
`2026-11-08-lucerna-phase1-design.md` trên bản sắp phát hành.

## Mục tiêu

- Một bộ công cụ trong `bench/compare/`, chạy bằng một lệnh, ra file kết
  quả (JSON và bảng Markdown).
- Mỗi editor chạy với hồ sơ trống riêng, không động tới dữ liệu thật của
  người dùng.
- Cùng một cách đo, từ bên ngoài app, cho cả ba editor.
- Một trang phương pháp (`docs/benchmarks.md`) mô tả đủ để người khác chạy lại.

## Không làm

- Không đo tính năng Lucerna không có (AI, debug, extension host của VS Code).
- Không dùng số đo bên trong của Lucerna để so với số đo bên ngoài của editor khác.
- Không tối ưu riêng cho bài đo.

## Chống mục tiêu

- Một con số không tái lập được, hoặc cấu hình bất lợi cho Zed/VS Code mà
  không nói rõ.
- Lời văn chê bai Zed hay VS Code. Theo `CLAUDE.md`: nêu số, trung lập.

## Môi trường sạch

**Hồ sơ riêng cho từng editor**, trong `<bench>/profiles/`:

| Editor | Cách chạy |
|---|---|
| VS Code | `code --user-data-dir <bench>/profiles/vscode --extensions-dir <bench>/profiles/vscode-ext`; settings: tắt telemetry, cập nhật, workspace trust, trang chào, git tự quét giữ mặc định |
| Zed | `zed --user-data-dir <bench>/profiles/zed` (kiểm tra cờ trên bản đang cài); settings: tắt telemetry, cập nhật, tính năng AI |
| Lucerna | `HOME=<bench>/profiles/lucerna-home luc …` |

Settings chung: cùng kích thước cửa sổ, cỡ chữ 14, theme mặc định của từng
editor, tắt minimap nếu có (ghi rõ), không mở lại phiên trước.

**Dữ liệu cố định** trong `<bench>/data.noindex/` (Spotlight bỏ qua đuôi
`.noindex`):
- Repo 80.000 file và `big.rs` ~10 MB: `scripts/gen-bench-repo.sh`.
- Repo cỡ vừa ~5.000 file ở một commit cố định (đề xuất: ripgrep hoặc
  một repo Rust cỡ tương đương, chọn khi lập plan) cho RAM khi rỗi.

**Máy:**
- Cắm sạc, tắt Low Power Mode, tắt app khác, dừng mọi session/cargo.
- Kiểm tra trước khi chạy: không có `cargo`/`rustc` đang chạy, tải CPU thấp,
  `pmset -g therm` không báo giảm xung; nếu không đạt thì script dừng.
- Mỗi lượt nghỉ cho máy nguội; thứ tự editor xoay vòng giữa các lượt.
- Hai chế độ: lạnh (sau `sudo purge`, người dùng chạy) và ấm (bỏ lượt đầu).

**Hai cấu hình:** không language server; và cùng một rust-analyzer (đường
dẫn và phiên bản ghi trong kết quả).

## Phép đo

Tất cả đo từ bên ngoài, giống nhau cho ba editor. 10 lượt mỗi phép; báo
trung vị và p90.

| Chỉ số | Cách đo |
|---|---|
| Khởi động tới khung hình đầu có nội dung (mở `big.rs`) | Quay màn hình ≥ 120 fps (ScreenCaptureKit), mốc 0 là lúc gọi lệnh; khung đầu có chữ của file |
| RAM khi rỗi (repo vừa, 10 tab) | Tổng phys_footprint mọi process của app (`footprint`), sau 60 s rỗi |
| Cuộn `big.rs` | Sự kiện cuộn tổng hợp giống nhau (CGEvent); `xctrace` template Animation Hitches đếm khung rớt |
| Đổi file trên đĩa → màn hình cập nhật | Script ghi file; quay màn hình đo độ trễ |
| Quick Open mỗi phím gõ (repo 80.000 file) | Script gõ phím (CGEvent); quay màn hình đo phím → kết quả |
| CPU khi rỗi | `top`/`ps` trong 60 s |
| Dung lượng app | `du` của bundle |

Mỗi kết quả ghi kèm: phiên bản ba editor, commit Lucerna, macOS, chip, RAM,
ngày đo.

## Kết quả: so sánh ba editor

Đầu ra chính là bảng so sánh, mỗi chỉ số một dòng, ba editor cạnh nhau:

| Chỉ số | Lucerna | Zed | VS Code |
|---|---|---|---|
| Khởi động tới khung đầu có nội dung (trung vị / p90) | … ms | … ms | … ms |
| RAM khi rỗi | … MB | … MB | … MB |
| … | | | |

- Mỗi ô ghi trung vị và p90. Mỗi dòng thêm hệ số so với Lucerna (ví dụ
  "Zed 1,4×"), tính từ trung vị; không tô màu "thắng/thua".
- Mỗi cấu hình (không LSP / có rust-analyzer; lạnh / ấm) một bảng riêng.
- Kèm biểu đồ cột (SVG) mỗi chỉ số, ba cột cho ba editor, dùng trên site.
- Bộ công cụ tự sinh bảng Markdown và SVG từ file JSON kết quả, để số trên
  README/site luôn khớp dữ liệu thô.

## Công bố

- `docs/benchmarks.md`: phương pháp, cấu hình, giới hạn đã biết (Lucerna ít
  tính năng hơn; mỗi editor có kiến trúc khác), bảng so sánh, ngày đo.
- Kết quả thô (JSON) commit trong `bench/compare/results/`.
- README và site trích bảng tóm tắt, dẫn về trang phương pháp.

## Tham khảo (bổ sung khi lập plan)

- VS Code: `--prof-startup`, lệnh Developer: Startup Performance.
- Zed: công cụ đo khởi động và khung hình của Zed.

## Cách làm đã cân nhắc

1. **Đo từ bên ngoài bằng quay màn hình và công cụ hệ thống** (chọn): công
   bằng, áp dụng được cho cả ba.
2. Dùng số đo bên trong của từng editor: mỗi editor đo một thời điểm khác
   nhau, không so được. Bỏ.
3. Chỉ đo RAM và thời gian khởi động bằng `time`: dễ nhưng không đo được
   khung hình đầu có nội dung. Bỏ.

## Người dùng phải tự làm

Cấp quyền Screen Recording và Accessibility cho terminal chạy script; chạy
các lệnh `sudo` (`purge`).

## Để người làm quyết

Ngôn ngữ của bộ công cụ (đề xuất Python chuẩn + một helper Swift nhỏ cho
ScreenCaptureKit/CGEvent nếu cần), repo cỡ vừa cụ thể, cách nhận diện
"khung có chữ".
