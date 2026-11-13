# Sổ ghi terminal tích hợp: hiệu năng và kiểm tra thủ công

Ngày: 2026-11-13. Spec: `2026-11-13-terminal-design.md` (§9.2 ngưỡng, §11 kiểm thử). Plan: `../plans/2026-11-13-terminal.md`, Task 13.

Máy đo: Apple M4, 10 nhân, 16 GB RAM, macOS 26 (Darwin 25.6). Spec đặt ngưỡng trên M1; M4 nhanh hơn M1, nên số dưới đây là cận dưới cho máy chậm hơn.

## 1. Số đo so với ngưỡng §9.2

| Đo | Ngưỡng | Kết quả | Trạng thái |
|---|---|---|---|
| Prepaint + paint một terminal 200 × 60 đầy chữ có màu (đổi kiểu mỗi 3 ô, 4020 run), bản release | ≤ 4 ms | Cả lần vẽ cửa sổ (render, layout, prepaint, paint): p95 **2,35 ms** khi màn không đổi, **2,46 ms** khi mọi dòng đổi mỗi khung (p50 2,22 / 2,25 ms) | Đạt |
| `cat` file 100 MB, phía PTY và parser | terminal xong ≤ 3 s | **0,71 s** (dòng 108 byte, 3000 wakeup) | Đạt |
| `cat` file 100 MB, khung hình editor bên cạnh ≤ 16 ms | ≤ 16 ms | chưa đo, cần app thật | Chờ người dùng kiểm tra |
| Terminal đứng yên | 0 khung hình mỗi giây | 0 khung hình, 0 `notify` trong 10 s giả lập | Đạt |
| Panel ẩn, terminal đang xả output | không vẽ | 0 khung hình qua 500 dòng output; output vẫn được parse | Đạt |
| RAM scrollback | bị chặn bởi `scrollback` (10 000 dòng) | scrollback tối đa 10 000 dòng; RSS đỉnh của tiến trình test 55,5 MB ở 200 cột, không đổi giữa `cat` 100 MB và `yes` 50 MB | Đạt |
| RAM cả app khi mở 1 terminal chạy Claude Code | tăng ≤ 50 MB | chưa đo, cần app thật | Chờ người dùng kiểm tra |
| Binary release `luc` | tăng ≤ 1,5 MB | **+1,09 MiB** (+1 147 760 byte) chưa strip; +0,88 MiB đã `strip -x`; `__TEXT` +0,80 MiB | Đạt |

### 1.1 Cách đo

**Vẽ** (`crates/lucerna-workbench/tests/terminal_perf.rs`, không dùng harness libtest vì text system thật của macOS chỉ tạo được trên main thread):

```
cargo test --release -p lucerna-workbench --test terminal_perf -- --ignored
```

- `TestAppContext::build_with_text_system` với text system thật của nền tảng, nên chữ được shape và glyph được rasterize thật. Font Lilex đi kèm, scale 2× như màn Retina, cửa sổ vừa đủ 200 × 60 ô (ô 8,40 × 17,00 px).
- Màn hình: 60 dòng, mỗi 3 ô đổi một trong 8 kiểu SGR (màu 16, đậm, nghiêng, gạch chân, nền 256 màu). Tổng 4020 run chữ.
- Đo 200 lần `window.draw` của một cửa sổ chỉ chứa `TerminalView`, sau 10 lần khởi động. Hai trường hợp: màn không đổi, và mỗi khung feed một màn mới (mọi dòng khác chữ). Không gồm việc GPU vẽ.
- Đo riêng: `layout_line` cho 60 dòng p95 0,55 ms; `snapshot()` 200 × 60 p95 0,07 ms.

Kết quả lần chạy cuối:

```
cell 8.40×17.00 px at 2×, grid 200×60
4020 text runs on screen
layout_line, 60 rows: p50 0.524 ms, p95 0.548 ms, max 0.572 ms (200 frames)
snapshot, 200×60: p50 0.059 ms, p95 0.065 ms, max 0.151 ms (200 frames)
window draw, unchanged screen: p50 2.218 ms, p95 2.348 ms, max 2.587 ms (200 frames)
window draw, every row new: p50 2.249 ms, p95 2.462 ms, max 3.990 ms (200 frames)
budget 4 ms: unchanged p95 2.348 ms, every row new p95 2.462 ms
```

