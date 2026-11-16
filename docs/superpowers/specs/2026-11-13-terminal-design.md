# Terminal tích hợp — thiết kế

Ngày 2026-11-13. Thuộc giai đoạn 5 của lộ trình (§1.3 trong `2026-11-08-lucerna-phase1-design.md`). Nghiên cứu nền: `docs/research/2026-11-13-terminal.md`.

## 1. Mục đích

Người dùng Lucerna để agent CLI (Claude Code, Codex) viết code. Hiện họ phải chạy agent trong một app terminal riêng, rồi chuyển qua lại giữa hai cửa sổ để vừa xem agent làm vừa đọc file nó sửa. Terminal tích hợp đưa agent vào cùng cửa sổ: agent chạy cả buổi ở một panel cạnh code, và bấm vào đường dẫn agent in ra thì mở ngay file đó trong editor.

**Tiêu chí thành công:** tác giả chạy Claude Code cả ngày trong panel terminal của Lucerna thay vì mở app terminal riêng. Agent vẽ đúng, gõ không trễ, và đọc code bên cạnh trong lúc agent đang in ra liên tục mà không thấy giật.

### 1.1 Mục tiêu (MVP)

- Panel terminal mặc định ở dưới, chuyển được sang phải, bật tắt bằng ⌃`.
- Mở terminal ở thư mục gốc của workspace, chạy shell lấy từ `$SHELL`.
- Nhiều terminal, mỗi terminal là một tab trong panel.
- Màu lấy theo theme.
- Chọn chữ, copy, paste, cuộn lại phần đã trôi qua.
- Thay đổi số dòng và cột khi panel đổi kích thước.
- ⌘+click vào URL hay đường dẫn file (kèm `:dòng:cột`) thì mở trong trình duyệt hoặc trong editor.
- Chạy mượt các TUI nặng như Claude Code, Codex, vim, htop.
- Gõ được tiếng Việt bằng bộ gõ (IME).

### 1.2 Ngoài MVP

- Shell integration (OSC 633, đánh dấu lệnh, cuộn theo lệnh).
- Chia ô terminal (split).
- Terminal làm tab ở vùng editor, kéo terminal giữa panel và vùng editor.
- Tìm trong terminal (⌘F).
- Giữ terminal qua lần mở lại app (persistent sessions).
- Profile picker, đổi icon hay màu tab, đổi tên tab.
- Panel bên trái, panel nổi.
- Problems, Output và các view khác trong panel. Panel được dựng sao cho sau này gắn thêm được (§4.1), nhưng MVP chỉ có Terminal.

### 1.3 Không được làm (anti-goals)

- Không làm chậm editor. Một agent in liên tục ở terminal không được làm khung hình của editor vượt 16 ms.
- Không tốn CPU khi đứng yên. Terminal không có dữ liệu mới thì không vẽ lại. Con trỏ mặc định không nhấp nháy.
- Không có tính năng AI. Terminal chỉ là terminal.
- Không chép code từ crate `terminal` hay `terminal_view` của Zed (GPL).

## 2. Ràng buộc

- Chỉ `lucerna-workbench` được dùng gpui. Phần lõi terminal nằm ở crate mới không dùng gpui.
- Giấy phép MIT OR Apache-2.0; `cargo deny check` phải qua. `alacritty_terminal` 0.26.0 đã được kiểm (research §4).
- macOS trước. Linux và Windows phải chạy được mà không phải viết lại: không giả định macOS ngoài các chỗ ghi rõ là theo nền tảng.
- Luồng UI không bao giờ chờ I/O. Đọc PTY và parse chạy ở luồng riêng.
- Hành vi, lệnh, settings, phím theo VS Code; giao diện và hiệu năng theo Zed. Chỗ lệch ghi rõ lý do.

## 3. Các hướng đã cân nhắc

| Hướng | Được | Mất |
|---|---|---|
| **1. `alacritty_terminal` + phần vẽ GPUI tự viết** (đã chọn) | Bộ giả lập Zed đã dùng thật với Claude Code và vim; có ConPTY cho Windows; Apache-2.0; binary tăng ~290 KB; 7 crate mới | Tự viết phần vẽ lưới ô và nhập liệu |
| 2. `wezterm-term` + `portable-pty` | Bộ giả lập tốt, MIT | Không có trên crates.io, phải thêm nguồn git vào `deny.toml`; cây dependency nặng; chưa có editor GPUI nào dùng để học |
| 3. `vt100` + `portable-pty` | Nhẹ | Thiếu reflow khi resize, chế độ chuột và alt screen chưa chắc; rủi ro vẽ sai TUI |

Người dùng chọn hướng 1.

**Vị trí panel**, người dùng chọn: mặc định ở dưới, chuyển được sang phải. Lý do: agent chạy cả buổi; panel dưới trên laptop chỉ ~15–20 dòng, còn đặt bên phải agent có một cột cao hết màn hình mà code vẫn hiện. Hai phương án bị loại: chỉ ở dưới có nút phóng to (phóng to thì che code); terminal làm tab editor (đã để ngoài MVP, kéo theo sửa editor group).

## 4. Giao diện

Theo Zed (`TerminalPanel` ở revision `b5cd310`, research §2.4).

### 4.1 Panel

```
Vị trí dưới (mặc định):
┌ title bar ───────────────────────────────────────────────┐
├ sidebar ┬ vùng editor ───────────────────────────────────┤
│         │                                                 │
│         ├═════════════ sash 6 px ═════════════════════════┤
│         │ ▣ zsh  ▣ claude ×                     ＋  ⤢  × │ ← thanh tab
│         │ $ claude                                        │
│         │ ╭─────────────────────────────╮                 │
│         │ │ ✻ Welcome to Claude Code    │                 │ ← thân terminal
│         │ ╰─────────────────────────────╯                 │
├ status bar: [📁][🔍][⊞][▣] …                              ┤
└──────────────────────────────────────────────────────────┘

