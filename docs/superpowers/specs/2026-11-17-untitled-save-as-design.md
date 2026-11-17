# File mới (⌘N) và Save As (⇧⌘S)

Ngày 2026-11-17. Trạng thái: đã triển khai (branch `claude/untitled-save-as`); kiểm tra thủ công §6 chưa làm. Mục thứ hai của tầng 1 trong `docs/roadmap.md`.

## 1. Mục đích

Người chuyển từ VS Code quen bấm ⌘N để mở một tab trống, gõ code, rồi ⌘S để đặt tên và lưu vào
project. Họ cũng quen ⇧⌘S để lưu file đang mở ra tên khác. Lucerna chưa có cả hai; muốn tạo file
mới phải dùng Explorer → New File (đặt tên trước khi gõ).

Người dùng xác nhận ⌘N dùng để **tạo file mới sẽ lưu vào project**, không chủ yếu làm nháp.

**Mục tiêu:**

- ⌘N mở tab Untitled; ⌘S trên tab đó hỏi chỗ lưu rồi biến tab thành file thật, giữ undo và con trỏ.
- ⇧⌘S lưu file bất kỳ ra đường dẫn khác; tab chuyển sang file mới.
- Không mất chữ: Untitled chưa lưu đi qua hot exit như file thường.

**Ngoài phạm vi:** đoán ngôn ngữ tự động (`workbench.editor.languageDetection`); dòng gợi ý "Select a
language…" trên tab trống (`workbench.editor.empty.hint`); `:w tên` trong Vim; Save As tới nơi không
phải ổ đĩa; mở Untitled từ CLI (`luc -n`); `workbench.editor.untitled.labelFormat` (luôn là
`content`); Untitled mang sẵn đường dẫn đích (`associatedResource` của VS Code); Save Copy As.

**Anti-goal:** Untitled bị ghi ra đĩa thành file tên `untitled:Untitled-1`, bị đưa cho git, LSP,
watcher; hay mất chữ trong Untitled khi thoát app mà không hỏi.

## 2. Tham khảo

- **VS Code** (`src/vs/workbench/services/untitled/common/untitledTextEditorService.ts`): mọi tài
  liệu định danh bằng URI. Untitled là `untitled:Untitled-N`, `N` là số nhỏ nhất từ 1 chưa dùng
  (`do { … counter++ } while (mapResourceToModel.has(...))`). File service chỉ đọc/ghi scheme nó biết,
  nên Untitled không thể vô tình ghi ra đĩa. Tên tab mặc định lấy dòng đầu
  (`workbench.editor.untitled.labelFormat: "content"`). ⌘N = `workbench.action.files.newUntitledFile`,
  ⇧⌘S = `workbench.action.files.saveAs`. Lưu Untitled mở hộp thoại Save As; Save All cũng hỏi từng
  Untitled. Autosave bỏ qua Untitled. Hot exit sao lưu Untitled. `files.defaultLanguage` đặt ngôn ngữ
  cho Untitled mới.
- **Zed** (`crates/language/src/buffer.rs`): `Buffer.file: Option<Arc<dyn File>>`, Untitled là
  `None` ("`None` when there is no path"); buffer định danh bằng id riêng. Save As gán file qua
  `file_updated`. Tên tab cũng lấy dòng đầu. ⌘N = `workspace::NewFile`.
- **Helix:** `Document.path: Option<PathBuf>`, định danh bằng `DocumentId`; `:new` tạo tài liệu
  `path = None`, `:w <path>` gán đường dẫn.

## 3. Các cách đã cân nhắc

