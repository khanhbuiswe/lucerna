# Git chỉ để xem (giai đoạn 5)

Trạng thái: người dùng đã duyệt ngày 2026-11-14. Các quyết định của người dùng: làm như Zed, gọi lệnh `git`, mốc mặc định là HEAD, chỉ một cột, git bật hộp thoại tin cậy.

Tra cứu VS Code và Zed: `docs/research/2026-11-13-git.md`.

## 1. Mục đích

Người dùng Lucerna để agent (Claude Code, Codex) viết code, và **agent tự commit**. Sau mỗi lượt chạy, người dùng cần thấy nhanh ba điều:
- agent đã sửa những file nào;
- sửa ở đâu trong file;
- cụ thể sửa gì.

Lucerna hiện chỉ có tên nhánh trên thanh tiêu đề (`git_branch.rs`). Chưa có gutter git, chưa có diff.

Bản đầu chỉ để **xem**: không stage, không commit, không revert. Giao diện theo Zed. Lệnh, phím và setting theo VS Code.

### Mục tiêu
1. Tô màu trạng thái git cho file và thư mục trong Explorer.
2. Vạch thay đổi ở gutter của editor, so với mốc `git.diffBase` (mặc định HEAD).
3. Mở diff của một file (`git.openChange`).
4. Tab **Uncommitted Changes**: gom mọi thay đổi chưa commit vào một view.
5. Tab **Changes since ‹nhánh›**: gom mọi thay đổi từ điểm rẽ nhánh (merge-base) tới bản trên đĩa, gồm cả các commit của agent. Đổi được nhánh làm mốc.
6. Tự cập nhật khi agent ghi file, commit, đổi nhánh hay rebase. Không cần bấm làm mới.
7. Không bao giờ chặn UI. Mọi lệnh `git` và mọi phép diff đều chạy nền.

### Không làm trong bản này
- Stage, unstage, commit, revert hunk, push, pull, đổi nhánh, Git panel.
- Diff hai cột (side-by-side). Bản này chỉ có một cột, dòng xóa và dòng thêm xen kẽ.
- Bung hunk ngay trong editor (kiểu Zed) và khung peek (kiểu VS Code).
- Commit View (diff của một commit), lịch sử, blame.
- Word diff (tô từng từ trong dòng).
- Vạch thay đổi trên minimap và overview ruler.
- Nhiều repo trong một cửa sổ, và submodule. Chỉ dùng repo chứa thư mục gốc của workspace.
- Sửa code ngay trong tab diff. Tab chỉ đọc.

### Phản mục tiêu (làm thế này thì coi như hỏng, dù chạy được)
- UI giật khi agent ghi hàng trăm file, hoặc khi mở repo lớn.
- Bật hộp thoại cài Command Line Tools của macOS trên máy chưa có git.
- Chạy lệnh do config của một repo lạ chỉ định, khi thư mục chưa được tin cậy.
- Kết quả khác với `git status` / `git diff` mà agent và người dùng thấy trong terminal.

## 2. Tham khảo Zed: màn hình tương ứng

Nguồn là mã nguồn Zed ở revision `b5cd310`. Chi tiết xem file tra cứu.

- **Gutter.**
  - Vạch rộng `floor(0.275 × line_height)` ở mép trái gutter, góc vuông, màu `version_control.added` hoặc `version_control.modified`.
  - Chỗ có dòng bị xóa: một viên thuốc màu `version_control.deleted`, cao một dòng, đặt giữa hai dòng và bị cắt một nửa ở mép trái.
- **Project panel.** Tên file tô theo `created`, `modified`, `deleted`, `conflict`, `ignored`. Thư mục lấy màu của con quan trọng nhất. Không có chữ cái.
- **Uncommitted Changes.**
  - Một tab trong vùng editor, gồm các file nối tiếp nhau. Mỗi file có header (icon, tên, đường dẫn mờ).
  - Dưới header là từng hunk bung sẵn, kèm 2 dòng ngữ cảnh. Dòng xóa nền `editor.diff_hunk.deleted.background`, dòng thêm nền `editor.diff_hunk.added.background`.
  - Thứ tự file: conflict, rồi file đã theo dõi, rồi file mới. File bị xóa thu gọn sẵn.
  - Không có thay đổi nào thì hiện chữ "No uncommitted changes" ở giữa.
