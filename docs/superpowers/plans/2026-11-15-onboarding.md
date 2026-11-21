# First-run onboarding ("Get Started"): implementation plan

> **For the executing agent:** REQUIRED: use the sub-skill superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to carry out this plan task by task. Steps use checkboxes (`- [ ]`) for tracking.

**Goal:** the first time Lucerna opens with nothing to reopen, show the Get Started page, where the user can set the theme (Light/Dark/System), the editor font and font size, Vim mode, and open `settings.json`. Every change is written straight into the user settings. It can be reopened with `lucerna.openOnboarding`, Help ▸ Get Started and a button on Welcome.

**Architecture:**
- Pure part (no gpui): writing several JSONC keys while keeping comments (`settings_edit.rs`), the `onboarded` flag in `<data_dir>/state.json` (`app_state_file.rs`), choosing the startup screen (`startup::startup_screen`), mapping Light/Dark/System to settings keys (`onboarding/model.rs`).
- `OnboardingPage` is a view with `Host::{Window, Tab}`. As `Window` it is the root of a window with no folder open (like `WelcomeView`; `Router` swaps the root back and forth). As `Tab` it is the content of `TabItem::Onboarding` (singleton, not saved), like `LspHealthView`.
- New components live in `ui.rs`. `FontPicker` is built like `RecentPicker`/`ThemePicker`.

**Tech stack:** Rust, GPUI (Zed rev `b5cd310`), `jsonc-parser` (already used in `theme_picker.rs`), serde_json, tempfile for tests.

**Spec:** `docs/superpowers/specs/2026-11-15-onboarding-design.md`. Research: `docs/research/2026-11-15-onboarding.md`. Read the spec before each task; each task lists the § sections it implements.

## Global Constraints

