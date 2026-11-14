# Explorer Context Menu — Part 1 (menu, selection, read-only commands) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Right-clicking an Explorer row (or `shift+f10`) opens a Zed-style context menu with the read-only commands Open to the Side, Reveal in Finder, Open in Terminal, Find in Folder…, Copy Path and Copy Relative Path, each also bound to its VS Code key.

**Architecture:** A reusable `ContextMenu` entity (own focus handle, emits `DismissEvent`, runs `CommandRegistry` IDs) drawn with `deferred(anchored())`. `ExplorerModel` gains a selected path; `ExplorerView` deploys the menu and owns it. Commands act on the Explorer's selection, or on `args.path` when given. OS side effects go through a swappable `SystemShell` global so tests can record them (GPUI's test platform panics on `reveal_path`).

**Tech Stack:** Rust, GPUI (`lucerna-workbench`), `lucerna-theme`, `lucerna-settings`.

**Spec:** `docs/superpowers/specs/2026-11-13-explorer-context-menu-design.md` (sections 1–3, 6, 7, 8). Research: `docs/research/2026-11-13-explorer-context-menu.md`. Part 2 (New File/Folder, Rename, Delete) is `docs/superpowers/plans/2026-11-13-explorer-context-menu-p2.md`.

## Global Constraints

- Only `lucerna-workbench` uses gpui. No macOS-only code outside `#[cfg(target_os = …)]` blocks.
- Command IDs, titles and keys are VS Code's (spec §3 table). Keys go in `crates/lucerna-settings/assets/default-keybindings.json`; it has macOS bindings only, so add the macOS key of each (Linux/Windows keys wait for per-platform keymaps).
- Menu labels exactly: `New File…`, `New Folder…`, `Open to the Side`, `Reveal in Finder` (Linux `Open Containing Folder`, Windows `Reveal in File Explorer`), `Open in Terminal`, `Find in Folder…`, `Copy Path`, `Copy Relative Path`, `Rename…`, `Delete`.
- Menu metrics: min width 200 px; max height 75% of the window; item height `6. * GRID`, padding x `2. * GRID`, radius `GRID`; label `UI_FONT_SIZE_SMALL`; separator 1 px `border_variant` with `GRID` margin top and bottom; 8 px from window edges.
- Learn from Zed's `ui::ContextMenu` and `project_panel` but never copy their code (GPL).
- Before any `cargo test`: `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"`, delete `target/release` if present (never while another cargo runs: `pgrep -x cargo`), run `scripts/prune-target.sh`. Keep ≥1.2 GB free.
- Commit with explicit paths (`git commit -m … -- <paths>`), trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

- **Menu open while pressing `enter` / `cmd+backspace`:** must act in the menu, never on the Explorer. `filesExplorerFocus` is false while a menu is open (Task 3 test `explorer_keys_off_while_menu_open`).
- **Right-click a second row while a menu is open:** old menu closes, new one opens for the new row, one outline only (Task 4 test `right_click_elsewhere_replaces_menu`).
- **Menu near the bottom or right edge of a small window:** stays fully inside with 8 px margin (Task 4 test `menu_stays_inside_window`).
- **Selected row disappears** (its folder collapses or the file is deleted on disk): selection falls back to the nearest visible ancestor, root last (Task 3 test `selection_falls_back_to_visible_ancestor`).
- **Single-file mode:** menu acts on the lone file, offers only the five file commands, and Copy Relative Path copies the file name (Task 4 test `single_file_menu_is_read_only`).

---

### Task 1: `ContextMenu` component and theme colors

