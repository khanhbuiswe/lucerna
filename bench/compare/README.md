# bench/compare: so sánh Lucerna, Zed và VS Code từ bên ngoài app

Spec: `docs/superpowers/specs/2026-11-14-editor-benchmark-design.md`. Chỉ dùng stdlib Python (`python3 -I`).
Test: `python3 -m unittest discover -s bench/compare/tests -v` (chạy từ gốc repo).

## Quyền cần cấp (một lần)

System Settings > Privacy & Security, cấp cho ứng dụng terminal đang chạy bench:
- **Screen Recording** (helper quay màn hình bằng ScreenCaptureKit).
- **Accessibility** (helper tạo sự kiện phím/cuộn bằng CGEvent và đặt kích thước cửa sổ bằng AX).

Mỗi lệnh helper chỉ đòi quyền nó dùng:

| Lệnh | Quyền |
|---|---|
| `now`, `check`, `display` | không cần |
| `sample`, `watch` (cả `watch --display`) | Screen Recording |
| `key`, `type`, `frame` | Accessibility |
| `scroll` | Screen Recording và Accessibility |

Từ đó mỗi phép đo cần: `startup_ms` Screen Recording; `idle_ram_mb`, `idle_cpu_pct`
Accessibility; `file_change_ms`, `quick_open_ms`, `scroll_dropped_pct` cả hai; `app_size_mb`
không cần (bảng `METRIC_HELPERS` × `HELPER_PERMISSIONS` trong `metrics.py`, `run.py` suy ra
từ đó). Thiếu quyền thì helper báo lỗi và thoát mã 3, và `run.py` dừng trước khi đo, in tên
quyền thiếu và gợi ý `--only` gồm các phép đo chạy được với quyền đang có (ví dụ chưa có
Screen Recording: `--only idle_ram_mb,idle_cpu_pct,app_size_mb`).

## Helper `bench-helper` (Swift)

Dựng bằng `swiftc` của Command Line Tools: `sh bench/compare/helper/build.sh [out]` (mặc định `<bench>/bin/bench-helper`). Mỗi lệnh in một dòng JSON ra stdout; lỗi ra stderr, mã thoát 1 = sai cú pháp/lỗi, 2 = hết giờ, 3 = thiếu quyền (không in số đo). Đối số được kiểm tra trước quyền, nên sai cú pháp luôn là mã 1. Quyền của từng lệnh: bảng ở trên.

`--pid` nhận danh sách cách nhau bởi dấu phẩy (truyền mọi pid của phiên bench, kể cả process con); helper chọn cửa sổ layer 0 lớn nhất (≥ 100×100 pt) đang hiện thuộc một trong các pid đó. Kích thước quay cố định lúc bắt đầu stream; cửa sổ đổi cỡ sau đó thì nội dung được co giãn vào kích thước ấy.

