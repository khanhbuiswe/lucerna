# Reopen Closed Editor và Close Others

Ngày: 2026-11-18. Trạng thái: bản nháp, chờ duyệt.

## Mục đích

Người chuyển từ VS Code sang có hai thói quen với tab mà Lucerna chưa có:

1. Lỡ tay ⌘W thì bấm ⇧⌘T để mở lại tab vừa đóng, đúng chỗ cũ, con trỏ và cuộn như trước. Bấm
   tiếp thì lùi dần về các tab đóng trước đó.
2. Dọn tab hàng loạt: Close Others (⌥⌘T), đóng các tab bên phải/trái, đóng các tab đã lưu, đóng
   cả nhóm. Họ làm bằng phím hoặc bằng chuột phải lên tab.

Đây là mục tier 1 của `docs/roadmap.md`: những gì chặn người mới từ VS Code. Lệnh, phím và hành vi
theo VS Code; giao diện menu theo Zed.

## Mục tiêu

- `workbench.action.reopenClosedEditor` (⇧⌘T): mở lại lượt đóng gần nhất, vào nhóm cũ, đúng vị
  trí, với con trỏ và cuộn. Bấm lại thì lùi tiếp. Lịch sử nhớ 20 tab.
- Đóng hàng loạt, mỗi lệnh nhắm vào một tab đích (tab đang active, hoặc tab bị chuột phải):

  | Lệnh | Tiêu đề trong Command Palette | Phím (mac) |
  |---|---|---|
  | `workbench.action.closeOtherEditors` | View: Close Other Editors in Group | ⌥⌘T |
  | `workbench.action.closeEditorsToTheRight` | View: Close Editors to the Right in Group | — |
  | `workbench.action.closeEditorsToTheLeft` | View: Close Editors to the Left in Group | — |
  | `workbench.action.closeUnmodifiedEditors` | View: Close Saved Editors in Group | ⌘K U |
  | `workbench.action.closeEditorsInGroup` | View: Close All Editors in Group | ⌘K W |
  | `workbench.action.closeAllEditors` (đã có) | View: Close All Editors | ⌘K ⌘W (thêm phím) |
  | `workbench.action.reopenClosedEditor` | View: Reopen Closed Editor | ⇧⌘T |

- Menu chuột phải trên tab, chỉ có nhóm lệnh đóng (thứ tự và cách chia nhóm theo Zed):

  ```
  Close
  Close Others
  ─────────────
  Close to the Left
  Close to the Right
  ─────────────
  Close Saved
  Close All
  ```

  Menu nhắm vào tab bị chuột phải. Mục không có tác dụng thì làm mờ: Close Others khi nhóm chỉ có
  1 tab; Close to the Left trên tab đầu; Close to the Right trên tab cuối. "Close All" ở đây là
  Close All Editors **in Group** (`closeEditorsInGroup`), giống VS Code và Zed.
- Đóng nhiều tab có document chưa lưu: hỏi **một hộp chung** (luồng `request_close_all` hiện có).

## Không làm (lần này)

- Các mục khác của menu tab: Copy Path, Copy Relative Path, Reveal in Explorer, Pin/Keep Open, Split.
- Tab sticky (pin kiểu VS Code). Lucerna chỉ có tab preview và tab thường, nên lệnh đóng hàng loạt
  không có tab nào phải chừa ra.
- Lưu lịch sử tab đã đóng qua lần mở app sau.
- Mở lại một tab Untitled đã bỏ (xem bên dưới).
- Context key `canReopenClosedEditor`, chọn nhiều tab rồi đóng một lần.
- Khôi phục nhóm đã bị xóa: sau Close All ở 2 nhóm, ⇧⌘T mở tất cả vào nhóm đang active.

## Không được xảy ra

- Một đường đóng tab nào đó (⌘W, ×, chuột giữa, `:q`, `:qa`, Close All, nút Close của thanh file bị
  xóa, các lệnh mới) không vào lịch sử, khiến ⇧⌘T "không làm gì" một cách khó hiểu.
- Đóng hàng loạt mà mất thay đổi chưa lưu không hỏi, hoặc hỏi một hộp riêng cho từng file.
- Bấm Cancel ở hộp hỏi mà vẫn có tab bị đóng.
- ⇧⌘T mở một tab rỗng cho file đã bị xóa, hoặc mở thêm một tab trùng với tab đang có.
- UI bị chặn: kiểm tra file còn tồn tại chỉ là một `stat` mỗi mục khi bấm phím, như Go Back.

## Tham khảo

Đọc từ mã nguồn bản main/master, tháng 11/2026.

### VS Code

