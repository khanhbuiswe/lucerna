# bench/compare: so sánh Lucerna, Zed và VS Code từ bên ngoài app

Spec: `docs/superpowers/specs/2026-11-14-editor-benchmark-design.md`. Chỉ dùng stdlib Python (`python3 -I`).
Test: `python3 -m unittest discover -s bench/compare/tests -v` (chạy từ gốc repo).

## Quyền cần cấp (một lần)

System Settings > Privacy & Security, cấp cho ứng dụng terminal đang chạy bench:
- **Screen Recording** (helper quay màn hình bằng ScreenCaptureKit).
- **Accessibility** (helper tạo sự kiện phím/cuộn bằng CGEvent).

Thiếu quyền thì helper báo lỗi và thoát mã khác 0.

## Helper `bench-helper` (Swift)

Dựng bằng `swiftc` của Command Line Tools: `sh bench/compare/helper/build.sh [out]` (mặc định `<bench>/bin/bench-helper`). Mỗi lệnh in một dòng JSON ra stdout; lỗi ra stderr, mã thoát 1 = sai cú pháp/lỗi, 2 = hết giờ, 3 = thiếu quyền (không in số đo).

- `now`: `{"t_ns"}` theo `CLOCK_UPTIME_RAW`, cùng đồng hồ với `time.monotonic_ns()`.
- `check`: `{"screen_recording", "accessibility"}`, chỉ preflight, không bật hộp thoại xin quyền. `BENCH_HELPER_FAKE_DENY=1` giả thiếu quyền.
- `watch --pid --since --until coverage:x|change [--timeout 60] [--baseline png] [--background RRGGBB]`: quay cửa sổ lớn nhất của pid ở tần số màn hình, thời điểm khung lấy từ `SCStreamFrameInfo.displayTime` (mach time → ns). Màu nền cho `coverage` là màu phổ biến nhất của khung đầu tiên (hoặc `--background`); vùng đo là 60%×60% giữa cửa sổ. `change` so với `--baseline`, hoặc khung cuối trước `--since`, hoặc khung đầu tiên; ngưỡng 0,5% bỏ qua con trỏ nhấp nháy. Khi đo thay đổi, chạy `watch` trước rồi mới gây thay đổi: helper ghi `ready` ra stderr khi đã quay.
- `scroll --pid --lines --rate`: dời con trỏ chuột vào giữa cửa sổ rồi gửi sự kiện cuộn 1 dòng; in `{"frames", "dropped", "refresh_hz"}`.
- `type --pid --text [--interval-ms 100]`, `key --pid --combo cmd+p`: gửi phím (`CGEvent.postToPid`, app nên ở trước); in thời điểm từng phím.

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
