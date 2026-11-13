# Lucerna — V2: Vim sửa

- **Ngày:** 2026-11-13
- **Trạng thái:** Bản nháp, chờ người dùng duyệt
- **Lộ trình:** mục V2 trong §1.3 của `2026-11-08-lucerna-phase1-design.md`
- **Dựa trên:**
  - V1 `2026-11-10-vim-navigation-design.md`
  - Phase 4 `2026-11-11-lucerna-phase4-editing-design.md`, nhất là §13 "Hợp đồng cho V2"
- **Nghiên cứu:** `docs/research/2026-11-09-vim-mode.md`, mục 6. Nguồn là Zed `crates/vim` tại `b5cd310` (chỉ học thiết kế) và VSCodeVim (MIT).

## 1. Mục tiêu

### 1.1 Kết quả

Người quen Vim sửa được code trong Lucerna bằng ngữ pháp Vim, không phải tắt Vim hay chuyển sang chuột. Ngữ pháp này gồm:
- toán tử kèm motion và count;
- register;
- lặp lại bằng `.`;
- undo;
- text object;
- macro;
- lệnh `:` cơ bản.

Mọi thay đổi vẫn đi qua mô hình sửa của Phase 4. Nhờ vậy undo, gộp thay đổi khi agent sửa file, chấm bẩn trên tab và LSP đều hoạt động như khi gõ bình thường.

### 1.2 Tiêu chí thành công

1. **Lệnh sửa cơ bản.**
   - `dd`, `3dd`, `dw`, `d$`, `ciw` kèm chữ gõ, `yy`, `p`, `P`, `x`, `r`, `J`, `>>` và `gUiw` cho kết quả như VSCodeVim trên các ca test ở §11.1.
   - Mỗi lệnh, kể cả `ciw` + chữ gõ + Esc, là **một** bước undo. `u` và ⌃R đưa con trỏ về chỗ lệnh bắt đầu.
2. **Lặp lại bằng `.`.** `.` lặp lại thay đổi cuối, kể cả chữ gõ qua Telex. `3.` thay count gốc.
3. **Register và clipboard.**
   - `"ayy` rồi `"ap` dùng register `a`.
   - `y` ghi clipboard hệ thống, nên app khác ⌘V được.
   - `dd` không đè clipboard, nhưng `p` ngay sau đó vẫn dán đúng dòng vừa xóa.
   - Copy một đoạn từ trình duyệt rồi gõ `p` thì dán đoạn đó.
4. **Macro.** `qa…q` rồi `5@a` chạy macro 5 lần thành một bước undo. Status bar hiện `recording @a` trong lúc ghi.
5. **Dòng lệnh `:`.**
   - `:` mở Quick Open ở chế độ lệnh Vim.
   - `:w`, `:q`, `:wq`, `:%s/foo/bar/g` và `:12` chạy đúng.
   - `:` mở từ Visual mode tự điền sẵn `'<,'>`.
6. **Agent sửa file trong lúc dùng Vim.** Nếu agent sửa file trong lúc người dùng dùng toán tử Vim, Phase 4.4 vẫn gộp thay đổi hoặc báo xung đột như khi gõ thường.
7. **Hiệu năng.** Một lệnh Vim xử lý xong trong dưới 1 ms trên file 100k dòng, trừ `:%s` và các lệnh tìm kiếm trên file lớn (§10).

### 1.3 Những gì đã quyết định khi brainstorm

| # | Quyết định | Lý do |
|---|---|---|
| B1 | **Clipboard chỉ nhận lần yank.** `vim.useSystemClipboard` nhận thêm giá trị `"onYank"` và lấy đó làm mặc định: `y` ghi clipboard (V1 vốn đã làm vậy), còn `d`, `c`, `x` thì không. Register không tên đọc từ clipboard nếu clipboard mới hơn lần yank cuối (giống `on_yank` của Zed). `true` và `false` giữ đúng nghĩa của VSCodeVim. | Người dùng chọn. Copy từ trình duyệt rồi `x` một ký tự sẽ không làm mất clipboard. Lệch VSCodeVim (mặc định `false`) ở chỗ này. |
| B2 | **Lõi thuần trong `lucerna-vim`.** Crate này tính phạm vi và kiểu của toán tử, rồi dựng `EditPlan` bằng hàm của `lucerna-editor`. | Đúng hợp đồng V2 (spec Phase 4 §13.1). Test toàn bộ không cần GPUI. |
| B3 | **`.` dùng một bản ghi thay đổi** gồm các phím Normal của lệnh, register, count và **các sự kiện gõ trong Insert** (§3.3). Khi phát lại, sự kiện gõ đi lại qua đường gõ của editor. | IME đã commit nên chữ là chữ cuối cùng; auto-close và gõ đè chạy lại giống hệt vì cùng cấu hình. |
| B4 | **Macro ghi phím vào register**, dạng chữ có ký hiệu Vim (`<Esc>`, `<CR>`, `<C-r>`). Chữ chèn trong Insert được ghi dưới dạng chữ. | Giống Vim và VSCodeVim: `"ap` dán được macro, `@x` chạy được text trong register. Zed tách macro khỏi register, nên không làm được hai việc này. |
| B5 | **`:` dùng Quick Open**, như Zed dùng command palette. Không có dòng lệnh ở status bar. | `design-principles.md` §1.1: giao diện theo Zed. Quick Open đã có tiền tố `:` (đi tới dòng). |
| B6 | **Chia hai bước giao:** V2a và V2b (§12). Visual block có toán tử để sau Phase 4.3 (multi-cursor). | Phạm vi lớn. Visual block cần nhiều selection (hợp đồng §13.5). |

