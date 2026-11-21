# Colored file icons (Seti) and icons on tabs: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bring VS Code's Seti icon theme in as the default icon theme, with colors by file type and separate colors for light themes. Add file icons to editor tabs, with the setting `workbench.editor.showIcons`.

**Architecture:**
- `scripts/gen-seti.py` runs once. It reads the downloaded seti-ui and vscode, then generates SVGs with a normalized viewBox plus two Rust data files, all committed to the repo.
- `lucerna-theme` adds a "Seti" theme kind with its own matching order like VS Code, and an `IconSource::Glyph` variant carrying the glyph name plus the dark/light colors.
- `lucerna-workbench` embeds the SVGs and tints them by the color theme's appearance. The Explorer, the file lists and tabs all draw through `ui::file_icon_sized`.

**Tech Stack:**
- Rust, GPUI (`svg()` uses the SVG as a mask and then fills it with `text_color`).
- Python 3 for the generator script, using `svgelements` (MIT) to compute the drawing box. This library is only installed in a temporary venv, it is not a dependency of the app.

**Spec:** `docs/superpowers/specs/2026-11-15-file-icons-design.md`. More references: `docs/research/2026-11-15-file-icons.md`.

## Global Constraints

- Work in a worktree on branch `claude/file-icons`. No merging, no pushing.
- Cargo: `export PATH="/opt/homebrew/opt/rustup/bin:$PATH"`. Use the shared `target` folder; do **not** set `CARGO_TARGET_DIR` or `--target-dir`.
- Run `scripts/prune-target.sh` before every build or `cargo test`. Before `cargo test`, delete `target/release`, unless another session's cargo is running (`pgrep -x cargo`). The disk must keep ≥ 1.2 GB free.
- Only `lucerna-workbench` may use gpui. `lucerna-theme` does not hold SVG bytes.
- No macOS-only assumptions outside platform code.
- Pin the sources: seti-ui commit `2d6c5e68` (the commit that `vs-seti-icon-theme.json` records). Take vscode at a specific commit and write that commit at the top of the generated files.
- Light theme colors: WCAG contrast ≥ 3:1 against `#ffffff`. Dark themes use exactly VS Code's colors.
- The theme name is `"Seti"`, the alias is `"vs-seti"`. The setting `workbench.iconTheme` defaults to `"Seti"`, `workbench.editor.showIcons` defaults to `true`.
- License: MIT OR Apache-2.0; `cargo deny check` must pass. Commits carry the trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Stage files by explicit path.
- Zed is GPL: do not copy code.

## Review Focus

1. **Unusually capitalized names** (`DOCKERFILE`, `Readme.MD`, `.ENV`) must match like VS Code, that is case-insensitively. The test is in Task 2.
2. **File names with several dots or starting with a dot** (`.env`, `.env.local`, `a.d.ts`, `x.test.ts`, `archive.tar.gz`) must get the icon of the longest extension Seti has, and never panic. The test is in Task 2.
3. **Switching between a light and a dark color theme while the app is running** changes the icon colors in the Explorer and on tabs right away, without reopening. The test is in Task 4.
4. **A user who set `"workbench.iconTheme": "Lucide"`** keeps Lucide after upgrading. An unknown name gets Seti without overwriting the setting. The test is in Task 3.
5. **The tab of a deleted file, a preview tab (italic text) and a diff tab** still have a file icon. With `showIcons` off no tab has an icon, including the Search tab. The test is in Task 5.

---

### Task 0: Worktree and the "before" screenshots

**Files:**
- Create: `docs/research/file-icons-screenshots/before-dark.png`, `before-light.png`