- Change only `crates/lucerna-workbench` (and `crates/lucerna-settings/assets/default-keybindings.json`). Add no dependency. `settings_edit.rs`, `app_state_file.rs`, `onboarding/model.rs`, `startup.rs` do not use gpui.
- Add no macOS-only assumption outside `cfg(target_os = "macos")`.
- The UI flow does not wait on I/O: reading/writing `state.json` (except the synchronous read at startup in `open_without_folder`) and fetching the font list run through `cx.background_spawn`. Writing `settings.json` is synchronous, like `set_theme_setting` today (a small file, the same way the theme picker does it).
- Before every `cargo test`/build: `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"`, `scripts/prune-target.sh`; keep only one profile; do not delete a profile while `pgrep -x cargo` shows another session building; free disk space not below ~1.2 GB. Use the shared `target` folder, and do **not** set `CARGO_TARGET_DIR` or `--target-dir`.
- Commits end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`, staged by specific path (`git commit … -- <paths>`).
- Test command: `cargo test -p lucerna-workbench <filter>` from the repo root.
- Display strings verbatim from the spec: "Welcome to Lucerna", "Set up the basics. Change them any time.", "Finish", "THEME", "EDITOR", "Light", "Dark", "System", "Font Family", "Font Size", "Vim Mode", "Enable Vim keybindings (vim.enabled)", "Open Settings (JSON)", tab "Get Started", command "Help: Get Started", menu item "Get Started".
- Fixed values: maximum column 640 px; padding `8×GRID`; block spacing `6×GRID`; font size 6–100, step 1; theme fallback "Lucerna Light"/"Lucerna Dark"; fallback font "Lilex".
- Command IDs: `lucerna.openOnboarding` (no key), `lucerna.onboarding.finish` (`cmd+enter` when `onboardingFocus`).

## Review Focus

1. **`settings.json` with comments, a trailing comma, or duplicate keys** — every write from the page must keep the rest intact and edit the last copy of a duplicate key (Task 1 has a test).
2. **`settings.json` has a syntax error when a control is clicked** — write nothing, report a config error, the control keeps its old value (Task 6 has a test).
3. **Rapid repeated clicks on + or −** — each click reads the latest merged value, no step is lost, never beyond 6–100 (Task 4 and Task 6 have tests).
4. **`state.json` corrupt or the data directory not writable** — the page still shows, no panic; a corrupt file becomes `.bak` (Task 2 has a test).
5. **Opening the app with a path on the first run** — no onboarding, no flag written (Task 2 checks that `startup_screen` is not called on this path; Task 7 has a router test).

---

### Task 1: Write several JSONC keys (`settings_edit.rs`)

Spec §5.3 (write two keys at once), O6.

**Files:**
- Create: `crates/lucerna-workbench/src/settings_edit.rs`
- Modify: `crates/lucerna-workbench/src/theme_picker.rs` (`set_string_setting`, `write_string_setting`, `write_atomically` move to the new module; `theme_picker` calls back into it), `src/lib.rs` (`mod settings_edit;`)

**Interfaces:**
- Produces:
  - `pub(crate) fn set_settings(text: &str, edits: &[(&str, serde_json::Value)]) -> Result<String, SettingsError>` — applies each top-level key in turn; values are string, bool or number.
  - `pub(crate) fn write_settings(path: &Path, edits: &[(&str, serde_json::Value)]) -> Result<(), SettingsError>` — reads the file (if missing, `USER_SETTINGS_TEMPLATE`), calls `set_settings`, writes atomically once.
  - `pub(crate) fn save_user_settings(edits: &[(&str, serde_json::Value)], cx: &mut App) -> Result<(), ()>` — like the existing `save_setting` (no `ConfigState` means `Ok(())` + warn; on error `record_error`), then `reload_config(&[user_settings], cx)`.
  - `set_string_setting(text, key, value)` keeps its signature and calls `set_settings(text, &[(key, Value::String)])`.

- [ ] **Step 1: Write the failing tests** in `settings_edit.rs` `mod tests`:
  - `writes_bool_and_number`: `set_settings("{\n  // c\n}\n", &[("vim.enabled", json!(true)), ("editor.fontSize", json!(15))])` parses back to `vim.enabled == true`, `editor.fontSize == 15`, and still contains `// c`.
  - `replaces_in_place_keeping_comments`: text has `"editor.fontSize": 14, // size` → the result contains `"editor.fontSize": 15, // size`.
  - `two_keys_one_existing`: `{"window.autoDetectColorScheme": true}` + edits autoDetect=false, colorTheme="Lucerna Dark" → both keys correct, autoDetect appears only once.
  - `edits_last_duplicate`, `trailing_comma_kept_valid`, `empty_text`, `invalid_text_is_error` (`"{ \"a\": "` and `"[]"` are `Err`).
  - The old `set_string_setting` tests (moved here) still pass.
- [ ] **Step 2:** `cargo test -p lucerna-workbench settings_edit` → FAIL (functions not there yet).
- [ ] **Step 3:** Write `set_settings` by repeating the existing `set_string_setting` logic for each edit (re-parse the text after each edit), serializing the value with `serde_json::to_string(&value)`. Move `write_atomically` here. Write `write_settings` and `save_user_settings`; `theme_picker::save_setting` calls `save_user_settings` (do not reload twice: `set_theme_setting` currently reloads itself, so remove that reload).
- [ ] **Step 4:** `cargo test -p lucerna-workbench settings_edit theme_picker icon_theme_picker` → PASS.
- [ ] **Step 5:** Commit `refactor(settings): write several JSONC settings at once`.

### Task 2: The `onboarded` flag and startup screen choice

Spec §4, O2–O4, §10 (upgrading users).

**Files:**
- Create: `crates/lucerna-workbench/src/app_state_file.rs`
- Modify: `src/startup.rs`, `src/lib.rs`

