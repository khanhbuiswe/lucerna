# Lucerna — V1: Vim di chuyển

- **Ngày:** 2026-11-10
- **Trạng thái:** Bản nháp, chờ người dùng duyệt
- **Lộ trình:** mục V1 trong §1.3 của `2026-11-08-lucerna-phase1-design.md`
- **Nghiên cứu nền:** `docs/research/2026-11-09-vim-mode.md`. Mục 5 của ghi chú đã đối chiếu với mã nguồn Zed `crates/vim` tại `b5cd310` và VSCodeVim tại `28f0275`.

## 1. Mục tiêu

### 1.1 Kết quả

Người quen Vim đọc code trong Lucerna hoàn toàn bằng bàn phím: di chuyển, tìm, cuộn, đặt mark, chọn rồi copy. Lucerna vẫn là trình xem chỉ đọc. V1 chưa có insert mode và chưa có toán tử sửa.

### 1.2 Tiêu chí thành công

1. Đặt `"vim.enabled": true` thì có hiệu lực ngay, không cần khởi động lại. Status bar hiện `-- NORMAL --` và con trỏ là ô block.
2. Mọi lệnh ở §4 chạy đúng hành vi mô tả. Các ca chuyển thể từ VSCodeVim (§9.1) đều qua.
3. Gõ vào Quick Open, find widget, ô Search và trang Extensions không bao giờ bị vim nuốt phím.
4. Khi vim tắt (mặc định), mọi hành vi của Lucerna giữ nguyên như trước.
5. Các mục tiêu hiệu năng của giai đoạn 1 (§6 spec giai đoạn 1) vẫn đạt. Một phím vim xử lý xong trong dưới 1 ms trên file 100k dòng, trừ `n`/`N`/`*`/`#` lần đầu trên file lớn (xem §8).

### 1.3 Những gì đã quyết định khi brainstorm

| Câu hỏi | Quyết định |
|---|---|
| Kiến trúc | Crate thuần `lucerna-vim`, không dùng gpui, cộng một lớp nối mỏng trong workbench (phương án A) |
| Đọc phím | Vim đọc **ký tự đã gõ** (`key_char`), giống cách VSCodeVim đè lệnh `type`, chứ không khai báo từng phím trong keymap |
| Ô nhập của `/` và `?` | Mở **find widget**, giống Zed. Chỉ thêm một hook nhỏ vào `find_widget.rs` |
| Hành vi | Theo VSCodeVim, cũng là theo Vim. Các chỗ lệch được ghi ở §4 |

### 1.4 Không có trong V1

- Insert mode, replace mode, các toán tử `d`/`c`/`y` kèm motion, text object, `.`, macro, register, `u`/`Ctrl+r` và lệnh `:`. Những thứ này thuộc V2, sau giai đoạn 4.
- Remap phím vim (`vim.normalModeKeyBindings`…).
- Jumplist `Ctrl+o`/`Ctrl+i`.
- Mark viết hoa (mark toàn cục) và lưu mark qua các phiên làm việc.
- `gj`/`gk` để đi theo dòng hiển thị khi bật word wrap.
- Màu status bar theo mode (`vim.statusBarColorControl`).

## 2. Kiến trúc

### 2.1 Crate mới: `lucerna-vim`

Crate này không dùng gpui, và chỉ phụ thuộc `lucerna-text`. Giấy phép MIT OR Apache-2.0.

```
crates/lucerna-vim/src/
  lib.rs       // pub use; Key, Mode, Action, ViewInfo
  state.rs     // Vim: máy trạng thái, phím đang chờ, count, goal column
  motion.rs    // các motion thuần trên BufferSnapshot
  classify.rs  // lớp ký tự word / punctuation / whitespace
  search.rs    // * #, n N trên danh sách match
  marks.rs     // Marks: a–z, mark ', remap khi reload
  visual.rs    // VimSelection, text khi yank
  tests/…      // test dạng chuỗi có đánh dấu con trỏ
```

### 2.2 Interface

