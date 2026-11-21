# Lucerna theme — design

Date: 2026-11-14. Status: Approved.

## Intent

Lucerna currently defaults to One Dark / One Light taken from Zed. Before the
first release, Lucerna needs a theme of its own. Lucerna's users come from
Claude (desktop app, Claude Code) and read diffs on GitHub every day. So the
new theme must feel **instantly familiar** to them: code colors follow the
"Claude" code theme of Claude desktop, and the missing colors (diagnostics,
git, terminal) follow GitHub. Lucerna's identity lies in the theme name and
in the brand blue accent `#4f8cff` (`assets/brand/README.md`).

## Goals

- Two themes, **Lucerna Dark** and **Lucerna Light**, in one Zed-format JSON
  file, being the default theme and the only theme shipped with Lucerna.
- Every color used for text reaches **WCAG AA, 4.5:1** on the background it
  appears on, including comments and line numbers. A test guarantees this.
- Cover **every color key** of the Zed schema that One has (141 style keys,
  `players`, `syntax`), including `terminal.ansi.*`, `version_control.*` and
  the status colors `created/modified/deleted/conflict/ignored/renamed`. That
  way the Git and Terminal branches that merge in have colors immediately.
- Lucerna switches theme automatically with the system light/dark mode
  (`window.autoDetectColorScheme`, **on by default**).

## Non-goals

- Not keeping One. `one.json` and `LICENSE-one` are deleted. Anyone who wants
  One can drop the file into the user's themes folder or install an
  extension; both already work.
- No high contrast theme or `window.autoDetectHighContrast`.
- No rainbow brackets. Claude colors brackets by nesting level, but Lucerna
  has only one color for `punctuation.bracket`.
- No "Toggle Light/Dark Theme" command.
- No change to the logo colors.

## Anti-goals

- A theme that looks like One or Zed renamed: users must recognize Claude's
  colors.
- Any text color below 4.5:1 (except disabled text, which WCAG exempts).
- A garish UI: the accent is used only for focus, the cursor, links and the
  unsaved dot. The UI chrome is nearly monochrome.

## Constraints

- JSON format and key names follow Zed (UI follows Zed). Settings and
  behavior follow VS Code (`docs/design-principles.md` §1).
- License: we only learn Claude's palette (it is not open source) by
  measuring colors from screenshots; no file from Claude is copied. Colors
  taken from GitHub Primer / `primer/github-vscode-theme` (MIT) must be
  credited in `THIRD_PARTY_NOTICES.md` and `scripts/gen-licenses.py`.
- Only `lucerna-workbench` uses gpui. Reading the system appearance lives in
  the workbench; `lucerna-theme` stays pure data.
- No macOS-only assumption: GPUI has `window_appearance()` on all three
  platforms.

## References

Details are in `docs/research/2026-11-14-default-theme.md`. Summary:

| | Zed | VS Code | Lucerna chooses |
|---|---|---|---|
| Format | Family JSON `$schema` v0.2.0, `themes[]` with `appearance` + `style` | `colors` + `tokenColors` (TextMate) | Zed |
| Default theme | One Dark / One Light | Dark Modern / Light Modern (new main: Dark 2026 / Light 2026) | Lucerna Dark / Lucerna Light |
| Follow system | Setting `theme` defaults to `{mode: "system", light, dark}` | `window.autoDetectColorScheme`, default `false`; `workbench.preferredDarkColorTheme` / `preferredLightColorTheme` | VS Code's setting names, on by default like Zed |
| Contrast | APCA, `minimum_contrast` for the terminal | `terminal.integrated.minimumContrastRatio` 4.5 | WCAG 4.5 built into the palette, with a test |
| Comment | One Dark 2.32:1, One Light 2.41:1 | Dark Modern 4.95:1 | ≥ 4.5:1 |

UI chrome follows Zed: the regions (sidebar, tab bar, status bar, editor)
share one background color and are separated only by 1 px borders. Claude
desktop does the same (the sidebar is the same color as the editor).

Picker behavior when follow-system is on, following VS Code
(`themes.contribution.ts`, `themeConfiguration.ts`):
- `workbench.colorTheme` is ignored. The theme in use is
  `preferredDarkColorTheme` or `preferredLightColorTheme` depending on the
  system mode.
- The picker is titled "Select Color Theme for System Dark Mode" (or Light
  Mode) and writes the choice to the preferred setting matching the current
  mode.
- When follow-system is off, the picker title is
  "Select Color Theme (detect system color mode disabled)", and the picker
  writes to `workbench.colorTheme` as it does today.

## Approaches considered

1. **One hand-written Zed-format JSON file, with a contrast test** (chosen).
   Same way Zed organizes themes. Users can copy the file as a template for
   their own theme. The test blocks contrast regressions and missing keys.