**Interfaces:**
- Consumes: `workspace_state::write_atomic(path, bytes)`.
- Produces:
  - `pub fn state_path(data_dir: &Path) -> PathBuf` → `<data_dir>/state.json`.
  - `pub fn onboarded(data_dir: &Path) -> bool` — file missing/key missing → `false`; corrupt or `version != 1` → renamed to `state.json.bak`, `false`.
  - `pub fn mark_onboarded(data_dir: &Path) -> io::Result<()>` — reads again, sets `onboarded: true`, keeps unknown keys, writes `{ "version": 1, "onboarded": true }` atomically.
  - `startup.rs`: `pub enum StartupScreen { Restore(PathBuf), Onboarding, Welcome, FolderPrompt }` and `pub fn startup_screen(restore: Option<&RecentEntry>, onboarded: Option<bool>, choice: Startup) -> StartupScreen` (`onboarded: None` means there is no data dir).

- [ ] **Step 1: Failing tests.**
  - `app_state_file`: `missing_file_is_not_onboarded`; `mark_then_read`; `corrupt_file_becomes_bak` (`"{"` → `false`, a `state.json.bak` exists); `other_version_becomes_bak`; `no_tmp_left` after `mark_onboarded`; `mark_creates_data_dir` (directory does not exist yet).
  - `startup`: `restore_wins` (`Some(entry)`, `Some(false)` → `Restore(path)`); `first_run_onboarding` (`None`, `Some(false)`, `Startup::Welcome` → `Onboarding`); `first_run_beats_folder_prompt` (`None`, `Some(false)`, `FolderPrompt` → `Onboarding`); `onboarded_follows_choice` (→ `Welcome` / `FolderPrompt`); `no_data_dir_follows_choice` (`onboarded: None` → follows `choice`).
- [ ] **Step 2:** `cargo test -p lucerna-workbench app_state_file startup` → FAIL.
- [ ] **Step 3:** Write the two modules per Interfaces (`.bak` the way `recent::load` does it).
- [ ] **Step 4:** Run again → PASS.
- [ ] **Step 5:** Commit `feat(startup): remember the first run in state.json`.

### Task 3: New components in `ui.rs`

Spec §5.2.

**Files:**
- Modify: `crates/lucerna-workbench/src/ui.rs`
- Test: `crates/lucerna-workbench/tests/ui_controls.rs` (new)

**Interfaces:**
- Produces (each function sets `debug_selector` to `id`, child elements use `"{id}-{suffix}"`):
  - `pub fn segmented_control(id: &'static str, options: &[&'static str], selected: usize, cx: &App) -> Vec<Stateful<Div>>` — returns the buttons so the caller can attach `on_click`; child selector `"{id}-{index}"`; wrapped by `segmented_group(cx) -> Div` (1 px `border` outline, `GRID` radius, `6×GRID` tall). The selected button has background `element_selected`, the others use `ghost_element_hover` on hover.
  - `pub fn switch(id: &'static str, on: bool, cx: &App) -> Stateful<Div>` — `7×GRID`×`4×GRID`, fully rounded; on: background `element_selected`, knob on the right; off: background `element_background`, knob on the left.
  - `pub fn number_stepper(id: &'static str, value: u32, at_min: bool, at_max: bool, cx: &App) -> (Div, Stateful<Div>, Stateful<Div>)` — (frame, `"{id}-minus"` button, `"{id}-plus"` button); a button at its limit is dimmed and ignores clicks. Uses `icon_button`. Add `minus`/`plus` icons to `icons.rs` if missing (Lucide, ISC, license already in the repo).
  - `pub fn dropdown_button(id: &'static str, label: impl Into<SharedString>, cx: &App) -> Stateful<Div>` — outline like `input_box`, `CHEVRON_DOWN_ICON` on the right.
  - `pub fn setting_row(label: &str, description: Option<&str>, control: impl IntoElement, cx: &App) -> Div`.
  - `pub fn theme_preview(id: &'static str, left: &Theme, right: Option<&Theme>, cx: &App) -> Stateful<Div>` — drawn with the colors of the theme passed in; with `right: Some` it is split in half (System). `30×GRID` tall, 8 px radius, 1 px outline. The pseudo-code bar table is a fixed `const PREVIEW_LINES: &[(indent: u8, &[(width: u8, Token)])]`, ~8 lines; `Token` maps to the theme's keyword/function/string/comment/type/variable syntax colors.