**Flood** (`crates/lucerna-terminal/tests/flood.rs`, PTY thật, không có UI; luồng test nhận sự kiện và lấy snapshot mỗi 64 wakeup như UI):

```
cargo test --release -p lucerna-terminal --test flood -- --ignored --nocapture --test-threads=1
```

```
cat 100 MB file (108-byte lines): exit Some(0) after 0.71 s, 3000 wakeups, 46 snapshots, scrollback max 10000 lines (cap 10000), peak RSS 55.4 MB (before 2.0 MB)
yes | head -c 50 MB: exit Some(0) after 15.77 s, 2951183 wakeups, 46112 snapshots, scrollback max 10000 lines (cap 10000), peak RSS 55.5 MB (before 55.4 MB)
```

`yes` là trường hợp xấu nhất theo số dòng: 25 triệu dòng 2 byte, mỗi dòng cuộn lưới một lần, khoảng 0,63 µs mỗi dòng trong parser của alacritty. `cat` 100 MB chỉ có ~0,93 triệu dòng. RSS 55 MB là scrollback đầy ở 200 cột (10 000 × 200 ô × ~24 byte ≈ 48 MB); ở 120 cột như ước tính của spec thì khoảng 30 MB.

**Đứng yên và panel ẩn** (`crates/lucerna-workbench/tests/terminal_view.rs`, chạy cùng bộ test thường): `idle_prompt_draws_nothing_for_ten_seconds`, `hidden_busy_panel_draws_nothing`.

**Binary**: `cargo build --release --bin luc` trên nhánh này (đã gồm thay đổi vẽ ở §1.2) và trên điểm tách nhánh `9f7d77c` (một `git worktree` tạm, target riêng, xóa sau khi đo). So với điểm tách chứ không với `main` hiện tại, vì `main` đã có thêm Markdown preview không thuộc terminal.

| | `9f7d77c` | nhánh terminal | Tăng |
|---|---|---|---|
| `luc` release | 48 823 568 | 49 971 328 | +1 147 760 (1,09 MiB) |
| sau `strip -x` | 40 182 904 | 41 104 288 | +921 384 (0,88 MiB) |
| `__TEXT` | 36 536 320 | 37 371 904 | +835 584 |

### 1.2 Vẽ: lần đo đầu vượt ngưỡng, đã sửa

Lần đo đầu (mỗi run một `shape_line` và một `ShapedLine::paint`): màn không đổi p95 5,55 ms, mọi dòng đổi p95 10,15 ms. Tách ra: prepaint 1,5 ms (cache dòng của GPUI trúng) hoặc 4,7–5,2 ms (shape lại), paint 2,8 ms cho 4020 lần paint một dòng.

Đã thử và bỏ:

- **Gộp cả dòng thành một `shape_line` nhiều run:** màn không đổi xuống 2,2 ms nhưng mọi dòng đổi lên 30 ms. GPUI tách font run ở mỗi lần đổi màu, và CoreText chậm hẳn với dòng có nhiều font run.
- **Vẽ từng glyph mà không có layer:** 7,8 ms cho 12 000 glyph, vì mỗi primitive được xếp thứ tự riêng trong cây bounds của scene.

Cách giữ lại (`element.rs`):

- Run chỉ gồm ký tự hẹp thường (rộng 1 ô, không dấu kết hợp) không shape. Glyph của mỗi ký tự theo từng kiểu (thường, đậm, nghiêng) được tra một lần bằng `layout_line` của một ký tự, rồi cache trong `TerminalView` (đổi font thì xóa; quá 8192 ký tự thì làm lại). Ligature đã tắt và lưới cố định, nên kết quả giống shape cả run. Font dự phòng vẫn đúng vì cache giữ `font_id` mà CoreText chọn.
- Paint vẽ các glyph đó bằng `paint_glyph` ở đúng ô, cùng baseline mà một dòng shape bằng font terminal có; gạch chân, gạch ngang vẽ theo run.
- Toàn bộ chữ vẽ trong một `paint_layer`.
- Run có ký tự rộng, dấu kết hợp, emoji hoặc ký tự không ra đúng một glyph vẫn shape như trước (ép độ rộng ô).