Vị trí phải:
┌ sidebar ┬ vùng editor ─────────┬║┬ ▣ claude ×   ＋ ⤢ × ┐
│         │                      │║│ $ claude            │
│         │                      │║│ …                   │
└─────────┴──────────────────────┴║┴─────────────────────┘
```

- Panel ở dưới chiếm chiều ngang của **vùng editor**, không lấn sang sidebar (giống Zed khi dock dưới). Panel ở phải chiếm toàn bộ chiều cao giữa title bar và status bar.
- Kích thước mặc định: cao 320 px khi ở dưới, rộng 640 px khi ở phải (như Zed). Kéo sash để đổi; tối thiểu 4 dòng hoặc 20 cột, tối đa để lại cho vùng editor ít nhất 120 px. Kích thước của từng vị trí, vị trí hiện tại và trạng thái ẩn/hiện được lưu trong `WorkspaceState` như `sidebar_width`.
- Nền panel `panel.background`; viền 1 px `border` ở cạnh giáp vùng editor. Sash dùng lại cách làm sash của sidebar (`layout/sidebar.rs`), tách thành một hàm dùng chung trong `ui.rs` cho cả hai hướng.
- Panel viết dưới dạng một khung chung (`layout/panel.rs`): vị trí, kích thước, sash, ẩn/hiện, phóng to. Terminal là nội dung duy nhất trong MVP. Không dựng hệ "view container" tổng quát như VS Code.
- **Phóng to** (`workbench.action.toggleMaximizedPanel`, nút ⤢): panel chiếm toàn bộ vùng editor; bấm lần nữa trả về kích thước cũ.

### 4.2 Thanh tab trong panel

- Thanh tab **luôn hiện**, kể cả khi chỉ có một terminal (như Zed; VS Code chỉ hiện danh sách tab bên phải khi có từ hai terminal). Chiều cao, nền `tab_bar.background`, màu tab active/inactive và viền dưới **giống hệt thanh tab của vùng editor**. Tách phần vẽ tab đang nằm trong `editor_groups/view.rs` thành component dùng chung trong `ui.rs`, rồi dùng ở cả hai nơi.
- Mỗi tab: icon `TERMINAL` (đã có trong `icons.rs`), tên, nút × khi hover hoặc khi active. Tên tab theo §6.6 (ví dụ `zsh`, `claude`, `vim`), cắt bớt bằng `…` khi dài hơn 24 ký tự.
- Tab có chuông (bell) hoặc tiến trình đã thoát với mã khác 0 thì tên đổi màu (`warning` / `error`).
- Bên phải thanh tab: `＋` (terminal mới), `⤢` (phóng to/thu nhỏ), `×` (ẩn panel; không giết terminal). Dùng `bar_icon_button` có sẵn. Khác Zed: Zed chỉ hiện nút khi pane có focus; Lucerna luôn hiện, vì chỉ có ba nút và không có split.
- Click giữa vào tab đóng terminal. Kéo tab để sắp lại thứ tự.

### 4.3 Thân terminal

- Nền `terminal.background`. Lề trái một ô, như Zed.
- Thanh cuộn dọc dùng `scrollbar_thumb`, chỉ hiện khi đang cuộn hoặc hover, như editor.
- Mất focus thì con trỏ khối thành khung rỗng (`cursorStyleInactive: outline`).

### 4.4 Nút ở status bar

Thêm nút terminal vào cụm `panel_toggles` ở đầu trái status bar, sau Extensions. Icon `TERMINAL`, tooltip "Terminal (⌃`)", sáng lên khi panel đang hiện. Bấm thì chạy `workbench.action.terminal.toggleTerminal`.

## 5. Kiến trúc

### 5.1 Crate `lucerna-terminal` (mới, không dùng gpui)

Phụ thuộc `alacritty_terminal = "0.26"`. Mọi kiểu của alacritty bị giấu sau crate này; workbench chỉ thấy kiểu của Lucerna. Gồm:

- **`Terminal`**: một phiên terminal. Giữ `Arc<FairMutex<Term<_>>>`, `Notifier` để ghi xuống PTY, và hàng đợi thao tác từ UI. API công khai (tên chỉ là gợi ý):
  - `spawn(TerminalOptions) -> Result<Terminal>`: `TerminalOptions` gồm shell, args, cwd, env, số dòng/cột, kích thước ô, số dòng scrollback.
  - `input(&[u8])`, `paste(&str)` (tự bọc bracketed paste khi app bật mode đó, lọc ký tự ESC trong nội dung dán).
  - `resize(rows, cols, cell_w, cell_h)`: chỉ ghi khi khác lần trước; resize liên tiếp gộp lại.
  - `scroll(Scroll)`, `select_start/update/end(point, kind)`, `selection_text()`, `clear()` (xóa scrollback và màn hình, giữ dòng prompt), `select_all()`.
  - `snapshot() -> Snapshot`: lấy lock **một lần**, áp hàng đợi thao tác, chép các ô **đang hiển thị** cùng con trỏ, vùng chọn, mode (alt screen, mouse mode, bracketed paste, app cursor), vị trí cuộn, tổng số dòng.
  - `mouse_report(event) -> Option<Vec<u8>>`, `key_to_bytes(key, mods, mode) -> Option<Vec<u8>>`: mã hóa phím và chuột theo mode hiện tại. Hàm thuần, test được.
  - `foreground_process() -> Option<ProcessInfo { name, cwd }>`: xem §6.6.
