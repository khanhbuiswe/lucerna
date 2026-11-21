# Lucerna Theme Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace One Dark/One Light with Lucerna Dark/Lucerna Light as the only bundled themes, meeting WCAG AA, and switching automatically with the system light/dark mode.

**Architecture:** A single `lucerna.json` file in Zed format in `lucerna-theme`, with a key-set test and a contrast test. The workbench remembers the system appearance in `AppState`, picks the theme name with a pure function from settings, and updates when the window reports an appearance change. The picker writes the preferred setting while following the system automatically.

**Tech Stack:** Rust, GPUI (rev `b5cd310`), serde_json/jsonc, Python 3 for `gen-licenses.py`.

**Spec:** `docs/superpowers/specs/2026-11-14-lucerna-theme-design.md` (approved). The executor reads the whole spec, especially the color tables.

## Global Constraints

- Work in the worktree `.claude/worktrees/lucerna-theme`, branch `claude/lucerna-theme`. No merging, no pushing.
- Cargo: `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"`. Use `CARGO_TARGET_DIR=/Users/duykhanh/Projects.nosync/lucerna/target` (shared with the main checkout, one profile only). Run `scripts/prune-target.sh` before every build/test. Before `cargo test`, delete `target/release` if it exists, unless `pgrep -x cargo` shows another session is building.
- Only `lucerna-workbench` uses gpui. `lucerna-theme` is pure data.
- No macOS-only assumptions.
- Exact theme names: `"Lucerna Dark"`, `"Lucerna Light"`; family `"Lucerna"`.
- Thresholds: text ≥ 4.5:1 (WCAG 2.x), non-text components ≥ 3:1.
- Settings: `window.autoDetectColorScheme` (default `true`), `workbench.colorTheme` (`"Lucerna Dark"`), `workbench.preferredDarkColorTheme` (`"Lucerna Dark"`), `workbench.preferredLightColorTheme` (`"Lucerna Light"`).
- Picker placeholder text, word for word: `Select Color Theme for System Dark Mode`, `Select Color Theme for System Light Mode`, `Select Color Theme (detect system color mode disabled)`.
- Commit: stage by path, `git commit -- <paths>`, trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. `cargo deny check` must pass.

## Review Focus

1. A user already has `"workbench.colorTheme": "One Dark"` in settings: that theme no longer exists, so it must fall back to Lucerna Dark without an error. Test in Task 2.
2. Following the system is on, but `preferredLightColorTheme` points to a theme that does not exist: use Lucerna Dark (the registry default), no panic. Test in Task 2.
3. The system changes mode while the picker is previewing: it must not overwrite the preview (`reapply_configured_theme` already skips when `theme_preview` is set). Test in Task 3.
4. A user theme is missing keys: it still uses `UiColors::fallback`, behavior unchanged. The existing fallback tests must still pass after Task 1.
5. Changing `window.autoDetectColorScheme` from `true` to `false` while running: the theme switches immediately to `workbench.colorTheme`. Test in Task 2.

---

### Task 1: `lucerna.json` replaces `one.json`, key-set test and contrast test

**Files:**
- Create: `crates/lucerna-theme/assets/lucerna.json`
- Create: `crates/lucerna-theme/assets/LICENSE-primer`
- Create: `crates/lucerna-theme/tests/lucerna_theme.rs`
- Delete: `crates/lucerna-theme/assets/one.json`, `crates/lucerna-theme/assets/LICENSE-one`
- Modify: `crates/lucerna-theme/src/registry.rs` (constants, `builtin`, doc comment)
- Modify: `crates/lucerna-theme/tests/theme.rs` (the tests that use One)
- Modify: every string `"One Dark"`/`"One Light"` in `crates/lucerna-workbench/src/**` and `crates/lucerna-workbench/tests/**`, including `app.rs:610`, `excerpt_list/render.rs`, `quick_open/picker.rs:164`, `config_files.rs:396` (doc comment), tests in `theme_picker.rs`

**Interfaces:**
- Produces: `pub const DEFAULT_DARK_THEME: &str = "Lucerna Dark"` and `pub const DEFAULT_LIGHT_THEME: &str = "Lucerna Light"`, both re-exported from `lucerna_theme`. `ThemeRegistry::get_or_default` returns "Lucerna Dark" when the name does not exist.

- [ ] **Step 1: Copy Zed's key set into the test before deleting `one.json`**

In `tests/lucerna_theme.rs`, create `const STYLE_KEYS: &[&str]` with exactly the 141 keys in `style` of One Dark (except `syntax`, `players`, `accents`), and `const SYNTAX_KEYS: &[&str]` with the 46 captures in `style.syntax`. Take them from `one.json` with `python3 -I -c` and paste them in; this is a one-time copy.