### 1.4 Không có trong V2

- Toán tử trong Visual Block (`<C-v>` rồi `d`, `c`, `I`, `A`…). Chờ Phase 4.3 có multi-cursor. Hiện `<C-v>` chỉ chọn và `y` như V1.
- Toán tử `=` và `gq`: chờ Phase 4.6 có format. Toán tử `g?`.
- Surround (`ys`, `cs`, `ds`), `gc` (dùng ⌘/ của editor), `gR`, `ReplaceWithRegister`, plugin kiểu EasyMotion hay Sneak.
- Replace mode `R`, undo theo dòng `U`, undo tree (`g-`, `:earlier`).
- Text object `ia`/`aa` (tham số), `ii`/`ai` (khối thụt lề), `if`/`af`/`ic`/`ac` (hàm và lớp theo tree-sitter).
- Lệnh `:` ngoài danh sách ở §8. `:s` với cờ `c` (hỏi từng chỗ). Range `/pat/`. Lịch sử lệnh `:`. `@:`.
- Remap phím (`vim.normalModeKeyBindings`…), `<leader>`.
- Lưu register hay macro qua các phiên làm việc.
- `⌃A` và `⌃X` (tăng giảm số).

## 2. Tham khảo

Chi tiết ở `docs/research/2026-11-09-vim-mode.md` §6. Lucerna chọn bên nào khi Zed và VSCodeVim khác nhau:

| Điểm | Zed | VSCodeVim | Lucerna |
|---|---|---|---|
| Clipboard mặc định | `always` | `false` | `"onYank"` (B1) |
| `N.` | thay count gốc | lặp cả lệnh N lần | **thay count** như Vim và Zed. VSCodeVim lặp N lần là lệch Vim, không có test, và cho kết quả lạ (`d2w` rồi `3.` xóa 6 từ) |
| `c` ghi `"1` và `"-` | có | không | có, như Vim và Zed |
| Vòng `"1`–`"9` khi đã chỉ định register tên | không dịch | vẫn dịch | không dịch, như Vim |
| `i(` khi con trỏ không nằm trong cặp nào | không làm gì | tìm tới cặp kế tiếp | tìm tới (hành vi theo VS Code) |
| Count với ngoặc (`c2i(`) | bỏ qua | đi ra ngoài các cặp lồng | đi ra ngoài các cặp lồng |
| Macro | tách khỏi register | nằm trong register | nằm trong register (B4) |
| Dòng lệnh `:` | command palette | status bar | Quick Open (B5) |
| `vim.joinspaces` | không có setting, chèn 1 dấu cách | `true` | `true` (theo VS Code) |
| Undo | transaction của editor | tự cài theo diff | transaction của Phase 4 (§5) |

Giấy phép: không chép code từ `crates/vim` của Zed. Ca test chuyển thể từ VSCodeVim ghi nguồn trong comment và trong `THIRD_PARTY_NOTICES.md` (mục "VSCodeVim test cases" đã có).

## 3. Kiến trúc

```
lucerna-editor   + case.rs: đổi hoa thường trên các khoảng → EditPlan
                 + edit_ranges(): thay một danh sách khoảng bằng text → EditPlan (helper chung)
lucerna-vim      phụ thuộc thêm lucerna-editor (crate thuần, không gpui)
                 + operator.rs   Operator, phạm vi + kiểu (Charwise / Linewise / Blockwise, inclusive / exclusive)
                 + object.rs     text object
                 + edit.rs       toán tử → EditPlan (d c y > < gu gU g~ ~ J r x p P)
                 + register.rs   Registers, quy tắc ghi và đọc, clipboard
                 + repeat.rs     LastChange (cho .) và bộ phát lại
                 + macros.rs     ghi và chạy macro, ký hiệu phím <Esc>
                 + ex.rs         parser lệnh : và chạy :s :d :y (thuần)
lucerna-workbench
                 vim.rs          áp Action::Edit qua CodeView::transact_group; undo; clipboard;
                                 ghi chữ chèn; Quick Open chế độ lệnh Vim
                 code_view       + transact_group; CodeViewEvent::Typed trong phiên Insert
                 quick_open      + QuickOpenQuery::VimCommand
```

### 3.1 `lucerna-vim` được dựng `EditPlan`

- `ViewInfo` có thêm `fn edit_ctx(&self) -> Option<lucerna_editor::Ctx<'_>>`, trả `None` khi view chỉ đọc.
- `Action` có thêm các biến thể:
  - `Edit(EditPlan)`;
  - `Type(Typed)`, chỉ dùng khi phát lại (§3.3);
  - `Undo(u32)` và `Redo(u32)`;
  - `WriteClipboard { text, linewise }`;
  - `RunEx(ExCommand)`, cho những lệnh workbench làm: lưu, đóng tab, tắt highlight;
  - `OpenCommandLine { prefill }`;
  - `Message(String)`, ví dụ `3 substitutions on 2 lines`.