- **Luồng sự kiện.** `EventLoop` của alacritty chạy trên luồng riêng, đọc và parse PTY (mỗi lần giữ lock parse tối đa 64 KB, có sẵn trong alacritty). Listener gửi sự kiện vào một channel. Phía nhận gom theo cửa sổ **4 ms**: sự kiện đầu tiên xử lý ngay, các `Wakeup` gộp thành một, tối đa 100 sự kiện một lô. Byte thô không đi qua channel. Kết quả trả cho workbench là `TerminalEvent`: `Wakeup`, `Title(String)`, `Bell`, `Exited(Option<i32>)`, `ClipboardStore(String)`.
  - `PtyWrite` (trả lời truy vấn của app, ví dụ báo vị trí con trỏ) ghi thẳng xuống PTY theo đúng thứ tự.
  - `ClipboardLoad` (OSC 52 đọc clipboard) bị **từ chối**: app trong terminal không được đọc clipboard. `ClipboardStore` (OSC 52 ghi) được cho phép, vì Claude Code và vim dùng nó để copy qua SSH.
- **Gộp run** (`runs.rs`): hàm thuần nhận một dòng ô của snapshot, trả về danh sách `TextRun { col_start, text, cells, style }` và `BgRect { col_start, cols, color }`. Ô liền nhau cùng font, màu chữ, màu nền, gạch chân, gạch ngang gộp thành một run; nền liền nhau cùng màu gộp thành một hình chữ nhật. Bỏ ô trống không có nền, bỏ ô đệm của ký tự rộng, nối ký tự zero-width vào ô trước. Ký tự khối U+2580–U+259F trả về dạng hình chữ nhật thay vì chữ.
- **Màu** (`color.rs`): đổi màu ô (tên ANSI, chỉ số 0–255, truecolor) sang RGBA theo `TerminalPalette` (16 màu theme + foreground/background/cursor/selection). Chỉ số 16–231 là khối màu xterm, 232–255 là dải xám. Có hàm độ tương phản tối thiểu (WCAG, §7).
- **Dò link** (`links.rs`): hàm thuần trên một dòng logic (nối các dòng bị wrap). Thứ tự: OSC 8, URL (`http`, `https`, `file`, `mailto`), rồi đường dẫn có hoặc không có đuôi vị trí. Đuôi nhận: `:L`, `:L:C`, `(L,C)`, `(L)`, `, line L`, `", line L"` (traceback Python). Có ngân sách thời gian 1 ms mỗi lần dò.

### 5.2 Trong `lucerna-workbench`

Module mới `terminal/`:

- `TerminalPanel` (Entity): danh sách `TerminalTab`, tab active, gọi khung panel §4.1.
- `TerminalView` (Entity, Focusable): một terminal. Giữ `lucerna_terminal::Terminal`, snapshot cuối, trạng thái chuột, link đang hover. Nhận `TerminalEvent` qua một task nền; `Wakeup` thì `cx.notify()`, nên chỉ vẽ lại khi có dữ liệu mới.
- `TerminalElement` (Element): prepaint đo ô, gọi `resize` nếu số dòng/cột đổi, lấy `snapshot()`, gọi gộp run cho các dòng nhìn thấy; paint vẽ theo thứ tự: nền, vùng chọn, chữ, ký tự khối, gạch chân link đang hover, con trỏ, chữ đang soạn của IME.
- Lệnh đăng ký vào `CommandRegistry` (§8).
- `lucerna-theme`: thêm `TerminalColors` vào `Theme`, đọc khóa theme Zed (§7).

### 5.3 Vì sao như vậy

Tách crate giúp test lõi (phím → byte, gộp run, dò link, màu) mà không cần GPUI, và giữ đúng quy tắc chỉ workbench dùng gpui. Lớp adapter học từ Zed để sau này đổi bộ giả lập hoặc lên bản alacritty mới chỉ phải sửa một crate.

## 6. Hành vi

### 6.1 Mở và đóng

- ⌃` (`toggleTerminal`): panel ẩn thì hiện và focus terminal active, chưa có terminal thì tạo một cái. Panel hiện mà focus đang ở chỗ khác thì focus vào terminal. Focus đang ở terminal thì ẩn panel và trả focus về editor. Như VS Code.
- ⌃⇧` (`new`): tạo terminal mới, hiện panel, focus nó.
- Ẩn panel không giết terminal; tiến trình vẫn chạy.
- **Tiến trình thoát** (theo VS Code): mã 0 thì đóng tab. Mã khác 0 hoặc bị tín hiệu giết thì giữ tab, in thêm dòng `[Process exited with code N]` (màu `text_muted`), tên tab đổi màu `error`. Gõ phím bất kỳ vào tab đó thì đóng nó.
- Đóng tab cuối cùng thì ẩn panel (`terminal.integrated.hideOnLastClosed`, mặc định true).
- `kill` (và × trên tab) gửi SIGHUP cho nhóm tiến trình rồi đóng tab, không hỏi (`confirmOnKill` của VS Code mặc định chỉ hỏi với terminal ở vùng editor).
- Đóng cửa sổ hoặc thoát app khi còn terminal đang chạy: **không hỏi** (`terminal.integrated.confirmOnExit` mặc định `never`, như VS Code). Hỗ trợ giá trị `always` và `hasChildProcesses`.
- Terminal không được giữ qua lần mở lại app. Trạng thái panel (hiện/ẩn, vị trí, kích thước) thì được giữ. Nếu panel đang hiện lúc đóng, lần mở sau hiện panel với một terminal mới.