- [ ] **Step 2: Write the failing tests**

```rust
// lucerna_theme.rs (reads lucerna.json via include_str! and serde_json; the test keeps its own WCAG code)
fn contrast(fg: &str, bg: &str) -> f64 // WCAG 2.x; if fg has alpha, blend it onto bg before computing

#[test] fn builtin_is_only_lucerna()
// ThemeRegistry::builtin().names() == ["Lucerna Dark", "Lucerna Light"];
// get("Lucerna Dark").appearance == Dark, get("Lucerna Light").appearance == Light;
// get_or_default("One Dark").name == "Lucerna Dark".

#[test] fn every_zed_key_is_present()
// For each theme: every STYLE_KEYS is in style, every SYNTAX_KEYS is in style.syntax,
// players has exactly 8 elements. Report the theme + missing key in the assert message.

#[test] fn text_colors_meet_aa_on_editor_background()
// ≥ 4.5 on editor.background for: text, text.muted, text.placeholder, text.accent,
// editor.foreground, editor.line_number, editor.active_line_number, error, warning, info,
// hint, success, created, modified, deleted, conflict, renamed, ignored, terminal.foreground,
// every color in style.syntax, and terminal.ansi.{,bright_}<color> except: black (Dark);
// white and bright_white (Light). dim_* is not checked.

#[test] fn list_text_meets_aa_on_selected_row()
// ≥ 4.5 on element.selected: text, text.muted, created, modified, deleted, conflict, renamed, ignored.

#[test] fn syntax_meets_aa_on_active_line()
// Every syntax color ≥ 4.5 on editor.active_line.background (blend alpha if present).

#[test] fn non_text_meets_3_to_1()
// ≥ 3.0 on editor.background: border.focused, players[0].cursor,
// version_control.added, version_control.modified, version_control.deleted.

#[test] fn spec_palette_values()
// Pin the [C]/[B] values of the spec, for example: Dark editor.background "#191919ff",
// syntax keyword "#cc7bf4ff", function "#70b8ffff", comment "#818898ff",
// players[0].cursor "#4f8cffff"; Light editor.background "#ffffffff",
// keyword "#8101c2ff", comment "#677080ff", number "#007a7aff".
```

- [ ] **Step 3: Run the tests and see them fail**

Run: `cargo test -p lucerna-theme --test lucerna_theme`
Expected: FAIL, because `lucerna.json` does not exist yet (include_str! errors) or `builtin_is_only_lucerna` fails.

- [ ] **Step 4: Write `lucerna.json`**

- Family `"Lucerna"`, `$schema` `https://zed.dev/schema/themes/v0.2.0.json`, two themes in the order Dark then Light.
- A comment at the top of the file records the sources: code colors measured from the "Claude" code theme of Claude desktop; diagnostics, git and ANSI after GitHub Primer / `primer/github-vscode-theme` (MIT, see `LICENSE-primer`).
- Every value comes from the spec's tables. Hex is written as `#rrggbbaa` like `one.json`.
- ANSI is taken from `primer/github-vscode-theme` (GitHub Dark Default / GitHub Light Default). Check against the source; if the source differs from the values below, use the source and rerun the tests. Values that fail AA are adjusted as follows:
  - Dark: black `#484f58`, red `#ff7b72`, green `#3fb950`, yellow `#d29922`, blue `#58a6ff`, magenta `#bc8cff`, cyan `#39c5cf`, white `#b1bac4`; bright: black `#7c848f` (Primer `#6e7681`, adjusted), red `#ffa198`, green `#56d364`, yellow `#e3b341`, blue `#79c0ff`, magenta `#d2a8ff`, cyan `#56d4dd`, white `#ffffff`.
  - Light: black `#24292f`, red `#cf222e`, green `#116329`, yellow `#4d2d00`, blue `#0969da`, magenta `#8250df`, cyan `#1b7c83`, white `#6e7781`; bright: black `#57606a`, red `#a40e26`, green `#1a7f37`, yellow `#633c01`, blue `#0070ea` (Primer `#218bff`, adjusted), magenta `#8a4ef7` (Primer `#a475f9`, adjusted), cyan `#2a7e92` (Primer `#3192aa`, adjusted), white `#8c959f`.
  - `dim_*` is the normal color with alpha `0xb3`.
- Keys the spec leaves to the builder: keep the UI chrome neutral (grays with no color cast), chosen so the tests pass.