- `now`: `{"t_ns"}` theo `CLOCK_UPTIME_RAW`, cùng đồng hồ với `editors.now_ns()` (`time.clock_gettime_ns(time.CLOCK_UPTIME_RAW)`; không dùng `time.monotonic_ns()` vì python3 3.9 của macOS đếm nó từ lúc process khởi động).
- `check`: `{"screen_recording", "accessibility"}`, chỉ preflight, không bật hộp thoại xin quyền. `BENCH_HELPER_FAKE_DENY=1` giả thiếu cả hai quyền; `=screen` hoặc `=accessibility` giả thiếu một quyền.
- `sample --pid [--timeout]`: hiệu chuẩn; lấy một khung, in `{"center_dominant": "RRGGBB", "center_dominant_frac", "window_dominant", "width", "height"}` (vùng giữa = vùng `coverage` đo). Python dùng để truyền `--background` riêng cho từng editor.
- `watch --pid --since --until coverage:x|change [--timeout 60] [--baseline png] [--background RRGGBB]`: quay ở tần số màn hình, thời điểm khung lấy từ `SCStreamFrameInfo.displayTime` (mach time → ns). In `{"t_ns", "frames", "first_frame_hit"}`; `first_frame_hit: true` nghĩa là khung đầu tiên đã đạt điều kiện, số đo đáng ngờ. `coverage`: tỉ lệ điểm ảnh vùng 60%×60% giữa khác màu nền; thiếu `--background` thì lấy màu phổ biến nhất của khung đầu và cảnh báo trên stderr. `change`: so với `--baseline`, hoặc khung cuối trước `--since`, hoặc khung đầu tiên; ngưỡng 0,5% bỏ qua con trỏ nhấp nháy. Helper ghi `ready` ra stderr sau khi khung đầu đã được lưu làm mốc; chờ dòng này rồi mới gây thay đổi.
- `watch --display --exe <đường dẫn tuyệt đối> [--exclude-pids a,b] --since 0 --until coverage:x --background RRGGBB [--timeout s]`: dùng cho khởi động. Quay **cả màn hình chính** (`SCContentFilter(display:excludingWindows: [])`), bắt đầu **trước** khi mở editor. Ở mỗi khung hoàn chỉnh, helper tìm bằng `CGWindowListCopyWindowInfo` cửa sổ layer 0 lớn nhất (≥ 100×100 pt) đang hiện mà process sở hữu có `proc_pidpath` bằng `--exe` hoặc nằm trong `Contents/` của cùng bundle `.app` (process con), và không thuộc `--exclude-pids`; cắt khung theo vùng cửa sổ (điểm → điểm ảnh theo tỉ lệ khung/màn hình) rồi đo `coverage` trên vùng 60%×60% giữa của phần cắt. Ghi `ready` ra stderr sau khung hoàn chỉnh đầu tiên. Python chưa biết t0 khi helper bắt đầu, nên truyền `--since 0` (helper chỉ báo `t_ns` tuyệt đối) và tự trừ t0 của lần mở. `first_frame_hit: true` ở chế độ này nghĩa là cửa sổ đã có và đủ nội dung ngay khung đầu, điều không thể xảy ra với cửa sổ mở sau `ready`: vẫn coi là đáng ngờ và bỏ lượt. Chỉ nhận `coverage`, bắt buộc `--background`, không nhận `--pid`/`--baseline`. In thêm `"window": [x, y, w, h]` của cửa sổ lúc đạt.
- `frame --pid --x --y --w --h [--timeout 10]`: chọn cửa sổ như `key`, lấy cửa sổ AX lớn nhất của app sở hữu (`AXUIElementCreateApplication` → `kAXWindowsAttribute`), đặt `kAXPositionAttribute`/`kAXSizeAttribute`, đọc lại; in `{"x", "y", "w", "h", "pid"}`. Chỉ ép **kích thước** (sai ≤ 1 pt): macOS giữ cửa sổ dưới thanh menu, thanh này cao hơn trên màn hình có notch, nên `y` thật có thể lớn hơn `--y`. Không đạt trước hạn → mã 2.
- `display`: `{"width", "height"}` (pt), `pixel_width`, `pixel_height`, `scale`, `refresh_hz` (`NSScreen.maximumFramesPerSecond`), `mode_refresh_hz` của màn hình chính; không cần quyền.
- `scroll --pid --lines [--rate]`: đưa app sở hữu cửa sổ lên trước, rồi kiểm tra bằng `CGWindowListCopyWindowInfo` (thứ tự trước → sau) rằng cửa sổ trên cùng tại tâm cửa sổ thuộc các pid đã cho (bỏ qua cửa sổ trong suốt alpha 0 và lớp hệ thống từ mức thanh menu trở lên); nếu không, mã 1 và không gửi sự kiện nào. Sau đó dời con trỏ chuột vào giữa cửa sổ rồi gửi sự kiện cuộn 1 dòng, mặc định với tốc độ bằng tần số màn hình; in `{"frames", "dropped", "refresh_hz", "rate"}`. Khung "rớt" là khoảng giữa hai khung mới > 1,5 × max(chu kỳ màn hình, chu kỳ sự kiện) (lệch khỏi định nghĩa trong brief: khi `rate` < tần số màn hình, định nghĩa gốc coi mọi khung là rớt).
- `type --pid --text [--interval-ms 100] [--timeout 10]`, `key --pid --combo cmd+p [--timeout 10]`: tìm cửa sổ bằng `CGWindowListCopyWindowInfo` (pid chủ, layer, khung không cần Screen Recording; cùng cách chọn "cửa sổ layer 0 lớn nhất"), đưa app sở hữu cửa sổ lên trước (`NSRunningApplication.activate`) rồi gửi phím bằng `CGEvent.postToPid` tới pid đó; in thời điểm từng phím. `type` chỉ nhận chữ ASCII, số, dấu cách, dấu câu cơ bản và `\n` (Return); ký tự khác → mã 1.

## Phép đo (`metrics.py`)

Mỗi hàm mở editor trong hồ sơ bench, đo, rồi luôn đóng editor; trả `None` khi lượt hỏng (helper lỗi/hết giờ 60 s, không thấy cửa sổ, khung đáng ngờ). Thiếu quyền (helper mã 3) thì ném `PermissionDenied` để dừng cả phiên.