Cache shape theo damage của §9.1 không làm: prepaint 200 × 60 bây giờ ~1,4 ms kể cả khi mọi dòng đổi (dưới mốc 2 ms của §9.1), và giá không còn phụ thuộc màn có đổi hay không. Ghi chú còn lại từ Task 8 (prepaint dựng lại cả lưới ở mỗi khung của cửa sổ khi terminal đang hiện) vì vậy chỉ tốn ~1,4 ms mỗi khung có vẽ lại; nếu đo trên M1 thấy vượt 2 ms thì làm cache theo dòng sau.

## 2. Kiểm tra thủ công (chờ người dùng kiểm tra)

Chạy bản release (`scripts/bundle-app.sh`), đặt cạnh Zed khi so giao diện (design-principles §1.1).

### 2.1 Hiệu năng trong app

- [ ] `cat` file 100 MB trong terminal trong khi cuộn editor bên cạnh: khung hình editor ≤ 16 ms (log thời gian khung của GPUI), `cat` xong ≤ 3 s. Chờ người dùng kiểm tra.
- [ ] `yes` 30 s với panel ẩn: RSS của app không tăng quá mức scrollback. Chờ người dùng kiểm tra.
- [ ] Terminal ở prompt 10 s: CPU ~0 trong Activity Monitor. Chờ người dùng kiểm tra.
- [ ] RAM cả app khi một terminal chạy Claude Code tăng ≤ 50 MB so với không có terminal. Chờ người dùng kiểm tra.

### 2.2 Chương trình thật (spec §11)

- [ ] Claude Code: khung viền, màu, cuộn khi đang chạy, ⇧Enter qua binding `sendSequence`, ⌘+click đường dẫn Claude Code in ra mở đúng file và dòng. Chờ người dùng kiểm tra.
- [ ] Codex: vẽ và nhập đúng. Chờ người dùng kiểm tra.
- [ ] vim: mouse mode, alt screen (thoát vim trả lại màn shell). Chờ người dùng kiểm tra.
- [ ] htop: vẽ, màu, phím. Chờ người dùng kiểm tra.
- [ ] Resize panel khi Claude Code đang vẽ: không vỡ khung, không panic. Chờ người dùng kiểm tra.
- [ ] Gõ tiếng Việt bằng Telex trong zsh và trong Claude Code: chữ đang gõ hiện ở con trỏ, dấu đúng. Chờ người dùng kiểm tra.
- [ ] Đổi theme khi terminal đang mở: màu đổi ngay. Chờ người dùng kiểm tra.
- [ ] Chữ có dấu kết hợp, CJK và emoji hiện đúng ô sau thay đổi cách vẽ ở §1.2 (đường shape vẫn dùng cho chúng). Chờ người dùng kiểm tra.

### 2.3 So giao diện với Zed (Task 13 bước 6)

- [ ] Chiều cao thanh tab, nền và viền panel, sash, khoảng cách nút ＋ ⤢ ×. Chờ người dùng kiểm tra.
- [ ] Trạng thái rỗng, trạng thái tiến trình đã thoát (`[Tiến trình kết thúc với mã N]`), tab có chuông hoặc mã khác 0. Chờ người dùng kiểm tra.
- [ ] Con trỏ khung rỗng khi mất focus, lề trái một ô, thanh cuộn. Chờ người dùng kiểm tra.

## 3. Khác

- Bảng màu ANSI mặc định của VS Code (light | dark) đã ghi vào research §1.5, đúng với `TerminalColors::fallback_ansi` trong `lucerna-theme`.
- `cargo deny check`: `advisories ok, bans ok, licenses ok, sources ok`.