- **Reopen.** `ReopenClosedEditorAction` (`browser/parts/editor/editorActions.ts`), ⇧⌘T, gọi
  `HistoryService.reopenLastClosedEditor()` (`services/history/browser/historyService.ts`).
  - `MAX_RECENTLY_CLOSED_EDITORS = 20`. Lịch sử chung cả cửa sổ; quá 20 thì bỏ mục cũ nhất.
  - Mỗi mục (`IRecentlyClosedEditor`) giữ: dạng untyped của editor, resource, `index`, `sticky`,
    `batchId`. **Không** giữ nhóm và cờ preview: luôn mở lại vào nhóm active, ở dạng ghim, đúng
    `index` cũ.
  - **Batch** (mới trên main): các lần đóng trong cùng một lượt đồng bộ có chung `batchId`; một lần
    ⇧⌘T mở lại cả loạt (sau Close Others hay Close All).
  - Không ghi lần đóng `REPLACE` (preview bị preview khác thay) và `MOVE`. Đóng lại cùng editor thì
    mục cũ bị xóa trước, không trùng.
  - Untitled được ghi nhưng nội dung không giữ: mở lại ra file rỗng.
  - Nhóm active đã chứa editor đó thì không mở thêm. Không mở được mục nào của một batch thì lùi
    sang batch trước.
  - File bị xóa (watcher hoặc thao tác xóa) thì bị gỡ khỏi lịch sử; file đổi tên thì mục được sửa
    theo.
  - Con trỏ và cuộn không nằm trong mục mà trong `EditorMemento` theo nhóm, khóa theo resource
    (100 mục), bật bằng `workbench.editor.restoreViewState`.
- **Lệnh đóng.** `closeOtherEditors` (⌥⌘T, chỉ mac), `closeEditorsToTheRight`/`Left` (không phím),
  `closeUnmodifiedEditors` "Close Saved Editors in Group" (⌘K U), `closeEditorsInGroup` (⌘K W),
  `closeAllEditors` (⌘K ⌘W). Tất cả chừa tab sticky. Từ menu tab thì nhắm vào tab bị chuột phải,
  không thì tab active. Close Others và Close to the Right ghim tab giữ lại.
  - Tab chưa lưu: các lệnh theo nhóm hỏi **từng file một**, Cancel đầu tiên dừng cả loạt; chỉ
    Close All Editors hỏi một hộp chung.
  - Menu tab: Close · Close Others · Close to the Right · Close Saved · Close All │ Copy Path · Copy
    Relative Path │ Reopen Editor With… │ Reveal… │ Keep Open · Pin │ Split… Không có Close to the
    Left.

### Zed

- `pane::ReopenClosedItem` (cmd-shift-t): `closed_stack` trong `NavHistory` của từng pane,
  `MAX_NAVIGATION_HISTORY_LEN = 1024`, không lưu qua lần mở sau. Mục giữ item, đường dẫn, dữ liệu
  điều hướng (anchor con trỏ, vị trí cuộn), cờ preview. **Tab preview đóng thì không ghi.**
  - Mở lại vào pane active, **cạnh tab active** (không theo vị trí cũ), mỗi lần một tab. Item còn mở
    thì activate nó; mở lỗi thì thử mục kế. Untitled không mở lại được. File bị xóa bị gỡ khỏi
    lịch sử.
- Lệnh đóng: `CloseOtherItems` (alt-cmd-t; `CloseInactiveItems` giờ là alias cũ),
  `CloseItemsToTheLeft` (cmd-k e), `CloseItemsToTheRight` (cmd-k t), `CloseCleanItems` (cmd-k u),
  `CloseAllItems` (cmd-k w). Mặc định chừa tab ghim.
  - Nhiều tab chưa lưu: **một hộp chung** "Do you want to save changes to the following files?",
    Save all / Discard all / Cancel; một tab thì hộp thường.
  - Menu tab: Close · Close Others │ Close Left · Close Right │ Close Clean · Close All │ Pin Tab │ …
    │ Copy Path · Copy Relative Path │ Reveal… Mục không tác dụng thì làm mờ.

### Helix

`:buffer-close-others` (`:bco`), `:buffer-close-all` (`:bca`), có bản `!`. Đóng các buffer sạch, rồi
báo lỗi "N unsaved buffers remaining" chứ không hỏi. Không có pin, không có mở lại buffer đã đóng.

### Lucerna chọn gì, và vì sao