```rust
pub enum Key { Char(char), Ctrl(char), Escape, Enter }

pub enum Mode { Normal, Visual, VisualLine, VisualBlock }

/// What the vim layer may read from the view.
pub trait ViewInfo {
    fn snapshot(&self) -> &BufferSnapshot;
    fn cursor(&self) -> Point;
    /// First visible buffer row and how many rows fit.
    fn top_row(&self) -> u32;
    fn visible_rows(&self) -> u32;
    fn is_row_hidden(&self, row: u32) -> bool;  // inside a closed fold
    fn tab_size(&self) -> u32;
    /// The partner bracket of the one at `offset`, syntax-aware when possible.
    fn matching_bracket(&self, offset: usize) -> Option<usize>;
    /// Every match of `query`, sorted byte ranges; `None` while a background
    /// search for a large file is still running. The workbench implements it
    /// with `find_in_snapshot` and a cache keyed by (revision, query).
    fn matches(&self, query: &SearchQuery) -> Option<Arc<[Range<usize>]>>;
}

pub struct SearchQuery {
    pub pattern: String,
    pub is_regex: bool,
    pub case_sensitive: bool,
    pub whole_word: bool,
}

pub struct Settings {
    pub ignorecase: bool,      // vim.ignorecase, default true
    pub wrapscan: bool,        // vim.wrapscan, default true
    pub startofline: bool,     // vim.startofline, default true
    pub scrolloff: u32,        // editor.cursorSurroundingLines, default 0
}

pub enum Action {
    MoveCursor { to: Point, jump: bool }, // jump: record for '' and ``
    Select(VimSelection),                 // Visual / VisualLine / VisualBlock
    ClearSelection,
    ScrollTopRow(u32),
    OpenSearch { backward: bool },
    Highlight { pattern: String, whole_word: bool, case_sensitive: bool },
    Yank(String),
    Fold(FoldAction),                     // Toggle / Close / Open at a row
    ModeChanged(Mode),
}

pub enum Outcome { Handled(Vec<Action>), Unhandled }

impl Vim {
    pub fn feed(&mut self, key: Key, view: &dyn ViewInfo,
                globals: &mut VimGlobals, settings: &Settings) -> Outcome;
    pub fn mode(&self) -> Mode;
    /// The pending keys for the indicator: "5", "g", "2f", "m"...
    pub fn pending(&self) -> &str;
    /// Called when the search started by `/` or `?` is submitted.
    pub fn search_submitted(&mut self, query: SearchQuery, view: &dyn ViewInfo,
                            globals: &mut VimGlobals, settings: &Settings) -> Vec<Action>;
    /// A mouse click or selection made outside vim: back to Normal.
    pub fn reset(&mut self);
}
```

- `Vim` sống theo từng CodeView.
- `VimGlobals` sống theo từng workbench, gồm:
  - `last_find`: ký tự, hướng, kiểu `f` hay `t`;
  - `last_search`: pattern, hướng và các cờ;
  - `scroll`: số dòng `Ctrl+d` đang dùng;
  - mark của từng file, khóa theo `Arc<Path>`.

### 2.3 Lớp nối trong workbench

File mới `crates/lucerna-workbench/src/vim.rs`:

- **`VimLayer`**
  - Giữ `HashMap<EntityId, Vim>` cho từng CodeView. Mục của một view bị xóa khi view đó bị hủy.
  - Giữ `VimGlobals`, cùng `Settings` đọc từ `AppState`. Settings được đọc lại mỗi khi `AppState` đổi.
- **Chỗ gọi:** `Workbench::dispatch_key` gọi `VimLayer::handle_key` **trước** keymap. Lớp vim chỉ nhận phím khi đủ cả các điều kiện sau:
  1. vim đang bật, xem §6;
  2. `editorFocus && !textInputFocus && !inQuickOpen`;
  3. không có `cmd` trong phím, và không đang ở giữa một chord của keymap như `cmd+k …`;
  4. tab đang mở là một `CodeView` ở trạng thái `DocState::Ready` hoặc `Deleted`.
- **Phím được chuyển thành `Key`:**
  - Có ctrl, và `key` là một chữ cái → `Ctrl(c)`.
  - `escape` → `Escape`; `enter` → `Enter`.
  - Có `key_char` gồm đúng một ký tự, và không giữ ctrl hay alt → `Char(c)`. `alt` trên macOS tạo ra ký tự khác, nên phím có alt được bỏ qua để đi xuống keymap. Ví dụ `alt+z` là bật/tắt word wrap.
  - Các phím mũi tên được đổi thành `h`/`j`/`k`/`l`.
  - Mọi phím khác trả về `Unhandled`.
- **Kết quả:**
  - `Handled(actions)`: thực thi các action trên CodeView rồi gọi `stop_propagation`.
  - `Unhandled`: phím đi tiếp vào keymap như cũ.
- **Phím chưa có nghĩa ở normal mode.** Một ký tự in được nhưng không phải lệnh vim, ví dụ `i`, `x`, `d`, vẫn được coi là `Handled`, không làm gì, và xóa phần phím đang chờ. Như vậy không có ký tự nào lọt xuống keymap rồi chạy nhầm lệnh khác. Vim thật cũng làm vậy.
- **Context key cho `when`**, đặt theo tên của VSCodeVim:
  - `vim.active`: true khi vim bật.
  - `vim.mode`: `'Normal'`, `'Visual'`, `'VisualLine'` hoặc `'VisualBlock'`. Giá trị là mode của view đang mở.
