# Review các flow cơ bản trước release: plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ra một báo cáo lỗi đã kiểm chứng, xếp theo mức độ, cho các flow cơ bản của Lucerna trước v0.1.0.

**Architecture:** Năm agent review chỉ đọc code, chạy song song, mỗi agent một vùng flow (Task 1–5). Mỗi lỗi chúng báo được một agent kiểm chứng độc lập đọc lại code (Task 6). Lỗi đứng vững được gộp vào một báo cáo và commit (Task 7). Không sửa code sản phẩm.

**Tech Stack:** Rust, GPUI. Công cụ: Agent tool (subagent `general-purpose`), Read, Grep, Bash chỉ để đọc.

**Spec:** `docs/superpowers/specs/2026-11-15-release-flow-review-design.md`

## Global Constraints

- Chỉ đọc code: không build, không `cargo test`, không tạo worktree, không sửa file trong `crates/`.
- Phạm vi gồm bốn nhóm flow trong spec. Vim, LSP, extension, Markdown preview, diff view, hiệu năng, giao diện, docs và license nằm ngoài phạm vi, **trừ khi** chúng làm hỏng một flow cơ bản.
- Mức độ: **P0** mất hoặc hỏng dữ liệu, crash. **P1** flow cơ bản không làm được. **P2** sai nhưng có cách khác. **P3** nhỏ.
- Trích code vào báo cáo thật ngắn, chỉ đủ chỉ ra lỗi.
- Commit theo đường dẫn cụ thể (`git commit -- <paths>`), trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Báo cáo: `docs/superpowers/reviews/2026-11-15-release-flow-review.md`.

## Review Focus

Năm loại đầu vào dễ làm hỏng người dùng nhất. Mỗi dòng được gán vào checklist của task phụ trách:

1. **Agent ghi file trong lúc buffer đang có thay đổi chưa lưu.** Người dùng mong không mất bên nào, và không ghi đè mà không hỏi. (Task 1)
2. **File CRLF, có BOM, không phải UTF-8, hoặc không có newline cuối.** Mong lưu lại đúng y như cũ, trừ phần người dùng sửa. (Task 1)
3. **Đường dẫn có dấu cách, ký tự Unicode (tiếng Việt, NFD trên macOS), symlink, hoặc là file ẩn.** Mong mở, đổi tên, xoá và lưu đều đúng. (Task 2, 3)
4. **Thoát app, hoặc đóng cửa sổ, khi một lần lưu hay một thao tác file đang dở.** Mong không mất dữ liệu và không còn file tạm. (Task 2)
5. **Thư mục đang mở hoặc file đang mở bị xoá, đổi tên hay mount lại từ bên ngoài.** Mong không crash, tab báo rõ trạng thái. (Task 3, 4)

---

## Định dạng phát hiện (mọi task review dùng chung)

Mỗi agent review trả về một danh sách. Mỗi mục có đủ các trường sau:

```
- id: <T{task}-{n}>, ví dụ T1-3
  severity: P0 | P1 | P2 | P3
  flow: <tên flow trong spec>
  location: <crate/src/file.rs:dòng>
  summary: <một câu nói lỗi là gì>
  repro: <các bước cụ thể của người dùng, hoặc chuỗi sự kiện, dẫn tới lỗi>
  evidence: <đường đi của code chứng minh, kèm file:dòng>
  fix_hint: <một câu>
  confidence: high | medium
```

Không báo mục `low`. Không báo vấn đề về style, đặt tên hay hiệu năng. Thêm một mục `not_checked:` liệt kê những gì đọc code không kiểm được (ví dụ: focus, IME, thứ tự vẽ).

## Brief chung cho agent review

Mỗi Task 1–5 gửi cho một subagent `general-purpose`. Brief gồm:
- đoạn này,
- Global Constraints,
- Định dạng phát hiện,
- phần "Flow" và "Bắt đầu đọc từ" của task đó.

> Bạn đang review code Lucerna, một code editor Rust/GPUI ở `/Users/duykhanh/Projects.nosync/lucerna`, trước bản phát hành đầu tiên. Chỉ đọc code, không sửa, không build, không chạy test. Với mỗi flow dưới đây, lần theo code từ thao tác của người dùng hoặc sự kiện file system đến kết quả cuối (ghi đĩa, cập nhật view, cập nhật cây thư mục). Tìm các loại lỗi sau:
> - trường hợp biên chưa xử lý,
> - race condition giữa watcher, ghi file và thao tác người dùng,
> - `unwrap`, `expect` và index/slice có thể panic với dữ liệu thật,
> - lỗi bị bỏ qua (`let _ =`, `.ok()`, `unwrap_or_default`) mà đáng ra phải báo cho người dùng,
> - trạng thái tab, buffer và cây thư mục lệch nhau.
>
> Đọc cả test hiện có để biết điều gì đã được kiểm. Chỉ báo lỗi bạn chỉ ra được bằng đường đi cụ thể trong code. Tài liệu hành vi mong đợi nằm ở `docs/features.md` và `docs/settings.md`.