- **Changes since main.** Cùng bố cục. Tab đặt tên theo nhánh làm mốc. Toolbar có ô chọn nhánh.

**Chỗ Lucerna khác Zed, và lý do:**
- Không có nút Stage, Commit hay Restore trên toolbar và trong hunk, vì bản này chỉ để xem.
- Không có nút chuyển một cột / hai cột, vì hai cột để bản sau.
- Tab chỉ đọc. Lucerna dựng tab trên excerpt list (dùng chung với Search và References), không dựng trên editor.

**Soát lại ở 5.2 (Task 6).** Đã đọc mã vẽ tab (`diff_view/render.rs`, `excerpt_list/render.rs`) đối chiếu với mô tả trên: header (icon, tên, đường dẫn mờ), nền dòng thêm/xóa theo `diff_*_background`, vạch đổi ở mép gutter, dòng xóa không có số dòng, toolbar dựng bằng `search_toolbar` / `search_bar_row` của `ui.rs`, thông báo rỗng căn giữa ("No uncommitted changes"), file bị xóa thu gọn sẵn. Không thấy chỗ nào lệch. Việc đặt cạnh Zed `git: diff` trên cùng repo để so bằng mắt (chiều cao header, màu, khoảng cách toolbar) chưa làm được vì không điều khiển được hai ứng dụng GUI từ phiên này. **Để người dùng so bằng mắt**; chỗ nào lệch thì sửa theo `docs/design-principles.md` §1.1 hoặc ghi vào đây kèm lý do.

## 3. Các cách đã cân nhắc

### 3.1 Mốc để so
Agent tự commit, nên nếu chỉ so với HEAD thì gutter và tab sẽ trống ngay sau khi agent commit.

- **A. So với merge-base của nhánh** (Zed `git::DiffBranch`). Xem được cả nhiệm vụ khi agent làm trên nhánh riêng. **Chọn**, làm thành tab riêng.
- **B. Xem từng commit** (Commit View của Zed). Để bản sau.
- **C. Mốc "đã xem tới đây".** Lucerna tự lưu một commit làm mốc. VS Code và Zed đều không có. **Bỏ**: người dùng chọn làm như Zed.

Gutter, màu Explorer và `git.openChange` đi theo setting `git.diffBase` của Zed. Mặc định là `"head"`, **người dùng chọn**.

### 3.2 Đọc git
- **Gọi lệnh `git`, như Zed. Chọn.** Binary không tăng, kết quả luôn trùng với git của agent. Cần máy có git.
- `gix`: nhanh nhất (~60 ms trên 4.400 file), nhưng binary tăng 3,3 MB và kéo theo ~440 crate. API còn ở bản 0.x, filter và submodule chưa đủ.
- `git2`: tăng 0,8 MB nhưng chậm nhất, phải build C, và libgit2 có chỗ khác hành vi của git.

Cả ba đều qua `cargo deny`. Số đo chi tiết ở file tra cứu, mục 1.

### 3.3 Hiện diff
- **Một cột, dựng trên excerpt list. Chọn.** Dùng lại view mà Search và References đang dùng.
- Hai cột như mặc định của Zed và VS Code: cần một view mới, các dòng hai bên so le nhau. Để bản sau.

## 4. Thiết kế

### 4.1 Chia crate
- **`lucerna-git` (crate mới, không phụ thuộc gpui).** Mọi thứ đụng tới git và tính diff:
  - tìm repo và tìm file `git`;
  - chạy lệnh `git`;
  - phân tích kết quả `status`;
  - tiến trình `cat-file --batch`;
  - tìm nhánh mặc định và merge-base;
  - diff theo dòng ra danh sách hunk.

  Có thể test không cần UI.
- **`lucerna-workbench`.** Phần gắn vào app:
  - `GitStore`: một entity cho mỗi cửa sổ;
  - gutter;
  - màu Explorer;
  - các tab diff;
  - lệnh, setting và theme.