- **Đồng bộ với chuột.** Khi selection của CodeView bị đổi bởi chuột hoặc bởi `go_to` (Quick Open, kết quả search, go-to-line), lớp vim gọi `reset()` và con trỏ được kẹp về ký tự cuối dòng. CodeView báo việc này bằng một số đếm `selection_generation`, tăng mỗi lần selection đổi. Lớp vim nhớ số đếm của lần ghi cuối của chính nó, và khi thấy số khác thì gọi `reset()`.

### 2.4 Thay đổi trong CodeView

- **Vẽ con trỏ.** Hiện CodeView chưa vẽ con trỏ.
  - Thêm trường `cursor_shape: Option<CursorShape>`, mặc định `None` để giữ nguyên hành vi cũ. `CursorShape` gồm `Block` và `Bar`.
  - Ở normal mode, vẽ một ô block bằng màu `editor.foreground` có alpha khoảng 0.4, rộng đúng bằng ký tự dưới con trỏ: ký tự CJK hay emoji rộng gấp đôi, tab rộng bằng số cột của nó. Trên dòng rỗng, ô rộng nửa ký tự.
  - Ở visual mode, ô block vẽ đè lên selection.
  - Màu lấy từ theme: `editor.foreground`, hoặc `terminal.ansiBrightBlack` nếu theme có.
- **Selection kiểu visual.**
  - `VimSelection::Char` và `Line` dùng `Selection` có sẵn. Ranh giới inclusive được chuyển sang exclusive ở lớp nối. Với `Line`, selection chạy từ cột 0 dòng đầu tới hết dòng cuối, kể cả ký tự xuống dòng.
  - `VimSelection::Block { rows, start_col, end_col }` tính cột theo **cột hiển thị**, có xét tab. Khi vẽ, mỗi dòng trong khối được đổi thành một khoảng byte. Thêm trường `block_selection: Option<Vec<Range<Point>>>` vào CodeView và vẽ bằng màu `selection`. `copy_selection` ghép các khoảng này bằng `\n`.
- **API cuộn**, phục vụ `zz`/`zt`/`zb`, `Ctrl+d`/`Ctrl+u`, `Ctrl+f`/`Ctrl+b` và giữ con trỏ trong khung nhìn:
  - `set_cursor(p)`: đặt con trỏ mà không căn giữa như `go_to`. Nếu con trỏ ra khỏi khung nhìn, cuộn vừa đủ để nó vào lại, có tính `editor.cursorSurroundingLines`. Hàm này cũng mở fold đang che dòng đích.
  - `scroll_top_row` / `set_scroll_top_row`: đã có.
  - Nếu `visible_rows()` trả về 0 trước lần vẽ đầu, các motion theo màn hình như H/M/L và `Ctrl+d` dùng tạm 30.
- **Fold.** Thêm `fold_at(row, FoldAction)`. Hàm tìm fold nhỏ nhất chứa `row` trong `fold_ranges`, rồi toggle, đóng hoặc mở fold đó.

### 2.5 Thay đổi trong find widget

Session `lucerna-search-ui` đang làm lại `find_widget.rs`, nên phần sửa ở đây giữ ở mức nhỏ nhất và được báo trước cho session đó.

- Thêm `FindWidgetEvent::Submitted` (khi nhấn Enter) và `FindWidgetEvent::Dismissed` (khi nhấn Esc), phát qua `EventEmitter`.
- `open` nhận thêm tham số `vim_backward: Option<bool>`.
  - Khi có giá trị, widget mở ra với ô nhập trống, không lấy sẵn selection.
  - Enter không nhảy tới match tiếp theo trong widget như khi dùng bình thường, mà phát `Submitted` rồi đóng widget và trả focus về CodeView.
- Lớp vim đọc query, các cờ case/word/regex và `matches()` của widget.

## 3. Mô hình mode và phím đang chờ

| Trạng thái chờ | Bắt đầu bằng | Phím tiếp theo |
|---|---|---|
| count | `1`–`9`, và `0` khi đã có count | thêm chữ số; giới hạn 99999 |
| `g` | `g` | `g`, `e`, `E`; phím khác thì bỏ |
| `z` | `z` | `z` `t` `b` `a` `c` `o`; phím khác thì bỏ |
| find | `f` `F` `t` `T` | một ký tự bất kỳ, kể cả tiếng Việt gõ qua IME |
| mark | `m` | `a`–`z`; phím khác thì bỏ |
| jump | `'` hoặc `` ` `` | `a`–`z`, `'` hoặc `` ` ``; phím khác thì bỏ |