2. A palette of about 30 tokens plus a script that generates the JSON. More
   consistent, but it means maintaining an extra script and a build step for
   a file that is rarely edited. Rejected.
3. Defining the theme in Rust code. Diverges from Zed, and users get no
   template file. Rejected.

Code colors: **keep them nearly identical to Claude**, adjusting only where
AA fails. The user chose this so that people coming from Claude feel at home
immediately.

## Design

### Palette

The colors below were measured from screenshots of the "Claude Light" /
"Claude Dark" code theme of Claude desktop (`[C]`), taken from GitHub Primer
(`[G]`), or are brand colors (`[B]`). The ratio in parentheses is the WCAG
contrast against the editor background. A row without a mark is a builder
decision (see the end of the spec).

**Background and text**

| Role | Dark | Light |
|---|---|---|
| Shared background (editor, sidebar, tab bar, status bar, title bar) | `#191919` [C] | `#ffffff` [C] |
| Elevated background (popover, Quick Open, menu) | `#222222` | `#ffffff` + border |
| Current line | `#1f1f1f` (Claude `#252525`, lightened slightly so comments reach 4.5) | `#fafafa` |
| Selected row (Explorer, list) | `#2a2a2a` (Claude `#302f2f`, lightened slightly) | `#ebebeb` (Claude `#e6e6e6`, lightened slightly) |
| Row under the mouse | `#252424` [C] | `#f0f0f0` |
| Border / secondary border | `#2e2e2e` / `#252525` | `#e5e5e5` / `#ededed` |
| UI text | `#efefef` [C] (15.3) | `#191919` [C] |
| Editor text | `#d3d7de` [C] (12.2) | `#2b303b` [C] (13.2) |
| Muted text (`text.muted`, line numbers) | `#9c9c9c` [C] (6.4) | `#666666` [C] (5.7) |
| Current line number | `#efefef` | `#191919` |
| Accent: focus border, cursor, unsaved dot | `#4f8cff` [B] (5.5) | `#4f8cff` [B] (border, cursor: 3.2, enough for 3:1 for non-text components) |
| Accent as text (`text.accent`, links, `renamed`) | `#5c96ff` (brand blue slightly lighter, 6.0; 4.96 on the selected row) | `#1a5dcc` (brand blue darker, 5.9; 5.06 on the selected row) |
| Selection | `#4f8cff` alpha ≈ 0x40 | `#4f8cff` alpha ≈ 0x33 |

**Syntax**

| Capture | Dark | Light |
|---|---|---|
| `keyword`, `preproc`, `variable.special`, `label` | purple `#cc7bf4` [C] (6.5) | purple `#8101c2` [C] (7.7) |
| `function` (including method, macro) | blue `#70b8ff` [C] (8.4) | blue `#0051c2` [C] (7.1) |
| `type`, `constructor`, `enum`, `variant`, `variable.parameter`, `attribute`, `selector` | orange `#f7ab5f` [C] (9.5) | earth orange `#b34a02` [C] (5.4) |
| `string`, `text.literal` | green `#9be963` [C] (11.9) | green `#027900` [C] (5.6) |
| `number`, `boolean`, `constant`, `string.escape`, `string.regex`, `string.special`, `link_uri` | cyan `#5eeded` [C] (12.4) | teal `#007a7a` (Claude `#008080`, slightly darker to reach 4.5 on the current line) |
| `property`, `tag` | pink-red `#f37c85` [C] (6.7) | red `#b80918` [C] (6.8) |
| `punctuation.bracket` | yellow `#ffd012` [C] (12.0) | brown `#945c00` [C] (5.5) |
| `comment`, `comment.doc` | `#818898` [C] (4.95; 4.64 on the current line) | `#677080` (Claude `#6d7686`, slightly darker; 4.78 on the current line) |
| `title` (markdown heading) | function blue, bold | function blue, bold |
| `link_text` | accent as text | accent as text |
| `emphasis` / `emphasis.strong` | italic / bold | italic / bold |
| `variable`, `namespace`, `operator`, remaining `punctuation`, `embedded`, `primary` | editor text | editor text |
| `hint`, `predictive` | muted text, italic | muted text, italic |
| `diff.plus` / `diff.minus` | `created` / `deleted` colors | `created` / `deleted` colors |

**Diagnostics** [G]: error `#f85149` / `#d1242f`, warning `#d29922` /
`#9a6700`, info = accent as text, hint `#9198a1` / `#59636e`, success
`#3fb950` / `#1a7f37`. The `<severity>.background` fill is that color with
low alpha (dark) or a light tint (light). The `<severity>.border` is one step
darker than the fill.

**Git**