### 4.2 Tìm git và repo
- **Tìm file `git`** (việc của `lucerna-workbench`; `lucerna-git` chỉ nhận đường dẫn tới file `git`, để không phụ thuộc `lucerna-lsp`):
  1. Setting `git.path` (của VS Code), nếu có.
  2. Không có thì tìm `git` trong PATH của login shell: `lucerna_lsp::env::login_shell_env()` rồi `which`. Hàm này có thể chờ tới 5 s, nên chạy nền và chạy một lần cho mỗi phiên. Tính năng git bật khi việc này xong.
  3. Trên macOS, nếu kết quả là `/usr/bin/git`, phải kiểm tra Command Line Tools đã cài chưa (`xcode-select -p` thành công) rồi mới chạy. Nếu chưa cài thì coi như không có git.
  4. Không có git: tắt mọi tính năng git, status bar báo một lần "Git not found" kèm gợi ý cài.
- **Tìm repo:** dùng lại cách `git_branch.rs` đang làm (tìm `.git` từ thư mục gốc đi lên, đi theo `gitdir:` của worktree), rồi hỏi `git rev-parse --show-toplevel --git-dir --git-common-dir` để có:
  - gốc worktree;
  - thư mục git riêng của worktree;
  - thư mục git chung (chứa `refs`).

  Không có repo thì tắt tính năng, không báo gì.
- **Restricted Mode:** thư mục chưa tin cậy (`trust.rs`) thì **không chạy git**, giống VS Code. Lý do: config cục bộ của repo (`core.fsmonitor`, `filter.*.clean`) có thể chạy lệnh tùy ý ngay cả khi chỉ đọc trạng thái. Tên nhánh trên thanh tiêu đề vẫn hiện, vì nó đọc file trực tiếp. Khi người dùng tin cậy thư mục thì bật git ngay, không cần mở lại.
  - **Git cũng bật hộp thoại tin cậy** (người dùng chọn). Mở một workspace nằm trong repo mà thư mục chưa được tin cậy thì hộp thoại tin cậy của `trust.rs` tự hiện, giống như khi mở file có language server. Hộp thoại chỉ tự hiện một lần cho mỗi phiên, như hiện nay. Nút "Restricted Mode" trên thanh tiêu đề vẫn mở lại được.
  - Nội dung hộp thoại phải nhắc tới Git, không chỉ language server, ví dụ "Language servers and Git run only in trusted folders".
  - Nếu người dùng bỏ qua hộp thoại: gutter và Explorer không có gì, còn tab diff hiện một dòng giải thích kèm nút mở hộp thoại tin cậy.

### 4.3 Chạy lệnh `git`
- Lệnh nào cũng có `-c core.fsmonitor=false -c core.hooksPath=/dev/null --no-optional-locks --no-pager` và `GIT_TERMINAL_PROMPT=0`, như Zed. `--no-optional-locks` giúp Lucerna không giành khóa `index.lock` với agent.
- **Hàng đợi việc:** mỗi repo có một hàng đợi chạy tuần tự ở luồng nền. Việc có khóa ("quét toàn bộ", "làm mới các đường dẫn") thay thế việc cùng khóa đang chờ. Cách này lấy theo Zed.
- **Trạng thái:** `git status --porcelain=v2 -z --untracked-files=all --no-renames [-- <paths>]`. Rename được hiện thành xóa cộng thêm, như Zed.
- **Nội dung file ở mốc:** một tiến trình `git cat-file --batch` sống suốt phiên. Gửi `‹rev›:‹path›`, nhận blob. Tiến trình chết thì khởi động lại khi cần.
- **Nhánh mặc định:** thử lần lượt `refs/remotes/upstream/HEAD`, `refs/remotes/origin/HEAD`, rồi nhánh cục bộ `main`, rồi `master`. Không có nhánh nào thì coi như "không có nhánh mặc định".
  - Vì remote được thử trước, nên khi agent commit thẳng lên `main`, mốc là `origin/main`. Tab vẫn hiện được các commit chưa push.
  - Chỉ khi repo không có remote mà agent commit thẳng lên `main` thì merge-base là HEAD, và tab giống hệt Uncommitted Changes. Zed cũng vậy. Mốc kiểu "đã xem tới đây" (cách C) là việc của bản sau, nếu cần.
- **Merge-base:** `git merge-base ‹ref› HEAD`. Danh sách file đổi so với merge-base là `git diff --name-status -z --no-renames ‹merge-base›` (so cây của merge-base với bản trên đĩa), cộng các file chưa theo dõi lấy từ status.
- **Lỗi:** lệnh lỗi hoặc chạy quá 10 s thì giữ trạng thái cũ, ghi log, và status bar báo một lần. Lỗi không bao giờ làm mất màu hay vạch đang hiện, trừ khi repo không còn.