- `Escape` khi đang chờ thì xóa phần đang chờ.
- `Escape` ở normal mode, khi không có gì đang chờ, trả về `Unhandled`, để Esc vẫn đóng find widget, Quick Open và các popup khác như trước.
- `Escape` ở visual mode thì về normal mode.
- **Chuyển mode:**
  - `v`, `V`, `Ctrl+v` đi từ normal sang visual mode tương ứng.
  - Đang ở một visual mode, bấm lại đúng phím của mode đó thì về normal; bấm phím của visual mode khác thì chuyển sang mode đó.
  - Tab mới mở luôn bắt đầu ở normal mode.

## 4. Hành vi từng lệnh

Hành vi theo VSCodeVim, cũng là theo Vim. Chỗ nào lệch đều có ghi chú. Trong mọi motion:
- con trỏ là `Point` với cột tính bằng byte;
- mọi bước đi đều theo ranh giới ký tự (`char`), không bao giờ cắt giữa một ký tự UTF-8;
- motion không có đích thì con trỏ đứng yên, không báo lỗi và không kêu bíp.

### 4.1 Con trỏ và cột mong muốn

- Ở normal mode, con trỏ đứng trên một ký tự, không đứng sau ký tự cuối dòng. Trên dòng rỗng, con trỏ ở cột 0.
- **Cột mong muốn** (`goal`) được đo bằng cột hiển thị, có tính tab.
  - `j`, `k`, `Ctrl+d`/`Ctrl+u`/`Ctrl+f`/`Ctrl+b`, `H`/`M`/`L`, `G`/`gg` khi `startofline=false`, và `zz`/`zt`/`zb` giữ nguyên `goal`.
  - Mọi motion ngang đều đặt lại `goal`.
  - `$` đặt `goal = ∞`, nên đi `j`/`k` sau đó vẫn bám cuối dòng.
- `j`/`k` nhảy qua các dòng bị fold ẩn, giống Zed. VSCodeVim mặc định thì mở fold khi đi qua; Lucerna chọn theo Zed vì không có lý do gì để mở fold khi chỉ đọc.
- Khi bật word wrap, `j`/`k` vẫn đi theo dòng buffer như Vim. `gj`/`gk` để V2.

### 4.2 Motion

| Phím | Hành vi |
|---|---|
| `h` `l` | Một ký tự trái/phải, không vượt sang dòng khác. Count được hỗ trợ |
| `j` `k` | Dòng dưới/trên, đi tới `goal`, kẹp ở ký tự cuối |
| `w` `b` `e` `ge` | Ký tự chia làm 3 lớp: **word** (`char::is_alphanumeric()` hoặc `_`, nên chữ có dấu tiếng Việt cũng là word), **whitespace** và **punctuation** (mọi thứ còn lại, kể cả emoji). Dòng rỗng là một điểm dừng của `w` và `b`. Dòng chỉ có khoảng trắng bị bỏ qua. `e` bỏ qua dòng rỗng. `w` ở từ cuối file đi tới ký tự cuối cùng (chỉ ở visual mode mới qua được hết dòng) |
| `W` `B` `E` `gE` | Như trên, nhưng punctuation tính như word |
| `0` | Cột 0. Nếu đã có count thì `0` là chữ số của count |
| `^` | Ký tự không trắng đầu tiên. Trên dòng toàn khoảng trắng thì tới ký tự cuối |
| `$` | Cuối dòng. `N$` thì xuống N−1 dòng trước. Đặt `goal = ∞` |
| `gg` `G` | `gg` về dòng 1, `G` về dòng cuối. Có count N thì tới dòng N, kẹp ở dòng cuối. Theo `vim.startofline` (mặc định true) thì con trỏ tới ký tự không trắng đầu tiên, ngược lại thì giữ `goal`. **Lệch Zed** (Zed giữ cột); theo VS Code/Vim. Là một jump |
| `{` `}` | Lên/xuống tới dòng **rỗng thật** kế tiếp, đi qua mọi dòng không rỗng (dòng chỉ có khoảng trắng không phải ranh giới). Nếu đang đứng trong một khối dòng rỗng thì ra khỏi khối đó trước. Hết file mà không thấy thì `}` tới ký tự cuối của dòng cuối, `{` tới (0,0). Là một jump |
| `%` | Tìm ký tự đầu tiên thuộc `()[]{}` từ con trỏ trở về sau trên dòng hiện tại, rồi tới cặp của nó qua `ViewInfo::matching_bracket`, nên bracket trong chuỗi hay comment được bỏ qua khi có cây cú pháp. Không có thì đứng yên. `N%` (1–100) tới dòng `(N*lines + 99) / 100`. Là một jump |
| `H` `M` `L` | `H`: dòng đầu khung nhìn + (count−1); `L`: dòng cuối − (count−1); `M`: giữa các dòng đang hiện. Lề cuộn là `editor.cursorSurroundingLines`, trừ khi khung nhìn đã chạm đầu hoặc cuối file. Dòng bị fold ẩn không tính. **Có count như Vim/Zed** (VSCodeVim bỏ qua count, đó là lỗi đã biết của VSCodeVim). Theo `startofline`. Là một jump |
| `f` `F` `t` `T` | Tìm ký tự thứ N trên dòng hiện tại. `t`/`T` dừng trước/sau ký tự đó một ký tự. So khớp chính xác, có phân biệt hoa thường |
| `;` `,` | Lặp lại lần tìm cuối, cùng hướng hoặc ngược hướng. Với `t`/`T`, nếu lặp lại mà không di chuyển (vì ký tự đích nằm sát con trỏ) thì tìm thêm một lần nữa, giống `cpo` không có `;` |