- Test của `lucerna-vim` dựng `Ctx` với `EditConfig` mặc định, nhờ feature `test-util` của `lucerna-editor`.
- **Một phím, một lần `feed`.** Mỗi `EditPlan` tính theo văn bản trước khi sửa, nên phím sau phải thấy văn bản đã đổi. Vì vậy `lucerna-vim` không tự chạy vòng lặp phát lại.
  - `Vim` trả về `Outcome::Replay(Vec<ReplayStep>)` khi gặp `.`, `@a` hoặc `@@`. `ReplayStep` là `Key(Key)`, `Type(Typed)`, `Search { pattern, backward }` hoặc `Ex(String)`.
  - **Workbench chạy vòng lặp** (`*(my call)*`): đưa từng bước vào `feed` (hoặc vào đường gõ, tìm kiếm, `:`), áp action của bước đó, rồi mới tới bước sau. Toàn bộ nằm trong **một** `transact_group`.
  - **Tìm kiếm trong lúc phát lại luôn chạy đồng bộ**, kể cả với file trên 1 MB. Nhờ vậy macro không bị cắt ngang bởi một lần tìm ở luồng nền.
  - **Motion thất bại:** mỗi `Outcome` cho biết motion có thất bại không, workbench dừng phát lại ở đó (§6.4).

### 3.2 Một lệnh là một bước undo

- **Gom nhóm.** Workbench áp mọi `Action::Edit` sinh ra từ **một** lần `feed` trong **một** `CodeView::transact_group(|v, cx| …)`.
  - `transact_group` xây trên cờ `suppress_stops` đã có: đẩy stop trước, giữ không đẩy stop trong lúc chạy, rồi đẩy stop sau khi xong.
  - Đây là hàm mà hợp đồng §13.2 của spec Phase 4 đã hứa.
- **Lệnh kết thúc trong Insert** (`c…`, `s`, `S`, `C`, `o`, `O`, `i` có count…): nhóm giữ mở cho tới Esc, như cách phiên Insert đang làm.
- **Macro và `.`:** cả lần chạy `@a` hay `.` là một nhóm, kể cả khi nó gồm nhiều lệnh.

### 3.3 Đi theo chữ chèn trong Insert

- **Sự kiện gõ.** Trong phiên Insert do Vim mở, CodeView phát `CodeViewEvent::Typed(Typed)` cho mỗi **thao tác gõ**, không phải mỗi thay đổi buffer. `Typed` có bốn dạng:

  | Dạng | Ghi lại khi |
  |---|---|
  | `Text(String)` | gõ chữ, hoặc IME commit (chữ cuối cùng) |
  | `Newline` | Enter |
  | `DeleteLeft` | ⌫ |
  | `Completion { delete_left, delete_right, text }` | nhận một mục completion; `text` là chữ thật sự được chèn, snippet đã mở ra |

- **Ghi lại.** `VimLayer` chuyển các sự kiện này vào `Vim::record_typed(…)`, và bản ghi cho `.` và macro (§6) giữ chúng nguyên dạng.
- **Phát lại** dùng **đúng đường gõ** của editor:
  - `Text` đi qua `commands::insert`, nên auto-close và gõ đè chạy lại;
  - `Newline` đi qua `commands::newline`;
  - `DeleteLeft` đi qua `commands::delete_left`;
  - `Completion` xóa và chèn đúng số ký tự quanh con trỏ.

  Ví dụ: `ci(` rồi gõ `foo(` và `bar` được ghi là `Text("foo(")`, `Text("bar")`. Phát lại ra `foo(bar)`, giống lần đầu, vì auto-close cũng chạy lại.
- Di chuyển con trỏ bằng phím mũi tên trong Insert sẽ đóng nhóm undo và bắt đầu bản ghi mới, giống VSCodeVim (`<Left>` tạo điểm undo).

### 3.4 Mark đi theo mọi edit

V1 chỉ dời mark khi file được reload. V2 dời mark theo `Patch` của **mọi** edit, kể cả edit từ view khác của cùng file, bằng `Patch::map_offset`. Lý do: `dd` ở trên một mark phải kéo mark đó lên.

## 4. Toán tử và motion

### 4.1 Ngữ pháp

`["x] [count] operator [count] (motion | text-object)`, hoặc toán tử gõ đúp, hoặc lệnh đơn.

- **Count:** count trước và sau toán tử nhân với nhau (`2d3w` = 6 từ), như Vim và Zed.
- **Toán tử:**

  | Phím | Tác dụng |
  |---|---|
  | `d` | xóa |
  | `c` | xóa rồi vào Insert |
  | `y` | yank |
  | `>` / `<` | thụt lề / lùi lề, dùng `lines::indent_lines` và `outdent_lines` của editor |
  | `gu` / `gU` / `g~` | chữ thường / chữ hoa / đảo hoa thường |

- **Gõ đúp** là dòng hiện tại cộng `count−1` dòng dưới: `dd`, `cc`, `yy`, `>>`, `<<`, `guu`, `gugu`, `gUU`, `gUgU`, `g~~`, `g~g~`.
- **`v`, `V` đặt giữa toán tử và motion** ép đổi kiểu, như Zed:

  | Motion gốc | Sau `v` | Sau `V` |
  |---|---|---|
  | theo dòng | exclusive | — |
  | exclusive | inclusive | theo dòng |
  | inclusive | exclusive | theo dòng |

- **Esc hoặc một phím không hợp lệ** giữa chừng thì hủy toán tử.
- Motion thất bại (ví dụ `f` không thấy ký tự) thì hủy, không sửa gì.

### 4.2 Kiểu của motion (theo Vim và Zed)

