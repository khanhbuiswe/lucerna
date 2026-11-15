# Chuyển nhánh từ thanh tiêu đề

Trạng thái: người dùng đã duyệt ngày 2026-11-15.

Tra cứu: `docs/research/2026-11-15-git-branch-switch.md`. Dựa trên Git 5.1 và 5.2 (`2026-11-13-git-view-design.md`), cả hai đã có trên `main`.

## 1. Mục đích

Người dùng muốn bấm vào tên nhánh trên thanh tiêu đề để thấy danh sách nhánh và chuyển sang nhánh khác ngay trong Lucerna, giống Zed. Lúc này tên nhánh chỉ là chữ, bấm vào không có tác dụng gì.

Đây là thao tác **đầu tiên làm thay đổi repo** của Lucerna. Mọi phần Git trước đây chỉ đọc. Vì vậy thiết kế ưu tiên: không làm mất thay đổi nào, và để git tự quyết như khi gõ lệnh trong terminal.

### Mục tiêu
1. Bấm tên nhánh trên thanh tiêu đề thì mở picker nhánh, đặt ngay dưới nút.
2. Lệnh `git.checkout` "Git: Checkout to…" mở cùng picker đó, đặt ở vị trí của Quick Open.
3. Picker liệt kê nhánh local và nhánh remote, gõ để lọc. Chọn một nhánh thì chuyển sang nhánh đó.
4. Gõ một tên chưa có thì có dòng "Create branch ‹tên›". Chọn dòng đó thì tạo nhánh từ HEAD và chuyển sang luôn.
5. Chọn nhánh remote chưa có bản local thì tạo nhánh local theo dõi nó rồi chuyển sang.
6. Sau khi chuyển, thanh tiêu đề, Explorer, gutter và các tab diff tự cập nhật.

### Không làm trong bản này
- Tag và detached HEAD.
- "Create branch from…" (chọn mốc khác HEAD).
- Stash, Migrate Changes, Force Checkout.
- Xóa nhánh, đổi tên nhánh, đặt upstream.
- Pull hoặc fetch trước khi chuyển.
- `git.branchPrefix`, `git.branchRandomName`.

### Phản mục tiêu
- Làm mất thay đổi chưa commit, hoặc ghi đè file đang sửa trong Lucerna mà chưa lưu.
- Chạy git trong thư mục chưa tin cậy.
- Chặn luồng UI trong lúc git chạy.
- Hành vi khác với `git switch` gõ trong terminal.

## 2. Tham khảo Zed: màn hình tương ứng

Zed (revision `b5cd310`, `git_ui/src/branch_picker.rs`):
- Nút tên nhánh trên title bar mở một popover gắn ngay dưới nó.
- Popover gồm ô nhập và danh sách: tên nhánh, tiêu đề commit, thời gian. Nhánh hiện tại đứng đầu.
- Gõ tên chưa có thì cuối danh sách có dòng tạo nhánh. Lỗi khi chuyển nhánh được hiện trong hộp thoại "Failed to change branch".

Chỗ Lucerna khác Zed:
- Không có bộ lọc All/Local/Remote. Thay vào đó, nhánh local và nhánh remote là hai nhóm theo thứ tự.
- Không xóa nhánh và không thêm remote, vì nằm ngoài phạm vi.
- Mỗi dòng không hiện tiêu đề commit, để danh sách gọn. Có thể thêm sau.

Bản phác:

```
lucerna  ⎇ main ▾                       ← bấm tên nhánh
  ┌─────────────────────────────────────┐
  │ fix-gu│                             │
  │ ✓ main                   2 hours ago│
  │   fix-gutter-click         yesterday│
  │   origin/fix-gutter-docs  3 days ago│
  │ + Create branch "fix-gu"            │
  └─────────────────────────────────────┘
```

## 3. Các cách đã cân nhắc

### 3.1 Khi có thay đổi chưa commit
- **A. Như Zed: chạy `git switch` và để git quyết. Chọn** (người dùng chọn).
  - Thay đổi không đụng nhau thì git mang sang nhánh mới.
  - Thay đổi đụng nhau thì git từ chối, và Lucerna hiện nguyên thông báo của git.
  - Không bao giờ làm mất thay đổi.