### 4.3 Tìm kiếm

- **`/` và `?`**
  - Mở find widget ở chế độ vim, ô nhập trống, và lưu vị trí con trỏ hiện tại.
  - Enter: từ vị trí đã lưu, nhảy tới match đầu tiên nằm **sau** (với `/`) hoặc **trước** (với `?`) con trỏ, có vòng lại nếu `vim.wrapscan`. Nếu không có match, con trỏ đứng yên. Lưu `last_search` gồm pattern, các cờ của widget và hướng. Đây là một jump.
  - Esc: con trỏ ở yên chỗ cũ.
  - Count trước `/`, ví dụ `3/foo`, nhảy tới match thứ ba.
- **`n` / `N`:** match kế tiếp theo hướng của `last_search` / ngược hướng đó. Có count. `vim.wrapscan=false` thì dừng ở match cuối. Đây là một jump.
  - Highlight match vẫn giữ sau khi find widget đóng, cho tới lần tìm sau hoặc lần reload sau. `Action::Highlight` gọi `set_match_highlights` có sẵn.
- **`*` / `#`**
  - Lấy từ (lớp word) dưới con trỏ. Nếu con trỏ đang ở khoảng trắng hoặc punctuation, lấy từ kế tiếp trên dòng. Không có từ nào thì không làm gì.
  - Tìm nguyên từ: `\b` cộng phần từ đã escape. Không phân biệt hoa thường khi `vim.ignorecase` bật; bỏ qua smartcase, theo VSCodeVim.
  - `*` đi tới, `#` đi lùi, bắt đầu từ đầu từ hiện tại. Lưu vào `last_search`, nên `n` sau `#` cũng đi lùi.
- **Cách tìm:** crate gọi `ViewInfo::matches`. Workbench cài phương thức này bằng `find_in_snapshot` có sẵn và lưu đệm kết quả theo `(revision, pattern, cờ)`. Crate lấy match kế tiếp bằng tìm nhị phân trên vị trí con trỏ. Nếu `matches` trả về `None` (đang tìm ở luồng nền), `feed` ghi nhận phím đang chờ; workbench gọi lại `feed` với cùng phím khi có kết quả.

### 4.4 Cuộn

| Phím | Hành vi |
|---|---|
| `Ctrl+d` `Ctrl+u` | Cuộn xuống/lên `scroll` dòng; mặc định là nửa số dòng đang hiện, có thể đặt bằng `vim.scroll`. Con trỏ đi theo cùng số dòng, kẹp trong file. `N Ctrl+d` đặt `scroll = N` cho các lần sau. Khi đã ở đầu file, `Ctrl+u` vẫn đưa con trỏ lên |
| `Ctrl+f` `Ctrl+b` | Cuộn một trang trừ 2 dòng, như Vim. Con trỏ được kẹp vào khung nhìn |
| `zz` `zt` `zb` | Đưa dòng con trỏ lên giữa/đầu/cuối khung nhìn. Con trỏ và `goal` không đổi |
| `za` `zc` `zo` | Toggle/đóng/mở fold nhỏ nhất chứa dòng con trỏ. Khi đóng fold, con trỏ về dòng đầu của fold |

Các dòng cuộn được tính theo dòng buffer đang hiện, tức đã bỏ qua dòng bị fold ẩn.

### 4.5 Mark