### 4.4 Mốc so: `git.diffBase`
- `"head"` (mặc định, như Zed): mốc là HEAD.
- `"defaultBranch"`: mốc là merge-base của HEAD với nhánh mặc định. Không tìm được nhánh mặc định thì dùng HEAD.
- Mốc này áp dụng cho gutter, màu Explorer và `git.openChange`.
- Hai tab có mốc riêng: Uncommitted Changes luôn so với HEAD, Changes since ‹nhánh› luôn so với merge-base của nhánh đang chọn.
- HEAD hoặc nhánh mặc định đổi (agent commit, rebase, fetch) thì tính lại merge-base.

### 4.5 Explorer
- Tô tên file theo trạng thái so với mốc `git.diffBase`, theo thứ tự ưu tiên của Zed: conflict, deleted, modified, created (gồm cả file chưa theo dõi), ignored. File đã stage hay chưa đều tính là đã đổi.
- Thư mục lấy màu của con quan trọng nhất, cộng dồn qua mọi cấp như Zed. Mỗi lần trạng thái đổi thì tính lại phần tổng hợp này ở luồng nền.
- Khóa màu theme của Zed: `created`, `modified`, `deleted`, `conflict`, `ignored`. Theme không có khóa nào thì dùng màu mặc định `gitDecoration.*` của VS Code (xem file tra cứu, mục 4).
- Không hiện chữ cái. `explorer.decorations.badges` mặc định là `false`, **lệch VS Code** để giao diện giống Zed. Khi bật thì hiện chữ cái kiểu VS Code ở mép phải hàng: `M`, `U`, `D`, `!`.
- Dấu "agent vừa ghi" đang có (`explorer.decorations.externalChanges`) vẫn giữ: chữ đậm và chấm tròn. Màu git chỉ đổi màu chữ, nên hai thứ cùng hiện được.
- Explorer luôn ẩn file bị gitignore (`worktree.rs`, `.git_ignore(true)`), nên bản này không dùng tới màu `ignored`. Vẫn đọc khóa này để dùng sau.
- Danh sách file có màu lấy từ `git status`, tức nội dung trên đĩa. File sửa trong Lucerna mà chưa lưu thì có vạch ở gutter nhưng chưa đổi màu trong Explorer cho tới khi lưu. Tôi chấp nhận chỗ lệch này, vì người dùng Lucerna chủ yếu đọc thay đổi do agent ghi lên đĩa. Tab diff cũng vậy: danh sách file theo đĩa, nội dung theo buffer.

### 4.6 Gutter
- **Mốc và thời điểm tính:**
  - So nội dung **đang có trong buffer**, kể cả phần chưa lưu, với blob ở mốc `git.diffBase`. Giống cả Zed và VS Code.
  - Diff lại cả file trong task nền, debounce 50 ms sau lần sửa cuối.
  - Blob ở mốc chỉ nạp lại khi mốc đổi, hoặc khi chính file đó đổi trạng thái.
  - Kết quả là danh sách hunk theo dòng: thêm, sửa, xóa. Giữ kết quả cũ cho tới khi có kết quả mới, để vạch không nhấp nháy.
- **Diff:** dùng `similar` theo dòng (crate đã có trong workspace). Bỏ qua khác biệt chỉ nằm ở ký tự xuống dòng (CRLF/LF), để file bị `core.autocrlf` đổi không bị tô cả file.
- **Khi nào không có vạch:**
  - file chưa theo dõi, hoặc file nằm ngoài repo (như Zed `tracked_files` và VS Code);
  - ngoại lệ: với `"defaultBranch"`, file đã theo dõi ở HEAD nhưng chưa có ở merge-base (agent tạo rồi commit trên nhánh) được so với nội dung rỗng, nên mọi dòng là dòng thêm, và Explorer tô màu `created`. Giống Zed;
  - file nhị phân;
  - file lớn hơn 8 MB (ngưỡng do tôi đặt, xem §6).
