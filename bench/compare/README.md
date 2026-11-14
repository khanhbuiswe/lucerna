# bench/compare: so sánh Lucerna, Zed và VS Code từ bên ngoài app

Spec: `docs/superpowers/specs/2026-11-14-editor-benchmark-design.md`. Chỉ dùng stdlib Python (`python3 -I`).
Test: `python3 -m unittest discover -s bench/compare/tests -v` (chạy từ gốc repo).

## Quyền cần cấp (một lần)

System Settings > Privacy & Security, cấp cho ứng dụng terminal đang chạy bench:
- **Screen Recording** (helper quay màn hình bằng ScreenCaptureKit).
- **Accessibility** (helper tạo sự kiện phím/cuộn bằng CGEvent).

Thiếu quyền thì helper báo lỗi và thoát mã khác 0.

## Helper `bench-helper` (Swift)

Dựng bằng `swiftc` của Command Line Tools: `sh bench/compare/helper/build.sh [out]` (mặc định `<bench>/bin/bench-helper`). Mỗi lệnh in một dòng JSON ra stdout; lỗi ra stderr, mã thoát 1 = sai cú pháp/lỗi, 2 = hết giờ, 3 = thiếu quyền (không in số đo). Đối số được kiểm tra trước quyền, nên sai cú pháp luôn là mã 1.

`--pid` nhận danh sách cách nhau bởi dấu phẩy (truyền mọi pid của phiên bench, vì launcher `code`/`cli` không sở hữu cửa sổ); helper chọn cửa sổ layer 0 lớn nhất đang hiện thuộc một trong các pid đó. Kích thước quay cố định lúc bắt đầu stream; cửa sổ đổi cỡ sau đó thì nội dung được co giãn vào kích thước ấy.

- `now`: `{"t_ns"}` theo `CLOCK_UPTIME_RAW`, cùng đồng hồ với `editors.now_ns()` (`time.clock_gettime_ns(time.CLOCK_UPTIME_RAW)`; không dùng `time.monotonic_ns()` vì python3 3.9 của macOS đếm nó từ lúc process khởi động).
- `check`: `{"screen_recording", "accessibility"}`, chỉ preflight, không bật hộp thoại xin quyền. `BENCH_HELPER_FAKE_DENY=1` giả thiếu quyền.
- `sample --pid [--timeout]`: hiệu chuẩn; lấy một khung, in `{"center_dominant": "RRGGBB", "center_dominant_frac", "window_dominant", "width", "height"}` (vùng giữa = vùng `coverage` đo). Python dùng để truyền `--background` riêng cho từng editor.
- `watch --pid --since --until coverage:x|change [--timeout 60] [--baseline png] [--background RRGGBB]`: quay ở tần số màn hình, thời điểm khung lấy từ `SCStreamFrameInfo.displayTime` (mach time → ns). In `{"t_ns", "frames", "first_frame_hit"}`; `first_frame_hit: true` nghĩa là khung đầu tiên đã đạt điều kiện, số đo đáng ngờ. `coverage`: tỉ lệ điểm ảnh vùng 60%×60% giữa khác màu nền; thiếu `--background` thì lấy màu phổ biến nhất của khung đầu và cảnh báo trên stderr. `change`: so với `--baseline`, hoặc khung cuối trước `--since`, hoặc khung đầu tiên; ngưỡng 0,5% bỏ qua con trỏ nhấp nháy. Helper ghi `ready` ra stderr sau khi khung đầu đã được lưu làm mốc; chờ dòng này rồi mới gây thay đổi.
- `scroll --pid --lines [--rate]`: dời con trỏ chuột vào giữa cửa sổ rồi gửi sự kiện cuộn 1 dòng, mặc định với tốc độ bằng tần số màn hình; in `{"frames", "dropped", "refresh_hz", "rate"}`. Khung "rớt" là khoảng giữa hai khung mới > 1,5 × max(chu kỳ màn hình, chu kỳ sự kiện) (lệch khỏi định nghĩa trong brief: khi `rate` < tần số màn hình, định nghĩa gốc coi mọi khung là rớt).
- `type --pid --text [--interval-ms 100]`, `key --pid --combo cmd+p`: đưa app sở hữu cửa sổ lên trước (`NSRunningApplication.activate`) rồi gửi phím bằng `CGEvent.postToPid` tới pid đó; in thời điểm từng phím. `type` chỉ nhận chữ ASCII, số, dấu cách, dấu câu cơ bản và `\n` (Return); ký tự khác → mã 1.

## Phép đo (`metrics.py`)