- Hiệu chuẩn nền: `calibrate` mở `data/empty.txt`, `sample` lấy màu giữa cửa sổ; lưu ở `<bench>/calibration.json` theo editor + phiên bản (`load_calibration`).
- Kích thước cửa sổ: mọi phép đo mở editor, trừ khởi động, đặt cửa sổ thành `WINDOW_FRAME` (1280×800 pt, yêu cầu ở 0,25) bằng `frame` ngay khi cửa sổ hiện; `frame` hỏng thì lượt đó `None`. Khởi động dùng kích thước mặc định của từng editor (khung đầu không đổi cỡ được).
- `startup_ms`: tính danh sách pid đang chạy cùng binary (ví dụ VS Code của bạn) làm `--exclude-pids`, chạy `watch --display --exe <binary chính> --since 0 --until coverage:0.3 --background <nền>`, chờ `ready`, **rồi** mới mở `marker.txt`; kết quả = `t_ns` − t0. `first_frame_hit` hoặc `t_ns` < t0 → `None`.
- `file_change_ms`: mở `data/reload.txt` (bản sao `marker.txt` mỗi lượt), đặt cửa sổ, chờ hiện, `watch --until change`, ghi file trống; mốc là ngay trước lệnh ghi.
- `quick_open_ms`: repo lớn, đặt cửa sổ, chờ 30 s, ⌘P, gõ `mod004`, `watch --until change`, gõ `2`; mốc là phím cuối. `mod0042` là thư mục `pkg00/mod0042/` của `gen-bench-repo.sh`.
- `idle_session` → (`idle_ram_mb`, `idle_cpu_pct`): repo vừa, đặt cửa sổ, mở 10 file `.rs` của ripgrep 14.1.1 bằng Quick Open (cùng cách cho ba editor; `luc` chỉ nhận một đường dẫn), rỗi 60 s; RAM = tổng `phys_footprint` (`footprint -f bytes --noCategories -p …`). CPU = thời gian CPU cộng dồn (`ps -o pid=,time=`) của mọi process bench trong 60 s: lấy mẫu mỗi giây, danh sách pid làm mới mỗi lần; process có ở mẫu đầu tính từ giá trị lúc đó, process xuất hiện sau tính từ 0, process thoát giữa chừng tính tới mẫu cuối của nó; % = CPU dùng / thời gian thực × 100 (100% = một lõi). Không dùng `%cpu` của `ps` (trung bình suy giảm). `report.py` ghi CPU với hai chữ số thập phân và không ghi hệ số khi trung vị của Lucerna dưới `CPU_RATIO_FLOOR` (0,1 %), kèm chú thích dưới bảng. Hồ sơ tắt tab xem trước khi mở từ Quick Open để 10 file là 10 tab; bench không kiểm chứng 10 tab đã mở (giới hạn đã biết).
- `scroll_dropped_pct`: `big.rs`, đặt cửa sổ, `scroll --lines 3000 --rate 120`, `dropped / frames × 100`.
- `app_size_mb`: `du -sk` bundle, MiB (1024).

## Dựng môi trường

Mặc định dưới `~/LucernaBench.noindex` (đổi bằng `--bench-dir`):
1. `env.prepare_data(paths)`: sinh repo lớn bằng `scripts/gen-bench-repo.sh` (80.000 file + `large/big.rs` ~10 MB), clone nông ripgrep `14.1.1` vào `data/medium` (cần mạng), ghi `data/marker.txt`.
2. Đầu mỗi cấu hình: `env.reset_profiles(paths)` xoá mọi hồ sơ dưới `<bench>/profiles` (chỉ xoá đường dẫn nằm trong thư mục đó, symlink bị xoá chứ không bị theo), trừ `vscode-ext-lsp` và file pid Lucerna trong `lucerna-home`; rồi `env.write_profiles(paths, lsp=False|True)` ghi settings sạch cho cả ba editor và `profiles/bench-profile.json` (`{"lsp": …}`, `editors.launch` đọc để chọn thư mục extension của VS Code).
3. Extension VS Code: `profiles/vscode-ext` luôn trống, dùng cho cấu hình không LSP cùng `--disable-extensions`; `profiles/vscode-ext-lsp` chứa rust-analyzer, dùng cho `lsp=True`. `lsp=True` cần `rust-analyzer` trên PATH và extension đã cài một lần (cần mạng):
   ```sh
   python3 -I -c 'import sys; sys.path.insert(0, "bench/compare"); import env; env.install_vscode_rust_analyzer(env.BenchPaths(env.DEFAULT_ROOT))'
   ```
   Thiếu `rust-lang.rust-analyzer*` trong `vscode-ext-lsp` thì `run.py --lsp on|both` dừng ngay từ đầu.