- **Vẽ** theo Zed:
  - Vạch ở mép trái gutter, bên trái cột icon diagnostics, rộng `floor(0.275 × line_height)`.
  - Dòng thêm màu `version_control.added`, dòng sửa màu `version_control.modified`.
  - Dòng xóa: viên thuốc màu `version_control.deleted` ở ranh giới hai dòng.
  - Màu trộn lên `editor.background`, như Zed. Theme thiếu khóa thì dùng màu `editorGutter.*Background` mặc định của VS Code.
- **Bấm vào vạch:** mở diff của file (§4.8), cuộn tới hunk đó. Con trỏ chuột đổi thành bàn tay khi rê lên vạch.
- **Settings** (VS Code):
  - `scm.diffDecorations`: `"all"` (mặc định) hoặc `"gutter"` thì hiện vạch, `"none"` thì tắt. `"overview"` và `"minimap"` được chấp nhận nhưng chưa vẽ gì, vì bản này chưa có vạch trên minimap.
  - `scm.diffDecorationsGutterAction`: `"diff"` (mặc định) hoặc `"none"` (bấm vào vạch không làm gì).

### 4.7 Tab diff (Uncommitted Changes và Changes since ‹nhánh›)
- **Hai loại tab mới trong `TabItem`, mỗi loại tối đa một tab trong mọi nhóm.** Chúng giống Search và References: không phải preview, không chia đôi được. Có lưu vào workspace (như Search) để mở lại khi khởi động. Tab theo nhánh lưu luôn tên nhánh đang chọn.
- **Tiêu đề tab:**
  - "Uncommitted Changes";
  - "Changes since ‹tên nhánh›", ví dụ "Changes since main".
- **Thanh trên cùng.** Dùng `search_toolbar` / `search_bar_row` trong `ui.rs`, giống References:
  - bên trái: số file và tổng số dòng `+N −M`;
  - bên phải: nút hunk trước và hunk sau, thu gọn tất cả / bung tất cả, làm mới;
  - tab theo nhánh có thêm nút tên nhánh. Bấm vào mở picker chọn nhánh (local và remote, lấy từ `git for-each-ref`), dùng component picker sẵn có.
- **Nội dung: excerpt list, mở rộng cho diff.**
  - Mỗi file có một header như Search, cộng thêm `+N −M`. File mới và file bị xóa có nhãn nhỏ "new" / "deleted".
  - Mỗi hunk là một excerpt kèm 2 dòng ngữ cảnh mỗi phía, như Zed. Hunk gần nhau thì gộp lại.
  - Mỗi dòng thuộc một trong ba loại: ngữ cảnh, thêm, xóa.
    - Dòng thêm: nền `editor.diff_hunk.added.background`, cộng một vạch `version_control.added` ở gutter.
    - Dòng xóa: nền `editor.diff_hunk.deleted.background`, cộng một vạch `version_control.deleted`. Gutter không có số dòng, giống Zed.
    - Theme thiếu khóa thì dùng `diffEditor.insertedLineBackground` / `removedLineBackground` mặc định của VS Code.
  - Tô màu cú pháp cho cả dòng xóa: lấy từ nội dung blob ở mốc, không lấy từ file trên đĩa.
  - Thứ tự file: conflict, rồi file đã theo dõi, rồi file mới, như Zed. Trong mỗi nhóm thì xếp theo đường dẫn.
  - File bị xóa thu gọn sẵn. File nhị phân hoặc quá lớn (§6) chỉ có header, kèm dòng "Binary file" hoặc "File too large to diff".
- **Hành vi:**
  - Bấm vào một dòng, hoặc nhấn `Enter`: mở file ở dòng đó. Với dòng xóa thì mở ở dòng ngay sau chỗ bị xóa.
  - Nút "open file" trên header: mở file. Với file bị xóa thì nút bị tắt.
  - Bàn phím di chuyển giữa các dòng như Search.
  - Alt+F5 / Shift+Alt+F5 nhảy tới hunk sau / trước (§4.9).
- **Nguồn nội dung:**
  - File đang mở trong Lucerna: dùng buffer, kể cả phần chưa lưu.
  - File chưa mở: đọc file trên đĩa.
  - Mọi việc chạy nền, tối đa 16 file cùng lúc như Zed. Danh sách file hiện ngay, hunk của từng file đổ vào dần, như Search đang làm với kết quả.