### 6.2 Shell, thư mục, môi trường

- **Shell:** `terminal.integrated.defaultProfile.<os>` trỏ tới một mục trong `terminal.integrated.profiles.<os>` (`path`, `args`, `env`). Không đặt thì dùng `$SHELL`; không có `$SHELL` thì `/bin/zsh` trên macOS, `/bin/sh` trên Linux, `powershell.exe` trên Windows.
- **Login shell trên macOS:** khi dùng shell mặc định, thêm `-l` (zsh nhận `-l`; VS Code dùng `-l`). App mở từ Finder có `PATH` tối thiểu; login shell nạp `PATH` của người dùng (Homebrew, nvm, rustup), nhờ vậy `claude` chạy được. Profile tự đặt `args` thì dùng đúng `args` đó. Linux và Windows không thêm.
- **Thư mục:** `terminal.integrated.cwd` nếu có (tuyệt đối, hoặc tương đối theo gốc workspace); không thì gốc workspace. Ở chế độ mở một file lẻ thì thư mục chứa file đó. Thư mục không tồn tại thì rơi về thư mục home và in một dòng cảnh báo.
- **Môi trường:** env của tiến trình Lucerna, rồi `terminal.integrated.env.<os>`, rồi `env` của profile; giá trị `null` xóa biến. Luôn đặt:
  - `TERM=xterm-256color`, `COLORTERM=truecolor`
  - `TERM_PROGRAM=lucerna`, `TERM_PROGRAM_VERSION=<phiên bản>`
  - `LANG=en_US.UTF-8` nếu `LANG` chưa có hoặc không phải UTF-8 (`detectLocale: auto` của VS Code; app GUI trên macOS thường thiếu `LANG`, khiến tiếng Việt vỡ).
- Không có `inheritEnv` trong MVP (luôn kế thừa).

### 6.3 Restricted Mode

**Quyết định: terminal chạy cả trong thư mục chưa được tin.** Người dùng đã chọn.

- VS Code chặn: mở terminal trong Restricted Mode hiện hộp thoại trust, bấm Cancel thì không mở. Có `terminal.integrated.allowInUntrustedWorkspace` (mặc định false) để bỏ kiểm tra. Zed không chặn terminal mở tay.
- Lucerna theo Zed, **lệch khỏi VS Code**. Lý do: người dùng mở terminal để chạy agent, mà agent tự chạy code trong repo. Hỏi trust lúc mở terminal không bảo vệ thêm được gì, chỉ thêm một bước bấm cho việc làm nhiều nhất trong ngày.
- Vẫn giữ phần an toàn của cả hai bên: khi thư mục chưa được tin, các setting `terminal.integrated.cwd`, `env.*`, `profiles.*`, `defaultProfile.*` **ở lớp Workspace bị bỏ qua**, vì một repo lạ có thể dùng chúng để trỏ shell sang file chạy độc hại. Đọc bằng hàm bỏ lớp Workspace có sẵn của settings store (`store.rs`). Giá trị ở lớp User vẫn áp dụng.
- Không thêm `allowInUntrustedWorkspace`, vì không có gì để cho phép.
- Mở terminal không bật hộp thoại trust, và không tính là "lần đầu cần trust" của language server.

### 6.4 Bàn phím

Khi focus ở terminal, mỗi phím đi qua `dispatch_key` của workbench như bình thường:

1. Vim mode **không** nhận phím khi focus ở terminal.
2. Keymap được giải với context `terminalFocus` (cộng `terminalTextSelected`, `terminalAltBufferActive` khi đúng). Nếu ra một lệnh **và** lệnh đó nằm trong danh sách "bỏ qua shell", workbench chạy lệnh đó.
3. Còn lại, phím được mã hóa theo mode của terminal (app cursor, app keypad, phím chức năng, modifier kiểu xterm `CSI 1;m`) và gửi xuống PTY.

Danh sách bỏ qua shell = danh sách mặc định §8.1, cộng `terminal.integrated.commandsToSkipShell` (thêm ID, hoặc bỏ bằng tiền tố `-`), như VS Code. Phím ⌃ và ⌥ đi xuống shell trừ khi khớp lệnh trong danh sách (⌃`, ⌃⇧`). Chord: `allowChords` mặc định true, như VS Code.

**Riêng phím có ⌘ trên macOS:** shell không nhận được ⌘, nên phím có ⌘ **không bao giờ** đi xuống PTY, kể cả khi lệnh không nằm trong danh sách bỏ qua shell. Nếu nó khớp một lệnh (theo `when`) thì chạy lệnh; không thì bỏ. Tiền tố `-` trong `commandsToSkipShell` chỉ tác dụng với phím không có ⌘. Hệ quả, theo VS Code:
- ⌘W chạy `workbench.action.closeActiveEditor` (keybinding không có `when`): đóng tab editor đang active, **không** đóng terminal. Như VS Code với terminal ở panel. Giết nhầm agent tệ hơn đóng nhầm một file mở lại được.
- ⌘S, ⌘\, ⌘P… chạy như khi focus ở editor.
- ⌘= / ⌘- (`editor.action.fontZoomIn/Out`) chỉ đổi chữ editor, không đổi chữ terminal, như VS Code. Zoom riêng cho terminal để sau.

