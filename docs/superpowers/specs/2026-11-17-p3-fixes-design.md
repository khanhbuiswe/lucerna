# Sửa các lỗi còn mở sau release 0.1.0, rồi V2b

Ngày 2026-11-17. Trạng thái: đã triển khai, đã merge vào `main` (9a95859).

## 1. Ý định

Trước khi đi tiếp tính năng, dọn sạch các lỗi còn mở từ [review trước release](../reviews/2026-11-15-release-flow-review.md): một lỗi P0 (T1-1), giới hạn còn lại của hot exit và các lỗi P3. Sau đó làm V2b (macro và dòng lệnh `:` của Vim) theo spec có sẵn [2026-11-13-vim-editing-design.md](2026-11-13-vim-editing-design.md), §6.4, §8, §9, §12.

Người dùng chọn thứ tự này: sửa lỗi trước, V2b sau.

## 2. Mục tiêu, không làm, phản mục tiêu

- **Mục tiêu:** mọi lỗi liệt kê ở §5 được sửa và có test. Review được đánh dấu "Đã sửa" từng mục. Bảng roadmap §1.3 được cập nhật.
- **Không làm:** "Reopen with Encoding" / "Save with Encoding" (chọn encoding bằng tay), giữ nguyên byte gốc khi lưu (hướng C đã bị loại), tính năng mới ngoài V2b.
- **Phản mục tiêu:** một bản sửa làm chậm đường mở file hoặc đường lưu (đo lại `docs/perf-checklist.md` nếu đụng `decode_file`); thanh báo hoặc hộp thoại hiện ra với file UTF-8 hợp lệ.

## 3. Ràng buộc

- Hành vi theo VS Code, giao diện theo Zed (`docs/design-principles.md`). Chuỗi hiển thị bằng tiếng Anh.
- Chỉ `lucerna-workbench` dùng gpui. Không phụ thuộc macOS ngoài code nền tảng.
- Ổ đĩa hẹp: một thư mục `target` chung cho mọi worktree (`CLAUDE.local.md`), chạy `scripts/prune-target.sh` trước mỗi lần build hoặc test. Bốn nhánh song song phải chờ nhau ở cargo lock, không tạo target riêng.

## 4. Cách làm (Approaches)

| Cách | Đánh đổi | Kết quả |
|---|---|---|
| **Mỗi nhóm lỗi một nhánh, chạy song song, gộp vào `claude/p3-fixes`; V2b sau khi merge** | Nhanh; các nhóm chia theo vùng code nên không xung đột. Cần một bước gộp. | **Chọn.** Giống đợt `claude/p2-fixes`. |
| Tuần tự trên một nhánh | Đơn giản nhưng chậm, trong khi bốn nhóm gần như độc lập. | Loại. |
| V2b song song với sửa lỗi | T5-4 và dòng lệnh `:` của V2b cùng sửa `quick_open/query.rs`. | Loại. |

Cho T1-1, người dùng đã chọn **hướng A** (cảnh báo rồi cho sửa) trong ba hướng: A cảnh báo, B mở chỉ đọc, C giữ byte gốc. Lý do: lỗi hiếm, A chặn được việc hỏng file trong im lặng với chi phí nhỏ. C cần đổi cấu trúc `lucerna-text`.

## 5. Checklist

Mỗi mục: hướng sửa đã chốt, và test cần có. Vị trí và cách tái hiện xem trong review.

### Nhóm 1: Lưu file (nhánh `claude/p3-save`)