**Files:**
- Modify: `crates/lucerna-theme/src/lib.rs` (`UiColors`: add `text_disabled`, `border_focused`; fill both in `UiColors::fallback`)
- Modify: `crates/lucerna-theme/src/schema.rs` (read `text.disabled` → fallback `text_muted`; `border.focused` → fallback `border`)
- Create: `crates/lucerna-workbench/src/context_menu.rs`
- Modify: `crates/lucerna-workbench/src/ui.rs` (`pub use crate::context_menu::{ContextMenu, MenuItem, deploy_context_menu, DeployedMenu};`), `src/lib.rs` (`mod context_menu;`)
- Test: `crates/lucerna-theme/tests/theme.rs`, unit tests in `context_menu.rs`, new `crates/lucerna-workbench/tests/context_menu.rs`

**Interfaces:**
- Produces:
  - `pub enum MenuItem { Entry { label: SharedString, command: &'static str, args: Option<serde_json::Value>, disabled: bool }, Separator }`
  - `pub struct ContextMenu` (entity). Builder, consuming `self`: `ContextMenu::new(owner: FocusHandle, cx: &mut Context<Self>) -> Self`, `.entry(label, command) -> Self`, `.entry_with_args(label, command, args: Value) -> Self`, `.disabled_when(bool) -> Self` (applies to the last entry), `.separator() -> Self`, `.when(cond, |m| m) -> Self` (GPUI's `FluentBuilder`).
  - `pub fn items(&self) -> &[MenuItem]` — already normalized: no leading/trailing separator, no two in a row.
  - `pub fn selected(&self) -> Option<usize>`, `pub fn select_next/select_prev/select_first/select_last(&mut self, cx)`.
  - `impl EventEmitter<DismissEvent>`, `impl EventEmitter<MenuCommand>` where `pub struct MenuCommand { pub command: &'static str, pub args: Option<Value> }` — emitted on confirm, **after** `DismissEvent`.
  - `pub struct DeployedMenu { pub menu: Entity<ContextMenu>, pub position: Point<Pixels>, _subs: Vec<Subscription> }` and `pub fn deploy_context_menu<V: 'static>(menu: Entity<ContextMenu>, position: Point<Pixels>, window: &mut Window, cx: &mut Context<V>, on_close: impl Fn(&mut V, Option<MenuCommand>, &mut Window, &mut Context<V>) + 'static) -> DeployedMenu` — focuses the menu; on dismiss refocuses `owner` then calls `on_close(view, command_or_none)`.
  - `pub fn render_deployed(d: &DeployedMenu) -> impl IntoElement` — `deferred(anchored().position(d.position).snap_to_window_with_margin(px(8.)).child(d.menu.clone())).with_priority(3)`.
  - Debug selectors: `context-menu`, `context-menu-item-{command}`, `context-menu-separator-{ix}`.

- [ ] **Step 1: Write the failing theme test** in `crates/lucerna-theme/tests/theme.rs`:
  `zed_theme_reads_text_disabled_and_border_focused` (a theme JSON with `"text.disabled": "#111111ff"`, `"border.focused": "#222222ff"` → those values) and `missing_disabled_and_focused_fall_back` (absent → `ui.text_disabled == ui.text_muted`, `ui.border_focused == ui.border`).
- [ ] **Step 2:** `cargo test -p lucerna-theme --test theme` → FAIL (no field).
- [ ] **Step 3:** Add the two fields, fallback values and schema reads.
- [ ] **Step 4:** Same command → PASS.
- [ ] **Step 5: Write failing unit tests** in `context_menu.rs` (`#[cfg(test)]`, pure functions over `Vec<MenuItem>`; put the normalization and navigation logic in free functions `normalize(Vec<MenuItem>) -> Vec<MenuItem>` and `next_enabled(items, from: Option<usize>, forward: bool) -> Option<usize>` so they test without GPUI):
  - `normalize_drops_leading_trailing_and_double_separators`: `[Sep, A, Sep, Sep, B, Sep]` → `[A, Sep, B]`; `[Sep]` → `[]`.
  - `next_skips_separators_and_disabled`: items `[A, Sep, B(disabled), C]`: from `None` forward → `Some(0)`; from `None` backward → `Some(3)`; from `Some(0)` forward → `Some(3)`; from `Some(3)` forward → `Some(0)` (wraps); all disabled → `None`.
- [ ] **Step 6:** `cargo test -p lucerna-workbench --lib context_menu` → FAIL.
- [ ] **Step 7:** Implement `normalize`, `next_enabled`, the builder and state.
- [ ] **Step 8:** Same command → PASS.
- [ ] **Step 9: Write failing GPUI tests** in `tests/context_menu.rs` (a tiny host view that builds a menu `[Copy Path (copyFilePath), Sep, Rename… (renameFile, disabled)]`, deploys it at (50,50), records `on_close` results; reuse the `init` helper shape from `tests/explorer_view.rs`):
  - `opens_with_nothing_selected_and_down_selects_first`: after deploy, `selected() == None`; `simulate_keystrokes("down")` → `Some(0)`.
  - `enter_runs_selected_and_closes`: `down enter` → `on_close` got `Some(MenuCommand { command: "copyFilePath", .. })`, `debug_bounds("context-menu")` is `None`, host focus handle is focused.
  - `escape_closes_without_command`: `escape` → `on_close(None)`, menu gone, host focused.
  - `click_outside_closes`: `simulate_click` at (5, 5) outside menu bounds → `on_close(None)`.
  - `disabled_entry_does_nothing_on_click`: clicking `context-menu-item-renameFile` → no `on_close`, menu stays.
  - `shows_keybinding_label`: load the keymap with a user layer `[{ "key": "alt+cmd+c", "command": "copyFilePath" }]` (Task 5 adds the default later); the item's text includes `keymap.label_for("copyFilePath").unwrap()`.
- [ ] **Step 10:** `cargo test -p lucerna-workbench --test context_menu` → FAIL.
- [ ] **Step 11:** Implement `Render for ContextMenu` and `deploy_context_menu`/`render_deployed`. Keys are handled in the menu's own `on_key_down` (`up`, `down`, `home`, `end`, `enter`, `escape`), as `quick_open/view.rs::key_down` does — the workbench keymap resolves `NoMatch` for them while `filesExplorerFocus` is false (Task 3), so they propagate to the focused menu. Close on `on_mouse_down_out` (left or right). Frame: `ui::popover`'s bg, border, radius and shadow without its max width; padding `GRID`; `.occlude()`; `overflow_y_scroll` with `max_h(window.viewport_size().height * 0.75)`. Item hover / keyboard-selected bg `ghost_element_hover`; disabled text `text_disabled`, no hover; key label right-aligned in `text_muted` at `UI_FONT_SIZE_STATUS`.
- [ ] **Step 12:** Same command → PASS.
- [ ] **Step 13: Commit** — `feat(ui): reusable context menu`, paths: the theme files, `context_menu.rs`, `ui.rs`, `lib.rs`, both test files.

### Task 2: Explorer selection model

**Files:**
- Modify: `crates/lucerna-workbench/src/explorer/model.rs`
- Test: `crates/lucerna-workbench/tests/explorer_model.rs`

**Interfaces:**
- Produces on `ExplorerModel`:
  - `pub enum Selection { Root, Path(Arc<Path>) }` (re-export from `explorer/mod.rs`)
  - `pub fn selection(&self) -> &Selection`, `pub fn select(&mut self, sel: Selection)`
  - `pub fn selected_kind(&self) -> SelectedKind` where `pub enum SelectedKind { Root, Dir, File }`
  - `set_snapshot` and `toggle` re-run `fix_selection()`: a selected path that is no longer a visible row becomes its nearest visible ancestor, else `Root`.

- [ ] **Step 1: Write failing tests** in `tests/explorer_model.rs`:
  - `selection_starts_at_root`.
  - `select_path_reports_kind`: select `src` → `Dir`; `a.rs` → `File`; `Root` → `Root`.
  - `selection_falls_back_to_visible_ancestor`: expand `src`, select `src/main.rs`, `toggle("src")` → `Selection::Path("src")`; then a snapshot without `src` → `Root`.
- [ ] **Step 2:** `cargo test -p lucerna-workbench --test explorer_model` → FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Same → PASS.
- [ ] **Step 5: Commit** — `feat(explorer): selection model`.

### Task 3: Selection in the view, focus and context keys

**Files:**
- Modify: `crates/lucerna-workbench/src/explorer/view.rs` (click handlers, `sync_active_tab`, `render_rows`, `render_root`, new `context_menu: Option<DeployedMenu>` field and `pub fn context_menu_open(&self) -> bool`)
- Modify: `crates/lucerna-workbench/src/workbench.rs` (`context_keys`)
- Test: `crates/lucerna-workbench/tests/explorer_view.rs`

**Interfaces:**
- Consumes: Task 2 `Selection`, `SelectedKind`.
- Produces context keys: `filesExplorerFocus` = `sidebar_focus && !explorer.context_menu_open()`; `explorerResourceIsFolder` = kind is `Dir` or `Root`; `explorerResourceIsRoot` = kind is `Root`; `contextMenuVisible` = `explorer.context_menu_open()`. (`explorerViewletFocus` already equals sidebar focus and the sidebar only ever shows the Explorer, so spec §2's fix needs no change; leave it.)
- Produces `pub fn selection(&self) -> &Selection` on `ExplorerView` (delegates) and `pub(crate) fn selected_abs_path(&self) -> PathBuf` (root joined; the root itself for `Root`; in single-file mode `Root` maps to the lone file).

- [ ] **Step 1: Write failing tests** in `tests/explorer_view.rs`:
  - `left_click_selects_and_focuses_sidebar`: click `explorer-row-src` → selection `Path("src")`; `cx.update(|w, cx| wb.read(cx).context_keys(w, cx))` has `filesExplorerFocus == true`, `explorerResourceIsFolder == true`, `explorerResourceIsRoot == false`.
  - `click_root_row_selects_root`: click `explorer-root` → `Root`, `explorerResourceIsRoot == true`.
  - `selection_follows_active_tab`: `wb.open_path("b.rs", Pinned)` → selection `Path("b.rs")`.
  - `selected_row_is_highlighted`: the row's background equals `element_selected` (read via the existing pattern in `active_tab_is_revealed`; if no colour probe exists, assert through `debug_selector` `explorer-row-selected-{path}` on the selected row only).
  - `explorer_keys_off_while_menu_open` is written in Task 4 (needs the menu).
- [ ] **Step 2:** `cargo test -p lucerna-workbench --test explorer_view` → new tests FAIL.
- [ ] **Step 3:** Implement: clicks call `model.select` then the existing behaviour, and `window.focus(sidebar focus)` via `Workbench::focus_sidebar` (rows already sit inside the sidebar's `track_focus`; focus explicitly so the click works even when the editor held focus). Selected-row bg comes from selection instead of `self.active`; the root row gets `.id("explorer-root")`, an `on_click` that selects `Root`, and the same bg when selected. `sync_active_tab` also `select`s the active file when `explorer.autoReveal` is on.
- [ ] **Step 4:** Same → PASS, and the whole `explorer_view` file passes.
- [ ] **Step 5: Commit** — `feat(explorer): selected row and explorer context keys`.

### Task 4: Deploy the menu from the Explorer

**Files:**
- Create: `crates/lucerna-workbench/src/explorer/menu.rs` (`fn build_menu(kind: SelectedKind, single_file: bool, owner: FocusHandle, cx) -> ContextMenu`)
- Modify: `crates/lucerna-workbench/src/explorer/view.rs` (right-click on rows, root row and the empty list area; outline; render the deployed menu; run the chosen command)
- Modify: `crates/lucerna-workbench/src/explorer/mod.rs`
- Modify: `crates/lucerna-settings/assets/default-keybindings.json` (`shift+f10` → `filesExplorer.openContextMenu`, `when: filesExplorerFocus`)
- Modify: `crates/lucerna-workbench/src/workbench.rs` (`register_shell_commands`: `filesExplorer.openContextMenu`, title `Explorer: Open Context Menu`)
- Test: `crates/lucerna-workbench/tests/explorer_view.rs`

**Interfaces:**
- Consumes: Task 1 `ContextMenu`, `deploy_context_menu`, `render_deployed`, `MenuCommand`; Task 3 selection.
- Produces: `pub(crate) fn open_context_menu(&mut self, position: Option<Point<Pixels>>, window, cx)` on `ExplorerView` (`None` = just below the selected row's bounds, left-aligned with its label). On `on_close(Some(cmd))` the view calls `workbench.run_command(cmd.command, cmd.args.as_ref(), window, cx)`.
- Item order and visibility exactly as spec §3 Part 2 adds `explorer.newFile`, `explorer.newFolder`, `renameFile` and `moveFileToTrash` to `build_menu`; keep the separators of spec §3 now, `normalize` drops the ones left dangling.

- [ ] **Step 1: Write failing tests** (helper `right_click(selector, cx)` = `simulate_event` `MouseDownEvent`/`MouseUpEvent` with `MouseButton::Right`; helper `menu_commands(cx) -> Vec<String>` reads the deployed menu's `items()` through `wb.read(cx).explorer()`):
  - `right_click_file_shows_file_menu`: right-click `explorer-row-a.rs` → selection `a.rs`; commands `["explorer.openToSide", "revealFileInOS", "openInTerminal", "copyFilePath", "copyRelativeFilePath"]`.
  - `right_click_folder_shows_folder_menu`: `src` → `["revealFileInOS", "openInTerminal", "filesExplorer.findInFolder", "copyFilePath", "copyRelativeFilePath"]`.
  - `right_click_empty_space_is_root_menu`: right-click below the last row inside `explorer-list` → selection `Root`, same commands as a folder.
  - `menu_row_gets_outline`: while open, `explorer-row-outline-a.rs` is drawn; after `escape` it is not.
  - `explorer_keys_off_while_menu_open`: context keys show `filesExplorerFocus == false`, `contextMenuVisible == true`.
  - `right_click_elsewhere_replaces_menu`: right-click `a.rs` then `b.rs` → one `context-menu`, selection `b.rs`, outline only on `b.rs`.
  - `shift_f10_opens_menu_below_selected_row`: click `a.rs`, `simulate_keystrokes("shift-f10")` → menu top ≥ row bottom.
  - `menu_stays_inside_window`: window 400×300 (`cx.simulate_resize`), right-click at (395, 295) → menu bounds within `(8, 8)..(392, 292)`.
  - `single_file_menu_is_read_only`: in `Workbench::single_file`, right-click `explorer-row-notes.md` → `["explorer.openToSide", "revealFileInOS", "openInTerminal", "copyFilePath", "copyRelativeFilePath"]`; right-click empty space → same list, selection still the file.
- [ ] **Step 2:** Run → FAIL.
- [ ] **Step 3:** Implement. Rows: `.on_mouse_down(MouseButton::Right, …)` selects and opens at `ev.position`. Empty area: a right-click handler on the `explorer-list` container that fires only when no row handled it (`cx.stop_propagation()` in the row handler). Outline: `border_1()` in `border_focused`, inset, on the selected row while `context_menu.is_some()`, debug selector `explorer-row-outline-{path}`. Render `render_deployed` as the last child of the Explorer root div.
- [ ] **Step 4:** Run → PASS.
- [ ] **Step 5: Commit** — `feat(explorer): right-click and shift+f10 open the context menu`.

### Task 5: Read-only commands

**Files:**
- Create: `crates/lucerna-workbench/src/system_shell.rs`
- Create: `crates/lucerna-workbench/src/explorer/commands.rs` (`pub(crate) fn register_commands(&mut CommandRegistry)`; called from `default_registry`)
- Modify: `crates/lucerna-workbench/src/workbench.rs` (`default_registry`), `src/lib.rs`, `src/app.rs` (install the real `SystemShell` at startup)
- Modify: `crates/lucerna-settings/assets/default-settings.json`, `default-keybindings.json`
- Test: new `crates/lucerna-workbench/tests/explorer_commands.rs`

**Interfaces:**
- Produces:
  - `pub trait SystemShell: 'static { fn reveal(&self, path: &Path, cx: &App); fn open_terminal(&self, dir: &Path, exec: &str) -> std::io::Result<()>; }`
  - `pub struct SystemShellGlobal(pub Arc<dyn SystemShell>)` (GPUI `Global`); `pub fn system_shell(cx: &App) -> Arc<dyn SystemShell>` returns the global, or `NativeShell` when unset.
  - `NativeShell`: `reveal` → `cx.reveal_path(path)` (cross-platform; panics on GPUI's test platform, hence the trait). `open_terminal`: macOS `open -a <exec> <dir>`; Linux spawn `<exec>` with `current_dir(dir)`; Windows spawn `cmd /c start "" <exec>` with `current_dir(dir)`.
  - `pub(crate) fn command_target(wb: &Workbench, args: Option<&Value>, cx: &App) -> (PathBuf, bool /*is_dir*/)` — `args.path` (absolute or root-relative) if present, else the Explorer's `selected_abs_path`.
  - `pub(crate) fn relative_path_text(root: &Path, target: &Path, separator: &str, root_name: &str) -> String` (pure).
- Commands (ID, title for the palette):
  - `explorer.openToSide` "Explorer: Open to the Side" → `open_path(target, OpenMode::ToSide)`; no-op on a directory.
  - `revealFileInOS` "File: Reveal in Finder" → `system_shell(cx).reveal(&target, cx)`.
  - `openInTerminal` "Explorer: Open in Terminal" → dir = target or its parent; exec = `terminal.external.osxExec` / `linuxExec` / `windowsExec` by `cfg!(target_os)`; on `Err(e)` → `status_message::show(format!("Couldn't open the terminal: {e}"), cx)`.
  - `copyFilePath` "File: Copy Path of Active File" → clipboard ← absolute path.
  - `copyRelativeFilePath` "File: Copy Relative Path of Active File" → clipboard ← `relative_path_text(...)`, separator from `explorer.copyRelativePathSeparator` (`"auto"` → `std::path::MAIN_SEPARATOR_STR`); for the root, the root folder name; in single-file mode, the file name.
- Settings defaults (add to `default-settings.json`): `"explorer.copyRelativePathSeparator": "auto"`, `"terminal.external.osxExec": "Terminal.app"`, `"terminal.external.linuxExec": "x-terminal-emulator"`, `"terminal.external.windowsExec": "cmd.exe"`. (`explorer.confirmDelete` comes in Part 2.)
- Keybindings (macOS): `ctrl+enter` `explorer.openToSide` `filesExplorerFocus && !explorerResourceIsFolder`; `alt+cmd+r` `revealFileInOS` `filesExplorerFocus`; `alt+cmd+c` `copyFilePath` `filesExplorerFocus`; `alt+cmd+shift+c` `copyRelativeFilePath` `filesExplorerFocus`.

- [ ] **Step 1: Write failing unit tests** for `relative_path_text` (in `explorer/commands.rs`): `("/p", "/p/src/a.rs", "/", "p") → "src/a.rs"`; separator `"\\"` → `"src\\a.rs"`; target == root → `"p"`.
- [ ] **Step 2: Write failing GPUI tests** in `tests/explorer_commands.rs` (install a `RecordingShell` that pushes `("reveal", path)` / `("terminal", dir, exec)` into an `Arc<Mutex<Vec<…>>>`; copy `init`/`open`/`right_click` helpers from `explorer_view.rs`):
  - `menu_copy_path_writes_absolute_path`: right-click `a.rs`, click `context-menu-item-copyFilePath` → `cx.read_from_clipboard()` text == `<root>/a.rs`.
  - `copy_relative_path_key`: click `src`, expand, click `src/main.rs`, `simulate_keystrokes("alt-cmd-shift-c")` → `"src/main.rs"`.
  - `reveal_records_target`: click `a.rs`, `alt-cmd-r` → recorded `("reveal", <root>/a.rs)`.
  - `terminal_opens_parent_of_file_with_setting`: user settings `{"terminal.external.osxExec": "iTerm.app"}` (gate assertion with `#[cfg(target_os = "macos")]`), menu on `a.rs` → `("terminal", <root>, "iTerm.app")`.
  - `terminal_error_shows_status`: shell returns `Err(NotFound)` → `status_message::current` contains `Couldn't open the terminal`.
  - `open_to_side_uses_second_group`: click `a.rs`, `ctrl-enter` → two groups, the second holds `a.rs`.
  - `args_path_overrides_selection`: `wb.run_command("copyFilePath", Some(&json!({"path": "b.rs"})))` with `a.rs` selected → clipboard `<root>/b.rs`.
- [ ] **Step 3:** Run both → FAIL.
- [ ] **Step 4:** Implement `system_shell.rs`, `explorer/commands.rs`, settings, keybindings, registration.
- [ ] **Step 5:** Run → PASS. Also `cargo test -p lucerna-settings` (keymap loads; no duplicate-key errors).
- [ ] **Step 6: Commit** — `feat(explorer): read-only context menu commands`.

### Task 6: Find in Folder…

**Files:**
- Modify: `crates/lucerna-workbench/src/search_view/view.rs` (new `pub fn set_include(&mut self, text: &str, window, cx)`: sets the include input, shows the filters row, does not move focus)
- Modify: `crates/lucerna-workbench/src/explorer/commands.rs`, `default-keybindings.json`
- Test: `crates/lucerna-workbench/tests/explorer_commands.rs`

**Interfaces:**
- Consumes: `Workbench::open_search`, `search_view().include_input()`.
- Command `filesExplorer.findInFolder` "Explorer: Find in Folder…": target dir's root-relative path with `/` separators (`""` for root) → `search_view.set_include`, then `open_search` (focuses the query). No-op on a file. Key `alt+shift+f`, `when: filesExplorerFocus && explorerResourceIsFolder`.

- [ ] **Step 1: Write failing tests:**
  - `find_in_folder_fills_include_and_focuses_query`: right-click `src`, click `context-menu-item-filesExplorer.findInFolder` → active tab is Search; `include_input` text == `"src"`; query input focused.
  - `find_in_folder_on_root_clears_include`: preset include `"x"`, run on root → `""`.
- [ ] **Step 2:** Run → FAIL. **Step 3:** Implement. **Step 4:** Run → PASS.
- [ ] **Step 5: Commit** — `feat(explorer): Find in Folder`.

### Task 7: Whole-part check

- [ ] **Step 1:** `cargo test -p lucerna-theme -p lucerna-settings -p lucerna-workbench` → all PASS. `cargo clippy --workspace --all-targets -- -D warnings` → clean. `cargo deny check` → ok.
- [ ] **Step 2:** UI review against Zed (design-principles §1.1): build (`scripts/bundle-app.sh` or `cargo run -p lucerna`), open this repo, right-click a file and a folder; compare with Zed's project panel menu: width, item height, padding, separator spacing, key labels, hover, outline on the row. Note differences in the commit message or fix them.
- [ ] **Step 3:** Commit any fixes with explicit paths.