- **Tự cập nhật:**
  - Trạng thái repo đổi: dựng lại danh sách file.
  - Một file đổi (trên đĩa hoặc trong buffer): chỉ tính lại file đó.
  - Dòng đang chọn và vị trí cuộn được giữ, neo theo đường dẫn và số dòng như excerpt list đang làm.
- **Trạng thái rỗng**, chữ ở giữa như Zed:
  - "No uncommitted changes";
  - "No changes since ‹nhánh›";
  - Không có nhánh mặc định: "No default branch found", kèm nút chọn nhánh.
  - Không có repo / không có git / Restricted Mode: nói rõ lý do bằng một dòng.

### 4.8 Diff của một file (`git.openChange`)
- Mở một tab "‹tên file› (Changes)", dùng cùng view với §4.7 nhưng chỉ có một file và **hiện toàn bộ file**, như `git.file_diff.show_full_file` của Zed. Mỗi file có tối đa một tab loại này, không phải preview.
- Mốc là `git.diffBase`.
- File chưa theo dõi: mọi dòng là dòng thêm. File không có thay đổi: tab hiện toàn bộ file, không có dòng tô màu.
- Thanh trên cùng có hunk trước/sau và nút "Open File" (`git.openFile`).
- Mở được từ:
  - Command Palette, cho file ở tab đang active;
  - bấm vào vạch ở gutter;
  - lệnh `git.openChange` gắn vào menu chuột phải của Explorer, khi có menu đó.

### 4.9 Lệnh và phím

| Lệnh | Tiêu đề | Phím mặc định | Ghi chú |
|---|---|---|---|
| `git.viewChanges` | Git: Open Changes (Uncommitted) | không có | ID của VS Code |
| `lucerna.git.viewBranchChanges` | Git: Open Changes Since Branch | không có | VS Code không có lệnh tương ứng. Tương đương Zed `git::DiffBranch` |
| `git.openChange` | Git: Open Changes | không có | VS Code |
| `git.openFile` | Git: Open File | không có | VS Code. Dùng trong tab diff |
| `git.refresh` | Git: Refresh | không có | VS Code. Quét lại toàn bộ |
| `workbench.action.editor.nextChange` / `previousChange` | Go to Next/Previous Change | Alt+F5 / Shift+Alt+F5 | VS Code. Trong editor: đưa con trỏ tới đầu hunk kế tiếp, quay vòng qua cuối file. Trong tab diff: nhảy giữa các hunk qua mọi file |
| Vim `]c` / `[c` | | | Như Vim và Zed. Dùng chung lệnh trên |

### 4.10 Settings mới (`default-settings.json`)

| Khóa | Mặc định | Nguồn |
|---|---|---|
| `git.enabled` | `true` | VS Code |
| `git.path` | `null` | VS Code |
| `git.decorations.enabled` | `true` | VS Code. Tắt màu Explorer |
| `git.diffBase` | `"head"` | Zed `git.diff_base` (`"head"` / `"defaultBranch"`) |
| `git.statusLimit` | `10000` | VS Code |
| `explorer.decorations.colors` | `true` | VS Code |
| `explorer.decorations.badges` | `false` | VS Code. Mặc định lệch, xem §4.5 |
| `scm.diffDecorations` | `"all"` | VS Code |
| `scm.diffDecorationsGutterAction` | `"diff"` | VS Code |

Đổi setting thì áp dụng ngay, không cần khởi động lại.

### 4.11 Theme
Thêm vào `UiColors` các khóa của theme Zed. Mỗi khóa có màu dự phòng cho dark và light, lấy theo mặc định của VS Code (xem file tra cứu), đặt trong `UiColors::fallback`:
- `version_control.added`, `version_control.modified`, `version_control.deleted`;
- `created`, `modified`, `deleted`, `conflict`, `ignored`;
- `editor.diff_hunk.added.background`, `editor.diff_hunk.deleted.background`.

### 4.12 Làm mới
Lucerna làm mới theo Zed (nhanh), không theo VS Code (chờ 1 s và chờ cửa sổ được focus). Lý do: người dùng muốn thấy ngay khi agent ghi file.

