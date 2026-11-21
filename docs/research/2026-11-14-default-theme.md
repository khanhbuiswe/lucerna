# Research: the default themes (dark + light) for Lucerna

Date: 2026-11-14. Purpose: prepare the spec for Lucerna's own pair of default themes, replacing the One Dark/One Light borrowed from Zed (`crates/lucerna-theme/assets/one.json`, identical to the upstream `assets/themes/one/one.json` at the time of checking).

Sources read: zed.dev/docs/themes, zed.dev/docs/reference/all-settings, Zed's source (`assets/themes/*`, `crates/theme/src/{schema,fallback_themes,default_colors}.rs`, `crates/ui/src/utils/apca_contrast.rs`). VS Code: code.visualstudio.com/docs/configure/themes, .../accessibility, the source of `extensions/theme-defaults/` and `src/vs/workbench/services/themes/common/{themeConfiguration,workbenchThemeService}.ts`, `terminalConfiguration.ts`. Zed is GPL, so only the design is described here, no code is copied.

## 1. Zed

### 1.1 How the built-in themes are organized
- `assets/themes/` has three families: `one/` (One Dark, One Light), `ayu/` (Ayu Dark, Ayu Light, Ayu Mirage), `gruvbox/` (6 variants), plus `LICENSES`.
- Each file is a **theme family**: `$schema` (`https://zed.dev/schema/themes/v0.2.0.json`), `name`, `author`, `themes: [...]`. Each child theme has `name`, `appearance` (`dark`/`light`), `style`.
- The binary also contains a fallback theme written in code (`fallback_themes.rs`, `zed_default_dark()`), along with a function that fills in status colors and default UI colors when the JSON file lacks keys. Lucerna should have a similar mechanism: a theme missing keys is still usable.

### 1.2 The `style` key set (One Dark has 141 keys)
| Group | Typical keys | Notes |
|---|---|---|
| Surfaces | `background`, `surface.background`, `elevated_surface.background`, `panel.background`, `editor.background`, `title_bar.background`, `status_bar.background`, `tab_bar.background`, `toolbar.background` | 3 tiers: app background → surface → elevated (popover, menu). |
| Element | `element.{background,hover,active,selected,disabled}` | Buttons, inputs that have a background. |
| Ghost element | `ghost_element.{background,hover,active,selected,disabled}` | Elements with no background (list items, icon buttons). |
| Text | `text`, `text.muted`, `text.placeholder`, `text.disabled`, `text.accent` | |
| Icon | `icon`, `icon.muted`, `icon.disabled`, `icon.placeholder`, `icon.accent` | |
| Border | `border`, `border.variant`, `border.focused`, `border.selected`, `border.transparent`, `border.disabled` | |
| Editor | `editor.{foreground,background,gutter.background,subheader.background,active_line.background,highlighted_line.background,line_number,active_line_number,invisible,wrap_guide,active_wrap_guide,document_highlight.read_background,…}` | 14 keys. |
| Scrollbar, tab, pane, search | `scrollbar.thumb.*`, `tab.{active,inactive}_background`, `pane.focused_border`, `search.match_background`, `drop_target.background`, `link_text.hover` | |
| Terminal | `terminal.{background,foreground,bright_foreground,dim_foreground}`, `terminal.ansi.{black…white}` + `bright_*` + `dim_*` | 28 keys. |
| Version control | `version_control.{added,modified,deleted,conflict_marker.ours,…}` | 7 keys. |
| Status | `created, modified, deleted, conflict, renamed, ignored, hidden, unreachable, predictive, error, warning, info, hint, success` | Each has 3 variants: `X`, `X.background`, `X.border`. |
| Players | `players: [{cursor, background, selection}]` × 8 | Player 0 is the user's own cursor/selection; selection = the accent color at alpha ~0x3d. |
| Accents | `accents: [...]` | Colors for rainbow brackets/indents; One does not declare them (uses the default). |
| Syntax | `syntax.<capture>: {color, font_style, font_weight}` | 46 captures: `comment`, `comment.doc`, `keyword`, `string`, `string.escape/regex/special`, `function`, `type`, `number`, `constant`, `boolean`, `variable`, `variable.special`, `property`, `punctuation.*`, `tag`, `attribute`, `title`, `emphasis*`, `link_*`, `diff.plus/minus`, `hint`, `predictive`… Names match the tree-sitter highlights captures, looked up by longest prefix. |