| Điểm | Chọn | Lý do |
|---|---|---|
| Lệnh, phím, tiêu đề | VS Code | quy tắc §1 của design-principles |
| Số mục | 20 (VS Code) | đủ cho "lỡ tay"; VS Code là thói quen của người dùng |
| Batch | mở lại cả loạt (VS Code main) | Close Others lỡ tay thì một phím là xong |
| Nhóm đích | **nhóm cũ nếu còn**, không thì nhóm active | lệch VS Code: brief yêu cầu "in its group"; với nhiều nhóm, mở lại vào nhóm khác làm người dùng lạc. Khi chỉ có một nhóm thì giống hệt VS Code |
| Vị trí | `index` cũ (VS Code) | Zed đặt cạnh tab active, kém chính xác hơn |
| Preview hay ghim | luôn ghim (VS Code) | |
| Preview bị thay chỗ | không ghi (cả hai) | |
| Con trỏ và cuộn | giữ trong mục (cách Zed) | Lucerna chưa có memento theo file; giữ trong mục là đủ cho tính năng này |
| Tab chưa lưu khi đóng nhiều | một hộp chung (Zed, và Close All của VS Code) | luồng Lucerna đã có; hỏi từng file một rất phiền |
| Menu tab | theo Zed, chỉ nhóm đóng, có Close to the Left | giao diện theo Zed; người dùng chọn chỉ nhóm đóng |

## Thiết kế

### Lịch sử trong model (`editor_groups/model.rs`)

`EditorGroups` có thêm danh sách tab đã đóng, cũ nhất trước, tối đa 20 mục:

```rust
pub struct ClosedTab {
    pub item: TabItem,
    pub group: GroupId,       // nhóm lúc đóng
    pub index: usize,         // vị trí trong nhóm trước lượt đóng (trước cả batch)
    pub was_active: bool,     // là tab active của nhóm lúc đóng
    pub batch: u64,           // các tab đóng trong một lần gọi có chung batch
    pub tab: TabId,           // id cũ, để view gắn con trỏ và cuộn
    pub view: Option<TabViewState>,
}
pub struct TabViewState { pub cursor: Point, pub scroll_top_row: u32 }
```

- `close(id)` vẫn như cũ và ghi một mục (batch riêng). Thêm `close_many(ids)`: đóng từng tab theo
  thứ tự, mọi mục chung một batch. `index` lấy từ vị trí **trước** khi batch bắt đầu đóng. Ví dụ
  Close Others trên C trong [A,B,C,D] ghi A=0, B=1, D=3, nên mở lại theo `index` tăng dần sẽ đưa
  từng tab về đúng chỗ cũ. Mọi lệnh đóng hàng loạt, cả `request_close_all`, đi qua đây.
- **Chỉ ghi tab có file:** `TabItem::File` và `TabItem::MarkdownPreview` (bản tĩnh). Các tab khác
  (Search, References, log và Health của server ngôn ngữ, Extensions, Uncommitted Changes, diff
  một file, Changes since, Get Started, preview động) không vào lịch sử. Chúng phụ thuộc trạng thái
  bên ngoài (kết quả tìm, server đang chạy, file đang active), nên mở lại dễ ra thứ cũ hoặc vô
  nghĩa, và đều mở lại được bằng lệnh riêng của chúng. Đây là lựa chọn của mình; VS Code ghi gần
  như mọi editor.
- **Chỉ ghi lần đóng do người dùng:** `close` và `close_many` ghi lịch sử. Thêm `remove(id)` đóng tab
  mà không ghi, dùng cho các lần code tự đóng tab:
  - `Workbench::close_file_tabs` (Save As ghi đè một file đang mở);
  - `Workbench::paths_deleted` (xóa file trong Explorer);
  - `finish_onboarding`;
  - lần `replace_item` tự đóng tab khi đã có tab cùng item.

  Nút Close trên thanh báo file đã bị xóa là thao tác của người dùng nên vẫn ghi. File không còn
  thì ⇧⌘T sẽ bỏ qua mục đó.
- **Không ghi:**
  - tab preview bị `open_file` thay chỗ;
  - tab bị `replace_item` thay (đổi qua lại source/preview Markdown);
  - tab Untitled (`TabItem::File` hay `MarkdownPreview` của đường `untitled:`): document mất theo
    tab, mở lại chỉ ra file rỗng. Lệch nhỏ so với VS Code, ghi rõ ở đây.
- Ghi một item đã có trong lịch sử với cùng nhóm thì xóa mục cũ trước, nên không có mục trùng.
  Quá 20 thì bỏ mục cũ nhất. Nếu một batch lớn hơn 20 (Close All với 30 tab) thì chỉ giữ 20 tab
  cuối của batch.
