# Open Folder switches in place — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Open Folder, Open Recent (Enter) and Close Folder change the root right in the current window instead of starting a new process and then closing the window.

**Architecture:** `Workbench::replace_with` (after asking to save and saving the layout) emits `WorkbenchEvent::Replace(SpawnRequest)`. `Router` (app.rs) subscribes to every `Workbench` it builds and handles the event: `Path` → resolve, `watch_config`, `window.replace_root` to the new `Workbench`, update `shown`, record recent; `Welcome` → `replace_root` to `WelcomeView`, `watch_config(user_only)`. The old `Workbench` is released (`on_release` shuts down LSP).

**Tech Stack:** Rust, GPUI.

**Spec:** `docs/superpowers/specs/2026-11-12-welcome-design.md` W8, W10 (amended 2026-11-13); `docs/research/2026-11-12-welcome.md` §5.

## Global Constraints
- Only `lucerna-workbench` uses gpui. Do not add macOS-only assumptions.
- ⌘Enter ("new window") and paths from Finder/CLI still start a new process as before.

## Review Focus
- The old `Workbench` must be released after the switch (no leaked LSP/watcher): the test checks that the `WeakEntity` cannot upgrade.
- Cancel in the save prompt: nothing changes.
- Target folder cannot be resolved: keep the current window, report via `status_message`.
- Two consecutive switches still work (the subscription is attached to the new `Workbench`).
- Close Folder, then reopen from Welcome in the same window.

---

### Task 1: `Replace` event from Workbench

**Files:** Modify `crates/lucerna-workbench/src/open_commands.rs` (`replace_with`), `src/workbench.rs` (enum + `EventEmitter`), `src/lib.rs` (re-export). Test: `crates/lucerna-workbench/tests/open_commands.rs`.

**Produces:** `pub enum WorkbenchEvent { Replace(SpawnRequest) }`, `impl EventEmitter<WorkbenchEvent> for Workbench`.

- [ ] Change tests `enter_replaces_the_window`, `open_folder_replaces_the_window`, `close_folder_spawns_welcome` (renamed `close_folder_returns_to_welcome`): subscribe to the event, assert event `Replace(Path(other))` / `Replace(Welcome)`, no spawn, `window_count == 1`. `cancel_on_unsaved_prompt_keeps_everything` asserts no event. `failed_spawn_keeps_window_and_reports` moves to ⌘Enter.
- [ ] Run, see it fail.
- [ ] `replace_with` emits the event instead of spawn + `remove_window`.
- [ ] Run, see it pass.

### Task 2: Router switches the root in place

**Files:** Modify `crates/lucerna-workbench/src/app.rs`. Test: `crates/lucerna-workbench/tests/welcome.rs`.

**Consumes:** `WorkbenchEvent::Replace`.

- [ ] Test `open_folder_replaces_root_in_the_same_window`: open through the router (`open_launch_for_test` on `a`), emit `Replace(Path(b))` → the same single window (`cx.windows().len() == 1`, same id), `router_shown_root_for_test == b`, the old `Workbench` cannot upgrade, `b` is first in recent, no spawn. Then emit `Replace(Path(a))` on the new workbench → root `a`.
- [ ] Test `close_folder_shows_welcome_in_the_same_window`: emit `Replace(Welcome)` → the window's root is `WelcomeView`, `router_shown_root_for_test == None`; then Welcome `Open` a folder → a workbench again in the same window.
- [ ] Test `replace_with_missing_folder_keeps_the_window`: `Replace(Path(không tồn tại))` → root unchanged, status starts with "Could not open ".
- [ ] Implement: `Router::open_first` uses the currently shown window (Welcome or Workbench) if there is one; every `Workbench` built through the router is subscribed to `Router::on_workbench_event`; `Replace(Welcome)` builds a `WelcomeView` in that window.
- [ ] Run all of `cargo test -p lucerna-workbench`, `cargo clippy`.
- [ ] Commit spec, research, plan and code with `git commit -- <paths>`.
