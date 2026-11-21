# First-run onboarding screen ("Get Started")

Date 2026-11-15. Status: implemented (branch claude/interesting-kapitsa-5d4c92, commits 50d9583–e48d6b1); manual check §9 not done yet.

Where the implementation differs from this spec:
- `ONBOARDING_COMMANDS` has no Open… commands.
- `number_stepper` returns `(value, minus, plus)`.
- File ▸ Open Recent lists no entries in the onboarding window.
- Welcome's Get Started button uses the settings icon.
- The router tests live in `tests/welcome.rs`.
- A broken `settings.json` shows a one-line warning when the page is the window root.
- **Font family picker and the sample code box dropped** (user decision 2026-11-15, after a trial run): users set `editor.fontFamily` in `settings.json` themselves. The page keeps only the font size (− / +) and Vim in the EDITOR section. The Font Family entries, `FontPicker`, `dropdown_button` and the font sample line were removed from this spec.

This spec amends item W12 of `2026-11-12-welcome-design.md`: Welcome still has no AI, sign-in, New File or Clone Repository, but **Lucerna has onboarding**, as a page of its own rather than a part of Welcome.

## 1. Intent

Zed has a dedicated onboarding page shown on first run, for choosing a theme and Vim. Lucerna is about to ship 0.1, and we want new users to have one place to adjust a few basics as soon as they open the app: theme, font, Vim. Today a new user has to find `settings.json` or the Command Palette to do any of it.

**Goals:**
- On the first launch of Lucerna (Dock, Finder, `luc` with no arguments) the "Get Started" page appears, where the user can pick the Light / Dark / System theme, the editor font size and Vim mode, and has a button to open `settings.json`.
- Each change is written to the user's `settings.json` immediately and takes effect immediately.
- The page appears automatically only once. It can be reopened with a command, from the Help menu, and from Welcome.
- It fits with `window.restoreWindows`: a user who already has a workspace to reopen is not blocked by onboarding.

**Non-goals:** choosing a base keymap (Lucerna only follows VS Code keys), AI, sign-in, telemetry, importing settings from another editor, UI font, choosing a theme family (Lucerna has only one default theme pair so far).

**Fails even though it runs, if:**
- the page shows again on every launch, or cuts in front of a workspace the user wants to reopen;
- changing a setting on the page loses comments or formatting in `settings.json`;
- the page shows a value different from the real setting (for example the user edited the file by hand).

## 2. References

Details in `docs/research/2026-11-15-onboarding.md`. Summary:
- **Zed:** onboarding is a tab in an empty workspace, a scrolling page (since version 0.208), with a column at most 780 px wide. The top has the logo, "Welcome to Zed" and a "Finish Setup" button (`cmd-enter`); clicking it replaces the tab with Welcome. The sections: Theme (a Light/Dark/System toggle and three preview tiles drawn with the theme colors), Base Keymap, AI, Import Settings, Vim Mode, Trust, Telemetry. Each control writes to `settings.json` immediately, with no Cancel. It shows automatically when there is nothing to restore and the `first_open` key is not set; the key is written the moment the page opens. It does not show when launched with a path. Reopen with `zed: open onboarding`; Welcome has a "Return to Onboarding" button.
- **VS Code:** the "Get started with VS Code" walkthrough on the Welcome page has a theme-picking step with image cards. The newer onboarding overlay is a modal over Welcome, shown only when application storage is new. Picking a theme while `window.autoDetectColorScheme` is on writes to the preferred theme key for the system mode.
- **Lucerna's choice:** the "first time" condition, writing settings immediately, the Finish button and the page layout follow Zed; setting names and the command to open settings follow VS Code. Deviations from Zed:
  - only one theme preview tile instead of three, because Lucerna has only one theme pair;
  - an editor font size control (older Zed versions had it, newer ones dropped it), at the user's request;
  - a Help ▸ Get Started entry (Zed has none), so VS Code users can find it.

## 3. Decisions