- `retarget(from, to)` (đổi tên, di chuyển file) cũng sửa đường dẫn trong lịch sử.
- `reopen_closed(exists: impl Fn(&Path) -> bool) -> Vec<(TabId, Option<TabViewState>)>`:
  1. Lấy batch mới nhất ra khỏi lịch sử. Bỏ những mục có `exists` trả `false` và những mục mà
     nhóm đích đã có tab cùng item.
  2. Không còn mục nào để mở:
     - nếu có mục bị bỏ vì tab của nó **đang mở lại** trong nhóm đích, activate tab đó rồi dừng
       (mục đã dùng hết, không lùi sang batch cũ hơn), như VS Code và Zed;
     - chỉ khi mọi mục bị bỏ vì **file không còn**, mới lùi sang batch trước;
     - hết lịch sử thì trả rỗng, không làm gì.
  3. Mỗi mục mở vào nhóm có `GroupId` cũ nếu còn, không thì nhóm active, ở vị trí
     `min(index, số tab)`, dạng ghim. Trong một nhóm, mở theo `index` tăng dần để các tab về
     đúng chỗ.
  4. Tab active sau đó: batch một tab thì tab đó được activate, nhóm của nó thành nhóm active.
     Batch nhiều tab thì những tab `was_active` được activate trong nhóm của chúng. Nhóm active
     là nhóm của tab `was_active` cuối cùng; nếu không có tab nào như vậy (Close Others), tab và
     nhóm active giữ nguyên.
  5. Trả về id mới của các tab đã mở cùng con trỏ và cuộn của chúng.
- `set_closed_view(tab: TabId, view: TabViewState)`: gắn con trỏ và cuộn vào mục có id cũ `tab`.

### Con trỏ và cuộn (`editor_groups/view.rs`)

`EditorGroupsView::update_model` chụp lại các mục mới vào lịch sử sau khi `f` chạy. Lúc này body
của tab vẫn còn trong `bodies` (chỉ bị dọn ở lần vẽ sau). Nếu body là `CodeView` thì đọc `cursor()`
và `scroll_top_row()` rồi gọi `set_closed_view`. Cách này bắt được mọi đường đóng mà không phải
sửa từng chỗ gọi.

Khi mở lại, lệnh `reopenClosedEditor` (ở `Workbench`) áp con trỏ và cuộn giống `navigate`: tab đã
có view thì `go_to_and_scroll`, chưa có thì `go_to_and_scroll_when_opened(path, …)`. Batch nhiều
tab mà một file mở ở hai tab cùng lúc thì chỉ đảm bảo tab active; đây là việc cho người làm.

### Đóng hàng loạt và hộp hỏi (`editor_groups/view.rs`)

Gộp `request_close` và `request_close_all` thành một `request_close_tabs(ids: Vec<TabId>, …)`:

- Document **cần hỏi** là document chưa lưu của một tab trong `ids` mà **không** tab nào ngoài
  `ids` đang hiện (tổng quát hóa `last_tab_of_dirty`).
- Không có document nào cần hỏi: `close_many(ids)` ngay.
- Có: `save::prompt_unsaved_all(names)`. Một file thì đây là hộp Save / Don't Save / Cancel như
  hôm nay; nhiều file thì Save All / Don't Save / Cancel.
  - Save / Save All: `save_each` rồi chỉ đóng khi mọi document đó vẫn sạch. Giữ nguyên quy tắc
    hiện có: gõ thêm trong lúc lưu thì không đóng tab nào.
  - Don't Save: `close_many(ids)` rồi `discard` từng document.
  - Cancel: không đóng gì.
- `request_close(id)` và `request_close_all` thành vỏ mỏng gọi hàm này. Mọi test hiện có
  (`save.rs`, `untitled.rs`, `hot_exit.rs`) phải vẫn qua.

Tập tab cho từng lệnh, trong nhóm của tab đích `t`:

| Lệnh | Đóng | Ghi chú |
|---|---|---|
| Close Others | mọi tab trừ `t` | ghim `t`, activate nó |
| Close to the Right | các tab sau `t` | ghim `t`; nếu tab active bị đóng thì activate `t` |
| Close to the Left | các tab trước `t` | như trên (VS Code không ghim ở đây; Lucerna làm giống Right cho nhất quán, đây là lựa chọn của mình) |
| Close Saved | mọi tab của nhóm mà document không dirty; tab không phải file coi là đã lưu | không bao giờ hỏi |
| Close All in Group | mọi tab của nhóm | nhóm rỗng bị xóa nếu còn nhóm khác, như `close` hôm nay |

Tab và nhóm active sau lệnh:
- Close Others, Left và Right: `t` thành tab active, nhóm của nó thành nhóm active, kể cả khi lệnh
  chạy từ menu trên một nhóm không active. Phím gõ tiếp sẽ vào `t`, giống bấm vào tab đó.
