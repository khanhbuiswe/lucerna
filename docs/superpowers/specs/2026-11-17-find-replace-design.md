# Find & Replace trong file

Ngày: 2026-11-17. Trạng thái: đã duyệt.

## Mục đích

Người chuyển từ VS Code sang quen bấm ⌥⌘F để thay chữ trong file đang mở. Lucerna mới chỉ có thanh
tìm (⌘F), nên họ phải mở lại VS Code. Spec này thêm ô Replace vào thanh tìm có sẵn, cùng Replace
(một match) và Replace All. Hành vi, lệnh và phím theo VS Code; giao diện theo Zed.

Logic thay thế (đọc chuỗi Replace, khai triển `$1`, giữ hoa thường) nằm ngoài UI và không dùng gpui,
để mục kế tiếp trong lộ trình, Replace trong project (tab Search), dùng lại được.

## Mục tiêu

- ⌥⌘F (`editor.action.startFindReplaceAction`) mở thanh tìm kèm hàng Replace.
- Replace một match (`editor.action.replaceOne`) và Replace All (`editor.action.replaceAll`), mỗi
  lệnh là một bước undo.
- Chuỗi Replace theo cú pháp VS Code ở chế độ regex: `$0` `$&` `$1…$99` `$$` `\n` `\t` `\\`
  `\u` `\U` `\l` `\L`.
- Toggle Preserve Case (`AB`) như VS Code.
- Phím tắt cho các toggle: ⌥⌘C, ⌥⌘W, ⌥⌘R, ⌥⌘P.

## Không làm (lần này)

- Find in Selection (⌥⌘L) và giới hạn Replace All trong vùng chọn.
- Lịch sử chuỗi Replace (↑/↓ trong ô), lưu toggle theo workspace.
- Ô Replace nhiều dòng (Ctrl+Enter chèn xuống dòng). Muốn xuống dòng thì dùng `\n` ở chế độ regex.
- Các setting `editor.find.*`.
- Replace trong project: spec riêng, dùng lại module ở đây.

## Không được xảy ra

- Replace All mà phải bấm undo nhiều lần mới quay lại được.
- Thay sai chỗ vì danh sách match đã cũ (file vừa đổi, hoặc tìm nền chưa xong).
- UI bị treo khi Replace All trên file lớn.
- Thay chữ trong view read-only (preview của picker, file không đọc được).

## Tham khảo

Đọc từ nhánh main của `microsoft/vscode`, `zed-industries/zed` và `helix-editor/helix` (2026-11-17).

### VS Code