| Kiểu | Motion |
|---|---|
| Theo dòng | `j k gg G H M L`, `'x`, `N%`, `+ - <CR>` |
| Inclusive | `$ e E ge gE f t %` |
| Exclusive | `w W b B h l 0 ^ { } F T n N * #`, `` `x ``, `/` và `?` |

**Toán tử với `/` và `?`** (`d/foo<CR>`, `c?bar<CR>`): toán tử được giữ chờ trong lúc find widget mở (cơ chế `VimSearch` của V1). Enter thì hoàn tất toán tử với match tìm được. Esc thì hủy cả hai.

Các luật của Vim:
- **Luật exclusive:** điểm cuối ở cột 0 của dòng sau thì lùi về cuối dòng trước và motion thành inclusive. Nếu điểm đầu lại ở trước ký tự không trắng đầu tiên thì motion thành theo dòng.
- **`dw`:** không xóa qua ký tự xuống dòng. `dw` trên dòng rỗng thì xóa dòng đó; `cw` trên dòng rỗng thì không.
- **`cw` / `cW` trên ký tự không trắng** là `ce` / `cE`. Trên khoảng trắng, `cw` chỉ thay khoảng trắng tới đầu từ kế tiếp. `vim.changeWordIncludesWhitespace=true` tắt luật này.
- **`cc`, `S`, `c` với motion theo dòng:** giữ thụt lề và không xóa ký tự xuống dòng cuối.
- **Xóa theo dòng ở dòng cuối file** thì lấy cả ký tự xuống dòng phía trước.
- **Vị trí con trỏ sau toán tử:**

  | Lệnh | Con trỏ |
  |---|---|
  | `d` theo dòng | ký tự không trắng đầu tiên của dòng đứng vào chỗ dòng bị xóa (theo `vim.startofline`) |
  | `y` | đầu phạm vi; nếu motion đi tới thì giữ nguyên |
  | `>`, `<` | ký tự không trắng đầu tiên của dòng đầu |
  | `gu`, `gU`, `g~` | đầu phạm vi |

### 4.3 Lệnh đơn

| Phím | Hành vi |
|---|---|
| `x`, `X` | Xóa `count` ký tự sau hoặc trước con trỏ, không qua dòng. Ghi `"-`. |
| `s` | `c` trên `count` ký tự (không qua dòng). |
| `S` | Bằng `cc`. |
| `C`, `D` | `c$` / `d$`. Có count thì đi tới cuối dòng thứ count−1 phía dưới. |
| `Y` | Bằng `yy`, như Vim. VSCodeVim cũng làm vậy khi không remap. |
| `r{c}` | Thay `count` ký tự bằng `c`. Còn ít hơn `count` ký tự trên dòng thì không làm gì. `r<CR>` thay bằng một xuống dòng. Con trỏ ở ký tự thay cuối cùng. |
| `~` | Đảo hoa thường `count` ký tự rồi đi tiếp sang phải, không qua dòng. `vim.tildeop=true` thì `~` thành toán tử như `g~`. |
| `J` | Nối `max(count, 2)` dòng: bỏ thụt lề dòng sau và chèn một dấu cách, trừ khi dòng trước rỗng hoặc kết thúc bằng khoảng trắng, hay dòng sau bắt đầu bằng `)`. `vim.joinspaces=true` thì chèn hai dấu cách sau `.`, `!`, `?`. Con trỏ ở chỗ nối. |
| `gJ` | Như `J` nhưng không bỏ khoảng trắng và không chèn dấu cách. |
| `p`, `P` | Bảng ở §6.3. |
| `gp`, `gP` | Như `p`/`P`, con trỏ đặt ngay sau đoạn dán. |
| `u`, `⌃R` | Undo hoặc redo `count` bước của tài liệu. Con trỏ về selection trước nhóm (với undo) hoặc sau nhóm (với redo), rồi kẹp về Normal. Hết bước thì không làm gì. |
| `i a I A o O` có count | `3ifoo<Esc>` chèn `foofoofoo`. `3o` mở ba dòng có cùng chữ. |
| `.` | §6.1. |

`x`, `X`, `D`, `C`, `s`, `S`, `r`, `J`, `~`, `p` và `P` cũng đặt bản ghi cho `.`.

## 5. Text object

Đi sau một toán tử, hoặc dùng trong Visual mode (mở rộng vùng chọn). Trong bảng dưới, `i…` là bên trong (inner) và `a…` là cả bao quanh (around).

| Đối tượng | Quy tắc |
|---|---|
| `iw` `aw` `iW` `aW` | Từ theo lớp ký tự của V1. `aw` lấy khoảng trắng phía sau; nếu không có thì lấy phía trước. Từ đầu dòng thì không lấy thụt lề. Đứng trên khoảng trắng: `iw` lấy cả dải khoảng trắng, `aw` lấy khoảng trắng cộng từ kế tiếp. Có count thì lấy nhiều từ, kể cả qua dòng. |
| `i(` `a(` `ib` `ab` `i{` `a{` `iB` `aB` `i[` `a[` `i<` `a<` (cả `)` `}` `]` `>`) | Cặp ngoặc bao con trỏ, qua nhiều dòng và có lồng nhau. Nếu con trỏ không nằm trong cặp nào, tìm tới cặp kế tiếp phía sau (VSCodeVim). Có count thì đi ra ngoài các cặp lồng. Khi có cây cú pháp, ngoặc nằm trong chuỗi hay comment được bỏ qua (`enclosing_nodes`). `i{` nhiều dòng: dòng chứa ngoặc đóng không bị lấy, và khoảng trắng ở đầu và cuối phần bên trong được bỏ. |
| `i"` `a"` `i'` `a'` `` i` `` `` a` `` | Chỉ tìm trên dòng hiện tại. Bỏ qua dấu nháy có `\` phía trước. Xác định cặp bằng cách đếm số dấu nháy từ đầu dòng. Con trỏ ở trước cặp đầu tiên thì lấy cặp kế tiếp. `a"` lấy cả khoảng trắng phía sau, hoặc phía trước nếu không có. Con trỏ ở sau cặp cuối thì thất bại. |
| `it` `at` | Thẻ HTML, XML hay JSX bao con trỏ. Có cây cú pháp thì tìm node có thẻ mở và đóng cùng tên. Không có cây thì quét text tìm cặp thẻ khớp tên. |
| `ip` `ap` | Đoạn là một khối dòng có chữ, hoặc một khối dòng rỗng. `ap` lấy cả khối rỗng phía sau, hoặc phía trước nếu ở cuối file. Luôn theo dòng. |
| `is` `as` | Câu (kết thúc bằng `.`, `!` hoặc `?` rồi khoảng trắng). `as` lấy khoảng trắng phía sau. |

