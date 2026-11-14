# So sánh hiệu năng: Lucerna, Zed và VS Code

Trang này mô tả cách đo, để người khác chạy lại được và hiểu các con số có
nghĩa gì và không có nghĩa gì. Thiết kế: [`docs/superpowers/specs/2026-11-14-editor-benchmark-design.md`](superpowers/specs/2026-11-14-editor-benchmark-design.md).
Mã đo: [`bench/compare/`](../bench/compare/) (xem thêm `bench/compare/README.md`).

## Mục đích

Đo ba editor trên cùng một máy, trong môi trường sạch, bằng cùng một cách đo
từ bên ngoài app, rồi công bố bảng so sánh cùng script và dữ liệu thô. Mục
tiêu là con số tái lập được, không phải xếp hạng. Lucerna là editor tối giản,
không có AI và ít tính năng hơn hai editor kia (xem [Giới hạn](#giới-hạn-đã-biết)).

## Vì sao đo từ bên ngoài

Mỗi editor có bộ đo riêng (VS Code: `--prof-startup`, Developer: Startup
Performance; Zed: công cụ profile cho người phát triển; Lucerna: bộ đo trong
`docs/perf-checklist.md`). Mỗi bộ đánh dấu một thời điểm khác nhau nên số
không so được với nhau. Bộ công cụ này chỉ dùng một mốc chung cho cả ba: lúc
cửa sổ **hiện nội dung trên màn hình**, quay bằng ScreenCaptureKit. Số đo
trong của Lucerna không dùng để so với số đo ngoài của editor khác.

## Môi trường

Mọi thứ nằm dưới thư mục bench (mặc định `~/LucernaBench.noindex`, đổi bằng
`--bench-dir`); dữ liệu thật của người dùng không bị đụng. Đuôi `.noindex`
để Spotlight bỏ qua.

**Hồ sơ riêng cho từng editor** (`<bench>/profiles/`). Đầu mỗi cấu hình
(không LSP / có rust-analyzer), bench xoá mọi hồ sơ dưới `<bench>/profiles`
rồi ghi lại settings, nên không cấu hình nào mang theo cache, lịch sử hay
extension của cấu hình trước. Chỉ giữ `vscode-ext-lsp` (extension
rust-analyzer của VS Code, cài cần mạng) và file pid của Lucerna. Việc xoá chỉ
chạm đường dẫn nằm trong `<bench>/profiles`.

Cả ba editor được mở bằng **binary chính** của chúng (binary sở hữu cửa sổ),
không qua launcher:

| Editor | Cách chạy | Settings chính |
|---|---|---|
| VS Code | `/Applications/Visual Studio Code.app/Contents/MacOS/Code --user-data-dir <bench>/profiles/vscode --extensions-dir <bench>/profiles/vscode-ext --disable-extensions --new-window <đích>`; cấu hình có LSP dùng `--extensions-dir <bench>/profiles/vscode-ext-lsp` và không có `--disable-extensions` | tắt telemetry, cập nhật, Workspace Trust, trang chào, mở lại cửa sổ cũ, minimap; `editor.fontSize` 14; tắt tab xem trước khi mở từ Quick Open |
| Zed | `/Applications/Zed.app/Contents/MacOS/zed --user-data-dir <bench>/profiles/zed <đích>` | tắt telemetry, cập nhật, AI (`disable_ai`), khôi phục phiên, minimap, tự cài extension `html` (`auto_install_extensions`) và gợi ý extension (`suggest_extensions`); `buffer_font_size` 14; tắt tab xem trước từ file finder |
| Lucerna | binary `luc` (xác định một lần mỗi phiên) với `HOME=<bench>/profiles/lucerna-home` | `editor.fontSize` 14, `workbench.startupEditor: "none"`, thư mục dữ liệu được đánh dấu tin cậy trước (không hiện Restricted Mode), tắt tab xem trước |

VS Code không chạy qua `bin/code`: lệnh đó khởi một bước node/CLI rồi mới gọi
app, một chi phí mà hai editor kia không có trong số khởi động. Zed không chạy
qua `cli`: `cli` chuyển qua LaunchServices và bỏ `--user-data-dir`. Zed ghi
thêm dữ liệu của nó vào `profiles/zed/config` và `profiles/zed/data` khi chạy;
thư mục được xoá đầu mỗi cấu hình.

Theme mặc định của từng editor. Cấu hình có language server ghi đường dẫn và
phiên bản `rust-analyzer` vào kết quả; cấu hình không LSP tắt LSP ở cả ba
(Lucerna `lucerna.lsp.enabled: false`, Zed bỏ language server của Rust, VS
Code dùng thư mục extension trống `vscode-ext` cùng `--disable-extensions`).
Cấu hình có LSP: VS Code dùng `vscode-ext-lsp`; thiếu extension
`rust-lang.rust-analyzer` ở đó thì bench dừng ngay từ đầu.

**Kích thước cửa sổ.** Mọi phép đo mở editor, trừ khởi động, đặt cửa sổ thành
1280×800 pt (yêu cầu ở toạ độ 0,25) qua Accessibility ngay khi cửa sổ hiện, nên
ba editor được đo ở cùng một kích thước. macOS giữ cửa sổ dưới thanh menu, nên
trên màn hình có notch cửa sổ có thể nằm thấp hơn; kích thước thì được kiểm
tra lại, sai thì lượt đó bị loại. Phép khởi động dùng **kích thước mặc định
của từng editor** (khung hình đầu tiên không đổi cỡ được). Kích thước, tỉ lệ và
tần số quét của màn hình chính được ghi vào kết quả.

**Dữ liệu cố định** trong `<bench>/data/`:
- Repo lớn sinh bởi `scripts/gen-bench-repo.sh`: 80.000 file và `big.rs`
  khoảng 10 MB.
- Repo vừa: ripgrep `14.1.1` (clone nông), dùng cho RAM và CPU khi rỗi.
- `marker.txt`: 200 dòng, mỗi dòng 80 ký tự `█` (khối đặc, để vùng giữa cửa
  sổ đổi màu rõ ràng).

**Kiểm tra máy** (`env.preflight`), chạy **trước mỗi lượt** chứ không chỉ đầu
phiên: không có `cargo`/`rustc` đang chạy, tải CPU 1 phút ≤ 2,0,
`CPU_Speed_Limit` = 100 (`pmset -g therm`), đang cắm điện, Low Power Mode tắt.
Không đạt thì chờ 30 s và thử lại (tối đa 10 lần), rồi dừng và lưu kết quả đã
có. Người chạy tự tắt app khác.

**Quyền cần cấp:** Screen Recording và Accessibility cho terminal chạy bench
(quyền của từng lệnh helper và từng phép đo: `bench/compare/README.md`).
Thiếu quyền thì helper báo lỗi rõ và thoát với mã khác 0, không trả số 0.

## Các phép đo

Mỗi phép mở editor trong hồ sơ bench, đo, rồi đóng editor. Chỉ process mang
đường dẫn hồ sơ bench (và con của chúng), hoặc `luc` ghi trong file pid của
bench, bị đóng; Zed hay VS Code đang chạy của người dùng không bị đụng. Lucerna
nhận diện bằng file pid (pid, thời điểm bắt đầu và đường dẫn chính xác của
binary) vì `ps` trên macOS không hiện biến môi trường của process khác.

Binary Lucerna được xác định **một lần** đầu phiên và ghi vào kết quả
(`meta.lucerna_binary`: đường dẫn, thời điểm sửa file, `luc --version`), cạnh
`lucerna_commit` (HEAD của checkout, có thể khác binary nếu chưa dựng lại).
Không có binary thì bench dừng ngay; binary biến mất giữa phiên (đang dựng lại)
thì bench dừng trước lần mở Lucerna kế tiếp.

Đồng hồ: mọi mốc là nano giây của `CLOCK_UPTIME_RAW`. Thời điểm của một khung
hình lấy từ `SCStreamFrameInfo.displayTime` (mach time) mà ScreenCaptureKit gắn
cho khung, nên không phụ thuộc độ trễ chuyển khung tới process đo. Helper
`bench-helper` (Swift, dựng bằng `swiftc` của Command Line Tools; không dùng
Xcode hay `xctrace`) quay cửa sổ lớn nhất đang hiện của editor ở tần số quét
của màn hình.

**Màu nền.** Với mỗi editor và phiên bản, bench mở một file trống, lấy màu
phổ biến nhất của vùng giữa cửa sổ làm màu nền; bỏ nếu màu phổ biến nhất chiếm
dưới 90% vùng giữa (có thể là hộp thoại hay cửa sổ chưa vẽ xong). Màu lưu trong
`<bench>/calibration.json` và ghi vào kết quả.

| Chỉ số | Cách đo |
|---|---|
| Khởi động tới khung đầu có nội dung | Mở `marker.txt`. Helper quay **cả màn hình chính** và bắt đầu quay **trước** khi mở editor (chờ helper báo sẵn sàng rồi mới gọi lệnh mở), nên khung đầu tiên của cửa sổ mới không bị lỡ. Ở mỗi khung, helper tìm cửa sổ lớn nhất của binary editor (theo đường dẫn binary của process sở hữu; bỏ các process cùng binary đã chạy trước, ví dụ VS Code của bạn), cắt khung theo cửa sổ đó, và dừng ở khung đầu tiên mà ≥ 30% điểm ảnh của vùng giữa (60% × 60% cửa sổ) khác màu nền đã hiệu chuẩn. Mốc 0 là ngay trước khi gọi lệnh mở; helper báo thời điểm tuyệt đối của khung và bench trừ mốc 0. Nếu cửa sổ đã đủ nội dung ngay khung đầu tiên quay được (không thể xảy ra với cửa sổ mở sau đó), lượt đó bị loại (ghi `null`). |
| Đổi file trên đĩa → màn hình cập nhật | Mở bản sao của `marker.txt`, chờ nội dung hiện và 1 s cho yên, bắt đầu quay, rồi script ghi file trống. Mốc là ngay trước lệnh ghi; dừng ở khung đầu tiên đổi hơn 0,5% so với khung tham chiếu (ngưỡng bỏ qua con trỏ nhấp nháy). |
| Quick Open mỗi phím gõ (repo 80.000 file) | Mở repo lớn, chờ 30 s cho quét file, ⌘P, gõ `mod004`, rồi bắt đầu quay và gõ phím cuối `2` (`mod0042` là thư mục có thật trong repo sinh ra). Mốc là lúc gửi phím cuối; dừng ở khung đầu tiên đổi. Phím gửi bằng `CGEvent.postToPid` tới đúng process, sau khi đưa app lên trước. |
| RAM khi rỗi | Mở repo vừa, mở 10 file `.rs` của ripgrep bằng Quick Open (cùng cách cho cả ba, vì `luc` chỉ nhận một đường dẫn trên dòng lệnh), rỗi 60 s, rồi cộng `phys_footprint` của mọi process của app (`footprint`). Đơn vị MiB (1024). |
| CPU khi rỗi | Cùng phiên với RAM (mỗi editor mỗi lượt một phiên: rỗi 60 s, đọc RAM, rồi đo CPU thêm 60 s, tổng khoảng 2 phút): thời gian CPU cộng dồn (`ps -o time=`) của mọi process của app, đọc mỗi giây với danh sách process làm mới mỗi lần; CPU dùng trong 60 s chia cho thời gian thực, ×100 (100% = một lõi). Process mới xuất hiện tính từ 0, process thoát giữa chừng tính tới lần đọc cuối của nó. Không dùng `%cpu` của `ps` (trung bình suy giảm, không phải tức thời). Báo cáo ghi hai chữ số thập phân; khi trung vị của Lucerna dưới 0,1% thì không ghi hệ số (tỉ số của hai giá trị nhỏ như vậy chủ yếu là nhiễu). |
| Khung rớt khi cuộn `big.rs` | Mở `big.rs`, chờ nội dung hiện, đưa editor lên trước và kiểm tra cửa sổ trên cùng ở tâm cửa sổ là của editor (nếu bị cửa sổ khác che, lượt bị loại mà không gửi sự kiện nào), rồi gửi 3000 sự kiện cuộn một dòng với tốc độ 120 sự kiện/giây (CGEvent), con trỏ đặt giữa cửa sổ. Khung rớt là khoảng cách giữa hai khung mới > 1,5 × max(chu kỳ quét màn hình, chu kỳ sự kiện) trong lúc còn gửi sự kiện. Báo tỉ lệ rớt / số khung (%). Vùng giữa của file code phủ ít điểm ảnh hơn nên ngưỡng "đã có nội dung" ở phép này là 2%, không phải 30%. |
| Dung lượng app | `du -sk` của bundle `.app`, MiB. Một giá trị mỗi cấu hình, không lặp lượt. |

## Lượt đo và thống kê

- **10 lượt** mỗi phép đo (`--rounds`, mặc định 10 theo spec). Thứ tự ba
  editor xoay vòng mỗi lượt để không editor nào luôn chạy đầu hay cuối. Với 10
  lượt, Lucerna chạy đầu 4 lần, Zed và VS Code mỗi editor 3 lần; nên chọn số
  lượt là bội của 3 (ví dụ 9 hay 12) để mỗi editor chạy đầu số lần bằng nhau.
- **Lượt 0 bị bỏ:** đầu mỗi cấu hình, bench chạy một lượt đầy đủ mọi phép đo
  đã chọn (kể cả khởi động) cho mỗi editor rồi bỏ kết quả, để lượt đo đầu tiên
  không phải lần mở đầu tiên của bất kỳ thứ gì (cache của hệ điều hành và của
  app). Mỗi phép đo mở editor vì vậy chạy `rounds + 1` lần mỗi editor.
- **Nghỉ 20 s** giữa hai lần mở editor để máy nguội. Preflight chạy trước mỗi
  lượt.
- Báo **trung vị** và **p90** (nearest rank: giá trị thứ ⌈0,9·n⌉ sau khi sắp
  xếp).
- Một lượt hỏng (helper lỗi, hết 60 s, không thấy cửa sổ, khung đáng ngờ)
  ghi `null`; thống kê bỏ qua và báo cáo ghi "n/N lượt hỏng". Một lượt hỏng
  không làm hỏng cả phiên; kết quả được ghi lại sau mỗi lượt. Lượt đổi file
  hoặc Quick Open bị loại (`null`) nếu khung đổi trước lúc gây sự kiện.
- **Hệ số** so với Lucerna = trung vị của editor kia / trung vị của Lucerna,
  tính cho từng cột editor, làm tròn một chữ số thập phân. Hệ số > 1 chỉ có
  nghĩa là số của editor kia lớn hơn số của Lucerna; số lớn hơn không tự nó là
  tốt hay xấu, tuỳ chỉ số. Không tô màu thắng/thua.
- Mỗi kết quả kèm: phiên bản ba editor, commit Lucerna, binary Lucerna (đường
  dẫn, thời điểm dựng, `--version`), đường dẫn và thời điểm dựng của helper,
  macOS, chip, RAM, màn hình chính (kích thước, tỉ lệ, tần số quét), khung cửa
  sổ đã đặt, ngày đo, `rust-analyzer` (nếu có), màu nền hiệu chuẩn.
- Hai chế độ: **ấm** (mặc định) và **lạnh** (`--cold`: trước mỗi lần khởi động,
  kể cả lượt 0, bench dừng chờ bạn chạy `sudo purge`). Chỉ phép khởi động là
  lạnh; các phép khác trong cùng cấu hình chạy như ở chế độ ấm. Báo cáo ghi cấu
  hình này là "lạnh (chỉ khởi động)".

## Giới hạn đã biết

- **Lucerna ít tính năng hơn.** Không AI, không debugger, không extension host
  kiểu VS Code. Cùng một phép đo chạy trên ba chương trình có phạm vi khác nhau;
  bench không đo tính năng Lucerna không có.
- **Kiến trúc khác nhau**: VS Code chạy trên Electron (nhiều process), Zed và
  Lucerna là app native vẽ bằng GPU. Số RAM cộng mọi process của app; cách tính
  này cộng tất cả process, nhưng không cho biết phần nào thuộc process nào,
  cũng không phản ánh bộ nhớ dùng chung giữa các process.
- **Đo ngoài gồm độ trễ hệ thống**: compositor, hiển thị và quay màn hình cộng
  vào số đo như nhau cho cả ba editor, nên các số không phải thời gian bên
  trong từng app và không so được với số đo trong của chúng. Độ trễ quay của
  ScreenCaptureKit làm số khởi động và độ trễ phản hồi lớn hơn một chút so với
  thực; độ phân giải thời gian bị giới hạn bởi tần số quét màn hình.
- **Quick Open và mở file khác nhau giữa các editor**: mỗi editor đánh chỉ mục
  và xếp hạng kết quả theo cách riêng, có thể quét nền sau khi mở thư mục. Bench
  chờ 30 s trước khi gõ nhưng không kiểm chứng quét đã xong. Mở file đi qua
  Quick Open ở cả ba để cùng đường đi; tab xem trước bị tắt ở cả ba để mười
  file thành mười tab. Lucerna không có khoá riêng cho Quick Open nên dùng
  `workbench.editor.enablePreview: false`.
- **Lựa chọn settings ảnh hưởng số đo**: tắt minimap, tắt tính năng AI của Zed,
  tắt telemetry, cỡ chữ 14 và hồ sơ trống (không extension) làm số khác một
  máy dùng hằng ngày. Cấu hình có rust-analyzer đo thêm phần language server;
  cả ba editor dùng chung một binary `rust-analyzer` (VS Code qua extension
  `rust-lang.rust-analyzer` cài một lần, trỏ tới cùng binary bằng
  `rust-analyzer.server.path`).
- **Mười file của phép RAM/CPU khi rỗi không được kiểm chứng là đã mở.** Bench
  mở chúng bằng Quick Open (gõ đường dẫn, Enter) và chờ, nhưng không kiểm tra
  mỗi editor thật sự có mười tab; một lần mở hỏng làm số RAM thấp hơn mà không
  bị loại.
- **Khởi động ở kích thước cửa sổ mặc định** của từng editor, các phép khác ở
  1280×800 pt. Kích thước mặc định khác nhau giữa các editor, nên số khởi động
  gồm cả khác biệt về diện tích cần vẽ.
- **Phép đo khởi động phụ thuộc vào `marker.txt`** (vùng giữa phủ ≥ 30%) và
  màu nền hiệu chuẩn; theme khác có thể cho số khác. Một lượt có nội dung đã
  hiện từ khung đầu tiên bị loại thay vì ghi số có thể sai.
- **Một máy, một lần chạy.** Kết quả chỉ nói về máy và phiên bản ghi kèm.
- Thiết lập Zed (`profiles/zed/config/settings.json`) dựa trên đọc help và binary
  của Zed; nếu bản Zed khác dùng đường dẫn khác, cần chỉnh `env.write_profiles`.

## Chạy lại

Từ gốc repo. Cần: macOS, Command Line Tools (`swiftc`), `python3`, VS Code và
Zed trong `/Applications`, một bản `luc` (build release bằng
`scripts/bundle-app.sh`, hoặc đặt `LUCERNA_BIN`), và mạng cho lần đầu (clone
ripgrep).

1. Cắm sạc, tắt Low Power Mode, đóng app khác, dừng mọi `cargo`/`rustc`.
   **Đóng Zed của bạn trước** và không mở Zed trong lúc chạy: Zed chỉ cho một
   phiên mỗi kênh phát hành, nên khi Zed của bạn đang chạy (ở bất kỳ đâu) bench
   dừng ngay (hoặc giữa chừng, trước lần mở Zed kế tiếp) và lưu kết quả dở.
   VS Code của bạn có thể để mở: bench chạy phiên riêng và bỏ qua cửa sổ của
   nó. Không dựng lại Lucerna trong lúc chạy.
2. Cấp **Screen Recording** và **Accessibility** cho ứng dụng terminal trong
   System Settings > Privacy & Security.
3. Dựng helper:
   ```sh
   sh bench/compare/helper/build.sh        # ra <bench>/bin/bench-helper
   ```
   Nếu dùng `--bench-dir D`, dựng bằng `BENCH_DIR=D sh bench/compare/helper/build.sh`
   (hoặc truyền đường dẫn ra làm đối số), hoặc đặt `BENCH_HELPER` trỏ tới helper.
4. (Chỉ cho cấu hình có LSP) cần `rust-analyzer` trên PATH; cài extension cho
   VS Code một lần vào `<bench>/profiles/vscode-ext-lsp` bằng
   `env.install_vscode_rust_analyzer` (xem `bench/compare/README.md`). Thiếu
   extension thì `--lsp on`/`both` dừng ngay từ đầu.
5. Chạy. Mỗi cấu hình mất khá lâu (mỗi lượt gồm nhiều lần khởi động và, cho mỗi editor, một
   phiên rỗi khoảng 2 phút: 60 s rỗi rồi 60 s lấy mẫu CPU):
   ```sh
   python3 -I bench/compare/run.py --rounds 10 --lsp off     # ấm, không LSP
   python3 -I bench/compare/run.py --rounds 10 --lsp on      # ấm, rust-analyzer
   python3 -I bench/compare/run.py --rounds 10 --lsp off --cold
   ```
   Với `--cold`, script dừng và nhắc "chạy `sudo purge` rồi nhấn Enter" trước
   **mỗi** lần khởi động đo (kể cả lượt 0); bạn chạy `sudo purge` ở terminal
   khác rồi nhấn Enter.
   Thiếu Screen Recording thì bench dừng trước khi đo và gợi ý `--only` với các
   phép chạy được (ví dụ `--only idle_ram_mb,idle_cpu_pct,app_size_mb`).
   Tuỳ chọn: `--lsp both`, `--only startup_ms,scroll_dropped_pct`,
   `--bench-dir D`, `--out FILE`, `--skip-prepare`. Mặc định ghi
   `bench/compare/results/<ngày>-<commit>.json`.
6. Sinh bảng và biểu đồ:
   ```sh
   python3 -I bench/compare/report.py bench/compare/results/<file>.json --out <dir>
   ```
   ra `results.md` và một SVG cho mỗi chỉ số và cấu hình.

Test của bộ công cụ: `python3 -m unittest discover -s bench/compare/tests -v`.

## Kết quả

Chưa có — chờ lần đo chính thức.