- **`m{a-z}`:** lưu `Point` của con trỏ cho file đang mở. Mark chỉ sống trong phiên làm việc.
- **`` `x ``:** tới đúng vị trí đã mark.
- **`'x`:** tới ký tự không trắng đầu tiên của dòng có mark.
- Mark chưa đặt thì không làm gì.
- **Mark `'`:** mỗi jump (xem §4.2–§4.3) ghi vị trí *trước khi* nhảy vào mark `'`, nên `''` và ` `` ` quay về đó.
- **Khi file được agent sửa và reload:** mọi mark của file được map lại bằng `Reload::adjust_row`, và cột được kẹp vào dòng mới. Workbench đã nhận `Reload` trong `CodeView::apply_reload`, và sẽ chuyển nó cho `VimGlobals::remap_marks(path, &reload, line_count)`.
- Jump tới mark ở một dòng nằm ngoài khung nhìn sẽ cuộn để căn giữa, giống `go_to`. Các motion khác chỉ cuộn vừa đủ (`set_cursor`).

### 4.6 Visual mode

- **`v` (Visual):** selection là `anchor` → `head`, tính cả ký tự dưới `head`. Mọi motion ở §4.2 và §4.3 đều di chuyển `head`. Ở visual mode, `$` lấy cả ký tự xuống dòng.
- **`V` (VisualLine):** chọn trọn các dòng từ dòng của `anchor` tới dòng của `head`.
- **`Ctrl+v` (VisualBlock):** khối hình chữ nhật từ dòng `anchor` tới dòng `head`, từ cột hiển thị min tới max. Cột tính theo cột hiển thị, nên dòng có tab cũng thẳng hàng. Sau `$`, khối chạy tới cuối mỗi dòng. Dòng ngắn hơn cột trái của khối thì bỏ qua.
- **`o`:** đổi chỗ `anchor` và `head`. Đây là thao tác đọc, rẻ và quen tay.
- **`y`**
  - Ghi text vào clipboard hệ thống. Lucerna chưa có register, nên không đọc `vim.useSystemClipboard`.
    - Visual: text từ đầu tới cuối, tính cả ký tự cuối.
    - VisualLine: các dòng ghép lại, kèm ký tự xuống dòng ở cuối.
    - VisualBlock: mỗi dòng của khối, ghép bằng `\n`.
  - Ghi mark `<` và `>`. Mark này chỉ dùng nội bộ ở V1.
  - Về normal mode, con trỏ ở **đầu** vùng chọn. Với VisualLine, con trỏ ở cột 0 của dòng đầu; với VisualBlock, ở góc trên trái.
- **`Esc`:** về normal mode, con trỏ ở `head`, giữ `goal`.
- **`cmd+c`** khi đang ở visual mode copy giống `y` mà không thoát visual. Keymap lo việc này qua `editor.action.clipboardCopyAction`, vì selection của CodeView đã đúng.

## 5. Giao diện

- **Chỉ báo mode**
  - Là một status item mới ở bên trái status bar, chỉ hiện khi vim bật và tab đang mở là code.
  - Chữ hiển thị: `-- NORMAL --`, `-- VISUAL --`, `-- VISUAL LINE --` hoặc `-- VISUAL BLOCK --`, dùng font UI và màu `text_muted`. Khi có phím đang chờ, hiện thêm phím đó bằng font mono, ví dụ `-- NORMAL --  2f`.
  - Thứ chữ này giống hệt cả Zed lẫn VSCodeVim.
- **Con trỏ:** xem §2.4.
- **Bật hoặc tắt vim lúc đang chạy:** khi tắt, mọi `Vim` bị xóa, selection visual biến mất và con trỏ không được vẽ nữa. Khi bật lại, mọi view bắt đầu ở normal mode.

## 6. Settings

| Khóa | Kiểu | Mặc định | Nguồn |
|---|---|---|---|
| `vim.enabled` | bool | false | Riêng của Lucerna: VSCodeVim bật bằng cách cài extension, nên không có khóa này |
| `vim_mode` | bool | — | Tên thay thế giống Zed. Chỉ được đọc khi `vim.enabled` chưa được đặt ở lớp settings nào |
| `vim.ignorecase` | bool | true | VSCodeVim. Ở V1 chỉ dùng cho `*`/`#`. `/` dùng nút case của find widget |
| `vim.wrapscan` | bool | true | VSCodeVim |
| `vim.startofline` | bool | true | VSCodeVim |
| `vim.scroll` | number | 0 (nửa trang) | VSCodeVim |
| `editor.cursorSurroundingLines` | number | 0 | VS Code; dùng làm lề cuộn |

- Settings có hiệu lực ngay khi `AppState` đổi.
- Các khóa trên được đưa vào schema hoặc tài liệu settings ở cùng nơi với các khóa đang có.