- Close Saved và Close All in Group: tab active của nhóm là tab còn lại được dùng gần nhất. Nhóm
  active giữ nguyên, trừ khi nó bị xóa (khi đó theo quy tắc của `close`).

Close Others, Left và Right ghim `t` trước khi hỏi. Nếu người dùng bấm Cancel thì `t` vẫn đã ghim,
và mình coi chuyện đó là vô hại.

### Tab đích và đối số lệnh

Các lệnh nhận đối số tùy chọn `{"tab": <số>}` (id của tab, `TabId::as_u64`). Không có đối số thì
tab đích là tab active của nhóm active; không có tab active thì lệnh không làm gì. Có đối số mà id
không còn (tab đã đóng) thì lệnh không làm gì, để không đóng nhầm tab quanh một tab khác. Menu chuột phải truyền id của tab bị bấm. `closeActiveEditor` cũng nhận đối số này để mục
"Close" của menu đóng đúng tab bị chuột phải.

### Menu chuột phải trên tab

- `EditorGroupsView` nghe `on_mouse_down(MouseButton::Right)` trên mỗi tab và dựng `ContextMenu`
  (`context_menu.rs`) với các mục ở phần Mục tiêu, có `disabled_when` cho mục không tác dụng.
- Giữ menu bằng `DeployedMenu` trong view và vẽ bằng `render_deployed`, giống Explorer.
- Mục được chọn thì chạy lệnh qua `Workbench::run_command(command, args)` với `{"tab": id}`, sau khi
  lượt cập nhật hiện tại xong (`window.defer`), như Explorer.
- Menu hiện phím tắt của từng mục (`shortcut_label`, đã có sẵn trong component).
- Chuột phải không đổi tab active (VS Code và Zed cũng vậy).

### Phím, menu app, tài liệu

- `default-keybindings.json`, thêm một khối ở cuối:
  - `shift+cmd+t` → reopenClosedEditor
  - `alt+cmd+t` → closeOtherEditors
  - `cmd+k u` → closeUnmodifiedEditors
  - `cmd+k w` → closeEditorsInGroup
  - `cmd+k cmd+w` → closeAllEditors

  Không cái nào đụng phím đang có (`cmd+k cmd+u` là removeCommentLine, khác `cmd+k u`).
- `app_menu.rs`: thêm "Reopen Closed Editor" vào menu File, ngay dưới "Close Editor" (lựa chọn của
  mình; VS Code đặt nó trong Open Recent).
- Cập nhật `docs/keyboard-shortcuts.md`, `CHANGELOG.md` (Unreleased → Added) và đánh dấu mục này
  trong `docs/roadmap.md`. Chỉ thêm dòng, không sửa dòng có sẵn, để dễ merge với nhánh Replace in
  project.

## Kiểm thử

- **Unit test model** (`tests/editor_groups_model.rs`):
  - ghi lịch sử, giới hạn 20, không trùng;
  - không ghi preview bị thay, `replace_item`, Untitled, tab không phải file, `remove`;
  - batch: `close_many` tạo một batch, mở lại cả batch;
  - đúng nhóm, đúng `index`, tab active sau khi mở lại;
  - nhóm đã mất thì mở vào nhóm active;
  - bỏ mục có file không còn và lùi batch; mục đã mở lại thì activate tab đó, không lùi batch;
  - `retarget` sửa lịch sử.
- **Test GPUI** (`tests/editor_groups_view.rs` hoặc file mới `tests/tab_close_commands.rs`):
  - ⇧⌘T sau ⌘W khôi phục tab, con trỏ và cuộn;
  - từng lệnh đóng hàng loạt, cả bản có đối số `tab`;
  - hộp hỏi chung khi có nhiều file dirty: Save All, Don't Save, Cancel;
  - document đang hiện ở nhóm khác thì không hỏi;
  - menu chuột phải hiện đúng mục, mục mờ đúng chỗ, chọn mục thì chạy lệnh với tab bị chuột phải;
  - các lệnh mới có trong registry và có phím.
- Toàn bộ test `lucerna-workbench`, clippy, fmt và `cargo deny check` phải sạch.

## Để lại cho người làm

- Đặt tên hàm và tách file (ví dụ tách lịch sử ra `editor_groups/closed.rs` nếu `model.rs` quá dài).
- Vị trí pixel của menu: tại con trỏ chuột như Explorer.
- Trường hợp một file mở ở hai tab trong cùng batch: chỉ cần tab active có đúng con trỏ.