4. Trước mỗi lượt đo, `env.preflight()` phải trả danh sách rỗng (không có `cargo`/`rustc`, load 1 phút <= 2.0, `CPU_Speed_Limit` = 100, đang cắm điện AC, Low Power Mode tắt).
5. Với `--cold`, trước mỗi lần khởi động đo (kể cả lượt 0) script dừng chờ; người dùng tự chạy `sudo purge` rồi nhấn Enter (script không chạy sudo). Chỉ phép khởi động là lạnh; báo cáo ghi cấu hình là "lạnh (chỉ khởi động)".
6. Binary Lucerna được xác định **một lần** đầu phiên (`LUCERNA_BIN`, rồi bundle release theo `scripts/target-dir.sh`, rồi `luc` trên PATH) và ghi vào `meta.lucerna_binary` (`path`, `mtime`, `version` = `luc --version`); không gọi `cargo metadata` lúc đang đo. Thiếu hoặc không chạy được → dừng ngay; binary biến mất giữa phiên → dừng trước lần mở Lucerna kế tiếp (lưu kết quả dở).

Lệnh mở mỗi editor (đều chạy thẳng binary chính, binary đó sở hữu cửa sổ):

| Editor | Lệnh |
|---|---|
| VS Code | `…/Visual Studio Code.app/Contents/MacOS/Code --user-data-dir <bench>/profiles/vscode --extensions-dir <bench>/profiles/vscode-ext --disable-extensions --new-window <đích>` (có LSP: `--extensions-dir …/vscode-ext-lsp`, không `--disable-extensions`). Không qua `bin/code`, vì nó thêm một bước node/CLI mà hai editor kia không có. |
| Zed | `…/Zed.app/Contents/MacOS/zed --user-data-dir <bench>/profiles/zed <đích>`. Không qua `cli`, vì `cli` chuyển qua LaunchServices và bỏ `--user-data-dir`. Zed chỉ cho một phiên mỗi kênh: Zed của bạn đang chạy (ở bất kỳ đâu, khớp `Zed.app/Contents/MacOS/zed`) thì bench dừng. |
| Lucerna | `<binary đã xác định> <đích>` với `HOME=<bench>/profiles/lucerna-home`; pid, thời điểm bắt đầu và đường dẫn binary ghi vào `lucerna-home/.bench-pids.json`. |

## Đường dẫn và khoá đã xác minh

| Editor | File settings | Ghi chú |
|---|---|---|
| VS Code | `profiles/vscode/User/settings.json` | Theo `--user-data-dir`. |
| Zed | `profiles/zed/config/settings.json` | Zed 1.28.1: help của `--user-data-dir` nói thư mục chứa dữ liệu; binary có các thư mục con `config` và `data` (`strings`); `config/settings.json` theo cách Zed tính `custom_data_dir`. Zed tự ghi thêm vào `profiles/zed/config` (và `data`) khi chạy; hồ sơ bị xoá đầu mỗi cấu hình nên không mang sang. Khoá `auto_install_extensions` (`{"html": false}`) và `suggest_extensions` (`false`) theo tài liệu Zed (Configuring Zed) và có trong binary. Chưa khởi chạy Zed để kiểm chứng. |
| Lucerna | `profiles/lucerna-home/.config/lucerna/settings.json` | Lucerna lấy `$HOME/.config/lucerna` (`crates/lucerna-workbench/src/config_files.rs:52`), không phải `Library/Application Support`. |
| Lucerna trust | `profiles/lucerna-home/Library/Application Support/Lucerna/trusted.json` | `{"folders": [...], "parents": [...]}` (`trust.rs:40-47`, tên file `trusted.json` ở `trust.rs:35`); thư mục dữ liệu từ `workspace_state.rs:236`. Ghi `<data>` vào cả hai danh sách. |

Khoá Lucerna: `editor.fontSize`, `workbench.startupEditor: "none"` (`startup.rs:14`; "none" hỏi chọn thư mục, Cancel thoát), tắt LSP bằng `lucerna.lsp.enabled: false` (`default-settings.json:62`). Với `lsp=True`: `"lsp": {"rust-analyzer": {"binary": {"path": ...}}}` (dạng của Zed, theo spec LSP phase 3). Lucerna không có khoá "không mở lại phiên trước": hồ sơ `HOME` trống nên không có phiên cũ để mở lại.