| # | Decision | Reason |
|---|---|---|
| O1 | The page is an `OnboardingPage` view usable in **two places**: (a) as the **root view** of a window with no folder open, like Welcome; (b) as a `TabItem::Onboarding` **tab** in the workbench. The content rendering is shared. | The user chose this (option A). Zed keeps the page in a tab; Lucerna has no empty workbench, so in a window with no folder the page has to be the root, like Welcome (W1). |
| O2 | Shown automatically at startup with **no path** (no arguments, no open-from-Finder request during the wait), **no workspace to reopen** (`window.restoreWindows` is `"none"` or no recent entry still exists), and **no `onboarded` flag** yet. | Follows Zed. Returning users with a workspace to reopen are not blocked. |
| O3 | The `onboarded: true` flag is written **the moment the page shows automatically**, without waiting for Finish. | Follows Zed: closing the window or quitting the app means it does not show again next time. |
| O4 | On the first run, onboarding goes before `workbench.startupEditor`: when O2 holds, onboarding shows even if `startupEditor` is `"none"`. | The first run usually has no settings yet. Someone who set `"none"` before the first run only sees the page once. |
| O5 | Finish (`⌘Enter`): at root, switch the root to Welcome (`window.replace_root`, like W1); in a tab, close the tab. | Zed replaces the page with Welcome. Lucerna's workbench has no Welcome tab, so it closes the tab. |
| O6 | Each control writes to the user's `settings.json` immediately, keeping comments (an existing function in `theme_picker.rs`), then reloads the configuration. The page renders from the current setting after merging the layers. No Undo/Cancel. | Follows Zed. Editing the file by hand changes the page. |
| O7 | Theme: a group of three buttons **Light / Dark / System**. System writes `window.autoDetectColorScheme: true`. Light/Dark write `window.autoDetectColorScheme: false` and `workbench.colorTheme` set to the current value of `workbench.preferredLightColorTheme` / `workbench.preferredDarkColorTheme` (defaults "Lucerna Light" / "Lucerna Dark"). | Keys follow VS Code, as the user asked. Keeps the preferred theme the user already set. |
| O8 | Command `lucerna.openOnboarding`, titled "Help: Get Started", no default key. It is in the Help menu ("Get Started", first entry) and is a "Get Started" button on Welcome. | VS Code has no matching command (its new onboarding is in the Developer group), so it uses the `lucerna.` prefix like `lucerna.openLogsFolder`. Zed has a "Return to Onboarding" button on Welcome. |
| O9 | Font size with − / + buttons from 6 to 100, step 1. No font family picker on the page: users set `editor.fontFamily` in `settings.json` themselves. | User decision 2026-11-15 after a trial run: the font list is long and few people change the font on a fresh install. |
| O10 | Vim mode is a **switch** that writes `vim.enabled`. | Zed uses a switch. |
| O11 | No base keymap, AI, sign-in, telemetry, import, UI font. | Scope the user settled. |

**Success when:**
- Delete `<data_dir>` (or use a fresh `HOME`) and open `Lucerna.app` from the Dock: the Get Started page fills the whole window. Click Dark: the app switches to Lucerna Dark immediately, `settings.json` has `"window.autoDetectColorScheme": false` and `"workbench.colorTheme": "Lucerna Dark"`, and the old comments are intact.
- Click + on Font size: editors in other windows get bigger immediately, and the file has `"editor.fontSize": 15`.
- Turn on Vim mode: the file has `"vim.enabled": true`; opening a folder puts the editor in Normal mode.
- ⌘Enter: the window switches to Welcome, with no second window.
- Quit and reopen from the Dock with no folder open: Welcome shows, not onboarding.
- Open a folder, quit, reopen from the Dock: that folder reopens (restoreWindows), no onboarding.
- `luc ~/code` on the first run: opens the folder, no onboarding, the flag is not written.
- In the workbench, run "Help: Get Started": the Get Started tab opens; running it again jumps back to that tab. ⌘Enter closes the tab.
- Hand-edit `"editor.fontSize": 20` in `settings.json` while the page is open: the font size box shows 20.

## 4. First run and the startup flow

`app.rs` `open_without_folder` currently does: read the user config → if `restores_last_workspace` and there is a `last_workspace`, open it → otherwise `startup_choice` (Welcome or the folder prompt). Add one step:

```
open_without_folder:
  watch_config(user_only)
  if restores_last_workspace && has last_workspace → open it; return
  if !onboarded(data_dir)                          → mark_onboarded(data_dir); Router::open_onboarding; return
  startup_choice → FolderPrompt | Welcome
```