- [ ] **Step 1: Failing tests** (`tests/ui_controls.rs`, drawn in a `VisualTestContext`, checked with `debug_bounds`):
  - `segmented_draws_each_option`: 3 options → `seg-0`, `seg-1`, `seg-2` exist.
  - `stepper_buttons_exist`; `switch_draws`; `dropdown_draws`; `theme_preview_split_has_two_halves` (`preview-left`, `preview-right` when `right: Some`, only `preview-left` when `None`).
- [ ] **Step 2:** `cargo test -p lucerna-workbench --test ui_controls` → FAIL.
- [ ] **Step 3:** Write the functions. Before creating one, check `ui.rs` for an equivalent to reuse (`toggle_button`, `checkbox`, `input_box`).
- [ ] **Step 4:** Run again → PASS.
- [ ] **Step 5:** Commit `feat(ui): segmented control, switch, stepper, dropdown and theme preview`.

### Task 4: The page's settings model (`onboarding/model.rs`)

Spec §5.3, §5.4, O7, O9.

**Files:**
- Create: `crates/lucerna-workbench/src/onboarding/mod.rs` (for now only `pub mod model;`), `src/onboarding/model.rs`
- Modify: `src/lib.rs`

**Interfaces:**
- Consumes: `SettingsStore::get`, `ThemeRegistry::get_or_default`, the constants `AUTO_DETECT_KEY`, `COLOR_THEME_KEY`, `PREFERRED_DARK_KEY`, `PREFERRED_LIGHT_KEY` (make them `pub(crate)` in `config_files.rs` if private).
- Produces:
  - `#[derive(Clone, Copy, PartialEq, Eq, Debug)] pub enum ThemeMode { Light, Dark, System }`.
  - `pub fn current_mode(settings: &SettingsStore, themes: &ThemeRegistry) -> ThemeMode` — autodetect (default `true`) → `System`; otherwise by the appearance of the `workbench.colorTheme` theme (not found → `get_or_default`).
  - `pub fn preferred_theme(settings: &SettingsStore, appearance: Appearance) -> String` — reads `PREFERRED_LIGHT_KEY`/`PREFERRED_DARK_KEY` **directly**, fallback "Lucerna Light"/"Lucerna Dark".
  - `pub fn mode_edits(mode: ThemeMode, settings: &SettingsStore) -> Vec<(&'static str, serde_json::Value)>` — System: `[(autoDetect, true)]`; Light: `[(autoDetect, false), (colorTheme, preferred_theme(Light))]`; Dark likewise.
  - `pub fn preview_themes(mode: ThemeMode, settings: &SettingsStore) -> (String, Option<String>)` — Light/Dark: (`workbench.colorTheme` after applying the mode, None); System: (preferred Light, Some(preferred Dark)).
  - `pub const FONT_SIZE_MIN: u32 = 6; pub const FONT_SIZE_MAX: u32 = 100;`
  - `pub fn current_font_size(settings: &SettingsStore) -> u32` (rounded, clamped to 6–100, default 14).
  - `pub fn stepped_font_size(current: u32, delta: i32) -> Option<u32>` — `None` when nothing changes (already at the limit).
  - `pub fn current_font_family(settings: &SettingsStore) -> String` (fallback "Lilex").