## 7. Xử lý lỗi và trường hợp biên

| Tình huống | Hành vi |
|---|---|
| File rỗng | Mọi motion đứng yên ở (0,0); `y` ghi chuỗi rỗng |
| Dòng cuối không có ký tự xuống dòng | `G`, `}`, `$` vẫn đúng; yank VisualLine vẫn thêm `\n` ở cuối, như Vim |
| Dòng dài hơn 10000 ký tự | Motion ngang đọc tối đa 10000 ký tự đầu, giống phần hiển thị. Con trỏ không đi xa hơn giới hạn đó. `$` tới ký tự thứ 10000 |
| Tiếng Việt, emoji, CJK | Mỗi bước đi là một `char`. Ô con trỏ rộng theo bề rộng hiển thị của ký tự |
| File reload khi con trỏ ở dòng đã bị xóa | Con trỏ và anchor được kẹp vào dòng và ký tự hợp lệ gần nhất. Visual mode được giữ nguyên |
| Tab Loading, Error hay Extensions | Lớp vim không nhận phím |
| Regex `/` không hợp lệ | Find widget hiện lỗi. Enter không làm gì |
| Tìm không có match | Con trỏ đứng yên |
| Panic trong `feed` | Bắt bằng `catch_panic` như các chỗ khác trên luồng UI; ghi log, xóa phần phím đang chờ, coi như `Handled` |

## 8. Hiệu năng

- Motion chỉ đọc các dòng cần thiết qua `BufferSnapshot::line` và `chunks_in_range`, không dựng lại toàn bộ text.
- `{`/`}` trên file 100k dòng không có dòng rỗng phải quét cả file. Mỗi dòng chỉ cần kiểm tra độ dài, nên vẫn dưới 5 ms.
- `n`/`N`/`*`/`#` lần đầu gọi `find_in_snapshot`, quét cả file. Kết quả được lưu đệm theo `(revision, pattern, cờ)`, nên các lần `n` sau chỉ còn là tìm nhị phân.
  - Snapshot lớn hơn `BACKGROUND_SEARCH_BYTES` (1 MB) thì việc tìm chạy ở luồng nền, giống find widget. Phím `n` được ghi nhận và thực hiện khi có kết quả. Nếu có phím khác tới trước, phím `n` đang chờ bị bỏ.
- Vẽ con trỏ chỉ thêm một quad cho mỗi frame.

## 9. Kiểm thử

### 9.1 `lucerna-vim`: unit test

- **Helper dạng chuỗi:** `assert_keys("one |two three", "w", "one two |three")`.
  - `|` đánh dấu con trỏ. Visual mode thêm `[` cho anchor.
  - Có các tham số kích thước khung nhìn và dòng bị fold, cho các test cuộn và H/M/L.
- **Chuyển thể từ VSCodeVim**, khoảng 45 ca. Mỗi ca có comment ghi file nguồn và tên test gốc:
  - `test/mode/normalModeTests/motions.test.ts`: w qua punctuation và qua cuối dòng, `[count]w`, `$` có count, `$jj`, `[count]gg` và `[count]G` khi `startofline` bật hoặc tắt, `f` rồi `,`, `fa;;;,,,`, `tttt`, `TtTt`, `5/tw` có wrap, `?two` rồi `n`, `*` nguyên từ, `*` với `ignorecase`, `*` trên khoảng trắng không qua dòng, `#` trên khoảng trắng, `#` trên từ, `zz` giữ `goal`, `vjjv'<`.
  - `test/motion.test.ts`: w, b, e, W, E qua dòng rỗng và dòng chỉ có khoảng trắng, w ở cuối file, `{`/`}` trên văn bản mẫu "paragraphs".
  - `test/mode/normalModeTests/matchingBracket.test.ts`: `%` trước dấu mở ngoặc, `%` với ngoặc lồng nhau, `%` sau `->`.
  - `test/register/repeatableMovement.test.ts`: `tc;`.
  - `test/mode/modeNormal.test.ts`: `2<C-d><C-d>`, `<C-u>` khi dòng đầu đang hiện.
  - `test/mode/normalModeTests/commands.test.ts`: `gegegegege` qua nhiều dòng.
  - `test/marks.test.ts`: `` wma2w`a ``, và `` vjlo<Esc>gg`> `` (chuyển thể dùng mark `>` nội bộ).
  - `test/mode/modeVisualLine.test.ts`: `Vj<Esc>` giữ cột.
  - `test/search/search.test.ts`: `?x` rồi `nn`.