---

### Task 1: Lưu file và conflict với thay đổi trên đĩa

**Flow:**
- Sửa và lưu: ghi atomic; giữ encoding, BOM và kiểu xuống dòng; `files.trimTrailingWhitespace`, `files.insertFinalNewline`, `files.trimFinalNewlines`, `files.eol`, `files.autoSave`.
- Save All, Save Without Formatting, Revert.
- Undo và redo (gồm cursor undo).
- File đổi trên đĩa khi buffer đang có thay đổi chưa lưu: conflict dot, conflict bar, hỏi trước khi ghi đè.
- Review Focus 1 và 2.

**Bắt đầu đọc từ:**
- `lucerna-text/src/{write,load,reload,buffer,history,patch,diff}.rs`
- `lucerna-workbench/src/{save,documents,conflict_bar,editing}.rs`
- `lucerna-workbench/src/code_view/transact.rs`
- `lucerna-editor/src/merge.rs`
- `lucerna-text/tests/`

- [ ] **Step 1:** Gửi brief cho subagent, với description "Review save and conflict flows".
- [ ] **Step 2:** Kiểm tra kết quả trả về đúng Định dạng phát hiện và mọi `location` trỏ tới file có thật (`test -f`). Mục nào thiếu trường thì gửi lại cho chính agent đó (SendMessage) để bổ sung.

### Task 2: Explorer, đóng tab và thoát app