- [ ] **Step 1: Failing tests** (`model.rs` `mod tests`, build the store with `SettingsStore::with_defaults()` + `set_layer(Layer::User, json)`):
  - `defaults_are_system`;
  - `autodetect_off_dark_theme_is_dark`, `…light_theme_is_light` (`"workbench.colorTheme": "Lucerna Light"`);
  - `dark_edits_use_preferred_dark` with user `"workbench.preferredDarkColorTheme": "My Dark"` and autodetect `false` → edits contain `("workbench.colorTheme", "My Dark")` (guards against the bug of using `configured_theme_name`);
  - `light_edits_default_to_lucerna_light`; `system_edits_only_autodetect`;
  - `system_preview_is_split`;
  - `font_size_steps_and_clamps`: `stepped(14, 1) == Some(15)`, `stepped(100, 1) == None`, `stepped(6, -1) == None`;
  - `font_size_rounds_and_clamps`: user `13.6` → 14; `200` → 100;
  - `family_fallback_is_lilex` (user `""` also → "Lilex").
- [ ] **Step 2:** `cargo test -p lucerna-workbench onboarding::model` → FAIL.
- [ ] **Step 3:** Write per Interfaces.
- [ ] **Step 4:** Run again → PASS.
- [ ] **Step 5:** Commit `feat(onboarding): map theme modes and font settings`.

### Task 5: `FontPicker`

Spec §5.4 (Font Family), O9.

**Files:**
- Create: `crates/lucerna-workbench/src/font_picker.rs`
- Modify: `src/lib.rs`
- Test: `crates/lucerna-workbench/tests/font_picker.rs`

**Interfaces:**
- Consumes: the picker frame as in `recent_picker.rs` (`popover`, `picker_input_row`, `picker_list`, `picker_row`, `fuzzy_filter`).
- Produces:
  - `pub struct FontPicker`; `pub enum FontPickerEvent { Confirmed(String), Dismissed }`.
  - `FontPicker::new(names: Vec<String>, current: &str, window: &mut Window, cx: &mut Context<Self>) -> Self` — the list has "Lilex" first, then `names` deduplicated (including dropping a duplicate "Lilex"), sorted alphabetically ignoring case; the `current` row is preselected.
  - `pub fn font_names(cx: &App) -> Task<Vec<String>>` — `all_font_names()` taken on a background thread.
  - `row_texts(&self) -> Vec<String>`, `selected_index(&self) -> usize` for tests (like `ThemePicker`).
  - Each row draws the name in that very font. ↑/↓ only move the selected row; Enter → `Confirmed(name)`; Escape → `Dismissed`. No preview.

- [ ] **Step 1: Failing tests:** `lilex_first_and_sorted_deduped` (`["Menlo", "Lilex", "Fira Code", "Menlo"]` → `["Lilex", "Fira Code", "Menlo"]`); `filters_fuzzily` (typing "fir" → only "Fira Code"); `enter_confirms_selected`; `escape_dismisses`; `current_is_selected`; `empty_names_only_lilex`.
- [ ] **Step 2:** `cargo test -p lucerna-workbench --test font_picker` → FAIL.
- [ ] **Step 3:** Write `FontPicker`.
- [ ] **Step 4:** Run again → PASS.
- [ ] **Step 5:** Commit `feat(onboarding): font family picker`.

### Task 6: `OnboardingPage` (root view)

Spec §5.1, §5.3–§5.6, §8, O5, O6, O10.

**Files:**
- Modify: `crates/lucerna-workbench/src/onboarding/mod.rs`, `src/lib.rs` (`pub use onboarding::{OnboardingPage, OnboardingEvent, Host}`)
- Test: `crates/lucerna-workbench/tests/onboarding.rs`