- [x] **T1-1 [P0] Byte sai encoding** (thiết kế ở §6).
- [x] **T1-2 Autosave tạo lại file agent vừa xoá:** với `SaveReason::Auto`, file không còn trên đĩa (NotFound) thì không ghi; tab chờ sự kiện watcher như bình thường. Lưu bằng tay (Cmd+S) vẫn tạo lại file như VS Code. Test: autosave sau khi xoá file không tạo file.
- [x] **T1-3 Stamp lấy sau rename:** lấy metadata từ handle của file tạm trước khi rename. Trường hợp ghi tại chỗ (`write_in_place`) lấy từ handle sau khi ghi. Test: stamp trả về khớp với file tạm.
- [x] **T1-5 Tên dài:** tên file tạm giữ tối đa 255 byte. Nếu `.{name}.luc~XXXXXXXX` vượt quá, cắt `name` theo ranh giới UTF-8 (không cắt giữa ký tự). Test: lưu file tên 250 byte, tên có tiếng Việt.
- [x] **T1-6 Keep Mine không hẹn lại autosave:** khi nhận `ConflictChanged{conflict:false}` mà văn bản vẫn bẩn và `files.autoSave` là `afterDelay`, hẹn lại timer. Test.
- [x] **T1-7 `trimFinalNewlines` với file toàn dòng trống:** đối chiếu `TrimFinalNewLinesParticipant` trong source VS Code rồi làm theo (review dự đoán ra file rỗng). Sửa test `\n` hiện có cho khớp.
- [x] **T2-4 File tạm còn lại khi Quit:** *(quyết định của mình)*
  - Đường `quit()` của Lucerna (Cmd+Q, menu), cả nhánh Don't Save lẫn hot exit: đợi các lần lưu đang chạy xong, tối đa 5 giây; quá hạn thì xoá các file tạm đã biết rồi thoát.
  - Đường platform (Dock Quit, Log Out, Restart) không qua `quit()`, và GPUI chỉ cho hook `on_app_quit` tối đa 200 ms (`SHUTDOWN_TIMEOUT`). Ở đường này chỉ xoá các file tạm đã biết trong hook, không patch GPUI. File gốc vẫn nguyên, trừ file có hardlink: file đó được ghi tại chỗ (`write_in_place`), không có file tạm, nên nếu bị cắt giữa chừng thì có thể bị cụt. Đây là giới hạn có từ trước, đợt này không đổi.
  - Test với lần lưu giả bị chậm.
- [x] **Hot exit (a): file của cửa sổ một file đã bị xoá:** cửa sổ vẫn được mở lại. Tab khôi phục **như bình thường**, đúng hot-exit spec §5.4: văn bản sao lưu, chưa lưu, không hiện thanh báo, lưu thì tạo lại file. Không đổi gì với cửa sổ folder (§5.4 đã chạy như vậy). *(Quyết định của mình, giữ đúng spec hot exit đã duyệt.)*
- [x] **Hot exit (b): khởi động kèm đường dẫn:** cửa sổ **một file** có bản sao lưu luôn được mở lại, kể cả khi khởi động bằng `luc <path>` hay từ Finder; đường dẫn được yêu cầu mở thêm như bình thường. Cửa sổ folder vẫn theo `window.restoreWindows` như §5.4: bản sao lưu chờ tới khi folder được mở lại. VS Code cũng chỉ luôn mở lại cửa sổ rỗng có backup ("Empty windows with backups are always restored"); cửa sổ một file của Lucerna tương đương cửa sổ đó. *(Quyết định của mình.)*
- [x] **T1-1 qua hot exit:** xem §6.2, cờ `lossy` được ghi trong bản sao lưu.
- [x] Cập nhật `docs/features.md`: bỏ đoạn "Known limits" của hot exit.

### Nhóm 2: Explorer và watcher (nhánh `claude/p3-explorer`)

- [x] **T2-5 New File `foo/`:** tên kết thúc bằng `/` thì tạo folder; `a/b/c.rs` tạo cả folder trung gian (VS Code làm cả hai). Test cả hai.
- [x] **T4-4 Watcher không khởi động được:** status bar hiện một mục cảnh báo (icon cảnh báo, tooltip "File watching is off for <tên folder>: <lý do>. Changes made outside Lucerna won't show."), cho cả watcher của worktree và của `.git`. Không dùng hộp thoại.
- [x] **T4-5 File bị ignore có thư mục cha bị xoá:** watch thư mục tổ tiên gần nhất còn tồn tại; khi thư mục cha xuất hiện lại thì chuyển watch xuống. Test: `rm -rf dist`, mở/đóng tab khác, tạo lại `dist/bundle.js` → tab reload.

### Nhóm 3: Quick Open, search, recent (nhánh `claude/p3-quick-open`)

