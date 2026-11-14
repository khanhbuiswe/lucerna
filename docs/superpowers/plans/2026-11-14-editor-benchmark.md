# So hiệu năng Lucerna, Zed, VS Code — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Một bộ công cụ `bench/compare/` đo Lucerna, Zed và VS Code trong môi trường sạch, từ bên ngoài app, rồi sinh bảng so sánh và biểu đồ để công bố.

**Architecture:** Python chuẩn (không thư viện ngoài) điều phối: dựng hồ sơ trống, kiểm tra máy, chạy từng editor, đo, ghi JSON. Một helper Swift nhỏ (`bench-helper`) làm phần cần API hệ thống: quay màn hình bằng ScreenCaptureKit để tìm thời điểm khung hình thay đổi, và tạo sự kiện phím/cuộn bằng CGEvent. `report.py` biến JSON thành bảng Markdown và SVG.

**Tech Stack:** python3 (bản đi kèm macOS) + `unittest`; Swift qua `swiftc` của Command Line Tools; `footprint`, `ps`, `pmset`, `du` của macOS.

**Spec:** `docs/superpowers/specs/2026-11-14-editor-benchmark-design.md`

## Global Constraints

- Không dùng thư viện Python ngoài stdlib; chạy bằng `python3 -I`. Test: `python3 -m unittest discover -s bench/compare/tests -v`.
- Không có Xcode trên máy (không có `xctrace`): mọi phép đo khung hình dùng `bench-helper` (ScreenCaptureKit), không dùng Instruments.
- Không đụng dữ liệu thật: mọi hồ sơ nằm dưới `<bench>` (mặc định `~/LucernaBench.noindex`, đổi bằng `--bench-dir`).
- VS Code chạy bằng `/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code --user-data-dir <bench>/profiles/vscode --extensions-dir <bench>/profiles/vscode-ext`; Zed bằng `/Applications/Zed.app/Contents/MacOS/cli --user-data-dir <bench>/profiles/zed`; Lucerna bằng `luc` với `HOME=<bench>/profiles/lucerna-home`.
- 10 lượt mỗi phép đo, báo trung vị và p90; thứ tự editor xoay vòng mỗi lượt.
- Lời văn công bố trung lập, không tô màu thắng/thua (CLAUDE.md).
- Commit: stage theo đường dẫn, trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`; kiểm tra `.git/MERGE_HEAD` trước khi commit.

## Review Focus

- Editor đã chạy sẵn (bản thật của người dùng đang mở): script phải mở phiên riêng với hồ sơ bench, không gắn vào cửa sổ đang có, và không đóng phiên của người dùng — Task 2 có test cho cách nhận diện process theo `--user-data-dir`/`HOME`.
- Máy không yên giữa chừng (một session khởi động cargo): preflight chạy lại trước mỗi lượt, không chỉ đầu phiên — Task 1 test.
- Hộp thoại lần đầu (VS Code trust, Zed welcome, Lucerna Restricted Mode) che nội dung và làm sai số khởi động: settings tạo sẵn phải tắt chúng; lượt khởi động đầu tiên luôn bị bỏ — Task 1 và Task 5 test.
- Thiếu quyền Screen Recording/Accessibility: helper phải báo lỗi rõ và thoát mã khác 0, không trả số 0 — Task 3 test.
- Một lượt hỏng (timeout): ghi `null` cho lượt đó, thống kê bỏ qua và ghi số lượt hỏng vào báo cáo, không làm hỏng cả phiên — Task 5 và Task 6 test.

---

### Task 1: Môi trường sạch và preflight

**Files:**
- Create: `bench/compare/env.py`, `bench/compare/tests/test_env.py`, `bench/compare/README.md`

**Interfaces:**
- Produces:
  - `BenchPaths(root: Path)` dataclass với `profiles`, `data`, `results`, `big_file`, `large_repo`, `medium_repo`, `marker_file`.
  - `prepare_data(paths: BenchPaths) -> None`: gọi `scripts/gen-bench-repo.sh <data>/large` nếu chưa có; clone repo vừa (ripgrep, tag `14.1.1`) vào `<data>/medium` nếu chưa có; viết `marker.txt` (200 dòng, mỗi dòng 80 ký tự `█`).
  - `write_profiles(paths: BenchPaths, lsp: bool) -> None`: settings cho ba editor (bảng dưới).
  - `preflight() -> list[str]`: danh sách lý do không đạt (rỗng = đạt).

Settings (cùng cỡ chữ 14, không mở lại phiên trước):

| Editor | File | Khoá |
|---|---|---|
| VS Code | `profiles/vscode/User/settings.json` | `telemetry.telemetryLevel: "off"`, `update.mode: "none"`, `security.workspace.trust.enabled: false`, `workbench.startupEditor: "none"`, `window.restoreWindows: "none"`, `editor.fontSize: 14`, `editor.minimap.enabled: false`, `extensions.autoCheckUpdates: false` |
| Zed | `profiles/zed/config/settings.json` (kiểm tra đường dẫn con dưới `--user-data-dir` trên bản đang cài, ghi vào README) | `telemetry: {diagnostics: false, metrics: false}`, `auto_update: false`, `buffer_font_size: 14`, `disable_ai: true`, `restore_on_startup: "none"`, `minimap.show: "never"` |
| Lucerna | `profiles/lucerna-home/Library/Application Support/Lucerna/config/settings.json` + `trust.json` tin `<data>` | `editor.fontSize: 14`, `workbench.startupEditor: "none"` |

Với `lsp=False`: VS Code không cài extension; Zed `languages.Rust.language_servers: []`; Lucerna `lsp.enabled: false` (hoặc khoá tương đương có trong settings, xác minh). Với `lsp=True`: cả ba trỏ tới cùng `rust-analyzer` trên PATH (VS Code cài `rust-lang.rust-analyzer` vào `vscode-ext`, đặt `rust-analyzer.server.path`).

Preflight không đạt khi: có process `cargo`/`rustc`; load trung bình 1 phút > 2.0 (`os.getloadavg`); `pmset -g therm` có `CPU_Speed_Limit` < 100; `pmset -g batt` không phải "AC Power"; Low Power Mode bật (`pmset -g` có `lowpowermode 1`).

- [ ] **Step 1: Test** `test_write_profiles_vscode_disables_trust_and_telemetry` (đọc JSON ghi ra, assert các khoá trên), `test_write_profiles_zed_no_lsp` (`language_servers == []` khi `lsp=False`), `test_preflight_flags_cargo_and_thermal` (patch các hàm đọc hệ thống trả `cargo` đang chạy và `CPU_Speed_Limit = 80` → hai lý do), `test_preflight_ok_when_quiet` (→ `[]`).
- [ ] **Step 2: Chạy, thấy FAIL** — `python3 -m unittest bench/compare/tests/test_env.py -v`.
- [ ] **Step 3: Viết `env.py`.** Các lệnh hệ thống gói trong hàm nhỏ (`_processes()`, `_pmset(args)`) để test patch.
- [ ] **Step 4: Chạy, thấy PASS.**
- [ ] **Step 5: README** ghi: quyền cần cấp (Screen Recording, Accessibility cho terminal), lệnh `sudo purge` người dùng tự chạy, cách dựng môi trường. **Commit** `feat(bench): clean profiles and preflight for the editor comparison`.

### Task 2: Chạy và dọn từng editor

**Files:**
- Create: `bench/compare/editors.py`, `bench/compare/tests/test_editors.py`

**Interfaces:**
- Consumes: `BenchPaths`.
- Produces:
  - `EDITORS = ("lucerna", "zed", "vscode")`.
  - `launch(editor: str, paths: BenchPaths, target: Path) -> Launched` (`Launched(t0_ns: int, popen)`; `t0_ns` = `time.monotonic_ns()` ngay trước `Popen`).
  - `bench_pids(editor: str, paths: BenchPaths) -> list[int]`: mọi process của phiên bench — nhận theo dòng lệnh chứa đường dẫn hồ sơ bench (`--user-data-dir <bench>…` cho VS Code/Zed và mọi helper con của chúng qua cây PPID; với Lucerna: process `luc` có biến môi trường `HOME` = hồ sơ bench, đọc bằng `ps eww`).
  - `quit(editor: str, paths: BenchPaths) -> None`: SIGTERM các `bench_pids`, chờ 5 s, rồi SIGKILL; không đụng process ngoài danh sách.
  - `app_bundle(editor) -> Path` và `version(editor) -> str` (đọc `CFBundleShortVersionString`; Lucerna: `luc --version` + `git rev-parse --short HEAD`).

- [ ] **Step 1: Test** `test_launch_command_vscode_uses_bench_profile` (dòng lệnh chứa `--user-data-dir <bench>/profiles/vscode` và `--extensions-dir`), `test_bench_pids_ignores_users_own_session` (cho `ps` giả có một Zed của người dùng không có `--user-data-dir` và một Zed bench cùng con của nó → chỉ trả PID bench và con), `test_lucerna_env_home` (env của `launch("lucerna")` có `HOME` = hồ sơ bench).
- [ ] **Step 2: FAIL.** **Step 3: Viết `editors.py`** (`_ps()` gói lời gọi `ps -axo pid,ppid,command` và `ps eww -o command= -p`). **Step 4: PASS.**
- [ ] **Step 5: Commit** `feat(bench): launch and clean up each editor in its own profile`.

### Task 3: Helper Swift `bench-helper`

**Files:**
- Create: `bench/compare/helper/main.swift`, `bench/compare/helper/build.sh`, `bench/compare/tests/test_helper.py`

**Interfaces:**
- Produces (CLI, mọi kết quả in một dòng JSON ra stdout; lỗi in ra stderr và thoát mã ≠ 0):
  - `bench-helper check` → `{"screen_recording": bool, "accessibility": bool}`; thoát 3 nếu thiếu quyền nào.
  - `bench-helper watch --pid <pid> --since <t0_ns> --until coverage:<0..1>|change [--timeout <s>] [--baseline <png>]` → `{"t_ns": <monotonic ns của khung đầu đạt điều kiện>, "frames": n}`. Quay cửa sổ lớn nhất của `pid` ở tần số màn hình. `coverage:x`: tỉ lệ điểm ảnh khác màu nền chiếm vùng giữa cửa sổ ≥ x (dùng cho file `marker.txt` toàn `█`). `change`: khung đầu tiên khác khung trước `t0` quá ngưỡng 0,5% điểm ảnh.
  - `bench-helper scroll --pid <pid> --lines <n> --rate <events/s>` và đồng thời đo: `{"frames": n, "dropped": d, "refresh_hz": r}`; khung "rớt" là khoảng giữa hai khung có nội dung mới > 1,5 × chu kỳ màn hình trong lúc sự kiện cuộn còn đang gửi.
  - `bench-helper type --pid <pid> --text <s>` gửi phím bằng CGEvent, in `{"t_ns": [...]}` thời điểm từng phím.
- Đồng hồ: `clock_gettime_nsec_np(CLOCK_UPTIME_RAW)`, khớp `time.monotonic_ns()` của Python trên macOS (xác minh trong test).
- `build.sh` biên dịch `swiftc -O main.swift -o <bench>/bin/bench-helper` (ScreenCaptureKit, CoreGraphics, AppKit).

- [ ] **Step 1: Test** `test_check_reports_permissions` (chạy `check`, JSON có hai khoá bool), `test_missing_permission_exits_nonzero` (bỏ qua với `skipUnless` khi máy có đủ quyền; với biến `BENCH_HELPER_FAKE_DENY=1` helper giả thiếu quyền → mã 3, không in số đo), `test_clock_matches_python` (`bench-helper now` in ns; hiệu với `time.monotonic_ns()` < 5 ms).
- [ ] **Step 2: FAIL.** **Step 3: Viết helper.** **Step 4: PASS** (và chạy tay `watch` trên một cửa sổ TextEdit mở `marker.txt` để thấy `t_ns` hợp lý).
- [ ] **Step 5: Commit** `feat(bench): screen-capture and input helper`.

### Task 4: Các phép đo

**Files:**
- Create: `bench/compare/metrics.py`, `bench/compare/tests/test_metrics.py`, `bench/compare/tests/fixtures/footprint.txt`

**Interfaces:**
- Consumes: `launch`, `bench_pids`, `quit`, helper CLI.
- Produces — mỗi hàm trả `float | None` (None = lượt hỏng/timeout 60 s):
  - `startup_ms(editor, paths)`: mở `marker.txt`, `watch --until coverage:0.3`, trả `(t_ns - t0_ns)/1e6`.
  - `idle_ram_mb(editor, paths)`: mở repo vừa, mở 10 file `.rs` cố định (danh sách trong code), rỗi 60 s, tổng `phys_footprint` của `bench_pids` từ `footprint -p <pid>…`.
  - `idle_cpu_pct(editor, paths)`: sau trạng thái trên, tổng `%cpu` của `bench_pids` lấy mẫu mỗi giây trong 60 s, trả trung bình.
  - `scroll_dropped_pct(editor, paths)`: mở `big.rs`, `scroll --lines 3000 --rate 120`, trả `dropped/frames*100`.
  - `file_change_ms(editor, paths)`: mở `marker.txt`, ghi đè thành file trống, `watch --until change`, trả ms.
  - `quick_open_ms(editor, paths)`: mở repo lớn, chờ quét xong (30 s), mở Quick Open (⌘P), `type --text mod0042`, `watch --until change` sau phím cuối, trả ms từ phím cuối.
  - `app_size_mb(editor)`: `du -sk` bundle.
  - `parse_footprint(text) -> int` (byte).
- [ ] **Step 1: Test** `test_parse_footprint_sums_processes` (fixture ghi lại từ `footprint` thật với hai process → tổng đúng), `test_metric_timeout_returns_none` (helper giả trả mã ≠ 0 → `None`), `test_startup_uses_marker_and_coverage` (patch `launch`/helper, kiểm tra tham số gửi helper).
- [ ] **Step 2: FAIL.** **Step 3: Viết `metrics.py`.** **Step 4: PASS.**
- [ ] **Step 5: Commit** `feat(bench): startup, memory, CPU, scroll, reload and Quick Open measurements`.

### Task 5: Bộ chạy

**Files:**
- Create: `bench/compare/run.py`, `bench/compare/tests/test_run.py`

**Interfaces:**
- Consumes: Task 1–4.
- Produces: `python3 -I bench/compare/run.py [--bench-dir D] [--rounds 10] [--lsp off|on|both] [--cold] [--only metric,…]` ghi `bench/compare/results/<yyyy-mm-dd>-<commit>.json`:
  `{"meta": {"date", "lucerna_commit", "versions": {editor: str}, "macos", "chip", "ram_gb", "rust_analyzer"}, "configs": [{"lsp": bool, "cold": bool, "metrics": {metric: {editor: [float|null, …]}}}]}`.
  - `stats(values) -> {"median", "p90", "failed"}` (bỏ `None`, p90 theo nearest-rank).
  - `order(round: int) -> list[str]`: xoay vòng `EDITORS` theo lượt.
- Quy tắc: preflight trước mỗi lượt (không đạt thì chờ 30 s, thử lại tối đa 10 lần rồi dừng có thông báo); nghỉ 20 s giữa hai lần chạy editor; lượt khởi động đầu tiên của mỗi editor chạy rồi bỏ; `--cold` dừng trước mỗi lượt khởi động, in "chạy `sudo purge` rồi nhấn Enter".
- [ ] **Step 1: Test** `test_order_rotates` (`order(0) != order(1)`, mỗi editor đứng đầu đúng một lần trong ba lượt), `test_stats_ignore_failed` (`[10, None, 30, 20]` → median 20, failed 1), `test_first_startup_discarded` (patch metric, kiểm tra 11 lần gọi cho 10 giá trị ghi), `test_preflight_rechecked_each_round`.
- [ ] **Step 2: FAIL.** **Step 3: Viết `run.py`.** **Step 4: PASS.**
- [ ] **Step 5: Commit** `feat(bench): rounds, rotation, preflight and result file`.

### Task 6: Báo cáo so sánh

**Files:**
- Create: `bench/compare/report.py`, `bench/compare/tests/test_report.py`, `bench/compare/tests/fixtures/result.json`

**Interfaces:**
- Consumes: file JSON của Task 5.
- Produces: `python3 -I bench/compare/report.py <result.json> --out <dir>` ghi `<dir>/results.md` (mỗi cấu hình một bảng: cột Chỉ số | Lucerna | Zed | VS Code; ô `trung vị (p90 …)` cùng đơn vị; dòng hệ số dưới mỗi chỉ số dạng `Zed 1,4× · VS Code 2,1×` tính từ trung vị so với Lucerna; lượt hỏng ghi "n/10 lượt hỏng") và `<dir>/<metric>-<config>.svg` (ba cột cùng một màu trung tính, nhãn số trên cột, không màu thắng/thua).
  - `ratio(other: float, lucerna: float) -> str` (một chữ số thập phân, dấu phẩy thập phân).
- [ ] **Step 1: Test** `test_table_has_three_editor_columns` (dòng tiêu đề đúng `| Chỉ số | Lucerna | Zed | VS Code |`), `test_ratio_format` (`ratio(140, 100) == "1,4×"`), `test_failed_rounds_reported`, `test_svg_has_three_bars` (đếm `<rect` của cột = 3).
- [ ] **Step 2: FAIL.** **Step 3: Viết `report.py`.** **Step 4: PASS.**
- [ ] **Step 5: Commit** `feat(bench): comparison tables and bar charts`.

### Task 7: Tài liệu, tham khảo, chạy thử

**Files:**
- Create: `docs/benchmarks.md`
- Modify: `docs/superpowers/specs/2026-11-14-editor-benchmark-design.md` (mục Tham khảo), `README.md` (một dòng dẫn tới `docs/benchmarks.md`)

- [ ] **Step 1: Tra cứu** cách VS Code (`--prof-startup`, Developer: Startup Performance) và Zed đo hiệu năng; ghi vào mục Tham khảo của spec, nêu vì sao đo bên ngoài.
- [ ] **Step 2: Viết `docs/benchmarks.md`**: mục đích, phương pháp (môi trường, từng phép đo, số lượt, thống kê), giới hạn đã biết (Lucerna ít tính năng; khác kiến trúc; đo ngoài nên gồm cả độ trễ hệ thống như nhau cho ba editor), cách chạy lại, chỗ để bảng kết quả (để trống tới lần đo chính thức).
- [ ] **Step 3: Chạy thử** `run.py --rounds 2 --lsp off` trên bản Lucerna hiện tại (build release bằng `scripts/bundle-app.sh` khi không session nào đang build), rồi `report.py`. Kết quả chạy thử **không commit**; ghi lại các vấn đề gặp vào báo cáo cuối.
- [ ] **Step 4: Commit** `docs: benchmark methodology`.

Lần đo chính thức (10 lượt, cả bốn cấu hình) làm sau khi Git và Terminal đã merge, lúc mọi session dừng; người dùng chạy và commit `results/`.