**Focus:** panel có focus handle riêng, **nằm ngoài** cây `editor_focus`, để `editorFocus` sai khi focus ở terminal. Nhờ vậy vim (`editorFocus && !textInputFocus`) và các lệnh chỉ cho editor (⌘C → `clipboardCopyAction`) không bắt phím của terminal.

- **Option:** `terminal.integrated.macOptionIsMeta` mặc định false (như VS Code). False thì ⌥ tạo ký tự theo bàn phím macOS; true thì ⌥+phím gửi `ESC` + phím.
- **Phím tắt sửa dòng của Zed** (chỉ macOS): ⌘← gửi `^A`, ⌘→ gửi `^E`, ⌘⌫ gửi `^U`, ⌥← gửi `ESC b`, ⌥→ gửi `ESC f`. VS Code không có mặc định này nhưng người dùng macOS quen từ Terminal.app/iTerm. Viết dưới dạng keybinding `sendSequence` trong `default-keybindings.json` với `when: terminalFocus && !terminalAltBufferActive`, để người dùng bỏ được. Ở alt screen (vim, htop, Claude Code ở chế độ toàn màn hình) chúng không chạy: ⌘ bị bỏ, ⌥ mã hóa bình thường xuống app.
- **IME:** `TerminalView` cài `InputHandler` của GPUI. Chữ đang soạn vẽ tại con trỏ, gạch chân; khi chốt thì gửi chuỗi UTF-8 xuống PTY. Vị trí ô con trỏ được báo cho IME để khung chọn chữ hiện đúng chỗ. Bắt buộc cho bộ gõ tiếng Việt.

### 6.5 Chuột, chọn, copy, paste, cuộn

- **Chọn:** kéo để chọn (ngưỡng 2 px), double-click chọn từ (theo `terminal.integrated.wordSeparators`), triple-click chọn dòng logic, ⇧+click mở rộng vùng chọn. ⌥+kéo chọn hình chữ nhật.
- Khi app bật mouse mode (vim, htop, Claude Code), click và kéo được báo xuống app. Giữ ⇧ thì chọn chữ như bình thường (như Zed); `macOptionClickForcesSelection` để ⌥ cũng ép chọn (như VS Code, mặc định false).
- **Copy:** ⌘C khi có vùng chọn (`terminalTextSelected`); không có vùng chọn thì ⌘C không làm gì (⌃C vẫn là ngắt). Vùng chọn được giữ sau khi copy. `copyOnSelection` mặc định false. Linux: ghi thêm primary selection.
- **Paste:** ⌘V; nhiều dòng thì không cảnh báo nếu app bật bracketed paste, có cảnh báo một lần nếu không (`enableMultiLinePasteWarning: auto`).
- **Chuột phải:** `rightClickBehavior` mặc định macOS là `selectWord`. MVP hỗ trợ `selectWord`, `copyPaste`, `paste`, `nothing`, `default`. Chưa có context menu nên `default` hoạt động như `selectWord`.
- **Cuộn:** lăn chuột cuộn scrollback theo `mouseWheelScrollSensitivity`, ⌥ cuộn nhanh theo `fastScrollSensitivity`. Ở alt screen: app bật mouse mode thì báo cuộn xuống app; không thì gửi phím ↑/↓ (alternate scroll). Gõ phím thì nhảy về đáy. Có dữ liệu mới trong lúc đang cuộn lên thì giữ nguyên chỗ đang xem.
- `scrollUp/Down`, `scrollUpPage/DownPage`, `scrollToTop/Bottom` chỉ chạy ở màn chính, như VS Code.

### 6.6 Tên tab và thư mục hiện tại

- Mỗi lần nhận `Wakeup` (tối đa một lần mỗi 500 ms, ở luồng nền), đọc tiến trình foreground của PTY: `tcgetpgrp` trên fd của PTY, rồi tên và cwd của tiến trình đó (macOS: `proc_pidinfo`; Linux: `/proc/<pid>/{comm,cwd}`; Windows: để sau). Không dùng crate `sysinfo` cho riêng việc này. Hàm nằm trong `lucerna-terminal`, có phần theo nền tảng.
- **Tên tab**, theo thứ tự:
  1. Tiêu đề OSC 0/2 nếu tiến trình foreground **hiện tại** đã đặt (Claude Code đặt tiêu đề kiểu `✳ <việc đang làm>`). Khi tiến trình foreground đổi, tiêu đề cũ bị bỏ, để tab không giữ tên của lệnh đã thoát. Shell tự đặt tiêu đề (oh-my-zsh…) thì dùng luôn.
  2. Tên tiến trình foreground. Nếu đó là trình thông dịch (`node`, `bun`, `deno`, `python`, `python3`, `ruby`) thì lấy tên file của đối số script đầu tiên trong argv (Claude Code và Codex cài qua npm chạy dưới `node`, nên tab hiện `claude` thay vì `node`). Đọc argv: macOS `sysctl KERN_PROCARGS2`; Linux `/proc/<pid>/cmdline`.
  3. `Terminal`.
  - Tooltip của tab ghi tên tiến trình và cwd.
- cwd hiện tại dùng để giải link tương đối (§6.7).

### 6.7 Link

Theo VS Code về cách mở, theo Zed về cách dò.