| Cách | Đánh đổi | Kết quả |
|---|---|---|
| **A. Đường dẫn giả** `untitled:Untitled-N` (gần VS Code) | Nhỏ: `Documents`, tab, hot exit vốn khóa bằng đường dẫn nên phần lớn chạy nguyên. Rủi ro: chỗ nào chạm đĩa mà quên kiểm tra sẽ coi nó là file thật. Chặn bằng một hàm kiểm tra duy nhất và test cho từng chỗ | **Chọn** (người dùng) |
| B. Kiểu riêng (path hoặc Untitled N), như Zed / Helix | An toàn nhất, compiler bắt mọi chỗ. Nhưng phải refactor `documents.rs` (~3000 dòng), tab, hot exit, mọi hàm nhận `&Path` | Bỏ: tốn gấp nhiều lần mà hành vi như nhau |
| C. Không có Untitled: ⌘N hỏi tên ngay rồi mở file thật | Gần như miễn phí, nhưng lệch VS Code và bắt nghĩ tên trước khi gõ | Bỏ |

## 4. Thiết kế

### 4.1 Định danh Untitled

- Untitled có đường dẫn giả `untitled:Untitled-N` (một `Arc<Path>` tương đối, không bao giờ
  ghép với thư mục gốc). `N` là số nhỏ nhất từ 1 chưa dùng **trong cửa sổ đó** (tab đã đóng trả số
  lại).
- Một hàm duy nhất `is_untitled(path: &Path) -> bool` (đường dẫn bắt đầu bằng `untitled:`). Mọi chỗ
  chạm đĩa hay dịch vụ ngoài kiểm tra nó và bỏ qua Untitled:
  - lưu (luôn đi đường Save As, §4.3), revert, autosave;
  - watcher và xử lý thay đổi trên đĩa / xung đột;
  - git (gutter, màu Explorer, diff);
  - LSP (không mở, không gửi `didOpen`);
  - Explorer: auto-reveal, đánh dấu file đã sửa;
  - lệnh lấy đường dẫn: Copy Path, Copy Relative Path, Reveal in Finder, Open Containing Folder —
    với Untitled thì không làm gì (menu chuột phải của tab, nếu có, hiện mờ);
  - Markdown preview, trạng thái workspace (Untitled không ghi vào danh sách tab mở lại; hot exit
    lo việc đó, §4.5), danh sách Recent.
- Một file thật có tên bắt đầu bằng `untitled:` (hợp lệ trên macOS) phải luôn có đường dẫn tuyệt
  đối, nên không trùng: `is_untitled` chỉ đúng với đường dẫn tương đối một thành phần.

### 4.2 ⌘N — New File

- Lệnh `workbench.action.files.newUntitledFile`, tiêu đề "File: New Untitled Text File", phím
  `cmd+n`, menu File → "New File" (đầu menu, trên "New Window"). Chạy trong cửa sổ thư mục và cửa
  sổ một file. **Không chạy ở Welcome và khi không có cửa sổ nào** (người dùng chọn): mục menu mờ,
  phím không làm gì. Lý do: cửa sổ Welcome chưa có `Documents`, nhóm editor, hot exit; hỗ trợ nó cần
  một kiểu cửa sổ mới không thư mục (và `WorkspaceKey` mới), gần gấp đôi việc, trong khi mục đích của
  ⌘N là tạo file cho project đang mở.
- Mở tab Untitled ghim (không phải preview) trong nhóm editor đang active, focus vào nó.
- Ngôn ngữ: `files.defaultLanguage` nếu đặt (id ngôn ngữ, ví dụ `"rust"`), không thì Plain Text.
  Thêm setting này vào `default-settings.json` với giá trị `""`.
- Tên tab: dòng đầu không rỗng của nội dung, cắt khoảng trắng hai đầu, tối đa 40 ký tự (thêm `…` khi
  cắt); nội dung rỗng thì `Untitled-N`. Tooltip / path bar hiện `Untitled-N`.
- Icon tab theo ngôn ngữ hiện tại (Plain Text → icon file chữ).

### 4.3 Lưu Untitled

- `workbench.action.files.save` (⌘S), `:w` trong Vim, và Save trong hộp thoại đóng tab / thoát đều
  đi đường Save As khi tab là Untitled.