- **Ca riêng của Lucerna:**
  - `w`/`e`/`b` trên `"Xin chào thế giới"` và trên emoji.
  - `f` với ký tự tiếng Việt (`fế`).
  - `$jj` qua dòng có tab.
  - Dòng 20000 ký tự.
  - File rỗng.
  - Dòng cuối không có ký tự xuống dòng.
  - j/k qua fold.
  - Count `0` (`10j`).
  - Remap mark sau `Reload`.
  - Yank block có tab.
  - `Esc` khi đang chờ.
  - Phím chưa có nghĩa thì không lọt xuống keymap.

### 9.2 `lucerna-workbench`: test tích hợp với `TestAppContext`

- `vim.enabled=false`: gõ `j` không làm gì, phím đi xuống như cũ.
- Bật vim: `j` di chuyển; `Esc` vẫn đóng find widget; gõ `j` trong Quick Open, find widget, ô Search và trang Extensions vẫn vào ô nhập; `cmd+p` vẫn mở Quick Open.
- `/foo` rồi Enter: con trỏ nhảy tới, widget đóng, `n` sang match kế tiếp, highlight vẫn còn.
- `vjy` ghi đúng text vào clipboard của test.
- Chỉ báo mode hiện `-- NORMAL --` rồi `-- VISUAL LINE --`, và có phần phím đang chờ.
- Mark vẫn đúng sau khi file reload (dùng chung luồng test với `documents.rs`).
- Bấm chuột trong visual mode thì về normal.
- Bật `vim_mode` thay cho `vim.enabled` cũng có tác dụng.
- Context key `vim.mode` dùng được trong `when` của một keybinding người dùng.

### 9.3 Kiểm tra bằng tay sau khi cài

Chạy `scripts/bundle-app.sh --install`, bật vim, rồi đọc một file Rust lớn. Kiểm tra:
- con trỏ block nhìn rõ trên theme sáng và tối;
- `Ctrl+d` mượt;
- `/` và Enter hoạt động.

## 10. Giấy phép

- Không chép code từ `crates/vim` của Zed (GPL-3). Chỉ học thiết kế, đúng như ghi chú nghiên cứu.
- Các ca test chuyển thể từ VSCodeVim (MIT): thêm một mục "VSCodeVim test cases" vào `THIRD_PARTY_NOTICES.md`, ghi `Copyright (c) 2015 VSCodeVim` kèm nguyên văn giấy phép MIT. Mỗi file test có comment ghi nguồn.
- Crate mới chỉ phụ thuộc `lucerna-text`. Không có dependency bên ngoài nào; regex nằm ở workbench, sau `ViewInfo::matches`. `cargo deny check` phải qua.

## 11. Rủi ro

| Rủi ro | Cách giảm |
|---|---|
| Xung đột với session `lucerna-search-ui` trong `find_widget.rs` | Chỉ thêm event và cờ mở. Pull/rebase ngay trước khi sửa. Đã báo trước cho session đó |
| `key_char` khác nhau giữa các layout bàn phím và IME | Dùng `key_char` cho ký tự in được, đúng với mọi layout. Phím có ctrl dùng `key`. Có test cho phím có shift (`$`, `{`) |
| Con trỏ chưa từng được vẽ, nên có thể lệch với dòng wrap hoặc tab | Dùng chung `x_for` mà selection đang dùng |
| Selection inclusive ↔ exclusive bị lệch một ký tự | Chuyển đổi gom về một hàm trong `visual.rs` và có test riêng |

## 12. Thứ tự triển khai dự kiến

1. Crate `lucerna-vim`: khung crate, `Key`/`Mode`/`Action`/`ViewInfo`, helper test, `classify`.
2. Motion ngang và dọc: `hjkl`, `0 ^ $`, `w b e ge W B E gE`, `goal`, count.
3. Motion còn lại: `gg G`, `{ }`, `%`, `f t ; ,`, `H M L`.
4. Cuộn và fold: `Ctrl+d u f b`, `zz zt zb`, `za zc zo`.
5. Mark và jump: `m`, `'`, `` ` ``, `''`, remap khi reload.
6. Visual mode: `v V Ctrl+v`, `o`, `y`, `Esc`.
7. Tìm kiếm trong crate: `*`, `#`, `n`, `N`, `search_submitted`.
8. CodeView: vẽ con trỏ, vẽ selection kiểu block, `set_cursor`, `fold_at`, `selection_generation`.
9. Workbench: `VimLayer`, chuyển phím, context key, settings, chỉ báo mode.
10. Find widget: hook cho `/` và `?`.
11. Test tích hợp, `THIRD_PARTY_NOTICES.md`, README và roadmap, `CLAUDE.md` (thêm `lucerna-vim` vào danh sách crate).