### 1.3 Choosing a theme by system appearance
- The `theme` setting takes a string (a theme name) or an object `{ "mode": "system" | "light" | "dark", "light": "...", "dark": "..." }`.
- Default: `{ mode: "system", light: "One Light", dark: "One Dark" }`, meaning **it follows the system by default**.
- The toggle command (`cmd-k cmd-shift-t`) converts the static string into an object, then changes only `mode`.
- `theme_overrides` (per theme name) lets you override individual keys, including `syntax` and `accents`.

### 1.4 Contrast handling
- Zed uses **APCA**, not WCAG 2.x. `crates/ui/src/utils/apca_contrast.rs` has `apca_contrast(text, bg)` (Lc about −108..106) and `ensure_minimum_contrast(fg, bg, min_lc)`. This function adjusts lightness first, keeping hue and saturation, then adjusts saturation, and goes to black/white only when forced to.
- The settings that use it: `terminal.minimum_contrast` (default **45**, "minimum for large text") and `minimum_contrast_for_highlights` (default **45**, for editor text on a highlight background). A value of 0 turns the adjustment off.
- Thresholds noted in the code: Lc 45 for large text, 60 for other content, 75 for body text, 90 is the level body text should reach.
- The One theme itself does **not** meet the standard for comments (see §3).

## 2. VS Code

### 2.1 The default theme: changed to "2026"
- On the current `main` branch, `ThemeSettingDefaults` is `COLOR_THEME_DARK = 'Dark 2026'`, `COLOR_THEME_LIGHT = 'Light 2026'`, `HC_DARK = 'Default High Contrast'`, `HC_LIGHT = 'Default High Contrast Light'`. The files are at `extensions/theme-defaults/themes/2026-{dark,light}.json` and inherit from `dark_modern` / `light_modern`.
- The docs still list Dark Modern / Light Modern as the default; the accessibility page still says "Dark+", so it is even older.
- Inheritance chain: `2026-dark → dark_modern → dark_plus → dark_vs`, `light_modern → light_plus → light_vs`. Each tier adds `colors` (UI) or `tokenColors` (TextMate scopes); later tiers override earlier ones.
- `workbench.colorTheme` defaults to a dark theme (on the web, light).

### 2.2 Main color table
| Key | Dark Modern | Light Modern | 2026 Dark | 2026 Light |
|---|---|---|---|---|
| editor.background | #1F1F1F | #FFFFFF | #121314 | #FFFFFF |
| editor.foreground | #CCCCCC | #3B3B3B | #BBBEBF | #202020 |
| sideBar/statusBar/activityBar/titleBar/panel | #181818 | #F8F8F8 | #191A1B | #FAFAFD |
| tab.activeBackground | #1F1F1F | #FFFFFF | #121314 | #FFFFFF |
| editorWidget.background | #202020 | #F8F8F8 | #202122 | #FAFAFD |
| input.background | #313131 | #FFFFFF | #191A1B | #FFFFFF |
| border (widget/sideBar) | #313131 / #2B2B2B | #E5E5E5 | #2A2B2C | #E2E2E5 / #F0F1F2 |
| focusBorder | #0078D4 | #005FB8 | #3994BCB3 | #0069CC |
| button.background | #0078D4 | #005FB8 | #297AA0 | #0069CC |
| descriptionForeground (muted) | #9D9D9D | #3B3B3B | #8C8C8C | #606060 |