Mỗi hàm mở editor trong hồ sơ bench, đo, rồi luôn đóng editor; trả `None` khi lượt hỏng (helper lỗi/hết giờ 60 s, không thấy cửa sổ, khung đáng ngờ). Thiếu quyền (helper mã 3) thì ném `PermissionDenied` để dừng cả phiên.

- Hiệu chuẩn nền: `calibrate` mở `data/empty.txt`, `sample` lấy màu giữa cửa sổ; lưu ở `<bench>/calibration.json` theo editor + phiên bản (`load_calibration`).
- `startup_ms`: mở `marker.txt`, `watch --until coverage:0.3 --background <nền>` từ lúc gọi lệnh; `first_frame_hit` → `None`.
- `file_change_ms`: mở `data/reload.txt` (bản sao `marker.txt` mỗi lượt), chờ hiện, `watch --until change`, ghi file trống; mốc là ngay trước lệnh ghi.
- `quick_open_ms`: repo lớn, chờ 30 s, ⌘P, gõ `mod004`, `watch --until change`, gõ `2`; mốc là phím cuối. `mod0042` là thư mục `pkg00/mod0042/` của `gen-bench-repo.sh`.
- `idle_session` → (`idle_ram_mb`, `idle_cpu_pct`): repo vừa, mở 10 file `.rs` của ripgrep 14.1.1 bằng Quick Open (cùng cách cho ba editor; `luc` chỉ nhận một đường dẫn), rỗi 60 s; RAM = tổng `phys_footprint` (`footprint -f bytes --noCategories -p …`), CPU = trung bình 60 mẫu tổng `ps -o %cpu=` (giá trị trung bình suy giảm của `ps`, không tức thời). Hồ sơ tắt tab xem trước khi mở từ Quick Open để 10 file là 10 tab.
- `scroll_dropped_pct`: `big.rs`, `scroll --lines 3000 --rate 120`, `dropped / frames × 100`.
- `app_size_mb`: `du -sk` bundle, MiB (1024).

## Dựng môi trường

Mặc định dưới `~/LucernaBench.noindex` (đổi bằng `--bench-dir`):
1. `env.prepare_data(paths)`: sinh repo lớn bằng `scripts/gen-bench-repo.sh` (80.000 file + `large/big.rs` ~10 MB), clone nông ripgrep `14.1.1` vào `data/medium` (cần mạng), ghi `data/marker.txt`.
2. `env.write_profiles(paths, lsp=False|True)`: ghi settings sạch cho cả ba editor.
3. `lsp=True` cần `rust-analyzer` trên PATH; cài thêm extension cho VS Code một lần (cần mạng) bằng `env.install_vscode_rust_analyzer(paths)`.
4. Trước mỗi lượt đo, `env.preflight()` phải trả danh sách rỗng (không có `cargo`/`rustc`, load 1 phút <= 2.0, `CPU_Speed_Limit` = 100, đang cắm điện AC, Low Power Mode tắt).
5. Trước phiên đo, người dùng tự chạy `sudo purge` (script không chạy sudo).

## Đường dẫn và khoá đã xác minh

| Editor | File settings | Ghi chú |
|---|---|---|
| VS Code | `profiles/vscode/User/settings.json` | Theo `--user-data-dir`. |
| Zed | `profiles/zed/config/settings.json` | Zed 1.28.1: help của `--user-data-dir` nói thư mục chứa dữ liệu; binary có các thư mục con `config` và `data` (`strings`); `config/settings.json` theo cách Zed tính `custom_data_dir`. Chưa khởi chạy Zed để kiểm chứng. |
| Lucerna | `profiles/lucerna-home/.config/lucerna/settings.json` | Lucerna lấy `$HOME/.config/lucerna` (`crates/lucerna-workbench/src/config_files.rs:52`), không phải `Library/Application Support`. |
| Lucerna trust | `profiles/lucerna-home/Library/Application Support/Lucerna/trusted.json` | `{"folders": [...], "parents": [...]}` (`trust.rs:40-47`, tên file `trusted.json` ở `trust.rs:35`); thư mục dữ liệu từ `workspace_state.rs:236`. Ghi `<data>` vào cả hai danh sách. |

Khoá Lucerna: `editor.fontSize`, `workbench.startupEditor: "none"` (`startup.rs:14`; "none" hỏi chọn thư mục, Cancel thoát), tắt LSP bằng `lucerna.lsp.enabled: false` (`default-settings.json:62`). Với `lsp=True`: `"lsp": {"rust-analyzer": {"binary": {"path": ...}}}` (dạng của Zed, theo spec LSP phase 3). Lucerna không có khoá "không mở lại phiên trước": hồ sơ `HOME` trống nên không có phiên cũ để mở lại.