- B. Như VS Code: thêm Stash & Checkout và Migrate Changes. Đây là thao tác ghi thứ hai, và có thể xung đột khi pop. Bỏ.
- C. Chặn hẳn khi còn bất kỳ thay đổi nào. An toàn nhất, nhưng chặn cả những trường hợp git làm được. Bỏ.

### 3.2 Tạo nhánh
**Có, tạo từ HEAD** (người dùng chọn). Không làm "from…".

### 3.3 Picker
- **Mở rộng picker so sánh của tab Changes since (`diff_view/branch_picker.rs`) với chế độ "chuyển nhánh". Chọn.** Một component cho cả hai, đúng tinh thần dùng chung trong `docs/design-principles.md` §1.1.
- Viết một picker riêng: trùng code. Bỏ.

## 4. Thiết kế

### 4.1 `lucerna-git`
- **`Repo::branch_list(&self) -> Result<Vec<BranchRef>, GitError>`:**
  - Chạy `for-each-ref --sort=-committerdate --format=%(refname)%00%(committerdate:unix)%00%(HEAD) refs/heads refs/remotes`.
  - Bỏ `refs/remotes/*/HEAD`.
  - `BranchRef { name: String, remote: Option<String>, committed: i64 (unix), is_head: bool }`.
    - `name` là tên ngắn: `main`, `origin/feature/x`.
    - `remote` là tên remote, lấy bằng cách so tiền tố của `refs/remotes/<remote>/` với danh sách `git remote`. Tên remote không bao giờ được tách theo dấu `/` đầu tiên, vì `origin/feature/x` có thể là remote `origin` với nhánh `feature/x`.
  - `Repo::branches()` hiện có, mà chế độ Compare dùng, giữ nguyên. Không bắt buộc chuyển nó sang dùng `branch_list`.
- **`Repo::switch_branch(&self, target: SwitchTarget) -> Result<(), GitError>`:**
  - `SwitchTarget::Local(name)` chạy `switch <name>`.
  - `SwitchTarget::Remote { remote, branch }` (ví dụ `origin`, `feature/x`):
    - nếu `refs/heads/<branch>` đã có thì chạy `switch <branch>`;
    - nếu chưa thì chạy `switch --track <remote>/<branch>`.
  - `SwitchTarget::Create(name)` chạy `switch -c <name>`.
  - Mọi tên đều đi sau `--` khi git cho phép, hoặc được kiểm tra trước, để một tên bắt đầu bằng `-` không bị hiểu thành cờ.
- **Chạy lệnh ghi:**
  - Vẫn dùng `--no-pager` và `-c core.fsmonitor=false`.
  - **Không** đặt `core.hooksPath=/dev/null`: hook `post-checkout` của git-lfs, husky… chạy như trong terminal. Lệnh ghi chỉ chạy ở thư mục đã tin cậy (§4.5).
  - Bỏ `--no-optional-locks`, vì đây là lệnh ghi.
  - **Không có thời gian chờ và không bao giờ kill.** Kill git giữa chừng có thể để lại cây file chuyển dở hoặc một `index.lock` cũ. Checkout kèm LFS hay hook có thể lâu, nên cứ để nó chạy xong.
  - Thêm một hàm riêng trên `GitBinary`, kiểu `command_for_write`. Hàm `command()` của các lệnh đọc và test `command_is_hardened` giữ nguyên.
- **`pub fn sanitize_branch_name(input: &str) -> String`:**
  - Bỏ khoảng trắng hai đầu.
  - Thay mỗi dãy khoảng trắng bằng `-`, như mặc định `git.branchWhitespaceChar` của VS Code.
- **`Repo::check_branch_name(&self, name: &str) -> bool`:** chạy `git check-ref-format --branch <name>`.

### 4.2 Picker
Mở rộng `BranchPicker` với chế độ `Compare` (như hiện nay) và `Checkout`.

**Chế độ Checkout:**
- **Placeholder:** "Switch to a branch or type to create one".
- **Thứ tự dòng khi chưa lọc:**
  1. nhánh hiện tại (dấu ✓);
  2. các nhánh local khác, commit mới nhất trước;
  3. các nhánh remote, commit mới nhất trước.