Observations:
- Chrome (sidebar, status bar, activity bar, title bar) uses **a single color**, one step darker (dark) or grayer (light) than the editor, separated by a 1px border. The "Modern" style drops the dark blue status bar.
- 2026 Dark has a near-black background, a **slightly cool-leaning** gray (#121314, #191A1B). 2026 Light has a gray with a slight purple-blue tint (#FAFAFD, #E2E2E5). The accent shifts to a soft cyan-blue (#297AA0).
- 2026's tokens borrow a lot from GitHub's Primer palette: string #a5d6ff / #0a3069, function #d2a8ff / #8250df, comment #8b949e / #6e7781.

### 2.3 Related settings
| Setting | Default | Meaning |
|---|---|---|
| `workbench.colorTheme` | Dark 2026 (web: Light 2026) | The theme in use. |
| `window.autoDetectColorScheme` | **false** | When on, picks the theme by the OS light/dark mode. |
| `workbench.preferredDarkColorTheme` / `preferredLightColorTheme` | Dark 2026 / Light 2026 | Used when auto-detect is on. |
| `window.autoDetectHighContrast` | **true** | Follows the OS high-contrast mode. |
| `workbench.preferredHighContrastColorTheme` / `…HighContrastLightColorTheme` | Default High Contrast (Light) | |
| `workbench.colorCustomizations`, `editor.tokenColorCustomizations`, `editor.semanticTokenColorCustomizations` | `{}` | Overrides, can be per theme using the key `"[Theme name]"`. |
| `terminal.integrated.minimumContrastRatio` | **4.5** (WCAG AA) | Changes the luminance of terminal text until the ratio is met, per **WCAG 2.x**. |

### 2.4 Contrast policy
- The docs make no WCAG commitment for the default themes. They rely on the high-contrast themes (available on every platform, switched on automatically per the OS) and suggest third-party themes that meet WCAG 2.1.
- In practice (§3), every main token color in Modern and 2026 is ≥ 4.5:1. Only Dark Modern's line numbers reach just 3.59.

## 3. WCAG 2.x contrast against `editor.background`

Script: `/private/tmp/claude-501/-Users-duykhanh-Projects-nosync-lucerna/de749e45-9d8b-4878-bf3c-b6ae4400ac50/scratchpad/contrast.py`, run with `python3 -I`. VS Code colors are resolved through the `include` chain, taking the longest matching scope: keyword = `keyword.control`, string = `string.quoted`, type = `entity.name.type`. For Zed, "muted" is `text.muted`; for VS Code it is `descriptionForeground`. ✗ = below 4.5:1, (L) = reaches only 3:1 (AA for large text).

| Theme (bg) | text | muted | line nr | comment | keyword | string | function | type | number |
|---|---|---|---|---|---|---|---|---|---|
| One Dark (#282c33) | 10.57 | 6.37 | 1.97 ✗ | **2.32 ✗** | 4.33 (L) | 6.98 | 5.93 | 5.97 | 5.15 |
| One Light (#fafafa) | 14.67 | 6.80 | 1.98 ✗ | **2.41 ✗** | 4.88 | 3.03 (L) | 3.80 (L) | 3.99 (L) | 4.00 (L) |
| Dark Modern (#1F1F1F) | 10.26 | 6.08 | 3.59 (L) | 4.95 | 5.92 | 6.24 | 11.66 | 8.09 | 9.70 |
| Light Modern (#FFFFFF) | 11.20 | 11.20 | 4.59 | 5.14 | 5.46 | 7.85 | 6.10 | 4.59 | 4.60 |
| 2026 Dark (#121314) | 9.95 | 5.53 | 5.21 | 6.05 | 6.68 | 12.10 | 9.55 | 9.13 | 10.95 |
| 2026 Light (#FFFFFF) | 16.29 | 6.29 | 6.29 | 4.55 | 5.46 | 12.81 | 5.05 | 4.59 | 4.60 |
| Ayu Dark (#0d1016) | 10.13 | 5.44 | 2.22 ✗ | 3.30 (L) | 8.39 | 11.53 | 10.72 | 9.60 | 9.65 |
| Ayu Mirage (#242835) | 8.95 | 5.21 | 2.20 ✗ | 2.55 ✗ | 7.98 | 12.79 | 10.22 | 8.46 | 9.12 |
| Ayu Light (#fcfcfc) | 6.10 | 3.21 (L) | 2.05 ✗ | 2.13 ✗ | 2.29 ✗ | 2.42 ✗ | 1.89 ✗ | 2.84 ✗ | 3.29 (L) |

APCA |Lc| for reference, on the exact scale Zed uses: comment One Dark ≈ 17.5, One Light ≈ 46.5, Dark Modern ≈ 39.1, 2026 Dark ≈ 43.4, Light Modern ≈ 74.6, 2026 Light ≈ 71.6. Text One Dark ≈ 83, 2026 Dark ≈ 67.

Conclusions:
- **One Dark/Light** (Lucerna's current default) has comments at about 2.3–2.4:1 and line numbers at about 2:1, very faint. One Light also has 4 token colors that reach only 3:1.
- Ayu Light fails nearly everything. Ayu Dark and Mirage have bright tokens but faint comments and line numbers.
- VS Code Modern and 2026 are the only group with **every main token ≥ 4.5:1**, comments included. With 2026, even line numbers. This is a sign that VS Code deliberately holds to AA. 2026 Dark raises comments from 4.95 to 6.05 and line numbers from 3.59 to 5.21 compared with Dark Modern.

## 4. What Lucerna chooses

**Format: follow Zed.** Lucerna's default theme is a JSON family file following the Zed v0.2.0 schema (`themes[]`, `appearance`, `style`), using exactly the key set in §1.2: surfaces/element/ghost_element/text/icon/border, `editor.*`, `terminal.ansi.*`, `version_control.*`, status ×3, `players`, `accents`, `syntax`. Placed next to `one.json` in `crates/lucerna-theme/assets/`. Keep One Dark/One Light as secondary themes to choose from, and keep the in-code fallback for missing keys.

**Behavior and settings: follow VS Code.**
- `workbench.colorTheme` (default is the name of Lucerna's dark theme), `workbench.preferredDarkColorTheme`, `workbench.preferredLightColorTheme`.
- `window.autoDetectColorScheme` is a **separate** behavior that needs a decision in the spec. VS Code defaults to `false`, Zed defaults to `mode: system`. For a minimal editor on macOS, turning it on by default benefits users, but the project principle is behavior following VS Code. The spec should pick one explicitly and record the reason. If following VS Code, leave it `false`.
- Can be done later: `workbench.colorCustomizations` / `editor.tokenColorCustomizations` (overrides per `"[Theme name]"`), `window.autoDetectHighContrast` along with a high-contrast theme, and `terminal.integrated.minimumContrastRatio` (4.5, WCAG) for the terminal.

**Practices worth learning:**
1. **Comments ≥ 4.5:1** (WCAG) or Lc about 60 (APCA). Comments should recede but stay readable. This is One's biggest weakness.
2. **Every main token ≥ 4.5:1**, including on light backgrounds (One Light and Ayu Light fail here). Line numbers and `text.muted` ≥ 4.5; line numbers at least 3:1.
3. **Neutral grays tinted slightly toward the accent hue.** One uses a blue-gray at ~220°, 2026 Light uses #FAFAFD/#E2E2E5. Very low saturation so the chrome harmonizes with the accent without looking "colored".
4. **Few surface tiers, small differences.** Chrome (sidebar, status, title, tab bar) uses one color, one step off the editor; separated by a 1px border rather than heavy color blocks. Zed's app background → surface → elevated trio is for popovers and menus.
5. **A single accent** for the focus border, the primary button, `text.accent`, player 0 and the selection (the accent at alpha about 0x30–0x40), like 2026 and One.
6. **Each status color has 3 variants** (`X`, `X.background`, `X.border`). Created/modified/deleted are shared with `version_control.*` so the git gutter, explorer and diff are consistent.
7. **A dark/light pair on the same hue map:** the same hue for keyword, string, function…, changing only lightness. Switching themes keeps the color memory.
8. **Automated contrast checks:** add a test in `lucerna-theme` that computes the WCAG ratio for the important token/background pairs to hold the threshold when colors are tuned. APCA can be added later if dynamic adjustment like Zed's (`minimum_contrast_for_highlights`) is needed.
