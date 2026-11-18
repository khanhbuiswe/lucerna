# Replace trong project (tab Search)

Ngày: 2026-11-18. Trạng thái: đã duyệt.

## Mục đích

Người chuyển từ VS Code sang quen bấm ⇧⌘H để đổi một chữ trên cả project. Lucerna đã có tab Search
(⇧⌘F) và Find & Replace trong một file (⌥⌘F), nhưng chưa thay được trên nhiều file. Muốn làm việc
đó, họ phải mở lại VS Code. Spec này thêm hàng Replace vào tab Search, cùng với Replace All, thay
theo từng file và thay theo từng dòng.

Người dùng Lucerna để agent (Claude Code, Codex) viết code, và agent đọc file từ đĩa. Vì thế chữ đã
thay phải nằm trên đĩa ngay sau khi thay. Agent cũng có thể đang ghi chính những file đó, nên
replace không bao giờ được ghi đè lên chữ agent vừa viết.

Hành vi, lệnh và phím theo VS Code; giao diện theo Zed.

## Mục tiêu

- ⇧⌘H (`workbench.action.replaceInFiles`) mở tab Search với hàng Replace.
- Ô Replace có toggle Preserve Case (`AB`). Cú pháp chuỗi Replace giống thanh tìm trong file:
  `$1`, `$&`, `\n`, `\u`… khi bật regex (dùng lại `lucerna_project::replace`).
- Khi ô Replace có chữ, kết quả tìm hiện trước chữ sẽ thay: chữ cũ gạch ngang, liền sau là chữ mới.
- Replace All có hộp thoại xác nhận theo câu chữ của VS Code.
- Thay theo từng dòng và từng file bằng nút hiện khi hover, hoặc ⇧⌘1. Bỏ một dòng hay một file khỏi
  kết quả (Dismiss, ⌘⌫).
- Ghi file theo phương án A mà người dùng đã chọn:
  - file đang đóng ghi thẳng xuống đĩa;
  - file đang mở và đã lưu nhận một bước undo rồi được lưu;
  - file đang có sửa chưa lưu chỉ được thay trong editor.
- UI không bao giờ chờ I/O. Việc đọc, tìm lại và ghi chạy ở luồng nền.
- Whole word của tab Search theo `editor.wordSeparators`, như thanh tìm trong file, để chữ được thay
  đúng là chữ đang hiện trong kết quả.

## Không làm (lần này)

- Một bước undo cho cả project (VS Code có "Workspace Edit"). File đang đóng không undo được trong
  app; người dùng quay lại bằng git.
- Mở diff xem trước khi chọn một match (`search.useReplacePreview`).
- Ô Replace nhiều dòng (Ctrl+Enter chèn xuống dòng); muốn xuống dòng thì dùng `\n` ở chế độ regex.
- Lịch sử chuỗi Replace, lưu chuỗi Replace vào workspace state.
- Thay theo từng match riêng trong một dòng có nhiều match. Kết quả của Lucerna tính theo dòng.
- Match vượt qua ranh giới dòng. Tab Search tìm theo từng dòng nên không hiện được loại match này.

## Không được xảy ra

- Ghi đè lên chữ mà chương trình khác (agent) đã ghi vào file sau khi replace đọc file đó.
- Âm thầm lưu phần người dùng đang sửa dở (VS Code làm vậy; Lucerna thì không).
- Thay một chữ không hiện trong kết quả. Ví dụ: luật whole word khác giữa lúc tìm và lúc thay, hoặc
  match vượt dòng.
- Hỏng file: mất BOM, đổi encoding, hoặc ghi file decode bị lossy (byte lạ đã thành U+FFFD).
- UI treo khi Replace All trên hàng nghìn file.
- Thay sai chỗ vì range trong kết quả đã cũ.

## Tham khảo

Đọc từ nhánh main của `microsoft/vscode` (2026-11-18) và Zed ở revision GPUI đang pin (`b5cd310`).
Zed là GPL nên chỉ học thiết kế.

### VS Code

Mã nằm ở `src/vs/workbench/contrib/search/browser/`: `searchActionsRemoveReplace.ts`,
`searchActionsNav.ts`, `searchWidget.ts`, `searchView.ts`, `replaceService.ts`,
`searchResultsView.ts` và `searchTreeModel/*`. Phần thay chuỗi nằm ở
`services/search/common/replace.ts`.

