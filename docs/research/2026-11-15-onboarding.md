# Research: first-run onboarding screen

Date 2026-11-15. Read Zed `main` and VS Code `main` (fetched on 2026-11-15). Zed is GPL: only the design is recorded, no code is copied. Line numbers are approximate because both repos change constantly.

## 1. Zed

### 1.1 When it appears on its own

`crates/zed/src/main.rs`, `restore_or_create_workspace`:

1. If there is a restorable workspace (per `restore_on_startup`: default `last_session`, or `last_workspace`), it is restored and onboarding is **not** shown.
2. If there is nothing to restore and the KV key `first_open` is not set, it calls `onboarding::show_onboarding_view`.
3. Otherwise an empty workspace is opened.

- `show_onboarding_view` writes `first_open = "false"` **as soon as it opens the screen**, not when the user finishes. Closing the tab, skipping it or quitting the app does not make the screen reappear.
- Quitting while the tab is open: the tab is serialized (`OnboardingPage`), so it reopens with the same workspace.
- Opening with a path (`zed <path>`, Open With) goes through `open_startup_request`, skips the step above, and so does not show onboarding. The key is still unwritten, so the screen may appear on a later bare launch, if there is nothing to restore then.

### 1.2 Form and layout

- It is **a tab** (a workspace `Item`, titled "Onboarding") in a new empty workspace, not a modal or a separate window.
- Since 0.208 (October 2025, PR #39724) it is **one single scrolling page**. Before that there was a 220 px left nav with three pages Basics / Editing / AI Setup (`cmd-1/2/3`).
- A column at most 780 px wide, centered, padding `p_12`, spacing `gap_6`, `editor_background` background, with a vertical scrollbar.
- Top of the page: logo, "Welcome to Zed", a muted italic subtitle. On the right of the header is a filled "Finish Setup" button (200 px wide, with the key `cmd-enter`). A divider follows.
- Keys (context `Onboarding`): `cmd-enter` Finish; `cmd-=`/`cmd--`/`cmd-0` change the UI font size without saving; `menu::SelectNext`/`SelectPrevious` move focus between controls (each control has a `tab_index`).
- Finish: replaces the Onboarding tab with a Welcome tab in the same position.

### 1.3 Content (in order)

1. **Theme.** A "Theme" label and a Light / Dark / System toggle group on the right. Below it, three preview tiles by theme family (One, Ayu, Gruvbox).
   - A tile is not a screenshot but a miniature workspace skeleton drawn in that theme's colors: a sidebar of ~20–25% with a few lines, a pane with "pseudo-code" bars colored by syntax (keyword, function, string, comment…). Widths, indentation and chosen colors are deterministic.
   - System mode draws each tile split light/dark.
   - The selected tile has a `border_selected` border; the others are 80% faded, with a border on hover. The theme family name sits below the tile. 8 px radius, 2 px border.
2. **Base Keymap:** Zed, VS Code, JetBrains, Sublime/Atom, Emacs, Cursor, TextMate.
3. **Agent Setup** (AI).
4. **Import Settings** from VS Code / Cursor.
5. **Vim Mode** (switch).
6. **Trust All Projects By Default** (switch).
7. Telemetry: metrics, crash reports.

The old version had an Editing page: UI font and buffer font (font family picker, font size stepper), ligatures, format on save, inlay hints, git blame, minimap. The current version dropped the font part.

### 1.4 How settings are written

- Each control writes **immediately** to the user's `settings.json` (`update_settings_file`). The view watches `SettingsStore`, so it always draws from the current setting, even when the user edits by hand elsewhere.
- There is no Undo/Cancel: Finish only changes the page.
- Keys: `theme: {mode, light, dark}`, `base_keymap`, `vim_mode`, `telemetry.*`, `session.trust_all_worktrees`; the old version had `buffer_font_family`, `buffer_font_size`, `ui_font_*`.
- The old minimap toggle applied the value in memory before writing the file, because waiting for the file write made the toggle lag.

### 1.5 Differences from the Welcome page

- Welcome (`WelcomePage`, `zed::ShowWelcome`) is the "Get Started" page: New File, Open Project, Clone Repository, Command Palette, Configure…, and Recent Projects (`cmd-1`…`cmd-5`) in launchpad mode. Welcome has a "Return to Onboarding" button.
- Onboarding is the first-run configuration page. It is reopened with the command `zed: open onboarding` (`OpenOnboarding`). Zed's Help menu only has "Show Welcome", no onboarding entry.

## 2. VS Code

### 2.1 Walkthrough on the Welcome page

- `welcomeGettingStarted/common/gettingStartedContent.ts`. The walkthrough "Get started with VS Code" (id `Setup`) now consists of the steps Copilot, `pickColorTheme` and a video. The old steps (sync, extensions, command palette…) moved to "Learn the Fundamentals" (id `Beginner`).
- The `pickColorTheme` step: four image cards (Dark Modern, Light Modern, Dark/Light High Contrast) and a "See More Themes…" link that opens the `workbench.action.selectTheme` quick pick. Clicking a card calls `setColorTheme(id, USER)`; a card is marked when `workbench.colorTheme` equals its id.
- Each step's progress is stored in the Memento `gettingStartedService` (`{done: true}` by step id). A step completes on events `onCommand:`, `onSettingChanged:`, `onContext:`…

### 2.2 The new onboarding overlay

- `contrib/welcomeOnboarding/`, setting `workbench.welcomePage.experimentalOnboarding` (default true): a **modal** "Welcome to Visual Studio Code" over the Welcome page. Steps: Sign In, "Make It Yours" (a Color Theme card drawn in SVG, choosing the keymap of an editor installed on the machine).
- It only appears when application storage is new (`isNew(APPLICATION)`), not on web, AI is not hidden, and the key `welcomeOnboarding.state` is not set (written on close). Esc skips; Tab is trapped inside the overlay; shortcuts do not reach the keybinding service.
- Reopen with `workbench.action.welcomeOnboarding2026` (Developer group).

### 2.3 When Welcome opens on its own, command, menu

- `browser/startupPage.ts`: after restore finishes, not a reload, no `--skip-welcome`, no editor open; per `workbench.startupEditor` (default `welcomePage`).
- The classic Welcome page has no notion of "first time"; it only follows `startupEditor`.
- Help ▸ Welcome (`workbench.action.openWalkthrough`), Help ▸ Open Walkthrough… (`welcome.showAllWalkthroughs`).

### 2.4 Theme and font settings

- `window.autoDetectColorScheme` (VS Code default false; Lucerna default true). When on, `workbench.colorTheme` is ignored and choosing a theme writes to `workbench.preferredDarkColorTheme` or `workbench.preferredLightColorTheme` according to the current system mode. Lucerna already does exactly this (`color_theme_setting_key`).
- Font: `editor.fontFamily`, `editor.fontSize`. No VS Code Welcome flow touches these two keys.

## 3. Similarities and differences

| | Zed | VS Code | Lucerna chooses |
|---|---|---|---|
| Form | Tab in an empty workspace | Modal over Welcome (new); walkthrough inside the Welcome tab (old) | UI after Zed (a page, not a modal). See the spec. |
| First-run condition | Nothing to restore and no `first_open` key; writes the key on open | New application storage; writes the key on close | Follow Zed |
| Open with a path | Not shown | Overlay still shows over the window | Follow Zed: not shown |
| Writing settings | Written immediately on each change, no Cancel | Written immediately (theme card) | Write immediately |
| Theme | Light/Dark/System + preview tiles drawn in theme colors | Image/SVG cards | Light/Dark/System after Zed; keys after VS Code |
| Base keymap | Yes | Yes (installs a keymap extension) | No: Lucerna only follows VS Code keys |
| Vim | `vim_mode` switch | No | `vim.enabled` switch |
| Font | Old version had it, new version dropped it | No | Only `editor.fontSize`; the user sets the font family in `settings.json` |
| AI, sign-in, telemetry | Yes | Yes | No |
| Reopen | Command `zed: open onboarding`; Welcome has a "Return to Onboarding" button | Developer group command | Command + Help menu + a button on Welcome |

## 4. Sources

- Zed: `crates/onboarding/src/{onboarding.rs, basics_page.rs, theme_preview.rs}`, `crates/zed/src/main.rs`, `crates/zed/src/zed/app_menus.rs`, `crates/workspace/src/{welcome.rs, pane.rs}`, `assets/keymaps/default-macos.json`; the pre-refactor version at commit 4dbd1864 (`editing_page.rs`). Release notes 0.208.4: https://zed.dev/releases/stable/0.208.4
- VS Code: `src/vs/workbench/contrib/welcomeGettingStarted/{browser/startupPage.ts, browser/gettingStartedService.ts, browser/gettingStarted.ts, common/gettingStartedContent.ts, common/media/theme_picker.ts}`, `src/vs/workbench/contrib/welcomeOnboarding/*`, `src/vs/workbench/services/themes/common/themeConfiguration.ts`.

Could not be verified: Esc/closing the tab on Zed's onboarding other than the ordinary way of closing a tab; the label "Dark Modern" versus VS Code's id `Dark 2026`.