- Giữ ⌘ (Linux, Windows: ⌃) và rê chuột: link dưới chuột được gạch chân, con trỏ chuột thành bàn tay, tooltip ghi đích và "⌘+click để mở". Dò chỉ chạy khi đang giữ ⌘, tối đa một lần mỗi 100 ms hoặc mỗi 5 px.
- ⌘+click:
  - **URL:** mở bằng trình duyệt mặc định nếu scheme thuộc `terminal.integrated.allowedLinkSchemes` (mặc định `http`, `https`, `mailto`, `file`). Scheme khác thì không mở. VS Code hỏi rồi thêm scheme vào setting; MVP bỏ hộp thoại đó.
  - **Đường dẫn:** giải theo thứ tự, dừng ở cái đầu tiên tồn tại (kiểm `stat` ở luồng nền):
    1. tuyệt đối, hoặc `~/…`;
    2. tương đối theo cwd hiện tại của terminal (§6.6);
    3. tương đối theo cwd lúc mở terminal;
    4. tương đối theo gốc workspace;
    5. bỏ tiền tố `a/` hoặc `b/` (đầu ra `git diff`) rồi thử lại 2–4.
  - File: mở trong editor qua `open_path_at`, đặt con trỏ ở dòng/cột nếu có. File ngoài workspace vẫn mở.
  - Thư mục trong workspace: reveal trong Explorer. Thư mục ngoài workspace: không làm gì.
  - Không tìm thấy: không có link (không gạch chân). MVP không có fallback "tìm trong workspace" của VS Code.
- OSC 8 cũng cần ⌘+click như mọi link khác (VS Code), không mở bằng click thường (khác Zed), để click trong Claude Code không vô tình mở trình duyệt.
- `terminal.integrated.enableFileLinks`: `on` (mặc định) hoặc `off`.

### 6.8 Chuông

`terminal.integrated.enableBell` mặc định false. Bật thì tab đổi màu `warning` tới khi gõ phím vào tab đó. Không phát âm thanh.

## 7. Màu và font

### 7.1 Khóa theme

Theme của Lucerna đến từ extension Zed, nên đọc **khóa theme Zed**:

| Khóa Zed | Dùng cho | Khi thiếu |
|---|---|---|
| `terminal.background` | nền | `editor.background` |
| `terminal.foreground` | chữ mặc định | `editor.foreground` |
| `terminal.bright_foreground`, `terminal.dim_foreground` | chữ đậm/mờ mặc định | `terminal.foreground` |
| `terminal.ansi.{black,red,green,yellow,blue,magenta,cyan,white}` | màu 0–7 | bảng mặc định của VS Code (dark/light theo `appearance`), research §1.5 |
| `terminal.ansi.bright_*` | màu 8–15 | như trên |
| `terminal.ansi.dim_*` | chữ mờ có màu | màu thường với alpha 0.7 |
| `players[0].cursor` | con trỏ | `cursor` của UI |
| `players[0].selection` | vùng chọn | `selection` của UI |

Màu vẽ đến từ theme đang chọn và đổi ngay khi đổi theme.

### 7.2 Độ tương phản tối thiểu

`terminal.integrated.minimumContrastRatio` mặc định 4.5 (WCAG, như VS Code; 1 là tắt). Chỉ áp cho màu đến từ theme (16 màu ANSI và màu mặc định), **không** áp cho chỉ số 16–255 và truecolor (app đã chọn chính xác), và không áp cho ký tự khối. Như Zed. Kết quả cache theo cặp (fg, bg).

### 7.3 Font

| Setting | Mặc định | Ghi chú |
|---|---|---|
| `terminal.integrated.fontFamily` | rỗng, tức `editor.fontFamily` | |
| `terminal.integrated.fontSize` | rỗng, tức `editor.fontSize` | Lệch VS Code (12 trên macOS): để terminal và editor khớp nhau như Zed |
| `terminal.integrated.lineHeight` | 1.2 | Hệ số. VS Code 1, Zed 1.3; 1 làm khung box-drawing của Claude Code dính sát, 1.3 hao dòng trên panel dưới |
| `terminal.integrated.letterSpacing` | 0 | |
| `terminal.integrated.fontWeight` / `fontWeightBold` | `normal` / `bold` | |
| `terminal.integrated.drawBoldTextInBrightColors` | true | |
| `terminal.integrated.cursorStyle` | `block` | `line`, `underline` |
| `terminal.integrated.cursorBlinking` | false | |

Tắt ligature. Độ rộng ô là advance của `m`. Chiều cao terminal được làm tròn thành số dòng nguyên; phần dư đặt ở trên khi đang bám đáy, ở dưới khi không.

## 8. Lệnh, phím, settings

### 8.1 Lệnh và phím mặc định (macOS)

Mọi lệnh dưới đây nằm trong danh sách bỏ qua shell mặc định.