- [x] **T3-3 Recent mất khi ổ bị rút:** lỗi thật nằm ở việc lọc lúc ghi (`record`, `remove` trong `recent.rs`); bỏ lọc ở đó. Làm như VS Code *(quyết định của mình, theo quy tắc hành vi theo VS Code)*: Welcome, menu Open Recent và picker hiện **mọi** mục, không gọi `.existing()` nữa; chọn một mục không còn tồn tại thì mục đó bị gỡ khỏi danh sách và hiện hộp thoại thông báo chỉ có nút OK, dùng đúng câu của VS Code (`windowsMainService.ts`): "Path does not exist" / "The path '<path>' does not exist on this computer." Khi không có cửa sổ nào (menu File) thì gỡ mà không báo, như menubar của VS Code. *(Sửa 2026-11-17 khi viết plan: bản trước ghi nhầm VS Code hỏi Remove / Cancel.)*
- [x] **T5-1 Observer search còn lại:** khi doc đã Ready mà dòng đích không còn thì gỡ observer, đặt con trỏ ở dòng cuối. Test.
- [x] **T5-4 `:-N` và `:N,M`:** parse dòng là `i64`, số âm đếm từ cuối file (`:-1` là dòng cuối); phần dòng và cột tách bằng `:`, `,` hoặc `#`. Đối chiếu `gotoLineQuickAccess` của VS Code. Đây là nghĩa khi **Vim tắt**. Khi Vim bật (V2b), `-1` và `12,5` không phải "số" theo vim spec §8.1 nên được đọc là range Vim (`-1` so với con trỏ, `12,5` là range dòng). Vì vậy hai nghĩa dùng hai hàm parse riêng; V2b không dùng lại hàm này. *(Quyết định của mình.)*
- [x] **T5-5 Regex `$` với CRLF:** matcher bật `multi_line(true)` và `crlf(true)` nhưng ép `line_terminator(Some(b'\n'))`; searcher giữ mặc định LF. *(Sửa khi triển khai: đặt CRLF cho cả searcher như bản trước của spec thì file LF bị bỏ qua.)* Test file CRLF và file LF.

### Nhóm 4: `luc`, socket, multi-window (nhánh `claude/p3-cli`)

- [x] **M1-2 `luc` lúc app đang thoát:** phần EOF đã được sửa cùng M1-1 (`claim_and_send` claim lại khi `SendError::Dropped`). Còn lại trả lời "quitting": app đang tắt vẫn accept nên claim lại ngay sẽ gặp lại nó. Thêm một loại trả lời có cấu trúc `Quitting` vào giao thức (thay cho chuỗi lỗi tự do); client gặp nó thì thử claim lại mỗi 100 ms, tối đa 5 giây, rồi khởi động app mới; quá hạn thì báo lỗi như hiện nay. *(Quyết định của mình.)*
- [x] **M1-3 Mở thất bại sau "will open when ready":** hiện status message cho cả `Source::Cli`, không chỉ `Source::Menu`.
- [x] **M1-4 App chạy trong terminal khi không có socket:** vẫn `spawn_detached` app (không socket) rồi `luc` thoát 0, kèm cảnh báo như hiện tại. Spec multi-window §6.5 sửa theo.
- [x] **M1-5 Tên socket có version:** bỏ version khỏi tên socket, giữ hậu tố `-dev`. App khác version nhưng cùng version giao thức thì nhận request bình thường. Giao thức mới hơn app thì vẫn từ chối như `reply_for_version` hiện nay, nhưng câu báo đổi thành "Lucerna was updated. Quit Lucerna and open it again." Spec multi-window M13/§10 sửa theo. *(Quyết định của mình.)*
- [x] **M1-6 `--trace-startup` khi app đang chạy:** socket đã bị giữ thì báo lỗi rõ ("Lucerna is already running; quit it to measure startup.") và thoát mã khác 0. `--foreground` của người dùng làm giống vậy. Tiến trình con mà `detach()` khởi động dùng một cờ ẩn riêng để hành vi của nó (và bản sửa M1-1) không đổi. *(Quyết định của mình.)*
- [x] **M2-3 Bấm Dock lúc đang khôi phục:** `reopen()` bỏ qua khi đang `restoring`.
- [x] **M3-4 jdtls dùng chung thư mục data:** dùng chung một server khi `(server, root)` trùng giữa các cửa sổ là việc lớn; thay vào đó thêm số "slot" vào khoá băm của `data_dir_for`: slot là số nhỏ nhất chưa có server nào đang chạy dùng cho cùng `(server, root)`. Cửa sổ đầu tiên luôn được slot 0, nên thư mục data được dùng lại qua các lần chạy và không đổi với người chỉ mở một cửa sổ. *(Quyết định của mình.)*

### Sau khi gộp