## 6. Register, `.` và macro

### 6.1 `.` (lặp lại)

- **Bản ghi `LastChange`** gồm:
  - register đã chỉ định;
  - count;
  - các phím của lệnh trong Normal hoặc Visual;
  - danh sách `Insertion` nếu lệnh vào Insert;
  - với lệnh Visual: hình dạng vùng chọn (số dòng và số cột, hoặc số dòng của `V`).
- **Lệnh được ghi:** mọi lệnh làm đổi văn bản. Yank, motion và undo không được ghi.
- **Count:** `N.` thay count của bản ghi. Không có count thì dùng count gốc.
- **Register:** `"1p` rồi `.` thành `"2p` (vòng register số tăng dần), như Vim và VSCodeVim.
- **Lệnh Visual:** khi phát lại, tạo lại vùng chọn cùng hình dạng tính từ con trỏ hiện tại.
- **Phát lại:** trả `Outcome::Replay` (§3.1). Workbench đưa lại các phím, rồi các sự kiện `Typed` qua đường gõ, rồi Esc. Toàn bộ là một nhóm undo.
- **Lệnh có `/` hay `?`** (`d/foo<CR>`): bản ghi giữ mẫu đã gõ dưới dạng `Search { pattern, backward }`, và lúc phát lại thì tìm thẳng, không mở find widget.

### 6.2 Register

- **Tên register:**

  | Tên | Ý nghĩa |
  |---|---|
  | `"` | không tên |
  | `a`–`z` | register có tên; `A`–`Z` nối thêm vào register tương ứng |
  | `0` | lần yank cuối |
  | `1`–`9` | vòng xóa |
  | `-` | lần xóa nhỏ |
  | `_` | bỏ đi |
  | `+`, `*` | clipboard hệ thống |
  | `.` | chữ chèn cuối, chỉ đọc |
  | `%` | đường dẫn file, chỉ đọc |
  | `:` | lệnh `:` cuối, chỉ đọc |
  | `/` | mẫu tìm cuối, chỉ đọc |

- **Nội dung:** mỗi register gồm `{ text, kind: Charwise | Linewise }`. Blockwise để dành cho lúc có Visual block.
- **Quy tắc ghi** (theo Vim):
  - **Yank:** ghi `"` và `0`. Nếu đã chỉ định register tên thì ghi register đó và `"`, không ghi `0`.
  - **Xóa hoặc thay đổi** (`d c x X s S C D`): ghi `"`.
    - Text có `\n` hoặc kiểu theo dòng thì dịch vòng `1`–`9` và ghi vào `1`.
    - Ngược lại ghi `-`.
    - Nếu đã chỉ định register tên thì ghi register đó và `"`, và không động tới vòng số hay `-`.
  - **Nối `A`–`Z`:** kiểu thành theo dòng nếu phần thêm vào theo dòng. Text là `cũ + ("\n" nếu theo dòng và `cũ` chưa kết thúc bằng `\n`) + mới`.
  - **`_`:** không ghi gì, kể cả `"`.
- **Clipboard theo `vim.useSystemClipboard`:**

  | Giá trị | Ghi clipboard | `"` đọc từ đâu |
  |---|---|---|
  | `"onYank"` (mặc định) | mọi lần yank vào `"` | clipboard nếu nó **đã đổi kể từ lần ghi `"` gần nhất** (yank hay xóa); ngược lại là register nội bộ |
  | `true` | mọi lần ghi vào `"` | clipboard |
  | `false` | chỉ khi dùng `"+` hoặc `"*` | register nội bộ |

  - **Kiểu khi đọc clipboard:** `Linewise` nếu metadata `whole_line` của Phase 4 (§5, Clipboard) bằng true, hoặc text khớp đúng lần ghi theo dòng gần nhất. Ngược lại là `Charwise`. Nhờ vậy ⌘C trên một dòng không chọn gì rồi `p` sẽ dán theo dòng.
  - **Nhận biết "đã đổi"** (`*(my call)*`):
    - Mỗi lần ghi `"` (yank hay xóa), `Registers` lưu một dấu vết của text clipboard lúc đó (hash). Với yank thì đó chính là text vừa ghi.
    - Khi đọc `"`, clipboard được dùng nếu hash hiện tại khác dấu vết đó.
  - **Ví dụ:** `yy` dòng A → copy B ở trình duyệt → `dd` dòng C → `p` dán **C**, vì lúc `dd` dấu vết là B và clipboard vẫn là B. Còn `yy` A → copy B → `p` dán **B**.
  - **Workbench đọc clipboard** trước mỗi lần `feed` có ghi hoặc đọc `"`. Đó là toán tử, `x`, `p`, `P` và `"+`. Việc này rẻ, vì chỉ là chuỗi trong bộ nhớ của GPUI.