- **Lệnh và phím (macOS; Windows/Linux trong ngoặc):**

  | Lệnh | Phím | Khi |
  |---|---|---|
  | `workbench.action.replaceInFiles` "Replace in Files" | ⇧⌘H (Ctrl+Shift+H) | luôn; có trong menu Edit |
  | `search.action.replace` "Replace" | ⇧⌘1 (Ctrl+Shift+1) | `replaceActive && matchFocus && isEditableItem` |
  | `search.action.replaceAllInFile` "Replace All" | ⇧⌘1, ⇧⌘Enter | `replaceActive && fileMatchFocus` |
  | `search.action.replaceAll` "Replace All" | ⌥⌘Enter (Ctrl+Alt+Enter) | `searchViewletVisible && replaceActive && findWidgetNotVisible` |
  | `search.action.remove` "Dismiss" | ⌘⌫ (Delete) | `fileMatchOrMatchFocus` |
  | `toggleSearchPreserveCase` | ⌥⌘P (Alt+P) | `searchViewletFocus` |
  | `closeReplaceInFilesWidget` | Esc | `replaceInputBoxFocus` |

  - Không có lệnh bật tắt hàng Replace; chỉ có nút chevron "Toggle Replace" ở mép trái.
  - ⇧⌘H mở view, bật hàng Replace, điền query từ selection rồi focus.
  - `replaceActive` chỉ đúng khi hàng Replace đang hiện **và** có kết quả.
  - Trong ô Replace: Enter chạy lại search ("press Enter to preview"); Ctrl+Enter chèn xuống dòng;
    Tab sang toggle của ô query.
- **Replace All:**
  - Không có kết quả thì không làm gì.
  - Nếu có thì **luôn** hỏi lại, không có setting nào bỏ bước này. Tiêu đề "Replace All", nút
    "Replace", Cancel là mặc định. Câu hỏi có 8 biến thể (số ít/số nhiều):
    `Replace {n} occurrence(s) across {m} file(s) with '{value}'?`, hoặc không có phần `with` khi
    chuỗi rỗng. Chuỗi dài hơn 10 dòng bị cắt còn 10 dòng và thêm `...`.
  - Thay xong thì xóa hết kết quả và hiện
    `Replaced {n} occurrence(s) across {m} file(s) with '{value}'.`
- **Ghi file** (`ReplaceService.replace`):
  - Mỗi match là một edit. Mọi edit nằm trong một bulk edit, nên mọi file đều được mở thành model và
    cả project có **một bước undo** ("Workspace Edit").
  - Sau đó VS Code **lưu mọi file đã thay**, kể cả file vốn đang có sửa chưa lưu.
  - Kết quả cũ: file đang mở được tìm lại 250 ms sau mỗi thay đổi. File đang đóng thì dùng range lúc
    tìm, không kiểm tra gì, nên nếu file đã đổi trên đĩa thì edit rơi vào chỗ sai.
  - Chỉ thay các match nằm trong kết quả (giới hạn bởi `search.maxResults`).
- **Thay từ cây kết quả:**
  - Nút inline: Replace (codicon `replace`) trên match, "Replace All" trên file hoặc folder (chỉ hiện
    khi `replaceActive`), và Dismiss (codicon `close`).
  - Thay một match thì lưu file đó, rồi tìm lại các match trên dòng đó. Thay một file thì bỏ file
    khỏi cây. Focus chuyển sang phần tử kế tiếp cùng loại.
- **Xem trước:**
  - Trong cây, chữ cũ bị gạch ngang trên nền `diffEditor.removedTextBackground`, chữ mới nằm trên
    nền `diffEditor.insertedTextBackground`. Chữ mới được khai triển theo từng match (nhóm regex có
    giá trị thật).
  - `search.useReplacePreview` (mặc định bật) mở diff editor
    `{file} ↔ {file} (Replace Preview)` khi chọn một match.
- **Widget:** chevron ở mép trái, các hàng thụt 18 px. Ô Replace có placeholder "Replace", toggle
  Preserve Case nằm trong ô, và một nút icon `replace-all` với tooltip
  "Replace All (Submit Search to Enable)" khi bị tắt.
- **Chuỗi thay:** parser riêng theo kiểu `String.replace` của JS, có thêm case op. Mỗi match được
  khai triển bằng cách chạy lại regex trên chính đoạn match.

### Zed

Mã nằm ở `crates/search/src/project_search.rs`, `search_bar.rs` và
`crates/project/src/project_search.rs`.

- **Bố cục `ProjectSearchBar`:** hàng query, dòng lỗi, hàng replace (khi bật), hàng filter, dòng lỗi
  filter.
  - Nút toggle là icon `Replace`, tooltip "Toggle Replace", sáng khi bật. Nó nằm sau nút Filter
    trong cụm nút của hàng query. Bật thì focus ô replace, tắt thì focus về ô query.
  - Hàng replace có một khoảng trống để thẳng cột với nút gập. Tiếp theo là ô replace (placeholder
    "Replace in project…", rộng bằng ô query), rồi hai nút icon `ReplaceNext` "Replace Next Match"
    (tắt khi đang tìm) và `ReplaceAll` "Replace All Matches". Tooltip có ghi phím.