**Flow:**
- Explorer: New File, New Folder, Rename (tab đang mở đi theo), Delete to Trash, Delete Permanently, copy và paste file.
- Đóng tab, đóng group, đóng cửa sổ và thoát app khi còn file chưa lưu (hộp thoại hỏi; các nút Save, Don't Save, Cancel).
- Close Folder khi còn file chưa lưu.
- Review Focus 3 (phần thao tác file) và 4.

**Bắt đầu đọc từ:**
- `lucerna-workbench/src/explorer/{commands,clipboard,model,view}.rs`
- `lucerna-project/src/fs_ops.rs`
- `lucerna-workbench/src/editor_groups/{model,view}.rs`
- `lucerna-workbench/src/{app,app_menu,workbench}.rs` (phần quit/close)
- `lucerna-project/tests/fs_ops.rs`

- [ ] **Step 1:** Gửi brief cho subagent, với description "Review explorer and close flows".
- [ ] **Step 2:** Kiểm tra định dạng và đường dẫn như Task 1, Step 2.

### Task 3: Mở folder, mở file và khôi phục workspace

**Flow:**
- `luc .`, `luc <file>`, `luc <đường dẫn không tồn tại>`.
- Mở từ Finder hoặc Dock, Open With (file trong folder đang mở thì mở thành tab, còn lại mở cửa sổ mới).
- Khôi phục workspace (`window.restoreWindows`), Welcome page, Open Recent, Close Folder (Cmd+K F).
- File lớn (trên 20 MB hoặc trên 300 nghìn dòng), dòng dài 5 MB, file nhị phân, UTF-16, Latin-1.
- Workspace đã lưu trỏ tới folder hoặc file không còn tồn tại.
- Review Focus 3 (phần mở) và 5 (phần folder biến mất).

**Bắt đầu đọc từ:**
- `lucerna/src/{main,target}.rs`
- `lucerna-workbench/src/{startup,open_commands,open_files,open_requests,recent,recent_picker,welcome,workspace_state,app_state,app_state_file}.rs`
- `lucerna-text/src/load.rs`
- `lucerna-workbench/src/documents.rs` (phần mở)

- [ ] **Step 1:** Gửi brief cho subagent, với description "Review open and restore flows".
- [ ] **Step 2:** Kiểm tra định dạng và đường dẫn như Task 1, Step 2.

### Task 4: Theo dõi thay đổi của agent

**Flow:**
- Reload khi file trên đĩa đổi: ghi atomic (rename đè lên), xoá rồi tạo lại, `git checkout` hàng nghìn file.
- Giữ vị trí cuộn, màu cú pháp và fold sau khi reload.
- Đánh dấu file vừa đổi trên cây thư mục (10 giây).
- Màu git trong Explorer và gutter cập nhật theo các lệnh `git`.
- Cây thư mục tự cập nhật, tôn trọng `.gitignore` và `files.exclude`.
- Review Focus 5 (phần file đang mở biến mất).

**Bắt đầu đọc từ:**
- `lucerna-project/src/{watch,worktree,coalesce,snapshot,ignored}.rs`
- `lucerna-git/src/{watch,status,repo,hunks}.rs`
- `lucerna-workbench/src/git/{mod,wiring,job,rollup}.rs`
- `lucerna-text/src/reload.rs`
- `lucerna-workbench/src/documents.rs` (phần reload)
- `lucerna-project/tests/`, `lucerna-git/tests/`

- [ ] **Step 1:** Gửi brief cho subagent, với description "Review agent-follow flows".
- [ ] **Step 2:** Kiểm tra định dạng và đường dẫn như Task 1, Step 2.

### Task 5: Điều hướng hằng ngày

**Flow:**
- Quick Open: tên file, `>` cho lệnh, `:` cho đi tới dòng (kể cả dòng vượt cuối file).
- Tìm trong project (Cmd+Shift+F), kể cả kết quả trong file vừa bị xoá.
- Tìm trong file (Cmd+F), kể cả regex sai.
- Split editor (Cmd+\), tab và preview tab.
- Terminal: mở, đóng, nhiều tab, shell thoát.
- Gõ IME tiếng Việt trong editor và các ô nhập.

**Bắt đầu đọc từ:**
- `lucerna-workbench/src/quick_open/`, `lucerna-workbench/src/search_view/`, `lucerna-project/src/{search,fuzzy}.rs`
- `lucerna-workbench/src/{find_widget,goto,text_input}.rs`
- `lucerna-workbench/src/editor_groups/`
- `lucerna-workbench/src/terminal/{panel,view,commands}.rs`, `lucerna-terminal/src/{terminal,process}.rs`
- `lucerna-workbench/src/code_view/input.rs`

- [ ] **Step 1:** Gửi brief cho subagent, với description "Review navigation flows".
- [ ] **Step 2:** Kiểm tra định dạng và đường dẫn như Task 1, Step 2.

> Task 1–5 độc lập với nhau: gửi cả năm trong cùng một message để chạy song song.

### Task 6: Kiểm chứng từng phát hiện

**Interfaces:**
- Consumes: danh sách phát hiện từ Task 1–5, theo Định dạng phát hiện.
- Produces: với mỗi `id`, một kết quả `verdict: confirmed | rejected | downgraded`, kèm `severity` cuối cùng và một câu lý do.

- [ ] **Step 1: Gộp và bỏ trùng.** Hai phát hiện cùng nguyên nhân gốc (cùng `location` hoặc cùng đường đi của code) được gộp thành một, giữ `id` nhỏ hơn và mức độ cao hơn.
- [ ] **Step 2: Gửi agent kiểm chứng.** Mỗi task review có phát hiện thì gửi một subagent `general-purpose`, chạy song song. Brief của agent kiểm chứng chỉ gồm `summary`, `location` và `repro` của từng mục, **không** gồm `evidence` hay `confidence` để agent không bị neo vào kết luận cũ. Yêu cầu agent đọc code và trả lời ba câu:
  - Với code hiện tại, các bước repro có dẫn tới lỗi này không?
  - Có chỗ nào khác trong code đã chặn lỗi không?
  - Mức độ đúng là gì?
- [ ] **Step 3: Đối chiếu.** Mục `rejected` bị loại. Mục `downgraded` lấy mức độ mới. Nếu agent review và agent kiểm chứng không đồng ý về một mục P0 hoặc P1, tự đọc `location` đó để quyết định, và ghi lý do.

### Task 7: Viết báo cáo và commit

**Files:**
- Create: `docs/superpowers/reviews/2026-11-15-release-flow-review.md`

- [ ] **Step 1: Viết báo cáo**, gồm các phần:
  1. **Tóm tắt:** số lỗi theo mức độ, và các lỗi P0 và P1 mỗi lỗi một dòng.
  2. **Danh sách lỗi:** mỗi lỗi một mục `### <id> [P?] <summary>`, kèm flow, `location` (dạng link markdown tương đối), cách tái hiện, hướng sửa gợi ý. Sắp từ P0 đến P3, trong cùng mức độ thì theo thứ tự nhóm flow.
  3. **Đã loại:** mỗi mục bị `rejected` một dòng, kèm lý do, để lần review sau không phải dò lại.
  4. **Cần thử tay trước release:** gộp các `not_checked` của Task 1–5 thành một checklist có ô đánh dấu.
- [ ] **Step 2: Kiểm tra link.** Mọi `location` trong báo cáo trỏ tới file có thật:
  `grep -o '](\.\./\.\./\.\./crates/[^):]*' docs/superpowers/reviews/2026-11-15-release-flow-review.md | sed 's/](\.\.\/\.\.\/\.\.\///' | sort -u | while read f; do test -f "$f" || echo "MISSING $f"; done`
  Kết quả mong đợi: không in gì.
- [ ] **Step 3: Commit.**

```bash
git add -- docs/superpowers/reviews/2026-11-15-release-flow-review.md docs/superpowers/plans/2026-11-15-release-flow-review.md
git commit -m "docs(review): basic-flow review before v0.1.0" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>" -- docs/superpowers/reviews/2026-11-15-release-flow-review.md docs/superpowers/plans/2026-11-15-release-flow-review.md
```

- [ ] **Step 4:** Gửi người dùng phần tóm tắt và link báo cáo. Hỏi họ chọn những lỗi nào phải sửa trước release.