- **Chuỗi Replace** (`src/vs/editor/contrib/find/browser/replacePattern.ts`, `parseReplaceString`):
  - Chỉ phân tích khi bật regex. Tắt regex thì chuỗi là chữ thường, kể cả `\n` và `$1`.
  - `$$` → `$`. `$0` và `$&` → cả match. `$1`…`$9` là nhóm.
  - Hai chữ số được đọc tham lam: `$NN` là nhóm NN. Nếu không có nhóm NN thì dùng nhóm N rồi chữ
    `0`, nên `$10` khi chỉ có một nhóm thành nhóm 1 + `"0"`.
  - Không có nhóm nào khớp chỉ số thì giữ nguyên `$` và các chữ số (`$5` khi có 2 nhóm → `"$5"`).
    Nhóm có nhưng không tham gia match → chuỗi rỗng.
  - `$` ở cuối chuỗi, hoặc `$` đi trước ký tự khác, giữ nguyên.
  - `\\` → `\`, `\n` → LF, `\t` → tab. Escape lạ như `\q` giữ nguyên `\q`; `\` đứng cuối cũng giữ.
  - `\u` `\l` đổi hoa/thường một ký tự, `\U` `\L` đổi cả phần; chúng tác động lên mảnh `$n`/`$&`
    ngay sau và có thể chồng nhau (`\u\L$1`). Bản thân chúng không sinh ra chữ.
- **Preserve case** (`src/vs/base/common/search.ts`, `buildReplaceStringWithCasePreserved`), xét
  chữ đã match:
  1. Nếu cả match lẫn chuỗi thay đều có `-` với cùng số đoạn thì xử lý từng đoạn rồi nối lại bằng
     `-`. `_` cũng vậy. Nếu cả `-` lẫn `_` đều thỏa thì bỏ qua bước này.
  2. Match toàn chữ hoa → chuỗi thay viết hoa hết.
  3. Match toàn chữ thường → viết thường hết.
  4. Ký tự đầu của match là chữ hoa → viết hoa ký tự đầu của chuỗi thay, phần còn lại giữ nguyên.
  5. Ký tự đầu là chữ thường → viết thường ký tự đầu.
  6. Còn lại: giữ nguyên.

  Preserve case chạy cả khi tắt regex.
- **Replace một match** (`findModel.ts` `replace()`): lấy match kế tiếp tính từ đầu selection.
  - Nếu selection trùng đúng match đó thì thay nó (một bước undo), rồi chọn match sau chỗ vừa chèn.
  - Nếu không thì chỉ chọn match đó. Vậy lần bấm đầu chọn, lần thứ hai thay.
- **Replace All** (`replaceAll()`): tìm mọi match, không giới hạn số lượng, rồi gom thành một lệnh
  và một bước undo. Selection được giữ qua các edit. Với từ 19999 match trở lên, nó thay cả file
  bằng `String.replace`.
- **Lệnh và phím (macOS):**

  | Lệnh | Phím | Khi |
  |---|---|---|
  | `editor.action.startFindReplaceAction` | ⌥⌘F | |
  | `editor.action.replaceOne` | ⇧⌘1 | `findWidgetVisible` |
  | `editor.action.replaceOne` | Enter | `replaceInputFocussed` |
  | `editor.action.replaceAll` | ⌥⌘Enter | `findWidgetVisible` |
  | `editor.action.replaceAll` | ⌘Enter | `replaceInputFocussed` |
  | `toggleFindCaseSensitive` | ⌥⌘C | `editorFocus` |
  | `toggleFindWholeWord` | ⌥⌘W | `editorFocus` |
  | `toggleFindRegex` | ⌥⌘R | `editorFocus` |
  | `togglePreserveCase` | ⌥⌘P | `editorFocus` |

  Trên Windows và Linux: Ctrl+H, Ctrl+Shift+1, Ctrl+Alt+Enter, Alt+C/W/R/P.
- **Widget** (`findWidget.ts`):
  - Có nút chevron bên trái để mở hoặc gập hàng Replace.
  - ⌥⌘F focus ô Replace nếu ô Find đang có focus hoặc vừa được điền từ selection; nếu không thì
    focus ô Find.
  - ⌘F khi thanh đang mở thì giữ hàng Replace; khi thanh đang ẩn thì mở mà không có hàng Replace.
  - Editor read-only: không mở được Replace, nút chevron bị tắt.
  - Nút Replace và Replace All bị tắt khi ô Find trống.
  - Chuỗi Replace được giữ theo từng editor qua các lần đóng/mở, và không bao giờ điền từ selection.
  - Tab trong ô Replace chuyển focus sang toggle trong ô Find; Shift+Tab chuyển về ô Find.

### Zed

- **Bố cục** (`crates/search/src/buffer_search.rs`):
  - Hàng 1: ô query có toggle CaseSensitive / WholeWord / Regex nằm trong ô. Tiếp theo là các nút
    icon: Toggle Replace (`IconName::Replace`, hiện trạng thái bật), Search Selection, Select All
    Matches. Sau đó là nhóm `‹ › n/m`, và nút Close ở bên phải.
  - Hàng 2 (khi bật replace): ô replace (placeholder "Replace with…") rộng bằng ô query, rồi hai nút
    **icon**: `ReplaceNext` ("Replace Next Match") và `ReplaceAll` ("Replace All Matches").
- **Phím:** trong ô replace, Enter là `search::ReplaceNext` và ⌘Enter là `search::ReplaceAll`.
  ⇧⌘H là `search::ToggleReplace`. ⌥⌘C/W/X bật tắt toggle; ⌥⌘X cho regex khác với ⌥⌘R của VS Code.
- **Hành vi:**
  - `replace_next` luôn thay match đang active, không cần selection trùng match, rồi chọn match
    kế.
  - Cả replace một match lẫn replace all đều là một transaction.
  - Khai triển chuỗi: tắt regex thì là chữ thường. Bật regex thì chỉ bỏ escape cho `\\` `\n` `\t`,
    rồi gọi `Captures::expand` của `fancy_regex`.
  - Zed không có case modifier và không có preserve case.

### Helix

Helix không có giao diện find/replace. Người dùng chọn cả file (`%`), chọn mọi match regex (`s`),
rồi gõ chữ mới vào tất cả con trỏ (`c`). Vì chữ được gõ chứ không khai triển nên không có tham
chiếu nhóm.

### Crate `regex`

`Captures::expand` khác VS Code ở mấy điểm sau:

- Tên nhóm là chuỗi dài nhất gồm `[_0-9A-Za-z]`, nên `$1a` là nhóm tên "1a" (rỗng), trong khi
  VS Code hiểu là nhóm 1 + `a`.
- Nhóm không tồn tại → rỗng, trong khi VS Code giữ `$5`.
- Không có `$&`, `\n` hay `\u`.

### Lucerna chọn

| Phần | Theo | Vì sao |
|---|---|---|
| Cú pháp chuỗi Replace, Preserve Case | VS Code | Hành vi theo VS Code; Zed thiếu case modifier và Preserve Case. Tự viết bộ phân tích (không chép code VS Code, ghi nguồn trong comment), không dùng `expand`. |
| Replace một match | VS Code | Hành vi. Trong Lucerna thanh tìm đã chọn sẵn match active, nên thực tế lần bấm đầu đã thay, giống cảm giác của Zed. |
| Một bước undo cho mỗi lệnh | Cả hai | |
| Lệnh, phím, key context | VS Code | ⌥⌘R cho regex (không phải ⌥⌘X của Zed). |
| Bố cục hai hàng, nút toggle sau ô query, hai nút icon | Zed | Giao diện. VS Code đặt chevron ở mép trái. |
| Toggle `AB` trong ô Replace | VS Code | Zed không có Preserve Case; đặt trong ô giống `Aa` `ab` `.*`. |

## Thiết kế

### 1. Module `lucerna_project::replace` (không gpui)

Đặt cạnh `build_pattern` (crate này đã dùng `regex` và là nơi Search của project sống). Gồm:

- `ReplacePattern::parse(text: &str, is_regex: bool) -> ReplacePattern`. Tắt regex thì là chữ
  tĩnh. Bật regex thì là danh sách mảnh (chữ, nhóm N, cả match, cùng case op), theo đúng các quy tắc
  VS Code ở trên.
- `ReplacePattern::expand(&self, caps: &regex::Captures) -> String`, kèm fallback `$NN` → nhóm N +
  chữ số, và giữ `$N` khi không có nhóm.
- `ReplacePattern::is_static()`: không cần captures, để đi đường nhanh.
- `preserve_case(matched: &str, replacement: &str) -> String`: sáu quy tắc ở trên.
- `replace_all(text: &str, regex: &Regex, pattern: &ReplacePattern, preserve_case: bool)
  -> Vec<(Range<usize>, String)>`: mỗi match **không rỗng** cho một edit, theo thứ tự, tọa độ byte
  trước khi sửa. Bỏ match rỗng để khớp với những gì thanh tìm tô sáng và đếm (`find_in_snapshot`
  cũng bỏ). Đây là quyết định của Lucerna.
- `replacement_at(text, regex, range, pattern, preserve_case) -> Option<String>`: chuỗi thay cho một
  match đã biết. Trả `None` nếu regex không còn khớp đúng `range`.

Regex lấy từ `build_regex` hiện có (cùng cờ case, whole word, multi-line như find). Unit test phủ
từng quy tắc VS Code ở mục Tham khảo, kể cả ví dụ tiếng Việt cho Preserve Case (`Đường` → hoa
chữ đầu).

### 2. Thanh tìm (`find_widget.rs`)

`FindWidget` thêm: `replace_input: Entity<TextInput>` (placeholder `Replace`),
`replace_visible: bool` và `preserve_case: bool`. Chuỗi Replace nằm trong widget, nên được giữ qua
các lần đóng/mở của view đó.

Bố cục, dùng `search_toolbar`, `search_bar_row`, `search_query_box`, `toggle_button`,
`bar_icon_button` và `bar_separator` trong `ui.rs`:

```
[Find…………………  Aa ab .*] [⇄] │ ‹ › 3/19          │ ×
[Replace……………  AB       ] │ [replace next] [replace all]
```

- **Hàng 1** giữ như hiện tại, chỉ thêm nút icon Toggle Replace ngay sau ô query (tooltip
  `Toggle Replace`, sáng khi bật). View read-only không có nút này.
- **Hàng 2** chỉ hiện khi `replace_visible`:
  - Ô Replace dùng `input_box` qua `search_query_column`, nên rộng bằng ô query. Toggle `AB`
    (`Preserve Case`) nằm trong ô.
  - Sau đó là `bar_separator` và hai `bar_icon_button`: `Replace Next Match` và
    `Replace All Matches`. Hai nút tắt (không bấm được, màu mờ) khi ô Find trống, regex lỗi hoặc
    không có match.
- **Icon:** thêm hai file Lucide nguyên bản `replace.svg` và `replace-all.svg` vào
  `assets/icons/` (ISC, đã có `LICENSE-lucide`).
  - `replace` dùng cho Toggle Replace, `replace-all` cho Replace All.
  - Replace Next cần một icon Lucide khác (gợi ý `arrow-right-to-line`); người làm chọn, miễn là
    khác icon toggle.
- **Vim:** khi thanh mở cho `/` hoặc `?`, hàng Replace và nút toggle ẩn.

### 3. Hành vi

- **⌥⌘F** (`editor.action.startFindReplaceAction`, nhãn `Replace`):
  - Nếu view read-only thì làm như ⌘F.
  - Nếu không, mở thanh và đặt `replace_visible = true`. Ô Find được điền từ selection một dòng,
    như ⌘F.
  - Focus sang ô Replace nếu ô Find đã có focus trước đó hoặc vừa được điền từ selection; nếu không
    thì focus ô Find.
- **⌘F:** thanh đang ẩn thì mở với `replace_visible = false`. Thanh đang mở thì giữ nguyên hàng
  Replace.
- **Nút Toggle Replace:** đảo `replace_visible`. Bật thì focus ô Replace, tắt thì focus ô Find.
- **Tab / Shift+Tab:** Tab trong ô Find chuyển sang ô Replace (khi hàng Replace đang hiện);
  Shift+Tab trong ô Replace chuyển về ô Find. VS Code đi qua cả các toggle; Lucerna chỉ nhảy giữa
  hai ô, vì toggle đã có phím tắt.
- **Replace một match** (`editor.action.replaceOne`, nhãn `Replace`):
  - Không làm gì nếu view read-only, hàng Replace đang ẩn, hoặc đang có tìm nền.
  - Lấy match active. Nếu selection trùng đúng range của nó thì tính chuỗi thay bằng
    `replacement_at` trên snapshot hiện tại, rồi áp dụng một `EditPlan` có `stop_before` và
    `stop_after`.
  - Sau đó match active là match đầu tiên bắt đầu từ cuối chỗ vừa chèn trở đi; nếu không còn thì
    quay về match đầu. Match này được chọn và cuộn tới.
  - Nếu selection không trùng (người dùng đã di chuyển con trỏ) thì chỉ chọn match gần nhất, như
    `next`.
- **Replace All** (`editor.action.replaceAll`, nhãn `Replace All`):
  - Gọi `replace_all` trên snapshot hiện tại, rồi áp dụng một `EditPlan` duy nhất có `stop_before`
    và `stop_after`.
  - Selection được đưa qua các edit, như VS Code giữ selection. Cách làm là việc của người làm,
    miễn con trỏ không nhảy lung tung.
  - File lớn hơn `BACKGROUND_SEARCH_BYTES` thì tính edit trên background executor rồi chỉ áp dụng
    nếu `revision` của view chưa đổi. Nếu đã đổi thì bỏ, không thay gì.
  - Không giới hạn số match.
  - Sau khi thay, thanh tìm lại (tự động khi revision đổi) và thường sẽ ra `0/0`.
- **Toggle:** `toggleFindCaseSensitive`, `toggleFindWholeWord`, `toggleFindRegex` và
  `togglePreserveCase` đổi toggle trên widget của view đang active. Nếu view chưa có widget thì tạo
  widget mà không hiện, để lần mở sau dùng trạng thái mới, giống VS Code giữ state khi widget ẩn.
- **Read-only:** cả hai lệnh replace đều không làm gì (`transact` cũng đã chặn). Nút Toggle Replace
  không hiện.

### 4. Lệnh, phím, key context, menu

- **Key context mới:** `replaceInputFocussed` (tên theo VS Code), bật khi ô Replace có focus. Đặt
  cùng chỗ với `findWidgetVisible` trong `workbench.rs`.
- **Thêm vào cuối `default-keybindings.json`**, gom một khối:

  ```json
  { "key": "alt+cmd+f", "command": "editor.action.startFindReplaceAction", "when": "editorFocus" },
  { "key": "shift+cmd+1", "command": "editor.action.replaceOne", "when": "editorFocus && findWidgetVisible" },
  { "key": "alt+cmd+enter", "command": "editor.action.replaceAll", "when": "editorFocus && findWidgetVisible" },
  { "key": "enter", "command": "editor.action.replaceOne", "when": "editorFocus && replaceInputFocussed" },
  { "key": "cmd+enter", "command": "editor.action.replaceAll", "when": "editorFocus && replaceInputFocussed" },
  { "key": "alt+cmd+c", "command": "toggleFindCaseSensitive", "when": "editorFocus" },
  { "key": "alt+cmd+w", "command": "toggleFindWholeWord", "when": "editorFocus" },
  { "key": "alt+cmd+r", "command": "toggleFindRegex", "when": "editorFocus" },
  { "key": "alt+cmd+p", "command": "togglePreserveCase", "when": "editorFocus" }
  ```

  - ⌥⌘C/W/R đã dùng cho `searchViewletFocus` và `filesExplorerFocus`. `editorFocus` không trùng với
    hai context đó, nên không xung đột. Người làm kiểm tra lại bằng test keymap.
  - Enter trong ô Replace phải thắng xử lý Enter hiện có của `FindWidget::key_down`, vốn chỉ dành
    cho ô Find.
  - Nếu `editorFocus` không phủ focus trong thanh tìm (cần kiểm tra bằng test), dùng đúng context
    mà `closeFindWidget` đang dựa vào.
- **Menu Edit:** thêm `Replace` (`editor.action.startFindReplaceAction`) ngay sau `Find`. Chỉ thêm
  một dòng, vì session ⌘N cũng sửa `app_menu.rs`.
- **Nhãn trong Command Palette:** `Replace`, `Replace One`, `Replace All`,
  `Toggle Case Sensitive`, `Toggle Whole Word`, `Toggle Use Regular Expression`,
  `Toggle Preserve Case`. VS Code dùng nhãn `Replace` cho cả `replaceOne`; Lucerna đổi thành
  `Replace One` để palette không có hai dòng `Replace`.
- **Tài liệu:**
  - `docs/keyboard-shortcuts.md` và `CHANGELOG.md`: chỉ thêm dòng, không sửa dòng cũ, vì session
    ⌘N cũng sửa hai file này.
  - `docs/roadmap.md`: đánh dấu mục này đã xong.
  - `docs/design-principles.md` §1.1: bỏ câu ví dụ "bỏ Replace vì Lucerna chỉ để xem code", vì
    câu này đã lỗi thời.

### 5. Test

- **`lucerna-project`:** unit test cho `ReplacePattern` (mọi quy tắc ở mục Tham khảo),
  `preserve_case` và `replace_all` (match rỗng, Unicode, nhiều dòng).
- **`lucerna-workbench`** (gpui test, theo kiểu test find hiện có):
  - ⌥⌘F hiện hàng Replace; ⌘F khi thanh ẩn thì không.
  - Replace một match thay đúng match active, chuyển sang match kế, và một lần undo trả lại.
  - Selection không trùng match thì chỉ chọn.
  - Replace All thay hết và một lần undo trả lại hết.
  - Có `$1` khi bật regex; có Preserve Case.
  - View read-only không đổi.
  - Enter và ⌘Enter trong ô Replace.
  - Các toggle qua phím.
  - Replace All trên file lớn hơn 1 MB.

## Còn để người làm quyết

- Icon Lucide cho Replace Next.
- Cách đưa selection qua edit của Replace All.
- Tên hàm và cấu trúc bên trong module `replace`.