- **Phím:** ⇧⌘H ngoài tab là `pane::DeploySearch { replace_enabled: true }`; trong tab là
  `search::ToggleReplace`. Trong ô replace: Enter là ReplaceNext, ⌘Enter là ReplaceAll.
- **Hành vi:**
  - Không có hộp thoại xác nhận, không có nút theo file hay theo match, không có xem trước.
  - Replace sửa buffer trong multibuffer kết quả. Replace All là một transaction cho mọi file, một
    bước undo. **Không lưu**: các buffer ở trạng thái dirty, và lưu tab Search thì lưu hết.
  - Nếu search đang chạy thì replace chờ kết quả xong. Nếu query khác lần tìm trước thì tìm lại rồi
    mới thay. Match là anchor nên vẫn đúng sau khi buffer bị sửa.
  - Giới hạn 5.000 file và 10.000 match ("Search Limits Reached"). Replace All chỉ thay những gì đã
    tìm được.
  - Replace chạy đồng bộ trên luồng chính.

### Helix

Không có replace cho cả project. `global_search` (`Space /`) chỉ là một picker. Người dùng dùng công
cụ ngoài (`sed`, `sd`, scooter), hoặc mở từng file rồi `%` `s` `c`.

### Lucerna chọn

| Phần | Theo | Vì sao |
|---|---|---|
| Lệnh, phím, context key, câu chữ hộp thoại và thông báo | VS Code | Hành vi. |
| Hộp thoại xác nhận Replace All | VS Code | Hành vi. Thay trên đĩa và không undo xuyên file được, nên càng cần hỏi lại. |
| Xem trước gạch ngang trong kết quả | VS Code | Zed không có. Người dùng soát code của agent cần thấy trước chữ sẽ thay. Lệch khỏi giao diện Zed, có lý do. |
| Nút theo dòng và theo file, Dismiss | VS Code | Hành vi. Nút trên header file dùng chỗ của nút "Open File" mà Zed đã có. |
| Bố cục hàng Replace, nút Toggle Replace bằng icon | Zed | Giao diện. VS Code đặt chevron ở mép trái. |
| Chỉ một nút Replace All, không có Replace Next | VS Code | VS Code không có Replace Next; Enter trong ô Replace là tìm lại. |
| Ghi xuống đĩa và lưu | VS Code | Người dùng chọn (phương án A): agent đọc từ đĩa. |
| Không lưu file đang có sửa chưa lưu | Lucerna | Lệch khỏi VS Code: lưu hộ người dùng phần đang sửa dở là bất ngờ. |
| Không có undo xuyên file | Lucerna | Lucerna chưa có undo cho cả workspace; để sau. |
| Tìm lại trên nội dung hiện tại lúc thay, kiểm tra hash trước khi ghi | Lucerna | VS Code ghi vào range cũ khi file đã đổi trên đĩa; agent ghi file thường xuyên nên Lucerna không được làm vậy. |
| Chạy ở luồng nền | Zed và nguyên tắc §3 | Cả VS Code và Zed đều thay trên luồng chính. |

## Thiết kế

### 1. Giao diện tab Search

Dùng các component có sẵn trong `ui.rs`: `search_bar_row`, `search_query_box`,
`search_query_column`, `input_box`, `toggle_button`, `bar_icon_button`, `bar_separator`.

```
[⌄⌃] [Search………………………  Aa ab .*] [⊟] [⇄] [↻] │ ‹ › 19 results
     [Replace……………………  AB         ] │ [replace-all]
     [files to include      ] [files to exclude      ]
───────────────────────────────────────────────────────────
 ▾ foo.rs  src/                              [⇄] [×]   ← hover
   12  let o̶l̶d̶N̶a̶m̶e̶newName = 1;                  [⇄] [×]   ← hover
```

- **Hàng query:** thêm nút `bar_icon_button` Toggle Replace (icon `replace`, đã có từ thanh tìm
  trong file) ngay sau nút filter. Tooltip `Toggle Replace`; nút sáng khi bật.
- **Hàng Replace:** chỉ hiện khi bật, nằm giữa hàng query và hàng filter.
  - Bên trái có một khoảng trống thẳng cột với nút gập, như hàng filter.
  - Ô Replace dùng `input_box` trong `search_query_column`, rộng bằng ô query. Placeholder `Replace`.
    Toggle `AB` (`Preserve Case`) nằm trong ô.
  - Sau ô là `bar_separator` và một `bar_icon_button` icon `replace-all`, tooltip `Replace All`.
    Nút tắt khi không có kết quả, khi search đang chạy, hoặc khi đang thay.