- **Mỗi dòng:** icon nhánh, tên nhánh, thời gian tương đối của commit cuối ở bên phải, màu mờ ("2 hours ago", "yesterday", "3 days ago"). Dùng helper sẵn có nếu đã có; nếu chưa thì tạo trong `ui.rs`.
- **Lọc:** dùng `fuzzy_filter` như hiện nay.
- **Dòng tạo nhánh:**
  - So bằng tên đã chuẩn hóa. Dòng hiện khi `sanitize_branch_name(query)` không rỗng, không trùng tên nhánh local nào, và không trùng phần `branch` của nhánh remote nào. Nếu chỉ có `origin/foo`, gõ "foo" thì người dùng chọn dòng `origin/foo` để có nhánh `foo` theo dõi nó, chứ không tạo một nhánh `foo` rời. Dòng nằm cuối danh sách: "+ Create branch "‹tên đã chuẩn hóa›"".
  - Nếu tên không hợp lệ theo `check_ref_format`, dòng vẫn hiện nhưng bị làm mờ, kèm "Invalid branch name", và chọn thì không làm gì.
  - Gõ đúng tên một nhánh đã có thì không hiện dòng tạo, chỉ hiện nhánh đó.
- **Chọn nhánh hiện tại:** chỉ đóng picker.
- **Chọn nhánh khác hoặc dòng tạo:** đóng picker rồi chuyển nhánh (§4.3).

**Vị trí:**
- Mở từ thanh tiêu đề: popover gắn dưới nút tên nhánh, căn trái theo nút.
- Mở từ lệnh: vị trí của Quick Open.
- Cả hai dùng `ui::popover` và các phần của `quick_open::picker`.

Danh sách nhánh được nạp ở luồng nền như hiện nay. Picker hiện ngay khi nạp xong, giữ đúng hành vi đang có.

### 4.3 Chuyển nhánh
1. **Kiểm tra file chưa lưu.** Nếu `Documents::dirty_paths()` không rỗng, hoặc có tài liệu đang ở trạng thái xung đột với đĩa (`has_conflict`), thì không chạy git. Status bar báo "Save or revert unsaved files before switching branches" và giữ nguyên picker đã đóng.
   - Lý do: git ghi file mới lên đĩa. Một buffer chưa lưu sẽ rơi vào trạng thái xung đột với đĩa. Lucerna đã có cách xử lý trạng thái đó (Phase 4.4), nhưng người dùng không cố ý gây ra nó.
2. **Chạy `Repo::switch_branch` ở luồng nền.** Trong lúc chạy:
   - tên nhánh trên thanh tiêu đề mờ đi, kèm chữ "Switching…";
   - lệnh `git.checkout` và các cú bấm tên nhánh bị bỏ qua;
   - editor vẫn sửa được. Nếu người dùng sửa một file mà git đang ghi lại thì cơ chế xung đột của Phase 4.4 xử lý như khi agent ghi file. Đây là chỗ hở đã biết và chấp nhận được, vì chuyển nhánh thường xong dưới một giây.
3. **Thành công:** không cần làm gì thêm. Watcher của 5.1 thấy `HEAD` và `index` đổi rồi quét lại. Watcher worktree thấy file đổi nên Documents nạp lại buffer. Thanh tiêu đề đọc lại `HEAD`.
   - Để không phải chờ watcher, gọi thêm `GitStore::rescan()`, và đọc lại tên nhánh bằng `git_branch::read_branch`, ngay sau khi lệnh xong. Tên nhánh trên thanh tiêu đề vốn đọc từ `HEAD` qua `git_branch.rs`, không đọc từ `GitState`.
4. **Thất bại** (git trả mã lỗi, hoặc không chạy được git): hiện hộp thoại native `window.prompt(PromptLevel::Critical, "Could not switch branches", Some(<stderr của git, đã cắt bớt khoảng trắng>), ["OK"])`, như cách Zed báo lỗi. Repo giữ nguyên, vì git không đổi gì khi từ chối.

### 4.4 Lệnh và phím

| Lệnh | Tiêu đề | Phím | Ghi chú |
|---|---|---|---|
| `git.checkout` | Git: Checkout to… | không có | ID của VS Code. Mở picker ở chế độ Checkout |