- Hộp thoại lưu native (`cx.prompt_for_new_path`):
  - thư mục bắt đầu: thư mục gốc của project; cửa sổ một file thì thư mục chứa file đó;
  - tên gợi ý: tên tab (§4.2) bỏ ký tự `/ \ : * ? " < > |` và ký tự điều khiển, cắt khoảng trắng;
    rỗng thì `Untitled-N`. Không tự thêm đuôi.
- Bấm Cancel: không có gì đổi, tab vẫn dirty. Nếu đang trong luồng đóng tab / thoát thì luồng đó dừng
  như khi bấm Cancel ở hộp thoại Save / Don't Save / Cancel.
- Lưu được: ghi file theo luồng lưu sẵn có (`files.trimTrailingWhitespace`, `insertFinalNewline`,
  `files.eol`… như file thường; encoding UTF-8 không BOM). Sau đó **chính tab đó** thành file thật:
  `Documents::rename(untitled, đích)` giữ undo, con trỏ, cuộn; ngôn ngữ, LSP, git, watcher chạy như
  file vừa mở. Số `N` được trả lại.
- Untitled đang hiện ở nhiều nhóm editor (chia màn hình): mọi view cùng thành file thật (một tài
  liệu, `rename` chuyển mọi view).
- Lưu ra ngoài thư mục gốc, hoặc trong cửa sổ một file: giống hệt khi mở một file ngoài thư mục hôm
  nay (cửa sổ giữ nguyên kiểu, không thêm gì riêng cho trường hợp này). Quyết định của người viết
  spec.
- Đích trùng một file đang mở ở tab khác: tài liệu cũ của đường dẫn đó bị thay bằng nội dung vừa lưu
  (như VS Code: hộp thoại macOS đã hỏi Replace). Tab cũ của đường dẫn đó đóng; tab Untitled giữ lại.
- Lỗi ghi (quyền, ổ đầy): báo như lưu thường hiện nay, tab vẫn Untitled và dirty.
- Format on save: chạy theo ngôn ngữ của **đường dẫn đích** nếu cơ chế format sẵn có làm được dễ;
  không thì bỏ qua cho lần lưu đầu (format chưa có, xem roadmap — ghi nhận khi làm Format).

### 4.4 ⇧⌘S — Save As

- Lệnh `workbench.action.files.saveAs`, tiêu đề "File: Save As...", phím `shift+cmd+s`, menu File →
  "Save As…" (ngay dưới "Save"). Chạy trên tab code đang active; tab khác (Search, diff…) thì không
  làm gì.
- Untitled: như §4.3.
- File thật: hộp thoại bắt đầu ở thư mục của file, tên gợi ý là tên file hiện tại. Lưu nội dung hiện
  tại (kể cả chưa lưu) ra đích. **Mọi tab** đang hiện tài liệu đó (kể cả ở nhóm khác) chuyển sang
  file mới, giữ undo / con trỏ / cuộn — dùng `Documents::rename`. File cũ trên đĩa **không đổi** và
  không còn mở ở đâu. Lệch VS Code có chủ ý (VS Code chỉ chuyển tab đang active, tab khác giữ file
  cũ): làm theo VS Code phải tách một tài liệu thành hai, tốn nhiều cho trường hợp hiếm. Quyết định
  của người viết spec.
- Chọn đúng đường dẫn hiện tại: bằng Save thường.

### 4.5 Untitled khi đóng, thoát và hot exit

- Autosave (`files.autoSave` mọi giá trị) bỏ qua Untitled.
- Đóng tab Untitled (⌘W, nút ×, Close Others…):
  - nội dung rỗng: đóng không hỏi;
  - có nội dung: hỏi Save / Don't Save / Cancel như file dirty (`save.rs`, `CLOSE_ANSWERS`). Save đi
    §4.3.