- The **flag** lives in `<data_dir>/state.json`: `{ "version": 1, "onboarded": true }`. A pure module `app_state_file.rs` (no gpui) has `load(path) -> AppFlags` and `mark_onboarded(data_dir)`. A missing file or missing key means `onboarded = false`. A corrupt or different-version file is renamed to `.bak` like `recent.json` and treated as `onboarded = false`. Written with the existing `write_atomic`. Reads and writes both happen on a background thread like `recent.rs`, except the startup read (a small file, read synchronously before any window exists, like `recent::load` in `open_without_folder`).
- No `HOME` (`default_data_dir()` is `None`): do not show onboarding, go straight to `startup_choice`. With nowhere to write the flag, showing it every time would be annoying.
- Writing the flag fails: `tracing::warn` and still show the page. It may show again next time; acceptable.
- Other paths (`luc <path>`, Open With, `open_urls`) are unchanged: they do not consider onboarding and do not write the flag.
- Choosing the root at startup is split into a pure function for testing: `startup_screen(restore: Option<&RecentEntry>, onboarded: bool, has_data_dir: bool, choice: Startup) -> StartupScreen::{Restore(path), Onboarding, Welcome, FolderPrompt}`. The functions `restores_last_workspace`, `last_workspace` and `startup_choice` in `startup.rs` stay as they are; the new function sits next to them.

## 5. The Get Started page

### 5.1 Layout (after Zed's `basics_page.rs`, minus the items dropped in O11)

```
┌──────────────────────────────────────────────────────────────┐
│ ● ● ●                       Lucerna                          │  title bar 34 px (only as root)
├──────────────────────────────────────────────────────────────┤
│      [logo] Welcome to Lucerna                 [ Finish  ⌘↵ ]│  header
│             Set up the basics. Change them any time.         │  muted
│      ─────────────────────────────────────────────────────── │  border_variant rule
│      THEME ───────────────────────────── [Light|Dark|System] │  section_header + segmented
│      ┌──────────────────────────────┐                        │
│      │ ▌▬▬  ▬▬▬▬▬                    │  theme_preview          │
│      │ ▌▬   ▬▬ ▬▬▬▬                  │  (System: split in two) │
│      └──────────────────────────────┘                        │
│      Lucerna Dark                                            │  theme name, muted
│      EDITOR ─────────────────────────────────────────────────│
│      Font Size                            [ −   14   + ]     │  setting_row + number_stepper
│      Vim Mode                                         ( ●)   │  setting_row + switch
│      Enable Vim keybindings (vim.enabled)                    │  muted description under the label
│      ─────────────────────────────────────────────────────── │
│      [⚙] Open Settings (JSON)                       ⌘,       │  list_button
└──────────────────────────────────────────────────────────────┘
```