- [ ] **Step 5: Change the registry, delete One, add the license**

- `registry.rs`: `const LUCERNA_JSON = include_str!("../assets/lucerna.json")`; add the two `pub const`s from Interfaces; `builtin()` has the doc comment "The bundled Lucerna family"; the `expect` message mentions `lucerna.json`; the doc of `get_or_default` says "Lucerna Dark".
- `lib.rs`: re-export the two constants, the crate doc drops the words "Zed theme loading" if they imply One (keep "Zed-format theme loading").
- `git rm` `one.json` and `LICENSE-one`.
- `LICENSE-primer`: the MIT text verbatim from `https://github.com/primer/github-vscode-theme/blob/main/LICENSE`.

- [ ] **Step 6: Fix old tests and every One string**

- `tests/theme.rs`: `builtin_has_one_dark_and_light` → check the two Lucerna names. `ui_colors_present_for_one_dark` → Lucerna Dark values: editor bg `#191919`, cursor `#4f8cff`, plus the values you set for `text`, `editor.line_number`, selection, status bar. The tests `*_from_theme_*`, `new_ui_colors_read_from_one_dark`, `error_color_*`, `scrollbar_*`, `editor_subheader_*` read "Lucerna Dark"/"Lucerna Light" and compare against the values in `lucerna.json`. `unknown_name_defaults_to_one_dark` → `_to_lucerna_dark`. `style_for_falls_back`: keep the meaning; if `string.escape` and `string` have the same color, compare another capture that has its own color.
- Workbench: replace every `"One Dark"` → `"Lucerna Dark"`, `"One Light"` → `"Lucerna Light"`. For the `config_files.rs` test (`tests/config_files.rs`) only change the name in this task; the auto-detect part is fixed in Task 2.
- Check: `rg -n "One Dark|One Light|one\.json" crates` returns nothing.

- [ ] **Step 7: Run the tests**

Run: `cargo test -p lucerna-theme && cargo test -p lucerna-workbench`
Expected: PASS. If a color fails a contrast test, adjust that color in `lucerna.json` (keep the hue, change the lightness) and update the table in the spec. Do not lower the threshold.

- [ ] **Step 8: Commit**

```bash
git add crates/lucerna-theme crates/lucerna-workbench docs/superpowers/specs/2026-11-14-lucerna-theme-design.md
git commit -m "feat(theme): Lucerna Dark and Lucerna Light replace One" -- crates/lucerna-theme crates/lucerna-workbench docs/superpowers/specs/2026-11-14-lucerna-theme-design.md
```

### Task 2: Settings and choosing the theme by the system appearance