**Interfaces:**
- Consumes: Task 1 `save_user_settings`; Task 3 components; Task 4 model; Task 5 `FontPicker`, `font_names`.
- Produces:
  - `#[derive(Clone, Copy, PartialEq, Eq)] pub enum Host { Window, Tab }`.
  - `pub enum OnboardingEvent { Finish, OpenSettingsInNewWindow(PathBuf), OpenSettingsInWorkbench }`.
  - `OnboardingPage::new(host: Host, window: &mut Window, cx: &mut Context<Self>) -> Self`; `impl Render, Focusable, EventEmitter<OnboardingEvent>`.
  - `pub fn run_command(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) -> bool` (like `WelcomeView::run_command`; handles `lucerna.onboarding.finish`, `workbench.action.openSettingsJson`, and, at `Host::Window`, the commands in `WELCOME_COMMANDS` that make sense without a recent list: About, Quit).
  - `pub const ONBOARDING_COMMANDS: &[&str]` — `lucerna.onboarding.finish`, `workbench.action.openSettingsJson`, `lucerna.openOnboarding`, `workbench.action.showAboutDialog`, `workbench.action.quit`, `workbench.action.files.openFileFolder`, `workbench.action.files.openFile`, `workbench.action.files.openFolder`.
  - Debug selectors: `onboarding-finish`, `onboarding-theme-{0,1,2}` (Light, Dark, System), `onboarding-preview`, `onboarding-font-family`, `onboarding-font-size-minus`/`-plus`/`-value`, `onboarding-vim`, `onboarding-font-sample`, `onboarding-open-settings`.
- Behavior:
  - Draws from `AppState.settings` every frame (no cache), so a hand edit of the file changes the page (`cx.observe_global::<AppState>` → `notify`).
  - Theme: clicking a button other than the selected one → `save_user_settings(&mode_edits(..))`. Clicking the selected button → no write.
  - −/+ → `stepped_font_size` → on `Some(n)` write `editor.fontSize: n`.
  - Vim → write `vim.enabled: !current`.
  - Font Family → opens `FontPicker` in `popover_overlay`; `Confirmed(name)` → write `editor.fontFamily: name`.
  - Open Settings: `Host::Tab` → `OpenSettingsInWorkbench`; `Host::Window` → make sure the file exists (create it from `USER_SETTINGS_TEMPLATE`) then `OpenSettingsInNewWindow(path)`; on error → a `warning_row` at the top of the column.
  - Keys (§5.6): at `Host::Window`, `on_key_down` dispatches through the keymap like `WelcomeView::dispatch_key`, with context `onboardingFocus = picker.is_none()`, `inQuickOpen = picker.is_some()`; only `ONBOARDING_COMMANDS` run. When the picker is open, keys go to the picker first. Then `page_key`: Tab/⇧Tab/↑/↓ move focus in the order Finish → Light → Dark → System → Font Family → − → + → Vim → Open Settings; Enter/Space click; ←/→ inside the theme group change the choice. Initial focus: the selected theme button.
  - The title bar (`layout::title_bar`) is drawn only at `Host::Window`.

- [ ] **Step 1: Failing tests** (`tests/onboarding.rs`, use `watch_config(ConfigPaths::user_only_with_user_dir(tmp))` like `tests/config_files.rs` to get a real user file):
  - `draws_current_settings`: user `{"editor.fontSize": 18, "vim.enabled": true}` → the text of `onboarding-font-size-value` is "18", the switch is on.
  - `dark_writes_autodetect_and_color_theme`: click `onboarding-theme-1` → the file has `"window.autoDetectColorScheme": false`, `"workbench.colorTheme": "Lucerna Dark"`, the template comment is still there; `AppState.theme.name == "Lucerna Dark"`.
  - `system_writes_autodetect_true`.
  - `plus_three_times_writes_17` (from 14) — Review Focus 3.
  - `plus_at_100_writes_nothing` (the file is unchanged by a single byte).
  - `vim_toggle_writes_true`.
  - `broken_settings_not_written`: file `{ "a": ` → click Dark → the file is unchanged, a config error is recorded — Review Focus 2.
  - `reflects_hand_edit`: hand-write `"editor.fontSize": 20` + `reload_config` → value "20".
  - `cmd_enter_emits_finish`; `cmd_enter_in_font_picker_does_nothing` (picker open, ⌘Enter → no `Finish`).
  - `open_settings_in_window_creates_file`: no `settings.json` → click → the file exists, event `OpenSettingsInNewWindow(path)`.
  - `font_picker_confirm_writes_family`.