- **Chuỗi Replace** được giữ khi ẩn và hiện lại hàng Replace (sống trong `SearchView`).
- **Xem trước trong kết quả:** khi hàng Replace hiện **và** ô Replace có chữ, mỗi range match của
  dòng được vẽ thành chữ cũ gạch ngang trên nền `diff.removed`, liền sau là chữ mới trên nền
  `diff.inserted`.
  - Chọn màu nào trong theme (`editor.diff.*` hay `version_control.*`) là việc của người làm, miễn
    là có sẵn trong theme và đọc được ở cả theme sáng lẫn tối.
  - Chữ mới được khai triển theo đúng dòng đó: `$1` hiện giá trị thật, có áp dụng Preserve Case.
    Cách tính: chạy regex trên `line_text` của dòng (đã cắt ở 1.000 ký tự), rồi lấy chữ thay cho
    từng range. Một range không khớp lại được (bị cắt, lookaround) thì chỉ gạch ngang, không có chữ
    mới.
  - Phần xem trước chỉ bật cho list của tab Search. `ExcerptList` dùng chung với diff view, nên
    thêm một tùy chọn render, mặc định tắt.
  - Chữ mới được tính lúc render, chỉ cho các hàng đang hiện, và cache theo (chuỗi Replace, toggle).
    Không tính cho mọi dòng mỗi khi gõ.
- **Nút khi hover** (chỉ khi hàng Replace hiện và search đã xong):
  - Dòng match: `[⇄]` (`replace`, tooltip `Replace`) và `[×]` (`x`, tooltip `Dismiss`). Dòng ngữ
    cảnh không có nút.
  - Header file: `[⇄]` (`replace-all`, tooltip `Replace All`) và `[×]` (tooltip `Dismiss`), đặt
    trước nút Open File.
  - Dismiss thì hiện cả khi hàng Replace ẩn, như VS Code.
- **Dòng trạng thái của list:** trong lúc thay, bộ đếm ghi `Replacing…`. Thay xong thì list trống
  và hiện thông báo kết quả ở giữa, ở chỗ `centered_message` hiện "No results found".

### 2. Lệnh, phím, context key, menu

| Lệnh | Nhãn | Phím (macOS) | Khi |
|---|---|---|---|
| `workbench.action.replaceInFiles` | `Search: Replace in Files` | ⇧⌘H | luôn |
| `search.action.replaceAll` | `Search: Replace All` | ⌥⌘Enter | `searchViewletFocus && replaceActive` |
| `search.action.replace` | `Search: Replace` | ⇧⌘1 | `searchViewletFocus && replaceActive && matchFocus` |
| `search.action.replaceAllInFile` | `Search: Replace All in File` | ⇧⌘1 | `searchViewletFocus && replaceActive && fileMatchFocus` |
| `search.action.replaceAllInFile` | | ⇧⌘Enter | `searchViewletFocus && replaceActive && fileMatchFocus` |
| `search.action.remove` | `Search: Dismiss` | ⌘⌫ | `searchViewletFocus && fileMatchOrMatchFocus` |
| `toggleSearchPreserveCase` | `Search: Toggle Preserve Case` | ⌥⌘P | `searchViewletFocus` |
| `closeReplaceInFilesWidget` | `Search: Close Replace Widget` | Escape | `searchViewletFocus && replaceInputBoxFocus` |

- **Context key mới**, đặt cùng chỗ với `searchViewletFocus` trong `workbench.rs`:
  - `replaceActive`: hàng Replace hiện và có ít nhất một kết quả (như VS Code).
  - `replaceInputBoxFocus`: ô Replace có focus.
  - `matchFocus`: list có focus và dòng được chọn là dòng match.
  - `fileMatchFocus`: list có focus và một header file được chọn.
- **Chọn được header file:** `ExcerptList` hiện cho ↑/↓ bỏ qua header. Spec này cho ↑/↓ dừng ở
  header khi tab là tab Search (diff view giữ như cũ), để có `fileMatchFocus`, ⇧⌘Enter, Dismiss cả
  file, và việc chọn phần tử kế sau khi thay. Enter trên header thì mở hoặc gập file như khi bấm vào
  header. F4 và `‹ ›` vẫn chỉ đi qua các dòng match.
  - `fileMatchOrMatchFocus`: một trong hai.
- **`togglePreserveCase` (⌥⌘P) của thanh tìm** đang có `editorFocus && !searchViewletFocus`, nên
  không xung đột với phím mới.
- **⇧⌘1** đang dùng cho `editor.action.replaceOne` với `editorFocus && findWidgetVisible`. Tab
  Search nằm trong `editorFocus`, nên phải kiểm tra bằng test keymap rằng ở tab Search thì lệnh của
  Search thắng (dùng `!searchViewletFocus` cho dòng của editor nếu cần, như đã làm với các toggle).
  ⌥⌘Enter cũng vậy.
- **Esc trong ô Replace** ẩn hàng Replace và focus ô query. Esc ở chỗ khác giữ hành vi hiện tại.
- **Enter trong ô Replace** chạy search ngay (như Enter trong ô query), để xem trước được làm mới.
  Thực ra phần xem trước đã tự cập nhật khi gõ; Enter chỉ cần bỏ qua debounce.