**Files:**
- Modify: `crates/lucerna-settings/assets/default-settings.json:40` (the settings block exactly as the spec's "Settings" section)
- Modify: `crates/lucerna-workbench/src/app_state.rs` (new field)
- Modify: `crates/lucerna-workbench/src/config_files.rs` (`watch_config`, `load` branch `ConfigFile::Themes`, `apply_configured_theme`)
- Modify: `crates/lucerna-workbench/src/extensions/service.rs` (`theme_files`, `extension_theme_files`)
- Modify: `crates/lucerna-workbench/src/app.rs` (each `cx.open_window` calls `follow_system_appearance`)
- Test: `crates/lucerna-workbench/tests/config_files.rs`, unit tests in `config_files.rs`

**Interfaces:**
- Consumes: `lucerna_theme::{Appearance, DEFAULT_DARK_THEME, DEFAULT_LIGHT_THEME}` (Task 1).
- Produces:
  - `AppState.system_appearance: lucerna_theme::Appearance`, initialized to `Dark` where `AppState` is built.
  - `pub(crate) fn configured_theme_name(settings: &SettingsStore, system: Appearance) -> String` in `config_files.rs`.
  - `pub(crate) fn color_theme_setting_key(settings: &SettingsStore, system: Appearance) -> &'static str`, returns `"workbench.colorTheme"`, `"workbench.preferredDarkColorTheme"` or `"workbench.preferredLightColorTheme"`.
  - `pub(crate) fn follow_system_appearance(window: &mut Window, cx: &mut App)`.
  - `pub(crate) fn to_appearance(a: gpui::WindowAppearance) -> Appearance`: `Dark`/`VibrantDark` → `Dark`, everything else → `Light`.

- [ ] **Step 1: Write the failing tests**

Unit tests in `config_files.rs` (build `SettingsStore` from JSON like the existing tests):
```rust
#[test] fn theme_name_follows_system_when_auto_detect_is_on()
// defaults: system Dark → "Lucerna Dark"; system Light → "Lucerna Light".
#[test] fn theme_name_uses_color_theme_when_auto_detect_is_off()
// {"window.autoDetectColorScheme": false, "workbench.colorTheme": "X"} → "X" for both Dark and Light.
#[test] fn preferred_settings_override_defaults()
// {"workbench.preferredLightColorTheme": "Y"} + Light → "Y".
#[test] fn setting_key_matches_mode()
// auto on + Dark → "workbench.preferredDarkColorTheme"; auto on + Light → "...Light..."; auto off → "workbench.colorTheme".
```
Integration tests in `tests/config_files.rs`:
```rust
#[gpui::test] fn system_appearance_change_switches_theme(cx)
// Default settings, open a test window; theme == "Lucerna Light" (the test platform reports Light).
// cx.simulate_window_appearance(Dark) or TestWindow::simulate_appearance_change(Dark) → theme == "Lucerna Dark".
#[gpui::test] fn turning_auto_detect_off_uses_color_theme(cx)
// Write {"window.autoDetectColorScheme": false, "workbench.colorTheme": "Lucerna Dark"} while the system is Light → "Lucerna Dark".
#[gpui::test] fn stale_one_dark_setting_falls_back(cx)
// {"window.autoDetectColorScheme": false, "workbench.colorTheme": "One Dark"} → "Lucerna Dark", no config error.
#[gpui::test] fn unknown_preferred_theme_falls_back(cx)
// {"workbench.preferredLightColorTheme": "Nope"} + Light → "Lucerna Dark".
```

- [ ] **Step 2: Run the tests and see them fail**

Run: `cargo test -p lucerna-workbench --lib config_files && cargo test -p lucerna-workbench --test config_files`
Expected: FAIL (the functions do not exist yet).

- [ ] **Step 3: Implement**

- `default-settings.json`: the block of four settings with comments as in the spec. Keep `workbench.iconTheme` right after.
- `watch_config`: before `load(...)`, set `AppState.system_appearance = to_appearance(cx.window_appearance())`.
- `apply_configured_theme`: name = `configured_theme_name(&state.settings, state.system_appearance)`, then `get_or_default` as before.
- First load (`ConfigFile::Themes`, `initial`): change `only: Option<&str>` to `only: Option<&[String]>` in `theme_files` and `extension_theme_files`. On first load pass the names needed: with auto-detect on, both `preferredDark` and `preferredLight`; off, `colorTheme`. `icon_theme_files` stays as it is.
- `follow_system_appearance`: `window.observe_window_appearance(|window, cx| { set AppState.system_appearance = to_appearance(window.appearance()); reapply_configured_theme(cx); }).detach()`. Call it in the closure of every `cx.open_window` in `app.rs` (the workbench window and Welcome). Also call it in the test window-opening helper if the integration tests do not go through `app.rs`.
- Fix old tests in `tests/config_files.rs` that wait for "Lucerna Dark" from `workbench.colorTheme`: add `"window.autoDetectColorScheme": false` to the test settings, or change the expectation to "Lucerna Light" when the test only needs the default theme. Choose whichever keeps the test's intent.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p lucerna-workbench && cargo test -p lucerna-settings`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(theme): follow the system color mode (window.autoDetectColorScheme)" -- crates/lucerna-settings crates/lucerna-workbench
```

### Task 3: Theme picker by system mode

**Files:**
- Modify: `crates/lucerna-workbench/src/theme_picker.rs` (`show_theme_picker`, `on_theme_picker_event`, `set_theme_setting`, sorting `names`)
- Test: `crates/lucerna-workbench/tests/theme_picker.rs` if it exists, otherwise `tests/config_files.rs` where the current picker tests live

**Interfaces:**
- Consumes: `color_theme_setting_key`, `AppState.system_appearance` (Task 2).

- [ ] **Step 1: Write the failing tests**

```rust
#[gpui::test] fn picker_placeholder_names_system_mode(cx)
// Default + Light: the input's placeholder == "Select Color Theme for System Light Mode".
// After simulating Dark: reopen → "...System Dark Mode".
// autoDetect false → "Select Color Theme (detect system color mode disabled)".
#[gpui::test] fn picker_confirm_writes_preferred_setting(cx)
// Light, choose "Lucerna Dark", Enter → settings.json has "workbench.preferredLightColorTheme": "Lucerna Dark",
// no "workbench.colorTheme"; the active theme == "Lucerna Dark".
#[gpui::test] fn picker_confirm_writes_color_theme_when_detect_off(cx)
// autoDetect false → writes "workbench.colorTheme".
#[gpui::test] fn picker_lists_system_mode_themes_first(cx)
// Light: row_texts()[0] == "Lucerna Light".
#[gpui::test] fn appearance_change_during_preview_keeps_preview(cx)
// Open the picker, preview "Lucerna Dark" while Light, simulate Dark then Light → the theme is still the preview;
// Esc → theme == the current configured_theme_name.
```

- [ ] **Step 2: Run the tests and see them fail**

Run: `cargo test -p lucerna-workbench picker`
Expected: FAIL.

- [ ] **Step 3: Implement**

- `show_theme_picker`: the placeholder is one of the three strings in Global Constraints. `show_picker` takes a `&'static str` placeholder, so the three strings are constants.
- `names`: themes whose `appearance` matches `system_appearance` come first (when auto-detect is off, use the appearance of the active theme). Keep the name order within each group. `themes_with_appearance()` already exists.
- `Confirm` and `set_theme_setting(name, icons: false)`: key = `color_theme_setting_key(...)` instead of `COLOR_THEME_KEY`. The icon theme is unchanged.
- Change the unit tests of `set_string_setting` in `theme_picker.rs` only in the theme names (already done in Task 1). The string-writing function is unchanged.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p lucerna-workbench`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(theme): picker writes the preferred theme for the system mode" -- crates/lucerna-workbench
```

### Task 4: Licenses, docs, site

**Files:**
- Modify: `scripts/gen-licenses.py:31` (`ASSETS`)
- Modify: `THIRD_PARTY_NOTICES.md` ("Themes" section)
- Modify: `README.md:67`, `README.md:184`
- Modify: `site/index.html:181`, `site/styles.css:21,36` (only text and comments that mention One; do not change the site's colors)

- [ ] **Step 1: Edit**

- `ASSETS`: remove the One line, add `("GitHub Primer colors (Lucerna themes)", "crates/lucerna-theme/assets/LICENSE-primer")`.
- `THIRD_PARTY_NOTICES.md`, Themes section: `lucerna-theme` bundles Lucerna Dark/Light (`assets/lucerna.json`); diagnostic, git and ANSI colors are based on GitHub Primer / primer/github-vscode-theme, MIT, Copyright (c) GitHub Inc.; the license is at `crates/lucerna-theme/assets/LICENSE-primer`.
- README: list "the Lucerna Dark and Lucerna Light themes"; settings examples use `"Lucerna Dark"`, add a line `"window.autoDetectColorScheme": true` if the example block lists theme settings.
- Site: "Lucerna Dark and Lucerna Light are built in and follow the system color mode." The CSS comment drops "One Dark".

- [ ] **Step 2: Check**

Run: `python3 -I scripts/gen-licenses.py "$TMPDIR/licenses.md" && grep -c "GitHub Primer" "$TMPDIR/licenses.md"`
Expected: runs without error, the count is ≥ 1.
Run: `cargo deny check`
Expected: `advisories ok, bans ok, licenses ok, sources ok`.
Run: `rg -n "One Dark|One Light|LICENSE-one|one\.json" --glob '!docs/**' .`
Expected: no results.

- [ ] **Step 3: Commit**

```bash
git commit -m "docs: credit GitHub Primer for the Lucerna theme, drop One" -- scripts/gen-licenses.py THIRD_PARTY_NOTICES.md README.md site/index.html site/styles.css
```

### Task 5: Before/after screenshots and handover notes

**Files:**
- Create: `docs/research/lucerna-theme-screenshots/{before,after}-{dark,light}.png`
- Create: handover notes in the shared notes folder `/Users/duykhanh/Library/Application Support/Claude/side-session-notes/local_fcd9b997-826b-47a7-83f7-6b39fdb9b476/lucerna-theme-handover.md`

- [ ] **Step 1: Take the "before" shots**

Build the app from `main` (One) or use `/Applications/Lucerna.app` if it is the build from before the new theme. Open the Lucerna repo, open a Rust file that has a git diff (temporarily edit one line and revert it after the shot), Explorer open. Capture the window in dark and light mode (`workbench.colorTheme` "One Dark" / "One Light").

- [ ] **Step 2: Take the "after" shots**

`scripts/bundle-app.sh` from this branch (it handles profiles itself). Same file, same window size. Switch the system between dark/light to also check the automatic theme switch. Capture two images.

- [ ] **Step 3: Handover notes**

Copy down the section "Notes when merging parallel branches" of the spec, plus this branch's commit names.

- [ ] **Step 4: Commit**

```bash
git commit -m "docs: before/after screenshots of the Lucerna theme" -- docs/research/lucerna-theme-screenshots
```