- **File trong worktree đổi** (watcher của `lucerna-project` đã gom mỗi 100 ms): xếp một việc "làm mới các đường dẫn" chạy `git status -- <paths>`, rồi tính lại diff gutter của những file đang mở trong số đó.
- **Thư mục git đổi** (`index`, `HEAD`, `refs/**`, `packed-refs`, `MERGE_HEAD`): xếp một việc "quét toàn bộ". Việc này tính lại trạng thái, mốc và merge-base, rồi nạp lại blob ở mốc của các file đang mở.
  - Bỏ qua `objects`, `hooks`, `lfs`, `logs`, `rebase-*`, `sequencer`, `fsmonitor--daemon`, `*.lock`, `COMMIT_EDITMSG`, `FETCH_HEAD`, `ORIG_HEAD`, `gc.pid`.
  - Watcher hiện bỏ qua mọi thứ trong `.git`. Thư mục git của worktree và thư mục git chung có thể nằm ngoài thư mục gốc. Vì vậy `GitStore` cần một watcher riêng (dùng `notify` như `lucerna-project::watch`) trên hai thư mục đó, không đệ quy vào `objects`.
- **Commit của agent:** `index` và `refs/heads/...` đổi, dẫn tới quét toàn bộ, và vạch ở gutter biến mất (với mốc HEAD). Đó là đúng ý.
- **Quá nhiều mục:** nếu status có hơn `git.statusLimit` mục:
  - ngừng tự làm mới cho repo đó, giữ kết quả cuối;
  - status bar báo một lần "Too many changes in this repository; automatic Git updates are off. Run Git: Refresh to update.";
  - tab diff vẫn mở được, nhưng chỉ cập nhật khi chạy `git.refresh`.

  Giống VS Code `isRepositoryHuge`.

## 5. Hiệu năng

| Việc | Ngưỡng |
|---|---|
| Lệnh `git` và phép diff trên luồng UI | không bao giờ. Luồng UI chỉ nhận kết quả đã tính xong |
| Mở workspace: màu Explorer hiện | ≤ 500 ms sau khi cây đã quét xong, với repo 10.000 file và 100 file đổi. Explorer hiện ngay, màu đến sau |
| Agent ghi 1 file → vạch gutter và màu Explorer cập nhật | ≤ 300 ms sau sự kiện của watcher |
| Agent ghi 500 file trong 1 s | UI không rớt khung hình. Việc làm mới gộp lại, tối đa một việc "làm mới các đường dẫn" đang chờ |
| Agent commit → gutter và Explorer cập nhật | ≤ 1 s, với repo 10.000 file |
| Gõ phím trong file 10.000 dòng | diff gutter mỗi lần ≤ 20 ms ở luồng nền (đo được ~6 ms với 13.000 dòng), debounce 50 ms. Không trễ khung hình |
| Mở tab Uncommitted Changes với 200 file đổi | header hiện ≤ 200 ms, hunk của các file trong khung nhìn ≤ 500 ms |
| Bộ nhớ | blob ở mốc chỉ giữ cho file đang mở và file đang hiện trong tab diff. Giải phóng khi đóng |

Benchmark trên repo Zed ở revision đang pin (4.401 file, 98 MB mã nguồn) và repo Lucerna. Ghi kết quả vào file tra cứu.

## 6. Trường hợp biên
- **File lớn hoặc nhị phân.**
  - File lớn hơn 8 MB không diff. Ngưỡng do tôi đặt: VS Code để 50 MB, Zed không giới hạn, nhưng 8 MB đủ giữ diff dưới vài chục ms.
  - File được coi là nhị phân nếu có byte NUL trong 8 KB đầu, giống cách git đoán.
  - Hai loại này không có vạch ở gutter. Trong tab diff chúng chỉ có header.
- **Buffer chưa lưu:** gutter và tab diff dùng nội dung buffer. Bấm "Revert" trong Lucerna thì vạch trở về theo bản trên đĩa.
- **Xung đột merge** (status `UU`, `AA`, ...):
  - Explorer tô màu `conflict`.
  - Tab diff xếp file đó lên đầu và hiện diff so với HEAD (đoạn có dấu `<<<<<<<` sẽ thành dòng thêm).
  - Không có giao diện giải quyết xung đột.