- [ ] **Step 2:** `cargo test -p lucerna-workbench --test onboarding` → FAIL.
- [ ] **Step 3:** Write `OnboardingPage` per Interfaces and the §5.1 layout (640 px column, logo/title as on Welcome, `section_header` "THEME"/"EDITOR", `primary_text_button` Finish with its key, `list_button` Open Settings with its key from the keymap, the `onboarding-font-sample` box is a `subheader_card` with two lines `fn main() {` / `    println!("Hello, Lucerna");` in the current font/size and the active theme's syntax colors). The column scrolls vertically.
- [ ] **Step 4:** Run again → PASS.
- [ ] **Step 5:** Commit `feat(onboarding): the Get Started page`.

### Task 7: Router, Welcome, Help menu

Spec §4, §5.5 (root), §6 (Welcome, Help), O1–O5, O8.

**Files:**
- Modify: `crates/lucerna-workbench/src/app.rs`, `src/welcome.rs`, `src/app_menu.rs`
- Test: `tests/welcome.rs`, `src/app_menu.rs` `mod tests`, `tests/onboarding.rs`

**Interfaces:**
- Consumes: Task 2 `onboarded`, `mark_onboarded`, `startup_screen`; Task 6 `OnboardingPage`, `OnboardingEvent`, `ONBOARDING_COMMANDS`.
- Produces:
  - `app.rs`: `Shown::Onboarding(WindowHandle<OnboardingPage>)`; `Router::open_onboarding(this, cx)` (new window, like `open_welcome`); `Router::replace_with_onboarding(this, cx)` and `replace_with_welcome` work when the root is Onboarding; `on_onboarding_event`: `Finish` → `replace_with_welcome`; `OpenSettingsInNewWindow(path)` → `process::spawn` like Welcome's "new window" (on error → `tracing::error`).
  - `pub(crate) fn first_screen(data_dir: Option<&Path>, settings: &SettingsStore) -> StartupScreen` — no gpui: reads `recent::load` + `last_workspace` (only when `restores_last_workspace`), `onboarded`, `startup_choice`, then calls `startup_screen`.
  - `open_without_folder` calls `first_screen` then `match`; for `Onboarding` it runs `mark_onboarded` in the background (on error → `tracing::warn`) then `open_onboarding`. The `open_urls`/CLI path is unchanged.
  - `welcome.rs`: a "Get Started" button after "Open Recent…" (`Button::Command("lucerna.openOnboarding")`), add `lucerna.openOnboarding` to `WELCOME_COMMANDS`, `WelcomeEvent::OpenOnboarding` → router `replace_with_onboarding`.
  - `app_menu.rs`: `WindowKind::Onboarding`; Help starts with `command("Get Started", "lucerna.openOnboarding")`, a separator, then the old items; at `Welcome` this item is also enabled (already in `WELCOME_COMMANDS`); at `Onboarding` enabled per `ONBOARDING_COMMANDS`. The menu's `run_command` tries `WindowHandle::<OnboardingPage>` after Welcome. `active_window` returns `WindowKind::Onboarding` for the onboarding root.

- [ ] **Step 1: Failing tests:**
  - `app_menu` unit: `help_starts_with_get_started`; `welcome_enables_get_started`; `onboarding_enables_only_its_commands` (Save disabled, Get Started/About/Quit enabled).
  - `tests/welcome.rs`: `get_started_button_emits_open_onboarding` (clicking the button → `WelcomeEvent::OpenOnboarding`); the old `ctrl_r` still works.
  - `app.rs` unit (`mod tests`, temp dir): `first_screen_fresh_is_onboarding` (nothing there → `Onboarding`); `first_screen_restores_existing_folder` (`recent.json` has an existing folder → `Restore`); `first_screen_after_onboarded_is_welcome` (`state.json` onboarded, no recent → `Welcome`); `first_screen_restore_none_first_run` (`window.restoreWindows: "none"` + recent has a folder → `Onboarding`); `first_screen_no_data_dir` (`None` → `Welcome`).
- [ ] **Step 2:** `cargo test -p lucerna-workbench app_menu welcome onboarding` → FAIL.
- [ ] **Step 3:** Write per Interfaces.
- [ ] **Step 4:** Run again → PASS; also the old `tests/welcome.rs`.
- [ ] **Step 5:** Commit `feat(onboarding): show Get Started on the first run, from Welcome and Help`.

### Task 8: The Get Started tab in the workbench

Spec §6 (command, tab), §5.6 (`onboardingFocus`, ⌘Enter closes the tab), O5.

**Files:**
- Modify: `crates/lucerna-workbench/src/editor_groups/model.rs`, `src/editor_groups/view.rs`, `src/workbench.rs`, `src/onboarding/mod.rs`, `crates/lucerna-settings/assets/default-keybindings.json`
- Test: `tests/onboarding.rs`, `tests/editor_groups_model.rs`

**Interfaces:**
- Consumes: Task 6 `OnboardingPage::new(Host::Tab, …)`.
- Produces:
  - `TabItem::Onboarding` — singleton, never preview, **never saved** (doc comment like `LspHealth`); title "Get Started"; icon up to the implementer (spec §11).
  - `EditorGroups::open_onboarding(&mut self) -> TabId` like `open_lsp_health`.
  - `onboarding::register_commands(commands: &mut CommandRegistry)` registers `lucerna.openOnboarding` ("Help: Get Started") and `lucerna.onboarding.finish`; called from the workbench's `register_commands`.
  - `Workbench`: one `Entity<OnboardingPage>` created lazily when the tab is first opened; `OnboardingEvent::Finish` → closes the Onboarding tab; `OpenSettingsInWorkbench` → `run_command("workbench.action.openSettingsJson")`.
  - `context_keys`: `flag("onboardingFocus", self.onboarding_focused(window, cx))` — true when focus is inside `OnboardingPage` and its picker is not open.
  - `default-keybindings.json`: `{ "key": "cmd+enter", "command": "lucerna.onboarding.finish", "when": "onboardingFocus" }`.
  - Workspace state skips `TabItem::Onboarding` when saving.

- [ ] **Step 1: Failing tests:**
  - `editor_groups_model`: `onboarding_is_singleton` (open twice → one tab, the old tab is activated); `onboarding_not_saved` (the serialized state does not contain it).
  - `tests/onboarding.rs`: `command_opens_tab` (run `lucerna.openOnboarding` twice → one "Get Started" tab); `cmd_enter_closes_tab`; `cmd_enter_in_editor_still_inserts_line` (focus the editor, ⌘Enter → new line, the Onboarding tab is not closed); `dark_from_tab_changes_theme`.
- [ ] **Step 2:** `cargo test -p lucerna-workbench --test onboarding --test editor_groups_model` → FAIL.
- [ ] **Step 3:** Write per Interfaces.
- [ ] **Step 4:** Run again → PASS.
- [ ] **Step 5:** Commit `feat(onboarding): the Get Started tab and lucerna.openOnboarding`.

### Task 9: Full check and comparison with Zed

Spec §9 (manual), design-principles §1.1.

- [ ] **Step 1:** `scripts/prune-target.sh`, then `cargo test -p lucerna-workbench` → all tests green.
- [ ] **Step 2:** `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`, `cargo deny check` → clean.
- [ ] **Step 3:** Update the status line of `docs/superpowers/specs/2026-11-15-onboarding-design.md`: "implemented (commit …)", and any other places where the spec differs.
- [ ] **Step 4 (manual, needs the user or a session with the app):** build with `scripts/bundle-app.sh`, run with a fresh `HOME`: the first run shows the page; Dark/+/Vim write correctly; ⌘Enter → Welcome; quit, reopen → Welcome; open a folder, quit, reopen → the folder; Help ▸ Get Started in the workbench opens a tab. Put it next to Zed (`zed: open onboarding`) and compare spacing, font size, buttons, switch, hover.
- [ ] **Step 5:** Commit `docs(onboarding): mark the spec implemented`.
