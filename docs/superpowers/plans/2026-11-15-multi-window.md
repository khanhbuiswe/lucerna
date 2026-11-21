# One process, many windows — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every Lucerna window runs in one `luc` process (one Dock icon), and each window still has its own workspace settings, trust, terminal and language server.

**Architecture:** `AppState.settings` keeps only the default and user layers; each `Workbench` owns a `WindowSettings` entity holding the workspace layer and passes the handle down to child entities. A global `Router` holds the list of windows, and every open path (CLI socket, Finder, menu, Welcome, restore) goes through one function, `router::open`. `luc` claims a Unix socket in the data folder: if it wins, it is the app (detached from the terminal by default); if not, it sends the path to the running app.

**Tech Stack:** Rust, GPUI (`b5cd310`), `std::os::unix::net`, `libc` (already in `crates/lucerna`), Python stdlib for the bench.

**Spec:** `docs/superpowers/specs/2026-11-15-multi-window-design.md` (read alongside; the decision codes M1–M18 and the § references below point into the spec). Research: `docs/research/2026-11-15-multi-window.md`.

## Global Constraints

- Talk to the user in Vietnamese; code, comments and commit messages in English, like the repo.
- `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"` before cargo. Run `scripts/prune-target.sh` before every build/test. Do not set `CARGO_TARGET_DIR`, no `--target-dir`. Keep one profile on disk (delete `target/release` before a debug test, and vice versa; do not delete when `pgrep -x cargo` shows another session's cargo). Free disk space not below ~1.2 GB.
- Only `lucerna-workbench` uses GPUI. No macOS assumptions outside platform code (`#[cfg(unix)]` for the socket; Windows comes later).
- The UI thread does not wait on I/O, except the first read of `.lucerna/settings.json` when creating a workbench (as today `watch_config` reads synchronously).
- Do not copy Zed code (GPL). VS Code (MIT) may be used as a reference, cite the source if reused.
- `cargo deny check` must pass; `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --check` pass after every task.
- Commit: stage by explicit path, commit with `-- <paths>`, trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. No merging, no pushing, no rebasing.
- Key names and values from the spec: `window.restoreWindows` defaults to `"all"`, accepts `"preserve" | "all" | "folders" | "one" | "none"`; command `workbench.action.newWindow`, key `cmd+shift+n` (macOS) / `ctrl+shift+n` (others); file `<data dir>/windows.json`; socket `<data dir>/luc-<CARGO_PKG_VERSION>.sock`, with `-dev` added under `debug_assertions`; hidden flag `--foreground`.

## Review Focus

1. **A child entity forgets to receive `WindowSettings` and reads the user settings instead of the workspace** — the user sets `editor.tabSize` in `.lucerna/settings.json` and a view (minimap, terminal, explorer) does not follow. Test: Task 6 `window_settings_reach_every_reader` sets a workspace key for each group (editor, terminal, explorer, search, lsp, vim) and checks every reader.
2. **A folder with a symlink or different letter case** — `luc ./proj` and Finder opening `/Users/x/Proj` must focus the same window. Test: Task 7 `choose_focuses_canonical_root` (the path is already canonical at `router::open`; a GPUI test opens through a symlink).
3. **Closing a window while ⌘Q's save prompt is open in another window** — no panic, no quitting before anyone agrees. Test: Task 9 `quit_survives_window_closed_during_prompt`.
4. **`luc` called twice in quick succession when the app is not running yet** (`luc a & luc b`) — only one app process, both folders open. Test: Task 11 `claim_second_caller_becomes_client` (unit, temp socket) and the manual run.
5. **`windows.json` corrupt or from an unknown version** — start as an empty list, no panic, overwrite with a correct file on the next write. Test: Task 10 `session_load_corrupt_is_empty`.

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/lucerna-settings/src/store.rs` | `WorkspaceLayer`, `SettingsView`, remove `Layer::Workspace` |
| `crates/lucerna-settings/src/scope.rs` (new) | `is_app_scope(key)` |
| `crates/lucerna-workbench/src/window_settings.rs` (new) | `WindowSettings` entity: workspace layer, `.lucerna` watcher, errors, the window's language registry |
| `crates/lucerna-workbench/src/config_files.rs` | only the user folder is left |
| `crates/lucerna-workbench/src/window_choice.rs` (new, replaces `open_requests.rs`) | pure window-choice function (§6.3), `file_paths(urls)` |
| `crates/lucerna-workbench/src/router.rs` (new, split from `app.rs`) | global `Router`, `open`, window list, `last_active`, restore queue |
| `crates/lucerna-workbench/src/session.rs` (new) | `windows.json`: read, write, filter by `window.restoreWindows` |
| `crates/lucerna-workbench/src/cli_socket.rs` (new) | protocol, `claim`, `send`, `serve` |
| `crates/lucerna-workbench/src/app.rs` | `run`, `start`, startup, `QuitMode`, `on_reopen` |
| `crates/lucerna-workbench/src/process.rs` | **delete** |
| `crates/lucerna-workbench/src/open_requests.rs` | **delete** (replaced by `window_choice.rs`) |
| `crates/lucerna/src/main.rs`, `crates/lucerna/src/launch.rs` (new) | claim the socket, client, detach from the terminal, `--foreground` |
| `bench/compare/multiwindow.py` (new) + tests | measure many windows |
| `crates/lucerna-workbench/tests/multi_window.rs` (new) | GPUI multi-window tests |

---

### Task 1: Measure a baseline before the switch

**Files:**
- Create: `bench/compare/multiwindow.py`, `bench/compare/tests/test_multiwindow.py`
- Modify: `bench/compare/README.md` (new section), `docs/benchmarks.md` (section "Multiple windows")

**Interfaces:**
- Produces: `python3 -I bench/compare/multiwindow.py --windows 1,2,3 --out <json>` and `--mode before|after`. Task 13 reruns it with `--mode after`.

Measure three numbers (§7.1) on a release build made from the branch's base commit (`git merge-base multi-window main`, currently `2a9c1e3` + the docs commit), with a separate `HOME` like `editors.launch`:
- `ram_mb[n]`: the sum of `footprint` (use the parser already in `metrics.py`) over every `luc` process in the bench session after opening n windows on n different repos and waiting for `wait_quiet`. Mode `before`: each window is a separate `luc <repo>` run. Mode `after`: first `luc --foreground <repo1>`, then `luc <repoK>` (client).
- `second_window_ms`: from the moment the command to open the second window is issued to the first frame with content in that window (`bench-helper watch ... coverage`, like `startup_ms`).
- `drop_pct_while_busy`: the fraction of dropped frames when scrolling window A (`bench-helper scroll`) while B opens a large repo (the bench's large repo), compared to when B is idle. Two windows with the same pid: make window A's `frame` larger than B's so the helper picks A. If the helper still cannot tell them apart, add `--window-index` to `bench-helper` (Swift) and note it in the README.

- [ ] **Step 1: Test the pure part** (`test_multiwindow.py`): `summarize(rounds)` returns median/min/max for each metric; `luc_commands("after", repos)` returns `[[bin, "--foreground", r1], [bin, r2], ...]`, `"before"` returns `[[bin, r1], [bin, r2], ...]`.
- [ ] **Step 2:** `python3 -m unittest discover -s bench/compare/tests -v` → FAIL (module does not exist yet).
- [ ] **Step 3:** Write `multiwindow.py` (reuse the existing `editors`, `metrics`, `env`; 5 rounds per metric).
- [ ] **Step 4:** Unit tests PASS.
- [ ] **Step 5:** Build release at the base commit (`scripts/bundle-app.sh`, following the one-profile rule), run `--mode before`, write the JSON to `bench/compare/results/multiwindow-before.json` and the table to `docs/benchmarks.md` section "Multiple windows — before". If there is no Screen Recording/Accessibility permission, stop and tell the user (only RAM can run without screen permissions).
- [ ] **Step 6: Commit** `bench: multi-window measurement and the baseline before one process`.

### Task 2: `WorkspaceLayer` and `SettingsView` in `lucerna-settings`

**Files:**
- Modify: `crates/lucerna-settings/src/store.rs`, `crates/lucerna-settings/src/lib.rs`
- Create: `crates/lucerna-settings/src/scope.rs`
- Test: in `store.rs`/`scope.rs` (`#[cfg(test)]`)

**Interfaces:**
- Produces:
  - `pub struct WorkspaceLayer` (a parsed map; `Default` = empty) with `pub fn parse(jsonc: &str) -> Result<WorkspaceLayer, SettingsError>` and `pub fn app_scope_keys(&self) -> Vec<String>` (the app-scope keys present in the layer, for logging).
  - `pub struct SettingsView<'a>` (`Copy`) with `pub fn get<T: DeserializeOwned>(&self, key: &str, language: Option<&str>) -> Option<T>` and `pub fn get_trusted<T>(..)`.
  - `impl SettingsStore { pub fn view<'a>(&'a self, workspace: Option<&'a WorkspaceLayer>) -> SettingsView<'a> }`. `SettingsStore::get`/`get_trusted` stay as they are = `view(None)`.
  - `pub fn is_app_scope(key: &str) -> bool` (scope.rs): `true` for `workbench.colorTheme`, `workbench.preferredDarkColorTheme`, `workbench.preferredLightColorTheme`, `workbench.iconTheme`, `workbench.startupEditor`, every key starting with `window.` or `extensions.`.
  - `Layer` keeps only `Default`, `User` (remove `Workspace`: compile errors lead to the places that need fixing).
- `SettingsView::get` when the workspace layer has an app-scope key: ignore that value (look up as if it were absent).

- [ ] **Step 1: Tests** — `view_prefers_workspace`: user `{"editor.tabSize":4}`, workspace `{"editor.tabSize":2}` → `view(Some(&ws)).get::<u32>("editor.tabSize", None) == Some(2)`, `view(None)` → `Some(4)`. `view_language_override_order`: workspace `[rust]` > user `[rust]` > workspace > user (port the existing order tests to `view`). `view_merges_objects` (R26 across three layers). `trusted_skips_workspace`. `app_scope_ignored_in_workspace`: workspace `{"workbench.colorTheme":"X","window.restoreWindows":"none"}` → `view(Some(&ws)).get::<String>("workbench.colorTheme", None)` equals the user/default value. `is_app_scope` for each key above and `false` for `editor.tabSize`, `files.associations`.
- [ ] **Step 2:** `cargo test -p lucerna-settings` → FAIL.
- [ ] **Step 3:** Implement; move the old tests that use `Layer::Workspace` in this crate to `WorkspaceLayer`.
- [ ] **Step 4:** `cargo test -p lucerna-settings` → PASS. (`lucerna-workbench` does not compile yet: Task 3 fixes it.)
- [ ] **Step 5: Commit** `settings: workspace layer as a separate value, views over user + workspace`. May be merged with the Task 3 commit if the workspace does not compile in between; the review order stays separate.

### Task 3: `WindowSettings` entity; `ConfigState` keeps only user

**Files:**
- Create: `crates/lucerna-workbench/src/window_settings.rs`
- Modify: `config_files.rs`, `app.rs` (call `watch_config` once in `start`), `workbench.rs` (create the entity), `lib.rs` (export), the config-error status item (in `config_files.rs`)
- Test: `crates/lucerna-workbench/tests/window_settings.rs`, fix `tests/config_files.rs`, `tests/trust.rs`, `tests/lsp_health.rs`, `tests/terminal_lifecycle.rs`, `src/git/tests.rs` (the places with `Layer::Workspace`)

**Interfaces:**
- Consumes: Task 2.
- Produces:
  - `pub struct WindowSettings`; `impl WindowSettings`:
    - `pub fn new(root: Option<&Path>, cx: &mut Context<Self>) -> Self` — `root` is `None` for a lone-file window. Reads `<root>/.lucerna/settings.json` synchronously; observes the global `AppState` to `cx.notify()` when the user layer changes; watcher on `<root>/.lucerna` (not run when `cx.background_executor().is_test()`), debounced by `CONFIG_DEBOUNCE`.
    - `pub fn view<'a>(&'a self, cx: &'a App) -> SettingsView<'a>`
    - `pub fn get<T: DeserializeOwned>(&self, key: &str, language: Option<&str>, cx: &App) -> Option<T>`; `pub fn get_trusted<T>(..)` likewise.
    - `pub fn reload(&mut self, cx: &mut Context<Self>)` — re-reads the file (background thread), updates the layer, error, `generation`, notifies.
    - `pub fn set_workspace_json(&mut self, jsonc: &str, cx: &mut Context<Self>) -> Result<(), SettingsError>` — for tests.
    - `pub fn error(&self) -> Option<&ConfigError>`; `pub fn generation(&self) -> u64`; `pub fn settings_path(&self) -> Option<&Path>`.
  - `impl Workbench { pub fn settings(&self) -> &Entity<WindowSettings> }` — created in `Workbench::build` before any child entity.
  - `ConfigPaths` drops the field `workspace_settings` and `with_user_dir(_, root)`/`for_root`; `user_only()` remains (renaming to `ConfigPaths::default_user()` is not needed — keep `user_only`, `user_only_with_user_dir`). `ConfigFile::WorkspaceSettings` moves to the `ConfigError` of `WindowSettings` (keep `status_text` ".lucerna/settings.json: …").
  - The config-error status item: shows the `ConfigState` (user) error and then the window's `WindowSettings::error()`.
  - Saving `.lucerna/settings.json` open in the editor: the workbench calls `WindowSettings::reload` (instead of `reload_config` seeing the workspace path).
- This step does **not** yet change the readers (Tasks 4–6): every place still reads `AppState.settings` (now user only), so workspace settings temporarily have no effect in the views. Tests that break for that reason are marked in the ledger and must be green again after Task 6.

- [ ] **Step 1: Tests** (`tests/window_settings.rs`): `reads_workspace_file` (a tempdir with `.lucerna/settings.json` `{"editor.tabSize":2}` → `get::<u32>("editor.tabSize")==Some(2)`); `user_change_notifies` (`reload_config` user → the entity's observer runs, the new value is visible); `reload_picks_up_edit`; `broken_file_keeps_previous_and_reports` (`error()` has a `status_text` starting with `.lucerna/settings.json:`); `single_file_has_no_workspace` (`new(None)`); `two_entities_independent` (two roots, two values).
- [ ] **Step 2:** Run `cargo test -p lucerna-workbench --test window_settings` → FAIL.
- [ ] **Step 3:** Implement; change `config_files.rs`, `app.rs` (`watch_config(ConfigPaths::user_only_with_user_dir(..))` once in `start`/`Router` is not called again when the folder changes), `workbench.rs`; change the tests that use `Layer::Workspace` to write the `.lucerna/settings.json` file before creating the workbench or `wb.settings().update(cx, |s, cx| s.set_workspace_json(..., cx))`.
- [ ] **Step 4:** `cargo test -p lucerna-workbench --test window_settings --test config_files` → PASS; `cargo build -p lucerna` passes.
- [ ] **Step 5: Commit** `workbench: per-window workspace settings entity; config state keeps the user files`.

### Task 4: Per-window settings reads — editor (documents, code view, excerpt list, vim, save, goto, fonts, symbols, editor groups, markdown preview)

**Files:**
- Modify: `documents.rs`, `code_view/mod.rs`, `code_view/transact.rs`, `code_view/minimap.rs`, `code_view/element.rs` (if it reads settings), `excerpt_list/render.rs`, `excerpt_list/mod.rs`, `vim.rs`, `save.rs`, `goto.rs`, `fonts.rs`, `symbols/breadcrumbs.rs`, `editor_groups/view.rs`, `markdown_preview/{mod,sync,commands}.rs`, `workbench.rs` (pass the handle)
- Test: `tests/multi_window.rs` (create the file), fix `tests/code_view*.rs`, `tests/excerpt_list*.rs` for the new signatures

**Interfaces:**
- Consumes: `WindowSettings` (Task 3).
- Produces (new signatures, used by later tasks and tests):
  - `Documents::new(root: PathBuf, settings: Entity<WindowSettings>) -> Self`
  - `CodeView::new(path: Arc<Path>, settings: Entity<WindowSettings>, cx: &mut Context<Self>) -> Self`
  - `ExcerptList::new(root: PathBuf, id_prefix: &'static str, settings: Entity<WindowSettings>, cx) -> Self`
  - `VimLayer::new(settings: Entity<WindowSettings>, cx) -> Self`
  - `MarkdownPreview::new(.., settings: Entity<WindowSettings>, ..)` (add the last parameter before `cx`)
  - `fonts::editor_font_family(settings: SettingsView<'_>) -> ..` (the parameter changes from `cx: &App`)
  - Other free functions that take `cx: &App` only to read settings: change to take `SettingsView<'_>`; the caller gets `self.settings.read(cx).view(cx)`.
  - `observe_global::<AppState>` used to listen to settings (vim.rs:172, code_view/mod.rs:449, workbench.rs:718) changes to `cx.observe(&settings, ..)`; the places that listen to the theme keep `observe_global`.
- The document language (`Documents::language_for`) still uses `AppState.languages` in this task; Task 5 switches it to the window's registry.

- [ ] **Step 1: Tests** (`tests/multi_window.rs`, helper `open_two(cx, ws_a: &str, ws_b: &str) -> (WindowHandle<Workbench>, WindowHandle<Workbench>)` opens two `Workbench::new` in two windows on two tempdirs with different `.lucerna/settings.json`): `editor_settings_per_window` — A `{"editor.tabSize":2,"editor.wordWrap":"on"}`, B `{"editor.tabSize":8}`; open `a.rs` in each window; the indent tab size (`Documents` indentation inputs) is 2 and 8; A's `CodeView` wraps, B's does not. `workspace_change_stays_in_window` — `set_workspace_json` in A changes tabSize → B does not change, A does.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** Pass the handle and change the reads in this task's files.
- [ ] **Step 4:** `cargo test -p lucerna-workbench --test multi_window` and the affected editor tests (`code_view`, `documents`, `editing`, `vim*`, `markdown_preview`, `excerpt_list*`) → PASS.
- [ ] **Step 5: Commit** `workbench: editor views read the window's settings`.

### Task 5: Per-window settings reads — language servers, git, trust, `files.associations`

**Files:**
- Modify: `language_servers/{mod,sync,lifecycle}.rs`, `lsp_log.rs`, `lsp_health.rs`, `lsp_status.rs` (if it reads), `git/*` (keys `git.*`, `scm.*`), `trust.rs:459`, `window_settings.rs` (registry), `documents.rs::language_for`, `extensions/service.rs` (the shared registry only uses user; tell windows to rebuild when the shared registry changes)
- Test: `tests/multi_window.rs`, fix `tests/lsp_*.rs`, `src/git/tests.rs`

**Interfaces:**
- Consumes: Tasks 3, 4.
- Produces:
  - `LanguageServers::new(.., settings: Entity<WindowSettings>, ..)` (add the parameter); `lsp`, `lucerna.lsp.enabled` read through the window's `get_trusted`/`get`.
  - `GitStore` takes `Entity<WindowSettings>` (reads `git.enabled`, `git.path`, `git.statusLimit`, `git.diffBase`, `scm.*`).
  - `impl WindowSettings { pub fn languages(&self, cx: &App) -> Arc<LanguageRegistry> }` — `AppState.languages` if the workspace layer has no `files.associations`; otherwise a private registry built (on a background thread) from the same extension inputs with associations merged from user + workspace; rebuilt when `AppState.languages_generation` changes or the workspace layer changes. Add `pub fn languages_generation(&self) -> u64` so `Documents` knows to look up again.
  - To build the private registry: `extensions::service` provides `pub(crate) fn registry_with_associations(assoc: Associations, cx: &App) -> Task<LanguageRegistry>` reusing the existing `RegistryInputs`.

- [ ] **Step 1: Tests:** `lsp_settings_per_window` — A workspace `{"lucerna.lsp.enabled":false}`, B none; use the fake server from `tests/support/lsp.rs`, open `a.rs` in both (both trusted) → only B starts a server. `lsp_binary_ignores_workspace` (the idea is already in the current test; move it to `WindowSettings`). `associations_per_window` — A `{"files.associations":{"*.foo":"rust"}}` → `x.foo` is Rust in A, plain text in B. `git_disabled_per_window` — A `{"git.enabled":false}` → A's `GitStore` does not scan, B's does.
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** `cargo test -p lucerna-workbench` for `multi_window`, `lsp_*`, `git_*`, `trust` and the unit tests in `src/git` → PASS.
- [ ] **Step 5: Commit** `workbench: language servers, git and file associations follow the window's settings`.

### Task 6: Per-window settings reads — terminal, explorer, search, panel, the rest of the workbench; architecture test

**Files:**
- Modify: `terminal/{settings,commands,links,panel,view}.rs`, `explorer/{view,menu,commands}.rs`, `search_view/view.rs`, `layout/panel.rs`, `workbench.rs` (1636, 2349, 2804, 3166), `onboarding/mod.rs` (user only: keep `AppState`, Get Started has no workbench window; inside a workbench's tab it reads through the handle)
- Create: `crates/lucerna-workbench/tests/architecture_settings.rs`
- Test: `tests/multi_window.rs`, fix `tests/terminal_*.rs`, `tests/explorer_*.rs`

**Interfaces:**
- Consumes: Tasks 3–5.
- Produces: the functions `terminal::settings::*`, `terminal::commands::skips_shell`, `explorer::commands::*` take `SettingsView<'_>` instead of `&App`. `TerminalPanel::new(settings: Entity<WindowSettings>, window, cx)`.

- [ ] **Step 1: Tests:** `window_settings_reach_every_reader` in `multi_window.rs`: A's workspace sets `terminal.integrated.scrollback: 123`, `explorer.confirmDelete: false`, `workbench.panel.defaultLocation: "right"`, `vim.enabled: true`, `files.autoSave: "afterDelay"`; B sets none. Check each value through the public API of the terminal panel, the explorer (delete without asking), the panel position, the vim layer being on, auto-save — A follows the workspace, B follows the defaults. `architecture_settings.rs`: scan `src/**/*.rs`, no file other than `config_files.rs`, `app.rs`, `router.rs`, `startup.rs`, `session.rs`, `extensions/service.rs`, `theme_picker.rs`, `onboarding/mod.rs`, `welcome.rs`, `window_settings.rs`, `app_menu.rs` contains `AppState>().settings` or `AppState>()\n            .settings`; no file contains `process::spawn`/`SpawnRequest` (this part turns on in Task 7: leave it `#[ignore]` with a note until Task 7 is done).
- [ ] **Step 2:** FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** `cargo test -p lucerna-workbench` (all) → PASS, including the tests recorded as temporarily broken in Task 3.
- [ ] **Step 5: Commit** `workbench: terminal, explorer and panels read the window's settings; guard test`.

### Task 7: Multi-window Router and `router::open`

**Files:**
- Create: `window_choice.rs`, `router.rs`
- Delete: `process.rs`, `open_requests.rs`
- Modify: `app.rs` (Router moves out to `router.rs`; `run` uses `router::open`), `open_commands.rs` (drop `SpawnRequest`, `spawn_or_report`, `WorkbenchEvent::Replace`), `welcome.rs`, `onboarding/mod.rs` (`OpenSettingsInNewWindow`), `recent_picker.rs`/`open_commands.rs` (⌘Enter), `lib.rs`, `crates/lucerna/src/main.rs` (drop `resolver`)
- Test: `window_choice.rs` (unit), `tests/multi_window.rs`, fix tests that use `WorkbenchEvent`/`Spawner`/`open_launch_for_test`/`router_shown_root_for_test`

**Interfaces:**
- Produces:
  - `window_choice.rs` (pure, no GPUI):
    - `pub enum WindowKind { Welcome, Folder(PathBuf), File(PathBuf) }` (`Welcome` includes Get Started)
    - `pub struct WindowInfo { pub kind: WindowKind, pub last_active: u64 }`
    - `pub enum Target { Auto, NewWindow, ReplaceIn(usize) }`
    - `pub enum Choice { Focus(usize), OpenTab(usize), ReplaceRoot(usize), NewWindow }`
    - `pub fn choose(windows: &[WindowInfo], path: &Path, is_dir: bool, target: Target) -> Choice` — rules 1–4 of §6.3 (index is the position in the slice).
    - `pub fn file_paths(urls: &[String]) -> Vec<PathBuf>` (moved from `open_requests`).
  - `router.rs`:
    - `pub struct Launch` keeps its fields; add `pub fn Launch::for_path(path: PathBuf) -> Result<Launch, String>` (folder → folder; file → single file with the parent as root; nonexistent → `"path not found: …"`), replacing `Resolve`.
    - `pub enum Source { Cli, Finder, Menu, Restore }`
    - `pub struct OpenRequest { pub paths: Vec<PathBuf>, pub target: Target, pub source: Source }`
    - `pub fn open(request: OpenRequest, cx: &mut App) -> Result<(), String>` — canonicalizes, calls `choose` for each path, performs it, records recent, `app_menu::refresh`, (Task 10 adds writing `windows.json`). Errors are combined into one string for `Source::Cli`.
    - `pub fn open_welcome(target: Target, cx: &mut App)`; `pub fn open_onboarding(cx: &mut App)`.
    - `pub fn windows(cx: &App) -> Vec<(AnyWindowHandle, WindowKind)>` in descending `last_active` (used for quit, session, tests).
    - Router is a `Global` (created lazily with `default_global` if absent, so tests that build `Workbench::new` directly still run); `last_active` is updated through `observe_window_activation` of every window the router opens; `on_window_closed` drops the entry.
  - `Workbench`: `replace_with(Replacement, window, cx)` with `pub enum Replacement { Path(PathBuf), Welcome }` — after `confirm_quit` it calls `router::open(.. Target::ReplaceIn(this window) ..)` or `open_welcome(ReplaceIn)` directly; a folder already open in another window focuses that window, without asking to save (rule 1 is checked before `confirm_quit`). `WorkbenchEvent` is deleted.
  - New command `workbench.action.newWindow` ("New Window"), key in `default-keybindings.json`, File menu item at the top of the Open group, Dock menu item (`cx.set_dock_menu`) "New Window". Works without a window: Task 9 registers the app-level handler; in this task it is registered as a workbench + Welcome command.
  - Test hooks replacing the old ones: `#[doc(hidden)] pub fn router::open_for_test(paths: Vec<PathBuf>, target: Target, cx: &mut App)` and `router::windows`.

- [ ] **Step 1: Unit tests for `choose`:** `choose_focuses_open_folder`, `choose_focuses_open_single_file`, `choose_deepest_root_wins` (roots `/a` and `/a/b`, file `/a/b/c.rs` → window `/a/b`), `choose_single_file_window_is_not_a_root` (a file window `/a/x.rs`, open `/a/y.rs`, there is a folder window `/z` used most recently → `OpenTab(z)`), `choose_loose_file_goes_to_last_active_folder`, `choose_loose_file_reuses_welcome`, `choose_loose_file_new_window_when_none`, `choose_folder_reuses_welcome` (the most recently used Welcome of two Welcomes), `choose_new_window_still_focuses_open` (`Target::NewWindow` + an already open folder → `Focus`), `choose_replace_in` (`ReplaceIn(1)` a folder not yet open → `ReplaceRoot(1)`), `choose_focuses_canonical_root` (the caller passes a canonical path; the test confirms the path comparison is component by component, `/a/bc` is not inside `/a/b`).
- [ ] **Step 2: GPUI tests** (`multi_window.rs`): `open_two_folders_one_app` (two `open_for_test` → `router::windows` has 2, both `Folder`); `reopen_focuses_existing` (reopen folder 1 → still 2, window 1 has the largest `last_active`); `open_through_symlink_focuses` (a symlink to folder 1); `file_inside_b_opens_in_b` (A active, open `b/src/x.rs` → pinned tab in B, A unchanged); `new_window_then_pick_folder` (New Window → 3 windows, one `Welcome`; `WelcomeEvent::Open` of a new folder on that Welcome → still 3, that entry becomes `Folder`); `open_recent_cmd_enter_new_window`; `replace_releases_old_workbench` (keep test W8: the old workbench cannot upgrade); `same_file_two_windows_reload` (M17: `a/x.rs` open in A and in a lone-file window B; saving in A → B receives the disk change and reloads, like any external change).
- [ ] **Step 3:** FAIL.
- [ ] **Step 4:** Implement; delete `process.rs`, `open_requests.rs`, `Spawner`; turn on the `process::spawn` part of the architecture test (Task 6).
- [ ] **Step 5:** `cargo test -p lucerna-workbench` → PASS; `cargo build -p lucerna`.
- [ ] **Step 6: Commit** `workbench: one router for every window; New Window; no more processes per window`.

### Task 8: Per-window `status_message`

**Files:**
- Modify: `status_message.rs` and 27 callers (`grep -rn "status_message::show"`), the status bar reads `current`
- Test: `tests/multi_window.rs`, fix the tests that use `status_message::` (13 places)

**Interfaces:**
- Produces: `pub fn show(window: WindowId, text: impl Into<SharedString>, cx: &mut App)`; `pub fn current(window: WindowId, cx: &App) -> Option<SharedString>`; `observe` keeps its signature. A map `WindowId → (text, generation)`; the entry is dropped when the window closes (the router calls `status_message::forget(window, cx)`). Callers in the workbench use `window.window_handle().window_id()`; callers without a `Window` (the router) pass the target window; with no window, log instead of showing.

- [ ] **Step 1: Test** `status_message_stays_in_window`: `show(a, "hi")` → `current(a) == Some("hi")`, `current(b) == None`; B's status bar does not render "hi".
- [ ] **Step 2:** FAIL. **Step 3:** Implement. **Step 4:** PASS the whole crate.
- [ ] **Step 5: Commit** `workbench: status messages belong to their window`.

### Task 9: Quit, closing the last window, Dock, app-level actions

**Files:**
- Modify: `app.rs` (`start`: remove the quit observer; `run`: `application().with_quit_mode(QuitMode::Default)`, `on_reopen` → `router::open_welcome(Target::NewWindow)`), `workbench.rs::quit` (sorted by `router::windows`, tolerates a window closing midway), `app_menu.rs` (app-level handler for M10 when there is no window; enabled/disabled state of items when there is no window)
- Test: `tests/multi_window.rs`, `tests/app_menu.rs`

**Interfaces:**
- Consumes: `router::windows`, `router::open`, `router::open_welcome` (Task 7).
- Produces: `pub(crate) fn quit(cx: &mut App)` keeps its name; Task 10 adds a step writing `windows.json` synchronously right before `cx.quit()` (so hook `session::save_now(cx)` at exactly that place; this task leaves a comment at the hook spot).
- Behavior with no window (M10): `workbench.action.newWindow` → a new Welcome; `workbench.action.files.openFileFolder`/`openFolder`/`openFile` → the app's picker (`cx.prompt_for_paths`) then `router::open(Target::NewWindow, Source::Menu)`; the menu's Open Recent item (`OpenRecentEntry`) → `router::open(Target::Auto)`; `workbench.action.openSettingsJson`/`openGlobalKeybindingsFile` (ids present in `config_files::register_commands`) → `router::open` the user file with `Target::NewWindow`; About as today; Quit as today.

- [ ] **Step 1: Tests:** `quit_cancel_in_second_window_keeps_all` (two windows with dirty files; Quit; the most recently used window asks first — `simulate_prompt_answer` "Don't Save" in it, "Cancel" in the other window → no quit, both windows remain); `quit_dont_save_both_quits` (use a test flag: replace `cx.quit` with a recording hook, because `TestPlatform::quit` is a no-op); `quit_survives_window_closed_during_prompt` (while asking in A, close B with `remove_window` → no panic; answer A → quit); `close_last_window_keeps_app` (`QuitMode::Explicit` in the test; close all → the app remains, `router::windows` is empty, menu: the New Window item is enabled, items needing a workbench are disabled); `menu_open_folder_without_window` (`simulate_path_prompt_response` → one new folder window); `reopen_opens_welcome` (call the handler function `app::on_reopen_for_test(cx)` → one Welcome).
- [ ] **Step 2:** FAIL. **Step 3:** Implement. **Step 4:** PASS the whole crate.
- [ ] **Step 5: Commit** `workbench: quit asks every window, the app outlives its last window on macOS, Dock reopens Welcome`.

### Task 10: `windows.json`, `window.restoreWindows: "all"`, queue during restore

**Files:**
- Create: `session.rs`
- Modify: `startup.rs` (`restores_last_workspace` → `restore_windows`), `app.rs::first_screen` and `open_without_folder`, `router.rs` (write the session after every change; flag `restoring`, queue), `workbench.rs::quit` (synchronous write), `crates/lucerna-settings/assets/default-settings.json` (`"window.restoreWindows": "all"`), `docs/superpowers/specs/2026-11-15-restore-last-workspace-design.md` (note: replaced by the multi-window spec)
- Test: `session.rs` (unit), `app.rs` tests (`first_screen_*`), `tests/multi_window.rs`

**Interfaces:**
- Produces:
  - `pub struct SessionWindow { pub kind: RecentKind, pub path: PathBuf }` (uses the existing `RecentKind::{Folder, File}`); `pub struct Session { pub windows: Vec<SessionWindow> }` (most recently used first).
  - `pub fn session_path(data_dir: &Path) -> PathBuf` (`windows.json`); `pub fn load(path: &Path) -> Session` (missing/corrupt/`version` ≠ 1 → empty, log); `pub fn save(path: &Path, session: &Session) -> io::Result<()>` (atomic write, temp name by pid like `recent.rs`).
  - `pub enum RestoreWindows { Preserve, All, Folders, One, None }`; `pub fn restore_windows(settings: &SettingsStore) -> RestoreWindows` (unknown value → `All`).
  - `pub fn to_restore(session: &Session, mode: RestoreWindows, exists: impl Fn(&Path) -> bool) -> Vec<SessionWindow>` (filtered, most recently used first).
  - `StartupScreen::Restore(Vec<SessionWindow>)` replaces `Restore(PathBuf)`; an empty list → fall back to `recent.json` as before (one entry) except `None`.
  - `router::save_session(cx)` (background) and `session::save_now(cx)` (synchronous, called in `quit`).
  - Startup with a path: `Preserve` reopens and then opens the path; the others only open the path.

- [ ] **Step 1: Unit tests:** `session_round_trip`; `session_load_corrupt_is_empty` (garbage, `{"version":2}`, missing file); `to_restore_all_skips_missing`; `to_restore_folders_only`; `to_restore_one_takes_first`; `to_restore_none_is_empty`; `restore_windows_default_is_all` (with `SettingsStore::with_defaults()`); `first_screen_restores_all_windows`; `first_screen_empty_session_falls_back_to_recent`; `first_screen_none_ignores_session`.
- [ ] **Step 2: GPUI tests:** `session_follows_open_and_close` (open 2 → the file has 2 in most-recently-used order; close 1 → 1 left; Welcome does not go into the file); `restore_opens_in_reverse_order` (a session with 3 entries, 1 already deleted → 2 windows, the most recently used window has the largest `last_active` after restore); `requests_wait_for_restore` (set `restoring`, `router::open` a folder that is in the session → queued; after restore finishes → no duplicate window, exactly one window for that folder).
- [ ] **Step 3:** FAIL. **Step 4:** Implement. **Step 5:** PASS the whole crate.
- [ ] **Step 6: Commit** `workbench: remember open windows and reopen them all at startup`.

### Task 11: `luc` over a socket, detached from the terminal, `--foreground`

**Files:**
- Create: `crates/lucerna-workbench/src/cli_socket.rs`, `crates/lucerna/src/launch.rs`
- Modify: `crates/lucerna/src/main.rs`, `crates/lucerna/src/lib.rs`, `app.rs::run` (takes a listener), `router.rs` (CLI request channel, reply), `bench/compare/editors.py` (add `--foreground` when `editor == "lucerna"`), `scripts/bundle-app.sh` (shim comment: `luc` detaches from the terminal itself), `bench/compare/multiwindow.py` (mode after)
- Test: `cli_socket.rs` (unit), `crates/lucerna/tests/cli_socket.rs` (`#[ignore]`), `bench/compare/tests`

**Interfaces:**
- Produces:
  - `cli_socket.rs` (`#[cfg(unix)]`):
    - `pub const PROTOCOL_VERSION: u32 = 1;`
    - `#[derive(Serialize, Deserialize)] pub struct CliRequest { pub version: u32, pub cwd: PathBuf, pub paths: Vec<PathBuf>, #[serde(default)] pub new_window: bool }`; `pub struct CliReply { pub ok: bool, #[serde(default)] pub error: Option<String> }`.
    - `pub fn socket_path(data_dir: &Path) -> PathBuf` — `luc-<ver>[-dev].sock`; if `as_os_str().len() > 100` then `std::env::temp_dir().join(format!("lucerna-{:016x}.sock", hash(data_dir)))`.
    - `pub enum Claim { Server(UnixListener), Client(UnixStream), Unavailable(io::Error) }`; `pub fn claim(path: &Path) -> Claim` — bind; `EADDRINUSE` → connect (timeout 1 s) → `Client`; connect `ECONNREFUSED` → delete the file, bind again once.
    - `pub fn send(stream: UnixStream, request: &CliRequest, timeout: Duration) -> Result<CliReply, String>`.
    - `pub fn serve(listener: UnixListener, tx: futures::channel::mpsc::UnboundedSender<(CliRequest, futures::channel::oneshot::Sender<CliReply>)>)` — runs on a `std::thread`.
  - `app::run(launch: Option<Launch>, listener: Option<UnixListener>)` (drop `Resolve`). The router receives from the channel, calls `router::open(Source::Cli, Target::Auto)` (`paths` empty → `[cwd]`), returns `CliReply`; `version > PROTOCOL_VERSION` → `ok:false`, `error:"this Lucerna is older than luc; update Lucerna"`. Quitting the app deletes the socket file (`on_app_quit`).
  - `crates/lucerna/src/launch.rs`: `pub fn spawn_detached(exe: &Path, args: &[OsString], cwd: &Path) -> io::Result<()>` — `Command` + `pre_exec(libc::setsid)`, stdio `/dev/null`, does not wait (a reap thread like the old `process.rs`). `pub fn send_with_retry(path: &Path, request: &CliRequest, total: Duration) -> Result<CliReply, String>` (try connecting every 50 ms up to 10 s).
  - `main.rs`: add `#[arg(long, hide = true)] foreground: bool`; `--trace-startup` turns on `foreground`. Flow: parse → `ext`/`lsp` as before → resolve the path (error → print, exit code 1) → `claim(socket_path)`:
    - `Client` → `send` → print the error/exit with code 0 or 1.
    - `Server` and `foreground` (or cwd `/` with no arguments: opened from Finder/Dock) → `app::run(.., Some(listener))`.
    - `Server` and not `foreground` → drop the listener (delete the file), `spawn_detached(current_exe, ["--foreground", <path>], cwd)`, `send_with_retry`; return the code per the reply (timeout: print "Lucerna is starting; could not confirm the window opened", code 1).
    - `Unavailable` → warn on stderr, `app::run(.., None)` in place.

- [ ] **Step 1: Unit tests** (`cli_socket.rs`, sockets in `tempfile::tempdir`): `claim_first_is_server`; `claim_second_caller_becomes_client` (a server is listening → `Client`); `claim_stale_socket_rebinds` (a socket file nobody listens on → `Server`); `request_round_trip` (a fake server returns `ok`); `unknown_fields_ignored`; `newer_version_rejected` (pure function `reply_for_version`); `long_data_dir_uses_temp` (path > 100 bytes).
- [ ] **Step 2: Integration test** `crates/lucerna/tests/cli_socket.rs` `#[ignore]`: temporary `HOME`; `luc <dir1>` exits in < 10 s, exactly one `luc --foreground` process remains; `luc <dir2>` code 0, the process count does not change; `luc /nope` code 1, stderr has `path not found`; kill the app process at the end of the test.
- [ ] **Step 3:** Unit FAIL. **Step 4:** Implement. **Step 5:** `cargo test -p lucerna-workbench cli_socket` and `cargo test -p lucerna` PASS; run by hand `cargo test -p lucerna --test cli_socket -- --ignored` on a machine with a display (record the result in the ledger). `python3 -m unittest discover -s bench/compare/tests -v` PASS.
- [ ] **Step 6: Commit** `luc: hand paths to the running app over a socket and leave the terminal`.

### Task 12: Trust synced between windows

**Files:**
- Modify: `trust.rs` (app-level event), `workbench.rs` (listen and re-read trust)
- Test: `tests/multi_window.rs`, `tests/trust.rs`

**Interfaces:**
- Produces: a global `TrustChanged { generation: u64 }` (or `cx.observe_global::<TrustGeneration>`); when a window finishes writing `trusted.json` it increments; every `Workbench` observes, re-reads the trust of its own root on a background thread (the existing `TrustStore`) and calls `trust_loaded` if it changed.

- [ ] **Step 1: Test** `trust_in_a_reaches_b`: A has root `/t`, B has root `/t/sub`, neither trusted yet; A trusts `/t` together with the parent folders (`parents`) → after `run_until_parked`, B's `LanguageServers` `is_trusted()`. `trust_other_folder_leaves_b`.
- [ ] **Step 2:** FAIL. **Step 3:** Implement. **Step 4:** PASS.
- [ ] **Step 5: Commit** `workbench: trusting a folder in one window updates the others`.

### Task 13: Measure after, thresholds, docs

**Files:**
- Modify: `docs/benchmarks.md`, `bench/compare/results/multiwindow-after.json`, `docs/superpowers/specs/2026-11-12-welcome-design.md` (W1, W8, note "replaced by 2026-11-15-multi-window-design.md" where it says one process per window), `README.md` if it has a sentence about multiple processes, `crates/lucerna-workbench/src/lib.rs`/module docs that say "a process shows one window"
- Test: none added

- [ ] **Step 1:** Build release at the branch head; run `multiwindow.py --mode after` for 5 rounds; run `bench/compare` `startup_ms` and `idle_ram_mb` for one window (baseline: the same command in Task 1 — if Task 1 did not measure these two numbers, measure them at the base commit first).
- [ ] **Step 2:** Compare against the §7.2 thresholds. Not met → stop, record the numbers, report back (do not treat it as done). Met → write the before/after table into `docs/benchmarks.md`.
- [ ] **Step 3:** `grep -rn "one window\|own process\|a process shows" crates docs README.md` and fix the old sentences.
- [ ] **Step 4:** `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, `cargo deny check` → pass.
- [ ] **Step 5: Manual run on macOS** (record the result in the ledger): one Dock icon with 3 windows opened from the menu, Welcome ⌘click, Finder Open With, `luc` in the terminal; ⌘` cycles through all 3; App Exposé shows all 3; ⌘Q with dirty files in two windows; close all windows then click the Dock → Welcome; quit and reopen → all windows are back.
- [ ] **Step 6: Commit** `docs: multi-window measurements and notes`.