- Untitled có nội dung luôn tính là dirty; rỗng thì không.
- Thoát / đóng cửa sổ:
  - `files.hotExit` bật (mặc định): không hỏi; Untitled có nội dung được sao lưu như file dirty. Mở
    lại cửa sổ: các Untitled hiện lại **ở cuối nhóm editor** (theo cơ chế mở backup không có tab
    hiện nay, `workbench.rs` ~622–649), đúng `Untitled-N`, đúng nội dung, ngôn ngữ, và vẫn dirty.
    Không giữ đúng vị trí tab cũ: muốn vậy phải ghi Untitled vào danh sách tab mở lại, trái §4.1.
    Quyết định của người viết spec.
  - `files.hotExit: "off"`: hỏi Save All / Don't Save / Cancel như hiện nay; Save All đi §4.3 lần
    lượt từng Untitled, Cancel ở bất kỳ hộp thoại nào dừng việc thoát.
- Save All (`workbench.action.files.saveAll`): lưu file thường như cũ, rồi lần lượt hỏi chỗ lưu cho
  từng Untitled có nội dung; Cancel một cái thì bỏ qua cái đó và đi tiếp.
- Backup của Untitled: cùng định dạng và `BACKUP_VERSION` hiện tại; header ghi đường dẫn giả (đủ để
  nhận ra Untitled), không có mtime / size gốc. Khôi phục không đọc đĩa và không đưa đường dẫn qua
  `Documents::key()` theo kiểu file thường. Quyết định của người viết spec.

### 4.6 Lệnh, phím, menu

| Lệnh | Tiêu đề | Phím | Menu |
|---|---|---|---|
| `workbench.action.files.newUntitledFile` | File: New Untitled Text File | `cmd+n` | File → New File |
| `workbench.action.files.saveAs` | File: Save As... | `shift+cmd+s` | File → Save As… |

Thêm dòng cho hai phím vào bảng `keys.rs` (`table_covers_every_default_binding`). Cập nhật
`docs/keyboard-shortcuts.md`, `docs/features.md`, `docs/settings.md` (`files.defaultLanguage`),
`CHANGELOG.md` (`[Unreleased]` → Added).

### 4.7 Vim

`:w` / `:wq` / `:x` trên Untitled đi §4.3 (`:wq`, `:x` chỉ đóng sau khi lưu xong; Cancel thì không
đóng). `:w tên` ngoài phạm vi.

## 5. Để người làm quyết định

- Đặt `is_untitled` ở đâu (gợi ý: `documents.rs` hoặc module nhỏ riêng) và danh sách chỗ chặn cụ thể
  — §4.1 là danh sách tối thiểu; tìm thêm bằng grep mọi chỗ dùng đường dẫn tài liệu để chạm đĩa.
- `Documents::key()` hiện đổi đường dẫn tương đối theo thư mục gốc: phải để Untitled đi qua nguyên.
- Cách hot exit ghi số `N` và thứ tự tab (dùng lại cơ chế khôi phục tab hiện có nếu được).

## 6. Kiểm tra

- Unit / gpui test cho: đánh số `N` (nhỏ nhất chưa dùng, trả lại khi đóng / lưu); tên tab theo dòng
  đầu; mỗi chỗ chặn ở §4.1 (không có file `untitled:*` nào được tạo trên đĩa, không gọi git / LSP);
  ⌘S trên Untitled với hộp thoại giả (trả đường dẫn / trả Cancel); Save As file thật (file cũ không
  đổi, tab chuyển, undo còn); đóng Untitled rỗng không hỏi, có nội dung thì hỏi; hot exit khôi phục
  Untitled; Save All với Untitled.
- Thủ công: ⌘N → gõ → ⌘S → lưu `foo.rs` trong project → có màu, LSP, Explorer hiện file; ⇧⌘S một file
  → file cũ còn nguyên; thoát với Untitled chưa lưu → mở lại còn nguyên.