- **Tab / Shift+Tab:** Tab ở ô query sang ô Replace (khi hàng Replace hiện), Shift+Tab ở ô Replace
  về ô query. Như thanh tìm trong file.
- **Menu Edit:** thêm `Replace in Files` (`workbench.action.replaceInFiles`) ngay sau
  `Find in Files`, nếu dòng đó có; không có thì đặt sau `Replace`. Chỉ thêm một dòng.
- **Thêm vào cuối `default-keybindings.json`** thành một khối, không sửa các dòng cũ.

### 3. Hành vi

- **⇧⌘H** (`workbench.action.replaceInFiles`):
  - Làm như ⇧⌘F (`find_in_files`: mở hoặc focus tab Search, điền query từ selection một dòng), rồi
    bật hàng Replace.
  - Focus ô Replace nếu ô query có chữ, ngược lại thì focus ô query.
- **⇧⌘H khi đang ở tab Search** cũng làm như trên, không đảo trạng thái. Muốn ẩn hàng Replace thì
  bấm nút Toggle Replace hoặc Esc trong ô Replace.
- **Nút Toggle Replace:** đảo trạng thái hàng Replace. Bật thì focus ô Replace, tắt thì focus ô
  query.
- **Replace All** (nút, ⌥⌘Enter, `search.action.replaceAll`):
  1. Không làm gì nếu không có kết quả, search đang chạy, đang có một lần thay khác chạy, hoặc query
     đã đổi mà chưa tìm lại (`stale`). Riêng trường hợp `stale` thì chạy search ngay, không thay.
  2. Hỏi bằng `window.prompt` (`PromptLevel::Warning`), nút `Replace` và `Cancel` (Cancel mặc
     định). Câu hỏi theo VS Code (8 biến thể số ít/số nhiều, có hoặc không có `with '{value}'`,
     chuỗi cắt ở 10 dòng). `n` và `m` là số match và số file đang hiện trong kết quả. Nếu kết quả
     đã bị cắt ở `MAX_RESULTS` thì thêm dòng chi tiết:
     `The results only contain a subset of all matches. Files not in the results are not changed.`
  3. Nếu đồng ý thì chạy job thay (mục 4) trên **đúng các dòng match đang hiện**, trừ các dòng và
     file đã Dismiss. Chữ không hiện trong kết quả không bao giờ bị thay (xem "Chỉ thay những gì
     đang hiện" ở mục 4).
  4. Khi xong: xóa kết quả và hiện
     `Replaced {n} occurrence(s) across {m} file(s) with '{value}'.` (không cắt chuỗi), trong đó
     `n` và `m` là số thật sự đã thay. Nếu có file bị bỏ qua thì thêm một dòng
     `Skipped {k} file(s): {lý do của từng file}` (xem bảng ở mục 4), tối đa 5 file và `…`.
- **Replace theo dòng** (nút trên dòng, ⇧⌘1 khi chọn dòng):
  - Thay các match của dòng đó. Job thay chạy cho một file, chỉ với dòng này, theo cùng quy tắc
    "Chỉ thay những gì đang hiện". Nếu dòng đã đổi thì không thay, và status bar báo
    `The file has changed since the search. Search again to replace it.`
  - Nếu file đang mở và có sửa chưa lưu thì status bar báo
    `Replaced in {name}. The file has unsaved changes.`
  - Sau khi thay, file được tìm lại một mình và các excerpt của nó được cập nhật: dòng vừa thay biến
    mất nếu không còn match. Không có hộp thoại.
  - Selection chuyển sang dòng match kế tiếp (hoặc dòng trước nếu đó là dòng cuối), như VS Code.
- **Replace theo file** (nút trên header, ⇧⌘1 hoặc ⇧⌘Enter khi chọn header): thay các dòng match
  đang hiện của file, theo cùng quy tắc như Replace All, không hỏi lại. File bị bỏ qua thì status
  bar báo lý do. Sau đó bỏ file khỏi kết quả và chọn dòng match đầu
  tiên của file kế tiếp.
- **Dismiss** (`search.action.remove`, nút `×`, ⌘⌫): bỏ dòng hoặc file khỏi kết quả, không sửa
  file. Bộ đếm và `replaceActive` tính lại theo kết quả còn lại. Lần search sau thì mọi thứ hiện lại
  (VS Code nhớ match đã Dismiss trong một lần tìm; Lucerna không cần nhớ qua các lần tìm).
- **Preserve Case** (`toggleSearchPreserveCase`, toggle `AB`): đổi cờ trên `SearchView`; phần xem
  trước vẽ lại. Không cần tìm lại.
- **Đổi chuỗi Replace không chạy lại search.** Chỉ phần xem trước vẽ lại.
- **Trong lúc thay:** các ô nhập vẫn sửa được, nhưng mọi lệnh thay khác đều bị chặn tới khi xong.
  Một search mới (gõ query) thì vẫn được phép: kết quả thay sẽ không xóa kết quả của search mới đó.
  Thông báo "Replaced…" chỉ hiện nếu không có search nào bắt đầu sau khi lần thay đã chạy.

### 4. Job thay (`lucerna_project::replace_in_files`, không gpui)

`lucerna-project` thêm phụ thuộc `lucerna-text` (đọc, decode, encode, ghi atomic đều nằm ở đó).

**Đầu vào:**

- query tìm (pattern, regex, case, whole word, `editor.wordSeparators`);
- `ReplacePattern` và cờ Preserve Case;
- danh sách file: đường dẫn tuyệt đối, kèm các dòng match đang hiện của file đó, mỗi dòng là
  `(line_number, line_text)` lấy từ kết quả (đã trừ các dòng Dismiss);
- `TempFiles` của các lần lưu (`save::save_temps`), để temp file bị dọn khi thoát app;
- cờ hủy (`AtomicBool`).

**Chỉ thay những gì đang hiện** (quyết định của Lucerna, để giữ luật "không thay chữ không hiện
trong kết quả"):

- Với mỗi file, job so từng dòng được giao với dòng cùng số trong nội dung hiện tại (so phần đầu dài
  bằng `line_text`, tức tối đa 1.000 ký tự).
- Nếu **có một dòng bất kỳ** không còn khớp (file đã bị sửa sau lần tìm, dòng bị dịch, file ngắn
  đi) thì bỏ qua **cả file** với lý do `changed since the search`. Thay một phần file khi file đã đổi
  dễ sai hơn là bỏ qua; người dùng tìm lại rồi thay tiếp.
- Nếu khớp hết thì chỉ thay các match nằm trên những dòng đó và **bắt đầu trong phần đã hiện** (1.000
  ký tự đầu). Match mới xuất hiện ở dòng khác, hoặc nằm ở phần dòng bị cắt, đều không bị thay.
- File cuối cùng bị cắt giữa chừng bởi `MAX_RESULTS` chỉ được thay ở các dòng đã hiện.
- Vì vậy số trong thông báo "Replaced…" bằng số trong hộp thoại, trừ đi các file bị bỏ qua.
- Hàm sẵn có `replace_ranges` / `replacement_at` của `lucerna_project::replace` nhận range đã lọc;
  dùng chúng thay vì `replace_all` (vốn thay mọi match).

**Cách tìm match khi thay** (dùng chung cho xem trước, cho job thay và cho search):

- Regex dựng giống search: case, multi-line cho `^`/`$`, không có `\b`.
- Bỏ match rỗng.
- Bỏ match chứa `\n`, vì search theo dòng không hiện được nó.
- Whole word lọc bằng **một luật duy nhất** theo `editor.wordSeparators`, cùng luật với
  `lucerna_editor::find::is_whole_word`.
  - Luật mức ký tự (ranh giới dựa trên ký tự đứng trước và sau, cùng ký tự ở mép match) được chuyển
    xuống `lucerna-text`, để `lucerna-editor` (trên `BufferSnapshot`) và `lucerna-project` (trên
    `&str`) cùng gọi nó. Luật này dựa vào `classify` và `CharClass` trong
    `lucerna-editor/src/words.rs` (cũng dùng cho word motion), nên hai thứ đó chuyển theo;
    `lucerna-editor` re-export để code cũ không đổi.
  - **Giá trị `editor.wordSeparators`:** thanh tìm trong file đọc theo ngôn ngữ của file. Tab Search
    dùng một giá trị cho cả lần tìm: setting ở mức window, không theo ngôn ngữ, vì luồng tìm nền
    không biết ngôn ngữ của từng file. Override `wordSeparators` theo ngôn ngữ rất hiếm; nếu có thì
    tab Search có thể khác thanh tìm ở file đó. Đây là quyết định của Lucerna. Xem trước, job thay
    và search luôn dùng cùng một giá trị, lấy lúc bắt đầu search.
  - Không có bản sao thứ hai của luật.
- **Search của tab cũng đổi theo.** `build_pattern` không còn bọc `\b` cho whole word. Matcher của
  `grep` tìm không có `\b`, và sink lọc từng match theo luật trên. Dòng mà mọi match đều bị lọc thì
  không phải là match: không đếm, không báo (nếu nằm trong ngữ cảnh của match khác thì thành dòng
  ngữ cảnh, hoặc bỏ; người làm chọn cách đơn giản nhất mà test vẫn đúng).
  - `SearchQuery` thêm trường `word_separators: String`, lấy từ setting `editor.wordSeparators` của
    window.
  - `search_view::model::first_match` và `build_regex` trong `find_widget.rs` (dùng cho tab Search)
    cũng đi qua cùng luật.
  - Test hiện có của `build_pattern` (`\b(?:…)\b`) được viết lại.

**Từng file đang đóng** (chạy song song trên luồng nền, số luồng có giới hạn):

1. Đọc byte, `check_regular` và `check_size`, `decode_file(bytes, false)`. Giữ lại hash và stamp
   của lúc đọc.
2. Bỏ qua và báo lý do nếu: binary, quá lớn, lossy, hoặc không đọc được.
3. Kiểm tra dòng theo "Chỉ thay những gì đang hiện", tìm match trên các dòng đó, rồi tính edit
   bằng `replace_ranges`. Không có edit thì không ghi.
4. Áp dụng edit vào text, rồi `Buffer::encode` với encoding, BOM và line ending của file. Ký tự mới
   không encode được (`Unencodable`) thì bỏ qua file.
5. Ngay trước khi ghi, đọc lại stamp. Nếu khác lúc đọc thì so hash; nếu hash khác thì **không ghi**
   và báo `changed on disk` (giống `changed_on_disk` của save). Sau đó `write_bytes_tracked`.
6. Trả về kết quả của file: số match đã thay, hoặc lý do bỏ qua.

Lưu ý: decode đổi CRLF thành LF, và ghi lại theo line ending trội. Một file có lẫn CRLF và LF sẽ bị
đồng nhất, giống khi mở file đó trong Lucerna rồi lưu (VS Code cũng vậy). Đây là hành vi chấp nhận
được.

**File đang mở trong `Documents`** (tức là có document và text đã load): job không đụng tới đĩa
của file này. Workbench xử lý chúng:

1. Trên luồng UI, lấy `snapshot_of`, `revision_of`, `is_dirty`, `has_conflict`, và trạng thái lossy.
2. Bỏ qua nếu: đang conflict (`has_conflict`), lossy, hoặc chưa load xong (`Loading`, `Error`).
3. Tính edit trên snapshot ở luồng nền, bằng cùng hàm của job.
4. Quay về luồng UI. Nếu revision chưa đổi thì `apply_edits` thành một bước undo (`push_stop`
   trước và sau). Nếu revision đã đổi thì tính lại một lần nữa; đổi lần nữa thì bỏ qua với lý do
   `changed while replacing`.
5. Nếu file **sạch trước khi thay** thì lưu qua đường save của `Documents`, với
   `participants: false` (cờ đã có trong `SaveOptions`): format on save hay trim whitespace không
   được tự chạy trên file mà người dùng chỉ thay chữ.
   - Lần lưu này **không bao giờ mở hộp thoại**. Nếu lúc ghi mà file đã đổi trên đĩa
     (`ChangedOnDisk`), document vào trạng thái conflict như một autosave gặp conflict: thanh
     conflict hiện, buffer giữ chữ đã thay ở dạng chưa lưu, và file được báo với lý do
     `changed on disk`. Một Replace All trên nhiều file đang mở không được bật ra nhiều hộp thoại.
   - Cách làm: thêm một `SaveReason` (hoặc một cờ trong `SaveOptions`) cho lần lưu này, cư xử như
     autosave: không hỏi conflict, và không tạo lại file vừa bị xóa (báo `no longer exists`).
     Người làm chọn cách gọn nhất.
6. Nếu file **đang có sửa chưa lưu** thì chỉ thay trong buffer, không lưu. File được tính là đã
   thay; dòng thông báo thêm `{k} file(s) with unsaved changes were not saved.` Phần thay là một
   bước undo nằm trên các sửa chưa lưu; undo một lần thì buffer trở lại đúng trạng thái dirty trước
   đó.
7. Các view của document tự cập nhật qua `Documents` như mọi edit khác. Selection được đưa qua edit
   như Replace All của thanh tìm.

Untitled document không bao giờ nằm trong kết quả tìm (search chỉ đi trên đĩa), nên không cần xử
lý.

**Lý do bỏ qua** (chữ hiện trong app):

| Lý do | Chữ |
|---|---|
| Đổi trên đĩa giữa lúc đọc và lúc ghi | `changed on disk` |
| Document đang conflict với đĩa | `has a conflict with the file on disk` |
| Decode lossy | `was not decoded cleanly` |
| Binary, quá lớn, hết quyền, lỗi I/O | `could not be read` hoặc `could not be written: {error}` |
| Ký tự không encode được | `cannot encode the replacement in {encoding}` |
| Document đổi hai lần trong lúc thay | `changed while replacing` |
| Dòng được giao không còn khớp với file | `changed since the search` |
| File không còn | `no longer exists` |

**Tiến độ và hủy:**

- Job gửi tiến độ (số file đã xong trên tổng số) qua channel. Bộ đếm của tab ghi
  `Replacing… {done}/{total} files`.
- Đóng tab Search không dừng lần thay đang chạy (thay xong vẫn an toàn hơn thay dở). Đóng window
  thì hủy job. File đã ghi xong thì giữ nguyên; file đang ghi dở thì nhờ ghi atomic nên không bị
  hỏng.
- Không có nút hủy riêng (VS Code cũng không có).

**Watcher:** file được job ghi sẽ phát sự kiện worktree như mọi thay đổi trên đĩa. Document của
file đó (nếu vừa được mở trong lúc thay) sẽ reload như bình thường.

### 5. Hiệu năng

- Luồng UI chỉ làm: hỏi xác nhận, gom danh sách file, lấy snapshot của các document đang mở, áp
  dụng edit đã tính sẵn, và cập nhật tiến độ theo lô (không phải theo từng file).
- Áp dụng edit cho một document lớn (ví dụ 500k match) vẫn tốn trên luồng UI. Đó là cùng chi phí
  với Replace All của thanh tìm, và là follow-up đã biết. Spec này không giải quyết, nhưng các
  document được áp dụng lần lượt qua nhiều lượt cập nhật, không gộp tất cả vào một frame.
- Có benchmark hoặc test thời gian: Replace All trên 2.000 file đóng, mỗi file 10 match, chạy xong
  và luồng UI không bị chặn quá một frame vì job.

### 6. Test

- **`lucerna-text`:** luật whole word mức ký tự (chuyển từ `lucerna-editor`, giữ nguyên test cũ).
- **`lucerna-editor`:** test `is_whole_word` hiện có vẫn qua.
- **`lucerna-project`:**
  - Search whole word theo `wordSeparators` (ví dụ `foo` không khớp trong `foo-bar` khi `-` là
    separator… và ngược lại khi không phải), kể cả dòng mà mọi match đều bị lọc.
  - Job thay trên thư mục tạm:
    - file UTF-8, UTF-16 có BOM, Windows-1252, CRLF: giữ encoding, BOM và line ending;
    - file lossy hoặc binary thì bỏ qua;
    - file bị sửa giữa lúc đọc và lúc ghi (dùng một hook test) thì không bị ghi đè;
    - chỉ các dòng được giao bị thay; match mới ở dòng khác không bị thay;
    - dòng không còn khớp thì cả file bị bỏ qua;
    - match bắt đầu sau 1.000 ký tự không bị thay;
    - match vượt dòng và match rỗng không được thay;
    - `$1`, Preserve Case;
    - hủy giữa chừng;
    - chữ tiếng Việt trong match và trong chuỗi thay.
- **`lucerna-workbench`** (gpui test, theo kiểu test của tab Search hiện có):
  - ⇧⌘H mở tab với hàng Replace, focus đúng ô; nút Toggle Replace; Esc trong ô Replace.
  - Xem trước: dòng có chữ gạch ngang và chữ mới (`$1` đã khai triển); diff view không bị ảnh hưởng.
  - Replace All: hộp thoại có đúng câu chữ (số ít/số nhiều, chuỗi rỗng, cắt 10 dòng, dòng báo kết
    quả bị cắt); Cancel không đổi gì; đồng ý thì file trên đĩa đã đổi, kết quả bị xóa và thông báo
    đúng.
  - File đang mở và sạch: đổi, đã lưu, một lần undo trả lại chữ cũ.
  - File đang mở và có sửa chưa lưu: đổi trong buffer, vẫn dirty, trên đĩa không đổi.
  - File đang conflict: không đổi, có trong danh sách bỏ qua.
  - Replace theo dòng, theo file, Dismiss; ⇧⌘1, ⌘⌫, ⌥⌘Enter qua keymap; selection chuyển sang dòng
    kế.
  - Replace theo dòng khi file đã đổi thì không thay và hiện thông báo.
  - Phím của thanh tìm (⇧⌘1, ⌥⌘Enter, ⌥⌘P) trong editor thường vẫn chạy lệnh của thanh tìm.
- **Tài liệu:**
  - `docs/keyboard-shortcuts.md` và `CHANGELOG.md`: chỉ thêm dòng.
  - `docs/roadmap.md`: đánh dấu mục này đã xong.

## Còn để người làm quyết

- Màu theme cho phần xem trước.
- Dòng mà mọi match đều bị luật whole word lọc: thành dòng ngữ cảnh hay bị bỏ.
- Cấu trúc bên trong job (thread pool, kích thước lô tiến độ), miễn là luồng UI không chờ.
- Tên hàm và kiểu dữ liệu bên trong module mới.

## Hạn chế đã biết

- Match nằm vắt qua mốc 1.000 ký tự đầu của dòng (bắt đầu trong phần hiện, kết thúc sau đó): phần
  xem trước không hiện chữ mới cho match đó, nhưng job vẫn thay nó. Vì vậy số trong hộp hỏi có thể
  khác số đã thay trong thông báo kết quả.