- **HEAD tách rời (detached), repo chưa có commit nào:** không có HEAD thì mọi file là file mới. Gutter không có vạch, vì các file đều chưa theo dõi.
- **Đổi nhánh, rebase:** quét toàn bộ (§4.12). Tab diff giữ vị trí nếu file còn trong danh sách.
- **Symlink:** theo git: blob của symlink là đường dẫn đích, không diff nội dung, chỉ hiện header.
- **Thư mục gốc là thư mục con của repo:** chỉ hiện các file nằm dưới thư mục gốc. Đường dẫn chuyển đổi giữa "tương đối với repo" và "tương đối với workspace".
- **Chế độ một file** (`2026-11-13-single-file-open-design.md`): gutter vẫn có nếu file nằm trong repo và thư mục được tin cậy. Không có Explorer nên không có màu.
- **Encoding và xuống dòng:** blob được giải mã bằng `lucerna_text::decode_file`, cùng đường mà Lucerna dùng để mở file (BOM, UTF-8, rồi chardetng). Hàm này cũng đổi CRLF thành LF. Vì buffer đã ở dạng LF, quy tắc bỏ qua CRLF ở §4.6 chỉ cần chuẩn hóa phía blob.

## 7. Test
- **`lucerna-git`** (dùng repo tạm, tạo bằng lệnh `git` trong test):
  - phân tích `status --porcelain=v2 -z`, gồm conflict, file mới, xóa, đường dẫn có dấu cách và ký tự Unicode;
  - `cat-file --batch`: đọc nhiều blob, blob không tồn tại, tiến trình chết rồi khởi động lại;
  - tìm nhánh mặc định theo đúng thứ tự, merge-base, danh sách file đổi so với merge-base;
  - hunk đúng loại thêm / sửa / xóa, CRLF không bị tính là thay đổi, phát hiện nhị phân;
  - không có git thì báo lỗi gọn, không panic;
  - không chạy hook và fsmonitor (repo test cài hook ghi ra file, kiểm tra file đó không xuất hiện).
- **`lucerna-workbench/tests/`:**
  - Explorer: màu file và thư mục theo trạng thái; tắt bằng `git.decorations.enabled` và `explorer.decorations.colors`; chữ cái khi bật `explorer.decorations.badges`;
  - gutter: sửa buffer thì vạch đổi; lưu, rồi commit từ bên ngoài, thì vạch mất; `git.diffBase` `"defaultBranch"` thì vạch còn sau khi commit;
  - Alt+F5 / Shift+Alt+F5 quay vòng giữa các hunk; `]c` / `[c` khi bật Vim;
  - tab Uncommitted Changes: thứ tự file, dòng xóa và dòng thêm, bấm dòng thì mở đúng vị trí, trạng thái rỗng, giữ dòng đang chọn khi một file đổi;
  - tab Changes since: gồm cả các commit trên nhánh lẫn phần chưa commit; đổi nhánh qua picker; không có nhánh mặc định;
  - Restricted Mode: không có lệnh `git` nào chạy; mở workspace trong repo chưa tin cậy thì hộp thoại tin cậy tự hiện; tin cậy thư mục thì tính năng bật;
  - vượt `git.statusLimit`: ngừng tự làm mới, `git.refresh` vẫn chạy.
- **Benchmark** (mục 5), chạy tay, không đưa vào CI.

## 8. Để người làm plan tự quyết
- Cách `GitStore` trao trạng thái cho Explorer và CodeView: snapshot bất biến kèm số thế hệ, theo nguyên tắc "luồng UI chỉ đọc snapshot".
- Cấu trúc dữ liệu cho trạng thái theo đường dẫn và phần tổng hợp cho thư mục (cây có tóm tắt như Zed, hoặc map cộng tổng hợp tính sẵn), miễn đạt §5.
- Cách mở rộng excerpt list cho dòng xóa và nguồn highlight lấy từ blob, sao cho Search và References không bị ảnh hưởng.
- Cách chia việc thành các bước merge được. Gợi ý thứ tự:
  1. `lucerna-git` và `GitStore`;
  2. màu Explorer;
  3. gutter;
  4. tab Uncommitted Changes;
  5. diff một file;
  6. tab Changes since.

## 9. Quyết định do tôi đặt (người dùng chưa duyệt từng mục)
- `explorer.decorations.badges` mặc định `false`.
- Ngưỡng 8 MB cho diff. Các ngưỡng hiệu năng ở mục 5.
- Bấm vào vạch gutter thì mở tab diff của file, thay cho việc bung hunk hay mở khung peek.
- Tab diff có lưu vào workspace.
- Tên lệnh riêng `lucerna.git.viewBranchChanges`.