| Key | Dark | Light |
|---|---|---|
| `modified` (Explorer) | `#e1c08d` [C] | `#895503` [C] |
| `created` | `#7ee787` [G] | `#116329` [G] |
| `deleted` | `#ff7b72` [G] | `#c21d29` (Primer `#cf222e`, slightly darker for the selected row) |
| `conflict` | `#f0883e` [G] | `#a84400` (Primer `#bc4c00`, slightly darker for the selected row) |
| `renamed` | accent as text | accent as text |
| `ignored`, `hidden` | muted text | muted text |
| `version_control.added` (gutter) | `#37ac42` [C] | `#218635` [C] |
| `version_control.deleted` | `#ff707b` [C] | `#b7224c` [C] |
| `version_control.modified` | `#4f8cff` [B] | `#4f8cff` [B] |
| Added-line / deleted-line background (`editor.diff_hunk.*`, `created.background`, `deleted.background`) | `#253a24` / `#2e1e1f` [C] | `#e7f3e7` / `#feeeef` [C] |
| Added-text / deleted-text within a line (`version_control.word_added` / `word_deleted`) | `#2b5a2c` / `#782633` [C] | `#cce7ce` / `#f8d2d8` [C] |

**Terminal**: `terminal.ansi.*` (normal, bright, dim) follow the ANSI table of
`primer/github-vscode-theme` [G] for GitHub Dark and GitHub Light. Terminal
background and text use the editor background and text.

**Players**: player 0 is the accent cursor together with the selection above.
The other seven players take the syntax colors so the Zed schema is complete;
Lucerna currently reads only player 0.

### Contrast rules (tested)

`lucerna-theme` has a test computing WCAG 2.x contrast for both themes:

- UI text, muted text, `text.placeholder`, `text.accent`, line numbers
  (normal and current), every syntax color, diagnostic colors, git status
  colors and `terminal.foreground` reach ≥ 4.5:1 on `editor.background`.
- UI text, muted text and git status colors reach ≥ 4.5:1 on
  `element.selected`. This is where the Explorer draws file names on the
  selected row. A color that fails gets adjusted by the builder (and the spec
  updated); the threshold is not lowered.
- Every syntax color reaches ≥ 4.5:1 on `editor.active_line.background`.
- The accent, `border.focused` and `version_control.*` reach ≥ 3:1
  (non-text components, WCAG 1.4.11).
- Not tested: `text.disabled`, `terminal.ansi.black` (dark),
  `terminal.ansi.white` (light) and the `dim_*` colors.

Another test compares each Lucerna theme's key set with the full Zed key list
(copied once from `one.json` into the test before the file is deleted). A
theme missing keys fails the test.

### File changes and defaults

- Add `crates/lucerna-theme/assets/lucerna.json` (family "Lucerna"). Delete
  `one.json` and `LICENSE-one`. The JSON file's opening comment (jsonc allows
  it) states the color sources: Claude (measured from screenshots) and GitHub
  Primer (MIT).
- `registry.rs`: load `lucerna.json`. Change `DEFAULT_THEME` to "Lucerna Dark"
  and add the constant "Lucerna Light". The doc comment no longer says
  "One Dark".
- `UiColors::fallback` keeps its role (colors for user themes missing keys).
  The builder may change the values to match Lucerna, but is not required to.
- `THIRD_PARTY_NOTICES.md`: replace the One entry with a GitHub Primer entry.
  Add the file `crates/lucerna-theme/assets/LICENSE-primer` (MIT, verbatim
  from `primer/github-vscode-theme`).
- `scripts/gen-licenses.py`: remove the One line in `ASSETS`, add a Primer
  line. Run `python3 -I scripts/gen-licenses.py <temp file>` to check.

### Settings

In `default-settings.json`, keep the order and comments in the file's style:

```jsonc
// If enabled, automatically select a color theme based on the system color mode:
// `workbench.preferredDarkColorTheme` when it is dark, `workbench.preferredLightColorTheme` when it is light.
"window.autoDetectColorScheme": true,
// The color theme used when `window.autoDetectColorScheme` is disabled.
"workbench.colorTheme": "Lucerna Dark",
// The color theme used when the system color mode is dark and `window.autoDetectColorScheme` is enabled.
"workbench.preferredDarkColorTheme": "Lucerna Dark",
// The color theme used when the system color mode is light and `window.autoDetectColorScheme` is enabled.
"workbench.preferredLightColorTheme": "Lucerna Light",
```

Lucerna differs from VS Code in the default value of
`window.autoDetectColorScheme` (`true` instead of `false`). Reason: the user
wants it that way, and Zed also follows the system by default.

### Choosing the active theme

`apply_configured_theme` (`config_files.rs`) picks the theme name as follows:

1. If `window.autoDetectColorScheme` is `true`: read the system appearance
   (`cx.window_appearance()`). `Dark`/`VibrantDark` use
   `workbench.preferredDarkColorTheme`, `Light`/`VibrantLight` use
   `workbench.preferredLightColorTheme`.
2. If it is `false`: use `workbench.colorTheme`.
3. A name not in the registry falls back to "Lucerna Dark" as it does today
   (`get_or_default`).

The workbench watches for changes of the system appearance
(`observe_window_appearance` on the window, or the app-level equivalent) and
then calls `apply_configured_theme` again. It repaints only when the theme
actually changes (the `Arc::ptr_eq` check already exists). Changing
`window.autoDetectColorScheme` in settings takes effect immediately, like the
other theme settings.

Startup state: the theme must be right from the first frame, with no flash of
the dark theme before switching to light.

### Theme picker

- Follow-system on: the title is "Select Color Theme for System Dark Mode" or
  "… for System Light Mode". The preview stays as it is today. Enter writes to
  `workbench.preferredDarkColorTheme` or `workbench.preferredLightColorTheme`
  matching the current mode. Esc restores the old theme.
- Follow-system off: the title is "Select Color Theme (detect system color mode
  disabled)". Enter writes `workbench.colorTheme` as it does today.
- The list puts themes of the same mode as the system first, as VS Code does.

### Existing tests

About 15 test files in `lucerna-workbench/tests` and `theme_picker.rs`
hard-code "One Dark" / "One Light". They change to "Lucerna Dark" /
"Lucerna Light". The `config_files.rs` tests about `workbench.colorTheme`
need to set `window.autoDetectColorScheme: false` or check against the
appearance the test platform returns. The builder picks whichever is
simpler. Add tests for: choosing by appearance, changing appearance at
runtime, and the picker writing the correct preferred setting.

### Notes for merging the parallel branches

- **Git** (`claude/confident-murdock-5e4f69`): this branch adds `vc_*`,
  `status_*`, `diff_*` to `UiColors` with VS Code color fallbacks. The Lucerna
  theme already has all the corresponding Zed keys, so the theme needs no
  change. If that branch's tests use "One Dark", the name must be changed when
  merging.
- **Terminal** (`claude/epic-hellman-a76cab`): adds `TerminalColors` reading
  `terminal.*`. The Lucerna theme has all these keys. That branch's
  `tests/terminal.rs`, if it relies on specific One colors, must be fixed
  when merging.
- **Scrollbar** (`claude/objective-cannon-ee3762`): may additionally read
  `scrollbar.thumb.hover_background` or a similar key. The Lucerna theme has
  Zed's `scrollbar.*` keys.

Record these notes in the handoff note in the shared notes folder when done.

### Comparison screenshots

Capture the app before and after in both dark and light mode, with the same
Rust file that has a git diff and the same Explorer open. Save in
`docs/research/lucerna-theme-screenshots/` (`before-dark.png`,
`after-dark.png`, `before-light.png`, `after-light.png`).

## Left to the implementer

- The exact values of colors not marked `[C]`/`[G]`/`[B]`: elevated
  background, borders, hover, the alphas, `ghost_element.*`, `element.active`,
  `scrollbar.*`, `drop_target`, `search.match_background`,
  `editor.document_highlight.*`, `editor.wrap_guide`, `editor.invisible`,
  `title_bar.inactive_background`, `pane.focused_border`,
  `panel.focused_border`, `players` 1–7, `accents`. The only constraints: the
  contrast rules above, and keeping the UI chrome neutral.
- The syntax mapping for rare captures not in the table: use the color
  closest in meaning.
- Whether to update `UiColors::fallback` to match Lucerna.

## Decisions settled during the spec review

- **Loading extension themes at startup:** the first load reads only the
  extension theme files that contain the theme to be used. When follow-system
  is on, that is the preferred theme for the current appearance. Load both
  preferred themes, so switching the system mode does not have to wait.
- **No flash at startup:** `watch_config` runs before the window opens, and
  `cx.window_appearance()` is usable at the app level. The theme is chosen
  right there.
- **Picker title:** the picker has no title row yet, so VS Code's text goes in
  the input's placeholder, like VS Code's quick pick.
- **Tests:** GPUI's test platform always reports Light. The old tests that
  wait for "Lucerna Dark" set `window.autoDetectColorScheme: false`. The new
  tests for follow-system set a fake appearance, or check against Light.
- **Adjusting colors to meet the contrast rules:** the spec review found a few
  colors that fail (comment and teal on the current line,
  `renamed`/`deleted`/`conflict` on the selected row). The tables above
  already contain the adjusted values. The current line and selected row are
  slightly lighter than Claude's, instead of darkening the text colors.