- [ ] **Step 1: Create the worktree.** Create `claude/file-icons` from `main` with the skill `superpowers:using-git-worktrees`, placed under `.claude/worktrees/`. Copy the spec, research and this plan (not yet committed on main) into the worktree, then commit on the branch: `git add docs/superpowers/specs/2026-11-15-file-icons-design.md docs/research/2026-11-15-file-icons.md docs/superpowers/plans/2026-11-15-file-icons.md`, `git commit -m "docs(file-icons): spec, research, plan" -- <3 paths>`. Do not commit these files to main.
- [ ] **Step 2: Build the sample folder.** Create `/private/tmp/.../scratchpad/icons-demo/` (in the session's scratchpad) containing: `src/app.ts`, `src/app.test.ts`, `src/main.rs`, `Dockerfile`, `docker-compose.yml`, `.env`, `.env.example`, `.gitignore`, `package.json`, `tsconfig.json`, `README.md`, `LICENSE`, `Cargo.toml`, `notes.txt`, `logo.png` (a 1×1 image), and `Makefile`.
- [ ] **Step 3: Take the "before" shots.** Build debug from the current main code (nothing changed yet), run `luc <sample folder>`, open the tabs `app.ts`, `Dockerfile` and `.env`, expand the `src` folder. Capture the window in Lucerna Dark and Lucerna Light: switch with `workbench.colorTheme` and `"window.autoDetectColorScheme": false` in a temporary user settings file. Use `screencapture -l <windowid>` or the computer-use tool. Save the two images in the folder above. Commit.

### Task 1: Script generating the Seti data, SVGs and license

**Files:**
- Create: `scripts/gen-seti.py`
- Create (generated): `crates/lucerna-theme/src/seti_data.rs`, `crates/lucerna-workbench/src/seti_svgs.rs`, `crates/lucerna-workbench/assets/icons/seti/*.svg`
- Create: `crates/lucerna-workbench/assets/icons/seti/LICENSE-seti-ui` (verbatim `LICENSE.md` of seti-ui), `crates/lucerna-workbench/assets/icons/seti/LICENSE-vscode` (verbatim `LICENSE.txt` at the vscode root), `crates/lucerna-workbench/assets/icons/seti/README.md` (the source, the two commits, a note "the viewBox has been reset; the drawing paths are unchanged")
- Modify: `scripts/gen-licenses.py` (`ASSETS`: add `("Seti icons (seti-ui)", ".../seti/LICENSE-seti-ui")` and `("Seti icon theme mapping (VS Code)", ".../seti/LICENSE-vscode")`), then rerun it to update `THIRD_PARTY_NOTICES.md`

**Interfaces — Produces** (`seti_data.rs`, generated, `pub(crate)`; the top of the file has `// @generated by scripts/gen-seti.py — seti-ui <commit>, vscode <commit>. Do not edit.`):

```rust
pub(crate) struct SetiDef { pub glyph: &'static str, pub dark: u32, pub light: u32 } // 0xRRGGBB
pub(crate) const DEFAULT_DEF: &str = "_default";
pub(crate) const DEFS: &[(&str, SetiDef)];            // key "_typescript_1", sorted by key
pub(crate) const FILE_NAMES: &[(&str, &str)];         // Seti fileNames, lowercase name → def
pub(crate) const FILE_EXTENSIONS: &[(&str, &str)];    // Seti fileExtensions, lowercase, no leading dot
pub(crate) const LANG_FILE_NAMES: &[(&str, &str)];    // from contributes.languages[].filenames
pub(crate) const LANG_NAME_PREFIXES: &[(&str, &str)]; // filenamePatterns "X.*" → "x."
pub(crate) const LANG_NAME_SUFFIXES: &[(&str, &str)]; // filenamePatterns "*.x" → ".x"
pub(crate) const LANG_EXTENSIONS: &[(&str, &str)];    // contributes.languages[].extensions, leading dot dropped
```

`seti_svgs.rs` (generated): `pub(crate) const SETI: &[(&str, &[u8])] = &[("typescript", include_bytes!(...)), …];`, sorted by name, containing only the glyphs that `DEFS` uses.

- [ ] **Step 1: Download the sources into the scratchpad.** Download seti-ui at `2d6c5e68` and vscode (take the current `main` commit and record it) into two separate, new, empty folders in the scratchpad, with `gh api .../tarball/<sha>` or `git clone --depth`. Create a venv in the scratchpad and `pip install svgelements`. Run the script with `python -I`, passing paths as arguments: `scripts/gen-seti.py --seti <dir> --vscode <dir>`.
- [ ] **Step 2: Write `gen-seti.py`.** The script does the following:
  - read `extensions/theme-seti/icons/vs-seti-icon-theme.json`;
  - for each def key without `_light`: glyph = drop the leading `_`, drop the trailing `_<number>`; the dark color is its `fontColor`; the light color is the `fontColor` of `<key>_light`, then contrast-adjusted. If the `_light` version is missing, take the dark color times 0.9. If `fontColor` is missing (like `_todo`), take the color of `_default`;
  - check that the `light` block matches the main block by the rule `def` → `def + "_light"`; if not, stop with an error message;
  - walk `extensions/*/package.json` → `contributes.languages[]`, join `id` with the theme's `languageIds` and fill the `LANG_*` tables. Do it like `extensions/theme-seti/build/update-icon-theme.js` at the same commit, including the hard-coded lists in that file, if they affect the mapping. A `filenamePatterns` that is neither of the forms `X.*` and `*.x` is skipped with a printed warning;
  - every key is lowercase. A duplicate key in the same table keeps the first entry by extension name order, so the result is stable;
  - **SVG:** for each glyph used, read `seti-ui/icons/<glyph>.svg` and compute the real drawing box with `svgelements` (accounting for transforms). Set the new `viewBox` to a square of side `max(w,h) * (1 + 2*PAD)` centered on the box, with `PAD = 0.0625`. Drop `width`/`height`, keep everything else. Write to `assets/icons/seti/<glyph>.svg`;
  - **light contrast adjustment:** use the WCAG relative luminance formula. If `contrast(c, #ffffff) < 3.0`, lower the HSL L by 0.01 at a time, keeping H and S, until the first value reaching ≥ 3.0;
  - print statistics: the number of defs, the number of glyphs, the adjusted light colors (before → after).
- [ ] **Step 3: Run the script and check by eye.** Open a few SVGs (`default`, `lock`, `typescript`, `docker`, `babel`) in the browser pane or Quick Look: they must be square and centered.
- [ ] **Step 4: Add the licenses and update `gen-licenses.py`.** Edit `ASSETS`, run `scripts/gen-licenses.py THIRD_PARTY_NOTICES.md` (read the script's docstring for the right output path), then check with `grep -n "seti" THIRD_PARTY_NOTICES.md`.
- [ ] **Step 5: Commit.** Includes `scripts/gen-seti.py`, the two generated `.rs` files (not yet wired into a `mod`, Tasks 2 and 4 wire them), the folder `assets/icons/seti/`, `scripts/gen-licenses.py` and `THIRD_PARTY_NOTICES.md`. Message: `feat(icons): vendor Seti icons and generate the mapping`.

### Task 2: The Seti theme in `lucerna-theme`

**Files:**
- Create: `crates/lucerna-theme/src/seti.rs`
- Modify: `crates/lucerna-theme/src/lib.rs` (`mod seti; mod seti_data;`, re-export `seti_icon_theme`)
- Modify: `crates/lucerna-theme/src/icon_theme.rs`
- Test: `crates/lucerna-theme/tests/icon_theme.rs`, `crates/lucerna-theme/tests/seti.rs`

**Interfaces:**
- Consumes: `seti_data::*` (Task 1).
- Produces:
  - `IconSource::Glyph { name: &'static str, dark: u32, light: u32 }`: a new variant of `IconSource`, still deriving `PartialEq, Eq`.
  - `pub fn seti_icon_theme() -> IconTheme` (name `"Seti"`, appearance Dark).
  - `IconThemeRegistry::names()` → `["Seti", "Lucide", …extension]`.
  - `IconThemeRegistry::resolve(name)`: `"Seti"` or `"vs-seti"` for Seti; `"Lucide"` for Lucide; an extension name for that theme; an unknown name for **Seti**. Lucide is always the fallback layer of `ResolvedIcons`.
  - `pub const DEFAULT_ICON_THEME: &str = "Seti";` exported from the crate.
  - `pub fn seti_glyph_names() -> impl Iterator<Item = &'static str>`: the glyphs that `DEFS` uses, so the workbench can check that the SVGs are all embedded.

- [ ] **Step 1: Write the failing tests** in `tests/seti.rs`. The sample table `(path, glyph, dark)` takes the expected results from real VS Code: look up `vs-seti-icon-theme.json` together with the file's language, then write the hex value into the test. At minimum:

```rust
#[test] fn seti_matches_vscode_table()        // app.ts→typescript, a.spec.ts & a.test.ts→typescript (orange def),
                                              // a.d.ts, Dockerfile, DOCKERFILE, Dockerfile.dev, docker-compose.yml (pink docker),
                                              // .env, .env.example, .env.local (config), .gitignore (git), Cargo.toml, README.md, Readme.MD,
                                              // LICENSE, main.rs, package.json, tsconfig.json, Makefile, notes.txt, archive.tar.gz,
                                              // "noext", "weird.zzz" → glyph "default"
#[test] fn seti_own_entries_beat_language_entries()   // a file name that is in both FILE_NAMES and LANG_FILE_NAMES gets Seti's def
#[test] fn longest_extension_wins()                   // "a.spec.ts" takes "spec.ts", not "ts"
#[test] fn seti_has_no_folder_or_chevron_icons()      // directory_icon/chevron → None, so ResolvedIcons falls back to Lucide
```

  Unit tests in `src/seti.rs` (because `seti_data` is `pub(crate)`): `every_light_color_has_3_to_1_contrast` (every `DEFS`: `contrast(light, 0xffffff) >= 3.0`) and `defs_referenced_by_tables_exist` (every def in the tables is in `DEFS`).

  In `tests/icon_theme.rs`, change `unknown_theme_name_resolves_to_lucide` to `unknown_theme_name_resolves_to_seti` (`names() == ["Seti","Lucide"]`, `resolve("Gone").name() == "Seti"`), and add:

```rust
#[test] fn vs_seti_alias_resolves_to_seti()
#[test] fn extension_cannot_replace_seti()   // an extension with a theme named "Seti" is ignored, as with Lucide
#[test] fn lucide_still_resolvable()         // resolve("Lucide").name() == "Lucide"
```

  The existing Lucide tests stay as they are.

- [ ] **Step 2: Run the tests and see them fail.** `cargo test -p lucerna-theme`. Expected: compile error because `seti_icon_theme` and `IconSource::Glyph` do not exist yet.
- [ ] **Step 3: Implement.**
  - In `icon_theme.rs`, `IconTheme` gets one more field of a private enum type: `Mapping` for the current data and `Seti` for Seti. `file_icon`, `directory_icon` and `chevron` branch on this enum. The `Mapping` part keeps its behavior.
  - `seti.rs` builds the `HashMap<&'static str, &'static SetiDef>` tables once, from the `const`s of `seti_data`.
  - Matching order on the lowercase file name `n`:
    1. `FILE_NAMES[n]`;
    2. for each extension in longest-first order, that is the part after each `.` from left to right (`.env` gives `env`; `a.spec.ts` gives `spec.ts` then `ts`): `FILE_EXTENSIONS`;
    3. `LANG_FILE_NAMES[n]`;
    4. a prefix in `LANG_NAME_PREFIXES` that `n` starts with, choose the longest prefix; then a suffix in `LANG_NAME_SUFFIXES` that `n` ends with, choose the longest suffix;
    5. the extensions as in step 2, but looked up in `LANG_EXTENSIONS`;
    6. `DEFAULT_DEF`.
  - `name.find('.')` is UTF-8 safe because `.` is ASCII.
  - `IconThemeRegistry` holds `seti: Arc<IconTheme>` next to `lucide`. `ResolvedIcons` keeps its signature.
- [ ] **Step 4: Run the tests and see them pass.** `cargo test -p lucerna-theme`. Expected: all PASS.
- [ ] **Step 5: Commit.** Message: `feat(theme): built-in Seti icon theme, default for unknown names`.

### Task 3: Default setting and startup

**Files:**
- Modify: `crates/lucerna-settings/assets/default-settings.json` (`"workbench.iconTheme": "Seti"`; add `"workbench.editor.showIcons": true` next to `workbench.editor.enablePreview`)
- Modify: `crates/lucerna-workbench/src/app.rs:731-732` (`resolve(DEFAULT_ICON_THEME)`)
- Modify: `crates/lucerna-workbench/src/extensions/service.rs:236-263` (the fallback is `DEFAULT_ICON_THEME`; fix the doc comment)
- Modify: `crates/lucerna-workbench/src/app_state.rs:26-29` (doc comment)
- Test: `crates/lucerna-settings/tests/store.rs:223`, `crates/lucerna-workbench/tests/icon_theme_picker.rs`

**Interfaces:** Consumes `lucerna_theme::DEFAULT_ICON_THEME` (Task 2).

- [ ] **Step 1: Change the tests to the new behavior.**
  - `store.rs`: the default is `Some("Seti")`; add an assert that `workbench.editor.showIcons` defaults to `Some(true)`.
  - `icon_theme_picker.rs`: the picker list is `["Seti","Lucide","Ext Icons"]`. The unknown name `"Gone"` gives `"Seti"` and the setting is kept. The test's `init` function should resolve by the configured setting (call the public `resolve_icons` if there is one, or `resolve(DEFAULT_ICON_THEME)`) instead of hard-coding `"Lucide"`.
  - Add `#[gpui::test] fn lucide_setting_kept()`: settings `{"workbench.iconTheme":"Lucide"}` → `icon_name == "Lucide"` after opening the workbench, and after reloading the config.
- [ ] **Step 2: Run the tests and see them fail.** `cargo test -p lucerna-settings --test store` and `cargo test -p lucerna-workbench --test icon_theme_picker`. Expected: FAIL on the default values.
- [ ] **Step 3: Change the default, `app.rs` and `service.rs`.** No need to change the roughly 57 other tests that call `.resolve("Lucide")`: they call the Lucide name explicitly.
- [ ] **Step 4: Run the two commands of Step 2 again.** Expected: PASS.
- [ ] **Step 5: Commit.** Message: `feat(settings): Seti is the default icon theme; workbench.editor.showIcons`.

### Task 4: Draw the colored Seti icons

**Files:**
- Modify: `crates/lucerna-workbench/src/icons.rs` (`mod` for `seti_svgs`; `pub fn seti_svg(name: &str) -> Option<&'static [u8]>`, binary search over the sorted `SETI`)
- Modify: `crates/lucerna-workbench/src/lib.rs` (declare `mod seti_svgs;` if `icons.rs` is not the parent module)
- Modify: `crates/lucerna-workbench/src/ui.rs:84-107`
- Modify: `crates/lucerna-workbench/src/explorer/view.rs:1571-1581` (`icon_slot`), `excerpt_list/render.rs:223`, `locations_picker.rs:419`
- Test: `crates/lucerna-workbench/src/icons.rs` (unit), `crates/lucerna-workbench/tests/explorer_view.rs`

**Interfaces:**
- Consumes: `IconSource::Glyph` (Task 2), `seti_svgs::SETI` (Task 1).
- Produces:
  - `pub fn icon_tint(source: &IconSource, fallback: Hsla, appearance: Appearance) -> Hsla` in `ui.rs`. For `Glyph`, returns the `dark` or `light` color by appearance, alpha 1. For other kinds, returns `fallback`.
  - `pub fn file_icon_sized(source: &IconSource, size: Pixels, color: Hsla, appearance: Appearance) -> AnyElement`: adds the parameter `appearance`; `file_icon` adds it too. `Glyph` is drawn with `svg().data(seti_svg(name)).text_color(icon_tint(..))`. A glyph whose SVG is not found draws the Lucide `file`.
  - Every call site passes `cx.global::<AppState>().theme.appearance`.

- [ ] **Step 1: Write the failing tests.**

```rust
// icons.rs
#[test] fn every_seti_glyph_is_bundled()      // every lucerna_theme::seti_glyph_names() has seti_svg(..).is_some()
#[test] fn seti_svgs_have_square_viewbox()    // every SVG in SETI has a viewBox "x y w h" with w == h
// ui.rs (unit test in the module)
#[test] fn glyph_tint_follows_appearance()    // Glyph{dark:0x519aba, light:L} → Dark: hsla of #519aba; Light: hsla of L; Builtin → fallback
// tests/explorer_view.rs
#[gpui::test] fn seti_icons_in_explorer()     // AppState with resolve("Seti"); row_icon("x.ts") is IconSource::Glyph{name:"typescript",..};
                                              // a folder gives Builtin("folder")/("folder-open"); the row icon's debug_bounds exist after drawing
#[gpui::test] fn glyph_color_tracks_color_theme() // change state.set_theme(Lucerna Light) and redraw: icon_tint with the current appearance gives the light color
```

- [ ] **Step 2: Run the tests and see them fail.** `cargo test -p lucerna-workbench --lib icons ui` and `--test explorer_view`. Expected: compile error or FAIL.
- [ ] **Step 3: Implement.**
  - The Explorer keeps `ui.icon_muted` as `fallback`. Lucide and extension icons are unchanged.
  - Change the comment at `icon_slot`: Lucide icons are filled with `tint`, Seti glyphs are filled with their own color, extension icons keep their original color.
- [ ] **Step 4: Run the tests and see them pass.** Rerun the commands of Step 2, then `cargo test -p lucerna-workbench --test perf`. Expected: PASS.
- [ ] **Step 5: Commit.** Message: `feat(icons): draw Seti glyphs in their colors, light colors on light themes`.

### Task 5: File icons on tabs and `workbench.editor.showIcons`

**Files:**
- Modify: `crates/lucerna-workbench/src/editor_groups/view.rs:655-677`
- Test: `crates/lucerna-workbench/tests/editor_groups_view.rs`

**Interfaces:** Consumes `file_icon_sized(…, appearance)` (Task 4) and `state.icons.file(path)`.

- [ ] **Step 1: Write the failing tests** (use the existing `init` and `open` in the test file; for `init`, change `icons` to `resolve("Seti")` only in the new tests, with a helper `init_seti`):

```rust
#[gpui::test] fn file_tabs_show_file_icons()       // open a.ts (preview) and b.rs (pinned): after drawing there are "tab-icon-0-0" and "tab-icon-0-1"
#[gpui::test] fn deleted_file_tab_keeps_icon()     // a tab with strikethrough still has "tab-icon-…" (use the way the external_changes tests mark a deleted file)
#[gpui::test] fn file_diff_tab_shows_file_icon()   // TabItem::FileDiff(path): the icon is state.icons.file(path), not GIT_BRANCH
                                                   // (check through `EditorGroupsView::tab_icon(gi, ti, cx) -> Option<IconSource>`, see Step 3)
#[gpui::test] fn show_icons_false_hides_all_tab_icons() // user settings {"workbench.editor.showIcons": false}: neither file tabs nor the Search tab have "tab-icon-0-*"
#[gpui::test] fn show_icons_toggle_redraws()       // set_layer(User) changes true→false, reload or refresh, redraw: the icon disappears without reopening the tab
```

- [ ] **Step 2: Run the tests and see them fail.** `cargo test -p lucerna-workbench --test editor_groups_view`. Expected: FAIL.
- [ ] **Step 3: Implement.**
  - Split choosing the tab's icon into `pub fn tab_icon(&self, gi: usize, ti: usize, cx: &App) -> Option<IconSource>` on the view (used by the tests). The function returns:
    - `TabItem::File(p)` and `TabItem::FileDiff(p)`: `Some(icons.file(p))`;
    - special tabs: `Some(IconSource::Builtin("<lucide name>"))`, matching the existing constants (`blocks`, `search`, `zap`, `file-code`, `settings`, `git-branch`, `eye`);
    - when the setting `workbench.editor.showIcons` (read like `BreadcrumbSettings::read`, default `true`) is `false`: `None`.
  - The drawing part uses `file_icon_sized(&src, px(14.), color(ui.icon_muted).into(), appearance)` inside a `div().flex_none()` with `debug_selector("tab-icon-{gi}-{ti}")`. That way `BLOCKS_ICON` and `SEARCH_ICON` go through `icons::lucide_svg`, the same drawing path as file icons.
  - The `ui::tab` layout stays as it is.
- [ ] **Step 4: Run the tests and see them pass.** Run `--test editor_groups_view`, `--test search_view`, `--test extensions_page`, `--test markdown_preview`. The last three already check `tab-icon-0-0`. Expected: PASS.
- [ ] **Step 5: Commit.** Message: `feat(tabs): file icons on editor tabs; workbench.editor.showIcons`.

### Task 6: "After" screenshots, docs and final checks

**Files:**
- Create: `docs/research/file-icons-screenshots/after-dark.png`, `after-light.png`
- Modify: `docs/research/2026-11-15-file-icons.md` (section "What Lucerna chooses": add the result and the images), the spec (status: implemented)

- [ ] **Step 1: Take the "after" shots.** Build, run on the same sample folder, with the same tabs and the same two themes as in Task 0. Compare the two pairs of images:
  - `.ts` is blue, `Dockerfile` has a whale, `.env` has a gear;
  - the icon on the tab is the same size as the icon in the Explorer;
  - icons on the light theme are still easy to see.
  
  If the icons look much bigger or smaller than Lucide at 14 px, adjust `PAD` in `gen-seti.py`, rerun the script, then write the reason into the plan.
- [ ] **Step 2: Final checks.**
  - `cargo test -p lucerna-theme -p lucerna-settings -p lucerna-workbench`, then `cargo test --workspace`.
  - `cargo clippy --workspace --all-targets -- -D warnings`.
  - `cargo deny check`.
  - `python3 scripts/gen-licenses.py <output path>`: no diff beyond what has been committed.
  
  Expected: everything passes.
- [ ] **Step 3: Commit the images and docs.** Write the handover into the shared notes folder (`file-icons-handover.md`): the branch, the commits, the points to watch when merging with the other branches that edit `editor_groups/view.rs`, `ui.rs` and `default-settings.json`.
