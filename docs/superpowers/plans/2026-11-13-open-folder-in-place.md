# Open Folder đổi tại chỗ — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Open Folder, Open Recent (Enter) và Close Folder đổi root ngay trong cửa sổ hiện tại thay vì khởi động tiến trình mới rồi đóng cửa sổ.

**Architecture:** `Workbench::replace_with` (sau khi hỏi lưu và lưu layout) phát `WorkbenchEvent::Replace(SpawnRequest)`. `Router` (app.rs) subscribe mọi `Workbench` nó dựng và xử lý event: `Path` → resolve, `watch_config`, `window.replace_root` sang `Workbench` mới, cập nhật `shown`, ghi recent; `Welcome` → `replace_root` sang `WelcomeView`, `watch_config(user_only)`. `Workbench` cũ được giải phóng (`on_release` tắt LSP).

**Tech Stack:** Rust, GPUI.

**Spec:** `docs/superpowers/specs/2026-11-12-welcome-design.md` W8, W10 (sửa 2026-11-13); `docs/research/2026-11-12-welcome.md` §5.

## Global Constraints
- Chỉ `lucerna-workbench` dùng gpui. Không thêm giả định chỉ có macOS.
- ⌘Enter ("cửa sổ mới") và đường dẫn từ Finder/CLI vẫn khởi động tiến trình mới như cũ.

## Review Focus
- `Workbench` cũ phải được giải phóng sau khi đổi (không rò LSP/watcher): test kiểm tra `WeakEntity` không upgrade được.
- Cancel ở hộp thoại hỏi lưu: không đổi gì.
- Thư mục đích không resolve được: giữ cửa sổ hiện tại, báo qua `status_message`.
- Đổi hai lần liên tiếp vẫn đúng (subscription gắn với `Workbench` mới).
- Close Folder rồi mở lại từ Welcome trong cùng cửa sổ.

---

### Task 1: Event `Replace` từ Workbench

**Files:** Modify `crates/lucerna-workbench/src/open_commands.rs` (`replace_with`), `src/workbench.rs` (enum + `EventEmitter`), `src/lib.rs` (re-export). Test: `crates/lucerna-workbench/tests/open_commands.rs`.

**Produces:** `pub enum WorkbenchEvent { Replace(SpawnRequest) }`, `impl EventEmitter<WorkbenchEvent> for Workbench`.

- [ ] Sửa test `enter_replaces_the_window`, `open_folder_replaces_the_window`, `close_folder_spawns_welcome` (đổi tên `close_folder_returns_to_welcome`): subscribe event, assert event `Replace(Path(other))` / `Replace(Welcome)`, không có spawn, `window_count == 1`. `cancel_on_unsaved_prompt_keeps_everything` assert không có event. `failed_spawn_keeps_window_and_reports` chuyển sang ⌘Enter.
- [ ] Chạy, thấy fail.
- [ ] `replace_with` phát event thay vì spawn + `remove_window`.
- [ ] Chạy, thấy pass.

### Task 2: Router đổi root tại chỗ

**Files:** Modify `crates/lucerna-workbench/src/app.rs`. Test: `crates/lucerna-workbench/tests/welcome.rs`.

**Consumes:** `WorkbenchEvent::Replace`.

- [ ] Test `open_folder_replaces_root_in_the_same_window`: mở qua router (`open_launch_for_test` trên `a`), emit `Replace(Path(b))` → cùng một cửa sổ (`cx.windows().len() == 1`, cùng id), `router_shown_root_for_test == b`, `Workbench` cũ không upgrade được, recent đứng đầu là `b`, không có spawn. Emit tiếp `Replace(Path(a))` trên workbench mới → root `a`.
- [ ] Test `close_folder_shows_welcome_in_the_same_window`: emit `Replace(Welcome)` → root của cửa sổ là `WelcomeView`, `router_shown_root_for_test == None`; rồi Welcome `Open` thư mục → lại là workbench trong cùng cửa sổ.
- [ ] Test `replace_with_missing_folder_keeps_the_window`: `Replace(Path(không tồn tại))` → root không đổi, status bắt đầu bằng "Could not open ".
- [ ] Cài: `Router::open_first` dùng cửa sổ đang hiện (Welcome hoặc Workbench) nếu có; mọi `Workbench` dựng qua router được subscribe tới `Router::on_workbench_event`; `Replace(Welcome)` dựng `WelcomeView` trong cửa sổ đó.
- [ ] Chạy toàn bộ `cargo test -p lucerna-workbench`, `cargo clippy`.
- [ ] Commit spec, research, plan và code bằng `git commit -- <paths>`.