| Lệnh | Phím | when |
|---|---|---|
| `workbench.action.terminal.toggleTerminal` | ⌃` | |
| `workbench.action.terminal.new` | ⌃⇧` | |
| `workbench.action.terminal.focus` | | |
| `workbench.action.terminal.kill` | | |
| `workbench.action.terminal.focusNext` / `focusPrevious` | ⌘⇧] / ⌘⇧[ | `terminalFocus` |
| `workbench.action.terminal.clear` | ⌘K | `terminalFocus` |
| `workbench.action.terminal.copySelection` | ⌘C | `terminalFocus && terminalTextSelected` |
| `workbench.action.terminal.paste` | ⌘V | `terminalFocus` |
| `workbench.action.terminal.selectAll` | ⌘A | `terminalFocus` |
| `workbench.action.terminal.clearSelection` | Esc | `terminalFocus && terminalTextSelected` |
| `workbench.action.terminal.scrollUp` / `scrollDown` | ⌘⌥PageUp / ⌘⌥PageDown | `terminalFocus && !terminalAltBufferActive` |
| `workbench.action.terminal.scrollUpPage` / `scrollDownPage` | PageUp / PageDown | như trên |
| `workbench.action.terminal.scrollToTop` / `scrollToBottom` | ⌘Home / ⌘End | như trên |
| `workbench.action.terminal.sendSequence` | (args `{ "text": "…" }`) | |
| `workbench.action.togglePanel` | ⌘J | |
| `workbench.action.toggleMaximizedPanel` | | |
| `workbench.action.positionPanelBottom` / `positionPanelRight` | | |

Ngoài ra là các phím ⌘←, ⌘→, ⌘⌫, ⌥←, ⌥→ dạng `sendSequence` ở §6.4.

**Linux và Windows:** thêm ⌃⇧C / ⌃⇧V cho copy/paste như VS Code; ⌃` giữ nguyên.

**Ghi chú cho Claude Code:** lệnh `/terminal-setup` của Claude Code thêm vào keybinding của VS Code một mục ⇧Enter → `sendSequence`. Có `sendSequence` thì người dùng Lucerna làm được điều tương tự trong `keybindings.json`.

Danh sách bỏ qua shell mặc định còn gồm các lệnh workbench không phải terminal: `workbench.action.quickOpen`, `workbench.action.showCommands`, `workbench.action.findInFiles`, `workbench.action.toggleSidebarVisibility`, `workbench.view.explorer`, `workbench.view.extensions`, `workbench.action.focusFirstEditorGroup`…`Eighth`, `workbench.action.quit`, `workbench.action.gotoLine` (VS Code cũng có các lệnh tương đương trong `DEFAULT_COMMANDS_TO_SKIP_SHELL`).

### 8.2 Settings mới

Ngoài font (§7.3): `defaultProfile.{osx,linux,windows}`, `profiles.{osx,linux,windows}`, `cwd`, `env.{osx,linux,windows}`, `scrollback`, `copyOnSelection`, `rightClickBehavior`, `macOptionIsMeta`, `macOptionClickForcesSelection`, `minimumContrastRatio`, `confirmOnExit`, `hideOnLastClosed`, `enableBell`, `enableFileLinks`, `allowedLinkSchemes`, `wordSeparators`, `commandsToSkipShell`, `allowChords`, `mouseWheelScrollSensitivity`, `fastScrollSensitivity`, `enableMultiLinePasteWarning`, `cursorStyleInactive`, `detectLocale` — tất cả dưới tiền tố `terminal.integrated.`, giá trị mặc định như VS Code trừ chỗ ghi khác ở đây. Thêm `workbench.panel.defaultLocation` (`bottom` | `right`, mặc định `bottom`).

Đổi setting font, màu, scrollback áp ngay cho terminal đang mở. Đổi shell, cwd, env chỉ áp cho terminal mở sau.

## 9. Hiệu năng

### 9.1 Vẽ

- **Theo dòng, gộp run:** mỗi dòng nhìn thấy cho ra các run chữ và hình chữ nhật nền như §5.1. Mỗi run được shape một lần bằng `shape_line` với độ rộng ô cố định để glyph khớp lưới.
- **Chỉ vẽ lại khi có dữ liệu mới:** `TerminalView` chỉ `notify` khi nhận `Wakeup` (đã gộp 4 ms), khi người dùng thao tác (chọn, cuộn, hover link), khi focus đổi, hoặc khi đổi kích thước. Terminal đứng yên thì không có khung hình nào, CPU ~0.
- **Không bao giờ duyệt scrollback khi vẽ.** Snapshot chỉ chứa phần đang hiển thị.
- Panel ẩn: không vẽ, nhưng luồng PTY vẫn đọc và parse để tiến trình không bị chặn.
- **Bước sau nếu đo thấy chậm:** alacritty có damage tracking theo dòng. Nếu prepaint của một terminal 200×60 vượt 2 ms, cache kết quả shape theo dòng và chỉ shape lại dòng bị damage. Không làm trong MVP nếu số đo đạt.

### 9.2 Ngưỡng

| Đo | Ngưỡng |
|---|---|
| Từ lúc nhấn phím tới khi ký tự hiện (shell echo) | ≤ 1 khung hình sau khi PTY trả byte; tổng ≤ 16 ms ở 60 Hz trên M1 |
| Prepaint + paint một terminal 200 cột × 60 dòng đầy chữ có màu | ≤ 4 ms trên M1, bản release |
| `cat` file 100 MB vào terminal | UI không đứng; editor bên cạnh vẫn cuộn được, khung hình ≤ 16 ms; terminal xong trong ≤ 3 s |
| Terminal đứng yên | 0 khung hình mỗi giây |
| RAM scrollback | `scrollback` mặc định **10 000 dòng** (VS Code 1000, Zed 10 000). Ước tính tối đa ~30 MB mỗi terminal ở 120 cột (ô alacritty ~24 byte; dòng chỉ được cấp khi được dùng). Trần cứng 100 000 dòng: setting lớn hơn bị kẹp lại |
| RAM cả app khi mở 1 terminal chạy Claude Code | tăng ≤ 50 MB so với khi không có terminal |
| Binary release | tăng ≤ 1,5 MB (đo: alacritty + PTY ~290 KB, phần còn lại cho workbench) |

**Lệch VS Code ở scrollback:** VS Code mặc định 1000 dòng. Đầu ra một phiên agent dài hơn nhiều, 1000 dòng làm mất phần người dùng cần cuộn lại xem. Lucerna lấy mặc định 10 000 của Zed.

### 9.3 Luồng

- Luồng PTY: đọc, parse, giữ lock tối đa 64 KB mỗi lần (alacritty). Không bao giờ chạm GPUI.
- Luồng UI: lấy lock một lần mỗi khung trong `snapshot()`, giữ ngắn. Không đọc file, không `stat` (giải link chạy nền).
- Đọc tiến trình foreground: luồng nền, có giới hạn tần suất.

## 10. Nền tảng

- **macOS:** mục tiêu chính. Login shell §6.2.
- **Linux:** chạy được: PTY của alacritty, `/proc` cho tiến trình foreground, primary selection, ⌃⇧C/⌃⇧V.
- **Windows:** ConPTY qua alacritty; shell mặc định `powershell.exe`; tên tiến trình foreground để sau (tên tab dùng tên shell). Không test trên Windows trong MVP, nhưng code không được giả định Unix ngoài các chỗ `#[cfg(unix)]` rõ ràng.
- `deny.toml`: khi bắt đầu build cho Linux/Windows, thêm target vào `[graph] targets`. Spec này không đổi file đó.

## 11. Kiểm thử

- **`lucerna-terminal` (unit, không cần GPUI):** mã hóa phím theo mode (app cursor, modifier, phím chức năng, ⌥ là Meta hay không); mã hóa chuột (SGR, normal); gộp run (cùng kiểu gộp, khác kiểu tách, ký tự rộng, zero-width, ký tự khối, nền liền nhau); màu (bảng 256, truecolor, độ tương phản, không áp cho 16–255); dò link (URL, `path:L:C`, `(L,C)`, traceback Python, tiền tố git, link bị wrap qua hai dòng, ngân sách thời gian); snapshot sau khi feed chuỗi byte (con trỏ, alt screen, vùng chọn); env và args khi spawn (login shell, `LANG`, `TERM_PROGRAM`, bỏ lớp Workspace khi chưa tin).
- **Spawn thật** (test tích hợp, Unix): mở `/bin/sh -c 'printf hi'`, đợi `Exited(Some(0))`, snapshot có `hi`. Resize đổi `stty size`.
- **`lucerna-workbench` (gpui test):** phím có ⌘ không bao giờ vào PTY, ⌘W khi focus ở terminal đóng editor chứ không đóng terminal; ⌃` bật tắt và chuyển focus đúng ba trường hợp; tab mới, đóng tab, đóng tab cuối thì ẩn panel; phím trong danh sách bỏ qua shell chạy lệnh, phím khác vào PTY; vim không nhận phím khi focus ở terminal; đổi vị trí panel; kích thước được lưu; tiến trình thoát mã 0 đóng tab, mã khác giữ tab.
- **Thủ công trước khi coi là xong** (đặt cạnh Zed, design-principles §1.1): Claude Code (vẽ khung, màu, cuộn khi đang chạy, ⇧Enter qua `sendSequence`), Codex, vim (mouse mode, alt screen), htop, `cat` file lớn, resize panel khi Claude Code đang vẽ, gõ tiếng Việt bằng bộ gõ Telex trong shell và trong Claude Code, ⌘+click đường dẫn mà Claude Code in ra, đổi theme khi terminal đang mở. So khoảng cách, chiều cao thanh tab, màu nền, viền với Zed.
- Đo các ngưỡng §9.2 và ghi số vào ledger của plan.

## 12. Thứ tự làm (gợi ý cho plan)

1. Crate `lucerna-terminal`: spawn, luồng sự kiện, snapshot, mã hóa phím/chuột, gộp run, màu. Test unit.
2. `lucerna-theme`: `TerminalColors`.
3. Khung panel (`layout/panel.rs`), sash dùng chung, tách component tab, nút status bar, lưu trạng thái.
4. `TerminalView` + `TerminalElement`: vẽ, resize, focus, phím, IME.
5. Chọn, copy, paste, cuộn, chuột.
6. Link.
7. Settings, keybindings, Restricted Mode, tên tab.
8. Đo hiệu năng, review giao diện cạnh Zed.

## 13. Để lại cho người làm

- Tên kiểu và hàm cụ thể trong `lucerna-terminal`.
- Cách chia file trong module `terminal/`.
- Chi tiết mã hóa phím (theo xterm; alacritty có sẵn bảng cho vài trường hợp, phần còn lại tự viết dựa trên tài liệu xterm ctlseqs, không chép từ Zed).
- Giá trị chính xác của regex đường dẫn, miễn qua các test ở §11.

## 14. Tham khảo

- `docs/research/2026-11-13-terminal.md`: VS Code (lệnh, settings, link, trust, env) và Zed (kiến trúc, cách vẽ, giao diện, hiệu năng) ở revision `b5cd310`, kèm kiểm tra giấy phép.
- Zed: `crates/terminal`, `crates/terminal_view` (GPL, chỉ học thiết kế).
- VS Code: `src/vs/workbench/contrib/terminal/`, `terminalContrib/links/`, tài liệu `docs/terminal/*`, `docs/editor/workspace-trust`.
- xterm control sequences: <https://invisible-island.net/xterm/ctlseqs/ctlseqs.html>.