### 6.3 Paste

| Kiểu | `p` | `P` | Con trỏ |
|---|---|---|---|
| Theo ký tự, một dòng | sau con trỏ | tại con trỏ | ký tự cuối của đoạn dán |
| Theo ký tự, nhiều dòng | sau con trỏ | tại con trỏ | đầu đoạn dán |
| Theo dòng | dòng mới phía dưới | dòng mới phía trên | ký tự không trắng đầu tiên của dòng dán đầu tiên |

- Count lặp lại text: `3p`.
- `gp` / `gP` đặt con trỏ ngay sau đoạn dán. Với kiểu theo dòng, đó là cột 0 của dòng kế tiếp.
- **Visual `p`:** thay vùng chọn, và register không tên nhận vùng chọn cũ. Visual `P` thì không ghi đè register không tên.
- Register rỗng thì không làm gì.

### 6.4 Macro

- **Ghi:** `q{a-z}` bắt đầu ghi, `q` lần nữa dừng lại; `q{A-Z}` nối thêm vào macro cũ.
  - Nội dung ghi là các phím đã `feed` và các sự kiện `Typed` trong Insert (§3.3), viết thành text theo ký hiệu Vim:

    | Thứ được ghi | Viết thành |
    |---|---|
    | `Text` | chữ nguyên văn |
    | `Newline` | `<CR>` |
    | `DeleteLeft` | `<BS>` |
    | phím đặc biệt | `<Esc>`, `<Tab>`, `<C-x>`, `<lt>` |
    | `Completion` | chữ đã chèn, cùng số `<BS>` tương ứng với phần bị xóa bên trái; phần bị xóa bên phải bị bỏ qua. Đây là cách làm gần đúng, ghi rõ trong README. |
    | tìm kiếm | `/pat<CR>` hoặc `?pat<CR>`, như Vim |
    | lệnh `:` | `:cmd<CR>`, như Vim |

  - Khi chạy macro, `lucerna-vim` đọc `/…<CR>` và `:…<CR>` thành `ReplayStep::Search` và `ReplayStep::Ex`, rồi chạy thẳng mà không mở giao diện.
  - Macro được lưu vào register dưới dạng text theo ký tự, nên `"ap` dán được nó.
- **Chạy:** `[count]@{a-z}` chạy macro `count` lần. `@@` chạy lại macro chạy gần nhất.
  - Text bất kỳ trong register cũng chạy được: nó được đọc theo ký hiệu ở trên.
  - Đang chạy mà một motion thất bại (ví dụ `f` không tìm thấy, `n` hết match khi `wrapscan` tắt) thì dừng cả macro, như VSCodeVim.
  - Cả lần chạy là một nhóm undo.
  - Trong lúc chạy, vòng register số không dịch và `.` không bị ghi đè.
- **Hiển thị:** trong lúc ghi, chỉ báo mode hiện `-- NORMAL -- recording @a` (cả Zed và VSCodeVim đều làm vậy).

## 7. Visual mode có toán tử

Áp cho Visual và Visual Line.

| Phím | Hành vi |
|---|---|
| `d`, `x` | Xóa vùng chọn. |
| `D`, `X` | Xóa theo dòng. |
| `c`, `s` | Xóa rồi vào Insert. |
| `C`, `S`, `R` | Như `c`, nhưng theo dòng. |
| `y` | Như V1, nhưng giờ ghi register theo §6.2. |
| `Y` | Yank theo dòng. |
| `>`, `<` | Thụt lề / lùi lề `count` mức. |
| `~`, `u`, `U` | Đảo hoa thường / chữ thường / chữ hoa. |
| `r{c}` | Thay mọi ký tự trong vùng chọn (trừ xuống dòng) bằng `c`. |
| `J`, `gJ` | Nối các dòng của vùng chọn. |
| `p`, `P` | §6.3. |

- Sau mỗi toán tử, con trỏ về đầu vùng chọn và Vim trở về Normal (Insert với `c`).
- Mark `<` và `>` được cập nhật.
- `:` từ Visual mở dòng lệnh với `'<,'>` đã điền sẵn.

## 8. Dòng lệnh `:` (bước V2b)

### 8.1 Giao diện (theo Zed)

- **Màn tương ứng ở Zed:** `:` mở command palette. Đó là modal ở trên giữa cửa sổ, một ô nhập và danh sách kết quả bên dưới (`command_palette` + `command_interceptor` của crate `vim`).
- **Lucerna:** `:` trong Normal hoặc Visual mở **Quick Open** (cũng là modal ở trên giữa, đã theo Zed) với chữ `:` có sẵn trong ô nhập.
- **Danh sách bên dưới:**
  - Một dòng duy nhất mô tả lệnh sẽ chạy, ví dụ `:%s/foo/bar/g — Replace in all lines`, `:w — Save`, `:12 — Go to line 12`.
  - Lệnh không hợp lệ thì hiện dòng lỗi màu mờ, ví dụ `Not an editor command: foo`.
  - Enter chạy lệnh, Esc đóng.
- **Hành vi khi tắt Vim:** `:` trong Quick Open vẫn chỉ là đi tới dòng như hiện nay. Chỉ khi Vim bật, phần sau `:` không phải số mới được đọc là lệnh Vim.
- **Kết quả của lệnh** (`3 substitutions on 2 lines`, `E492: Not an editor command`) hiện ở status bar, chỗ chỉ báo mode, cho tới phím kế tiếp.
- **Khác Zed:** không có danh sách gợi ý nhiều lệnh, vì Lucerna chỉ có ít lệnh `:`.