- Background `editor_background`, no status bar when it is the root. The column is at most **640 px** wide (between Welcome's 512 px and Zed's 780 px, since Lucerna has fewer sections), centered horizontally, `8×GRID` padding, blocks spaced `6×GRID` apart, scrolls vertically when the window is short (Lucerna's shared scrollbar).
- The title and logo reuse Welcome's rendering (`mark-mono.svg`, 45 px, 20 px title). The subtitle uses `UI_FONT_SIZE_SMALL` in `text_muted`.
- The Finish button uses `primary_text_button` with the key shown on the right.
- `section_header` and `list_button` are reused from `ui.rs`.
- The "Open Settings (JSON)" button shows the key of `workbench.action.openSettingsJson` per the user's keymap.

### 5.2 New components in `ui.rs`

Names and sizes follow the existing components (`GRID`, theme colors):
- **`segmented_control(id, options, selected, cx)`**: adjoining buttons, 1 px `border` outline, `GRID` radius, `6×GRID` high. The selected button has the `element_selected` background; the others get `ghost_element_hover` on hover. Returns each button with an `on_click`.
- **`switch(id, on, cx)`**: `7×GRID` wide, `4×GRID` high, fully rounded. On: `element_selected` background (or the theme's accent color if there is one), knob on the right. Off: `element_background` background, knob on the left.
- **`number_stepper(id, value, cx)`**: − / + buttons (`icon_button`) on both sides of the number; a button dims when it hits its limit.
- **`setting_row(label, description, control)`**: label at `UI_FONT_SIZE`, description at `UI_FONT_SIZE_SMALL` in `text_muted` under the label, control aligned right.
- **`theme_preview(theme, cx)`**: draws a miniature workspace using the passed-in theme's colors (not the active theme): a sidebar of about 22% with 6 lines, an editor area with about 8 "pseudo-code" bars filled with syntax colors (keyword, function, string, comment, type, variable). The bar widths and indents come from a fixed table, not randomness. `30×GRID` high, 8 px radius, 1 px outline. For System it draws two halves: the light theme on the left, the dark theme on the right.

Check `ui.rs` before creating one: if an equivalent component already exists, reuse it.

### 5.3 Theme

- **Selected state:** if `window.autoDetectColorScheme` is true it is System; if false it is Light or Dark according to the appearance of the `workbench.colorTheme` theme.
- **Click** (O7):
  - System: write `window.autoDetectColorScheme: true`.
  - Light: write `window.autoDetectColorScheme: false` and `workbench.colorTheme` = the value of `workbench.preferredLightColorTheme` read **directly** from the store (not through `configured_theme_name`, since that function returns `workbench.colorTheme` when autodetect is off), or "Lucerna Light" if unset.
  - Dark: likewise with `preferredDarkColorTheme` / "Lucerna Dark".
  - Clicking the already-selected button writes nothing.
- The **preview tile** draws the theme that would be used with the current choice: for Light/Dark it is `workbench.colorTheme`; for System it draws two halves, preferred Light and preferred Dark. Under the tile is the theme name (System: "Lucerna Light / Lucerna Dark").
- A theme name that is not found falls back with `get_or_default`, like `apply_configured_theme`.
- **Write two keys at once:** write `set_settings(text, &[(key, serde_json::Value)])` in `theme_picker.rs` (split out to `settings_edit.rs` if needed), built on `set_string_setting`, extended to bool and number values, and writing the file once so the watcher sees a single change. `set_string_setting` calls the new function.

### 5.4 Editor

- **Font Size:** shows the current `editor.fontSize` (rounded). − / + write the new value, limited to 6–100 (VS Code accepts 6–100).
- **Vim Mode:** a switch following `vim.enabled`; clicking writes the inverted value. Open editors update through the existing mechanism when `vim.enabled` changes.
- These changes only write the user layer. If the workspace layer (`.lucerna/settings.json`) or a language override sets that key, the page shows the merged value, and the value just written may not take effect. This spec does **not** report that on the page, because `SettingsStore` does not say which layer sets a key and the spec only changes `lucerna-workbench`. Recorded as a known limit (§10).

### 5.5 Open Settings (JSON)

- In a tab: run `workbench.action.openSettingsJson` as the Command Palette does.
- At root (no workbench yet): open the user's `settings.json` in a **new window** (`luc <user settings path>`, like the "new window" of W7), so the onboarding page stays. If the file does not exist yet, create it from `USER_SETTINGS_TEMPLATE` first.

### 5.6 Keyboard

- Focus order: Finish → Light → Dark → System → − → + → Vim Mode → Open Settings (JSON). Tab/⇧Tab and ↑/↓ move focus; Enter or Space activates the focused control; ←/→ in the theme group change the selection.
- Initial focus: the selected theme button.
- `⌘Enter` (command `lucerna.onboarding.finish`, `when: onboardingFocus`) runs Finish in both modes. `onboardingFocus` is true when focus is inside the page. This binding does not clash with `editor.action.insertLineAfter` (`editorTextFocus`) since the page has no editor.
- At root: global keys go through the user keymap like Welcome (`WELCOME_COMMANDS`), adding `lucerna.openOnboarding`, `workbench.action.openSettingsJson`, `lucerna.onboarding.finish`.
- In a tab: workbench keys work as on other tabs (⌘W closes the tab).

## 6. Reopening

- **Command** `lucerna.openOnboarding`:
  - In the workbench: open or activate the `TabItem::Onboarding` tab (only one per window, opened in the active group).
  - In Welcome: switch the root to onboarding.
  - In onboarding root: do nothing.
- **Help menu:** add "Get Started" (`lucerna.openOnboarding`) as the first entry of the Help menu, followed by a separator and the old entries. The Welcome window enables this entry (add it to the list of entries Welcome handles in `build_menus`).
- **Welcome:** add a "Get Started" button to GET STARTED, after "Open Recent…", with its own icon (for example `sparkles` or `settings` from Lucide if already in `icons`), key per the keymap.
- **Tab:** title "Get Started", no unsaved dot, no path bar. The tab is **not** saved in the workspace state; reopening the folder does not bring this tab back.

## 7. Code architecture

| File | Change |
|---|---|
| `onboarding.rs` (new) | `OnboardingPage` (render §5, focus, keys), `OnboardingEvent::{Finish}`, `register_commands`. Knows whether it is root or tab through a `Host::{Window, Tab}` flag, to draw the title bar and choose the Open Settings behavior. |
| `app_state_file.rs` (new) | Read/write `<data_dir>/state.json` (§4). No gpui. |
| `startup.rs` | Add `StartupScreen` and `startup_screen` (§4). |
| `app.rs` | `Shown::Onboarding(WindowHandle<OnboardingPage>)`; `Router::open_onboarding`, switching the root between Welcome ↔ Onboarding; `open_without_folder` per §4. |
| `welcome.rs` | "Get Started" button; `WelcomeEvent::OpenOnboarding`. |
| `editor_groups/model.rs`, `editor_groups/view.rs` | `TabItem::Onboarding` (singleton like the dynamic preview), render `OnboardingPage` in the pane. |
| `workbench.rs` | Register the commands; do not save the Onboarding tab in the workspace state. |
| `app_menu.rs` | Help ▸ Get Started entry; enabled in Welcome and Onboarding root. |
| `ui.rs` | The components in §5.2. |
| `theme_picker.rs` (or `settings_edit.rs`) | `set_settings` writing several keys, arbitrary JSON values (§5.3). |
| `default-keybindings.json` | `cmd+enter` → `lucerna.onboarding.finish` when `onboardingFocus`. |

Only `lucerna-workbench` changes; no new dependency.

## 8. Error handling

- `settings.json` cannot be read or parsed: do not write, report a configuration error like the theme picker (`record_error`), the control keeps its old value.
- No `settings.json` yet: create it from `USER_SETTINGS_TEMPLATE`, then write (like `write_string_setting`).
- Opening the settings window (§5.5) fails: `tracing::error`, show a `warning_row` at the top of the column.

## 9. Testing

- **Unit:**
  - `app_state_file`: no file → not onboarded; write then read; corrupt file → `.bak`; atomic write.
  - `startup_screen`: has restore → Restore; no restore, not onboarded → Onboarding; onboarded → per `startupEditor`; no data dir → per `startupEditor`; `startupEditor: none` on first run → Onboarding.
  - `set_settings`: writes bool, number, string; several keys at once; keeps comments; replaces an existing key; empty file.
  - Theme: the mapping of Light/Dark/System to the keys written (per O7), with and without a user-set preferred theme; the selected state is read from settings.
- **GPUI test:**
  - The page renders the right values from settings; editing the user layer makes the page re-render.
  - Clicking Dark/System/+/Vim leaves the right keys in the user file.
  - ⌘Enter at root emits Finish (the Router switches to Welcome); in a tab it closes the tab.
  - `lucerna.openOnboarding` in the workbench opens one tab, and a second call opens no more.
  - The Help menu has Get Started, enabled in the Welcome window.
- **Manual (design-principles §1.1):** run the app with a fresh `HOME`, put it next to Zed (`zed: open onboarding`) and compare spacing, font size, buttons, hover, switch. Try the sequence first run → Finish → quit → reopen (Welcome) → open a folder → quit → reopen (the folder).

## 10. Known limits

- No warning when the workspace or a language override overrides the setting the page just wrote (§5.4).
- A user upgrading from a pre-0.1 build has no `state.json` yet, so if at startup there is nothing to reopen (`restoreWindows: "none"` or every recent entry is gone) they see onboarding **once**, even though they already have a `settings.json`. Zed does the same. Accepted, because Lucerna has not been released yet and it only shows once.

## 11. Left to the implementer

- The specific icon for the Get Started button.
- The table of "pseudo-code" bars in the preview tile, as long as it looks like code and uses all the syntax colors.
- The subtitle under the heading may have its wording adjusted.
