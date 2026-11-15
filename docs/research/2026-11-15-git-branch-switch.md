# Chuyển nhánh: VS Code và Zed làm thế nào

Tra cứu cho tính năng bấm tên nhánh trên thanh tiêu đề để chuyển nhánh. Spec: `docs/superpowers/specs/2026-11-15-git-branch-switch-design.md`.

Nguồn:
- VS Code: `microsoft/vscode` nhánh `main`, giấy phép MIT. Đọc `extensions/git/src/{commands,statusbar,repository,git}.ts`, `package.json` và `package.nls.json`.
- Zed: revision `b5cd310` mà Lucerna đang pin, giấy phép GPL. Chỉ học thiết kế, không chép code. Đọc `crates/git_ui/src/branch_picker.rs` và `crates/git/src/repository.rs`.

## 1. Cách mở

| | VS Code | Zed |
|---|---|---|
| Từ đâu | bấm mục nhánh trên status bar, hoặc lệnh `git.checkout` "Git: Checkout to..." | bấm nút tên nhánh trên title bar; lệnh `git::Branch` |
| Dạng | Quick Pick ở giữa phía trên | popover gắn ngay dưới nút tên nhánh (`GitBranchSelector`) |
| Phím mặc định | không có | không có cho lệnh mở |

## 2. Danh sách

- **VS Code:**
  - Placeholder là "Select a branch or tag to checkout".
  - Khi ô lọc còn trống, ba mục đứng đầu: "+ Create new branch...", "+ Create new branch from...", "Checkout detached...".
  - Sau đó là các nhóm "branches", "remote branches", "tags", theo thứ tự của `git.checkoutType`.
  - Sắp theo ngày commit giảm dần (`git.branchSortOrder`).
  - Mỗi dòng gồm: nhãn; mô tả `N↓ M↑` kèm ngày tương đối; dòng chi tiết `tác giả • hash • tiêu đề` (`git.showReferenceDetails`).
  - Lọc mờ (fuzzy) theo nhãn và mô tả. Không hiện `origin/HEAD`.
- **Zed:**
  - Placeholder là "Switch or type to create a branch…".
  - Liệt kê nhánh local và remote. Có bộ lọc All / Local / Remote.
  - Nhánh hiện tại được ưu tiên đứng đầu. Mỗi dòng hiện tên nhánh, tiêu đề commit và thời gian.
  - Gõ một tên chưa có thì hiện dòng tạo nhánh.
  - Giữ một phím bổ trợ để xóa nhánh.

## 3. Chuyển nhánh khi có thay đổi chưa commit

- **VS Code:**
  - Chạy `git checkout -q <ref>`. Thay đổi không đụng nhau được git tự mang sang nhánh mới.
  - Nếu stderr có "commit your changes or stash them", hiện hộp thoại "Your local changes would be overwritten by checkout." với ba nút:
    - **Stash & Checkout**: stash, gồm cả file chưa theo dõi;
    - **Migrate Changes**: stash, chuyển nhánh, rồi pop;
    - **Force Checkout**: xóa sạch rồi chuyển.
  - `git.autoStash` không áp dụng cho checkout.
- **Zed:** `change_branch` chạy `git checkout <tên>`. Lỗi được hiện nguyên văn trong hộp thoại "Failed to change branch". Không có stash.

## 4. Nhánh remote

Cả hai đều làm giống nhau:
- Đã có nhánh local cùng tên: chuyển sang nhánh đó. Zed còn đặt lại upstream.
- Chưa có: tạo nhánh local theo dõi nhánh remote. VS Code dùng `checkout --track`, Zed dùng `branch --track` rồi `checkout`.

## 5. Tạo nhánh

- **VS Code:**
  - Ô nhập có prompt "Please provide a new branch name". Giá trị ban đầu lấy từ `git.branchPrefix`.
  - Khoảng trắng và ký tự không hợp lệ được thay bằng `git.branchWhitespaceChar` (mặc định `-`).
  - Báo "Branch "x" already exists" khi trùng tên. Kiểm tra theo `git.branchValidationRegex`.
  - Tạo xong thì chuyển sang luôn (`checkout -b`).
- **Zed:** gõ tên rồi Enter thì chạy `git switch -c <tên>` từ HEAD. Phím phụ thì tạo từ nhánh mặc định.

## 6. Restricted Mode và hook

- VS Code tắt toàn bộ extension git khi workspace chưa tin cậy.
- Zed chạy git, nhưng tắt hook ở thư mục chưa tin cậy. Ở thư mục đã tin cậy thì hook chạy bình thường.

## 7. Lucerna chọn gì

Người dùng duyệt ngày 2026-11-15:
- **Mở và hiển thị:** popover dưới tên nhánh trên thanh tiêu đề, theo Zed. Lệnh và ID lấy theo VS Code (`git.checkout`).
- **Thay đổi chưa commit:** để git tự quyết, như Zed. Lucerna chỉ chặn trước khi còn file chưa lưu.
- **Tạo nhánh:** có, từ HEAD.
- **Nhánh remote:** tạo nhánh local theo dõi nó.
- **Phạm vi bản này:** không có tag, detached HEAD, stash hay xóa nhánh.