### 8.2 Lệnh

| Lệnh | Tác dụng |
|---|---|
| `:N`, `:$` | Đi tới dòng (giữ hành vi go-to-line hiện có). |
| `:w`, `:write` | `workbench.action.files.save`. |
| `:w!` | Lưu, bỏ qua cảnh báo xung đột (`files.saveConflictResolution` của Phase 4.4: overwrite). |
| `:wa`, `:wall` | `workbench.action.files.saveAll`. |
| `:q`, `:quit` | `workbench.action.closeActiveEditor`. Tab bẩn thì hiện prompt của Phase 4. |
| `:q!` | Revert rồi đóng tab, không hỏi. |
| `:qa`, `:qall` | Đóng mọi tab (có prompt). |
| `:wq`, `:x`, `:xit` | Lưu rồi đóng tab. `:x` chỉ lưu khi file bẩn. |
| `:[range]d [x] [count]` | Xóa dòng vào register. |
| `:[range]y [x] [count]` | Yank dòng. |
| `:[range]>`, `:[range]<` | Thụt lề / lùi lề. |
| `:[range]s/pat/rep/[flags] [count]` | Thay thế, xem dưới. |
| `:noh`, `:nohlsearch` | Tắt highlight tìm kiếm. |

Abbreviation theo bảng `[viết tắt, đầy đủ]` như Zed và VSCodeVim.

**Range:**
- `N`, `.`, `$`, `%`, `'x`, `'<,'>`, `+N`, `-N`, và tổ hợp như `.+3`, `'a,$`.
- Range ngược thì đảo lại.
- `3:` mở với `.,.+2`.

**`:s`:**
- **Dấu phân cách:** mọi ký tự không phải chữ hay số.
- **Mẫu:** viết theo cú pháp regex của Rust, có đổi các ký hiệu Vim phổ biến: `\(` `\)` `\{` `\}` `\+` `\?` `\|` `\<` `\>`.
- **Mẫu rỗng:** dùng mẫu tìm cuối. `:s` không có gì phía sau thì lặp lại lần thay cuối.
- **Phần thay:**

  | Ký hiệu | Nghĩa |
  |---|---|
  | `&`, `\0` | cả match |
  | `\1`–`\9` | nhóm |
  | `\n` | xuống dòng |
  | `\t` | tab |
  | `\\`, `\&` | ký tự chính nó |
  | `~` | phần thay của lần trước |
  | `\u` `\U` `\l` `\L` `\e` `\E` | đổi hoa thường |

- **Cờ:** `g`, `i`, `I`, `n` (chỉ đếm). `vim.gdefault=true` đảo cờ `g`.
- **Phạm vi mặc định:** dòng hiện tại.
- **Sau khi chạy:**
  - Là một bước undo.
  - Con trỏ ở cột 0 của dòng thay cuối cùng.
  - Mẫu thành mẫu tìm cuối, nên `n` dùng được.
  - Status bar báo `N substitutions on M lines`.
- **Không có match:** `E486: Pattern not found: foo`.

## 9. Settings mới

| Khóa | Kiểu | Mặc định | Nguồn |
|---|---|---|---|
| `vim.useSystemClipboard` | bool \| `"onYank"` | `"onYank"` | VSCodeVim (bool, mặc định `false`). Lucerna thêm `"onYank"` (B1). |
| `vim.gdefault` | bool | false | VSCodeVim |
| `vim.joinspaces` | bool | true | VSCodeVim |
| `vim.changeWordIncludesWhitespace` | bool | false | VSCodeVim |
| `vim.tildeop` | bool | false | VSCodeVim |
| `vim.highlightedyank.enable` | bool | false | VSCodeVim (bước V2b) |
| `vim.highlightedyank.duration` | number (ms) | 200 | VSCodeVim (bước V2b) |

- `>` và `<` dùng `editor.tabSize` và `editor.insertSpaces`, như VSCodeVim. Không có `vim.shiftwidth`.
- Màu của highlight khi yank lấy từ theme (`editor.findMatchHighlightBackground`), không có `vim.highlightedyank.color`. Đây là chỗ lệch VSCodeVim, vì giao diện theo theme của Zed.

## 10. Hiệu năng và lỗi

- **Chi phí của một lệnh:** toán tử, text object và register chỉ đọc vài dòng quanh con trỏ.
  - `ip`/`ap` và `i{` có thể phải quét xa, nhưng mỗi dòng chỉ được kiểm tra một lần.
  - Ngoặc dùng `enclosing_nodes` khi có cây cú pháp (O(độ sâu)), và chỉ quét text khi không có cây.
- **`:%s` trên file lớn:**
  - File dưới 1 MB: chạy đồng bộ.
  - File lớn hơn: chạy ở luồng nền trên snapshot. Sau đó áp kết quả nếu revision chưa đổi; nếu đã đổi thì báo `E: file changed, :s cancelled`.
- **Lỗi:**
  - Panic trong `feed` hoặc khi dựng plan bị bắt như V1 (`catch_panic`): không áp gì, Vim về Normal.
  - View chỉ đọc: các toán tử sửa không làm gì. `y` vẫn chạy.
  - Undo hay redo khi hết bước: không làm gì.

## 11. Kiểm thử

### 11.1 `lucerna-vim`, test bảng có đánh dấu con trỏ