- [x] Review: đánh dấu "Đã sửa" kèm commit cho từng mục, cập nhật dòng tóm tắt.
- [x] Roadmap §1.3: dòng 5 ghi Git và chuyển nhánh đã xong (đã merge); thêm dòng cho đợt sửa lỗi này.
- [x] Merge `claude/p3-fixes` vào `main`.
- [ ] Viết plan V2b từ spec có sẵn, làm trên nhánh `claude/vim-v2b`.

## 6. Thiết kế T1-1: cảnh báo byte sai encoding

### 6.1 Tham khảo

- **VS Code:** mở được, byte sai thành `U+FFFD`, lưu ghi `EF BF BD`, không báo gì.
- **Zed:** báo lỗi và không mở; lệnh `ForceOpen` mở bất chấp, chấp nhận thay byte hỏng (commit `b629b1f`, `44abaed` trong repo Zed).
- **Lucerna (hướng A):** mở và sửa được như VS Code, nhưng báo như Zed, và hỏi trước lần lưu đầu tiên.

### 6.2 Phát hiện

- `Decoded` thêm trường `lossy: bool`, lấy từ cờ `had_errors` của `encoding_rs` ở cả hai nhánh decode (có BOM, và theo `chardetng`). Nhánh UTF-8 hợp lệ luôn `false`.
- Cờ đi theo document như `encoding` và `has_bom`, được tính lại sau mỗi lần reload từ đĩa.
- **Hot exit:** `BackupHeader` thêm trường `lossy`, nếu không thì văn bản khôi phục chứa `U+FFFD` mà không có cờ và Cmd+S lại ghi đè trong im lặng. Sự đồng ý ở §6.4 không được lưu: sau khi khôi phục, lần lưu đầu tiên hỏi lại. Bản sao lưu cũ không có trường này thì coi là `false`. *(Quyết định của mình.)*

### 6.3 Thanh báo (theo Zed, cùng kiểu `conflict_bar.rs`)

- Khi `lossy` là `true`, hiện một thanh mỏng trên editor, dùng cùng component và màu cảnh báo với thanh conflict:
  > This file contains bytes that aren't valid {encoding}. Saving will replace them with �.
- Nút **Dismiss** ẩn thanh cho tới khi tab được mở lại hoặc file reload còn lỗi.
- Nếu file vừa conflict vừa lossy thì thanh conflict ưu tiên; thanh lossy hiện lại sau khi conflict được giải quyết.

### 6.4 Lần lưu đầu tiên

- **Lưu bằng tay** (Cmd+S, Save All, `:w` sau này, lưu khi đóng tab hay thoát app): lần đầu với tab đó hỏi bằng hộp thoại cảnh báo:
  - Tiêu đề: "Save '{file}' and replace invalid bytes?"
  - Nội dung: "Bytes that aren't valid {encoding} will be written as �. This can't be undone."
  - Nút: **Save** / **Cancel** (Cancel là mặc định). Với GPUI hiện tại trên macOS, nút đầu nhận Return, nên thứ tự là `[Cancel, Save]`; Space vẫn rơi vào Save. Người làm thử tay Return, Escape và Space.
  - Đồng ý một lần thì các lần lưu sau của tab đó không hỏi nữa; sau lần lưu đó file đã sạch nên `lossy` cũng thành `false`.
- **Autosave:** bỏ qua tab lossy chưa được đồng ý; tab giữ chấm chưa lưu. Hot exit vẫn sao lưu văn bản như mọi tab.
- **Save Without Formatting** và Save All dùng cùng luồng.

### 6.5 Test

- `decode_file` trả `lossy` cho: UTF-8 có BOM lẫn `0xE9`; UTF-16 số byte lẻ; Shift_JIS có byte sai. Trả `false` cho UTF-8 hợp lệ và Latin-1.
- Workbench: tab lossy hiện thanh; Cmd+S hỏi, Cancel không ghi; Save ghi và lần sau không hỏi; autosave bỏ qua.
- Đo lại thời gian mở file theo `docs/perf-checklist.md`: không được chậm hơn.

## 7. Để lại cho người làm

- Chia task trong plan, số commit, tên hàm.
- Vị trí cụ thể của mục cảnh báo watcher trên status bar.
- Biến thể T1-1 "lưu thất bại với encoding nhiều byte" (Unencodable ở dòng không sửa) được giải quyết luôn vì `U+FFFD` không encode được thì sao: người làm kiểm tra, và nếu `U+FFFD` không encode được trong encoding đó thì hộp thoại §6.4 vẫn hiện, nhưng lưu sẽ báo lỗi Unencodable như hiện nay.