Bấm vào icon hoặc tên nhánh trên thanh tiêu đề cũng mở picker. Con trỏ chuột đổi thành bàn tay khi rê lên. Nút tên nhánh phải chặn sự kiện chuột lan lên thanh tiêu đề, để cú bấm không thành kéo cửa sổ hay double-click phóng to.

### 4.5 Restricted Mode và trạng thái git
Tên nhánh trên thanh tiêu đề có thể hiện ngay cả khi git không chạy, vì nó đọc thẳng file `HEAD`. Vì vậy cú bấm phải xem `GitState`:
- `Restricted` (thư mục chưa tin cậy): mở hộp thoại tin cậy (`workbench.trust.manage`). Lệnh `git.checkout` cũng vậy.
- `Off`, `Locating`, `NoGit`, `NoRepo`: không mở picker. Status bar báo một dòng:
  - `Off`: "Git is off";
  - `Locating`: "Git is still loading";
  - `NoGit` và `NoRepo`: "Git is not available".
- `TooManyChanges` vẫn chuyển nhánh được.

### 4.6 Settings
Không thêm setting nào.

## 5. Test

### `lucerna-git` (repo tạm tạo bằng lệnh `git`)
- `branch_list`: đúng thứ tự theo ngày; đánh dấu `is_head`; bỏ `origin/HEAD`; tách Local và Remote.
- `switch_branch`:
  - chuyển sang nhánh local;
  - nhánh remote chưa có bản local thì tạo nhánh theo dõi;
  - nhánh remote đã có bản local thì chuyển sang bản local;
  - tạo nhánh mới;
  - có thay đổi đụng nhau thì trả lỗi kèm stderr của git, và repo không đổi;
  - có thay đổi không đụng nhau thì thay đổi được mang sang.
- Hook `post-checkout` có chạy khi chuyển nhánh, trong khi các lệnh đọc vẫn không chạy hook.
- Nhánh remote có `/` trong tên (`origin/feature/x`) thì tạo được nhánh local `feature/x`.
- `sanitize_branch_name` và `check_branch_name`: khoảng trắng, tên bắt đầu bằng `-`, `..`, tên rỗng.

### `lucerna-workbench/tests/`
- Bấm tên nhánh thì picker mở ở chế độ Checkout. Nhánh hiện tại đứng đầu, có ✓.
- Chọn một nhánh: HEAD đổi, thanh tiêu đề hiện tên mới, statuses và hunk được cập nhật.
- Gõ tên mới: có dòng "Create branch …". Enter tạo nhánh và chuyển sang.
- Gõ tên không hợp lệ: dòng tạo bị mờ, Enter không làm gì.
- Có file chưa lưu: không chạy git, status bar có thông báo, HEAD không đổi.
- Git từ chối: có hộp thoại lỗi (dùng prompt giả của test), HEAD không đổi.
- Restricted Mode: bấm tên nhánh mở hộp thoại tin cậy, không có lệnh git nào chạy.
- Chế độ Compare của tab Changes since vẫn chạy như cũ.

## 6. Để người làm plan tự quyết
- Cách tổ chức chế độ trong `BranchPicker` (enum mode, hoặc truyền delegate vào), và có nên chuyển picker ra khỏi `diff_view/` sang chỗ dùng chung hay không.
- Cách lấy vị trí nút tên nhánh để đặt popover.
- Định dạng thời gian tương đối.

## 7. Quyết định do tôi đặt (người dùng duyệt chung khi nói "ok")
- Lệnh chuyển nhánh chạy hook bình thường. Các lệnh đọc vẫn tắt hook.
- Lệnh ghi không có thời gian chờ và không bị kill.
- Chặn chuyển nhánh khi còn file chưa lưu.
- Báo lỗi bằng hộp thoại native.
- Mỗi dòng không hiện tiêu đề commit. Không có bộ lọc Local/Remote.
- Dòng tạo nhánh với tên không hợp lệ vẫn hiện, nhưng bị làm mờ. Không hiện dòng tạo khi tên gõ vào trùng một nhánh remote.
- Sửa file trong lúc đang chuyển nhánh là chỗ hở đã biết và được chấp nhận.
- Chặn chuyển nhánh cả khi có tài liệu đang xung đột với đĩa.