- **Chuyển thể khoảng 70 ca từ VSCodeVim**, danh sách và nguồn ở ghi chú nghiên cứu §6. Mỗi ca ghi file và tên test gốc:

  | File của VSCodeVim | Nội dung |
  |---|---|
  | `test/mode/modeNormal.test.ts` | `dw` qua dòng, `cw`, `c2w`, `de`, `dj`, `d}`, `3dd` ở cuối file, `d2gg`, `D`/`C` có count, `Nr`, `r<CR>`, `Nx`, `~`, `g~~`, `guw`, `gUae` |
  | `normalModeTests/commands.test.ts` | `J` có count, joinspaces, `)`, `gJ` |
  | `test/register/register.test.ts` | vòng số, `"-`, `"0`, `"_`, nối `"A` |
  | `test/operator/put.test.ts` | `p` và `gp` có count |
  | `modeVisualLine.test.ts` | `Vp` |
  | text object | `cib` tìm tới, `c3i)`, `ci"` đứng sau, `ca"`, `c2i"`, `daw` hai phía, `d2aw`, `cit`/`cat`, `dip`/`dap`, `das` |
  | `normalModeTests/dot.test.ts` | `.` |
  | `undo.test.ts` | undo |
  | `test/macro.test.ts` | macro |
  | `test/cmd_line/substitute.test.ts`, `delete.test.ts`, `yank.test.ts` | lệnh `:` |

- **Ca riêng của Lucerna:**
  - `ciw` trên `"Xin chào"` và trên emoji.
  - `dd` trên file một dòng và file rỗng.
  - `p` theo dòng ở dòng cuối không có `\n`.
  - Register qua clipboard với ba giá trị của `vim.useSystemClipboard`.
  - `3.` thay count.
  - Macro dừng khi motion thất bại.
  - Macro chứa chữ tiếng Việt.
  - `:s` với `\U&`.
  - Range `'a,'b`.
  - Panic trong lúc dựng plan không làm hỏng trạng thái.

### 11.2 `lucerna-workbench`, dùng `TestAppContext`

- **Undo:** `3dd` rồi `u` khôi phục cả ba dòng trong một bước. `ciw` + gõ (qua input handler, có cả marked text) + Esc rồi `u` cũng chỉ một bước.
- **`.` sau Insert:** gõ có auto-close (`ci(` gõ `foo(`), rồi `.` ở chỗ khác cho cùng text.
- **Clipboard:**
  - `y` ghi `ClipboardItem`.
  - `dd` không ghi clipboard.
  - Clipboard đổi từ bên ngoài thì `p` dán clipboard.
  - ⌘C một dòng (metadata `whole_line`) rồi `p` dán theo dòng.
- **Agent sửa file:** file đổi trên đĩa khi đang có toán tử chưa lưu, thì gộp hoặc xung đột như Phase 4.4.
- **Mark:** đi theo `dd` ở phía trên.
- **Macro:** `qa…q`, `3@a`, chỉ báo `recording @a`.
- **Dòng lệnh `:`:**
  - `:` mở Quick Open; `:w` lưu; `:q` trên tab bẩn hiện prompt; `:%s` thay và báo số lần thay.
  - Khi tắt Vim, `:12` vẫn đi tới dòng.
- **View chỉ đọc** (preview của picker): `dd` không làm gì.

### 11.3 Kiểm tra tay

Bật Vim, rồi sửa một file Rust trong lúc Claude Code sửa cùng file đó. Gõ `ciw` bằng Telex. Dùng `:` và so với command palette của Zed.

## 12. Chia bước giao

| Bước | Nội dung | Phụ thuộc |
|---|---|---|
| **V2a: toán tử** | §3 (trừ phần dòng lệnh); §4; §5; §6.1–§6.3; §7; settings `useSystemClipboard`, `joinspaces`, `changeWordIncludesWhitespace`, `tildeop`; mark theo edit; `transact_group`; `CodeViewEvent::Typed`; toán tử với `/` `?`. | Phase 4.1, 4.2 (đã có) |
| **V2b: macro và `:`** | §6.4 macro; §8 dòng lệnh; `vim.gdefault`; highlight khi yank. | V2a |
| **Sau 4.3** | Toán tử trong Visual Block; register Blockwise; `I` và `A` trên khối. | Phase 4.3 |

Mỗi bước có plan, review từng task và review toàn nhánh riêng. Sau mỗi bước, cập nhật README, bảng roadmap (§1.3 spec Phase 1) và `THIRD_PARTY_NOTICES.md`.

## 13. Rủi ro

| Rủi ro | Cách giảm |
|---|---|
| Chữ chèn qua IME hay completion không khớp với bản ghi `.` | Ghi **thao tác gõ** (chữ sau khi IME commit, Enter, ⌫, completion), rồi phát lại qua đúng đường gõ. Có test có marked text, auto-close, gõ đè và completion. |
| `transact_group` lồng với nhóm của phiên Insert, hoặc với undo stop của Phase 4 | Một chỗ duy nhất mở và đóng nhóm trong `VimLayer`. Có test undo cho mọi lệnh kết thúc trong Insert. |
| Clipboard `"onYank"` khó đoán khi người dùng copy ở app khác | So text clipboard với lần yank cuối, giống Zed. Có test cho từng trường hợp. Ghi rõ trong README. |
| Luật exclusive và theo dòng của Vim dễ sai | Chuyển thể nhiều ca test từ VSCodeVim, có thêm ca riêng cho dòng cuối không có `\n`. |
| Xung đột phím `:` với Quick Open khi tắt Vim | Chỉ đọc lệnh Vim khi Vim bật và phần sau `:` không phải số. Có test khi tắt Vim. |
